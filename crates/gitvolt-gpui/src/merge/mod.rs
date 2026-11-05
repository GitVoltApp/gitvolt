//! The merge tool (`ui/src/conflicts/`): Current and Incoming side by side, the output below. A conflicted WIP
//! file opens it in place of the diff. The panes show each side's whole file with its regions tinted; a tick
//! rebuilds only its region in the output, an undoable edit that takes the ticks back too; hand edits elsewhere
//! stay. The work lives in `Tab::merge` while its file is open: every way of leaving it asks first
//! (`GitVolt::ask_unsaved`), and a hidden tab keeps it. Saving marks the file resolved.

pub mod model;
pub mod view;

use crate::app::{GitVolt, KeyArea, Tab};
use crate::details::{FileChange, Origin};
use crate::diff::edit::EditState;
use crate::diff::model::Side as Lines;
use crate::diff::textmate::{Span as TmSpan, Tokenizer};
use crate::ui::arm::{self, ArmRequest, Origin as ArmOrigin, Tone};
use crate::ui::popover::{self, Choice, ChoiceRequest};
use crate::ui::toast;
use crate::wip::Which;
use crate::write::{Write, WriteCtx, call};
use gitvolt_core::api::Request;
use gitvolt_core::error::{GbError, GbErrorKind};
use gpui::*;
use model::*;
use serde_json::{Value, json};
use std::cell::Cell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

pub const LINE_H: f32 = 18.;
const COLOUR_CHUNK: usize = 128;
/// A switch keeps the previous file up this long while the next one is read.
const SWITCH_WAIT: Duration = Duration::from_millis(200);
/// The merge tool's unsaved work on a file would be replaced: its row's actions wait.
pub const MERGE_SAVE_FIRST: &str = "Save the merge first";

/// What the panes' header and the output's bar take of the height.
const BAR_H: f32 = 29.;
pub const RULER_W: f32 = 10.;
/// The panes' gutter: the hunk column, the numbers, the line buttons' lane.
pub const HUNK_W: f32 = 18.;
pub const NUM_W: f32 = 40.;
pub const LANE_W: f32 = 22.;
pub const PANE_GUTTER: f32 = HUNK_W + NUM_W + LANE_W;
pub const OUT_GUTTER: f32 = NUM_W + 10.;
const MINIMAP_MAX: f32 = 120.;

/// Monaco's minimap for an editor this wide: a character a pixel wide, 8 px of gutter, 120 columns at most.
pub fn minimap_width(width: f32, gutter: f32) -> f32 {
    (((width - gutter - 12.) / 9.).floor().max(0.) + 8.).min(MINIMAP_MAX)
}

/// The conflict's file, whichever side of the worktree shows it.
pub fn is_conflict(file: &FileChange) -> bool {
    file.status == 'U' && matches!(file.origin, Origin::Wip { .. })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneDrag {
    None,
    Text,
    Strip,
    Horizontal,
}

/// One of the three editors' view of its text.
pub struct TextView {
    pub lines: Arc<Lines>,
    pub colours: Vec<Arc<Vec<Vec<TmSpan>>>>,
    pub scroll: f32,
    pub hscroll: f32,
    pub height: f32,
    pub width: f32,
    pub widest: usize,
    /// The editor, as last painted, for the mouse.
    pub body: Rc<Cell<Bounds<Pixels>>>,
    pub hover: Option<usize>,
    pub pointer_in: bool,
    pub drag: PaneDrag,
    pub focus: FocusHandle,
}

impl TextView {
    fn new(text: &str, cx: &mut App) -> Self {
        let lines = Lines::new(Some(text));
        let widest = lines.lines.iter().map(|l| l.text.len()).max().unwrap_or(0);
        Self { lines: Arc::new(lines), colours: Vec::new(), scroll: 0., hscroll: 0., height: 0., width: 0., widest, body: Rc::default(), hover: None, pointer_in: false, drag: PaneDrag::None, focus: cx.focus_handle() }
    }

    fn set_text(&mut self, text: &str) {
        let lines = Lines::new(Some(text));
        self.widest = lines.lines.iter().map(|l| l.text.len()).max().unwrap_or(0);
        self.lines = Arc::new(lines);
    }

    pub fn line_count(&self) -> usize {
        self.lines.lines.len().max(1)
    }

    pub fn content(&self) -> f32 {
        self.line_count() as f32 * LINE_H
    }

    pub fn max_scroll(&self) -> f32 {
        (self.content() - self.height).max(0.)
    }

    /// Where line `n` (0-based) goes to stand in the middle.
    fn centre_on(&self, n: usize) -> f32 {
        (n as f32 * LINE_H - (self.height - LINE_H) / 2.).clamp(0., self.max_scroll())
    }

    /// The line at `y` px from the editor's top.
    pub fn line_at(&self, y: f32) -> usize {
        (((y + self.scroll) / LINE_H).max(0.) as usize).min(self.line_count() - 1)
    }

}

/// A read-only side: a caret and a selection, in display bytes.
pub struct SidePane {
    pub side: Side,
    pub view: TextView,
    pub data: PaneData,
    pub caret: (usize, usize),
    pub anchor: Option<(usize, usize)>,
    pub want_col: Option<usize>,
}

impl SidePane {
    pub fn selected_text(&self) -> Option<String> {
        let a = self.anchor.filter(|a| *a != self.caret)?;
        let (from, to) = if a < self.caret { (a, self.caret) } else { (self.caret, a) };
        let lines = &self.view.lines.lines;
        let mut out = String::new();
        for n in from.0..=to.0 {
            let text = lines.get(n)?.text.as_ref();
            let a = if n == from.0 { from.1.min(text.len()) } else { 0 };
            let b = if n == to.0 { to.1.min(text.len()) } else { text.len() };
            out.push_str(&text[a..b]);
            if n != to.0 {
                out.push('\n');
            }
        }
        Some(out)
    }

    fn line_len(&self, n: usize) -> usize {
        self.view.lines.lines.get(n).map_or(0, |l| l.text.len())
    }

    /// The caret goes to `at`; `select` keeps or starts a selection.
    fn caret_to(&mut self, at: (usize, usize), select: bool) {
        if select {
            self.anchor = self.anchor.or(Some(self.caret));
        } else {
            self.anchor = None;
        }
        self.caret = at;
        let n = at.0 as f32 * LINE_H;
        if n < self.view.scroll {
            self.view.scroll = n;
        } else if n + LINE_H > self.view.scroll + self.view.height {
            self.view.scroll = n + LINE_H - self.view.height;
        }
    }
}

/// The output's marks as of an edit: what an undo or redo brings back with its text.
#[derive(Debug, Clone, PartialEq)]
struct Marks {
    picks: Picks,
    spans: Vec<Span>,
    edited: BTreeSet<u32>,
    typed: bool,
}

pub struct Output {
    pub view: TextView,
    pub edit: EditState,
    pub spans: Vec<Span>,
    pub edited: BTreeSet<u32>,
    pub typed: bool,
    pub regions: Vec<OutRegion>,
    marks: HashMap<u64, Marks>,
    /// The output's length, and whether it ends without a line break, as of the last change.
    last_len: usize,
    last_open_end: bool,
    version_seen: u64,
    pub recolour: u64,
}

impl Output {
    fn relayout(&mut self) {
        let text = &self.edit.buffer.text;
        self.regions = self.spans.iter().map(|s| region_of(text, s)).collect();
        self.view.set_text(text);
    }

    fn remember(&mut self) {
        let text = &self.edit.buffer.text;
        self.last_len = text.len();
        self.last_open_end = !text.is_empty() && !text.ends_with('\n');
    }

    pub fn caret_line(&self) -> usize {
        self.edit.line_col(self.edit.buffer.caret()).0
    }

    /// Each region's place, top to bottom.
    pub fn regions_sorted(&self) -> Vec<OutRegion> {
        let mut r = self.regions.clone();
        r.sort_by_key(|x| x.start);
        r
    }

    /// What Save sends: an output nobody typed in goes out as built, so a mixed-EOL file keeps each line's own.
    fn text_to_save(&self, built: &str) -> String {
        if normalized(&self.edit.buffer.text) == normalized(built) { built.to_string() } else { self.edit.text_to_save() }
    }

    fn caret_to(&mut self, at: usize) {
        self.edit.buffer.move_to(at);
        self.edit.want_col = None;
    }

    pub fn show_caret(&mut self) {
        let n = self.caret_line() as f32 * LINE_H;
        if n < self.view.scroll {
            self.view.scroll = n;
        } else if n + LINE_H > self.view.scroll + self.view.height {
            self.view.scroll = (n + LINE_H - self.view.height).min(self.view.max_scroll());
        }
    }
}

pub enum Loaded {
    Loading,
    Failed(String),
    /// `None`: the file isn't conflicted any more.
    Ready(Option<Rc<ConflictFile>>),
}

pub struct MergeTool {
    pub worktree: String,
    pub path: String,
    pub loaded: Loaded,
    pub picks: Picks,
    pub panes: Vec<SidePane>,
    pub output: Option<Output>,
    pub saving: bool,
    /// A non-text conflict's write is on its way.
    pub busy: bool,
    /// Where it opens, once it's been given its size.
    pending_reveal: Option<u32>,
    /// The file the header names while this one's still up.
    pub generation: u64,
    /// The base as the work started from it; `None` with a file means no file on disk.
    pub base: Option<String>,
}

impl MergeTool {
    pub fn loading(worktree: String, path: String) -> Self {
        Self { worktree, path, loaded: Loaded::Loading, picks: Picks::new(), panes: Vec::new(), output: None, saving: false, busy: false, pending_reveal: None, generation: 0, base: None }
    }

    pub fn file(&self) -> Option<&Rc<ConflictFile>> {
        match &self.loaded {
            Loaded::Ready(f) => f.as_ref(),
            _ => None,
        }
    }

    /// Nothing to keep: no tick, no hand edit.
    pub fn dirty(&self) -> bool {
        self.output.as_ref().is_some_and(|o| o.typed || !o.edited.is_empty()) || any_picked(&self.picks)
    }

    pub fn textual(&self) -> bool {
        self.file().is_some_and(|f| f.textual())
    }

    fn build(file: Rc<ConflictFile>, picks: Picks, base: Option<String>, generation: u64, worktree: String, cx: &mut App) -> Self {
        let path = file.path.clone();
        let mut tool = Self::loading(worktree.clone(), path);
        tool.generation = generation;
        tool.base = base.clone();
        if file.textual() {
            let eol = eol_text(&file.eol);
            let (text, _) = build_output(&file.segments, &picks, eol);
            let mut panes = Vec::new();
            for side in [Side::Current, Side::Incoming] {
                let data = file.pane(side).cloned().unwrap_or_default();
                panes.push(SidePane { side, view: TextView::new(&data.text, cx), data, caret: (0, 0), anchor: None, want_col: None });
            }
            let edit = EditState::new(&text, None, base.unwrap_or_default(), worktree, cx);
            let view = TextView::new(&edit.buffer.text, cx);
            let spans = spans_of_built(&file.segments, &picks, eol);
            let mut output = Output { view, edit, spans, edited: BTreeSet::new(), typed: false, regions: Vec::new(), marks: HashMap::new(), last_len: 0, last_open_end: false, version_seen: 0, recolour: 0 };
            output.relayout();
            output.remember();
            output.marks.insert(output.edit.buffer.version(), Marks { picks: picks.clone(), spans: output.spans.clone(), edited: BTreeSet::new(), typed: false });
            output.version_seen = output.edit.buffer.version();
            tool.panes = panes;
            tool.output = Some(output);
            tool.pending_reveal = opening_region(&file.segments, &picks, &BTreeSet::new());
        }
        tool.picks = picks;
        tool.loaded = Loaded::Ready(Some(file));
        tool
    }

    /// The panel's room for it: `width` by `height` under the file's header.
    pub fn set_size(&mut self, width: f32, height: f32) {
        let editor = ((height - 1.) / 2. - BAR_H).floor().max(0.);
        let half = (width / 2.).floor();
        for (i, p) in self.panes.iter_mut().enumerate() {
            p.view.height = editor;
            p.view.width = if i == 0 { half } else { width - half - 1. };
            p.view.scroll = p.view.scroll.clamp(0., p.view.max_scroll());
        }
        if let Some(o) = self.output.as_mut() {
            o.view.height = (height - 1. - editor - BAR_H - BAR_H).max(0.);
            o.view.width = width;
            o.view.scroll = o.view.scroll.clamp(0., o.view.max_scroll());
        }
        if editor > 0. && let Some(id) = self.pending_reveal.take() {
            self.reveal(id);
        }
    }

    /// Region `id` in view in all three, with the output's caret at its start.
    pub fn reveal(&mut self, id: u32) {
        for p in &mut self.panes {
            if let Some(r) = p.data.regions.iter().find(|g| g.id == id) {
                p.view.scroll = p.view.centre_on(r.start.saturating_sub(1));
            }
        }
        let Some(o) = self.output.as_mut() else { return };
        let Some(s) = o.spans.iter().find(|s| s.id == id).copied() else { return };
        o.caret_to(s.from);
        let line = o.caret_line();
        o.view.scroll = o.view.centre_on(line);
    }

    /// The others follow one that scrolled: down, to the same fraction of each one's range (their texts differ
    /// in length); sideways, to the same px.
    pub fn sync_scroll(&mut self, from: usize) {
        let (top, left, max) = {
            let v = self.view(from);
            (v.scroll, v.hscroll, v.max_scroll())
        };
        let f = if max > 0. { top / max } else { 0. };
        for i in 0..3 {
            if i == from || i >= self.editor_count() {
                continue;
            }
            let v = self.view_mut(i);
            v.scroll = f * v.max_scroll();
            v.hscroll = left;
        }
    }

    pub fn editor_count(&self) -> usize {
        self.panes.len() + usize::from(self.output.is_some())
    }

    /// 0 and 1: the panes; 2: the output.
    pub fn view(&self, i: usize) -> &TextView {
        if i < self.panes.len() { &self.panes[i].view } else { &self.output.as_ref().expect("an output").view }
    }

    pub fn view_mut(&mut self, i: usize) -> &mut TextView {
        if i < self.panes.len() { &mut self.panes[i].view } else { &mut self.output.as_mut().expect("an output").view }
    }

    /// `next` becomes the picks; the regions it changes are rebuilt in the output, as one edit.
    pub fn apply(&mut self, next: Picks) -> bool {
        let Some(file) = self.file().cloned() else { return false };
        let eol = eol_text(&file.eol);
        let prev = std::mem::replace(&mut self.picks, next);
        let Some(o) = self.output.as_mut() else { return false };
        let list: Vec<(u32, String)> = conflicts(&file.segments)
            .filter_map(|s| {
                let lines = region_lines(s, self.picks.get(&s.id));
                let same = prev.get(&s.id).is_some_and(|p| region_lines(s, Some(p)) == lines);
                (!same).then(|| (s.id, normalized(&region_text(&lines, eol))))
            })
            .collect();
        if list.is_empty() {
            o.marks.insert(o.edit.buffer.version(), Marks { picks: self.picks.clone(), spans: o.spans.clone(), edited: o.edited.clone(), typed: o.typed });
            return false;
        }
        // Later regions first: earlier ones' offsets stay good. Each span's place is worked out as it goes.
        let mut spans = o.spans.clone();
        spans.sort_by_key(|s| s.from);
        let mut ops = Vec::new();
        let mut next_spans = Vec::new();
        let mut delta: isize = 0;
        for s in &spans {
            let from = (s.from as isize + delta) as usize;
            match list.iter().find(|(id, _)| *id == s.id) {
                Some((_, t)) => {
                    ops.push((s.from..s.to, t.clone()));
                    next_spans.push(Span { id: s.id, from, to: from + t.len() });
                    delta += t.len() as isize - (s.to - s.from) as isize;
                }
                None => next_spans.push(Span { id: s.id, from, to: (s.to as isize + delta) as usize }),
            }
        }
        o.edit.buffer.apply_ops(&ops);
        o.spans = next_spans;
        for (id, _) in &list {
            o.edited.remove(id);
        }
        o.version_seen = o.edit.buffer.version();
        o.marks.insert(o.version_seen, Marks { picks: self.picks.clone(), spans: o.spans.clone(), edited: o.edited.clone(), typed: o.typed });
        o.edit.refresh_lines();
        o.relayout();
        o.remember();
        true
    }

    /// The output's text changed through the editor: its marks follow, or an undo or redo brings them back.
    pub fn after_output_edit(&mut self) {
        let Some(o) = self.output.as_mut() else { return };
        let version = o.edit.buffer.version();
        if version == o.version_seen {
            return;
        }
        o.version_seen = version;
        match o.edit.buffer.take_change() {
            None => {
                if let Some(m) = o.marks.get(&version).cloned() {
                    o.spans = m.spans;
                    o.edited = m.edited;
                    o.typed = m.typed;
                    self.picks = m.picks;
                }
            }
            Some((start, removed, inserted)) => {
                o.typed = true;
                let before = o.spans.clone();
                let mut tracked = track_spans(&before, start, removed, inserted);
                for (s, now) in before.iter().zip(tracked.iter_mut()) {
                    let open_end = s.from < s.to && s.to == o.last_len && o.last_open_end;
                    if !change_hits(start, removed, inserted, s.from, s.to, open_end) {
                        continue;
                    }
                    o.edited.insert(s.id);
                    // Typing into an empty region, or at the end of one that ends the file unterminated,
                    // extends it: those edges don't grow by themselves.
                    if (s.from == s.to && start == s.from) || (open_end && start == s.to) {
                        now.from = s.from;
                        now.to = s.to + inserted;
                    }
                }
                o.spans = tracked;
                let marks = Marks { picks: self.picks.clone(), spans: o.spans.clone(), edited: o.edited.clone(), typed: o.typed };
                o.marks.insert(version, marks);
            }
        }
        o.edit.refresh_lines();
        o.relayout();
        o.remember();
        o.show_caret();
    }

    /// A pane's Space or click: its region's line, or the whole hunk.
    pub fn toggle(&mut self, id: u32, side: Side, line: Option<usize>) -> bool {
        let Some(file) = self.file().cloned() else { return false };
        let Some(seg) = file.conflict(id) else { return false };
        let next = match line {
            Some(i) => toggle_line(&self.picks, id, side, i),
            None => toggle_hunk(&self.picks, seg, side),
        };
        self.apply(next);
        true
    }

    /// The count over the arrows: "Conflict 1 of 2", or "2 conflicts".
    pub fn count_text(&self) -> String {
        let Some(o) = self.output.as_ref() else { return String::new() };
        let regions = o.regions_sorted();
        if regions.is_empty() {
            return String::new();
        }
        let line = o.caret_line() + 1;
        match regions.iter().position(|r| line >= r.start && line < r.start + r.lines.max(1)) {
            Some(at) => format!("Conflict {} of {}", at + 1, regions.len()),
            None => format!("{} {}", regions.len(), if regions.len() == 1 { "conflict" } else { "conflicts" }),
        }
    }

    /// F7 and Shift+F7.
    pub fn step(&mut self, forward: bool) -> bool {
        let Some(o) = self.output.as_ref() else { return false };
        let Some(r) = next_region(&o.regions, o.caret_line() + 1, forward) else { return false };
        self.reveal(r.id);
        true
    }
}

impl Tab {
    /// The merge tool, while it's what the open file shows.
    pub fn merge_shown(&self) -> Option<&MergeTool> {
        self.diff.as_ref().filter(|d| is_conflict(&d.file) && self.history.is_none())?;
        self.merge.as_ref()
    }

    pub fn merge_shown_mut(&mut self) -> Option<&mut MergeTool> {
        self.diff.as_ref().filter(|d| is_conflict(&d.file) && self.history.is_none())?;
        self.merge.as_mut()
    }

    /// Unsaved merge work, in this tab.
    pub fn merge_dirty(&self) -> bool {
        self.merge_shown().is_some_and(MergeTool::dirty)
    }

    /// Unsaved merge work on this file (the WIP row's actions wait for it).
    pub fn merge_dirty_on(&self, path: &str) -> bool {
        self.merge_shown().is_some_and(|m| m.path == path && m.dirty())
    }
}

/// `sidesIn`: each side's name for the worktree's operation, as its pane titles name them.
pub fn sides_of(tab: &Tab, worktree: &str, target: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(g) = tab.graph.as_ref() else { return (None, None) };
    let branch = g.worktree_branch(worktree).map(|b| b.trim_start_matches("refs/heads/").to_string());
    let short = |s: &str| s.chars().take(7).collect::<String>();
    let name_at = |sha: &str| {
        let row = g.by_sha.get(sha).and_then(|i| g.rows.get(*i))?;
        let chips: Vec<_> = row.chips.iter().filter(|c| !c.tag).collect();
        chips.iter().find(|c| c.local).map(|c| c.name.to_string()).or_else(|| chips.iter().flat_map(|c| c.remotes.first()).next().map(|r| r.1.trim_start_matches("refs/remotes/").to_string()))
    };
    use gitvolt_graph::InProgress as P;
    match g.in_progress.get(worktree) {
        Some(P::Merge { merge_head, message, .. }) => (branch, Some(target.map(str::to_string).or_else(|| crate::commit_box::merged_name(message)).unwrap_or_else(|| short(merge_head)))),
        Some(P::Rebase { onto, head_name, stopped_at, .. }) => (Some(target.map(str::to_string).or_else(|| name_at(onto)).unwrap_or_else(|| short(onto))), if !head_name.is_empty() { Some(head_name.trim_start_matches("refs/heads/").to_string()) } else { stopped_at.as_deref().map(short) }),
        Some(P::CherryPick { head, .. }) => (branch, head.as_deref().map(short)),
        Some(P::Revert { head, .. }) => (branch, head.as_deref().map(|h| format!("revert of {}", short(h)))),
        _ => (None, None),
    }
}

fn colour_pieces(path: &str, text: String, lines: Arc<Lines>, tx: tokio::sync::mpsc::UnboundedSender<Vec<Vec<TmSpan>>>, stop: Arc<std::sync::atomic::AtomicU64>, generation: u64) {
    let Some(lang) = crate::diff::language::highlight_language(path, &text) else { return };
    let Some(mut tokenizer) = Tokenizer::new(lang) else { return };
    let raw: Vec<&str> = text.split('\n').collect();
    let mut at = 0;
    while at < raw.len() {
        if stop.load(std::sync::atomic::Ordering::Relaxed) != generation {
            return;
        }
        let end = (at + COLOUR_CHUNK).min(raw.len());
        let Some(spans) = tokenizer.lines(&raw[at..end]) else { return };
        if tx.send(lines.mapped_spans(at, spans)).is_err() {
            return;
        }
        at = end;
    }
}

impl GitVolt {
    fn merge_tab(&mut self, repo: u32) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == repo)
    }

    /// Runs `f` on the shown tool of the active tab.
    pub fn with_merge(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut MergeTool, &mut Context<Self>)) {
        if let Some(m) = self.tabs.get_mut(self.active).and_then(Tab::merge_shown_mut) {
            f(m, cx);
            cx.notify();
        }
    }

    /// The conflicted file just opened: its conflict read, the previous file's tool up meanwhile for a moment.
    pub fn read_conflict(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(diff) = tab.diff.as_ref() else { return };
        let Origin::Wip { worktree, .. } = &diff.file.origin else { return };
        let (repo, worktree, path) = (tab.id, worktree.clone(), diff.file.path.to_string());
        if tab.merge.as_ref().is_some_and(|m| m.path == path && m.worktree == worktree) {
            return;
        }
        self.read_conflict_again(repo, worktree, path, true, cx);
    }

    /// Reads `path`'s conflict and shows it; `wait`: what's on screen stays for `SWITCH_WAIT` first.
    pub fn read_conflict_again(&mut self, repo: u32, worktree: String, path: String, wait: bool, cx: &mut Context<Self>) {
        self.merge_generation += 1;
        let generation = self.merge_generation;
        let has_old = self.merge_tab(repo).is_some_and(|t| t.merge.is_some());
        if !wait || !has_old {
            if let Some(tab) = self.merge_tab(repo) {
                let mut loading = MergeTool::loading(worktree.clone(), path.clone());
                loading.generation = generation;
                tab.merge = Some(loading);
            }
        } else {
            let (w, p) = (worktree.clone(), path.clone());
            let timer = cx.background_executor().timer(SWITCH_WAIT);
            cx.spawn(async move |this, cx| {
                timer.await;
                this.update(cx, |this, cx| {
                    if this.merge_generation != generation {
                        return;
                    }
                    let Some(tab) = this.merge_tab(repo) else { return };
                    if tab.merge.as_ref().is_none_or(|m| m.path != p || m.worktree != w) {
                        let mut loading = MergeTool::loading(w.clone(), p.clone());
                        loading.generation = generation;
                        tab.merge = Some(loading);
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
        let api = self.api.clone();
        let (w, p) = (worktree.clone(), path.clone());
        let read = self.rt.spawn(async move { api.dispatch(Request::ConflictFile { repo, worktree: w, path: p }).await });
        cx.spawn(async move |this, cx| {
            let answer = read.await.unwrap_or_else(|e| Err(GbError::other(e.to_string())));
            this.update(cx, |this, cx| {
                if this.merge_generation != generation {
                    return;
                }
                this.adopt_conflict(repo, worktree, path, answer, cx);
            })
            .ok();
        })
        .detach();
    }

    /// The conflict's in: the tool shows it. Ticks that were kept for the same conflict stay.
    fn adopt_conflict(&mut self, repo: u32, worktree: String, path: String, answer: Result<Value, GbError>, cx: &mut Context<Self>) {
        let Some(tab) = self.merge_tab(repo) else { return };
        let same_file = tab.merge.as_ref().filter(|m| m.path == path && m.worktree == worktree);
        let kept = same_file.and_then(|m| Some((m.file()?.clone(), m.picks.clone(), m.dirty())));
        let generation = same_file.map_or(0, |m| m.generation);
        match answer {
            Err(e) => {
                let mut failed = MergeTool::loading(worktree, path);
                failed.loaded = Loaded::Failed(e.message);
                failed.generation = generation;
                tab.merge = Some(failed);
            }
            Ok(v) => {
                let Some(file) = ConflictFile::of(&v) else {
                    let mut gone = MergeTool::loading(worktree, path.clone());
                    gone.loaded = Loaded::Ready(None);
                    gone.generation = generation;
                    tab.merge = Some(gone);
                    if kept.is_some_and(|k| k.2) {
                        self.show_toast(format!("{path} isn't conflicted any more: your merge of it was dropped"), cx);
                    }
                    cx.notify();
                    return;
                };
                let (mut picks, mut dropped) = (empty_picks(&file.segments), false);
                if let Some((old, old_picks, dirty)) = kept {
                    if old.segments == file.segments {
                        picks = old_picks;
                    } else if dirty {
                        dropped = true;
                    }
                }
                let base = file.base.clone();
                let file = Rc::new(file);
                let tool = MergeTool::build(file.clone(), picks, base, generation, worktree, cx);
                let tab = self.merge_tab(repo).expect("the tab");
                tab.merge = Some(tool);
                tab.keys = KeyArea::Diff;
                if dropped {
                    self.show_toast(format!("The conflict in {path} changed: your merge of it was dropped"), cx);
                }
                self.colour_merge(repo, generation, cx);
            }
        }
        cx.notify();
    }

    /// Colours the panes and the output, from their tops.
    fn colour_merge(&mut self, repo: u32, generation: u64, cx: &mut Context<Self>) {
        for i in 0..3 {
            self.colour_merge_view(repo, generation, i, cx);
        }
    }

    fn colour_merge_view(&mut self, repo: u32, generation: u64, index: usize, cx: &mut Context<Self>) {
        let Some(tool) = self.merge_tab(repo).and_then(|t| t.merge.as_mut()) else { return };
        if index >= tool.editor_count() {
            return;
        }
        let path = tool.path.clone();
        let text = if index < 2 { tool.panes[index].data.text.clone() } else { tool.output.as_ref().map_or(String::new(), |o| o.edit.buffer.text.clone()) };
        let recolour = tool.output.as_ref().map_or(0, |o| o.recolour);
        let lines = tool.view(index).lines.clone();
        let stop = self.merge_colour_stop.clone();
        let token = self.merge_generation;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        self.rt.spawn_blocking(move || colour_pieces(&path, text, lines, tx, stop, token));
        self.merge_colour_stop.store(token, std::sync::atomic::Ordering::Relaxed);
        cx.spawn(async move |this, cx| {
            let mut at = 0;
            while let Some(piece) = rx.recv().await {
                let live = this.update(cx, |this, cx| {
                    let Some(tool) = this.merge_tab(repo).and_then(|t| t.merge.as_mut()).filter(|m| m.generation == generation) else { return false };
                    if index >= tool.editor_count() || (index == 2 && tool.output.as_ref().is_none_or(|o| o.recolour != recolour)) {
                        return false;
                    }
                    let list = &mut tool.view_mut(index).colours;
                    match list.get_mut(at) {
                        Some(slot) => *slot = Arc::new(piece),
                        None => list.push(Arc::new(piece)),
                    }
                    at += 1;
                    cx.notify();
                    true
                });
                if !matches!(live, Ok(true)) {
                    return;
                }
            }
        })
        .detach();
    }

    /// Typing done: the output's colours, once it pauses.
    pub fn merge_colour_after_edit(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let repo = tab.id;
        let Some(tool) = tab.merge_shown_mut() else { return };
        let generation = tool.generation;
        let Some(o) = tool.output.as_mut() else { return };
        o.recolour += 1;
        let at = o.recolour;
        let timer = cx.background_executor().timer(Duration::from_millis(120));
        cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |this, cx| {
                let current = this.merge_tab(repo).and_then(|t| t.merge.as_ref()).and_then(|m| m.output.as_ref()).map(|o| o.recolour);
                if current == Some(at) {
                    this.colour_merge_view(repo, generation, 2, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// `next` as the picks, its colours following.
    pub fn merge_apply(&mut self, next: Picks, cx: &mut Context<Self>) {
        self.with_merge(cx, |m, _| {
            m.apply(next);
        });
        self.merge_colour_after_edit(cx);
    }

    pub fn merge_toggle(&mut self, id: u32, side: Side, line: Option<usize>, cx: &mut Context<Self>) {
        self.with_merge(cx, |m, _| {
            m.toggle(id, side, line);
        });
        self.merge_colour_after_edit(cx);
    }

    pub fn merge_take_all(&mut self, side: Side, cx: &mut Context<Self>) {
        let Some(m) = self.tabs.get(self.active).and_then(Tab::merge_shown) else { return };
        let Some(file) = m.file().cloned() else { return };
        let on = side_state(&file.segments, &m.picks, side) != CheckState::All;
        let next = take_all(&m.picks, &file.segments, side, on);
        self.merge_apply(next, cx);
    }

    /// F7 or an arrow: the keyboard goes in the output too.
    pub fn merge_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let mut focus = None;
        self.with_merge(cx, |m, _| {
            if m.step(forward) {
                focus = m.output.as_ref().map(|o| o.edit.focus.clone());
            }
        });
        if focus.is_some() {
            self.focus_next_frame = focus;
        }
    }

    /// Ctrl+S or the button: saves and marks the file resolved. A region with nothing picked or typed arms it
    /// first, a stale save asks.
    pub fn merge_save(&mut self, confirmed: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let repo = tab.id;
        let Some(tool) = tab.merge_shown_mut() else { return };
        if tool.saving || tool.base.is_none() {
            return;
        }
        let (Some(file), Some(o)) = (tool.file().cloned(), tool.output.as_ref()) else { return };
        let n = unpicked_count(&file.segments, &tool.picks, &o.edited);
        if n > 0 && !confirmed {
            let (word, have) = if n == 1 { ("conflict", "1 conflict is") } else { ("conflicts", "") };
            let body = if n == 1 { format!("{have} still unresolved (nothing picked or typed): it will be saved empty.") } else { format!("{n} conflicts are still unresolved (nothing picked or typed): they will be saved empty.") };
            let mut req = ArmRequest::new(Tone::Warn, format!("Click again to save with {n} unresolved {word}"), "Mark resolved with unresolved conflicts?", "Mark resolved anyway");
            req.body = body.clone().into();
            req.caption = Some(body.into());
            let this = cx.weak_entity();
            self.with_window(cx, move |window, cx| {
                let origin = ArmOrigin::Control { key: "merge-save".into(), cover: None, grow_right: Some(false) };
                arm::request(
                    req,
                    origin,
                    move |a, _, cx| {
                        this.update(cx, |v, cx| {
                            if a.ok {
                                v.merge_save(true, cx);
                            } else {
                                v.merge_after_save = None;
                            }
                        })
                        .ok();
                    },
                    window,
                    cx,
                );
            });
            return;
        }
        let built = build_output(&file.segments, &tool.picks, eol_text(&file.eol)).0;
        let text = o.text_to_save(&built);
        let base = tool.base.clone();
        let (worktree, path) = (tool.worktree.clone(), tool.path.clone());
        tool.saving = true;
        cx.notify();
        self.send_merge_save(repo, worktree, path, text, base, cx);
    }

    fn send_merge_save(&mut self, repo: u32, worktree: String, path: String, text: String, base: Option<String>, cx: &mut Context<Self>) {
        let ctx = WriteCtx { repo, worktree: worktree.clone() };
        let (send_path, send_worktree) = (path.clone(), worktree.clone());
        let (done_path, stale_path, stale_worktree, stale_text) = (path.clone(), path, worktree, text.clone());
        let w = Write::new(&ctx, move |asked| call("resolveFile", json!({ "repo": repo, "worktree": send_worktree, "path": send_path, "resolution": { "kind": "text", "text": text }, "base": base, "confirmMarkers": asked.markers, "confirmDiscard": asked.discard })))
            .on_success(move |this, out, cx| {
                this.toast_submodule_behind(out, cx);
                this.merge_left_resolved(repo, Some(done_path.clone()), cx);
                this.reload(repo, cx);
                if let Some((_, go)) = this.merge_after_save.take() {
                    go(this, cx);
                }
            })
            .handle(move |this, err, cx| {
                if err.kind != GbErrorKind::Stale {
                    return false;
                }
                this.merge_ask_stale(repo, stale_worktree.clone(), stale_path.clone(), stale_text.clone(), err.message.clone(), cx);
                true
            })
            .on_end(move |this, ok, cx| {
                if !ok {
                    this.merge_after_save = None;
                }
                if let Some(m) = this.merge_tab(repo).and_then(|t| t.merge.as_mut()) {
                    m.saving = false;
                }
                cx.notify();
            });
        self.write(w, cx);
    }

    /// The file changed on disk since the work began: Reload rebuilds from it (the ticks stay while the conflict
    /// is the same, hand edits go); Overwrite re-reads only the base and sends the same text.
    fn merge_ask_stale(&mut self, repo: u32, worktree: String, path: String, text: String, title: String, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        let arm_label = format!("Click again to overwrite {path} with your merge");
        let req = ChoiceRequest {
            title: title.into(),
            body: Some("Reload it from disk (your ticks stay while the conflict is the same; hand edits are lost), or overwrite it with your merge.".into()),
            note: None,
            choices: vec![Choice { id: "reload".into(), label: "Reload from disk".into(), primary: true, ..Default::default() }, Choice { id: "overwrite".into(), label: "Overwrite".into(), danger: true, arm: Some(arm_label.into()), ..Default::default() }],
        };
        self.with_window(cx, move |window, cx| {
            popover::ask_choice(
                req,
                None,
                move |pick, _, cx| {
                    let (w, p, t) = (worktree.clone(), path.clone(), text.clone());
                    this.update(cx, |v, cx| match pick.as_deref() {
                        Some("reload") => v.read_conflict_again(repo, w, p, false, cx),
                        Some("overwrite") => v.merge_overwrite(repo, w, p, t, cx),
                        _ => {}
                    })
                    .ok();
                },
                window,
                cx,
            );
        });
    }

    fn merge_overwrite(&mut self, repo: u32, worktree: String, path: String, text: String, cx: &mut Context<Self>) {
        if cfg!(test) {
            self.sent.borrow_mut().push(call("conflictFile", json!({ "path": path })));
            return;
        }
        let api = self.api.clone();
        let (w, p) = (worktree.clone(), path.clone());
        let read = self.rt.spawn(async move { api.dispatch(Request::ConflictFile { repo, worktree: w, path: p }).await });
        cx.spawn(async move |this, cx| {
            let now = read.await.unwrap_or_else(|e| Err(GbError::other(e.to_string())));
            this.update(cx, |this, cx| {
                let now = match now {
                    Ok(v) => ConflictFile::of(&v),
                    Err(e) => return this.show_toast(format!("Couldn't re-read {path}: {}", e.message), cx),
                };
                let Some(now) = now else {
                    this.show_toast(format!("{path} isn't conflicted any more"), cx);
                    return this.read_conflict_again(repo, worktree, path, false, cx);
                };
                let Some(base) = now.base.clone() else {
                    this.show_toast(format!("{path} was deleted on disk"), cx);
                    if let Some(m) = this.merge_tab(repo).and_then(|t| t.merge.as_mut()) {
                        m.base = None;
                    }
                    return cx.notify();
                };
                if let Some(m) = this.merge_tab(repo).and_then(|t| t.merge.as_mut()) {
                    m.base = Some(base.clone());
                    m.saving = true;
                }
                this.send_merge_save(repo, worktree, path, text, Some(base), cx);
            })
            .ok();
        })
        .detach();
    }

    /// A non-text conflict's button, or a row's: one resolution, with `conflictFile`'s base when there's one.
    pub fn resolve_file(&mut self, ctx: &WriteCtx, path: String, kind: &'static str, base: Option<String>, origin: ArmOrigin, cx: &mut Context<Self>) {
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let (done_path, stale_path) = (path.clone(), path.clone());
        let w = Write::new(ctx, move |asked| call("resolveFile", json!({ "repo": repo, "worktree": worktree, "path": path, "resolution": { "kind": kind }, "base": base, "confirmMarkers": asked.markers, "confirmDiscard": asked.discard })))
            .origin(origin)
            .on_success(move |this, out, cx| {
                this.toast_submodule_behind(out, cx);
                this.merge_left_resolved(repo, Some(done_path.clone()), cx);
                this.reload(repo, cx);
            })
            .handle(move |this, err, cx| {
                if err.kind != GbErrorKind::Stale {
                    return false;
                }
                this.show_toast(err.message.clone(), cx);
                let open = this.merge_tab(repo).and_then(|t| t.merge.as_ref()).filter(|m| m.path == stale_path).map(|m| m.worktree.clone());
                if let Some(worktree) = open {
                    this.read_conflict_again(repo, worktree, stale_path.clone(), false, cx);
                }
                true
            })
            .on_end(move |this, _, cx| {
                if let Some(m) = this.merge_tab(repo).and_then(|t| t.merge.as_mut()) {
                    m.busy = false;
                }
                cx.notify();
            });
        if let Some(m) = self.merge_tab(repo).and_then(|t| t.merge.as_mut()) {
            m.busy = true;
        }
        self.write(w, cx);
    }

    fn toast_submodule_behind(&mut self, out: &Value, cx: &mut Context<Self>) {
        if let (Some(path), Some(head), Some(taken)) = (out["path"].as_str(), out["head"].as_str(), out["taken"].as_str()) {
            let short = |s: &str| s.chars().take(7).collect::<String>();
            toast::show_with(format!("Submodule {path} is still checked out at {}: update it to {} before committing", short(head), short(taken)), toast::Options { error: true, ..Default::default() }, cx);
        }
    }

    /// The open file was resolved: here, from a row's menu, a non-text prompt's buttons or a save. The tool moves
    /// to the next conflicted file of the worktree, or closes when none is left. `gone`: the write's answer
    /// said so; without it, the lists held say whether the file's still conflicted.
    pub fn merge_left_resolved(&mut self, repo: u32, gone: Option<String>, cx: &mut Context<Self>) {
        let Some(tab) = self.merge_tab(repo) else { return };
        let Some(diff) = tab.diff.as_ref().filter(|d| is_conflict(&d.file)) else { return };
        let open_path = diff.file.path.to_string();
        let Some(files) = tab.files.as_ref().filter(|f| f.wip.is_some()) else { return };
        let conflicted = files.section_files(Which::Conflicted);
        if gone.as_deref() != Some(open_path.as_str()) && conflicted.iter().any(|f| f.path.as_ref() == open_path) {
            return;
        }
        let rest: Vec<&FileChange> = conflicted.iter().filter(|f| f.path.as_ref() != open_path).collect();
        let next = rest.iter().find(|f| f.path.as_ref() > open_path.as_str()).or(rest.first()).map(|f| (*f).clone());
        tab.merge = None;
        match next {
            Some(file) => {
                tab.diff = Some(crate::diff::view::DiffState::carrying(file.clone(), tab.diff.as_ref()));
                let at = tab.files.as_ref().and_then(|f| f.changes.iter().position(|c| c.path == file.path && c.origin == file.origin));
                if let (Some(i), Some(files)) = (at, tab.files.as_mut()) {
                    files.cursor = Some(crate::details::Cursor::File(i));
                }
                self.read_contents(false, cx);
            }
            None => {
                tab.diff = None;
                if tab.keys == KeyArea::Diff {
                    tab.keys = KeyArea::Files;
                }
                if let Some(files) = tab.files.as_mut() {
                    files.cursor = None;
                }
            }
        }
        cx.notify();
    }

    /// The Copy of a pane, or of the output.
    pub fn merge_selected_text(&self) -> Option<String> {
        let m = self.tabs.get(self.active)?.merge_shown()?;
        for p in &m.panes {
            if let Some(t) = p.selected_text() {
                return Some(t);
            }
        }
        let o = m.output.as_ref()?;
        (!o.edit.buffer.sel.is_empty()).then(|| o.edit.buffer.selected().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn file() -> ConflictFile {
        let conflict = |id: u32, a: &str, b: &str| Segment::Conflict(ConflictSegment { id, current: vec![a.to_string()], incoming: vec![b.to_string()] });
        ConflictFile {
            path: "a.rs".into(),
            kind: "bothModified".into(),
            text: true,
            segments: vec![Segment::Common("top\n".into()), conflict(1, "one\n", "uno\n"), Segment::Common("mid\n".into()), conflict(2, "two\n", "dos\n"), Segment::Common("end\n".into())],
            current: Some(PaneData { text: "top\none\nmid\ntwo\nend\n".into(), regions: vec![PaneRegion { id: 1, start: 2, lines: 1 }, PaneRegion { id: 2, start: 4, lines: 1 }] }),
            incoming: Some(PaneData { text: "top\nuno\nmid\ndos\nend\n".into(), regions: vec![PaneRegion { id: 1, start: 2, lines: 1 }, PaneRegion { id: 2, start: 4, lines: 1 }] }),
            label_current: "main".into(),
            label_incoming: "topic".into(),
            eol: "lf".into(),
            base: Some("h".into()),
        }
    }

    fn tool(cx: &mut App) -> MergeTool {
        let f = file();
        let picks = empty_picks(&f.segments);
        let mut t = MergeTool::build(Rc::new(f), picks, Some("h".into()), 1, "/w".into(), cx);
        t.set_size(1158., 733.);
        t
    }

    fn text(t: &MergeTool) -> &str {
        &t.output.as_ref().unwrap().edit.buffer.text
    }

    #[gpui::test]
    fn it_opens_at_the_first_region_to_resolve_with_nothing_kept(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let t = tool(cx);
            assert_eq!(text(&t), "top\nmid\nend\n");
            assert!(!t.dirty());
            assert_eq!(t.count_text(), "Conflict 1 of 2", "the caret's on the first region");
            assert_eq!(t.view(2).height, 337., "the sizes the web UI's layout gives");
            assert_eq!(t.view(0).height, 337.);
            assert_eq!(t.view(0).width, 579.);
        });
    }

    #[gpui::test]
    fn a_tick_rebuilds_only_its_region_and_hand_edits_elsewhere_stay(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut t = tool(cx);
            // A hand edit in the common part.
            let o = t.output.as_mut().unwrap();
            o.edit.buffer.move_to(0);
            o.edit.buffer.replace(None, "x", crate::ui::input::EditKind::Typing);
            t.after_output_edit();
            assert_eq!(text(&t), "xtop\nmid\nend\n");
            assert!(t.output.as_ref().unwrap().edited.is_empty(), "typed outside the regions");
            assert!(t.output.as_ref().unwrap().typed);
            t.toggle(2, Side::Incoming, None);
            assert_eq!(text(&t), "xtop\nmid\ndos\nend\n");
            t.toggle(1, Side::Current, Some(0));
            assert_eq!(text(&t), "xtop\none\nmid\ndos\nend\n");
            assert_eq!(t.output.as_ref().unwrap().spans, [Span { id: 1, from: 5, to: 9 }, Span { id: 2, from: 13, to: 17 }]);
        });
    }

    #[gpui::test]
    fn undo_takes_a_tick_back_with_its_text_and_redo_returns_it(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut t = tool(cx);
            t.toggle(1, Side::Current, None);
            assert!(t.dirty());
            t.output.as_mut().unwrap().edit.buffer.undo();
            t.after_output_edit();
            assert_eq!(text(&t), "top\nmid\nend\n");
            assert!(!any_picked(&t.picks), "the tick went with it");
            assert!(!t.dirty());
            t.output.as_mut().unwrap().edit.buffer.redo();
            t.after_output_edit();
            assert_eq!(text(&t), "top\none\nmid\nend\n");
            assert!(any_picked(&t.picks));
        });
    }

    #[gpui::test]
    fn typing_in_a_region_marks_it_edited_until_a_tick_rebuilds_it(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut t = tool(cx);
            t.toggle(1, Side::Current, None);
            let o = t.output.as_mut().unwrap();
            let at = o.spans[0].from + 1;
            o.edit.buffer.move_to(at);
            o.edit.buffer.replace(None, "Z", crate::ui::input::EditKind::Typing);
            t.after_output_edit();
            assert_eq!(text(&t), "top\noZne\nmid\nend\n");
            assert!(t.output.as_ref().unwrap().edited.contains(&1));
            t.toggle(1, Side::Incoming, None);
            assert_eq!(text(&t), "top\none\nuno\nmid\nend\n", "its text is the ticks' again");
            assert!(!t.output.as_ref().unwrap().edited.contains(&1));
        });
    }

    #[gpui::test]
    fn typing_into_an_empty_region_fills_it(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut t = tool(cx);
            let o = t.output.as_mut().unwrap();
            let at = o.spans[0].from;
            o.edit.buffer.move_to(at);
            o.edit.buffer.replace(None, "new\n", crate::ui::input::EditKind::Other);
            t.after_output_edit();
            let o = t.output.as_ref().unwrap();
            assert_eq!(o.spans[0], Span { id: 1, from: 4, to: 8 });
            assert_eq!(o.spans[1], Span { id: 2, from: 12, to: 12 }, "the next one moved on");
            assert!(o.edited.contains(&1));
        });
    }

    #[gpui::test]
    fn take_all_and_the_steps_go_through_the_regions(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut t = tool(cx);
            let f = t.file().unwrap().clone();
            let next = take_all(&t.picks, &f.segments, Side::Incoming, true);
            t.apply(next);
            assert_eq!(text(&t), "top\nuno\nmid\ndos\nend\n");
            assert_eq!(unpicked_count(&f.segments, &t.picks, &t.output.as_ref().unwrap().edited), 0);
            assert!(t.step(true));
            assert_eq!(t.count_text(), "Conflict 2 of 2");
            assert!(t.step(true));
            assert_eq!(t.count_text(), "Conflict 1 of 2");
            assert!(t.step(false));
            assert_eq!(t.count_text(), "Conflict 2 of 2");
        });
    }

    #[gpui::test]
    fn a_crlf_file_nobody_typed_in_goes_out_as_built(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let f = ConflictFile { eol: "crlf".into(), segments: vec![Segment::Common("top\r\n".into()), Segment::Conflict(ConflictSegment { id: 1, current: vec!["one\r\n".into()], incoming: vec!["uno\r\n".into()] })], current: Some(PaneData { text: "top\r\none\r\n".into(), regions: vec![PaneRegion { id: 1, start: 2, lines: 1 }] }), incoming: Some(PaneData { text: "top\r\nuno\r\n".into(), regions: vec![PaneRegion { id: 1, start: 2, lines: 1 }] }), ..file() };
            let segs = f.segments.clone();
            let picks = empty_picks(&segs);
            let mut t = MergeTool::build(Rc::new(f), picks, Some("h".into()), 1, "/w".into(), cx);
            t.toggle(1, Side::Current, None);
            let built = build_output(&segs, &t.picks, "\r\n").0;
            assert_eq!(built, "top\r\none\r\n");
            assert_eq!(t.output.as_ref().unwrap().text_to_save(&built), built);
            let o = t.output.as_mut().unwrap();
            o.edit.buffer.move_to(0);
            o.edit.buffer.replace(None, "x", crate::ui::input::EditKind::Typing);
            assert_eq!(t.output.as_ref().unwrap().text_to_save(&built), "xtop\r\none\r\n");
        });
    }

    #[tokio::test]
    async fn the_core_resolves_a_real_conflict_from_a_saved_merge_and_refuses_a_stale_one() {
        use crate::write::testing::Fixture;
        let f = Fixture::new().await;
        f.git(&["checkout", "-q", "-b", "topic"]);
        f.write("a.txt", "topic\n");
        f.git(&["commit", "-qam", "Topic"]);
        f.git(&["checkout", "-q", "main"]);
        f.write("a.txt", "main\n");
        f.git(&["commit", "-qam", "Main"]);
        let out = std::process::Command::new("git").args(["merge", "--no-edit", "topic"]).current_dir(&f.path).env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap();
        assert!(!out.status.success());
        let body = call("conflictFile", json!({ "repo": f.repo, "worktree": f.worktree, "path": "a.txt" }));
        let v = f.ok(body).await;
        let file = ConflictFile::of(&v).unwrap();
        assert!(file.textual());
        assert_eq!(file.pane(Side::Incoming).unwrap().text, "topic\n");
        let picks = take_all(&empty_picks(&file.segments), &file.segments, Side::Incoming, true);
        let (text, _) = build_output(&file.segments, &picks, eol_text(&file.eol));
        assert_eq!(text, "topic\n");
        let save = |text: &str, base: Option<&str>| call("resolveFile", json!({ "repo": f.repo, "worktree": f.worktree, "path": "a.txt", "resolution": { "kind": "text", "text": text }, "base": base, "confirmMarkers": false, "confirmDiscard": false }));
        let stale = f.send(save(&text, Some("not-the-hash"))).await.unwrap_err();
        assert_eq!(stale.kind, GbErrorKind::Stale);
        f.ok(save(&text, file.base.as_deref())).await;
        assert_eq!(f.read("a.txt"), "topic\n");
        assert_eq!(f.git(&["status", "--porcelain"]), "M  a.txt");
    }
}
