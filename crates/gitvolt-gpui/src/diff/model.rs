//! What a diff shows, with no window: both sides' lines, how they align, the words that changed in
//! them, and the rows each mode draws.

use super::textmate::Span;
use gpui::SharedString;
use similar::{Algorithm, DiffOp, capture_diff_slices_deadline};
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Monaco's `stopRenderingLineAfter`: what's past this in a line isn't drawn.
const MAX_LINE_CHARS: usize = 10_000;
/// Hunk mode's unchanged regions (`HUNK_REGIONS`, ui/src/diff/options.ts).
const CONTEXT_LINES: usize = 3;
const MIN_HIDDEN: usize = 3;
/// A change bigger than this gets no word-level marks: comparing it would take too long to help.
const MAX_WORD_DIFF_BYTES: usize = 64 * 1024;
const LINE_DIFF_LIMIT: Duration = Duration::from_secs(3);
const WORD_DIFF_LIMIT: Duration = Duration::from_millis(200);

/// One line as drawn: tabs expanded. Spans and marks are byte offsets into `text`.
pub struct Line {
    pub text: SharedString,
    /// What a tab-free line can skip: (raw byte, display byte) where the mapping changes.
    tabs: Vec<(u32, u32)>,
    /// Where each tab's spaces start, for showing whitespace.
    pub tab_starts: Vec<u32>,
    /// The words changed in this line.
    pub marks: Vec<(u32, u32)>,
    /// A change that goes on past this line's end, from this column: Monaco tints on to the row's end. A line added or removed
    /// whole is one from its start.
    pub to_end: Option<u32>,
    /// Indent guides, in tab stops; a blank line takes its neighbours'.
    pub guides: u16,
}

impl Line {
    fn new(raw: &str) -> Self {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let raw = match raw.char_indices().nth(MAX_LINE_CHARS) {
            Some((at, _)) => &raw[..at],
            None => raw,
        };
        let mut tabs = Vec::new();
        let mut tab_starts = Vec::new();
        let text = if raw.contains('\t') {
            let mut out = String::with_capacity(raw.len() + 16);
            let mut column = 0usize;
            for (i, c) in raw.char_indices() {
                if c == '\t' {
                    let width = crate::metrics::tab_size() - column % crate::metrics::tab_size();
                    tab_starts.push(out.len() as u32);
                    out.extend(std::iter::repeat_n(' ', width));
                    column += width;
                    tabs.push(((i + 1) as u32, out.len() as u32));
                } else {
                    out.push(c);
                    column += 1;
                }
            }
            out
        } else {
            raw.to_string()
        };
        let indent = text.bytes().take_while(|b| *b == b' ').count();
        let blank = indent == text.len();
        Self { text: text.into(), tabs, tab_starts, marks: Vec::new(), to_end: None, guides: if blank { u16::MAX } else { (indent / crate::metrics::tab_size()) as u16 } }
    }

    /// A byte of `text` → the raw line's: inside a tab's spaces, the tab.
    pub fn raw(&self, display: u32) -> u32 {
        let (mut r0, mut d0) = (0, 0);
        for &(r, d) in &self.tabs {
            if d > display {
                // Before this tab's spaces, one to one; inside them, the tab itself.
                return (r0 + (display - d0)).min(r - 1);
            }
            (r0, d0) = (r, d);
        }
        r0 + (display - d0)
    }

    /// A byte of the raw line → a byte of `text`.
    pub fn display(&self, raw: u32) -> u32 {
        let (mut from, mut to) = (0, 0);
        for &(r, d) in &self.tabs {
            if r > raw {
                break;
            }
            (from, to) = (r, d);
        }
        (to + (raw - from)).min(self.text.len() as u32)
    }
}

pub struct Side {
    pub lines: Vec<Line>,
}

impl Side {
    pub fn new(text: Option<&str>) -> Self {
        let mut lines: Vec<Line> = text.map_or_else(Vec::new, |t| {
            let mut parts: Vec<&str> = t.split('\n').collect();
            if t.is_empty() {
                parts.clear();
            }
            parts.into_iter().map(Line::new).collect()
        });
        // A blank line's guides are the lesser of the lines around it, as Monaco's.
        let mut last = 0;
        for line in &mut lines {
            if line.guides == u16::MAX {
                line.guides = last;
            } else {
                last = line.guides;
            }
        }
        let mut next = 0;
        for line in lines.iter_mut().rev() {
            if line.text.bytes().all(|b| b == b' ') {
                line.guides = line.guides.min(next);
            } else {
                next = line.guides;
            }
        }
        Self { lines }
    }

    /// Colours of the raw text's lines from line `first` on, in display offsets.
    pub fn mapped_spans(&self, first: usize, highlighted: Vec<Vec<Span>>) -> Vec<Vec<Span>> {
        self.lines[first..].iter().zip(highlighted).map(|(line, spans)| spans.into_iter().map(|s| Span { start: line.display(s.start), end: line.display(s.end), ..s }).filter(|s| s.end > s.start).collect()).collect()
    }

    pub fn len(&self) -> u32 {
        self.lines.len() as u32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Hunk,
    Inline,
    Split,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Same,
    Removed,
    Added,
    /// A split row of a change: its left is removed, its right added; either may be a filler.
    Change,
    /// An unchanged region folded away (hunk mode): `count` lines, region `region`.
    Hidden { count: u32, region: u32 },
    /// Hunk mode's header above hunk `hunk` of a WIP diff.
    HunkHeader { hunk: u32 },
    /// A review card's space among the lines: the card itself is laid over it (`forge::review::zones`).
    Zone { zone: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub kind: RowKind,
}

/// Compares lines by what's left without their ends' whitespace (`ignoreTrimWhitespace`).
fn keys(side: &[&str], ignore_whitespace: bool) -> Vec<String> {
    side.iter().map(|l| if ignore_whitespace { l.trim().to_string() } else { l.strip_suffix('\r').unwrap_or(l).to_string() }).collect()
}

pub fn split_lines(text: Option<&str>) -> Vec<&str> {
    match text {
        Some("") | None => Vec::new(),
        Some(t) => t.split('\n').collect(),
    }
}

pub fn line_ops(old: &[&str], new: &[&str], ignore_whitespace: bool) -> Vec<DiffOp> {
    let (a, b) = (keys(old, ignore_whitespace), keys(new, ignore_whitespace));
    capture_diff_slices_deadline(Algorithm::Myers, &a, &b, Some(Instant::now() + LINE_DIFF_LIMIT))
}

/// A word, or one character of anything else: what a change is marked in.
fn tokens(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        if c.is_alphanumeric() || c == '_' {
            start.get_or_insert(i);
        } else {
            if let Some(s) = start.take() {
                out.push((s, &text[s..i]));
            }
            out.push((i, &text[i..i + c.len_utf8()]));
        }
    }
    if let Some(s) = start {
        out.push((s, &text[s..]));
    }
    out
}

type Ranges = Vec<(usize, usize)>;

/// `getBoundaryScore`'s worth of the place between two characters: higher, a better place for a change to start or end.
fn boundary_score(text: &[u8], at: usize) -> u32 {
    let category = |b: Option<&u8>| match b {
        None => (0, 10),
        Some(b) if b.is_ascii_lowercase() => (1, 0),
        Some(b) if b.is_ascii_uppercase() => (2, 0),
        Some(b) if b.is_ascii_digit() => (3, 0),
        Some(b',' | b';') => (4, 30),
        Some(b' ' | b'\t') => (5, 3),
        Some(b'\n' | b'\r') => (6, 10),
        Some(_) => (7, 2),
    };
    let (before, after) = (category(at.checked_sub(1).and_then(|i| text.get(i))), category(text.get(at)));
    u32::from(before.0 != after.0) * 10 + before.1 + after.1
}

/// Monaco's `shiftSequenceDiffs`: a change on one side only can often lie a few characters earlier and mean the same (`rank or ` and
/// ` rank or`): it takes the best place, and the earliest of those.
fn shift_left(text: &str, (mut start, mut end): (usize, usize)) -> (usize, usize) {
    let bytes = text.as_bytes();
    let (mut best, mut best_score) = ((start, end), boundary_score(bytes, start) + boundary_score(bytes, end));
    while start > 0 && end > start && bytes[start - 1] == bytes[end - 1] && bytes[start - 1].is_ascii() {
        start -= 1;
        end -= 1;
        let score = boundary_score(bytes, start) + boundary_score(bytes, end);
        if score >= best_score {
            (best, best_score) = ((start, end), score);
        }
    }
    best
}

/// The byte ranges that differ between `old` and `new`, on each side.
pub fn word_changes(old: &str, new: &str) -> (Ranges, Ranges) {
    if old.len() + new.len() > MAX_WORD_DIFF_BYTES {
        return (Vec::new(), Vec::new());
    }
    let (a, b) = (tokens(old), tokens(new));
    let (ta, tb): (Vec<&str>, Vec<&str>) = (a.iter().map(|t| t.1).collect(), b.iter().map(|t| t.1).collect());
    let range = |list: &[(usize, &str)], total: usize, from: usize, to: usize| {
        let start = list.get(from).map_or(total, |t| t.0);
        let end = list.get(to).map_or(total, |t| t.0);
        (start, end)
    };
    let (mut left, mut right): (Ranges, Ranges) = (Vec::new(), Vec::new());
    let push = |list: &mut Vec<(usize, usize)>, r: (usize, usize)| {
        if r.1 > r.0 {
            match list.last_mut() {
                Some(l) if l.1 == r.0 => l.1 = r.1,
                _ => list.push(r),
            }
        }
    };
    for op in capture_diff_slices_deadline(Algorithm::Myers, &ta, &tb, Some(Instant::now() + WORD_DIFF_LIMIT)) {
        match op {
            DiffOp::Equal { .. } => {}
            DiffOp::Delete { old_index, old_len, .. } => push(&mut left, shift_left(old, range(&a, old.len(), old_index, old_index + old_len))),
            DiffOp::Insert { new_index, new_len, .. } => push(&mut right, shift_left(new, range(&b, new.len(), new_index, new_index + new_len))),
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                push(&mut left, range(&a, old.len(), old_index, old_index + old_len));
                push(&mut right, range(&b, new.len(), new_index, new_index + new_len));
            }
        }
    }
    (left, right)
}

/// The word marks of every replaced block: line by line when its sides have as many lines,
/// else over the whole block's text. A block with lines on one side only is changed whole.
pub fn mark_words(ops: &[DiffOp], old_raw: &[&str], new_raw: &[&str], old: &mut Side, new: &mut Side) {
    for op in ops {
        match *op {
            DiffOp::Delete { old_index, old_len, .. } => old.lines[old_index..old_index + old_len].iter_mut().for_each(|l| l.to_end = Some(0)),
            DiffOp::Insert { new_index, new_len, .. } => new.lines[new_index..new_index + new_len].iter_mut().for_each(|l| l.to_end = Some(0)),
            _ => {}
        }
        let DiffOp::Replace { old_index, old_len, new_index, new_len } = *op else { continue };
        if old_len == new_len {
            for i in 0..old_len {
                let (l, r) = word_changes(strip(old_raw[old_index + i]), strip(new_raw[new_index + i]));
                old.lines[old_index + i].marks = display_marks(&old.lines[old_index + i], &l);
                new.lines[new_index + i].marks = display_marks(&new.lines[new_index + i], &r);
            }
        } else if let Some(pairs) = align_lines(&old_raw[old_index..old_index + old_len], &new_raw[new_index..new_index + new_len]) {
            old.lines[old_index..old_index + old_len].iter_mut().for_each(|l| l.to_end = Some(0));
            new.lines[new_index..new_index + new_len].iter_mut().for_each(|l| l.to_end = Some(0));
            for (o, n) in pairs {
                let (o, n) = (old_index + o, new_index + n);
                let (l, r) = word_changes(strip(old_raw[o]), strip(new_raw[n]));
                old.lines[o].to_end = None;
                new.lines[n].to_end = None;
                old.lines[o].marks = display_marks(&old.lines[o], &l);
                new.lines[n].marks = display_marks(&new.lines[n], &r);
            }
        } else {
            let (a, b) = (old_raw[old_index..old_index + old_len].iter().map(|l| strip(l)).collect::<Vec<_>>().join("\n"), new_raw[new_index..new_index + new_len].iter().map(|l| strip(l)).collect::<Vec<_>>().join("\n"));
            let (l, r) = word_changes(&a, &b);
            spread_marks(&a, &l, old, old_index);
            spread_marks(&b, &r, new, new_index);
        }
    }
}

/// A block of unequal sides, line to line by the words they share: what Monaco's inner changes come to, a line with a partner marked
/// in its words and the rest changed whole. Equally good, the later lines pair. `None`: too big a block to compare.
fn align_lines(old: &[&str], new: &[&str]) -> Option<Vec<(usize, usize)>> {
    const MAX_CELLS: usize = 2500;
    let bytes: usize = old.iter().chain(new).map(|l| l.len()).sum();
    if old.len() * new.len() > MAX_CELLS || bytes > MAX_WORD_DIFF_BYTES {
        return None;
    }
    let words = |line: &&str| -> Vec<String> {
        let mut list: Vec<String> = tokens(strip(line)).into_iter().map(|t| t.1.to_string()).filter(|t| !t.trim().is_empty()).collect();
        list.sort_unstable();
        list
    };
    let (a, b): (Vec<_>, Vec<_>) = (old.iter().map(words).collect(), new.iter().map(words).collect());
    let shared = |x: &[String], y: &[String]| {
        let (mut i, mut j, mut n) = (0, 0, 0u32);
        while i < x.len() && j < y.len() {
            match x[i].cmp(&y[j]) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    (i, j, n) = (i + 1, j + 1, n + 1);
                }
            }
        }
        n
    };
    let mut best = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let score = shared(&a[i - 1], &b[j - 1]);
            let pair = if score > 0 { best[i - 1][j - 1] + score } else { 0 };
            best[i][j] = pair.max(best[i - 1][j]).max(best[i][j - 1]);
        }
    }
    let (mut i, mut j, mut pairs) = (a.len(), b.len(), Vec::new());
    while i > 0 && j > 0 {
        let score = shared(&a[i - 1], &b[j - 1]);
        if score > 0 && best[i][j] == best[i - 1][j - 1] + score {
            pairs.push((i - 1, j - 1));
            (i, j) = (i - 1, j - 1);
        } else if best[i][j] == best[i][j - 1] {
            j -= 1;
        } else {
            i -= 1;
        }
    }
    Some(pairs)
}

fn strip(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

fn display_marks(line: &Line, marks: &[(usize, usize)]) -> Vec<(u32, u32)> {
    marks.iter().map(|m| (line.display(m.0 as u32), line.display(m.1 as u32))).filter(|m| m.1 > m.0).collect()
}

/// Marks in a block's joined text → the marks of the lines they fall in.
fn spread_marks(joined: &str, marks: &[(usize, usize)], side: &mut Side, first: usize) {
    let mut start = 0;
    for (n, text) in joined.split('\n').enumerate() {
        let end = start + text.len();
        let line = &mut side.lines[first + n];
        // What goes on past the line's end is `to_end`'s, not a mark over it too.
        let on = |m: &&(usize, usize)| m.0 <= end && m.1 > end;
        let here: Vec<(usize, usize)> = marks.iter().filter(|m| m.0 < end && m.1 > start && !on(m)).map(|m| (m.0.max(start) - start, m.1.min(end) - start)).collect();
        line.marks = display_marks(line, &here);
        line.to_end = marks.iter().find(|m| on(m)).map(|m| line.display((m.0.max(start) - start) as u32));
        start = end + 1;
    }
}

/// The rows of `mode`. `expanded`: the hunk mode regions the user opened.
pub fn rows(ops: &[DiffOp], mode: Mode, expanded: &HashSet<u32>) -> Vec<Row> {
    let mut out = Vec::new();
    let same = |out: &mut Vec<Row>, o: usize, n: usize, len: usize| {
        for i in 0..len {
            out.push(Row { old: Some((o + i) as u32), new: Some((n + i) as u32), kind: RowKind::Same });
        }
    };
    let last = ops.len().saturating_sub(1);
    for (i, op) in ops.iter().enumerate() {
        match *op {
            DiffOp::Equal { old_index, new_index, len } => {
                let head = if i == 0 { 0 } else { CONTEXT_LINES };
                let tail = if i == last { 0 } else { CONTEXT_LINES };
                if mode == Mode::Hunk && !expanded.contains(&(i as u32)) && len >= head + tail + MIN_HIDDEN {
                    same(&mut out, old_index, new_index, head);
                    out.push(Row { old: Some((old_index + head) as u32), new: Some((new_index + head) as u32), kind: RowKind::Hidden { count: (len - head - tail) as u32, region: i as u32 } });
                    same(&mut out, old_index + len - tail, new_index + len - tail, tail);
                } else {
                    same(&mut out, old_index, new_index, len);
                }
            }
            DiffOp::Delete { old_index, old_len, .. } => {
                for k in 0..old_len {
                    out.push(Row { old: Some((old_index + k) as u32), new: None, kind: if mode == Mode::Split { RowKind::Change } else { RowKind::Removed } });
                }
            }
            DiffOp::Insert { new_index, new_len, .. } => {
                for k in 0..new_len {
                    out.push(Row { old: None, new: Some((new_index + k) as u32), kind: if mode == Mode::Split { RowKind::Change } else { RowKind::Added } });
                }
            }
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                if mode == Mode::Split {
                    for k in 0..old_len.max(new_len) {
                        out.push(Row { old: (k < old_len).then_some((old_index + k) as u32), new: (k < new_len).then_some((new_index + k) as u32), kind: RowKind::Change });
                    }
                } else {
                    for k in 0..old_len {
                        out.push(Row { old: Some((old_index + k) as u32), new: None, kind: RowKind::Removed });
                    }
                    for k in 0..new_len {
                        out.push(Row { old: None, new: Some((new_index + k) as u32), kind: RowKind::Added });
                    }
                }
            }
        }
    }
    out
}

/// File View's rows: the new side alone (or the old one, for a deleted file), without the lines
/// inside a folded range. `ends`: `fold_ends` of that side.
pub fn file_rows(new: &Side, old: &Side, folded: &HashSet<u32>, ends: &[Option<u32>]) -> Vec<Row> {
    let side_new = new.len() > 0;
    let count = if side_new { new.len() } else { old.len() };
    let mut out = Vec::with_capacity(count as usize);
    let mut i = 0;
    while i < count {
        out.push(if side_new { Row { old: None, new: Some(i), kind: RowKind::Same } } else { Row { old: Some(i), new: None, kind: RowKind::Same } });
        i = match ends.get(i as usize).copied().flatten().filter(|_| folded.contains(&i)) {
            Some(end) => end + 1,
            None => i + 1,
        };
    }
    out
}

/// Where each line's fold ends, by indentation (Monaco's without a language service): a line opens
/// one when the next text is indented deeper, and it runs to the last line deeper than it.
pub fn fold_ends(side: &Side) -> Vec<Option<u32>> {
    let mut ends = vec![None; side.lines.len()];
    let mut below: Vec<(usize, u32)> = Vec::new();
    for (i, line) in side.lines.iter().enumerate().rev() {
        let text = line.text.as_ref();
        let indent = text.bytes().take_while(|b| *b == b' ').count();
        if indent == text.len() {
            continue;
        }
        let mut end = None;
        while below.last().is_some_and(|b| b.0 > indent) {
            let (_, last) = below.pop().expect("checked");
            end = Some(end.map_or(last, |e: u32| e.max(last)));
        }
        ends[i] = end;
        below.push((indent, end.unwrap_or(i as u32)));
    }
    ends
}

/// Where `text` breaks to fit `columns`, as Monaco's simple wrapping: after a space or closing
/// punctuation or before an opening one where it can, anywhere else when it must. The bytes each
/// continuation starts at; `indent` columns of them are taken by its indentation.
pub fn wrap_points(text: &str, columns: usize, indent: usize) -> Vec<u32> {
    let mut out = Vec::new();
    if columns == 0 {
        return out;
    }
    let (mut width, mut line_start, mut opportunity) = (0usize, 0usize, None);
    let mut room = columns;
    for (i, c) in text.char_indices() {
        let w = if u32::from(c) >= 0x1100 { 2 } else { 1 };
        if "([{".contains(c) && i > line_start {
            opportunity = Some(i);
        }
        if width + w > room && i > line_start {
            let at = opportunity.filter(|o| *o > line_start).unwrap_or(i);
            out.push(at as u32);
            line_start = at;
            width = text[at..i].chars().map(|c| if u32::from(c) >= 0x1100 { 2 } else { 1 }).sum();
            opportunity = None;
            room = columns.saturating_sub(indent).max(1);
        }
        width += w;
        if i >= indent && " \t})]?|/&.,;".contains(c) {
            opportunity = Some(i + c.len_utf8());
        }
    }
    out
}

pub fn is_change(kind: RowKind) -> bool {
    matches!(kind, RowKind::Removed | RowKind::Added | RowKind::Change)
}

/// Where each block of changed rows starts: what F7 steps through.
pub fn change_starts(rows: &[Row]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut in_change = false;
    for (i, r) in rows.iter().enumerate() {
        let c = is_change(r.kind);
        if c && !in_change {
            out.push(i);
        }
        in_change = c;
    }
    out
}

/// A viewport's slider on a strip (`mdRuler.ts`'s `Slider`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slider {
    pub top: f32,
    pub height: f32,
    pub ratio: f32,
}

/// `sliderOf`, Monaco's `ScrollbarState`: proportional, at least 20 px; `None` when nothing scrolls.
pub fn slider_of(visible: f32, content: f32, scroll: f32, strip: f32) -> Option<Slider> {
    if content <= visible || content <= 0. {
        return None;
    }
    let height = (visible * strip / content).floor().max(20.).round();
    let ratio = (strip - height) / (content - visible);
    Some(Slider { top: (scroll * ratio).round(), height, ratio })
}

/// `scrollTopAt`: the scroll that puts the slider's point `at` (a share of its height) on `y`.
pub fn scroll_top_at(y: f32, visible: f32, content: f32, strip: f32, at: f32) -> f32 {
    match slider_of(visible, content, 0., strip) {
        Some(s) if s.ratio > 0. => ((y - s.height * at) / s.ratio).clamp(0., content - visible),
        _ => 0.,
    }
}

/// `MinimapLayout.create` for a proportional map at a pixel ratio of 1: a slider of the view's lines at the map's line height,
/// moving from the top to the map's last place as the view goes from the file's start to its end.
pub fn minimap_slider(scroll: f32, line_h: f32, map_line: f32, strip: f32, visible: f32, content: f32) -> Slider {
    let lines = visible / line_h;
    let height = (lines * map_line).floor();
    // Scrolling past the last line: the map shows the file's lines and the view's, less the one that stays.
    let file_lines = (content - (visible - line_h).max(0.)) / line_h;
    let extra = (lines - 1.).max(0.);
    let furthest = if extra > 0. { (file_lines + extra - lines - 1.) * map_line } else { (file_lines * map_line - height).max(0.) };
    let room = furthest.min(strip - height).max(0.);
    let ratio = if content > visible { room / (content - visible) } else { 0. };
    Slider { top: scroll * ratio, height, ratio }
}

/// What a press at `y` holds: the slider's point it's on, which then stays under the pointer (the view doesn't move), or off
/// the slider its middle, which centres the view there (`sliderPointAt`, and Monaco's scrollbar).
pub fn slider_grab(y: f32, visible: f32, content: f32, scroll: f32, strip: f32) -> f32 {
    let Some(s) = slider_of(visible, content, scroll, strip) else { return 0.5 };
    let exact = scroll * s.ratio;
    if y >= exact && y < exact + s.height && s.height > 0. { (y - exact) / s.height } else { 0.5 }
}

/// The scroll that keeps the slider's point `at` on `y`.
pub fn scroll_held(s: &Slider, y: f32, at: f32, max: f32) -> f32 {
    if s.ratio > 0. { ((y - s.height * at) / s.ratio).clamp(0., max) } else { 0. }
}

/// `markRects`' rows on the strip: at least 2 px, centred on the change and kept on the strip: (top, height).
pub fn mark_span(top: f32, height: f32, ratio: f32, strip: f32) -> (f32, f32) {
    let (y1, y2) = ((top * ratio).floor(), ((top + height) * ratio).floor());
    let centre = (y1 + y2) / 2.;
    let half = (y2 - centre).max(1.);
    (centre.min(strip - half).max(half) - half, 2. * half)
}

/// The overview ruler's blocks: (first row, rows, removed?), runs of one colour.
pub fn ruler_marks(rows: &[Row]) -> Vec<(usize, usize, bool)> {
    let mut out: Vec<(usize, usize, bool)> = Vec::new();
    let mut add = |i: usize, removed: bool| match out.iter_mut().rev().find(|m| m.2 == removed) {
        Some(m) if m.0 + m.1 == i => m.1 += 1,
        _ => out.push((i, 1, removed)),
    };
    for (i, r) in rows.iter().enumerate() {
        match r.kind {
            RowKind::Removed => add(i, true),
            RowKind::Added => add(i, false),
            RowKind::Change => {
                if r.old.is_some() {
                    add(i, true);
                }
                if r.new.is_some() {
                    add(i, false);
                }
            }
            RowKind::Same | RowKind::Hidden { .. } | RowKind::HunkHeader { .. } | RowKind::Zone { .. } => {}
        }
    }
    out
}

/// `renderWhitespace`: where in `text`, within `from..to`, a space's dot goes, and a tab's arrow. `mode` 1 is Monaco's
/// `boundary`: single spaces between words stay plain.
pub fn whitespace_marks(text: &str, tab_starts: &[u32], from: usize, to: usize, mode: u8, tab: usize) -> (Vec<usize>, Vec<usize>) {
    let b = text.as_bytes();
    let in_tab = |i: usize| tab_starts.iter().any(|s| (*s as usize) <= i && i < *s as usize + tab);
    let mut dots = Vec::new();
    for i in from..to.min(b.len()) {
        if b[i] != b' ' || in_tab(i) {
            continue;
        }
        let single = i > 0 && i + 1 < b.len() && b[i - 1] != b' ' && b[i + 1] != b' ';
        if mode == 2 || !single {
            dots.push(i);
        }
    }
    (dots, tab_starts.iter().map(|s| *s as usize).filter(|s| *s >= from && *s < to).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_shows_by_its_mode() {
        let (dots, _) = whitespace_marks("  a b  ", &[], 0, 7, 1, 4);
        assert_eq!(dots, [0, 1, 5, 6], "at boundaries: not the single one between words");
        let (dots, _) = whitespace_marks("  a b  ", &[], 0, 7, 2, 4);
        assert_eq!(dots, [0, 1, 3, 5, 6]);
        let (dots, tabs) = whitespace_marks("    a", &[0], 0, 5, 2, 4);
        assert_eq!((dots, tabs), (vec![], vec![0]), "a tab's spaces are its arrow");
    }

    fn ops(a: &str, b: &str) -> Vec<DiffOp> {
        line_ops(&split_lines(Some(a)), &split_lines(Some(b)), false)
    }

    #[test]
    fn inline_rows_put_a_changes_removed_lines_before_its_added_ones() {
        let r = rows(&ops("a\nb\nc", "a\nB\nc"), Mode::Inline, &HashSet::new());
        assert_eq!(r.iter().map(|x| (x.old, x.new, x.kind)).collect::<Vec<_>>(), [
            (Some(0), Some(0), RowKind::Same),
            (Some(1), None, RowKind::Removed),
            (None, Some(1), RowKind::Added),
            (Some(2), Some(2), RowKind::Same),
        ]);
    }

    #[test]
    fn split_rows_pair_a_change_and_fill_the_shorter_side() {
        let r = rows(&ops("a\nb\nz", "a\nB\nC\nz"), Mode::Split, &HashSet::new());
        assert_eq!(r.iter().map(|x| (x.old, x.new)).collect::<Vec<_>>(), [(Some(0), Some(0)), (Some(1), Some(1)), (None, Some(2)), (Some(2), Some(3))]);
        assert_eq!(r[2].kind, RowKind::Change);
    }

    #[test]
    fn hunk_mode_folds_all_but_three_lines_around_a_change_until_opened() {
        let old: String = (0..30).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l15\n", "changed\n");
        let d = ops(&old, &new);
        let folded = rows(&d, Mode::Hunk, &HashSet::new());
        let hidden: Vec<(usize, u32)> = folded.iter().enumerate().filter_map(|(i, r)| match r.kind {
            RowKind::Hidden { count, .. } => Some((i, count)),
            _ => None,
        }).collect();
        // Start of file to 3 above; 3 below to the end, and the final empty line.
        assert_eq!(hidden.iter().map(|h| h.1).collect::<Vec<_>>(), [12, 12]);
        assert_eq!(folded.iter().filter(|r| !matches!(r.kind, RowKind::Hidden { .. })).filter_map(|r| r.old).count(), 7, "3 + the removed line + 3");
        let RowKind::Hidden { region, .. } = folded[hidden[0].0].kind else { unreachable!() };
        let opened = rows(&d, Mode::Hunk, &HashSet::from([region]));
        assert_eq!(opened.iter().filter(|r| matches!(r.kind, RowKind::Hidden { .. })).count(), 1);
    }

    #[test]
    fn a_short_unchanged_run_isnt_folded() {
        let d = ops("a\nb\nc\nd\ne\nf\ng\nh\ni", "A\nb\nc\nd\ne\nf\ng\nh\nI");
        assert!(!rows(&d, Mode::Hunk, &HashSet::new()).iter().any(|r| matches!(r.kind, RowKind::Hidden { .. })), "7 lines: 3 + 3 + 1, under the minimum of 3 to hide");
    }

    #[test]
    fn whitespace_at_a_lines_ends_can_be_ignored() {
        let (a, b) = (split_lines(Some("  x\ny")), split_lines(Some("x  \ny")));
        assert!(line_ops(&a, &b, true).iter().all(|o| matches!(o, DiffOp::Equal { .. })));
        assert!(line_ops(&a, &b, false).iter().any(|o| !matches!(o, DiffOp::Equal { .. })));
    }

    #[test]
    fn a_change_marks_the_words_that_differ() {
        let (l, r) = word_changes("let count = 1;", "let total = 1 + 2;");
        assert_eq!((l.as_slice(), r.as_slice()), (&[(4, 9)][..], &[(4, 9), (13, 17)][..]));
    }

    #[test]
    fn a_block_of_unequal_length_spreads_its_marks_over_its_lines() {
        let (old_raw, new_raw) = (vec!["one two"], vec!["one", "two three"]);
        let d = vec![DiffOp::Replace { old_index: 0, old_len: 1, new_index: 0, new_len: 2 }];
        let (mut old, mut new) = (Side::new(Some("one two")), Side::new(Some("one\ntwo three")));
        mark_words(&d, &old_raw, &new_raw, &mut old, &mut new);
        assert!(!old.lines[0].marks.is_empty());
        assert_eq!(new.lines[1].marks.last().map(|m| m.1), Some(9), "through the line's end, within it");
        assert!(new.lines[0].marks.iter().all(|m| m.1 <= 3));
    }

    #[test]
    fn tabs_expand_to_their_stops_and_offsets_follow() {
        let l = Line::new("a\tb\tc");
        assert_eq!(l.text.as_ref(), "a   b   c");
        assert_eq!((l.display(0), l.display(2), l.display(4)), (0, 4, 8));
        assert_eq!(Line::new("\r").text.as_ref(), "");
    }

    #[test]
    fn a_blank_line_takes_the_lesser_guides_around_it() {
        let side = Side::new(Some("a\n        b\n\n    c\n\nd"));
        assert_eq!(side.lines.iter().map(|l| l.guides).collect::<Vec<_>>(), [0, 2, 1, 1, 0, 0]);
    }

    #[test]
    fn an_insertion_takes_the_earliest_of_its_equally_good_places() {
        let (left, right) = word_changes("if visited[rank] then", "if rank == nil or visited[rank] then");
        assert!(left.is_empty(), "nothing of the old line changed");
        assert_eq!(&"if rank == nil or visited[rank] then"[right[0].0..right[0].1], " rank == nil or");
    }

    #[test]
    fn a_line_added_or_removed_whole_tints_to_its_end_and_a_paired_one_only_its_words() {
        let (old_raw, new_raw) = (["keep", "gone", "a b"], ["keep", "a c", "new"]);
        let ops = [DiffOp::Equal { old_index: 0, new_index: 0, len: 1 }, DiffOp::Delete { old_index: 1, old_len: 1, new_index: 1 }, DiffOp::Replace { old_index: 2, old_len: 1, new_index: 1, new_len: 1 }, DiffOp::Insert { old_index: 3, new_index: 2, new_len: 1 }];
        let mut old = Side::new(Some(&old_raw.join("\n")));
        let mut new = Side::new(Some(&new_raw.join("\n")));
        mark_words(&ops, &old_raw, &new_raw, &mut old, &mut new);
        assert_eq!(old.lines[1].to_end, Some(0));
        assert_eq!(new.lines[2].to_end, Some(0));
        assert_eq!(old.lines[2].to_end, None);
        assert_eq!(new.lines[1].to_end, None);
        assert!(!new.lines[1].marks.is_empty());
    }

    #[test]
    fn a_block_of_unequal_sides_pairs_its_lines_by_their_words_and_the_rest_are_whole() {
        let pairs = align_lines(&["local function step30(rank)", "    local v31 = 31"], &["    replaced_a = 1", "    replaced_b = 2", "    replaced_c = 3"]);
        assert_eq!(pairs, Some(vec![(1, 2)]), "the later line of the equally good ones");
        let huge = vec!["x"; 60];
        assert_eq!(align_lines(&huge, &huge[..59]), None);
    }

    #[test]
    fn a_change_across_lines_tints_to_the_end_of_each_but_the_last() {
        let (a, b) = ("x", "one\ntwo\nthree");
        let (_, r) = word_changes(a, b);
        let mut new = Side::new(Some(b));
        spread_marks(b, &r, &mut new, 0);
        assert_eq!(new.lines[0].to_end, Some(0));
        assert_eq!(new.lines[1].to_end, Some(0));
        assert_eq!(new.lines[2].to_end, None);
    }

    #[test]
    fn a_minimaps_slider_moves_slower_than_the_file_and_ends_at_its_end() {
        let s = minimap_slider(0., 18., 2., 700., 700., 18_000.);
        assert_eq!(s.height, (2. * 700f32 / 18.).floor());
        assert_eq!(s.top, 0.);
        let max = 18_000. - 700.;
        let end = minimap_slider(max, 18., 2., 700., 700., 18_000.);
        assert!((end.top - (700. - s.height)).abs() < 1., "the slider's at the strip's end at the file's end: {}", end.top);
    }

    #[test]
    fn a_held_slider_stays_under_the_pointer_and_a_press_off_it_centres() {
        let (visible, content, strip) = (400., 1000., 400.);
        let s = slider_of(visible, content, 300., strip).unwrap();
        assert_eq!((s.top, s.height), (120., 160.), "proportional");
        let y = 150.;
        let at = slider_grab(y, visible, content, 300., strip);
        assert!(at > 0. && at < 1.);
        assert!((scroll_top_at(y, visible, content, strip, at) - 300.).abs() < 0.5, "holding it doesn't move the view");
        assert_eq!(slider_grab(380., visible, content, 300., strip), 0.5);
        assert_eq!(scroll_top_at(10_000., visible, content, strip, 0.5), content - visible, "past the end");
        assert_eq!(scroll_top_at(-10_000., visible, content, strip, 0.5), 0., "before the start");
    }

    #[test]
    fn a_long_files_slider_keeps_its_least_size_and_a_short_one_has_none() {
        assert_eq!(slider_of(400., 100_000., 0., 400.).unwrap().height, 20.);
        assert_eq!(slider_of(400., 300., 0., 400.), None);
        assert_eq!(slider_grab(5., 400., 300., 0., 400.), 0.5);
    }

    #[test]
    fn a_short_change_still_shows_on_the_strip_and_stays_on_it() {
        assert_eq!(mark_span(100., 1., 0.1, 400.), (9., 2.));
        assert_eq!(mark_span(0., 1., 0.1, 400.), (0., 2.), "at the top");
        assert_eq!(mark_span(3999., 1., 0.1, 400.), (398., 2.), "at the end");
    }

    #[test]
    fn changes_step_by_block_and_the_ruler_has_one_mark_per_run_and_colour() {
        let r = rows(&ops("a\nb\nc\nd", "a\nB\nc\nD\nE"), Mode::Inline, &HashSet::new());
        assert_eq!(change_starts(&r), [1, 4]);
        assert_eq!(ruler_marks(&r), [(1, 1, true), (2, 1, false), (4, 1, true), (5, 2, false)]);
    }

    #[test]
    fn a_fold_runs_over_what_is_indented_deeper() {
        let side = Side::new(Some("a {\n    b {\n        c\n\n        d\n    }\n\n    e\n}\nf"));
        assert_eq!(fold_ends(&side), [Some(7), Some(4), None, None, None, None, None, None, None, None]);
        let open = rows_of_file(&side, &HashSet::new());
        assert_eq!(open.len(), 10);
        assert_eq!(rows_of_file(&side, &HashSet::from([1])).iter().filter_map(|r| r.new).collect::<Vec<_>>(), [0, 1, 5, 6, 7, 8, 9]);
        assert_eq!(rows_of_file(&side, &HashSet::from([0])).iter().filter_map(|r| r.new).collect::<Vec<_>>(), [0, 8, 9]);
    }

    fn rows_of_file(side: &Side, folded: &HashSet<u32>) -> Vec<Row> {
        file_rows(side, &Side::new(None), folded, &fold_ends(side))
    }

    #[test]
    fn wrapping_prefers_a_space_or_punctuation_and_keeps_the_indent() {
        assert_eq!(wrap_points("short", 10, 0), Vec::<u32>::new());
        assert_eq!(wrap_points("aaaa bbbb cccc", 10, 0), [10], "after the space that fits");
        assert_eq!(wrap_points("aaaaaaaaaaaaaaa", 10, 0), [10], "anywhere when it must");
        assert_eq!(wrap_points("aaaaaaaaaaaaaaaaaaaaaaaaa", 10, 0), [10, 20]);
        assert_eq!(wrap_points("foo(bar, baz)", 6, 0), [3, 9], "before the opening one, after the comma");
        assert_eq!(wrap_points("    aaaaaaaaaaaaaaaa", 10, 4), [10, 16], "continuations have the indent's fewer columns");
        assert_eq!(wrap_points("語語語", 4, 0), [6], "a wide character takes two columns");
    }

    #[test]
    fn file_view_shows_the_new_side_or_a_deleted_files_old_one() {
        let (a, b) = (Side::new(Some("x\ny")), Side::new(None));
        assert_eq!(file_rows(&a, &b, &HashSet::new(), &[])[1], Row { old: None, new: Some(1), kind: RowKind::Same });
        assert_eq!(file_rows(&b, &a, &HashSet::new(), &[])[1], Row { old: Some(1), new: None, kind: RowKind::Same });
    }
}
