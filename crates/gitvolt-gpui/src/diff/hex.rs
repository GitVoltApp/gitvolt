//! A binary's hex view (`ui/src/diff/hex.tsx`, `hexModel.ts`, `monaco/hexPanes.ts`): offset, hex and
//! text panes per side, byte i against byte i, the changed bytes coloured in both panes. It scrolls as
//! the diff does, on `DiffState`'s `scroll` and `layout`, with one row of the layout per row of bytes.

use super::view::{DiffState, LINE_H, RULER_W, mono, strip_events};
use crate::app::GitVolt;
use crate::theme;
use gpui::prelude::*;
use gpui::*;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

pub const ROW_BYTES: usize = 16;
pub const NARROW_ROW_BYTES: usize = 8;
/// The core's dump rows (`hex_lines`) are always 16 bytes.
const DUMP_ROW_BYTES: usize = 16;
/// Monaco's cell here, whose advance for this font and size it rounds to whole pixels.
const CELL: f32 = 8.;
const OFFSET_W: f32 = 64.;
/// The room from a hex pane's left to its first digit, a text pane's to its first character.
const HEX_LEFT: f32 = OFFSET_W + 10.;
const TEXT_LEFT: f32 = 6.;
/// Room after a pane's last column.
const PANE_PAD: f32 = 4.;
/// Rows of context above a change that Next and Previous change scroll to.
const CONTEXT_ROWS: usize = 3;
/// At most this many marks on the bar: closer changes share one.
const RULER_MARKS: usize = 400;
const FONT_SIZE: f32 = 13.;
const TEXT: u32 = 0xD4D4D4;

/// The column of byte `j`'s first digit in a hex row: `48 65 6c 6c  6f 20 …`.
pub fn hex_col(j: usize, per: usize) -> usize {
    3 * j + usize::from(j >= per / 2)
}

/// The bytes a core dump covers (`hexdump -C` lines): its first `shown`.
pub fn dump_bytes(dump: &str, shown: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(shown);
    for line in dump.split('\n') {
        for j in 0..DUMP_ROW_BYTES.min(shown.saturating_sub(out.len())) {
            // The offset (8), two spaces, then `hex_col`'s layout.
            let c = 10 + hex_col(j, DUMP_ROW_BYTES);
            let Some(v) = line.get(c..c + 2).and_then(|d| u8::from_str_radix(d, 16).ok()) else { return out };
            out.push(v);
        }
        if out.len() >= shown {
            break;
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    Changed,
    Added,
    Removed,
}

/// A run of differing bytes `start..end`: on both sides, or only on one, past the other's end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRun {
    pub start: usize,
    pub end: usize,
    pub kind: RunKind,
}

/// Byte i against byte i: the runs where the sides differ.
pub fn byte_runs(old: &[u8], new: &[u8]) -> Vec<ByteRun> {
    let mut runs = Vec::new();
    let common = old.len().min(new.len());
    let mut i = 0;
    while i < common {
        if old[i] == new[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < common && old[i] != new[i] {
            i += 1;
        }
        runs.push(ByteRun { start, end: i, kind: RunKind::Changed });
    }
    if new.len() > common {
        runs.push(ByteRun { start: common, end: new.len(), kind: RunKind::Added });
    }
    if old.len() > common {
        runs.push(ByteRun { start: common, end: old.len(), kind: RunKind::Removed });
    }
    runs
}

/// The changes, as blocks of adjacent changed rows, 0-based and inclusive.
pub fn changed_rows(runs: &[ByteRun], per: usize) -> Vec<(usize, usize)> {
    let mut blocks: Vec<(usize, usize)> = Vec::new();
    for r in runs {
        let (first, last) = (r.start / per, (r.end - 1) / per);
        match blocks.last_mut() {
            Some(prev) if first <= prev.1 + 1 => prev.1 = prev.1.max(last),
            _ => blocks.push((first, last)),
        }
    }
    blocks
}

/// The runs that overlap bytes `start..end`, clipped to them. 256 KB of alternating changes is 131,072
/// runs, too many to walk per frame.
pub fn runs_in(runs: &[ByteRun], start: usize, end: usize) -> impl Iterator<Item = ByteRun> + '_ {
    let from = runs.partition_point(|r| r.end <= start);
    runs[from..].iter().take_while(move |r| r.start < end).map(move |r| ByteRun { start: r.start.max(start), end: r.end.min(end), kind: r.kind })
}

/// `blocks` with those at most `gap` rows apart merged: the bar's marks.
pub fn merge_blocks(blocks: &[(usize, usize)], gap: usize) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for b in blocks {
        match out.last_mut() {
            Some(prev) if b.0 - prev.1 <= gap => prev.1 = b.1,
            _ => out.push(*b),
        }
    }
    out
}

/// The row Next (or Previous) change goes to from `row`, wrapping around.
pub fn step_change(blocks: &[(usize, usize)], row: usize, forward: bool) -> Option<usize> {
    let found = if forward { blocks.iter().find(|b| b.0 > row).or(blocks.first()) } else { blocks.iter().rev().find(|b| b.0 < row).or(blocks.last()) };
    found.map(|b| b.0)
}

fn decimal(x: f64) -> String {
    let s = format!("{x:.1}");
    s.strip_suffix(".0").map_or(s.clone(), str::to_string)
}

/// "1 byte", "6 bytes", "1.2 KB", "256 KB", "4.2 MB".
pub fn format_size(n: u64) -> String {
    if n < 1024 {
        return if n == 1 { "1 byte".into() } else { format!("{n} bytes") };
    }
    if n < 1024 * 1024 {
        return format!("{} KB", decimal(n as f64 / 1024.));
    }
    format!("{} MB", decimal(n as f64 / (1024. * 1024.)))
}

pub struct HexSide {
    pub size: u64,
    pub bytes: Vec<u8>,
}

impl HexSide {
    fn of(v: &serde_json::Value) -> Option<Self> {
        let size = v["size"].as_u64()?;
        let shown = v["shown"].as_u64().unwrap_or(0) as usize;
        Some(Self { size, bytes: dump_bytes(v["dump"].as_str().unwrap_or_default(), shown) })
    }

    fn capped(&self) -> bool {
        (self.bytes.len() as u64) < self.size
    }
}

/// Both sides' dumps, parsed off the UI thread.
pub struct HexData {
    pub old: Option<HexSide>,
    pub new: Option<HexSide>,
    pub cap: u64,
    pub runs: Vec<ByteRun>,
}

impl HexData {
    pub fn of(v: &serde_json::Value) -> Self {
        let (old, new) = (HexSide::of(&v["old"]), HexSide::of(&v["new"]));
        let runs = match (&old, &new) {
            (Some(a), Some(b)) => byte_runs(&a.bytes, &b.bytes),
            (Some(a), None) if !a.bytes.is_empty() => vec![ByteRun { start: 0, end: a.bytes.len(), kind: RunKind::Removed }],
            (None, Some(b)) if !b.bytes.is_empty() => vec![ByteRun { start: 0, end: b.bytes.len(), kind: RunKind::Added }],
            _ => Vec::new(),
        };
        Self { old, new, cap: v["cap"].as_u64().unwrap_or(0), runs }
    }

    /// What a view shows, each with whether it's the old side. File View: one side's bytes.
    pub fn sides(&self, file: bool) -> Vec<(&HexSide, bool)> {
        if file {
            return self.new.as_ref().map(|s| (s, false)).or(self.old.as_ref().map(|s| (s, true))).into_iter().collect();
        }
        self.old.iter().map(|s| (s, true)).chain(self.new.iter().map(|s| (s, false))).collect()
    }

    /// The file bar's text: "Binary · 6 bytes (added)"; File View: the shown revision's size only.
    /// `None` for a side with no size at all.
    pub fn summary(&self, file: bool) -> Vec<String> {
        if file {
            return vec![format_size(self.new.as_ref().or(self.old.as_ref()).map_or(0, |s| s.size))];
        }
        match (&self.old, &self.new) {
            (Some(a), Some(b)) => vec![format_size(a.size), format_size(b.size)],
            (Some(a), None) => vec![format!("{} (deleted)", format_size(a.size))],
            (None, Some(b)) => vec![format!("{} (added)", format_size(b.size))],
            (None, None) => Vec::new(),
        }
    }

    /// When a shown dump stops short of its side: "Showing the first 256 KB of 4.2 MB".
    pub fn capped_note(&self, file: bool) -> Option<String> {
        let over: Vec<u64> = self.sides(file).iter().filter(|(s, _)| s.capped()).map(|(s, _)| s.size).collect();
        match over.as_slice() {
            [] => None,
            [one] => Some(format!("Showing the first {} of {}", format_size(self.cap), format_size(*one))),
            _ => Some(format!("Showing the first {} of each side", format_size(self.cap))),
        }
    }
}

/// A shown hex view: the dumps, and how they're laid out at this width.
pub struct HexView {
    pub data: Arc<HexData>,
    pub per: usize,
    pub blocks: Rc<Vec<(usize, usize)>>,
}

impl HexView {
    pub fn new(data: Arc<HexData>) -> Self {
        Self { data, per: ROW_BYTES, blocks: Rc::new(Vec::new()) }
    }

    pub fn rows(&self, file: bool) -> usize {
        self.data.sides(file).iter().map(|(s, _)| s.bytes.len().div_ceil(self.per)).max().unwrap_or(0).max(1)
    }
}

/// A hex pane's and a text pane's widths at `per` bytes a row.
pub fn pane_widths(per: usize) -> (f32, f32) {
    let hex = (HEX_LEFT + (per * 3) as f32 * CELL + PANE_PAD).ceil();
    let text = (TEXT_LEFT + per as f32 * CELL + PANE_PAD).ceil() + 1.;
    (hex, text)
}

/// 16 bytes a row when every side fits at that width, with its divider and the bar; else 8.
pub fn choose_per(sides: usize, room: f32) -> usize {
    let (hex, text) = pane_widths(ROW_BYTES);
    if sides as f32 * (hex + text + 1.) + RULER_W <= room { ROW_BYTES } else { NARROW_ROW_BYTES }
}

/// Row `row`'s offset, 8 hex digits.
pub fn offset_label(row: usize, per: usize) -> String {
    format!("{:08x}", row * per)
}

static HEX: LazyLock<Vec<SharedString>> = LazyLock::new(|| (0..=255u8).map(|b| format!("{b:02x}").into()).collect());
static CHAR: LazyLock<Vec<SharedString>> = LazyLock::new(|| (0..=255u8).map(|b| if (0x20..0x7f).contains(&b) { (b as char).to_string().into() } else { ".".into() }).collect());

fn paint_cell(text: &SharedString, x: Pixels, y: Pixels, font: &Font, window: &mut Window, cx: &mut App) {
    let run = TextRun { len: text.len(), font: font.clone(), color: theme::c(TEXT), background_color: None, underline: None, strikethrough: None };
    let line = window.text_system().shape_line(text.clone(), px(FONT_SIZE), &[run], None);
    line.paint(point(x, y), px(LINE_H), TextAlign::Left, None, window, cx).ok();
}

fn band(x: Pixels, y: Pixels, from: f32, to: f32, color: Hsla, window: &mut Window) {
    window.paint_quad(fill(Bounds::from_corners(point(x + px(from), y), point(x + px(to), y + px(LINE_H))), color));
}

#[allow(clippy::too_many_arguments)]
fn paint_side(side: &HexSide, old_side: bool, file: bool, view: (usize, &[ByteRun], &[(usize, usize)]), rows: (usize, usize, f32), x: Pixels, bounds: Bounds<Pixels>, cursor: usize, font: &Font, window: &mut Window, cx: &mut App) {
    let (per, all_runs, blocks) = view;
    let (first, last, scroll) = rows;
    let (hex_w, text_w) = pane_widths(per);
    let (row_tone, byte_tone) = if old_side { (theme::diff_removed_line(), theme::diff_removed_text()) } else { (theme::diff_inserted_line(), theme::diff_inserted_text()) };
    let own = side.bytes.len().div_ceil(per);
    let text_x = hex_w + 1. + TEXT_LEFT;
    for row in first..=last {
        let y = bounds.origin.y + px(row as f32 * LINE_H - scroll);
        if row >= own {
            // A side's rows past its end, hatched as the diff's empty side is.
            window.paint_quad(fill(Bounds::new(point(x, y), size(px(hex_w + text_w), px(LINE_H))), pattern_slash(theme::diff_diagonal(), 2., 6.)));
            continue;
        }
        let at = row * per;
        if !file && blocks.iter().any(|b| b.0 <= row && row <= b.1) {
            band(x, y, HEX_LEFT, hex_w, row_tone, window);
            band(x, y, hex_w + 1., hex_w + text_w, row_tone, window);
        }
        if !file {
            for r in runs_in(all_runs, at, at + per) {
                let end = r.end.min(side.bytes.len());
                if r.start < end {
                    let (a, b) = (r.start - at, end - 1 - at);
                    band(x, y, HEX_LEFT + hex_col(a, per) as f32 * CELL, HEX_LEFT + (hex_col(b, per) + 2) as f32 * CELL, byte_tone, window);
                    band(x, y, text_x + a as f32 * CELL, text_x + (b + 1) as f32 * CELL, byte_tone, window);
                }
            }
        }
        let number = offset_label(row, per);
        let color = theme::c(if row == cursor { 0xC6C6C6 } else { 0x858585 });
        let run = TextRun { len: number.len(), font: font.clone(), color, background_color: None, underline: None, strikethrough: None };
        window.text_system().shape_line(number.into(), px(FONT_SIZE), &[run], None).paint(point(x, y), px(LINE_H), TextAlign::Left, None, window, cx).ok();
        for (j, b) in side.bytes[at..(at + per).min(side.bytes.len())].iter().enumerate() {
            paint_cell(&HEX[*b as usize], x + px(HEX_LEFT + hex_col(j, per) as f32 * CELL), y, font, window, cx);
            paint_cell(&CHAR[*b as usize], x + px(text_x + j as f32 * CELL), y, font, window, cx);
        }
    }
    let divider = |at: f32| Bounds::new(point(x + px(at), bounds.origin.y), size(px(1.), bounds.size.height));
    window.paint_quad(fill(divider(hex_w), theme::section_border()));
    window.paint_quad(fill(divider(hex_w + text_w), theme::section_border()));
}

/// The view's one vertical scrollbar, at its far right: the changes' marks under a viewport slider.
fn bar(diff: &DiffState, view: &HexView, file: bool, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let (layout, scroll) = (diff.layout.clone(), diff.scroll);
    let rows = view.rows(file);
    let marks = if file { Vec::new() } else { merge_blocks(&view.blocks, rows / RULER_MARKS) };
    let tones: Vec<Hsla> = if file { Vec::new() } else { view.data.sides(false).iter().map(|(_, old)| if *old { theme::ruler_removed() } else { theme::ruler_added() }).collect() };
    let cell = Rc::new(Cell::new(Bounds::<Pixels>::default()));
    let paint_cell = cell.clone();
    strip_events(div().id("hex-bar").flex_none().w(px(RULER_W)).h_full(), cell, cx).child(
        canvas(
            |_, _, _| {},
            move |b, _, window, _| {
                paint_cell.set(b);
                let height = b.size.height.as_f32();
                let share = |row: usize| row as f32 / rows.max(1) as f32 * height;
                for &(first, last) in &marks {
                    for (i, tone) in tones.iter().enumerate() {
                        let w = b.size.width / tones.len() as f32;
                        window.paint_quad(fill(Bounds::new(point(b.origin.x + w * i as f32, b.origin.y + px(share(first))), size(w, px((share(last + 1) - share(first)).max(2.)))), *tone));
                    }
                }
                let total = layout.total().max(1.);
                if total > height {
                    window.paint_quad(fill(Bounds::new(point(b.origin.x, b.origin.y + px(scroll * height / total)), size(b.size.width, px(height * height / total))), theme::scroll_thumb()));
                }
            },
        )
        .size_full(),
    )
}

pub fn hex_body(diff: &DiffState, view: &HexView, cx: &mut Context<GitVolt>) -> AnyElement {
    let file = diff.view == super::view::ViewKind::File;
    let (data, per, blocks, scroll, cursor) = (view.data.clone(), view.per, view.blocks.clone(), diff.scroll, diff.cursor);
    let rows = view.rows(file);
    let entity = cx.entity().downgrade();
    div()
        .id("hex-view")
        .flex()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .bg(theme::app_bg())
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let font = mono();
                    let first = (scroll / LINE_H) as usize;
                    let last = (((scroll + bounds.size.height.as_f32()) / LINE_H) as usize).min(rows.saturating_sub(1));
                    let (hex_w, text_w) = pane_widths(per);
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        let mut x = bounds.origin.x;
                        for (side, old_side) in data.sides(file) {
                            paint_side(side, old_side, file, (per, &data.runs, &blocks), (first, last, scroll), x, bounds, cursor, &font, window, cx);
                            x += px(hex_w + text_w + 1.);
                        }
                    });
                    let entity = entity.clone();
                    window.on_mouse_event(move |e: &ScrollEvent, phase, window, cx| {
                        if phase != DispatchPhase::Capture || !bounds.contains(&e.position) {
                            return;
                        }
                        cx.stop_propagation();
                        let dy = e.delta.pixel_delta(window.line_height()).y.as_f32();
                        entity.update(cx, |this, cx| this.with_diff(cx, |d| d.scroll_by(-dy))).ok();
                    });
                },
            )
            .flex_1()
            .min_w_0()
            .h_full(),
        )
        .child(bar(diff, view, file, cx))
        .into_any_element()
}

/// Where a shown hex view moves on Next or Previous change: `Some(row)` to put the cursor on.
pub fn step(view: &HexView, cursor: usize, forward: bool) -> Option<usize> {
    step_change(&view.blocks, cursor, forward)
}

/// The scroll that puts `row` under its context.
pub fn top_of(row: usize) -> f32 {
    row.saturating_sub(CONTEXT_ROWS) as f32 * LINE_H
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    const DUMP: &str = "00000000  48 65 6c 6c 6f 20 77 6f  72 6c 64 21 0a 00 01 02  |Hello world!....|\n00000010  03 04                                             |..|\n00000012\n";

    #[test]
    fn a_dump_reads_back_as_its_bytes() {
        let bytes = dump_bytes(DUMP, 18);
        assert_eq!(bytes.len(), 18);
        assert_eq!(&bytes[..5], b"Hello");
        assert_eq!(bytes[17], 4);
        assert_eq!(dump_bytes(DUMP, 5), b"Hello", "its first shown");
        assert_eq!(dump_bytes("garbage", 4), Vec::<u8>::new());
    }

    #[test]
    fn a_rows_digits_have_a_wider_gap_between_its_halves() {
        assert_eq!((hex_col(0, 16), hex_col(7, 16), hex_col(8, 16)), (0, 21, 25));
        assert_eq!((hex_col(3, 8), hex_col(4, 8)), (9, 13));
    }

    #[test]
    fn runs_are_by_position_with_the_tail_on_its_own() {
        assert_eq!(byte_runs(b"abcd", b"abXY"), [ByteRun { start: 2, end: 4, kind: RunKind::Changed }]);
        assert_eq!(byte_runs(b"ab", b"abc"), [ByteRun { start: 2, end: 3, kind: RunKind::Added }]);
        assert_eq!(byte_runs(b"abc", b"a"), [ByteRun { start: 1, end: 3, kind: RunKind::Removed }]);
        assert_eq!(byte_runs(b"same", b"same"), []);
    }

    #[test]
    fn adjacent_changed_rows_are_one_block_and_marks_merge_by_a_gap() {
        let runs = [ByteRun { start: 0, end: 1, kind: RunKind::Changed }, ByteRun { start: 17, end: 18, kind: RunKind::Changed }, ByteRun { start: 64, end: 65, kind: RunKind::Changed }];
        assert_eq!(changed_rows(&runs, 16), [(0, 1), (4, 4)]);
        assert_eq!(merge_blocks(&[(0, 1), (4, 4), (40, 41)], 3), [(0, 4), (40, 41)]);
    }

    #[test]
    fn only_the_runs_around_the_viewport_are_walked_clipped_to_it() {
        let runs = [ByteRun { start: 0, end: 10, kind: RunKind::Changed }, ByteRun { start: 20, end: 40, kind: RunKind::Changed }];
        assert_eq!(runs_in(&runs, 12, 30).collect::<Vec<_>>(), [ByteRun { start: 20, end: 30, kind: RunKind::Changed }]);
        assert_eq!(runs_in(&runs, 5, 21).count(), 2);
        assert_eq!(runs_in(&runs, 10, 20).count(), 0);
    }

    #[test]
    fn next_and_previous_change_wrap_around() {
        let blocks = [(2, 3), (9, 9)];
        assert_eq!(step_change(&blocks, 0, true), Some(2));
        assert_eq!(step_change(&blocks, 2, true), Some(9));
        assert_eq!(step_change(&blocks, 9, true), Some(2));
        assert_eq!(step_change(&blocks, 9, false), Some(2));
        assert_eq!(step_change(&blocks, 2, false), Some(9));
        assert_eq!(step_change(&[], 2, false), None);
    }

    #[test]
    fn sizes_read_as_the_file_bars() {
        assert_eq!(format_size(1), "1 byte");
        assert_eq!(format_size(6), "6 bytes");
        assert_eq!(format_size(1024), "1 KB");
        assert_eq!(format_size(1229), "1.2 KB");
        assert_eq!(format_size(4_404_019), "4.2 MB");
    }

    #[test]
    fn sixteen_a_row_when_both_sides_fit_else_eight() {
        // As measured on the web UI: 270 and 75 px at 8 a row.
        assert_eq!(pane_widths(8), (270., 75.));
        assert_eq!(pane_widths(16), (462., 139.));
        assert_eq!(choose_per(2, 1158.), 8);
        assert_eq!(choose_per(2, 1234.), 16);
        assert_eq!(choose_per(1, 1158.), 16);
    }

    #[test]
    fn the_notes_say_what_stops_short() {
        let side = |size, len: usize| Some(HexSide { size, bytes: vec![0; len] });
        let d = HexData { old: side(1024, 1024), new: side(4_404_019, 262_144), cap: 262_144, runs: Vec::new() };
        assert_eq!(d.summary(false), ["1 KB", "4.2 MB"]);
        assert_eq!(d.summary(true), ["4.2 MB"]);
        assert_eq!(d.capped_note(false).as_deref(), Some("Showing the first 256 KB of 4.2 MB"));
        let both = HexData { old: side(4_404_019, 262_144), new: side(4_404_019, 262_144), cap: 262_144, runs: Vec::new() };
        assert_eq!(both.capped_note(false).as_deref(), Some("Showing the first 256 KB of each side"));
        let added = HexData { old: None, new: side(6, 6), cap: 262_144, runs: Vec::new() };
        assert_eq!(added.summary(false), ["6 bytes (added)"]);
        assert_eq!(added.capped_note(false), None);
    }
}
