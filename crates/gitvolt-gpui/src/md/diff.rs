//! The rendered diff (`ui/src/markdown/diff/`): two trees lined up block by block into one document the renderer shows
//! like any other. `blocks.ts` lines them up, `words.ts` marks a changed block's words and lines, `diffTree.ts` merges,
//! `split.ts` makes the side-by-side rows. `similar` stands in for jsdiff; its Myers gives the same changes, a tie aside.

use super::ast::{Flavor, Kind, Mark, Node, Parsed, parse};
use similar::{Algorithm, ChangeTag, DiffOp, capture_diff_slices_deadline};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// How far apart, in one gap of removed and added units, two units may still pair.
pub const PAIR_LOOKAHEAD: usize = 8;
pub const PAIR_SIMILARITY: f32 = 0.4;
pub const PAIR_CONTAINMENT: f32 = 0.65;
pub const CONTAIN_MIN_WORDS: usize = 4;
pub const SHORT_WORDS: usize = 3;
/// Longer blocks never word-diff: they show as removed and added.
pub const WORD_DIFF_MAX_CHARS: usize = 10_000;
/// The whole rendered diff gives up after this long: the view falls back to Source.
pub const ALIGN_TIMEOUT: Duration = Duration::from_millis(1500);
const CODE_WORDS_MAX_CHARS: usize = 1000;

/// Past the diff's one deadline.
#[derive(Debug)]
pub struct GaveUp;

pub type Res<T> = Result<T, GaveUp>;

fn left(deadline: Instant) -> Res<Instant> {
    if Instant::now() >= deadline { Err(GaveUp) } else { Ok(deadline) }
}

/// jsdiff's `tokenizeIncludingWhitespace`: word runs, whitespace runs, any other character alone.
fn tokens(s: &str) -> Vec<&str> {
    let is_word = |c: char| matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '\u{AD}' | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2C6}' | '\u{2C8}'..='\u{2D7}' | '\u{2DE}'..='\u{2FF}' | '\u{1E00}'..='\u{1EFF}');
    let mut out = Vec::new();
    let mut start = 0;
    let mut class = 0u8;
    for (i, c) in s.char_indices() {
        let k = if is_word(c) { 1 } else if c.is_whitespace() { 2 } else { 3 };
        if i > start && (k != class || k == 3) {
            out.push(&s[start..i]);
            start = i;
        }
        class = k;
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Change<'a> {
    pub tag: ChangeTag,
    pub value: std::borrow::Cow<'a, str>,
}

/// `diffWordsWithSpace`.
pub fn diff_words<'a>(a: &'a str, b: &'a str, deadline: Instant) -> Res<Vec<Change<'a>>> {
    left(deadline)?;
    let (ta, tb) = (tokens(a), tokens(b));
    let ops = capture_diff_slices_deadline(Algorithm::Myers, &ta, &tb, Some(deadline));
    if Instant::now() >= deadline {
        return Err(GaveUp);
    }
    let mut out: Vec<Change<'a>> = Vec::with_capacity(ops.len());
    let (mut pa, mut pb): (Vec<usize>, Vec<usize>) = (Vec::with_capacity(ta.len() + 1), Vec::with_capacity(tb.len() + 1));
    let mut at = 0;
    for t in &ta {
        pa.push(at);
        at += t.len();
    }
    pa.push(at);
    at = 0;
    for t in &tb {
        pb.push(at);
        at += t.len();
    }
    pb.push(at);
    let mut push = |tag: ChangeTag, s: &'a str| {
        if !s.is_empty() {
            out.push(Change { tag, value: s.into() });
        }
    };
    for op in &ops {
        match *op {
            DiffOp::Equal { new_index, len, .. } => push(ChangeTag::Equal, &b[pb[new_index]..pb[new_index + len]]),
            DiffOp::Delete { old_index, old_len, .. } => push(ChangeTag::Delete, &a[pa[old_index]..pa[old_index + old_len]]),
            DiffOp::Insert { new_index, new_len, .. } => push(ChangeTag::Insert, &b[pb[new_index]..pb[new_index + new_len]]),
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                push(ChangeTag::Delete, &a[pa[old_index]..pa[old_index + old_len]]);
                push(ChangeTag::Insert, &b[pb[new_index]..pb[new_index + new_len]]);
            }
        }
    }
    Ok(out)
}

/// The share of the longer text both have in common, word by word; 0 past `WORD_DIFF_MAX_CHARS`.
pub fn similarity(a: &str, b: &str, deadline: Instant) -> Res<f32> {
    let max = a.len().max(b.len());
    if max == 0 {
        return Ok(1.);
    }
    if max > WORD_DIFF_MAX_CHARS {
        return Ok(0.);
    }
    let common: usize = diff_words(a, b, deadline)?.iter().filter(|c| c.tag == ChangeTag::Equal).map(|c| c.value.len()).sum();
    Ok(common as f32 / max as f32)
}

/// The share of the shorter text's words the longer one has, in the same order.
pub fn containment(a: &str, b: &str, deadline: Instant) -> Res<f32> {
    if a.len().max(b.len()) > WORD_DIFF_MAX_CHARS {
        return Ok(0.);
    }
    let (wa, wb): (Vec<&str>, Vec<&str>) = (a.split_whitespace().collect(), b.split_whitespace().collect());
    let shorter = wa.len().min(wb.len());
    if shorter < CONTAIN_MIN_WORDS {
        return Ok(0.);
    }
    left(deadline)?;
    let ops = capture_diff_slices_deadline(Algorithm::Myers, &wa, &wb, Some(deadline));
    let common: usize = ops.iter().map(|o| if let DiffOp::Equal { len, .. } = o { *len } else { 0 }).sum();
    Ok(common as f32 / shorter as f32)
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// CRLF, or a lone CR, as LF: a file's line endings never show as a change.
fn lf(s: &str) -> String {
    if s.contains('\r') { s.replace("\r\n", "\n").replace('\r', "\n") } else { s.to_string() }
}

/// `blockKey`: structure and text, without positions, heading ids, list tightness or whitespace that doesn't render.
fn canon(n: &Node, out: &mut String) {
    use std::fmt::Write;
    match &n.kind {
        Kind::Text(s) => {
            out.push_str("T:");
            out.push_str(&squash_keep_edges(s));
        }
        Kind::Html(s) => {
            out.push_str("H:");
            out.push_str(&squash(s));
        }
        Kind::Code { lang, meta, value } => {
            let _ = write!(out, "C:{lang:?}:{meta:?}:");
            for l in lf(value).trim_end_matches('\n').split('\n') {
                out.push_str(l.trim_end_matches([' ', '\t']));
                out.push('\n');
            }
        }
        Kind::Link { url, title, reference } => match reference {
            Some(r) => {
                let _ = write!(out, "LR:{r}");
            }
            None => {
                let _ = write!(out, "L:{url}:{title:?}");
            }
        },
        Kind::Image { url, title, alt, reference } => match reference {
            Some(r) => {
                let _ = write!(out, "IR:{r}:{alt}");
            }
            None => {
                let _ = write!(out, "I:{url}:{title:?}:{alt}");
            }
        },
        other => {
            let _ = write!(out, "{other:?}");
        }
    }
    if !n.children.is_empty() {
        out.push('[');
        for c in &n.children {
            canon(c, out);
            out.push(',');
        }
        out.push(']');
    }
}

fn squash_keep_edges(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    out
}

pub fn block_key(nodes: &[Node]) -> String {
    let mut out = String::new();
    for n in nodes {
        canon(n, &mut out);
        out.push(';');
    }
    out
}

fn is_block_parent(k: &Kind) -> bool {
    matches!(k, Kind::Root | Kind::Blockquote | Kind::List { .. } | Kind::ListItem { .. } | Kind::Table { .. } | Kind::TableRow | Kind::FootnoteDefinition { .. } | Kind::DiffBlock { .. } | Kind::DiffPair)
}

/// A node's text: blocks joined by newlines, inline content run together, images by their alt.
pub fn plain_text(n: &Node) -> String {
    match &n.kind {
        Kind::Text(s) | Kind::InlineCode(s) | Kind::Html(s) => s.clone(),
        Kind::Code { value, .. } => value.clone(),
        Kind::Reference(r) => r.value.clone(),
        Kind::Image { alt, .. } => alt.clone(),
        k => n.children.iter().map(plain_text).collect::<Vec<_>>().join(if is_block_parent(k) { "\n" } else { "" }),
    }
}

/// One alignable unit: one block, or the run of blocks an HTML block holds open (`<details>` … `</details>`).
#[derive(Clone)]
pub struct Unit<'a> {
    nodes: Vec<&'a Node>,
    kind: String,
    key: String,
    text: String,
}

enum Op<'a> {
    Same(Unit<'a>, Unit<'a>),
    Removed(Unit<'a>),
    Added(Unit<'a>),
    Changed(Unit<'a>, Unit<'a>),
}

/// `htmlDepth`: the net count of block-level tags a raw HTML node leaves open.
pub fn html_depth(value: &str) -> i32 {
    const TAGS: [&str; 10] = ["details", "div", "table", "blockquote", "section", "picture", "ul", "ol", "dl", "p"];
    let b = value.as_bytes();
    let mut d = 0;
    let mut i = 0;
    while let Some(k) = value[i..].find('<') {
        let at = i + k;
        i = at + 1;
        let closing = b.get(at + 1) == Some(&b'/');
        let from = at + 1 + usize::from(closing);
        let end = value[from..].find(|c: char| !c.is_ascii_alphanumeric()).map_or(value.len(), |e| from + e);
        let name = value[from..end].to_ascii_lowercase();
        if !TAGS.contains(&name.as_str()) {
            continue;
        }
        let Some(close) = value[end..].find('>') else { continue };
        let self_closed = value[..end + close].ends_with('/');
        if !self_closed {
            d += if closing { -1 } else { 1 };
        }
    }
    d
}

fn kind_of(n: &Node) -> String {
    match &n.kind {
        Kind::Code { lang, .. } if lang.as_deref() == Some("mermaid") => "mermaid".into(),
        Kind::Code { lang, .. } => format!("code:{}", lang.as_deref().unwrap_or("")),
        Kind::List { ordered, .. } => if *ordered { "ol" } else { "ul" }.into(),
        Kind::Table { align } => {
            if n.data().is_some_and(|d| d.frontmatter.is_some()) { "frontmatter".into() } else { format!("table:{}", align.len()) }
        }
        Kind::Paragraph => "paragraph".into(),
        Kind::Heading { .. } => "heading".into(),
        Kind::Blockquote => "blockquote".into(),
        Kind::ListItem { .. } => "listItem".into(),
        Kind::TableRow => "tableRow".into(),
        Kind::Html(_) => "html".into(),
        Kind::ThematicBreak => "thematicBreak".into(),
        _ => "other".into(),
    }
}

fn unit_of<'a>(nodes: Vec<&'a Node>, kind: impl Into<String>) -> Unit<'a> {
    let mut key = String::new();
    for n in &nodes {
        canon(n, &mut key);
        key.push(';');
    }
    let text = nodes.iter().map(|n| plain_text(n)).collect::<Vec<_>>().join("\n");
    Unit { nodes, kind: kind.into(), key, text }
}

fn flow_units<'a>(children: &'a [Node]) -> Vec<Unit<'a>> {
    let mut units = Vec::new();
    let mut run: Option<Vec<&'a Node>> = None;
    let mut depth = 0;
    for n in children {
        if matches!(n.kind, Kind::FootnoteDefinition { .. }) {
            continue;
        }
        if let Some(r) = run.as_mut() {
            r.push(n);
            if let Kind::Html(v) = &n.kind {
                depth = (depth + html_depth(v)).max(0);
            }
            if depth == 0 {
                units.push(unit_of(run.take().unwrap_or_default(), "html"));
            }
            continue;
        }
        if let Kind::Html(v) = &n.kind
            && html_depth(v) > 0
        {
            run = Some(vec![n]);
            depth = html_depth(v);
            continue;
        }
        units.push(unit_of(vec![n], kind_of(n)));
    }
    if let Some(r) = run {
        units.push(unit_of(r, "html"));
    }
    units
}

fn words_of(s: &str) -> Vec<String> {
    let lower = s.to_lowercase();
    lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

fn level(u: &Unit) -> u8 {
    if let Kind::Heading { depth } = u.nodes[0].kind { depth } else { 0 }
}

fn short(a: &Unit, b: &Unit) -> bool {
    if level(a) != level(b) {
        return false;
    }
    let (x, y) = (words_of(&a.text).len(), words_of(&b.text).len());
    (if a.kind == "heading" { x.min(y) } else { x.max(y) }) <= SHORT_WORDS
}

fn loosely_alike(a: &str, b: &str) -> bool {
    let theirs: HashSet<String> = words_of(b).into_iter().collect();
    if words_of(a).iter().any(|w| theirs.contains(w)) {
        return true;
    }
    let (x, y) = (squash(a).to_lowercase(), squash(b).to_lowercase());
    !x.is_empty() && !y.is_empty() && (x.starts_with(&y) || y.starts_with(&x))
}

fn can_pair(a: &Unit, b: &Unit, only: bool, deadline: Instant) -> Res<bool> {
    if a.kind != b.kind || a.nodes.len() != 1 || b.nodes.len() != 1 {
        return Ok(false);
    }
    if matches!(a.kind.as_str(), "ul" | "ol" | "blockquote" | "mermaid" | "frontmatter") || a.kind.starts_with("code:") || a.kind.starts_with("table:") {
        return Ok(true);
    }
    if !matches!(a.kind.as_str(), "paragraph" | "heading" | "listItem" | "tableRow") {
        return Ok(false);
    }
    if only && level(a) == level(b) && (a.kind == "heading" || short(a, b)) {
        return Ok(true);
    }
    if short(a, b) && loosely_alike(&a.text, &b.text) {
        return Ok(true);
    }
    Ok(similarity(&a.text, &b.text, deadline)? >= PAIR_SIMILARITY || containment(&a.text, &b.text, deadline)? >= PAIR_CONTAINMENT)
}

/// One gap's removed and added units paired, in order.
fn pair_gap<'a>(old: Vec<Unit<'a>>, neu: Vec<Unit<'a>>, deadline: Instant, out: &mut Vec<Op<'a>>) -> Res<()> {
    let only = old.len() == 1 && neu.len() == 1;
    let (mut oi, mut nj) = (0, 0);
    for r in 0..old.len() {
        let mut k = None;
        for (c, candidate) in neu.iter().enumerate().take(nj + PAIR_LOOKAHEAD).skip(nj) {
            if can_pair(&old[r], candidate, only, deadline)? {
                k = Some(c);
                break;
            }
        }
        let Some(k) = k else { continue };
        while oi < r {
            out.push(Op::Removed(old[oi].clone()));
            oi += 1;
        }
        while nj < k {
            out.push(Op::Added(neu[nj].clone()));
            nj += 1;
        }
        out.push(Op::Changed(old[r].clone(), neu[k].clone()));
        oi = r + 1;
        nj = k + 1;
    }
    while oi < old.len() {
        out.push(Op::Removed(old[oi].clone()));
        oi += 1;
    }
    while nj < neu.len() {
        out.push(Op::Added(neu[nj].clone()));
        nj += 1;
    }
    Ok(())
}

/// Two flows lined up: Myers over the units' keys, each gap's units paired.
fn align<'a>(old: Vec<Unit<'a>>, neu: Vec<Unit<'a>>, c: &Ctx) -> Res<Vec<Op<'a>>> {
    left(c.deadline)?;
    let (ka, kb): (Vec<&str>, Vec<&str>) = (old.iter().map(|u| u.key.as_str()).collect(), neu.iter().map(|u| u.key.as_str()).collect());
    let diff = capture_diff_slices_deadline(Algorithm::Myers, &ka, &kb, Some(c.deadline));
    left(c.deadline)?;
    let mut ops = Vec::with_capacity(old.len().max(neu.len()));
    let (mut gap_old, mut gap_new): (Vec<Unit>, Vec<Unit>) = (Vec::new(), Vec::new());
    for op in diff {
        match op {
            DiffOp::Equal { old_index, new_index, len } => {
                pair_gap(std::mem::take(&mut gap_old), std::mem::take(&mut gap_new), c.deadline, &mut ops)?;
                for k in 0..len {
                    ops.push(Op::Same(old[old_index + k].clone(), neu[new_index + k].clone()));
                }
            }
            DiffOp::Delete { old_index, old_len, .. } => gap_old.extend_from_slice(&old[old_index..old_index + old_len]),
            DiffOp::Insert { new_index, new_len, .. } => gap_new.extend_from_slice(&neu[new_index..new_index + new_len]),
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                gap_old.extend_from_slice(&old[old_index..old_index + old_len]);
                gap_new.extend_from_slice(&neu[new_index..new_index + new_len]);
            }
        }
    }
    pair_gap(gap_old, gap_new, c.deadline, &mut ops)?;
    // A kept unit that uses a changed link definition is changed: its links show the change.
    if !c.refs.is_empty() {
        for op in &mut ops {
            if let Op::Same(a, b) = op
                && b.nodes.iter().any(|n| uses_changed_ref(n, c))
            {
                *op = Op::Changed(a.clone(), b.clone());
            }
        }
    }
    Ok(ops)
}

/// `refs`: the old side's link definitions whose URL or title changed, by label.
struct Ctx {
    changes: usize,
    deadline: Instant,
    refs: HashMap<String, (String, Option<String>)>,
}

fn reference_of(n: &Node) -> Option<&str> {
    match &n.kind {
        Kind::Link { reference: Some(r), .. } | Kind::Image { reference: Some(r), .. } => Some(r),
        _ => None,
    }
}

fn is_changed_ref(n: &Node, c: &Ctx) -> bool {
    reference_of(n).is_some_and(|r| c.refs.contains_key(r))
}

fn uses_changed_ref(n: &Node, c: &Ctx) -> bool {
    is_changed_ref(n, c) || n.children.iter().any(|k| uses_changed_ref(k, c))
}

/// A reference to a changed definition as the old side resolved it.
fn resolved_old(n: &Node, c: &Ctx) -> Node {
    let Some((url, title)) = reference_of(n).and_then(|r| c.refs.get(r)) else { return n.clone() };
    let mut out = n.clone();
    match &mut out.kind {
        Kind::Link { url: u, title: t, reference } | Kind::Image { url: u, title: t, reference, .. } => {
            *u = url.clone();
            *t = title.clone();
            *reference = None;
        }
        _ => {}
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum RefSide {
    Both,
    Old,
    New,
}

fn mark_refs(nodes: Vec<Node>, c: &Ctx, side: RefSide) -> Vec<Node> {
    if c.refs.is_empty() {
        return nodes;
    }
    nodes
        .into_iter()
        .flat_map(|mut n| match n.kind {
            Kind::DiffDel => {
                n.children = mark_refs(std::mem::take(&mut n.children), c, RefSide::Old);
                vec![n]
            }
            Kind::DiffIns => {
                n.children = mark_refs(std::mem::take(&mut n.children), c, RefSide::New);
                vec![n]
            }
            _ if is_changed_ref(&n, c) => match side {
                RefSide::Old => vec![resolved_old(&n, c)],
                RefSide::New => vec![n],
                RefSide::Both => vec![Node::with(Kind::DiffDel, vec![resolved_old(&n, c)]), Node::with(Kind::DiffIns, vec![n])],
            },
            _ => {
                n.children = mark_refs(std::mem::take(&mut n.children), c, side);
                vec![n]
            }
        })
        .collect()
}

/// A copy of a removed node as the old side shows it: without heading ids, its changed references as they were.
fn old_side(n: &Node, c: &Ctx) -> Node {
    let mut copy = if is_changed_ref(n, c) { resolved_old(n, c) } else { n.clone() };
    if matches!(copy.kind, Kind::Heading { .. })
        && let Some(d) = copy.data.as_mut()
    {
        d.id = None;
    }
    copy.children = n.children.iter().map(|k| old_side(k, c)).collect();
    copy
}

fn block(mark: Mark, nodes: Vec<Node>, note: Option<String>) -> Node {
    Node::with(Kind::DiffBlock { mark, note }, nodes)
}

fn removed_block(nodes: &[&Node], c: &Ctx) -> Node {
    block(Mark::Removed, nodes.iter().map(|n| old_side(n, c)).collect(), None)
}

fn added_block(nodes: &[&Node]) -> Node {
    block(Mark::Added, nodes.iter().map(|n| (*n).clone()).collect(), None)
}

// --- words.ts ---

const PUA_START: u32 = 0xE000;
const PUA_SIZE: usize = 0x1900;

fn is_atom(n: &Node) -> bool {
    matches!(n.kind, Kind::InlineCode(_) | Kind::Image { .. } | Kind::Break | Kind::FootnoteReference { .. } | Kind::Reference(_) | Kind::Html(_)) || (n.children.is_empty() && !matches!(n.kind, Kind::Text(_) | Kind::Emphasis | Kind::Strong | Kind::Delete | Kind::Link { .. }))
}

const VOID: [&str; 13] = ["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"];

/// The net count of elements an inline HTML node leaves open.
fn tag_depth(value: &str) -> i32 {
    let mut d = 0;
    let mut i = 0;
    while let Some(k) = value[i..].find('<') {
        let at = i + k;
        i = at + 1;
        let closing = value.as_bytes().get(at + 1) == Some(&b'/');
        let from = at + 1 + usize::from(closing);
        if !value.as_bytes().get(from).is_some_and(u8::is_ascii_alphabetic) {
            continue;
        }
        let end = value[from..].find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_')).map_or(value.len(), |e| from + e);
        let name = value[from..end].to_ascii_lowercase();
        let Some(close) = value[end..].find('>') else { continue };
        if !value[..end + close].ends_with('/') && !VOID.contains(&name.as_str()) {
            d += if closing { -1 } else { 1 };
        }
    }
    d
}

/// A token of a word diff: a node, or a run of inline HTML from an opening tag to the one that closes it.
enum InlineItem<'a> {
    One(&'a Node),
    Run(&'a [Node]),
}

fn inline_items(list: &[Node]) -> Vec<InlineItem<'_>> {
    let mut out = Vec::with_capacity(list.len());
    let mut i = 0;
    while i < list.len() {
        let mut end = i;
        if let Kind::Html(v) = &list[i].kind
            && tag_depth(v) > 0
        {
            let mut depth = tag_depth(v);
            for (j, m) in list.iter().enumerate().skip(i + 1) {
                if let Kind::Html(w) = &m.kind {
                    depth += tag_depth(w);
                }
                if depth <= 0 {
                    end = j;
                    break;
                }
            }
        }
        out.push(if end > i { InlineItem::Run(&list[i..=end]) } else { InlineItem::One(&list[i]) });
        i = end + 1;
    }
    out
}

fn atom_key(nodes: &[&Node]) -> String {
    let mut s = String::new();
    for n in nodes {
        canon(n, &mut s);
    }
    s
}

/// The inline content's text, each atom as one private-use character. `None`: it can't be word-diffed.
fn flatten(nodes: &[Node], atoms: &mut HashMap<String, char>, out: &mut String) -> Option<()> {
    for item in inline_items(nodes) {
        match item {
            InlineItem::One(Node { kind: Kind::Text(t), .. }) => {
                if t.chars().any(|c| (0xE000..=0xF8FF).contains(&(c as u32))) {
                    return None;
                }
                out.push_str(t);
            }
            InlineItem::One(n) if !is_atom(n) => flatten(&n.children, atoms, out)?,
            InlineItem::One(n) => out.push(atom_char(&[n], atoms)?),
            InlineItem::Run(run) => {
                let refs: Vec<&Node> = run.iter().collect();
                out.push(atom_char(&refs, atoms)?);
            }
        }
    }
    Some(())
}

fn atom_char(nodes: &[&Node], atoms: &mut HashMap<String, char>) -> Option<char> {
    let key = atom_key(nodes);
    if let Some(c) = atoms.get(&key) {
        return Some(*c);
    }
    if atoms.len() >= PUA_SIZE {
        return None;
    }
    let c = char::from_u32(PUA_START + atoms.len() as u32)?;
    atoms.insert(key, c);
    Some(c)
}

/// The part of `nodes` over flattened byte offsets `[start, end)`.
fn slice_inline(nodes: &[Node], start: usize, end: usize) -> Vec<Node> {
    fn walk(list: &[Node], start: usize, end: usize, pos: &mut usize, into: &mut Vec<Node>) {
        for item in inline_items(list) {
            if *pos >= end {
                break;
            }
            match item {
                InlineItem::One(Node { kind: Kind::Text(t), .. }) => {
                    let (s, e) = (start.max(*pos), end.min(*pos + t.len()));
                    if e > s {
                        into.push(Node::text(&t[s - *pos..e - *pos]));
                    }
                    *pos += t.len();
                }
                InlineItem::One(n) if !is_atom(n) => {
                    let mut kids = Vec::new();
                    walk(&n.children, start, end, pos, &mut kids);
                    if !kids.is_empty() {
                        let mut copy = Node::new(n.kind.clone());
                        copy.data = n.data.clone();
                        copy.children = kids;
                        into.push(copy);
                    }
                }
                InlineItem::One(n) => {
                    if *pos >= start {
                        into.push(n.clone());
                    }
                    *pos += '\u{e000}'.len_utf8();
                }
                InlineItem::Run(run) => {
                    if *pos >= start {
                        into.extend(run.iter().cloned());
                    }
                    *pos += '\u{e000}'.len_utf8();
                }
            }
        }
    }
    let mut into = Vec::new();
    let mut pos = 0;
    walk(nodes, start, end, &mut pos, &mut into);
    into
}

/// A changed block's inline content with its word-level changes. `None`: it can't be word-diffed.
fn inline_diff(old: &[Node], neu: &[Node], deadline: Instant) -> Res<Option<Vec<Node>>> {
    let mut atoms = HashMap::new();
    let (mut a, mut b) = (String::new(), String::new());
    if flatten(old, &mut atoms, &mut a).is_none() || flatten(neu, &mut atoms, &mut b).is_none() || a.len().max(b.len()) > WORD_DIFF_MAX_CHARS {
        return Ok(None);
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    for c in diff_words(&a, &b, deadline)? {
        let n = c.value.len();
        let blank = c.value.trim().is_empty();
        match c.tag {
            ChangeTag::Delete => {
                if !blank {
                    out.push(Node::with(Kind::DiffDel, slice_inline(old, i, i + n)));
                }
                i += n;
            }
            ChangeTag::Insert => {
                if blank {
                    out.extend(slice_inline(neu, j, j + n));
                } else {
                    out.push(Node::with(Kind::DiffIns, slice_inline(neu, j, j + n)));
                }
                j += n;
            }
            ChangeTag::Equal => {
                out.extend(slice_inline(neu, j, j + n));
                i += n;
                j += n;
            }
        }
    }
    Ok(Some(out))
}

fn pair_lines(old: &[&str], neu: &[&str], deadline: Instant) -> Res<Vec<(usize, usize)>> {
    if old.len() == neu.len() {
        return Ok((0..old.len()).map(|i| (i, i)).collect());
    }
    let mut pairs = Vec::new();
    let mut next = 0;
    for (i, o) in old.iter().enumerate() {
        if next >= neu.len() {
            break;
        }
        for (j, n) in neu.iter().enumerate().skip(next) {
            if similarity(o, n, deadline)? >= PAIR_SIMILARITY {
                pairs.push((i, j));
                next = j + 1;
                break;
            }
        }
    }
    Ok(pairs)
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// The changed words of a paired removed and added line, as `start-end` UTF-16 ranges on each.
fn line_words(a: &str, b: &str, deadline: Instant) -> Res<Option<(String, String)>> {
    if a.len().max(b.len()) > CODE_WORDS_MAX_CHARS {
        return Ok(None);
    }
    let (mut old, mut neu): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    let mut common = false;
    for c in diff_words(a, b, deadline)? {
        let n = utf16_len(&c.value);
        let blank = c.value.trim().is_empty();
        match c.tag {
            ChangeTag::Delete => {
                if !blank {
                    old.push(format!("{i}-{}", i + n));
                }
                i += n;
            }
            ChangeTag::Insert => {
                if !blank {
                    neu.push(format!("{j}-{}", j + n));
                }
                j += n;
            }
            ChangeTag::Equal => {
                common |= !blank;
                i += n;
                j += n;
            }
        }
    }
    Ok(common.then(|| (old.join(","), neu.join(","))))
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct CodeLines {
    pub value: String,
    pub marks: String,
    pub words: Option<String>,
}

fn code_lines_at(old: &str, neu: &str, deadline: Instant) -> Res<CodeLines> {
    let ended = |s: &str| if s.is_empty() { String::new() } else { format!("{}\n", lf(s)) };
    let (a, b) = (ended(old), ended(neu));
    let (la, lb): (Vec<&str>, Vec<&str>) = (a.split_inclusive('\n').collect(), b.split_inclusive('\n').collect());
    let ops = capture_diff_slices_deadline(Algorithm::Myers, &la, &lb, Some(deadline));
    left(deadline)?;
    let mut marks = String::new();
    let mut value: Vec<String> = Vec::new();
    for op in ops {
        let mut add = |list: &[&str], m: char| {
            for l in list {
                value.push(l.strip_suffix('\n').unwrap_or(l).to_string());
                marks.push(m);
            }
        };
        match op {
            DiffOp::Equal { new_index, len, .. } => add(&lb[new_index..new_index + len], ' '),
            DiffOp::Delete { old_index, old_len, .. } => add(&la[old_index..old_index + old_len], '-'),
            DiffOp::Insert { new_index, new_len, .. } => add(&lb[new_index..new_index + new_len], '+'),
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                add(&la[old_index..old_index + old_len], '-');
                add(&lb[new_index..new_index + new_len], '+');
            }
        }
    }
    let mut words = vec![String::new(); value.len()];
    let mut any = false;
    let m = marks.as_bytes();
    let mut i = 0;
    while i < m.len() {
        if m[i] != b'-' {
            i += 1;
            continue;
        }
        let r0 = i;
        while i < m.len() && m[i] == b'-' {
            i += 1;
        }
        let a0 = i;
        while i < m.len() && m[i] == b'+' {
            i += 1;
        }
        let a1 = i;
        if a1 == a0 {
            continue;
        }
        let (ro, ad): (Vec<&str>, Vec<&str>) = (value[r0..a0].iter().map(String::as_str).collect(), value[a0..a1].iter().map(String::as_str).collect());
        for (x, y) in pair_lines(&ro, &ad, deadline)? {
            if let Some((w_old, w_new)) = line_words(ro[x], ad[y], deadline)? {
                any |= !w_old.is_empty() || !w_new.is_empty();
                words[r0 + x] = w_old;
                words[a0 + y] = w_new;
            }
        }
    }
    Ok(CodeLines { value: value.join("\n"), marks, words: any.then(|| words.join(";")) })
}

/// A changed code block's lines, old and new merged in order, with one mark per line. `None`: past `deadline`.
pub fn code_lines(old: &str, neu: &str, deadline: Option<Instant>) -> Option<CodeLines> {
    code_lines_at(old, neu, deadline.unwrap_or_else(|| Instant::now() + ALIGN_TIMEOUT)).ok()
}

// --- diffTree.ts ---

fn diff_flow(old: &[Node], neu: &[Node], c: &mut Ctx) -> Res<Vec<Node>> {
    let ops = align(flow_units(old), flow_units(neu), c)?;
    let mut out = Vec::new();
    for op in ops {
        match op {
            Op::Same(_, n) => out.extend(n.nodes.iter().map(|x| (*x).clone())),
            Op::Added(n) => {
                c.changes += 1;
                out.push(added_block(&n.nodes));
            }
            Op::Removed(o) => {
                c.changes += 1;
                out.push(removed_block(&o.nodes, c));
            }
            Op::Changed(o, n) => {
                if o.nodes.len() != 1 || n.nodes.len() != 1 {
                    c.changes += 2;
                    out.push(removed_block(&o.nodes, c));
                    out.push(added_block(&n.nodes));
                } else {
                    out.extend(changed(o.nodes[0], n.nodes[0], c)?);
                }
            }
        }
    }
    Ok(out)
}

fn replaced(o: &Node, n: &Node, c: &mut Ctx) -> Vec<Node> {
    c.changes += 2;
    vec![removed_block(&[o], c), added_block(&[n])]
}

fn changed(o: &Node, n: &Node, c: &mut Ctx) -> Res<Vec<Node>> {
    match &n.kind {
        Kind::Paragraph | Kind::Heading { .. } => {
            let Some(merged) = inline_diff(&o.children, &n.children, c.deadline)? else { return Ok(replaced(o, n, c)) };
            c.changes += 1;
            let mut copy = n.clone();
            copy.children = mark_refs(merged, c, RefSide::Both);
            Ok(vec![block(Mark::Changed, vec![copy], None)])
        }
        Kind::Code { lang, value, meta } => {
            let Kind::Code { lang: was_lang, value: was_value, meta: was_meta } = &o.kind else { return Ok(replaced(o, n, c)) };
            let mermaid = lang.as_deref() == Some("mermaid");
            if !mermaid && was_value.len().max(value.len()) > WORD_DIFF_MAX_CHARS {
                return Ok(replaced(o, n, c));
            }
            c.changes += 1;
            if mermaid {
                return Ok(vec![Node::with(Kind::DiffPair, vec![block(Mark::Removed, vec![o.clone()], None), block(Mark::Added, vec![n.clone()], None)])]);
            }
            let m = code_lines_at(was_value, value, c.deadline)?;
            let note = if m.marks.contains(['+', '-']) {
                None
            } else if was_lang != lang {
                Some(format!("Language changed: {} → {}", was_lang.as_deref().unwrap_or("none"), lang.as_deref().unwrap_or("none")))
            } else {
                Some(format!("Code block info changed: {} → {}", was_meta.as_deref().unwrap_or("none"), meta.as_deref().unwrap_or("none")))
            };
            let mut copy = n.clone();
            copy.kind = Kind::Code { lang: lang.clone(), meta: meta.clone(), value: m.value };
            let d = copy.data_mut();
            d.lines = Some(m.marks);
            d.words = m.words;
            Ok(vec![block(Mark::Changed, vec![copy], note)])
        }
        Kind::Blockquote => {
            let mut copy = n.clone();
            copy.children = diff_flow(&o.children, &n.children, c)?;
            Ok(vec![copy])
        }
        Kind::List { ordered, start } => {
            let before = c.changes;
            let list = diff_list(o, n, c)?;
            let was_start = if let Kind::List { start, .. } = o.kind { start } else { 1 };
            if *ordered && was_start != *start {
                return Ok(noted(list, "Start number changed", before, c));
            }
            Ok(vec![list])
        }
        Kind::Table { .. } if n.data().is_some_and(|d| d.frontmatter.is_some()) => Ok(vec![diff_frontmatter(o, n, c)?]),
        Kind::Table { align: new_align } => {
            let before = c.changes;
            let Some(t) = diff_table(o, n, c)? else { return Ok(replaced(o, n, c)) };
            let was = if let Kind::Table { align } = &o.kind { align.clone() } else { Vec::new() };
            if was != *new_align {
                return Ok(noted(t, "Alignment changed", before, c));
            }
            Ok(vec![t])
        }
        _ => Ok(replaced(o, n, c)),
    }
}

fn noted(node: Node, note: &str, before: usize, c: &mut Ctx) -> Vec<Node> {
    if c.changes != before {
        return vec![node];
    }
    c.changes += 1;
    vec![block(Mark::Changed, vec![node], Some(note.into()))]
}

fn set_item(it: &mut Node, diff: Option<Mark>, value: Option<u64>, was: Option<(Option<u64>, Option<bool>, Option<bool>)>) {
    if let Some(m) = diff {
        it.data_mut().diff = Some(m);
    }
    if let Some(v) = value {
        it.data_mut().value = Some(v);
    }
    if let Some((old_value, was_checked, now_checked)) = was {
        if old_value.is_some() && old_value != value {
            it.data_mut().old_value = old_value;
        }
        if was_checked != now_checked {
            it.data_mut().old_checked = Some(was_checked);
        }
    }
}

fn checked_of(n: &Node) -> Option<bool> {
    if let Kind::ListItem { checked } = n.kind { checked } else { None }
}

/// Two lists' items lined up in the new list.
fn diff_list(o: &Node, n: &Node, c: &mut Ctx) -> Res<Node> {
    let (ordered, new_start) = if let Kind::List { ordered, start } = n.kind { (ordered, start) } else { (false, 1) };
    let old_start = if let Kind::List { start, .. } = o.kind { start } else { 1 };
    let index_of = |list: &Node, it: &Node| list.children.iter().position(|x| std::ptr::eq(x, it)).unwrap_or(0) as u64;
    let num = |list: &Node, start: u64, it: &Node| ordered.then(|| start + index_of(list, it));
    let ops = align(o.children.iter().map(|i| unit_of(vec![i], "listItem")).collect(), n.children.iter().map(|i| unit_of(vec![i], "listItem")).collect(), c)?;
    let mut items = Vec::with_capacity(ops.len());
    for op in ops {
        match op {
            Op::Same(a, b) => {
                let (was, it) = (a.nodes[0], b.nodes[0]);
                let mut copy = it.clone();
                set_item(&mut copy, None, num(n, new_start, it), Some((num(o, old_start, was), checked_of(was), checked_of(it))));
                items.push(copy);
            }
            Op::Added(b) => {
                c.changes += 1;
                let it = b.nodes[0];
                let mut copy = it.clone();
                set_item(&mut copy, Some(Mark::Added), num(n, new_start, it), None);
                items.push(copy);
            }
            Op::Removed(a) => {
                c.changes += 1;
                let it = a.nodes[0];
                let mut copy = old_side(it, c);
                set_item(&mut copy, Some(Mark::Removed), num(o, old_start, it), None);
                items.push(copy);
            }
            Op::Changed(a, b) => {
                let (was, it) = (a.nodes[0], b.nodes[0]);
                let before = c.changes;
                let kids = diff_flow(&was.children, &it.children, c)?;
                // An item whose content is one changed paragraph carries the mark itself, its paragraph inline.
                let only = kids.len() == 1 && matches!(&kids[0].kind, Kind::DiffBlock { mark: Mark::Changed, .. }) && kids[0].children.len() == 1 && matches!(kids[0].children[0].kind, Kind::Paragraph);
                let mut copy = it.clone();
                let meta = Some((num(o, old_start, was), checked_of(was), checked_of(it)));
                if only {
                    copy.children = kids.into_iter().next().map(|k| k.children).unwrap_or_default();
                    set_item(&mut copy, Some(Mark::Changed), num(n, new_start, it), meta);
                } else {
                    let own = c.changes == before && checked_of(was) != checked_of(it);
                    if own {
                        c.changes += 1;
                    }
                    copy.children = kids;
                    set_item(&mut copy, own.then_some(Mark::Changed), num(n, new_start, it), meta);
                }
                items.push(copy);
            }
        }
    }
    let mut out = n.clone();
    out.children = items;
    Ok(out)
}

fn changed_row(a: Option<&Node>, b: &Node, c: &mut Ctx) -> Res<Node> {
    let mut cells = Vec::with_capacity(b.children.len());
    for (k, cell) in b.children.iter().enumerate() {
        let empty = Vec::new();
        let was = a.and_then(|r| r.children.get(k)).map_or(&empty, |x| &x.children);
        let merged = inline_diff(was, &cell.children, c.deadline)?.unwrap_or_else(|| {
            let mut f = Vec::new();
            if !was.is_empty() {
                f.push(Node::with(Kind::DiffDel, was.clone()));
            }
            if !cell.children.is_empty() {
                f.push(Node::with(Kind::DiffIns, cell.children.clone()));
            }
            f
        });
        let mut copy = cell.clone();
        copy.children = mark_refs(merged, c, RefSide::Both);
        cells.push(copy);
    }
    let mut out = b.clone();
    out.children = cells;
    out.data_mut().diff = Some(Mark::Changed);
    Ok(out)
}

fn marked_row(r: &Node, m: Mark) -> Node {
    let mut copy = r.clone();
    copy.data_mut().diff = Some(m);
    copy
}

fn row_units(rows: &[Node]) -> Vec<Unit<'_>> {
    rows.iter().map(|r| unit_of(vec![r], "tableRow")).collect()
}

fn keyed_rows(rows: &[Node]) -> Vec<Unit<'_>> {
    rows.iter()
        .map(|r| {
            let mut u = unit_of(vec![r], "frontmatterRow");
            u.key = r.children.first().map_or(String::new(), plain_text);
            u
        })
        .collect()
}

fn diff_table(o: &Node, n: &Node, c: &mut Ctx) -> Res<Option<Node>> {
    let Some(nh) = n.children.first() else { return Ok(None) };
    let oh = o.children.first();
    let same_head = oh.is_some_and(|h| block_key(std::slice::from_ref(h)) == block_key(std::slice::from_ref(nh)));
    let head = if !same_head || uses_changed_ref(nh, c) {
        c.changes += 1;
        changed_row(oh, nh, c)?
    } else {
        nh.clone()
    };
    let ops = align(row_units(o.children.get(1..).unwrap_or(&[])), row_units(&n.children[1..]), c)?;
    let mut rows = vec![head];
    for op in ops {
        match op {
            Op::Same(_, b) => rows.push(b.nodes[0].clone()),
            Op::Added(b) => {
                c.changes += 1;
                rows.push(marked_row(b.nodes[0], Mark::Added));
            }
            Op::Removed(a) => {
                c.changes += 1;
                rows.push(marked_row(&old_side(a.nodes[0], c), Mark::Removed));
            }
            Op::Changed(a, b) => {
                c.changes += 1;
                rows.push(changed_row(Some(a.nodes[0]), b.nodes[0], c)?);
            }
        }
    }
    let mut out = n.clone();
    out.children = rows;
    Ok(Some(out))
}

fn diff_frontmatter(o: &Node, n: &Node, c: &mut Ctx) -> Res<Node> {
    let mut rows = Vec::new();
    for op in align(keyed_rows(&o.children), keyed_rows(&n.children), c)? {
        match op {
            Op::Added(b) => {
                c.changes += 1;
                rows.push(marked_row(b.nodes[0], Mark::Added));
            }
            Op::Removed(a) => {
                c.changes += 1;
                rows.push(marked_row(&old_side(a.nodes[0], c), Mark::Removed));
            }
            Op::Same(a, b) | Op::Changed(a, b) => {
                if block_key(std::slice::from_ref(a.nodes[0])) == block_key(std::slice::from_ref(b.nodes[0])) {
                    rows.push(b.nodes[0].clone());
                } else {
                    c.changes += 1;
                    rows.push(changed_row(Some(a.nodes[0]), b.nodes[0], c)?);
                }
            }
        }
    }
    let mut out = n.clone();
    out.children = rows;
    Ok(out)
}

fn notes_in<'a>(nodes: &'a [Node], out: &mut HashMap<&'a str, &'a Node>) {
    for n in nodes {
        if let Kind::FootnoteDefinition { id } = &n.kind {
            out.entry(id).or_insert(n);
        }
        notes_in(&n.children, out);
    }
}

fn cited_in(nodes: &[Node], out: &mut HashSet<String>) {
    for n in nodes {
        if let Kind::FootnoteReference { id } = &n.kind {
            out.insert(id.clone());
        }
        cited_in(&n.children, out);
    }
}

/// The merged document's footnotes, diffed by identifier.
fn diff_definitions(old: &Parsed, neu: &Parsed, merged: &[Node], c: &mut Ctx) -> Res<Vec<Node>> {
    let (mut was, mut now) = (HashMap::new(), HashMap::new());
    notes_in(&old.root, &mut was);
    notes_in(&neu.root, &mut now);
    let mut cited = HashSet::new();
    cited_in(merged, &mut cited);
    let mut out = Vec::new();
    // In the new document's order, then what only the old one has.
    let mut ids: Vec<&str> = Vec::new();
    fn order<'a>(nodes: &'a [Node], ids: &mut Vec<&'a str>) {
        for n in nodes {
            if let Kind::FootnoteDefinition { id } = &n.kind
                && !ids.contains(&id.as_str())
            {
                ids.push(id);
            }
            order(&n.children, ids);
        }
    }
    order(&neu.root, &mut ids);
    for id in &ids {
        let Some(n) = now.get(id).copied() else { continue };
        let o = was.get(id).copied();
        if !cited.contains(*id) {
            out.push(n.clone());
        } else if let Some(o) = o {
            let before = c.changes;
            let kids = diff_flow(&o.children, &n.children, c)?;
            let mut copy = n.clone();
            if c.changes != before {
                copy.children = kids;
            }
            out.push(copy);
        } else {
            c.changes += 1;
            let mut copy = n.clone();
            copy.children = vec![added_block(&n.children.iter().collect::<Vec<_>>())];
            out.push(copy);
        }
    }
    let mut old_ids: Vec<&str> = Vec::new();
    order(&old.root, &mut old_ids);
    for id in old_ids {
        if now.contains_key(id) {
            continue;
        }
        let Some(o) = was.get(id).copied() else { continue };
        if !cited.contains(id) {
            out.push(o.clone());
            continue;
        }
        c.changes += 1;
        let mut copy = o.clone();
        copy.children = vec![removed_block(&o.children.iter().collect::<Vec<_>>(), c)];
        out.push(copy);
    }
    Ok(out)
}

pub struct DiffResult {
    pub root: Vec<Node>,
    pub changes: usize,
    pub gave_up: bool,
}

/// The rendered diff's tree.
pub fn diff_trees(old: &Parsed, neu: &Parsed, timeout: Duration) -> DiffResult {
    let deadline = Instant::now() + timeout;
    let mut refs = HashMap::new();
    for (id, was) in &old.links {
        if let Some(now) = neu.links.get(id)
            && now != was
        {
            refs.insert(id.clone(), was.clone());
        }
    }
    let mut c = Ctx { changes: 0, deadline, refs };
    let run = |c: &mut Ctx| -> Res<Vec<Node>> {
        let mut merged = diff_flow(&old.root, &neu.root, c)?;
        let notes = diff_definitions(old, neu, &merged, c)?;
        merged.extend(notes);
        Ok(merged)
    };
    match run(&mut c) {
        Ok(root) => DiffResult { root, changes: c.changes, gave_up: false },
        Err(GaveUp) => DiffResult { root: Vec::new(), changes: 0, gave_up: true },
    }
}

/// Both texts parsed, with their front matter (they're files), and diffed.
pub fn diff_markdown(old: &str, neu: &str, flavor: Flavor) -> DiffResult {
    let (a, b) = (parse(old, flavor, true), parse(neu, flavor, true));
    if a.too_large || b.too_large {
        return DiffResult { root: Vec::new(), changes: 0, gave_up: true };
    }
    diff_trees(&a, &b, ALIGN_TIMEOUT)
}

// --- split.ts ---

#[derive(Clone, Copy, PartialEq)]
enum Side {
    Old,
    New,
}

fn has_marks(n: &Node) -> bool {
    matches!(n.kind, Kind::DiffBlock { .. } | Kind::DiffPair | Kind::DiffIns | Kind::DiffDel) || (matches!(n.kind, Kind::ListItem { .. } | Kind::TableRow) && n.data().is_some_and(|d| d.diff.is_some())) || n.children.iter().any(has_marks)
}

fn kids_of(n: &Node, side: Side) -> Vec<Node> {
    n.children.iter().flat_map(|c| sided(c, side)).collect()
}

fn with_kids(n: &Node, side: Side) -> Node {
    let mut copy = Node::new(n.kind.clone());
    copy.pos = n.pos;
    copy.data = n.data.clone();
    copy.children = kids_of(n, side);
    copy
}

/// A merged node as one side shows it.
fn sided(n: &Node, side: Side) -> Vec<Node> {
    let other = if side == Side::Old { Mark::Added } else { Mark::Removed };
    match &n.kind {
        Kind::DiffBlock { mark, .. } => {
            if *mark == other {
                Vec::new()
            } else {
                vec![with_kids(n, side)]
            }
        }
        Kind::DiffPair => n.children.get(usize::from(side == Side::New)).map_or(Vec::new(), |h| sided(h, side)),
        Kind::DiffIns if side == Side::Old => Vec::new(),
        Kind::DiffDel if side == Side::New => Vec::new(),
        Kind::ListItem { .. } => {
            if n.data().and_then(|d| d.diff) == Some(other) {
                return Vec::new();
            }
            let mut item = with_kids(n, side);
            if side == Side::Old
                && let Some(d) = n.data().cloned()
            {
                if let Some(checked) = d.old_checked {
                    item.kind = Kind::ListItem { checked };
                }
                if let Some(v) = d.old_value {
                    item.data_mut().value = Some(v);
                }
            }
            vec![item]
        }
        Kind::TableRow => {
            if n.data().and_then(|d| d.diff) == Some(other) {
                let mut row = Node::with(Kind::TableRow, n.children.iter().map(|_| Node::with(Kind::TableCell, vec![Node::text(" ")])).collect());
                row.data_mut().empty = true;
                vec![row]
            } else {
                vec![with_kids(n, side)]
            }
        }
        Kind::Code { lang, meta, value } => {
            let Some(marks) = n.data().and_then(|d| d.lines.clone()) else { return vec![n.clone()] };
            let drop = if side == Side::Old { b'+' } else { b'-' };
            let keep: Vec<bool> = value.split('\n').enumerate().map(|(i, _)| marks.as_bytes().get(i) != Some(&drop)).collect();
            let lines: Vec<&str> = value.split('\n').zip(&keep).filter(|(_, k)| **k).map(|(l, _)| l).collect();
            let words = n.data().and_then(|d| d.words.as_ref()).map(|w| w.split(';').zip(&keep).filter(|(_, k)| **k).map(|(x, _)| x).collect::<Vec<_>>().join(";"));
            let mut code = n.clone();
            code.kind = Kind::Code { lang: lang.clone(), meta: meta.clone(), value: lines.join("\n") };
            let d = code.data_mut();
            d.lines = Some(marks.bytes().zip(&keep).filter(|(_, k)| **k).map(|(m, _)| m as char).collect());
            d.words = words;
            vec![code]
        }
        Kind::Heading { .. } => {
            let mut h = with_kids(n, side);
            if side == Side::Old
                && let Some(d) = h.data.as_mut()
            {
                d.id = None;
            }
            vec![h]
        }
        _ => {
            if n.children.is_empty() { vec![n.clone()] } else { vec![with_kids(n, side)] }
        }
    }
}

fn side_of(nodes: &[Node], side: Side) -> Vec<Node> {
    nodes.iter().flat_map(|n| sided(n, side)).collect()
}

struct Row {
    mark: Option<Mark>,
    old: Option<Vec<Node>>,
    neu: Option<Vec<Node>>,
    joined: bool,
}

fn row_for(nodes: Vec<Node>) -> Row {
    if nodes.iter().any(has_marks) {
        Row { mark: Some(Mark::Changed), old: Some(side_of(&nodes, Side::Old)), neu: Some(side_of(&nodes, Side::New)), joined: false }
    } else {
        Row { mark: None, old: Some(side_of(&nodes, Side::Old)), neu: Some(nodes), joined: false }
    }
}

fn within(rows: Vec<Row>, shell: impl Fn(Vec<Node>) -> Node) -> Vec<Row> {
    let last = rows.len().saturating_sub(1);
    rows.into_iter()
        .enumerate()
        .map(|(i, r)| Row { mark: r.mark, old: r.old.map(|o| vec![shell(o)]), neu: r.neu.map(|o| vec![shell(o)]), joined: i < last })
        .collect()
}

fn list_rows(list: &Node) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut run: Vec<Node> = Vec::new();
    for it in &list.children {
        if !has_marks(it) {
            run.push(it.clone());
            continue;
        }
        if !run.is_empty() {
            rows.push(row_for(std::mem::take(&mut run)));
        }
        match it.data().and_then(|d| d.diff) {
            Some(Mark::Added) => rows.push(Row { mark: Some(Mark::Added), old: None, neu: Some(vec![it.clone()]), joined: false }),
            Some(Mark::Removed) => rows.push(Row { mark: Some(Mark::Removed), old: Some(side_of(std::slice::from_ref(it), Side::Old)), neu: None, joined: false }),
            _ => rows.push(row_for(vec![it.clone()])),
        }
    }
    if !run.is_empty() {
        rows.push(row_for(run));
    }
    within(rows, |items| {
        let mut copy = Node::new(list.kind.clone());
        copy.children = items;
        copy
    })
}

fn node_rows(n: &Node) -> Vec<Row> {
    match &n.kind {
        Kind::DiffBlock { mark: Mark::Added, .. } => vec![Row { mark: Some(Mark::Added), old: None, neu: Some(vec![n.clone()]), joined: false }],
        Kind::DiffBlock { mark: Mark::Removed, .. } => vec![Row { mark: Some(Mark::Removed), old: Some(side_of(std::slice::from_ref(n), Side::Old)), neu: None, joined: false }],
        Kind::DiffBlock { .. } => vec![row_for(vec![n.clone()])],
        Kind::DiffPair => vec![Row { mark: Some(Mark::Changed), old: n.children.first().map(|h| vec![h.clone()]), neu: n.children.get(1).map(|h| vec![h.clone()]), joined: false }],
        Kind::List { .. } if has_marks(n) => list_rows(n),
        Kind::Blockquote if has_marks(n) => within(flow_rows(&n.children), |children| {
            let mut copy = Node::new(Kind::Blockquote);
            copy.children = children;
            copy
        }),
        _ => vec![row_for(vec![n.clone()])],
    }
}

fn flow_rows(flow: &[Node]) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut i = 0;
    while i < flow.len() {
        let n = &flow[i];
        if let Kind::Html(v) = &n.kind
            && html_depth(v) > 0
        {
            let mut run = vec![n.clone()];
            let mut depth = html_depth(v);
            while depth > 0 && i + 1 < flow.len() {
                i += 1;
                run.push(flow[i].clone());
                if let Kind::Html(w) = &flow[i].kind {
                    depth = (depth + html_depth(w)).max(0);
                }
            }
            rows.push(row_for(run));
        } else {
            rows.extend(node_rows(n));
        }
        i += 1;
    }
    rows
}

fn cell(old: bool, nodes: Option<Vec<Node>>) -> Node {
    match nodes {
        Some(n) => Node::with(Kind::SplitCell { old, empty: false }, n),
        None => Node::new(Kind::SplitCell { old, empty: true }),
    }
}

/// The split view of a rendered diff: rows of the old side's cell and the new side's; footnotes follow the rows.
pub fn split_tree(root: Vec<Node>) -> Vec<Node> {
    let (defs, flow): (Vec<Node>, Vec<Node>) = root.into_iter().partition(|n| matches!(n.kind, Kind::FootnoteDefinition { .. }));
    let mut out: Vec<Node> = flow_rows(&flow).into_iter().map(|r| Node::with(Kind::SplitRow { mark: r.mark, joined: r.joined }, vec![cell(true, r.old), cell(false, r.neu)])).collect();
    out.extend(defs);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(30)
    }

    fn diff(old: &str, new: &str) -> DiffResult {
        let r = diff_trees(&parse(old, Flavor::Github, true), &parse(new, Flavor::Github, true), Duration::from_secs(30));
        assert!(!r.gave_up);
        r
    }

    fn shape(nodes: &[Node]) -> String {
        nodes
            .iter()
            .map(|n| match &n.kind {
                Kind::DiffBlock { mark, .. } => format!("{}({})", mark.label(), shape(&n.children)),
                Kind::DiffIns => format!("+[{}]", n.text_of()),
                Kind::DiffDel => format!("-[{}]", n.text_of()),
                Kind::Paragraph => format!("p({})", shape(&n.children)),
                Kind::Text(t) => t.clone(),
                _ => {
                    let name = format!("{:?}", n.kind);
                    name.split([' ', '(', '{']).next().unwrap_or("").to_string() + &format!("({})", shape(&n.children))
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn jsdiff_tokens() {
        assert_eq!(tokens("foo bar, baz"), ["foo", " ", "bar", ",", " ", "baz"]);
        assert_eq!(tokens("  a\t\n"), ["  ", "a", "\t\n"]);
        assert_eq!(tokens("été!"), ["été", "!"]);
    }

    #[test]
    fn the_same_text_has_no_changes() {
        let r = diff("# T\n\npara\n\n- a\n", "# T\n\npara\n\n- a\n");
        assert_eq!(r.changes, 0);
        assert_eq!(r.root.len(), 3);
    }

    #[test]
    fn a_changed_paragraph_marks_its_words() {
        let r = diff("A slow brown fox.\n", "A quick brown fox.\n");
        assert_eq!(r.changes, 1);
        assert_eq!(shape(&r.root), "Changed(p(A  -[slow] +[quick]  brown fox.))");
    }

    #[test]
    fn whitespace_that_does_not_show_is_no_change() {
        assert_eq!(diff("a  b\nc\n", "a b c\n").changes, 0);
    }

    #[test]
    fn added_and_removed_blocks_stand_alone() {
        let r = diff("one\n\ntwo\n", "one\n\nsomething entirely different here now\n");
        assert!(shape(&r.root).contains("Removed("), "{}", shape(&r.root));
        assert!(shape(&r.root).contains("Added("));
        assert_eq!(r.changes, 2);
        let r = diff("", "new\n");
        assert_eq!(shape(&r.root), "Added(p(new))", "an added file");
        let r = diff("old\n", "");
        assert_eq!(shape(&r.root), "Removed(p(old))", "a deleted one");
    }

    #[test]
    fn a_changed_list_marks_its_items_and_keeps_their_numbers() {
        let r = diff("1. a\n2. b is here\n3. c\n", "1. a\n2. b is there\n3. c\n4. d\n");
        let list = &r.root[0];
        assert_eq!(list.children.len(), 4);
        assert_eq!(list.children[1].data().unwrap().diff, Some(Mark::Changed));
        assert_eq!(list.children[3].data().unwrap().diff, Some(Mark::Added));
        assert_eq!(list.children[3].data().unwrap().value, Some(4));
        assert_eq!(r.changes, 2);
    }

    #[test]
    fn a_changed_table_marks_its_rows_and_cells() {
        let r = diff("| a | b |\n|---|---|\n| kept | 1 |\n| changed one | 2 |\n| gone gone | 3 |\n", "| a | b |\n|---|---|\n| kept | 1 |\n| changed one | 22 |\n| fresh fresh | 4 |\n");
        let t = &r.root[0];
        let marks: Vec<_> = t.children.iter().map(|x| x.data().and_then(|d| d.diff)).collect();
        assert_eq!(marks[0], None);
        assert_eq!(marks[1], None);
        assert_eq!(marks[2], Some(Mark::Changed));
        assert!(marks[3..].contains(&Some(Mark::Removed)) || marks[3..].contains(&Some(Mark::Changed)));
    }

    #[test]
    fn a_changed_code_block_merges_its_lines_with_marks() {
        let m = code_lines("fn a() {\n  old();\n}", "fn a() {\n  new();\n}", Some(soon())).unwrap();
        assert_eq!(m.value, "fn a() {\n  old();\n  new();\n}");
        assert_eq!(m.marks, " -+ ");
        let words = m.words.unwrap();
        assert_eq!(words.split(';').collect::<Vec<_>>(), ["", "2-5", "2-5", ""]);
    }

    #[test]
    fn crlf_never_shows_as_a_change() {
        let m = code_lines("a\r\nb", "a\nb", Some(soon())).unwrap();
        assert_eq!(m.marks, "  ");
        assert_eq!(m.value, "a\nb");
    }

    #[test]
    fn only_a_fences_info_changing_says_so() {
        let r = diff("```rust a\nx\n```\n", "```rust b\nx\n```\n");
        assert!(matches!(&r.root[0].kind, Kind::DiffBlock { mark: Mark::Changed, note: Some(n) } if n == "Code block info changed: a → b"), "{:?}", r.root[0].kind);
        // Another language isn't the same kind of block: it's replaced.
        let r = diff("```rust\nx\n```\n", "```ts\nx\n```\n");
        assert!(matches!(&r.root[0].kind, Kind::DiffBlock { mark: Mark::Removed, .. }));
    }

    #[test]
    fn a_changed_diagram_is_a_pair_of_its_two_sides() {
        let r = diff("```mermaid\ngraph TD\n```\n", "```mermaid\ngraph LR\n```\n");
        assert!(matches!(r.root[0].kind, Kind::DiffPair));
        assert_eq!(r.changes, 1);
    }

    #[test]
    fn a_changed_definition_shows_at_its_uses() {
        let r = diff("[a][r]\n\n[r]: https://old.example\n", "[a][r]\n\n[r]: https://new.example\n");
        assert_eq!(r.changes, 1, "{}", shape(&r.root));
        let s = format!("{:?}", r.root);
        assert!(s.contains("https://old.example") && s.contains("https://new.example"), "{s}");
    }

    #[test]
    fn html_runs_stay_whole() {
        let r = diff("<details>\n\nx\n\n</details>\n", "<details>\n\ny y y y\n\n</details>\n");
        assert_eq!(shape(&r.root).matches("Removed(").count(), 1, "{}", shape(&r.root));
        assert_eq!(shape(&r.root).matches("Added(").count(), 1);
    }

    #[test]
    fn footnotes_diff_by_id() {
        let r = diff("a[^x]\n\n[^x]: Old note.\n", "a[^x]\n\n[^x]: New note.\n");
        assert!(r.changes >= 1, "{}", shape(&r.root));
    }

    #[test]
    fn it_gives_up_past_its_deadline() {
        let r = diff_trees(&parse("a\n", Flavor::Github, true), &parse("b\n", Flavor::Github, true), Duration::ZERO);
        assert!(r.gave_up);
    }

    #[test]
    fn huge_blocks_do_not_word_diff() {
        let (a, b) = ("word ".repeat(5000), "other ".repeat(5000));
        assert_eq!(similarity(&a, &b, soon()).unwrap(), 0.);
    }

    #[test]
    fn hostile_html_in_a_diff_stays_bounded() {
        let (a, b) = ("<div>".repeat(50_000), "<details>".repeat(50_000));
        let started = Instant::now();
        let _ = diff_markdown(&a, &b, Flavor::Github);
        assert!(started.elapsed().as_secs() < 30);
        // Many paragraphs, each like the next, must end at the deadline at the latest.
        let (a, b) = ((0..20_000).map(|i| format!("para {i} x\n\n")).collect::<String>(), (0..20_000).map(|i| format!("para {i} y\n\n")).collect::<String>());
        let started = Instant::now();
        let _ = diff_markdown(&a, &b, Flavor::Github);
        assert!(started.elapsed().as_secs() < 30);
    }

    #[test]
    fn split_rows_show_each_side_only_what_it_has() {
        let r = diff("A slow fox.\n\ngone gone gone gone gone\n", "A quick fox.\n\nfresh fresh fresh fresh fresh\n");
        let rows = split_tree(r.root);
        let Kind::SplitRow { mark, .. } = rows[0].kind else { panic!() };
        assert_eq!(mark, Some(Mark::Changed));
        let (old, new) = (shape(&rows[0].children[0].children), shape(&rows[0].children[1].children));
        assert!(old.contains("-[slow]") && !old.contains("quick"), "{old}");
        assert!(new.contains("+[quick]") && !new.contains("slow"), "{new}");
        let empties: Vec<bool> = rows[1..].iter().flat_map(|r| r.children.iter().map(|c| matches!(c.kind, Kind::SplitCell { empty: true, .. }))).collect();
        assert!(empties.contains(&true), "a block only one side has leaves the other a placeholder");
    }

    #[test]
    fn a_split_list_is_a_row_an_item_with_marks() {
        let r = diff("- a\n- b\n- c\n", "- a\n- b\n- d is new here now\n");
        let rows = split_tree(r.root);
        assert!(rows.len() >= 2);
        assert!(matches!(rows[0].kind, Kind::SplitRow { joined: true, .. }));
    }

    #[test]
    fn a_table_row_only_one_side_has_keeps_the_rows_level() {
        let r = diff("| a |\n|---|\n| x |\n", "| a |\n|---|\n| x |\n| brand new row |\n");
        let rows = split_tree(r.root);
        let old = &rows[0].children[0].children[0];
        assert!(old.children.last().unwrap().data().unwrap().empty);
    }
}
