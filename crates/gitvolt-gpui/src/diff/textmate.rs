//! A TextMate tokenizer on Oniguruma: `vscode-textmate`'s algorithm, which is what Shiki runs, on the
//! same engine and with the same grammars and theme, so a file gets the web UI's colours. Tested
//! against Shiki's own output (`testdata/shiki-golden.json`).

use super::languages::LANGS;
use onig::{Regex, RegexOptions, Region, SearchOptions, Syntax};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub struct Lang {
    pub id: &'static str,
    pub scope: &'static str,
    pub aliases: &'static [&'static str],
    /// What its `injectionSelector` applies to, when it's loaded.
    pub inject_to: &'static [&'static str],
    /// The grammars Shiki loads with it, itself included: an injection needs its grammar loaded.
    pub deps: &'static [&'static str],
    pub gz: &'static [u8],
}

/// A scope selector's matcher (`vscode-textmate`'s `matcher.ts`).
#[derive(Debug, Clone, PartialEq)]
enum Matcher {
    Names(Vec<String>),
    Not(Option<Box<Matcher>>),
    And(Vec<Matcher>),
    Or(Vec<Matcher>),
}

/// Its tokens: `[LR]:`, a name, or one of `,|-()`; what's none of those is skipped.
fn selector_tokens(input: &str) -> Vec<String> {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == ':';
    let chars: Vec<char> = input.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if (c == 'L' || c == 'R') && chars.get(i + 1) == Some(&':') {
            out.push(format!("{c}:"));
            i += 2;
        } else if is_word(c) {
            let start = i;
            while i < chars.len() && (is_word(chars[i]) || chars[i] == '-') {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
        } else {
            if ",|-()".contains(c) {
                out.push(c.to_string());
            }
            i += 1;
        }
    }
    out
}

fn is_identifier(token: Option<&String>) -> bool {
    token.is_some_and(|t| t.chars().any(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == ':'))
}

struct SelectorParser {
    tokens: Vec<String>,
    at: usize,
}

impl SelectorParser {
    fn token(&self) -> Option<&String> {
        self.tokens.get(self.at)
    }

    fn operand(&mut self) -> Option<Matcher> {
        if self.token().is_some_and(|t| t == "-") {
            self.at += 1;
            return Some(Matcher::Not(self.operand().map(Box::new)));
        }
        if self.token().is_some_and(|t| t == "(") {
            self.at += 1;
            let inner = self.inner();
            if self.token().is_some_and(|t| t == ")") {
                self.at += 1;
            }
            return Some(inner);
        }
        if is_identifier(self.token()) {
            let mut names = Vec::new();
            while is_identifier(self.token()) {
                names.push(self.tokens[self.at].clone());
                self.at += 1;
            }
            return Some(Matcher::Names(names));
        }
        None
    }

    fn conjunction(&mut self) -> Matcher {
        let mut list = Vec::new();
        while let Some(m) = self.operand() {
            list.push(m);
        }
        Matcher::And(list)
    }

    fn inner(&mut self) -> Matcher {
        let mut list = Vec::new();
        loop {
            list.push(self.conjunction());
            if self.token().is_some_and(|t| t == "|" || t == ",") {
                while self.token().is_some_and(|t| t == "|" || t == ",") {
                    self.at += 1;
                }
            } else {
                break;
            }
        }
        Matcher::Or(list)
    }
}

/// A selector's alternatives, each with its priority: `L:` -1, `R:` 1, else 0.
fn parse_selector(selector: &str) -> Vec<(Matcher, i8)> {
    let mut p = SelectorParser { tokens: selector_tokens(selector), at: 0 };
    let mut out = Vec::new();
    while p.token().is_some() {
        let mut priority = 0;
        if let Some(t) = p.token().filter(|t| t.len() == 2 && t.ends_with(':')) {
            priority = match t.as_str() {
                "R:" => 1,
                "L:" => -1,
                _ => 0,
            };
            p.at += 1;
        }
        out.push((p.conjunction(), priority));
        if p.token().is_none_or(|t| t != ",") {
            break;
        }
        p.at += 1;
    }
    out
}

impl Matcher {
    /// `scopes`: outermost first.
    fn matches(&self, scopes: &[Rc<str>]) -> bool {
        match self {
            Matcher::Names(names) => {
                if scopes.len() < names.len() {
                    return false;
                }
                let mut last = 0;
                names.iter().all(|n| match scopes[last..].iter().position(|s| scope_matches(s, n)) {
                    Some(i) => {
                        last += i + 1;
                        true
                    }
                    None => false,
                })
            }
            Matcher::Not(inner) => inner.as_ref().is_some_and(|m| !m.matches(scopes)),
            Matcher::And(list) => list.iter().all(|m| m.matches(scopes)),
            Matcher::Or(list) => list.iter().any(|m| m.matches(scopes)),
        }
    }
}

struct Injection {
    matcher: Matcher,
    rule: RuleId,
    priority: i8,
}

/// What Shiki gives up on: a line this long stays plain, as does what a slow line didn't reach.
const MAX_LINE_BYTES: usize = 20_000;
const LINE_BUDGET: Duration = Duration::from_millis(if cfg!(test) { 60_000 } else { 500 });
pub const FONT_ITALIC: u8 = 1;
pub const FONT_BOLD: u8 = 2;
pub const FONT_UNDERLINE: u8 = 4;

/// A run of one style in a line: byte offsets into the line, without its line ending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub color: u32,
    pub font: u8,
}

type RuleId = usize;
type RegexId = usize;
type ScopeId = usize;
/// What a frame searches for, in order.
type Scanner = Rc<Vec<(RegexId, RuleId)>>;
/// A match's groups, as byte ranges; an unmatched group is empty.
type Groups = Rc<Vec<(usize, usize)>>;

struct Capture {
    name: Option<String>,
    content_name: Option<String>,
    retokenize: Option<RuleId>,
}

type Captures = Vec<Option<Capture>>;

enum Kind {
    /// Not compiled yet; a rule can reach itself through its includes.
    Pending,
    Match { re: RegexId, captures: Captures },
    Container { patterns: Vec<RuleId> },
    BeginEnd { begin: RegexId, end: String, begin_captures: Captures, end_captures: Captures, end_last: bool, patterns: Vec<RuleId> },
    BeginWhile { begin: RegexId, until: String, begin_captures: Captures, while_captures: Captures, patterns: Vec<RuleId> },
}

struct Rule {
    name: Option<String>,
    content_name: Option<String>,
    kind: Kind,
    /// Some of its patterns name what isn't there. With none left, nothing includes it.
    missing: bool,
}

/// A pattern with its `\A` and `\G` variants, compiled when first asked for: most never are.
struct Pattern {
    source: String,
    anchored: bool,
    compiled: [Option<Option<Rc<Regex>>>; 4],
    /// The line's last search: from where, and what it found.
    cache: [Option<CacheEntry>; 4],
}

struct CacheEntry {
    stamp: u64,
    from: usize,
    found: Option<Groups>,
}

fn has_anchor(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\\' {
            if bytes[i + 1] == b'A' || bytes[i + 1] == b'G' {
                return true;
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    false
}

/// `\A` and `\G` that can't apply here become a character no line has, as `vscode-textmate`.
fn with_anchors(source: &str, allow_a: bool, allow_g: bool) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('A') if !allow_a => out.push('\u{FFFF}'),
            Some('G') if !allow_g => out.push('\u{FFFF}'),
            Some(n) => {
                out.push('\\');
                out.push(n);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn has_backrefs(source: &str) -> bool {
    let bytes = source.as_bytes();
    (0..bytes.len().saturating_sub(1)).any(|i| bytes[i] == b'\\' && bytes[i + 1].is_ascii_digit())
}

fn escape_regex(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if "-\\{}*+?|^$.,[]()#".contains(c) || c.is_whitespace() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `\1` in an end pattern is what the begin pattern's group 1 matched.
fn resolve_backrefs(source: &str, line: &str, captures: &[(usize, usize)]) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(&(_, next)) = chars.peek() else {
            out.push('\\');
            break;
        };
        if next.is_ascii_digit() {
            let mut n = 0usize;
            while let Some(&(_, d)) = chars.peek() {
                let Some(v) = d.to_digit(10) else { break };
                n = n * 10 + v as usize;
                chars.next();
            }
            let text = captures.get(n).filter(|c| c.1 > c.0).map_or("", |c| &line[c.0..c.1]);
            out.push_str(&escape_regex(text));
        } else {
            chars.next();
            out.push('\\');
            out.push(next);
        }
    }
    out
}

/// `$1` and `${1:/downcase}` in a scope name are what the group matched.
fn resolve_name(name: &str, line: &str, captures: &[(usize, usize)]) -> String {
    if !name.contains('$') {
        return name.to_string();
    }
    let group = |n: usize| {
        let text = captures.get(n).filter(|c| c.1 > c.0).map_or("", |c| &line[c.0..c.1]);
        text.trim_start_matches('.').to_string()
    };
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let digits = rest[1..].bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            out.push_str(&group(rest[1..=digits].parse().unwrap_or(usize::MAX)));
            rest = &rest[1 + digits..];
        } else if let Some(inner) = rest.strip_prefix("${")
            && let Some(end) = inner.find('}')
            && let Some((n, how)) = inner[..end].split_once(":/")
            && let Ok(n) = n.parse::<usize>()
        {
            let text = group(n);
            out.push_str(&match how {
                "downcase" => text.to_lowercase(),
                "upcase" => text.to_uppercase(),
                _ => text,
            });
            rest = &inner[end + 1..];
        } else {
            out.push('$');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

#[derive(Clone)]
struct Frame {
    rule: RuleId,
    enter_pos: i64,
    anchor_pos: i64,
    captured_eol: bool,
    end: Option<RegexId>,
    name_scopes: ScopeId,
    content_scopes: ScopeId,
}

struct Scope {
    parent: Option<ScopeId>,
    name: Rc<str>,
    style: (u32, u8),
}

#[derive(Default)]
struct Producer {
    last_end: usize,
    tokens: Vec<(usize, ScopeId)>,
}

impl Producer {
    fn produce(&mut self, scopes: ScopeId, end: usize) {
        if self.last_end >= end {
            return;
        }
        self.tokens.push((end, scopes));
        self.last_end = end;
    }
}

const END_RULE: usize = usize::MAX;

/// One grammar and those it includes, compiled for its root.
struct Engine {
    theme: &'static Theme,
    rules: Vec<Rule>,
    patterns: Vec<Pattern>,
    pattern_ids: HashMap<String, RegexId>,
    /// A rule's description → its id, by address (grammars are never freed).
    compiled: HashMap<usize, RuleId>,
    grammars: HashMap<&'static str, &'static Value>,
    base: &'static Value,
    roots: HashMap<usize, RuleId>,
    scanners: HashMap<(RuleId, Option<RegexId>), Scanner>,
    /// What's injected into this grammar, sorted by priority; and which apply under a scope stack.
    injections: Vec<Injection>,
    active_injections: HashMap<ScopeId, Rc<Vec<usize>>>,
    scopes: Vec<Scope>,
    scope_ids: HashMap<(Option<ScopeId>, Rc<str>), ScopeId>,
    root_scope: ScopeId,
    stamp: u64,
    deadline: Instant,
}

/// The repositories a rule's `#name` includes look through, innermost first.
#[derive(Clone)]
struct RepoChain(Rc<Vec<&'static Value>>);

impl RepoChain {
    fn with(&self, repo: &'static Value) -> Self {
        let mut v = (*self.0).clone();
        v.insert(0, repo);
        Self(Rc::new(v))
    }

    fn find(&self, name: &str) -> Option<&'static Value> {
        self.0.iter().find_map(|r| r.get(name))
    }
}

fn load_grammar(scope: &str) -> Option<&'static Value> {
    static LOADED: OnceLock<Mutex<HashMap<&'static str, &'static Value>>> = OnceLock::new();
    let lang = LANGS.iter().find(|l| l.scope == scope)?;
    let mut loaded = LOADED.get_or_init(Default::default).lock().ok()?;
    if let Some(g) = loaded.get(lang.scope) {
        return Some(g);
    }
    let mut json = String::new();
    flate2::read::GzDecoder::new(lang.gz).read_to_string(&mut json).ok()?;
    let value: &'static Value = Box::leak(Box::new(serde_json::from_str(&json).ok()?));
    loaded.insert(lang.scope, value);
    Some(value)
}

impl Engine {
    fn new(lang: &'static Lang, theme: &'static Theme) -> Option<Self> {
        let base = load_grammar(lang.scope)?;
        let mut engine = Self {
            theme,
            rules: Vec::new(),
            patterns: Vec::new(),
            pattern_ids: HashMap::new(),
            compiled: HashMap::new(),
            grammars: HashMap::from([(lang.scope, base)]),
            base,
            roots: HashMap::new(),
            scanners: HashMap::new(),
            injections: Vec::new(),
            active_injections: HashMap::new(),
            scopes: Vec::new(),
            scope_ids: HashMap::new(),
            root_scope: 0,
            stamp: 0,
            deadline: Instant::now(),
        };
        let root = engine.root_of(base);
        engine.root_scope = root;
        engine.collect_injections(lang);
        engine.rules.shrink_to_fit();
        Some(engine)
    }

    /// The root grammar's own `injections`, then the loaded grammars that inject into its scope.
    fn collect_injections(&mut self, lang: &'static Lang) {
        let base = self.base;
        let chain = RepoChain(Rc::new(vec![&base["repository"]]));
        for selector in base["injectionOrder"].as_array().map_or(&[][..], Vec::as_slice).iter().filter_map(Value::as_str) {
            let rule = self.compile_rule(&base["injections"][selector], base, &chain, false);
            self.add_injection(selector, rule);
        }
        // Shiki's `getInjections`: its scope's prefixes, shortest first, each by what's loaded in order.
        let parts: Vec<&str> = lang.scope.split('.').collect();
        for i in 1..=parts.len() {
            let target = parts[..i].join(".");
            for dep in lang.deps {
                let Some(other) = LANGS.iter().find(|l| l.scope == *dep).filter(|l| l.inject_to.contains(&target.as_str())) else { continue };
                let Some(grammar) = load_grammar(other.scope) else { continue };
                let Some(selector) = grammar["injectionSelector"].as_str() else { continue };
                let rule = self.root_of(grammar);
                self.add_injection(selector, rule);
            }
        }
        self.injections.sort_by_key(|i| i.priority);
    }

    fn add_injection(&mut self, selector: &str, rule: RuleId) {
        for (matcher, priority) in parse_selector(selector) {
            self.injections.push(Injection { matcher, rule, priority });
        }
    }

    /// The scope stack's names, outermost first.
    fn scope_names(&self, mut at: ScopeId) -> Vec<Rc<str>> {
        let mut out = vec![self.scopes[at].name.clone()];
        while let Some(p) = self.scopes[at].parent {
            out.push(self.scopes[p].name.clone());
            at = p;
        }
        out.reverse();
        out
    }

    /// `matchInjections`: the earliest match of what applies here, a sooner injection on a tie.
    /// Whether it's a left one (`L:`), which wins a tie with the grammar's own match.
    fn match_injections(&mut self, scopes: ScopeId, line: &str, from: usize, first_line: bool, at_anchor: bool) -> Option<(RuleId, Groups, bool)> {
        let active = match self.active_injections.get(&scopes) {
            Some(a) => a.clone(),
            None => {
                let names = self.scope_names(scopes);
                let list: Rc<Vec<usize>> = Rc::new(self.injections.iter().enumerate().filter(|(_, i)| i.matcher.matches(&names)).map(|(n, _)| n).collect());
                self.active_injections.insert(scopes, list.clone());
                list
            }
        };
        let mut best: Option<(RuleId, Groups, i8)> = None;
        for n in active.iter() {
            let (rule, priority) = (self.injections[*n].rule, self.injections[*n].priority);
            let scanner = self.scanner(rule, None);
            let Some((matched, found)) = self.scan(&scanner, line, from, first_line, at_anchor) else { continue };
            let start = found[0].0;
            if best.as_ref().is_some_and(|b| start >= b.1[0].0) {
                continue;
            }
            best = Some((matched, found, priority));
            if start == from {
                break;
            }
        }
        best.map(|(rule, found, priority)| (rule, found, priority == -1))
    }

    /// `matchRuleOrInjections`: the grammar's next match, unless an injection's is sooner, or as soon
    /// and a left one.
    fn next_match(&mut self, top: &Frame, line: &str, pos: usize, first_line: bool, at_anchor: bool) -> Option<(RuleId, Groups)> {
        let scanner = self.scanner(top.rule, top.end.filter(|_| matches!(self.rules[top.rule].kind, Kind::BeginEnd { .. })));
        let own = self.scan(&scanner, line, pos, first_line, at_anchor);
        if self.injections.is_empty() {
            return own;
        }
        let Some((rule, found, left)) = self.match_injections(top.content_scopes, line, pos, first_line, at_anchor) else { return own };
        match own {
            None => Some((rule, found)),
            Some((_, ref o)) if found[0].0 < o[0].0 || (left && found[0].0 == o[0].0) => Some((rule, found)),
            own => own,
        }
    }

    fn pattern(&mut self, source: &str) -> RegexId {
        if let Some(id) = self.pattern_ids.get(source) {
            return *id;
        }
        let id = self.patterns.len();
        self.patterns.push(Pattern { source: source.to_string(), anchored: has_anchor(source), compiled: Default::default(), cache: Default::default() });
        self.pattern_ids.insert(source.to_string(), id);
        id
    }

    fn root_of(&mut self, grammar: &'static Value) -> RuleId {
        let key = std::ptr::from_ref(grammar) as usize;
        if let Some(id) = self.roots.get(&key) {
            return *id;
        }
        let id = self.rules.len();
        self.rules.push(Rule { name: grammar["scopeName"].as_str().map(str::to_string), content_name: None, kind: Kind::Pending, missing: false });
        self.roots.insert(key, id);
        let chain = RepoChain(Rc::new(vec![&grammar["repository"]]));
        let (patterns, missing) = self.compile_patterns(&grammar["patterns"], grammar, &chain);
        self.rules[id].kind = Kind::Container { patterns };
        self.rules[id].missing = missing;
        id
    }

    fn captures(&mut self, desc: &'static Value, grammar: &'static Value, chain: &RepoChain) -> Captures {
        let Some(map) = desc.as_object() else { return Vec::new() };
        let len = map.keys().filter_map(|k| k.parse::<usize>().ok()).max().map_or(0, |m| m + 1);
        let mut out: Captures = (0..len).map(|_| None).collect();
        for (k, v) in map {
            let Ok(i) = k.parse::<usize>() else { continue };
            let retokenize = v.get("patterns").is_some().then(|| self.compile_rule(v, grammar, chain, true));
            out[i] = Some(Capture { name: v["name"].as_str().map(str::to_string), content_name: v["contentName"].as_str().map(str::to_string), retokenize });
        }
        out
    }

    /// The patterns' rules, and whether some aren't there: `_compilePatterns`, which also leaves out
    /// a rule that had patterns and has none left.
    fn compile_patterns(&mut self, list: &'static Value, grammar: &'static Value, chain: &RepoChain) -> (Vec<RuleId>, bool) {
        self.compile_list(list.as_array().map_or(&[][..], Vec::as_slice), grammar, chain)
    }

    fn compile_list(&mut self, list: &'static [Value], grammar: &'static Value, chain: &RepoChain) -> (Vec<RuleId>, bool) {
        let mut out = Vec::new();
        for p in list {
            let id = match p["include"].as_str() {
                Some(include) => self.resolve_include(include, grammar, chain),
                None => Some(self.compile_rule(p, grammar, chain, false)),
            };
            let Some(id) = id else { continue };
            let empty = match &self.rules[id].kind {
                Kind::Container { patterns } | Kind::BeginEnd { patterns, .. } | Kind::BeginWhile { patterns, .. } => patterns.is_empty(),
                Kind::Match { .. } | Kind::Pending => false,
            };
            if !(empty && self.rules[id].missing) {
                out.push(id);
            }
        }
        let missing = out.len() != list.len();
        (out, missing)
    }

    fn resolve_include(&mut self, include: &str, grammar: &'static Value, chain: &RepoChain) -> Option<RuleId> {
        if include == "$base" {
            return Some(self.root_of(self.base));
        }
        if include == "$self" {
            return Some(self.root_of(grammar));
        }
        if let Some(name) = include.strip_prefix('#') {
            let desc = chain.find(name)?;
            return Some(self.compile_rule(desc, grammar, chain, false));
        }
        let (scope, name) = match include.split_once('#') {
            Some((s, n)) => (s, Some(n)),
            None => (include, None),
        };
        let other = load_grammar(scope)?;
        self.grammars.insert(other["scopeName"].as_str().unwrap_or(""), other);
        match name {
            None => Some(self.root_of(other)),
            Some(n) => {
                let desc = other["repository"].get(n)?;
                let chain = RepoChain(Rc::new(vec![&other["repository"]]));
                Some(self.compile_rule(desc, other, &chain, false))
            }
        }
    }

    /// `as_container`: a capture's `patterns`: its other keys belong to the capture.
    fn compile_rule(&mut self, desc: &'static Value, grammar: &'static Value, chain: &RepoChain, as_container: bool) -> RuleId {
        let key = std::ptr::from_ref(desc) as usize * 2 + usize::from(as_container);
        if let Some(id) = self.compiled.get(&key) {
            return *id;
        }
        let id = self.rules.len();
        let text = |k: &str| desc[k].as_str().map(str::to_string);
        let (name, content_name) = if as_container { (None, None) } else { (text("name"), text("contentName")) };
        self.rules.push(Rule { name, content_name, kind: Kind::Pending, missing: false });
        self.compiled.insert(key, id);
        let chain = match desc.get("repository") {
            Some(repo) => chain.with(repo),
            None => chain.clone(),
        };
        let mut missing = false;
        let kind = if as_container {
            let (patterns, gone) = self.compile_patterns(&desc["patterns"], grammar, &chain);
            missing = gone;
            Kind::Container { patterns }
        } else if let Some(m) = desc["match"].as_str() {
            Kind::Match { re: self.pattern(m), captures: self.captures(&desc["captures"], grammar, &chain) }
        } else if let Some(begin) = desc["begin"].as_str() {
            let begin_id = self.pattern(begin);
            let shared = self.captures(&desc["captures"], grammar, &chain);
            let begin_captures = if desc.get("beginCaptures").is_some() { self.captures(&desc["beginCaptures"], grammar, &chain) } else { self.captures(&desc["captures"], grammar, &chain) };
            let (patterns, gone) = self.compile_patterns(&desc["patterns"], grammar, &chain);
            missing = gone;
            if let Some(until) = desc["while"].as_str() {
                let while_captures = if desc.get("whileCaptures").is_some() { self.captures(&desc["whileCaptures"], grammar, &chain) } else { shared };
                Kind::BeginWhile { begin: begin_id, until: until.to_string(), begin_captures, while_captures, patterns }
            } else {
                let end_captures = if desc.get("endCaptures").is_some() { self.captures(&desc["endCaptures"], grammar, &chain) } else { shared };
                Kind::BeginEnd { begin: begin_id, end: desc["end"].as_str().unwrap_or("\u{FFFF}").to_string(), begin_captures, end_captures, end_last: desc["applyEndPatternLast"].as_bool() == Some(true) || desc["applyEndPatternLast"].as_i64() == Some(1), patterns }
            }
        } else if desc["include"].is_string() && desc.get("patterns").is_none() {
            let (patterns, gone) = self.compile_list(std::slice::from_ref(desc), grammar, &chain);
            missing = gone;
            Kind::Container { patterns }
        } else {
            let (patterns, gone) = self.compile_patterns(&desc["patterns"], grammar, &chain);
            missing = gone;
            Kind::Container { patterns }
        };
        self.rules[id].kind = kind;
        self.rules[id].missing = missing;
        id
    }

    fn collect(&self, rule: RuleId, out: &mut Vec<(RegexId, RuleId)>, seen: &mut Vec<RuleId>) {
        match &self.rules[rule].kind {
            Kind::Match { re, .. } => out.push((*re, rule)),
            Kind::BeginEnd { begin, .. } | Kind::BeginWhile { begin, .. } => out.push((*begin, rule)),
            Kind::Container { patterns } => {
                if seen.contains(&rule) {
                    return;
                }
                seen.push(rule);
                for p in patterns {
                    self.collect(*p, out, seen);
                }
            }
            Kind::Pending => {}
        }
    }

    /// What a frame searches for: its rule's patterns, with the end pattern first (or last).
    fn scanner(&mut self, rule: RuleId, end: Option<RegexId>) -> Scanner {
        if let Some(s) = self.scanners.get(&(rule, end)) {
            return s.clone();
        }
        let mut list = Vec::new();
        let mut seen = vec![rule];
        let (patterns, end_last) = match &self.rules[rule].kind {
            Kind::BeginEnd { patterns, end_last, .. } => (patterns.clone(), *end_last),
            Kind::BeginWhile { patterns, .. } | Kind::Container { patterns } => (patterns.clone(), false),
            Kind::Match { .. } | Kind::Pending => (Vec::new(), false),
        };
        for p in patterns {
            self.collect(p, &mut list, &mut seen);
        }
        if let Some(e) = end {
            if end_last {
                list.push((e, END_RULE));
            } else {
                list.insert(0, (e, END_RULE));
            }
        }
        let list = Rc::new(list);
        self.scanners.insert((rule, end), list.clone());
        list
    }

    fn regex(&mut self, id: RegexId, variant: usize) -> Option<Rc<Regex>> {
        let p = &mut self.patterns[id];
        if p.compiled[variant].is_none() {
            let source = if p.anchored { with_anchors(&p.source, variant & 2 != 0, variant & 1 != 0) } else { p.source.clone() };
            p.compiled[variant] = Some(Regex::with_options(&source, RegexOptions::REGEX_OPTION_CAPTURE_GROUP, Syntax::default()).ok().map(Rc::new));
        }
        p.compiled[variant].clone().flatten()
    }

    fn search(&mut self, id: RegexId, line: &str, from: usize, first_line: bool, at_anchor: bool) -> Option<Groups> {
        let anchored = self.patterns[id].anchored;
        let variant = if anchored { usize::from(first_line) * 2 + usize::from(at_anchor) } else { 0 };
        if let Some(c) = &self.patterns[id].cache[variant]
            && c.stamp == self.stamp
            && c.from <= from
            && !anchored
        {
            match &c.found {
                None => return None,
                Some(f) if f[0].0 >= from => return Some(f.clone()),
                Some(_) => {}
            }
        }
        let regex = self.regex(id, variant)?;
        let mut region = Region::new();
        let found = regex.search_with_options(line, from, line.len(), SearchOptions::SEARCH_OPTION_NONE, Some(&mut region)).map(|_| {
            Rc::new((0..region.len()).map(|i| region.pos(i).unwrap_or((usize::MAX, usize::MAX))).map(|(s, e)| if s == usize::MAX { (0, 0) } else { (s, e) }).collect::<Vec<_>>())
        });
        self.patterns[id].cache[variant] = Some(CacheEntry { stamp: self.stamp, from, found: found.clone() });
        found
    }

    fn scan(&mut self, list: &[(RegexId, RuleId)], line: &str, from: usize, first_line: bool, at_anchor: bool) -> Option<(RuleId, Groups)> {
        let mut best: Option<(RuleId, Groups)> = None;
        for (re, rule) in list {
            let Some(found) = self.search(*re, line, from, first_line, at_anchor) else { continue };
            let start = found[0].0;
            if best.as_ref().is_none_or(|(_, b)| start < b[0].0) {
                let at_start = start == from;
                best = Some((*rule, found));
                if at_start {
                    break;
                }
            }
        }
        best
    }

    fn push_scope(&mut self, parent: ScopeId, names: Option<&str>) -> ScopeId {
        let Some(names) = names else { return parent };
        let mut at = parent;
        for name in names.split_ascii_whitespace() {
            at = self.push_one(Some(at), name);
        }
        at
    }

    fn push_one(&mut self, parent: Option<ScopeId>, name: &str) -> ScopeId {
        let key: Rc<str> = name.into();
        if let Some(id) = self.scope_ids.get(&(parent, key.clone())) {
            return *id;
        }
        let id = self.scopes.len();
        let inherited = parent.map_or(self.theme.default, |p| self.scopes[p].style);
        self.scopes.push(Scope { parent, name: key.clone(), style: (0, 0) });
        let style = self.theme.style_of(&self.scopes, id, inherited);
        self.scopes[id].style = style;
        self.scope_ids.insert((parent, key), id);
        id
    }

    fn rule_name(&self, rule: RuleId, line: &str, captures: &[(usize, usize)]) -> Option<String> {
        self.rules[rule].name.as_deref().map(|n| resolve_name(n, line, captures))
    }

    fn handle_captures(&mut self, list: &'static [Option<Capture>], stack: &[Frame], line: &str, first_line: bool, indices: &[(usize, usize)], out: &mut Producer) {
        if list.is_empty() {
            return;
        }
        let Some(top) = stack.last().cloned() else { return };
        let max_end = indices[0].1;
        let mut local: Vec<(ScopeId, usize)> = Vec::new();
        for (i, capture) in list.iter().enumerate().take(indices.len()) {
            let Some(capture) = capture else { continue };
            let (start, end) = indices[i];
            if end <= start {
                continue;
            }
            if start > max_end {
                break;
            }
            while let Some(&(scopes, last_end)) = local.last() {
                if last_end > start {
                    break;
                }
                out.produce(scopes, last_end);
                local.pop();
            }
            match local.last() {
                Some(&(scopes, _)) => out.produce(scopes, start),
                None => out.produce(top.content_scopes, start),
            }
            if let Some(retokenize) = capture.retokenize {
                let name = capture.name.as_deref().map(|n| resolve_name(n, line, indices));
                let content = capture.content_name.as_deref().map(|n| resolve_name(n, line, indices));
                let name_scopes = self.push_scope(top.content_scopes, name.as_deref());
                let content_scopes = self.push_scope(name_scopes, content.as_deref());
                let mut inner = stack.to_vec();
                inner.push(Frame { rule: retokenize, enter_pos: start as i64, anchor_pos: -1, captured_eol: false, end: None, name_scopes, content_scopes });
                self.stamp += 1;
                self.tokenize_from(&line[..end], first_line && start == 0, start, &mut inner, out, false);
                self.stamp += 1;
                continue;
            }
            if let Some(name) = capture.name.as_deref() {
                let base = local.last().map_or(top.content_scopes, |l| l.0);
                let resolved = resolve_name(name, line, indices);
                let scopes = self.push_scope(base, Some(&resolved));
                local.push((scopes, end));
            }
        }
        while let Some((scopes, last_end)) = local.pop() {
            out.produce(scopes, last_end);
        }
    }

    /// The `while` patterns of the frames on the stack, outermost first: the first that doesn't
    /// match ends its rule here.
    fn check_while(&mut self, line: &str, first_line: &mut bool, pos: &mut usize, anchor: &mut i64, stack: &mut Vec<Frame>, out: &mut Producer) {
        for i in 0..stack.len() {
            let frame = stack[i].clone();
            let Kind::BeginWhile { .. } = self.rules[frame.rule].kind else { continue };
            let Some(until) = frame.end else { continue };
            match self.search(until, line, *pos, *first_line, *pos as i64 == *anchor) {
                Some(found) => {
                    let (start, end) = found[0];
                    out.produce(frame.content_scopes, start);
                    let captures: &'static [Option<Capture>] = match &self.rules[frame.rule].kind {
                        // Rules live as long as the engine, which a static holds.
                        Kind::BeginWhile { while_captures, .. } => unsafe { &*std::ptr::from_ref(while_captures.as_slice()) },
                        _ => &[],
                    };
                    self.handle_captures(captures, &stack[..=i], line, *first_line, &found, out);
                    out.produce(frame.content_scopes, end);
                    *anchor = end as i64;
                    if end > *pos {
                        *pos = end;
                        *first_line = false;
                    }
                }
                None => {
                    stack.truncate(i);
                    return;
                }
            }
        }
    }

    fn tokenize_from(&mut self, line: &str, mut first_line: bool, mut pos: usize, stack: &mut Vec<Frame>, out: &mut Producer, check_while: bool) {
        let len = line.len();
        let mut anchor = stack.last().map_or(-1, |f| if f.captured_eol { 0 } else { -1 });
        if check_while {
            self.check_while(line, &mut first_line, &mut pos, &mut anchor, stack, out);
        } else {
            anchor = stack.last().map_or(-1, |f| f.anchor_pos);
        }
        loop {
            if Instant::now() > self.deadline {
                return;
            }
            let Some(top) = stack.last().cloned() else { return };
            let Some((matched, found)) = self.next_match(&top, line, pos, first_line, pos as i64 == anchor) else {
                out.produce(top.content_scopes, len);
                return;
            };
            let (start, end) = found[0];
            let advanced = end > pos;
            if matched == END_RULE {
                out.produce(top.content_scopes, start);
                let captures: &'static [Option<Capture>] = match &self.rules[top.rule].kind {
                    // Rules live as long as the engine, which a static holds.
                    Kind::BeginEnd { end_captures, .. } => unsafe { &*std::ptr::from_ref(end_captures.as_slice()) },
                    _ => &[],
                };
                if let Some(f) = stack.last_mut() {
                    f.content_scopes = f.name_scopes;
                }
                self.handle_captures(captures, stack, line, first_line, &found, out);
                out.produce(top.name_scopes, end);
                stack.pop();
                anchor = top.anchor_pos;
                if !advanced && top.enter_pos == pos as i64 {
                    // Pushed and popped without moving: the grammar would loop.
                    stack.push(top.clone());
                    out.produce(top.content_scopes, len);
                    return;
                }
            } else {
                out.produce(top.content_scopes, start);
                let before = stack.len();
                let name = self.rule_name(matched, line, &found);
                let name_scopes = self.push_scope(top.content_scopes, name.as_deref());
                stack.push(Frame { rule: matched, enter_pos: pos as i64, anchor_pos: anchor, captured_eol: end == len, end: None, name_scopes, content_scopes: name_scopes });
                match &self.rules[matched].kind {
                    Kind::BeginEnd { end: end_source, begin_captures, .. } => {
                        // Rules live as long as the engine, which a static holds.
                        let captures: &'static [Option<Capture>] = unsafe { &*std::ptr::from_ref(begin_captures.as_slice()) };
                        let end_source = end_source.clone();
                        self.handle_captures(captures, stack, line, first_line, &found, out);
                        out.produce(name_scopes, end);
                        anchor = end as i64;
                        let content = self.rules[matched].content_name.as_deref().map(|n| resolve_name(n, line, &found));
                        let content_scopes = self.push_scope(name_scopes, content.as_deref());
                        let end_id = if has_backrefs(&end_source) { self.pattern(&resolve_backrefs(&end_source, line, &found)) } else { self.pattern(&end_source) };
                        if let Some(f) = stack.last_mut() {
                            f.content_scopes = content_scopes;
                            f.end = Some(end_id);
                        }
                        if !advanced && same_rule_entered_here(&stack[..before], stack.last()) {
                            stack.pop();
                            out.produce(top.content_scopes, len);
                            return;
                        }
                    }
                    Kind::BeginWhile { until, begin_captures, .. } => {
                        // Rules live as long as the engine, which a static holds.
                        let captures: &'static [Option<Capture>] = unsafe { &*std::ptr::from_ref(begin_captures.as_slice()) };
                        let until = until.clone();
                        self.handle_captures(captures, stack, line, first_line, &found, out);
                        out.produce(name_scopes, end);
                        anchor = end as i64;
                        let content = self.rules[matched].content_name.as_deref().map(|n| resolve_name(n, line, &found));
                        let content_scopes = self.push_scope(name_scopes, content.as_deref());
                        let while_id = if has_backrefs(&until) { self.pattern(&resolve_backrefs(&until, line, &found)) } else { self.pattern(&until) };
                        if let Some(f) = stack.last_mut() {
                            f.content_scopes = content_scopes;
                            f.end = Some(while_id);
                        }
                        if !advanced && same_rule_entered_here(&stack[..before], stack.last()) {
                            stack.pop();
                            out.produce(top.content_scopes, len);
                            return;
                        }
                    }
                    Kind::Match { captures, .. } => {
                        // Rules live as long as the engine, which a static holds.
                        let captures: &'static [Option<Capture>] = unsafe { &*std::ptr::from_ref(captures.as_slice()) };
                        self.handle_captures(captures, stack, line, first_line, &found, out);
                        out.produce(name_scopes, end);
                        stack.pop();
                        if !advanced {
                            if stack.len() > 1 {
                                stack.pop();
                            }
                            let at = stack.last().map_or(top.content_scopes, |f| f.content_scopes);
                            out.produce(at, len);
                            return;
                        }
                    }
                    Kind::Container { .. } | Kind::Pending => {
                        stack.pop();
                    }
                }
            }
            if end > pos {
                pos = end;
                first_line = false;
            }
        }
    }

    fn start(&mut self) -> LineState {
        let root_rule = self.root_scope;
        let name = self.rules[root_rule].name.clone();
        let root_scopes = self.push_one(None, name.as_deref().unwrap_or("source"));
        LineState { stack: vec![Frame { rule: root_rule, enter_pos: -1, anchor_pos: -1, captured_eol: false, end: None, name_scopes: root_scopes, content_scopes: root_scopes }], first_line: true, root_rule, root_scopes }
    }

    fn tokenize_line(&mut self, raw: &str, state: &mut LineState) -> Vec<Span> {
        let default = self.theme.default;
        let content = raw.strip_suffix('\r').unwrap_or(raw);
        if content.len() > MAX_LINE_BYTES {
            state.first_line = false;
            return vec![Span { start: 0, end: content.len() as u32, color: default.0, font: default.1 }];
        }
        self.deadline = Instant::now() + LINE_BUDGET;
        self.stamp += 1;
        let line = format!("{content}\n");
        for f in &mut state.stack {
            f.enter_pos = -1;
            f.anchor_pos = -1;
        }
        let mut producer = Producer::default();
        self.tokenize_from(&line, state.first_line, 0, &mut state.stack, &mut producer, true);
        state.first_line = false;
        if state.stack.is_empty() {
            state.stack.push(Frame { rule: state.root_rule, enter_pos: -1, anchor_pos: -1, captured_eol: false, end: None, name_scopes: state.root_scopes, content_scopes: state.root_scopes });
        }
        let mut spans: Vec<Span> = Vec::new();
        let mut start = 0;
        for (end, scopes) in &producer.tokens {
            let end = (*end).min(content.len());
            if end <= start {
                continue;
            }
            let (color, font) = self.scopes[*scopes].style;
            match spans.last_mut() {
                Some(l) if l.color == color && l.font == font => l.end = end as u32,
                _ => spans.push(Span { start: start as u32, end: end as u32, color, font }),
            }
            start = end;
        }
        if start < content.len() {
            // A line the budget cut short.
            spans.push(Span { start: start as u32, end: content.len() as u32, color: default.0, font: default.1 });
        }
        spans
    }
}

/// Where a tokenizer is between lines. Its ids belong to its language's engine.
struct LineState {
    stack: Vec<Frame>,
    first_line: bool,
    root_rule: RuleId,
    root_scopes: ScopeId,
}

fn same_rule_entered_here(below: &[Frame], pushed: Option<&Frame>) -> bool {
    let Some(pushed) = pushed else { return false };
    below.iter().rev().take_while(|f| f.enter_pos == pushed.enter_pos).any(|f| f.rule == pushed.rule)
}

/// A TextMate theme's token colours, resolved as `vscode-textmate` does: the longest dotted prefix
/// of a scope, the rule with the most specific parents, and a later rule over an earlier one.
pub struct Theme {
    rules: Vec<ThemeRule>,
    pub default: (u32, u8),
    pub editor: EditorColors,
}

/// What an editor draws with besides its tokens: the theme's own, or Monaco's defaults (`assets/generate.mjs`).
pub struct EditorColors {
    pub foreground: crate::theme::Col,
    pub line_number: crate::theme::Col,
    pub line_number_active: crate::theme::Col,
    pub indent_guide: crate::theme::Col,
    pub whitespace: crate::theme::Col,
    pub cursor: crate::theme::Col,
    pub find_match: crate::theme::Col,
    pub find_highlight: crate::theme::Col,
    pub fold_icon: crate::theme::Col,
    pub line_highlight: Option<crate::theme::Col>,
    pub line_highlight_border: Option<crate::theme::Col>,
}

struct ThemeRule {
    scope: String,
    parents: Vec<String>,
    color: Option<u32>,
    font: Option<u8>,
}

fn scope_matches(scope: &str, pattern: &str) -> bool {
    scope == pattern || (scope.len() > pattern.len() && scope.starts_with(pattern) && scope.as_bytes()[pattern.len()] == b'.')
}

fn parse_color(c: &str) -> Option<u32> {
    let hex = c.strip_prefix('#')?;
    u32::from_str_radix(hex.get(..6)?, 16).ok()
}

impl Theme {
    pub fn parse(json: &str) -> Option<Self> {
        let v: Value = serde_json::from_str(json).ok()?;
        let e = &v["editor"];
        let col = |k: &str| e[k].as_str().and_then(crate::theme::Col::parse);
        let editor = EditorColors { foreground: col("foreground")?, line_number: col("lineNumber")?, line_number_active: col("lineNumberActive")?, indent_guide: col("indentGuide")?, whitespace: col("whitespace")?, cursor: col("cursor")?, find_match: col("findMatch")?, find_highlight: col("findMatchHighlight")?, fold_icon: col("foldIcon")?, line_highlight: col("lineHighlight"), line_highlight_border: col("lineHighlightBorder") };
        let mut rules = Vec::new();
        let mut default = (parse_color(v["foreground"].as_str()?)?, 0u8);
        for entry in v["tokenColors"].as_array()? {
            let color = entry["settings"]["foreground"].as_str().and_then(parse_color);
            let font = entry["settings"]["fontStyle"].as_str().map(|s| {
                s.split_ascii_whitespace().fold(0u8, |f, w| match w {
                    "italic" => f | FONT_ITALIC,
                    "bold" => f | FONT_BOLD,
                    "underline" => f | FONT_UNDERLINE,
                    _ => f,
                })
            });
            let scopes: Vec<String> = match &entry["scope"] {
                Value::String(s) => s.split(',').map(|x| x.trim().to_string()).collect(),
                Value::Array(a) => a.iter().filter_map(|x| x.as_str()).map(|x| x.trim().to_string()).collect(),
                _ => {
                    default = (color.unwrap_or(default.0), font.unwrap_or(default.1));
                    continue;
                }
            };
            for scope in scopes {
                let mut parts: Vec<String> = scope.split_ascii_whitespace().map(str::to_string).collect();
                let Some(leaf) = parts.pop() else { continue };
                parts.reverse();
                rules.push(ThemeRule { scope: leaf, parents: parts, color, font });
            }
        }
        Some(Self { rules, default, editor })
    }

    fn parents_match(scopes: &[Scope], from: Option<ScopeId>, parents: &[String]) -> bool {
        let mut at = from;
        for p in parents {
            loop {
                let Some(i) = at else { return false };
                at = scopes[i].parent;
                if scope_matches(&scopes[i].name, p) {
                    break;
                }
            }
        }
        true
    }

    /// `scope`'s style: `inherited`, with what the best rules set over it, attribute by attribute.
    fn style_of(&self, scopes: &[Scope], scope: ScopeId, inherited: (u32, u8)) -> (u32, u8) {
        let name = &scopes[scope].name;
        let parent = scopes[scope].parent;
        // (dotted depth, parents' depth total, parents' count, order): the highest wins.
        let mut color: Option<(usize, usize, u32)> = None;
        let mut font: Option<(usize, usize, u8)> = None;
        for (order, r) in self.rules.iter().enumerate() {
            let _ = order;
            if !scope_matches(name, &r.scope) || !Self::parents_match(scopes, parent, &r.parents) {
                continue;
            }
            let rank = (r.scope.len(), r.parents.len());
            if let Some(c) = r.color
                && color.is_none_or(|(a, b, _)| rank >= (a, b))
            {
                color = Some((rank.0, rank.1, c));
            }
            if let Some(f) = r.font
                && font.is_none_or(|(a, b, _)| rank >= (a, b))
            {
                font = Some((rank.0, rank.1, f));
            }
        }
        (color.map_or(inherited.0, |c| c.2), font.map_or(inherited.1, |f| f.2))
    }
}

/// The editor themes the app themes name, as `assets/generate.mjs` writes them.
const EDITOR_THEMES: [(&str, &str); 10] = [
    ("dark-plus", include_str!("../../assets/editor-themes/dark-plus.json")),
    ("light-plus", include_str!("../../assets/editor-themes/light-plus.json")),
    ("monokai", include_str!("../../assets/editor-themes/monokai.json")),
    ("darcula", include_str!("../../assets/editor-themes/darcula.json")),
    ("dracula", include_str!("../../assets/editor-themes/dracula.json")),
    ("one-dark-pro", include_str!("../../assets/editor-themes/one-dark-pro.json")),
    ("solarized-dark", include_str!("../../assets/editor-themes/solarized-dark.json")),
    ("solarized-light", include_str!("../../assets/editor-themes/solarized-light.json")),
    ("github-dark-default", include_str!("../../assets/editor-themes/github-dark-default.json")),
    ("nord", include_str!("../../assets/editor-themes/nord.json")),
];

/// An editor theme, parsed once; an unknown name is Dark+.
pub fn theme_named(name: &str) -> &'static Theme {
    static THEMES: [OnceLock<Theme>; 10] = [const { OnceLock::new() }; 10];
    let i = EDITOR_THEMES.iter().position(|(n, _)| *n == name).unwrap_or(0);
    THEMES[i].get_or_init(|| Theme::parse(EDITOR_THEMES[i].1).expect("the embedded theme parses"))
}

/// The editor theme new tokenizers colour with: a worker reads it, so it isn't on the UI thread's palette.
static TOKEN_THEME: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn set_token_theme(name: &str) {
    let i = EDITOR_THEMES.iter().position(|(n, _)| *n == name).unwrap_or(0);
    TOKEN_THEME.store(i, std::sync::atomic::Ordering::Relaxed);
}

/// The theme's default foreground: the colour of the text no token covers.
pub fn token_theme() -> &'static Theme {
    theme_named(EDITOR_THEMES[TOKEN_THEME.load(std::sync::atomic::Ordering::Relaxed)].0)
}

/// Compiled engines, one per language, kept for the session: only ever used under the lock.
struct Engines(HashMap<(&'static str, usize), Engine>);
// SAFETY: an `Engine`'s `Rc`s never leave it, and it's only reached under `ENGINES`'s lock.
unsafe impl Send for Engines {}

static ENGINES: OnceLock<Mutex<Engines>> = OnceLock::new();

/// Colours a text's lines a few at a time, so a long file shows from its top without waiting for
/// its end, and two files can take turns with one language's engine. In the theme shown when it's made.
pub struct Tokenizer {
    lang: &'static Lang,
    theme: &'static Theme,
    state: Option<LineState>,
}

impl Tokenizer {
    /// `None`: there's no grammar `id` (`Lang::id`).
    pub fn new(id: &str) -> Option<Self> {
        Some(Self { lang: LANGS.iter().find(|l| l.id == id)?, theme: token_theme(), state: None })
    }

    /// The next `lines`, in order. `None`: its grammar can't be read. Blocking: call it off the UI
    /// thread.
    pub fn lines(&mut self, lines: &[&str]) -> Option<Vec<Vec<Span>>> {
        let mut engines = ENGINES.get_or_init(|| Mutex::new(Engines(HashMap::new()))).lock().ok()?;
        let key = (self.lang.id, self.theme as *const Theme as usize);
        let engine = match engines.0.entry(key) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => e.insert(Engine::new(self.lang, self.theme)?),
        };
        let state = self.state.get_or_insert_with(|| engine.start());
        Some(lines.iter().map(|l| engine.tokenize_line(l, state)).collect())
    }
}

/// `text`'s lines coloured as language `id`, all at once.
#[cfg(test)]
pub fn highlight(id: &str, text: &str) -> Option<Vec<Vec<Span>>> {
    highlight_in("dark-plus", id, text)
}

/// In the editor theme `name`.
#[cfg(test)]
pub fn highlight_in(name: &str, id: &str, text: &str) -> Option<Vec<Vec<Span>>> {
    let mut t = Tokenizer::new(id)?;
    t.theme = theme_named(name);
    t.lines(&text.split('\n').collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_every_sample_as_shiki_does() {
        let golden: Value = serde_json::from_str(include_str!("testdata/shiki-golden.json")).unwrap();
        let mut wrong = Vec::new();
        for sample in golden.as_array().unwrap() {
            let (lang, code) = (sample["lang"].as_str().unwrap(), sample["code"].as_str().unwrap());
            let lines = highlight_in(sample["theme"].as_str().unwrap(), lang, code).unwrap_or_else(|| panic!("no grammar for {lang}"));
            for (n, want) in sample["lines"].as_array().unwrap().iter().enumerate() {
                let mut at = 0u32;
                for token in want.as_array().unwrap() {
                    let text = token[0].as_str().unwrap();
                    let color = parse_color(token[1].as_str().unwrap()).unwrap();
                    for (i, ch) in text.char_indices() {
                        // Whitespace has no visible colour.
                        if !ch.is_whitespace() {
                            let byte = at + i as u32;
                            let got = lines[n].iter().find(|s| s.start <= byte && byte < s.end).map(|s| (s.color, s.font));
                            if got.map(|g| g.0) != Some(color) || got.map(|g| g.1) != Some(token[2].as_u64().unwrap() as u8) {
                                wrong.push(format!("{lang}:{n} {text:?}: want {color:06X}/{}, got {got:?}", token[2]));
                                break;
                            }
                        }
                    }
                    at += text.len() as u32;
                }
            }
        }
        assert!(wrong.is_empty(), "{} tokens differ from Shiki's:\n{}", wrong.len(), wrong.join("\n"));
    }

    /// By hand: `GITVOLT_GOLDEN=<file>` with the samples' format, for real files.
    #[test]
    #[ignore]
    fn compare_with_a_golden_file() {
        let golden: Value = serde_json::from_str(&std::fs::read_to_string(std::env::var("GITVOLT_GOLDEN").unwrap()).unwrap()).unwrap();
        for sample in golden.as_array().unwrap() {
            let (lang, code) = (sample["lang"].as_str().unwrap(), sample["code"].as_str().unwrap());
            let cold = Instant::now();
            let lines = highlight(lang, code).unwrap();
            let cold = cold.elapsed();
            let warm = Instant::now();
            let _ = highlight(lang, code).unwrap();
            let (mut total, mut wrong, mut shown) = (0usize, 0usize, 0);
            for (n, want) in sample["lines"].as_array().unwrap().iter().enumerate() {
                let mut at = 0u32;
                for token in want.as_array().unwrap() {
                    let text = token[0].as_str().unwrap();
                    let color = parse_color(token[1].as_str().unwrap()).unwrap();
                    for (i, ch) in text.char_indices() {
                        if !ch.is_whitespace() {
                            total += 1;
                            let byte = at + i as u32;
                            let got = lines[n].iter().find(|s| s.start <= byte && byte < s.end).map(|s| (s.color, s.font));
                            if got != Some((color, token[2].as_u64().unwrap() as u8)) {
                                wrong += 1;
                                if shown < 4 && i == 0 {
                                    shown += 1;
                                    println!("  {lang}:{} {text:?} want {color:06X} got {got:?}", n + 1);
                                }
                            }
                        }
                    }
                    at += text.len() as u32;
                }
            }
            println!("{} {} lines: {wrong} of {total} characters differ; cold {cold:?}, warm {:?}, Shiki {:.0} ms", sample["file"], lines.len(), warm.elapsed(), sample["shikiMs"].as_f64().unwrap_or(0.));
        }
    }

    #[test]
    fn a_selector_parses_as_vscode_textmates() {
        assert_eq!(selector_tokens("L:(a.b - c), R:d-e|f"), ["L:", "(", "a.b", "-", "c", ")", ",", "R:", "d-e", "|", "f"]);
        // What's no token is skipped: `*` splits a name.
        assert_eq!(selector_tokens("meta.tag.*.*.html"), ["meta.tag.", ".", ".html"]);
        let names = |s: &[&str]| s.iter().map(|x| Rc::<str>::from(*x)).collect::<Vec<_>>();
        let one = |s: &str| parse_selector(s).remove(0);
        let (m, p) = one("L:text.html - comment");
        assert_eq!(p, -1);
        assert!(m.matches(&names(&["text.html.basic", "meta.tag"])));
        assert!(!m.matches(&names(&["text.html.basic", "comment.block"])), "excluded");
        assert!(!m.matches(&names(&["text.htmlx"])), "not at a dot");
        let (m, p) = one("R:a b");
        assert_eq!(p, 1);
        assert!(m.matches(&names(&["a", "x", "b"])), "in order, with others between");
        assert!(!m.matches(&names(&["b", "a"])));
        let (m, _) = one("(a | b) c");
        assert!(m.matches(&names(&["b", "c"])));
        assert!(!m.matches(&names(&["c"])));
        let (m, _) = one("a -(b, c)");
        assert!(m.matches(&names(&["a"])));
        assert!(!m.matches(&names(&["a", "c"])));
        let all = parse_selector("L:a, b, R:c");
        assert_eq!(all.iter().map(|x| x.1).collect::<Vec<_>>(), [-1, 0, 1]);
    }

    #[test]
    fn a_grammars_injections_are_sorted_by_priority() {
        // Each of these has its own `injections`, or injects by `injectTo` when it's loaded with it.
        let engine = |id: &str| Engine::new(LANGS.iter().find(|l| l.id == id).unwrap(), theme_named("dark-plus")).unwrap();
        for id in ["html", "svelte", "astro", "twig", "angular-html", "ts-tags"] {
            let e = engine(id);
            assert!(!e.injections.is_empty(), "{id}");
            assert!(e.injections.windows(2).all(|w| w[0].priority <= w[1].priority), "{id}");
        }
        assert!(engine("rust").injections.is_empty());
        assert!(engine("markdown").injections.is_empty(), "nothing injects into it unless it's loaded with it");
    }

    #[test]
    fn colouring_in_pieces_gives_what_colouring_at_once_does() {
        let code = "/* a\nb */ fn x() {\n  let s = \"t\";\n}\n";
        let whole = highlight("rust", code).unwrap();
        let lines: Vec<&str> = code.split('\n').collect();
        let mut t = Tokenizer::new("rust").unwrap();
        let mut pieces = t.lines(&lines[..2]).unwrap();
        // Another file takes a turn with the engine in between.
        highlight("rust", "\"unclosed").unwrap();
        pieces.extend(t.lines(&lines[2..]).unwrap());
        assert_eq!(pieces, whole);
        assert!(Tokenizer::new("no-such-language").is_none());
    }

    #[test]
    fn every_embedded_grammar_loads_and_tokenizes() {
        for lang in LANGS {
            let lines = highlight(lang.id, "a = 1 # b\n\"c\"\n").unwrap_or_else(|| panic!("{} didn't load", lang.id));
            assert!(lines.len() >= 2, "{}", lang.id);
        }
    }

    #[test]
    fn a_backreference_and_a_scope_template_take_the_groups_text() {
        assert_eq!(resolve_backrefs(r"\1\.x", "a+b", &[(0, 3), (0, 3)]), r"a\+b\.x");
        assert_eq!(resolve_name("e.$1.${1:/upcase}", "abc", &[(0, 3), (0, 3)]), "e.abc.ABC");
        assert_eq!(with_anchors(r"\Ga\\G\A", false, true), "\\Ga\\\\G\u{FFFF}");
    }
}
