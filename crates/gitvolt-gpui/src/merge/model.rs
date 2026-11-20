//! The merge tool's model, pure (`ui/src/conflicts/model.ts`, `lineMarks.ts`, `sides.ts`). The output is the
//! merged text: the common parts as they are, each conflict region filled with its ticked lines, the side
//! ticked first going first. Every conflict line carries its own terminator, so the output is them joined;
//! the file's line ending goes in only after a ticked line without one that another ticked line follows.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Current,
    Incoming,
}

impl Side {
    pub fn other(self) -> Self {
        match self {
            Side::Current => Side::Incoming,
            Side::Incoming => Side::Current,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Side::Current => 0,
            Side::Incoming => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictSegment {
    pub id: u32,
    pub current: Vec<String>,
    pub incoming: Vec<String>,
}

impl ConflictSegment {
    pub fn of(&self, side: Side) -> &[String] {
        match side {
            Side::Current => &self.current,
            Side::Incoming => &self.incoming,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Common(String),
    Conflict(ConflictSegment),
}

/// A region's ticks. `first`: whose lines go first in the output, the side that got a tick first. A side
/// whose last tick goes and that is ticked again comes after the other. `None`: Current's go first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pick {
    pub current: Vec<bool>,
    pub incoming: Vec<bool>,
    pub first: Option<Side>,
}

impl Pick {
    pub fn of(&self, side: Side) -> &[bool] {
        match side {
            Side::Current => &self.current,
            Side::Incoming => &self.incoming,
        }
    }

    fn set(&mut self, side: Side, flags: Vec<bool>) {
        match side {
            Side::Current => self.current = flags,
            Side::Incoming => self.incoming = flags,
        }
    }

    pub fn any(&self) -> bool {
        self.current.iter().any(|b| *b) || self.incoming.iter().any(|b| *b)
    }
}

pub type Picks = BTreeMap<u32, Pick>;

/// A region's place in the output: its 1-based first line and its line count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutRegion {
    pub id: u32,
    pub start: usize,
    pub lines: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    All,
    None,
    Some,
}

/// A pane's region: `start` is 1-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneRegion {
    pub id: u32,
    pub start: usize,
    pub lines: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneData {
    pub text: String,
    pub regions: Vec<PaneRegion>,
}

impl PaneData {
    /// The region holding line `line` (1-based), and the line's index in it.
    pub fn region_at(&self, line: usize) -> Option<(u32, usize)> {
        self.regions.iter().find(|g| g.lines > 0 && line >= g.start && line < g.start + g.lines).map(|g| (g.id, line - g.start))
    }
}

/// `ConflictFilePayload`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictFile {
    pub path: String,
    pub kind: String,
    pub text: bool,
    pub segments: Vec<Segment>,
    pub current: Option<PaneData>,
    pub incoming: Option<PaneData>,
    pub label_current: String,
    pub label_incoming: String,
    pub eol: String,
    pub base: Option<String>,
}

impl ConflictFile {
    /// The core's answer; `None`: the file isn't conflicted.
    pub fn of(v: &Value) -> Option<Self> {
        if !v.is_object() {
            return None;
        }
        let strs = |x: &Value| x.as_array().map_or(Vec::new(), |a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect::<Vec<_>>());
        let segments = v["segments"].as_array().map_or(Vec::new(), |list| {
            list.iter()
                .map(|s| {
                    if s["kind"] == "conflict" {
                        Segment::Conflict(ConflictSegment { id: s["id"].as_u64().unwrap_or(0) as u32, current: strs(&s["current"]), incoming: strs(&s["incoming"]) })
                    } else {
                        Segment::Common(s["text"].as_str().unwrap_or_default().to_string())
                    }
                })
                .collect()
        });
        let pane = |p: &Value| {
            p.is_object().then(|| PaneData {
                text: p["text"].as_str().unwrap_or_default().to_string(),
                regions: p["regions"].as_array().map_or(Vec::new(), |r| r.iter().map(|g| PaneRegion { id: g["id"].as_u64().unwrap_or(0) as u32, start: g["start"].as_u64().unwrap_or(0) as usize, lines: g["lines"].as_u64().unwrap_or(0) as usize }).collect()),
            })
        };
        Some(Self {
            path: v["path"].as_str().unwrap_or_default().to_string(),
            kind: v["kind"].as_str().unwrap_or_default().to_string(),
            text: v["text"].as_bool().unwrap_or(false),
            segments,
            current: pane(&v["current"]),
            incoming: pane(&v["incoming"]),
            label_current: v["labels"]["current"].as_str().unwrap_or("Current").to_string(),
            label_incoming: v["labels"]["incoming"].as_str().unwrap_or("Incoming").to_string(),
            eol: v["eol"].as_str().unwrap_or("lf").to_string(),
            base: v["base"].as_str().map(str::to_string),
        })
    }

    /// Text on both sides: the panes and the output. Otherwise the buttons.
    pub fn textual(&self) -> bool {
        self.text && self.current.is_some() && self.incoming.is_some()
    }

    pub fn pane(&self, side: Side) -> Option<&PaneData> {
        match side {
            Side::Current => self.current.as_ref(),
            Side::Incoming => self.incoming.as_ref(),
        }
    }

    pub fn conflict(&self, id: u32) -> Option<&ConflictSegment> {
        conflicts(&self.segments).find(|s| s.id == id)
    }
}

pub fn conflicts(segs: &[Segment]) -> impl Iterator<Item = &ConflictSegment> {
    segs.iter().filter_map(|s| if let Segment::Conflict(c) = s { Some(c) } else { None })
}

fn newlines(t: &str) -> usize {
    t.bytes().filter(|b| *b == b'\n').count()
}

/// Whether any line of any region is ticked.
pub fn any_picked(picks: &Picks) -> bool {
    picks.values().any(Pick::any)
}

/// Whether `side` has lines in any region (else its Take all box has nothing to take).
pub fn side_has_lines(segs: &[Segment], side: Side) -> bool {
    conflicts(segs).any(|s| !s.of(side).is_empty())
}

pub fn eol_text(eol: &str) -> &'static str {
    if eol == "crlf" { "\r\n" } else { "\n" }
}

/// A region's ticked lines as output text: joined as they are, with `eol` after a line with no terminator of
/// its own when another line follows it.
pub fn region_text(lines: &[String], eol: &str) -> String {
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        out.push_str(l);
        if i + 1 < lines.len() && !l.ends_with('\n') {
            out.push_str(eol);
        }
    }
    out
}

pub fn empty_picks(segs: &[Segment]) -> Picks {
    conflicts(segs).map(|s| (s.id, Pick { current: vec![false; s.current.len()], incoming: vec![false; s.incoming.len()], first: None })).collect()
}

/// The sides in a region's output order.
pub fn side_order(pick: Option<&Pick>) -> (Side, Side) {
    if pick.is_some_and(|p| p.first == Some(Side::Incoming)) { (Side::Incoming, Side::Current) } else { (Side::Current, Side::Incoming) }
}

pub fn region_lines(seg: &ConflictSegment, pick: Option<&Pick>) -> Vec<String> {
    let Some(pick) = pick else { return Vec::new() };
    let (a, b) = side_order(Some(pick));
    let mut out = Vec::new();
    for side in [a, b] {
        for (i, line) in seg.of(side).iter().enumerate() {
            if pick.of(side).get(i).copied().unwrap_or(false) {
                out.push(line.clone());
            }
        }
    }
    out
}

/// `eol`: the file's, put after a ticked line with no terminator of its own.
pub fn build_output(segs: &[Segment], picks: &Picks, eol: &str) -> (String, Vec<OutRegion>) {
    let mut text = String::new();
    let mut line = 1;
    let mut out = Vec::new();
    for s in segs {
        match s {
            Segment::Common(t) => {
                text.push_str(t);
                line += newlines(t);
            }
            Segment::Conflict(c) => {
                let lines = region_lines(c, picks.get(&c.id));
                out.push(OutRegion { id: c.id, start: line, lines: lines.len() });
                text.push_str(&region_text(&lines, eol));
                line += lines.len();
            }
        }
    }
    (text, out)
}

fn state_of(flags: &[bool]) -> CheckState {
    if flags.iter().all(|b| *b) {
        CheckState::All
    } else if flags.iter().any(|b| *b) {
        CheckState::Some
    } else {
        CheckState::None
    }
}

pub fn hunk_state(seg: &ConflictSegment, pick: &Pick, side: Side) -> CheckState {
    if seg.of(side).is_empty() { CheckState::All } else { state_of(pick.of(side)) }
}

pub fn side_state(segs: &[Segment], picks: &Picks, side: Side) -> CheckState {
    let states: Vec<CheckState> = conflicts(segs).filter(|s| !s.of(side).is_empty()).map(|s| picks.get(&s.id).map_or(CheckState::None, |p| state_of(p.of(side)))).collect();
    if states.is_empty() || states.iter().all(|s| *s == CheckState::All) {
        CheckState::All
    } else if states.iter().all(|s| *s == CheckState::None) {
        CheckState::None
    } else {
        CheckState::Some
    }
}

fn with_side(picks: &Picks, id: u32, side: Side, flags: Vec<bool>) -> Picks {
    let mut out = picks.clone();
    let p = out.entry(id).or_default();
    let had = p.of(side).iter().any(|b| *b);
    let has = flags.iter().any(|b| *b);
    let other_has = p.of(side.other()).iter().any(|b| *b);
    if has && !had {
        p.first = Some(if other_has { side.other() } else { side });
    } else if !has && had {
        p.first = other_has.then(|| side.other());
    }
    p.set(side, flags);
    out
}

pub fn toggle_hunk(picks: &Picks, seg: &ConflictSegment, side: Side) -> Picks {
    let pick = picks.get(&seg.id).cloned().unwrap_or_default();
    let all = hunk_state(seg, &pick, side) == CheckState::All && !seg.of(side).is_empty();
    with_side(picks, seg.id, side, vec![!all; seg.of(side).len()])
}

pub fn toggle_line(picks: &Picks, id: u32, side: Side, index: usize) -> Picks {
    let flags = picks.get(&id).map_or(Vec::new(), |p| p.of(side).iter().enumerate().map(|(i, f)| if i == index { !f } else { *f }).collect());
    with_side(picks, id, side, flags)
}

/// "Take all from this side": that side alone.
pub fn take_all(picks: &Picks, segs: &[Segment], side: Side, on: bool) -> Picks {
    let mut p = picks.clone();
    for s in conflicts(segs) {
        p = with_side(&p, s.id, side, vec![on; s.of(side).len()]);
        if on {
            p = with_side(&p, s.id, side.other(), vec![false; s.of(side.other()).len()]);
        }
    }
    p
}

pub fn unpicked_count(segs: &[Segment], picks: &Picks, edited: &BTreeSet<u32>) -> usize {
    conflicts(segs).filter(|s| !edited.contains(&s.id) && region_lines(s, picks.get(&s.id)).is_empty()).count()
}

/// Where the tool opens: the first region with nothing ticked or typed in it, else the first region.
pub fn opening_region(segs: &[Segment], picks: &Picks, edited: &BTreeSet<u32>) -> Option<u32> {
    conflicts(segs).find(|s| !edited.contains(&s.id) && region_lines(s, picks.get(&s.id)).is_empty()).or_else(|| conflicts(segs).next()).map(|s| s.id)
}

/// F7 and Shift+F7, wrapping round.
pub fn next_region(rs: &[OutRegion], line: usize, forward: bool) -> Option<OutRegion> {
    let mut sorted = rs.to_vec();
    sorted.sort_by_key(|r| r.start);
    if forward {
        sorted.iter().find(|r| r.start > line).or(sorted.first()).copied()
    } else {
        sorted.iter().rev().find(|r| r.start < line).or(sorted.last()).copied()
    }
}

/// Whether a change at `offset` (replacing `length` bytes with `inserted` bytes), in the output as it was
/// before it, is inside the region `[a, b]`. Typing at the region's end, column 1 of the line after it, isn't;
/// typing at its start, or into an empty region, is. `open_end`: the region ends the file on an unterminated
/// line, so typing at `b` is on its last line.
pub fn change_hits(offset: usize, length: usize, inserted: usize, a: usize, b: usize, open_end: bool) -> bool {
    if length > 0 && offset < b && offset + length > a {
        return true;
    }
    if offset > a && offset < b {
        return true;
    }
    if open_end && offset == b && inserted > 0 {
        return true;
    }
    offset == a && inserted > 0
}

/// A region's place in the output, in bytes of the text as the editor holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub id: u32,
    pub from: usize,
    pub to: usize,
}

/// An edge of a tracked range through one edit, as Monaco's `nodeAcceptEdit` moves an edge that sticks to the
/// character before it (`GrowsOnlyWhenTypingBefore`: the region's start takes what's typed there, its end doesn't).
pub fn move_edge(pos: usize, start: usize, end: usize, inserted: usize) -> usize {
    let deleting = end - start;
    let common = deleting.min(inserted);
    if pos <= start {
        return pos;
    }
    // Inside what's replaced, up to the common length, the edge stays where it is.
    if common > 0 && (pos < start + common || (pos == start + common && deleting > inserted)) {
        return pos;
    }
    if pos <= end {
        return start + inserted;
    }
    pos + inserted - deleting
}

/// The spans through one edit.
pub fn track_spans(spans: &[Span], start: usize, removed: usize, inserted: usize) -> Vec<Span> {
    let end = start + removed;
    spans
        .iter()
        .map(|s| {
            let from = move_edge(s.from, start, end, inserted);
            let to = move_edge(s.to, start, end, inserted).max(from);
            Span { id: s.id, from, to }
        })
        .collect()
}

/// Where each span is once `text` is built: from a region's start line, the length of what fills it.
pub fn spans_of_built(segs: &[Segment], picks: &Picks, eol: &str) -> Vec<Span> {
    let mut at = 0;
    let mut out = Vec::new();
    for s in segs {
        match s {
            Segment::Common(t) => at += normalized_len(t),
            Segment::Conflict(c) => {
                let len = normalized_len(&region_text(&region_lines(c, picks.get(&c.id)), eol));
                out.push(Span { id: c.id, from: at, to: at + len });
                at += len;
            }
        }
    }
    out
}

/// What `text`'s length comes to in the editor, whose lines all end in `\n`.
pub fn normalized_len(text: &str) -> usize {
    normalized(text).len()
}

pub fn normalized(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// A span's place in lines of `text`.
pub fn region_of(text: &str, s: &Span) -> OutRegion {
    let start = text[..s.from].bytes().filter(|b| *b == b'\n').count() + 1;
    let inner = &text[s.from..s.to];
    let breaks = inner.bytes().filter(|b| *b == b'\n').count();
    let lines = if inner.is_empty() { 0 } else { breaks + usize::from(!inner.ends_with('\n')) };
    OutRegion { id: s.id, start, lines }
}

/// Which lines of the region the side `lead` gave, in order: `(lead's count, exact)`.
pub fn tint_kinds(pick: Option<&Pick>, lines: usize, edited: bool) -> Vec<OutputTint> {
    let (lead, then) = side_order(pick);
    let count = |s: Side| pick.map_or(0, |p| p.of(s).iter().filter(|b| **b).count());
    let led = count(lead);
    let exact = !edited && pick.is_some() && lines == led + count(then);
    (0..lines)
        .map(|i| match (exact, i < led) {
            (false, _) => OutputTint::Edited,
            (true, true) => OutputTint::From(lead),
            (true, false) => OutputTint::From(then),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputTint {
    From(Side),
    Edited,
}

// ---- The sides' names (`sides.ts`) ----

/// `CONFLICT_SENTENCE`: "Deleted in main (current), modified in feature/x (incoming)".
pub fn conflict_sentence(kind: &str, current: Option<&str>, incoming: Option<&str>) -> String {
    let c = current.map_or("current".to_string(), |n| format!("{n} (current)"));
    let i = incoming.map_or("incoming".to_string(), |n| format!("{n} (incoming)"));
    match kind {
        "bothAdded" => format!("Added in both {c} and {i}"),
        "bothDeleted" => format!("Deleted in both {c} and {i}"),
        "addedByUs" => format!("Added in {c} only"),
        "addedByThem" => format!("Added in {i} only"),
        "deletedByUs" => format!("Deleted in {c}, modified in {i}"),
        "deletedByThem" => format!("Modified in {c}, deleted in {i}"),
        _ => format!("Changed in both {c} and {i}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn seg(id: u32, current: &[&str], incoming: &[&str]) -> ConflictSegment {
        ConflictSegment { id, current: current.iter().map(|s| s.to_string()).collect(), incoming: incoming.iter().map(|s| s.to_string()).collect() }
    }

    fn segs() -> Vec<Segment> {
        vec![Segment::Common("top\n".into()), Segment::Conflict(seg(1, &["a\n", "b\n"], &["x\n"])), Segment::Common("mid\n".into()), Segment::Conflict(seg(2, &["c\n"], &[])), Segment::Common("end\n".into())]
    }

    #[test]
    fn nothing_ticked_builds_the_common_parts_with_empty_regions() {
        let s = segs();
        let (text, regions) = build_output(&s, &empty_picks(&s), "\n");
        assert_eq!(text, "top\nmid\nend\n");
        assert_eq!(regions, [OutRegion { id: 1, start: 2, lines: 0 }, OutRegion { id: 2, start: 3, lines: 0 }]);
    }

    #[test]
    fn the_side_ticked_first_goes_first_and_a_side_ticked_again_goes_after() {
        let s = segs();
        let c = seg(1, &["a\n", "b\n"], &["x\n"]);
        let p = toggle_hunk(&empty_picks(&s), &c, Side::Incoming);
        let p = toggle_hunk(&p, &c, Side::Current);
        assert_eq!(region_lines(&c, p.get(&1)), ["x\n", "a\n", "b\n"]);
        // Incoming's tick goes, then comes back: it now follows Current.
        let p = toggle_line(&p, 1, Side::Incoming, 0);
        let p = toggle_line(&p, 1, Side::Incoming, 0);
        assert_eq!(region_lines(&c, p.get(&1)), ["a\n", "b\n", "x\n"]);
    }

    #[test]
    fn take_all_takes_that_side_alone_and_clearing_it_leaves_nothing() {
        let s = segs();
        let p = take_all(&empty_picks(&s), &s, Side::Current, true);
        assert_eq!(build_output(&s, &p, "\n").0, "top\na\nb\nmid\nc\nend\n");
        assert_eq!(side_state(&s, &p, Side::Current), CheckState::All);
        let q = take_all(&p, &s, Side::Incoming, true);
        assert_eq!(build_output(&s, &q, "\n").0, "top\nx\nmid\nend\n");
        assert_eq!(side_state(&s, &q, Side::Current), CheckState::None);
        assert!(!any_picked(&take_all(&q, &s, Side::Incoming, false)));
    }

    #[test]
    fn a_hunk_with_some_lines_ticks_the_rest_first() {
        let s = segs();
        let c = seg(1, &["a\n", "b\n"], &["x\n"]);
        let p = toggle_line(&empty_picks(&s), 1, Side::Current, 0);
        assert_eq!(hunk_state(&c, &p[&1], Side::Current), CheckState::Some);
        let p = toggle_hunk(&p, &c, Side::Current);
        assert_eq!(hunk_state(&c, &p[&1], Side::Current), CheckState::All);
        let p = toggle_hunk(&p, &c, Side::Current);
        assert_eq!(hunk_state(&c, &p[&1], Side::Current), CheckState::None);
    }

    #[test]
    fn a_line_with_no_terminator_gets_the_files_only_before_another() {
        let lines = vec!["a".to_string(), "b".to_string()];
        assert_eq!(region_text(&lines, "\r\n"), "a\r\nb");
        assert_eq!(region_text(&lines[..1], "\r\n"), "a");
    }

    #[test]
    fn the_counts_and_the_opening_region_skip_what_has_work() {
        let s = segs();
        let mut edited = BTreeSet::new();
        let p = empty_picks(&s);
        assert_eq!(unpicked_count(&s, &p, &edited), 2);
        assert_eq!(opening_region(&s, &p, &edited), Some(1));
        edited.insert(1);
        assert_eq!(unpicked_count(&s, &p, &edited), 1);
        assert_eq!(opening_region(&s, &p, &edited), Some(2));
        edited.insert(2);
        assert_eq!(opening_region(&s, &p, &edited), Some(1), "all done: the first");
    }

    #[test]
    fn next_and_previous_wrap_round() {
        let rs = [OutRegion { id: 1, start: 5, lines: 1 }, OutRegion { id: 2, start: 9, lines: 0 }];
        assert_eq!(next_region(&rs, 1, true).map(|r| r.id), Some(1));
        assert_eq!(next_region(&rs, 5, true).map(|r| r.id), Some(2));
        assert_eq!(next_region(&rs, 9, true).map(|r| r.id), Some(1));
        assert_eq!(next_region(&rs, 5, false).map(|r| r.id), Some(2));
        assert_eq!(next_region(&rs, 9, false).map(|r| r.id), Some(1));
        assert_eq!(next_region(&[], 1, true), None);
    }

    #[test]
    fn typing_at_a_regions_end_stays_out_and_at_its_start_goes_in() {
        assert!(change_hits(10, 0, 1, 10, 20, false), "at its start");
        assert!(change_hits(15, 0, 1, 10, 20, false), "inside");
        assert!(!change_hits(20, 0, 1, 10, 20, false), "at its end: the next line's start");
        assert!(change_hits(20, 0, 1, 10, 20, true), "unless it ends the file unterminated");
        assert!(change_hits(10, 0, 1, 10, 10, false), "into an empty one");
        assert!(change_hits(5, 8, 0, 10, 20, false), "a deletion over its edge");
        assert!(!change_hits(0, 10, 0, 10, 20, false), "a deletion up to it");
    }

    #[test]
    fn edges_move_as_monacos_do() {
        let s = [Span { id: 1, from: 10, to: 20 }];
        assert_eq!(track_spans(&s, 0, 0, 3), [Span { id: 1, from: 13, to: 23 }], "typing before it moves it");
        assert_eq!(track_spans(&s, 10, 0, 3), [Span { id: 1, from: 10, to: 23 }], "at its start, it grows");
        assert_eq!(track_spans(&s, 15, 0, 3), [Span { id: 1, from: 10, to: 23 }], "inside");
        assert_eq!(track_spans(&s, 20, 0, 3), s, "at its end, it doesn't");
        assert_eq!(track_spans(&s, 30, 0, 3), s, "after it");
        assert_eq!(track_spans(&s, 12, 4, 0), [Span { id: 1, from: 10, to: 16 }], "a deletion inside shrinks it");
        assert_eq!(track_spans(&s, 5, 10, 0), [Span { id: 1, from: 5, to: 10 }], "over its start");
        assert_eq!(track_spans(&s, 10, 10, 2), [Span { id: 1, from: 10, to: 12 }], "the whole of it replaced keeps the new text inside");
        assert_eq!(track_spans(&s, 5, 20, 0), [Span { id: 1, from: 5, to: 5 }], "gone, it's empty where it was");
    }

    #[test]
    fn a_span_knows_its_lines() {
        let text = "a\nb\nc\nd";
        assert_eq!(region_of(text, &Span { id: 1, from: 2, to: 6 }), OutRegion { id: 1, start: 2, lines: 2 });
        assert_eq!(region_of(text, &Span { id: 1, from: 2, to: 2 }), OutRegion { id: 1, start: 2, lines: 0 });
        assert_eq!(region_of(text, &Span { id: 1, from: 6, to: 7 }), OutRegion { id: 1, start: 4, lines: 1 });
    }

    #[test]
    fn built_spans_cover_what_fills_each_region_in_the_editors_line_endings() {
        let s = vec![Segment::Common("top\r\n".into()), Segment::Conflict(seg(1, &["a\r\n"], &[]))];
        let p = take_all(&empty_picks(&s), &s, Side::Current, true);
        assert_eq!(spans_of_built(&s, &p, "\r\n"), [Span { id: 1, from: 4, to: 6 }]);
    }

    #[test]
    fn each_output_line_is_tinted_by_its_side_and_an_edited_region_by_the_base() {
        let p = Pick { current: vec![true], incoming: vec![true], first: Some(Side::Incoming) };
        assert_eq!(tint_kinds(Some(&p), 2, false), [OutputTint::From(Side::Incoming), OutputTint::From(Side::Current)]);
        assert_eq!(tint_kinds(Some(&p), 2, true), [OutputTint::Edited, OutputTint::Edited]);
        assert_eq!(tint_kinds(Some(&p), 3, false), [OutputTint::Edited, OutputTint::Edited, OutputTint::Edited], "a line count that doesn't add up was typed");
    }

    #[test]
    fn a_core_answer_reads_as_the_payload() {
        let v = serde_json::json!({ "path": "a", "kind": "bothModified", "text": true, "segments": [{ "kind": "common", "text": "x\n" }, { "kind": "conflict", "id": 3, "base": [], "current": ["a\n"], "incoming": ["b\n"] }], "current": { "text": "x\na\n", "regions": [{ "id": 3, "start": 2, "lines": 1 }] }, "incoming": { "text": "x\nb\n", "regions": [{ "id": 3, "start": 2, "lines": 1 }] }, "labels": { "current": "main", "incoming": "topic" }, "encoding": "UTF-8", "eol": "lf", "base": "h" });
        let f = ConflictFile::of(&v).unwrap();
        assert!(f.textual());
        assert_eq!(f.conflict(3).unwrap().incoming, ["b\n"]);
        assert_eq!(f.pane(Side::Incoming).unwrap().region_at(2), Some((3, 0)));
        assert_eq!(f.pane(Side::Incoming).unwrap().region_at(1), None);
        assert_eq!(f.label_incoming, "topic");
        assert_eq!(ConflictFile::of(&Value::Null), None);
    }

    #[test]
    fn the_sentence_names_the_sides_it_knows() {
        assert_eq!(conflict_sentence("deletedByUs", Some("main"), Some("topic")), "Deleted in main (current), modified in topic (incoming)");
        assert_eq!(conflict_sentence("bothModified", None, None), "Changed in both current and incoming");
    }
}
