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

const SIGN_W: f32 = 18.;
/// File View's column between the numbers and the text: the fold chevrons.
const FOLD_W: f32 = 26.;
/// JetBrains Mono's advance at `crate::metrics::editor_font_size()`; lines wrap by it.
/// Monaco's hidden-lines row.
const HIDDEN_H: f32 = 24.;
/// The minimap's scale: a character is 1 px wide, a line 2 px high.
const MINIMAP_LINE: f32 = 2.;
const MINIMAP_MAX: f32 = 120.;
const SCROLLBAR_W: f32 = 10.;
pub const RULER_W: f32 = 30.;
/// A split's left pane has more margin before its text than the right one (Monaco's).
pub const SPLIT_LEFT_MARGIN: f32 = 18.;
const TOOLBAR_H: f32 = 33.;
const TOOLBAR_NARROW: f32 = 900.;
pub const FONT: &str = "JetBrains Mono";

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
    /// A Markdown file's two texts, for its rendered views.
    pub raw: Option<(Arc<str>, Arc<str>)>,
}

/// How many bytes `a` and `b` start with alike, compared a block at a time.
fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    const BLOCK: usize = 128;
    let blocks = a.as_chunks::<BLOCK>().0.iter().zip(b.as_chunks::<BLOCK>().0).take_while(|(x, y)| x == y).count() * BLOCK;
    blocks + a[blocks..].iter().zip(&b[blocks..]).take_while(|(x, y)| x == y).count()
}

/// How many bytes `a` and `b` end with alike.
fn common_prefix_rev(a: &[u8], b: &[u8]) -> usize {
    const BLOCK: usize = 128;
    let blocks = a.as_rchunks::<BLOCK>().1.iter().rev().zip(b.as_rchunks::<BLOCK>().1.iter().rev()).take_while(|(x, y)| x == y).count() * BLOCK;
    blocks + a[..a.len() - blocks].iter().rev().zip(b[..b.len() - blocks].iter().rev()).take_while(|(x, y)| x == y).count()
}

impl DiffData {
    pub fn build(old: Option<&str>, new: Option<&str>, ignore_whitespace: bool, encoding: SharedString, eol_only: bool) -> Self {
        let (old_raw, new_raw) = (split_lines(old), split_lines(new));
        let ops = line_ops(&old_raw, &new_raw, ignore_whitespace);
        let (mut a, mut b) = (Side::new(old), Side::new(new));
        mark_words(&ops, &old_raw, &new_raw, &mut a, &mut b);
        let widest = a.lines.iter().chain(&b.lines).map(|l| l.text.len()).max().unwrap_or(0);
        Self { old: a, new: b, ops, has_old: old.is_some(), encoding, eol_only, widest, raw: None }
    }

    /// After typing: what `build` makes of `after`, from `self`, built of `before` against the same `old`. The two texts
    /// differ in one stretch, found by their common start and end; only its lines are made anew, then the lines are
    /// compared and marked again. On a big file, making every line of both sides was most of a keystroke's cost.
    pub fn rebuild(mut self, old: Option<&str>, before: &str, after: &str, ignore_whitespace: bool) -> Self {
        if before.is_empty() || after.is_empty() {
            return Self::build(old, Some(after), ignore_whitespace, self.encoding, self.eol_only);
        }
        let (old_raw, new_raw) = (split_lines(old), split_lines(Some(after)));
        let (b, a) = (before.as_bytes(), after.as_bytes());
        let prefix = common_prefix(b, a);
        let suffix = common_prefix_rev(&b[prefix..], &a[prefix..]);
        // A line's index is the line breaks before it: the changed bytes run from the first line to the last, inclusive.
        let breaks = |bytes: &[u8]| memchr::memchr_iter(b'\n', bytes).count();
        let first = breaks(&b[..prefix]);
        let (last_before, last_after) = (first + breaks(&b[prefix..b.len() - suffix]), first + breaks(&a[prefix..a.len() - suffix]));
        clear_marks(&self.ops, &mut self.old, &mut self.new);
        self.new.splice(first, last_before + 1, &new_raw[first..=last_after]);
        self.ops = line_ops(&old_raw, &new_raw, ignore_whitespace);
        mark_words(&self.ops, &old_raw, &new_raw, &mut self.old, &mut self.new);
        self.widest = self.old.lines.iter().chain(&self.new.lines).map(|l| l.text.len()).max().unwrap_or(0);
        self.raw = None;
        self
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
    /// An LFS file that isn't downloaded: its sides.
    Lfs(Vec<crate::lfs::LfsSide>),
}

pub enum Binary {
    Hex(Arc<HexData>),
    /// A raster image, or an SVG with its text's diff for its Source toggle.
    Image { old: ImageSide, new: ImageSide, svg: Option<(Arc<DiffData>, Texts)> },
    /// A PDF's bytes and sizes, by side.
    Pdf { old: Option<(Arc<Vec<u8>>, u64)>, new: Option<(Arc<Vec<u8>>, u64)>, binary: bool, encoding: SharedString },
    /// Audio, video or a font, by side.
    Media(Box<(Option<crate::media::MediaSide>, Option<crate::media::MediaSide>)>),
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

/// What a change of mode or a toggle keeps in place (`scrollAnchor.ts`): the first change the open showed, the top or the bottom
/// when it's scrolled to one, or the line at the viewport's centre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollAnchor {
    First,
    Top,
    Bottom,
    Line { new: Option<u32>, old: Option<u32>, fraction: f32 },
}

/// Monaco's own step: 50 px a notch, where the page's is 53 (`zoom::wheel_delta`).
pub const MONACO_WHEEL: f32 = 50. / 53.;

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
        self.content() + (self.height - crate::metrics::line_h()).max(0.)
    }

    pub fn max_scroll(&self) -> f32 {
        (self.total() - self.height).max(0.)
    }

    fn breaks(&self, old_side: bool, n: u32) -> &[u32] {
        if old_side { &self.breaks_old } else { &self.breaks_new }.get(&n).map_or(&[], Vec::as_slice)
    }
}

/// The open file of a tab.
/// What review mode's gutter and the frame need of the session.
pub struct ReviewState {
    pub old: std::collections::HashSet<u32>,
    pub new: std::collections::HashSet<u32>,
    pub icons: HashMap<(bool, u32), Vec<gitvolt_core::forge::types::ForgeDiscussion>>,
    /// Where a folded thread's range starts, by its icon.
    pub icon_starts: HashMap<(bool, u32), u32>,
    pub banner: Option<String>,
    pub tab: u32,
}

pub struct DiffState {
    /// The place `set_mode` and the toggles keep (`setDiffPrefs`), until the next layout.
    pub pending_anchor: Option<ScrollAnchor>,
    /// Where the open's reveal of the first change left it: while it's still there, a change of mode reveals it again.
    pub first_change_at: Option<f32>,
    /// Review mode: its cards' places among the rows, what they measured, and the elements built for this frame.
    pub zones: Vec<crate::forge::review::zones::ZoneSlot>,
    pub zone_heights: Rc<std::cell::RefCell<HashMap<String, f32>>>,
    pub zone_elements: Rc<std::cell::RefCell<Vec<(String, AnyElement)>>>,
    pub review: Option<ReviewState>,
    pub review_drag: Option<(bool, u32, u32)>,
    /// A folded thread's icon under the pointer: its side and line, whose range shows.
    pub review_icon_hover: Option<(bool, u32)>,
    pub ruler_hover: bool,
    /// The strip that's held (`Drag::Ruler`), the slider's point the press took, and whether it's the minimap's.
    pub strip_held: Option<(Bounds<Pixels>, f32, bool)>,
    pub file: FileChange,
    pub view: ViewKind,
    pub mode: Mode,
    pub ignore_whitespace: bool,
    pub notice: Option<Notice>,
    /// An LFS file's Show pointer, and its Download running.
    pub lfs_pointer: bool,
    pub lfs_busy: bool,
    pub lfs_no_git: bool,
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
    /// The pointer's over File View's minimap: its slider shows.
    pub minimap_hover: bool,
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
    /// A PDF's pages.
    pub pdf: Option<crate::pdf::PdfView>,
    /// Audio, video or a font.
    pub media: Option<crate::media::MediaView>,
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
    /// A Markdown file: its view switch and what shows it.
    pub markdown: crate::md::files::MdPick,
    pub md: Option<crate::md::files::MdPane>,
    /// A heading to go to, once the rendered view has it.
    pub md_anchor: Option<String>,
    /// How wide the body last was.
    pub md_width: Option<f32>,
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
            lfs_pointer: false,
            lfs_busy: false,
            lfs_no_git: false,
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
            pending_anchor: None,
            first_change_at: None,
            hover: None,
            hscroll: 0.,
            selection: None,
            zones: Vec::new(),
            zone_heights: Default::default(),
            zone_elements: Default::default(),
            review: None,
            review_drag: None,
            review_icon_hover: None,
            pending_line: None,
            found: Rc::new(Vec::new()),
            found_at: None,
            minimap_hover: false,
            ruler_hover: false,
            strip_held: None,
            cursor: 0,
            drag: Drag::None,
            body: Rc::new(Cell::new(Bounds::default())),
            generation: 0,
            history_file: false,
            hex: None,
            image: None,
            pdf: None,
            media: None,
            background: Background::Checker,
            match_picks: Rc::default(),
            restored: false,
            hunks: None,
            bar_counts: None,
            commit_held: false,
            edit: None,
            caret: None,
            reveal_caret: false,
            markdown: crate::md::files::MdPick::Rendered,
            md: None,
            md_anchor: None,
            md_width: None,
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
        (self.mode, self.ignore_whitespace, self.wrap, self.background, self.markdown) = (from.mode, from.ignore_whitespace, from.wrap, from.background, from.markdown);
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
        self.hex.is_some() || self.image.as_ref().is_some_and(|i| !i.svg) || self.pdf.as_ref().is_some_and(|p| p.binary) || self.media.is_some()
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
                let list = match self.hunks.as_ref().filter(|_| self.mode == Mode::Hunk) {
                    Some(h) => super::staging::with_headers(list, &h.hunks),
                    None => list,
                };
                crate::forge::review::zones::with_zones(list, &self.zones, self.mode == Mode::Split)
            }
        };
        self.changes = change_starts(&rows);
        self.ruler = Rc::new(ruler_marks(&rows));
        // The caret's on a line, wherever the new rows show it.
        if let Some(was) = self.rows.get(self.cursor).copied() {
            let shows = |r: &Row| !matches!(r.kind, RowKind::Hidden { .. } | RowKind::Zone { .. }) && if was.new.is_some() { r.new == was.new } else { r.old == was.old };
            if let Some(at) = rows.iter().position(shows) {
                self.cursor = at;
            }
        }
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
            (ViewKind::File, _) => num_w() + FOLD_W,
            (_, true) => SPLIT_LEFT_MARGIN + num_w() + SIGN_W,
            (_, false) => 2. * num_w() + SIGN_W,
        }
    }

    /// Lays the rows out for a body `width` by `height` px, when any of that changed.
    /// The sizes changed: the next frame lays it out again.
    pub fn invalidate_layout(&mut self) {
        self.layout = Rc::new(Layout::default());
    }

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
        if self.pdf.as_ref().is_some_and(|p| !p.source_on) || self.media.as_ref().is_some_and(|m| !m.source_on) {
            return;
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
        let columns = |text: f32| (text / crate::metrics::advance()).floor().max(8.) as usize;
        let (old_cols, new_cols) = if split { (columns(width / 2. - self.gutter()), columns(width / 2. - num_w() - SIGN_W)) } else { (columns(width - self.gutter()), columns(width - self.gutter())) };
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
            if let RowKind::Zone { zone } = r.kind {
                y += self.zones.get(zone as usize).map_or(0., |z| z.height);
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
            y += lines as f32 * crate::metrics::line_h();
        }
        layout.offsets.push(y);
        self.layout = Rc::new(layout);
        if std::mem::take(&mut self.reveal_first)
            && let Some(first) = self.changes.first().copied()
        {
            self.center_on(first);
            self.first_change_at = Some(self.scroll);
            // Monaco's caret goes with it: the first line of the change on the new side.
            if let Some(at) = self.rows[first..].iter().position(|r| r.new.is_some()) {
                self.cursor = first + at;
            }
        }
        if let Some(anchor) = self.pending_anchor.take() {
            self.restore_anchor(anchor);
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
                let room = (if self.split() { width / 2. - num_w() - SIGN_W } else { width - self.gutter() }).max(crate::metrics::advance());
                let x = at.byte as f32 * crate::metrics::advance();
                if x < self.hscroll {
                    self.hscroll = x;
                } else if x > self.hscroll + room - crate::metrics::advance() {
                    self.hscroll = x - room + crate::metrics::advance();
                }
            }
        }
    }

    /// 16 or 8 bytes a row by the room there is, the same byte staying at the top. A new file opens at
    /// its top, or with its first change in view when that's further down.
    fn layout_hex(&mut self, width: f32, height: f32) {
        let file = self.view == ViewKind::File;
        let Some(hex) = self.hex.as_mut() else { return };
        let top_byte = (self.scroll / crate::metrics::line_h()) * hex.per as f32;
        let fresh = self.layout.offsets.is_empty() && self.reveal_first;
        hex.per = choose_per(hex.data.sides(file).len(), width + RULER_W);
        hex.blocks = Rc::new(if file { Vec::new() } else { super::hex::changed_rows(&hex.data.runs, hex.per) });
        self.changes = hex.blocks.iter().map(|b| b.0).collect();
        let rows = hex.rows(file);
        let offsets = (0..=rows).map(|r| r as f32 * crate::metrics::line_h()).collect();
        self.scroll = top_byte / hex.per as f32 * crate::metrics::line_h();
        self.layout = Rc::new(Layout { width, height, wrap: self.wrap, offsets, ..Layout::default() });
        if fresh {
            self.reveal_first = false;
            self.scroll = self.changes.first().filter(|first| (**first + 1) as f32 * crate::metrics::line_h() > height).map_or(0., |first| super::hex::top_of(*first));
        }
        self.scroll = self.scroll.clamp(0., self.layout.max_scroll());
    }

    /// As Monaco's `revealLineInCenter`.
    pub fn center_on(&mut self, row: usize) {
        let y = self.layout.offsets.get(row).copied().unwrap_or(0.);
        self.scroll = (y - self.layout.height / 2. + crate::metrics::line_h() / 2.).clamp(0., self.layout.max_scroll());
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
        self.keep_place();
        self.mode = mode;
        self.hscroll = 0.;
        self.relayout();
    }

    /// `captureAnchor`: what the next layout puts back.
    pub fn keep_place(&mut self) {
        if self.pending_anchor.is_some() || self.layout.offsets.is_empty() {
            return;
        }
        let (top, height) = (self.scroll, self.layout.height);
        self.pending_anchor = Some(if self.first_change_at == Some(top) {
            ScrollAnchor::First
        } else if top <= 0. {
            ScrollAnchor::Top
        } else if top + height >= self.layout.content() - 1. {
            ScrollAnchor::Bottom
        } else {
            let y = top + height / 2.;
            let row = self.layout.row_at(y);
            let (from, to) = (self.layout.offsets[row], self.layout.offsets.get(row + 1).copied().unwrap_or(y));
            let r = self.rows.get(row);
            ScrollAnchor::Line { new: r.and_then(|r| r.new), old: r.and_then(|r| r.old), fraction: if to > from { (y - from) / (to - from) } else { 0. } }
        });
    }

    /// `restoreAnchor`, on the new rows: a line a mode hides goes to the first one shown after it.
    fn restore_anchor(&mut self, anchor: ScrollAnchor) {
        let height = self.layout.height;
        match anchor {
            ScrollAnchor::First => {
                if let Some(first) = self.changes.first() {
                    self.center_on(*first);
                    self.first_change_at = Some(self.scroll);
                }
            }
            ScrollAnchor::Top => self.scroll = 0.,
            ScrollAnchor::Bottom => self.scroll = (self.layout.content() - height).clamp(0., self.layout.max_scroll()),
            ScrollAnchor::Line { new, old, fraction } => {
                let exact = self.rows.iter().position(|r| (new.is_some() && r.new == new) || (new.is_none() && old.is_some() && r.old == old));
                let row = exact.or_else(|| self.rows.iter().position(|r| match (new, r.new, old, r.old) {
                    (Some(a), Some(b), _, _) => b >= a,
                    (_, _, Some(a), Some(b)) => b >= a,
                    _ => false,
                }));
                if let Some(row) = row {
                    let (from, to) = (self.layout.offsets[row], self.layout.offsets.get(row + 1).copied().unwrap_or(0.));
                    let y = from + if exact.is_some() { fraction * (to - from) } else { 0. };
                    self.scroll = (y - height / 2.).clamp(0., self.layout.max_scroll());
                }
            }
        }
    }

    /// Find's matches in what's shown, in row order.
    pub fn find_text(&self, query: &str, flags: crate::find::FindFlags) -> Vec<crate::find::TextMatch> {
        let Some(data) = self.data.as_ref() else { return Vec::new() };
        if query.is_empty() {
            return Vec::new();
        }
        let matcher = crate::find::Matcher::new(query, flags);
        let within = self.selection.filter(|_| flags.selection).map(|s| s.range());
        let split = self.split();
        let mut out = Vec::new();
        for (row, r) in self.rows.iter().enumerate() {
            let sides: &[bool] = if split { &[true, false] } else { &[false] };
            for &old_side in sides {
                if let Some(line) = row_line(data, r, split, old_side) {
                    for (from, to) in matcher.in_line(&line.text) {
                        if within.is_none_or(|(a, b)| row >= a.row && row <= b.row) {
                            out.push(crate::find::TextMatch { row, old_side, from, to });
                        }
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
        if self.split() { (SPLIT_LEFT_MARGIN + num_w() + 2., width / 2. + num_w() + 2.) } else { (num_w() + 1., num_w() + 1.) }
    }

    pub fn split_shown(&self) -> bool {
        self.split()
    }

    /// The lines a selection covers on a side, 1-based.
    pub fn selection_lines(&self, new_side: bool) -> Option<(u32, u32)> {
        let sel = self.selection?;
        let (from, to) = sel.range();
        let line = |row: usize| self.rows.get(row).and_then(|r| if new_side { r.new } else { r.old }).map(|n| n + 1);
        let a = (from.row..=to.row).find_map(line)?;
        let b = (from.row..=to.row).rev().find_map(line)?;
        Some((a, b))
    }

    /// The text's left edge, for what's placed against it.
    /// Monaco's sticky scroll, in File View with the setting on: the rows of the scopes around what's at the view's top, at most five.
    pub fn sticky_rows(&self) -> Vec<usize> {
        if self.view != ViewKind::File || !crate::metrics::editor_style().sticky || self.rows.is_empty() {
            return Vec::new();
        }
        let scopes_at = |y: f32| {
            let ix = self.layout.row_at(y).min(self.rows.len() - 1);
            let Some(n) = self.rows[ix].new.or(self.rows[ix].old) else { return Vec::new() };
            (0..n).filter(|i| self.fold_ends.get(*i as usize).copied().flatten().is_some_and(|end| end > n)).collect::<Vec<u32>>()
        };
        let mut lines = scopes_at(self.scroll);
        // What's under the pinned lines is what they have to be the scopes of.
        lines = scopes_at(self.scroll + lines.len().min(5) as f32 * crate::metrics::line_h());
        let keep = lines.len().saturating_sub(5);
        lines[keep..].iter().filter_map(|n| self.rows.iter().position(|r| r.new.or(r.old) == Some(*n))).collect()
    }

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
    if matches!(r.kind, RowKind::Hidden { .. } | RowKind::HunkHeader { .. } | RowKind::Zone { .. }) {
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
        // A tool left from a file that closed some other way isn't this one's work.
        if tab.diff.is_none() {
            tab.merge = None;
        }
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

    /// Whether the file list's file at `index` is the one the diff shows.
    pub fn file_is_open(&self, index: usize) -> bool {
        let Some(tab) = self.tabs.get(self.active) else { return false };
        let Some(file) = tab.files.as_ref().and_then(|f| f.changes.get(index)) else { return false };
        tab.diff.as_ref().is_some_and(|d| d.file.path == file.path && d.file.origin == file.origin)
    }

    pub fn close_file(&mut self, cx: &mut Context<Self>) {
        if self.ask_unsaved(|v, cx| v.close_file(cx), cx) {
            return;
        }
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.diff = None;
            tab.merge = None;
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
        if crate::merge::is_conflict(&diff.file) && tab.history.is_none() {
            return self.read_conflict(cx);
        }
        let (repo, generation, ignore, history_file) = (tab.id, diff.generation, diff.ignore_whitespace, diff.history_file);
        let file = diff.file.clone();
        let (old_side, new_side) = super::edit::sides_for(&file, diff.view);
        let api = self.api.clone();
        let svg = cx.svg_renderer();
        let read = self.rt.spawn(async move {
            let (old, new) = (serde_json::from_value(old_side.clone()), serde_json::from_value(new_side.clone()));
            let (Ok(old), Ok(new)) = (old, new) else { return Err(GbError::other("This file's sides can't be read")) };
            let v = api.dispatch(Request::DiffContents { repo: crate::app::core_repo(repo), path: file.path.to_string(), old, new, force, old_path: file.old_path.clone() }).await?;
            let path = file.path.to_string();
            let old_md = file.old_path.as_deref().is_some_and(crate::md::files::is_markdown_path);
            let editable = super::edit::is_editable(&file, &new_side, &v).then(|| (v["new"]["text"].as_str().unwrap_or_default().to_string(), v["old"]["text"].as_str().map(str::to_string), v["new"]["hash"].as_str().unwrap_or_default().to_string(), new_side["worktree"].as_str().unwrap_or_default().to_string()));
            // A WIP diff's hunks, so its header rows are laid out with it.
            let wip_hunks = match &file.origin {
                crate::details::Origin::Wip { worktree, staged } if file.status != UNCHANGED && file.status != 'U' => api.dispatch(Request::WipHunks { repo: crate::app::core_repo(repo), worktree: worktree.clone(), path: path.clone(), staged: *staged }).await.ok().map(|h| super::staging::WipHunks::of(&h)),
                _ => None,
            };
            // A binary's dumps load with its contents, so it's ready to show as a text file is.
            let hex = if wants_hex(&v, history_file) && !(shows_media(&v) && crate::media::media_of(&path).is_some() && !history_file) {
                let (Ok(old), Ok(new)) = (serde_json::from_value(old_side), serde_json::from_value(new_side)) else { return Err(GbError::other("This file's sides can't be read")) };
                Some(api.dispatch(Request::HexDump { repo: crate::app::core_repo(repo), path: path.clone(), old, new }).await?)
            } else {
                None
            };
            tokio::task::spawn_blocking(move || match hex {
                Some(hex) => (Read::Binary(Binary::Hex(Arc::new(HexData::of(&hex)))), wip_hunks, editable),
                None => (contents(&path, old_md, &v, ignore, history_file, &svg), wip_hunks, editable),
            })
            .await
            .map_err(|e| GbError::other(e.to_string()))
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                let no_git = this.lfs.status.get(&repo).is_some_and(|s| s.version.is_none());
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
                        this.sync_markdown(cx);
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
                    Ok(Ok(Read::Binary(Binary::Pdf { old, new, binary, encoding }))) => {
                        let mut pdf = crate::pdf::PdfView::new(old.as_ref().map(|s| s.1), new.as_ref().map(|s| s.1), diff.view == ViewKind::File, this.retired_images.clone());
                        pdf.binary = binary;
                        pdf.encoding = encoding;
                        diff.pdf = Some(pdf);
                        diff.hex = None;
                        diff.notice = None;
                        this.open_pdf_sides(old.map(|s| s.0), new.map(|s| s.0), generation, cx);
                    }
                    Ok(Ok(Read::Binary(Binary::Media(sides)))) => {
                        let (old, new) = *sides;
                        diff.media = Some(crate::media::MediaView::new(old, new, diff.view == ViewKind::File, this.retired_images.clone()));
                        diff.hex = None;
                        diff.notice = None;
                    }
                    Ok(Ok(Read::Notice(n))) => {
                        diff.notice = Some(n);
                        diff.lfs_no_git = no_git;
                    }
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
            let v = api.dispatch(Request::HexDump { repo: crate::app::core_repo(repo), path: file.path.to_string(), old, new }).await?;
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
            d.keep_place();
            d.ignore_whitespace = !d.ignore_whitespace;
            d.generation += 1;
        });
        self.read_contents(false, cx);
    }

    pub fn step_change(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.tabs.get(self.active).and_then(Tab::merge_shown).is_some() {
            return self.merge_step(forward, cx);
        }
        if let Some(view) = self.shown_md() {
            view.update(cx, |v, cx| {
                v.state.borrow_mut().step_change(forward);
                cx.notify();
            });
            return;
        }
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
        if let Some(pdf) = self.tabs.get_mut(self.active).and_then(|t| t.diff.as_mut()).and_then(|d| d.pdf.as_mut()).filter(|p| !p.source_on) {
            let by = if rows.abs() <= 1. { rows * 40. } else { rows.signum() * (pdf.view_height() - 40.).max(40.) };
            pdf.scroll_by(by);
            self.draw_pdf_pages(cx);
            cx.notify();
            return;
        }
        if self.tabs.get(self.active).and_then(|t| t.diff.as_ref()).and_then(|d| d.media.as_ref()).is_some_and(|m| !m.source_on) {
            return self.media_scroll(rows * 40., cx);
        }
        if let Some(view) = self.shown_md() {
            // Chromium's arrow step, and a page less a little.
            let by = if rows.abs() <= 1. { rows * 40. } else { rows.signum() * (view.read(cx).state.borrow().view_height() - 40.).max(40.) };
            view.update(cx, |v, cx| {
                v.state.borrow_mut().scroll_by(by);
                cx.notify();
            });
            return;
        }
        self.with_diff(cx, |d| d.scroll_by(rows * crate::metrics::line_h()));
    }

    pub fn copy_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = self.shown_md() {
            if let Some(text) = view.read(cx).state.borrow().selected_text() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            return;
        }
        let text = self.merge_selected_text().or_else(|| self.tabs.get(self.active).and_then(|t| t.diff.as_ref()).and_then(DiffState::selected_text));
        if let Some(text) = text {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// Linux's primary selection, as Chromium's page and Monaco's `selectionClipboard` keep it: what a press selected in a
    /// diff, a merge editor or rendered Markdown. A field keeps its own (`ui::input`).
    pub fn publish_selection(&mut self, cx: &mut Context<Self>) {
        let text = if let Some(view) = self.shown_md() {
            view.read(cx).state.borrow().selected_text()
        } else {
            self.merge_selected_text().or_else(|| self.tabs.get(self.active).and_then(|t| t.diff.as_ref()).and_then(DiffState::selected_text))
        };
        if let Some(text) = text.filter(|t| !t.is_empty() && self.last_primary.as_ref() != Some(t)) {
            cx.write_to_primary(ClipboardItem::new_string(text.clone()));
            self.last_primary = Some(text);
        }
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = self.shown_md() {
            view.update(cx, |v, cx| {
                v.state.borrow_mut().select_all();
                cx.notify();
            });
            return;
        }
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
    any && v["tooLarge"].as_bool() != Some(true) && (v["image"].as_bool() != Some(true) || history_file) && (history_file || !shows_pdf(v))
}

/// A press in a view that isn't the code's: the file's keys are the ones that apply, as over the code.
fn takes_keys(body: AnyElement, cx: &mut Context<GitVolt>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .capture_any_mouse_down(cx.listener(|this, _, _, _| {
            if let Some(tab) = this.tabs.get_mut(this.active) {
                tab.keys = KeyArea::Diff;
            }
        }))
        .child(body)
        .into_any_element()
}

/// `isPdf`: the core's `pdf` flag, and every side there with its bytes.
fn shows_pdf(v: &Value) -> bool {
    let with_bytes = |s: &Value| !s.is_object() || s["base64"].as_str().is_some();
    v["pdf"].as_bool() == Some(true) && v["tooLarge"].as_bool() != Some(true) && (v["old"].is_object() || v["new"].is_object()) && with_bytes(&v["old"]) && with_bytes(&v["new"])
}

/// `showsMedia`: a media path, loaded, and each side there binary with its bytes.
fn shows_media(v: &Value) -> bool {
    let with_bytes = |s: &Value| !s.is_object() || (s["binary"].as_bool() == Some(true) && s["base64"].as_str().is_some());
    v["media"].as_bool() == Some(true) && v["tooLarge"].as_bool() != Some(true) && (v["old"].is_object() || v["new"].is_object()) && with_bytes(&v["old"]) && with_bytes(&v["new"])
}

fn side_bytes(s: &Value) -> Option<(Arc<Vec<u8>>, u64)> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(s["base64"].as_str()?).ok()?;
    let size = s["size"].as_u64().unwrap_or(bytes.len() as u64);
    Some((Arc::new(bytes), size))
}

fn contents(path: &str, old_path_markdown: bool, v: &Value, ignore_whitespace: bool, history_file: bool, svg: &SvgRenderer) -> Read {
    if v["tooLarge"].as_bool() == Some(true) {
        return Read::Notice(Notice::TooLarge);
    }
    let missing = crate::lfs::lfs_missing(v);
    if !missing.is_empty() {
        return Read::Notice(Notice::Lfs(missing));
    }
    let binary = |s: &Value| s["binary"].as_bool() == Some(true);
    if shows_pdf(v) && !history_file {
        let encoding = v["new"]["encoding"].as_str().or(v["old"]["encoding"].as_str()).unwrap_or_default().to_string().into();
        return Read::Binary(Binary::Pdf { old: side_bytes(&v["old"]), new: side_bytes(&v["new"]), binary: binary(&v["old"]) || binary(&v["new"]), encoding });
    }
    if shows_media(v) && !history_file
        && let Some((kind, mime)) = crate::media::media_of(path)
    {
        let side = |s: &Value| side_bytes(s).map(|(b, size)| crate::media::MediaSide::of(&b, size, kind, mime));
        return Read::Binary(Binary::Media(Box::new((side(&v["old"]), side(&v["new"])))));
    }
    if v["image"].as_bool() == Some(true) && (binary(&v["old"]) || binary(&v["new"])) {
        return Read::Binary(Binary::Image { old: decode_side(&v["old"], svg), new: decode_side(&v["new"], svg), svg: None });
    }
    let (old, new) = (blob_text(&v["old"]), blob_text(&v["new"]));
    let encoding = v["new"]["encoding"].as_str().or(v["old"]["encoding"].as_str()).unwrap_or_default().to_string();
    let mut built = DiffData::build(old.as_deref(), new.as_deref(), ignore_whitespace, encoding.into(), v["eolOnly"].as_bool() == Some(true));
    if crate::md::files::is_markdown_path(path) || old_path_markdown {
        built.raw = Some((old.as_deref().unwrap_or("").into(), new.as_deref().unwrap_or("").into()));
    }
    let data = Arc::new(built);
    let texts = Texts { path: path.to_string(), old, new };
    // An SVG is an image with its text behind the Source toggle; File History's File shows it as text.
    if path.to_lowercase().ends_with(".svg") && !history_file {
        return Read::Binary(Binary::Image { old: decode_side(&v["old"], svg), new: decode_side(&v["new"], svg), svg: Some((data, texts)) });
    }
    Read::Text(data, texts)
}

/// Monaco's line number column: five characters' room, a pixel over.
pub fn num_w() -> f32 {
    (crate::metrics::advance() * 5.).floor()
}

pub fn mono() -> Font {
    let style = crate::metrics::editor_style();
    let family = crate::settings::fonts::families_of(&style.family).into_iter().next().unwrap_or_else(|| FONT.to_string());
    Font { family: family.into(), features: if style.ligatures { FontFeatures::default() } else { FontFeatures::disable_ligatures() }, fallbacks: Some(FontFallbacks::from_fonts(vec![FONT.into(), "Noto Sans Mono".into(), "monospace".into()])), weight: FontWeight::NORMAL, style: FontStyle::Normal }
}

fn run(len: usize, base: &Font, color: Hsla, flags: u8) -> TextRun {
    let font = Font { weight: if flags & FONT_BOLD != 0 { FontWeight::BOLD } else { FontWeight::NORMAL }, style: if flags & FONT_ITALIC != 0 { FontStyle::Italic } else { FontStyle::Normal }, ..base.clone() };
    let underline = (flags & FONT_UNDERLINE != 0).then(|| UnderlineStyle { thickness: px(1.), ..Default::default() });
    TextRun { len, font, color, background_color: None, underline, strikethrough: None }
}

pub(crate) fn runs(text: &str, spans: &[Span], base: &Font) -> Vec<TextRun> {
    let text_color = theme::editor().foreground;
    let mut out = Vec::with_capacity(spans.len() + 1);
    let mut at = 0usize;
    for s in spans {
        let (start, end) = ((s.start as usize).min(text.len()), (s.end as usize).min(text.len()));
        if start > at {
            out.push(run(start - at, base, text_color, 0));
        }
        if end > start {
            out.push(run(end - start, base, crate::theme::c(s.color), s.font));
        }
        at = at.max(end);
    }
    if at < text.len() {
        out.push(run(text.len() - at, base, text_color, 0));
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
    window.text_system().shape_line(line.text.clone(), px(crate::metrics::editor_font_size()), &runs(&line.text, spans, font), None)
}

/// A visual line: bytes `from..to` of a line that wraps.
fn shape_part(text: &str, from: usize, to: usize, spans: &[Span], font: &Font, window: &Window) -> ShapedLine {
    let part = &text[from..to];
    let clipped: Vec<Span> = spans.iter().filter(|s| (s.start as usize) < to && (s.end as usize) > from).map(|s| Span { start: (s.start as usize).saturating_sub(from) as u32, end: ((s.end as usize).min(to) - from) as u32, ..*s }).collect();
    window.text_system().shape_line(part.to_string().into(), px(crate::metrics::editor_font_size()), &runs(part, &clipped, font), None)
}

fn paint_number(n: u32, right: Pixels, y: Pixels, color: Hsla, p: &Painter, window: &mut Window, cx: &mut App) {
    let text: SharedString = (n + 1).to_string().into();
    let line = window.text_system().shape_line(text.clone(), px(crate::metrics::editor_font_size()), &[TextRun { len: text.len(), font: p.font.clone(), color, background_color: None, underline: None, strikethrough: None }], None);
    line.paint(point(right - line.width, y), px(crate::metrics::line_h()), TextAlign::Left, None, window, cx).ok();
}

fn paint_sign(glyph: &'static str, x: Pixels, y: Pixels, color: Hsla, _: &Painter, window: &mut Window, _: &mut App) {
    // Monaco's diff glyphs: strokes of a pixel, nine across, in the margin's first pixels, at `.insert-sign`'s opacity.
    let color = Hsla { a: color.a * 0.7, ..color };
    let (left, mid) = (x + px(1.), y + px((crate::metrics::line_h() / 2.).floor()));
    window.paint_quad(fill(Bounds::new(point(left, mid), size(px(9.), px(1.))), color));
    if glyph == "+" {
        window.paint_quad(fill(Bounds::new(point(left + px(4.), mid - px(4.)), size(px(1.), px(9.))), color));
    }
}

/// One pane's text from `x`, clipped to `clip`: each visual line with its guides, word marks and
/// selection under it.
#[allow(clippy::too_many_arguments)]
fn paint_text(ix: usize, n: u32, line: &Line, spans: &[Span], old_side: bool, mark: Hsla, x: Pixels, y: Pixels, clip: Bounds<Pixels>, p: &Painter, window: &mut Window, cx: &mut App) {
    let breaks = p.layout.breaks(old_side, n);
    let text = line.text.as_ref();
    let indent = text.bytes().take_while(|b| *b == b' ').count() as f32 * crate::metrics::advance();
    window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
        for part in 0..=breaks.len() {
            let from = if part == 0 { 0 } else { breaks[part - 1] as usize };
            let to = breaks.get(part).map_or(text.len(), |b| *b as usize);
            let y = y + px(part as f32 * crate::metrics::line_h());
            let start = x - px(p.hscroll) + if part == 0 { px(0.) } else { px(indent) };
            let shaped = if breaks.is_empty() { shape(line, spans, &p.font, window) } else { shape_part(text, from, to, spans, &p.font, window) };
            for level in 1..=line.guides {
                let gx = x - px(p.hscroll) + px(crate::metrics::advance() * ((usize::from(level) - 1) * crate::metrics::tab_size()) as f32);
                window.paint_quad(fill(Bounds::new(point(gx, y), size(px(1.), px(crate::metrics::line_h()))), theme::editor().indent_guide));
            }
            for &(a, b) in line.marks.iter().filter(|_| p.view != ViewKind::File) {
                let (a, b) = ((a as usize).max(from), (b as usize).min(to));
                if b > a {
                    let (l, r) = (shaped.x_for_index(a - from), shaped.x_for_index(b - from));
                    window.paint_quad(fill(Bounds::from_corners(point(start + l, y), point(start + r, y + px(crate::metrics::line_h()))), mark));
                }
            }
            if let Some(a) = line.to_end.filter(|_| p.view != ViewKind::File).map(|a| (a as usize).max(from)).filter(|a| *a <= to) {
                // Inline, Monaco draws a removed line in a zone of its own, whose tint ends with the text.
                let left = if a == 0 { x - px(p.hscroll) } else { start + shaped.x_for_index(a - from) };
                let right = if old_side && !p.split { start + shaped.x_for_index(to - from) } else { clip.right() };
                if right > left {
                    window.paint_quad(fill(Bounds::from_corners(point(left, y), point(right, y + px(crate::metrics::line_h()))), mark));
                }
            }
            let mode = crate::metrics::whitespace_mode();
            if mode > 0 {
                let (dots, arrows) = super::model::whitespace_marks(text, &line.tab_starts, from, to, mode, crate::metrics::tab_size());
                let c = theme::editor().whitespace;
                for i in dots {
                    let cx_ = start + shaped.x_for_index(i - from) + px(crate::metrics::advance() / 2. - 1.);
                    window.paint_quad(fill(Bounds::new(point(cx_, y + px(crate::metrics::line_h() / 2. - 1.)), size(px(2.), px(2.))), c));
                }
                for i in arrows {
                    let at = start + shaped.x_for_index(i - from);
                    let arrow = window.text_system().shape_line("→".into(), px(crate::metrics::editor_font_size()), &[TextRun { len: "→".len(), font: p.font.clone(), color: c, background_color: None, underline: None, strikethrough: None }], None);
                    arrow.paint(point(at, y), px(crate::metrics::line_h()), TextAlign::Left, None, window, cx).ok();
                }
            }
            // Monaco's colours for a match and the current one.
            let first = p.found.partition_point(|m| m.row < ix);
            for (k, m) in p.found.iter().enumerate().skip(first).take_while(|(_, m)| m.row == ix) {
                if m.old_side == old_side || !p.split {
                    let (a, b) = (m.from.max(from), m.to.min(to));
                    if b > a {
                        let (l, r) = (shaped.x_for_index(a - from), shaped.x_for_index(b - from));
                        let color = if p.found_at == Some(k) { theme::editor().find_match } else { theme::editor().find_highlight };
                        window.paint_quad(fill(Bounds::from_corners(point(start + l, y), point(start + r, y + px(crate::metrics::line_h()))), color));
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
                        rx += px(crate::metrics::advance());
                    }
                    if rx > lx {
                        window.paint_quad(fill(Bounds::from_corners(point(start + lx, y), point(start + rx, y + px(crate::metrics::line_h()))), theme::selection_bg()));
                    }
                }
            }
            if to > from {
                shaped.paint(point(start, y), px(crate::metrics::line_h()), TextAlign::Left, None, window, cx).ok();
            }
            if !old_side
                && let Some(at) = p.caret.get().filter(|c| c.row == ix && c.byte >= from && (c.byte < to || part == breaks.len()))
            {
                window.paint_quad(fill(Bounds::new(point(start + shaped.x_for_index(at.byte - from), y), size(px(2.), px(crate::metrics::line_h()))), theme::editor().cursor));
            }
        }
    });
}

fn spans_of(pieces: &[Arc<Vec<Vec<Span>>>], n: u32) -> &[Span] {
    let n = n as usize;
    pieces.get(n / COLOUR_CHUNK).and_then(|p| p.get(n % COLOUR_CHUNK)).map_or(&[], Vec::as_slice)
}

/// Monaco's current line in a diff: 2 px above and below it, across the text.
fn paint_current_line(left: Pixels, right: Pixels, y: Pixels, window: &mut Window) {
    let Some(border) = theme::editor().line_highlight_border else { return };
    let h = px(crate::metrics::line_h());
    for at in [y, y + h - px(2.)] {
        window.paint_quad(fill(Bounds::from_corners(point(left, at), point(right, at + px(2.))), border));
    }
}

fn paint_inline(b: Bounds<Pixels>, ix: usize, r: &Row, p: &Painter, window: &mut Window, cx: &mut App) {
    let y = b.origin.y;
    let bottom = y + b.size.height;
    let num = if ix == p.cursor && r.kind != RowKind::Removed { theme::editor().line_number_active } else { theme::editor().line_number };
    let (old_right, new_right) = (b.origin.x + px(num_w() - 1.), b.origin.x + px(2. * num_w() - 1.));
    let sign_x = b.origin.x + px(2. * num_w());
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
            // Monaco's old numbers' strip, an editor behind the new one's, keeps the removed tint beside an added line.
            window.paint_quad(fill(Bounds::from_corners(b.origin, point(b.origin.x + px(num_w()), bottom)), theme::diff_removed_line()));
            window.paint_quad(fill(Bounds::from_corners(point(b.origin.x + px(num_w()), y), point(b.origin.x + b.size.width, bottom)), theme::diff_inserted_line()));
            if ix == p.cursor {
                paint_current_line(text_x, b.origin.x + b.size.width, y, window);
            }
            if let Some(n) = r.new {
                paint_number(n, new_right, y, num, p, window, cx);
                paint_sign("+", sign_x, y, theme::status_added(), p, window, cx);
                paint_text(ix, n, &p.data.new.lines[n as usize], spans_of(&hl.new, n), false, theme::diff_inserted_text(), text_x, y, clip, p, window, cx);
            }
        }
        _ if p.view == ViewKind::File => {
            let right = b.origin.x + px(num_w() - 1.);
            let at = b.origin.x + px(num_w() + FOLD_W);
            let clip = Bounds::from_corners(point(at, y), point(b.origin.x + b.size.width, bottom));
            // Monaco's current line.
            if ix == p.cursor && let Some(band) = theme::editor().line_highlight {
                window.paint_quad(fill(clip, band));
            }
            let (n, old_side) = match (r.new, r.old) {
                (Some(n), _) => (n, false),
                (None, Some(n)) => (n, true),
                _ => return,
            };
            paint_number(n, right, y, num, p, window, cx);
            let folded = p.folded.contains(&n);
            if folded || (p.fold_hover && p.fold_ends.get(n as usize).copied().flatten().is_some()) {
                let chevron = Bounds::new(point(b.origin.x + px(num_w() + 6.), y + px(2.)), size(px(14.), px(14.)));
                window.paint_svg(chevron, if folded { "icons/chevron-right.svg" } else { "icons/chevron-down.svg" }.into(), None, TransformationMatrix::unit(), theme::editor().fold_icon, cx).ok();
            }
            let (line, spans, mark) = if old_side { (&p.data.old.lines[n as usize], spans_of(&hl.old, n), theme::diff_removed_text()) } else { (&p.data.new.lines[n as usize], spans_of(&hl.new, n), theme::diff_inserted_text()) };
            paint_text(ix, n, line, spans, old_side, mark, at, y, clip, p, window, cx);
            if folded {
                let x = at - px(p.hscroll) + px(line.text.chars().count() as f32 * crate::metrics::advance() + 6.);
                window.paint_quad(quad(Bounds::new(point(x, y + px(3.)), size(px(22.), px(12.))), px(2.), theme::control_hover_bg(), px(0.), transparent_black(), BorderStyle::default()));
            }
        }
        _ => {
            if ix == p.cursor {
                paint_current_line(text_x, b.origin.x + b.size.width, y, window);
            }
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
    // Monaco's unchanged region has no ground of its own: a shadow fades in from its top and bottom, 3 px.
    for (i, alpha) in [0.15, 0.08, 0.04].into_iter().enumerate() {
        let shade = Hsla { a: alpha, ..black() };
        window.paint_quad(fill(Bounds::new(point(b.origin.x, b.origin.y + px(i as f32)), size(b.size.width, px(1.))), shade));
        window.paint_quad(fill(Bounds::new(point(b.origin.x, b.origin.y + b.size.height - px(i as f32 + 1.)), size(b.size.width, px(1.))), shade));
    }
    let hovered = p.hover == Some(ix);
    if hovered {
        window.set_window_cursor_style(CursorStyle::PointingHand);
    }
    let color = if hovered { theme::text_selected() } else { theme::text_normal() };
    let icon_at = Bounds::new(point(b.origin.x + px(2. * num_w() - 16.), b.origin.y + px(5.)), size(px(14.), px(14.)));
    window.paint_svg(icon_at, "icons/chevrons-up-down.svg".into(), None, TransformationMatrix::unit(), color, cx).ok();
    let text: SharedString = format!("{count} hidden lines").into();
    let run = TextRun { len: text.len(), font: window.text_style().font(), color, background_color: None, underline: None, strikethrough: None };
    let line = window.text_system().shape_line(text, px(12.), &[run], None);
    line.paint(point(b.origin.x + px(2. * num_w() + SIGN_W), b.origin.y), b.size.height, TextAlign::Left, None, window, cx).ok();
}

fn paint_split(b: Bounds<Pixels>, ix: usize, r: &Row, p: &Painter, window: &mut Window, cx: &mut App) {
    let y = b.origin.y;
    let height = b.size.height;
    let half = b.size.width / 2.;
    let hl = &p.highlight;
    let change = r.kind == RowKind::Change;
    let pane = |left: Pixels, n: Option<u32>, old_side: bool, window: &mut Window, cx: &mut App| {
        let area = Bounds::new(point(left, y), size(half, height));
        let margin = if old_side { SPLIT_LEFT_MARGIN } else { 0. };
        let text_at = px(margin + num_w() + SIGN_W);
        let Some(n) = n else {
            let hatch = Bounds::from_corners(point(left + text_at, y), area.bottom_right());
            window.paint_quad(fill(hatch, pattern_slash(theme::diff_diagonal(), 2., 6.)));
            return;
        };
        if change {
            window.paint_quad(fill(area, if old_side { theme::diff_removed_line() } else { theme::diff_inserted_line() }));
        }
        if ix == p.cursor && !old_side {
            paint_current_line(left + text_at, area.right(), y, window);
        }
        let (side, spans, mark) = if old_side { (&p.data.old, spans_of(&hl.old, n), theme::diff_removed_text()) } else { (&p.data.new, spans_of(&hl.new, n), theme::diff_inserted_text()) };
        paint_number(n, left + px(margin + num_w() - 1.), y, if ix == p.cursor && !old_side { theme::editor().line_number_active } else { theme::editor().line_number }, p, window, cx);
        if change {
            let (glyph, color) = if old_side { ("−", theme::status_deleted()) } else { ("+", theme::status_added()) };
            paint_sign(glyph, left + px(margin + num_w()), y, color, p, window, cx);
        }
        let text_x = left + px(margin + num_w() + SIGN_W);
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
        (left, if left { px(diff.gutter()) } else { half + px(num_w() + SIGN_W) })
    } else {
        (r.new.is_none(), px(diff.gutter()))
    };
    let x = (position.x - b.origin.x - gutter + px(diff.hscroll)).max(px(0.));
    let Some(line) = row_line(data, r, split, old_side) else { return Some((TextPos { row, byte: 0 }, old_side)) };
    let n = if old_side { r.old } else { r.new }.or(r.old).unwrap_or(0);
    let breaks = diff.layout.breaks(old_side, n);
    let text = line.text.as_ref();
    let part = (((y - diff.layout.offsets[row]) / crate::metrics::line_h()).max(0.) as usize).min(breaks.len());
    let from = if part == 0 { 0 } else { breaks[part - 1] as usize };
    let to = breaks.get(part).map_or(text.len(), |b| *b as usize);
    let indent = if part == 0 { 0. } else { text.bytes().take_while(|b| *b == b' ').count() as f32 * crate::metrics::advance() };
    let shaped = shape_part(text, from, to, &[], &mono(), window);
    Some((TextPos { row, byte: from + shaped.closest_index_for_x((x - px(indent)).max(px(0.))) }, old_side))
}

pub(crate) fn word_at(text: &str, byte: usize) -> (usize, usize) {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let byte = byte.min(text.len());
    let start = text[..byte].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(byte, |(i, _)| i);
    let end = text[byte..].char_indices().find(|(_, c)| !is_word(*c)).map_or(text.len(), |(i, _)| byte + i);
    (start, end)
}

/// The widest line in px, as far as a pane scrolls sideways.
fn content_width(data: &DiffData) -> f32 {
    data.widest as f32 * crate::metrics::advance()
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
    // A press on a pinned line goes to it.
    let pinned = ((e.position.y - b.origin.y).as_f32() / crate::metrics::line_h()) as usize;
    if let Some(ix) = diff.sticky_rows().get(pinned).copied() {
        diff.scroll = diff.layout.offsets.get(ix).copied().unwrap_or(0.).clamp(0., diff.layout.max_scroll());
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
    if matches!(r.kind, RowKind::HunkHeader { .. } | RowKind::Zone { .. }) {
        return;
    }
    if diff.view == ViewKind::File && (num_w()..num_w() + FOLD_W).contains(&x) {
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
    if this.review_drag_move(e, cx) {
        return;
    }
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
                diff.scroll_by(-above.min(crate::metrics::line_h() * 3.));
            } else if below > 0. {
                diff.scroll_by(below.min(crate::metrics::line_h() * 3.));
            }
            if let Some((at, _)) = pos_at(diff, e.position, window) {
                // A press in an editor leaves no selection until the head moves: only the caret.
                if let Some(sel) = diff.selection.as_mut() {
                    sel.head = at;
                }
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
            let changed = shows(hover, diff) != shows(diff.hover, diff) || (diff.review.is_some() && hover.map(|h| h.0) != diff.hover.map(|h| h.0));
            diff.hover = hover;
            if changed {
                cx.notify();
            }
        }
    }
}

impl GitVolt {
    /// Where a selection's drag is going on, if one is: the text's box.
    fn text_drag_bounds(&self, cx: &App) -> Option<Bounds<Pixels>> {
        if let Some(view) = self.shown_md() {
            let view = view.read(cx);
            let st = view.state.borrow();
            return st.text_dragging().then(|| st.view_bounds());
        }
        let tab = self.tabs.get(self.active)?;
        if let Some(m) = tab.merge_shown() {
            let i = (0..m.editor_count()).find(|i| m.view(*i).drag == crate::merge::PaneDrag::Text)?;
            return Some(m.view(i).body.get());
        }
        tab.diff.as_ref().filter(|d| d.drag == Drag::Text).map(|d| d.body.get())
    }

    fn text_drag_step(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.shown_md() {
            view.update(cx, |m, cx| m.on_move(e, window, cx));
        } else if self.tabs.get(self.active).is_some_and(|t| t.merge_shown().is_some()) {
            self.merge_drag(e, window, cx);
        } else {
            on_mouse_move(self, e, window, cx);
        }
    }

    /// A press moved. A selection's drag that's left the text goes on, wherever the pointer is, even out of the window; past
    /// the top or the bottom the text keeps coming while the pointer rests there, as Monaco's and Chromium's do.
    pub fn text_drag_moved(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bounds) = self.text_drag_bounds(cx) else {
            self.text_drag_at = None;
            return;
        };
        if bounds.contains(&e.position) {
            self.text_drag_at = None;
            return;
        }
        self.text_drag_step(e, window, cx);
        if self.text_drag_at.replace(e.position).is_some() {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(50)).await;
                let more = this.update_in(cx, |this, window, cx| {
                    let Some(position) = this.text_drag_at else { return false };
                    if this.text_drag_bounds(cx).is_none() {
                        this.text_drag_at = None;
                        return false;
                    }
                    this.text_drag_step(&MouseMoveEvent { position, pressed_button: Some(MouseButton::Left), modifiers: Modifiers::default() }, window, cx);
                    true
                });
                if !matches!(more, Ok(true)) {
                    return;
                }
            }
        })
        .detach();
    }
}

fn text_area(diff: &DiffState, cx: &mut Context<GitVolt>) -> AnyElement {
    let Some(data) = diff.data.clone() else {
        return message(diff, cx).into_any_element();
    };
    let mut staging = crate::forge::review::zones::range_marks(diff);
    staging.extend(crate::forge::review::zones::drag_marks(diff));
    staging.extend(super::staging::overlays(diff, cx));
    staging.extend(crate::forge::review::zones::overlays(diff, cx));
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
    let sticky = diff.sticky_rows();
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
        .on_mouse_down(MouseButton::Middle, cx.listener(|this, e: &MouseDownEvent, window, cx| {
            // X's primary selection, at the place pressed, as Monaco's.
            if !this.tabs.get(this.active).and_then(|t| t.diff.as_ref()).is_some_and(|d| d.edit.is_some()) {
                return;
            }
            on_mouse_down(this, e, window, cx);
            this.with_diff_quiet(|d| d.drag = Drag::None);
            if let Some(text) = cx.read_from_primary().and_then(|i| i.text()) {
                this.paste_text(&text, cx);
            }
        }))
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
                                    RowKind::HunkHeader { .. } | RowKind::Zone { .. } => {}
                                    _ if p.split => paint_split(b, ix, &rows[ix], &p, window, cx),
                                    _ => paint_inline(b, ix, &rows[ix], &p, window, cx),
                                }
                            }
                            let h = crate::metrics::line_h();
                            for (k, ix) in sticky.iter().enumerate() {
                                let b = Bounds::new(point(bounds.origin.x, bounds.origin.y + px(k as f32 * h)), size(bounds.size.width, px(h)));
                                window.paint_quad(fill(b, theme::app_bg()));
                                paint_inline(b, *ix, &rows[*ix], &p, window, cx);
                            }
                            if !sticky.is_empty() {
                                window.paint_quad(fill(Bounds::new(point(bounds.origin.x, bounds.origin.y + px(sticky.len() as f32 * h)), size(bounds.size.width, px(1.))), theme::section_border()));
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
                        let (dx, dy) = (delta.x * MONACO_WHEEL, delta.y * MONACO_WHEEL);
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

pub(crate) fn edit_keys_of(el: Stateful<Div>, cx: &mut Context<GitVolt>) -> Stateful<Div> {
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

impl DiffState {
    fn strip_slider(&self, minimap: bool, strip: f32) -> Option<Slider> {
        let (visible, content) = (self.layout.height, self.layout.total());
        if minimap { (content > visible).then(|| minimap_slider(self.scroll, crate::metrics::line_h(), MINIMAP_LINE, strip, visible, content)) } else { slider_of(visible, content, self.scroll, strip) }
    }

    /// The minimap's first row: `MinimapLayout`'s start line, so the slider lies over the lines it stands for.
    fn minimap_first(&self, slider_top: f32) -> usize {
        self.layout.row_at(self.scroll).saturating_sub((slider_top / MINIMAP_LINE) as usize)
    }
}

/// A press on a strip: on its slider it holds the point it's on. Off it the view centres there: on the minimap on the line
/// pressed, as Monaco's, elsewhere by how far along.
pub fn strip_events(el: Stateful<Div>, cell: Rc<Cell<Bounds<Pixels>>>, minimap: bool, cx: &mut Context<GitVolt>) -> Stateful<Div> {
    el.on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            let b = cell.get();
            this.with_diff(cx, |d| {
                let (y, strip) = ((e.position.y - b.origin.y).as_f32(), b.size.height.as_f32());
                let Some(s) = d.strip_slider(minimap, strip) else { return };
                let on_it = y >= s.top && y < s.top + s.height;
                if minimap && !on_it {
                    d.center_on(d.minimap_first(s.top) + (y / MINIMAP_LINE) as usize);
                }
                let s = d.strip_slider(minimap, strip).unwrap_or(s);
                let at = if on_it || minimap { ((y - s.top) / s.height).clamp(0., 1.) } else { 0.5 };
                d.drag = Drag::Ruler;
                d.strip_held = Some((b, at, minimap));
                if !minimap {
                    d.scroll = scroll_held(&s, y, at, d.layout.max_scroll());
                }
            });
        }),
    )
}

/// While a strip is held the pointer counts wherever it goes, off the strip and out of the window, until it's let go.
pub fn strip_drag(view: WeakEntity<GitVolt>, window: &mut Window) {
    let up = view.clone();
    window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
        if phase != DispatchPhase::Capture {
            return;
        }
        let _ = view.update(cx, |this, cx| {
            let Some(d) = this.tabs.get_mut(this.active).and_then(|t| t.diff.as_mut()) else { return };
            let (Drag::Ruler, Some((b, at, minimap))) = (d.drag, d.strip_held) else { return };
            if e.pressed_button == Some(MouseButton::Left) {
                if let Some(s) = d.strip_slider(minimap, b.size.height.as_f32()) {
                    d.scroll = scroll_held(&s, (e.position.y - b.origin.y).as_f32(), at, d.layout.max_scroll());
                }
            } else {
                d.drag = Drag::None;
                d.strip_held = None;
            }
            cx.notify();
            cx.stop_propagation();
        });
    });
    window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
        if phase != DispatchPhase::Capture || e.button != MouseButton::Left {
            return;
        }
        let _ = up.update(cx, |this, cx| {
            let Some(d) = this.tabs.get_mut(this.active).and_then(|t| t.diff.as_mut()) else { return };
            if d.drag == Drag::Ruler {
                d.drag = Drag::None;
                d.strip_held = None;
                cx.notify();
            }
        });
    });
}

/// A slider's colour: Monaco's on hover and while it's held.
fn slider_tint(diff_hover: bool, held: bool) -> Hsla {
    if diff_hover || held { theme::scroll_thumb_hover() } else { theme::scroll_thumb() }
}

/// The overview ruler: each change's place in the file, and the viewport.
fn ruler(diff: &DiffState, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let marks = diff.ruler.clone();
    let layout = diff.layout.clone();
    let scroll = diff.scroll;
    let tint = slider_tint(diff.ruler_hover, diff.drag == Drag::Ruler);
    let cell = Rc::new(Cell::new(Bounds::<Pixels>::default()));
    let paint_cell = cell.clone();
    let view = cx.weak_entity();
    let bar = div().id("diff-ruler").flex_none().w(px(RULER_W)).h_full().on_hover(cx.listener(|this, on: &bool, _, cx| this.with_diff(cx, |d| d.ruler_hover = *on)));
    strip_events(bar, cell, false, cx).child(
        canvas(
            |_, _, _| {},
            move |b, _, window, _| {
                paint_cell.set(b);
                strip_drag(view.clone(), window);
                let total = layout.total().max(1.);
                let strip = b.size.height.as_f32();
                let ratio = strip / total;
                let half = b.size.width / 2.;
                let top_of = |row: usize| layout.offsets.get(row).copied().unwrap_or(0.);
                for &(first, rows, removed) in marks.iter() {
                    let (top, h) = mark_span(top_of(first), top_of(first + rows) - top_of(first), ratio, strip);
                    let x = if removed { b.origin.x } else { b.origin.x + half };
                    window.paint_quad(fill(Bounds::new(point(x, b.origin.y + px(top)), size(half, px(h))), if removed { theme::ruler_removed() } else { theme::ruler_added() }));
                }
                if let Some(s) = slider_of(layout.height, total, scroll, strip) {
                    window.paint_quad(fill(Bounds::new(point(b.origin.x, b.origin.y + px(s.top)), size(b.size.width, px(s.height))), tint));
                }
            },
        )
        .size_full(),
    )
}

/// File View's strip: the file in small, each character a pixel wide, and the viewport over it.
fn minimap(diff: &DiffState, width: f32, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let hover = diff.minimap_hover || diff.strip_held.is_some_and(|h| h.2);
    let (data, hl, rows, layout, scroll) = (diff.data.clone(), diff.highlight.clone(), diff.rows.clone(), diff.layout.clone(), diff.scroll);
    let cell = Rc::new(Cell::new(Bounds::<Pixels>::default()));
    let paint_cell = cell.clone();
    let view = cx.weak_entity();
    let bar = div().id("minimap").flex_none().w(px(width)).h_full().on_hover(cx.listener(|this, on: &bool, _, cx| {
        this.with_diff(cx, |d| d.minimap_hover = *on);
    }));
    strip_events(bar, cell, true, cx).child(
        canvas(
            |_, _, _| {},
            move |b, _, window, _| {
                paint_cell.set(b);
                strip_drag(view.clone(), window);
                let Some(data) = &data else { return };
                let height = b.size.height.as_f32();
                let shown = (height / MINIMAP_LINE) as usize;
                let top_row = layout.row_at(scroll);
                let slider = minimap_slider(scroll, crate::metrics::line_h(), MINIMAP_LINE, height, layout.height, layout.total().max(1.));
                // A long file's map moves slower than the file, so it stays under the viewport.
                let first = top_row.saturating_sub((slider.top / MINIMAP_LINE) as usize);
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
                            paint(0, text.len(), Hsla { a: 0.6, ..theme::editor().foreground });
                        } else {
                            for s in spans {
                                paint(s.start as usize, (s.end as usize).min(text.len()), Hsla { a: 0.75, ..theme::c(s.color) });
                            }
                        }
                    }
                });
                // Monaco's `showSlider: mouseover`.
                if hover {
                    window.paint_quad(fill(Bounds::new(point(b.origin.x, b.origin.y + px(slider.top)), size(b.size.width, px(slider.height))), theme::scroll_thumb()));
                }
            },
        )
        .size_full(),
    )
}

/// File View's scrollbar: a thumb, with no track.
fn scrollbar(diff: &DiffState, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let (layout, scroll) = (diff.layout.clone(), diff.scroll);
    let tint = slider_tint(diff.ruler_hover, diff.drag == Drag::Ruler);
    let cell = Rc::new(Cell::new(Bounds::<Pixels>::default()));
    let paint_cell = cell.clone();
    let view = cx.weak_entity();
    let bar = div().id("diff-scrollbar").flex_none().w(px(SCROLLBAR_W)).h_full().on_hover(cx.listener(|this, on: &bool, _, cx| this.with_diff(cx, |d| d.ruler_hover = *on)));
    strip_events(bar, cell, false, cx).child(
        canvas(
            |_, _, _| {},
            move |b, _, window, _| {
                paint_cell.set(b);
                strip_drag(view.clone(), window);
                let Some(s) = slider_of(layout.height, layout.total().max(1.), scroll, b.size.height.as_f32()) else { return };
                window.paint_quad(fill(Bounds::new(point(b.origin.x, b.origin.y + px(s.top)), size(b.size.width, px(s.height))), tint));
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
        Some(Notice::Lfs(_)) => base,
        _ => base,
    }
}

pub(crate) fn header(diff: &DiffState, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let dirty = diff.editing_dirty();
    let can_save = diff.edit.is_some();
    let path = diff.file.path.as_ref();
    let (dir, name) = match path.rsplit_once('/') {
        Some((d, n)) => (format!("{d}/"), n.to_string()),
        None => (String::new(), path.to_string()),
    };
    let (glyph, color) = status_icon(diff.file.status);
    let encoding = diff.data.as_ref().map(|d| d.encoding.clone()).or(diff.pdf.as_ref().map(|p| p.encoding.clone())).unwrap_or_default();
    let file = diff.view == ViewKind::File;
    let binary = diff.hex.as_ref().filter(|_| diff.image.is_none() && diff.pdf.is_none() && diff.media.is_none()).map(|h| (h.data.summary(file), h.data.capped_note(file)));
    let image_cap = diff.hex.as_ref().filter(|_| diff.image.as_ref().is_some_and(|i| i.source_on)).and_then(|h| h.data.capped_note(file));
    div()
        .flex()
        .flex_none()
        .h(px(crate::metrics::panel_bar_h()))
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
                    this.sync_markdown(cx);
                }))
            })
            .into_any_element()
    };
    let rendered_now = diff.shows_rendered(cx);
    let mode_button = |id: &'static str, label: &'static str, mode: Mode, cx: &mut Context<GitVolt>| {
        let enabled = in_diff && diff.hex.is_none();
        // A rendered diff is Split or one column: a Hunk pick shows it as Inline, and Hunk says why it's off.
        if rendered_now && mode == Mode::Hunk {
            return tip(crate::md::files::RENDERED_HUNK_TIP, segment(id, label, false, false)).into_any_element();
        }
        let on = if rendered_now && diff.mode == Mode::Hunk { mode == Mode::Inline } else { diff.mode == mode };
        segment(id, label, on, enabled)
            .when(enabled, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    this.with_diff(cx, |d| {
                        d.mode = mode;
                        d.hscroll = 0.;
                        d.relayout();
                    });
                    this.sync_markdown(cx);
                }))
            })
            .into_any_element()
    };
    let icon_button = |id: &'static str, name: &'static str, pressed: bool, enabled: bool| {
        let color = if pressed { theme::text_selected() } else { theme::text_secondary() };
        div().id(id).flex().flex_none().size(px(22.)).items_center().justify_center().rounded(px(3.)).when(pressed, |d| d.bg(theme::selected_row())).when(enabled, |d| d.hover(|d| d.bg(theme::hover_row()))).when(!enabled, |d| d.opacity(0.4)).child(icon(name, 14., color))
    };
    let rendered = diff.shows_rendered(cx);
    let md_changes = diff.md.as_ref().is_some_and(|m| !m.view.read(cx).state.borrow().change_marks().is_empty());
    let can_step = in_diff && if rendered { md_changes } else { !diff.changes.is_empty() };
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
    let text_tools = diff.image.as_ref().is_none_or(|i| i.source_on) && diff.pdf.is_none() && diff.media.is_none();
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
                .when(crate::md::files::is_markdown(&diff.file), |d| d.child(crate::md::files::view_toggle(diff, cx)))
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
                        .when(diff_tools && rendered, |d| d.child(tip(crate::md::files::RENDERED_WHITESPACE_TIP, icon_button("ignore-ws", "pilcrow", diff.ignore_whitespace, false))))
                        .when(diff_tools && !rendered, |d| d.child(icon_button("ignore-ws", "pilcrow", diff.ignore_whitespace, in_diff && !hex).when(in_diff && !hex, |d| d.on_click(cx.listener(|this, _, _, cx| this.toggle_whitespace(cx))))))
                        .when(rendered, |d| d.child(tip(crate::md::files::RENDERED_WRAP_TIP, icon_button("wrap", "text-wrap", diff.wrap, false))))
                        .when(!rendered, |d| d.child(tip("Word wrap", icon_button("wrap", "text-wrap", diff.wrap, !hex).when(!hex, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| {
                                this.with_diff(cx, |d| {
                                    d.keep_place();
                                    d.wrap = !d.wrap;
                                    d.hscroll = 0.;
                                });
                            }))
                        })))),
                    )
                }),
        )
}

/// The text's body in a panel `width` by `height` px: what `ensure_layout` lays out.
pub fn body_size(diff: &DiffState, width: f32, height: f32) -> (f32, f32) {
    // `height`: what's under the panel's header.
    if diff.pdf.is_some() || diff.media.is_some() {
        let bar = if diff.pdf.is_some() { crate::pdf::TOOLBAR_H } else { crate::media::TOOLBAR_H };
        if diff.pdf.as_ref().is_some_and(|p| p.source_on) || diff.media.as_ref().is_some_and(|m| m.source_on) {
            return (width - RULER_W, height - TOOLBAR_H - bar);
        }
        return (width, height - TOOLBAR_H - bar);
    }
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
    if !crate::metrics::editor_style().minimap {
        return 0.;
    }
    // Monaco's layout: what's left beside the margin and the scrollbar, at a character's width and two.
    ((width - num_w() - FOLD_W - SCROLLBAR_W) / (crate::metrics::advance() + 2.)).min(MINIMAP_MAX).floor()
}

/// The panel for `tab`'s open file, `width` px wide.
pub fn diff_panel(tab: &Tab, width: f32, last_opener: Option<&str>, undo: Option<&gitvolt_core::write::types::StagingUndoState>, window: &Window, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let diff = tab.diff.as_ref()?;
    if crate::merge::is_conflict(&diff.file) {
        return Some(crate::merge::view::merge_panel(tab, diff, tab.merge.as_ref(), cx));
    }
    let staging = tab.wip_ctx().map(|ctx| crate::wip::staging_buttons(("diff-staging-undo", "diff-staging-redo"), ctx, undo, tab.wip.committing, cx).into_any_element());
    let open_in = crate::open_in::open_in_button(tab, last_opener, cx);
    let media_open_in = if diff.media.is_some() { crate::open_in::open_in_button(tab, last_opener, cx) } else { None };
    let find = tab.find.open.then(|| crate::find::editor_find(&tab.find, if diff.view == ViewKind::File { minimap_width(width) } else { RULER_W }, window, cx));
    Some(div().relative().flex().flex_col().flex_1().min_w_0().min_h_0().bg(theme::app_bg()).child(header(diff, cx)).child(toolbar(diff, ToolbarKind::Panel { has_history: crate::history::history_start(&diff.file, tab.commit.as_ref().map_or(&[][..], |c| c.parents.as_slice()), &tab.worktree).is_some(), open_in, staging }, width, cx)).child(diff_body(diff, width, &tab.zoom_input, media_open_in, cx)).children(find).into_any_element())
}

/// What's under the toolbar, in an editor `width` px wide.
pub fn diff_body(diff: &DiffState, width: f32, zoom_input: &Entity<crate::ui::input::TextInput>, open_in_el: Option<AnyElement>, cx: &mut Context<GitVolt>) -> AnyElement {
    if let Some(Notice::Lfs(missing)) = &diff.notice {
        return crate::lfs::lfs_placeholder(missing, diff.lfs_pointer, diff.lfs_busy, diff.lfs_no_git, cx);
    }
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
    if let Some(pdf) = &diff.pdf {
        let source = diff.hex.as_ref().filter(|_| pdf.source_on).map(|h| hex_body(diff, h, cx));
        return takes_keys(crate::pdf::pdf_body(pdf, source, cx), cx);
    }
    if let Some(media) = &diff.media {
        let source = diff.hex.as_ref().filter(|_| media.source_on).map(|h| hex_body(diff, h, cx));
        return takes_keys(crate::media::media_body(media, source, open_in_el, cx), cx);
    }
    if let Some(hex) = &diff.hex {
        return hex_body(diff, hex, cx);
    }
    if let Some(body) = crate::md::files::rendered_body(diff, cx) {
        return takes_keys(body, cx);
    }
    let banner = crate::md::files::too_large_banner(diff, cx);
    if let Some(banner) = banner {
        return div().flex().flex_col().flex_1().min_w_0().min_h_0().child(banner).child(text_body(diff, width, cx)).into_any_element();
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
                .relative()
                .flex_1()
                .min_h_0()
                .child(text_area(diff, cx))
                .when(has_text && file_view, |d| d.when(crate::metrics::editor_style().minimap, |d| d.child(minimap(diff, minimap_width(width), cx))).child(scrollbar(diff, cx)))
                .when(has_text && !file_view, |d| d.child(div().absolute().top_0().bottom_0().right(px(RULER_W)).w(px(1.)).bg(theme::section_border())))
                .when(has_text && !file_view, |d| d.child(crate::ui::anchor("diff-ruler", ruler(diff, cx)))),
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

    fn same_data(got: &DiffData, want: &DiffData, what: &str) {
        assert_eq!(got.ops, want.ops, "{what}: ops");
        assert_eq!(got.widest, want.widest, "{what}: widest");
        for (side, (g, w)) in [("old", (&got.old, &want.old)), ("new", (&got.new, &want.new))] {
            assert_eq!(g.lines.len(), w.lines.len(), "{what}: {side}'s lines");
            for (i, (a, b)) in g.lines.iter().zip(&w.lines).enumerate() {
                let line = |l: &Line| (l.text.clone(), l.marks.clone(), l.to_end, l.guides, l.tab_starts.clone());
                assert_eq!(line(a), line(b), "{what}: {side} line {i}");
            }
        }
    }

    #[test]
    fn a_keystrokes_rebuild_is_what_a_whole_build_makes() {
        let old = "fn main() {\n\tlet a = 1;\n\n    \n    if a > 0 {\r\n        go(a);\n    }\n}\n\nmod rest {\n    // more\n}\n";
        // Each: replace `range` of the text so far with `with`, a range past the end clamped to it.
        let edits: &[(std::ops::Range<usize>, &str)] = &[
            (0..0, "x"),
            (14..15, ""),
            (20..20, "\n"),
            (5..30, ""),
            (6..6, "\n\n    \n"),
            (1000..1000, "tail\n"),
            (0..3, "pub fn"),
            (9..10, "\t"),
            (30..45, "one\ntwo\n\nthree"),
            (2..2, "\r\n"),
            (0..1000, "all of it\n"),
            (0..0, "    \n\n"),
        ];
        for ignore_whitespace in [false, true] {
            let mut text = old.replace("let a = 1;", "let a = 2;");
            let mut data = DiffData::build(Some(old), Some(&text), ignore_whitespace, "UTF-8".into(), false);
            for (n, (range, with)) in edits.iter().enumerate() {
                let before = text.clone();
                let (start, end) = (range.start.min(text.len()), range.end.min(text.len()));
                text.replace_range(start..end, with);
                data = data.rebuild(Some(old), &before, &text, ignore_whitespace);
                let whole = DiffData::build(Some(old), Some(&text), ignore_whitespace, "UTF-8".into(), false);
                same_data(&data, &whole, &format!("edit {n} {:?}, ignoring whitespace {ignore_whitespace}", (start..end, with)));
            }
        }
    }

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

    fn long_pair() -> (String, String) {
        let old: Vec<String> = (0..400).map(|i| format!("line {i}")).collect();
        let mut new = old.clone();
        new[100] = "changed 100".into();
        new[300] = "changed 300".into();
        (old.join("\n"), new.join("\n"))
    }

    /// The e2e's `ledger.php`: Monaco's two editors drifted apart on a file opened again. Here both sides are one list of rows with one scroll.
    #[test]
    fn split_opens_a_file_again_with_both_sides_at_its_first_change() {
        let old: Vec<String> = (0..2800).map(|i| format!("line {i}")).collect();
        let mut new = old.clone();
        new.insert(1654, "inserted one".into());
        new.insert(1655, "inserted two".into());
        let (old, new) = (old.join("\n"), new.join("\n"));
        for _ in 0..2 {
            let mut d = state(&old, &new);
            d.mode = Mode::Split;
            d.relayout();
            d.reveal_first = true;
            d.ensure_layout(800., 600.);
            assert!(d.scroll > 0., "at the change, not the top");
            let top = &d.rows[d.layout.row_at(d.scroll)];
            let (top_old, top_new) = (top.old.unwrap(), top.new.unwrap());
            assert_eq!(top_old, top_new, "above the insertions the two sides show the same line at the top");
            assert!(top_new > 1600, "{top_new}");
            let change = d.rows[d.changes[0]].new.unwrap();
            assert!(d.rows[d.layout.row_at(d.scroll + 300.)].new.is_some_and(|n| n.abs_diff(change) <= 1), "the change in the middle");
        }
    }

    #[test]
    fn a_change_of_mode_keeps_the_line_at_the_viewports_centre() {
        let (old, new) = long_pair();
        let mut d = state(&old, &new);
        d.ensure_layout(800., 600.);
        // The user's place, somewhere that's no change's: into the hidden lines Hunk leaves out.
        d.scroll = d.layout.offsets[200];
        let y = d.scroll + 300.;
        let line = d.rows[d.layout.row_at(y)].new;
        d.set_mode(Mode::Split);
        d.ensure_layout(800., 600.);
        assert_eq!(d.rows[d.layout.row_at(d.scroll + 300.)].new, line, "the same line in Split");
        d.set_mode(Mode::Hunk);
        d.ensure_layout(800., 600.);
        assert!(d.scroll < d.layout.max_scroll(), "not the empty end of a shorter view");
        let shown = d.rows[d.layout.row_at(d.scroll + 300.)].new;
        assert!(shown.is_some_and(|n| n >= line.unwrap_or(0)), "the first line shown after it, not the end: {shown:?}");
    }

    #[test]
    fn a_change_of_mode_shows_the_opens_first_change_while_its_still_there() {
        let (old, new) = long_pair();
        let mut d = state(&old, &new);
        d.reveal_first = true;
        d.ensure_layout(800., 600.);
        let first = d.first_change_at;
        assert_eq!(first, Some(d.scroll));
        d.set_mode(Mode::Hunk);
        d.ensure_layout(800., 600.);
        let at = d.changes.first().map_or(-1., |c| d.layout.offsets[*c]);
        assert!(at >= d.scroll && at < d.scroll + 600., "the change is in view: at {at}, scrolled {}", d.scroll);
    }

    #[test]
    fn a_change_of_mode_at_the_top_stays_at_the_top() {
        let (old, new) = long_pair();
        let mut d = state(&old, &new);
        d.ensure_layout(800., 600.);
        d.scroll = 0.;
        d.set_mode(Mode::Split);
        d.ensure_layout(800., 600.);
        assert_eq!(d.scroll, 0.);
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
        assert_eq!((d.layout.offsets[0], d.layout.offsets[1], d.layout.offsets[2]), (0., HIDDEN_H, HIDDEN_H + crate::metrics::line_h()));
        assert_eq!((d.layout.row_at(0.), d.layout.row_at(HIDDEN_H - 1.), d.layout.row_at(HIDDEN_H)), (0, 0, 1));

        let long = "word ".repeat(100);
        let mut d = state(&format!("a\n{long}\nb"), &format!("a\n{long}\nb"));
        d.ensure_layout(600., 200.);
        assert_eq!(d.layout.content(), 3. * crate::metrics::line_h());
        d.wrap = true;
        d.ensure_layout(600., 200.);
        let lines = 1 + d.layout.breaks(false, 1).len();
        assert!(lines > 1);
        assert_eq!(d.layout.content(), (2 + lines) as f32 * crate::metrics::line_h());
        assert_eq!(d.layout.row_at(crate::metrics::line_h() * 2.), 1, "a continuation belongs to its row");
    }

    #[test]
    fn a_diff_opens_with_its_first_change_centred_and_scroll_stays_in_range() {
        let old: String = (0..100).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l50\n", "changed\n");
        let mut d = state(&old, &new);
        d.ensure_layout(600., 180.);
        assert_eq!(d.scroll, 50. * crate::metrics::line_h() - 90. + crate::metrics::line_h() / 2., "row 50 in the middle");
        d.scroll_by(-1e6);
        assert_eq!(d.scroll, 0.);
        d.scroll_by(1e6);
        assert_eq!(d.scroll, d.layout.content() - crate::metrics::line_h(), "the last line can reach the top");
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
        assert_eq!(d.scroll, 122. * crate::metrics::line_h(), "three rows above the change");
        // Wider: the same byte stays at the top.
        d.ensure_layout(1400., 600.);
        assert_eq!(d.hex.as_ref().unwrap().per, 16);
        assert_eq!(d.scroll, 61. * crate::metrics::line_h());
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
        let notice = |v: Value| match contents("a.rs", false, &v, false, false, &svg) {
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
