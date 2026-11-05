//! The left sidebar (`ui/src/sidebar`): stacked panels sharing the column's height, one filter over
//! all of them, dividers between open panels, the width's resizer, and the narrow strip. `build_panels`
//! and `layout_panels` are `model.ts` and `layout.ts`.

use crate::app::{GitVolt, Tab, icon};
use crate::model::SHORT_SHA;
use crate::theme;
use crate::ui::controls::clearable;
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::menu;
use crate::ui::tooltip::{Anchor, Placement, tip};
use gitvolt_core::api::Request;
use gpui::prelude::*;
use gpui::*;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

pub const HEADER_H: f32 = 26.;
pub const ROW_H: f32 = 24.;
pub const MIN_ROWS: f32 = 3.;
pub const MIN_PANEL_H: f32 = HEADER_H + MIN_ROWS * ROW_H;
const ICON_W: f32 = 13.;
const ROW_GAP: f32 = 6.;
const ROW_PAD: f32 = 8.;
const DEFAULT_WEIGHT: f32 = 200.;
pub const DEFAULT_SIDEBAR_W: f32 = 240.;
pub const MIN_W: f32 = 160.;
pub const MAX_W: f32 = 480.;
pub const NARROW_W: f32 = 41.;
const FILTER_H: f32 = 41.;

/// A tree row's left padding at `depth` (1-based): a child's icon sits under its parent's name.
pub fn row_indent(depth: u8) -> f32 {
    ROW_PAD + (f32::from(depth.max(1)) - 1.) * (ICON_W + ROW_GAP)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionKind {
    Local,
    Remote,
    Worktrees,
    Stashes,
    Tags,
    Submodules,
}

impl SectionKind {
    pub fn key(self) -> &'static str {
        match self {
            SectionKind::Local => "local",
            SectionKind::Remote => "remote",
            SectionKind::Worktrees => "worktrees",
            SectionKind::Stashes => "stashes",
            SectionKind::Tags => "tags",
            SectionKind::Submodules => "submodules",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SectionKind::Local => "Local",
            SectionKind::Remote => "Remote",
            SectionKind::Worktrees => "Worktrees",
            SectionKind::Stashes => "Stashes",
            SectionKind::Tags => "Tags",
            SectionKind::Submodules => "Submodules",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            SectionKind::Local => "laptop",
            SectionKind::Remote => "cloud",
            SectionKind::Worktrees => "tree-pine",
            SectionKind::Stashes => "archive",
            SectionKind::Tags => "tag",
            SectionKind::Submodules => "boxes",
        }
    }

    fn nests(self) -> bool {
        matches!(self, SectionKind::Local | SectionKind::Remote | SectionKind::Tags)
    }

    fn empty(self) -> &'static str {
        "Nothing here"
    }
}

/// What a row is, with what its menu and card need.
#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    Local { full: String, is_head: bool, upstream: Option<String>, ahead: u32, behind: u32, gone: bool, summary: String, author: String },
    Remote { remote: String, full: String, summary: String, author: String },
    Worktree { path: String, branch: Option<String>, head: Option<String>, is_main: bool, is_current: bool },
    Stash { index: u32, message: String },
    Tag { annotated: bool, message: Option<String>, tagger: Option<String> },
    Submodule { url: Option<String>, recorded: String, head: Option<String>, branch: Option<String>, status: String, real_name: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SideItem {
    pub key: String,
    pub name: String,
    pub target: Option<String>,
    pub time: i64,
    pub kind: ItemKind,
}

impl SideItem {
    /// What the filter matches against: a remote branch carries its remote's name.
    fn full_name(&self) -> String {
        match &self.kind {
            ItemKind::Remote { remote, .. } => format!("{remote}/{}", self.name),
            _ => self.name.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub kind: SectionKind,
    pub items: Vec<SideItem>,
}

fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

fn opt(v: &Value, key: &str) -> Option<String> {
    v[key].as_str().map(str::to_string)
}

/// `sectionsOf`: the sidebar's sections, in their fixed order.
pub fn sections_of(v: &Value, worktree: &str) -> Vec<Section> {
    let list = |key: &str| v[key].as_array().cloned().unwrap_or_default();
    let n = |b: &Value, key: &str| b[key].as_u64().unwrap_or(0) as u32;
    let locals = list("locals")
        .iter()
        .map(|b| SideItem {
            key: text(b, "fullName"),
            name: text(b, "name"),
            target: opt(b, "target"),
            time: b["tipTime"].as_i64().unwrap_or(0),
            kind: ItemKind::Local { full: text(b, "fullName"), is_head: b["isHead"].as_bool() == Some(true), upstream: opt(b, "upstream"), ahead: n(b, "ahead"), behind: n(b, "behind"), gone: b["gone"].as_bool() == Some(true), summary: text(b, "summary"), author: text(b, "author") },
        })
        .collect();
    let mut remotes = Vec::new();
    for r in list("remotes") {
        let remote = text(&r, "name");
        for b in r["branches"].as_array().map_or(&[][..], Vec::as_slice) {
            remotes.push(SideItem {
                key: text(b, "fullName"),
                name: text(b, "name").trim_start_matches(&format!("{remote}/")).to_string(),
                target: opt(b, "target"),
                time: b["tipTime"].as_i64().unwrap_or(0),
                kind: ItemKind::Remote { remote: remote.clone(), full: text(b, "fullName"), summary: text(b, "summary"), author: text(b, "author") },
            });
        }
    }
    let worktrees = list("worktrees")
        .iter()
        .map(|w| SideItem {
            key: format!("wt:{}", text(w, "path")),
            name: text(w, "name"),
            target: opt(w, "head"),
            time: 0,
            kind: ItemKind::Worktree { path: text(w, "path"), branch: opt(w, "branch"), head: opt(w, "head"), is_main: w["isMain"].as_bool() == Some(true), is_current: w["isCurrent"].as_bool() == Some(true) },
        })
        .collect();
    let stashes = list("stashes")
        .iter()
        .map(|s| SideItem { key: format!("stash:{}", n(s, "index")), name: text(s, "message"), target: opt(s, "id"), time: s["time"].as_i64().unwrap_or(0), kind: ItemKind::Stash { index: n(s, "index"), message: text(s, "message") } })
        .collect();
    let tags = list("tags")
        .iter()
        .map(|t| SideItem {
            key: text(t, "fullName"),
            name: text(t, "name"),
            target: opt(t, "target"),
            time: t["time"].as_i64().unwrap_or(0),
            kind: ItemKind::Tag { annotated: t.get("annotation").is_some_and(|a| !a.is_null()), message: t["annotation"]["message"].as_str().map(str::to_string), tagger: t["annotation"]["tagger"]["name"].as_str().or_else(|| t["annotation"]["taggerName"].as_str()).map(str::to_string) },
        })
        .collect();
    let subs: Vec<SideItem> = v["submodules"][worktree]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|m| SideItem {
            key: format!("submodule:{}", text(m, "path")),
            name: text(m, "path"),
            target: None,
            time: 0,
            kind: ItemKind::Submodule { url: opt(m, "url"), recorded: text(m, "recorded"), head: opt(m, "head"), branch: opt(m, "branch"), status: text(m, "status"), real_name: text(m, "name") },
        })
        .collect();
    let mut out = vec![
        Section { kind: SectionKind::Local, items: locals },
        Section { kind: SectionKind::Remote, items: remotes },
        Section { kind: SectionKind::Worktrees, items: worktrees },
        Section { kind: SectionKind::Stashes, items: stashes },
        Section { kind: SectionKind::Tags, items: tags },
    ];
    if !subs.is_empty() {
        out.push(Section { kind: SectionKind::Submodules, items: subs });
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub enum FlatRow {
    Folder { key: String, name: String, depth: u8, collapsed: bool, remote: Option<String> },
    Item { key: String, item: Rc<SideItem>, depth: u8, label: String },
}

impl FlatRow {
    pub fn key(&self) -> &str {
        match self {
            FlatRow::Folder { key, .. } | FlatRow::Item { key, .. } => key,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Panel {
    pub kind: SectionKind,
    pub collapsed: bool,
    /// What the filter leaves; the count the header shows.
    pub matched: usize,
    pub recent: bool,
    pub filtering: bool,
    pub rows: Vec<FlatRow>,
}

pub fn section_key(kind: SectionKind) -> String {
    format!("section:{}", kind.key())
}

fn folder_key(kind: SectionKind, path: &str) -> String {
    format!("{}:{path}", kind.key())
}

/// Case-insensitive, as the web UI's `localeCompare` with `sensitivity: 'base'` sorts names.
fn by_name(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase())
}

#[derive(Default)]
struct Node<'a> {
    folders: BTreeMap<String, Node<'a>>,
    items: Vec<&'a SideItem>,
}

fn walk(n: &Node<'_>, depth: u8, prefix: &str, kind: SectionKind, filtering: bool, collapsed: &HashSet<String>, out: &mut Vec<FlatRow>) {
    enum Entry<'b, 'a> {
        Folder(&'b str, &'b Node<'a>),
        Leaf(&'a SideItem, &'a str),
    }
    let mut entries: Vec<Entry<'_, '_>> = n.folders.iter().map(|(k, v)| Entry::Folder(k, v)).collect();
    entries.extend(n.items.iter().map(|i| Entry::Leaf(i, i.name.rsplit('/').next().unwrap_or(&i.name))));
    // One alphabetical list per level, folders and branches mixed.
    let name_of = |e: &Entry<'_, '_>| match e {
        Entry::Folder(f, _) => f.to_string(),
        Entry::Leaf(_, n) => n.to_string(),
    };
    entries.sort_by(|a, b| by_name(&name_of(a), &name_of(b)));
    for e in entries {
        match e {
            Entry::Leaf(item, name) => out.push(FlatRow::Item { key: item.key.clone(), item: Rc::new(item.clone()), depth, label: name.to_string() }),
            Entry::Folder(folder, child) => {
                let path = if prefix.is_empty() { folder.to_string() } else { format!("{prefix}/{folder}") };
                let key = folder_key(kind, &path);
                let closed = !filtering && collapsed.contains(&key);
                let remote = (kind == SectionKind::Remote && depth == 1).then(|| folder.to_string());
                out.push(FlatRow::Folder { key, name: folder.to_string(), depth, collapsed: closed, remote });
                if !closed {
                    walk(child, depth + 1, &path, kind, filtering, collapsed, out);
                }
            }
        }
    }
}

/// `buildPanels`: a case-insensitive substring filter over every panel, forcing open the parents of a
/// match. `tree` sort nests by `/`; `recent` is flat, newest first (a remote's stays grouped).
pub fn build_panels(sections: &[Section], filter: &str, recent: &HashSet<SectionKind>, collapsed: &HashSet<String>) -> Vec<Panel> {
    let q = filter.trim().to_lowercase();
    sections
        .iter()
        .map(|s| {
            let matched: Vec<&SideItem> = if q.is_empty() { s.items.iter().collect() } else { s.items.iter().filter(|i| i.full_name().to_lowercase().contains(&q)).collect() };
            let is_recent = s.kind.nests() && recent.contains(&s.kind);
            let closed = collapsed.contains(&section_key(s.kind));
            let mut panel = Panel { kind: s.kind, collapsed: closed, matched: matched.len(), recent: is_recent, filtering: !q.is_empty(), rows: Vec::new() };
            if closed {
                return panel;
            }
            let item = |it: &SideItem, depth: u8, label: String| FlatRow::Item { key: it.key.clone(), item: Rc::new(it.clone()), depth, label };
            if !s.kind.nests() {
                panel.rows = matched.iter().map(|it| item(it, 1, it.name.clone())).collect();
                return panel;
            }
            let newest = |a: &&SideItem, b: &&SideItem| b.time.cmp(&a.time).then_with(|| by_name(&a.name, &b.name));
            if is_recent && s.kind == SectionKind::Remote {
                let mut groups: BTreeMap<String, Vec<&SideItem>> = BTreeMap::new();
                for it in &matched {
                    if let ItemKind::Remote { remote, .. } = &it.kind {
                        groups.entry(remote.clone()).or_default().push(it);
                    }
                }
                let mut ordered: Vec<(String, Vec<&SideItem>)> = groups.into_iter().collect();
                let top = |items: &[&SideItem]| items.iter().map(|i| i.time).max().unwrap_or(0);
                ordered.sort_by(|a, b| top(&b.1).cmp(&top(&a.1)).then_with(|| by_name(&a.0, &b.0)));
                for (remote, mut items) in ordered {
                    let key = folder_key(s.kind, &remote);
                    let fc = q.is_empty() && collapsed.contains(&key);
                    panel.rows.push(FlatRow::Folder { key, name: remote.clone(), depth: 1, collapsed: fc, remote: Some(remote) });
                    if fc {
                        continue;
                    }
                    items.sort_by(newest);
                    panel.rows.extend(items.iter().map(|it| item(it, 2, it.name.clone())));
                }
                return panel;
            }
            if is_recent {
                let mut items = matched.clone();
                items.sort_by(newest);
                panel.rows = items.iter().map(|it| item(it, 1, it.full_name())).collect();
                return panel;
            }
            let mut root = Node::default();
            for it in &matched {
                let mut parts: Vec<&str> = it.name.split('/').collect();
                parts.pop();
                let mut n = &mut root;
                if let ItemKind::Remote { remote, .. } = &it.kind {
                    n = n.folders.entry(remote.clone()).or_default();
                }
                for p in parts {
                    n = n.folders.entry(p.to_string()).or_default();
                }
                n.items.push(it);
            }
            walk(&root, 1, "", s.kind, !q.is_empty(), collapsed, &mut panel.rows);
            panel
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelSpec {
    pub collapsed: bool,
    pub weight: Option<f32>,
}

/// `layoutPanels`: heights for `avail` px. A collapsed panel is its header; the open ones share the
/// rest by weight, none under the minimum; too short for the minimums, they share it evenly.
pub fn layout_panels(avail: f32, panels: &[PanelSpec]) -> Vec<f32> {
    let open: Vec<usize> = (0..panels.len()).filter(|i| !panels[*i].collapsed).collect();
    let space = (avail - (panels.len() - open.len()) as f32 * HEADER_H).max(0.);
    let known: Vec<f32> = open.iter().filter_map(|i| panels[*i].weight).collect();
    let fallback = if known.is_empty() { DEFAULT_WEIGHT } else { known.iter().sum::<f32>() / known.len() as f32 };
    let w = |i: usize| panels[i].weight.unwrap_or(fallback);
    let mut heights = vec![HEADER_H; panels.len()];
    if open.is_empty() {
        return heights;
    }
    if space <= open.len() as f32 * MIN_PANEL_H {
        let mut left = space;
        for (k, i) in open.iter().enumerate() {
            let h = if k + 1 == open.len() { HEADER_H.max(left) } else { HEADER_H.max((left / (open.len() - k) as f32).floor()) };
            left -= h;
            heights[*i] = h;
        }
        return heights;
    }
    let mut free = open;
    let mut left = space;
    loop {
        let total: f32 = free.iter().map(|i| w(*i)).sum();
        let small: Vec<usize> = free.iter().copied().filter(|i| left * w(*i) / total < MIN_PANEL_H).collect();
        if small.is_empty() {
            let mut used = 0.;
            let count = free.len();
            for (k, i) in free.iter().enumerate() {
                let h = if k + 1 == count { left - used } else { (left * w(*i) / total).floor() };
                used += h;
                heights[*i] = h;
            }
            return heights;
        }
        for i in &small {
            heights[*i] = MIN_PANEL_H;
            left -= MIN_PANEL_H;
        }
        free.retain(|i| !small.contains(i));
        if free.is_empty() {
            return heights;
        }
    }
}

/// The panel a panel's bottom divider trades height with: the next open one, when it's open itself.
pub fn divider_targets(panels: &[PanelSpec]) -> Vec<Option<usize>> {
    panels.iter().enumerate().map(|(i, p)| if p.collapsed { None } else { panels.iter().enumerate().skip(i + 1).find(|(_, q)| !q.collapsed).map(|(j, _)| j) }).collect()
}

/// A divider moved by `delta` px: neither panel goes under the minimum.
pub fn resize_pair(upper: f32, lower: f32, delta: f32) -> (f32, f32) {
    let d = delta.round().min(lower - MIN_PANEL_H).max(MIN_PANEL_H - upper);
    (upper + d, lower - d)
}

/// `stashLabel`: git's "On <branch>: " or "WIP on <branch>: " split off, the branch dimmed after it.
pub fn stash_label(message: &str) -> (String, Option<String>) {
    let rest = message.strip_prefix("WIP on ").or_else(|| message.strip_prefix("On "));
    match rest.and_then(|r| r.split_once(": ")) {
        Some((branch, text)) if !branch.contains(':') && !branch.is_empty() => (if text.is_empty() { message.to_string() } else { text.to_string() }, Some(branch.to_string())),
        _ => (message.to_string(), None),
    }
}

/// A tab's sidebar: the filter, the sort and collapse states, the panels they make.
pub struct SideState {
    pub filter: Entity<TextInput>,
    pub sections: Vec<Section>,
    pub recent: HashSet<SectionKind>,
    pub collapsed: HashSet<String>,
    pub panels: Rc<Vec<Panel>>,
    pub scroll: HashMap<SectionKind, UniformListScrollHandle>,
    /// The keyboard's row, per panel.
    pub cursor: HashMap<SectionKind, usize>,
    /// The panel with the keyboard, after `Alt+1` or a click.
    pub keys: Option<SectionKind>,
}

impl SideState {
    pub fn new(repo: u32, cx: &mut Context<GitVolt>) -> Self {
        let filter = cx.new(|cx| TextInput::new(cx).placeholder(format!("Filter ({})", crate::commands::display_chord("Mod+Alt+F"))).claim_escape().claim_arrows());
        cx.subscribe(&filter, move |this, input, e: &InputEvent, cx| match e {
            InputEvent::Changed => this.relist_sidebar(repo, cx),
            InputEvent::Escape => {
                input.update(cx, |i, cx| i.set_text("", cx));
                this.relist_sidebar(repo, cx);
                this.focus_sidebar_body(repo, cx);
            }
            InputEvent::Arrow { down: true, .. } => this.focus_sidebar_body(repo, cx),
            _ => {}
        })
        .detach();
        Self { filter, sections: Vec::new(), recent: HashSet::new(), collapsed: HashSet::new(), panels: Rc::new(Vec::new()), scroll: HashMap::new(), cursor: HashMap::new(), keys: None }
    }

    pub fn relist(&mut self, cx: &App) {
        let filter = self.filter.read(cx).text().to_string();
        self.panels = Rc::new(build_panels(&self.sections, &filter, &self.recent, &self.collapsed));
        // A list keeps its scroll in its handle, so each panel's lasts as long as the tab.
        for s in &self.sections {
            self.scroll.entry(s.kind).or_default();
        }
    }

    fn toggle(&mut self, key: String, cx: &App) {
        if !self.collapsed.remove(&key) {
            self.collapsed.insert(key);
        }
        self.relist(cx);
    }

    fn scroll_of(&mut self, kind: SectionKind) -> UniformListScrollHandle {
        self.scroll.entry(kind).or_default().clone()
    }
}

/// What a drag on the sidebar is moving.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SideDrag {
    /// The WIP lists' divider.
    WipSplit { y: f32, ratio: f32 },
    None,
    /// The width: where it started, and the width then.
    Width { x: f32, w: f32 },
    /// A divider: where it started and the two panels' heights then.
    Divider { y: f32, upper: usize, lower: usize, up_h: f32, low_h: f32 },
    /// The details panel's width, which grows as the edge goes left.
    Details { x: f32, w: f32 },
}

/// Last pushes, by remote ref, for the hover card; `None`: loading.
pub type PushCache = Rc<RefCell<HashMap<String, Option<Value>>>>;

impl GitVolt {
    pub fn relist_sidebar(&mut self, repo: u32, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) {
            tab.side.relist(cx);
            cx.notify();
        }
    }

    /// Esc or ↓ in the filter: the keyboard goes to the first open panel.
    fn focus_sidebar_body(&mut self, repo: u32, cx: &mut Context<Self>) {
        let focus = self.focus.clone();
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) {
            tab.side.keys = tab.side.panels.iter().find(|p| !p.collapsed).map(|p| p.kind);
        }
        self.focus_next_frame = Some(focus);
        cx.notify();
    }

    pub fn focus_sidebar_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_narrow = false;
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        tab.sidebar_open = true;
        let input = tab.side.filter.clone();
        input.update(cx, |i, cx| i.select_all_text(cx));
        window.focus(&input.read(cx).focus.clone(), cx);
        cx.notify();
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        let open_file = self.tabs.get(self.active).is_some_and(|t| t.diff.is_some() || t.history.is_some());
        if open_file {
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.sidebar_open = !tab.sidebar_open;
            }
        } else {
            self.sidebar_narrow = !self.sidebar_narrow;
        }
        cx.notify();
    }

    pub fn focus_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_narrow = false;
        window.focus(&self.focus, cx);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.sidebar_open = true;
            tab.side.keys = tab.side.panels.iter().find(|p| !p.collapsed).map(|p| p.kind);
        }
        cx.notify();
    }

    /// A key while the sidebar has the keyboard. Whether it took it.
    pub fn sidebar_key(&mut self, k: &Keystroke, cx: &mut Context<Self>) -> bool {
        if k.modifiers.modified() {
            return false;
        }
        let Some(tab) = self.tabs.get_mut(self.active) else { return false };
        let Some(kind) = tab.side.keys else { return false };
        let Some(panel) = tab.side.panels.iter().find(|p| p.kind == kind).cloned() else { return false };
        let count = panel.rows.len();
        let cursor = tab.side.cursor.get(&kind).copied().unwrap_or(0).min(count.saturating_sub(1));
        let page = 8;
        let to = match k.key.as_str() {
            "down" => cursor + 1,
            "up" => cursor.saturating_sub(1),
            "home" => 0,
            "end" => count.saturating_sub(1),
            "pagedown" => cursor + page,
            "pageup" => cursor.saturating_sub(page),
            "enter" | "space" => {
                match panel.rows.get(cursor) {
                    Some(FlatRow::Item { item, .. }) => {
                        let target = item.target.clone();
                        self.jump_to(target, cx);
                    }
                    Some(FlatRow::Folder { key, .. }) => {
                        let key = key.clone();
                        tab.side.toggle(key, cx);
                        cx.notify();
                    }
                    None => {}
                }
                return true;
            }
            "left" | "right" => {
                if let Some(FlatRow::Folder { key, collapsed, .. }) = panel.rows.get(cursor)
                    && *collapsed == (k.key == "right")
                {
                    let key = key.clone();
                    tab.side.toggle(key, cx);
                    cx.notify();
                }
                return true;
            }
            "tab" | "escape" => {
                tab.side.keys = None;
                cx.notify();
                return k.key == "escape";
            }
            _ => return false,
        };
        let to = to.min(count.saturating_sub(1));
        tab.side.cursor.insert(kind, to);
        tab.side.scroll_of(kind).scroll_to_item(to, ScrollStrategy::Nearest);
        cx.notify();
        true
    }

    /// A row's commit, selected; or says it's not there.
    pub fn jump_to(&mut self, target: Option<String>, cx: &mut Context<Self>) {
        match target {
            Some(sha) => self.go_to_sha(&sha, cx),
            None => self.show_toast("Not in the loaded history", cx),
        }
    }

    pub fn go_to_sha(&mut self, sha: &str, cx: &mut Context<Self>) {
        let at = self.tabs.get(self.active).and_then(|t| t.graph.as_ref()?.by_sha.get(sha).copied());
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

    fn side_item_menu(&self, item: &SideItem, this: &WeakEntity<GitVolt>) -> Vec<menu::MenuRow> {
        match &item.kind {
            ItemKind::Local { .. } | ItemKind::Remote { .. } => {
                let row = item.target.as_ref().and_then(|t| self.tabs.get(self.active)?.graph.as_ref()?.by_sha.get(t).copied());
                let Some(row) = row else { return vec![menu::MenuRow::pending("sidebar.showInGraph", "Show in graph", "git-graph").disabled("Not in the loaded history")] };
                let chip = self.tabs.get(self.active).and_then(|t| t.graph.as_ref()).and_then(|g| g.rows[row].chips.iter().find(|c| c.name.as_ref() == item.name.as_str() || c.name.as_ref() == item.full_name()).cloned());
                let fallback = crate::model::Chip { name: item.full_name().into(), head: matches!(item.kind, ItemKind::Local { is_head: true, .. }), local: matches!(item.kind, ItemKind::Local { .. }), remote: matches!(item.kind, ItemKind::Remote { .. }), tag: false, remotes: Vec::new() };
                let mut rows = self.commit_rows(row, Some(chip.as_ref().unwrap_or(&fallback)), this);
                rows.push(menu::MenuRow::Separator);
                let (weak, sha) = (this.clone(), item.target.clone().unwrap_or_default());
                rows.push(menu::MenuRow::action("sidebar.showInGraph", "Show in graph", "git-graph", "Select this commit in the graph", move |_, cx| {
                    let sha = sha.clone();
                    weak.update(cx, |v, cx| v.go_to_sha(&sha, cx)).ok();
                }));
                rows
            }
            ItemKind::Worktree { path, branch, head, .. } => self.worktree_rows(path, branch.as_deref(), head.as_deref(), this),
            ItemKind::Stash { message, .. } => self.stash_rows(item.target.as_deref().unwrap_or_default(), message, this),
            ItemKind::Tag { .. } => self.tag_rows(&item.name, item.target.as_deref(), true, this),
            ItemKind::Submodule { .. } => vec![menu::MenuRow::pending("sidebar.submodule.open", "Open in a tab", "boxes"), menu::MenuRow::pending("sidebar.submodule.update", "Update this submodule", "refresh-cw")],
        }
    }

    pub fn on_side_drag(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        match self.side_drag {
            SideDrag::None => {}
            SideDrag::Details { x, w } => {
                if e.pressed_button != Some(MouseButton::Left) {
                    return self.end_side_drag(cx);
                }
                self.details_w = (w + x - e.position.x.as_f32()).round().clamp(crate::details::DETAILS_MIN, crate::details::DETAILS_MAX);
                cx.notify();
            }
            SideDrag::WipSplit { y, ratio } => {
                if e.pressed_button != Some(MouseButton::Left) {
                    return self.end_side_drag(cx);
                }
                let height = crate::ui::Ui::get(cx).anchor("wip-split").map_or(0., |b| b.size.height.as_f32());
                if height > 0. {
                    let (lo, hi) = crate::wip::split_bounds(height);
                    self.wip_ratio = (ratio + (e.position.y.as_f32() - y) / height).clamp(lo, hi);
                    cx.notify();
                }
            }
            SideDrag::Width { x, w } => {
                if e.pressed_button != Some(MouseButton::Left) {
                    return self.end_side_drag(cx);
                }
                self.sidebar_w = (w + e.position.x.as_f32() - x).round().clamp(MIN_W, MAX_W);
                cx.notify();
            }
            SideDrag::Divider { y, upper, lower, up_h, low_h } => {
                if e.pressed_button != Some(MouseButton::Left) {
                    return self.end_side_drag(cx);
                }
                let (u, l) = resize_pair(up_h, low_h, e.position.y.as_f32() - y);
                if let Some(tab) = self.tabs.get(self.active) {
                    let (a, b) = (tab.side.panels[upper].kind, tab.side.panels[lower].kind);
                    self.side_weights.insert(a, u);
                    self.side_weights.insert(b, l);
                    cx.notify();
                }
            }
        }
    }

    pub fn end_side_drag(&mut self, cx: &mut Context<Self>) {
        if self.side_drag != SideDrag::None {
            self.side_drag = SideDrag::None;
            self.save_layout();
            cx.notify();
        }
    }

    fn side_specs(&self, panels: &[Panel]) -> Vec<PanelSpec> {
        panels.iter().map(|p| PanelSpec { collapsed: p.collapsed, weight: self.side_weights.get(&p.kind).copied() }).collect()
    }
}

/// The folded strip: each panel's icon and count; a click opens the sidebar on that panel.
pub fn narrow_strip(tab: &Tab, cx: &mut Context<GitVolt>) -> AnyElement {
    let toggle = tip(
        format!("Expand sidebar ({})", crate::commands::display_chord("Mod+B")),
        div().id("sidebar-open").flex().flex_none().size(px(22.)).items_center().justify_center().rounded_full().bg(theme::panel_bg1()).hover(|d| d.bg(theme::hover_row())).cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.expand_sidebar(None, cx))).child(icon("chevron-right", 14., theme::text_secondary())),
    );
    let mut strip = div().flex().flex_col().flex_none().items_center().gap(px(4.)).w(px(NARROW_W)).h_full().pt(px(6.)).bg(theme::panel_bg()).border_r_1().border_color(theme::section_border()).child(toggle);
    for panel in tab.side.panels.iter() {
        let kind = panel.kind;
        strip = strip.child(tip(
            format!("{}: {}", kind.label(), panel.matched),
            div()
                .id(("side-strip", kind as usize))
                .flex()
                .flex_col()
                .flex_none()
                .items_center()
                .gap(px(1.))
                .w(px(36.))
                .py(px(4.))
                .rounded(px(4.))
                .cursor_pointer()
                .text_color(theme::text_secondary())
                .hover(|d| d.bg(theme::hover_row()))
                .on_click(cx.listener(move |this, _, _, cx| this.expand_sidebar(Some(kind), cx)))
                .child(icon(kind.icon(), 15., theme::sidebar_icon()))
                .child(div().text_size(px(10.)).font_weight(FontWeight::BOLD).text_color(theme::count_blue()).child(panel.matched.to_string())),
        ));
    }
    strip.into_any_element()
}

impl GitVolt {
    /// The strip's chevron or a panel's icon: leaves a file view for the graph, that panel open.
    fn expand_sidebar(&mut self, kind: Option<SectionKind>, cx: &mut Context<Self>) {
        self.sidebar_narrow = false;
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.sidebar_open = true;
            if let Some(kind) = kind {
                tab.side.collapsed.remove(&section_key(kind));
                tab.side.relist(cx);
                // On a file, the strip's click goes back to the graph, as the web UI does.
                tab.history = None;
                tab.diff = None;
            }
        }
        cx.notify();
    }
}

pub fn sidebar(this: &GitVolt, body_h: f32, window: &Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let width = this.sidebar_w;
    let weak = cx.weak_entity();
    let tab = this.tabs.get(this.active);
    let mut bar = div().relative().flex().flex_col().flex_none().w(px(width)).h_full().min_h_0().bg(theme::panel_bg()).border_r_1().border_color(theme::section_border());
    let collapse = tip(
        format!("Collapse sidebar ({})", crate::commands::display_chord("Mod+B")),
        div().id("sidebar-fold").flex().flex_none().size(px(22.)).items_center().justify_center().rounded_full().bg(theme::panel_bg1()).hover(|d| d.bg(theme::hover_row())).cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))).child(icon("chevron-left", 14., theme::text_secondary())),
    );
    let filter = tab.map(|t| {
        let input = t.side.filter.clone();
        let repo = t.id;
        let clear = cx.weak_entity();
        let clear_input = input.clone();
        clearable(
            "sidebar-filter",
            &input,
            move |_, cx| {
                clear_input.update(cx, |i, cx| i.set_text("", cx));
                clear.update(cx, |v, cx| v.relist_sidebar(repo, cx)).ok();
            },
            window,
            cx,
        )
    });
    bar = bar.child(
        div()
            .flex()
            .flex_none()
            .h(px(FILTER_H))
            .items_center()
            .gap(px(6.))
            .p(px(6.))
            .child(collapse)
            .children(filter)
            .child(tip(crate::ui::menu::NOT_YET, div().flex().flex_none().size(px(26.)).items_center().justify_center().rounded(px(4.)).bg(theme::control_bg()).child(icon("git-graph", 13., theme::text_disabled())))),
    );
    let Some(tab) = tab else { return bar.into_any_element() };
    let panels = tab.side.panels.clone();
    let specs = this.side_specs(&panels);
    let heights = layout_panels((body_h - FILTER_H).max(0.), &specs);
    let targets = divider_targets(&specs);
    let sidebar_right = width + 6.;
    let mut stack = div().flex().flex_col().flex_1().min_h_0().overflow_hidden();
    for (p, panel) in panels.iter().enumerate() {
        let kind = panel.kind;
        let height = heights[p];
        let header = div()
            .flex()
            .flex_none()
            .h(px(HEADER_H - 1.))
            .items_center()
            .pr(px(6.))
            .child(
                div()
                    .id(("side-head", p))
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::hover_row()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(tab) = this.tabs.get_mut(this.active) {
                            tab.side.toggle(section_key(kind), cx);
                            cx.notify();
                        }
                    }))
                    .child(icon(if panel.collapsed { "chevron-right" } else { "chevron-down" }, 12., theme::sidebar_icon()))
                    .child(icon(kind.icon(), 13., theme::text_secondary()))
                    .child(div().flex_1().truncate().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_secondary()).child(kind.label().to_uppercase()))
                    .child(div().text_size(px(11.)).font_weight(FontWeight::BOLD).text_color(theme::count_blue()).child(panel.matched.to_string())),
            )
            .when(matches!(kind, SectionKind::Remote | SectionKind::Worktrees), |d| {
                let (glyph, label) = if kind == SectionKind::Remote { ("plus", "Add remote…") } else { ("folder-plus", "Add worktree…") };
                d.child(tip(format!("{label} ({})", crate::ui::menu::NOT_YET), div().flex().flex_none().size(px(18.)).items_center().justify_center().child(icon(glyph, 13., theme::text_disabled()))))
            })
            .when(kind.nests() && !panel.collapsed, |d| {
                let recent = panel.recent;
                d.child(tip(
                    if recent { "Newest first. Click: directory tree" } else { "Sorted as a directory tree. Click: newest first" },
                    div()
                        .id(("side-sort", p))
                        .flex()
                        .flex_none()
                        .size(px(18.))
                        .items_center()
                        .justify_center()
                        .rounded(px(3.))
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::hover_row()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(tab) = this.tabs.get_mut(this.active) {
                                if !tab.side.recent.remove(&kind) {
                                    tab.side.recent.insert(kind);
                                }
                                tab.side.relist(cx);
                                cx.notify();
                            }
                        }))
                        .child(icon(if recent { "clock" } else { "list-tree" }, 12., theme::text_secondary())),
                ))
            });
        let mut section = div().relative().flex().flex_col().flex_none().h(px(height)).min_h_0().border_t_1().border_color(theme::section_border()).child(header);
        if !panel.collapsed {
            if panel.rows.is_empty() {
                section = section.child(div().px(px(12.)).py(px(6.)).italic().text_color(theme::text_disabled()).child(if panel.filtering { "No matches" } else { kind.empty() }));
            } else {
                let rows = panels.clone();
                let keys_here = tab.side.keys == Some(kind);
                let cursor = tab.side.cursor.get(&kind).copied().unwrap_or(0);
                let weak = weak.clone();
                let push = this.last_pushes.clone();
                let list = uniform_list(
                    ("side-list", p),
                    panel.rows.len(),
                    cx.processor(move |_, range: Range<usize>, _, cx| {
                        let list = &rows[p].rows;
                        range.map(|i| side_row(&list[i], i, kind, keys_here && cursor == i, sidebar_right, &weak, &push, cx)).collect()
                    }),
                )
                .with_sizing_behavior(ListSizingBehavior::Auto)
                .when_some(tab.side.scroll.get(&kind), |list, handle| list.track_scroll(handle))
                .flex_1()
                .min_h_0();
                section = section.child(list);
            }
        }
        if let Some(j) = targets[p] {
            let (up_h, low_h) = (heights[p], heights[j]);
            section = section.child(
                div()
                    .id(("side-divider", p))
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px(-3.))
                    .h(px(6.))
                    .cursor(CursorStyle::ResizeUpDown)
                    .hover(|d| d.bg(theme::a(0x4d88ff80)))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        if e.click_count == 2 {
                            // A double click evens these two out again.
                            let upper = ((up_h + low_h) / 2.).round();
                            if let Some(tab) = this.tabs.get(this.active) {
                                let (a, b) = (tab.side.panels[p].kind, tab.side.panels[j].kind);
                                this.side_weights.insert(a, upper);
                                this.side_weights.insert(b, up_h + low_h - upper);
                                this.save_layout();
                                cx.notify();
                            }
                            return;
                        }
                        // What's on screen becomes every open panel's weight, so the drag moves only these two.
                        this.side_drag = SideDrag::Divider { y: e.position.y.as_f32(), upper: p, lower: j, up_h, low_h };
                        cx.notify();
                    })),
            );
        }
        stack = stack.child(section);
    }
    bar = bar.child(stack);
    bar.child(
        div()
            .id("side-resize")
            .absolute()
            .top_0()
            .bottom_0()
            .right(px(-3.))
            .w(px(6.))
            .cursor(CursorStyle::ResizeLeftRight)
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                if e.click_count == 2 {
                    this.sidebar_w = DEFAULT_SIDEBAR_W;
                    this.save_layout();
                    cx.notify();
                    return;
                }
                this.side_drag = SideDrag::Width { x: e.position.x.as_f32(), w: width };
            })),
    )
    .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn side_row(row: &FlatRow, i: usize, kind: SectionKind, on_key: bool, sidebar_right: f32, this: &WeakEntity<GitVolt>, push: &PushCache, cx: &mut Context<GitVolt>) -> AnyElement {
    let base = div().id(i).relative().flex().w_full().h(px(ROW_H)).items_center().gap(px(ROW_GAP)).pr(px(8.)).whitespace_nowrap();
    let outlined = on_key || menu::is_target(row.key(), cx);
    let outline = |d: Stateful<Div>| d.when(outlined, |d| d.child(crate::ui::outline()));
    match row {
        FlatRow::Folder { key, name, depth, collapsed, remote } => {
            let (toggle_key, menu_key, remote) = (key.clone(), key.clone(), remote.clone());
            let weak = this.clone();
            outline(base)
                .pl(px(row_indent(*depth)))
                .hover(|d| d.bg(theme::hover_row()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(tab) = this.tabs.get_mut(this.active) {
                        tab.side.cursor.insert(kind, i);
                        tab.side.keys = Some(kind);
                        tab.side.toggle(toggle_key.clone(), cx);
                        cx.notify();
                    }
                }))
                .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    let rows = match &remote {
                        Some(r) => this.remote_rows(r, &weak),
                        None => this.folder_rows(),
                    };
                    menu::open_context_for(menu_key.clone(), rows, e.position, window, cx);
                }))
                .child(icon(if *collapsed { "folder" } else { "folder-open" }, 13., theme::sidebar_icon()))
                .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                .into_any_element()
        }
        FlatRow::Item { key, item, label, depth } => {
            let head = matches!(item.kind, ItemKind::Local { is_head: true, .. } | ItemKind::Worktree { is_current: true, .. });
            let target = item.target.clone();
            let (weak, menu_item, menu_key) = (this.clone(), item.clone(), key.clone());
            let hover_item = item.clone();
            let glyph: AnyElement = match &item.kind {
                ItemKind::Local { is_head: true, .. } => div().flex().flex_none().items_center().justify_center().size(px(15.)).mx(px(-1.)).rounded(px(3.)).bg(theme::checked_out()).child(icon("check", 11., theme::text_on_accent())).into_any_element(),
                ItemKind::Local { .. } | ItemKind::Remote { .. } => icon("git-branch", 13., theme::sidebar_icon()).into_any_element(),
                ItemKind::Worktree { is_main: true, .. } => icon("house", 13., theme::sidebar_icon()).into_any_element(),
                ItemKind::Worktree { .. } => icon("tree-pine", 13., theme::sidebar_icon()).into_any_element(),
                ItemKind::Stash { .. } => icon("archive", 13., theme::sidebar_icon()).into_any_element(),
                ItemKind::Tag { .. } => icon("tag", 13., theme::sidebar_icon()).into_any_element(),
                ItemKind::Submodule { .. } => icon("boxes", 13., theme::sidebar_icon()).into_any_element(),
            };
            let label_el = match &item.kind {
                ItemKind::Stash { .. } => {
                    let (text, branch) = stash_label(label);
                    div().flex().flex_1().min_w_0().overflow_hidden().child(div().min_w_0().truncate().child(text)).when_some(branch, |d, b| d.child(div().flex_none().ml(px(4.)).text_size(px(11.)).text_color(theme::text_secondary()).child(b)))
                }
                _ => div().flex_1().min_w_0().truncate().child(label.clone()),
            };
            let ab = match &item.kind {
                ItemKind::Local { ahead, behind, .. } if *ahead > 0 || *behind > 0 => Some(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(11.))
                        .text_color(theme::text_secondary())
                        .child(div().flex().items_center().gap(px(1.)).child(ahead.to_string()).child(icon("arrow-up", 11., theme::text_secondary())))
                        .child(div().flex().items_center().gap(px(1.)).child(behind.to_string()).child(icon("arrow-down", 11., theme::text_secondary()))),
                ),
                _ => None,
            };
            let state = match &item.kind {
                ItemKind::Submodule { status, .. } if status != "current" => Some(status.clone()),
                _ => None,
            };
            let body = outline(base)
                .pl(px(row_indent(*depth)))
                .when(head, |d| d.bg(theme::checked_out_row()).text_color(theme::text_selected()).font_weight(FontWeight::SEMIBOLD))
                .when(!head, |d| d.hover(|d| d.bg(theme::hover_row())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(tab) = this.tabs.get_mut(this.active) {
                        tab.side.cursor.insert(kind, i);
                        tab.side.keys = Some(kind);
                    }
                    this.jump_to(target.clone(), cx);
                }))
                .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    crate::ui::tooltip::hide(cx);
                    let rows = this.side_item_menu(&menu_item, &weak);
                    menu::open_context_for(menu_key.clone(), rows, e.position, window, cx);
                }))
                .child(glyph)
                .child(label_el)
                .children(ab)
                .when_some(state, |d, s| d.child(div().flex_none().ml_auto().pl(px(6.)).text_size(px(11.)).text_color(theme::text_secondary()).child(s)));
            let (card_this, card_push) = (this.clone(), push.clone());
            Anchor::new(body).card(format!("side:{key}"), move |_, cx| hover_card(&hover_item, &card_push, &card_this, cx)).placement(Placement::BesideAt(sidebar_right)).into_any_element()
        }
    }
}

/// `sidebar/HoverCard.tsx`: shows at once; only "Last push" waits on the core.
fn hover_card(item: &SideItem, push: &PushCache, this: &WeakEntity<GitVolt>, cx: &mut App) -> AnyElement {
    let shell = div().flex().flex_col().flex_none().w(px(300.)).px(px(12.)).py(px(10.)).bg(theme::panel_bg1()).border_1().border_color(theme::section_border()).rounded(px(6.)).shadow(crate::ui::shadow(theme::shadow_popup(), 6., 20.)).text_color(theme::text_normal());
    let name = |t: &str| div().mb(px(4.)).truncate().font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).child(t.to_string());
    let summary = |t: String| div().mb(px(4.)).child(t);
    let meta = |t: String| div().text_size(px(11.)).text_color(theme::text_secondary()).child(t);
    let short = |s: &str| s.chars().take(SHORT_SHA).collect::<String>();
    let card = match &item.kind {
        ItemKind::Worktree { path, branch, head, is_main, is_current } => {
            let state = [if *is_main { "Main checkout" } else { "Linked worktree" }, if *is_current { "current" } else { "" }].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · ");
            shell
                .child(name(&item.name))
                .child(summary(tilde(path)))
                .child(meta(match (branch, head) {
                    (Some(b), _) => format!("Branch: {b}"),
                    (None, Some(h)) => format!("Detached at {}", short(h)),
                    (None, None) => "Detached".to_string(),
                }))
                .when_some(head.as_ref().filter(|_| branch.is_some()), |d, h| d.child(meta(format!("HEAD {}", short(h)))))
                .child(meta(state))
        }
        ItemKind::Submodule { url, recorded, head, branch, status, real_name } => shell
            .child(name(&item.name))
            .when_some(url.clone(), |d, u| d.child(summary(u)))
            .when(real_name != &item.name, |d| d.child(meta(format!("Name: {real_name}"))))
            .child(meta(format!("Recorded: {}", short(recorded))))
            .when_some(head.clone(), |d, h| d.child(meta(format!("Checked out: {}{}", branch.as_ref().map_or(String::new(), |b| format!("{b} at ")), short(&h)))))
            .child(meta(status.clone())),
        ItemKind::Tag { message, tagger, .. } => {
            let sha = item.target.as_deref().unwrap_or_default();
            let line = match (message, tagger) {
                (Some(m), Some(t)) => format!("{m}\n{t} · {}", crate::format::relative_time(item.time, crate::format::now())),
                (Some(m), None) => m.clone(),
                _ => format!("Lightweight tag at {}", short(sha)),
            };
            shell.child(name(&item.name)).child(summary(line))
        }
        ItemKind::Local { .. } | ItemKind::Remote { .. } | ItemKind::Stash { .. } => {
            let (text, author) = match &item.kind {
                ItemKind::Local { summary, author, .. } | ItemKind::Remote { summary, author, .. } => (summary.clone(), author.clone()),
                ItemKind::Stash { message, .. } => (message.clone(), String::new()),
                _ => (String::new(), String::new()),
            };
            let when = if item.time != 0 { crate::format::relative_time(item.time, crate::format::now()) } else { String::new() };
            let who = if author.is_empty() { when } else { format!("{author} · {when}") };
            let remote_ref = match &item.kind {
                ItemKind::Local { upstream, .. } => upstream.clone(),
                ItemKind::Remote { full, .. } => Some(full.clone()),
                _ => None,
            };
            let vs = match &item.kind {
                ItemKind::Local { upstream: Some(u), gone: true, .. } => Some({
                    let _ = u;
                    "Upstream is gone".to_string()
                }),
                ItemKind::Local { upstream: Some(u), ahead, behind, .. } => Some(format!("{ahead}↑ {behind}↓ vs {}", u.trim_start_matches("refs/remotes/"))),
                _ => None,
            };
            let last = remote_ref.map(|r| last_push_line(&r, push, this, cx));
            shell
                .child(name(&item.name))
                .when(!text.is_empty(), |d| d.child(summary(text)))
                .child(meta(who))
                .when_some(vs, |d, v| d.child(meta(v)))
                .when_some(last, |d, l| d.child(meta(l)))
        }
    };
    card.into_any_element()
}

fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => format!("~{}", &path[home.len()..]),
        _ => path.to_string(),
    }
}

/// "Last push: 3 days ago", asked for once per ref; "Loading…" meanwhile.
fn last_push_line(remote_ref: &str, push: &PushCache, this: &WeakEntity<GitVolt>, cx: &mut App) -> String {
    let known = push.borrow().get(remote_ref).cloned();
    match known {
        None => {
            push.borrow_mut().insert(remote_ref.to_string(), None);
            let (this, r) = (this.clone(), remote_ref.to_string());
            // Not during this frame: the view is being drawn.
            cx.defer(move |cx| {
                this.update(cx, |v, cx| v.read_last_push(r, cx)).ok();
            });
            "Loading…".to_string()
        }
        Some(None) => "Loading…".to_string(),
        Some(Some(v)) if v.is_null() => "Never seen on the remote".to_string(),
        Some(Some(v)) => {
            let time = crate::format::relative_time(v["time"].as_i64().unwrap_or(0), crate::format::now());
            if v["kind"].as_str() == Some("push") { format!("Last push: {time}") } else { format!("Last seen on remote: {time}") }
        }
    }
}

impl GitVolt {
    fn read_last_push(&mut self, remote_ref: String, cx: &mut Context<Self>) {
        let Some(repo) = self.tabs.get(self.active).map(|t| t.id) else { return };
        if cfg!(test) {
            return;
        }
        let (api, r) = (self.api.clone(), remote_ref.clone());
        let done = self.rt.spawn(async move { api.dispatch(Request::LastPush { repo, remote_ref: r }).await });
        cx.spawn(async move |this, cx| {
            let v = match done.await {
                Ok(Ok(v)) => v,
                _ => Value::Null,
            };
            this.update(cx, |this, cx| {
                this.last_pushes.borrow_mut().insert(remote_ref, Some(v));
                cx.refresh_windows();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::prelude::v1::test;

    fn sample() -> Vec<Section> {
        sections_of(
            &json!({
                "locals": [
                    {"name": "main", "fullName": "refs/heads/main", "target": "a", "isHead": true, "tipTime": 5},
                    {"name": "feature/b10", "fullName": "refs/heads/feature/b10", "target": "c", "tipTime": 9},
                    {"name": "feature/b1", "fullName": "refs/heads/feature/b1", "target": "b", "tipTime": 1},
                    {"name": "dev", "fullName": "refs/heads/dev", "target": "d", "tipTime": 3},
                ],
                "remotes": [{"name": "origin", "branches": [{"name": "origin/main", "fullName": "refs/remotes/origin/main", "target": "a", "tipTime": 2}]}],
                "worktrees": [{"name": "w", "path": "/w", "isCurrent": true, "head": "a"}],
                "stashes": [],
                "tags": [{"name": "v1", "fullName": "refs/tags/v1", "target": "t", "time": 1}],
                "submodules": {},
            }),
            "/w",
        )
    }

    fn labels(p: &Panel) -> Vec<String> {
        p.rows
            .iter()
            .map(|r| match r {
                FlatRow::Folder { name, .. } => format!("{name}/"),
                FlatRow::Item { label, .. } => label.clone(),
            })
            .collect()
    }

    #[test]
    fn one_list_per_level_folders_and_branches_mixed() {
        let p = build_panels(&sample(), "", &HashSet::new(), &HashSet::new());
        assert_eq!(labels(&p[0]), ["dev", "feature/", "b1", "b10", "main"]);
        assert_eq!(p[0].matched, 4);
        assert_eq!(labels(&p[1]), ["origin/", "main"]);
    }

    #[test]
    fn the_filter_keeps_and_opens_the_parents_of_a_match_and_counts_it() {
        let mut collapsed = HashSet::new();
        collapsed.insert("local:feature".to_string());
        let closed = build_panels(&sample(), "", &HashSet::new(), &collapsed);
        assert_eq!(labels(&closed[0]), ["dev", "feature/", "main"]);
        let p = build_panels(&sample(), "B1", &HashSet::new(), &collapsed);
        assert_eq!(labels(&p[0]), ["feature/", "b1", "b10"], "open while filtering");
        assert_eq!(p[0].matched, 2);
        assert_eq!(p[1].matched, 0);
        assert_eq!(build_panels(&sample(), "origin/m", &HashSet::new(), &collapsed)[1].matched, 1, "a remote's name counts");
    }

    #[test]
    fn newest_first_is_flat_and_a_collapsed_panel_has_no_rows() {
        let mut recent = HashSet::new();
        recent.insert(SectionKind::Local);
        recent.insert(SectionKind::Remote);
        let p = build_panels(&sample(), "", &recent, &HashSet::new());
        assert_eq!(labels(&p[0]), ["feature/b10", "main", "dev", "feature/b1"]);
        assert_eq!(labels(&p[1]), ["origin/", "main"]);
        let mut collapsed = HashSet::new();
        collapsed.insert(section_key(SectionKind::Local));
        let c = build_panels(&sample(), "", &HashSet::new(), &collapsed);
        assert!(c[0].rows.is_empty());
        assert_eq!(c[0].matched, 4);
    }

    fn spec(collapsed: bool, weight: Option<f32>) -> PanelSpec {
        PanelSpec { collapsed, weight }
    }

    // Web: sidebar/layout.test.ts.
    #[test]
    fn the_stack_fills_its_height_exactly() {
        let h = layout_panels(600., &[spec(false, None), spec(true, None), spec(false, None)]);
        assert_eq!(h, [287., 26., 287.]);
        let weighted = layout_panels(600., &[spec(false, Some(400.)), spec(false, Some(200.))]);
        assert_eq!(weighted, [400., 200.]);
        let pinned = layout_panels(400., &[spec(false, Some(1000.)), spec(false, Some(10.))]);
        assert_eq!(pinned[1], MIN_PANEL_H);
        assert_eq!(pinned.iter().sum::<f32>(), 400.);
        let tight = layout_panels(100., &[spec(false, None), spec(false, None), spec(false, None)]);
        assert_eq!(tight.iter().sum::<f32>(), 100.);
    }

    #[test]
    fn a_divider_trades_with_the_next_open_panel_within_the_minimums() {
        assert_eq!(divider_targets(&[spec(false, None), spec(true, None), spec(false, None), spec(true, None)]), [Some(2), None, None, None]);
        assert_eq!(resize_pair(200., 200., 30.4), (230., 170.));
        assert_eq!(resize_pair(200., 200., 500.), (400. - MIN_PANEL_H, MIN_PANEL_H));
        assert_eq!(resize_pair(200., 200., -500.), (MIN_PANEL_H, 400. - MIN_PANEL_H));
    }

    #[test]
    fn a_stash_says_its_message_and_dims_its_branch() {
        assert_eq!(stash_label("WIP on main: 5df155c Merge"), ("5df155c Merge".to_string(), Some("main".to_string())));
        assert_eq!(stash_label("On dev: my work"), ("my work".to_string(), Some("dev".to_string())));
        assert_eq!(stash_label("plain"), ("plain".to_string(), None));
    }
}
