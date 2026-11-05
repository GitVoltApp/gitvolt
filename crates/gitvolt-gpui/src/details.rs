//! The right panel: a commit's ids, people and message, and its changed files by path or as a tree
//! (`ui/src/details`, `ui/src/files`).

use crate::app::{GitVolt, Tab, icon};
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

/// What the panel shows for the selected row, read when it's selected.
pub struct Commit {
    pub id: SharedString,
    pub parent: Option<SharedString>,
    /// Every parent, whole.
    pub parents: Vec<SharedString>,
    pub author: SharedString,
    pub date: SharedString,
    pub co_authors: Vec<SharedString>,
    pub summary: SharedString,
    pub body: Vec<SharedString>,
    pub signed: bool,
}

/// The commit's or worktree's files, and how they're listed.
pub struct Files {
    pub changes: Rc<Vec<FileChange>>,
    pub added: u32,
    pub deleted: u32,
    pub mode: ListMode,
    pub collapsed: HashSet<String>,
    pub rows: Rc<Vec<ListRow>>,
    pub list: UniformListScrollHandle,
}

impl Files {
    pub fn new(changes: Vec<FileChange>, added: u32, deleted: u32) -> Self {
        let (mode, collapsed) = (ListMode::Tree, HashSet::new());
        let rows = Rc::new(list_rows(&changes, mode, &collapsed));
        Self { changes: Rc::new(changes), added, deleted, mode, collapsed, rows, list: UniformListScrollHandle::new() }
    }

    fn relist(&mut self) {
        self.rows = Rc::new(list_rows(&self.changes, self.mode, &self.collapsed));
    }

    pub fn set_mode(&mut self, mode: ListMode) {
        self.mode = mode;
        self.relist();
    }

    pub fn toggle_folder(&mut self, path: &str) {
        if !self.collapsed.remove(path) {
            self.collapsed.insert(path.to_string());
        }
        self.relist();
    }

    pub fn toggle_all(&mut self) {
        self.collapsed = if self.collapsed.is_empty() { all_folders(&self.changes) } else { HashSet::new() };
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
                tab.files = Some(Files::new(file_changes(&files, &Origin::Commit { id }), count("added"), count("deleted")));
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
                tab.files = Some(Files::new(files, sum("added"), sum("deleted")));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn commit_of(details: &Value, message: &Value) -> Commit {
    let short = |s: &str| s.chars().take(SHORT_SHA).collect::<String>();
    Commit {
        id: short(details["id"].as_str().unwrap_or_default()).into(),
        parent: details["parents"][0].as_str().map(|p| short(p).into()),
        parents: details["parents"].as_array().map_or(&[][..], Vec::as_slice).iter().filter_map(|p| p.as_str()).map(|p| p.to_string().into()).collect(),
        author: details["author"]["name"].as_str().unwrap_or_default().to_string().into(),
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

fn commit_view(c: &Commit) -> impl IntoElement + use<> {
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
                .child(div().flex().flex_1().justify_end().when_some(c.parent.clone(), |d, p| d.child(id("parent", p)))),
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
                .child(div().flex().flex_none().h(px(28.)).items_center().gap(px(6.)).child(avatar(&c.author, "", 28.)).child(div().flex_1().min_w_0().truncate().child(c.author.clone())).child(div().flex_none().text_color(theme::text_normal()).child(c.date.clone())))
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

fn files_view(files: &Files, selected: Option<&SharedString>, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let (mut added, mut changed, mut gone, mut renamed) = (0, 0, 0, 0);
    for f in files.changes.iter() {
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
        .when(files.added > 0, |d| d.child(div().text_color(theme::status_added()).child(format!("+{}", files.added))))
        .when(files.deleted > 0, |d| d.child(div().text_color(theme::status_deleted()).child(format!("−{}", files.deleted))));
    let mode = files.mode;
    let tools = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .px(px(12.))
        .pb(px(6.))
        .child(
            div()
                .id("collapse-all")
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(8.))
                .py(px(2.))
                .border_1()
                .border_color(theme::section_border())
                .rounded(px(4.))
                .text_color(theme::text_secondary())
                .hover(|d| d.text_color(theme::text_selected()))
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(files) = this.tabs.get_mut(this.active).and_then(|t| t.files.as_mut()) {
                        files.toggle_all();
                        cx.notify();
                    }
                }))
                .child(icon("chevrons-down-up", 12., theme::text_secondary()))
                .child(if files.collapsed.is_empty() { "Collapse all" } else { "Expand all" }),
        )
        .child(segmented(vec![
            segment_with_icon("list-path", "list", "Path", mode == ListMode::Path)
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(files) = this.tabs.get_mut(this.active).and_then(|t| t.files.as_mut()) {
                        files.set_mode(ListMode::Path);
                        cx.notify();
                    }
                }))
                .into_any_element(),
            segment_with_icon("list-tree", "list-tree", "Tree", mode == ListMode::Tree)
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(files) = this.tabs.get_mut(this.active).and_then(|t| t.files.as_mut()) {
                        files.set_mode(ListMode::Tree);
                        cx.notify();
                    }
                }))
                .into_any_element(),
        ]));
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
                            let path = path.clone();
                            base.pl(px(TREE_BASE + f32::from(*depth) * TREE_LEVEL))
                                .hover(|d| d.bg(theme::hover_row()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(files) = this.tabs.get_mut(this.active).and_then(|t| t.files.as_mut()) {
                                        files.toggle_folder(&path);
                                        cx.notify();
                                    }
                                }))
                                .child(icon(if *collapsed { "chevron-right" } else { "chevron-down" }, 14., theme::text_secondary()))
                                .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                        }
                        ListRow::File { index, name, dir, depth } => {
                            let change = &changes[*index];
                            let (glyph, color) = status_icon(change.status);
                            let on = selected.as_ref() == Some(&change.path);
                            let index = *index;
                            let group: SharedString = format!("file-{i}").into();
                            let stat = |text: String, c: Hsla| div().text_size(px(11.)).text_color(c).child(text);
                            base.group(group.clone())
                                .pl(px(TREE_BASE + f32::from(*depth) * TREE_LEVEL))
                                .when(on, |d| d.bg(theme::selected_row()).text_color(theme::text_selected()))
                                .when(!on, |d| d.hover(|d| d.bg(theme::hover_row())))
                                .on_click(cx.listener(move |this, _, _, cx| this.open_file(index, cx)))
                                .child(icon(glyph, 14., color))
                                .child(div().flex_1().min_w_0().truncate().child(name.clone()).when(!dir.is_empty(), |d| d.child(div().text_color(theme::text_secondary()).child(format!("  {dir}")))))
                                .child(
                                    div()
                                        .flex()
                                        .flex_none()
                                        .gap(px(4.))
                                        .when(!on, |d| d.invisible().group_hover(group, |d| d.visible()))
                                        .when_some(change.additions.filter(|n| *n > 0), |d, n| d.child(stat(format!("+{n}"), theme::status_added())))
                                        .when_some(change.deletions.filter(|n| *n > 0), |d, n| d.child(stat(format!("−{n}"), theme::status_deleted()))),
                                )
                        }
                    }
                })
                .collect()
        }),
    )
    .track_scroll(&files.list)
    .flex_1()
    .min_h_0();
    div().flex().flex_col().flex_1().min_h_0().child(summary).child(tools).child(list)
}

pub fn details_panel(tab: &Tab, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    tab.selected?;
    let selected = tab.diff.as_ref().map(|d| d.file.path.clone());
    let files = tab.files.as_ref().map(|f| files_view(f, selected.as_ref(), cx));
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
            .when_some(tab.commit.as_ref(), |d, c| d.child(commit_view(c)))
            .children(files)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

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
