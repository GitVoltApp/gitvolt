//! Selecting rows of the graph (`ui/src/repo/store.ts`' `selectRow` and `pick`, K27; `details/CompareHeader.tsx`,
//! `details/MultiSummary.tsx`; the `selection` menu): a click selects a row, Ctrl+click toggles one, Shift+click and
//! Shift+arrows take a range from the anchor. Two rows are a compare, the first picked as FROM; three or more a
//! multi-selection, with nothing to load.

use crate::app::{GitVolt, KeyArea, Tab, icon};
use crate::details::{Files, avatar, file_changes, Origin};
use crate::model::{RowView, SHORT_SHA};
use crate::theme;
use crate::ui::menu::{MenuRow, Variant, grouped};
use crate::ui::tooltip::{Anchor, tip};
use gitvolt_core::api::Request;
use gitvolt_graph::NodeKind;
use gpui::prelude::*;
use gpui::*;
use std::rc::Rc;

/// The rows of a compare or a multi-selection, in the order they were picked; a compare's FROM is the first, its TO the
/// second. Empty for a single selection. `cursor` is the keyboard's row, `anchor` where a Shift range starts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Picks {
    pub rows: Vec<usize>,
    pub anchor: Option<usize>,
    pub cursor: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelectMods {
    pub ctrl: bool,
    pub shift: bool,
}

impl From<Modifiers> for SelectMods {
    fn from(m: Modifiers) -> Self {
        Self { ctrl: m.control || m.platform, shift: m.shift }
    }
}

/// What a compare has on its TO side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompareTo {
    Commit(String),
    Worktree(String),
}

/// A compare in the details panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compare {
    pub from: String,
    pub to: CompareTo,
}

/// What a press comes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Nothing,
    One(usize),
    Pick { rows: Vec<usize>, anchor: Option<usize>, cursor: usize },
}

/// What `rows` (pick order) are, by their kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    One(usize),
    Compare(usize, usize),
    /// A commit's row and a WIP row's, in that order.
    Worktree(usize, usize),
    Multi(Vec<usize>),
}

/// `pick`: a ⋮ row never joins. One row is a single selection; two are a compare, a commit with a WIP row against that
/// working tree, and two WIP rows can't be compared, so the cursor's is selected alone.
pub fn shape_of(picked: &[usize], cursor: usize, kind: &dyn Fn(usize) -> Option<NodeKind>) -> Shape {
    let rows: Vec<usize> = picked.iter().copied().filter(|i| kind(*i) != Some(NodeKind::Collapsed)).collect();
    match rows.as_slice() {
        [] => Shape::One(cursor),
        [one] => Shape::One(*one),
        [a, b] => {
            let wip = |i: usize| kind(i) == Some(NodeKind::Wip);
            match (wip(*a), wip(*b)) {
                (false, false) => Shape::Compare(*a, *b),
                (true, true) => Shape::One(cursor),
                (false, true) => Shape::Worktree(*a, *b),
                (true, false) => Shape::Worktree(*b, *a),
            }
        }
        _ => Shape::Multi(rows),
    }
}

/// `selectRow`: from the selection as it is, what a press on `index` with `mods` does.
pub fn step(selected: Option<usize>, picks: &Picks, kind: &dyn Fn(usize) -> Option<NodeKind>, index: usize, mods: SelectMods) -> Step {
    let Some(here) = kind(index) else { return Step::Nothing };
    if here == NodeKind::Collapsed && !mods.shift {
        return Step::One(index);
    }
    let current: Vec<usize> = if picks.rows.is_empty() { selected.into_iter().collect() } else { picks.rows.clone() };
    let anchor = if picks.rows.is_empty() { selected } else { picks.anchor.or(picks.cursor) };
    if mods.shift {
        let Some(from) = anchor else { return Step::One(index) };
        let span: Vec<usize> = if index >= from { (from..=index).collect() } else { (index..=from).rev().collect() };
        let rows = if mods.ctrl {
            let mut rows = current.clone();
            rows.extend(span.into_iter().filter(|i| !current.contains(i)));
            rows
        } else {
            span
        };
        return Step::Pick { rows, anchor: Some(from), cursor: index };
    }
    if mods.ctrl && current.contains(&index) {
        if current.len() == 1 {
            return Step::Nothing;
        }
        let rows: Vec<usize> = current.into_iter().filter(|i| *i != index).collect();
        let last = *rows.last().unwrap_or(&index);
        let cursor = match picks.cursor {
            Some(c) if c != index => c,
            _ => last,
        };
        return Step::Pick { rows, anchor: Some(if anchor == Some(index) { last } else { anchor.unwrap_or(last) }), cursor };
    }
    if mods.ctrl && !current.is_empty() {
        let mut rows = current;
        rows.push(index);
        return Step::Pick { rows, anchor: Some(index), cursor: index };
    }
    Step::One(index)
}

/// `CommitRef`: one selected commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRef {
    pub oid: String,
    pub summary: String,
    pub merge: bool,
}

/// The rows a selection's menu goes by: commits only (no WIP, no stash), newest first.
pub fn commit_refs(rows: &[usize], all: &[RowView]) -> Vec<CommitRef> {
    let mut sorted = rows.to_vec();
    sorted.sort_unstable();
    sorted
        .into_iter()
        .filter_map(|i| all.get(i))
        .filter(|r| matches!(r.kind, NodeKind::Commit | NodeKind::Merge))
        .map(|r| CommitRef { oid: r.id.to_string(), summary: r.summary.to_string(), merge: r.parents.len() > 1 })
        .collect()
}

impl Tab {
    /// The selected rows as a set, for painting.
    pub fn picked(&self) -> Rc<std::collections::HashSet<usize>> {
        Rc::new(self.picks.rows.iter().copied().collect())
    }

    /// A multi-selection's commits, newest first: more than a compare holds.
    pub fn multi_ids(&self) -> Option<Vec<usize>> {
        if self.picks.rows.len() < 3 {
            return None;
        }
        let mut rows = self.picks.rows.clone();
        rows.sort_unstable();
        Some(rows)
    }

    /// The selected commits, however many: a single commit's, a compare's two, a multi-selection's.
    pub fn selected_commits(&self) -> Vec<CommitRef> {
        let Some(g) = self.graph.as_ref() else { return Vec::new() };
        if self.picks.rows.is_empty() { commit_refs(&self.selected.into_iter().collect::<Vec<_>>(), &g.rows) } else { commit_refs(&self.picks.rows, &g.rows) }
    }

    /// Whether the row belongs to a selection of two or more commits: its right-click is the selection's.
    pub fn in_commit_selection(&self, row: usize) -> bool {
        self.picks.rows.contains(&row) && self.selected_commits().len() >= 2
    }
}

impl GitVolt {
    /// A press on a graph row, or a keyboard move with `mods`.
    pub fn select_row(&mut self, index: usize, mods: SelectMods, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(g) = tab.graph.clone() else { return };
        let kind = |i: usize| g.rows.get(i).map(|r| r.kind);
        match step(tab.selected, &tab.picks, &kind, index, mods) {
            Step::Nothing => {}
            // A press on what's already selected alone changes nothing.
            Step::One(i) if tab.selected == Some(i) && tab.picks.rows.is_empty() && tab.compare.is_none() => {}
            Step::One(i) => self.select(i, cx),
            Step::Pick { rows, anchor, cursor } => self.pick(rows, anchor, cursor, cx),
        }
    }

    /// The graph's own keys: Home, End, → and Enter on the first changed file, the menu key and Shift+F10 on the row.
    pub fn graph_key(&mut self, k: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.tabs.get(self.active) else { return false };
        if tab.keys != KeyArea::Graph || tab.diff.is_some() || tab.history.is_some() || tab.rebase_shown().is_some() {
            return false;
        }
        let Some(g) = tab.graph.clone() else { return false };
        let m = k.modifiers;
        if m.control || m.alt || m.platform {
            return false;
        }
        match k.key.as_str() {
            "home" if !g.rows.is_empty() => self.select(0, cx),
            "end" if !g.rows.is_empty() => self.select(g.rows.len() - 1, cx),
            "enter" if !m.shift && tab.selected.and_then(|i| g.rows.get(i)).is_some_and(|r| !r.ghosts.is_empty()) => {
                let Some(ix) = tab.selected else { return false };
                let ghosts = g.rows[ix].ghosts.clone();
                let handle = tab.list.0.borrow().base_handle.clone();
                let at = handle.bounds().origin + point(px(300.), handle.offset().y + px(ix as f32 * crate::metrics::row_h()));
                self.open_ghosts(ghosts, Bounds::new(at, size(px(8.), px(8.))), window, cx);
            }
            "enter" if !m.shift && tab.selected.and_then(|i| g.rows.get(i)).is_some_and(|r| r.collapsed.is_some() || r.fold.is_some()) => {
                let Some(ix) = tab.selected else { return false };
                self.open_reveal(ix, true, cx);
            }
            "right" | "enter" if !m.shift => {
                let first = tab.files.as_ref().and_then(|f| f.rows.iter().find_map(|r| if let crate::details::ListRow::File { index, .. } = r { Some(*index) } else { None }));
                let Some(first) = first else { return k.key == "right" };
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.keys = KeyArea::Files;
                }
                self.open_file(first, cx);
            }
            "menu" | "f10" if k.key == "menu" || m.shift => {
                let Some(ix) = tab.selected else { return false };
                let Some(row) = g.rows.get(ix) else { return false };
                if matches!(row.kind, NodeKind::Wip | NodeKind::Collapsed) {
                    return false;
                }
                let top = tab.list.0.borrow().base_handle.bounds().origin.y + tab.list.0.borrow().base_handle.offset().y;
                let weak = cx.weak_entity();
                let rows = self.row_menu(ix, &weak);
                let x = tab.list.0.borrow().base_handle.bounds().origin.x;
                crate::ui::menu::open_context_for(format!("row:{ix}"), rows, point(x, top + px((ix + 1) as f32 * crate::metrics::row_h())), window, cx);
            }
            _ => return false,
        }
        true
    }

    /// The commit under the pointer shows its branch's dimmed chip. Nothing re-renders crossing rows without one.
    pub fn hover_row(&mut self, ix: usize, entered: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let Some(g) = tab.graph.as_ref() else { return };
        let Some(row) = g.rows.get(ix) else { return };
        let opens = row.collapsed.is_some() && g.view.is_some();
        let target = (entered && g.membership.get(ix).is_some_and(Option::is_some)).then(|| row.id.clone());
        if opens {
            if entered {
                self.hover_reveal(ix, true, cx);
            } else {
                self.hover_reveal(ix, false, cx);
                self.leave_reveal(cx);
            }
            return;
        }
        if !entered && tab.hover_chip.as_ref() != Some(&row.id) {
            return;
        }
        if tab.hover_chip != target {
            tab.hover_chip = target;
            cx.notify();
        }
    }

    /// A right-click on a row: in a selection of two or more commits, the selection's menu (the row's own when none of
    /// its rows applies); elsewhere its commit's. A ⋮ row has none.
    pub fn row_menu(&self, ix: usize, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        if tab.graph.as_ref().and_then(|g| g.rows.get(ix)).is_some_and(|r| r.kind == NodeKind::Collapsed) {
            return Vec::new();
        }
        if tab.in_commit_selection(ix) {
            let rows = self.selection_rows(this);
            if !rows.is_empty() {
                return rows;
            }
        }
        self.commit_rows(ix, None, this)
    }

    /// A double click on a row: another worktree's WIP row becomes the active one.
    pub fn double_click_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(row) = tab.graph.as_ref().and_then(|g| g.rows.get(ix)) else { return };
        if row.kind == NodeKind::Wip
            && let Some(w) = row.worktree.clone().filter(|w| *w != tab.worktree)
        {
            self.switch_worktree(w, cx);
        }
    }

    /// `pick`: the rows, shown as selected, and what they come to.
    pub fn pick(&mut self, rows: Vec<usize>, anchor: Option<usize>, cursor: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(g) = tab.graph.clone() else { return };
        let again = rows.clone();
        if tab.selected != Some(cursor) && self.ask_unsaved(move |v, cx| v.pick(again.clone(), anchor, cursor, cx), cx) {
            return;
        }
        let kind = |i: usize| g.rows.get(i).map(|r| r.kind);
        let shape = shape_of(&rows, cursor, &kind);
        let kept: Vec<usize> = rows.into_iter().filter(|i| kind(*i) != Some(NodeKind::Collapsed)).collect();
        let picks = Picks { rows: kept, anchor, cursor: Some(cursor) };
        match shape {
            Shape::One(i) => self.select(i, cx),
            Shape::Compare(a, b) => {
                let compare = Compare { from: g.rows[a].id.to_string(), to: CompareTo::Commit(g.rows[b].id.to_string()) };
                self.start_compare(compare, picks, None, cx);
            }
            Shape::Worktree(commit, wip) => {
                let worktree = g.rows[wip].worktree.clone().unwrap_or_default();
                let compare = Compare { from: g.rows[commit].id.to_string(), to: CompareTo::Worktree(worktree) };
                self.start_compare(compare, picks, None, cx);
            }
            Shape::Multi(_) => {
                let Some(tab) = self.tabs.get_mut(self.active) else { return };
                tab.selected = Some(cursor);
                tab.picks = picks;
                tab.compare = None;
                tab.keys = KeyArea::Graph;
                tab.commit = None;
                tab.files = None;
                tab.diff = None;
                tab.history = None;
                tab.list.scroll_to_item(cursor, ScrollStrategy::Nearest);
                cx.notify();
            }
        }
    }

    /// Selects `picks` and reads the compare. `reopen`: the file open before a swap, which moves to the same file in the
    /// reversed list, or closes.
    pub fn start_compare(&mut self, compare: Compare, picks: Picks, reopen: Option<(String, Option<String>)>, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let repo = tab.id;
        if let Some(c) = picks.cursor {
            tab.selected = Some(c);
            tab.list.scroll_to_item(c, ScrollStrategy::Nearest);
        }
        tab.picks = picks;
        tab.history = None;
        if reopen.is_none() {
            tab.diff = None;
        }
        tab.keys = KeyArea::Graph;
        tab.commit = None;
        tab.files = None;
        let spec = match &compare.to {
            CompareTo::Commit(to) => gitvolt_core::diff::DiffSpec::Compare { from: compare.from.clone(), to: to.clone() },
            CompareTo::Worktree(worktree) => gitvolt_core::diff::DiffSpec::Worktree { from: compare.from.clone(), worktree: worktree.clone() },
        };
        tab.compare = Some(compare);
        cx.notify();
        if cfg!(test) {
            return;
        }
        let api = self.api.clone();
        let done = self.rt.spawn(async move { api.dispatch(Request::FileList { repo, spec }).await });
        cx.spawn(async move |this, cx| {
            let answer = done.await;
            this.update(cx, |this, cx| {
                let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo).filter(|t| t.compare.is_some()) else { return };
                let Ok(Ok(files)) = answer else {
                    if reopen.is_some() {
                        tab.diff = None;
                    }
                    return cx.notify();
                };
                let count = |k: &str| files[k].as_u64().unwrap_or(0) as u32;
                let mut list = Files::new(file_changes(&files, &Origin::Commit { id: String::new() }), count("added"), count("deleted"));
                list.by_status = tab.sort_by_status;
                if let Some(Compare { to: CompareTo::Commit(to), .. }) = &tab.compare {
                    list.commit = Some(to.clone());
                }
                let at = reopen.as_ref().and_then(|(path, old)| list.changes.iter().position(|f| f.path.as_ref() == path.as_str() || (old.as_deref() == Some(f.path.as_ref()) && f.old_path.as_deref() == Some(path.as_str()))));
                tab.files = Some(list);
                if reopen.is_some() {
                    tab.diff = None;
                }
                if let Some(at) = at {
                    this.open_file(at, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// `from` → `to` in the details panel; Esc or the × leaves it.
    pub fn compare_commits(&mut self, from: String, to: String, cx: &mut Context<Self>) {
        let Some(g) = self.tabs.get(self.active).and_then(|t| t.graph.clone()) else { return };
        let (i, j) = (g.by_sha.get(&from).copied(), g.by_sha.get(&to).copied());
        if let (Some(i), Some(j)) = (i, j) {
            return self.pick(vec![i, j], Some(i), i, cx);
        }
        self.start_compare(Compare { from, to: CompareTo::Commit(to) }, Picks::default(), None, cx);
    }

    /// A commit against the open worktree's files on disk; its WIP row, if it has one, is selected too.
    pub fn compare_with_worktree(&mut self, from: String, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let worktree = tab.worktree.clone();
        let commit = tab.graph.as_ref().and_then(|g| g.by_sha.get(&from).copied());
        let wip = tab.graph.as_ref().and_then(|g| g.rows.iter().position(|r| r.worktree.as_deref() == Some(worktree.as_str())));
        let rows: Vec<usize> = [commit, wip].into_iter().flatten().collect();
        self.start_compare(Compare { from, to: CompareTo::Worktree(worktree) }, Picks { rows, anchor: commit, cursor: commit }, None, cx);
    }

    /// Reverses a two-commit compare, keeping the open file if the reversed list has it.
    pub fn swap_compare(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(Compare { from, to: CompareTo::Commit(to) }) = tab.compare.clone() else { return };
        let reopen = tab.diff.as_ref().map(|d| (d.file.path.to_string(), d.file.old_path.as_ref().map(|p| p.to_string())));
        let mut picks = tab.picks.clone();
        picks.rows.reverse();
        self.start_compare(Compare { from: to, to: CompareTo::Commit(from) }, picks, reopen, cx);
    }

    /// Leaves a compare or multi-selection for its anchor row alone. False: there was none to leave.
    pub fn exit_compare(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active) else { return false };
        if tab.compare.is_none() && tab.picks.rows.is_empty() {
            return false;
        }
        let target = tab.picks.anchor.or(tab.picks.rows.last().copied());
        tab.compare = None;
        tab.picks = Picks::default();
        tab.keys = KeyArea::Graph;
        tab.files = None;
        tab.diff = None;
        match target {
            Some(i) => {
                tab.selected = None;
                self.select(i, cx);
            }
            None => {
                tab.selected = None;
                tab.commit = None;
                cx.notify();
            }
        }
        true
    }

    /// A new graph: the picked rows follow their commits; those gone leave, so two left are a compare and one a single
    /// selection. `before`: the old rows' ids.
    pub fn remap_picks(&mut self, repo: u32, before: &[SharedString], cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) else { return };
        if tab.picks.rows.is_empty() {
            return;
        }
        let Some(g) = tab.graph.clone() else { return };
        let at = |i: Option<usize>| i.and_then(|i| before.get(i)).and_then(|id| g.by_sha.get(id.as_ref()).copied());
        let was = tab.picks.rows.len();
        let rows: Vec<usize> = tab.picks.rows.iter().filter_map(|i| at(Some(*i))).collect();
        let (anchor, cursor) = (at(tab.picks.anchor), at(tab.picks.cursor));
        let last = rows.last().copied();
        tab.picks = Picks { rows: rows.clone(), anchor, cursor };
        if rows.len() == was {
            if cursor.is_some() {
                tab.selected = cursor;
            }
            return;
        }
        let Some(last) = last else {
            tab.picks = Picks::default();
            tab.compare = None;
            tab.selected = None;
            tab.commit = None;
            tab.files = None;
            tab.diff = None;
            return cx.notify();
        };
        if self.tabs.get(self.active).is_some_and(|t| t.id == repo) {
            self.pick(rows, anchor.or(Some(last)), cursor.unwrap_or(last), cx);
        }
    }

    /// A right-click in a selection of two or more commits: its own rows.
    pub fn selection_rows(&self, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let commits = tab.selected_commits();
        grouped(vec![self.squash_rows(&commits, this), self.pick_rows(&commits, this), self.selection_from_here_rows(&commits, this)])
    }

    /// `pickRows`: Cherry-pick and Revert for `commits`, newest first.
    pub fn pick_rows(&self, commits: &[CommitRef], this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        if commits.is_empty() || commits.iter().any(|c| c.merge) {
            return Vec::new();
        }
        let head = tab.head_branch();
        let head_sha = tab.head_sha();
        let branch = head.clone().unwrap_or_else(|| "HEAD".to_string());
        let conflicted = tab.graph.as_ref().and_then(|g| g.rows.iter().find(|r| r.worktree.as_deref() == Some(tab.worktree.as_str()))).and_then(|r| r.wip.as_ref()).is_some_and(|w| w.conflicted > 0);
        let busy = tab.busy_reason().or_else(|| conflicted.then(|| "Resolve conflicts first".to_string())).or_else(|| head.is_none().then(|| "Check out a branch first".to_string())).or_else(|| head_sha.is_none().then(|| "Make a first commit first".to_string()));
        let g = tab.graph.as_deref();
        let in_head = |c: &CommitRef| head_sha.as_deref().and_then(|h| g.and_then(|g| crate::refs::is_ancestor(g, &c.oid, h)));
        let short = |s: &str| s.chars().take(SHORT_SHA).collect::<String>();
        let name_of = |cs: &[CommitRef]| if let [one] = cs { format!("{} {}", short(&one.oid), one.summary).trim().to_string() } else { format!("the {} commits", cs.len()) };
        let go = |pick: bool, cs: Vec<CommitRef>, no_commit: bool| -> crate::ui::Action {
            let (weak, branch) = (this.clone(), head.clone());
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let (cs, branch) = (cs.clone(), branch.clone());
                if let Some(branch) = branch {
                    weak.update(cx, |v, cx| v.start_sequence(pick, cs, no_commit, branch, cx)).ok();
                }
            })
        };
        let mut rows = Vec::new();
        let fresh: Vec<CommitRef> = commits.iter().filter(|c| in_head(c) != Some(true)).cloned().collect();
        let again = fresh.is_empty() && !commits.iter().any(|c| Some(c.oid.as_str()) == head_sha.as_deref());
        let picks: Vec<CommitRef> = if again { commits.to_vec() } else { fresh };
        if !picks.is_empty() {
            let named = name_of(&picks);
            let what = if picks.len() == 1 { "Cherry-pick".to_string() } else { format!("Cherry-pick {} commits", picks.len()) };
            let tooltip = if picks.len() == 1 { format!("Apply {named} on top of {branch}") } else { format!("Apply {named} on top of {branch}, oldest first") };
            let run = go(true, picks.clone(), false);
            rows.push(
                MenuRow::action("commit.cherryPick", format!("{what} onto {branch}"), "cherry", tooltip, move |w, cx| run(w, cx))
                    .variants(vec![Variant { id: "noCommit".into(), label: Some("No commit".into()), icon: None, tooltip: format!("Apply the changes to {branch}'s files and stage them, without committing").into(), run: go(true, picks, true), disabled: busy.clone().map(Into::into) }], None)
                    .disabled_if(busy.clone()),
            );
        }
        if !commits.iter().any(|c| in_head(c) == Some(false)) {
            let named = name_of(commits);
            let one = commits.len() == 1;
            let label = if one { "Revert this commit".to_string() } else { format!("Revert {} commits", commits.len()) };
            let tooltip = if one { format!("Make a commit on {branch} that undoes {named}") } else { format!("Make a commit on {branch} undoing each of {named}, newest first") };
            let run = go(false, commits.to_vec(), false);
            rows.push(
                MenuRow::action("commit.revert", label, "undo-2", tooltip, move |w, cx| run(w, cx))
                    .variants(vec![Variant { id: "noCommit".into(), label: Some("No commit".into()), icon: None, tooltip: format!("Undo the changes in {branch}'s files and stage that, without committing").into(), run: go(false, commits.to_vec(), true), disabled: busy.clone().map(Into::into) }], None)
                    .disabled_if(busy),
            );
        }
        rows
    }
}

/// One compared or multi-selected commit: its author's avatar, its one line and its date, from its graph row; outside the
/// loaded graph, its short id.
fn compare_commit(id: &str, row: Option<&RowView>) -> AnyElement {
    let Some(row) = row else {
        return div().flex().flex_none().h(px(28.)).items_center().child(div().font_family("JetBrains Mono").text_color(theme::text_selected()).child(id.chars().take(SHORT_SHA).collect::<String>())).into_any_element();
    };
    let (title, who) = (row.summary.clone(), crate::details::person_label(&row.author, &row.email));
    let summary = div().flex_1().min_w_0().truncate().text_color(theme::text_selected()).child(row.summary.clone());
    let card = Anchor::new(summary).card(format!("compare:{id}"), move |_, _| {
        div().flex().flex_col().flex_none().gap(px(2.)).px(px(8.)).py(px(5.)).rounded(px(4.)).border_1().border_color(theme::section_border()).bg(theme::panel_bg1()).whitespace_nowrap().shadow(crate::ui::shadow(theme::shadow_tooltip(), 2., 8.)).child(div().text_color(theme::text_selected()).font_weight(FontWeight::SEMIBOLD).child(title.clone())).child(who.clone()).into_any_element()
    });
    div()
        .flex()
        .flex_none()
        .h(px(28.))
        .items_center()
        .gap(px(8.))
        .min_w_0()
        .child(avatar(&row.author, &row.email, 20.))
        .child(card)
        .child(div().flex_none().text_size(px(12.)).line_height(px(16.8)).text_color(theme::text_normal()).child(if row.time > 0 { SharedString::from(crate::model::date_cell(row.time)) } else { row.date.clone() }))
        .into_any_element()
}

/// The compared (or multi-selected) working tree, as the WIP header names it.
fn compare_worktree(row: Option<&RowView>) -> AnyElement {
    let name = row.and_then(|r| r.wip.as_ref()).and_then(|w| w.name.clone()).unwrap_or_else(|| "Working tree".into());
    let counts = row.and_then(|r| r.wip.as_ref()).map(crate::model::wip_counts_text);
    div()
        .flex()
        .flex_none()
        .h(px(28.))
        .items_center()
        .gap(px(8.))
        .min_w_0()
        .child(div().flex().flex_1().min_w_0().gap(px(4.)).child(div().flex_none().italic().text_color(theme::text_secondary()).child("// WIP")).child(div().min_w_0().truncate().text_color(theme::text_selected()).child(name)))
        .children(counts.map(|c| div().flex_none().text_size(px(12.)).text_color(theme::text_disabled()).child(c)))
        .into_any_element()
}

/// `CompareHeader`: the bar `Comparing a1b2c3 → e4f5g6 [⇄] [×]`, then the two sides.
pub fn compare_header(tab: &Tab, c: &Compare, cx: &mut Context<GitVolt>) -> AnyElement {
    let g = tab.graph.as_deref();
    let row_of = |id: &str| g.and_then(|g| g.by_sha.get(id).and_then(|i| g.rows.get(*i)));
    let code = |t: String| div().flex_none().font_family("JetBrains Mono").text_color(theme::text_selected()).child(t);
    let short = |s: &str| s.chars().take(SHORT_SHA).collect::<String>();
    let to_text: AnyElement = match &c.to {
        CompareTo::Commit(to) => code(short(to)).into_any_element(),
        CompareTo::Worktree(_) => div().flex_none().child("working tree").into_any_element(),
    };
    let swap = matches!(c.to, CompareTo::Commit(_));
    let to_row: AnyElement = match &c.to {
        CompareTo::Commit(to) => compare_commit(to, row_of(to)),
        CompareTo::Worktree(w) => compare_worktree(g.and_then(|g| g.rows.iter().find(|r| r.worktree.as_deref() == Some(w.as_str())))),
    };
    div()
        .flex()
        .flex_col()
        .flex_none()
        .px(px(12.))
        .pb(px(6.))
        .border_b_1()
        .border_color(theme::section_border())
        .child(
            div()
                .flex()
                .flex_none()
                .h(px(37.))
                .mx(px(-12.))
                .pl(px(12.))
                .pr(px(8.))
                .items_center()
                .gap(px(6.))
                .border_b_1()
                .border_color(theme::section_border())
                .child(div().flex().flex_1().min_w_0().items_center().whitespace_nowrap().overflow_hidden().gap(px(4.)).child(div().flex_none().child("Comparing")).child(code(short(&c.from))).child(div().flex_none().child(icon("arrow-right", 12., theme::text_normal()))).child(to_text))
                .when(swap, |d| d.child(tip("Swap from and to", crate::ui::anchor("compare-swap", div().id("compare-swap").flex().flex_none().size(px(22.)).items_center().justify_center().rounded(px(3.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).on_click(cx.listener(|this, _, _, cx| this.swap_compare(cx))).child(icon("arrow-left-right", 14., theme::text_secondary()))))))
                .child(tip("Exit compare (Esc)", div().id("compare-close").flex().flex_none().size(px(22.)).items_center().justify_center().rounded(px(3.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).on_click(cx.listener(|this, _, _, cx| {
                    this.exit_compare(cx);
                })).child(icon("x", 14., theme::text_secondary())))),
        )
        .child(div().flex().flex_col().flex_none().gap(px(2.)).pt(px(4.)).child(compare_commit(&c.from, row_of(&c.from))).child(to_row))
        .into_any_element()
}

/// `MultiSummary`: three or more selected rows: the bar, then one row per commit, newest first. No diff and no file list.
pub fn multi_summary(tab: &Tab, rows: Vec<usize>, cx: &mut Context<GitVolt>) -> AnyElement {
    const ROW_H: f32 = 30.;
    let Some(g) = tab.graph.clone() else { return div().into_any_element() };
    let count = rows.len();
    let list = uniform_list(
        "multi-commits",
        count,
        cx.processor(move |_, range: std::ops::Range<usize>, _, _| {
            range
                .map(|i| {
                    let row = g.rows.get(rows[i]);
                    let inner = match row {
                        Some(r) if r.kind == NodeKind::Wip => compare_worktree(Some(r)),
                        Some(r) => compare_commit(&r.id, Some(r)),
                        None => div().into_any_element(),
                    };
                    div().h(px(ROW_H)).min_w_0().child(inner).into_any_element()
                })
                .collect()
        }),
    )
    .flex_1()
    .min_h_0()
    .px(px(12.))
    .pt(px(4.))
    .pb(px(6.));
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(
            div()
                .flex()
                .flex_none()
                .h(px(37.))
                .pl(px(12.))
                .pr(px(8.))
                .items_center()
                .gap(px(6.))
                .border_b_1()
                .border_color(theme::section_border())
                .child(div().flex_1().min_w_0().truncate().child(format!("{count} commits selected")))
                .child(tip("Exit multi-selection (Esc)", div().id("multi-close").flex().flex_none().size(px(22.)).items_center().justify_center().rounded(px(3.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).on_click(cx.listener(|this, _, _, cx| {
                    this.exit_compare(cx);
                })).child(icon("x", 14., theme::text_secondary())))),
        )
        .child(list)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn kinds(wip: &'static [usize], collapsed: &'static [usize]) -> impl Fn(usize) -> Option<NodeKind> {
        move |i| (i < 100).then(|| if wip.contains(&i) { NodeKind::Wip } else if collapsed.contains(&i) { NodeKind::Collapsed } else { NodeKind::Commit })
    }
    const SHIFT: SelectMods = SelectMods { ctrl: false, shift: true };
    const CTRL: SelectMods = SelectMods { ctrl: true, shift: false };

    #[test]
    fn a_plain_press_selects_one_row() {
        let picks = Picks { rows: vec![1, 2, 3], anchor: Some(1), cursor: Some(3) };
        assert_eq!(step(Some(3), &picks, &kinds(&[], &[]), 7, SelectMods::default()), Step::One(7));
    }

    #[test]
    fn ctrl_adds_a_row_and_takes_it_out_but_never_the_only_one() {
        let k = kinds(&[], &[]);
        assert_eq!(step(Some(2), &Picks::default(), &k, 5, CTRL), Step::Pick { rows: vec![2, 5], anchor: Some(5), cursor: 5 });
        assert_eq!(step(Some(2), &Picks::default(), &k, 2, CTRL), Step::Nothing);
        let picks = Picks { rows: vec![2, 5, 7], anchor: Some(7), cursor: Some(7) };
        assert_eq!(step(Some(7), &picks, &k, 7, CTRL), Step::Pick { rows: vec![2, 5], anchor: Some(5), cursor: 5 });
        assert_eq!(step(Some(7), &picks, &k, 2, CTRL), Step::Pick { rows: vec![5, 7], anchor: Some(7), cursor: 7 });
    }

    #[test]
    fn shift_takes_the_range_from_the_anchor_out() {
        let k = kinds(&[], &[]);
        assert_eq!(step(Some(4), &Picks::default(), &k, 7, SHIFT), Step::Pick { rows: vec![4, 5, 6, 7], anchor: Some(4), cursor: 7 });
        assert_eq!(step(Some(4), &Picks::default(), &k, 2, SHIFT), Step::Pick { rows: vec![4, 3, 2], anchor: Some(4), cursor: 2 });
        assert_eq!(step(None, &Picks::default(), &k, 2, SHIFT), Step::One(2));
        // Ctrl with it: the range joins what's selected.
        let picks = Picks { rows: vec![1, 2], anchor: Some(2), cursor: Some(2) };
        assert_eq!(step(Some(2), &picks, &k, 4, SelectMods { ctrl: true, shift: true }), Step::Pick { rows: vec![1, 2, 3, 4], anchor: Some(2), cursor: 4 });
    }

    #[test]
    fn a_collapsed_row_stands_alone_and_a_range_goes_around_it() {
        let k = kinds(&[], &[3]);
        assert_eq!(step(Some(1), &Picks::default(), &k, 3, CTRL), Step::One(3));
        assert_eq!(shape_of(&[2, 3, 4], 4, &k), Shape::Compare(2, 4));
    }

    #[test]
    fn what_rows_come_to() {
        let k = kinds(&[0, 9], &[]);
        assert_eq!(shape_of(&[5], 5, &k), Shape::One(5));
        assert_eq!(shape_of(&[3, 5], 5, &k), Shape::Compare(3, 5));
        assert_eq!(shape_of(&[0, 5], 5, &k), Shape::Worktree(5, 0));
        assert_eq!(shape_of(&[5, 0], 0, &k), Shape::Worktree(5, 0));
        assert_eq!(shape_of(&[0, 9], 9, &k), Shape::One(9));
        assert_eq!(shape_of(&[1, 2, 3], 3, &k), Shape::Multi(vec![1, 2, 3]));
    }
}
