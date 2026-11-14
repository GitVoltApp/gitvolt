//! The Markdown tree: `ui/src/markdown/parse.ts`'s mdast, read by `pulldown-cmark` with GFM's tables, task
//! lists, strikethrough and footnotes, then its plugins: autolink literals, GitLab's image attributes, emoji,
//! references, heading ids and a file's front matter. What a rendered diff adds is here too (`diff.rs`).

use super::frontmatter;
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Flavor {
    #[default]
    Github,
    Gitlab,
    Forgejo,
}

/// Where a node came from: bytes and 1-based lines of the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pos {
    pub start: usize,
    pub end: usize,
    pub line_start: u32,
    pub line_end: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Added,
    Removed,
    Changed,
}

impl Mark {
    pub fn label(self) -> &'static str {
        match self {
            Mark::Added => "Added",
            Mark::Removed => "Removed",
            Mark::Changed => "Changed",
        }
    }
}

/// A reference the `references` plugin found (`MdReferenceNode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefNode {
    pub kind: RefKind,
    /// `owner/repo` or `group/sub/project` when written; `None`: this project.
    pub project: Option<String>,
    pub number: Option<u64>,
    pub sha: Option<String>,
    pub user: Option<String>,
    /// The text as written: what the link shows.
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Issue,
    Mr,
    Commit,
    Mention,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Root,
    Paragraph,
    Heading { depth: u8 },
    ThematicBreak,
    Blockquote,
    List { ordered: bool, start: u64 },
    ListItem { checked: Option<bool> },
    Code { lang: Option<String>, meta: Option<String>, value: String },
    /// Raw HTML, a block's or an inline one's.
    Html(String),
    FootnoteDefinition { id: String },
    Table { align: Vec<Align> },
    TableRow,
    TableCell,
    Text(String),
    Emphasis,
    Strong,
    Delete,
    InlineCode(String),
    Break,
    /// `reference`: the definition's label, when it's `[text][ref]`.
    Link { url: String, title: Option<String>, reference: Option<String> },
    Image { url: String, title: Option<String>, alt: String, reference: Option<String> },
    FootnoteReference { id: String },
    Reference(RefNode),
    // A rendered diff's (`diff/nodes.ts`).
    DiffBlock { mark: Mark, note: Option<String> },
    DiffPair,
    DiffIns,
    DiffDel,
    SplitRow { mark: Option<Mark>, joined: bool },
    SplitCell { old: bool, empty: bool },
}

/// `node.data`: what the plugins and the diff put on a node.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Data {
    /// A heading's slug.
    pub id: Option<String>,
    /// An image's GitLab attributes.
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frontmatter: Option<frontmatter::Lang>,
    /// `gbDiff`, on a list item or a table row.
    pub diff: Option<Mark>,
    /// `gbValue`: an ordered item's number.
    pub value: Option<u64>,
    pub old_value: Option<u64>,
    pub old_checked: Option<Option<bool>>,
    /// `gbLines`, `gbWords` on code.
    pub lines: Option<String>,
    pub words: Option<String>,
    /// `gbEmpty`: a table row the other side has.
    pub empty: bool,
    /// A tight list's item: its paragraph shows without one.
    pub tight: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub kind: Kind,
    pub children: Vec<Node>,
    pub pos: Option<Pos>,
    pub data: Option<Box<Data>>,
}

impl Node {
    pub fn new(kind: Kind) -> Self {
        Self { kind, children: Vec::new(), pos: None, data: None }
    }

    pub fn with(kind: Kind, children: Vec<Node>) -> Self {
        Self { kind, children, pos: None, data: None }
    }

    pub fn text(s: impl Into<String>) -> Self {
        Self::new(Kind::Text(s.into()))
    }

    pub fn data_mut(&mut self) -> &mut Data {
        self.data.get_or_insert_with(Default::default)
    }

    pub fn data(&self) -> Option<&Data> {
        self.data.as_deref()
    }

    /// `textOf`: what it says, blocks run together.
    pub fn text_of(&self) -> String {
        let mut out = String::new();
        self.push_text(&mut out);
        out
    }

    fn push_text(&self, out: &mut String) {
        match &self.kind {
            Kind::Text(s) | Kind::InlineCode(s) | Kind::Html(s) => out.push_str(s),
            Kind::Code { value, .. } => out.push_str(value),
            Kind::Reference(r) => out.push_str(&r.value),
            _ => {
                for c in &self.children {
                    c.push_text(out);
                }
            }
        }
    }
}

/// The parsed document: its tree, and its link definitions, by normalized label. `too_large`: it ran past what a body
/// may cost (`MAX_EVENTS`, `PARSE_BUDGET`), and shows as written.
#[derive(Debug, Clone, Default)]
pub struct Parsed {
    pub root: Vec<Node>,
    pub links: HashMap<String, (String, Option<String>)>,
    pub too_large: bool,
}

/// A repository's file is untrusted: these keep what it can cost in proportion, whatever it says.
///
/// How deep the tree goes: what walks it recurses. Past this a node's children go to the node above.
const MAX_DEPTH: usize = 100;
/// The most nodes a body makes before it's too large.
const MAX_EVENTS: usize = 2_000_000;
/// `PARSE_BUDGET_MS`: a parse taking longer shows Source.
const PARSE_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);
/// What `[text][ref]` may copy of its definitions in all: one long definition and a million uses is terabytes.
const MAX_REFERENCE_BYTES: usize = 32 << 20;

/// A label as a definition matches it: case folded, spaces collapsed.
pub fn normalize_label(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn line_index(text: &str) -> Vec<usize> {
    let mut v = vec![0];
    v.extend(text.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(i, _)| i + 1));
    v
}

struct Builder<'a> {
    stack: Vec<Node>,
    lines: &'a [usize],
    text: &'a str,
    base: usize,
    base_line: u32,
    /// What's open past `MAX_DEPTH`.
    flattened: usize,
    copied: usize,
}

impl Builder<'_> {
    fn pos(&self, r: std::ops::Range<usize>) -> Pos {
        let line = |at: usize| self.lines.partition_point(|l| *l <= at) as u32;
        // A block's range can end after blank lines, which aren't its lines.
        let kept = self.text.get(r.clone()).map_or(0, |t| t.trim_end_matches(['\n', '\r', ' ', '\t']).len());
        let end_at = r.start + kept.saturating_sub(1);
        Pos { start: r.start + self.base, end: r.end + self.base, line_start: line(r.start) + self.base_line, line_end: line(end_at) + self.base_line }
    }

    fn open(&mut self, kind: Kind, r: std::ops::Range<usize>) {
        if self.stack.len() >= MAX_DEPTH {
            self.flattened += 1;
            return;
        }
        let mut n = Node::new(kind);
        n.pos = Some(self.pos(r));
        self.stack.push(n);
    }

    fn close(&mut self) {
        if self.flattened > 0 {
            self.flattened -= 1;
            return;
        }
        if self.stack.len() > 1 {
            let n = self.stack.pop().expect("a node to close");
            self.stack.last_mut().expect("the root").children.push(n);
        }
    }

    fn push(&mut self, n: Node) {
        self.stack.last_mut().expect("the root").children.push(n);
    }

    fn text(&mut self, s: &str) {
        let top = self.stack.last_mut().expect("the root");
        // An image's alt is its text.
        if let Kind::Image { alt, .. } = &mut top.kind {
            alt.push_str(s);
            return;
        }
        if let Some(Node { kind: Kind::Text(t), .. }) = top.children.last_mut() {
            t.push_str(s);
        } else {
            top.children.push(Node::text(s));
        }
    }
}

impl Builder<'_> {
    /// A reference's copy of its definition, while the document has some left: past it, the link has no target.
    fn reference_copy(&mut self, is_reference: bool, url: &str, title: &str) -> (String, String) {
        if is_reference {
            self.copied = self.copied.saturating_add(url.len() + title.len());
            if self.copied > MAX_REFERENCE_BYTES {
                return (String::new(), String::new());
            }
        }
        (url.to_string(), title.to_string())
    }
}

fn to_align(a: &Alignment) -> Align {
    match a {
        Alignment::None => Align::None,
        Alignment::Left => Align::Left,
        Alignment::Center => Align::Center,
        Alignment::Right => Align::Right,
    }
}

/// `text`'s tree. `frontmatter`: a leading YAML or TOML block is a table; only a file has one.
pub fn parse(text: &str, flavor: Flavor, with_frontmatter: bool) -> Parsed {
    let (matter, rest, base) = if with_frontmatter { frontmatter::split(text) } else { (None, text, 0) };
    let base_line = text[..base].bytes().filter(|b| *b == b'\n').count() as u32;
    let opts = Options::ENABLE_TABLES | Options::ENABLE_FOOTNOTES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(rest, opts);
    let mut links = HashMap::new();
    for (label, def) in parser.reference_definitions().iter() {
        links.insert(normalize_label(label), (def.dest.to_string(), def.title.as_ref().map(|t| t.to_string())));
    }
    let lines = line_index(rest);
    let mut b = Builder { stack: vec![Node::new(Kind::Root)], lines: &lines, text: rest, base, base_line, flattened: 0, copied: 0 };
    let started = std::time::Instant::now();
    for (count, (event, range)) in parser.into_offset_iter().enumerate() {
        if count >= MAX_EVENTS || (count % 4096 == 4095 && started.elapsed() > PARSE_BUDGET) {
            return Parsed { root: Vec::new(), links: HashMap::new(), too_large: true };
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => b.open(Kind::Paragraph, range),
                Tag::Heading { level, .. } => b.open(
                    Kind::Heading {
                        depth: match level {
                            HeadingLevel::H1 => 1,
                            HeadingLevel::H2 => 2,
                            HeadingLevel::H3 => 3,
                            HeadingLevel::H4 => 4,
                            HeadingLevel::H5 => 5,
                            HeadingLevel::H6 => 6,
                        },
                    },
                    range,
                ),
                Tag::BlockQuote(_) => b.open(Kind::Blockquote, range),
                Tag::CodeBlock(kind) => {
                    let (lang, meta) = match kind {
                        CodeBlockKind::Indented => (None, None),
                        CodeBlockKind::Fenced(info) => {
                            let info = info.trim();
                            match info.split_once(char::is_whitespace) {
                                Some((l, m)) => (Some(l.to_string()), Some(m.trim().to_string())),
                                None if info.is_empty() => (None, None),
                                None => (Some(info.to_string()), None),
                            }
                        }
                    };
                    b.open(Kind::Code { lang, meta, value: String::new() }, range);
                }
                Tag::HtmlBlock => b.open(Kind::Html(String::new()), range),
                Tag::List(start) => b.open(Kind::List { ordered: start.is_some(), start: start.unwrap_or(1) }, range),
                Tag::Item => b.open(Kind::ListItem { checked: None }, range),
                Tag::FootnoteDefinition(id) => b.open(Kind::FootnoteDefinition { id: normalize_label(&id) }, range),
                Tag::Table(aligns) => b.open(Kind::Table { align: aligns.iter().map(to_align).collect() }, range),
                Tag::TableHead | Tag::TableRow => b.open(Kind::TableRow, range),
                Tag::TableCell => b.open(Kind::TableCell, range),
                Tag::Emphasis => b.open(Kind::Emphasis, range),
                Tag::Strong => b.open(Kind::Strong, range),
                Tag::Strikethrough => b.open(Kind::Delete, range),
                Tag::Link { link_type, dest_url, title, id } => {
                    let url = if link_type == LinkType::Email { format!("mailto:{dest_url}") } else { dest_url.to_string() };
                    let reference = matches!(link_type, LinkType::Reference | LinkType::Collapsed | LinkType::Shortcut).then(|| normalize_label(&id));
                    let (url, title) = b.reference_copy(reference.is_some(), &url, &title);
                    b.open(Kind::Link { url, title: Some(title).filter(|t| !t.is_empty()), reference }, range);
                }
                Tag::Image { link_type, dest_url, title, id } => {
                    let reference = matches!(link_type, LinkType::Reference | LinkType::Collapsed | LinkType::Shortcut).then(|| normalize_label(&id));
                    let (url, title) = b.reference_copy(reference.is_some(), &dest_url, &title);
                    b.open(Kind::Image { url, title: Some(title).filter(|t| !t.is_empty()), alt: String::new(), reference }, range);
                }
                _ => b.open(Kind::Paragraph, range),
            },
            Event::End(tag) => {
                if let TagEnd::CodeBlock = tag
                    && let Some(Node { kind: Kind::Code { value, .. }, .. }) = b.stack.last_mut()
                    && value.ends_with('\n')
                {
                    value.pop();
                }
                if let TagEnd::HtmlBlock = tag
                    && let Some(Node { kind: Kind::Html(value), .. }) = b.stack.last_mut()
                    && value.ends_with('\n')
                {
                    value.pop();
                }
                b.close();
            }
            Event::Text(t) => match b.stack.last_mut().map(|n| &mut n.kind) {
                Some(Kind::Code { value, .. }) => value.push_str(&t),
                _ => b.text(&t),
            },
            Event::Code(t) => match b.stack.last_mut().map(|n| &mut n.kind) {
                Some(Kind::Image { alt, .. }) => alt.push_str(&t),
                _ => b.push(Node::new(Kind::InlineCode(t.to_string()))),
            },
            Event::Html(t) => match b.stack.last_mut().map(|n| &mut n.kind) {
                Some(Kind::Html(value)) => value.push_str(&t),
                _ => {
                    let mut n = Node::new(Kind::Html(t.to_string()));
                    n.pos = Some(b.pos(range));
                    b.push(n);
                }
            },
            Event::InlineHtml(t) => {
                if !matches!(b.stack.last().map(|n| &n.kind), Some(Kind::Image { .. })) {
                    b.push(Node::new(Kind::Html(t.to_string())));
                }
            }
            Event::FootnoteReference(id) => b.push(Node::new(Kind::FootnoteReference { id: normalize_label(&id) })),
            Event::SoftBreak => b.text("\n"),
            Event::HardBreak => {
                if matches!(b.stack.last().map(|n| &n.kind), Some(Kind::Image { .. })) {
                    b.text(" ");
                } else {
                    b.push(Node::new(Kind::Break));
                }
            }
            Event::Rule => {
                let mut n = Node::new(Kind::ThematicBreak);
                n.pos = Some(b.pos(range));
                b.push(n);
            }
            Event::TaskListMarker(on) => {
                for n in b.stack.iter_mut().rev() {
                    if let Kind::ListItem { checked } = &mut n.kind {
                        *checked = Some(on);
                        break;
                    }
                }
            }
            Event::InlineMath(t) | Event::DisplayMath(t) => b.text(&t),
        }
    }
    while b.stack.len() > 1 {
        b.close();
    }
    let mut root = b.stack.pop().map(|n| n.children).unwrap_or_default();
    tight_paragraphs(&mut root);
    transform(&mut root, flavor, false);
    heading_ids(&mut root, &mut Slugger::default());
    if let Some(block) = matter {
        let mut front = frontmatter::nodes(&block);
        front.append(&mut root);
        root = front;
    }
    Parsed { root, links, too_large: false }
}

fn is_inline(n: &Node) -> bool {
    matches!(n.kind, Kind::Text(_) | Kind::Emphasis | Kind::Strong | Kind::Delete | Kind::InlineCode(_) | Kind::Break | Kind::Link { .. } | Kind::Image { .. } | Kind::FootnoteReference { .. } | Kind::Reference(_)) || (matches!(n.kind, Kind::Html(_)) && n.pos.is_none())
}

/// mdast has a paragraph in every list item; a tight list's just doesn't show. The parser leaves it out: a diff needs it.
fn tight_paragraphs(nodes: &mut [Node]) {
    for n in nodes {
        tight_paragraphs(&mut n.children);
        if !matches!(n.kind, Kind::ListItem { .. }) || !n.children.iter().any(is_inline) {
            continue;
        }
        let mut out = Vec::with_capacity(n.children.len());
        let mut run = Vec::new();
        for k in std::mem::take(&mut n.children) {
            if is_inline(&k) {
                run.push(k);
                continue;
            }
            if !run.is_empty() {
                let mut p = Node::with(Kind::Paragraph, std::mem::take(&mut run));
                p.data_mut().tight = true;
                out.push(p);
            }
            out.push(k);
        }
        if !run.is_empty() {
            let mut p = Node::with(Kind::Paragraph, run);
            p.data_mut().tight = true;
            out.push(p);
        }
        n.children = out;
    }
}

/// Tokens (runs without ASCII whitespace) longer than this aren't autolinked (`autolinkGuard.ts`).
pub const MAX_AUTOLINK_TOKEN: usize = 1024;

fn has_long_token(s: &str) -> bool {
    s.split([' ', '\t', '\n', '\r']).any(|t| t.len() > MAX_AUTOLINK_TOKEN)
}

fn transform(nodes: &mut Vec<Node>, flavor: Flavor, in_link: bool) {
    if flavor == Flavor::Gitlab {
        image_attrs(nodes);
    }
    let mut out = Vec::with_capacity(nodes.len());
    for mut n in nodes.drain(..) {
        match &n.kind {
            Kind::Text(s) => {
                let s = s.clone();
                let keep_pos = n.pos;
                let mut parts = if in_link || has_long_token(&s) { vec![Node::text(s)] } else { autolinks(&s) };
                parts = parts.into_iter().flat_map(emoji).collect();
                if !in_link {
                    parts = references(parts, flavor);
                }
                for p in &mut parts {
                    p.pos = p.pos.or(keep_pos);
                }
                out.append(&mut parts);
            }
            Kind::Link { .. } => {
                transform(&mut n.children, flavor, true);
                out.push(n);
            }
            _ => {
                transform(&mut n.children, flavor, in_link);
                out.push(n);
            }
        }
    }
    *nodes = out;
}

/// `remarkImageAttrs`: GitLab's `![a](b.png){width=900 height=575}`.
fn image_attrs(kids: &mut Vec<Node>) {
    let mut i = 0;
    let mut emptied = false;
    while i + 1 < kids.len() {
        let is_image = matches!(kids[i].kind, Kind::Image { .. });
        let attrs = match (&kids[i + 1].kind, is_image) {
            (Kind::Text(t), true) => parse_image_attrs(t),
            _ => None,
        };
        if let Some((used, w, h)) = attrs {
            if w.is_some() || h.is_some() {
                let d = kids[i].data_mut();
                if w.is_some() {
                    d.width = w;
                }
                if h.is_some() {
                    d.height = h;
                }
            }
            let rest = match &kids[i + 1].kind {
                Kind::Text(t) => t[used..].to_string(),
                _ => String::new(),
            };
            emptied |= rest.is_empty();
            kids[i + 1].kind = Kind::Text(rest);
        }
        i += 1;
    }
    // Out in one go: one at a time is quadratic in a line of pictures.
    if emptied {
        kids.retain(|k| !matches!(&k.kind, Kind::Text(t) if t.is_empty()));
    }
}

/// How many bytes an attribute list at `t`'s start takes, and the size it gives.
pub fn parse_image_attrs(t: &str) -> Option<(usize, Option<u32>, Option<u32>)> {
    let b = t.as_bytes();
    if b.first() != Some(&b'{') {
        return None;
    }
    let mut i = 1;
    let skip = |i: &mut usize| {
        while matches!(b.get(*i), Some(b' ' | b'\t')) {
            *i += 1;
        }
    };
    skip(&mut i);
    let (mut w, mut h, mut any) = (None, None, false);
    loop {
        let name_at = i;
        while b.get(i).is_some_and(u8::is_ascii_alphabetic) {
            i += 1;
        }
        if i == name_at || b.get(i) != Some(&b'=') {
            break;
        }
        let name = t[name_at..i].to_ascii_lowercase();
        i += 1;
        let value = if b.get(i) == Some(&b'"') {
            let end = t[i + 1..].find(['"', '\n']).map(|k| k + i + 1)?;
            if b[end] != b'"' {
                return None;
            }
            let v = &t[i + 1..end];
            i = end + 1;
            v
        } else {
            let from = i;
            while b.get(i).is_some_and(|c| !c.is_ascii_whitespace() && !matches!(c, b'"' | b'{' | b'}')) {
                i += 1;
            }
            if i == from {
                return None;
            }
            &t[from..i]
        };
        any = true;
        let digits = value.strip_suffix("px").or(value.strip_suffix("PX")).unwrap_or(value);
        let px = (!digits.is_empty() && digits.len() <= 5 && digits.bytes().all(|c| c.is_ascii_digit())).then(|| digits.parse::<u32>().ok()).flatten();
        match name.as_str() {
            "width" => w = px.or(w),
            "height" => h = px.or(h),
            _ => {}
        }
        skip(&mut i);
    }
    (any && b.get(i) == Some(&b'}')).then_some((i + 1, w, h))
}

fn is_ascii_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn unicode_punct_or_space(c: char) -> bool {
    c.is_whitespace() || !c.is_alphanumeric() && !unicode_is_mark(c)
}

fn unicode_is_mark(c: char) -> bool {
    matches!(c as u32, 0x300..=0x36f | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff | 0x20d0..=0x20ff | 0xfe00..=0xfe0f | 0xfe20..=0xfe2f)
}

/// `previous` in mdast-util-gfm-autolink-literal.
fn previous_ok(text: &str, at: usize, email: bool) -> bool {
    match text[..at].chars().next_back() {
        None => true,
        Some(c) => unicode_punct_or_space(c) && !(email && c == '/'),
    }
}

fn correct_domain(domain: &str) -> bool {
    let parts: Vec<&str> = domain.split('.').collect();
    if parts.len() < 2 {
        return false;
    }
    let bad = |p: &str| !p.is_empty() && (p.contains('_') || !p.chars().any(|c| c.is_ascii_alphanumeric()));
    !bad(parts[parts.len() - 1]) && !bad(parts[parts.len() - 2])
}

/// `splitUrl`: the URL, and what's left of its trailing punctuation.
fn split_url(url: &str) -> (String, String) {
    let is_trail = |c: char| "!\"&'),.:;<>?]}".contains(c);
    let keep = url.trim_end_matches(is_trail).len();
    if keep == url.len() {
        return (url.to_string(), String::new());
    }
    let (mut head, mut trail) = (url[..keep].to_string(), url[keep..].to_string());
    let opening = head.matches('(').count();
    let mut closing = head.matches(')').count();
    while let Some(at) = trail.find(')') {
        if opening <= closing {
            break;
        }
        head.push_str(&trail[..=at]);
        trail = trail[at + 1..].to_string();
        closing += 1;
    }
    (head, trail)
}

fn link_node(url: String, shown: String) -> Node {
    Node::with(Kind::Link { url, title: None, reference: None }, vec![Node::text(shown)])
}

/// GFM's literal autolinks: `http://`, `https://` and `www.`, then emails.
pub fn autolinks(s: &str) -> Vec<Node> {
    if !s.contains("://") && !s.contains('@') && !s.to_ascii_lowercase().contains("www.") {
        return vec![Node::text(s)];
    }
    let urls = autolink_urls(s);
    urls.into_iter().flat_map(|n| if let Kind::Text(t) = &n.kind { autolink_emails(t) } else { vec![n] }).collect()
}

fn autolink_urls(s: &str) -> Vec<Node> {
    let lower = s.to_ascii_lowercase();
    let mut out = Vec::new();
    let (mut from, mut search) = (0, 0);
    // Where each is next, found again only once it's passed: a text of many `www.` and no `http` stays linear.
    const PROTOS: [&str; 3] = ["https://", "http://", "www."];
    let mut found: [Option<usize>; 3] = [Some(0); 3];
    while search < s.len() {
        for (slot, p) in found.iter_mut().zip(PROTOS) {
            if slot.is_some_and(|at| at < search || (at == 0 && search == 0)) {
                *slot = lower[search..].find(p).map(|k| k + search);
            }
        }
        let next = found.iter().zip(PROTOS).filter_map(|(at, p)| at.map(|a| (a, p))).min_by_key(|x| x.0);
        let Some((at, proto)) = next else { break };
        let after = at + proto.len();
        let end_domain = s[after..].char_indices().find(|(_, c)| !(is_ascii_word(*c) || *c == '-' || *c == '.')).map_or(s.len(), |(i, _)| after + i);
        let end = s[end_domain..].char_indices().find(|(_, c)| matches!(c, ' ' | '\t' | '\r' | '\n')).map_or(s.len(), |(i, _)| end_domain + i);
        let www = proto == "www.";
        // `www(?=\.)`: the dot is the domain's.
        let (prefix, shown_proto, domain) = if www { ("http://", "", &s[at..end_domain]) } else { ("", &s[at..after], &s[after..end_domain]) };
        if end_domain > after - usize::from(www) && previous_ok(s, at, false) && correct_domain(domain) {
            let (head, trail) = split_url(&format!("{domain}{}", &s[end_domain..end]));
            if !head.is_empty() {
                if at > from {
                    out.push(Node::text(&s[from..at]));
                }
                out.push(link_node(format!("{prefix}{shown_proto}{head}"), format!("{shown_proto}{head}")));
                if !trail.is_empty() {
                    out.push(Node::text(trail));
                }
                from = end;
                search = end;
                continue;
            }
        }
        search = at + proto.len();
    }
    if from < s.len() {
        out.push(Node::text(&s[from..]));
    }
    out
}

fn autolink_emails(s: &str) -> Vec<Node> {
    if !s.contains('@') {
        return vec![Node::text(s)];
    }
    let mut out = Vec::new();
    let mut from = 0;
    let mut search = 0;
    while let Some(k) = s[search..].find('@') {
        let at = search + k;
        let is_atext = |c: char| is_ascii_word(c) || matches!(c, '-' | '.' | '+');
        let start = s[from..at].char_indices().rev().take_while(|(_, c)| is_atext(*c)).last().map_or(at, |(i, _)| from + i);
        // `([-\w]+(?:\.[-\w]+)+)`.
        let rest = &s[at + 1..];
        let mut end = 0;
        let mut dots = 0;
        let mut label_len = 0;
        for (i, c) in rest.char_indices() {
            if is_ascii_word(c) || c == '-' {
                label_len += c.len_utf8();
                end = i + c.len_utf8();
            } else if c == '.' && label_len > 0 {
                dots += 1;
                label_len = 0;
            } else {
                break;
            }
        }
        let label = &rest[..end];
        let good = start < at && dots > 0 && label.contains('.') && previous_ok(s, start, true) && !label.ends_with(|c: char| c == '-' || c == '_' || c.is_ascii_digit());
        if good {
            if start > from {
                out.push(Node::text(&s[from..start]));
            }
            let whole = format!("{}@{}", &s[start..at], label);
            out.push(link_node(format!("mailto:{whole}"), whole));
            from = at + 1 + end;
            search = from;
        } else {
            search = at + 1;
        }
    }
    if from < s.len() {
        out.push(Node::text(&s[from..]));
    }
    out
}

/// `remarkEmoji`: `:shortcode:` to the emoji; an unknown one stays as text.
fn emoji(n: Node) -> Vec<Node> {
    let Kind::Text(s) = &n.kind else { return vec![n] };
    if !s.contains(':') {
        return vec![n];
    }
    vec![Node { kind: Kind::Text(super::emoji::emojify(s)), ..n }]
}

fn regex(cell: &'static OnceLock<onig::Regex>, src: &str) -> &'static onig::Regex {
    cell.get_or_init(|| onig::Regex::new(src).expect("a valid pattern"))
}

const SEG: &str = "[A-Za-z0-9_.][A-Za-z0-9_.\\-]{0,99}";

fn issue_re(flavor: Flavor) -> &'static onig::Regex {
    static GH: OnceLock<onig::Regex> = OnceLock::new();
    static GL: OnceLock<onig::Regex> = OnceLock::new();
    match flavor {
        Flavor::Gitlab => regex(&GL, &format!("(?<![A-Za-z0-9_/#&.\\-])(?:({SEG}(?:/{SEG}){{1,20}}))?#([0-9]+)(?![A-Za-z0-9_#])")),
        _ => regex(&GH, &format!("(?<![A-Za-z0-9_/#&.\\-])(?:({SEG}/{SEG}))?#([0-9]+)(?![A-Za-z0-9_#])")),
    }
}

fn mr_re() -> &'static onig::Regex {
    static RE: OnceLock<onig::Regex> = OnceLock::new();
    regex(&RE, &format!("(?<![A-Za-z0-9_/!.\\-])(?:({SEG}(?:/{SEG}){{1,20}}))?!([0-9]+)(?![A-Za-z0-9_!])"))
}

fn sha_re() -> &'static onig::Regex {
    static RE: OnceLock<onig::Regex> = OnceLock::new();
    regex(&RE, "(?<![A-Za-z0-9_@/.#\\-])[0-9a-f]{7,40}(?![A-Za-z0-9_\\-])")
}

fn mention_re() -> &'static onig::Regex {
    static RE: OnceLock<onig::Regex> = OnceLock::new();
    regex(&RE, "(?<![A-Za-z0-9_`@/.+\\-])@([A-Za-z0-9_](?:[A-Za-z0-9_.\\-]*[A-Za-z0-9_])?)(?![A-Za-z0-9_@])")
}

fn replace_refs(list: Vec<Node>, re: &onig::Regex, make: &dyn Fn(&str, &onig::Captures) -> Option<RefNode>) -> Vec<Node> {
    let mut out = Vec::with_capacity(list.len());
    for n in list {
        let Kind::Text(s) = &n.kind else {
            out.push(n);
            continue;
        };
        let mut from = 0;
        let mut hit = false;
        for caps in re.captures_iter(s) {
            let Some((a, b)) = caps.pos(0) else { continue };
            if a < from {
                continue;
            }
            if let Some(r) = make(&s[a..b], &caps) {
                if a > from {
                    out.push(Node::text(&s[from..a]));
                }
                out.push(Node::new(Kind::Reference(r)));
                from = b;
                hit = true;
            }
        }
        if !hit {
            out.push(n);
        } else if from < s.len() {
            out.push(Node::text(&s[from..]));
        }
    }
    out
}

fn issue_or_mr(kind: RefKind) -> impl Fn(&str, &onig::Captures) -> Option<RefNode> {
    move |whole, caps| {
        let n: u64 = caps.at(2)?.parse().ok()?;
        (n > 0 && n <= 9_007_199_254_740_991).then(|| RefNode { kind, project: caps.at(1).filter(|p| !p.is_empty()).map(str::to_string), number: Some(n), sha: None, user: None, value: whole.to_string() })
    }
}

/// `remarkReferences`.
fn references(list: Vec<Node>, flavor: Flavor) -> Vec<Node> {
    let mut list = list;
    let any = |c: char| list.iter().any(|n| matches!(&n.kind, Kind::Text(s) if s.contains(c)));
    let (bang, hash, at) = (any('!'), any('#'), any('@'));
    if flavor == Flavor::Gitlab && bang {
        list = replace_refs(list, mr_re(), &issue_or_mr(RefKind::Mr));
    }
    if hash {
        list = replace_refs(list, issue_re(flavor), &issue_or_mr(RefKind::Issue));
    }
    if at {
        list = replace_refs(list, mention_re(), &|whole, caps| Some(RefNode { kind: RefKind::Mention, project: None, number: None, sha: None, user: caps.at(1).map(str::to_string), value: whole.to_string() }));
    }
    replace_refs(list, sha_re(), &|whole, _| {
        let lettered = whole.bytes().any(|c| (b'a'..=b'f').contains(&c)) && whole.bytes().any(|c| c.is_ascii_digit());
        (lettered || whole.len() == 40).then(|| RefNode { kind: RefKind::Commit, project: None, number: None, sha: Some(whole.to_string()), user: None, value: whole.to_string() })
    })
}

/// `github-slugger`: the slugs of a document, unique.
#[derive(Default)]
pub struct Slugger {
    seen: HashMap<String, u32>,
}

impl Slugger {
    pub fn slug(&mut self, value: &str) -> String {
        let original: String = value.to_lowercase().chars().filter_map(|c| if c == ' ' { Some('-') } else if c.is_alphanumeric() || c == '-' || c == '_' || unicode_is_mark(c) { Some(c) } else { None }).collect();
        let mut result = original.clone();
        while self.seen.contains_key(&result) {
            let n = self.seen.entry(original.clone()).or_insert(0);
            *n += 1;
            result = format!("{original}-{n}");
        }
        self.seen.insert(result.clone(), 0);
        result
    }

    /// An id already taken.
    pub fn take(&mut self, id: &str) {
        self.seen.entry(id.to_string()).or_insert(0);
    }
}

fn heading_ids(nodes: &mut [Node], slugger: &mut Slugger) {
    for n in nodes {
        if matches!(n.kind, Kind::Heading { .. }) {
            let slug = slugger.slug(&n.text_of());
            n.data_mut().id = Some(slug);
        } else {
            heading_ids(&mut n.children, slugger);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn kinds(p: &Parsed) -> Vec<String> {
        p.root.iter().map(|n| format!("{:?}", n.kind).split([' ', '(', '{']).next().unwrap().to_string()).collect()
    }

    fn inline(text: &str, flavor: Flavor) -> Vec<Node> {
        parse(text, flavor, false).root.remove(0).children
    }

    #[test]
    fn the_gfm_blocks_become_their_nodes() {
        let p = parse("# T\n\n- [x] a\n\n| a |\n|---|\n| b |\n\n```rust info\nx\n```\n\n<div>\n\n---\n", Flavor::Github, false);
        assert_eq!(kinds(&p), ["Heading", "List", "Table", "Code", "Html", "ThematicBreak"]);
        assert_eq!(p.root[0].data().unwrap().id.as_deref(), Some("t"));
        assert!(matches!(p.root[1].children[0].kind, Kind::ListItem { checked: Some(true) }));
        assert_eq!(p.root[2].children.len(), 2);
        assert!(matches!(&p.root[3].kind, Kind::Code { lang: Some(l), meta: Some(m), value } if l == "rust" && m == "info" && value == "x"));
    }

    #[test]
    fn a_reference_link_remembers_its_definition() {
        let p = parse("[a][Ref]\n\n[ref]: /x \"T\"\n", Flavor::Github, false);
        assert_eq!(p.links.get("ref"), Some(&("/x".to_string(), Some("T".to_string()))));
        assert!(matches!(&p.root[0].children[0].kind, Kind::Link { url, reference: Some(r), .. } if url == "/x" && r == "ref"));
    }

    #[test]
    fn literal_autolinks_as_gfm_has_them() {
        let urls = |s: &str| inline(s, Flavor::Github).iter().filter_map(|n| if let Kind::Link { url, .. } = &n.kind { Some(url.clone()) } else { None }).collect::<Vec<_>>();
        assert_eq!(urls("see https://example.com/a. And more"), ["https://example.com/a"]);
        assert_eq!(urls("www.example.org!"), ["http://www.example.org"]);
        assert_eq!(urls("mail a.b@example.org now"), ["mailto:a.b@example.org"]);
        assert_eq!(urls("(https://example.com/a_(b))"), ["https://example.com/a_(b)"]);
        assert_eq!(urls("xhttps://example.com"), Vec::<String>::new(), "not after a letter");
        assert_eq!(urls("http://localhost"), Vec::<String>::new(), "a domain has a dot");
        assert_eq!(urls("a@b"), Vec::<String>::new());
        let long = format!("https://example.com/{}", "a".repeat(2000));
        assert_eq!(urls(&long), Vec::<String>::new(), "past the guard");
    }

    #[test]
    fn references_match_the_web_plugins() {
        let refs = |s: &str, f| inline(s, f).iter().filter_map(|n| if let Kind::Reference(r) = &n.kind { Some(r.value.clone()) } else { None }).collect::<Vec<_>>();
        assert_eq!(refs("see #12 and owner/repo#3, a#b", Flavor::Github), ["#12", "owner/repo#3"]);
        assert_eq!(refs("x!5 !5 group/sub/p!9", Flavor::Github), Vec::<String>::new(), "only GitLab has !n");
        assert_eq!(refs("!5 group/sub/p!9", Flavor::Gitlab), ["!5", "group/sub/p!9"]);
        assert_eq!(refs("abc1234def 1234567 deadbeef", Flavor::Github), ["abc1234def"], "a letter and a digit");
        assert_eq!(refs("@someone a@b.c `@no` (@yes)", Flavor::Github), ["@someone", "@yes"]);
        assert!(refs("[#1](http://a.example)", Flavor::Github).is_empty(), "never in a link");
    }

    #[test]
    fn emoji_become_glyphs_and_unknown_ones_stay() {
        assert!(matches!(&inline("a :rocket: b :nonesuch:", Flavor::Github)[0].kind, Kind::Text(t) if t == "a 🚀 b :nonesuch:"));
        assert!(matches!(&inline("`:rocket:`", Flavor::Github)[0].kind, Kind::InlineCode(t) if t == ":rocket:"));
    }

    #[test]
    fn slugs_stay_unique() {
        let p = parse("# A b!\n\n# A b\n\n# A b\n", Flavor::Github, false);
        let ids: Vec<_> = p.root.iter().map(|n| n.data().unwrap().id.clone().unwrap()).collect();
        assert_eq!(ids, ["a-b", "a-b-1", "a-b-2"]);
    }

    #[test]
    fn gitlab_image_attributes_size_an_image() {
        let kids = inline("![a](b.png){width=900 height=\"575px\"} rest", Flavor::Gitlab);
        assert_eq!(kids[0].data().unwrap().width, Some(900));
        assert_eq!(kids[0].data().unwrap().height, Some(575));
        assert!(matches!(&kids[1].kind, Kind::Text(t) if t == " rest"));
        let kids = inline("![a](b.png){width=50%}", Flavor::Gitlab);
        assert!(kids[0].data().is_none());
        assert_eq!(kids.len(), 1);
        assert_eq!(inline("![a](b.png){width=9}", Flavor::Github).len(), 2, "GitLab's only");
    }

    /// Its work on a small stack, as an off-thread read has.
    fn on_small_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new().stack_size(1 << 21).spawn(f).unwrap().join().unwrap();
    }

    fn depth(nodes: &[Node]) -> usize {
        nodes.iter().map(|n| 1 + depth(&n.children)).max().unwrap_or(0)
    }

    #[test]
    fn nesting_is_bounded_however_deep_the_source_goes() {
        on_small_stack(|| {
            for s in [">".repeat(200_000) + " x", "- ".repeat(100_000) + "x", "*a **b ".repeat(20_000), "[".repeat(200_000) + "x", "1. ".repeat(100_000) + "x"] {
                let p = parse(&s, Flavor::Github, false);
                assert!(depth(&p.root) <= MAX_DEPTH + 1, "{}: {}", &s[..8], depth(&p.root));
                assert!(p.too_large || !p.root.is_empty());
            }
        });
    }

    #[test]
    fn one_long_definition_used_many_times_is_not_copied_each_time() {
        let def = "x".repeat(1 << 20);
        let s = format!("{}\n\n[r]: <{def}>\n", "[a][r] ".repeat(100_000));
        let p = parse(&s, Flavor::Github, false);
        fn bytes(nodes: &[Node]) -> usize {
            nodes.iter().map(|n| bytes(&n.children) + if let Kind::Link { url, .. } = &n.kind { url.len() } else { 0 }).sum()
        }
        assert!(bytes(&p.root) <= MAX_REFERENCE_BYTES, "{}", bytes(&p.root));
    }

    #[test]
    fn long_runs_of_what_almost_matches_stay_linear() {
        // Each of these took minutes, or gigabytes, before it was bounded.
        let n = 300_000;
        for s in ["www. ".repeat(n), "http:// ".repeat(n), "a@ ".repeat(n), ": ".repeat(n), "#".repeat(n), "a/".repeat(n) + "#1", "@".repeat(n)] {
            let started = std::time::Instant::now();
            let _ = parse(&s, Flavor::Gitlab, false);
            assert!(started.elapsed().as_secs() < 30, "{}", &s[..8]);
        }
        let s = "![a](b.png){width=1} ".repeat(50_000);
        let started = std::time::Instant::now();
        let _ = parse(&s, Flavor::Gitlab, false);
        assert!(started.elapsed().as_secs() < 30);
    }

    #[test]
    fn a_nodes_lines_are_its_source_lines() {
        let p = parse("para\n\n- a\n- b\n\nend\n", Flavor::Github, false);
        let lines: Vec<_> = p.root.iter().map(|n| (n.pos.unwrap().line_start, n.pos.unwrap().line_end)).collect();
        assert_eq!(lines, [(1, 1), (3, 4), (6, 6)]);
    }
}
