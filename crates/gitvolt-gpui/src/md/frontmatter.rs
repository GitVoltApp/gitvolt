//! A file's front matter (`plugins/frontmatter.ts`): a leading `---` YAML or `+++` TOML block as a table, a row per
//! key, its value as text. Plain data only: the core schema, no custom tags, a capped alias expansion. A block that
//! doesn't parse, or isn't a map, is a code block of its source; an empty one is nothing.

use super::ast::{Kind, Node};
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust2::scanner::{Marker, TScalarStyle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Yaml,
    Toml,
}

impl Lang {
    fn name(self) -> &'static str {
        match self {
            Lang::Yaml => "yaml",
            Lang::Toml => "toml",
        }
    }
}

pub struct Block {
    pub lang: Lang,
    pub value: String,
}

/// The block at `text`'s very start, what follows it, and where that begins.
pub fn split(text: &str) -> (Option<Block>, &str, usize) {
    let fence = match text.get(..3) {
        Some("---") => "---",
        Some("+++") => "+++",
        _ => return (None, text, 0),
    };
    let lang = if fence == "---" { Lang::Yaml } else { Lang::Toml };
    let line_end = |from: usize| text[from..].find('\n').map_or(text.len(), |k| from + k);
    let first = line_end(0);
    if !text[3..first].trim_matches([' ', '\t', '\r']).is_empty() || first == text.len() {
        return (None, text, 0);
    }
    let mut at = first + 1;
    while at <= text.len() {
        let end = line_end(at);
        let line = text[at..end].trim_end_matches([' ', '\t', '\r']);
        if line == fence {
            let value = text[first + 1..at].trim_end_matches(['\n', '\r']).to_string();
            let rest = (end + 1).min(text.len());
            return (Some(Block { lang, value }), &text[rest..], rest);
        }
        if end >= text.len() {
            break;
        }
        at = end + 1;
    }
    (None, text, 0)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Text(String),
    List(Vec<Value>),
    Map(Vec<(Value, Value)>),
}

/// YAML's default cap on aliases.
const MAX_ALIASES: usize = 100;
/// What the aliases of a block may add up to, in values and bytes of text. Counting aliases alone isn't enough: each
/// takes a copy of all it names, so ten levels of a dozen aliases are 12^10 values for only 120 of them.
const MAX_EXPANDED: usize = 1 << 20;
/// A block longer than this is no front matter: it shows as code, as one that doesn't parse.
const MAX_BLOCK_BYTES: usize = 256 * 1024;
/// How deep a block may nest. The YAML parser and what walks its answer recurse.
const MAX_NESTING: usize = 256;

#[derive(Default)]
struct Loader {
    stack: Vec<(Value, Option<Value>, usize)>,
    anchors: Vec<(usize, Value, usize)>,
    docs: Vec<Value>,
    aliases: usize,
    expanded: usize,
    failed: bool,
}

/// Its size in values and text: what a copy of it costs.
fn weight(v: &Value) -> usize {
    match v {
        Value::Null => 1,
        Value::Text(s) => 1 + s.len(),
        Value::List(l) => 1 + l.iter().map(weight).sum::<usize>(),
        Value::Map(m) => 1 + m.iter().map(|(k, v)| weight(k) + weight(v)).sum::<usize>(),
    }
}

/// How deep a line can nest: its indent and the `- ` and `? ` on it. An over-estimate, for what no real block reaches.
fn too_deep(src: &str) -> bool {
    src.lines().any(|l| {
        let indent = l.len() - l.trim_start_matches(' ').len();
        indent + l.matches("- ").count() + l.matches("? ").count() > MAX_NESTING
    })
}

/// The core schema's reading of a plain scalar, as JavaScript's `String()` shows it.
fn core_scalar(s: &str) -> Value {
    match s {
        "" | "~" | "null" | "Null" | "NULL" => return Value::Null,
        "true" | "True" | "TRUE" => return Value::Text("true".into()),
        "false" | "False" | "FALSE" => return Value::Text("false".into()),
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => return Value::Text("Infinity".into()),
        "-.inf" | "-.Inf" | "-.INF" => return Value::Text("-Infinity".into()),
        ".nan" | ".NaN" | ".NAN" => return Value::Text("NaN".into()),
        _ => {}
    }
    let digits = s.strip_prefix(['-', '+']).unwrap_or(s);
    if let Some(oct) = s.strip_prefix("0o")
        && let Ok(n) = u64::from_str_radix(oct, 8)
    {
        return Value::Text(n.to_string());
    }
    if let Some(hex) = s.strip_prefix("0x")
        && let Ok(n) = u64::from_str_radix(hex, 16)
    {
        return Value::Text(n.to_string());
    }
    let starts_num = digits.starts_with(|c: char| c.is_ascii_digit()) || (digits.starts_with('.') && digits[1..].starts_with(|c: char| c.is_ascii_digit()));
    if starts_num
        && digits.bytes().all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'-' | b'+'))
        && let Ok(n) = s.parse::<f64>()
    {
        return Value::Text(if n.fract() == 0. && n.abs() < 1e21 { format!("{n:.0}") } else { n.to_string() });
    }
    Value::Text(s.to_string())
}

impl Loader {
    fn add(&mut self, v: Value, anchor: usize) {
        if anchor > 0 {
            let w = weight(&v);
            self.expanded += w;
            if self.expanded > MAX_EXPANDED {
                self.failed = true;
                return;
            }
            self.anchors.push((anchor, v.clone(), w));
        }
        let Some((parent, key, _)) = self.stack.last_mut() else {
            self.docs.push(v);
            return;
        };
        match parent {
            Value::List(items) => items.push(v),
            Value::Map(entries) => match key.take() {
                None => *key = Some(v),
                Some(k) => {
                    // `uniqueKeys`: a repeated key doesn't parse.
                    if entries.iter().any(|(x, _)| *x == k) {
                        self.failed = true;
                    }
                    entries.push((k, v));
                }
            },
            _ => {}
        }
    }
}

impl MarkedEventReceiver for Loader {
    fn on_event(&mut self, ev: Event, _: Marker) {
        // It's already refused: nothing more is kept.
        if self.failed {
            return;
        }
        match ev {
            Event::Scalar(s, style, anchor, tag) => {
                let v = if style == TScalarStyle::Plain && tag.is_none() { core_scalar(&s) } else { Value::Text(s) };
                self.add(v, anchor);
            }
            Event::SequenceStart(anchor, _) => self.stack.push((Value::List(Vec::new()), None, anchor)),
            Event::MappingStart(anchor, _) => self.stack.push((Value::Map(Vec::new()), None, anchor)),
            Event::SequenceEnd | Event::MappingEnd => {
                if let Some((v, _, anchor)) = self.stack.pop() {
                    self.add(v, anchor);
                }
            }
            Event::Alias(id) => {
                self.aliases += 1;
                if self.aliases > MAX_ALIASES {
                    self.failed = true;
                    return;
                }
                let Some((_, v, w)) = self.anchors.iter().rev().find(|(a, ..)| *a == id) else {
                    self.failed = true;
                    return;
                };
                self.expanded += *w;
                if self.expanded > MAX_EXPANDED {
                    self.failed = true;
                    return;
                }
                let v = v.clone();
                self.add(v, 0);
            }
            _ => {}
        }
    }
}

/// `None`: it doesn't parse, or it isn't a map.
fn yaml_entries(src: &str) -> Option<Vec<(String, Value)>> {
    if src.len() > MAX_BLOCK_BYTES || too_deep(src) {
        return None;
    }
    let mut loader = Loader::default();
    let mut parser = Parser::new_from_str(src);
    parser.load(&mut loader, true).ok()?;
    if loader.failed || loader.docs.len() > 1 {
        return None;
    }
    match loader.docs.pop() {
        None | Some(Value::Null) => Some(Vec::new()),
        Some(Value::Map(m)) => Some(m.into_iter().map(|(k, v)| (key_text(&k), v)).collect()),
        _ => None,
    }
}

fn key_text(k: &Value) -> String {
    match k {
        Value::Text(s) => s.clone(),
        other => flow(other),
    }
}

fn toml_value(v: toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::Text(s),
        toml::Value::Integer(n) => Value::Text(n.to_string()),
        toml::Value::Float(n) => Value::Text(n.to_string()),
        toml::Value::Boolean(b) => Value::Text(b.to_string()),
        toml::Value::Datetime(d) => Value::Text(d.to_string()),
        toml::Value::Array(a) => Value::List(a.into_iter().map(toml_value).collect()),
        toml::Value::Table(t) => Value::Map(t.into_iter().map(|(k, v)| (Value::Text(k), toml_value(v))).collect()),
    }
}

fn toml_entries(src: &str) -> Option<Vec<(String, Value)>> {
    if src.len() > MAX_BLOCK_BYTES {
        return None;
    }
    let t: toml::Table = src.parse().ok()?;
    Some(t.into_iter().map(|(k, v)| (k, toml_value(v))).collect())
}

/// Quoted when a plain scalar couldn't say it.
fn flow_text(s: &str) -> String {
    let plain = !s.is_empty() && !s.contains([':', '#', ',', '[', ']', '{', '}', '"', '\'', '\n']) && !s.starts_with(['-', '?', '!', '&', '*', '|', '>', '%', '@', '`', ' ']) && !s.ends_with(' ') && !matches!(core_scalar(s), Value::Text(t) if t != s) && core_scalar(s) != Value::Null;
    if plain { s.to_string() } else { format!("{s:?}") }
}

/// A list or a map as its YAML, flow style, as `stringify` with `collectionStyle: 'flow'` writes it.
pub fn flow(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Text(s) => flow_text(s),
        Value::List(l) if l.is_empty() => "[]".into(),
        Value::List(l) => format!("[ {} ]", l.iter().map(flow).collect::<Vec<_>>().join(", ")),
        Value::Map(m) if m.is_empty() => "{}".into(),
        Value::Map(m) => format!("{{ {} }}", m.iter().map(|(k, v)| format!("{}: {}", flow(k), flow(v))).collect::<Vec<_>>().join(", ")),
    }
}

fn cell_of(v: &Value) -> Vec<Node> {
    match v {
        Value::Null => Vec::new(),
        Value::Text(s) => {
            let t = s.trim_end_matches('\n');
            if t.is_empty() { Vec::new() } else { vec![Node::text(t)] }
        }
        other => vec![Node::new(Kind::InlineCode(flow(other)))],
    }
}

/// What the block shows as.
pub fn nodes(block: &Block) -> Vec<Node> {
    let entries = match block.lang {
        Lang::Yaml => yaml_entries(&block.value),
        Lang::Toml => toml_entries(&block.value),
    };
    match entries {
        None => vec![Node::new(Kind::Code { lang: Some(block.lang.name().into()), meta: None, value: block.value.clone() })],
        Some(list) if list.is_empty() => Vec::new(),
        Some(list) => {
            let rows = list
                .into_iter()
                .map(|(k, v)| {
                    let key = if k.is_empty() { Vec::new() } else { vec![Node::text(k)] };
                    Node::with(Kind::TableRow, vec![Node::with(Kind::TableCell, key), Node::with(Kind::TableCell, cell_of(&v))])
                })
                .collect();
            let mut table = Node::with(Kind::Table { align: vec![super::ast::Align::None, super::ast::Align::None] }, rows);
            table.data_mut().frontmatter = Some(block.lang);
            vec![table]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn table_of(text: &str) -> Vec<(String, String)> {
        let (block, ..) = split(text);
        let nodes = nodes(&block.unwrap());
        nodes[0].children.iter().map(|r| (r.children[0].text_of(), r.children[1].text_of())).collect()
    }

    #[test]
    fn a_yaml_block_is_a_row_per_key_with_its_values_as_text() {
        let rows = table_of("---\ntitle: Harbour guide\nversion: 3\ndraft: false\nnone: ~\ntags: [boats, tides]\nowner:\n  name: Ada\n---\nrest");
        assert_eq!(rows[0], ("title".into(), "Harbour guide".into()));
        assert_eq!(rows[1], ("version".into(), "3".into()));
        assert_eq!(rows[2], ("draft".into(), "false".into()));
        assert_eq!(rows[3], ("none".into(), "".into()));
        assert_eq!(rows[4], ("tags".into(), "[ boats, tides ]".into()));
        assert_eq!(rows[5], ("owner".into(), "{ name: Ada }".into()));
    }

    #[test]
    fn what_follows_the_block_starts_after_it() {
        let (b, rest, at) = split("---\na: 1\n---\n# T\n");
        assert_eq!(b.unwrap().value, "a: 1");
        assert_eq!((rest, at), ("# T\n", 13));
        assert!(split("--- \nno close").0.is_none());
        assert!(split("text\n---\na: 1\n---\n").0.is_none(), "only at the very start");
    }

    #[test]
    fn toml_keeps_its_order() {
        let rows = table_of("+++\nz = 1\na = \"x\"\nlist = [1, 2]\n+++\n");
        assert_eq!(rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), ["z", "a", "list"]);
        assert_eq!(rows[2].1, "[ 1, 2 ]");
    }

    #[test]
    fn a_malformed_block_or_one_that_is_not_a_map_shows_as_code() {
        for text in ["---\n: : :\n  - [\n---\n", "---\n- a\n- b\n---\n", "---\nd: 1\nd: 2\n---\n", "+++\nnot toml\n+++\n"] {
            let (b, ..) = split(text);
            assert!(matches!(nodes(&b.unwrap())[0].kind, Kind::Code { .. }), "{text}");
        }
        assert!(nodes(&split("---\n---\n").0.unwrap()).is_empty(), "empty: nothing");
    }

    fn is_code(text: &str) -> bool {
        matches!(nodes(&split(text).0.unwrap())[0].kind, Kind::Code { .. })
    }

    /// Its work on a small stack, as an off-thread read has.
    fn on_small_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new().stack_size(1 << 21).spawn(f).unwrap().join().unwrap();
    }

    #[test]
    fn an_alias_bomb_is_refused_before_it_is_copied() {
        // Each level names the last a dozen times: 12^11 values, in under a hundred and fifty aliases.
        let mut s = String::from("---\na0: &a0 [x, x]\n");
        for i in 1..12 {
            let list = vec![format!("*a{}", i - 1); 12].join(", ");
            s.push_str(&format!("a{i}: &a{i} [{list}]\n"));
        }
        s.push_str("---\n");
        assert!(is_code(&s));
        // Fewer aliases than the cap, each a large copy.
        let mut s = format!("---\na: &a \"{}\"\n", "x".repeat(200_000));
        for i in 0..50 {
            s.push_str(&format!("b{i}: *a\n"));
        }
        s.push_str("---\n");
        assert!(is_code(&s));
        // And what's refused keeps nothing it copied.
        let mut l = Loader::default();
        Parser::new_from_str("a: &a [1, 2]\nb: *a\nc: *a\n").load(&mut l, true).unwrap();
        assert!(!l.failed);
        assert_eq!(l.aliases, 2);
    }

    #[test]
    fn a_few_aliases_still_read() {
        let rows = table_of("---\nbase: &b [one, two]\ncopy: *b\n---\n");
        assert_eq!(rows[1], ("copy".into(), "[ one, two ]".into()));
    }

    #[test]
    fn a_block_nested_too_deep_is_code_and_not_a_stack_overflow() {
        on_small_stack(|| {
            for s in [
                format!("---\n{}a\n---\n", "- ".repeat(100_000)),
                format!("---\n{}a\n---\n", "- &x ".repeat(40_000)),
                format!("---\n{}a\n---\n", "? ".repeat(100_000)),
                format!("---\n{}\n---\n", "[".repeat(100_000)),
                format!("---\n{}\n---\n", "{a: ".repeat(30_000)),
                format!("+++\na = {}\n+++\n", "[".repeat(100_000)),
                format!("+++\na = {}\n+++\n", "{b = ".repeat(30_000)),
            ] {
                assert!(is_code(&s), "{}", &s[..20]);
            }
            // As deep as it lets through, one level an indent: it still reads and goes.
            let mut deep = String::from("---\n");
            for i in 0..200 {
                deep.push_str(&format!("{}k:\n", " ".repeat(i)));
            }
            deep.push_str("---\n");
            assert!(!is_code(&deep));
        });
    }

    #[test]
    fn a_huge_block_is_not_read() {
        let s = format!("---\n{}---\n", "k: v\n".repeat(100_000));
        assert!(is_code(&s));
    }
}
