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
        Self { text: text.into(), tabs, tab_starts, marks: Vec::new(), guides: if blank { u16::MAX } else { (indent / crate::metrics::tab_size()) as u16 } }
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
            DiffOp::Delete { old_index, old_len, .. } => push(&mut left, range(&a, old.len(), old_index, old_index + old_len)),
            DiffOp::Insert { new_index, new_len, .. } => push(&mut right, range(&b, new.len(), new_index, new_index + new_len)),
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                push(&mut left, range(&a, old.len(), old_index, old_index + old_len));
                push(&mut right, range(&b, new.len(), new_index, new_index + new_len));
            }
        }
    }
    (left, right)
}

/// The word marks of every replaced block: line by line when its sides have as many lines,
/// else over the whole block's text.
pub fn mark_words(ops: &[DiffOp], old_raw: &[&str], new_raw: &[&str], old: &mut Side, new: &mut Side) {
    for op in ops {
        let DiffOp::Replace { old_index, old_len, new_index, new_len } = *op else { continue };
        if old_len == new_len {
            for i in 0..old_len {
                let (l, r) = word_changes(strip(old_raw[old_index + i]), strip(new_raw[new_index + i]));
                old.lines[old_index + i].marks = display_marks(&old.lines[old_index + i], &l);
                new.lines[new_index + i].marks = display_marks(&new.lines[new_index + i], &r);
            }
        } else {
            let (a, b) = (old_raw[old_index..old_index + old_len].iter().map(|l| strip(l)).collect::<Vec<_>>().join("\n"), new_raw[new_index..new_index + new_len].iter().map(|l| strip(l)).collect::<Vec<_>>().join("\n"));
            let (l, r) = word_changes(&a, &b);
            spread_marks(&a, &l, old, old_index);
            spread_marks(&b, &r, new, new_index);
        }
    }
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
        let here: Vec<(usize, usize)> = marks.iter().filter(|m| m.0 < end && m.1 > start).map(|m| (m.0.max(start) - start, m.1.min(end) - start)).collect();
        line.marks = display_marks(line, &here);
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
