//! The right panel: a commit's ids, people and message, and its changed files by path or as a tree
//! (`ui/src/details`, `ui/src/files`).

use crate::app::{GitVolt, KeyArea, Tab, icon};
use crate::ui::input::TextInput;
use crate::model::{SHORT_SHA, initials};
use crate::theme;
use gitvolt_core::api::Request;
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use crate::write::call;
use crate::ui::tooltip::tip;
use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

pub const DETAILS_W: f32 = 400.;
pub const DETAILS_MIN: f32 = 280.;
pub const DETAILS_MAX: f32 = 720.;
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
    /// An unmerged WIP path's conflict kind.
    pub conflict: Option<String>,
}

/// The list a file came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Commit { id: String },
    Wip { worktree: String, staged: bool },
}

impl FileChange {
    /// A submodule's own changes are discarded inside it.
    pub fn is_submodule(&self) -> bool {
        self.old["kind"] == "submodule" || self.new["kind"] == "submodule"
    }
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
            conflict: f["conflict"].as_str().map(str::to_string),
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
    /// `key`: what its collapsed state and the cursor go by, `path` but for a WIP section's own.
    Folder { key: SharedString, path: SharedString, name: SharedString, depth: u8, collapsed: bool },
    File { index: usize, name: SharedString, dir: SharedString, depth: u8 },
}

#[derive(Default)]
struct Node {
    folders: BTreeMap<String, Node>,
    files: Vec<usize>,
}

#[allow(clippy::too_many_arguments)]
fn add_rows(node: &Node, prefix: &str, depth: u8, files: &[FileChange], collapsed: &HashSet<String>, scope: &str, offset: usize, out: &mut Vec<ListRow>) {
    for (name, child) in &node.folders {
        // A folder whose only child is a folder merges into one row (`a/b/c`, spec §9.3).
        let (mut name, mut child) = (name.clone(), child);
        while child.files.is_empty() && child.folders.len() == 1 {
            let (next, grand) = child.folders.iter().next().expect("len is 1");
            name = format!("{name}/{next}");
            child = grand;
        }
        let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        let key = folder_key(scope, &path);
        let closed = collapsed.contains(&key);
        out.push(ListRow::Folder { key: key.into(), path: path.clone().into(), name: name.into(), depth, collapsed: closed });
        if !closed {
            add_rows(child, &path, depth + 1, files, collapsed, scope, offset, out);
        }
    }
    for &index in &node.files {
        let name = files[index].path.rsplit('/').next().unwrap_or_default().to_string();
        out.push(ListRow::File { index: index + offset, name: name.into(), dir: SharedString::default(), depth });
    }
}

/// A folder's key: two WIP sections can both have a `src`, each with its own collapsed state.
pub fn folder_key(scope: &str, path: &str) -> String {
    if scope.is_empty() { path.to_string() } else { format!("{scope}\0{path}") }
}

/// What a folder's key says of its path.
pub fn folder_path(key: &str) -> &str {
    key.rsplit('\0').next().unwrap_or(key)
}

pub fn list_rows(files: &[FileChange], mode: ListMode, collapsed: &HashSet<String>) -> Vec<ListRow> {
    list_rows_in(files, mode, collapsed, "", 0)
}

/// The rows of `files`; their indexes count from `offset`, in the whole list a section is part of.
pub fn list_rows_in(files: &[FileChange], mode: ListMode, collapsed: &HashSet<String>, scope: &str, offset: usize) -> Vec<ListRow> {
    if mode == ListMode::Path {
        return files
            .iter()
            .enumerate()
            .map(|(index, f)| {
                let (dir, name) = f.path.rsplit_once('/').unwrap_or(("", &f.path));
                ListRow::File { index: index + offset, name: name.to_string().into(), dir: dir.to_string().into(), depth: 0 }
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
    add_rows(&root, "", 0, files, collapsed, scope, offset, &mut out);
    out
}

pub fn all_folders(files: &[FileChange]) -> HashSet<String> {
    list_rows(files, ListMode::Tree, &HashSet::new())
        .into_iter()
        .filter_map(|r| match r {
            ListRow::Folder { key, .. } => Some(key.to_string()),
            ListRow::File { .. } => None,
        })
        .collect()
}

pub fn status_icon(status: char) -> (&'static str, Hsla) {
    match status {
        'A' | 'C' => ("status-added", theme::status_added()),
        'D' => ("status-deleted", theme::status_deleted()),
        'R' => ("status-renamed", theme::blue()),
        'U' | 'X' => ("status-conflicted", theme::status_conflicted()),
        _ => ("status-modified", theme::status_modified()),
    }
}

/// A file only "View all files" lists: it has nothing to diff against.
pub const UNCHANGED: char = ' ';

const STATUS_ORDER: &str = "AMTRCDUX";

/// An unchanged file of commit `id`, as File View opens it.
fn unchanged_file(path: &str, id: &str) -> FileChange {
    FileChange { path: path.to_string().into(), old_path: None, status: UNCHANGED, additions: None, deletions: None, old: serde_json::json!({ "kind": "absent" }), new: serde_json::json!({ "kind": "atCommit", "commit": id }), conflict: None, origin: Origin::Commit { id: id.to_string() } }
}

/// A file as it is at a commit, opened by name (the palette's `/`).
pub fn file_at_commit(path: &str, id: &str) -> FileChange {
    unchanged_file(path, id)
}

/// A file of the working tree, opened by name (a pasted path).
pub fn file_in_worktree(path: &str, worktree: &str) -> FileChange {
    FileChange { path: path.to_string().into(), old_path: None, status: UNCHANGED, additions: None, deletions: None, old: serde_json::json!({ "kind": "absent" }), new: serde_json::json!({ "kind": "worktree", "worktree": worktree }), conflict: None, origin: Origin::Wip { worktree: worktree.to_string(), staged: false } }
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
    pub full_id: SharedString,
    /// A committer who isn't the author: their row carries the committed date.
    pub committer: Option<(SharedString, String)>,
    /// The author's date, when it differs from the committed one.
    pub authored: Option<SharedString>,
    /// Every parent, whole.
    pub parents: Vec<SharedString>,
    pub author: SharedString,
    pub email: String,
    pub date: SharedString,
    pub co_authors: Vec<SharedString>,
    pub summary: SharedString,
    pub body: Vec<SharedString>,
    pub signed: bool,
    /// What checking its signature said: the status's name and the card's lines.
    pub signature: Option<(String, Vec<String>)>,
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
    /// "View all files"' filter (`Tab::file_filter` holds what's typed): never kept when it's off.
    pub filter: String,
    /// The WIP row's sections; `changes` is then theirs, one after another.
    pub wip: Option<crate::wip::WipList>,
}

impl GitVolt {
    /// A file row's View: its changes in the centre.
    pub fn open_change(&mut self, file: FileChange, cx: &mut Context<Self>) {
        let at = self.tabs.get(self.active).and_then(|t| t.files.as_ref()).and_then(|f| f.changes.iter().position(|c| c.path == file.path && c.origin == file.origin));
        if let Some(at) = at {
            self.open_file(at, cx);
        }
    }

    /// View's File variant: the whole file.
    pub fn open_change_as_file(&mut self, file: FileChange, cx: &mut Context<Self>) {
        self.open_change(file, cx);
        self.with_diff(cx, |d| d.view = crate::diff::view::ViewKind::File);
    }

    pub fn open_change_history(&mut self, file: FileChange, blame: bool, cx: &mut Context<Self>) {
        self.open_change(file, cx);
        self.open_history(blame, cx);
    }

    /// Copies a commit's whole message.
    pub fn copy_message(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(repo) = self.tabs.get(self.active).map(|t| t.id) else { return };
        let api = self.api.clone();
        let done = self.rt.spawn(async move { api.dispatch(Request::CommitMessage { repo: crate::app::core_repo(repo), id }).await });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(v)) = done.await else { return };
            let (summary, body) = (v["summary"].as_str().unwrap_or_default(), v["body"].as_str().unwrap_or_default().trim());
            let text = if body.is_empty() { summary.to_string() } else { format!("{summary}\n\n{body}") };
            this.update(cx, |this, cx| this.copy_text(text, cx)).ok();
        })
        .detach();
    }

}

impl Files {
    pub fn new(changes: Vec<FileChange>, added: u32, deleted: u32) -> Self {
        let (mode, collapsed) = (ListMode::Tree, HashSet::new());
        let rows = Rc::new(list_rows(&changes, mode, &collapsed));
        let changes = Rc::new(changes);
        Self { changed: changes.clone(), changes, added, deleted, mode, by_status: false, collapsed, rows, list: UniformListScrollHandle::new(), cursor: None, commit: None, tree: None, tree_error: None, filter: String::new(), wip: None }
    }

    pub(crate) fn relist(&mut self) {
        if self.wip.is_some() {
            return self.relist_wip();
        }
        self.rows = Rc::new(list_rows(&self.changes, self.mode, &self.collapsed));
    }

    /// What's listed anew: the tree, the filter or the sort changed. The cursor stays on its file.
    pub fn rebuild(&mut self) {
        let on = match &self.cursor {
            Some(Cursor::File(i)) => self.changes.get(*i).cloned(),
            _ => None,
        };
        let query = self.filter.trim().to_lowercase();
        let tree = self.tree.as_deref().zip(self.commit.as_deref());
        let by_status = self.by_status && self.mode == ListMode::Path;
        if self.wip.is_some() {
            self.changes = Rc::new(self.wip_listed(&query, by_status));
        } else {
            self.changes = Rc::new(listed_files(&self.changed, tree, &query, by_status));
        }
        if let Some(file) = on {
            self.cursor = self.changes.iter().position(|f| f.path == file.path && f.origin == file.origin).map(Cursor::File);
        }
        self.relist();
    }

    /// Every file in display order, whatever's collapsed: in a WIP list, of the sections that aren't.
    fn all_files(&self) -> Vec<usize> {
        if self.wip.is_some() {
            return self.wip_order();
        }
        list_rows(&self.changes, self.mode, &HashSet::new()).iter().filter_map(|r| if let ListRow::File { index, .. } = r { Some(*index) } else { None }).collect()
    }

    /// Whether file `i` is under the folder with this key.
    fn in_folder(&self, i: usize, key: &str) -> bool {
        let path = folder_path(key);
        self.changes[i].path.starts_with(&format!("{path}/")) && self.wip.as_ref().is_none_or(|w| w.scope_of(i) == key.split('\0').next().unwrap_or_default())
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
                let first = order.iter().position(|i| self.in_folder(*i, path));
                return first.map(|f| {
                    if down {
                        let past = order[f..].iter().position(|i| !self.in_folder(*i, path));
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
            let scope = self.wip.as_ref().map_or("", |w| w.scope_of(index));
            self.collapsed.retain(|c| !(path.starts_with(&format!("{}/", folder_path(c))) && (scope.is_empty() || c.starts_with(&format!("{scope}\0")))));
        }
        self.relist();
        if let Some(wip) = self.wip.as_ref() {
            wip.reveal(index);
            return;
        }
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
        if self.wip.is_some() {
            return self.toggle_all_wip();
        }
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
            let details = api.dispatch(Request::CommitDetails { repo: crate::app::core_repo(repo), id: id.clone() }).await?;
            let message = api.dispatch(Request::CommitMessage { repo: crate::app::core_repo(repo), id: id.clone() }).await?;
            let files = api.dispatch(Request::FileList { repo: crate::app::core_repo(repo), spec: gitvolt_core::diff::DiffSpec::Commit { id: id.clone(), parent: 0 } }).await?;
            Ok::<_, gitvolt_core::error::GbError>((details, message, files, id))
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok((details, message, files, id))) = read.await else { return };
            this.update(cx, |this, cx| {
                let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) else { return };
                if tab.selected != Some(ix) && tab.revealed.as_deref() != Some(id.as_str()) {
                    return;
                }
                let commit = commit_of(&details, &message);
                if commit.signed {
                    this.check_signature(repo, id.clone(), cx);
                }
                let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) else { return };
                tab.commit = Some(commit);
                let count = |k: &str| files[k].as_u64().unwrap_or(0) as u32;
                let mut list = Files::new(file_changes(&files, &Origin::Commit { id: id.clone() }), count("added"), count("deleted"));
                list.commit = Some(id);
                list.by_status = tab.sort_by_status;
                let all_files = tab.all_files;
                this.seed_start_collapsed(&mut list);
                let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) else { return };
                tab.files = Some(list);
                if all_files {
                    this.read_tree(repo, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

}

impl GitVolt {
    /// The signature's verdict, as `SignatureBadge` asks it once a commit's selected.
    fn check_signature(&mut self, repo: u32, id: String, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let api = self.api.clone();
        let asked = id.clone();
        let read = self.rt.spawn(async move { api.dispatch(Request::Signature { repo: crate::app::core_repo(repo), id: asked }).await });
        cx.spawn(async move |this, cx| {
            let answer = read.await;
            this.update(cx, |this, cx| {
                let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) else { return };
                let Some(commit) = tab.commit.as_mut().filter(|c| c.full_id.as_ref() == id) else { return };
                commit.signature = Some(match answer {
                    Ok(Ok(s)) => {
                        let line = |k: &str, label: &str| s[k].as_str().filter(|v| !v.is_empty()).map(|v| format!("{label}{v}"));
                        let lines = [line("signer", "Signer: "), line("key", "Key: "), (s["fingerprint"] != s["key"]).then(|| line("fingerprint", "Fingerprint: ")).flatten(), line("trust", "Trust: "), line("detail", "")];
                        (s["kind"].as_str().unwrap_or("unknownKey").to_string(), lines.into_iter().flatten().collect())
                    }
                    _ => ("error".to_string(), Vec::new()),
                });
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
        let read = self.rt.spawn(async move { api.dispatch(Request::TreeFiles { repo: crate::app::core_repo(repo), id: asked }).await });
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
            files.filter.clear();
            files.rebuild();
        }
        if !on {
            self.tabs[self.active].file_filter.update(cx, |input, cx| input.set_text("", cx));
        }
        if on {
            self.read_tree(repo, cx);
        }
        cx.notify();
    }

    pub fn with_files(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Files)) {
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
            Some(Cursor::Folder(key)) => Some((key.clone(), files.collapsed.contains(key.as_ref()))),
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
                    let at = files.rows.iter().position(|r| matches!(r, ListRow::Folder { key, .. } if *key == path));
                    match at.and_then(|a| files.rows.get(a + 1)).cloned() {
                        Some(ListRow::File { index, .. }) => self.move_to_file(index, cx),
                        Some(ListRow::Folder { key: child, path: inner, .. }) if inner.starts_with(folder_path(&path)) => self.with_files(cx, |f| f.cursor = Some(Cursor::Folder(child))),
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

    /// The filter's text changed.
    pub fn file_filter_changed(&mut self, repo: u32, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) else { return };
        let text = tab.file_filter.read(cx).text().to_string();
        if let Some(files) = tab.files.as_mut() {
            files.filter = text;
            files.rebuild();
            cx.notify();
        }
    }

    /// A parent's id: selects its commit, or says it isn't loaded.
    pub fn go_to_commit(&mut self, sha: SharedString, cx: &mut Context<Self>) {
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
    let (who, by) = (&details["author"], &details["committer"]);
    let differs = who["name"] != by["name"] || who["email"] != by["email"];
    Commit {
        id: short(details["id"].as_str().unwrap_or_default()).into(),
        full_id: details["id"].as_str().unwrap_or_default().to_string().into(),
        committer: differs.then(|| (by["name"].as_str().unwrap_or_default().to_string().into(), by["email"].as_str().unwrap_or_default().to_string())),
        authored: (who["time"] != by["time"]).then(|| crate::model::format_date(who["time"].as_i64().unwrap_or(0)).into()),
        parents: details["parents"].as_array().map_or(&[][..], Vec::as_slice).iter().filter_map(|p| p.as_str()).map(|p| p.to_string().into()).collect(),
        author: details["author"]["name"].as_str().unwrap_or_default().to_string().into(),
        email: details["author"]["email"].as_str().unwrap_or_default().to_string(),
        date: crate::model::format_date(details["committer"]["time"].as_i64().unwrap_or(0)).into(),
        co_authors: details["coAuthors"].as_array().map_or(&[][..], Vec::as_slice).iter().filter_map(|c| c["name"].as_str()).map(|n| n.to_string().into()).collect(),
        summary: message["summary"].as_str().unwrap_or_default().to_string().into(),
        body: message["body"].as_str().unwrap_or_default().trim().split('\n').filter(|_| message["body"].as_str().is_some_and(|b| !b.trim().is_empty())).map(|l| l.to_string().into()).collect(),
        signed: details["signed"].as_bool() == Some(true),
        signature: None,
    }
}

/// `personLabel`.
pub fn person_label(name: &str, email: &str) -> String {
    if email.is_empty() { name.to_string() } else { format!("{name} <{email}>") }
}

pub fn avatar(name: &SharedString, email: &str, size: f32) -> Div {
    if let Some(picture) = crate::avatars::image_for(email, name) {
        return div().flex_none().size(px(size)).rounded_full().overflow_hidden().bg(theme::avatar_backdrop()).child(img(picture).size(px(size)).rounded_full());
    }
    let lane = crate::model::avatar_lane(name, email, 10);
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

/// `SignatureBadge`: a shield per verdict, in a box of its own; a bare dim one when it's not signed or not checked yet.
fn signature_badge(c: &Commit) -> AnyElement {
    let kind = if !c.signed { "unsigned" } else { c.signature.as_ref().map_or("loading", |s| s.0.as_str()) };
    let (name, color, title): (&'static str, Hsla, &str) = match kind {
        "verified" => ("sig-verified", theme::status_added(), "Verified"),
        "unverified" => ("sig-unverified", theme::status_modified(), "Unverified"),
        "bad" => ("sig-bad", theme::status_deleted(), "Bad signature"),
        "expired" => ("sig-expired", theme::status_modified(), "Expired"),
        "unknownKey" => ("sig-unknownKey", theme::status_modified(), "Unknown key"),
        "error" => ("sig-error", theme::status_deleted(), "Couldn't check the signature"),
        "loading" => ("sig-loading", theme::text_disabled(), "Checking signature…"),
        _ => ("sig-unsigned", theme::text_disabled(), "Not signed"),
    };
    let mut text = title.to_string();
    if let Some((_, lines)) = &c.signature {
        for l in lines {
            text.push('\n');
            text.push_str(l);
        }
    }
    crate::ui::tooltip::tip(text, icon(name, 16., color)).into_any_element()
}

/// A hash in the commit's bar: white monospace, underlined under the pointer.
fn hash_button(id: impl Into<ElementId>, hash: &SharedString, tooltip: &'static str, cx: &mut Context<GitVolt>, run: impl Fn(&mut GitVolt, &mut Context<GitVolt>) + 'static) -> AnyElement {
    let short: String = hash.chars().take(SHORT_SHA).collect();
    crate::ui::tooltip::tip(tooltip, div().id(id).font_family("JetBrains Mono").text_color(theme::text_selected()).cursor_pointer().hover(|d| d.underline()).on_click(cx.listener(move |this, _, _, cx| run(this, cx))).child(short)).into_any_element()
}

/// The commit's header and message: the split's share of the panel, the header fixed and only the message scrolling.
/// `HeadMessageEditor`: the message as two fields in place, Save (Ctrl+Enter) and Cancel (Esc). HEAD's is an amend of the message
/// only; an older commit's is a reword, an interactive rebase.
pub struct MessageEdit {
    pub id: String,
    pub summary: Entity<crate::ui::input::TextInput>,
    pub body: Entity<crate::ui::input::TextInput>,
    pub older: bool,
    pub upstream: Option<String>,
    pub busy: bool,
}

/// `rewordableOlder`: an older commit of HEAD's branch "Edit message" can reword: one parent, under HEAD, and not what the trunk has.
pub fn rewordable_older(g: &crate::model::GraphView, id: &str) -> bool {
    let (Some(target), branch) = (g.head_sha.as_deref(), g.head_branch.as_ref()) else { return false };
    if branch.is_empty() || target == id {
        return false;
    }
    let Some(row) = g.by_sha.get(id).and_then(|i| g.rows.get(*i)) else { return false };
    if row.parents.len() != 1 || crate::refs::is_ancestor(g, id, target) != Some(true) {
        return false;
    }
    let trunk = ["refs/heads/main", "refs/heads/master", "refs/remotes/origin/main", "refs/remotes/origin/master"].into_iter().find_map(|t| crate::stacks::find_base(g, t).map(|f| (t, f)));
    if let Some((trunk, (at, name))) = trunk
        && branch != trunk
        && branch != format!("refs/heads/{name}").as_str()
        && branch != name.as_str()
        && let Some(tip) = g.rows.get(at)
        && crate::refs::is_ancestor(g, id, &tip.id) == Some(true)
    {
        return false;
    }
    true
}

impl GitVolt {
    fn start_message_edit(&mut self, older: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(c) = tab.commit.as_ref() else { return };
        let (id, summary_text, body_text, repo, worktree) = (c.full_id.to_string(), c.summary.to_string(), c.body.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n"), tab.id, tab.wip_worktree().unwrap_or_default().to_string());
        let summary = cx.new(|cx| {
            let mut i = crate::ui::input::TextInput::new(cx).claim_escape();
            i.set_text(&summary_text, cx);
            i
        });
        let body = cx.new(|cx| {
            let mut i = crate::ui::input::TextInput::multiline(cx).placeholder("Description").claim_escape();
            i.set_text(&body_text, cx);
            i
        });
        for input in [&summary, &body] {
            let first = input == &summary;
            let (down, repo_) = (body.clone(), repo);
            cx.subscribe(input, move |this, _, e: &crate::ui::input::InputEvent, cx| match e {
                crate::ui::input::InputEvent::Escape => {
                    if let Some(t) = this.tabs.iter_mut().find(|t| t.id == repo_) {
                        t.message_edit = None;
                    }
                    cx.notify();
                }
                crate::ui::input::InputEvent::Submit { ctrl: true, .. } => this.save_message_edit(repo_, cx),
                crate::ui::input::InputEvent::Submit { .. } if first => {
                    this.focus_next_frame = Some(down.read(cx).focus.clone());
                    cx.notify();
                }
                _ => cx.notify(),
            })
            .detach();
        }
        let first_focus = summary.read(cx).focus.clone();
        window.focus(&first_focus, cx);
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        tab.message_edit = Some(MessageEdit { id, summary, body, older, upstream: None, busy: false });
        if cfg!(test) {
            return;
        }
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(Request::HeadOnUpstream { repo: crate::app::core_repo(repo), worktree }).await });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(v)) = read.await {
                this.update(cx, |this, cx| {
                    if let Some(e) = this.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.message_edit.as_mut()) {
                        e.upstream = v.as_str().map(String::from);
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }

    fn save_message_edit(&mut self, repo: u32, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == repo) else { return };
        let Some(e) = tab.message_edit.as_ref().filter(|e| !e.busy) else { return };
        let (summary, body) = (e.summary.read(cx).text().trim().to_string(), e.body.read(cx).text().trim().to_string());
        if summary.is_empty() {
            return;
        }
        let Some(ctx) = tab.wip_ctx() else { return };
        let message = if body.is_empty() { summary } else { format!("{summary}\n\n{body}") };
        let (id, older, worktree) = (e.id.clone(), e.older, ctx.worktree.clone());
        let head = tab.graph.as_ref().and_then(|g| g.head_sha.clone()).unwrap_or_else(|| id.clone());
        if let Some(e) = self.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.message_edit.as_mut()) {
            e.busy = true;
        }
        let w = crate::write::Write::new(&ctx, move |asked| {
            if older {
                call("rewordCommit", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "oid": id, "message": message, "expect": { "head": head, "refs": {} }, "confirm": { "autostash": asked.autostash } }))
            } else {
                call("editHeadMessage", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "message": message, "expect": { "head": head, "refs": {} } }))
            }
        })
        .on_success(move |this, out, cx| {
            if let Some(t) = this.tabs.iter_mut().find(|t| t.id == repo) {
                t.message_edit = None;
            }
            this.reload(repo, cx);
            let oid = out["oid"].as_str().or(out["rewritten"].as_str()).map(|s| s.to_string());
            if let Some(oid) = oid {
                this.go_to_commit(oid.into(), cx);
            }
        })
        .on_end(move |this, _, cx| {
            if let Some(e) = this.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.message_edit.as_mut()) {
                e.busy = false;
            }
            cx.notify();
        });
        self.write(w, cx);
    }
}

fn message_editor(e: &MessageEdit, repo: u32, cx: &mut Context<GitVolt>) -> AnyElement {
    use crate::ui::controls::{ButtonKind, button};
    let can = !e.busy && !e.summary.read(cx).text().trim().is_empty();
    let field = |input: Entity<crate::ui::input::TextInput>| div().px(px(8.)).py(px(5.)).bg(theme::app_bg()).border_1().border_color(theme::section_border()).rounded(px(4.)).child(input);
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .mb(px(8.))
        .p(px(2.))
        .child(field(e.summary.clone()))
        .child(field(e.body.clone()).min_h(px(46.)))
        .when_some(e.upstream.clone(), |d, u| d.child(div().text_size(px(12.)).text_color(theme::status_modified()).child(format!("This commit is on {u}; you'll need to force push."))))
        .when(e.older, |d| d.child(div().text_size(px(12.)).text_color(theme::status_modified()).child("Saving rebases the commits above it.")))
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(6.))
                .child(crate::ui::anchor("message-cancel", button("message-cancel", "Cancel", ButtonKind::Plain, true).on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(t) = this.tabs.iter_mut().find(|t| t.id == repo) {
                        t.message_edit = None;
                    }
                    cx.notify();
                }))))
                .child(crate::ui::anchor("message-save", button("message-save", "Save", ButtonKind::Positive, can).on_click(cx.listener(move |this, _, _, cx| this.save_message_edit(repo, cx))))),
        )
        .into_any_element()
}

fn commit_view(c: &Commit, height: f32, edit: Option<(&MessageEdit, u32)>, editable: Option<bool>, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let parents = c.parents.iter().enumerate().map(|(i, p)| {
        let sha = p.clone();
        hash_button(("parent", i), p, "Go to parent commit", cx, move |this, cx| this.go_to_commit(sha.clone(), cx))
    });
    let parents: Vec<AnyElement> = parents.collect();
    let parent_label = match c.parents.len() {
        0 => None,
        1 => Some("parent:"),
        _ => Some("parents:"),
    };
    let full = c.full_id.clone();
    let date_col = |top: Option<SharedString>, authored: Option<SharedString>| div().flex().flex_col().flex_none().ml_auto().items_end().text_size(px(12.)).line_height(px(15.6)).text_color(theme::text_normal()).children(top).children(authored.map(|a| div().text_size(px(11.)).text_color(theme::text_disabled()).child(format!("authored {a}"))));
    let person = |name: &SharedString, email: &str, size: f32| div().flex().min_w_0().items_center().gap(px(6.)).child(avatar(name, email, size)).child(div().min_w_0().truncate().text_color(theme::text_selected()).child(name.clone()));
    let author = div().flex().flex_none().min_h(px(32.)).items_center().gap(px(8.)).min_w_0().child(person(&c.author, &c.email, 28.)).child(date_col(c.committer.is_none().then(|| c.date.clone()), c.authored.clone()));
    let committer = c.committer.as_ref().map(|(name, email)| div().flex().flex_none().min_h(px(32.)).items_center().gap(px(8.)).min_w_0().child(person(name, email, 20.)).child(div().text_color(theme::text_disabled()).child("committed")).child(date_col(Some(c.date.clone()), None)));
    div()
        .flex()
        .flex_col()
        .flex_none()
        .h(px(height))
        .min_h(px(72.))
        .overflow_hidden()
        .px(px(12.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap(px(4.))
                .mb(px(6.))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .h(px(37.))
                        .mx(px(-12.))
                        .px(px(12.))
                        .items_center()
                        .bg(theme::file_bar_bg())
                        .border_b_1()
                        .border_color(theme::section_border())
                        .gap(px(10.))
                        .child(div().flex().flex_1().min_w_0().child(signature_badge(c)))
                        .child(div().flex().flex_none().gap(px(8.)).text_color(theme::text_secondary()).child("commit:").child(hash_button("commit-sha", &c.id, "Copy full SHA", cx, move |this, cx| this.copy_text(full.to_string(), cx))))
                        .child(div().flex().flex_1().justify_end().gap(px(8.)).whitespace_nowrap().text_color(theme::text_secondary()).children(parent_label).child(div().flex().flex_none().gap(px(6.)).children(parents))),
                )
                .child(author)
                .children(committer)
                .when(!c.co_authors.is_empty(), |d| d.child(div().flex().flex_none().gap(px(6.)).children(c.co_authors.iter().map(|n| div().flex().items_center().gap(px(4.)).pr(px(6.)).pl(px(1.)).py(px(1.)).rounded(px(10.)).bg(theme::panel_bg1()).child(avatar(n, "", 18.)).child(n.clone()))))),
        )
        .map(|d| match edit.filter(|(e, _)| e.id == c.full_id.as_ref()) {
            Some((e, repo)) => d.child(message_editor(e, repo, cx)),
            None => d.child(
                div()
                    .id("message")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .mb(px(8.))
                    .px(px(10.))
                    .pt(px(6.))
                    .pb(px(8.))
                    .rounded(px(4.))
                    .bg(theme::app_bg())
                    // The pencil comes first in the web's box, so the summary keeps its 4 px top margin.
                    .when_some(editable, |d, older| {
                        d.pr(px(28.)).child(crate::ui::anchor(
                            "message-edit",
                            tip(
                                if older { "Edit the commit message (rebases the commits above it)" } else { "Edit the commit message" },
                                div().id("message-edit").absolute().top(px(4.)).right(px(4.)).flex().size(px(22.)).items_center().justify_center().rounded(px(4.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).on_click(cx.listener(move |this, _, window, cx| this.start_message_edit(older, window, cx))).child(icon("pencil", 13., theme::text_secondary())),
                            ),
                        ))
                    })
                    .child(div().when(editable.is_some(), |d| d.mt(px(4.))).text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).mb(px(6.)).child(c.summary.clone()))
                    .children(c.body.iter().map(|l| div().min_h(px(18.2)).child(l.clone()))),
            ),
        })
}

/// The line between the commit and its files, which moves.
fn split_resizer(panel_h: f32, header: f32, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    div().relative().flex_none().h(px(1.)).bg(theme::section_border()).child(
        div()
            .id("details-split")
            .absolute()
            .left_0()
            .right_0()
            .top(px(-2.))
            .h(px(5.))
            .cursor(CursorStyle::ResizeUpDown)
            .hover(|d| d.bg(theme::blue()))
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                if e.click_count == 2 {
                    this.details_split = SPLIT_DEFAULT;
                    this.save_layout();
                    return cx.notify();
                }
                this.split_drag = Some(SplitDrag { y: e.position.y.as_f32(), ratio: this.details_split, panel_h, header });
            })),
    )
}

/// The commit's share of the panel, dragged: the top keeps its header and a couple of message lines, the files 160 px.
pub const SPLIT_DEFAULT: f32 = 0.25;

pub fn split_bounds(panel: f32, header: f32) -> (f32, f32) {
    if panel <= 0. {
        return (0.1, 0.75);
    }
    let hi = (1. - 160. / panel).min(0.75);
    let lo = ((header + 56.) / panel).max(0.1);
    (lo.min(hi), hi)
}

#[derive(Debug, Clone, Copy)]
pub struct SplitDrag {
    pub y: f32,
    pub ratio: f32,
    pub panel_h: f32,
    pub header: f32,
}

impl GitVolt {
    pub fn on_split_drag(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(d) = self.split_drag else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            return self.end_split_drag();
        }
        let (lo, hi) = split_bounds(d.panel_h, d.header);
        self.details_split = (d.ratio + (e.position.y.as_f32() - d.y) / d.panel_h).clamp(lo, hi);
        cx.notify();
    }

    pub fn end_split_drag(&mut self) {
        if self.split_drag.take().is_some() {
            self.save_layout();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn files_view(files: &Files, creating: Option<&crate::manage::CreateFile>, input: &Entity<TextInput>, selected: Option<&FileChange>, all_files: bool, keys_here: bool, panel_w: f32, window: &Window, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
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
        .h(px(30.))
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .text_color(theme::text_secondary())
        .children(count("status-modified", changed, theme::status_modified()))
        .children(count("status-added", added, theme::status_added()))
        .children(count("status-deleted", gone, theme::status_deleted()))
        .children(count("status-renamed", renamed, theme::blue()))
        .child(div().flex_1())
        .child(div().flex().flex_none().gap(px(4.)).child(div().text_color(theme::status_added()).child(format!("+{}", files.added))).child(div().text_color(theme::status_deleted()).child(format!("−{}", files.deleted))));
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
            let has_folders = files.rows.iter().any(|r| matches!(r, ListRow::Folder { .. }));
            lead_button("collapse-all", if expanded { "chevrons-down-up" } else { "chevrons-up-down" }, if expanded { "Collapse all" } else { "Expand all" }, false).when(!has_folders, |d| d.opacity(0.4)).when(has_folders, |d| d.on_click(cx.listener(|this, _, _, cx| this.with_files(cx, Files::toggle_all))))
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
    let filter_row = all_on.then(|| filter_row(files, input, window, cx));
    let tree_error = files.tree_error.clone().filter(|_| all_on).map(|e| {
        div().flex().flex_none().gap(px(6.)).px(px(12.)).pb(px(6.)).text_color(theme::text_secondary()).child(format!("Couldn't list all files: {e}")).child(div().id("tree-retry").cursor_pointer().text_color(theme::blue()).child("Retry").on_click(cx.listener(|this, _, _, cx| {
            let Some(repo) = this.tabs.get(this.active).map(|t| t.id) else { return };
            this.read_tree(repo, cx);
        })))
    });
    let query = if all_on { files.filter.trim().to_lowercase() } else { String::new() };
    let view = ListView { rows: files.rows.clone(), changes: files.changes.clone(), cursor: files.cursor.clone(), keys_here, selected: selected.cloned(), query, panel_w, section: None };
    let list = file_list("file-list", view, &files.list, cx);
    let creating = creating.map(|c| crate::manage::create_file_input(c, cx));
    div()
        .id("files-body")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .on_mouse_down(MouseButton::Right, cx.listener(|_, e: &MouseDownEvent, window, cx| {
            let weak = cx.weak_entity();
            let rows = vec![crate::menus::create_file_row(None, None, &weak)];
            crate::ui::menu::open_context(rows, e.position, window, cx);
        }))
        .child(summary)
        .child(tools)
        .children(filter_row)
        .children(tree_error)
        .children(creating)
        .child(list)
}

/// What a file list shows: all of it, or a WIP section's part of it.
pub struct ListView {
    pub rows: Rc<Vec<ListRow>>,
    pub changes: Rc<Vec<FileChange>>,
    pub cursor: Option<Cursor>,
    pub keys_here: bool,
    /// The open file.
    pub selected: Option<FileChange>,
    /// The filter, marked in the names.
    pub query: String,
    pub panel_w: f32,
    /// The WIP section it is, for its rows' actions.
    pub section: Option<crate::wip::Which>,
}

/// The rows: whichever are in view, each with what its hover and right-click do.
pub fn file_list(id: &'static str, view: ListView, handle: &UniformListScrollHandle, cx: &mut Context<GitVolt>) -> UniformList {
    let ListView { rows, changes, cursor, keys_here: on_keys, selected, query, panel_w, section } = view;
    let count = rows.len();
    uniform_list(
        id,
        count,
        cx.processor(move |this, range: Range<usize>, window, cx| {
            let left_of = window.viewport_size().width.as_f32() - panel_w;
            range
                .map(|i| {
                    let base = div().id(i).relative().flex().w_full().h(px(crate::metrics::file_row_h())).items_center().gap(px(6.)).pr(px(8.));
                    let full_path: SharedString = match &rows[i] {
                        ListRow::Folder { path, .. } => path.clone(),
                        ListRow::File { index, .. } => changes[*index].path.clone(),
                    };
                    let group: SharedString = format!("{id}-{i}").into();
                    let row = match &rows[i] {
                        ListRow::Folder { key, path, name, depth, collapsed } => {
                            let on = on_keys && cursor.as_ref() == Some(&Cursor::Folder(key.clone()));
                            let cursor_key = key.clone();
                            let menu_path = path.clone();
                            let actions = section.and_then(|w| crate::wip::row_actions(this, w, &rows[i], &changes, &group, on, panel_w, cx));
                            base.group(group.clone())
                                .pl(px(TREE_BASE + f32::from(*depth) * TREE_LEVEL))
                                .when(on, |d| d.bg(theme::selected_row()))
                                .when(!on, |d| d.hover(|d| d.bg(theme::hover_row())))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(tab) = this.tabs.get_mut(this.active) {
                                        tab.keys = KeyArea::Files;
                                    }
                                    this.with_files(cx, |f| {
                                        f.cursor = Some(Cursor::Folder(cursor_key.clone()));
                                        f.toggle_folder(&cursor_key);
                                    });
                                }))
                                .on_mouse_down(MouseButton::Right, cx.listener({
                                    let path = menu_path.clone();
                                    move |this, e: &MouseDownEvent, window, cx| {
                                        cx.stop_propagation();
                                        let weak = cx.weak_entity();
                                        let rows = this.file_folder_rows(&path, section, &weak);
                                        crate::ui::menu::open_context_for(format!("folder:{path}"), rows, e.position, window, cx);
                                    }
                                }))
                                .when_some(this.colours().and_then(|m| m.folder_tint(&menu_path)), |d, t| d.bg(t))
                                .when(crate::ui::menu::is_target(&format!("folder:{menu_path}"), cx), |d| d.child(crate::ui::outline()))
                                .child(icon(if *collapsed { "chevron-right" } else { "chevron-down" }, 14., theme::text_secondary()))
                                .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                                .children(actions)
                        }
                        ListRow::File { index, name, dir, depth } => {
                            let change = &changes[*index];
                            let (glyph, color) = status_icon(change.status);
                            let on = selected.as_ref().is_some_and(|s| s.path == change.path && (section.is_none() || s.origin == change.origin)) && cursor.as_ref().is_none_or(|c| !matches!(c, Cursor::File(i) if changes[*i].path == change.path && *i != *index && section.is_none()));
                            let index = *index;
                            let unchanged = change.status == UNCHANGED;
                            let stat = |text: String, c: Hsla| div().text_size(px(11.)).text_color(c).child(text);
                            let actions = section.and_then(|w| crate::wip::row_actions(this, w, &rows[i], &changes, &group, on, panel_w, cx));
                            let tint = this.colours().and_then(|m| m.file_tint(&change.path));
                            base.group(group.clone())
                                .pl(px(TREE_BASE + f32::from(*depth) * TREE_LEVEL))
                                .when_some(tint.filter(|_| !on), |d, t| d.bg(t))
                                .when(on, |d| d.bg(theme::selected_row()).text_color(theme::text_selected()))
                                .when(!on, |d| d.hover(|d| d.bg(theme::hover_row())))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(tab) = this.tabs.get_mut(this.active) {
                                        tab.keys = KeyArea::Files;
                                    }
                                    this.with_files(cx, |f| f.cursor = Some(Cursor::File(index)));
                                    this.open_file(index, cx);
                                }))
                                .on_mouse_down(MouseButton::Right, cx.listener({
                                    let file = change.clone();
                                    move |this, e: &MouseDownEvent, window, cx| {
                                        cx.stop_propagation();
                                        let weak = cx.weak_entity();
                                        let rows = this.file_rows(&file, &weak);
                                        crate::ui::menu::open_context_for(format!("file:{}", file.path), rows, e.position, window, cx);
                                    }
                                }))
                                .when(crate::ui::menu::is_target(&format!("file:{}", change.path), cx), |d| d.child(crate::ui::outline()))
                                .child(if unchanged { div().flex_none().size(px(14.)).into_any_element() } else { icon(glyph, 14., color).into_any_element() })
                                .when(!dir.is_empty(), |d| d.child(div().flex().flex_none().max_w(relative(0.6)).overflow_hidden().text_color(theme::text_secondary()).children(marked(dir, &query)).child("/")))
                                .child(div().flex().flex_initial().min_w_0().overflow_hidden().when(unchanged && !on, |d| d.text_color(theme::text_secondary())).children(marked(name, &query)))
                                .children(this.lfs_mark(&change.path, cx).map(crate::lfs::lfs_mark_element))
                                .child(div().flex_1())
                                .children(crate::forge::review::zones::row_badge(this, &change.path, cx))
                                .when_some(change.conflict.as_deref().map(crate::wip::conflict_label), |d, label| d.child(div().flex_none().ml_auto().text_size(px(11.)).text_color(theme::status_conflicted()).child(label)))
                                .when(!unchanged && actions.is_none() && change.conflict.is_none(), |d| {
                                    d.child(
                                        div()
                                            .flex()
                                            .flex_none()
                                            .gap(px(4.))
                                            .when(!on, |d| d.invisible().group_hover(group.clone(), |d| d.visible()))
                                            .when(change.additions.is_none(), |d| d.child(stat("binary".to_string(), theme::text_normal())))
                                            .when_some(change.additions, |d, n| d.child(stat(format!("+{n}"), theme::status_added())).child(stat(format!("−{}", change.deletions.unwrap_or(0)), theme::status_deleted()))),
                                    )
                                })
                                .children(actions)
                        }
                    };
                    crate::ui::tooltip::Anchor::new(row).tip(full_path).placement(crate::ui::tooltip::Placement::LeftOf(left_of))
                })
                .collect()
        }),
    )
    .track_scroll(handle)
    .flex_1()
    .min_h_0()
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
fn filter_row(files: &Files, input: &Entity<TextInput>, window: &Window, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let empty = files.filter.is_empty();
    let focused = input.read(cx).is_focused(window);
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
    let clear_input = input.clone();
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
                .text_color(theme::text_selected())
                .child(input.clone())
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
                            .on_click(cx.listener(move |this, _, _, cx| {
                                clear_input.update(cx, |input, cx| input.set_text("", cx));
                                let Some(repo) = this.tabs.get(this.active).map(|t| t.id) else { return };
                                this.file_filter_changed(repo, cx);
                            }))
                            .child(icon("x", 12., theme::text_secondary())),
                    )
                }),
        )
        .child(div().flex().flex_none().gap(px(2.)).child(step("files-prev", "arrow-up", false, cx)).child(step("files-next", "arrow-down", true, cx)))
}

pub fn details_panel(v: &GitVolt, tab: &Tab, width: f32, height: f32, window: &Window, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    if tab.selected.is_none() && tab.compare.is_none() && tab.picks.rows.is_empty() && tab.revealed.is_none() {
        return None;
    }
    // A ⋮ row is no commit: there's nothing to show of it.
    if tab.compare.is_none() && tab.picks.rows.is_empty() && tab.revealed.is_none() && tab.files.is_none() && tab.commit.is_none() && tab.selected.and_then(|i| tab.graph.as_ref()?.rows.get(i)).is_some_and(|r| r.kind == gitvolt_graph::NodeKind::Collapsed) {
        return None;
    }
    let selected = tab.diff.as_ref().map(|d| d.file.clone());
    if tab.is_wip_selected() {
        return Some(crate::wip::wip_panel(v, tab, width, height, window, cx));
    }
    let files = tab.files.as_ref().map(|f| files_view(f, tab.create_file.as_ref().filter(|c| c.section.is_none()), &tab.file_filter, selected.as_ref(), tab.all_files, tab.keys == KeyArea::Files, width, window, cx));
    let header = 37. + 4. + 32. + 6. + if tab.commit.as_ref().is_some_and(|c| c.committer.is_some()) { 36. } else { 0. } + if tab.commit.as_ref().is_some_and(|c| !c.co_authors.is_empty()) { 26. } else { 0. };
    let (lo, hi) = split_bounds(height, header);
    let commit_h = (v.details_split.clamp(lo, hi) * height).round();
    let editable = tab.commit.as_ref().zip(tab.graph.as_ref()).and_then(|(c, g)| if g.head_sha.as_deref() == Some(c.full_id.as_ref()) { Some(false) } else { rewordable_older(g, &c.full_id).then_some(true) });
    let commit = tab.commit.as_ref().filter(|_| tab.compare.is_none()).map(|c| commit_view(c, commit_h, tab.message_edit.as_ref().map(|e| (e, tab.id)), editable, cx));
    let split = tab.commit.as_ref().filter(|_| tab.compare.is_none()).map(|_| split_resizer(height, header, cx));
    let compare = tab.compare.as_ref().map(|c| crate::selection::compare_header(tab, c, cx));
    let multi = tab.multi_ids().map(|rows| crate::selection::multi_summary(tab, rows, cx));
    Some(
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(width + 1.))
            .h_full()
            .min_h_0()
            .bg(theme::panel_bg())
            .border_l_1()
            .border_color(theme::section_border())
            .children(compare)
            .children(commit)
            .children(split)
            .children(multi)
            .children(files)
            .child(details_resizer(width, cx))
            .into_any_element(),
    )
}

/// The panel's resizer, on its left edge.
pub fn details_resizer(width: f32, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    div()
        .id("details-resize")
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(-3.))
        .w(px(6.))
        .cursor(CursorStyle::ResizeLeftRight)
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            if e.click_count == 2 {
                this.details_w = DETAILS_W;
                this.save_layout();
                cx.notify();
                return;
            }
            this.side_drag = crate::sidebar::SideDrag::Details { x: e.position.x.as_f32(), w: width };
        }))
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
        f.filter = "c".into();
        f.rebuild();
        assert_eq!(f.changes.len(), 1);
        assert_eq!(f.cursor, Some(Cursor::File(0)));
        f.filter = "a".into();
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
                v.tabs[0].keys = KeyArea::Files;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |v, window, cx| window.focus(&v.tabs[0].file_filter.read(cx).focus.clone(), cx))
            .unwrap();
        cx.run_until_parked();
        window.read_with(&cx, |v, _| assert_eq!(v.tabs[0].files.as_ref().unwrap().changes.len(), 3)).unwrap();
        cx.simulate_input("b");
        cx.run_until_parked();
        window.read_with(&cx, |v, _| {
            let f = v.tabs[0].files.as_ref().unwrap();
            assert_eq!(f.filter, "b");
            assert_eq!(f.changes.iter().map(|c| c.path.as_ref()).collect::<Vec<_>>(), ["src/b.rs"]);
        }).unwrap();
        // Esc clears it first; empty, the app's Esc goes on.
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        window.read_with(&cx, |v, _| {
            let f = v.tabs[0].files.as_ref().unwrap();
            assert_eq!(f.filter, "");
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
        FileChange { path: path.to_string().into(), old_path: None, status: 'M', additions: Some(1), deletions: None, old: Value::Null, new: Value::Null, conflict: None, origin: Origin::Commit { id: "c".into() } }
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
