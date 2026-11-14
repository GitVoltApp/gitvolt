//! The tree as the screen gets it: `render.tsx`'s `toSafeHast`. Markdown's nodes and raw HTML become one tree of
//! elements, as rehype-raw makes them, and only what `sanitize.ts`'s schema lets through stays. A native tree runs
//! nothing and loads nothing by itself: what's left to guard is where its links and images go (`links.rs`). What the
//! app itself puts on a node (`Special`) isn't an attribute, so no document can write one.

use super::ast::{Align, Kind, Mark, Node, RefNode, Slugger};
use super::frontmatter;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Dom {
    Text(String),
    El(El),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct El {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub kids: Vec<Dom>,
    pub special: Option<Box<Special>>,
}

impl El {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn has(&self, name: &str) -> bool {
        self.attrs.iter().any(|(k, _)| k == name)
    }

    fn special(tag: &str, s: Special) -> Self {
        Self { tag: tag.to_string(), special: Some(Box::new(s)), ..Default::default() }
    }
}

/// What the app puts on the tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Special {
    Reference(RefNode),
    /// A fenced block, as `MdCode`, `MdMermaid` or `MdSuggestion` shows it. `marks`, `words`: a changed block's.
    Code { lang: Option<String>, value: String, marks: Option<String>, words: Option<String> },
    DiffBlock { mark: Mark, note: Option<String> },
    DiffPair,
    DiffIns,
    DiffDel,
    SplitRow { mark: Option<Mark>, joined: bool },
    SplitCell { old: bool },
    SplitEmpty,
    /// A list item's or a table row's mark.
    Marked(Mark),
    EmptyRow,
    Frontmatter(frontmatter::Lang),
    /// A task list's checkbox.
    Task(bool),
}

const VOID: [&str; 14] = ["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];
const CLOSES_P: [&str; 37] = ["address", "article", "aside", "blockquote", "center", "details", "dialog", "dir", "div", "dl", "fieldset", "figcaption", "figure", "footer", "header", "hgroup", "main", "menu", "nav", "ol", "p", "search", "section", "summary", "ul", "h1", "h2", "h3", "h4", "h5", "h6", "pre", "listing", "form", "table", "hr", "xmp"];
const RAW_TEXT: [&str; 8] = ["script", "style", "xmp", "iframe", "noembed", "noframes", "textarea", "title"];
/// `rehype-sanitize` drops these with what they hold.
const STRIP: [&str; 12] = ["script", "style", "iframe", "object", "embed", "svg", "math", "noscript", "template", "textarea", "select", "title"];
const ALLOWED: [&str; 54] = ["a", "b", "blockquote", "br", "code", "dd", "del", "details", "div", "dl", "dt", "em", "h1", "h2", "h3", "h4", "h5", "h6", "hr", "i", "img", "input", "ins", "kbd", "li", "ol", "p", "picture", "pre", "q", "rp", "rt", "ruby", "s", "samp", "section", "source", "span", "strike", "strong", "sub", "summary", "sup", "table", "tbody", "td", "tfoot", "th", "thead", "tr", "tt", "ul", "var", "caption"];
/// What the views read, of what the schema lets through.
const ATTRS: [&str; 19] = ["align", "alt", "aria-describedby", "colspan", "height", "href", "id", "name", "open", "rowspan", "src", "start", "title", "valign", "value", "width", "checked", "data-footnotes", "data-footnote-backref"];
const ENTITIES: &[(&str, &str)] = &[
    ("amp", "&"), ("lt", "<"), ("gt", ">"), ("quot", "\""), ("apos", "'"), ("nbsp", "\u{a0}"), ("iexcl", "¡"), ("cent", "¢"), ("pound", "£"), ("curren", "¤"), ("yen", "¥"), ("brvbar", "¦"), ("sect", "§"), ("uml", "¨"), ("copy", "©"), ("ordf", "ª"), ("laquo", "«"), ("not", "¬"), ("shy", "\u{ad}"), ("reg", "®"), ("macr", "¯"), ("deg", "°"), ("plusmn", "±"), ("sup2", "²"), ("sup3", "³"), ("acute", "´"), ("micro", "µ"), ("para", "¶"), ("middot", "·"), ("cedil", "¸"), ("sup1", "¹"), ("ordm", "º"), ("raquo", "»"), ("frac14", "¼"), ("frac12", "½"), ("frac34", "¾"), ("iquest", "¿"), ("times", "×"), ("divide", "÷"), ("ndash", "–"), ("mdash", "—"), ("lsquo", "‘"), ("rsquo", "’"), ("sbquo", "‚"), ("ldquo", "“"), ("rdquo", "”"), ("bdquo", "„"), ("dagger", "†"), ("Dagger", "‡"), ("bull", "•"), ("hellip", "…"), ("permil", "‰"), ("prime", "′"), ("Prime", "″"), ("lsaquo", "‹"), ("rsaquo", "›"), ("euro", "€"), ("trade", "™"), ("larr", "←"), ("uarr", "↑"), ("rarr", "→"), ("darr", "↓"), ("harr", "↔"), ("crarr", "↵"), ("check", "✓"), ("cross", "✗"), ("ensp", "\u{2002}"), ("emsp", "\u{2003}"), ("thinsp", "\u{2009}"), ("zwnj", "\u{200c}"), ("zwj", "\u{200d}"), ("minus", "−"), ("infin", "∞"), ("ne", "≠"), ("le", "≤"), ("ge", "≥"), ("asymp", "≈"), ("Agrave", "À"), ("Aacute", "Á"), ("Auml", "Ä"), ("Aring", "Å"), ("AElig", "Æ"), ("Ccedil", "Ç"), ("Eacute", "É"), ("Ntilde", "Ñ"), ("Ouml", "Ö"), ("Oslash", "Ø"), ("Uuml", "Ü"), ("szlig", "ß"), ("agrave", "à"), ("aacute", "á"), ("acirc", "â"), ("auml", "ä"), ("aring", "å"), ("aelig", "æ"), ("ccedil", "ç"), ("egrave", "è"), ("eacute", "é"), ("ecirc", "ê"), ("euml", "ë"), ("iacute", "í"), ("ntilde", "ñ"), ("oacute", "ó"), ("ocirc", "ô"), ("ouml", "ö"), ("oslash", "ø"), ("uacute", "ú"), ("uuml", "ü"),
];

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        // Near, or it's no entity: a text of `&`s and no `;` mustn't search to its end for each one.
        let end = after.as_bytes().iter().take(33).position(|c| *c == b';').filter(|e| after.as_bytes()[..*e].iter().all(|c| c.is_ascii_alphanumeric() || *c == b'#'));
        let decoded = end.and_then(|e| {
            let name = &after[..e];
            if let Some(num) = name.strip_prefix('#') {
                let code = match num.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => num.parse().ok(),
                }?;
                Some(char::from_u32(code).filter(|c| *c != '\0').unwrap_or('\u{fffd}').to_string())
            } else {
                ENTITIES.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string())
            }
        });
        match (decoded, end) {
            (Some(d), Some(e)) => {
                out.push_str(&d);
                rest = &after[e + 1..];
            }
            _ => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Past this many open elements, what opens is the last one's sibling, as Chromium's parser does past 512. What walks
/// the tree recurses, and a crafted file can open a million; what's real doesn't come near it.
pub const MAX_DEPTH: usize = 100;
/// A tag's attributes past this are dropped: a tag with a million of them mustn't cost the square of it.
const MAX_ATTRS: usize = 64;

/// `haystack`'s first `needle`, whose letters are lower case, in any case: without a copy, however often it's asked.
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    let (h, n) = (haystack.as_bytes(), needle.as_bytes());
    h.windows(n.len()).position(|w| w.eq_ignore_ascii_case(n))
}

/// The stack of open elements, as the HTML parser keeps it, with its simpler rules: no reconstruction of misnested
/// formatting (the adoption agency), and only text fostered out of a table.
pub struct Builder {
    stack: Vec<El>,
    /// How deep the element it fills is already.
    base: usize,
}

impl Default for Builder {
    fn default() -> Self {
        Self { stack: vec![El::default()], base: 0 }
    }
}

impl Builder {
    fn open_at(&self, tag: &str) -> Option<usize> {
        self.stack.iter().rposition(|e| e.tag == tag)
    }

    fn inside(&self, tag: &str) -> bool {
        self.open_at(tag).is_some()
    }

    fn pop(&mut self) {
        if self.stack.len() > 1 {
            let done = self.stack.pop().expect("an open element");
            self.stack.last_mut().expect("the root").kids.push(Dom::El(done));
        }
    }

    fn pop_to(&mut self, depth: usize) {
        while self.stack.len() > depth {
            self.pop();
        }
    }

    fn scoped_p(&self) -> Option<usize> {
        for (i, e) in self.stack.iter().enumerate().rev() {
            match e.tag.as_str() {
                "p" => return Some(i),
                "table" | "td" | "th" | "caption" | "button" | "object" | "template" => return None,
                _ => {}
            }
        }
        None
    }

    fn close_implied(&mut self, tag: &str) {
        if (CLOSES_P.contains(&tag) || matches!(tag, "li" | "dd" | "dt"))
            && let Some(at) = self.scoped_p()
        {
            self.pop_to(at);
        }
        let stops = |t: &str| matches!(t, "ul" | "ol" | "dl" | "blockquote" | "table" | "td" | "th" | "section" | "details" | "summary" | "figure");
        match tag {
            "li" | "dd" | "dt" => {
                let closes: &[&str] = if tag == "li" { &["li"] } else { &["dd", "dt"] };
                for i in (1..self.stack.len()).rev() {
                    let t = self.stack[i].tag.as_str();
                    if closes.contains(&t) {
                        self.pop_to(i);
                        break;
                    }
                    if stops(t) {
                        break;
                    }
                }
            }
            "td" | "th" | "tr" | "tbody" | "thead" | "tfoot" | "caption" => {
                let Some(table) = self.open_at("table") else { return };
                let closes: &[&str] = match tag {
                    "td" | "th" => &["td", "th"],
                    "tr" => &["td", "th", "tr"],
                    _ => &["td", "th", "tr", "tbody", "thead", "tfoot", "caption"],
                };
                while self.stack.len() > table + 1 {
                    let t = self.stack.last().map_or("", |e| e.tag.as_str());
                    if closes.contains(&t) || !matches!(t, "tr" | "tbody" | "thead" | "tfoot" | "table") {
                        self.pop();
                    } else {
                        break;
                    }
                }
            }
            "a" => {
                if let Some(at) = self.open_at("a") {
                    self.pop_to(at);
                }
            }
            "option" if self.stack.last().is_some_and(|e| e.tag == "option") => self.pop(),
            _ => {}
        }
    }

    /// `false`: a table's part with no table: the parser ignores the tag.
    pub fn open(&mut self, tag: &str, attrs: Vec<(String, String)>) -> bool {
        if matches!(tag, "td" | "th" | "tr" | "tbody" | "thead" | "tfoot" | "caption") && !self.inside("table") {
            return false;
        }
        self.close_implied(tag);
        if self.base + self.stack.len() >= MAX_DEPTH {
            self.pop();
        }
        self.stack.push(El { tag: tag.to_string(), attrs, ..Default::default() });
        true
    }

    pub fn close(&mut self, tag: &str) {
        if tag == "p" && self.scoped_p().is_none() {
            self.open("p", Vec::new());
        }
        if let Some(at) = self.open_at(tag).filter(|at| *at > 0) {
            self.pop_to(at);
        }
    }

    pub fn void(&mut self, tag: &str, attrs: Vec<(String, String)>) {
        self.close_implied(tag);
        self.stack.last_mut().expect("the root").kids.push(Dom::El(El { tag: tag.to_string(), attrs, ..Default::default() }));
    }

    pub fn element(&mut self, el: El) {
        self.stack.last_mut().expect("the root").kids.push(Dom::El(el));
    }

    pub fn text(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let top = self.stack.last().map_or("", |e| e.tag.as_str());
        let target = if matches!(top, "table" | "tbody" | "thead" | "tfoot" | "tr") {
            if s.trim_matches([' ', '\t', '\n', '\r', '\u{c}']).is_empty() {
                return;
            }
            // Fostered: it comes before the table.
            let table = self.open_at("table").unwrap_or(1);
            table.saturating_sub(1)
        } else {
            self.stack.len() - 1
        };
        let kids = &mut self.stack[target].kids;
        if let Some(Dom::Text(t)) = kids.last_mut() {
            t.push_str(s);
        } else {
            kids.push(Dom::Text(s.to_string()));
        }
    }

    /// Raw HTML, a fragment at a time: what it opens stays open for what follows, as with rehype-raw.
    pub fn raw(&mut self, html: &str) {
        let b = html.as_bytes();
        let mut i = 0;
        let mut text_from = 0;
        let flush = |this: &mut Self, from: usize, to: usize| {
            if to > from {
                this.text(&decode_entities(&html[from..to]));
            }
        };
        while let Some(k) = html[i..].find('<') {
            let at = i + k;
            let next = b.get(at + 1).copied();
            if html[at..].starts_with("<!--") {
                flush(self, text_from, at);
                i = html[at + 4..].find("-->").map_or(html.len(), |e| at + 4 + e + 3);
                text_from = i;
            } else if matches!(next, Some(b'!' | b'?')) {
                flush(self, text_from, at);
                i = html[at..].find('>').map_or(html.len(), |e| at + e + 1);
                text_from = i;
            } else if next == Some(b'/') && b.get(at + 2).is_some_and(u8::is_ascii_alphabetic) {
                flush(self, text_from, at);
                let end = html[at..].find('>').map_or(html.len(), |e| at + e);
                let name = html[at + 2..end].split(|c: char| c.is_ascii_whitespace() || c == '/').next().unwrap_or("").to_ascii_lowercase();
                self.close(&name);
                i = (end + 1).min(html.len());
                text_from = i;
            } else if next.is_some_and(|c| c.is_ascii_alphabetic()) {
                flush(self, text_from, at);
                let (tag, attrs, closed, end) = read_tag(html, at + 1);
                let tag = if tag == "image" { "img".to_string() } else { tag };
                i = end;
                text_from = end;
                if RAW_TEXT.contains(&tag.as_str()) {
                    let closing = format!("</{tag}");
                    let stop = find_ci(&html[end..], &closing).map(|e| end + e);
                    if self.open(&tag, attrs) {
                        self.text(&html[end..stop.unwrap_or(html.len())]);
                        // Not closed in this fragment: what follows goes in it, as with rehype-raw.
                        if stop.is_some() {
                            self.close(&tag);
                        }
                    }
                    i = stop.map_or(html.len(), |stop| html[stop..].find('>').map_or(html.len(), |e| stop + e + 1));
                    text_from = i;
                } else if VOID.contains(&tag.as_str()) || (closed && (self.inside("svg") || self.inside("math"))) {
                    self.void(&tag, attrs);
                } else {
                    self.open(&tag, attrs);
                }
            } else {
                i = at + 1;
            }
        }
        flush(self, text_from, html.len());
    }

    pub fn finish(mut self) -> Vec<Dom> {
        self.pop_to(1);
        self.stack.pop().map(|r| r.kids).unwrap_or_default()
    }
}

/// A start tag from just after its `<`: its name, attributes, whether it's self-closed, and where it ends.
fn read_tag(s: &str, from: usize) -> (String, Vec<(String, String)>, bool, usize) {
    let b = s.as_bytes();
    let mut i = from;
    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'/' && b[i] != b'>' {
        i += 1;
    }
    let tag = s[from..i].to_ascii_lowercase();
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut closed = false;
    loop {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            closed = b[i] == b'/';
            i += 1;
        }
        if i >= b.len() {
            return (tag, attrs, closed, i);
        }
        if b[i] == b'>' {
            return (tag, attrs, closed, i + 1);
        }
        closed = false;
        let name_at = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'/' | b'>' | b'=') {
            i += 1;
        }
        // A lone `=` at a name's start is part of the name.
        if i == name_at {
            i += 1;
        }
        let name = s[name_at..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if b.get(i) == Some(&b'=') {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            match b.get(i) {
                Some(&q @ (b'"' | b'\'')) => {
                    let end = s[i + 1..].find(q as char).map_or(s.len(), |e| i + 1 + e);
                    value = decode_entities(&s[i + 1..end]);
                    i = (end + 1).min(s.len());
                }
                _ => {
                    let start = i;
                    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                        i += 1;
                    }
                    value = decode_entities(&s[start..i]);
                }
            }
        }
        if attrs.len() < MAX_ATTRS && !attrs.iter().any(|(k, _)| *k == name) {
            attrs.push((name, value));
        }
    }
}

struct Notes<'a> {
    defs: HashMap<&'a str, &'a Node>,
    /// In the order they're first called: the identifier and how many calls.
    calls: Vec<(String, u32)>,
}

fn collect_notes<'a>(nodes: &'a [Node], defs: &mut HashMap<&'a str, &'a Node>) {
    for n in nodes {
        if let Kind::FootnoteDefinition { id } = &n.kind {
            defs.entry(id.as_str()).or_insert(n);
        }
        collect_notes(&n.children, defs);
    }
}

fn attr(k: &str, v: impl Into<String>) -> (String, String) {
    (k.to_string(), v.into())
}

fn align_attr(a: Align) -> Vec<(String, String)> {
    match a {
        Align::None => Vec::new(),
        Align::Left => vec![attr("align", "left")],
        Align::Center => vec![attr("align", "center")],
        Align::Right => vec![attr("align", "right")],
    }
}

/// A diff wrapper's content, its raw HTML parsed on its own (`isolated`): a stray closing tag in it can't close the
/// wrapper, and the wrapper closes whatever it leaves open.
fn isolated(base: usize, nodes: &[Node], notes: &mut Notes) -> Vec<Dom> {
    let mut b = Builder { base, ..Default::default() };
    walk(&mut b, nodes, notes, false);
    b.finish()
}

fn wrap(b: &mut Builder, tag: &str, s: Special, kids: Vec<Dom>) {
    let mut el = El::special(tag, s);
    el.kids = kids;
    if CLOSES_P.contains(&tag) {
        b.close_implied(tag);
    }
    b.element(el);
}

fn walk(b: &mut Builder, nodes: &[Node], notes: &mut Notes, in_raw_code: bool) {
    for n in nodes {
        node(b, n, notes, in_raw_code);
    }
}

fn node(b: &mut Builder, n: &Node, notes: &mut Notes, x: bool) {
    let data = n.data();
    match &n.kind {
        Kind::Root => walk(b, &n.children, notes, x),
        Kind::Paragraph if data.is_some_and(|d| d.tight) => walk(b, &n.children, notes, x),
        Kind::Paragraph => {
            b.open("p", Vec::new());
            walk(b, &n.children, notes, x);
            b.close("p");
        }
        Kind::Heading { depth } => {
            let tag = format!("h{depth}");
            let attrs = data.and_then(|d| d.id.clone()).map(|id| vec![attr("id", id)]).unwrap_or_default();
            b.open(&tag, attrs);
            walk(b, &n.children, notes, x);
            b.close(&tag);
        }
        Kind::ThematicBreak => b.void("hr", Vec::new()),
        Kind::Blockquote => {
            b.open("blockquote", Vec::new());
            walk(b, &n.children, notes, x);
            b.close("blockquote");
        }
        Kind::List { ordered, start } => {
            let tag = if *ordered { "ol" } else { "ul" };
            b.open(tag, if *ordered && *start != 1 { vec![attr("start", start.to_string())] } else { Vec::new() });
            walk(b, &n.children, notes, x);
            b.close(tag);
        }
        Kind::ListItem { checked } => {
            let mut attrs = Vec::new();
            if let Some(v) = data.and_then(|d| d.value) {
                attrs.push(attr("value", v.to_string()));
            }
            b.open("li", attrs);
            if let Some(mark) = data.and_then(|d| d.diff)
                && let Some(top) = b.stack.last_mut()
            {
                top.special = Some(Box::new(Special::Marked(mark)));
            }
            let task = checked.map(|c| El::special("input", Special::Task(c)));
            match (task, n.children.first()) {
                (Some(input), Some(Node { kind: Kind::Paragraph, children, data: d, .. })) if d.as_ref().is_some_and(|d| d.tight) => {
                    b.element(input);
                    if !children.is_empty() {
                        b.text(" ");
                    }
                    walk(b, children, notes, x);
                    walk(b, &n.children[1..], notes, x);
                }
                (Some(input), Some(Node { kind: Kind::Paragraph, children, .. })) => {
                    // A loose item: its first paragraph holds the box.
                    b.open("p", Vec::new());
                    b.element(input);
                    if !children.is_empty() {
                        b.text(" ");
                    }
                    walk(b, children, notes, x);
                    b.close("p");
                    walk(b, &n.children[1..], notes, x);
                }
                (Some(input), first) => {
                    b.element(input);
                    if first.is_some() {
                        b.text(" ");
                    }
                    walk(b, &n.children, notes, x);
                }
                (None, _) => walk(b, &n.children, notes, x),
            }
            b.close("li");
        }
        Kind::Code { lang, value, .. } => {
            let code = Special::Code { lang: lang.clone().filter(|l| safe_lang(l)), value: value.clone(), marks: data.and_then(|d| d.lines.clone()), words: data.and_then(|d| d.words.clone()) };
            b.close_implied("pre");
            b.element(El::special("pre", code));
        }
        Kind::Html(value) => b.raw(value),
        // Shown at the end, when something calls them.
        Kind::FootnoteDefinition { .. } => {}
        Kind::Table { align } => {
            b.open("table", Vec::new());
            if let Some(lang) = data.and_then(|d| d.frontmatter)
                && let Some(top) = b.stack.last_mut()
            {
                top.special = Some(Box::new(Special::Frontmatter(lang)));
            }
            let front = data.is_some_and(|d| d.frontmatter.is_some());
            let (head, body) = if front { n.children.split_at(0) } else { n.children.split_at(n.children.len().min(1)) };
            let row = |b: &mut Builder, r: &Node, cell: &str, first_th: bool, notes: &mut Notes| {
                b.open("tr", Vec::new());
                let d = r.data();
                if let Some(top) = b.stack.last_mut() {
                    if d.is_some_and(|d| d.empty) {
                        top.special = Some(Box::new(Special::EmptyRow));
                    } else if let Some(mark) = d.and_then(|d| d.diff) {
                        top.special = Some(Box::new(Special::Marked(mark)));
                    }
                }
                for (i, c) in r.children.iter().enumerate() {
                    let tag = if first_th && i == 0 { "th" } else { cell };
                    b.open(tag, align_attr(align.get(i).copied().unwrap_or(Align::None)));
                    walk(b, &c.children, notes, x);
                    b.close(tag);
                }
                b.close("tr");
            };
            if !head.is_empty() {
                b.open("thead", Vec::new());
                for r in head {
                    row(b, r, "th", false, notes);
                }
                b.close("thead");
            }
            if !body.is_empty() {
                b.open("tbody", Vec::new());
                for r in body {
                    row(b, r, "td", front, notes);
                }
                b.close("tbody");
            }
            b.close("table");
        }
        Kind::TableRow | Kind::TableCell => walk(b, &n.children, notes, x),
        Kind::Text(s) => b.text(s),
        Kind::Emphasis => inline(b, "em", n, notes, x),
        Kind::Strong => inline(b, "strong", n, notes, x),
        Kind::Delete => inline(b, "del", n, notes, x),
        Kind::InlineCode(s) => {
            b.open("code", Vec::new());
            b.text(s);
            b.close("code");
        }
        Kind::Break => {
            b.void("br", Vec::new());
            b.text("\n");
        }
        Kind::Link { url, title, .. } => {
            let mut attrs = vec![attr("href", url.clone())];
            if let Some(t) = title {
                attrs.push(attr("title", t.clone()));
            }
            b.open("a", attrs);
            walk(b, &n.children, notes, x);
            b.close("a");
        }
        Kind::Image { url, title, alt, .. } => {
            let mut attrs = vec![attr("src", url.clone()), attr("alt", alt.clone())];
            if let Some(t) = title {
                attrs.push(attr("title", t.clone()));
            }
            if let Some(w) = data.and_then(|d| d.width) {
                attrs.push(attr("width", w.to_string()));
            }
            if let Some(h) = data.and_then(|d| d.height) {
                attrs.push(attr("height", h.to_string()));
            }
            b.void("img", attrs);
        }
        Kind::FootnoteReference { id } => {
            if !notes.defs.contains_key(id.as_str()) {
                b.text(&format!("[^{id}]"));
                return;
            }
            let index = match notes.calls.iter().position(|(i, _)| i == id) {
                Some(i) => {
                    notes.calls[i].1 += 1;
                    i
                }
                None => {
                    notes.calls.push((id.clone(), 1));
                    notes.calls.len() - 1
                }
            };
            let call = notes.calls[index].1;
            let suffix = if call > 1 { format!("-{call}") } else { String::new() };
            b.open("sup", Vec::new());
            b.open("a", vec![attr("href", format!("#fn-{id}")), attr("id", format!("fnref-{id}{suffix}")), attr("aria-describedby", "footnote-label")]);
            b.text(&(index + 1).to_string());
            b.close("a");
            b.close("sup");
        }
        Kind::Reference(r) => {
            // `rehypeUnwrapNestedRefs`: inside a raw `a`, `code` or `pre`, it goes back to its text.
            if b.inside("a") || b.inside("code") || b.inside("pre") {
                b.text(&r.value);
            } else {
                b.element(El::special("span", Special::Reference(r.clone())));
            }
        }
        Kind::DiffBlock { mark, note } => {
            let kids = isolated(b.base + b.stack.len(), &n.children, notes);
            wrap(b, "div", Special::DiffBlock { mark: *mark, note: note.clone() }, kids);
        }
        Kind::DiffPair => {
            let kids = isolated(b.base + b.stack.len(), &n.children, notes);
            wrap(b, "div", Special::DiffPair, kids);
        }
        Kind::DiffIns => {
            let kids = isolated(b.base + b.stack.len(), &n.children, notes);
            wrap(b, "span", Special::DiffIns, kids);
        }
        Kind::DiffDel => {
            let kids = isolated(b.base + b.stack.len(), &n.children, notes);
            wrap(b, "span", Special::DiffDel, kids);
        }
        Kind::SplitRow { mark, joined } => {
            let mut kids = Vec::new();
            for c in &n.children {
                let Kind::SplitCell { old, empty } = c.kind else { continue };
                if empty {
                    kids.push(Dom::El(El::special("div", Special::SplitEmpty)));
                } else {
                    let mut el = El::special("div", Special::SplitCell { old });
                    el.kids = isolated(b.base + b.stack.len(), &c.children, notes);
                    kids.push(Dom::El(el));
                }
            }
            wrap(b, "div", Special::SplitRow { mark: *mark, joined: *joined }, kids);
        }
        Kind::SplitCell { .. } => walk(b, &n.children, notes, x),
    }
}

fn inline(b: &mut Builder, tag: &str, n: &Node, notes: &mut Notes, x: bool) {
    b.open(tag, Vec::new());
    walk(b, &n.children, notes, x);
    b.close(tag);
}

/// A `language-*` class the schema keeps.
fn safe_lang(l: &str) -> bool {
    let plain = !l.is_empty() && l.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'+' | b'#' | b'.' | b'-'));
    plain || suggestion_lang(l)
}

/// GitHub's `suggestion`, GitLab's `suggestion:-N+M`.
pub fn suggestion_lang(l: &str) -> bool {
    if l == "suggestion" {
        return true;
    }
    let Some(rest) = l.strip_prefix("suggestion:-") else { return false };
    let Some((a, b)) = rest.split_once('+') else { return false };
    !a.is_empty() && !b.is_empty() && a.bytes().all(|c| c.is_ascii_digit()) && b.bytes().all(|c| c.is_ascii_digit())
}

fn notes_section(b: &mut Builder, notes: &mut Notes) {
    if notes.calls.is_empty() {
        return;
    }
    b.open("section", vec![attr("data-footnotes", "")]);
    b.open("ol", Vec::new());
    let mut i = 0;
    // A note's own text can call another: it's added to the list as it's read.
    while i < notes.calls.len() {
        let id = notes.calls[i].0.clone();
        i += 1;
        let Some(def) = notes.defs.get(id.as_str()).copied() else { continue };
        b.open("li", vec![attr("id", format!("fn-{id}"))]);
        let mut sub = Builder::default();
        walk(&mut sub, &def.children, notes, false);
        let mut kids = sub.finish();
        let calls = notes.calls[i - 1].1;
        let mut backs: Vec<Dom> = Vec::new();
        for call in 1..=calls {
            if call > 1 {
                backs.push(Dom::Text(" ".into()));
            }
            let href = if call > 1 { format!("#fnref-{id}-{call}") } else { format!("#fnref-{id}") };
            let mut a = El { tag: "a".into(), attrs: vec![attr("href", href), attr("data-footnote-backref", "")], kids: vec![Dom::Text("↩".into())], ..Default::default() };
            if call > 1 {
                a.kids.push(Dom::El(El { tag: "sup".into(), kids: vec![Dom::Text(call.to_string())], ..Default::default() }));
            }
            backs.push(Dom::El(a));
        }
        match kids.last_mut() {
            Some(Dom::El(p)) if p.tag == "p" => {
                match p.kids.last_mut() {
                    Some(Dom::Text(t)) => t.push(' '),
                    _ => p.kids.push(Dom::Text(" ".into())),
                }
                p.kids.append(&mut backs);
            }
            _ => kids.append(&mut backs),
        }
        for k in kids {
            match k {
                Dom::El(el) => b.element(el),
                Dom::Text(t) => b.text(&t),
            }
        }
        b.close("li");
    }
    b.close("ol");
    b.close("section");
}

const SCHEMES_HREF: [&str; 3] = ["http", "https", "mailto"];
const SCHEMES_SRC: [&str; 3] = ["http", "https", "data"];

/// `hast-util-sanitize`'s protocol check: relative, or one of `allowed`, case and all.
fn safe_protocol(value: &str, allowed: &[&str]) -> bool {
    let Some(colon) = value.find(':') else { return true };
    if value.find(['/', '?', '#']).is_some_and(|k| k < colon) {
        return true;
    }
    allowed.iter().any(|p| &value[..colon] == *p)
}

/// A network path a browser reads as `//host` that isn't written as one.
fn disguised_network_path(u: &str) -> bool {
    let lead = u.trim_start_matches(|c: char| (c as u32) <= 0x20);
    let clean: String = lead.chars().filter(|c| !matches!(c, '\t' | '\n' | '\r')).collect();
    let mut b = clean.bytes();
    matches!((b.next(), b.next()), (Some(b'/' | b'\\'), Some(b'/' | b'\\'))) && !lead.starts_with("//")
}

fn starts_data(u: &str) -> bool {
    u.trim_start().to_ascii_lowercase().starts_with("data:")
}

fn clean_attrs(el: &mut El) {
    let tag = el.tag.clone();
    el.attrs.retain_mut(|(k, v)| {
        if !ATTRS.contains(&k.as_str()) {
            return false;
        }
        match k.as_str() {
            "id" | "name" => *v = format!("user-content-{v}"),
            "href" => return tag == "a" && safe_protocol(v, &SCHEMES_HREF) && !starts_data(v) && !disguised_network_path(v),
            "src" => return tag == "img" && safe_protocol(v, &SCHEMES_SRC) && !(starts_data(v) && !super::links::data_image(v.trim())) && !disguised_network_path(v),
            "checked" => return false,
            _ => {}
        }
        true
    });
}

fn sanitize_kids(kids: Vec<Dom>, parent_is_item: bool, parent_is_item_p: bool) -> Vec<Dom> {
    let first_el = kids.iter().position(|k| matches!(k, Dom::El(_)));
    let mut out = Vec::with_capacity(kids.len());
    for (i, k) in kids.into_iter().enumerate() {
        match k {
            Dom::Text(t) => match out.last_mut() {
                Some(Dom::Text(prev)) => prev.push_str(&t),
                _ => out.push(Dom::Text(t)),
            },
            Dom::El(mut el) => {
                if let Some(s) = &el.special {
                    // The app's own: only what it holds is the document's.
                    let item = matches!(**s, Special::Marked(_)) && el.tag == "li";
                    el.kids = sanitize_kids(std::mem::take(&mut el.kids), item, false);
                    clean_attrs(&mut el);
                    out.push(Dom::El(el));
                    continue;
                }
                let tag = el.tag.as_str();
                if STRIP.contains(&tag) {
                    continue;
                }
                if !ALLOWED.contains(&tag) {
                    let inner = sanitize_kids(el.kids, parent_is_item, parent_is_item_p);
                    for x in inner {
                        match x {
                            Dom::Text(t) => match out.last_mut() {
                                Some(Dom::Text(prev)) => prev.push_str(&t),
                                _ => out.push(Dom::Text(t)),
                            },
                            e => out.push(e),
                        }
                    }
                    continue;
                }
                if tag == "input" {
                    // `rehypeTaskInputsOnly`: only where GFM puts a task's box, disabled.
                    let in_place = (parent_is_item || parent_is_item_p) && first_el == Some(i);
                    if in_place {
                        let on = el.has("checked");
                        out.push(Dom::El(El::special("input", Special::Task(on))));
                    }
                    continue;
                }
                let is_item = tag == "li";
                let item_p = parent_is_item && tag == "p";
                el.kids = sanitize_kids(std::mem::take(&mut el.kids), is_item, item_p);
                clean_attrs(&mut el);
                out.push(Dom::El(el));
            }
        }
    }
    out
}

fn heading_text(el: &El) -> String {
    fn push(d: &[Dom], out: &mut String) {
        for k in d {
            match k {
                Dom::Text(t) => out.push_str(t),
                Dom::El(e) => {
                    if let Some(Special::Reference(r)) = e.special.as_deref() {
                        out.push_str(&r.value);
                    }
                    push(&e.kids, out);
                }
            }
        }
    }
    let mut s = String::new();
    push(&el.kids, &mut s);
    s
}

/// `rehypeHeadingIds`: a heading written as raw HTML gets GitHub's slug too, never one a Markdown heading has.
fn raw_heading_ids(kids: &mut [Dom], slugger: &mut Slugger, take_only: bool) {
    for k in kids {
        if let Dom::El(el) = k {
            let heading = matches!(el.tag.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
            if take_only {
                if let Some(id) = el.attr("id") {
                    slugger.take(id);
                }
            } else if heading && !el.has("id") {
                let id = slugger.slug(&heading_text(el));
                el.attrs.push(attr("id", id));
            }
            raw_heading_ids(&mut el.kids, slugger, take_only);
        }
    }
}

/// `nodes` as the sanitized tree.
pub fn to_dom(nodes: &[Node]) -> Vec<Dom> {
    let mut defs = HashMap::new();
    collect_notes(nodes, &mut defs);
    let mut notes = Notes { defs, calls: Vec::new() };
    let mut b = Builder::default();
    walk(&mut b, nodes, &mut notes, false);
    notes_section(&mut b, &mut notes);
    let mut kids = b.finish();
    let mut slugger = Slugger::default();
    raw_heading_ids(&mut kids, &mut slugger, true);
    raw_heading_ids(&mut kids, &mut slugger, false);
    sanitize_kids(kids, false, false)
}

#[cfg(test)]
mod tests {
    use super::super::ast::{Flavor, parse};
    use super::*;
    use std::prelude::v1::test;

    fn dom(s: &str) -> Vec<Dom> {
        to_dom(&parse(s, Flavor::Github, false).root)
    }

    /// The tree as HTML, for reading in a test.
    fn html(d: &[Dom]) -> String {
        d.iter()
            .map(|k| match k {
                Dom::Text(t) => t.clone(),
                Dom::El(e) => {
                    let attrs: String = e.attrs.iter().map(|(k, v)| format!(" {k}={v:?}")).collect();
                    format!("<{}{attrs}>{}</{}>", e.tag, html(&e.kids), e.tag)
                }
            })
            .collect()
    }

    #[test]
    fn scripts_and_their_kin_go_with_what_they_hold() {
        assert_eq!(html(&dom("<script>alert(1)</script>\n\nA <style>p{}</style>b")).replace('\n', ""), "<p>A b</p>");
        assert_eq!(html(&dom("<svg><title>t</title><script>x</script></svg>")).replace('\n', ""), "<p></p>");
        assert_eq!(html(&dom("<iframe src=\"https://evil.example\">in</iframe>x")).replace('\n', ""), "x");
        assert_eq!(html(&dom("<textarea><b>x</b></textarea><select><option>o</select>")).replace('\n', ""), "");
        assert_eq!(html(&dom("a<!-- c -->b")).replace('\n', ""), "<p>ab</p>");
    }

    #[test]
    fn a_tag_the_schema_leaves_out_keeps_its_children() {
        assert_eq!(html(&dom("<mark>a</mark> <small>b</small> <blink>c</blink>")).replace('\n', ""), "<p>a b c</p>");
        assert_eq!(html(&dom("<custom-tag>\n<em>a</em>\n</custom-tag>")).replace('\n', ""), "<em>a</em>");
    }

    #[test]
    fn attributes_are_narrowed_and_ids_prefixed() {
        assert_eq!(html(&dom("<div id=\"x\" class=\"c\" style=\"color:red\" onclick=\"y\" align=\"center\">a</div>")).replace('\n', ""), "<div id=\"user-content-x\" align=\"center\">a</div>");
        assert_eq!(html(&dom("<a href=\"javascript:alert(1)\" name=\"n\">x</a>")).replace('\n', ""), "<p><a name=\"user-content-n\">x</a></p>");
        assert_eq!(html(&dom("<a href=\"data:text/html,x\">x</a>")).replace('\n', ""), "<p><a>x</a></p>");
        assert_eq!(html(&dom("<img src=\"data:image/svg+xml;base64,AA\" alt=\"a\">")).replace('\n', ""), "<img alt=\"a\"></img>");
        assert_eq!(html(&dom("<img src=\"data:image/png;base64,AA\">")).replace('\n', ""), "<img src=\"data:image/png;base64,AA\"></img>");
        assert_eq!(html(&dom("<a href=\"\\\\\\\\host\">x</a>")).replace('\n', ""), "<p><a>x</a></p>");
    }

    #[test]
    fn a_document_cannot_forge_what_the_app_puts_on_a_node() {
        let d = dom("<span data-gb-ref=\"0\" data-gb-diff=\"x:added\">a</span>\n\n<div data-gb-diff=\"removed\">b</div>");
        let flat = html(&d).replace('\n', "");
        assert!(!flat.contains("data-gb"), "{flat}");
        let has_special = |d: &[Dom]| d.iter().any(|k| matches!(k, Dom::El(e) if e.special.is_some()));
        assert!(!has_special(&d));
    }

    #[test]
    fn only_a_task_item_keeps_an_input() {
        assert_eq!(html(&dom("<input type=\"text\" value=\"x\">")).replace('\n', ""), "");
        let d = dom("- [x] done");
        let Dom::El(ul) = &d[0] else { panic!() };
        let Dom::El(li) = &ul.kids[0] else { panic!() };
        assert_eq!(li.kids[0], Dom::El(El::special("input", Special::Task(true))));
        assert!(html(&dom("<ul><li><b>a</b> <input type=\"checkbox\"> b</li></ul>")).contains("<li><b>a</b>  b</li>"), "not where GFM puts one");
    }

    #[test]
    fn raw_html_and_markdown_nest_as_one_tree() {
        let flat = html(&dom("<details>\n<summary>S</summary>\n\nText **b**\n\n</details>\n\nAfter")).replace('\n', "");
        assert_eq!(flat, "<details><summary>S</summary><p>Text <strong>b</strong></p></details><p>After</p>");
        // A block element closes an open paragraph; an unknown closer does nothing.
        assert_eq!(html(&dom("<p>one<div>two</div></span>")).replace('\n', ""), "<p>one</p><div>two</div>");
    }

    #[test]
    fn a_table_written_as_html_reads_like_one() {
        let flat = html(&dom("<table><tr><td>a<td>b<tr><td>c</table>")).replace('\n', "");
        assert_eq!(flat, "<table><tr><td>a</td><td>b</td></tr><tr><td>c</td></tr></table>");
        assert_eq!(html(&dom("<td>orphan</td>")).replace('\n', ""), "orphan", "no table: the tag's ignored");
    }

    #[test]
    fn references_inside_raw_code_or_links_stay_text() {
        let d = dom("<code>#12</code> and #13");
        let flat = html(&d);
        assert!(flat.contains("<code>#12</code>"), "{flat}");
        assert!(flat.contains("<span></span>"), "{flat}");
    }

    #[test]
    fn footnotes_number_in_call_order_with_their_backrefs() {
        let flat = html(&dom("a[^x] b[^y] c[^x]\n\n[^x]: X.\n[^y]: Y.\n")).replace('\n', "");
        assert!(flat.contains("<sup><a href=\"#fn-x\" id=\"user-content-fnref-x\" aria-describedby=\"footnote-label\">1</a></sup>"), "{flat}");
        assert!(flat.contains("<a href=\"#fn-x\" id=\"user-content-fnref-x-2\""), "{flat}");
        assert!(flat.contains("<section data-footnotes=\"\"><ol><li id=\"user-content-fn-x\"><p>X. <a href=\"#fnref-x\""), "{flat}");
        assert!(flat.contains("href=\"#fnref-x-2\" data-footnote-backref=\"\">↩<sup>2</sup></a>"), "{flat}");
    }

    #[test]
    fn a_raw_heading_gets_a_slug_no_markdown_heading_has() {
        let flat = html(&dom("# Top\n\n<h2>Top</h2>\n")).replace('\n', "");
        assert!(flat.contains("<h1 id=\"user-content-top\">"), "{flat}");
        assert!(flat.contains("<h2 id=\"user-content-top-1\">"), "{flat}");
    }

    #[test]
    fn entities_decode_in_raw_html_only() {
        assert_eq!(html(&dom("<div>a &amp; &#65; &nosuch; &</div>")).replace('\n', ""), "<div>a & A &nosuch; &</div>");
        assert_eq!(html(&dom("&lt;b&gt;")), "<p><b></p>");
    }

    fn depth(d: &[Dom]) -> usize {
        d.iter().map(|k| if let Dom::El(e) = k { 1 + depth(&e.kids) } else { 0 }).max().unwrap_or(0)
    }

    #[test]
    fn a_million_open_tags_nest_no_deeper_than_the_cap() {
        std::thread::Builder::new()
            .stack_size(1 << 21)
            .spawn(|| {
                for s in ["<div>".repeat(300_000), "<blockquote><ul><li>".repeat(100_000), "<details>".repeat(200_000), "<a href=x><b>".repeat(100_000), "<table><tr><td>".repeat(100_000)] {
                    let d = dom(&s);
                    assert!(depth(&d) <= MAX_DEPTH + 1, "{}: {}", &s[..8], depth(&d));
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn long_runs_in_raw_html_stay_linear() {
        for s in ["&".repeat(1_000_000), "<script></script>".repeat(100_000), "<style>".repeat(100_000), "<".repeat(1_000_000), "</".repeat(500_000), "<!".repeat(500_000), "<a ".repeat(300_000), format!("<a {}>", "b ".repeat(300_000)), format!("<a {}>", (0..100_000).map(|i| format!("b{i}=1 ")).collect::<String>())] {
            let started = std::time::Instant::now();
            let _ = dom(&format!("<div>{s}</div>"));
            assert!(started.elapsed().as_secs() < 30, "{}", &s[..8]);
        }
    }

    #[test]
    fn a_wrapper_keeps_stray_html_to_itself() {
        let block = Node::with(Kind::DiffBlock { mark: Mark::Removed, note: None }, vec![Node::new(Kind::Html("</div><p>x".into()))]);
        let d = to_dom(&[Node::new(Kind::Html("<div>".into())), block, Node::text("after")]);
        let flat = html(&d);
        assert_eq!(flat, "<div><div><p>x</p></div>after</div>");
    }
}
