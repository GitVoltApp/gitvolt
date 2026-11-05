//! The diff panel: its header, toolbar, and the rows, drawn with Monaco's metrics from the web UI
//! (`ui/src/diff/{DiffPanel,DiffToolbar,options}`). Each row is shaped and painted when it's on
//! screen; GPUI keeps what it shaped from frame to frame.

use super::hex::{HexData, HexView, choose_per, hex_body};
use super::image::{Background, IMAGE_TOOLBAR_H, ImageView, Side as ImageSide, decode_side, image_body};
use super::language::highlight_language;
use super::model::*;
use super::textmate::{FONT_BOLD, FONT_ITALIC, FONT_UNDERLINE, Span, Tokenizer};
use crate::app::{GitVolt, KeyArea, Tab, icon};
use crate::details::{FileChange, UNCHANGED, segment, segmented, status_icon};
use crate::theme;
use gitvolt_core::api::Request;
use gitvolt_core::error::GbError;
use gpui::prelude::*;
use gpui::*;
use serde_json::Value;
use similar::DiffOp;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const LINE_H: f32 = 18.;
const FONT_SIZE: f32 = 13.;
const NUM_W: f32 = 40.;
const SIGN_W: f32 = 18.;
/// File View's column between the numbers and the text: the fold chevrons.
const FOLD_W: f32 = 26.;
/// JetBrains Mono's advance at `FONT_SIZE`; lines wrap by it.
const ADVANCE: f32 = FONT_SIZE * 0.6;
/// Monaco's hidden-lines row.
const HIDDEN_H: f32 = 24.;
/// The minimap's scale: a character is 1 px wide, a line 2 px high.
const MINIMAP_LINE: f32 = 2.;
const MINIMAP_MAX: f32 = 120.;
const SCROLLBAR_W: f32 = 10.;
pub const RULER_W: f32 = 30.;
/// A split's left pane has more margin before its text than the right one (Monaco's).
const SPLIT_LEFT_MARGIN: f32 = 18.;
pub const HEADER_H: f32 = 37.;
const TOOLBAR_H: f32 = 33.;
const TOOLBAR_NARROW: f32 = 900.;
pub const FONT: &str = "JetBrains Mono";
const TEXT: u32 = 0xD4D4D4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    File,
    Diff,
}

/// Both sides, how they align, and what only the text decides. Built off the UI thread.
pub struct DiffData {
    pub old: Side,
    pub new: Side,
    pub ops: Vec<DiffOp>,
    pub has_old: bool,
    pub encoding: SharedString,
    pub eol_only: bool,
    /// The widest line, in characters: how far text can scroll sideways.
    pub widest: usize,
}

impl DiffData {
    pub fn build(old: Option<&str>, new: Option<&str>, ignore_whitespace: bool, encoding: SharedString, eol_only: bool) -> Self {
        let (old_raw, new_raw) = (split_lines(old), split_lines(new));
        let ops = line_ops(&old_raw, &new_raw, ignore_whitespace);
        let (mut a, mut b) = (Side::new(old), Side::new(new));
        mark_words(&ops, &old_raw, &new_raw, &mut a, &mut b);
        let widest = a.lines.iter().chain(&b.lines).map(|l| l.text.len()).max().unwrap_or(0);
        Self { old: a, new: b, ops, has_old: old.is_some(), encoding, eol_only, widest }
    }
}

/// How many lines are coloured at a time, and held per entry of `Highlight`.
const COLOUR_CHUNK: usize = 128;

/// Each side's colours so far, from its top, in display offsets: pieces, so a frame can hold them
/// while more arrive.
#[derive(Clone, Default)]
pub struct Highlight {
    pub old: Vec<Arc<Vec<Vec<Span>>>>,
    pub new: Vec<Arc<Vec<Vec<Span>>>>,
}

struct Colours {
    old_side: bool,
    spans: Vec<Vec<Span>>,
}

/// What shows instead of text.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    Loading,
    Failed(SharedString),
    TooLarge,
    /// File History: the row's commit deleted the file.
    Deleted,
}

pub enum Binary {
    Hex(Arc<HexData>),
    /// A raster image, or an SVG with its text's diff for its Source toggle.
    Image { old: ImageSide, new: ImageSide, svg: Option<(Arc<DiffData>, Texts)> },
}

/// A position in the text: a row, and a byte of what it shows on `new` (or the old side).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPos {
    pub row: usize,
    pub byte: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection {
    pub anchor: TextPos,
    pub head: TextPos,
    /// Which side, in Split; otherwise each row's own.
    pub old_side: bool,
}

impl Selection {
    pub fn range(&self) -> (TextPos, TextPos) {
        if self.anchor <= self.head { (self.anchor, self.head) } else { (self.head, self.anchor) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drag {
    None,
    Text,
    Ruler,
    Horizontal,
}

/// Where each row lies at a width: rows can be taller than a line when they wrap or are hidden.
#[derive(Default)]
pub struct Layout {
    width: f32,
    pub height: f32,
    wrap: bool,
    /// Each row's top, and the end of the last one.
    pub offsets: Vec<f32>,
    /// The lines that wrap, by side: where their continuations start.
    breaks_old: HashMap<u32, Vec<u32>>,
    breaks_new: HashMap<u32, Vec<u32>>,
}

impl Layout {
    /// The row at `y` px from the content's top.
    pub fn row_at(&self, y: f32) -> usize {
        self.offsets.partition_point(|o| *o <= y).saturating_sub(1).min(self.offsets.len().saturating_sub(2))
    }

    pub fn content(&self) -> f32 {
        self.offsets.last().copied().unwrap_or(0.)
    }

    /// As `scrollBeyondLastLine`: the last line can scroll to the top.
    pub fn total(&self) -> f32 {
        self.content() + (self.height - LINE_H).max(0.)
    }

    pub fn max_scroll(&self) -> f32 {
        (self.total() - self.height).max(0.)
    }

    fn breaks(&self, old_side: bool, n: u32) -> &[u32] {
        if old_side { &self.breaks_old } else { &self.breaks_new }.get(&n).map_or(&[], Vec::as_slice)
    }
}

/// The open file of a tab.
pub struct DiffState {
    pub file: FileChange,
    pub view: ViewKind,
    pub mode: Mode,
    pub ignore_whitespace: bool,
    pub notice: Option<Notice>,
    pub data: Option<Arc<DiffData>>,
    pub highlight: Arc<Highlight>,
    /// The first colours arrived, or there are none to come.
    pub coloured: bool,
    /// Tells the colouring under way to stop: this file closed, or is being read again.
    pub stop_colours: Arc<AtomicBool>,
    pub rows: Rc<Vec<Row>>,
    pub expanded: HashSet<u32>,
    pub changes: Vec<usize>,
    pub ruler: Rc<Vec<(usize, usize, bool)>>,
    pub scroll: f32,
    pub layout: Rc<Layout>,
    pub wrap: bool,
    /// File View's folded lines, and where each line's fold ends.
    pub folded: HashSet<u32>,
    pub fold_ends: Rc<Vec<Option<u32>>>,
    /// Scroll to the first change once the rows' layout is known.
    reveal_first: bool,
    /// The row under the pointer, and whether it's over the gutter: what hovering shows.
    pub hover: Option<(usize, bool)>,
    pub hscroll: f32,
    pub selection: Option<Selection>,
    /// A line to go to, once the file's there (a pasted `path:12`).
    pub pending_line: Option<u32>,
    /// Find's marks in the file, and the one it's on.
    pub found: Rc<Vec<crate::find::TextMatch>>,
    pub found_at: Option<usize>,
    pub cursor: usize,
    pub drag: Drag,
    /// The text area, as last painted, for the mouse.
    pub body: Rc<Cell<Bounds<Pixels>>>,
    /// A newer read replaces what an older one was going to show.
    pub generation: u64,
    /// File History's File: an image shows as hex there, as any binary.
    pub history_file: bool,
    /// A binary's hex view.
    pub hex: Option<HexView>,
    /// An image's diff.
    pub image: Option<ImageView>,
    /// The picks that hold from image to image, and "Match sizes" per file for the session.
    pub background: Background,
    pub match_picks: Rc<std::cell::RefCell<HashMap<String, bool>>>,
    /// Coming back from File History: the same file, where it was.
    restored: bool,
    /// A WIP file's hunks, read with its text.
    pub hunks: Option<super::staging::WipHunks>,
    /// The selection's bar: what its Stage and Discard take.
    pub bar_counts: Option<(usize, usize)>,
    /// A commit of the worktree is queued: staging waits.
    pub commit_held: bool,
    /// The working copy, when this file's new side can be edited.
    pub edit: Option<super::edit::EditState>,
    /// Where the editor's caret is drawn.
    pub caret: Option<TextPos>,
    /// Bring the caret into view at the next layout.
    pub reveal_caret: bool,
}

/// Which bar `toolbar` draws.
pub enum ToolbarKind {
    /// The diff panel's; whether the file has a history to show.
    Panel { has_history: bool, open_in: Option<AnyElement>, staging: Option<AnyElement> },
    /// File History's: its `File | Changes` in File View / Diff View's place, and no Blame or History.
    History(AnyElement),
}

impl DiffState {
    pub fn new(file: FileChange) -> Self {
        Self {
            file,
            view: ViewKind::Diff,
            mode: Mode::Inline,
            ignore_whitespace: false,
            notice: Some(Notice::Loading),
            data: None,
            highlight: Arc::default(),
            coloured: false,
            stop_colours: Arc::default(),
            rows: Rc::new(Vec::new()),
            expanded: HashSet::new(),
            changes: Vec::new(),
            ruler: Rc::new(Vec::new()),
            scroll: 0.,
            layout: Rc::new(Layout::default()),
            wrap: false,
            folded: HashSet::new(),
            fold_ends: Rc::new(Vec::new()),
            reveal_first: false,
            hover: None,
            hscroll: 0.,
            selection: None,
            pending_line: None,
            found: Rc::new(Vec::new()),
            found_at: None,
            cursor: 0,
            drag: Drag::None,
            body: Rc::new(Cell::new(Bounds::default())),
            generation: 0,
            history_file: false,
            hex: None,
            image: None,
            background: Background::Checker,
            match_picks: Rc::default(),
            restored: false,
            hunks: None,
            bar_counts: None,
            commit_held: false,
            edit: None,
            caret: None,
            reveal_caret: false,
        }
    }

    /// A file opened from another: the view's choices carry from file to file.
    pub fn carrying(file: FileChange, from: Option<&DiffState>) -> Self {
        let unchanged = file.status == UNCHANGED;
        let mut state = Self::new(file);
        if let Some(d) = from {
            // An unchanged file only has File View; the next one's as it was.
            state.view = if d.file.status == UNCHANGED { ViewKind::Diff } else { d.view };
            state.take_prefs(d);
            state.generation = d.generation + 1;
        }
        if unchanged {
            state.view = ViewKind::File;
        }
        state
    }

    /// What's the web UI's global prefs.
    pub fn take_prefs(&mut self, from: &DiffState) {
        (self.mode, self.ignore_whitespace, self.wrap, self.background) = (from.mode, from.ignore_whitespace, from.wrap, from.background);
        self.match_picks = from.match_picks.clone();
    }

    /// There's typing in it that's not saved.
    pub fn editing_dirty(&self) -> bool {
        self.edit.as_ref().is_some_and(super::edit::EditState::dirty)
    }

    /// The next read is the same file again: it stays where it is.
    pub fn keep_scroll(&mut self) {
        self.restored = true;
    }

    /// A binary is shown, not lines: it has no blame.
    pub fn shows_binary(&self) -> bool {
        self.hex.is_some() || self.image.as_ref().is_some_and(|i| !i.svg)
    }

    /// Lays out anew at the next frame: what's under the toolbar changed.
    pub fn layout_again(&mut self) {
        self.layout = Rc::new(Layout { width: -1., offsets: self.layout.offsets.clone(), ..Layout::default() });
    }

    pub fn relayout(&mut self) {
        if let Some(image) = self.image.as_mut() {
            image.view_changed();
        }
        if self.hex.is_some() {
            // `ensure_layout` lays it out anew, for this view's sides.
            self.layout = Rc::new(Layout { width: -1., ..Layout::default() });
            return;
        }
        let Some(data) = &self.data else { return };
        let rows = match self.view {
            ViewKind::File => {
                if self.fold_ends.is_empty() {
                    self.fold_ends = Rc::new(fold_ends(if data.new.len() > 0 { &data.new } else { &data.old }));
                }
                file_rows(&data.new, &data.old, &self.folded, &self.fold_ends)
            }
            ViewKind::Diff => {
                let list = rows(&data.ops, self.mode, &self.expanded);
                match self.hunks.as_ref().filter(|_| self.mode == Mode::Hunk) {
                    Some(h) => super::staging::with_headers(list, &h.hunks),
                    None => list,
                }
            }
        };
        self.changes = change_starts(&rows);
        self.ruler = Rc::new(ruler_marks(&rows));
        self.rows = Rc::new(rows);
        self.selection = None;
        self.layout = Rc::new(Layout { width: -1., ..Layout::default() });
    }

    fn split(&self) -> bool {
        self.mode == Mode::Split && self.view == ViewKind::Diff
    }

    /// The text's left edge in the body, the left pane's in a split.
    fn gutter(&self) -> f32 {
        match (self.view, self.split()) {
            (ViewKind::File, _) => NUM_W + FOLD_W,
            (_, true) => SPLIT_LEFT_MARGIN + NUM_W + SIGN_W,
            (_, false) => 2. * NUM_W + SIGN_W,
        }
    }

    /// Lays the rows out for a body `width` by `height` px, when any of that changed.
    pub fn ensure_layout(&mut self, width: f32, height: f32) {
        if let Some(line) = self.pending_line.filter(|_| self.data.is_some() && !self.rows.is_empty()) {
            self.pending_line = None;
            if let Some(row) = self.rows.iter().position(|r| r.new == Some(line)) {
                self.center_on(row);
                self.cursor = row;
            }
        }
        let same = self.layout.width == width && self.layout.height == height && self.layout.wrap == self.wrap;
        if let Some(image) = self.image.as_mut() {
            image.sync((width, height), self.view == ViewKind::File);
            if !image.source_on {
                return;
            }
        }
        if same {
            return;
        }
        if self.hex.is_some() {
            self.layout_hex(width, height);
            return;
        }
        if self.data.is_none() {
            return;
        }
        let Some(data) = self.data.clone() else { return };
        let split = self.split();
        let columns = |text: f32| (text / ADVANCE).floor().max(8.) as usize;
        let (old_cols, new_cols) = if split { (columns(width / 2. - self.gutter()), columns(width / 2. - NUM_W - SIGN_W)) } else { (columns(width - self.gutter()), columns(width - self.gutter())) };
        let mut layout = Layout { width, height, wrap: self.wrap, offsets: Vec::with_capacity(self.rows.len() + 1), ..Layout::default() };
        let mut y = 0.;
        for r in self.rows.iter() {
            layout.offsets.push(y);
            if matches!(r.kind, RowKind::Hidden { .. }) {
                y += HIDDEN_H;
                continue;
            }
            if matches!(r.kind, RowKind::HunkHeader { .. }) {
                y += super::staging::HUNK_ROW_H;
                continue;
            }
            let mut lines = 1;
            if self.wrap {
                let mut wrapped = |n: Option<u32>, side: &Side, cols: usize, map: &mut HashMap<u32, Vec<u32>>| {
                    let Some(n) = n else { return };
                    let text = side.lines[n as usize].text.as_ref();
                    if text.len() <= cols {
                        return;
                    }
                    let points = wrap_points(text, cols, text.bytes().take_while(|b| *b == b' ').count());
                    if !points.is_empty() {
                        lines = lines.max(points.len() + 1);
                        map.insert(n, points);
                    }
                };
                wrapped(r.old, &data.old, old_cols, &mut layout.breaks_old);
                wrapped(r.new, &data.new, new_cols, &mut layout.breaks_new);
            }
            y += lines as f32 * LINE_H;
        }
        layout.offsets.push(y);
        self.layout = Rc::new(layout);
        if std::mem::take(&mut self.reveal_first)
            && let Some(first) = self.changes.first()
        {
            self.center_on(*first);
        }
        self.scroll = self.scroll.clamp(0., self.layout.max_scroll());
        if std::mem::take(&mut self.reveal_caret)
            && let Some(at) = self.caret
        {
            let (top, end) = (self.layout.offsets.get(at.row).copied().unwrap_or(0.), self.layout.offsets.get(at.row + 1).copied().unwrap_or(0.));
            if top < self.scroll {
                self.scroll = top;
            } else if end > self.scroll + height {
                self.scroll = (end - height).clamp(0., self.layout.max_scroll());
            }
            // Sideways too: the caret's column stays in the text's room.
            if !self.wrap {
                let room = (if self.split() { width / 2. - NUM_W - SIGN_W } else { width - self.gutter() }).max(ADVANCE);
                let x = at.byte as f32 * ADVANCE;
                if x < self.hscroll {
                    self.hscroll = x;
                } else if x > self.hscroll + room - ADVANCE {
                    self.hscroll = x - room + ADVANCE;
                }
            }
        }
    }

    /// 16 or 8 bytes a row by the room there is, the same byte staying at the top. A new file opens at
    /// its top, or with its first change in view when that's further down.
    fn layout_hex(&mut self, width: f32, height: f32) {
        let file = self.view == ViewKind::File;
        let Some(hex) = self.hex.as_mut() else { return };
        let top_byte = (self.scroll / LINE_H) * hex.per as f32;
        let fresh = self.layout.offsets.is_empty() && self.reveal_first;
        hex.per = choose_per(hex.data.sides(file).len(), width + RULER_W);
        hex.blocks = Rc::new(if file { Vec::new() } else { super::hex::changed_rows(&hex.data.runs, hex.per) });
        self.changes = hex.blocks.iter().map(|b| b.0).collect();
        let rows = hex.rows(file);
        let offsets = (0..=rows).map(|r| r as f32 * LINE_H).collect();
        self.scroll = top_byte / hex.per as f32 * LINE_H;
        self.layout = Rc::new(Layout { width, height, wrap: self.wrap, offsets, ..Layout::default() });
        if fresh {
            self.reveal_first = false;
            self.scroll = self.changes.first().filter(|first| (**first + 1) as f32 * LINE_H > height).map_or(0., |first| super::hex::top_of(*first));
        }
        self.scroll = self.scroll.clamp(0., self.layout.max_scroll());
    }

    /// As Monaco's `revealLineInCenter`.
    pub fn center_on(&mut self, row: usize) {
        let y = self.layout.offsets.get(row).copied().unwrap_or(0.);
        self.scroll = (y - self.layout.height / 2. + LINE_H / 2.).clamp(0., self.layout.max_scroll());
    }

    pub fn scroll_by(&mut self, px: f32) {
        self.scroll = (self.scroll + px).clamp(0., self.layout.max_scroll());
    }

    pub fn set_hex(&mut self, data: Arc<HexData>) {
        self.hex = Some(HexView::new(data));
        self.notice = None;
        self.reveal_first = !std::mem::take(&mut self.restored);
        self.scroll = if self.reveal_first { 0. } else { self.scroll };
        self.layout = Rc::new(Layout { width: -1., ..Layout::default() });
    }

    pub fn set_data(&mut self, data: Arc<DiffData>) {
        self.data = Some(data);
        self.notice = None;
        self.relayout();
        // As Monaco: a diff opens with its first change in the middle.
        self.reveal_first = !std::mem::take(&mut self.restored);
    }

    /// The first change at or past the top (F7), or the last one before it (Shift+F7).
    pub fn step_change(&mut self, forward: bool, top: usize) -> Option<usize> {
        let found = if forward { self.changes.iter().find(|c| **c > top).or(self.changes.first()) } else { self.changes.iter().rev().find(|c| **c < top).or(self.changes.last()) };
        found.copied()
    }

    /// The selected text, lines joined by `\n`.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.hscroll = 0.;
        self.relayout();
    }

    /// Find's matches in what's shown, in row order.
    pub fn find_text(&self, query: &str) -> Vec<crate::find::TextMatch> {
        let Some(data) = self.data.as_ref() else { return Vec::new() };
        if query.is_empty() {
            return Vec::new();
        }
        let split = self.split();
        let mut out = Vec::new();
        for (row, r) in self.rows.iter().enumerate() {
            let sides: &[bool] = if split { &[true, false] } else { &[false] };
            for &old_side in sides {
                if let Some(line) = row_line(data, r, split, old_side) {
                    for (from, to) in crate::find::find_in_line(&line.text, query) {
                        out.push(crate::find::TextMatch { row, old_side, from, to });
                    }
                }
            }
        }
        out
    }

    /// The changed lines the selection covers, by side, 1-based. `None`: no selection, or it's empty.
    pub fn selected_changes(&self) -> Option<(Vec<u32>, Vec<u32>)> {
        let (sel, h) = (self.selection?, self.hunks.as_ref()?);
        let (from, to) = sel.range();
        if from == to {
            return None;
        }
        // A selection ending at a line's very start doesn't take that line.
        let last = if to.byte == 0 && to.row > from.row { to.row - 1 } else { to.row };
        Some(super::staging::changed_in_rows(&self.rows, from.row, last, &h.hunks, self.split().then_some(sel.old_side)))
    }

    /// Whether the selection holds row `row`.
    pub fn selection_holds(&self, row: usize) -> bool {
        self.selection.is_some_and(|s| {
            let (a, b) = s.range();
            a != b && a.row <= row && row <= b.row
        })
    }

    /// Where the gutter's button goes on a removed line and on an added one.
    pub fn glyph_columns(&self) -> (f32, f32) {
        let width = self.body.get().size.width.as_f32();
        if self.split() { (SPLIT_LEFT_MARGIN + NUM_W + 2., width / 2. + NUM_W + 2.) } else { (2. * NUM_W + 2., 2. * NUM_W + 2.) }
    }

    /// The text's left edge, for what's placed against it.
    pub fn gutter_w(&self) -> f32 {
        self.gutter()
    }

    pub fn selected_text(&self) -> Option<String> {
        let (data, sel) = (self.data.as_ref()?, self.selection?);
        let (from, to) = sel.range();
        if from == to {
            return None;
        }
        let mut out = String::new();
        for row in from.row..=to.row {
            let Some(r) = self.rows.get(row) else { break };
            let Some(line) = row_line(data, r, self.mode == Mode::Split, sel.old_side) else {
                continue;
            };
            let text = line.text.as_ref();
            let start = if row == from.row { from.byte.min(text.len()) } else { 0 };
            let end = if row == to.row { to.byte.min(text.len()) } else { text.len() };
            if !out.is_empty() || row > from.row {
                out.push('\n');
            }
            out.push_str(text.get(start..end).unwrap_or_default());
        }
        Some(out)
    }
}

impl Drop for DiffState {
    fn drop(&mut self) {
        self.stop_colours.store(true, Ordering::Relaxed);
    }
}

/// The line a selection takes from row `r`.
fn row_line<'a>(data: &'a DiffData, r: &Row, split: bool, old_side: bool) -> Option<&'a Line> {
    if matches!(r.kind, RowKind::Hidden { .. } | RowKind::HunkHeader { .. }) {
        return None;
    }
    if split {
        if old_side { r.old.map(|i| &data.old.lines[i as usize]) } else { r.new.map(|i| &data.new.lines[i as usize]) }
    } else {
        r.new.map(|i| &data.new.lines[i as usize]).or_else(|| r.old.map(|i| &data.old.lines[i as usize]))
    }
}

fn blob_text(v: &Value) -> Option<String> {
    v["text"].as_str().map(str::to_string)
}

impl GitVolt {
    /// Opens file `index` of the panel's list in the tab's diff.
    pub fn open_file(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(file) = tab.files.as_ref().and_then(|f| f.changes.get(index)).cloned() else { return };
        let same = tab.diff.as_ref().is_some_and(|d| d.file.path == file.path && d.file.origin == file.origin);
        if !same && self.ask_unsaved(move |v, cx| v.open_file(index, cx), cx) {
            return;
        }
        if self.follow_file(file.clone(), cx) {
            return;
        }
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        // A file with no history (new in the working tree) ends File History.
        if let Some(history) = tab.history.take() {
            tab.diff = tab.diff.take().or(history.saved);
        }
        tab.diff = Some(DiffState::carrying(file, tab.diff.as_ref()));
        if let Some(files) = tab.files.as_mut() {
            files.cursor = Some(crate::details::Cursor::File(index));
        }
        tab.sidebar_open = false;
        self.read_contents(false, cx);
        cx.notify();
    }

    pub fn close_file(&mut self, cx: &mut Context<Self>) {
        if self.ask_unsaved(|v, cx| v.close_file(cx), cx) {
            return;
        }
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.diff = None;
            tab.history = None;
            tab.keys = KeyArea::Graph;
            if let Some(files) = tab.files.as_mut() {
                files.cursor = None;
            }
            cx.notify();
        }
    }

    /// Reads the open file's sides, aligns them off the UI thread, then colours them.
    pub fn read_contents(&mut self, force: bool, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(diff) = &tab.diff else { return };
        let (repo, generation, ignore, history_file) = (tab.id, diff.generation, diff.ignore_whitespace, diff.history_file);
        let file = diff.file.clone();
        let (old_side, new_side) = super::edit::sides_for(&file, diff.view);
        let api = self.api.clone();
        let svg = cx.svg_renderer();
        let read = self.rt.spawn(async move {
            let (old, new) = (serde_json::from_value(old_side.clone()), serde_json::from_value(new_side.clone()));
            let (Ok(old), Ok(new)) = (old, new) else { return Err(GbError::other("This file's sides can't be read")) };
            let v = api.dispatch(Request::DiffContents { repo, path: file.path.to_string(), old, new, force, old_path: file.old_path.clone() }).await?;
            let path = file.path.to_string();
            let editable = super::edit::is_editable(&file, &new_side, &v).then(|| (v["new"]["text"].as_str().unwrap_or_default().to_string(), v["old"]["text"].as_str().map(str::to_string), v["new"]["hash"].as_str().unwrap_or_default().to_string(), new_side["worktree"].as_str().unwrap_or_default().to_string()));
            // A WIP diff's hunks, so its header rows are laid out with it.
            let wip_hunks = match &file.origin {
                crate::details::Origin::Wip { worktree, staged } if file.status != UNCHANGED && file.status != 'U' => api.dispatch(Request::WipHunks { repo, worktree: worktree.clone(), path: path.clone(), staged: *staged }).await.ok().map(|h| super::staging::WipHunks::of(&h)),
                _ => None,
            };
            // A binary's dumps load with its contents, so it's ready to show as a text file is.
            let hex = if wants_hex(&v, history_file) {
                let (Ok(old), Ok(new)) = (serde_json::from_value(old_side), serde_json::from_value(new_side)) else { return Err(GbError::other("This file's sides can't be read")) };
                Some(api.dispatch(Request::HexDump { repo, path: path.clone(), old, new }).await?)
            } else {
                None
            };
            tokio::task::spawn_blocking(move || match hex {
                Some(hex) => (Read::Binary(Binary::Hex(Arc::new(HexData::of(&hex)))), wip_hunks, editable),
                None => (contents(&path, &v, ignore, history_file, &svg), wip_hunks, editable),
            })
            .await
            .map_err(|e| GbError::other(e.to_string()))
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                let Some(diff) = this.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.diff.as_mut()).filter(|d| d.generation == generation) else { return };
                let (result, hunks, editable) = match result {
                    Ok(Ok((read, hunks, editable))) => (Ok(Ok(read)), hunks, editable),
                    Ok(Err(e)) => (Ok(Err(e)), None, None),
                    Err(e) => (Err(e), None, None),
                };
                diff.hunks = hunks;
                diff.bar_counts = None;
                // Unsaved typing stays: what's on disk meets it at the save.
                if diff.editing_dirty() {
                    return;
                }
                match (editable, diff.edit.as_mut()) {
                    (Some((text, old, base, _)), Some(edit)) => edit.reload(&text, base, old),
                    (Some((text, old, base, worktree)), None) => diff.edit = Some(super::edit::EditState::new(&text, old, base, worktree, cx)),
                    (None, _) => {
                        diff.edit = None;
                        diff.caret = None;
                    }
                }
                match result {
                    Ok(Ok(Read::Text(data, texts))) => {
                        diff.set_data(data.clone());
                        diff.sync_selection();
                        diff.stop_colours.store(true, Ordering::Relaxed);
                        diff.stop_colours = Arc::default();
                        let stop = diff.stop_colours.clone();
                        this.color(repo, generation, data, texts, stop, cx);
                    }
                    Ok(Ok(Read::Binary(Binary::Hex(data)))) => diff.set_hex(data),
                    Ok(Ok(Read::Binary(Binary::Image { old, new, svg }))) => {
                        let picked = diff.match_picks.borrow().get(diff.file.path.as_ref()).copied();
                        diff.image = Some(ImageView::new(old, new, svg.is_some(), diff.background, picked, this.retired_images.clone()));
                        diff.notice = None;
                        if let Some((data, texts)) = svg {
                            diff.set_data(data.clone());
                            let stop = diff.stop_colours.clone();
                            this.color(repo, generation, data, texts, stop, cx);
                        }
                    }
                    Ok(Ok(Read::Notice(n))) => diff.notice = Some(n),
                    Ok(Err(e)) => diff.notice = Some(Notice::Failed(e.message.into())),
                    Err(e) => diff.notice = Some(Notice::Failed(e.to_string().into())),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A raster image's dumps, for its Hex toggle.
    pub fn read_hex(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(diff) = &tab.diff else { return };
        if cfg!(test) {
            return;
        }
        let (repo, generation) = (tab.id, diff.generation);
        let file = diff.file.clone();
        let api = self.api.clone();
        let read = self.rt.spawn(async move {
            let (Ok(old), Ok(new)) = (serde_json::from_value(file.old.clone()), serde_json::from_value(file.new.clone())) else { return Err(GbError::other("This file's sides can't be read")) };
            let v = api.dispatch(Request::HexDump { repo, path: file.path.to_string(), old, new }).await?;
            tokio::task::spawn_blocking(move || HexData::of(&v)).await.map_err(|e| GbError::other(e.to_string()))
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                let Some(diff) = this.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.diff.as_mut()).filter(|d| d.generation == generation) else { return };
                match result {
                    Ok(Ok(data)) => diff.set_hex(Arc::new(data)),
                    Ok(Err(e)) => diff.notice = Some(Notice::Failed(e.message.into())),
                    Err(e) => diff.notice = Some(Notice::Failed(e.to_string().into())),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Colours both sides from their tops, taking turns, a piece at a time.
    pub fn color(&mut self, repo: u32, generation: u64, data: Arc<DiffData>, texts: Texts, stop: Arc<AtomicBool>, cx: &mut Context<Self>) {
        let (keep_old, keep_new) = (texts.old.is_none(), texts.new.is_none());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        self.rt.spawn_blocking(move || {
            let mut sides = [(colourable(&texts.path, texts.old.as_deref()), &data.old, true), (colourable(&texts.path, texts.new.as_deref()), &data.new, false)];
            while sides.iter().any(|s| s.0.as_ref().is_some_and(|t| t.2 < t.1.len())) {
                for (state, lines, old_side) in &mut sides {
                    let Some((tokenizer, raw, at)) = state.as_mut().filter(|t| t.2 < t.1.len()) else { continue };
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let end = (*at + COLOUR_CHUNK).min(raw.len());
                    let Some(spans) = tokenizer.lines(&raw[*at..end]) else {
                        *state = None;
                        continue;
                    };
                    if tx.send(Colours { old_side: *old_side, spans: lines.mapped_spans(*at, spans) }).is_err() {
                        return;
                    }
                    *at = end;
                }
            }
        });
        cx.spawn(async move |this, cx| {
            // A read again keeps what was coloured until its new colours come, piece over piece.
            let (mut old_at, mut new_at) = (0, 0);
            loop {
                let piece = rx.recv().await;
                let done = piece.is_none();
                let live = this.update(cx, |this, cx| {
                    let Some(diff) = this.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.diff.as_mut()).filter(|d| d.generation == generation) else { return false };
                    diff.coloured = true;
                    let h = Arc::make_mut(&mut diff.highlight);
                    match piece {
                        Some(piece) => {
                            let (list, at) = if piece.old_side { (&mut h.old, &mut old_at) } else { (&mut h.new, &mut new_at) };
                            let spans = Arc::new(piece.spans);
                            match list.get_mut(*at) {
                                Some(slot) => *slot = spans,
                                None => list.push(spans),
                            }
                            *at += 1;
                            cx.notify();
                        }
                        None => {
                            if !keep_old {
                                h.old.truncate(old_at);
                            }
                            if !keep_new {
                                h.new.truncate(new_at);
                            }
                            cx.notify();
                        }
                    }
                    true
                });
                if done || !matches!(live, Ok(true)) {
                    return;
                }
            }
        })
        .detach();
    }

    pub fn with_diff(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut DiffState)) {
        if let Some(diff) = self.tabs.get_mut(self.active).and_then(|t| t.diff.as_mut()) {
            f(diff);
            cx.notify();
        }
    }

    pub fn toggle_whitespace(&mut self, cx: &mut Context<Self>) {
        self.with_diff(cx, |d| {
            d.ignore_whitespace = !d.ignore_whitespace;
            d.generation += 1;
        });
        self.read_contents(false, cx);
    }

    pub fn step_change(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.with_diff(cx, |d| {
            if let Some(hex) = &d.hex {
                if let Some(row) = super::hex::step(hex, d.cursor, forward) {
                    d.cursor = row;
                    d.scroll = super::hex::top_of(row).clamp(0., d.layout.max_scroll());
                }
                return;
            }
            if let Some(row) = d.step_change(forward, d.layout.row_at(d.scroll + d.layout.height / 2.)) {
                d.center_on(row);
                d.cursor = row;
            }
        });
    }

    pub fn scroll_diff(&mut self, rows: f32, cx: &mut Context<Self>) {
        self.with_diff(cx, |d| d.scroll_by(rows * LINE_H));
    }

    pub fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let text = self.tabs.get(self.active).and_then(|t| t.diff.as_ref()).and_then(DiffState::selected_text);
        if let Some(text) = text {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.with_diff(cx, |d| {
            let last = d.rows.len().saturating_sub(1);
            d.selection = Some(Selection { anchor: TextPos { row: 0, byte: 0 }, head: TextPos { row: last, byte: usize::MAX }, old_side: false });
        });
    }
}

/// A side's tokenizer, raw lines and how far it's got; `None`: it stays plain.
fn colourable<'a>(path: &str, text: Option<&'a str>) -> Option<(Tokenizer, Vec<&'a str>, usize)> {
    let text = text?;
    Some((Tokenizer::new(highlight_language(path, text)?)?, text.split('\n').collect(), 0))
}

pub struct Texts {
    path: String,
    old: Option<String>,
    new: Option<String>,
}

impl Texts {
    /// Colours only the new side: what typing changed.
    pub fn new_side(path: String, new: String) -> Self {
        Self { path, old: None, new: Some(new) }
    }
}

enum Read {
    Text(Arc<DiffData>, Texts),
    Binary(Binary),
    Notice(Notice),
}

/// A binary that isn't shown as an image: its hex dump. An image too, in File History's File. Media
/// and PDFs, which this build can't show yet, fall here as well.
fn wants_hex(v: &Value, history_file: bool) -> bool {
    let binary = |s: &Value| s["binary"].as_bool() == Some(true);
    let any = binary(&v["old"]) || binary(&v["new"]);
    any && v["tooLarge"].as_bool() != Some(true) && (v["image"].as_bool() != Some(true) || history_file)
}

fn contents(path: &str, v: &Value, ignore_whitespace: bool, history_file: bool, svg: &SvgRenderer) -> Read {
    if v["tooLarge"].as_bool() == Some(true) {
        return Read::Notice(Notice::TooLarge);
    }
    let binary = |s: &Value| s["binary"].as_bool() == Some(true);
    if v["image"].as_bool() == Some(true) && (binary(&v["old"]) || binary(&v["new"])) {
        return Read::Binary(Binary::Image { old: decode_side(&v["old"], svg), new: decode_side(&v["new"], svg), svg: None });
    }
    let (old, new) = (blob_text(&v["old"]), blob_text(&v["new"]));
    let encoding = v["new"]["encoding"].as_str().or(v["old"]["encoding"].as_str()).unwrap_or_default().to_string();
    let data = Arc::new(DiffData::build(old.as_deref(), new.as_deref(), ignore_whitespace, encoding.into(), v["eolOnly"].as_bool() == Some(true)));
    let texts = Texts { path: path.to_string(), old, new };
    // An SVG is an image with its text behind the Source toggle; File History's File shows it as text.
    if path.to_lowercase().ends_with(".svg") && !history_file {
        return Read::Binary(Binary::Image { old: decode_side(&v["old"], svg), new: decode_side(&v["new"], svg), svg: Some((data, texts)) });
    }
    Read::Text(data, texts)
}

pub fn mono() -> Font {
    Font { family: FONT.into(), features: FontFeatures::disable_ligatures(), fallbacks: Some(FontFallbacks::from_fonts(vec!["Noto Sans Mono".into(), "monospace".into()])), weight: FontWeight::NORMAL, style: FontStyle::Normal }
}

fn run(len: usize, base: &Font, color: u32, flags: u8) -> TextRun {
    let font = Font { weight: if flags & FONT_BOLD != 0 { FontWeight::BOLD } else { FontWeight::NORMAL }, style: if flags & FONT_ITALIC != 0 { FontStyle::Italic } else { FontStyle::Normal }, ..base.clone() };
    let underline = (flags & FONT_UNDERLINE != 0).then(|| UnderlineStyle { thickness: px(1.), ..Default::default() });
    TextRun { len, font, color: rgb(color).into(), background_color: None, underline, strikethrough: None }
}

fn runs(text: &str, spans: &[Span], base: &Font) -> Vec<TextRun> {
    let mut out = Vec::with_capacity(spans.len() + 1);
    let mut at = 0usize;
    for s in spans {
        let (start, end) = ((s.start as usize).min(text.len()), (s.end as usize).min(text.len()));
        if start > at {
            out.push(run(start - at, base, TEXT, 0));
        }
        if end > start {
            out.push(run(end - start, base, s.color, s.font));
        }
        at = at.max(end);
    }
    if at < text.len() {
        out.push(run(text.len() - at, base, TEXT, 0));
    }
    out
}

/// What every row of a frame shares.
struct Painter {
    data: Arc<DiffData>,
    highlight: Arc<Highlight>,
    view: ViewKind,
    split: bool,
    hscroll: f32,
    selection: Option<Selection>,
    cursor: usize,
    font: Font,
    layout: Rc<Layout>,
    /// File View: the chevrons show while the pointer's over the gutter, a folded one always.
    fold_hover: bool,
    folded: HashSet<u32>,
    fold_ends: Rc<Vec<Option<u32>>>,
    hover: Option<usize>,
    /// Find's marks, and the one it's on.
    found: Rc<Vec<crate::find::TextMatch>>,
    found_at: Option<usize>,
    /// The editor's caret, while it has the keyboard.
    caret: Rc<Cell<Option<TextPos>>>,
}

fn shape(line: &Line, spans: &[Span], font: &Font, window: &Window) -> ShapedLine {
    window.text_system().shape_line(line.text.clone(), px(FONT_SIZE), &runs(&line.text, spans, font), None)
}

/// A visual line: bytes `from..to` of a line that wraps.
fn shape_part(text: &str, from: usize, to: usize, spans: &[Span], font: &Font, window: &Window) -> ShapedLine {
    let part = &text[from..to];
    let clipped: Vec<Span> = spans.iter().filter(|s| (s.start as usize) < to && (s.end as usize) > from).map(|s| Span { start: (s.start as usize).saturating_sub(from) as u32, end: ((s.end as usize).min(to) - from) as u32, ..*s }).collect();
    window.text_system().shape_line(part.to_string().into(), px(FONT_SIZE), &runs(part, &clipped, font), None)
}

fn paint_number(n: u32, right: Pixels, y: Pixels, color: Hsla, p: &Painter, window: &mut Window, cx: &mut App) {
    let text: SharedString = (n + 1).to_string().into();
    let line = window.text_system().shape_line(text.clone(), px(FONT_SIZE), &[TextRun { len: text.len(), font: p.font.clone(), color, background_color: None, underline: None, strikethrough: None }], None);
    line.paint(point(right - line.width, y), px(LINE_H), TextAlign::Left, None, window, cx).ok();
}

fn paint_sign(glyph: &'static str, x: Pixels, y: Pixels, color: Hsla, p: &Painter, window: &mut Window, cx: &mut App) {
    let line = window.text_system().shape_line(glyph.into(), px(FONT_SIZE), &[TextRun { len: glyph.len(), font: p.font.clone(), color, background_color: None, underline: None, strikethrough: None }], None);
    line.paint(point(x + (px(SIGN_W) - line.width) / 2., y), px(LINE_H), TextAlign::Left, None, window, cx).ok();
}

/// One pane's text from `x`, clipped to `clip`: each visual line with its guides, word marks and
/// selection under it.
#[allow(clippy::too_many_arguments)]
fn paint_text(ix: usize, n: u32, line: &Line, spans: &[Span], old_side: bool, mark: Hsla, x: Pixels, y: Pixels, clip: Bounds<Pixels>, p: &Painter, window: &mut Window, cx: &mut App) {
    let breaks = p.layout.breaks(old_side, n);
    let text = line.text.as_ref();
    let indent = text.bytes().take_while(|b| *b == b' ').count() as f32 * ADVANCE;
    window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
        for part in 0..=breaks.len() {
            let from = if part == 0 { 0 } else { breaks[part - 1] as usize };
            let to = breaks.get(part).map_or(text.len(), |b| *b as usize);
            let y = y + px(part as f32 * LINE_H);
            let start = x - px(p.hscroll) + if part == 0 { px(0.) } else { px(indent) };
            let shaped = if breaks.is_empty() { shape(line, spans, &p.font, window) } else { shape_part(text, from, to, spans, &p.font, window) };
            for level in 1..=line.guides {
                let gx = x - px(p.hscroll) + px(ADVANCE * ((usize::from(level) - 1) * TAB_SIZE) as f32);
                window.paint_quad(fill(Bounds::new(point(gx, y), size(px(1.), px(LINE_H))), theme::c(0x404040)));
            }
            for &(a, b) in &line.marks {
                let (a, b) = ((a as usize).max(from), (b as usize).min(to));
                if b > a {
                    let (l, r) = (shaped.x_for_index(a - from), shaped.x_for_index(b - from));
                    window.paint_quad(fill(Bounds::from_corners(point(start + l, y), point(start + r, y + px(LINE_H))), mark));
                }
            }
            // Monaco's colours for a match and the current one.
            let first = p.found.partition_point(|m| m.row < ix);
            for (k, m) in p.found.iter().enumerate().skip(first).take_while(|(_, m)| m.row == ix) {
                if m.old_side == old_side || !p.split {
                    let (a, b) = (m.from.max(from), m.to.min(to));
                    if b > a {
                        let (l, r) = (shaped.x_for_index(a - from), shaped.x_for_index(b - from));
                        let color = if p.found_at == Some(k) { theme::c(0x515c6a) } else { theme::a(0xea5c0055) };
                        window.paint_quad(fill(Bounds::from_corners(point(start + l, y), point(start + r, y + px(LINE_H))), color));
                    }
                }
            }
            if let Some(sel) = p.selection.filter(|s| !p.split || s.old_side == old_side) {
                let (a, b) = sel.range();
                if a.row <= ix && ix <= b.row {
                    let l = if ix == a.row { a.byte.clamp(from, to) } else { from };
                    let last = part == breaks.len();
                    let r = if ix == b.row { b.byte.clamp(from, to) } else { to };
                    let (lx, mut rx) = (shaped.x_for_index(l - from), shaped.x_for_index(r - from));
                    // A selected line's end shows, as a sliver past its text.
                    if ix != b.row && last {
                        rx += px(ADVANCE);
                    }
                    if rx > lx {
                        window.paint_quad(fill(Bounds::from_corners(point(start + lx, y), point(start + rx, y + px(LINE_H))), theme::selection_bg()));
                    }
                }
            }
            if to > from {
                shaped.paint(point(start, y), px(LINE_H), TextAlign::Left, None, window, cx).ok();
            }
            if !old_side
                && let Some(at) = p.caret.get().filter(|c| c.row == ix && c.byte >= from && (c.byte < to || part == breaks.len()))
            {
                window.paint_quad(fill(Bounds::new(point(start + shaped.x_for_index(at.byte - from), y), size(px(2.), px(LINE_H))), theme::c(0xAEAFAD)));
            }
        }
    });
}

fn spans_of(pieces: &[Arc<Vec<Vec<Span>>>], n: u32) -> &[Span] {
    let n = n as usize;
    pieces.get(n / COLOUR_CHUNK).and_then(|p| p.get(n % COLOUR_CHUNK)).map_or(&[], Vec::as_slice)
}

fn paint_inline(b: Bounds<Pixels>, ix: usize, r: &Row, p: &Painter, window: &mut Window, cx: &mut App) {
    let y = b.origin.y;
    let bottom = y + b.size.height;
    let num = theme::c(if ix == p.cursor { 0xC6C6C6 } else { 0x858585 });
    let (old_right, new_right) = (b.origin.x + px(NUM_W - 4.), b.origin.x + px(2. * NUM_W - 4.));
    let sign_x = b.origin.x + px(2. * NUM_W);
    let text_x = sign_x + px(SIGN_W);
    let clip = Bounds::from_corners(point(text_x, y), point(b.origin.x + b.size.width, bottom));
    let hl = &p.highlight;
    match r.kind {
        RowKind::Removed => {
            window.paint_quad(fill(b, theme::diff_removed_line()));
            if let Some(n) = r.old {
                paint_number(n, old_right, y, num, p, window, cx);
                paint_sign("−", sign_x, y, theme::status_deleted(), p, window, cx);
                paint_text(ix, n, &p.data.old.lines[n as usize], spans_of(&hl.old, n), true, theme::diff_removed_text(), text_x, y, clip, p, window, cx);
            }
        }
        RowKind::Added => {
            window.paint_quad(fill(Bounds::from_corners(point(b.origin.x + px(NUM_W), y), point(b.origin.x + b.size.width, bottom)), theme::diff_inserted_line()));
            if let Some(n) = r.new {
                paint_number(n, new_right, y, num, p, window, cx);
                paint_sign("+", sign_x, y, theme::status_added(), p, window, cx);
                paint_text(ix, n, &p.data.new.lines[n as usize], spans_of(&hl.new, n), false, theme::diff_inserted_text(), text_x, y, clip, p, window, cx);
            }
        }
        _ if p.view == ViewKind::File => {
            let right = b.origin.x + px(NUM_W - 4.);
            let at = b.origin.x + px(NUM_W + FOLD_W);
            let clip = Bounds::from_corners(point(at, y), point(b.origin.x + b.size.width, bottom));
            let (n, old_side) = match (r.new, r.old) {
                (Some(n), _) => (n, false),
                (None, Some(n)) => (n, true),
                _ => return,
            };
            paint_number(n, right, y, num, p, window, cx);
            let folded = p.folded.contains(&n);
            if folded || (p.fold_hover && p.fold_ends.get(n as usize).copied().flatten().is_some()) {
                let chevron = Bounds::new(point(b.origin.x + px(NUM_W + 6.), y + px(2.)), size(px(14.), px(14.)));
                window.paint_svg(chevron, if folded { "icons/chevron-right.svg" } else { "icons/chevron-down.svg" }.into(), None, TransformationMatrix::unit(), theme::c(0xC5C5C5), cx).ok();
            }
            let (line, spans, mark) = if old_side { (&p.data.old.lines[n as usize], spans_of(&hl.old, n), theme::diff_removed_text()) } else { (&p.data.new.lines[n as usize], spans_of(&hl.new, n), theme::diff_inserted_text()) };
            paint_text(ix, n, line, spans, old_side, mark, at, y, clip, p, window, cx);
            if folded {
                let x = at - px(p.hscroll) + px(line.text.chars().count() as f32 * ADVANCE + 6.);
                window.paint_quad(quad(Bounds::new(point(x, y + px(3.)), size(px(22.), px(12.))), px(2.), theme::control_hover_bg(), px(0.), transparent_black(), BorderStyle::default()));
            }
        }
        _ => {
            if let Some(n) = r.old {
                paint_number(n, old_right, y, num, p, window, cx);
            }
            if let Some(n) = r.new {
                paint_number(n, new_right, y, num, p, window, cx);
                paint_text(ix, n, &p.data.new.lines[n as usize], spans_of(&hl.new, n), false, theme::diff_inserted_text(), text_x, y, clip, p, window, cx);
            }
        }
    }
}

/// A hunk mode region's row: in the gutter what opens it, at the text how many lines it holds.
fn paint_hidden(b: Bounds<Pixels>, ix: usize, count: u32, p: &Painter, window: &mut Window, cx: &mut App) {
    window.paint_quad(fill(b, theme::panel_bg()));
    let hovered = p.hover == Some(ix);
    if hovered {
        window.set_window_cursor_style(CursorStyle::PointingHand);
    }
    let color = if hovered { theme::text_selected() } else { theme::text_normal() };
    let icon_at = Bounds::new(point(b.origin.x + px(2. * NUM_W - 16.), b.origin.y + px(5.)), size(px(14.), px(14.)));
    window.paint_svg(icon_at, "icons/unfold-vertical.svg".into(), None, TransformationMatrix::unit(), color, cx).ok();
    let text: SharedString = format!("{count} hidden lines").into();
    let run = TextRun { len: text.len(), font: window.text_style().font(), color, background_color: None, underline: None, strikethrough: None };
    let line = window.text_system().shape_line(text, px(12.), &[run], None);
    line.paint(point(b.origin.x + px(2. * NUM_W + SIGN_W), b.origin.y), b.size.height, TextAlign::Left, None, window, cx).ok();
}

fn paint_split(b: Bounds<Pixels>, ix: usize, r: &Row, p: &Painter, window: &mut Window, cx: &mut App) {
    let y = b.origin.y;
    let height = b.size.height;
    let half = b.size.width / 2.;
    let hl = &p.highlight;
    let change = r.kind == RowKind::Change;
    let pane = |left: Pixels, n: Option<u32>, old_side: bool, window: &mut Window, cx: &mut App| {
        let area = Bounds::new(point(left, y), size(half, height));
        let Some(n) = n else {
            window.paint_quad(fill(area, pattern_slash(theme::diff_diagonal(), 2., 6.)));
            return;
        };
        if change {
            window.paint_quad(fill(area, if old_side { theme::diff_removed_line() } else { theme::diff_inserted_line() }));
        }
        let (side, spans, mark) = if old_side { (&p.data.old, spans_of(&hl.old, n), theme::diff_removed_text()) } else { (&p.data.new, spans_of(&hl.new, n), theme::diff_inserted_text()) };
        let margin = if old_side { SPLIT_LEFT_MARGIN } else { 0. };
        paint_number(n, left + px(margin + NUM_W - 4.), y, theme::c(if ix == p.cursor { 0xC6C6C6 } else { 0x858585 }), p, window, cx);
        if change {
            let (glyph, color) = if old_side { ("−", theme::status_deleted()) } else { ("+", theme::status_added()) };
            paint_sign(glyph, left + px(margin + NUM_W), y, color, p, window, cx);
        }
        let text_x = left + px(margin + NUM_W + SIGN_W);
        let clip = Bounds::from_corners(point(text_x, y), point(left + half, y + height));
        paint_text(ix, n, &side.lines[n as usize], spans, old_side, mark, text_x, y, clip, p, window, cx);
    };
    pane(b.origin.x, r.old, true, window, cx);
    pane(b.origin.x + half, r.new, false, window, cx);
    window.paint_quad(fill(Bounds::new(point(b.origin.x + half, y), size(px(1.), height)), theme::section_border()));
}

/// The text position under the pointer: its row, the byte of its visual line, and the side.
fn pos_at(diff: &DiffState, position: Point<Pixels>, window: &Window) -> Option<(TextPos, bool)> {
    let (data, b) = (diff.data.as_ref()?, diff.body.get());
    let y = (position.y - b.origin.y).as_f32() + diff.scroll;
    let row = diff.layout.row_at(y.max(0.));
    let r = diff.rows.get(row)?;
    let split = diff.split();
    let (old_side, gutter) = if split {
        let half = b.size.width / 2.;
        let left = position.x < b.origin.x + half;
        (left, if left { px(diff.gutter()) } else { half + px(NUM_W + SIGN_W) })
    } else {
        (r.new.is_none(), px(diff.gutter()))
    };
    let x = (position.x - b.origin.x - gutter + px(diff.hscroll)).max(px(0.));
    let Some(line) = row_line(data, r, split, old_side) else { return Some((TextPos { row, byte: 0 }, old_side)) };
    let n = if old_side { r.old } else { r.new }.or(r.old).unwrap_or(0);
    let breaks = diff.layout.breaks(old_side, n);
    let text = line.text.as_ref();
    let part = (((y - diff.layout.offsets[row]) / LINE_H).max(0.) as usize).min(breaks.len());
    let from = if part == 0 { 0 } else { breaks[part - 1] as usize };
    let to = breaks.get(part).map_or(text.len(), |b| *b as usize);
    let indent = if part == 0 { 0. } else { text.bytes().take_while(|b| *b == b' ').count() as f32 * ADVANCE };
    let shaped = shape_part(text, from, to, &[], &mono(), window);
    Some((TextPos { row, byte: from + shaped.closest_index_for_x((x - px(indent)).max(px(0.))) }, old_side))
}

fn word_at(text: &str, byte: usize) -> (usize, usize) {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let byte = byte.min(text.len());
    let start = text[..byte].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(byte, |(i, _)| i);
    let end = text[byte..].char_indices().find(|(_, c)| !is_word(*c)).map_or(text.len(), |(i, _)| byte + i);
    (start, end)
}

/// The widest line in px, as far as a pane scrolls sideways.
fn content_width(data: &DiffData) -> f32 {
    data.widest as f32 * ADVANCE
}

fn on_mouse_down(this: &mut GitVolt, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<GitVolt>) {
    let Some(tab) = this.tabs.get_mut(this.active) else { return };
    tab.keys = KeyArea::Diff;
    let Some(diff) = tab.diff.as_mut() else { return };
    let Some(data) = diff.data.clone() else { return };
    let b = diff.body.get();
    let y = (e.position.y - b.origin.y).as_f32() + diff.scroll;
    let x = (e.position.x - b.origin.x).as_f32();
    let row = diff.layout.row_at(y.max(0.));
    // The horizontal scrollbar, along the bottom.
    let (pane, gutter) = (if diff.split() { b.size.width.as_f32() / 2. } else { b.size.width.as_f32() }, diff.gutter());
    let max_h = (content_width(&data) - (pane - gutter)).max(0.);
    if max_h > 0. && !diff.wrap && (e.position.y - b.origin.y).as_f32() >= b.size.height.as_f32() - SCROLLBAR_W && x >= gutter {
        diff.hscroll = ((x - gutter) / (pane - gutter)).clamp(0., 1.) * max_h;
        diff.drag = Drag::Horizontal;
        cx.notify();
        return;
    }
    let Some(r) = diff.rows.get(row).copied() else { return };
    if y >= diff.layout.content() {
        return;
    }
    if let RowKind::Hidden { region, .. } = r.kind {
        diff.expanded.insert(region);
        diff.relayout();
        cx.notify();
        return;
    }
    if matches!(r.kind, RowKind::HunkHeader { .. }) {
        return;
    }
    if diff.view == ViewKind::File && (NUM_W..NUM_W + FOLD_W).contains(&x) {
        let n = r.new.or(r.old).unwrap_or(0);
        if diff.fold_ends.get(n as usize).copied().flatten().is_some() {
            if !diff.folded.remove(&n) {
                diff.folded.insert(n);
            }
            diff.relayout();
            cx.notify();
        }
        return;
    }
    let Some((at, old_side)) = pos_at(diff, e.position, window) else { return };
    let line = row_line(&data, &r, diff.split(), old_side);
    let selection = match (e.click_count, line) {
        (2, Some(line)) => {
            let (a, b) = word_at(&line.text, at.byte);
            Selection { anchor: TextPos { row: at.row, byte: a }, head: TextPos { row: at.row, byte: b }, old_side }
        }
        (n, Some(line)) if n >= 3 => Selection { anchor: TextPos { row: at.row, byte: 0 }, head: TextPos { row: at.row, byte: line.text.len() }, old_side },
        _ if e.modifiers.shift => diff.selection.map_or(Selection { anchor: at, head: at, old_side }, |s| Selection { head: at, ..s }),
        _ => Selection { anchor: at, head: at, old_side },
    };
    diff.selection = Some(selection);
    diff.cursor = at.row;
    diff.drag = Drag::Text;
    let editing = diff.edit.is_some() && !old_side;
    if editing {
        this.edit_press(at, e.modifiers.shift, e.click_count, window, cx);
    }
    cx.notify();
}

/// A right-click on the text: a press inside a selection keeps it; elsewhere, the caret goes there.
fn on_text_menu(this: &mut GitVolt, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<GitVolt>) {
    let Some(diff) = this.tabs.get_mut(this.active).and_then(|t| t.diff.as_mut()) else { return };
    let Some((at, old_side)) = pos_at(diff, e.position, window) else { return };
    let line_of = |row: usize, diff: &DiffState| diff.rows.get(row).and_then(|r| r.new.or(r.old)).map_or(1, |n| n + 1);
    let (from, to) = match diff.selection.map(|s| s.range()) {
        Some((a, b)) if a != b && a.row <= at.row && at.row <= b.row => (line_of(a.row, diff), line_of(b.row, diff)),
        _ => {
            let n = line_of(at.row, diff);
            (n, n)
        }
    };
    let (file, text) = (diff.file.clone(), diff.selected_text());
    let weak = cx.weak_entity();
    let staging = this.staging_menu_rows(at.row, old_side, &weak);
    let rows = crate::ui::menu::grouped(vec![staging, this.diff_text_rows(&file, text, (from.min(to), from.max(to)), &weak)]);
    crate::ui::menu::open_context(rows, e.position, window, cx);
}

fn on_mouse_move(this: &mut GitVolt, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<GitVolt>) {
    let Some(diff) = this.tabs.get_mut(this.active).and_then(|t| t.diff.as_mut()) else { return };
    let b = diff.body.get();
    let inside = b.contains(&e.position);
    let pressed = e.pressed_button == Some(MouseButton::Left);
    match diff.drag {
        Drag::Text if pressed => {
            // Past the top or the bottom, the text comes to the pointer.
            let above = (b.origin.y - e.position.y).as_f32();
            let below = (e.position.y - b.origin.y - b.size.height).as_f32();
            if above > 0. {
                diff.scroll_by(-above.min(LINE_H * 3.));
            } else if below > 0. {
                diff.scroll_by(below.min(LINE_H * 3.));
            }
            if let Some((at, _)) = pos_at(diff, e.position, window)
                && let Some(sel) = diff.selection.as_mut()
            {
                sel.head = at;
                diff.cursor = at.row;
                if diff.edit.is_some() {
                    this.edit_drag(at, cx);
                }
                cx.notify();
            }
        }
        Drag::Horizontal if pressed => {
            let Some(data) = diff.data.clone() else { return };
            let (pane, gutter) = (if diff.split() { b.size.width.as_f32() / 2. } else { b.size.width.as_f32() }, diff.gutter());
            let x = (e.position.x - b.origin.x).as_f32();
            diff.hscroll = ((x - gutter) / (pane - gutter)).clamp(0., 1.) * (content_width(&data) - (pane - gutter)).max(0.);
            cx.notify();
        }
        _ => {
            let x = (e.position.x - b.origin.x).as_f32();
            let row = diff.layout.row_at((e.position.y - b.origin.y).as_f32() + diff.scroll);
            let hover = inside.then_some((row, x < diff.gutter()));
            // Only what hovering changes: a hidden row, and whether the chevrons show.
            let shows = |h: Option<(usize, bool)>, d: &DiffState| h.map(|(r, g)| (if d.hunks.is_some() || matches!(d.rows.get(r).map(|x| x.kind), Some(RowKind::Hidden { .. })) { Some(r) } else { None }, g && d.view == ViewKind::File));
            let changed = shows(hover, diff) != shows(diff.hover, diff);
            diff.hover = hover;
            if changed {
                cx.notify();
            }
        }
    }
}

fn text_area(diff: &DiffState, cx: &mut Context<GitVolt>) -> AnyElement {
    let Some(data) = diff.data.clone() else {
        return message(diff, cx).into_any_element();
    };
    let staging = super::staging::overlays(diff, cx);
    let split = diff.split();
    let hover_row = diff.hover.and_then(|(r, _)| matches!(diff.rows.get(r).map(|x| x.kind), Some(RowKind::Hidden { .. })).then_some(r));
    let p = Painter {
        found: diff.found.clone(),
        found_at: diff.found_at,
        data: data.clone(),
        highlight: diff.highlight.clone(),
        view: diff.view,
        split,
        hscroll: if diff.wrap { 0. } else { diff.hscroll },
        selection: diff.selection,
        cursor: diff.cursor,
        font: mono(),
        layout: diff.layout.clone(),
        fold_hover: diff.hover.is_some_and(|h| h.1),
        folded: diff.folded.clone(),
        fold_ends: diff.fold_ends.clone(),
        hover: hover_row,
        caret: Rc::new(Cell::new(None)),
    };
    let caret_cell = p.caret.clone();
    let caret_at = diff.caret;
    let editor = diff.edit.as_ref().map(|e| e.focus.clone());
    let input_entity = cx.entity();
    let rows = diff.rows.clone();
    let body_cell = diff.body.clone();
    let scroll = diff.scroll;
    let (gutter, wrap) = (diff.gutter(), diff.wrap);
    let hscroll = diff.hscroll;
    let entity = cx.entity().downgrade();
    let body = div().id("diff-body");
    let body = match diff.edit.as_ref() {
        Some(e) => edit_keys_of(body.track_focus(&e.focus), cx),
        None => body,
    };
    body
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_hidden()
        .cursor(CursorStyle::IBeam)
        .on_mouse_down(MouseButton::Left, cx.listener(on_mouse_down))
        .on_mouse_down(MouseButton::Right, cx.listener(on_text_menu))
        .on_mouse_move(cx.listener(on_mouse_move))
        .on_mouse_up(MouseButton::Left, cx.listener(selection_settled))
        .on_mouse_up_out(MouseButton::Left, cx.listener(selection_settled))
        .child(
            canvas(
                move |_, _, _| {},
                move |bounds, _, window, cx| {
                    body_cell.set(bounds);
                    if let Some(focus) = &editor {
                        caret_cell.set(caret_at.filter(|_| focus.is_focused(window)));
                        window.handle_input(focus, ElementInputHandler::new(bounds, input_entity.clone()), cx);
                    }
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        if !rows.is_empty() {
                            let first = p.layout.row_at(scroll);
                            for ix in first..rows.len() {
                                let (top, end) = (p.layout.offsets[ix], p.layout.offsets[ix + 1]);
                                if top - scroll >= bounds.size.height.as_f32() {
                                    break;
                                }
                                let b = Bounds::new(point(bounds.origin.x, bounds.origin.y + px(top - scroll)), size(bounds.size.width, px(end - top)));
                                match rows[ix].kind {
                                    RowKind::Hidden { count, .. } => paint_hidden(b, ix, count, &p, window, cx),
                                    RowKind::HunkHeader { .. } => {}
                                    _ if p.split => paint_split(b, ix, &rows[ix], &p, window, cx),
                                    _ => paint_inline(b, ix, &rows[ix], &p, window, cx),
                                }
                            }
                        }
                    });
                    let pane = if p.split { bounds.size.width.as_f32() / 2. } else { bounds.size.width.as_f32() };
                    let content = content_width(&p.data);
                    let max_h = (content - (pane - gutter)).max(0.);
                    // As Monaco's: only while the pointer is over the text, or it's scrolled.
                    if !wrap && max_h > 0. && (hscroll > 0. || bounds.contains(&window.mouse_position())) {
                        let track = pane - gutter;
                        let thumb = (track * (track / content)).max(24.);
                        let x = bounds.origin.x + px(gutter + (track - thumb) * (hscroll / max_h));
                        window.paint_quad(fill(Bounds::new(point(x, bounds.origin.y + bounds.size.height - px(SCROLLBAR_W)), size(px(thumb), px(SCROLLBAR_W))), theme::scroll_thumb()));
                    }
                    let entity = entity.clone();
                    window.on_mouse_event(move |e: &ScrollEvent, phase, window, cx| {
                        if phase != DispatchPhase::Capture || !bounds.contains(&e.position) {
                            return;
                        }
                        cx.stop_propagation();
                        let delta = e.delta.pixel_delta(window.line_height());
                        let (dx, dy) = if e.modifiers.shift { (delta.y, px(0.)) } else { (delta.x, delta.y) };
                        entity
                            .update(cx, |this, cx| {
                                this.with_diff(cx, |d| {
                                    d.scroll_by(-dy.as_f32());
                                    if !d.wrap {
                                        d.hscroll = (d.hscroll - dx.as_f32()).clamp(0., max_h);
                                    }
                                });
                            })
                            .ok();
                    });
                },
            )
            .size_full(),
        )
        .children(staging)
        .into_any_element()
}

fn edit_keys_of(el: Stateful<Div>, cx: &mut Context<GitVolt>) -> Stateful<Div> {
    let tab_key = cx.listener(|this: &mut GitVolt, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<GitVolt>| {
        if e.keystroke.key == "tab" && !e.keystroke.modifiers.modified() {
            this.edit_tab(cx);
            cx.stop_propagation();
        }
    });
    GitVolt::edit_keys_static(el.on_key_down(tab_key), cx)
}

/// The press ended: what a selection covers is known.
fn selection_settled(this: &mut GitVolt, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<GitVolt>) {
    let dragging = this.tabs.get(this.active).and_then(|t| t.diff.as_ref()).is_some_and(|d| d.drag == Drag::Text);
    this.with_diff_quiet(|d| d.drag = Drag::None);
    if dragging {
        this.read_selection_counts(cx);
        cx.notify();
    }
}

/// Dragging or pressing a strip scrolls by where the pointer is along it.
pub fn strip_events(el: Stateful<Div>, cell: Rc<Cell<Bounds<Pixels>>>, cx: &mut Context<GitVolt>) -> Stateful<Div> {
    let jump = |this: &mut GitVolt, y: Pixels, b: Bounds<Pixels>, cx: &mut Context<GitVolt>| {
        this.with_diff(cx, |d| {
            let share = ((y - b.origin.y) / b.size.height).clamp(0., 1.);
            d.scroll = d.layout.max_scroll() * share;
        });
    };
    let moved = cell.clone();
    el.on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            this.with_diff_quiet(|d| d.drag = Drag::Ruler);
            jump(this, e.position.y, cell.get(), cx);
        }),
    )
    .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
        let dragging = this.tabs.get(this.active).and_then(|t| t.diff.as_ref()).is_some_and(|d| d.drag == Drag::Ruler);
        if dragging && e.pressed_button == Some(MouseButton::Left) {
            jump(this, e.position.y, moved.get(), cx);
        }
    }))
    .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.with_diff_quiet(|d| d.drag = Drag::None)))
    .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, _| this.with_diff_quiet(|d| d.drag = Drag::None)))
}

/// The overview ruler: each change's place in the file, and the viewport.
fn ruler(diff: &DiffState, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let marks = diff.ruler.clone();
    let layout = diff.layout.clone();
    let scroll = diff.scroll;
    let cell = Rc::new(Cell::new(Bounds::<Pixels>::default()));
    let paint_cell = cell.clone();
    strip_events(div().id("diff-ruler").flex_none().w(px(RULER_W)).h_full(), cell, cx).child(
        canvas(
            |_, _, _| {},
            move |b, _, window, _| {
                paint_cell.set(b);
                let total = layout.total().max(1.);
                let scale = b.size.height.as_f32() / total;
                let half = b.size.width / 2.;
                let top_of = |row: usize| layout.offsets.get(row).copied().unwrap_or(0.);
                for &(first, rows, removed) in marks.iter() {
                    let top = top_of(first) * scale;
                    let h = ((top_of(first + rows) - top_of(first)) * scale).max(4.);
                    let x = if removed { b.origin.x } else { b.origin.x + half };
                    window.paint_quad(fill(Bounds::new(point(x, b.origin.y + px(top)), size(half, px(h))), if removed { theme::ruler_removed() } else { theme::ruler_added() }));
                }
                window.paint_quad(fill(Bounds::new(point(b.origin.x, b.origin.y + px(scroll * scale)), size(b.size.width, px(layout.height * scale).min(b.size.height))), theme::scroll_thumb()));
            },
        )
        .size_full(),
    )
}

/// File View's strip: the file in small, each character a pixel wide, and the viewport over it.
fn minimap(diff: &DiffState, width: f32, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let (data, hl, rows, layout, scroll) = (diff.data.clone(), diff.highlight.clone(), diff.rows.clone(), diff.layout.clone(), diff.scroll);
    let cell = Rc::new(Cell::new(Bounds::<Pixels>::default()));
    let paint_cell = cell.clone();
    strip_events(div().id("minimap").flex_none().w(px(width)).h_full(), cell, cx).child(
        canvas(
            |_, _, _| {},
            move |b, _, window, _| {
                paint_cell.set(b);
                let Some(data) = &data else { return };
                let height = b.size.height.as_f32();
                let shown = (height / MINIMAP_LINE) as usize;
                let view_rows = (layout.height / LINE_H).ceil() as usize;
                let top_row = layout.row_at(scroll);
                let share = if layout.max_scroll() > 0. { scroll / layout.max_scroll() } else { 0. };
                // A long file's map moves slower than the file, so it stays under the viewport.
                let first = (share * (rows.len() + view_rows).saturating_sub(shown) as f32) as usize;
                window.with_content_mask(Some(ContentMask { bounds: b }), |window| {
                    for (i, r) in rows.iter().skip(first).take(shown).enumerate() {
                        let (side, pieces, n) = match (r.new, r.old) {
                            (Some(n), _) => (&data.new, &hl.new, n),
                            (None, Some(n)) => (&data.old, &hl.old, n),
                            _ => continue,
                        };
                        let text = side.lines[n as usize].text.as_bytes();
                        let y = b.origin.y + px(i as f32 * MINIMAP_LINE);
                        let mut paint = |from: usize, to: usize, color: Hsla| {
                            let to = to.min(width as usize);
                            let mut at = from;
                            while at < to {
                                if text.get(at) == Some(&b' ') {
                                    at += 1;
                                    continue;
                                }
                                let end = (at..to).find(|k| text.get(*k) == Some(&b' ')).unwrap_or(to);
                                window.paint_quad(fill(Bounds::new(point(b.origin.x + px(at as f32), y), size(px((end - at) as f32), px(MINIMAP_LINE - 0.5))), color));
                                at = end;
                            }
                        };
                        let spans = spans_of(pieces, n);
                        if spans.is_empty() {
                            paint(0, text.len(), Hsla { a: 0.6, ..theme::c(TEXT) });
                        } else {
                            for s in spans {
                                paint(s.start as usize, (s.end as usize).min(text.len()), Hsla { a: 0.75, ..theme::c(s.color) });
                            }
                        }
                    }
                });
                let top = (top_row as f32 - first as f32) * MINIMAP_LINE;
                window.paint_quad(fill(Bounds::new(point(b.origin.x, b.origin.y + px(top)), size(b.size.width, px(view_rows as f32 * MINIMAP_LINE))), theme::scroll_thumb()));
            },
        )
        .size_full(),
    )
}

/// File View's scrollbar: a thumb, with no track.
fn scrollbar(diff: &DiffState, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let (layout, scroll) = (diff.layout.clone(), diff.scroll);
    let cell = Rc::new(Cell::new(Bounds::<Pixels>::default()));
    let paint_cell = cell.clone();
    strip_events(div().id("diff-scrollbar").flex_none().w(px(SCROLLBAR_W)).h_full(), cell, cx).child(
        canvas(
            |_, _, _| {},
            move |b, _, window, _| {
                paint_cell.set(b);
                let (height, total) = (b.size.height.as_f32(), layout.total().max(1.));
                if total <= height {
                    return;
                }
                let thumb = (height * height / total).max(20.);
                let top = (height - thumb) * (scroll / layout.max_scroll().max(1.));
                window.paint_quad(fill(Bounds::new(point(b.origin.x, b.origin.y + px(top)), size(b.size.width, px(thumb))), theme::scroll_thumb()));
            },
        )
        .size_full(),
    )
}

fn message(diff: &DiffState, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let base = div().flex().flex_col().flex_1().items_center().justify_center().gap(px(8.)).text_color(theme::text_secondary());
    match &diff.notice {
        Some(Notice::Failed(e)) => base.child(format!("Couldn't load this file: {e}")),
        Some(Notice::TooLarge) => base.child("Large file").child(
            div()
                .id("load-anyway")
                .px(px(8.))
                .py(px(2.))
                .rounded(px(4.))
                .bg(theme::control_bg())
                .hover(|d| d.bg(theme::control_hover_bg()))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.with_diff(cx, |d| d.notice = Some(Notice::Loading));
                    this.read_contents(true, cx);
                }))
                .child("Load anyway"),
        ),
        Some(Notice::Deleted) => base.child(format!("{} was deleted in this commit", diff.file.path)),
        _ => base,
    }
}

fn header(diff: &DiffState, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let dirty = diff.editing_dirty();
    let can_save = diff.edit.is_some();
    let path = diff.file.path.as_ref();
    let (dir, name) = match path.rsplit_once('/') {
        Some((d, n)) => (format!("{d}/"), n.to_string()),
        None => (String::new(), path.to_string()),
    };
    let (glyph, color) = status_icon(diff.file.status);
    let encoding = diff.data.as_ref().map(|d| d.encoding.clone()).unwrap_or_default();
    let file = diff.view == ViewKind::File;
    let binary = diff.hex.as_ref().filter(|_| diff.image.is_none()).map(|h| (h.data.summary(file), h.data.capped_note(file)));
    let image_cap = diff.hex.as_ref().filter(|_| diff.image.as_ref().is_some_and(|i| i.source_on)).and_then(|h| h.data.capped_note(file));
    div()
        .flex()
        .flex_none()
        .h(px(HEADER_H))
        .items_center()
        .gap(px(10.))
        .pl(px(12.))
        .pr(px(8.))
        .bg(theme::file_bar_bg())
        .border_b_1()
        .border_color(theme::section_border())
        .child(icon(glyph, 14., color))
        .child(div().flex().flex_1().min_w_0().overflow_hidden().child(div().flex_none().text_color(theme::text_normal()).child(dir)).child(div().flex_none().font_weight(FontWeight::BOLD).text_color(theme::text_selected()).child(name)).when(dirty, |d| d.child(div().flex_none().ml(px(6.)).mr(px(4.)).text_color(theme::orange()).child("●"))))
        .child(div().flex_none().text_size(px(11.)).text_color(theme::text_secondary()).child(encoding))
        .when_some(binary, |d, (sizes, capped)| {
            let mut summary = div().flex().flex_none().items_center().gap(px(4.)).text_size(px(12.)).text_color(theme::text_secondary()).child("Binary ·");
            for (i, size) in sizes.into_iter().enumerate() {
                if i > 0 {
                    summary = summary.child(icon("arrow-right", 12., theme::text_secondary()));
                }
                summary = summary.child(size);
            }
            d.child(summary).when_some(capped, |d, note| d.child(div().flex_none().text_size(px(12.)).text_color(theme::text_normal()).child(note)))
        })
        .when_some(image_cap, |d, note| d.child(div().flex_none().text_size(px(12.)).text_color(theme::text_normal()).child(note)))
        .when(can_save, |d| {
            d.child(crate::ui::tooltip::tip(
                format!("Save ({})", crate::commands::display_chord("Mod+S")),
                div()
                    .id("diff-save")
                    .flex_none()
                    .px(px(10.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .bg(theme::blue())
                    .text_color(theme::text_on_accent())
                    .cursor_pointer()
                    .when(dirty, |d| d.on_click(cx.listener(|this, _, _, cx| this.save_working_copy(None, cx))))
                    .child("Save"),
            ))
        })
        .child(
            div()
                .id("diff-close")
                .flex()
                .flex_none()
                .size(px(20.))
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .hover(|d| d.bg(theme::control_hover_bg()))
                .on_click(cx.listener(|this, _, _, cx| this.close_file(cx)))
                .child(icon("x", 14., theme::text_secondary())),
        )
}

pub fn toolbar(diff: &DiffState, kind: ToolbarKind, width: f32, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    use crate::ui::tooltip::tip;
    let blame_tip = format!("Show who last changed each line of {}", diff.file.path);
    let history_tip = format!("Show the commits that changed {}", diff.file.path);
    let in_diff = diff.view == ViewKind::Diff;
    let can_diff = diff.file.status != UNCHANGED && (diff.file.status != 'A' || diff.data.as_ref().is_none_or(|d| d.has_old));
    let view_button = |id: &'static str, label: &'static str, kind: ViewKind, on: bool, enabled: bool, cx: &mut Context<GitVolt>| {
        segment(id, label, on, enabled)
            .when(enabled, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    let before = this.tabs.get(this.active).and_then(|t| t.diff.as_ref()).map(|d| super::edit::sides_for(&d.file, d.view));
                    this.with_diff(cx, |d| {
                        d.view = kind;
                        d.hscroll = 0.;
                        d.relayout();
                    });
                    let after = this.tabs.get(this.active).and_then(|t| t.diff.as_ref()).map(|d| super::edit::sides_for(&d.file, d.view));
                    if before != after {
                        this.with_diff(cx, |d| {
                            d.generation += 1;
                            d.keep_scroll();
                        });
                        this.read_contents(false, cx);
                    }
                }))
            })
            .into_any_element()
    };
    let mode_button = |id: &'static str, label: &'static str, mode: Mode, cx: &mut Context<GitVolt>| {
        let enabled = in_diff && diff.hex.is_none();
        segment(id, label, diff.mode == mode, enabled)
            .when(enabled, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    this.with_diff(cx, |d| {
                        d.mode = mode;
                        d.hscroll = 0.;
                        d.relayout();
                    });
                }))
            })
            .into_any_element()
    };
    let icon_button = |id: &'static str, name: &'static str, pressed: bool, enabled: bool| {
        let color = if pressed { theme::text_selected() } else { theme::text_secondary() };
        div().id(id).flex().flex_none().size(px(22.)).items_center().justify_center().rounded(px(3.)).when(pressed, |d| d.bg(theme::selected_row())).when(enabled, |d| d.hover(|d| d.bg(theme::hover_row()))).when(!enabled, |d| d.opacity(0.4)).child(icon(name, 14., color))
    };
    let can_step = in_diff && !diff.changes.is_empty();
    let (panel, has_history) = match &kind {
        ToolbarKind::Panel { has_history, .. } => (true, *has_history),
        ToolbarKind::History(_) => (false, false),
    };
    let (views, open_in, staging) = match kind {
        ToolbarKind::History(views) => (views, None, None),
        ToolbarKind::Panel { open_in, staging, .. } => (segmented(vec![view_button("view-file", "File View", ViewKind::File, !in_diff, true, cx), view_button("view-diff", "Diff View", ViewKind::Diff, in_diff, can_diff, cx)]).into_any_element(), open_in, staging),
    };
    // File View hides what never applies there; File History keeps it in place, off.
    let diff_tools = in_diff || !panel;
    let binary = diff.shows_binary();
    let hex = diff.hex.is_some();
    // Not for an image, unless it's an SVG's Source, which is a text diff.
    let text_tools = diff.image.as_ref().is_none_or(|i| i.source_on);
    div()
        .flex()
        .flex_none()
        .h(px(TOOLBAR_H))
        .items_center()
        .gap(px(6.))
        .px(px(8.))
        .bg(theme::app_bg())
        .border_b_1()
        .border_color(theme::section_border())
        // Too narrow for three columns, the view switch isn't centred.
        .child(div().flex().when(width > TOOLBAR_NARROW, |d| d.flex_1()).children(open_in))
        .child(views)
        .child(
            div()
                .flex()
                .flex_1()
                .justify_end()
                .items_center()
                .gap(px(10.))
                .children(staging)
                .when(panel, |d| {
                    d.child(segmented(vec![
                        tip(blame_tip, segment("blame", "Blame", false, has_history && !binary).when(has_history && !binary, |d| d.on_click(cx.listener(|this, _, _, cx| this.open_history(true, cx))))).into_any_element(),
                        tip(history_tip, segment("history", "History", false, has_history).when(has_history, |d| d.on_click(cx.listener(|this, _, _, cx| this.open_history(false, cx))))).into_any_element(),
                    ]))
                })
                .when(diff_tools && text_tools, |d| {
                    d.child(
                        div()
                            .flex()
                            .gap(px(2.))
                            .child(tip(if can_step { "Previous change (Shift+F7)" } else { "" }, icon_button("change-prev", "arrow-up", false, can_step).when(can_step, |d| d.on_click(cx.listener(|this, _, _, cx| this.step_change(false, cx))))))
                            .child(tip(if can_step { "Next change (F7)" } else { "" }, icon_button("change-next", "arrow-down", false, can_step).when(can_step, |d| d.on_click(cx.listener(|this, _, _, cx| this.step_change(true, cx)))))),
                    )
                    .child(segmented(vec![mode_button("mode-hunk", "Hunk", Mode::Hunk, cx), mode_button("mode-inline", "Inline", Mode::Inline, cx), mode_button("mode-split", "Split", Mode::Split, cx)]))
                })
                .when(text_tools, |d| {
                    d.child(
                    div()
                        .flex()
                        .gap(px(2.))
                        .when(diff_tools, |d| d.child(icon_button("ignore-ws", "pilcrow", diff.ignore_whitespace, in_diff && !hex).when(in_diff && !hex, |d| d.on_click(cx.listener(|this, _, _, cx| this.toggle_whitespace(cx))))))
                        .child(tip("Word wrap", icon_button("wrap", "text-wrap", diff.wrap, !hex).when(!hex, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| {
                                this.with_diff(cx, |d| {
                                    d.wrap = !d.wrap;
                                    d.hscroll = 0.;
                                });
                            }))
                        }))),
                    )
                }),
        )
}

/// The text's body in a panel `width` by `height` px: what `ensure_layout` lays out.
pub fn body_size(diff: &DiffState, width: f32, height: f32) -> (f32, f32) {
    // `height`: what's under the panel's header.
    if let Some(image) = diff.image.as_ref().filter(|i| !i.source_on) {
        let _ = image;
        return (width, height - TOOLBAR_H - IMAGE_TOOLBAR_H);
    }
    let height = height - if diff.image.is_some() { IMAGE_TOOLBAR_H } else { 0. };
    if diff.hex.is_some() {
        return (width - RULER_W, height - TOOLBAR_H);
    }
    let right = match diff.view {
        ViewKind::File => minimap_width(width) + SCROLLBAR_W,
        ViewKind::Diff if diff.split() => RULER_W,
        ViewKind::Diff => RULER_W + SCROLLBAR_W,
    };
    (width - right, height - TOOLBAR_H - if diff.data.as_ref().is_some_and(|d| d.eol_only) { 26. } else { 0. })
}

fn minimap_width(width: f32) -> f32 {
    (width * 0.0975).min(MINIMAP_MAX).floor()
}

/// The panel for `tab`'s open file, `width` px wide.
pub fn diff_panel(tab: &Tab, width: f32, last_opener: Option<&str>, undo: Option<&gitvolt_core::write::types::StagingUndoState>, window: &Window, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let diff = tab.diff.as_ref()?;
    let staging = tab.wip_ctx().map(|ctx| crate::wip::staging_buttons(("diff-staging-undo", "diff-staging-redo"), ctx, undo, tab.wip.committing, cx).into_any_element());
    let open_in = crate::open_in::open_in_button(tab, last_opener, cx);
    let find = tab.find.open.then(|| crate::find::find_box(&tab.find, window, cx));
    Some(div().relative().flex().flex_col().flex_1().min_w_0().min_h_0().bg(theme::app_bg()).children(find).child(header(diff, cx)).child(toolbar(diff, ToolbarKind::Panel { has_history: crate::history::history_start(&diff.file, tab.commit.as_ref().map_or(&[][..], |c| c.parents.as_slice()), &tab.worktree).is_some(), open_in, staging }, width, cx)).child(diff_body(diff, width, &tab.zoom_input, cx)).into_any_element())
}

/// What's under the toolbar, in an editor `width` px wide.
pub fn diff_body(diff: &DiffState, width: f32, zoom_input: &Entity<crate::ui::input::TextInput>, cx: &mut Context<GitVolt>) -> AnyElement {
    let file = diff.view == ViewKind::File;
    if let Some(image) = &diff.image {
        let source = match (image.source_on, image.svg, &diff.hex) {
            (false, ..) => None,
            (_, true, _) => Some(text_body(diff, width, cx)),
            (_, _, Some(hex)) => Some(hex_body(diff, hex, cx)),
            _ => Some(message(diff, cx).into_any_element()),
        };
        return image_body(image, file, source, zoom_input, cx);
    }
    if let Some(hex) = &diff.hex {
        return hex_body(diff, hex, cx);
    }
    text_body(diff, width, cx)
}

fn text_body(diff: &DiffState, width: f32, cx: &mut Context<GitVolt>) -> AnyElement {
    let banner = diff.data.as_ref().is_some_and(|d| d.eol_only);
    let has_text = diff.data.is_some();
    let file_view = diff.view == ViewKind::File;
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .when_some(diff.hunks.as_ref().and_then(|h| h.refused.clone()), |d, why| d.child(div().flex_none().px(px(12.)).py(px(4.)).bg(theme::yellow_banner()).text_color(theme::text_selected()).child(why)))
        .when(banner, |d| d.child(div().flex_none().h(px(26.)).px(px(12.)).py(px(4.)).bg(theme::yellow_banner()).text_color(theme::text_selected()).child("Only the line endings changed")))
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(text_area(diff, cx))
                .when(has_text && file_view, |d| d.child(minimap(diff, minimap_width(width), cx)).child(scrollbar(diff, cx)))
                .when(has_text && !file_view && !diff.split(), |d| d.child(div().flex_none().w(px(SCROLLBAR_W)).h_full().border_l_1().border_color(theme::section_border())))
                .when(has_text && !file_view, |d| d.child(ruler(diff, cx))),
        )
        .into_any_element()
}

impl GitVolt {
    fn with_diff_quiet(&mut self, f: impl FnOnce(&mut DiffState)) {
        if let Some(diff) = self.tabs.get_mut(self.active).and_then(|t| t.diff.as_mut()) {
            f(diff);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn change() -> FileChange {
        FileChange { path: "a.rs".into(), old_path: None, status: 'M', additions: None, deletions: None, old: Value::Null, new: Value::Null, conflict: None, origin: crate::details::Origin::Commit { id: "c".into() } }
    }

    fn state(old: &str, new: &str) -> DiffState {
        let mut d = DiffState::new(change());
        d.set_data(Arc::new(DiffData::build(Some(old), Some(new), false, "UTF-8".into(), false)));
        d
    }

    #[test]
    fn runs_cover_the_whole_line_with_the_default_colour_in_the_gaps() {
        let font = mono();
        let spans = [Span { start: 2, end: 4, color: 0x123456, font: 0 }];
        let r = runs("abcdef", &spans, &font);
        assert_eq!(r.iter().map(|x| x.len).collect::<Vec<_>>(), [2, 2, 2]);
        assert_eq!(runs("abc", &[], &font).iter().map(|x| x.len).sum::<usize>(), 3);
    }

    #[test]
    fn a_selection_copies_its_lines_from_the_shown_side() {
        let mut d = state("one\ntwo\nthree", "one\nTWO\nthree");
        // Rows: one, two (removed), TWO (added), three.
        d.selection = Some(Selection { anchor: TextPos { row: 2, byte: 1 }, head: TextPos { row: 3, byte: 2 }, old_side: false });
        assert_eq!(d.selected_text().as_deref(), Some("WO\nth"));
        d.selection = Some(Selection { anchor: TextPos { row: 3, byte: 2 }, head: TextPos { row: 2, byte: 1 }, old_side: false });
        assert_eq!(d.selected_text().as_deref(), Some("WO\nth"), "backwards too");
        d.selection = Some(Selection { anchor: TextPos { row: 1, byte: 0 }, head: TextPos { row: 1, byte: 0 }, old_side: false });
        assert_eq!(d.selected_text(), None, "nothing selected");
    }

    #[test]
    fn split_copies_from_the_side_it_started_on() {
        let mut d = state("one\ntwo", "one\nTWO");
        d.mode = Mode::Split;
        d.relayout();
        d.selection = Some(Selection { anchor: TextPos { row: 1, byte: 0 }, head: TextPos { row: 1, byte: 3 }, old_side: true });
        assert_eq!(d.selected_text().as_deref(), Some("two"));
    }

    #[test]
    fn stepping_changes_wraps_around() {
        let mut d = state("a\nb\nc\nd\ne\nf\ng\nh\ni\nj", "A\nb\nc\nd\ne\nf\ng\nh\ni\nJ");
        assert_eq!(d.changes, [0, 10]);
        assert_eq!(d.step_change(true, 0), Some(10));
        assert_eq!(d.step_change(true, 10), Some(0), "past the last");
        assert_eq!(d.step_change(false, 5), Some(0));
        assert_eq!(d.step_change(false, 0), Some(10), "before the first");
    }

    #[test]
    fn file_view_lists_the_new_side_and_has_no_changes() {
        let mut d = state("a\nb", "a\nc\nd");
        d.view = ViewKind::File;
        d.relayout();
        assert_eq!(d.rows.len(), 3);
        assert!(d.changes.is_empty());
    }

    #[test]
    fn an_added_file_has_no_old_side() {
        let data = DiffData::build(None, Some("x\ny"), false, "UTF-8".into(), false);
        assert!(!data.has_old);
        assert_eq!(rows(&data.ops, Mode::Inline, &HashSet::new()).iter().filter(|r| r.kind == RowKind::Added).count(), 2);
    }

    #[test]
    fn a_hidden_row_is_taller_and_wrapped_rows_take_their_lines() {
        let old: String = (0..30).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l15\n", "changed\n");
        let mut d = state(&old, &new);
        d.mode = Mode::Hunk;
        d.relayout();
        d.ensure_layout(600., 200.);
        let hidden = d.rows.iter().position(|r| matches!(r.kind, RowKind::Hidden { .. })).unwrap();
        assert_eq!(hidden, 0);
        assert_eq!((d.layout.offsets[0], d.layout.offsets[1], d.layout.offsets[2]), (0., HIDDEN_H, HIDDEN_H + LINE_H));
        assert_eq!((d.layout.row_at(0.), d.layout.row_at(HIDDEN_H - 1.), d.layout.row_at(HIDDEN_H)), (0, 0, 1));

        let long = "word ".repeat(100);
        let mut d = state(&format!("a\n{long}\nb"), &format!("a\n{long}\nb"));
        d.ensure_layout(600., 200.);
        assert_eq!(d.layout.content(), 3. * LINE_H);
        d.wrap = true;
        d.ensure_layout(600., 200.);
        let lines = 1 + d.layout.breaks(false, 1).len();
        assert!(lines > 1);
        assert_eq!(d.layout.content(), (2 + lines) as f32 * LINE_H);
        assert_eq!(d.layout.row_at(LINE_H * 2.), 1, "a continuation belongs to its row");
    }

    #[test]
    fn a_diff_opens_with_its_first_change_centred_and_scroll_stays_in_range() {
        let old: String = (0..100).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l50\n", "changed\n");
        let mut d = state(&old, &new);
        d.ensure_layout(600., 180.);
        assert_eq!(d.scroll, 50. * LINE_H - 90. + LINE_H / 2., "row 50 in the middle");
        d.scroll_by(-1e6);
        assert_eq!(d.scroll, 0.);
        d.scroll_by(1e6);
        assert_eq!(d.scroll, d.layout.content() - LINE_H, "the last line can reach the top");
    }

    #[test]
    fn folding_a_line_in_file_view_takes_out_what_is_under_it() {
        let mut d = state("x", "a {\n    b\n    c\n}");
        d.view = ViewKind::File;
        d.relayout();
        assert_eq!(d.rows.len(), 4);
        d.folded.insert(0);
        d.relayout();
        assert_eq!(d.rows.iter().filter_map(|r| r.new).collect::<Vec<_>>(), [0, 3]);
    }

    #[test]
    fn a_double_click_takes_the_word_under_it() {
        assert_eq!(word_at("let some_name = 1", 6), (4, 13));
        assert_eq!(word_at("a b", 1), (0, 1));
        assert_eq!(word_at("", 0), (0, 0));
    }

    #[test]
    fn a_hex_view_lays_out_by_width_and_opens_on_its_first_change_when_that_is_further_down() {
        let bytes = |at: Option<usize>| serde_json::json!({ "size": 2048, "shown": 2048, "dump": (0..128).map(|r| format!("{:08x}  {}\n", r * 16, (0..16).map(|j| format!("{:02x}{}", if Some(r * 16 + j) == at { 1 } else { 0 }, if j == 7 { "  " } else { " " })).collect::<String>())).collect::<String>() });
        let data = Arc::new(HexData::of(&serde_json::json!({ "old": bytes(None), "new": bytes(Some(1000)), "cap": 262_144 })));
        let mut d = DiffState::new(change());
        d.set_hex(data);
        d.ensure_layout(1128., 600.);
        let hex = d.hex.as_ref().unwrap();
        assert_eq!((hex.per, hex.rows(false)), (8, 256), "too narrow for 16 a row beside each other");
        assert_eq!(d.changes, [125]);
        assert_eq!(d.scroll, 122. * LINE_H, "three rows above the change");
        // Wider: the same byte stays at the top.
        d.ensure_layout(1400., 600.);
        assert_eq!(d.hex.as_ref().unwrap().per, 16);
        assert_eq!(d.scroll, 61. * LINE_H);
        // One side in File View, with nothing to step through.
        d.view = ViewKind::File;
        d.relayout();
        d.ensure_layout(1400., 600.);
        assert!(d.changes.is_empty());
    }

    struct NoAssets;

    impl AssetSource for NoAssets {
        fn load(&self, _: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
            Ok(None)
        }

        fn list(&self, _: &str) -> Result<Vec<SharedString>> {
            Ok(Vec::new())
        }
    }

    fn cx_free_svg_renderer() -> SvgRenderer {
        SvgRenderer::new(Arc::new(NoAssets))
    }

    #[test]
    fn what_a_sides_read_says_decides_what_shows() {
        let svg = cx_free_svg_renderer();
        let notice = |v: Value| match contents("a.rs", &v, false, false, &svg) {
            Read::Notice(n) => Some(n),
            Read::Text(..) | Read::Binary(_) => None,
        };
        assert_eq!(notice(serde_json::json!({"tooLarge": true})), Some(Notice::TooLarge));
        assert!(wants_hex(&serde_json::json!({"new": {"binary": true}}), false));
        assert!(!wants_hex(&serde_json::json!({"new": {"binary": true}, "image": true}), false), "an image shows as one");
        assert!(wants_hex(&serde_json::json!({"new": {"binary": true}, "image": true}), true), "but as hex in File History's File");
        assert!(!wants_hex(&serde_json::json!({"new": {"binary": true}, "tooLarge": true}), false));
        assert_eq!(notice(serde_json::json!({"old": {"text": "a"}, "new": {"text": "b", "encoding": "UTF-8"}})), None);
    }
}
