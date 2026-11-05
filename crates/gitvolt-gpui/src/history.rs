//! File History and Blame (`ui/src/history`): the file's commits on the left, the file at the picked
//! commit or that commit's changes on the right, in the graph's place. The right side is an ordinary
//! `DiffState` in `Tab::diff`, so all of the diff's code applies; the file that was open waits in
//! `FileHistory::saved` for the panel to close.

use crate::app::{GitVolt, KeyArea, Tab, icon};
use crate::details::{FileChange, Origin, avatar, file_changes, segment, segmented};
use crate::diff::view::{DiffState, Notice, ToolbarKind, ViewKind, diff_body, toolbar};
use crate::format::{compact_relative_time, relative_time};
use crate::model::SHORT_SHA;
use crate::theme;
use gitvolt_core::api::Request;
use gitvolt_core::diff::DiffSpec;
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

pub const HISTORY_H: f32 = 31.;
pub const LIST_W: f32 = 360.;
const LIST_MIN: f32 = 220.;
const LIST_MAX: f32 = 640.;
const ROW_H: f32 = 41.6;
const PAGE: u32 = 200;
/// How many more pages a blame click looks through for its commit.
const SEEK_PAGES: u32 = 10;
/// The blame strip: 200 px, at most this share of the editor's width, so a narrow one keeps its code.
const BLAME_W: f32 = 200.;
const BLAME_SHARE: f32 = 0.35;
const BLAME_CACHE: usize = 32;
const AVATAR_PX: f32 = 16.;

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryRow {
    pub sha: SharedString,
    pub parents: Vec<String>,
    pub author: SharedString,
    pub email: String,
    pub time: i64,
    pub summary: SharedString,
    pub path: String,
    /// `A`, `M`, `D`, `R`, `C` or `T`; empty for a merge that shows no change of its own.
    pub status: String,
    pub old_path: Option<String>,
}

fn rows_of(v: &Value) -> (Vec<HistoryRow>, bool) {
    let rows = v["rows"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|r| HistoryRow {
            sha: r["sha"].as_str().unwrap_or_default().to_string().into(),
            parents: r["parents"].as_array().map_or(&[][..], Vec::as_slice).iter().filter_map(|p| p.as_str().map(str::to_string)).collect(),
            author: r["author"].as_str().unwrap_or_default().to_string().into(),
            email: r["email"].as_str().unwrap_or_default().to_string(),
            time: r["time"].as_i64().unwrap_or(0),
            summary: r["summary"].as_str().unwrap_or_default().to_string().into(),
            path: r["path"].as_str().unwrap_or_default().to_string(),
            status: r["status"].as_str().unwrap_or_default().to_string(),
            old_path: r["oldPath"].as_str().map(str::to_string),
        })
        .collect();
    (rows, v["more"].as_bool() == Some(true))
}

const ABSENT: fn() -> Value = || json!({ "kind": "absent" });

fn at_commit(commit: &str) -> Value {
    json!({ "kind": "atCommit", "commit": commit })
}

/// The file at `row`'s commit, as File View opens it.
pub fn file_target(row: &HistoryRow) -> FileChange {
    FileChange { path: row.path.clone().into(), old_path: None, status: ' ', additions: None, deletions: None, old: ABSENT(), new: at_commit(&row.sha), conflict: None, origin: Origin::Commit { id: row.sha.to_string() } }
}

fn renamed(row: &HistoryRow) -> bool {
    row.old_path.as_ref().is_some_and(|p| *p != row.path)
}

/// Whether the row's sides come from its commit's file list: a rename or copy, whose old side is at
/// another path, and a merge, whose first parent may not have the file at all.
pub fn needs_file_list(row: &HistoryRow) -> bool {
    renamed(row) || row.status.is_empty() || row.parents.len() > 1
}

/// The commit's change to the file, against its first parent. `files`: the commit's list, for a row
/// that `needs_file_list`; without an entry the sides are the commit and its parent at the row's path.
pub fn changes_target(row: &HistoryRow, files: Option<&[FileChange]>) -> FileChange {
    if needs_file_list(row)
        && let Some(entry) = files.and_then(|f| f.iter().find(|f| f.path.as_ref() == row.path))
    {
        return FileChange { origin: Origin::Commit { id: row.sha.to_string() }, ..entry.clone() };
    }
    let parent = row.parents.first();
    let old = match parent {
        Some(p) if row.status != "A" && !renamed(row) => at_commit(p),
        _ => ABSENT(),
    };
    let new = if row.status == "D" { ABSENT() } else { at_commit(&row.sha) };
    FileChange { path: row.path.clone().into(), old_path: None, status: row.status.chars().next().unwrap_or('M'), additions: None, deletions: None, old, new, conflict: None, origin: Origin::Commit { id: row.sha.to_string() } }
}

/// Where a file's history starts: its commit's version (a deleted file: the parent's), or a WIP
/// file's worktree HEAD. `None`: a file new in the WIP, or no commit to start from.
pub fn history_start(f: &FileChange, parents: &[SharedString], tab_worktree: &str) -> Option<HistoryStart> {
    let deleted = f.new["kind"].as_str() == Some("absent");
    match &f.origin {
        Origin::Wip { worktree, .. } => (f.status != 'A').then(|| HistoryStart { path: f.old_path.clone().unwrap_or_else(|| f.path.to_string()), rev: None, worktree: worktree.clone() }),
        Origin::Commit { id } => {
            let rev = if deleted { parents.first()?.to_string() } else { id.clone() };
            Some(HistoryStart { path: if deleted { f.old_path.clone().unwrap_or_else(|| f.path.to_string()) } else { f.path.to_string() }, rev: Some(rev), worktree: tab_worktree.to_string() })
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryStart {
    pub path: String,
    pub rev: Option<String>,
    pub worktree: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryView {
    File,
    Changes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameGroup {
    pub sha: SharedString,
    /// The first line, 1-based.
    pub start: u32,
    pub lines: u32,
}

#[derive(Debug, Clone)]
pub struct BlameCommit {
    pub author: SharedString,
    pub email: String,
    pub time: i64,
    pub summary: SharedString,
}

pub struct BlameData {
    pub groups: Vec<BlameGroup>,
    pub commits: HashMap<SharedString, BlameCommit>,
}

/// git's hunks, merged where one commit's lines run on, in line order.
pub fn group_blame(mut hunks: Vec<(String, u32, u32)>) -> Vec<BlameGroup> {
    hunks.sort_by_key(|h| h.1);
    let mut out: Vec<BlameGroup> = Vec::new();
    for (sha, start, lines) in hunks {
        match out.last_mut() {
            Some(last) if last.sha.as_ref() == sha && last.start + last.lines == start => last.lines += lines,
            _ => out.push(BlameGroup { sha: sha.into(), start, lines }),
        }
    }
    out
}

/// The groups that overlap lines `first..=last` (1-based), by binary search.
pub fn visible_groups(groups: &[BlameGroup], first: u32, last: u32) -> Range<usize> {
    if last < first {
        return 0..0;
    }
    let from = groups.partition_point(|g| g.start + g.lines - 1 < first);
    let to = groups.partition_point(|g| g.start <= last);
    from..to.max(from)
}

/// A stable lane-palette index per commit (FNV-1a over the sha).
pub fn color_index(sha: &str, n: usize) -> usize {
    let mut h: u32 = 2166136261;
    for c in sha.encode_utf16() {
        h ^= u32::from(c);
        h = h.wrapping_mul(16777619);
    }
    h as usize % n
}

fn blame_of(v: &Value) -> BlameData {
    let list = |k: &str| v[k].as_array().map_or(Vec::new(), Clone::clone);
    let hunks = list("hunks").iter().map(|h| (h["sha"].as_str().unwrap_or_default().to_string(), h["start"].as_u64().unwrap_or(0) as u32, h["lines"].as_u64().unwrap_or(0) as u32)).collect();
    let commits = list("commits")
        .iter()
        .map(|c| {
            let text = |k: &str| SharedString::from(c[k].as_str().unwrap_or_default().to_string());
            (text("sha"), BlameCommit { author: text("author"), email: c["email"].as_str().unwrap_or_default().to_string(), time: c["time"].as_i64().unwrap_or(0), summary: text("summary") })
        })
        .collect();
    BlameData { groups: group_blame(hunks), commits }
}

pub enum BlameState {
    Idle,
    Loading,
    Ready(Rc<BlameData>),
    Failed(SharedString),
}

/// One open File History.
pub struct FileHistory {
    pub path: SharedString,
    pub rev: Option<String>,
    pub worktree: String,
    pub rows: Rc<Vec<HistoryRow>>,
    pub more: bool,
    pub loading: bool,
    pub error: Option<SharedString>,
    pub selected: Option<SharedString>,
    pub blame_on: bool,
    pub view: HistoryView,
    pub blame: BlameState,
    blame_cache: Vec<((String, String), Rc<BlameData>)>,
    pub list: ListState,
    pub list_w: f32,
    /// A resize under way: where the pointer pressed, and the width then.
    pub resizing: Option<(f32, f32)>,
    /// The file that was open, for when this closes.
    pub saved: Option<DiffState>,
    /// A blame click looking for its commit, and the pages it may still read.
    seek: Option<(String, u32)>,
    /// A newer open or row replaces what an older read was going to show.
    pub generation: u64,
}

impl FileHistory {
    pub fn new(start: HistoryStart, blame_on: bool, view: HistoryView, saved: Option<DiffState>) -> Self {
        Self {
            path: start.path.into(),
            rev: start.rev,
            worktree: start.worktree,
            rows: Rc::new(Vec::new()),
            more: true,
            loading: false,
            error: None,
            selected: None,
            blame_on,
            view,
            blame: BlameState::Idle,
            blame_cache: Vec::new(),
            list: ListState::new(1, ListAlignment::Top, px(200.)),
            list_w: LIST_W,
            resizing: None,
            saved,
            seek: None,
            generation: 0,
        }
    }

    /// A page in: appended, a row already there skipped, and the first selected if nothing is.
    pub fn with_page(&mut self, rows: Vec<HistoryRow>, more: bool) {
        let mut all = self.rows.as_ref().clone();
        for r in rows {
            if !all.iter().any(|x| x.sha == r.sha) {
                all.push(r);
            }
        }
        self.selected = self.selected.take().or_else(|| all.first().map(|r| r.sha.clone()));
        self.rows = Rc::new(all);
        self.more = more;
        self.loading = false;
        self.error = None;
        self.list.reset(self.rows.len() + 1);
    }

    /// ↑ / ↓ in the list. Whether the selection moved.
    pub fn step(&mut self, down: bool) -> bool {
        if self.rows.is_empty() {
            return false;
        }
        let i = self.rows.iter().position(|r| Some(&r.sha) == self.selected.as_ref());
        let next = match i {
            None => 0,
            Some(i) if down => (i + 1).min(self.rows.len() - 1),
            Some(i) => i.saturating_sub(1),
        };
        let sha = self.rows[next].sha.clone();
        let moved = self.selected.as_ref() != Some(&sha);
        self.selected = Some(sha);
        self.list.scroll_to_reveal_item(next);
        moved
    }

    pub fn selected_row(&self) -> Option<&HistoryRow> {
        self.rows.iter().find(|r| Some(&r.sha) == self.selected.as_ref())
    }

    /// The list's end, once the last page is in: `Some(added in)`.
    pub fn end(&self) -> Option<Option<SharedString>> {
        if self.more || self.loading || self.error.is_some() {
            return None;
        }
        Some(self.rows.last().filter(|r| r.status == "A").map(|r| r.sha.clone()))
    }

    /// Blame is the file's: off, in place, for a binary and in Changes.
    pub fn no_blame(&self, diff: Option<&DiffState>) -> bool {
        self.view == HistoryView::Changes || diff.is_some_and(DiffState::shows_binary)
    }

    /// The strip's width in an editor `width` px wide, while it's reserved: it stays while it loads.
    pub fn margin(&self, diff: Option<&DiffState>, width: f32) -> f32 {
        let text = diff.is_some_and(|d| d.data.is_some() || d.notice == Some(Notice::Loading));
        if self.blame_on && !self.no_blame(diff) && text { BLAME_W.min(width * BLAME_SHARE) } else { 0. }
    }
}

impl GitVolt {
    /// The toolbar's Blame and History, Ctrl+Shift+B and Ctrl+Shift+H: from Diff View it opens on each
    /// version's Changes, from File View on the File.
    pub fn open_history(&mut self, blame: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        if tab.history.is_some() {
            return;
        }
        let Some(diff) = tab.diff.as_ref() else { return };
        let parents = tab.commit.as_ref().map_or(&[][..], |c| c.parents.as_slice());
        let Some(start) = history_start(&diff.file, parents, &tab.worktree) else { return };
        if blame && diff.shows_binary() {
            return;
        }
        let view = if diff.view == ViewKind::Diff { HistoryView::Changes } else { HistoryView::File };
        let saved = tab.diff.take();
        tab.history = Some(FileHistory::new(start, blame, view, saved));
        tab.keys = KeyArea::History;
        tab.sidebar_open = false;
        self.load_history(cx);
        cx.notify();
    }

    /// A file picked in the right panel while File History is open opens in it too.
    pub fn follow_file(&mut self, file: FileChange, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active) else { return false };
        let Some(open) = tab.history.as_ref() else { return false };
        let parents = tab.commit.as_ref().map_or(&[][..], |c| c.parents.as_slice());
        let Some(start) = history_start(&file, parents, &tab.worktree) else { return false };
        let same = open.path.as_ref() == start.path && open.rev == start.rev && open.worktree == start.worktree;
        if !same {
            let saved = DiffState::carrying(file, tab.diff.as_ref().or(open.saved.as_ref()));
            let mut next = FileHistory::new(start, open.blame_on, open.view, Some(saved));
            next.list_w = open.list_w;
            next.generation = open.generation + 1;
            tab.history = Some(next);
            self.load_history(cx);
        }
        cx.notify();
        true
    }

    pub fn close_history(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let Some(mut history) = tab.history.take() else { return };
        let inner = tab.diff.take();
        let mut saved = history.saved.take();
        if let (Some(s), Some(i)) = (saved.as_mut(), inner.as_ref()) {
            // What the view's prefs are holds across it, as in the web UI, where they're global.
            s.take_prefs(i);
            // It may have been colouring, or still reading, when it was put aside.
            s.generation = i.generation.max(s.generation) + 1;
            s.keep_scroll();
        }
        tab.diff = saved;
        tab.keys = KeyArea::Files;
        if tab.diff.is_some() {
            self.read_contents(false, cx);
        }
        cx.notify();
    }

    /// The next page; none while one is on its way.
    pub fn load_history(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let repo = tab.id;
        let Some(h) = tab.history.as_mut() else { return };
        if h.loading || !h.more || cfg!(test) {
            return;
        }
        h.loading = true;
        h.error = None;
        h.generation += 1;
        let generation = h.generation;
        let (worktree, path, rev, skip) = (h.worktree.clone(), h.path.to_string(), h.rev.clone(), h.rows.len() as u32);
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(Request::FileHistory { repo: crate::app::core_repo(repo), worktree, path, rev, skip, limit: PAGE }).await });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) else { return };
                let Some(h) = tab.history.as_mut().filter(|h| h.generation == generation) else { return };
                let had = h.selected.clone();
                match result {
                    Ok(Ok(v)) => {
                        let (rows, more) = rows_of(&v);
                        h.with_page(rows, more);
                    }
                    Ok(Err(e)) => {
                        h.loading = false;
                        h.error = Some(e.message.into());
                    }
                    Err(e) => {
                        h.loading = false;
                        h.error = Some(e.to_string().into());
                    }
                }
                if h.selected != had {
                    this.show_history_row(cx);
                }
                this.continue_seek(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Selects a row of the list.
    pub fn select_history_row(&mut self, sha: SharedString, cx: &mut Context<Self>) {
        let Some(h) = self.tabs.get_mut(self.active).and_then(|t| t.history.as_mut()) else { return };
        if h.selected.as_ref() == Some(&sha) {
            return;
        }
        if let Some(i) = h.rows.iter().position(|r| r.sha == sha) {
            h.selected = Some(sha);
            h.list.scroll_to_reveal_item(i);
            self.show_history_row(cx);
        }
    }

    pub fn step_history(&mut self, down: bool, cx: &mut Context<Self>) {
        let Some(h) = self.tabs.get_mut(self.active).and_then(|t| t.history.as_mut()) else { return };
        if h.step(down) {
            self.show_history_row(cx);
        }
        cx.notify();
    }

    pub fn set_history_view(&mut self, view: HistoryView, cx: &mut Context<Self>) {
        let Some(h) = self.tabs.get_mut(self.active).and_then(|t| t.history.as_mut()) else { return };
        if h.view != view {
            h.view = view;
            self.show_history_row(cx);
            cx.notify();
        }
    }

    pub fn toggle_blame(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let diff = tab.diff.as_ref();
        let Some(h) = tab.history.as_mut() else { return };
        if h.no_blame(diff) {
            return;
        }
        h.blame_on = !h.blame_on;
        self.read_blame(cx);
        cx.notify();
    }

    /// Shows the selected row on the right: its file, or its commit's change to it.
    pub fn show_history_row(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let repo = tab.id;
        let Some(h) = tab.history.as_mut() else { return };
        let Some(row) = h.selected_row().cloned() else { return };
        h.generation += 1;
        let generation = h.generation;
        let view = h.view;
        if view == HistoryView::Changes && needs_file_list(&row) {
            let api = self.api.clone();
            let id = row.sha.to_string();
            let read = self.rt.spawn(async move { api.dispatch(Request::FileList { repo: crate::app::core_repo(repo), spec: DiffSpec::Commit { id, parent: 0 } }).await });
            cx.spawn(async move |this, cx| {
                let files = read.await.ok().and_then(Result::ok).map(|v| file_changes(&v, &Origin::Commit { id: row.sha.to_string() }));
                this.update(cx, |this, cx| {
                    let live = this.tabs.iter().find(|t| t.id == repo).and_then(|t| t.history.as_ref()).is_some_and(|h| h.generation == generation);
                    if live {
                        this.open_inner(changes_target(&row, files.as_deref()), ViewKind::Diff, false);
                        this.read_contents(false, cx);
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
            return;
        }
        match view {
            HistoryView::File if row.status == "D" => {
                self.open_inner(file_target(&row), ViewKind::File, true);
                self.with_diff(cx, |d| d.notice = Some(Notice::Deleted));
                self.tabs[self.active].history.as_mut().expect("just read").blame = BlameState::Idle;
            }
            HistoryView::File => {
                self.open_inner(file_target(&row), ViewKind::File, true);
                self.read_contents(false, cx);
                self.read_blame(cx);
            }
            HistoryView::Changes => {
                self.open_inner(changes_target(&row, None), ViewKind::Diff, false);
                self.read_contents(false, cx);
            }
        }
    }

    fn open_inner(&mut self, file: FileChange, view: ViewKind, history_file: bool) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let prefs = tab.diff.as_ref().or(tab.history.as_ref().and_then(|h| h.saved.as_ref()));
        let mut state = DiffState::carrying(file, prefs);
        state.view = view;
        state.history_file = history_file;
        tab.diff = Some(state);
    }

    /// The blame of the selected row's file, from the cache if it's there.
    fn read_blame(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let repo = tab.id;
        let Some(h) = tab.history.as_mut() else { return };
        if !h.blame_on || h.view != HistoryView::File || cfg!(test) {
            return;
        }
        let Some(row) = h.selected_row() else { return };
        let (rev, path) = (row.sha.to_string(), row.path.clone());
        let key = (rev.clone(), path.clone());
        if let Some(hit) = h.blame_cache.iter().find(|(k, _)| *k == key) {
            h.blame = BlameState::Ready(hit.1.clone());
            return;
        }
        h.blame = BlameState::Loading;
        let worktree = h.worktree.clone();
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(Request::Blame { repo: crate::app::core_repo(repo), worktree, rev, path }).await });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                let Some(h) = this.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.history.as_mut()) else { return };
                let shown = h.selected_row().is_some_and(|r| r.sha.as_ref() == key.0 && r.path == key.1);
                match result {
                    Ok(Ok(v)) => {
                        let data = Rc::new(blame_of(&v));
                        if shown {
                            h.blame = BlameState::Ready(data.clone());
                        }
                        h.blame_cache.push((key, data));
                        if h.blame_cache.len() > BLAME_CACHE {
                            h.blame_cache.remove(0);
                        }
                    }
                    Ok(Err(e)) if shown => h.blame = BlameState::Failed(e.message.into()),
                    Err(e) if shown => h.blame = BlameState::Failed(e.to_string().into()),
                    _ => {}
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A blame group's commit opens in this list, pages loading until it shows; Alt+click selects it
    /// in the graph, which the view closes to show.
    pub fn pick_blame(&mut self, sha: SharedString, in_graph: bool, cx: &mut Context<Self>) {
        if in_graph {
            self.close_history(cx);
            let at = self.tabs.get(self.active).and_then(|t| t.graph.as_ref()?.by_sha.get(sha.as_ref()).copied());
            match at {
                Some(at) => self.select(at, cx),
                None => self.show_toast("Not in the loaded history", cx),
            }
            return;
        }
        let Some(h) = self.tabs.get_mut(self.active).and_then(|t| t.history.as_mut()) else { return };
        h.seek = Some((sha.to_string(), SEEK_PAGES));
        self.continue_seek(cx);
    }

    fn continue_seek(&mut self, cx: &mut Context<Self>) {
        let Some(h) = self.tabs.get_mut(self.active).and_then(|t| t.history.as_mut()) else { return };
        let Some((sha, pages)) = h.seek.clone() else { return };
        if h.rows.iter().any(|r| r.sha.as_ref() == sha) {
            h.seek = None;
            self.select_history_row(sha.into(), cx);
            return;
        }
        if !h.more || h.error.is_some() || pages == 0 {
            let short: String = sha.chars().take(SHORT_SHA).collect();
            h.seek = None;
            self.show_toast(format!("{short} isn't in this file's history"), cx);
            return;
        }
        if !h.loading {
            h.seek = Some((sha, pages - 1));
            self.load_history(cx);
        }
    }

    fn with_history(&mut self, f: impl FnOnce(&mut FileHistory)) {
        if let Some(h) = self.tabs.get_mut(self.active).and_then(|t| t.history.as_mut()) {
            f(h);
        }
    }
}

fn sha_button(sha: SharedString, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let short: String = sha.chars().take(SHORT_SHA).collect();
    let id: SharedString = format!("copy-{sha}").into();
    div().id(id).flex_none().font_family(crate::app::MONO_FONT).text_size(px(11.)).text_color(theme::text_secondary()).cursor_pointer().child(short).on_click(cx.listener(move |this, _, _, cx| {
        cx.stop_propagation();
        cx.write_to_clipboard(ClipboardItem::new_string(sha.to_string()));
        this.show_toast("Copied", cx);
    }))
}

fn history_row(r: &HistoryRow, on: bool, cx: &mut Context<GitVolt>) -> AnyElement {
    let sha = r.sha.clone();
    let id: SharedString = format!("fh-{sha}").into();
    let meta = format!("{} · {}", relative_time(r.time, crate::format::now()), r.author);
    div()
        .id(id)
        .flex()
        .w_full()
        .h(px(ROW_H))
        .items_center()
        .gap(px(8.))
        .px(px(8.))
        .py(px(4.))
        .text_color(if on { theme::text_selected() } else { theme::text_normal() })
        .when(on, |d| d.bg(theme::selected_row()))
        .when(!on, |d| d.hover(|d| d.bg(theme::hover_row())))
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(tab) = this.tabs.get_mut(this.active) {
                tab.keys = KeyArea::History;
            }
            this.select_history_row(sha.clone(), cx);
            cx.notify();
        }))
        .child(avatar(&r.author, &r.email, 20.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().flex().gap(px(8.)).items_baseline().child(div().flex_1().min_w_0().truncate().child(r.summary.clone())).child(sha_button(r.sha.clone(), cx)))
                .child(div().truncate().text_size(px(11.)).line_height(px(15.4)).text_color(theme::text_secondary()).child(meta)),
        )
        .into_any_element()
}

/// What follows the rows: a failure, "Load more", or where the history ends.
fn list_end(h: &FileHistory, cx: &mut Context<GitVolt>) -> AnyElement {
    let mut out = div().flex().flex_col().w_full().text_color(theme::text_secondary());
    if let Some(e) = &h.error {
        out = out.child(
            div().flex().flex_wrap().gap(px(4.)).px(px(8.)).py(px(6.)).child(format!("Couldn't load the history: {e}")).child(
                div()
                    .id("fh-retry")
                    .cursor_pointer()
                    .text_color(theme::blue())
                    .child("Retry")
                    .on_click(cx.listener(|this, _, _, cx| this.load_history(cx))),
            ),
        );
    }
    if h.more && h.error.is_none() {
        let loading = h.loading;
        out = out.child(div().flex().justify_center().m(px(8.)).child(
            div().id("fh-more").px(px(8.)).py(px(2.)).rounded(px(4.)).bg(theme::control_bg()).when(!loading, |d| d.hover(|d| d.bg(theme::control_hover_bg())).on_click(cx.listener(|this, _, _, cx| this.load_history(cx)))).child(if loading { "Loading…" } else { "Load more" }),
        ));
    }
    if let Some(added) = h.end() {
        out = out.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(2.))
                .p(px(8.))
                .text_size(px(12.))
                .line_height(px(16.8))
                .when(h.rows.is_empty(), |d| d.child(format!("No commit changed {}", h.path)))
                .when_some(added, |d, sha| d.child(div().flex().items_center().gap(px(3.)).child("Added in").child(sha_button(sha, cx))))
                .child("End of history"),
        );
    }
    out.into_any_element()
}

/// Where a group lies in the rows' layout: its first line's top and its last's bottom, wherever the
/// lines are folded or wrapped.
fn group_span(diff: &DiffState, g: &BlameGroup) -> (f32, f32) {
    let line = |r: &crate::diff::model::Row| r.new.map_or(0, |n| n + 1);
    let top = diff.rows.partition_point(|r| line(r) < g.start);
    let end = diff.rows.partition_point(|r| line(r) < g.start + g.lines);
    let at = |i: usize| diff.layout.offsets.get(i).copied().unwrap_or_else(|| diff.layout.content());
    (at(top), at(end))
}

/// The gutter, left of the line numbers: per line group a colour bar, the avatar, the summary and the
/// date, on the code's line box. It's reserved while `blame` loads.
fn blame_strip(h: &FileHistory, diff: &DiffState, width: f32, cx: &mut Context<GitVolt>) -> AnyElement {
    let mut strip = div().id("blame-strip").relative().flex_none().w(px(width)).h_full().overflow_hidden().bg(theme::app_bg()).on_scroll(cx.listener(|this, e: &ScrollEvent, window, cx| {
        let dy = e.delta.pixel_delta(window.line_height()).y.as_f32();
        this.with_diff(cx, |d| d.scroll_by(-dy));
    }));
    if let BlameState::Ready(data) = &h.blame
        && !diff.rows.is_empty()
    {
        let (first, last) = (diff.layout.row_at(diff.scroll), diff.layout.row_at(diff.scroll + diff.layout.height));
        let line = |i: usize| diff.rows.get(i).and_then(|r| r.new).map_or(0, |n| n + 1);
        let now = crate::format::now();
        for g in &data.groups[visible_groups(&data.groups, line(first), line(last))] {
            let (top, bottom) = group_span(diff, g);
            if bottom <= top {
                continue;
            }
            let c = data.commits.get(&g.sha);
            let sha = g.sha.clone();
            let id: SharedString = format!("blame-{}", g.start).into();
            let tip_key = format!("blame:{}:{}", g.sha, g.start);
            let card = c.map(|c| (c.summary.clone(), c.author.clone(), c.time));
            strip = strip.child(
                crate::ui::tooltip::Anchor::new(div()
                    .id(id)
                    .absolute()
                    .top(px(top - diff.scroll))
                    .left_0()
                    .right_0()
                    .h(px(bottom - top))
                    .flex()
                    .gap(px(6.))
                    .pr(px(6.))
                    .overflow_hidden()
                    .bg(theme::panel_bg())
                    .text_color(theme::text_secondary())
                    .line_height(px(crate::metrics::line_h()))
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::hover_row()).text_color(theme::text_normal()))
                    .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| this.pick_blame(sha.clone(), e.modifiers().alt, cx)))
                    .child(div().flex_none().w(px(3.)).h_full().bg(theme::lane(color_index(&g.sha, 10))))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap(px(6.))
                            .h(px(crate::metrics::line_h()))
                            .when_some(c, |d, c| d.child(avatar(&c.author, &c.email, AVATAR_PX)))
                            .child(div().flex_1().min_w_0().truncate().when_some(c, |d, c| d.child(c.summary.clone())))
                            .child(div().flex().flex_none().w(px(45.5)).justify_end().when_some(c, |d, c| d.child(compact_relative_time(c.time, now)))),
                    )
                    // The separator is a line on top, so it doesn't push the row down.
                    .child(div().absolute().top_0().left_0().right_0().h(px(1.)).bg(theme::section_border())))
                .card(tip_key, move |_, _| match &card {
                    Some((summary, author, time)) => blame_tip(summary, author, *time),
                    None => div().into_any_element(),
                })
                .placement(crate::ui::tooltip::Placement::Pointer),
            );
        }
    }
    strip.into_any_element()
}

fn view_switch(view: HistoryView, cx: &mut Context<GitVolt>) -> AnyElement {
    segmented(vec![
        segment("fh-file", "File", view == HistoryView::File, true).on_click(cx.listener(|this, _, _, cx| this.set_history_view(HistoryView::File, cx))).into_any_element(),
        segment("fh-changes", "Changes", view == HistoryView::Changes, true).on_click(cx.listener(|this, _, _, cx| this.set_history_view(HistoryView::Changes, cx))).into_any_element(),
    ])
    .into_any_element()
}

/// `BlameTip`: the commit's summary; who and when under it.
fn blame_tip(summary: &SharedString, author: &SharedString, time: i64) -> AnyElement {
    let when = format!("{author} · {} · {}", crate::format::relative_time(time, crate::format::now()), crate::model::format_date(time));
    div().flex().flex_col().flex_none().max_w(px(320.)).px(px(8.)).py(px(5.)).rounded(px(4.)).border_1().border_color(theme::section_border()).bg(theme::panel_bg1()).text_color(theme::text_selected()).text_size(px(12.)).line_height(px(16.8)).shadow(crate::ui::shadow(theme::shadow_tooltip(), 2., 8.)).child(summary.clone()).child(div().text_color(theme::text_secondary()).child(when)).into_any_element()
}

pub fn history_panel(tab: &Tab, width: f32, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let h = tab.history.as_ref()?;
    let diff = tab.diff.as_ref();
    let no_blame = h.no_blame(diff);
    let pressed = !no_blame && h.blame_on;
    let list_w = h.list_w;
    let header = div()
        .flex()
        .flex_none()
        .h(px(HISTORY_H))
        .items_center()
        .bg(theme::file_bar_bg())
        .border_b_1()
        .border_color(theme::section_border())
        .child(div().flex_none().w(px(list_w)).truncate().px(px(8.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).flex().gap(px(3.)).child("File History:").child(div().min_w_0().truncate().font_weight(FontWeight::NORMAL).child(h.path.clone())))
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .justify_between()
                .pl(px(4.))
                .pr(px(8.))
                .child(
                    crate::ui::tooltip::tip(if pressed { "Hide who last changed each line" } else { "Show who last changed each line" }, div()
                        .id("blame-toggle")
                        .px(px(8.))
                        .py(px(2.))
                        .rounded(px(4.))
                        .text_color(theme::text_secondary())
                        .when(pressed, |d| d.bg(theme::selected_row()).text_color(theme::text_selected()))
                        .when(no_blame, |d| d.opacity(0.5))
                        .when(!no_blame, |d| d.cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.toggle_blame(cx))))
                        .child("Blame")),
                )
                .child(
                    crate::ui::tooltip::tip("Close (Esc)", div()
                        .id("history-close")
                        .flex()
                        .flex_none()
                        .size(px(22.))
                        .items_center()
                        .justify_center()
                        .rounded(px(3.))
                        .hover(|d| d.bg(theme::control_hover_bg()))
                        .on_click(cx.listener(|this, _, _, cx| this.close_history(cx)))
                        .child(icon("x", 14., theme::text_secondary()))),
                ),
        );
    let rows = h.rows.clone();
    let selected = h.selected.clone();
    let items = list(
        h.list.clone(),
        cx.processor(move |this, ix: usize, _, cx| {
            match rows.get(ix) {
                Some(r) => history_row(r, Some(&r.sha) == selected.as_ref(), cx),
                None => this.tabs.get(this.active).and_then(|t| t.history.as_ref()).map_or_else(|| div().into_any_element(), |h| list_end(h, cx)),
            }
        }),
    )
    .flex_1()
    .min_h_0();
    let left = div().flex().flex_col().flex_none().w(px(list_w)).h_full().min_h_0().border_r_1().border_color(theme::section_border()).bg(theme::panel_bg()).child(items);
    let editor = width - list_w;
    let margin = h.margin(diff, editor);
    let right = div().flex().flex_col().flex_1().min_w_0().min_h_0().when_some(diff, |d, diff| {
        d.child(toolbar(diff, ToolbarKind::History(view_switch(h.view, cx)), editor, cx)).child(
            div()
                .relative()
                .flex()
                .flex_1()
                .min_h_0()
                .when(margin > 0., |d| d.child(blame_strip(h, diff, margin, cx)))
                .child(diff_body(diff, editor - margin, &tab.zoom_input, cx))
                .when_some(if let BlameState::Failed(e) = &h.blame { Some(e.clone()) } else { None }, |d, e| {
                    d.child(div().absolute().left(px(8.)).bottom(px(8.)).px(px(8.)).py(px(4.)).rounded(px(4.)).bg(theme::panel_bg1()).text_color(theme::text_secondary()).child(format!("Couldn't load the blame: {e}")))
                }),
        )
    });
    let resizer = div()
        .id("fh-resizer")
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(list_w - 2.))
        .w(px(4.))
        .cursor(CursorStyle::ResizeColumn)
        .hover(|d| d.bg(theme::blue()))
        .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            this.with_history(|h| h.resizing = Some((e.position.x.as_f32(), h.list_w)));
        }))
        .on_click(cx.listener(|this, e: &ClickEvent, _, cx| {
            // A double click resets it.
            if e.click_count() == 2 {
                this.with_history(|h| h.list_w = LIST_W);
                cx.notify();
            }
        }));
    Some(
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(theme::app_bg())
            .child(header)
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                        let resizing = this.tabs.get(this.active).and_then(|t| t.history.as_ref()).and_then(|h| h.resizing);
                        if let (Some((from, w)), Some(MouseButton::Left)) = (resizing, e.pressed_button) {
                            this.with_history(|h| h.list_w = (w + e.position.x.as_f32() - from).clamp(LIST_MIN, LIST_MAX).round());
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.with_history(|h| h.resizing = None)))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, _| this.with_history(|h| h.resizing = None)))
                    .child(left)
                    .child(right)
                    .child(resizer),
            )
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::with_rows;
    use crate::diff::view::DiffData;
    use std::prelude::v1::test;
    use std::sync::Arc;

    /// A strip's top: title bar, tab bar, toolbar, the history's header.
    const BODY_TOP: f32 = 30. + 34. + 44. + HISTORY_H;

    fn opened(v: &mut GitVolt, cx: &mut Context<GitVolt>, blame: bool) {
        let mut h = FileHistory::new(start(), blame, HistoryView::File, Some(DiffState::new(FileChange { path: "open.rs".into(), old_path: None, status: 'M', additions: None, deletions: None, old: Value::Null, new: Value::Null, conflict: None, origin: Origin::Commit { id: "c".into() } })));
        h.with_page(vec![row("aa", "M"), row("bb", "M"), row("cc", "A")], false);
        let tab = &mut v.tabs[0];
        tab.history = Some(h);
        tab.keys = KeyArea::History;
        v.show_history_row(cx);
        cx.notify();
    }

    #[gpui::test]
    fn a_click_or_an_arrow_picks_a_commit_and_esc_gives_back_the_open_file(cx: &mut TestAppContext) {
        let (window, mut cx, _runtime) = with_rows(cx, 1);
        window.update(&mut cx, |v, _, cx| opened(v, cx, false)).unwrap();
        cx.run_until_parked();
        let shown = |window: &WindowHandle<GitVolt>, cx: &mut VisualTestContext| window.read_with(cx, |v, _| (v.tabs[0].history.as_ref().and_then(|h| h.selected.clone()), v.tabs[0].diff.as_ref().map(|d| d.file.new["commit"].as_str().unwrap_or_default().to_string()))).unwrap();
        assert_eq!(shown(&window, &mut cx), (Some("aa".into()), Some("aa".into())));
        cx.simulate_click(point(px(150.), px(BODY_TOP + ROW_H * 1.5)), Modifiers::none());
        assert_eq!(shown(&window, &mut cx), (Some("bb".into()), Some("bb".into())));
        cx.simulate_keystrokes("down");
        assert_eq!(shown(&window, &mut cx), (Some("cc".into()), Some("cc".into())));
        cx.simulate_keystrokes("up");
        assert_eq!(shown(&window, &mut cx).0, Some("bb".into()));
        cx.simulate_keystrokes("escape");
        window
            .read_with(&cx, |v, _| {
                assert!(v.tabs[0].history.is_none());
                assert_eq!(v.tabs[0].diff.as_ref().unwrap().file.path.as_ref(), "open.rs");
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_click_on_a_blame_group_picks_its_commit_and_the_toggle_hides_the_strip(cx: &mut TestAppContext) {
        let (window, mut cx, _runtime) = with_rows(cx, 1);
        window
            .update(&mut cx, |v, _, cx| {
                opened(v, cx, true);
                let tab = &mut v.tabs[0];
                tab.diff.as_mut().unwrap().set_data(Arc::new(DiffData::build(None, Some("one\ntwo\nthree\nfour"), false, "UTF-8".into(), false)));
                let commit = |s: &str| (SharedString::from(s.to_string()), BlameCommit { author: "Ada".into(), email: String::new(), time: 0, summary: "s".into() });
                tab.history.as_mut().unwrap().blame = BlameState::Ready(Rc::new(BlameData { groups: group_blame(vec![("aa".into(), 1, 2), ("bb".into(), 3, 2)]), commits: HashMap::from([commit("aa"), commit("bb")]) }));
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        // The third line, 33 px of toolbar under the header, is the second group's.
        let group = point(px(41. + LIST_W + 60.), px(BODY_TOP + 33. + 18. * 2.5));
        cx.simulate_click(group, Modifiers::none());
        window.read_with(&cx, |v, _| assert_eq!(v.tabs[0].history.as_ref().unwrap().selected.as_deref(), Some("bb"))).unwrap();
        window.read_with(&cx, |v, _| assert_eq!(v.tabs[0].history.as_ref().unwrap().margin(v.tabs[0].diff.as_ref(), 800.), 200.)).unwrap();
        window.update(&mut cx, |v, _, cx| v.toggle_blame(cx)).unwrap();
        window.read_with(&cx, |v, _| assert_eq!(v.tabs[0].history.as_ref().unwrap().margin(v.tabs[0].diff.as_ref(), 800.), 0.)).unwrap();
    }

    fn row(sha: &str, status: &str) -> HistoryRow {
        HistoryRow { sha: sha.to_string().into(), parents: vec!["p".into()], author: "Ada".into(), email: String::new(), time: 0, summary: "s".into(), path: "a.rs".into(), status: status.into(), old_path: None }
    }

    fn start() -> HistoryStart {
        HistoryStart { path: "a.rs".into(), rev: None, worktree: "/w".into() }
    }

    #[test]
    fn a_page_appends_skips_what_is_there_and_selects_the_first() {
        let mut h = FileHistory::new(start(), false, HistoryView::File, None);
        h.with_page(vec![row("a", "M"), row("b", "M")], true);
        assert_eq!(h.selected.as_deref(), Some("a"));
        h.selected = Some("b".into());
        h.with_page(vec![row("b", "M"), row("c", "A")], false);
        assert_eq!(h.rows.iter().map(|r| r.sha.as_ref()).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(h.selected.as_deref(), Some("b"));
        assert_eq!(h.end(), Some(Some("c".into())));
    }

    #[test]
    fn the_arrows_step_and_stop_at_the_ends() {
        let mut h = FileHistory::new(start(), false, HistoryView::File, None);
        h.with_page(vec![row("a", "M"), row("b", "M")], false);
        assert!(h.step(true));
        assert_eq!(h.selected.as_deref(), Some("b"));
        assert!(!h.step(true));
        assert!(h.step(false));
        assert!(!h.step(false));
        assert_eq!(h.end(), Some(None), "the oldest row didn't add the file");
    }

    #[test]
    fn the_end_waits_for_the_last_page() {
        let mut h = FileHistory::new(start(), false, HistoryView::File, None);
        assert_eq!(h.end(), None);
        h.with_page(vec![], false);
        assert_eq!(h.end(), Some(None));
    }

    #[test]
    fn blame_groups_merge_what_runs_on_and_search_by_line() {
        let g = group_blame(vec![("b".into(), 3, 1), ("a".into(), 1, 2), ("b".into(), 4, 2), ("a".into(), 7, 1)]);
        assert_eq!(g.iter().map(|g| (g.sha.as_ref(), g.start, g.lines)).collect::<Vec<_>>(), [("a", 1, 2), ("b", 3, 3), ("a", 7, 1)]);
        assert_eq!(visible_groups(&g, 2, 3), 0..2);
        assert_eq!(visible_groups(&g, 6, 6), 2..2, "a line in no group");
        assert_eq!(visible_groups(&g, 7, 100), 2..3);
        assert_eq!(visible_groups(&g, 5, 2), 0..0);
    }

    #[test]
    fn a_commits_colour_is_fnv_of_its_sha() {
        // FNV-1a of "a" is 0xe40c292c, as `blame.test.ts` has it.
        assert_eq!(color_index("a", 1 << 16), 0xe40c292c_usize % (1 << 16));
        assert_eq!(color_index("a", 10), 0xe40c292c_usize % 10);
    }

    #[test]
    fn changes_are_against_the_first_parent_with_an_add_or_delete_one_sided() {
        let m = changes_target(&row("c", "M"), None);
        assert_eq!((m.old["commit"].as_str(), m.new["commit"].as_str(), m.status), (Some("p"), Some("c"), 'M'));
        let a = changes_target(&row("c", "A"), None);
        assert_eq!(a.old["kind"].as_str(), Some("absent"));
        let d = changes_target(&row("c", "D"), None);
        assert_eq!(d.new["kind"].as_str(), Some("absent"));
    }

    #[test]
    fn a_rename_takes_its_sides_from_the_commits_list() {
        let mut r = row("c", "R");
        r.old_path = Some("b.rs".into());
        assert!(needs_file_list(&r));
        assert_eq!(changes_target(&r, None).old["kind"].as_str(), Some("absent"), "no list: it shows as added");
        let entry = FileChange { path: "a.rs".into(), old_path: Some("b.rs".into()), status: 'R', additions: None, deletions: None, old: json!({"kind": "atCommit", "commit": "p"}), new: at_commit("c"), conflict: None, origin: Origin::Commit { id: "c".into() } };
        let t = changes_target(&r, Some(&[entry]));
        assert_eq!((t.old_path.as_deref(), t.status), (Some("b.rs"), 'R'));
        assert!(needs_file_list(&row("c", "")), "a merge");
        assert!(!needs_file_list(&row("c", "M")));
    }

    #[test]
    fn history_starts_at_the_commit_the_parent_or_head() {
        let file = |status: char, new: Value, origin: Origin| FileChange { path: "n.rs".into(), old_path: Some("o.rs".into()), status, additions: None, deletions: None, old: Value::Null, new, conflict: None, origin };
        let commit = Origin::Commit { id: "c".into() };
        let at = history_start(&file('M', at_commit("c"), commit.clone()), &[], "/w").unwrap();
        assert_eq!((at.path.as_str(), at.rev.as_deref()), ("n.rs", Some("c")));
        let gone = history_start(&file('D', ABSENT(), commit.clone()), &["p".into()], "/w").unwrap();
        assert_eq!((gone.path.as_str(), gone.rev.as_deref()), ("o.rs", Some("p")));
        assert_eq!(history_start(&file('D', ABSENT(), commit), &[], "/w"), None, "a root commit has nothing");
        let wip = Origin::Wip { worktree: "/x".into(), staged: false };
        let head = history_start(&file('M', Value::Null, wip.clone()), &[], "/w").unwrap();
        assert_eq!((head.path.as_str(), head.rev, head.worktree.as_str()), ("o.rs", None, "/x"));
        assert_eq!(history_start(&file('A', Value::Null, wip), &[], "/w"), None, "new in the WIP");
    }
}
