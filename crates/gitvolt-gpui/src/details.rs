//! The right panel: a commit's ids, people and message, and its changed files by path or as a tree
//! (`ui/src/details`, `ui/src/files`).

use crate::app::{GitVolt, KeyArea, Tab, icon};
use crate::field::{Field, Typed};
use crate::model::{SHORT_SHA, initials};
use crate::theme;
use gitvolt_core::api::Request;
use gpui::prelude::*;
use gpui::*;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

pub const DETAILS_W: f32 = 400.;
const COMMIT_H: f32 = 188.;
const FILE_ROW_H: f32 = 26.;
/// A tree row: `[chevron | status icon][gap][name]`, indented by level (`TREE`, fileTree.ts).
const TREE_BASE: f32 = 8.;
const TREE_LEVEL: f32 = 20.;

/// A changed file of a commit or a worktree.
#[derive(Debug, Clone, PartialEq)]
pub struct FileChange {
    pub path: SharedString,
    pub old_path: Option<String>,
    pub status: char,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub old: Value,
    pub new: Value,
    pub origin: Origin,
}

/// The list a file came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Commit { id: String },
    Wip { worktree: String, staged: bool },
}

pub fn file_changes(v: &Value, origin: &Origin) -> Vec<FileChange> {
    v["files"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|f| FileChange {
            path: f["path"].as_str().unwrap_or_default().to_string().into(),
            old_path: f["oldPath"].as_str().map(str::to_string),
            status: f["status"].as_str().and_then(|s| s.chars().next()).unwrap_or('M'),
            additions: f["additions"].as_u64().map(|n| n as u32),
            deletions: f["deletions"].as_u64().map(|n| n as u32),
            old: f["old"].clone(),
            new: f["new"].clone(),
            origin: origin.clone(),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListMode {
    Path,
    Tree,
}

/// A row of the file list.
#[derive(Debug, Clone, PartialEq)]
pub enum ListRow {
    Folder { path: SharedString, name: SharedString, depth: u8, collapsed: bool },
    File { index: usize, name: SharedString, dir: SharedString, depth: u8 },
}

#[derive(Default)]
struct Node {
    folders: BTreeMap<String, Node>,
    files: Vec<usize>,
}

fn add_rows(node: &Node, prefix: &str, depth: u8, files: &[FileChange], collapsed: &HashSet<String>, out: &mut Vec<ListRow>) {
    for (name, child) in &node.folders {
        // A folder whose only child is a folder merges into one row (`a/b/c`, spec §9.3).
        let (mut name, mut child) = (name.clone(), child);
        while child.files.is_empty() && child.folders.len() == 1 {
            let (next, grand) = child.folders.iter().next().expect("len is 1");
            name = format!("{name}/{next}");
            child = grand;
        }
        let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        let closed = collapsed.contains(&path);
        out.push(ListRow::Folder { path: path.clone().into(), name: name.into(), depth, collapsed: closed });
        if !closed {
            add_rows(child, &path, depth + 1, files, collapsed, out);
        }
    }
    for &index in &node.files {
        let name = files[index].path.rsplit('/').next().unwrap_or_default().to_string();
        out.push(ListRow::File { index, name: name.into(), dir: SharedString::default(), depth });
    }
}

pub fn list_rows(files: &[FileChange], mode: ListMode, collapsed: &HashSet<String>) -> Vec<ListRow> {
    if mode == ListMode::Path {
        return files
            .iter()
            .enumerate()
            .map(|(index, f)| {
                let (dir, name) = f.path.rsplit_once('/').unwrap_or(("", &f.path));
                ListRow::File { index, name: name.to_string().into(), dir: dir.to_string().into(), depth: 0 }
            })
            .collect();
    }
    let mut root = Node::default();
    for (i, f) in files.iter().enumerate() {
        let mut parts: Vec<&str> = f.path.split('/').collect();
        parts.pop();
        let mut node = &mut root;
        for p in parts {
            node = node.folders.entry(p.to_string()).or_default();
        }
        node.files.push(i);
    }
    let mut out = Vec::new();
    add_rows(&root, "", 0, files, collapsed, &mut out);
    out
}

pub fn all_folders(files: &[FileChange]) -> HashSet<String> {
    list_rows(files, ListMode::Tree, &HashSet::new())
        .into_iter()
        .filter_map(|r| match r {
            ListRow::Folder { path, .. } => Some(path.to_string()),
            ListRow::File { .. } => None,
        })
        .collect()
}

pub fn status_icon(status: char) -> (&'static str, Hsla) {
    match status {
        'A' | 'C' => ("status-added", theme::status_added()),
        'D' => ("status-deleted", theme::status_deleted()),
        'R' => ("status-renamed", theme::blue()),
        'U' | 'X' => ("status-conflicted", theme::status_deleted()),
        _ => ("status-modified", theme::status_modified()),
    }
}

/// A file only "View all files" lists: it has nothing to diff against.
pub const UNCHANGED: char = ' ';

const STATUS_ORDER: &str = "AMTRCDUX";

/// An unchanged file of commit `id`, as File View opens it.
fn unchanged_file(path: &str, id: &str) -> FileChange {
    FileChange { path: path.to_string().into(), old_path: None, status: UNCHANGED, additions: None, deletions: None, old: serde_json::json!({ "kind": "absent" }), new: serde_json::json!({ "kind": "atCommit", "commit": id }), origin: Origin::Commit { id: id.to_string() } }
}

/// What's listed: the changed files and, with `tree`, the others; narrowed by `query` (lower case).
/// Path mode can sort by status, then path, the unchanged last.
pub fn listed_files(changed: &[FileChange], tree: Option<(&[String], &str)>, query: &str, by_status: bool) -> Vec<FileChange> {
    let mut all: Vec<FileChange> = changed.to_vec();
    if let Some((paths, id)) = tree {
        all.extend(paths.iter().filter(|p| !changed.iter().any(|c| c.path.as_ref() == p.as_str())).map(|p| unchanged_file(p, id)));
    }
    if !query.is_empty() {
        all.retain(|f| f.path.to_lowercase().contains(query));
    }
    let rank = |f: &FileChange| if by_status { STATUS_ORDER.find(f.status).unwrap_or(STATUS_ORDER.len()) } else { 0 };
    if tree.is_some() || by_status {
        all.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.path.cmp(&b.path)));
    }
    all
}

/// The key cursor in the list.
#[derive(Debug, Clone, PartialEq)]
pub enum Cursor {
    File(usize),
    Folder(SharedString),
}

/// What the panel shows for the selected row, read when it's selected.
pub struct Commit {
    pub id: SharedString,
    /// Every parent, whole.
    pub parents: Vec<SharedString>,
    pub author: SharedString,
    pub email: String,
    pub date: SharedString,
    pub co_authors: Vec<SharedString>,
    pub summary: SharedString,
    pub body: Vec<SharedString>,
    pub signed: bool,
}

/// The commit's or worktree's files, and how they're listed.
pub struct Files {
    /// What's listed, and the rows it makes.
    pub changes: Rc<Vec<FileChange>>,
    /// The commit's or worktree's own changes.
    pub changed: Rc<Vec<FileChange>>,
    pub added: u32,
    pub deleted: u32,
    pub mode: ListMode,
    pub by_status: bool,
    pub collapsed: HashSet<String>,
    pub rows: Rc<Vec<ListRow>>,
    pub list: UniformListScrollHandle,
    pub cursor: Option<Cursor>,
    /// The commit whose tree "View all files" can list, and every path of it, or why not.
    pub commit: Option<String>,
    pub tree: Option<Vec<String>>,
    pub tree_error: Option<SharedString>,
    /// "View all files"' filter: never kept when it's turned off.
    pub filter: Field,
    pub filter_focused: bool,
}

impl Files {
    pub fn new(changes: Vec<FileChange>, added: u32, deleted: u32) -> Self {
        let (mode, collapsed) = (ListMode::Tree, HashSet::new());
        let rows = Rc::new(list_rows(&changes, mode, &collapsed));
        let changes = Rc::new(changes);
        Self { changed: changes.clone(), changes, added, deleted, mode, by_status: false, collapsed, rows, list: UniformListScrollHandle::new(), cursor: None, commit: None, tree: None, tree_error: None, filter: Field::default(), filter_focused: false }
    }

    fn relist(&mut self) {
        self.rows = Rc::new(list_rows(&self.changes, self.mode, &self.collapsed));
    }

    /// What's listed anew: the tree, the filter or the sort changed. The cursor stays on its file.
    pub fn rebuild(&mut self) {
        let on = match &self.cursor {
            Some(Cursor::File(i)) => self.changes.get(*i).cloned(),
            _ => None,
        };
        let query = self.filter.text.trim().to_lowercase();
        let tree = self.tree.as_deref().zip(self.commit.as_deref());
        let by_status = self.by_status && self.mode == ListMode::Path;
        self.changes = Rc::new(listed_files(&self.changed, tree, &query, by_status));
        if let Some(file) = on {
            self.cursor = self.changes.iter().position(|f| f.path == file.path && f.origin == file.origin).map(Cursor::File);
        }
        self.relist();
    }

    /// Every file in display order, whatever's collapsed.
    fn all_files(&self) -> Vec<usize> {
        list_rows(&self.changes, self.mode, &HashSet::new()).iter().filter_map(|r| if let ListRow::File { index, .. } = r { Some(*index) } else { None }).collect()
    }

    /// ↑ / ↓ from the cursor, past a folder and wrapping around: the file there.
    pub fn step_file(&self, down: bool) -> Option<usize> {
        let order = self.all_files();
        if order.is_empty() {
            return None;
        }
        let at = match &self.cursor {
            Some(Cursor::File(i)) => order.iter().position(|o| o == i),
            Some(Cursor::Folder(path)) => {
                // The folder's first file going down, the file before it going up.
                let first = order.iter().position(|i| self.changes[*i].path.starts_with(&format!("{path}/")));
                return first.map(|f| {
                    if down {
                        let past = order[f..].iter().position(|i| !self.changes[*i].path.starts_with(&format!("{path}/")));
                        if self.collapsed.contains(path.as_ref()) { order[(f + past.unwrap_or(order.len() - f)) % order.len()] } else { order[f] }
                    } else {
                        order[(f + order.len() - 1) % order.len()]
                    }
                });
            }
            None => None,
        };
        Some(match at {
            None => *if down { order.first() } else { order.last() }?,
            Some(a) if down => order[(a + 1) % order.len()],
            Some(a) => order[(a + order.len() - 1) % order.len()],
        })
    }

    /// "Previous/Next changed file": the next one with changes in the rows as shown, wrapping around.
    pub fn jump_changed(&self, down: bool) -> Option<usize> {
        let rows: Vec<usize> = self.rows.iter().filter_map(|r| if let ListRow::File { index, .. } = r { Some(*index) } else { None }).filter(|i| self.changes[*i].status != UNCHANGED).collect();
        let at = match &self.cursor {
            Some(Cursor::File(i)) => {
                let all: Vec<usize> = self.rows.iter().filter_map(|r| if let ListRow::File { index, .. } = r { Some(*index) } else { None }).collect();
                let pos = all.iter().position(|o| o == i)?;
                if down { all[pos + 1..].iter().find(|o| rows.contains(o)).copied() } else { all[..pos].iter().rev().find(|o| rows.contains(o)).copied() }
            }
            _ => None,
        };
        at.or_else(|| if down { rows.first().copied() } else { rows.last().copied() })
    }

    /// Opens the folders above file `index`, and puts the list there.
    pub fn reveal(&mut self, index: usize) {
        if let Some(f) = self.changes.get(index) {
            let path = f.path.to_string();
            self.collapsed.retain(|c| !path.starts_with(&format!("{c}/")));
        }
        self.relist();
        if let Some(row) = self.rows.iter().position(|r| matches!(r, ListRow::File { index: i, .. } if *i == index)) {
            self.list.scroll_to_item(row, ScrollStrategy::Nearest);
        }
    }

    pub fn set_mode(&mut self, mode: ListMode) {
        self.mode = mode;
        self.rebuild();
    }

    pub fn toggle_folder(&mut self, path: &str) {
        if !self.collapsed.remove(path) {
            self.collapsed.insert(path.to_string());
        }
        self.relist();
    }

    pub fn toggle_all(&mut self) {
        self.collapsed = if self.collapsed.is_empty() { all_folders(&self.changes) } else { HashSet::new() };
        if let Some(Cursor::Folder(_)) = self.cursor {
            self.cursor = None;
        }
        self.relist();
    }
}

impl GitVolt {
    /// Reads the panel for commit `id`.
    pub fn read_commit(&mut self, repo: u32, id: String, ix: usize, cx: &mut Context<Self>) {
        let api = self.api.clone();
        let read = self.rt.spawn(async move {
            let details = api.dispatch(Request::CommitDetails { repo, id: id.clone() }).await?;
            let message = api.dispatch(Request::CommitMessage { repo, id: id.clone() }).await?;
            let files = api.dispatch(Request::FileList { repo, spec: gitvolt_core::diff::DiffSpec::Commit { id: id.clone(), parent: 0 } }).await?;
            Ok::<_, gitvolt_core::error::GbError>((details, message, files, id))
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok((details, message, files, id))) = read.await else { return };
            this.update(cx, |this, cx| {
                let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) else { return };
                if tab.selected != Some(ix) {
                    return;
                }
                tab.commit = Some(commit_of(&details, &message));
                let count = |k: &str| files[k].as_u64().unwrap_or(0) as u32;
                let mut list = Files::new(file_changes(&files, &Origin::Commit { id: id.clone() }), count("added"), count("deleted"));
                list.commit = Some(id);
                list.by_status = tab.sort_by_status;
                tab.files = Some(list);
                if tab.all_files {
                    this.read_tree(repo, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Reads the panel for a WIP row: the worktree's unstaged changes, then its staged ones.
    pub fn read_wip(&mut self, repo: u32, worktree: String, ix: usize, cx: &mut Context<Self>) {
        let api = self.api.clone();
        let tree = worktree.clone();
        let read = self.rt.spawn(async move {
            let unstaged = api.dispatch(Request::FileList { repo, spec: gitvolt_core::diff::DiffSpec::Wip { worktree: worktree.clone(), staged: false } }).await?;
            let staged = api.dispatch(Request::FileList { repo, spec: gitvolt_core::diff::DiffSpec::Wip { worktree, staged: true } }).await?;
            Ok::<_, gitvolt_core::error::GbError>((unstaged, staged))
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok((unstaged, staged))) = read.await else { return };
            this.update(cx, |this, cx| {
                let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) else { return };
                if tab.selected != Some(ix) {
                    return;
                }
                let mut files = file_changes(&unstaged, &Origin::Wip { worktree: tree.clone(), staged: false });
                files.extend(file_changes(&staged, &Origin::Wip { worktree: tree.clone(), staged: true }));
                let sum = |k: &str| (unstaged[k].as_u64().unwrap_or(0) + staged[k].as_u64().unwrap_or(0)) as u32;
                let mut list = Files::new(files, sum("added"), sum("deleted"));
                list.by_status = tab.sort_by_status;
                tab.files = Some(list);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl GitVolt {
    /// "View all files": every path in the commit's tree.
    pub fn read_tree(&mut self, repo: u32, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == repo) else { return };
        let Some(id) = tab.files.as_ref().and_then(|f| f.commit.clone()) else { return };
        if cfg!(test) {
            return;
        }
        let api = self.api.clone();
        let asked = id.clone();
        let read = self.rt.spawn(async move { api.dispatch(Request::TreeFiles { repo, id: asked }).await });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                let Some(files) = this.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.files.as_mut()).filter(|f| f.commit.as_ref() == Some(&id)) else { return };
                match result {
                    Ok(Ok(v)) => {
                        files.tree = Some(v.as_array().map_or(&[][..], Vec::as_slice).iter().filter_map(|p| p.as_str().map(str::to_string)).collect());
                        files.tree_error = None;
                        files.rebuild();
                    }
                    Ok(Err(e)) => files.tree_error = Some(e.message.into()),
                    Err(e) => files.tree_error = Some(e.to_string().into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn toggle_all_files(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        tab.all_files = !tab.all_files;
        let (repo, on) = (tab.id, tab.all_files);
        let Some(files) = tab.files.as_mut() else { return };
        if !on {
            files.tree = None;
            files.tree_error = None;
            files.filter = Field::default();
            files.filter_focused = false;
            files.rebuild();
        }
        if on {
            self.read_tree(repo, cx);
        }
        cx.notify();
    }

    fn with_files(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Files)) {
        if let Some(files) = self.tabs.get_mut(self.active).and_then(|t| t.files.as_mut()) {
            f(files);
            cx.notify();
        }
    }

    /// Goes to a file of the list: the cursor's there and it opens.
    pub fn move_to_file(&mut self, index: usize, cx: &mut Context<Self>) {
        self.with_files(cx, |f| {
            f.reveal(index);
            f.cursor = Some(Cursor::File(index));
        });
        self.open_file(index, cx);
    }

    /// ↑ / ↓ in the list.
    pub fn step_files(&mut self, down: bool, cx: &mut Context<Self>) {
        let next = self.tabs.get(self.active).and_then(|t| t.files.as_ref()).and_then(|f| f.step_file(down));
        if let Some(index) = next {
            self.move_to_file(index, cx);
        }
    }

    pub fn page_files(&mut self, down: bool, page: usize, cx: &mut Context<Self>) {
        let Some(files) = self.tabs.get(self.active).and_then(|t| t.files.as_ref()) else { return };
        let visible: Vec<usize> = files.rows.iter().filter_map(|r| if let ListRow::File { index, .. } = r { Some(*index) } else { None }).collect();
        let at = match &files.cursor {
            Some(Cursor::File(i)) => visible.iter().position(|o| o == i).unwrap_or(0),
            _ => 0,
        };
        let to = if down { (at + page).min(visible.len().saturating_sub(1)) } else { at.saturating_sub(page) };
        if let Some(index) = visible.get(to).copied() {
            self.move_to_file(index, cx);
        }
    }

    /// A key while the list has the keyboard. Whether it took it.
    pub fn file_list_key(&mut self, k: &Keystroke, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active) else { return false };
        if tab.keys != KeyArea::Files {
            return false;
        }
        let open_path = tab.diff.as_ref().map(|d| d.file.path.clone());
        let Some(files) = tab.files.as_mut() else { return false };
        if k.modifiers.control || k.modifiers.alt || k.modifiers.platform {
            return false;
        }
        let cursor = files.cursor.clone();
        let folder = match &cursor {
            Some(Cursor::Folder(path)) => Some((path.clone(), files.collapsed.contains(path.as_ref()))),
            _ => None,
        };
        let file = match &cursor {
            Some(Cursor::File(i)) => Some(*i),
            _ => None,
        };
        match k.key.as_str() {
            "left" => match folder {
                Some((path, false)) => self.with_files(cx, |f| f.toggle_folder(&path)),
                _ => self.close_file(cx),
            },
            "right" => match (folder, file) {
                (Some((path, true)), _) => self.with_files(cx, |f| f.toggle_folder(&path)),
                (Some((path, false)), _) => {
                    // On to its first child.
                    let at = files.rows.iter().position(|r| matches!(r, ListRow::Folder { path: p, .. } if *p == path));
                    match at.and_then(|a| files.rows.get(a + 1)).cloned() {
                        Some(ListRow::File { index, .. }) => self.move_to_file(index, cx),
                        Some(ListRow::Folder { path: child, .. }) if child.starts_with(path.as_ref()) => self.with_files(cx, |f| f.cursor = Some(Cursor::Folder(child))),
                        _ => {}
                    }
                }
                (None, Some(i)) if files.changes.get(i).map(|f| &f.path) != open_path.as_ref() => self.open_file(i, cx),
                _ => {}
            },
            "enter" | "space" => match (folder, file) {
                (Some((path, _)), _) => self.with_files(cx, |f| f.toggle_folder(&path)),
                // Pressed on the open file, it closes.
                (None, Some(i)) if files.changes.get(i).map(|f| &f.path) == open_path.as_ref() => self.close_file(cx),
                (None, Some(i)) => self.open_file(i, cx),
                _ => {}
            },
            "home" | "end" => {
                let order: Vec<usize> = files.rows.iter().filter_map(|r| if let ListRow::File { index, .. } = r { Some(*index) } else { None }).collect();
                let to = if k.key == "home" { order.first() } else { order.last() }.copied();
                if let Some(index) = to {
                    self.move_to_file(index, cx);
                }
            }
            _ => return false,
        }
        true
    }

    /// A key for the filter, if it has the keyboard. Whether it took it.
    pub fn filter_key(&mut self, k: &Keystroke, cx: &mut Context<Self>) -> bool {
        let Some(files) = self.tabs.get_mut(self.active).and_then(|t| t.files.as_mut()).filter(|f| f.filter_focused) else { return false };
        if k.modifiers.control && k.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                files.filter.paste(&text);
                files.rebuild();
                cx.notify();
            }
            return true;
        }
        match files.filter.key(k) {
            Typed::None => false,
            Typed::Enter => true,
            Typed::Changed => {
                files.rebuild();
                cx.notify();
                true
            }
        }
    }

    /// Esc in the filter: it clears first; empty, it's the app's Esc as usual. Whether it took it.
    pub fn filter_escape(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(files) = self.tabs.get_mut(self.active).and_then(|t| t.files.as_mut()).filter(|f| f.filter_focused) else { return false };
        if files.filter.text.is_empty() {
            files.filter_focused = false;
            return false;
        }
        files.filter = Field::default();
        files.rebuild();
        cx.notify();
        true
    }

    /// A parent's id: selects its commit, or says it isn't loaded.
    fn go_to_commit(&mut self, sha: SharedString, cx: &mut Context<Self>) {
        let at = self.tabs.get(self.active).and_then(|t| t.graph.as_ref()?.by_sha.get(sha.as_ref()).copied());
        match at {
            Some(at) => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.history = None;
                    tab.diff = None;
                }
                self.select(at, cx);
            }
            None => self.show_toast("Not in the loaded history", cx),
        }
    }
}

fn commit_of(details: &Value, message: &Value) -> Commit {
    let short = |s: &str| s.chars().take(SHORT_SHA).collect::<String>();
    Commit {
        id: short(details["id"].as_str().unwrap_or_default()).into(),
        parents: details["parents"].as_array().map_or(&[][..], Vec::as_slice).iter().filter_map(|p| p.as_str()).map(|p| p.to_string().into()).collect(),
        author: details["author"]["name"].as_str().unwrap_or_default().to_string().into(),
        email: details["author"]["email"].as_str().unwrap_or_default().to_string(),
        date: crate::model::format_date(details["committer"]["time"].as_i64().unwrap_or(0)).into(),
        co_authors: details["coAuthors"].as_array().map_or(&[][..], Vec::as_slice).iter().filter_map(|c| c["name"].as_str()).map(|n| n.to_string().into()).collect(),
        summary: message["summary"].as_str().unwrap_or_default().to_string().into(),
        body: message["body"].as_str().unwrap_or_default().trim().split('\n').filter(|_| message["body"].as_str().is_some_and(|b| !b.trim().is_empty())).map(|l| l.to_string().into()).collect(),
        signed: details["signed"].as_bool() == Some(true),
    }
}

pub fn avatar(name: &SharedString, email: &str, size: f32) -> Div {
    let lane = crate::model::avatar_lane(name, email, theme::LANES.len());
    div().flex().flex_none().items_center().justify_center().size(px(size)).rounded_full().bg(theme::lane(lane)).text_color(theme::lane_text(lane)).text_size(px((size * 0.4).round().max(7.))).font_weight(FontWeight::SEMIBOLD).child(initials(name))
}

pub fn segmented(items: Vec<AnyElement>) -> Div {
    div().flex().flex_none().border_1().border_color(theme::section_border()).rounded(px(4.)).overflow_hidden().children(items)
}

pub fn segment_with_icon(id: impl Into<ElementId>, glyph: &'static str, label: &'static str, on: bool) -> Stateful<Div> {
    segment_base(id, on, true).child(icon(glyph, 12., if on { theme::text_selected() } else { theme::text_secondary() })).child(label)
}

pub fn segment(id: impl Into<ElementId>, label: &'static str, on: bool, enabled: bool) -> Stateful<Div> {
    segment_base(id, on, enabled).child(label)
}

fn segment_base(id: impl Into<ElementId>, on: bool, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(4.))
        .px(px(8.))
        .py(px(2.))
        .when(on, |d| d.bg(theme::selected_row()).text_color(theme::text_selected()))
        .when(!on, |d| d.text_color(theme::text_secondary()))
        .when(!on && enabled, |d| d.hover(|d| d.text_color(theme::text_selected())))
        .when(!enabled, |d| d.opacity(0.4))
}

fn commit_view(c: &Commit, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let parents = c.parents.iter().enumerate().map(|(i, p)| {
        let sha = p.clone();
        let short: String = p.chars().take(SHORT_SHA).collect();
        div().id(("parent", i)).font_family("JetBrains Mono").text_color(theme::text_selected()).cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.go_to_commit(sha.clone(), cx))).child(short).into_any_element()
    });
    let parents: Vec<AnyElement> = parents.collect();
    let parent_label = match c.parents.len() {
        0 => None,
        1 => Some("parent:"),
        _ => Some("parents:"),
    };
    let id = |label: &'static str, sha: SharedString| div().flex().gap(px(4.)).text_color(theme::text_secondary()).child(format!("{label}:")).child(div().font_family("JetBrains Mono").text_color(theme::text_selected()).child(sha));
    div()
        .flex()
        .flex_col()
        .flex_none()
        .h(px(COMMIT_H))
        .child(
            div()
                .flex()
                .flex_none()
                .h(px(37.))
                .items_center()
                .px(px(12.))
                .bg(theme::file_bar_bg())
                .border_b_1()
                .border_color(theme::section_border())
                .child(div().flex().flex_1().child(icon("shield-check", 16., if c.signed { theme::status_added() } else { theme::text_disabled() })))
                .child(id("commit", c.id.clone()))
                .child(div().flex().flex_1().justify_end().gap(px(4.)).text_color(theme::text_secondary()).children(parent_label).children(parents)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .px(px(12.))
                .pt(px(6.))
                .gap(px(4.))
                .child(div().flex().flex_none().h(px(28.)).items_center().gap(px(6.)).child(avatar(&c.author, &c.email, 28.)).child(div().flex_1().min_w_0().truncate().child(c.author.clone())).child(div().flex_none().text_color(theme::text_normal()).child(c.date.clone())))
                .when(!c.co_authors.is_empty(), |d| d.child(div().flex().flex_none().gap(px(6.)).children(c.co_authors.iter().map(|n| div().flex().items_center().gap(px(4.)).pr(px(6.)).pl(px(1.)).py(px(1.)).rounded(px(10.)).bg(theme::panel_bg1()).child(avatar(n, "", 18.)).child(n.clone())))))
                .child(
                    div()
                        .id("message")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(px(4.))
                        .bg(theme::app_bg())
                        .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).mb(px(6.)).child(c.summary.clone()))
                        .children(c.body.iter().map(|l| div().min_h(px(18.2)).child(l.clone()))),
                ),
        )
}

fn files_view(files: &Files, selected: Option<&SharedString>, all_files: bool, keys_here: bool, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let (mut added, mut changed, mut gone, mut renamed) = (0, 0, 0, 0);
    for f in files.changed.iter() {
        match status_icon(f.status).0 {
            "status-added" => added += 1,
            "status-deleted" => gone += 1,
            "status-renamed" => renamed += 1,
            _ => changed += 1,
        }
    }
    let count = |name: &'static str, n: usize, color: Hsla| (n > 0).then(|| div().flex().items_center().gap(px(4.)).child(icon(name, 14., color)).child(n.to_string()));
    let summary = div()
        .flex()
        .flex_none()
        .h(px(24.))
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .text_color(theme::text_secondary())
        .children(count("status-modified", changed, theme::status_modified()))
        .children(count("status-added", added, theme::status_added()))
        .children(count("status-deleted", gone, theme::status_deleted()))
        .children(count("status-renamed", renamed, theme::blue()))
        .child(div().flex_1())
        .child(div().text_color(theme::status_added()).child(format!("+{}", files.added)))
        .child(div().text_color(theme::status_deleted()).child(format!("−{}", files.deleted)));
    let mode = files.mode;
    let all_on = all_files && files.commit.is_some();
    let lead_button = |id: &'static str, glyph: &'static str, label: &'static str, on: bool| {
        div()
            .id(id)
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
            .text_color(if on { theme::text_selected() } else { theme::text_secondary() })
            .when(on, |d| d.bg(theme::selected_row()))
            .hover(|d| d.text_color(theme::text_selected()))
            .child(icon(glyph, 12., if on { theme::text_selected() } else { theme::text_secondary() }))
            .child(label)
    };
    let start = match mode {
        ListMode::Tree => {
            let expanded = files.collapsed.is_empty();
            lead_button("collapse-all", if expanded { "chevrons-down-up" } else { "chevrons-up-down" }, if expanded { "Collapse all" } else { "Expand all" }, false).on_click(cx.listener(|this, _, _, cx| this.with_files(cx, Files::toggle_all)))
        }
        ListMode::Path => lead_button("sort-status", "arrow-down-wide-narrow", "Sort by status", files.by_status).on_click(cx.listener(|this, _, _, cx| {
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
    let tools = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .px(px(12.))
        .pb(px(6.))
        .child(div().flex().flex_1().min_w_0().child(start))
        .child(segmented(vec![
            segment_with_icon("list-path", "list", "Path", mode == ListMode::Path)
                .on_click(cx.listener(|this, _, _, cx| this.with_files(cx, |f| f.set_mode(ListMode::Path))))
                .into_any_element(),
            segment_with_icon("list-tree", "list-tree", "Tree", mode == ListMode::Tree)
                .on_click(cx.listener(|this, _, _, cx| this.with_files(cx, |f| f.set_mode(ListMode::Tree))))
                .into_any_element(),
        ]))
        .child(div().flex().flex_1().min_w_0().justify_end().when(files.commit.is_some(), |d| d.child(lead_button("view-all", "folder-open", "View all files", all_on).on_click(cx.listener(|this, _, _, cx| this.toggle_all_files(cx))))));
    let filter_row = all_on.then(|| filter_row(files, cx));
    let tree_error = files.tree_error.clone().filter(|_| all_on).map(|e| {
        div().flex().flex_none().gap(px(6.)).px(px(12.)).pb(px(6.)).text_color(theme::text_secondary()).child(format!("Couldn't list all files: {e}")).child(div().id("tree-retry").cursor_pointer().text_color(theme::blue()).child("Retry").on_click(cx.listener(|this, _, _, cx| {
            let Some(repo) = this.tabs.get(this.active).map(|t| t.id) else { return };
            this.read_tree(repo, cx);
        })))
    });
    let query = if all_on { files.filter.text.trim().to_lowercase() } else { String::new() };
    let cursor = files.cursor.clone();
    let on_keys = keys_here;
    let rows = files.rows.clone();
    let changes = files.changes.clone();
    let selected = selected.cloned();
    let list = uniform_list(
        "file-list",
        files.rows.len(),
        cx.processor(move |_, range: Range<usize>, _, cx| {
            range
                .map(|i| {
                    let base = div().id(i).flex().w_full().h(px(FILE_ROW_H)).items_center().gap(px(6.)).pr(px(8.));
                    match &rows[i] {
                        ListRow::Folder { path, name, depth, collapsed } => {
                            let on = on_keys && cursor.as_ref() == Some(&Cursor::Folder(path.clone()));
                            let path = path.clone();
                            base.pl(px(TREE_BASE + f32::from(*depth) * TREE_LEVEL))
                                .when(on, |d| d.bg(theme::selected_row()))
                                .when(!on, |d| d.hover(|d| d.bg(theme::hover_row())))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(tab) = this.tabs.get_mut(this.active) {
                                        tab.keys = KeyArea::Files;
                                    }
                                    this.with_files(cx, |f| {
                                        f.cursor = Some(Cursor::Folder(path.clone()));
                                        f.toggle_folder(&path);
                                    });
                                }))
                                .child(icon(if *collapsed { "chevron-right" } else { "chevron-down" }, 14., theme::text_secondary()))
                                .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                        }
                        ListRow::File { index, name, dir, depth } => {
                            let change = &changes[*index];
                            let (glyph, color) = status_icon(change.status);
                            let on = selected.as_ref() == Some(&change.path) && cursor.as_ref().is_none_or(|c| !matches!(c, Cursor::File(i) if changes[*i].path == change.path && *i != *index));
                            let index = *index;
                            let unchanged = change.status == UNCHANGED;
                            let group: SharedString = format!("file-{i}").into();
                            let stat = |text: String, c: Hsla| div().text_size(px(11.)).text_color(c).child(text);
                            base.group(group.clone())
                                .pl(px(TREE_BASE + f32::from(*depth) * TREE_LEVEL))
                                .when(on, |d| d.bg(theme::selected_row()).text_color(theme::text_selected()))
                                .when(!on, |d| d.hover(|d| d.bg(theme::hover_row())))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(tab) = this.tabs.get_mut(this.active) {
                                        tab.keys = KeyArea::Files;
                                    }
                                    this.with_files(cx, |f| f.cursor = Some(Cursor::File(index)));
                                    this.open_file(index, cx);
                                }))
                                .child(if unchanged { div().flex_none().size(px(14.)).into_any_element() } else { icon(glyph, 14., color).into_any_element() })
                                .when(!dir.is_empty(), |d| d.child(div().flex().flex_none().max_w(relative(0.6)).overflow_hidden().text_color(theme::text_secondary()).children(marked(dir, &query)).child("/")))
                                .child(div().flex().flex_1().min_w_0().overflow_hidden().when(unchanged && !on, |d| d.text_color(theme::text_secondary())).children(marked(name, &query)))
                                .when(!unchanged, |d| {
                                    d.child(
                                        div()
                                            .flex()
                                            .flex_none()
                                            .gap(px(4.))
                                            .when(!on, |d| d.invisible().group_hover(group, |d| d.visible()))
                                            .when(change.additions.is_none(), |d| d.child(stat("binary".to_string(), theme::text_normal())))
                                            .when_some(change.additions, |d, n| d.child(stat(format!("+{n}"), theme::status_added())).child(stat(format!("−{}", change.deletions.unwrap_or(0)), theme::status_deleted()))),
                                    )
                                })
                        }
                    }
                })
                .collect()
        }),
    )
    .track_scroll(&files.list)
    .flex_1()
    .min_h_0();
    div().flex().flex_col().flex_1().min_h_0().child(summary).child(tools).children(filter_row).children(tree_error).child(list)
}

/// Bits of `text`, the filter's match marked.
fn marked(text: &SharedString, query: &str) -> Vec<AnyElement> {
    let found = if query.is_empty() { None } else { text.to_lowercase().find(query).filter(|i| text.is_char_boundary(*i) && text.is_char_boundary(i + query.len())) };
    let piece = |t: &str| div().flex_none().whitespace_nowrap().child(t.to_string());
    let Some(at) = found else { return vec![div().min_w_0().truncate().child(text.clone()).into_any_element()] };
    let (before, rest) = text.split_at(at);
    let (hit, after) = rest.split_at(query.len());
    vec![piece(before).into_any_element(), piece(hit).rounded(px(2.)).bg(theme::filter_match()).into_any_element(), piece(after).into_any_element()]
}

/// "View all files"' filter row: a path filter with its clear button, and the changed-file jumps.
fn filter_row(files: &Files, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let focused = files.filter_focused;
    let empty = files.filter.text.is_empty();
    let can_step = files.rows.iter().any(|r| matches!(r, ListRow::File { index, .. } if files.changes[*index].status != UNCHANGED));
    let step = |id: &'static str, glyph: &'static str, down: bool, cx: &mut Context<GitVolt>| {
        div()
            .id(id)
            .flex()
            .flex_none()
            .size(px(22.))
            .items_center()
            .justify_center()
            .rounded(px(3.))
            .when(!can_step, |d| d.opacity(0.4))
            .when(can_step, |d| {
                d.hover(|d| d.bg(theme::hover_row())).on_click(cx.listener(move |this, _, _, cx| {
                    let to = this.tabs.get(this.active).and_then(|t| t.files.as_ref()).and_then(|f| f.jump_changed(down));
                    if let Some(index) = to {
                        this.move_to_file(index, cx);
                    }
                }))
            })
            .child(icon(glyph, 13., theme::text_secondary()))
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .px(px(12.))
        .pb(px(6.))
        .child(
            div()
                .id("files-filter")
                .relative()
                .flex()
                .flex_1()
                .min_w_0()
                .h(px(26.))
                .items_center()
                .pl(px(8.))
                .pr(px(24.))
                .border_1()
                .border_color(if focused { theme::blue() } else { theme::section_border() })
                .rounded(px(4.))
                .bg(theme::app_bg())
                .cursor(CursorStyle::IBeam)
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    if let Some(tab) = this.tabs.get_mut(this.active) {
                        tab.keys = KeyArea::Files;
                    }
                    this.with_files(cx, |f| f.filter_focused = true);
                }))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| this.with_files(cx, |f| f.filter_focused = false)))
                .when(empty && !focused, |d| d.text_color(theme::text_disabled()).child("Filter files"))
                .when(!empty || focused, |d| d.child(files.filter.view(focused)))
                .when(!empty, |d| {
                    d.child(
                        div()
                            .id("files-filter-clear")
                            .absolute()
                            .right(px(4.))
                            .flex()
                            .size(px(16.))
                            .items_center()
                            .justify_center()
                            .rounded(px(3.))
                            .cursor_pointer()
                            .hover(|d| d.bg(theme::hover_row()))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.with_files(cx, |f| {
                                    f.filter = Field::default();
                                    f.rebuild();
                                });
                            }))
                            .child(icon("x", 12., theme::text_secondary())),
                    )
                }),
        )
        .child(div().flex().flex_none().gap(px(2.)).child(step("files-prev", "arrow-up", false, cx)).child(step("files-next", "arrow-down", true, cx)))
}

pub fn details_panel(tab: &Tab, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    tab.selected?;
    let selected = tab.diff.as_ref().map(|d| d.file.path.clone());
    let files = tab.files.as_ref().map(|f| files_view(f, selected.as_ref(), tab.all_files, tab.keys == KeyArea::Files, cx));
    let commit = tab.commit.as_ref().map(|c| commit_view(c, cx));
    Some(
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(DETAILS_W))
            .h_full()
            .min_h_0()
            .bg(theme::panel_bg())
            .border_l_1()
            .border_color(theme::section_border())
            .children(commit)
            .children(files)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn view_all_merges_the_tree_keeping_the_changed_and_filters_by_path() {
        let changed = [change("b/x.rs")];
        let tree = ["a.txt".to_string(), "b/x.rs".to_string(), "b/y.rs".to_string()];
        let all = listed_files(&changed, Some((&tree, "c1")), "", false);
        assert_eq!(all.iter().map(|f| (f.path.as_ref(), f.status)).collect::<Vec<_>>(), [("a.txt", UNCHANGED), ("b/x.rs", 'M'), ("b/y.rs", UNCHANGED)]);
        assert_eq!(all[0].new["commit"].as_str(), Some("c1"));
        assert_eq!(all[0].old["kind"].as_str(), Some("absent"));
        let some = listed_files(&changed, Some((&tree, "c1")), "y", false);
        assert_eq!(some.len(), 1);
        assert_eq!(some[0].path.as_ref(), "b/y.rs");
    }

    #[test]
    fn sorting_by_status_keeps_the_unchanged_last() {
        let mut a = change("a");
        a.status = 'D';
        let mut b = change("b");
        b.status = 'A';
        let order = listed_files(&[a, b, change("c")], Some((&["d".to_string()], "c1")), "", true);
        assert_eq!(order.iter().map(|f| f.path.as_ref()).collect::<Vec<_>>(), ["b", "c", "a", "d"]);
    }

    fn files(paths: &[&str]) -> Files {
        Files::new(paths.iter().map(|p| change(p)).collect(), 0, 0)
    }

    #[test]
    fn the_arrows_walk_the_files_into_collapsed_folders_and_wrap() {
        let mut f = files(&["a/one", "a/two", "z"]);
        assert_eq!(f.step_file(true), Some(0), "no cursor: the first");
        assert_eq!(f.step_file(false), Some(2), "or the last");
        f.cursor = Some(Cursor::File(1));
        assert_eq!(f.step_file(true), Some(2));
        f.cursor = Some(Cursor::File(2));
        assert_eq!(f.step_file(true), Some(0), "past the end it wraps");
        f.toggle_folder("a");
        assert_eq!(f.rows.len(), 2);
        f.cursor = Some(Cursor::File(2));
        assert_eq!(f.step_file(false), Some(1), "into the collapsed folder");
        f.reveal(1);
        assert!(f.collapsed.is_empty(), "which opens");
        // From a folder: its first file if it's open, past it if it's collapsed.
        f.cursor = Some(Cursor::Folder("a".into()));
        assert_eq!(f.step_file(true), Some(0));
        f.toggle_folder("a");
        assert_eq!(f.step_file(true), Some(2));
    }

    #[test]
    fn the_filter_keeps_the_cursor_on_its_file() {
        let mut f = files(&["a", "b", "c"]);
        f.cursor = Some(Cursor::File(2));
        f.filter.text = "c".into();
        f.rebuild();
        assert_eq!(f.changes.len(), 1);
        assert_eq!(f.cursor, Some(Cursor::File(0)));
        f.filter.text = "a".into();
        f.rebuild();
        assert_eq!(f.cursor, None, "filtered out");
    }

    #[test]
    fn the_changed_file_jumps_skip_the_unchanged_and_wrap() {
        let mut f = files(&["a", "c"]);
        f.commit = Some("c1".into());
        f.tree = Some(vec!["a".into(), "b".into(), "c".into()]);
        f.rebuild();
        f.cursor = Some(Cursor::File(0));
        assert_eq!(f.jump_changed(true), Some(2), "past b");
        f.cursor = Some(Cursor::File(2));
        assert_eq!(f.jump_changed(true), Some(0), "wrapping");
        assert_eq!(f.jump_changed(false), Some(0));
        f.cursor = None;
        assert_eq!(f.jump_changed(false), Some(2));
    }

    #[gpui::test]
    fn view_all_shows_the_filter_row_and_typing_narrows_the_list(cx: &mut TestAppContext) {
        let (window, mut cx, _runtime) = crate::app::tests::with_rows(cx, 1);
        window
            .update(&mut cx, |v, _, cx| {
                let tab = &mut v.tabs[0];
                tab.selected = Some(0);
                let mut list = files(&["src/a.rs"]);
                list.commit = Some("c1".into());
                tab.files = Some(list);
                v.toggle_all_files(cx);
                assert!(v.tabs[0].all_files);
                let list = v.tabs[0].files.as_mut().unwrap();
                list.tree = Some(vec!["README".into(), "src/a.rs".into(), "src/b.rs".into()]);
                list.rebuild();
                list.filter_focused = true;
                v.tabs[0].keys = KeyArea::Files;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        window.read_with(&cx, |v, _| assert_eq!(v.tabs[0].files.as_ref().unwrap().changes.len(), 3)).unwrap();
        cx.simulate_keystrokes("b");
        window.read_with(&cx, |v, _| {
            let f = v.tabs[0].files.as_ref().unwrap();
            assert_eq!(f.filter.text, "b");
            assert_eq!(f.changes.iter().map(|c| c.path.as_ref()).collect::<Vec<_>>(), ["src/b.rs"]);
        }).unwrap();
        // Esc clears it first; empty, the app's Esc goes on.
        cx.simulate_keystrokes("escape");
        window.read_with(&cx, |v, _| {
            let f = v.tabs[0].files.as_ref().unwrap();
            assert_eq!(f.filter.text, "");
            assert_eq!(f.changes.len(), 3);
        }).unwrap();
        // Off: the filter and the tree go.
        window.update(&mut cx, |v, _, cx| v.toggle_all_files(cx)).unwrap();
        window.read_with(&cx, |v, _| assert_eq!(v.tabs[0].files.as_ref().unwrap().changes.len(), 1)).unwrap();
    }

    #[gpui::test]
    fn the_keyboard_moves_in_the_file_list_after_a_click_there(cx: &mut TestAppContext) {
        let (window, mut cx, _runtime) = crate::app::tests::with_rows(cx, 1);
        window
            .update(&mut cx, |v, _, cx| {
                let tab = &mut v.tabs[0];
                tab.selected = Some(0);
                tab.files = Some(files(&["a/one", "a/two", "z"]));
                tab.keys = KeyArea::Files;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        let open = |window: &WindowHandle<GitVolt>, cx: &mut VisualTestContext| window.read_with(cx, |v, _| v.tabs[0].diff.as_ref().map(|d| d.file.path.to_string())).unwrap();
        cx.simulate_keystrokes("down");
        assert_eq!(open(&window, &mut cx).as_deref(), Some("a/one"));
        cx.simulate_keystrokes("down down");
        assert_eq!(open(&window, &mut cx).as_deref(), Some("z"));
        cx.simulate_keystrokes("home");
        assert_eq!(open(&window, &mut cx).as_deref(), Some("a/one"));
        cx.simulate_keystrokes("end");
        assert_eq!(open(&window, &mut cx).as_deref(), Some("z"));
        cx.simulate_keystrokes("enter");
        assert_eq!(open(&window, &mut cx), None, "enter on the open file closes it");
    }

    #[test]
    fn a_commit_lists_every_parent_whole() {
        let c = commit_of(&serde_json::json!({ "id": "abcdef123", "parents": ["111111111", "222222222"], "author": { "name": "A" }, "committer": { "time": 0 } }), &serde_json::json!({ "summary": "s" }));
        assert_eq!(c.parents, [SharedString::from("111111111"), SharedString::from("222222222")]);
        assert_eq!(c.id.as_ref(), "abcdef");
    }

    fn change(path: &str) -> FileChange {
        FileChange { path: path.to_string().into(), old_path: None, status: 'M', additions: Some(1), deletions: None, old: Value::Null, new: Value::Null, origin: Origin::Commit { id: "c".into() } }
    }

    fn shown(rows: &[ListRow]) -> Vec<(String, u8)> {
        rows.iter()
            .map(|r| match r {
                ListRow::Folder { name, depth, .. } => (format!("{name}/"), *depth),
                ListRow::File { name, depth, .. } => (name.to_string(), *depth),
            })
            .collect()
    }

    #[test]
    fn the_tree_puts_folders_first_and_merges_a_folder_of_one_folder() {
        let files = [change("z.txt"), change("crates/app/caps/default.json"), change("crates/core/src/a.rs"), change("crates/core/b.rs")];
        let rows = list_rows(&files, ListMode::Tree, &HashSet::new());
        assert_eq!(shown(&rows), [
            ("crates/".to_string(), 0),
            ("app/caps/".to_string(), 1),
            ("default.json".to_string(), 2),
            ("core/".to_string(), 1),
            ("src/".to_string(), 2),
            ("a.rs".to_string(), 3),
            ("b.rs".to_string(), 2),
            ("z.txt".to_string(), 0),
        ]);
    }

    #[test]
    fn a_collapsed_folder_hides_what_is_under_it() {
        let files = [change("a/b/c.txt"), change("d.txt")];
        let closed = HashSet::from(["a/b".to_string()]);
        assert_eq!(shown(&list_rows(&files, ListMode::Tree, &closed)), [("a/b/".to_string(), 0), ("d.txt".to_string(), 0)]);
        assert_eq!(all_folders(&files), closed);
    }

    #[test]
    fn the_path_list_keeps_the_order_and_names_each_dir() {
        let rows = list_rows(&[change("b/x.rs"), change("a.rs")], ListMode::Path, &HashSet::new());
        assert_eq!(rows[0], ListRow::File { index: 0, name: "x.rs".into(), dir: "b".into(), depth: 0 });
        assert_eq!(rows[1], ListRow::File { index: 1, name: "a.rs".into(), dir: "".into(), depth: 0 });
    }

    #[test]
    fn a_file_list_reads_the_camel_case_payload() {
        let list = file_changes(&serde_json::json!({"files": [{"path": "a", "oldPath": "b", "status": "R100", "additions": 2, "old": {"kind": "absent"}}]}), &Origin::Commit { id: "c".into() });
        assert_eq!((list[0].status, list[0].old_path.as_deref(), list[0].additions, list[0].deletions), ('R', Some("b"), Some(2), None));
    }

    #[test]
    fn each_status_has_its_glyph_and_colour() {
        assert_eq!(status_icon('A').0, "status-added");
        assert_eq!(status_icon('C').0, "status-added");
        assert_eq!(status_icon('T').0, "status-modified");
        assert_eq!(status_icon('U').0, "status-conflicted");
    }
}
