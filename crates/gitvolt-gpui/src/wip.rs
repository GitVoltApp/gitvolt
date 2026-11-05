//! The WIP row's panel (`ui/src/details/{WipHeader,WipSections}.tsx`, `ui/src/stage/*`,
//! `ui/src/commit/*`): `N file changes on [branch]`, the Conflicted, Unstaged and Staged lists with their
//! stage, unstage and discard, one filter over them, and the commit box below.

use crate::app::{GitVolt, KeyArea, Tab, icon};
use crate::details::{Cursor, FileChange, Files, ListMode, ListRow, ListView, Origin, file_changes, file_in_worktree, file_list, list_rows_in, segment_with_icon, segmented, status_icon, UNCHANGED};
use crate::diff::view::DiffState;
use crate::theme;
use crate::ui::arm::{self, ArmRequest, Origin as ArmOrigin};
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::menu::{self, MenuRow, grouped};
use crate::ui::tooltip::tip;
use crate::write::{Write, WriteCtx, call};
use gitvolt_core::api::Request;
use gitvolt_core::write::types::StagingUndoState;
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

pub const COMMIT_QUEUED: &str = "Commit queued";
pub const TYPE_A_MESSAGE: &str = "Type a message to commit";
pub const STAGING_BUSY: &str = "Staging…";
const FILE_ROW_H: f32 = 26.;
const HEADER_H: f32 = 37.;
const VIEW_BAR_H: f32 = 36.;
pub const WIP_DRAFT_COUNTER_FROM: usize = 60;
pub const WIP_DRAFT_WARN_FROM: usize = 72;
/// `wipPanelPrefs`: each section keeps its header and tool line plus three rows.
const SPLIT_CHROME: f32 = 64.;
const SPLIT_MIN_ROWS: f32 = 3.;
pub const SPLIT_DEFAULT: f32 = 0.5;

/// A WIP list section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Which {
    Conflicted,
    Unstaged,
    Staged,
}

const SECTIONS: [Which; 3] = [Which::Conflicted, Which::Unstaged, Which::Staged];

impl Which {
    /// A file's section: an unmerged path goes in Conflicted, and in neither of the others.
    pub fn of(f: &FileChange) -> Option<Self> {
        match (&f.origin, f.status) {
            (Origin::Wip { staged: false, .. }, 'U') => Some(Which::Conflicted),
            (Origin::Wip { staged: true, .. }, 'U') => None,
            (Origin::Wip { staged: false, .. }, _) => Some(Which::Unstaged),
            (Origin::Wip { staged: true, .. }, _) => Some(Which::Staged),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Which::Conflicted => "Conflicted",
            Which::Unstaged => "Unstaged",
            Which::Staged => "Staged",
        }
    }

    fn scope(self) -> &'static str {
        match self {
            Which::Conflicted => "c",
            Which::Unstaged => "u",
            Which::Staged => "s",
        }
    }

    fn index(self) -> usize {
        match self {
            Which::Conflicted => 0,
            Which::Unstaged => 1,
            Which::Staged => 2,
        }
    }
}

pub struct WipSection {
    pub which: Which,
    /// Where its files are in `Files::changes`.
    pub start: usize,
    pub count: usize,
    /// Before the filter.
    pub total: usize,
    pub rows: Rc<Vec<ListRow>>,
    pub list: UniformListScrollHandle,
    pub added: u32,
    pub deleted: u32,
}

pub struct WipList {
    pub worktree: String,
    pub sections: Vec<WipSection>,
    /// The sections a collapse hides, from the app's prefs: the arrow keys pass them by.
    pub closed: [bool; 3],
    /// What "View all files" lists around the changes: the worktree's tracked files.
    pub all_paths: Option<Vec<String>>,
}

impl WipList {
    fn at(&self, index: usize) -> Option<&WipSection> {
        self.sections.iter().find(|s| index >= s.start && index < s.start + s.count)
    }

    pub fn scope_of(&self, index: usize) -> &'static str {
        self.at(index).map_or("", |s| s.which.scope())
    }

    pub fn section(&self, which: Which) -> &WipSection {
        &self.sections[which.index()]
    }

    /// Puts its section's list at file `index`.
    pub fn reveal(&self, index: usize) {
        let Some(s) = self.at(index) else { return };
        if let Some(row) = s.rows.iter().position(|r| matches!(r, ListRow::File { index: i, .. } if *i == index)) {
            s.list.scroll_to_item(row, ScrollStrategy::Nearest);
        }
    }
}

fn empty_sections() -> Vec<WipSection> {
    SECTIONS.iter().map(|w| WipSection { which: *w, start: 0, count: 0, total: 0, rows: Rc::new(Vec::new()), list: UniformListScrollHandle::new(), added: 0, deleted: 0 }).collect()
}

impl Files {
    /// A WIP row's lists.
    pub fn wip(worktree: String, unstaged: &Value, staged: &Value) -> Self {
        let mut f = Files::new(Vec::new(), 0, 0);
        f.wip = Some(WipList { worktree, sections: empty_sections(), closed: [false; 3], all_paths: None });
        f.set_wip_lists(unstaged, staged);
        f
    }

    /// Fresh lists: the mode, the filter and what's collapsed stay; the cursor stays on its file.
    pub fn set_wip_lists(&mut self, unstaged: &Value, staged: &Value) {
        let Some(wip) = self.wip.as_ref() else { return };
        let worktree = wip.worktree.clone();
        let mut all = file_changes(unstaged, &Origin::Wip { worktree: worktree.clone(), staged: false });
        all.extend(file_changes(staged, &Origin::Wip { worktree, staged: true }));
        self.changed = Rc::new(all);
        self.rebuild();
    }

    /// What's listed: the changes by section, each filtered by `query` and sorted. Its ranges are noted.
    pub(crate) fn wip_listed(&mut self, query: &str, by_status: bool) -> Vec<FileChange> {
        let Some(wip) = self.wip.as_mut() else { return Vec::new() };
        let mut out = Vec::new();
        let rank = |f: &FileChange| if by_status { "AMTRCDUX".find(f.status).unwrap_or(8) } else { 0 };
        for s in &mut wip.sections {
            let mine: Vec<&FileChange> = self.changed.iter().filter(|f| Which::of(f) == Some(s.which)).collect();
            s.total = mine.len();
            let matches = |p: &str| query.is_empty() || p.to_lowercase().contains(query);
            let mut shown: Vec<FileChange> = mine.iter().filter(|f| matches(&f.path)).map(|f| (*f).clone()).collect();
            s.added = shown.iter().map(|f| f.additions.unwrap_or(0)).sum();
            s.deleted = shown.iter().map(|f| f.deletions.unwrap_or(0)).sum();
            if s.which == Which::Unstaged
                && let Some(paths) = &wip.all_paths
            {
                let listed: HashSet<&str> = mine.iter().map(|f| f.path.as_ref()).collect();
                shown.extend(paths.iter().filter(|p| !listed.contains(p.as_str()) && matches(p)).map(|p| file_in_worktree(p, &wip.worktree)));
                shown.sort_by(|a, b| a.path.cmp(&b.path));
            }
            shown.sort_by_key(rank);
            s.start = out.len();
            s.count = shown.len();
            out.extend(shown);
        }
        out
    }

    pub(crate) fn relist_wip(&mut self) {
        let (changes, mode) = (self.changes.clone(), self.mode);
        let Some(wip) = self.wip.as_mut() else { return };
        let mut rows = Vec::new();
        for s in &mut wip.sections {
            let list = list_rows_in(&changes[s.start..s.start + s.count], mode, &self.collapsed, s.which.scope(), s.start);
            rows.extend(list.iter().cloned());
            s.rows = Rc::new(list);
        }
        self.rows = Rc::new(rows);
    }

    /// Every file of the sections that aren't collapsed, in display order.
    pub(crate) fn wip_order(&self) -> Vec<usize> {
        let Some(wip) = self.wip.as_ref() else { return Vec::new() };
        let mut out = Vec::new();
        for s in &wip.sections {
            if wip.closed[s.which.index()] {
                continue;
            }
            let rows = list_rows_in(&self.changes[s.start..s.start + s.count], self.mode, &HashSet::new(), s.which.scope(), s.start);
            out.extend(rows.iter().filter_map(|r| if let ListRow::File { index, .. } = r { Some(*index) } else { None }));
        }
        out
    }

    /// Collapse all: of each section's folders.
    pub(crate) fn toggle_all_wip(&mut self) {
        if self.collapsed.is_empty() {
            let Some(wip) = self.wip.as_ref() else { return };
            let mut keys = HashSet::new();
            for s in &wip.sections {
                let rows = list_rows_in(&self.changes[s.start..s.start + s.count], ListMode::Tree, &HashSet::new(), s.which.scope(), s.start);
                keys.extend(rows.iter().filter_map(|r| if let ListRow::Folder { key, .. } = r { Some(key.to_string()) } else { None }));
            }
            self.collapsed = keys;
        } else {
            self.collapsed.clear();
        }
        if let Some(Cursor::Folder(_)) = self.cursor {
            self.cursor = None;
        }
        self.relist();
    }

    /// The files of a section, shown ones.
    pub fn section_files(&self, which: Which) -> Vec<FileChange> {
        let Some(s) = self.wip.as_ref().map(|w| w.section(which)) else { return Vec::new() };
        self.changes[s.start..s.start + s.count].iter().filter(|f| f.status != UNCHANGED).cloned().collect()
    }

    /// Each changed path once, in all sections, whatever the filter.
    pub fn changed_paths(&self) -> HashSet<&str> {
        self.changed.iter().map(|f| f.path.as_ref()).collect()
    }

    /// What Discard all confirms: the count shown, and every path, a rename's both.
    pub fn discard_scope(&self) -> (usize, Vec<String>) {
        let shown = self.changed_paths().len();
        let mut paths: Vec<String> = Vec::new();
        for f in self.changed.iter() {
            for p in [Some(f.path.to_string()), f.old_path.clone()].into_iter().flatten() {
                if !paths.contains(&p) {
                    paths.push(p);
                }
            }
        }
        (shown, paths)
    }

    /// `(staged, unstaged, conflicted)`, whatever's filtered.
    pub fn counts(&self) -> (usize, usize, usize) {
        let n = |w: Which| self.changed.iter().filter(|f| Which::of(f) == Some(w)).count();
        (n(Which::Staged), n(Which::Unstaged), n(Which::Conflicted))
    }
}

/// The paths a row stands for: a rename's both.
pub fn paths_of(files: &[FileChange]) -> Vec<String> {
    files.iter().flat_map(|f| [Some(f.path.to_string()), f.old_path.clone()].into_iter().flatten()).collect()
}

/// A row's files: the file itself, or every file under a tree-mode folder.
pub fn files_under(row: &ListRow, changes: &[FileChange], which: Which, section: Option<&WipSection>) -> Vec<FileChange> {
    match row {
        ListRow::File { index, .. } => changes.get(*index).filter(|f| f.status != UNCHANGED).cloned().into_iter().collect(),
        ListRow::Folder { path, .. } => {
            let prefix = format!("{path}/");
            let range = section.map_or(0..changes.len(), |s| s.start..s.start + s.count);
            changes[range].iter().filter(|f| Which::of(f) == Some(which) && f.path.starts_with(&prefix) && f.status != UNCHANGED).cloned().collect()
        }
    }
}

/// `{ armed label, whether it's disabled and why }` for Discard on `files` of a row named `name`.
fn discard_arm(n: usize, name: &str) -> String {
    if n == 1 { format!("Click again to discard {name}") } else { format!("Click again to discard {n} files in {name}") }
}

/// The commit button: its label, why it's disabled, and whether it stages everything first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitView {
    pub label: String,
    pub reason: Option<String>,
    pub stage_all: bool,
}

pub struct CommitState<'a> {
    pub staged: usize,
    pub unstaged: usize,
    pub conflicted: usize,
    pub amend: bool,
    pub in_merge: bool,
    pub summary: &'a str,
    pub staging: bool,
}

fn files_word(n: usize) -> String {
    format!("{n} {}", if n == 1 { "file" } else { "files" })
}

/// "Resolve 2 conflicted files first".
pub fn resolve_first(n: usize) -> String {
    format!("Resolve {n} conflicted {} first", if n == 1 { "file" } else { "files" })
}

pub fn commit_button(s: &CommitState) -> CommitView {
    let stage_all = !s.amend && !s.in_merge && s.staged == 0 && s.unstaged > 0;
    let action = if s.amend {
        "Amend previous commit".to_string()
    } else if s.in_merge {
        "Commit and merge".to_string()
    } else if stage_all {
        "Stage all & commit".to_string()
    } else {
        format!("Commit changes to {}", files_word(s.staged))
    };
    let empty = s.summary.trim().is_empty();
    let reason = if s.staging {
        Some(STAGING_BUSY.to_string())
    } else if s.conflicted > 0 {
        Some(resolve_first(s.conflicted))
    } else if !s.amend && !s.in_merge && s.staged == 0 && s.unstaged == 0 {
        Some("Nothing to commit".to_string())
    } else if empty {
        Some("Write a commit summary".to_string())
    } else {
        None
    };
    CommitView { label: if empty { TYPE_A_MESSAGE.to_string() } else { action }, reason, stage_all }
}

/// The WIP draft: one per worktree, shown in the WIP row's box and the commit box.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    pub summary: String,
    pub description: String,
}

fn lf(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

impl Draft {
    /// A whole message (HEAD's) as the two fields; CRLF and a description's leading blank lines go.
    pub fn of_message(message: &str) -> Self {
        let text = lf(message);
        let text = text.trim_end_matches('\n');
        match text.split_once('\n') {
            None => Draft { summary: text.to_string(), description: String::new() },
            Some((s, rest)) => {
                let mut d = rest;
                while let Some((first, more)) = d.split_once('\n') {
                    if !first.trim().is_empty() {
                        break;
                    }
                    d = more;
                }
                Draft { summary: s.to_string(), description: d.to_string() }
            }
        }
    }

    /// The commit or stash message.
    pub fn message(&self) -> String {
        if self.description.trim().is_empty() { self.summary.clone() } else { format!("{}\n\n{}", self.summary, self.description) }
    }
}

/// What the tab keeps of its WIP panel.
/// What the box holds at a stop: git's message for it (`original`), and the user's edits.
#[derive(Debug, Clone, PartialEq)]
pub struct StopBox {
    pub key: String,
    pub original: Draft,
    pub text: Draft,
}

impl StopBox {
    /// What Continue sends: the box's message, only when the user edited it (`continueMessage`).
    pub fn edited_message(&self) -> Option<String> {
        (self.text != self.original && !self.text.summary.trim().is_empty()).then(|| self.text.message())
    }
}

/// `CONFLICT_LABEL`: a conflicted row's short status.
pub fn conflict_label(kind: &str) -> &'static str {
    match kind {
        "bothAdded" => "added in both",
        "bothDeleted" => "deleted in both",
        "addedByUs" => "added in current",
        "addedByThem" => "added in incoming",
        "deletedByUs" => "deleted in current",
        "deletedByThem" => "deleted in incoming",
        _ => "changed in both",
    }
}

/// A stopped merge's draft, set aside: `head` the merge, `applied` what the merge wrote to the box.
#[derive(Debug, Clone, PartialEq)]
pub struct MergeAside {
    pub head: String,
    pub draft: Draft,
    pub applied: Draft,
}

/// MERGE_MSG goes below what's typed: the summary stays, and an empty one takes its first line.
pub fn with_merge_message(d: &Draft, merge_msg: &str) -> Draft {
    let text: String = lf(merge_msg).split('\n').filter(|l| !l.starts_with('#')).collect::<Vec<_>>().join("\n");
    let m = Draft::of_message(&text);
    let join = |a: &str, b: &str| [a, b].into_iter().filter(|p| !p.trim().is_empty()).collect::<Vec<_>>().join("\n\n");
    if d.summary.trim().is_empty() {
        Draft { summary: m.summary, description: join(&d.description, &m.description) }
    } else {
        Draft { summary: d.summary.clone(), description: join(&d.description, &m.message()) }
    }
}

pub struct WipUi {
    pub filter_open: bool,
    pub filter: Entity<TextInput>,
    pub summary: Entity<TextInput>,
    pub description: Entity<TextInput>,
    /// Amend: HEAD's message in the box, the draft put aside. Ends with a HEAD move.
    pub amend: Option<(Draft, String)>,
    /// A rebase, cherry-pick or revert stopped: its message in the box, the draft put aside.
    pub stop: Option<StopBox>,
    /// A commit for the worktree is queued or running: staging and discards wait.
    pub committing: bool,
    /// Staging writes sent and not answered: the commit button waits for them.
    pub staging: u32,
    pub discarding: bool,
    /// `None`: not asked yet. `Some(None)`: git has no identity.
    pub identity: Option<Option<(String, String)>>,
    pub all_files: bool,
    /// When Commit's write started: past 30 s the box says so.
    pub committing_since: Option<std::time::Instant>,
    /// What Abort counted when it armed: a different count disarms it.
    pub abort_armed_at: Option<String>,
}

impl WipUi {
    pub fn new(id: u32, cx: &mut Context<GitVolt>) -> Self {
        let filter = cx.new(|cx| TextInput::new(cx).placeholder("Filter files").claim_escape());
        cx.subscribe(&filter, move |this, _, e: &InputEvent, cx| match e {
            InputEvent::Changed => this.wip_filter_changed(id, cx),
            InputEvent::Escape => this.toggle_wip_filter(id, false, cx),
            _ => {}
        })
        .detach();
        let summary = cx.new(|cx| TextInput::new(cx).placeholder("Summary").placeholder_color(theme::wip_input_placeholder()).claim_escape().claim_arrows());
        cx.subscribe(&summary, move |this, _, e: &InputEvent, cx| this.commit_field_event(id, true, e, cx)).detach();
        let description = cx.new(|cx| TextInput::multiline(cx).placeholder("Description").placeholder_color(theme::wip_input_placeholder()).claim_escape().claim_arrows());
        cx.subscribe(&description, move |this, _, e: &InputEvent, cx| this.commit_field_event(id, false, e, cx)).detach();
        Self { filter_open: false, filter, summary, description, amend: None, stop: None, committing: false, staging: 0, discarding: false, identity: None, all_files: false, committing_since: None, abort_armed_at: None }
    }
}

impl Tab {
    /// The worktree of the selected WIP row.
    pub fn wip_worktree(&self) -> Option<&str> {
        if self.compare.is_some() {
            return None;
        }
        let row = self.graph.as_ref()?.rows.get(self.selected?)?;
        (row.kind == gitvolt_graph::NodeKind::Wip).then_some(row.worktree.as_deref()).flatten()
    }

    pub fn is_wip_selected(&self) -> bool {
        self.wip_worktree().is_some()
    }

    /// The WIP row's HEAD commit: its parent.
    pub fn wip_head(&self, worktree: &str) -> Option<String> {
        let row = self.graph.as_ref()?.rows.iter().find(|r| r.worktree.as_deref() == Some(worktree))?;
        row.parents.first().map(|p| p.to_string())
    }

    pub fn wip_ctx(&self) -> Option<WriteCtx> {
        Some(WriteCtx { repo: self.id, worktree: self.wip_worktree()?.to_string() })
    }

}

/// The open diff's place in its WIP section before a whole-file write.
#[derive(Debug, Clone, PartialEq)]
pub struct AdvanceFrom {
    pub staged: bool,
    pub path: String,
    pub order: Vec<String>,
}

/// Where the view goes after a whole-file write took the open file out of its section: the file now at
/// its place, else the one above; with the section empty, a stage or unstage follows the file (`None`),
/// a discard takes the other section's first file, or nothing (`Some(None)`).
pub fn advance_target(from: &AdvanceFrom, files: &Files) -> Option<Option<FileChange>> {
    let which = if from.staged { Which::Staged } else { Which::Unstaged };
    let mine = files.section_files(which);
    if mine.iter().any(|f| f.path.as_ref() == from.path) {
        return None;
    }
    let listed = |p: &String| mine.iter().find(|f| f.path.as_ref() == p);
    let at = from.order.iter().position(|p| *p == from.path)?;
    let next = from.order[at + 1..].iter().find_map(listed).or_else(|| from.order[..at].iter().rev().find_map(listed));
    if let Some(f) = next {
        return Some(Some(f.clone()));
    }
    let other = files.section_files(if from.staged { Which::Unstaged } else { Which::Staged });
    if other.iter().any(|f| f.path.as_ref() == from.path) {
        return None;
    }
    Some(other.first().cloned())
}

/// Where the open file went: its section's copy with fresh sides, else the other section's, else
/// nowhere (discarded).
pub fn follow_target(open: &FileChange, files: &Files) -> Option<FileChange> {
    let Origin::Wip { staged, .. } = &open.origin else { return Some(open.clone()) };
    let find = |w: Which| files.section_files(w).into_iter().find(|f| f.path == open.path);
    let (mine, other) = if *staged { (Which::Staged, Which::Unstaged) } else { (Which::Unstaged, Which::Staged) };
    find(mine).or_else(|| find(other))
}

impl GitVolt {
    fn wip_tab_mut(&mut self, repo: u32) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == repo)
    }

    /// Reads the panel for a WIP row: the worktree's unstaged changes, then its staged ones.
    pub fn read_wip(&mut self, repo: u32, worktree: String, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let api = self.api.clone();
        let tree = worktree.clone();
        let read = self.rt.spawn(async move {
            let unstaged = api.dispatch(Request::FileList { repo, spec: gitvolt_core::diff::DiffSpec::Wip { worktree: worktree.clone(), staged: false } }).await?;
            let staged = api.dispatch(Request::FileList { repo, spec: gitvolt_core::diff::DiffSpec::Wip { worktree: worktree.clone(), staged: true } }).await?;
            let staging = api.dispatch(Request::StagingState { repo, worktree: worktree.clone() }).await.ok();
            let identity = api.dispatch(Request::CommitIdentity { repo, worktree }).await.ok();
            Ok::<_, gitvolt_core::error::GbError>((unstaged, staged, staging, identity))
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok((unstaged, staged, staging, identity))) = read.await else { return };
            this.update(cx, |this, cx| {
                if let Some(state) = staging.and_then(|s| serde_json::from_value(s).ok()) {
                    this.ops.staging.insert((repo, tree.clone()), state);
                }
                if let Some(id) = identity {
                    let who = id["name"].as_str().zip(id["email"].as_str()).map(|(n, e)| (n.to_string(), e.to_string()));
                    if let Some(tab) = this.wip_tab_mut(repo) {
                        tab.wip.identity = Some(who);
                    }
                }
                this.put_wip(repo, tree.clone(), &unstaged, &staged, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Fresh lists for `worktree`: the panel shows them if it's on that row, and the open file follows.
    pub fn put_wip(&mut self, repo: u32, worktree: String, unstaged: &Value, staged: &Value, cx: &mut Context<Self>) {
        let closed = self.wip_closed;
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) else { return };
        if tab.wip_worktree() != Some(worktree.as_str()) {
            return;
        }
        let open_conflict = tab.diff.as_ref().is_some_and(|d| crate::merge::is_conflict(&d.file));
        match tab.files.as_mut().filter(|f| f.wip.is_some()) {
            Some(files) => files.set_wip_lists(unstaged, staged),
            None => {
                let mut files = Files::wip(worktree, unstaged, staged);
                files.by_status = tab.sort_by_status;
                if let Some(w) = files.wip.as_mut() {
                    w.closed = closed;
                }
                files.rebuild();
                tab.files = Some(files);
                if tab.wip.all_files {
                    self.read_worktree_paths(repo, cx);
                }
            }
        }
        // Resolved elsewhere (a row's Take, `git add`, Abort): the tool mustn't keep offering its buttons.
        if open_conflict {
            self.merge_left_resolved(repo, None, cx);
        }
        cx.notify();
    }

    /// The panel's worktree lists, read again: a watched change.
    pub fn reread_wip(&mut self, repo: u32, cx: &mut Context<Self>) {
        let Some(worktree) = self.tabs.iter().find(|t| t.id == repo).and_then(|t| t.wip_worktree()).map(str::to_string) else { return };
        self.read_wip(repo, worktree, cx);
    }

    /// The open file's place, for a whole-file write.
    fn advance_from(&self, ctx: &WriteCtx) -> Option<AdvanceFrom> {
        let tab = self.tabs.iter().find(|t| t.id == ctx.repo)?;
        let diff = tab.diff.as_ref()?;
        let Origin::Wip { worktree, staged } = &diff.file.origin else { return None };
        if *worktree != ctx.worktree {
            return None;
        }
        let files = tab.files.as_ref()?;
        let order: Vec<String> = files.section_files(if *staged { Which::Staged } else { Which::Unstaged }).iter().map(|f| f.path.to_string()).collect();
        order.contains(&diff.file.path.to_string()).then(|| AdvanceFrom { staged: *staged, path: diff.file.path.to_string(), order })
    }

    /// Re-targets the open diff after a write's fresh lists.
    pub fn follow_open_file(&mut self, repo: u32, from: Option<AdvanceFrom>, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) else { return };
        let (Some(diff), Some(files)) = (tab.diff.as_ref(), tab.files.as_ref()) else { return };
        // A conflicted file stays until it's resolved (`merge_left_resolved`).
        if !matches!(diff.file.origin, Origin::Wip { .. }) || diff.file.status == UNCHANGED || diff.file.status == 'U' {
            return;
        }
        let open = diff.file.clone();
        let target = match from.filter(|f| f.path == open.path.as_ref()).and_then(|f| advance_target(&f, files)) {
            Some(next) => next,
            None => follow_target(&open, files),
        };
        match target {
            Some(file) => self.reopen_file(repo, file, cx),
            None => {
                let tab = self.tabs.iter_mut().find(|t| t.id == repo);
                if let Some(tab) = tab {
                    tab.diff = None;
                    if tab.keys == KeyArea::Diff {
                        tab.keys = KeyArea::Files;
                    }
                }
                cx.notify();
            }
        }
    }

    /// The open diff shows `file`: the same file stays where it was, with its fresh sides.
    fn reopen_file(&mut self, repo: u32, file: FileChange, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) else { return };
        // The same file stays on screen, with its fresh sides when they come.
        match tab.diff.as_mut().filter(|d| d.file.path == file.path && d.file.origin == file.origin) {
            Some(d) => {
                d.file = file;
                d.generation += 1;
                d.keep_scroll();
            }
            None => tab.diff = Some(DiffState::carrying(file, tab.diff.as_ref())),
        }
        let at = tab.files.as_ref().and_then(|f| tab.diff.as_ref().and_then(|d| f.changes.iter().position(|c| c.path == d.file.path && c.origin == d.file.origin)));
        if let (Some(i), Some(files)) = (at, tab.files.as_mut()) {
            files.cursor = Some(Cursor::File(i));
        }
        self.read_contents(false, cx);
        cx.notify();
    }

    fn staging_gate(&self, ctx: &WriteCtx) -> bool {
        self.tabs.iter().find(|t| t.id == ctx.repo).is_some_and(|t| t.wip.committing)
    }

    /// One staging or discard write: refused while a commit is queued; the commit button holds off until
    /// it's answered; then the open diff follows. `advance`: a whole-file one.
    pub fn staging_write(&mut self, ctx: &WriteCtx, origin: ArmOrigin, advance: bool, send: impl Fn() -> Value + 'static, cx: &mut Context<Self>) {
        if self.staging_gate(ctx) {
            return;
        }
        let from = if advance { self.advance_from(ctx) } else { None };
        let repo = ctx.repo;
        if let Some(tab) = self.wip_tab_mut(repo) {
            tab.wip.staging += 1;
        }
        let w = Write::new(ctx, move |_| send())
            .origin(origin)
            .on_success(move |this, _, cx| this.follow_open_file(repo, from.clone(), cx))
            .on_end(move |this, _, cx| {
                if let Some(tab) = this.wip_tab_mut(repo) {
                    tab.wip.staging = tab.wip.staging.saturating_sub(1);
                }
                cx.notify();
            });
        self.write(w, cx);
    }

    pub fn stage_paths(&mut self, ctx: &WriteCtx, paths: Vec<String>, cx: &mut Context<Self>) {
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        self.staging_write(ctx, ArmOrigin::None, true, move || call("stage", json!({ "repo": repo, "worktree": worktree, "paths": paths })), cx);
    }

    /// Renames unstage with both paths.
    pub fn unstage_files(&mut self, ctx: &WriteCtx, files: Vec<FileChange>, cx: &mut Context<Self>) {
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let paths: Vec<String> = files.iter().map(|f| f.path.to_string()).collect();
        let old: Vec<String> = files.iter().filter_map(|f| f.old_path.clone()).collect();
        self.staging_write(ctx, ArmOrigin::None, true, move || call("unstage", json!({ "repo": repo, "worktree": worktree, "paths": paths, "oldPaths": old })), cx);
    }

    pub fn stage_all(&mut self, ctx: &WriteCtx, cx: &mut Context<Self>) {
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        self.staging_write(ctx, ArmOrigin::None, true, move || call("stageAll", json!({ "repo": repo, "worktree": worktree })), cx);
    }

    /// While files are conflicted, only the other paths: `git add -A` would mark them resolved.
    pub fn stage_all_guarded(&mut self, ctx: &WriteCtx, cx: &mut Context<Self>) {
        let Some(files) = self.tabs.iter().find(|t| t.id == ctx.repo).and_then(|t| t.files.as_ref()) else { return };
        let unstaged = files.section_files(Which::Unstaged);
        if files.counts().2 > 0 {
            let paths = paths_of(&unstaged);
            self.stage_paths(ctx, paths, cx);
        } else {
            self.stage_all(ctx, cx);
        }
    }

    pub fn unstage_all(&mut self, ctx: &WriteCtx, cx: &mut Context<Self>) {
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        self.staging_write(ctx, ArmOrigin::None, true, move || call("unstageAll", json!({ "repo": repo, "worktree": worktree })), cx);
    }

    pub fn discard_paths(&mut self, ctx: &WriteCtx, paths: Vec<String>, cx: &mut Context<Self>) {
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        self.staging_write(ctx, ArmOrigin::None, true, move || call("discard", json!({ "repo": repo, "worktree": worktree, "scope": { "kind": "paths", "paths": paths } })), cx);
    }

    pub fn discard_unstaged(&mut self, ctx: &WriteCtx, cx: &mut Context<Self>) {
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        self.staging_write(ctx, ArmOrigin::None, true, move || call("discard", json!({ "repo": repo, "worktree": worktree, "scope": { "kind": "unstaged" } })), cx);
    }

    /// A row's Discard: arms where it started, then discards those files' changes (undoable).
    pub fn discard_files(&mut self, ctx: WriteCtx, files: Vec<FileChange>, name: String, origin: ArmOrigin, window: &mut Window, cx: &mut Context<Self>) {
        let n = files.len();
        let mut req = ArmRequest::danger(discard_arm(n, &name), format!("Discard {name}?"), "Discard");
        req.body = "Its changes are removed. You can undo this.".into();
        let this = cx.weak_entity();
        arm::request(
            req,
            origin,
            move |a, _, cx| {
                if a.ok {
                    let (ctx, paths) = (ctx.clone(), paths_of(&files));
                    this.update(cx, |v, cx| v.discard_paths(&ctx, paths, cx)).ok();
                }
            },
            window,
            cx,
        );
    }

    /// Discard all: staged, unstaged and untracked, of what the panel shows. One at a time per worktree.
    pub fn discard_all(&mut self, ctx: WriteCtx, origin: ArmOrigin, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == ctx.repo) else { return };
        if tab.wip.discarding {
            return;
        }
        let (count, confirmed) = tab.files.as_ref().map_or((0, Vec::new()), Files::discard_scope);
        let arm_label = if count > 0 { format!("Click again to discard {}", files_word(count)) } else { "Click again to discard every change".to_string() };
        let mut req = ArmRequest::danger(arm_label, "Discard all changes?", "Discard all").icon("trash-2");
        req.body = "Staged, unstaged and untracked changes are removed. You can undo this.".into();
        let this = cx.weak_entity();
        arm::request(
            req,
            origin,
            move |a, _, cx| {
                if !a.ok {
                    return;
                }
                let (ctx, confirmed) = (ctx.clone(), confirmed.clone());
                this.update(cx, |v, cx| v.discard_all_now(ctx, confirmed, cx)).ok();
            },
            window,
            cx,
        );
    }

    fn discard_all_now(&mut self, ctx: WriteCtx, confirmed: Vec<String>, cx: &mut Context<Self>) {
        let repo = ctx.repo;
        let Some(tab) = self.wip_tab_mut(repo) else { return };
        if tab.wip.discarding || tab.wip.committing {
            return;
        }
        tab.wip.discarding = true;
        let worktree = ctx.worktree.clone();
        let w = Write::new(&ctx, move |_| {
            let scope = if confirmed.is_empty() { json!({ "kind": "all" }) } else { json!({ "kind": "all", "confirmed": confirmed }) };
            call("discard", json!({ "repo": repo, "worktree": worktree, "scope": scope }))
        })
        .on_success(move |this, _, cx| this.follow_open_file(repo, None, cx))
        .on_end(move |this, _, cx| {
            if let Some(tab) = this.wip_tab_mut(repo) {
                tab.wip.discarding = false;
            }
            cx.notify();
        });
        self.write(w, cx);
    }

    pub fn staging_undo(&mut self, ctx: &WriteCtx, redo: bool, cx: &mut Context<Self>) {
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let method = if redo { "stagingRedo" } else { "stagingUndo" };
        self.staging_write(ctx, ArmOrigin::None, false, move || call(method, json!({ "repo": repo, "worktree": worktree })), cx);
    }

    pub fn toggle_wip_filter(&mut self, repo: u32, open: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.wip_tab_mut(repo) else { return };
        tab.wip.filter_open = open;
        let focus = tab.wip.filter.read(cx).focus.clone();
        tab.wip.filter.update(cx, |i, cx| i.set_text("", cx));
        if let Some(files) = tab.files.as_mut() {
            files.filter.clear();
            files.rebuild();
        }
        if open {
            self.focus_next_frame = Some(focus);
        } else {
            self.focus_next_frame = Some(self.focus.clone());
        }
        cx.notify();
    }

    fn wip_filter_changed(&mut self, repo: u32, cx: &mut Context<Self>) {
        let Some(tab) = self.wip_tab_mut(repo) else { return };
        let text = tab.wip.filter.read(cx).text().to_string();
        if let Some(files) = tab.files.as_mut() {
            files.filter = text;
            files.rebuild();
        }
        cx.notify();
    }

    pub fn toggle_wip_section(&mut self, which: Which, cx: &mut Context<Self>) {
        self.wip_closed[which.index()] = !self.wip_closed[which.index()];
        let closed = self.wip_closed;
        if let Some(w) = self.tabs.get_mut(self.active).and_then(|t| t.files.as_mut()).and_then(|f| f.wip.as_mut()) {
            w.closed = closed;
        }
        self.save_layout();
        cx.notify();
    }

    /// "View all files" of the WIP: the worktree's tracked files around its changes.
    pub fn toggle_wip_all_files(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        tab.wip.all_files = !tab.wip.all_files;
        let (repo, on) = (tab.id, tab.wip.all_files);
        if on {
            self.read_worktree_paths(repo, cx);
        } else if let Some(files) = self.tabs[self.active].files.as_mut() {
            if let Some(w) = files.wip.as_mut() {
                w.all_paths = None;
            }
            files.rebuild();
        }
        cx.notify();
    }

    fn read_worktree_paths(&mut self, repo: u32, cx: &mut Context<Self>) {
        let Some(worktree) = self.tabs.iter().find(|t| t.id == repo).and_then(|t| t.wip_worktree()).map(str::to_string) else { return };
        if cfg!(test) {
            return;
        }
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(Request::WorktreeFiles { repo, worktree }).await });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                let Some(files) = this.wip_tab_mut(repo).and_then(|t| t.files.as_mut()) else { return };
                match result {
                    Ok(Ok(v)) => {
                        if let Some(w) = files.wip.as_mut() {
                            w.all_paths = Some(v.as_array().map_or(&[][..], Vec::as_slice).iter().filter_map(|p| p.as_str().map(str::to_string)).collect());
                        }
                        files.rebuild();
                    }
                    Ok(Err(e)) => this.show_toast(format!("Couldn't list all files: {}", e.message), cx),
                    Err(e) => this.show_toast(format!("Couldn't list all files: {e}"), cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ---- The draft and the commit ----

    /// The tab's draft boxes follow the graph's WIP rows: one per worktree.
    pub fn sync_row_drafts(&mut self, repo: u32, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == repo) else { return };
        let worktrees: Vec<String> = tab.graph.as_ref().map_or(Vec::new(), |g| g.rows.iter().filter_map(|r| r.worktree.clone()).collect());
        let missing: Vec<String> = worktrees.into_iter().filter(|w| !tab.row_drafts.contains_key(w)).collect();
        for worktree in missing {
            let text = self.drafts.get(&worktree).map_or(String::new(), |d| d.summary.clone());
            let input = cx.new(|cx| {
                let mut i = TextInput::new(cx).placeholder("// WIP").placeholder_color(theme::wip_input_placeholder()).claim_escape();
                i.set_text(&text, cx);
                i
            });
            let key = worktree.clone();
            cx.subscribe(&input, move |this, input, e: &InputEvent, cx| match e {
                InputEvent::Changed => {
                    let text = input.read(cx).text().to_string();
                    this.row_draft_changed(repo, &key, text, cx);
                }
                InputEvent::Escape | InputEvent::Submit { .. } => {
                    this.flush_drafts();
                    this.focus_next_frame = Some(this.focus.clone());
                    cx.notify();
                }
                InputEvent::Blurred => this.flush_drafts(),
                _ => {}
            })
            .detach();
            if let Some(tab) = self.wip_tab_mut(repo) {
                tab.row_drafts.insert(worktree, input);
            }
        }
    }

    fn row_draft_changed(&mut self, repo: u32, worktree: &str, text: String, cx: &mut Context<Self>) {
        let d = self.drafts.entry(worktree.to_string()).or_default();
        d.summary = text.clone();
        self.touch_drafts(cx);
        // The commit box shows it too, unless it's HEAD's message there.
        if let Some(tab) = self.tabs.iter().find(|t| t.id == repo)
            && tab.wip_worktree() == Some(worktree)
            && tab.wip.amend.is_none()
        {
            tab.wip.summary.clone().update(cx, |i, cx| i.set_text(&text, cx));
        }
        cx.notify();
    }

    /// Puts a worktree's draft (or what's being amended) in the commit box.
    pub fn show_draft(&mut self, repo: u32, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == repo) else { return };
        let Some(worktree) = tab.wip_worktree() else { return };
        let op_box = tab.box_state().op_box();
        let d = match (&tab.wip.amend, tab.wip.stop.as_ref().filter(|_| op_box)) {
            (Some((text, _)), _) => text.clone(),
            (None, Some(stop)) => stop.text.clone(),
            (None, None) => self.drafts.get(worktree).cloned().unwrap_or_default(),
        };
        let (summary, description) = (tab.wip.summary.clone(), tab.wip.description.clone());
        summary.update(cx, |i, cx| i.set_text(&d.summary, cx));
        description.update(cx, |i, cx| i.set_text(&d.description, cx));
    }

    fn commit_field_event(&mut self, repo: u32, is_summary: bool, e: &InputEvent, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == repo) else { return };
        let (summary, description) = (tab.wip.summary.clone(), tab.wip.description.clone());
        match e {
            InputEvent::Changed => {
                let d = Draft { summary: summary.read(cx).text().to_string(), description: description.read(cx).text().to_string() };
                let Some(worktree) = tab.wip_worktree().map(str::to_string) else { return };
                let row = tab.row_drafts.get(&worktree).cloned();
                let amending = tab.wip.amend.is_some();
                let stopped = tab.wip.stop.is_some() && tab.box_state().op_box();
                if amending || stopped {
                    if let Some(tab) = self.wip_tab_mut(repo) {
                        match (tab.wip.amend.as_mut(), tab.wip.stop.as_mut()) {
                            (Some(a), _) => a.0 = d,
                            (None, Some(s)) => s.text = d,
                            _ => {}
                        }
                    }
                } else {
                    let text = d.summary.clone();
                    self.drafts.insert(worktree, d);
                    self.touch_drafts(cx);
                    if let Some(row) = row {
                        row.update(cx, |i, cx| i.set_text(&text, cx));
                    }
                }
                cx.notify();
            }
            // Enter or Down in the summary: the description, caret at its start.
            InputEvent::Submit { ctrl: false, .. } if is_summary => self.focus_description(repo, cx),
            InputEvent::Arrow { down: true, .. } if is_summary => self.focus_description(repo, cx),
            // Up on the description's first line: the summary's end.
            InputEvent::Arrow { down: false, .. } if !is_summary => {
                let at = summary.read(cx).text().len();
                let focus = summary.read(cx).focus.clone();
                summary.update(cx, |i, cx| {
                    i.buffer.move_to(at);
                    cx.notify();
                });
                self.focus_next_frame = Some(focus);
                cx.notify();
            }
            InputEvent::Submit { ctrl: true, .. } => self.submit_commit(cx),
            InputEvent::Escape => {
                self.flush_drafts();
                self.focus_next_frame = Some(self.focus.clone());
                cx.notify();
            }
            InputEvent::Blurred => self.flush_drafts(),
            _ => {}
        }
    }

    fn focus_description(&mut self, repo: u32, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == repo) else { return };
        let description = tab.wip.description.clone();
        let focus = description.read(cx).focus.clone();
        description.update(cx, |i, cx| {
            i.buffer.move_to(0);
            cx.notify();
        });
        self.focus_next_frame = Some(focus);
        cx.notify();
    }

    /// The keyboard goes in the commit box's summary.
    pub fn focus_commit_box(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get(self.active) {
            self.focus_next_frame = Some(tab.wip.summary.read(cx).focus.clone());
            cx.notify();
        }
    }

    /// Drafts are written debounced, and at once on a blur.
    fn touch_drafts(&mut self, cx: &mut Context<Self>) {
        self.drafts_dirty += 1;
        let at = self.drafts_dirty;
        let timer = cx.background_executor().timer(std::time::Duration::from_millis(300));
        cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |this, _| {
                if this.drafts_dirty == at {
                    this.flush_drafts();
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn flush_drafts(&mut self) {
        if self.drafts_saved == self.drafts_dirty {
            return;
        }
        self.drafts_saved = self.drafts_dirty;
        let Some(path) = drafts_file() else { return };
        let mut all = drafts_json(&self.drafts);
        all[ASIDES_KEY] = asides_json(&self.merge_asides);
        let text = all.to_string();
        self.rt.spawn(async move {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).ok();
            }
            std::fs::write(path, text).ok();
        });
    }

    pub fn load_drafts(&mut self) {
        let Some(v) = drafts_file().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { return };
        self.drafts = drafts_of(&v);
        self.merge_asides = asides_of(&v);
    }

    /// Commit, from its button, Ctrl+Enter or the box's fields.
    pub fn submit_commit(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(ctx) = tab.wip_ctx() else { return };
        let view = commit_view_of(tab, cx);
        if view.reason.is_some() || tab.wip.committing || self.integrating(ctx.repo) {
            return;
        }
        let state = tab.box_state();
        if let Some(op) = &state.box_op {
            let message = tab.wip.stop.as_ref().and_then(StopBox::edited_message);
            return self.operation_control(op.kind, "continue", message, ArmOrigin::None, cx);
        }
        let (edit_staged, at_stop) = (state.edit_staged, state.edit_moved || state.edit_staged);
        let d = Draft { summary: tab.wip.summary.read(cx).text().to_string(), description: tab.wip.description.read(cx).text().to_string() };
        let amend = tab.wip.amend.is_some();
        let head = tab.wip_head(&ctx.worktree);
        let stage_all = view.stage_all;
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let Some(tab) = self.wip_tab_mut(repo) else { return };
        tab.wip.committing = true;
        tab.wip.committing_since = Some(std::time::Instant::now());
        self.remind_still_working(cx);
        let Some(tab) = self.wip_tab_mut(repo) else { return };
        tab.wip.summary.update(cx, |i, cx| i.set_disabled(true, cx));
        tab.wip.description.update(cx, |i, cx| i.set_disabled(true, cx));
        let done_worktree = worktree.clone();
        let w = Write::new(&ctx, move |_| call("commit", json!({ "repo": repo, "worktree": worktree, "summary": d.summary, "description": d.description, "amend": amend, "stageAll": stage_all, "expect": { "head": head, "refs": {} } })))
            .on_success(move |this, out, cx| {
                let oid = out["oid"].as_str().unwrap_or_default().to_string();
                if amend {
                    if let Some(tab) = this.wip_tab_mut(repo) {
                        tab.wip.amend = None;
                    }
                } else if edit_staged {
                    // A piece committed at the stop: the box empties for the next one.
                    if let Some(stop) = this.wip_tab_mut(repo).and_then(|t| t.wip.stop.as_mut()) {
                        stop.text = Draft::default();
                    }
                } else {
                    this.drafts.remove(&done_worktree);
                    this.touch_drafts(cx);
                    this.flush_drafts();
                    if let Some(row) = this.wip_tab_mut(repo).and_then(|t| t.row_drafts.get(&done_worktree).cloned()) {
                        row.update(cx, |i, cx| i.set_text("", cx));
                    }
                }
                this.show_draft(repo, cx);
                // The new commit, selected once the graph that holds it arrives; at an Edit stop the WIP stays
                // selected: the next piece and Continue are here.
                if !at_stop && let Some(tab) = this.wip_tab_mut(repo) {
                    tab.select_after_load = Some(oid);
                }
                this.reload(repo, cx);
            })
            .on_end(move |this, _, cx| {
                if let Some(tab) = this.wip_tab_mut(repo) {
                    tab.wip.committing = false;
                    tab.wip.committing_since = None;
                    let (summary, description) = (tab.wip.summary.clone(), tab.wip.description.clone());
                    let focus = summary.read(cx).focus.clone();
                    summary.update(cx, |i, cx| i.set_disabled(false, cx));
                    description.update(cx, |i, cx| i.set_disabled(false, cx));
                    this.focus_next_frame = Some(focus);
                }
                cx.notify();
            });
        self.write(w, cx);
    }

    /// Ctrl+Enter outside the message: commits, or with nothing it can do yet, puts the keyboard in the
    /// message and says why.
    pub fn commit_key(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        if tab.wip.committing {
            return;
        }
        let view = commit_view_of(tab, cx);
        match view.reason {
            None => self.submit_commit(cx),
            Some(why) => {
                self.focus_commit_box(cx);
                self.show_toast(why, cx);
            }
        }
    }

    /// Amend: HEAD's message in the box, the draft put aside.
    pub fn toggle_amend(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let repo = tab.id;
        if tab.wip.amend.take().is_some() {
            return self.show_draft(repo, cx);
        }
        let Some(worktree) = tab.wip_worktree().map(str::to_string) else { return };
        let Some(head) = tab.wip_head(&worktree) else { return };
        if cfg!(test) {
            return;
        }
        let api = self.api.clone();
        let id = head.clone();
        let read = self.rt.spawn(async move { api.dispatch(Request::CommitMessage { repo, id }).await });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(m)) = read.await else { return };
            this.update(cx, |this, cx| {
                let text = format!("{}\n{}", m["summary"].as_str().unwrap_or_default(), m["body"].as_str().unwrap_or_default());
                this.start_amend(repo, Draft::of_message(&text), head, cx);
            })
            .ok();
        })
        .detach();
    }

    pub fn start_amend(&mut self, repo: u32, text: Draft, head: String, cx: &mut Context<Self>) {
        let Some(tab) = self.wip_tab_mut(repo) else { return };
        if tab.wip.amend.is_none() {
            tab.wip.amend = Some((text, head));
        }
        self.show_draft(repo, cx);
        cx.notify();
    }

    /// A HEAD move ends the amend; the draft shows again.
    pub fn check_amend(&mut self, repo: u32, cx: &mut Context<Self>) {
        self.sync_merge_drafts(repo, cx);
        let Some(tab) = self.tabs.iter().find(|t| t.id == repo) else { return };
        let Some(worktree) = tab.wip_worktree() else { return };
        let owns = tab.box_state().owns;
        let moved = tab.wip.amend.as_ref().is_some_and(|(_, head)| owns || tab.wip_head(worktree).as_deref() != Some(head));
        if moved {
            if let Some(tab) = self.wip_tab_mut(repo) {
                tab.wip.amend = None;
            }
            self.show_draft(repo, cx);
        }
        let Some(tab) = self.tabs.iter().find(|t| t.id == repo) else { return };
        // Each stop fills the box with git's message once; edits stay until the next stop. The operation over,
        // the draft shows again.
        let operation = tab.operation();
        let stop = operation.clone().filter(|o| !o.is_merge() && o.primary.is_some());
        let key = stop.as_ref().map(|o| o.stop.clone());
        let changes = match &key {
            Some(k) => tab.wip.stop.as_ref().map(|s| &s.key) != Some(k),
            None => operation.is_none() && tab.wip.stop.is_some(),
        };
        if changes {
            let needs_read = stop.as_ref().and_then(|o| o.stopped_at.clone().filter(|_| o.message.trim().is_empty()));
            let filled = stop.map(|o| {
                let text = Draft::of_message(&o.message);
                StopBox { key: o.stop, original: text.clone(), text }
            });
            if let Some(tab) = self.wip_tab_mut(repo) {
                tab.wip.stop = filled;
            }
            self.show_draft(repo, cx);
            // No message of git's: the stopped commit's own.
            if let Some(id) = needs_read
                && !cfg!(test)
            {
                let api = self.api.clone();
                let read = self.rt.spawn(async move { api.dispatch(Request::CommitMessage { repo, id }).await });
                cx.spawn(async move |this, cx| {
                    let Ok(Ok(m)) = read.await else { return };
                    this.update(cx, |this, cx| {
                        let text = Draft::of_message(&format!("{}\n{}", m["summary"].as_str().unwrap_or_default(), m["body"].as_str().unwrap_or_default()));
                        if let Some(stop) = this.wip_tab_mut(repo).and_then(|t| t.wip.stop.as_mut()) {
                            stop.original = text.clone();
                            stop.text = text;
                        }
                        this.show_draft(repo, cx);
                    })
                    .ok();
                })
                .detach();
            }
        }
    }

    /// The worktree's draft is now `d`, wherever it shows.
    fn set_draft(&mut self, repo: u32, worktree: &str, d: Draft, cx: &mut Context<Self>) {
        let row = self.tabs.iter().find(|t| t.id == repo).and_then(|t| t.row_drafts.get(worktree).cloned());
        if let Some(row) = row {
            let text = d.summary.clone();
            row.update(cx, |i, cx| i.set_text(&text, cx));
        }
        self.drafts.insert(worktree.to_string(), d);
        self.touch_drafts(cx);
        self.show_draft(repo, cx);
    }

    /// Once per stopped merge, its message joins the draft, never replacing what's typed. A merge that
    /// ended without Abort (a commit, a terminal): an untouched message gives way to the old draft.
    fn sync_merge_drafts(&mut self, repo: u32, cx: &mut Context<Self>) {
        let Some(g) = self.tabs.iter().find(|t| t.id == repo).and_then(|t| t.graph.clone()) else { return };
        for w in &g.worktrees {
            let merge = match g.in_progress.get(&w.path) {
                Some(gitvolt_graph::InProgress::Merge { merge_head, message, .. }) => Some((merge_head.clone(), message.clone())),
                _ => None,
            };
            match merge {
                Some((head, message)) => {
                    if self.merge_asides.get(&w.path).is_some_and(|a| a.head == head) {
                        continue;
                    }
                    let draft = self.drafts.get(&w.path).cloned().unwrap_or_default();
                    let applied = with_merge_message(&draft, &message);
                    self.merge_asides.insert(w.path.clone(), MergeAside { head, draft, applied: applied.clone() });
                    self.set_draft(repo, &w.path, applied, cx);
                }
                None if self.merge_aborting.contains(&w.path) => {}
                None => {
                    if let Some(a) = self.merge_asides.remove(&w.path) {
                        self.drafts_dirty += 1;
                        if self.drafts.get(&w.path).cloned().unwrap_or_default() == a.applied {
                            self.set_draft(repo, &w.path, a.draft, cx);
                        }
                    }
                }
            }
        }
    }

    /// Abort puts the draft back as it was; if the merge message has since been edited, it asks first.
    pub fn restore_draft_after_abort(&mut self, repo: u32, worktree: String, cx: &mut Context<Self>) {
        let Some(a) = self.merge_asides.remove(&worktree) else { return };
        self.drafts_dirty += 1;
        let current = self.drafts.get(&worktree).cloned().unwrap_or_default();
        if current == a.applied || current == Draft::default() {
            return self.set_draft(repo, &worktree, a.draft, cx);
        }
        let mut req = ArmRequest::new(arm::Tone::Positive, "Click again to restore your earlier message", "Restore your earlier message?", "Restore");
        req.body = "You edited the merge message. Replace it with the commit message you had before the merge?".into();
        let this = cx.weak_entity();
        self.with_window(cx, move |window, cx| {
            arm::request(
                req,
                ArmOrigin::None,
                move |answer, _, cx| {
                    if answer.ok {
                        let (d, w) = (a.draft.clone(), worktree.clone());
                        this.update(cx, |v, cx| v.set_draft(repo, &w, d, cx)).ok();
                    }
                },
                window,
                cx,
            );
        });
    }

    /// The box says "Still working…" past 30 s: a frame then shows it, and nothing wakes before.
    pub fn remind_still_working(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(crate::commit_box::STILL_WORKING + std::time::Duration::from_millis(50)).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
    }

    /// A merge, rebase or pull of this repo is running: the commit panel's controls wait.
    pub fn integrating(&self, repo: u32) -> bool {
        use gitvolt_core::events::OpKind;
        self.ops.ops.values().any(|o| o.repo == Some(repo) && matches!(o.kind, OpKind::Merge | OpKind::Rebase | OpKind::Pull))
    }

    /// A section's own rows, as its header's buttons; then Discard all.
    pub fn section_menu(&self, which: Which, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let (Some(ctx), Some(files)) = (tab.wip_ctx(), tab.files.as_ref()) else { return Vec::new() };
        let queued = tab.wip.committing.then_some(COMMIT_QUEUED);
        let shown = files.section_files(which);
        let mut own = Vec::new();
        let (w1, c1) = (this.clone(), ctx.clone());
        if !shown.is_empty() && which == Which::Unstaged {
            own.push(MenuRow::action("wip.stageAll", "Stage all changes", "list-plus", "Stage every unstaged change", move |_, cx| {
                let c = c1.clone();
                w1.update(cx, |v, cx| v.stage_all_guarded(&c, cx)).ok();
            }).disabled_if(queued));
        } else if !shown.is_empty() {
            own.push(MenuRow::action("wip.unstageAll", "Unstage all changes", "list-minus", "Move every staged change back to Unstaged", move |_, cx| {
                let c = c1.clone();
                w1.update(cx, |v, cx| v.unstage_all(&c, cx)).ok();
            }).disabled_if(queued));
        }
        let mut discard = Vec::new();
        if which == Which::Unstaged && !shown.is_empty() && !shown.iter().all(FileChange::is_submodule) {
            let (w2, c2) = (this.clone(), ctx.clone());
            discard.push(MenuRow::action("wip.discardUnstaged", "Discard unstaged changes", "trash-2", "Discard every unstaged change and delete the untracked files; the staged ones stay (you can undo this)", move |_, cx| {
                let c = c2.clone();
                w2.update(cx, |v, cx| v.discard_unstaged(&c, cx)).ok();
            }).disabled_if(queued));
        }
        discard.push(self.discard_all_row(this));
        grouped(vec![own, discard])
    }

    /// `Discard all changes…`: on the WIP header, a section's menu and the graph's WIP row.
    pub fn discard_all_row(&self, this: &WeakEntity<GitVolt>) -> MenuRow {
        let tab = self.tabs.get(self.active);
        let ctx = tab.and_then(Tab::wip_ctx).or_else(|| self.write_ctx());
        let count = tab.and_then(|t| t.files.as_ref()).filter(|f| f.wip.is_some()).map(|f| f.changed_paths().len());
        let count = count.or_else(|| tab.and_then(|t| t.graph.as_ref()?.rows.first()?.wip.as_ref()).map(|w| w.files() as usize)).unwrap_or(0);
        let reason = match tab {
            Some(t) if t.wip.committing => Some(COMMIT_QUEUED),
            Some(t) if t.wip.discarding => Some("Discarding…"),
            Some(t) if t.operation().is_some() => Some("Abort instead"),
            _ if count == 0 => Some("Nothing to discard"),
            _ => None,
        };
        let weak = this.clone();
        MenuRow::action("wip.discardAll", "Discard all changes…", "trash-2", "Discard every staged, unstaged and untracked change, back to HEAD (you can undo this)", move |window, cx| {
            let Some(ctx) = ctx.clone() else { return };
            let origin = ArmOrigin::current(cx);
            weak.update(cx, |v, cx| v.discard_all(ctx, origin, window, cx)).ok();
        })
        .disabled_if(reason)
    }
}

impl GitVolt {
    /// What a write from the WIP panel targets.
    pub fn panel_ctx(&self) -> Option<WriteCtx> {
        self.tabs.get(self.active)?.wip_ctx()
    }

    pub fn wip_counts(&self) -> Option<(usize, usize, usize)> {
        self.tabs.get(self.active)?.files.as_ref().filter(|f| f.wip.is_some()).map(Files::counts)
    }

    /// The open diff, when it's a staged or unstaged WIP file: which, and whether it's staged.
    pub fn open_wip_file(&self) -> Option<(FileChange, Option<bool>)> {
        let d = self.tabs.get(self.active)?.diff.as_ref()?;
        let Origin::Wip { staged, .. } = &d.file.origin else { return None };
        (d.file.status != 'U' && d.file.status != UNCHANGED).then(|| (d.file.clone(), Some(*staged)))
    }

    /// Ctrl+Shift+S: the open file to the other list.
    pub fn stage_open_file(&mut self, cx: &mut Context<Self>) {
        let Some((file, staged)) = self.open_wip_file() else { return };
        let Origin::Wip { worktree, .. } = &file.origin else { return };
        let Some(repo) = self.tabs.get(self.active).map(|t| t.id) else { return };
        let ctx = WriteCtx { repo, worktree: worktree.clone() };
        if staged == Some(true) {
            self.unstage_files(&ctx, vec![file], cx);
        } else {
            self.stage_paths(&ctx, paths_of(std::slice::from_ref(&file)), cx);
        }
    }

    /// A WIP file's Stage or Unstage, and Discard.
    pub fn file_stage_rows(&self, file: &FileChange, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Origin::Wip { worktree, staged } = &file.origin else { return Vec::new() };
        if file.status == UNCHANGED {
            return Vec::new();
        }
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let ctx = WriteCtx { repo: tab.id, worktree: worktree.clone() };
        let queued = tab.wip.committing.then_some(COMMIT_QUEUED);
        if file.status == 'U' {
            return self.conflict_rows(ctx, file.path.as_ref(), this);
        }
        let name = file.path.rsplit('/').next().unwrap_or_default().to_string();
        if *staged {
            let (weak, c, f) = (this.clone(), ctx, file.clone());
            return vec![MenuRow::action("file.unstage", "Unstage", "minus", "Move this file's staged change back to Unstaged", move |_, cx| {
                let (c, f) = (c.clone(), f.clone());
                weak.update(cx, |v, cx| v.unstage_files(&c, vec![f], cx)).ok();
            }).disabled_if(queued)];
        }
        let (w1, c1, f1) = (this.clone(), ctx.clone(), file.clone());
        let mut rows = vec![MenuRow::action("file.stageFile", "Stage", "plus", "Stage this file's change", move |_, cx| {
            let (c, p) = (c1.clone(), paths_of(std::slice::from_ref(&f1)));
            w1.update(cx, |v, cx| v.stage_paths(&c, p, cx)).ok();
        }).disabled_if(queued)];
        if !file.is_submodule() {
            let (w2, f2) = (this.clone(), file.clone());
            rows.push(MenuRow::action("file.discard", "Discard changes…", "trash-2", "Discard this file's unstaged change (you can undo this)", move |window, cx| {
                let (c, f, n) = (ctx.clone(), f2.clone(), name.clone());
                let origin = ArmOrigin::current(cx);
                w2.update(cx, |v, cx| v.discard_files(c, vec![f], n, origin, window, cx)).ok();
            }).disabled_if(queued));
        }
        rows
    }

    /// A WIP folder's: on every file under it.
    pub fn folder_stage_rows(&self, path: &str, which: Which, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let (Some(ctx), Some(files)) = (tab.wip_ctx(), tab.files.as_ref()) else { return Vec::new() };
        let prefix = format!("{path}/");
        let under: Vec<FileChange> = files.section_files(which).into_iter().filter(|f| f.path.starts_with(&prefix)).collect();
        if under.is_empty() {
            return Vec::new();
        }
        let queued = tab.wip.committing.then_some(COMMIT_QUEUED);
        if which == Which::Conflicted {
            // Each as it is, in one write; files that still have markers arm the row first.
            let paths: Vec<String> = under.iter().map(|f| f.path.to_string()).collect();
            let n = paths.len();
            let word = if n == 1 { "file" } else { "files" };
            let dirty = paths.iter().find(|p| tab.merge_dirty_on(p)).cloned();
            let reason = queued.map(str::to_string).or(dirty.as_ref().map(|_| crate::merge::MERGE_SAVE_FIRST.to_string()));
            let (weak, c, tooltip) = (this.clone(), ctx, format!("Mark the {n} {word} conflicted in {path}/ resolved as they are now"));
            return vec![MenuRow::action("folder.markResolved", format!("Mark {n} {word} resolved"), "check-check", tooltip, move |_, cx| {
                let (c, paths) = (c.clone(), paths.clone());
                let origin = ArmOrigin::current(cx);
                weak.update(cx, |v, cx| v.mark_all_resolved(&c, paths, origin, cx)).ok();
            }).disabled_if(reason)];
        }
        if which == Which::Staged {
            let (weak, c) = (this.clone(), ctx);
            return vec![MenuRow::action("folder.unstage", "Unstage", "minus", format!("Move the staged changes in {path} back to Unstaged"), move |_, cx| {
                let (c, f) = (c.clone(), under.clone());
                weak.update(cx, |v, cx| v.unstage_files(&c, f, cx)).ok();
            }).disabled_if(queued)];
        }
        let (w1, c1, p1) = (this.clone(), ctx.clone(), paths_of(&under));
        let mut rows = vec![MenuRow::action("folder.stage", "Stage", "plus", format!("Stage the changes in {path}"), move |_, cx| {
            let (c, p) = (c1.clone(), p1.clone());
            w1.update(cx, |v, cx| v.stage_paths(&c, p, cx)).ok();
        }).disabled_if(queued)];
        let discardable: Vec<FileChange> = under.into_iter().filter(|f| !f.is_submodule()).collect();
        if !discardable.is_empty() {
            let (w2, name) = (this.clone(), path.to_string());
            rows.push(MenuRow::action("folder.discard", "Discard changes…", "trash-2", format!("Discard the unstaged changes in {path} (you can undo this)"), move |window, cx| {
                let (c, f, n) = (ctx.clone(), discardable.clone(), name.clone());
                let origin = ArmOrigin::current(cx);
                w2.update(cx, |v, cx| v.discard_files(c, f, n, origin, window, cx)).ok();
            }).disabled_if(queued));
        }
        rows
    }

    /// A conflicted WIP file's menu: Take current, Take incoming, Mark resolved, by the sides' names.
    fn conflict_rows(&self, ctx: WriteCtx, path: &str, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let dirty = tab.merge_dirty_on(path);
        let reason = if tab.wip.committing { Some(COMMIT_QUEUED) } else if dirty { Some(crate::merge::MERGE_SAVE_FIRST) } else { None };
        let target = self.ops.journal(ctx.repo, &ctx.worktree).and_then(|j| j.paused.as_ref()).map(|p| p.target.clone());
        let (cur, inc) = crate::merge::sides_of(tab, &ctx.worktree, target.as_deref());
        let side = |name: Option<String>, generic: &str| name.map_or(format!("the {generic} side"), |n| format!("{n} ({generic})"));
        let row = |id: &str, label: &str, glyph: &'static str, tooltip: String, kind: &'static str| {
            let (weak, c, p) = (this.clone(), ctx.clone(), path.to_string());
            MenuRow::action(id.to_string(), label.to_string(), glyph, tooltip, move |_, cx| {
                let (c, p) = (c.clone(), p.clone());
                let origin = ArmOrigin::current(cx);
                weak.update(cx, |v, cx| v.resolve_file(&c, p, kind, None, origin, cx)).ok();
            })
            .disabled_if(reason)
        };
        let (weak, c, p) = (this.clone(), ctx.clone(), path.to_string());
        vec![
            row("file.takeCurrent", "Take current", "arrow-left-to-line", format!("Resolve {path} with the version in {}", side(cur, "current")), "current"),
            row("file.takeIncoming", "Take incoming", "arrow-right-to-line", format!("Resolve {path} with the version in {}", side(inc, "incoming")), "incoming"),
            MenuRow::action("file.markResolved", "Mark resolved", "check-check", format!("Mark {path} resolved as it is now"), move |_, cx| {
                let (c, p) = (c.clone(), p.clone());
                let origin = ArmOrigin::current(cx);
                weak.update(cx, |v, cx| v.mark_resolved(&c, p, origin, cx)).ok();
            })
            .disabled_if(reason),
        ]
    }

    /// Several files at once, each as it is: one write, one question for those that still have markers.
    pub fn mark_all_resolved(&mut self, ctx: &WriteCtx, paths: Vec<String>, origin: ArmOrigin, cx: &mut Context<Self>) {
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let w = Write::new(ctx, move |asked| call("markResolved", json!({ "repo": repo, "worktree": worktree, "paths": paths, "confirmMarkers": asked.markers }))).origin(origin);
        self.write(w, cx);
    }

    /// Mark resolved: the file as it is; asks first while conflict markers remain.
    pub fn mark_resolved(&mut self, ctx: &WriteCtx, path: String, origin: ArmOrigin, cx: &mut Context<Self>) {
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let w = Write::new(ctx, move |asked| call("markResolved", json!({ "repo": repo, "worktree": worktree, "paths": [path], "confirmMarkers": asked.markers }))).origin(origin);
        self.write(w, cx);
    }

    /// What a rebase's outcome says.
    pub fn toast_rebase_outcome(&mut self, out: &Value, cx: &mut Context<Self>) {
        let warn = |text: &str, cx: &mut Context<Self>| crate::ui::toast::show_with(text.to_string(), crate::ui::toast::Options { warning: true, ..Default::default() }, cx);
        match out["status"].as_str() {
            Some("done" | "upToDate" | "stopped") => {
                if let Some(w) = out["warning"].as_str() {
                    warn(w, cx);
                }
            }
            _ => {
                if let Some(b) = out["branch"].as_str() {
                    warn(&format!("Your commits from the stop are on {b}"), cx);
                }
            }
        }
    }

    /// A watched change under the open WIP file: its sides again, where it was.
    pub fn reread_open_file(&mut self, repo: u32, cx: &mut Context<Self>) {
        let is_active = self.tabs.get(self.active).is_some_and(|t| t.id == repo);
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo).filter(|_| is_active) else { return };
        let Some(diff) = tab.diff.as_mut().filter(|d| matches!(d.file.origin, Origin::Wip { .. }) && tab.history.is_none()) else { return };
        // The merge tool's work is its own: a change on disk meets it at the save.
        if diff.editing_dirty() || crate::merge::is_conflict(&diff.file) {
            return;
        }
        diff.keep_scroll();
        diff.generation += 1;
        self.read_contents(false, cx);
    }
}

/// The commit button's state now: the fields' text is read through `cx`.
pub fn commit_view_of(tab: &Tab, cx: &App) -> CommitView {
    let (staged, unstaged, conflicted) = tab.files.as_ref().map_or((0, 0, 0), Files::counts);
    let summary = tab.wip.summary.read(cx).text().to_string();
    let state = tab.box_state();
    if let Some(op) = &state.box_op {
        return crate::commit_box::continue_button(op, &summary, tab.wip.description.read(cx).text());
    }
    let in_merge = state.operation.is_some_and(|o| o.is_merge());
    commit_button(&CommitState { staged, unstaged, conflicted, amend: tab.wip.amend.is_some(), in_merge, summary: &summary, staging: tab.wip.staging > 0 })
}

fn drafts_file() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    Some(gitvolt_core::paths::data_base().join("gitvolt-gpui").join("drafts.json"))
}

fn drafts_json(drafts: &std::collections::HashMap<String, Draft>) -> Value {
    let map: serde_json::Map<String, Value> = drafts.iter().filter(|(_, d)| !d.summary.is_empty() || !d.description.is_empty()).map(|(k, d)| (k.clone(), json!({ "summary": d.summary, "description": d.description }))).collect();
    Value::Object(map)
}

fn asides_json(asides: &std::collections::HashMap<String, MergeAside>) -> Value {
    let part = |d: &Draft| json!({ "summary": d.summary, "description": d.description });
    Value::Object(asides.iter().map(|(k, a)| (k.clone(), json!({ "head": a.head, "draft": part(&a.draft), "applied": part(&a.applied) }))).collect())
}

fn asides_of(v: &Value) -> std::collections::HashMap<String, MergeAside> {
    let part = |d: &Value| Draft { summary: d["summary"].as_str().unwrap_or_default().to_string(), description: d["description"].as_str().unwrap_or_default().to_string() };
    v[ASIDES_KEY].as_object().map_or_else(Default::default, |m| m.iter().map(|(k, a)| (k.clone(), MergeAside { head: a["head"].as_str().unwrap_or_default().to_string(), draft: part(&a["draft"]), applied: part(&a["applied"]) })).collect())
}

/// In `drafts.json`, beside the worktrees' drafts: no worktree's path starts so.
const ASIDES_KEY: &str = "#mergeAsides";

fn drafts_of(v: &Value) -> std::collections::HashMap<String, Draft> {
    v.as_object().map_or_else(Default::default, |m| m.iter().filter(|(k, _)| k.as_str() != ASIDES_KEY).map(|(k, d)| (k.clone(), Draft { summary: d["summary"].as_str().unwrap_or_default().to_string(), description: d["description"].as_str().unwrap_or_default().to_string() })).collect())
}

// ---- Views ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Danger,
    Positive,
}

/// `.row-action`: an icon button in a row, a press never selecting the row; with `text`, an outlined one.
pub fn action_icon(id: impl Into<ElementId>, tooltip: impl Into<SharedString>, glyph: &'static str, tone: Tone, text: Option<SharedString>, disabled: bool, on_click: impl Fn(&mut Window, &mut App) + 'static) -> impl IntoElement {
    action_sized(id, tooltip, glyph, tone, text, disabled, (22., 22.), on_click)
}

/// `.row-action`'s sizes where it isn't a row's: `(icon button's width, height)`; a text one has the height.
#[allow(clippy::too_many_arguments)]
pub fn action_sized(id: impl Into<ElementId>, tooltip: impl Into<SharedString>, glyph: &'static str, tone: Tone, text: Option<SharedString>, disabled: bool, sized: (f32, f32), on_click: impl Fn(&mut Window, &mut App) + 'static) -> impl IntoElement {
    let color = match tone {
        Tone::Plain => theme::text_secondary(),
        Tone::Danger => theme::red(),
        Tone::Positive => theme::green(),
    };
    let hover_fill = match tone {
        Tone::Plain => theme::control_hover_bg(),
        Tone::Danger => theme::red(),
        Tone::Positive => theme::green(),
    };
    let hover_text = match tone {
        Tone::Plain => theme::text_normal(),
        Tone::Danger => theme::text_selected(),
        Tone::Positive => theme::app_bg(),
    };
    let id: ElementId = id.into();
    let group: SharedString = format!("act-{id:?}").into();
    let has_text = text.is_some();
    let bordered = tone != Tone::Plain || has_text;
    let glyph_el = icon(glyph, if has_text { 14. } else { 13. }, color).when(!disabled, |s| s.group_hover(group.clone(), move |s| s.text_color(hover_text)));
    tip(
        tooltip.into(),
        div()
            .id(id)
            .group(group)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(5.))
            .rounded(px(if has_text { 4. } else { 3. }))
            .text_size(px(12.))
            .whitespace_nowrap()
            .text_color(color)
            .when(has_text, |d| d.h(px(sized.1)).px(px(10.)))
            .when(!has_text, |d| d.w(px(sized.0)).h(px(sized.1)))
            .when(bordered, |d| d.border_1().border_color(color))
            .when(disabled, |d| d.opacity(0.4))
            .when(!disabled, |d| d.cursor_pointer().hover(move |d| d.bg(hover_fill).text_color(hover_text).border_color(hover_fill)))
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                if !disabled {
                    on_click(window, cx);
                }
            })
            .child(glyph_el)
            .children(text),
    )
}

/// A row's hover actions: unstaged rows Discard and Stage, staged rows Unstage; a folder acts on every file
/// under it; a Conflicted row's is Mark resolved.
#[allow(clippy::too_many_arguments)]
pub fn row_actions(v: &GitVolt, which: Which, row: &ListRow, changes: &[FileChange], group: &SharedString, on: bool, panel_w: f32, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let this = cx.weak_entity();
    let tab = v.tabs.get(v.active)?;
    let ctx = tab.wip_ctx()?;
    let section = tab.files.as_ref()?.wip.as_ref().map(|w| w.section(which));
    let files = files_under(row, changes, which, section);
    if files.is_empty() {
        return None;
    }
    let committing = tab.wip.committing;
    let name = match row {
        ListRow::File { name, .. } => name.to_string(),
        ListRow::Folder { path, .. } => path.to_string(),
    };
    let key: SharedString = match row {
        ListRow::File { index, .. } => format!("row-actions:{}:{}", which.scope(), changes[*index].path).into(),
        ListRow::Folder { path, .. } => format!("row-actions:{}:{path}/", which.scope()).into(),
    };
    let queued = |t: &str| if committing { COMMIT_QUEUED.to_string() } else { t.to_string() };
    let bg = theme::over(if on { theme::selected_row() } else { theme::hover_row() }, theme::panel_bg());
    let narrow = panel_w <= 280.;
    let mut bar = div()
        .absolute()
        .right(px(8.))
        .top_0()
        .bottom_0()
        .flex()
        .items_center()
        .gap(px(4.))
        .pl(px(6.))
        .bg(bg)
        .when(!on, |d| d.invisible().group_hover(group.clone(), |d| d.visible()));
    match which {
        Which::Conflicted => {
            let path = files[0].path.to_string();
            let (weak, ctx, k) = (this.clone(), ctx.clone(), key.clone());
            if !matches!(row, ListRow::File { .. }) {
                return None;
            }
            let control: SharedString = format!("{key}:resolve").into();
            let merging = tab.merge_dirty_on(&path);
            let resolve_tip = if merging && !committing { crate::merge::MERGE_SAVE_FIRST.to_string() } else { queued(&format!("Mark {path} resolved")) };
            bar = bar.child(crate::ui::anchor(control.clone(), action_icon(ElementId::Name(control.clone()), resolve_tip, "check-check", Tone::Plain, None, committing || merging, move |_, cx| {
                let (path, ctx) = (path.clone(), ctx.clone());
                let origin = ArmOrigin::Control { key: format!("{k}:resolve").into(), cover: None, grow_right: Some(false) };
                weak.update(cx, |v, cx| v.mark_resolved(&ctx, path, origin, cx)).ok();
            })));
        }
        Which::Unstaged => {
            let discardable: Vec<FileChange> = files.iter().filter(|f| !f.is_submodule()).cloned().collect();
            if !discardable.is_empty() {
                let (weak, ctx, list, nm, k) = (this.clone(), ctx.clone(), discardable, name.clone(), key.clone());
                let control: SharedString = format!("{key}:discard").into();
                bar = bar.child(crate::ui::anchor(control.clone(), action_icon(ElementId::Name(control.clone()), queued("Discard changes"), "trash-2", Tone::Danger, None, committing, move |window, cx| {
                    let origin = ArmOrigin::Control { key: format!("{k}:discard").into(), cover: Some(k.clone()), grow_right: Some(false) };
                    let (ctx, list, nm) = (ctx.clone(), list.clone(), nm.clone());
                    weak.update(cx, |v, cx| v.discard_files(ctx, list, nm, origin, window, cx)).ok();
                })));
            }
            let (weak, ctx, paths) = (this.clone(), ctx.clone(), paths_of(&files));
            bar = bar.child(action_icon(ElementId::Name(format!("{key}:stage").into()), queued("Stage"), "plus", Tone::Positive, (!narrow).then(|| "Stage File".into()), committing, move |_, cx| {
                let (ctx, paths) = (ctx.clone(), paths.clone());
                weak.update(cx, |v, cx| v.stage_paths(&ctx, paths, cx)).ok();
            }));
        }
        Which::Staged => {
            let (weak, ctx, list) = (this.clone(), ctx.clone(), files.clone());
            bar = bar.child(action_icon(ElementId::Name(format!("{key}:unstage").into()), queued("Unstage"), "minus", Tone::Danger, (!narrow).then(|| "Unstage File".into()), committing, move |_, cx| {
                let (ctx, list) = (ctx.clone(), list.clone());
                weak.update(cx, |v, cx| v.unstage_files(&ctx, list, cx)).ok();
            }));
        }
    }
    Some(crate::ui::anchor(key, bar).into_any_element())
}

fn status_counts(files: &[FileChange], size: f32) -> Vec<AnyElement> {
    let (mut added, mut changed, mut gone, mut renamed, mut conflicted) = (0, 0, 0, 0, 0);
    for f in files {
        match status_icon(f.status).0 {
            "status-added" => added += 1,
            "status-deleted" => gone += 1,
            "status-renamed" => renamed += 1,
            "status-conflicted" => conflicted += 1,
            _ => changed += 1,
        }
    }
    let count = |name: &'static str, n: usize, color: Hsla| (n > 0).then(|| div().flex().flex_none().items_center().gap(px(3.)).child(icon(name, size, color)).child(n.to_string()).into_any_element());
    [count("status-modified", changed, theme::status_modified()), count("status-added", added, theme::status_added()), count("status-deleted", gone, theme::status_deleted()), count("status-renamed", renamed, theme::blue()), count("status-conflicted", conflicted, theme::status_conflicted())].into_iter().flatten().collect()
}

#[allow(clippy::too_many_arguments)]
fn section_view(which: Which, tab: &Tab, keys_here: bool, selected: Option<FileChange>, panel_w: f32, closed: bool, basis: Option<f32>, conflicted_h: Option<f32>, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let files = tab.files.as_ref();
    let section = files.and_then(|f| f.wip.as_ref()).map(|w| w.section(which));
    let shown = files.map_or(Vec::new(), |f| f.section_files(which));
    if which == Which::Conflicted && section.is_none_or(|s| s.total == 0) {
        return None;
    }
    let filtering = files.is_some_and(|f| !f.filter.trim().is_empty());
    let count = section.map(|s| if filtering { format!(" ({} of {})", shown.len(), s.total) } else { format!(" ({})", shown.len()) });
    let ctx = tab.wip_ctx();
    let committing = tab.wip.committing;
    let queued = |t: &str| if committing { COMMIT_QUEUED.to_string() } else { t.to_string() };
    let weak = cx.weak_entity();
    let head_actions = ctx.clone().filter(|_| !shown.is_empty() && which != Which::Conflicted).map(|ctx| {
        // Full label, then short under 460 px, then the icon alone under 340 px.
        let label = |full: &'static str, short: &'static str| if panel_w > 460. { Some(SharedString::from(full)) } else if panel_w > 340. { Some(SharedString::from(short)) } else { None };
        let mut row = div().id(("wip-head-actions", which.index())).flex().flex_none().ml_auto().gap(px(6.)).on_click(|_, _, cx| cx.stop_propagation());
        if which == Which::Unstaged {
            if !shown.iter().all(FileChange::is_submodule) {
                let (w, c) = (weak.clone(), ctx.clone());
                row = row.child(action_sized("wip-discard-unstaged", queued("Discard unstaged changes"), "trash-2", Tone::Danger, None, committing, (24., 24.), move |_, cx| {
                    let c = c.clone();
                    w.update(cx, |v, cx| v.discard_unstaged(&c, cx)).ok();
                }));
            }
            let (w, c) = (weak.clone(), ctx.clone());
            row = row.child(action_sized("wip-stage-all", queued("Stage all changes"), "list-plus", Tone::Positive, label("Stage All Changes", "Stage all"), committing, (24., 24.), move |_, cx| {
                let c = c.clone();
                w.update(cx, |v, cx| v.stage_all_guarded(&c, cx)).ok();
            }));
        } else {
            let (w, c) = (weak.clone(), ctx.clone());
            row = row.child(action_sized("wip-unstage-all", queued("Unstage all changes"), "list-minus", Tone::Danger, label("Unstage All Changes", "Unstage all"), committing, (24., 24.), move |_, cx| {
                let c = c.clone();
                w.update(cx, |v, cx| v.unstage_all(&c, cx)).ok();
            }));
        }
        row
    });
    let head = div()
        .id(("wip-section-head", which.index()))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .pl(px(9.))
        .pr(px(8.))
        // Its line's on top of its height; with both open the handle's line is the one above Staged.
        .when(which != Which::Conflicted && (basis.is_none() || which != Which::Staged), |d| d.h(px(FILE_ROW_H + 1.)).border_t_1().border_color(theme::section_border()))
        .when(which == Which::Conflicted || (basis.is_some() && which == Which::Staged), |d| d.h(px(FILE_ROW_H)))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_wip_section(which, cx)))
        .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, window, cx| {
            if which == Which::Conflicted {
                return;
            }
            cx.stop_propagation();
            let weak = cx.weak_entity();
            let rows = this.section_menu(which, &weak);
            menu::open_context(rows, e.position, window, cx);
        }))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text_secondary())
                .child(icon(if closed { "chevron-right" } else { "chevron-down" }, 12., theme::text_secondary()))
                .child(format!("{}{}", which.label().to_uppercase(), count.unwrap_or_default())),
        )
        .child(div().flex().flex_shrink(1.).min_w_0().overflow_hidden().gap(px(8.)).items_center().text_color(theme::text_secondary()).children(status_counts(&shown, 12.)))
        .children(head_actions);
    let body = section.filter(|_| !closed).map(|s| {
        let f = files.expect("a section has its files");
        let mode = f.mode;
        let expanded = f.collapsed.is_empty();
        let has_folders = s.rows.iter().any(|r| matches!(r, ListRow::Folder { .. }));
        let tool = |id: &'static str, glyph: &'static str, label: &'static str, on: bool, off: bool| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .px(px(4.))
                .py(px(1.))
                .rounded(px(3.))
                .text_size(px(11.))
                .line_height(px(15.))
                .text_color(if on { theme::text_selected() } else { theme::text_secondary() })
                .when(off, |d| d.opacity(0.5))
                .when(!off, |d| d.cursor_pointer().hover(|d| d.text_color(theme::text_selected()).bg(theme::control_hover_bg())))
                .child(icon(glyph, 12., if on { theme::text_selected() } else { theme::text_secondary() }))
                .child(label)
        };
        let lead = match mode {
            ListMode::Tree => tool(if which == Which::Unstaged { "wip-collapse-u" } else if which == Which::Staged { "wip-collapse-s" } else { "wip-collapse-c" }, if expanded { "chevrons-down-up" } else { "chevrons-up-down" }, if expanded { "Collapse all" } else { "Expand all" }, false, !has_folders).when(has_folders, |d| d.on_click(cx.listener(|this, _, _, cx| this.with_files(cx, Files::toggle_all)))),
            ListMode::Path => tool(if which == Which::Unstaged { "wip-sort-u" } else if which == Which::Staged { "wip-sort-s" } else { "wip-sort-c" }, "arrow-down-wide-narrow", "Sort by status", f.by_status, false).on_click(cx.listener(|this, _, _, cx| {
                if let Some(tab) = this.tabs.get_mut(this.active) {
                    tab.sort_by_status = !tab.sort_by_status;
                    let on = tab.sort_by_status;
                    this.with_files(cx, |f| {
                        f.by_status = on;
                        f.rebuild();
                    });
                }
            })),
        };
        let totals = (s.added > 0 || s.deleted > 0).then(|| {
            div()
                .ml_auto()
                .flex()
                .flex_none()
                .gap(px(4.))
                .text_size(px(12.))
                .when(s.added > 0, |d| d.child(div().text_color(theme::stats_added_bright()).child(format!("+{}", s.added))))
                .when(s.deleted > 0, |d| d.child(div().text_color(theme::stats_deleted_bright()).child(format!("−{}", s.deleted))))
        });
        let tool_line = (!s.rows.is_empty()).then(|| div().flex().flex_none().items_center().pl(px(5.)).pr(px(8.)).pt(px(2.)).pb(px(4.)).child(lead).children(totals));
        let id: &'static str = match which {
            Which::Conflicted => "wip-list-c",
            Which::Unstaged => "wip-list-u",
            Which::Staged => "wip-list-s",
        };
        let query = f.filter.trim().to_lowercase();
        let list = file_list(id, ListView { rows: s.rows.clone(), changes: f.changes.clone(), cursor: f.cursor.clone(), keys_here, selected: selected.clone(), query, panel_w, section: Some(which) }, &s.list, cx);
        div()
            .id(("wip-section-body", which.index()))
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                if which == Which::Conflicted {
                    return;
                }
                let weak = cx.weak_entity();
                let rows = this.section_menu(which, &weak);
                menu::open_context(rows, e.position, window, cx);
            }))
            .children(tool_line)
            .child(list)
    });
    let el = div().flex().flex_col().min_h_0().overflow_hidden().child(head).children(body);
    let el = match (closed, basis, conflicted_h) {
        (true, _, _) => el.flex_none(),
        (false, Some(b), _) => el.flex_grow(0.).flex_shrink_0().flex_basis(relative(b)),
        (false, None, Some(h)) => el.flex_none().h(px(h)),
        _ => el.flex_1(),
    };
    Some(el.into_any_element())
}

/// `wipSplitBounds`.
pub fn split_bounds(height: f32) -> (f32, f32) {
    if height <= 0. {
        return (0.1, 0.9);
    }
    let lo = (SPLIT_CHROME + SPLIT_MIN_ROWS * FILE_ROW_H) / height;
    if lo >= 0.5 { (0.5, 0.5) } else { (lo, 1. - lo) }
}

/// Undo and Redo for the staging log (`StagingUndoButtons`), in the file list's header and a WIP diff's toolbar.
pub fn staging_buttons(ids: (&'static str, &'static str), ctx: WriteCtx, staging: Option<&StagingUndoState>, committing: bool, cx: &mut Context<GitVolt>) -> Div {
    let view = |which: &str, label: Option<&String>| -> (String, bool) {
        if committing {
            return (COMMIT_QUEUED.to_string(), true);
        }
        if let Some(off) = staging.and_then(|s| s.off.clone()) {
            return (off, true);
        }
        match label {
            None => (format!("Nothing to {which} in staging"), true),
            Some(l) => (format!("{} {l} ({})", if which == "undo" { "Undo" } else { "Redo" }, crate::commands::display_chord(if which == "undo" { "Mod+Z" } else { "Mod+Shift+Z" })), false),
        }
    };
    let (u, r) = (view("undo", staging.and_then(|s| s.undo.as_ref())), view("redo", staging.and_then(|s| s.redo.as_ref())));
    let weak = cx.weak_entity();
    let (w1, c1, w2, c2) = (weak.clone(), ctx.clone(), weak, ctx);
    div()
        .flex()
        .flex_none()
        .gap(px(2.))
        .child(action_sized(ids.0, u.0, "undo-2", Tone::Plain, None, u.1, (20., 18.), move |_, cx| {
            let c = c1.clone();
            w1.update(cx, |v, cx| v.staging_undo(&c, false, cx)).ok();
        }))
        .child(action_sized(ids.1, r.0, "redo-2", Tone::Plain, None, r.1, (20., 18.), move |_, cx| {
            let c = c2.clone();
            w2.update(cx, |v, cx| v.staging_undo(&c, true, cx)).ok();
        }))
}

/// The whole panel for a WIP row.
pub fn wip_panel(v: &GitVolt, tab: &Tab, width: f32, height: f32, window: &Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let (ratio, closed) = (v.wip_ratio, v.wip_closed);
    let selected = tab.diff.as_ref().map(|d| d.file.clone());
    let keys_here = tab.keys == KeyArea::Files;
    let count = tab.files.as_ref().filter(|f| f.wip.is_some()).map(|f| f.changed_paths().len());
    let worktree = tab.wip_worktree().unwrap_or_default().to_string();
    let count = count.unwrap_or_else(|| tab.graph.as_ref().and_then(|g| g.rows.iter().find(|r| r.worktree.as_deref() == Some(worktree.as_str()))).and_then(|r| r.wip.as_ref()).map_or(0, |w| w.files() as usize));
    let branch: Option<SharedString> = tab.graph.as_ref().and_then(|g| g.worktree_branch(&worktree));
    let ctx = tab.wip_ctx();
    let in_progress = tab.operation().is_some();
    let weak = cx.weak_entity();
    let discard_tip = if tab.wip.committing { COMMIT_QUEUED } else if tab.wip.discarding { "Discarding…" } else if in_progress { "Abort instead" } else if count == 0 { "Nothing to discard" } else { "Discard all changes" };
    let discard_off = tab.wip.committing || tab.wip.discarding || in_progress || count == 0;
    // What the armed label covers dims underneath (`data-arm-dim`).
    let armed = crate::ui::arm::armed_control(cx).is_some_and(|k| k == "wip-discard-all");
    let header = div()
        .flex()
        .flex_none()
        .h(px(HEADER_H))
        .items_center()
        .gap(px(6.))
        .px(px(12.))
        .border_b_1()
        .border_color(theme::section_border())
        .child(
            div().flex().flex_1().min_w_0().when_some(ctx.clone(), |d, ctx| {
                let w = weak.clone();
                d.child(crate::ui::anchor(
                    "wip-discard-all",
                    action_sized("wip-discard-all-btn", discard_tip, "trash-2", Tone::Danger, None, discard_off, (20., 18.), move |window, cx| {
                        let ctx = ctx.clone();
                        let origin = ArmOrigin::Control { key: "wip-discard-all".into(), cover: None, grow_right: Some(true) };
                        w.update(cx, |v, cx| v.discard_all(ctx, origin, window, cx)).ok();
                    }),
                ))
            }),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .when(armed, |d| d.opacity(0.3))
                .child(format!("{count} file {}", if count == 1 { "change" } else { "changes" }))
                .when_some(branch, |d, b| d.child("on").child(div().px(px(6.)).rounded(px(3.)).bg(theme::mix(theme::blue(), 0.3, transparent_black())).text_color(theme::text_selected()).truncate().child(b))),
        )
        .child(
            div().flex().flex_1().min_w_0().justify_end().when(armed, |d| d.opacity(0.3)).child(tip(
                "Filter files",
                div()
                    .id("wip-filter-toggle")
                    .flex()
                    .size(px(22.))
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .cursor_pointer()
                    .when(tab.wip.filter_open, |d| d.bg(theme::selected_row()))
                    .hover(|d| d.bg(theme::hover_row()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let Some(tab) = this.tabs.get(this.active) else { return };
                        let (id, open) = (tab.id, tab.wip.filter_open);
                        this.toggle_wip_filter(id, !open, cx);
                    }))
                    .child(icon("funnel", 14., theme::text_secondary())),
            )),
        );
    let mode = tab.files.as_ref().map_or(ListMode::Tree, |f| f.mode);
    let staging_buttons = ctx.clone().map(|ctx| staging_buttons(("wip-staging-undo", "wip-staging-redo"), ctx, v.ops.staging(tab.id, tab.wip_worktree().unwrap_or_default()), tab.wip.committing, cx));
    let all_on = tab.wip.all_files;
    let view_bar = div()
        .relative()
        .flex()
        .flex_none()
        .h(px(VIEW_BAR_H))
        .items_center()
        .justify_center()
        .px(px(12.))
        .child(
            div().absolute().left(px(12.)).top_0().bottom_0().flex().items_center().child(tip(
                "Every tracked file of the working tree, not only the changed ones",
                div()
                    .id("wip-all-files")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(4.))
                    .pl(px(5.))
                    .pr(px(8.))
                    .py(px(2.))
                    .border_1()
                    .border_color(theme::section_border())
                    .rounded(px(4.))
                    .text_color(if all_on { theme::text_selected() } else { theme::text_secondary() })
                    .when(all_on, |d| d.bg(theme::selected_row()))
                    .hover(|d| d.text_color(theme::text_selected()))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_wip_all_files(cx)))
                    .child(icon("folder-open", 12., if all_on { theme::text_selected() } else { theme::text_secondary() }))
                    .when(width > 330., |d| d.child("View all files")),
            )),
        )
        .child(segmented(vec![
            segment_with_icon("wip-path", "list", "Path", mode == ListMode::Path).on_click(cx.listener(|this, _, _, cx| this.with_files(cx, |f| f.set_mode(ListMode::Path)))).into_any_element(),
            segment_with_icon("wip-tree", "list-tree", "Tree", mode == ListMode::Tree).on_click(cx.listener(|this, _, _, cx| this.with_files(cx, |f| f.set_mode(ListMode::Tree)))).into_any_element(),
        ]))
        .child(div().absolute().right(px(12.)).top_0().bottom_0().flex().items_center().children(staging_buttons));
    let filter_row = tab.wip.filter_open.then(|| {
        let input = tab.wip.filter.clone();
        let clear = input.clone();
        let id = tab.id;
        let w = weak.clone();
        div().flex().flex_none().px(px(8.)).py(px(4.)).child(crate::ui::controls::clearable("wip-filter", &input, move |_, cx| {
            clear.update(cx, |i, cx| i.set_text("", cx));
            w.update(cx, |v, cx| v.wip_filter_changed(id, cx)).ok();
        }, window, cx).h(px(24.)).text_size(px(12.)))
    });
    let both = !closed[1] && !closed[2];
    let split_h = crate::ui::Ui::get(cx).anchor("wip-split").map_or(0., |b| b.size.height.as_f32());
    let (lo, hi) = split_bounds(split_h);
    let shown = ratio.clamp(lo, hi);
    let conflicted_rows = tab.files.as_ref().and_then(|f| f.wip.as_ref()).map_or(0, |w| w.section(Which::Conflicted).rows.len());
    let conflicted_h = (26. + 25. + conflicted_rows as f32 * FILE_ROW_H).min((height / 3.).max(2. * FILE_ROW_H + 25.));
    let conflicted = section_view(Which::Conflicted, tab, keys_here, selected.clone(), width, closed[0], None, Some(conflicted_h), cx);
    let unstaged = section_view(Which::Unstaged, tab, keys_here, selected.clone(), width, closed[1], both.then_some(shown), None, cx);
    let resizer = both.then(|| {
        div()
            .id("wip-split-resizer")
            .relative()
            .flex_none()
            .h(px(5.))
            .my(px(-2.))
            .cursor(CursorStyle::ResizeUpDown)
            .group("wip-split-resizer")
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                if e.click_count == 2 {
                    this.wip_ratio = SPLIT_DEFAULT;
                    this.save_layout();
                    cx.notify();
                    return;
                }
                this.side_drag = crate::sidebar::SideDrag::WipSplit { y: e.position.y.as_f32(), ratio: shown };
            }))
            .child(div().absolute().left_0().right_0().top(px(2.)).h(px(1.)).bg(theme::section_border()).group_hover("wip-split-resizer", |d| d.top(px(1.)).h(px(3.)).bg(theme::blue())))
    });
    let staged = section_view(Which::Staged, tab, keys_here, selected, width, closed[2], both.then_some(shown), None, cx);
    let split = crate::ui::anchor("wip-split", div().flex().flex_col().flex_1().min_h_0().children(unstaged).children(resizer).children(staged));
    let commit_box = crate::commit_box::commit_box(v, tab, height, window, cx);
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        .w(px(width))
        .h_full()
        .min_h_0()
        .bg(theme::panel_bg())
        .border_l_1()
        .border_color(theme::section_border())
        .child(header)
        .child(div().flex().flex_col().flex_1().min_h_0().overflow_hidden().child(view_bar).children(filter_row).children(conflicted).child(split))
        .child(commit_box)
        .child(crate::details::details_resizer(width, cx))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::testing::Fixture;
    use serde_json::json;
    use std::prelude::v1::test;

    fn payload(files: &[(&str, &str)]) -> Value {
        let list: Vec<Value> = files.iter().map(|(p, s)| json!({ "path": p, "status": s, "additions": 1, "deletions": 2, "old": {"kind": "absent"}, "new": {"kind": "absent"} })).collect();
        json!({ "files": list, "added": files.len(), "deleted": 2 * files.len() })
    }

    fn lists() -> Files {
        Files::wip("/w".into(), &payload(&[("src/a", "M"), ("b", "A"), ("c", "U")]), &payload(&[("src/a", "M"), ("d", "D")]))
    }

    fn paths(f: &Files, w: Which) -> Vec<String> {
        f.section_files(w).iter().map(|x| x.path.to_string()).collect()
    }

    #[test]
    fn a_merge_message_joins_the_draft_and_never_replaces_it() {
        let msg = "Merge branch 'x'\n\nBody\n# Conflicts:\n#\tREADME\n";
        assert_eq!(with_merge_message(&Draft::default(), msg), Draft { summary: "Merge branch 'x'".into(), description: "Body".into() });
        let typed = Draft { summary: "Mine".into(), description: "Why".into() };
        assert_eq!(with_merge_message(&typed, msg), Draft { summary: "Mine".into(), description: "Why\n\nMerge branch 'x'\n\nBody".into() });
    }

    #[test]
    fn a_stops_box_sends_a_message_only_when_it_was_edited() {
        let text = Draft { summary: "Git's".into(), description: String::new() };
        let mut stop = StopBox { key: "k".into(), original: text.clone(), text };
        assert_eq!(stop.edited_message(), None);
        stop.text.summary = "Mine".into();
        assert_eq!(stop.edited_message().as_deref(), Some("Mine"));
        stop.text.summary = "  ".into();
        assert_eq!(stop.edited_message(), None, "an emptied box: git's own");
    }

    #[test]
    fn asides_keep_their_place_in_the_drafts_file() {
        let mut all = drafts_json(&[("/w".to_string(), Draft { summary: "s".into(), description: String::new() })].into_iter().collect());
        let aside = MergeAside { head: "h".into(), draft: Draft::default(), applied: Draft { summary: "m".into(), description: String::new() } };
        all[ASIDES_KEY] = asides_json(&[("/w".to_string(), aside.clone())].into_iter().collect());
        assert_eq!(drafts_of(&all).len(), 1, "the asides aren't a worktree's draft");
        assert_eq!(asides_of(&all)["/w"], aside);
    }

    #[test]
    fn a_conflicted_file_leaves_the_other_sections() {
        let f = lists();
        assert_eq!(paths(&f, Which::Conflicted), ["c"]);
        assert_eq!(paths(&f, Which::Unstaged), ["src/a", "b"]);
        assert_eq!(paths(&f, Which::Staged), ["src/a", "d"]);
        assert_eq!(f.counts(), (2, 2, 1));
        // A file in two lists counts once.
        assert_eq!(f.discard_scope().0, 4);
    }

    #[test]
    fn the_filter_narrows_every_section_and_the_totals_follow() {
        let mut f = lists();
        f.filter = "src".into();
        f.rebuild();
        assert_eq!(paths(&f, Which::Unstaged), ["src/a"]);
        let s = f.wip.as_ref().unwrap().section(Which::Unstaged);
        assert_eq!((s.total, s.count, s.added, s.deleted), (2, 1, 1, 2));
    }

    #[test]
    fn folders_of_the_same_name_collapse_section_by_section() {
        let mut f = lists();
        let key = |f: &Files, w: Which| f.wip.as_ref().unwrap().section(w).rows.iter().find_map(|r| if let ListRow::Folder { key, .. } = r { Some(key.clone()) } else { None }).unwrap();
        let k = key(&f, Which::Unstaged);
        f.toggle_folder(&k);
        let open = |f: &Files, w: Which| f.wip.as_ref().unwrap().section(w).rows.iter().any(|r| matches!(r, ListRow::File { name, .. } if name.as_ref() == "a"));
        assert!(!open(&f, Which::Unstaged));
        assert!(open(&f, Which::Staged), "Staged's src stays open");
    }

    #[test]
    fn the_arrows_run_on_through_the_sections_skipping_a_collapsed_one() {
        let mut f = lists();
        let order = |f: &Files| f.wip_order().iter().map(|i| format!("{}{}", if matches!(f.changes[*i].origin, Origin::Wip { staged: true, .. }) { "s:" } else { "u:" }, f.changes[*i].path)).collect::<Vec<_>>();
        assert_eq!(order(&f), ["u:c", "u:src/a", "u:b", "s:src/a", "s:d"]);
        f.wip.as_mut().unwrap().closed[1] = true;
        assert_eq!(order(&f), ["u:c", "s:src/a", "s:d"]);
    }

    #[test]
    fn the_open_file_follows_where_it_went() {
        let f = lists();
        let open = FileChange { origin: Origin::Wip { worktree: "/w".into(), staged: false }, ..f.section_files(Which::Unstaged)[1].clone() };
        // Still in its section: it stays.
        assert_eq!(follow_target(&open, &f).unwrap().path.as_ref(), "b");
        // Staged whole: it shows where it went.
        let moved = Files::wip("/w".into(), &payload(&[]), &payload(&[("b", "A")]));
        assert_eq!(follow_target(&open, &moved).unwrap().origin, Origin::Wip { worktree: "/w".into(), staged: true });
        // Discarded: nothing.
        let gone = Files::wip("/w".into(), &payload(&[]), &payload(&[]));
        assert_eq!(follow_target(&open, &gone), None);
    }

    #[test]
    fn after_a_stage_the_view_moves_to_the_next_file_or_the_one_above() {
        let from = AdvanceFrom { staged: false, path: "b".into(), order: vec!["a".into(), "b".into(), "c".into()] };
        let f = Files::wip("/w".into(), &payload(&[("a", "M"), ("c", "M")]), &payload(&[("b", "M")]));
        assert_eq!(advance_target(&from, &f).unwrap().unwrap().path.as_ref(), "c");
        let last = AdvanceFrom { path: "c".into(), ..from.clone() };
        let f = Files::wip("/w".into(), &payload(&[("a", "M")]), &payload(&[("c", "M")]));
        assert_eq!(advance_target(&last, &f).unwrap().unwrap().path.as_ref(), "a");
        // The section emptied by a stage: it follows the file. By a discard: the other section's first.
        let only = AdvanceFrom { path: "b".into(), staged: false, order: vec!["b".into()] };
        let f = Files::wip("/w".into(), &payload(&[]), &payload(&[("b", "M")]));
        assert_eq!(advance_target(&only, &f), None);
        let f = Files::wip("/w".into(), &payload(&[]), &payload(&[("z", "M")]));
        assert_eq!(advance_target(&only, &f).unwrap().unwrap().path.as_ref(), "z");
        // Still there: a partial write.
        let f = Files::wip("/w".into(), &payload(&[("b", "M")]), &payload(&[("b", "M")]));
        assert_eq!(advance_target(&only, &f), None);
    }

    #[test]
    fn the_commit_button_says_what_it_does_and_why_it_cant() {
        let base = CommitState { staged: 0, unstaged: 0, conflicted: 0, amend: false, in_merge: false, summary: "", staging: false };
        assert_eq!(commit_button(&base).reason.as_deref(), Some("Nothing to commit"));
        assert_eq!(commit_button(&base).label, TYPE_A_MESSAGE);
        let staged = CommitState { staged: 2, summary: "s", ..base };
        assert_eq!(commit_button(&staged), CommitView { label: "Commit changes to 2 files".into(), reason: None, stage_all: false });
        let all = CommitState { unstaged: 3, summary: "s", ..base };
        assert_eq!(commit_button(&all), CommitView { label: "Stage all & commit".into(), reason: None, stage_all: true });
        assert_eq!(commit_button(&CommitState { amend: true, summary: "s", ..base }).label, "Amend previous commit");
        assert_eq!(commit_button(&CommitState { in_merge: true, summary: "s", ..base }).label, "Commit and merge");
        assert_eq!(commit_button(&CommitState { staged: 1, conflicted: 2, summary: "s", ..base }).reason.as_deref(), Some("Resolve 2 conflicted files first"));
        assert_eq!(commit_button(&CommitState { staged: 1, summary: "s", staging: true, ..base }).reason.as_deref(), Some(STAGING_BUSY));
    }

    #[test]
    fn a_message_splits_into_the_two_fields() {
        assert_eq!(Draft::of_message("one\r\n\r\n\r\nbody\n"), Draft { summary: "one".into(), description: "body".into() });
        assert_eq!(Draft { summary: "s".into(), description: "d".into() }.message(), "s\n\nd");
        assert_eq!(Draft { summary: "s".into(), description: " ".into() }.message(), "s");
    }

    #[test]
    fn drafts_read_back_and_empty_ones_go() {
        let mut d = std::collections::HashMap::new();
        d.insert("/w".to_string(), Draft { summary: "s".into(), description: "d".into() });
        d.insert("/e".to_string(), Draft::default());
        let back = drafts_of(&drafts_json(&d));
        assert_eq!(back.len(), 1);
        assert_eq!(back["/w"].description, "d");
    }

    fn status(f: &Fixture) -> String {
        f.git(&["status", "--porcelain"])
    }

    #[tokio::test]
    async fn stage_unstage_and_discard_reach_the_repository() {
        let f = Fixture::new().await;
        f.write("a.txt", "changed\n");
        f.write("new.txt", "new\n");
        let (repo, worktree) = (f.repo, f.worktree.clone());
        let r = f.ok(call("stage", json!({ "repo": repo, "worktree": worktree, "paths": ["a.txt"] }))).await;
        assert!(status(&f).contains("M  a.txt"), "{}", status(&f));
        assert_eq!(r["wip"]["staged"]["files"][0]["path"], "a.txt", "the answer carries the fresh lists");
        assert!(r["staging"]["undo"].as_str().is_some(), "and the staging log");
        f.ok(call("unstage", json!({ "repo": repo, "worktree": worktree, "paths": ["a.txt"], "oldPaths": [] }))).await;
        assert!(status(&f).contains(" M a.txt"), "{}", status(&f));
        f.ok(call("stageAll", json!({ "repo": repo, "worktree": worktree }))).await;
        assert!(status(&f).contains("A  new.txt"));
        f.ok(call("stagingUndo", json!({ "repo": repo, "worktree": worktree }))).await;
        assert!(status(&f).contains("?? new.txt"), "{}", status(&f));
        f.ok(call("stagingRedo", json!({ "repo": repo, "worktree": worktree }))).await;
        assert!(status(&f).contains("A  new.txt"));
        f.ok(call("unstageAll", json!({ "repo": repo, "worktree": worktree }))).await;
        f.ok(call("discard", json!({ "repo": repo, "worktree": worktree, "scope": { "kind": "paths", "paths": ["a.txt"] } }))).await;
        assert!(!status(&f).contains("a.txt"));
        f.ok(call("discard", json!({ "repo": repo, "worktree": worktree, "scope": { "kind": "unstaged" } }))).await;
        assert_eq!(status(&f), "", "the untracked file goes too");
    }

    #[tokio::test]
    async fn discard_all_is_undoable_and_takes_no_more_than_was_confirmed() {
        let f = Fixture::new().await;
        f.write("a.txt", "changed\n");
        f.git(&["add", "a.txt"]);
        f.write("b.txt", "other\n");
        let (repo, worktree) = (f.repo, f.worktree.clone());
        // A change it didn't confirm refuses it.
        let e = f.send(call("discard", json!({ "repo": repo, "worktree": worktree, "scope": { "kind": "all", "confirmed": ["a.txt"] } }))).await.unwrap_err();
        assert_eq!(e.kind, gitvolt_core::error::GbErrorKind::Stale);
        let r = f.ok(call("discard", json!({ "repo": repo, "worktree": worktree, "scope": { "kind": "all", "confirmed": ["a.txt", "b.txt"] } }))).await;
        assert_eq!(status(&f), "");
        let entry = r["journal"]["undo"]["entry"].as_u64().unwrap();
        f.ok(call("undo", json!({ "repo": repo, "worktree": worktree, "entry": entry }))).await;
        assert!(status(&f).contains("a.txt") && status(&f).contains("b.txt"), "{}", status(&f));
    }

    #[tokio::test]
    async fn a_commit_runs_with_its_message_and_a_stale_head_refuses() {
        let f = Fixture::new().await;
        f.write("a.txt", "changed\n");
        let head = f.git(&["rev-parse", "HEAD"]);
        let (repo, worktree) = (f.repo, f.worktree.clone());
        let body = |h: &str, amend: bool, stage_all: bool, summary: &str| call("commit", json!({ "repo": repo, "worktree": worktree, "summary": summary, "description": "body", "amend": amend, "stageAll": stage_all, "expect": { "head": h, "refs": {} } }));
        let wrong = f.send(body("0000000000000000000000000000000000000000", false, true, "x")).await;
        assert!(wrong.is_err());
        let r = f.ok(body(&head, false, true, "A commit")).await;
        assert_eq!(f.git(&["log", "-1", "--format=%B"]), "A commit\n\nbody");
        assert_eq!(r["outcome"]["oid"].as_str().unwrap(), f.git(&["rev-parse", "HEAD"]));
        // Amend replaces it, and the author is the repository's own.
        let head = f.git(&["rev-parse", "HEAD"]);
        f.ok(body(&head, true, false, "Amended")).await;
        assert_eq!(f.git(&["log", "-1", "--format=%s %an"]), "Amended Ada Fixture");
        assert_eq!(f.git(&["rev-list", "--count", "HEAD"]), "3");
    }

    #[tokio::test]
    async fn a_failing_hook_keeps_the_commit_from_happening_and_names_itself() {
        let f = Fixture::new().await;
        f.write("a.txt", "changed\n");
        let hook = f.path.join(".git/hooks/pre-commit");
        std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
        std::fs::write(&hook, "#!/bin/sh\necho nope >&2\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let head = f.git(&["rev-parse", "HEAD"]);
        let e = f.send(call("commit", json!({ "repo": f.repo, "worktree": f.worktree, "summary": "s", "description": "", "amend": false, "stageAll": true, "expect": { "head": head, "refs": {} } }))).await.unwrap_err();
        assert_eq!(e.kind, gitvolt_core::error::GbErrorKind::HookFailed);
        assert_eq!(crate::write::describe_error(&e).unwrap().0, "pre-commit hook failed");
        assert_eq!(f.git(&["rev-parse", "HEAD"]), head);
    }
}

#[cfg(test)]
mod ui_tests {
    use super::*;
    use crate::app::tests::with_rows;
    use crate::model::{GraphView, RowView};
    use gitvolt_graph::NodeKind;
    use serde_json::json;
    use std::prelude::v1::test;

    fn payload(files: &[(&str, &str)]) -> Value {
        let list: Vec<Value> = files.iter().map(|(p, s)| json!({ "path": p, "status": s, "additions": 1, "deletions": 0, "old": {"kind": "absent"}, "new": {"kind": "worktree", "worktree": "/w"} })).collect();
        json!({ "files": list, "added": files.len(), "deleted": 0 })
    }

    fn wip_row() -> RowView {
        RowView { id: "wip".into(), kind: NodeKind::Wip, lane: 0, color: 0, segments: Vec::new(), summary: SharedString::default(), author: SharedString::default(), date: SharedString::default(), sha: SharedString::default(), initials: SharedString::default(), avatar: 0, chips: Vec::new(), worktree: Some("/w".into()), email: String::new(), parents: vec!["abc".into()], stash_branch: None, wip: Some(Default::default()) }
    }

    /// A window with the WIP row selected and its lists in.
    fn wip_window(cx: &mut TestAppContext, unstaged: &[(&str, &str)], staged: &[(&str, &str)]) -> (WindowHandle<GitVolt>, VisualTestContext, tokio::runtime::Runtime) {
        let (window, mut cx, rt) = with_rows(cx, 0);
        let (u, s) = (payload(unstaged), payload(staged));
        window
            .update(&mut cx, |v, _, cx| {
                let by_sha = std::collections::HashMap::new();
                v.tabs[0].worktree = "/w".into();
                v.tabs[0].graph = Some(Rc::new(GraphView { rows: vec![wip_row()], max_lanes: 1, head_row: None, head_branch: "main".into(), pinned: SharedString::default(), truncated: false, by_sha, head_sha: Some("abc".into()), worktrees: Vec::new(), in_progress: Default::default() }));
                v.tabs[0].selected = Some(0);
                v.put_wip(1, "/w".into(), &u, &s, cx);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        (window, cx, rt)
    }

    fn sent(window: &WindowHandle<GitVolt>, cx: &VisualTestContext) -> Vec<Value> {
        window.read_with(cx, |v, _| v.sent.borrow().clone()).unwrap()
    }

    #[gpui::test]
    fn the_activity_log_opens_with_its_entries_and_commands(cx: &mut TestAppContext) {
        let (window, mut cx, _rt) = wip_window(cx, &[("a.txt", "M")], &[]);
        window
            .update(&mut cx, |v, window, cx| {
                v.ops.activity.push(crate::ops::ActivityEntry { op: 1, kind: gitvolt_core::events::OpKind::Fetch, label: "fetch origin".into(), background: false, duration_ms: 10, outcome: gitvolt_core::events::OpOutcome::Failed, message: Some("no route".into()), command: None, output: Vec::new(), remote: Vec::new() });
                v.activity.commands = vec![json!({ "id": 1, "args": ["fetch"], "ok": false, "stderr": "no route" })];
                v.open_activity(None, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |v, _, cx| {
                assert_eq!(v.activity_shown.borrow().ops.activity.len(), 1, "it drew from what it was given");
                v.activity.commands_view = true;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        window.read_with(&cx, |v, _| assert!(v.activity_shown.borrow().activity.commands_view)).unwrap();
    }

    #[gpui::test]
    fn stage_all_sends_the_core_its_request_and_the_answer_moves_the_lists(cx: &mut TestAppContext) {
        let (window, mut cx, _rt) = wip_window(cx, &[("a.txt", "M"), ("b.txt", "A")], &[]);
        window.update(&mut cx, |v, window, cx| crate::commands::invoke("stage.all", v, window, cx)).unwrap();
        assert_eq!(sent(&window, &cx)[0], json!({ "method": "stageAll", "params": { "repo": 1, "worktree": "/w" } }));
        // The commit button holds off until it's answered.
        window.read_with(&cx, |v, cx| assert_eq!(commit_view_of(&v.tabs[0], cx).reason.as_deref(), Some(STAGING_BUSY))).unwrap();
        let answer = json!({ "outcome": null, "journal": { "undo": null, "redo": null, "undoBlocked": null, "redoBlocked": null, "banners": [], "paused": null }, "staging": { "undo": "stage all", "redo": null, "off": null }, "wip": { "worktree": "/w", "unstaged": payload(&[]), "staged": payload(&[("a.txt", "M"), ("b.txt", "A")]), "version": "1" } });
        window.update(&mut cx, |v, _, cx| v.answer_write(Ok(answer), cx)).unwrap();
        window.read_with(&cx, |v, _| {
            let f = v.tabs[0].files.as_ref().unwrap();
            assert_eq!(f.counts(), (2, 0, 0));
            assert_eq!(v.ops.staging(1, "/w").unwrap().undo.as_deref(), Some("stage all"));
            assert_eq!(v.tabs[0].wip.staging, 0);
        })
        .unwrap();
    }

    #[gpui::test]
    fn a_click_on_a_rows_stage_button_stages_that_file(cx: &mut TestAppContext) {
        let (window, mut cx, _rt) = wip_window(cx, &[("a.txt", "M")], &[]);
        window.update(&mut cx, |v, _, cx| v.with_files(cx, |f| f.set_mode(ListMode::Path))).unwrap();
        cx.run_until_parked();
        // The first row of the Unstaged list, its Stage File at the panel's right edge. The panel's top:
        // bars 108 + header 37 + the view bar ~36 + the head 26 + the tool line 25.
        let (w, y) = (1600., 108. + 37. + 36. + 26. + 25. + 13.);
        cx.simulate_mouse_move(point(px(w - 200.), px(y)), None, Modifiers::none());
        cx.run_until_parked();
        cx.simulate_click(point(px(w - 40.), px(y)), Modifiers::none());
        cx.run_until_parked();
        assert_eq!(sent(&window, &cx).first().map(|s| s["method"].clone()), Some(json!("stage")), "{:?}", sent(&window, &cx));
        assert_eq!(sent(&window, &cx)[0]["params"]["paths"], json!(["a.txt"]));
    }

    #[gpui::test]
    fn discard_all_arms_first_and_a_second_click_sends_what_it_confirmed(cx: &mut TestAppContext) {
        let (window, mut cx, _rt) = wip_window(cx, &[("a.txt", "M")], &[("c.txt", "A")]);
        // The header's trash, at the panel's left edge.
        let at = point(px(1600. - 400. + 12. + 11.), px(108. + 18.));
        cx.simulate_mouse_move(at, None, Modifiers::none());
        cx.run_until_parked();
        cx.simulate_click(at, Modifiers::none());
        cx.run_until_parked();
        assert!(cx.update(|_, cx| crate::ui::arm::is_armed(cx)));
        assert!(sent(&window, &cx).is_empty(), "nothing yet");
        cx.update(|_, cx| crate::ui::arm::age_for_test(cx));
        cx.simulate_click(at, Modifiers::none());
        cx.run_until_parked();
        let s = sent(&window, &cx);
        assert_eq!(s[0]["params"]["scope"]["kind"], "all");
        assert_eq!(s[0]["params"]["scope"]["confirmed"], json!(["a.txt", "c.txt"]));
    }

    #[gpui::test]
    fn a_failed_write_says_so_and_a_question_arms_before_it_sends_again(cx: &mut TestAppContext) {
        use gitvolt_core::error::{ErrorDetail, GbError, GbErrorKind};
        let (window, mut cx, _rt) = wip_window(cx, &[("a.txt", "M")], &[]);
        window.update(&mut cx, |v, _, cx| v.reset_to("abc".into(), "hard", ArmOrigin::None, cx)).unwrap();
        let e = GbError { kind: GbErrorKind::DirtyWorktree, message: "Reset main?".into(), command_id: None, stderr: None, detail: Some(ErrorDetail::ResetDiscards { branch: "main".into(), to: "abc".into(), files: 2 }) };
        window.update(&mut cx, |v, _, cx| v.answer_write(Err(e), cx)).unwrap();
        cx.run_until_parked();
        assert!(cx.update(|_, cx| crate::ui::popover::is_open(cx)), "no control to arm: it asks in a popover");
        cx.update(|_, cx| crate::ui::popover::age_for_test(cx));
        // Cancel first, the risky answer's a step away.
        cx.simulate_keystrokes("right enter");
        cx.run_until_parked();
        let s = sent(&window, &cx);
        assert_eq!(s.len(), 2, "sent again: {s:?}");
        assert_eq!(s[1]["params"]["discard"], json!(true));
        // A failure that's no question goes to the toast, with its kind's word.
        let e = GbError { kind: GbErrorKind::Other, message: "It broke".into(), command_id: None, stderr: None, detail: None };
        window.update(&mut cx, |v, _, cx| v.answer_write(Err(e), cx)).unwrap();
        assert_eq!(cx.update(|_, cx| crate::ui::toast::message(cx)).map(|m| m.to_string()).as_deref(), Some("Something went wrong: It broke"));
    }

    #[gpui::test]
    fn typing_in_either_box_is_the_same_draft_and_commit_needs_a_summary(cx: &mut TestAppContext) {
        let (window, mut cx, _rt) = wip_window(cx, &[("a.txt", "M")], &[]);
        window.update(&mut cx, |v, _, cx| v.sync_row_drafts(1, cx)).unwrap();
        let focus = window.read_with(&cx, |v, cx| v.tabs[0].wip.summary.read(cx).focus.clone()).unwrap();
        window.update(&mut cx, |_, window, cx| window.focus(&focus, cx)).unwrap();
        cx.run_until_parked();
        window.read_with(&cx, |v, cx| assert_eq!(commit_view_of(&v.tabs[0], cx).reason.as_deref(), Some("Write a commit summary"))).unwrap();
        cx.simulate_input("Fix");
        cx.run_until_parked();
        window
            .read_with(&cx, |v, cx| {
                assert_eq!(v.drafts["/w"].summary, "Fix");
                assert_eq!(v.tabs[0].row_drafts["/w"].read(cx).text(), "Fix", "the row's box shows it");
                assert_eq!(commit_view_of(&v.tabs[0], cx), CommitView { label: "Stage all & commit".into(), reason: None, stage_all: true });
            })
            .unwrap();
        // Enter moves to the description; Ctrl+Enter commits from there.
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        cx.simulate_input("Body");
        cx.simulate_keystrokes("ctrl-enter");
        cx.run_until_parked();
        let s = sent(&window, &cx);
        assert_eq!(s[0]["method"], "commit");
        assert_eq!((s[0]["params"]["summary"].as_str(), s[0]["params"]["description"].as_str(), s[0]["params"]["stageAll"].as_bool()), (Some("Fix"), Some("Body"), Some(true)));
        assert_eq!(s[0]["params"]["expect"]["head"], "abc");
        // While it runs, the fields wait; the answer clears the draft.
        window.read_with(&cx, |v, _| assert!(v.tabs[0].wip.committing)).unwrap();
        let answer = json!({ "outcome": { "oid": "def" }, "journal": null, "staging": null, "wip": null });
        window.update(&mut cx, |v, _, cx| v.answer_write(Ok(answer), cx)).unwrap();
        window
            .read_with(&cx, |v, cx| {
                assert!(!v.tabs[0].wip.committing);
                assert!(!v.drafts.contains_key("/w"));
                assert_eq!(v.tabs[0].wip.summary.read(cx).text(), "");
            })
            .unwrap();
    }

    #[gpui::test]
    fn amend_puts_the_draft_aside_and_a_head_move_ends_it(cx: &mut TestAppContext) {
        let (window, mut cx, _rt) = wip_window(cx, &[("a.txt", "M")], &[]);
        window
            .update(&mut cx, |v, _, cx| {
                v.drafts.insert("/w".into(), Draft { summary: "Mine".into(), description: String::new() });
                v.show_draft(1, cx);
                v.start_amend(1, Draft { summary: "Head's".into(), description: "x".into() }, "abc".into(), cx);
            })
            .unwrap();
        window
            .read_with(&cx, |v, cx| {
                assert_eq!(v.tabs[0].wip.summary.read(cx).text(), "Head's");
                assert_eq!(commit_view_of(&v.tabs[0], cx).label, "Amend previous commit");
            })
            .unwrap();
        window.update(&mut cx, |v, _, cx| {
            let g = Rc::get_mut(v.tabs[0].graph.as_mut().unwrap()).unwrap();
            g.rows[0].parents = vec!["moved".into()];
            v.check_amend(1, cx);
        }).unwrap();
        window.read_with(&cx, |v, cx| {
            assert!(v.tabs[0].wip.amend.is_none());
            assert_eq!(v.tabs[0].wip.summary.read(cx).text(), "Mine");
        }).unwrap();
    }

    #[gpui::test]
    fn leaving_a_dirty_file_asks_and_discard_goes_on(cx: &mut TestAppContext) {
        let (window, mut cx, _rt) = wip_window(cx, &[("a.txt", "M"), ("b.txt", "M")], &[]);
        window
            .update(&mut cx, |v, _, cx| {
                let file = v.tabs[0].files.as_ref().unwrap().changes[0].clone();
                let mut d = DiffState::new(file);
                d.edit = Some(crate::diff::edit::EditState::new("x\n", None, "h".into(), "/w".into(), cx));
                d.edit.as_mut().unwrap().buffer.replace(None, "typed", crate::ui::input::EditKind::Typing);
                v.tabs[0].diff = Some(d);
                v.close_file(cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert!(cx.update(|_, cx| crate::ui::popover::is_open(cx)), "it asks");
        window.read_with(&cx, |v, _| assert!(v.tabs[0].diff.is_some(), "still there")).unwrap();
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        window.read_with(&cx, |v, _| assert!(v.tabs[0].diff.is_some(), "Cancel keeps it")).unwrap();
    }
}
