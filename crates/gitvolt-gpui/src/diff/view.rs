//! The diff panel: its header, toolbar, and the rows, drawn with Monaco's metrics from the web UI
//! (`ui/src/diff/{DiffPanel,DiffToolbar,options}`). Each row is shaped and painted when it's on
//! screen; GPUI keeps what it shaped from frame to frame.

use super::language::highlight_language;
use super::model::*;
use super::textmate::{FONT_BOLD, FONT_ITALIC, FONT_UNDERLINE, Span, Tokenizer};
use crate::app::{GitVolt, Tab, icon};
use crate::details::{FileChange, segment, segmented, status_icon};
use crate::theme;
use gitvolt_core::api::Request;
use gitvolt_core::error::GbError;
use gpui::prelude::*;
use gpui::*;
use serde_json::Value;
use similar::DiffOp;
use std::cell::Cell;
use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const LINE_H: f32 = 18.;
const FONT_SIZE: f32 = 13.;
const NUM_W: f32 = 40.;
const SIGN_W: f32 = 18.;
const SCROLLBAR_W: f32 = 10.;
const RULER_W: f32 = 30.;
/// A split's left pane has more margin before its text than the right one (Monaco's).
const SPLIT_LEFT_MARGIN: f32 = 18.;
const HEADER_H: f32 = 37.;
const TOOLBAR_H: f32 = 33.;
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
    Binary,
    Image,
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
    pub list: UniformListScrollHandle,
    pub hscroll: f32,
    pub selection: Option<Selection>,
    pub cursor: usize,
    pub drag: Drag,
    /// The text area, as last painted, for the mouse.
    pub body: Rc<Cell<Bounds<Pixels>>>,
    /// A newer read replaces what an older one was going to show.
    pub generation: u64,
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
            list: UniformListScrollHandle::new(),
            hscroll: 0.,
            selection: None,
            cursor: 0,
            drag: Drag::None,
            body: Rc::new(Cell::new(Bounds::default())),
            generation: 0,
        }
    }

    pub fn relayout(&mut self) {
        let Some(data) = &self.data else { return };
        let rows = match self.view {
            ViewKind::File => file_rows(&data.new, &data.old),
            ViewKind::Diff => rows(&data.ops, self.mode, &self.expanded),
        };
        self.changes = change_starts(&rows);
        self.ruler = Rc::new(ruler_marks(&rows));
        self.rows = Rc::new(rows);
        self.selection = None;
    }

    pub fn set_data(&mut self, data: Arc<DiffData>) {
        self.data = Some(data);
        self.notice = None;
        self.relayout();
        // As Monaco: a diff opens with its first change in the middle.
        if let Some(first) = self.changes.first() {
            self.list.scroll_to_item(*first, ScrollStrategy::Center);
        }
    }

    /// The first change at or past the top (F7), or the last one before it (Shift+F7).
    pub fn step_change(&mut self, forward: bool, top: usize) -> Option<usize> {
        let found = if forward { self.changes.iter().find(|c| **c > top).or(self.changes.first()) } else { self.changes.iter().rev().find(|c| **c < top).or(self.changes.last()) };
        found.copied()
    }

    /// The selected text, lines joined by `\n`.
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
    if matches!(r.kind, RowKind::Hidden { .. }) {
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
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let Some(file) = tab.files.as_ref().and_then(|f| f.changes.get(index)).cloned() else { return };
        // The view's choices carry from file to file.
        let (view, mode, ignore) = tab.diff.as_ref().map_or((ViewKind::Diff, Mode::Inline, false), |d| (d.view, d.mode, d.ignore_whitespace));
        let generation = tab.diff.as_ref().map_or(0, |d| d.generation) + 1;
        let mut state = DiffState::new(file);
        (state.view, state.mode, state.ignore_whitespace, state.generation) = (view, mode, ignore, generation);
        tab.diff = Some(state);
        tab.sidebar_open = false;
        self.read_contents(false, cx);
        cx.notify();
    }

    pub fn close_file(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.diff = None;
            cx.notify();
        }
    }

    /// Reads the open file's sides, aligns them off the UI thread, then colours them.
    pub fn read_contents(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(diff) = &tab.diff else { return };
        let (repo, generation, ignore) = (tab.id, diff.generation, diff.ignore_whitespace);
        let file = diff.file.clone();
        let api = self.api.clone();
        let read = self.rt.spawn(async move {
            let (old, new) = (serde_json::from_value(file.old.clone()), serde_json::from_value(file.new.clone()));
            let (Ok(old), Ok(new)) = (old, new) else { return Err(GbError::other("This file's sides can't be read")) };
            let v = api.dispatch(Request::DiffContents { repo, path: file.path.to_string(), old, new, force, old_path: file.old_path.clone() }).await?;
            let path = file.path.to_string();
            tokio::task::spawn_blocking(move || contents(&path, &v, ignore)).await.map_err(|e| GbError::other(e.to_string()))
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                let Some(diff) = this.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.diff.as_mut()).filter(|d| d.generation == generation) else { return };
                match result {
                    Ok(Ok(Read::Text(data, texts))) => {
                        diff.set_data(data.clone());
                        diff.stop_colours.store(true, Ordering::Relaxed);
                        diff.stop_colours = Arc::default();
                        diff.highlight = Arc::default();
                        let stop = diff.stop_colours.clone();
                        this.color(repo, generation, data, texts, stop, cx);
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

    /// Colours both sides from their tops, taking turns, a piece at a time.
    fn color(&mut self, repo: u32, generation: u64, data: Arc<DiffData>, texts: Texts, stop: Arc<AtomicBool>, cx: &mut Context<Self>) {
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
            loop {
                let piece = rx.recv().await;
                let done = piece.is_none();
                let live = this.update(cx, |this, cx| {
                    let Some(diff) = this.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.diff.as_mut()).filter(|d| d.generation == generation) else { return false };
                    diff.coloured = true;
                    if let Some(piece) = piece {
                        let h = Arc::make_mut(&mut diff.highlight);
                        if piece.old_side { &mut h.old } else { &mut h.new }.push(Arc::new(piece.spans));
                        cx.notify();
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
            let top = (-d.list.0.borrow().base_handle.offset().y.as_f32() / LINE_H) as usize;
            if let Some(row) = d.step_change(forward, top) {
                d.list.scroll_to_item(row, ScrollStrategy::Center);
                d.cursor = row;
            }
        });
    }

    pub fn scroll_diff(&mut self, rows: f32, cx: &mut Context<Self>) {
        self.with_diff(cx, |d| {
            let handle = d.list.0.borrow().base_handle.clone();
            let y = (handle.offset().y - px(rows * LINE_H)).clamp(-handle.max_offset().y, px(0.));
            handle.set_offset(point(handle.offset().x, y));
        });
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

struct Texts {
    path: String,
    old: Option<String>,
    new: Option<String>,
}

enum Read {
    Text(Arc<DiffData>, Texts),
    Notice(Notice),
}

fn contents(path: &str, v: &Value, ignore_whitespace: bool) -> Read {
    if v["tooLarge"].as_bool() == Some(true) {
        return Read::Notice(Notice::TooLarge);
    }
    if v["image"].as_bool() == Some(true) {
        return Read::Notice(Notice::Image);
    }
    let binary = |s: &Value| s["binary"].as_bool() == Some(true);
    if binary(&v["old"]) || binary(&v["new"]) {
        return Read::Notice(Notice::Binary);
    }
    let (old, new) = (blob_text(&v["old"]), blob_text(&v["new"]));
    let encoding = v["new"]["encoding"].as_str().or(v["old"]["encoding"].as_str()).unwrap_or_default().to_string();
    let data = DiffData::build(old.as_deref(), new.as_deref(), ignore_whitespace, encoding.into(), v["eolOnly"].as_bool() == Some(true));
    Read::Text(Arc::new(data), Texts { path: path.to_string(), old, new })
}

fn mono() -> Font {
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
}

fn shape(line: &Line, spans: &[Span], font: &Font, window: &Window) -> ShapedLine {
    window.text_system().shape_line(line.text.clone(), px(FONT_SIZE), &runs(&line.text, spans, font), None)
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

/// One pane's text: from `x`, clipped to `clip`, with its guides, word marks and selection under it.
#[allow(clippy::too_many_arguments)]
fn paint_text(ix: usize, line: &Line, spans: &[Span], old_side: bool, mark: Hsla, x: Pixels, y: Pixels, clip: Bounds<Pixels>, p: &Painter, window: &mut Window, cx: &mut App) {
    window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
        let start = x - px(p.hscroll);
        let shaped = shape(line, spans, &p.font, window);
        let advance = shaped.x_for_index(1).max(px(1.));
        for level in 1..=line.guides {
            let gx = start + advance * ((usize::from(level) - 1) * TAB_SIZE) as f32;
            window.paint_quad(fill(Bounds::new(point(gx, y), size(px(1.), px(LINE_H))), theme::c(0x404040)));
        }
        for &(a, b) in &line.marks {
            let (from, to) = (shaped.x_for_index(a as usize), shaped.x_for_index(b as usize));
            window.paint_quad(fill(Bounds::from_corners(point(start + from, y), point(start + to, y + px(LINE_H))), mark));
        }
        if let Some(sel) = p.selection.filter(|s| !p.split || s.old_side == old_side) {
            let (a, b) = sel.range();
            if a.row <= ix && ix <= b.row {
                let from = if ix == a.row { shaped.x_for_index(a.byte.min(line.text.len())) } else { px(0.) };
                let to = if ix == b.row { shaped.x_for_index(b.byte.min(line.text.len())) } else { shaped.width + advance };
                if to > from {
                    window.paint_quad(fill(Bounds::from_corners(point(start + from, y), point(start + to, y + px(LINE_H))), theme::selection_bg()));
                }
            }
        }
        if !line.text.is_empty() {
            shaped.paint(point(start, y), px(LINE_H), TextAlign::Left, None, window, cx).ok();
        }
    });
}

fn spans_of(pieces: &[Arc<Vec<Vec<Span>>>], n: u32) -> &[Span] {
    let n = n as usize;
    pieces.get(n / COLOUR_CHUNK).and_then(|p| p.get(n % COLOUR_CHUNK)).map_or(&[], Vec::as_slice)
}

fn paint_inline(b: Bounds<Pixels>, ix: usize, r: &Row, p: &Painter, window: &mut Window, cx: &mut App) {
    let y = b.origin.y;
    let num = theme::c(if ix == p.cursor { 0xC6C6C6 } else { 0x858585 });
    let (old_right, new_right) = (b.origin.x + px(NUM_W - 4.), b.origin.x + px(2. * NUM_W - 4.));
    let sign_x = b.origin.x + px(2. * NUM_W);
    let text_x = sign_x + px(SIGN_W);
    let clip = Bounds::from_corners(point(text_x, y), point(b.origin.x + b.size.width, y + px(LINE_H)));
    let hl = &p.highlight;
    match r.kind {
        RowKind::Removed => {
            window.paint_quad(fill(b, theme::diff_removed_line()));
            if let Some(n) = r.old {
                paint_number(n, old_right, y, num, p, window, cx);
                paint_sign("−", sign_x, y, theme::status_deleted(), p, window, cx);
                paint_text(ix, &p.data.old.lines[n as usize], spans_of(&hl.old, n), true, theme::diff_removed_text(), text_x, y, clip, p, window, cx);
            }
        }
        RowKind::Added => {
            window.paint_quad(fill(Bounds::from_corners(point(b.origin.x + px(NUM_W), y), point(b.origin.x + b.size.width, y + px(LINE_H))), theme::diff_inserted_line()));
            if let Some(n) = r.new {
                paint_number(n, new_right, y, num, p, window, cx);
                paint_sign("+", sign_x, y, theme::status_added(), p, window, cx);
                paint_text(ix, &p.data.new.lines[n as usize], spans_of(&hl.new, n), false, theme::diff_inserted_text(), text_x, y, clip, p, window, cx);
            }
        }
        _ => {
            if p.view == ViewKind::Diff
                && let Some(n) = r.old
            {
                paint_number(n, old_right, y, num, p, window, cx);
            }
            let (right, text_at, clip) = if p.view == ViewKind::File { (b.origin.x + px(NUM_W - 4.), b.origin.x + px(NUM_W + SIGN_W), Bounds::from_corners(point(b.origin.x + px(NUM_W + SIGN_W), y), point(b.origin.x + b.size.width, y + px(LINE_H)))) } else { (new_right, text_x, clip) };
            if let Some(n) = r.new {
                paint_number(n, right, y, num, p, window, cx);
                paint_text(ix, &p.data.new.lines[n as usize], spans_of(&hl.new, n), false, theme::diff_inserted_text(), text_at, y, clip, p, window, cx);
            } else if let Some(n) = r.old {
                paint_number(n, right, y, num, p, window, cx);
                paint_text(ix, &p.data.old.lines[n as usize], spans_of(&hl.old, n), true, theme::diff_removed_text(), text_at, y, clip, p, window, cx);
            }
        }
    }
}

fn paint_split(b: Bounds<Pixels>, ix: usize, r: &Row, p: &Painter, window: &mut Window, cx: &mut App) {
    let y = b.origin.y;
    let half = b.size.width / 2.;
    let hl = &p.highlight;
    let change = r.kind == RowKind::Change;
    let pane = |left: Pixels, n: Option<u32>, old_side: bool, window: &mut Window, cx: &mut App| {
        let area = Bounds::new(point(left, y), size(half, px(LINE_H)));
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
        let clip = Bounds::from_corners(point(text_x, y), point(left + half, y + px(LINE_H)));
        paint_text(ix, &side.lines[n as usize], spans, old_side, mark, text_x, y, clip, p, window, cx);
    };
    pane(b.origin.x, r.old, true, window, cx);
    pane(b.origin.x + half, r.new, false, window, cx);
    window.paint_quad(fill(Bounds::new(point(b.origin.x + half, y), size(px(1.), px(LINE_H))), theme::section_border()));
}

fn hidden_row(ix: usize, count: u32, region: u32, cx: &mut Context<GitVolt>) -> AnyElement {
    div()
        .id(ix)
        .flex()
        .w_full()
        .h(px(LINE_H))
        .items_center()
        .bg(theme::panel_bg())
        .text_size(px(12.))
        .text_color(theme::text_normal())
        .hover(|d| d.text_color(theme::text_selected()))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.with_diff(cx, |d| {
                d.expanded.insert(region);
                d.relayout();
            });
        }))
        .child(div().flex().flex_none().w(px(2. * NUM_W + SIGN_W)).pl(px(2. * NUM_W - 16.)).child(icon("unfold-vertical", 14., theme::text_secondary())))
        .child(format!("{count} hidden lines"))
        .into_any_element()
}

/// The text position under the pointer.
fn pos_at(diff: &DiffState, position: Point<Pixels>, window: &Window) -> Option<(TextPos, bool)> {
    let (data, b) = (diff.data.as_ref()?, diff.body.get());
    let scroll = diff.list.0.borrow().base_handle.offset().y;
    let row = (((position.y - b.origin.y - scroll) / px(LINE_H)).floor().max(0.) as usize).min(diff.rows.len().saturating_sub(1));
    let r = diff.rows.get(row)?;
    let split = diff.mode == Mode::Split && diff.view == ViewKind::Diff;
    let (old_side, gutter) = if split {
        let half = b.size.width / 2.;
        let left = position.x < b.origin.x + half;
        (left, if left { px(SPLIT_LEFT_MARGIN + NUM_W + SIGN_W) } else { half + px(NUM_W + SIGN_W) })
    } else if diff.view == ViewKind::File {
        (false, px(NUM_W + SIGN_W))
    } else {
        (r.new.is_none(), px(2. * NUM_W + SIGN_W))
    };
    let x = position.x - b.origin.x - gutter + px(diff.hscroll);
    let byte = row_line(data, r, split, old_side).map_or(0, |line| {
        let spans: &[Span] = &[];
        shape(line, spans, &mono(), window).closest_index_for_x(x.max(px(0.)))
    });
    Some((TextPos { row, byte }, old_side))
}

fn word_at(text: &str, byte: usize) -> (usize, usize) {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let byte = byte.min(text.len());
    let start = text[..byte].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(byte, |(i, _)| i);
    let end = text[byte..].char_indices().find(|(_, c)| !is_word(*c)).map_or(text.len(), |(i, _)| byte + i);
    (start, end)
}

fn text_area(diff: &DiffState, height: f32, width: f32, cx: &mut Context<GitVolt>) -> AnyElement {
    let Some(data) = diff.data.clone() else {
        return message(diff, cx).into_any_element();
    };
    let split = diff.mode == Mode::Split && diff.view == ViewKind::Diff;
    let painter = Rc::new(Painter { data: data.clone(), highlight: diff.highlight.clone(), view: diff.view, split, hscroll: diff.hscroll, selection: diff.selection, cursor: diff.cursor, font: mono() });
    let rows = diff.rows.clone();
    // As `scrollBeyondLastLine`: the last line can scroll to the top.
    let beyond = ((height / LINE_H) as usize).saturating_sub(1);
    let count = rows.len() + beyond;
    let list = uniform_list(
        "diff-rows",
        count,
        cx.processor(move |_, range: Range<usize>, _, cx| {
            range
                .map(|ix| {
                    let Some(r) = rows.get(ix).copied() else {
                        return div().id(ix).h(px(LINE_H)).into_any_element();
                    };
                    if let RowKind::Hidden { count, region } = r.kind {
                        return hidden_row(ix, count, region, cx);
                    }
                    let p = painter.clone();
                    div()
                        .id(ix)
                        .h(px(LINE_H))
                        .w_full()
                        .child(
                            canvas(
                                |_, _, _| {},
                                move |b, _, window, cx| {
                                    if p.split {
                                        paint_split(b, ix, &r, &p, window, cx);
                                    } else {
                                        paint_inline(b, ix, &r, &p, window, cx);
                                    }
                                },
                            )
                            .size_full(),
                        )
                        .into_any_element()
                })
                .collect()
        }),
    )
    .track_scroll(&diff.list)
    .size_full();

    let body_cell = diff.body.clone();
    let gutter = if diff.view == ViewKind::File { NUM_W + SIGN_W } else if split { SPLIT_LEFT_MARGIN + NUM_W + SIGN_W } else { 2. * NUM_W + SIGN_W };
    // JetBrains Mono's advance at this size, near enough for the horizontal scrollbar.
    let content = data.widest as f32 * FONT_SIZE * 0.6;
    let pane = if split { width / 2. } else { width };
    let max_h = (content - (pane - gutter)).max(0.);
    let hscroll = diff.hscroll;
    let entity = cx.entity().downgrade();
    div()
        .id("diff-body")
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .cursor(CursorStyle::IBeam)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                let Some(diff) = this.tabs.get_mut(this.active).and_then(|t| t.diff.as_mut()) else { return };
                let Some((at, old_side)) = pos_at(diff, e.position, window) else { return };
                let split = diff.mode == Mode::Split && diff.view == ViewKind::Diff;
                let line = diff.data.as_ref().and_then(|d| diff.rows.get(at.row).and_then(|r| row_line(d, r, split, old_side)));
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
                cx.notify();
            }),
        )
        .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
            let Some(diff) = this.tabs.get_mut(this.active).and_then(|t| t.diff.as_mut()) else { return };
            if diff.drag != Drag::Text || e.pressed_button != Some(MouseButton::Left) {
                return;
            }
            let Some((at, _)) = pos_at(diff, e.position, window) else { return };
            if let Some(sel) = diff.selection.as_mut() {
                sel.head = at;
                diff.cursor = at.row;
                cx.notify();
            }
        }))
        .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.with_diff_quiet(|d| d.drag = Drag::None)))
        .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, _| this.with_diff_quiet(|d| d.drag = Drag::None)))
        .child(list)
        .child(
            canvas(
                move |_, _, _| {},
                move |bounds, _, window, _| {
                    body_cell.set(bounds);
                    // As Monaco's: only while the pointer is over the text, or it's scrolled.
                    if max_h > 0. && (hscroll > 0. || bounds.contains(&window.mouse_position())) {
                        let track = bounds.size.width - px(gutter);
                        let thumb = (track * (track / px(content))).max(px(24.));
                        let x = bounds.origin.x + px(gutter) + (track - thumb) * (hscroll / max_h);
                        window.paint_quad(quad(Bounds::new(point(x, bounds.origin.y + bounds.size.height - px(SCROLLBAR_W)), size(thumb, px(SCROLLBAR_W))), px(0.), theme::scroll_thumb(), px(0.), transparent_black(), BorderStyle::default()));
                    }
                    // Sideways wheel, and Shift+wheel, before the list takes it.
                    let entity = entity.clone();
                    window.on_mouse_event(move |e: &ScrollEvent, phase, window, cx| {
                        if phase != DispatchPhase::Capture || !bounds.contains(&e.position) {
                            return;
                        }
                        let delta = e.delta.pixel_delta(window.line_height());
                        let (dx, consume) = if e.modifiers.shift { (delta.y, true) } else { (delta.x, delta.x.abs() > delta.y.abs()) };
                        if dx == px(0.) {
                            return;
                        }
                        if consume {
                            cx.stop_propagation();
                        }
                        entity
                            .update(cx, |this, cx| {
                                this.with_diff(cx, |d| d.hscroll = (d.hscroll - dx.as_f32()).clamp(0., max_h));
                            })
                            .ok();
                    });
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .into_any_element()
}

/// The overview ruler: each change's place in the file, and the viewport.
fn ruler(diff: &DiffState, height: f32, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let marks = diff.ruler.clone();
    let handle = diff.list.0.borrow().base_handle.clone();
    let beyond = ((height / LINE_H) as usize).saturating_sub(1);
    let total = (diff.rows.len() + beyond).max(1) as f32 * LINE_H;
    let jump = move |this: &mut GitVolt, y: Pixels, cx: &mut Context<GitVolt>, top: Pixels, h: Pixels| {
        this.with_diff(cx, |d| {
            let handle = d.list.0.borrow().base_handle.clone();
            let share = ((y - top) / h).clamp(0., 1.);
            handle.set_offset(point(handle.offset().x, -handle.max_offset().y * share));
        });
    };
    let cell = Rc::new(Cell::new(Bounds::<Pixels>::default()));
    let (paint_cell, down, moved) = (cell.clone(), cell.clone(), cell);
    div()
        .id("diff-ruler")
        .relative()
        .flex_none()
        .w(px(RULER_W))
        .h_full()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                this.with_diff_quiet(|d| d.drag = Drag::Ruler);
                let b = down.get();
                jump(this, e.position.y, cx, b.origin.y, b.size.height);
            }),
        )
        .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
            let dragging = this.tabs.get(this.active).and_then(|t| t.diff.as_ref()).is_some_and(|d| d.drag == Drag::Ruler);
            if dragging && e.pressed_button == Some(MouseButton::Left) {
                let b = moved.get();
                jump(this, e.position.y, cx, b.origin.y, b.size.height);
            }
        }))
        .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.with_diff_quiet(|d| d.drag = Drag::None)))
        .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, _| this.with_diff_quiet(|d| d.drag = Drag::None)))
        .child(
            canvas(
                |_, _, _| {},
                move |b, _, window, _| {
                    paint_cell.set(b);
                    let scale = b.size.height / px(total);
                    let half = b.size.width / 2.;
                    for &(first, rows, removed) in marks.iter() {
                        let top = px(first as f32 * LINE_H) * scale;
                        let h = (px(rows as f32 * LINE_H) * scale).max(px(4.));
                        let x = if removed { b.origin.x } else { b.origin.x + half };
                        window.paint_quad(fill(Bounds::new(point(x, b.origin.y + top), size(half, h)), if removed { theme::ruler_removed() } else { theme::ruler_added() }));
                    }
                    let top = -handle.offset().y * scale;
                    window.paint_quad(fill(Bounds::new(point(b.origin.x, b.origin.y + top), size(b.size.width, (px(height) * scale).min(b.size.height))), theme::scroll_thumb()));
                },
            )
            .absolute()
            .top_0()
            .left_0()
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
        Some(Notice::Binary) => base.child("Binary file"),
        Some(Notice::Image) => base.child("Image preview isn't in this build yet"),
        _ => base,
    }
}

fn header(diff: &DiffState, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let path = diff.file.path.as_ref();
    let (dir, name) = match path.rsplit_once('/') {
        Some((d, n)) => (format!("{d}/"), n.to_string()),
        None => (String::new(), path.to_string()),
    };
    let (glyph, color) = status_icon(diff.file.status);
    let encoding = diff.data.as_ref().map(|d| d.encoding.clone()).unwrap_or_default();
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
        .child(div().flex().flex_1().min_w_0().overflow_hidden().child(div().flex_none().text_color(theme::text_normal()).child(dir)).child(div().flex_none().font_weight(FontWeight::BOLD).text_color(theme::text_selected()).child(name)))
        .child(div().flex_none().text_size(px(11.)).text_color(theme::text_secondary()).child(encoding))
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

fn toolbar(diff: &DiffState, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let in_diff = diff.view == ViewKind::Diff;
    let can_diff = diff.file.status != 'A' || diff.data.as_ref().is_none_or(|d| d.has_old);
    let view_button = |id: &'static str, label: &'static str, kind: ViewKind, on: bool, enabled: bool, cx: &mut Context<GitVolt>| {
        segment(id, label, on, enabled)
            .when(enabled, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    this.with_diff(cx, |d| {
                        d.view = kind;
                        d.hscroll = 0.;
                        d.relayout();
                    });
                }))
            })
            .into_any_element()
    };
    let mode_button = |id: &'static str, label: &'static str, mode: Mode, cx: &mut Context<GitVolt>| {
        segment(id, label, in_diff && diff.mode == mode, in_diff)
            .when(in_diff, |d| {
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
        let color = if !enabled { theme::text_disabled() } else if pressed { theme::text_selected() } else { theme::text_secondary() };
        div().id(id).flex().flex_none().size(px(22.)).items_center().justify_center().rounded(px(4.)).when(pressed, |d| d.bg(theme::selected_row())).when(enabled, |d| d.hover(|d| d.bg(theme::hover_row()))).child(icon(name, 14., color))
    };
    let can_step = in_diff && !diff.changes.is_empty();
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
        .child(div().flex_1())
        .child(segmented(vec![view_button("view-file", "File View", ViewKind::File, !in_diff, true, cx), view_button("view-diff", "Diff View", ViewKind::Diff, in_diff, can_diff, cx)]))
        .child(
            div()
                .flex()
                .flex_1()
                .justify_end()
                .items_center()
                .gap(px(10.))
                // Blame and History aren't in this build yet.
                .child(segmented(vec![segment("blame", "Blame", false, false).into_any_element(), segment("history", "History", false, false).into_any_element()]))
                .child(
                    div()
                        .flex()
                        .gap(px(2.))
                        .child(icon_button("change-prev", "arrow-up", false, can_step).when(can_step, |d| d.on_click(cx.listener(|this, _, _, cx| this.step_change(false, cx)))))
                        .child(icon_button("change-next", "arrow-down", false, can_step).when(can_step, |d| d.on_click(cx.listener(|this, _, _, cx| this.step_change(true, cx))))),
                )
                .child(segmented(vec![mode_button("mode-hunk", "Hunk", Mode::Hunk, cx), mode_button("mode-inline", "Inline", Mode::Inline, cx), mode_button("mode-split", "Split", Mode::Split, cx)]))
                .child(
                    div()
                        .flex()
                        .gap(px(2.))
                        .child(icon_button("ignore-ws", "pilcrow", diff.ignore_whitespace, in_diff).when(in_diff, |d| d.on_click(cx.listener(|this, _, _, cx| this.toggle_whitespace(cx)))))
                        // Word wrap isn't in this build yet.
                        .child(icon_button("wrap", "text-wrap", false, false)),
                ),
        )
}

/// The panel for `tab`'s open file, `width` by `height` px.
pub fn diff_panel(tab: &Tab, width: f32, height: f32, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let diff = tab.diff.as_ref()?;
    let body_h = height - HEADER_H - TOOLBAR_H;
    let split = diff.mode == Mode::Split && diff.view == ViewKind::Diff;
    let body_w = width - RULER_W - if split { 0. } else { SCROLLBAR_W };
    let banner = diff.data.as_ref().is_some_and(|d| d.eol_only);
    Some(
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(theme::app_bg())
            .child(header(diff, cx))
            .child(toolbar(diff, cx))
            .when(banner, |d| d.child(div().flex_none().px(px(12.)).py(px(4.)).bg(theme::yellow_banner()).text_color(theme::text_selected()).child("Only the line endings changed")))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(text_area(diff, body_h, body_w, cx))
                    .when(diff.data.is_some() && !split, |d| d.child(div().flex_none().w(px(SCROLLBAR_W)).h_full().border_l_1().border_color(theme::section_border())))
                    .when(diff.data.is_some(), |d| d.child(ruler(diff, body_h, cx))),
            )
            .into_any_element(),
    )
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
        FileChange { path: "a.rs".into(), old_path: None, status: 'M', additions: None, deletions: None, old: Value::Null, new: Value::Null }
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
    fn a_double_click_takes_the_word_under_it() {
        assert_eq!(word_at("let some_name = 1", 6), (4, 13));
        assert_eq!(word_at("a b", 1), (0, 1));
        assert_eq!(word_at("", 0), (0, 0));
    }

    #[test]
    fn what_a_sides_read_says_decides_what_shows() {
        let notice = |v: Value| match contents("a.rs", &v, false) {
            Read::Notice(n) => Some(n),
            Read::Text(..) => None,
        };
        assert_eq!(notice(serde_json::json!({"tooLarge": true})), Some(Notice::TooLarge));
        assert_eq!(notice(serde_json::json!({"image": true})), Some(Notice::Image));
        assert_eq!(notice(serde_json::json!({"new": {"binary": true}})), Some(Notice::Binary));
        assert_eq!(notice(serde_json::json!({"old": {"text": "a"}, "new": {"text": "b", "encoding": "UTF-8"}})), None);
    }
}
