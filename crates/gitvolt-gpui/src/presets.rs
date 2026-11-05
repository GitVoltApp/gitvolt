//! Branch presets and Hide (`ui/src/presets/`, `ui/src/graph/reveal/`): the button beside the sidebar's filter, what a graph
//! request carries for the current preset, hiding by hand (a branch, a tag, a folder; on a built-in, in the repository's
//! Custom), the ⋮ rows' reveal panel and the hidden list. The presets live in the settings and the profile; "Edit presets…"
//! is Settings', which isn't here yet.

pub mod settings_page;

use crate::app::{GitVolt, Tab, icon};
use crate::model::{Chip, Ghost, RowView, graph_rows};
use crate::theme;
use crate::ui::menu::{self, MenuRow, OpenOptions};
use crate::ui::modal;
use crate::ui::tooltip::tip;
use gitvolt_core::api::Request;
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::rc::Rc;
use std::time::Duration;

pub const ALL_ID: &str = "all";
pub const CUSTOM_ID: &str = "custom";
/// The rest of a ⋮ row opens its panel after this long.
pub const REVEAL_HOVER_MS: u64 = 400;
/// Above the row only when the title and this many rows (fewer for a shorter run) fit there.
pub const REVEAL_MIN_ROWS: f32 = 8.;

fn no_repo() -> Value {
    json!({ "pin": null, "columns": null, "hiddenColumns": [], "sidebarSort": {}, "collapsed": [], "editor": null, "mrFilter": null, "forgeTargetRemote": null, "forkTrunkNoticeDismissed": false })
}

fn builtin_presets() -> Vec<Value> {
    let builtin = |id: &str, name: &str, color: &str, rules: Value, otherwise: &str, ghost: bool| json!({ "id": id, "name": name, "color": color, "scope": "all", "rules": rules, "otherwise": otherwise, "foldMerged": false, "ghostLines": ghost, "ghostPicks": false, "inCycle": true });
    vec![
        builtin("all", "All", "gray", json!([]), "show", false),
        builtin("all-collapsed", "All, collapsed", "cyan", json!([{ "action": "show", "matcher": { "kind": "trunk" } }, { "action": "show", "matcher": { "kind": "checkedOut" } }]), "collapse", true),
        builtin("mine", "Mine", "green", json!([{ "action": "show", "matcher": { "kind": "trunk" } }, { "action": "show", "matcher": { "kind": "checkedOut" } }, { "action": "show", "matcher": { "kind": "commitsBy", "author": { "kind": "me" } } }]), "hide", true),
    ]
}

#[derive(Debug, Clone, PartialEq)]
pub struct PresetEntry {
    pub preset: Value,
    /// `builtin`, `all` or `repo`.
    pub group: &'static str,
    pub in_cycle: bool,
    pub read_only: bool,
}

impl PresetEntry {
    pub fn id(&self) -> &str {
        self.preset["id"].as_str().unwrap_or_default()
    }

    pub fn name(&self) -> &str {
        self.preset["name"].as_str().unwrap_or_default()
    }
}

/// `presetList`: the built-ins in the settings' order, the presets for all repositories, this repository's, then its Custom.
pub fn preset_list(settings: &Value, repo: &Value) -> Vec<PresetEntry> {
    let all = builtin_presets();
    let mut out = Vec::new();
    let slots = settings["builtinPresets"].as_array().cloned().unwrap_or_default();
    for s in &slots {
        if let Some(b) = all.iter().find(|b| b["id"] == s["id"]) {
            out.push(PresetEntry { preset: b.clone(), group: "builtin", in_cycle: s["inCycle"] != false, read_only: true });
        }
    }
    for b in &all {
        if !slots.iter().any(|s| s["id"] == b["id"]) {
            out.push(PresetEntry { preset: b.clone(), group: "builtin", in_cycle: true, read_only: true });
        }
    }
    let own = |p: &Value, group: &'static str| PresetEntry { preset: p.clone(), group, in_cycle: p["inCycle"] != false, read_only: false };
    out.extend(settings["branchPresets"].as_array().map_or(&[][..], Vec::as_slice).iter().map(|p| own(p, "all")));
    out.extend(repo["branchPresets"].as_array().map_or(&[][..], Vec::as_slice).iter().map(|p| own(p, "repo")));
    if repo["customPreset"].is_object() {
        out.push(own(&repo["customPreset"], "repo"));
    }
    out
}

/// `currentPreset`: All when it names none, or one that's gone.
pub fn current_preset<'a>(list: &'a [PresetEntry], repo: &Value) -> &'a PresetEntry {
    let id = repo["branchPreset"].as_str().unwrap_or(ALL_ID);
    list.iter().find(|e| e.id() == id).or_else(|| list.iter().find(|e| e.id() == ALL_ID)).expect("All is always listed")
}

/// `nextInCycle`: the ticked preset after `current` in list order, wrapping.
pub fn next_in_cycle<'a>(list: &'a [PresetEntry], current: &str) -> Option<&'a PresetEntry> {
    let at = list.iter().position(|e| e.id() == current).unwrap_or(0);
    (1..=list.len()).map(|i| &list[(at + i) % list.len()]).find(|e| e.in_cycle && e.id() != current)
}

/// `viewSpecOf`: what a graph request carries: nothing for All, so its graph is the plain one.
pub fn view_spec_of(entry: &PresetEntry, repo: &Value, expanded: &[String], placement: &Value) -> Option<Value> {
    let p = &entry.preset;
    if entry.id() == ALL_ID {
        return None;
    }
    let picks = &repo["branchPicks"][entry.id()];
    Some(json!({
        "rules": p["rules"], "otherwise": p["otherwise"],
        "picks": if picks.is_object() { picks.clone() } else { json!({ "shown": [], "hidden": [] }) },
        "foldMerged": p["foldMerged"], "ghostLines": p["ghostLines"], "ghostPicks": p["ghostPicks"],
        "trunkPatterns": repo.get("trunkPatterns").filter(|t| t.is_array()).cloned().unwrap_or_else(|| json!([])),
        "expanded": expanded, "placement": if placement.is_null() { json!("hide") } else { placement.clone() },
    }))
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().map_or(Vec::new(), |a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
}

/// Shown and hidden, as lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Picks {
    pub shown: Vec<String>,
    pub hidden: Vec<String>,
}

/// `folderPickHides`: the nearest folder with a pick, from `folder` itself up, decides.
pub fn folder_pick_hides(p: &Picks, folder: &str) -> bool {
    let mut end = folder.len();
    while end > 0 {
        let f = &folder[..end];
        if p.shown.iter().any(|x| x == f) {
            return false;
        }
        if p.hidden.iter().any(|x| x == f) {
            return true;
        }
        end = folder[..end - 1].rfind('/').map_or(0, |i| i + 1);
    }
    false
}

fn pick_one(old: &Picks, name: &str, show: bool) -> Picks {
    let without = |l: &[String]| l.iter().filter(|n| *n != name).cloned().collect::<Vec<_>>();
    if show {
        let mut shown = without(&old.shown);
        shown.push(name.to_string());
        return Picks { shown, hidden: without(&old.hidden) };
    }
    let mut rest = Picks { shown: without(&old.shown), hidden: without(&old.hidden) };
    let folder = &name[..name.rfind('/').map_or(0, |i| i + 1)];
    if !folder_pick_hides(&rest, folder) {
        rest.hidden.push(name.to_string());
    }
    rest
}

/// `folderEdit`: hiding replaces every pick under the folder with the folder itself; showing drops the hidden picks under
/// it, and adds it to `shown` only when that can't bring it back another way.
fn folder_edit(old: &Picks, folder: &str, show: bool) -> Picks {
    let outside = |l: &[String]| l.iter().filter(|n| !n.starts_with(folder)).cloned().collect::<Vec<_>>();
    if !show {
        let mut rest = Picks { shown: outside(&old.shown), hidden: outside(&old.hidden) };
        if !folder_pick_hides(&rest, folder) {
            rest.hidden.push(folder.to_string());
        }
        return rest;
    }
    let hidden = outside(&old.hidden);
    let pin = hidden.len() == old.hidden.len() || folder_pick_hides(&Picks { shown: old.shown.clone(), hidden: hidden.clone() }, folder);
    if pin {
        let mut shown = outside(&old.shown);
        shown.push(folder.to_string());
        Picks { shown, hidden }
    } else {
        Picks { shown: old.shown.clone(), hidden }
    }
}

/// `editPicks`: changes the current preset's picks; on a read-only preset, switches to Custom (made from it if none).
fn edit_picks(settings: &Value, repo_in: &Value, tags: bool, edit: impl FnOnce(&Picks) -> Picks) -> Value {
    let mut repo = if repo_in.is_object() { repo_in.clone() } else { no_repo() };
    let list = preset_list(settings, &repo);
    let cur = current_preset(&list, &repo).clone();
    let mut id = cur.id().to_string();
    if cur.read_only {
        id = CUSTOM_ID.to_string();
        if !repo["customPreset"].is_object() {
            let mut custom = cur.preset.clone();
            custom["id"] = json!(CUSTOM_ID);
            custom["name"] = json!("Custom");
            custom["scope"] = json!("repo");
            custom["inCycle"] = json!(true);
            repo["customPreset"] = custom;
            if let Some(m) = repo["branchPicks"].as_object_mut() {
                m.remove(CUSTOM_ID);
            }
        }
        repo["branchPreset"] = json!(CUSTOM_ID);
    }
    if !repo["branchPicks"].is_object() {
        repo["branchPicks"] = json!({});
    }
    let old = repo["branchPicks"][&id].clone();
    let (s, h) = if tags { ("shownTags", "hiddenTags") } else { ("shown", "hidden") };
    let before = Picks { shown: strs(&old[s]), hidden: strs(&old[h]) };
    let after = edit(&before);
    let mut next = if old.is_object() { old } else { json!({ "shown": [], "hidden": [] }) };
    if tags {
        for (key, list) in [(s, &after.shown), (h, &after.hidden)] {
            if list.is_empty() {
                next.as_object_mut().map(|m| m.remove(key));
            } else {
                next[key] = json!(list);
            }
        }
    } else {
        next["shown"] = json!(after.shown);
        next["hidden"] = json!(after.hidden);
    }
    repo["branchPicks"][&id] = next;
    repo
}

/// `unitNameOf`: the short branch name, remote prefix stripped. A tag or a detached HEAD has none.
pub fn unit_name(c: &Chip) -> Option<String> {
    if c.tag {
        return None;
    }
    if c.local {
        return Some(c.name.to_string());
    }
    c.remotes.first().map(|(remote, full)| full.trim_start_matches(&format!("refs/remotes/{remote}/")).to_string())
}

/// `cannotHide`: the trunk's the view's; under All, the graph's pinned refs.
pub fn cannot_hide(view: Option<&Value>, name: &str, head: Option<&str>, pinned: &[String]) -> Option<&'static str> {
    let trunk = match view {
        Some(v) => strs(&v["trunk"]).iter().any(|t| t == name),
        None => pinned.iter().any(|r| r == &format!("refs/heads/{name}") || r.strip_prefix("refs/remotes/").and_then(|x| x.split_once('/')).is_some_and(|x| x.1 == name)),
    };
    if trunk {
        return Some("The trunk is always shown");
    }
    (head == Some(name)).then_some("Checked out: always shown")
}

/// `branchViewOf`'s state: `shown`, `hidden`, `collapsed`, `folded`.
pub fn branch_state(view: Option<&Value>, name: &str) -> Option<String> {
    view?["branches"].as_array()?.iter().find(|b| b["name"] == name)?["state"].as_str().map(str::to_string)
}

/// `expandKeyOf`: a collapsed run's own key unfolds its whole branch.
pub fn expand_key(key: &str) -> String {
    if key.starts_with("branch:")
        && let Some(i) = key.rfind('@')
        && key.len() - i - 1 == 40
        && key[i + 1..].chars().all(|c| c.is_ascii_hexdigit())
    {
        return key[..i].to_string();
    }
    key.to_string()
}

/// `revealPlacement`: above when the space above the row fits the title and min(count, 8) rows; a fold's always below.
pub fn reveal_below(row_top: f32, view_top: f32, row_h: f32, count: f32, fold: bool) -> bool {
    fold || row_top - view_top < (1. + count.min(REVEAL_MIN_ROWS)) * row_h
}

/// `panelBox`: the panel on one side of the row: rows and the title row, as many as fit, at least one.
pub fn panel_box(below: bool, row_top: f32, view_top: f32, view_bottom: f32, row_h: f32, rows: f32) -> (f32, f32) {
    let room = if below { view_bottom - (row_top + row_h) } else { row_top - view_top };
    let fit = (room / row_h).floor() - 1.;
    let n = rows.min(fit).max(rows.min(1.));
    let height = (n + 1.) * row_h;
    (if below { row_top + row_h } else { row_top - height }, height)
}

/// An open reveal panel.
pub struct Reveal {
    pub key: String,
    pub row: usize,
    pub below: bool,
    pub pinned: bool,
    pub branch: String,
    pub count: u32,
    pub rows: Rc<Vec<RowView>>,
    pub tags: Vec<(u32, String)>,
    pub list: UniformListScrollHandle,
}

impl Tab {
    pub fn repo_settings<'a>(&self, profile: &'a Value) -> &'a Value {
        &profile["repos"][self.repo_path.as_str()]
    }
}

impl GitVolt {
    fn preset_state(&self) -> Option<(Vec<PresetEntry>, PresetEntry)> {
        let tab = self.tabs.get(self.active).filter(|t| !t.is_open_tab())?;
        let repo = tab.repo_settings(&self.profile);
        let list = preset_list(&self.settings, repo);
        let cur = current_preset(&list, repo).clone();
        Some((list, cur))
    }

    /// What the tab's next graph read carries, and its pin.
    pub fn graph_options(&self, repo: u32) -> (Option<gitvolt_graph::ViewSpec>, Option<gitvolt_core::settings::PinSetting>) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == repo) else { return (None, None) };
        if self.profile.is_null() {
            return (None, None);
        }
        let rs = tab.repo_settings(&self.profile);
        let list = preset_list(&self.settings, rs);
        let spec = view_spec_of(current_preset(&list, rs), rs, &tab.expanded, &self.settings["collapsePlacement"]);
        let view = spec.and_then(|s| serde_json::from_value(s).map_err(|e| eprintln!("A preset that can't be read: {e}")).ok());
        let pin = rs.get("pin").filter(|p| !p.is_null()).and_then(|p| serde_json::from_value(p.clone()).ok());
        (view, pin)
    }

    /// Changes this tab's repository's settings in the profile, and saves.
    pub fn update_repo(&mut self, path: &str, change: impl FnOnce(&mut Value), cx: &mut Context<Self>) {
        let path = path.to_string();
        self.update_profile(
            move |p| {
                if !p["repos"].is_object() {
                    p["repos"] = json!({});
                }
                if !p["repos"][&path].is_object() {
                    p["repos"][&path] = no_repo();
                }
                change(&mut p["repos"][&path]);
            },
            cx,
        );
    }

    /// Makes `id` the repository's current preset and reloads; the tab's expansions go.
    pub fn set_preset(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let (repo, path) = (tab.id, tab.repo_path.clone());
        tab.expanded.clear();
        tab.reveal = None;
        let id = id.to_string();
        self.update_repo(
            &path,
            move |rs| {
                if id == ALL_ID {
                    rs.as_object_mut().map(|m| m.remove("branchPreset"));
                } else {
                    rs["branchPreset"] = json!(id);
                }
            },
            cx,
        );
        self.reload(repo, cx);
    }

    /// The button's click: the next ticked preset.
    pub fn cycle_preset(&mut self, cx: &mut Context<Self>) {
        let Some((list, cur)) = self.preset_state() else { return };
        if let Some(next) = next_in_cycle(&list, cur.id()) {
            let id = next.id().to_string();
            self.set_preset(&id, cx);
        }
    }

    fn edit_current_picks(&mut self, tags: bool, drop: Option<String>, edit: impl FnOnce(&Picks) -> Picks, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let (repo, path) = (tab.id, tab.repo_path.clone());
        if let Some(d) = &drop {
            tab.expanded.retain(|k| k != d && !(d.ends_with('/') && k.starts_with(d.as_str())));
        }
        let settings = self.settings.clone();
        self.update_repo(
            &path,
            move |rs| {
                let next = edit_picks(&settings, rs, tags, edit);
                *rs = next;
            },
            cx,
        );
        self.reload(repo, cx);
    }

    pub fn pick_branch(&mut self, name: String, show: bool, cx: &mut Context<Self>) {
        let drop = (!show).then(|| format!("branch:{name}"));
        self.edit_current_picks(false, drop, move |p| pick_one(p, &name, show), cx);
    }

    pub fn pick_tag(&mut self, name: String, show: bool, cx: &mut Context<Self>) {
        let drop = (!show).then(|| format!("tag:{name}"));
        self.edit_current_picks(true, drop, move |p| pick_one(p, &name, show), cx);
    }

    pub fn pick_view_folder(&mut self, folder: String, tags: bool, show: bool, cx: &mut Context<Self>) {
        let what = if tags { "tag" } else { "branch" };
        let drop = (!show).then(|| format!("{what}:{folder}"));
        self.edit_current_picks(tags, drop, move |p| folder_edit(p, &folder, show), cx);
    }

    /// `showBranches`: the hidden list's Show all: one save, one reload.
    pub fn show_ghosts(&mut self, ghosts: Vec<Ghost>, cx: &mut Context<Self>) {
        let (branches, tags): (Vec<Ghost>, Vec<Ghost>) = ghosts.into_iter().partition(|g| !g.tag);
        let settings = self.settings.clone();
        let Some(tab) = self.tabs.get(self.active) else { return };
        let (repo, path) = (tab.id, tab.repo_path.clone());
        self.update_repo(
            &path,
            move |rs| {
                for g in &branches {
                    *rs = edit_picks(&settings, rs, false, |p| pick_one(p, &g.branch, true));
                }
                for g in &tags {
                    *rs = edit_picks(&settings, rs, true, |p| pick_one(p, &g.branch, true));
                }
            },
            cx,
        );
        self.reload(repo, cx);
    }

    /// `toggleBranch`, and Alt+click: hides it when it's shown, collapsed or folded; shows it when hidden.
    pub fn toggle_branch(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(g) = tab.graph.as_ref() else { return };
        let view = g.view.as_ref();
        if branch_state(view, &name).as_deref() == Some("hidden") {
            return self.pick_branch(name, true, cx);
        }
        if cannot_hide(view, &name, tab.head_branch().as_deref(), &g.pinned_refs).is_none() {
            self.pick_branch(name, false, cx);
        }
    }

    pub fn toggle_tag(&mut self, name: String, cx: &mut Context<Self>) {
        let hidden = self.tabs.get(self.active).and_then(|t| t.graph.as_ref()).and_then(|g| g.view.as_ref()).is_some_and(|v| strs(&v["hiddenTags"]).contains(&name));
        self.pick_tag(name, hidden, cx);
    }

    /// Alt+click on a chip: its tag, or its branch.
    pub fn alt_click_chip(&mut self, c: &Chip, cx: &mut Context<Self>) {
        if c.tag {
            self.toggle_tag(c.name.to_string(), cx);
        } else if let Some(n) = unit_name(c) {
            self.toggle_branch(n, cx);
        }
    }

    /// Unfolds `key` until the tab switches presets, or Collapse folds it again.
    pub fn expand_view(&mut self, key: &str, cx: &mut Context<Self>) {
        let k = expand_key(key);
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        if !tab.expanded.contains(&k) {
            tab.expanded.push(k);
            let repo = tab.id;
            tab.reveal = None;
            self.reload(repo, cx);
        }
    }

    pub fn collapse_view(&mut self, key: &str, cx: &mut Context<Self>) {
        let k = expand_key(key);
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let before = tab.expanded.len();
        tab.expanded.retain(|x| *x != k);
        if tab.expanded.len() != before {
            let repo = tab.id;
            self.reload(repo, cx);
        }
    }

    /// The commit menu's View rows: Hide or Show branch, Collapse branch, Fold merged branch.
    pub fn preset_branch_rows(&self, sha: &str, chip: Option<&Chip>, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let Some(g) = tab.graph.as_ref() else { return Vec::new() };
        let mut rows = Vec::new();
        if let Some(name) = chip.and_then(unit_name) {
            let view = g.view.as_ref();
            let (w, n) = (this.clone(), name.clone());
            if branch_state(view, &name).as_deref() == Some("hidden") {
                rows.push(MenuRow::action("presets.showBranch", "Show branch", "eye", format!("Show {name} in the graph again"), move |_, cx| {
                    let n = n.clone();
                    w.update(cx, |v, cx| v.pick_branch(n, true, cx)).ok();
                }));
            } else {
                let why = cannot_hide(view, &name, tab.head_branch().as_deref(), &g.pinned_refs);
                rows.push(
                    MenuRow::action("presets.hideBranch", "Hide branch", "eye-off", why.map_or_else(|| format!("Hide {name}'s commits from the graph"), str::to_string), move |_, cx| {
                        let n = n.clone();
                        w.update(cx, |v, cx| v.pick_branch(n, false, cx)).ok();
                    })
                    .disabled_if(why),
                );
            }
            if tab.expanded.contains(&format!("branch:{name}")) {
                let (w, k) = (this.clone(), format!("branch:{name}"));
                rows.push(MenuRow::action("presets.collapseBranch", "Collapse branch", "chevrons-down-up", format!("Show {name} as the branch preset has it again"), move |_, cx| {
                    let k = k.clone();
                    w.update(cx, |v, cx| v.collapse_view(&k, cx)).ok();
                }));
            }
        }
        let fold = format!("fold:{sha}");
        if tab.expanded.contains(&fold) {
            let w = this.clone();
            rows.push(MenuRow::action("presets.foldMerge", "Fold merged branch", "fold-vertical", "Fold the branch this commit merged back into it", move |_, cx| {
                let k = fold.clone();
                w.update(cx, |v, cx| v.collapse_view(&k, cx)).ok();
            }));
        }
        rows
    }

    /// A tag's Hide or Show.
    pub fn preset_tag_row(&self, name: &str, this: &WeakEntity<GitVolt>) -> MenuRow {
        let hidden = self.tabs.get(self.active).and_then(|t| t.graph.as_ref()).and_then(|g| g.view.as_ref()).is_some_and(|v| strs(&v["hiddenTags"]).iter().any(|t| t == name));
        let (w, n) = (this.clone(), name.to_string());
        let run = move |_: &mut Window, cx: &mut App| {
            let n = n.clone();
            w.update(cx, |v, cx| v.pick_tag(n, hidden, cx)).ok();
        };
        if hidden { MenuRow::action("presets.showTag", "Show tag", "eye", format!("Show {name} in the graph again"), run) } else { MenuRow::action("presets.hideTag", "Hide tag", "eye-off", format!("Hide {name}'s label from the graph, and the commits only it keeps"), run) }
    }

    /// A sidebar folder's Hide or Show. `hidden`: it looks hidden.
    pub fn preset_folder_row(&self, folder: &str, tags: bool, hidden: bool, this: &WeakEntity<GitVolt>) -> MenuRow {
        let (what, all) = if tags { ("tag", "tags") } else { ("branch", "branches") };
        let (w, f) = (this.clone(), folder.to_string());
        let run = move |_: &mut Window, cx: &mut App| {
            let f = f.clone();
            w.update(cx, |v, cx| v.pick_view_folder(f, tags, hidden, cx)).ok();
        };
        if hidden { MenuRow::action("presets.showFolder", "Show folder", "eye", format!("Show the {all} under {folder} in the graph again"), run) } else { MenuRow::action("presets.hideFolder", "Hide folder", "eye-off", format!("Hide every {what} under {folder} from the graph, also those created later"), run) }
    }

    /// `hidden` of a folder: its folder pick, or every branch or tag under it, hides.
    pub fn folder_looks_hidden(&self, kind: crate::sidebar::SectionKind, folder: &str) -> bool {
        use crate::sidebar::SectionKind;
        let Some(tab) = self.tabs.get(self.active) else { return false };
        let rs = tab.repo_settings(&self.profile);
        let list = preset_list(&self.settings, rs);
        let picks = &rs["branchPicks"][current_preset(&list, rs).id()];
        let tags = kind == SectionKind::Tags;
        let (s, h) = if tags { ("shownTags", "hiddenTags") } else { ("shown", "hidden") };
        if folder_pick_hides(&Picks { shown: strs(&picks[s]), hidden: strs(&picks[h]) }, folder) {
            return true;
        }
        let Some(section) = tab.side.sections.iter().find(|x| x.kind == kind) else { return false };
        let under: Vec<_> = section.items.iter().filter(|i| i.name.starts_with(folder)).collect();
        !under.is_empty() && under.iter().all(|i| i.state.as_deref() == Some("hidden"))
    }

    /// The sidebar's eye, and H: hides or shows a branch or a tag. `None`: it can't be hidden.
    pub fn side_item_toggle(&self, item: &crate::sidebar::SideItem) -> Option<bool> {
        use crate::sidebar::ItemKind;
        let tab = self.tabs.get(self.active)?;
        let g = tab.graph.as_ref()?;
        match &item.kind {
            ItemKind::Tag { .. } => Some(item.state.as_deref() == Some("hidden")),
            ItemKind::Local { .. } | ItemKind::Remote { .. } => {
                let hidden = item.state.as_deref() == Some("hidden");
                if !hidden && cannot_hide(g.view.as_ref(), &item.name, tab.head_branch().as_deref(), &g.pinned_refs).is_some() {
                    return None;
                }
                Some(hidden)
            }
            _ => None,
        }
    }

    pub fn toggle_side_item(&mut self, item: &crate::sidebar::SideItem, cx: &mut Context<Self>) {
        use crate::sidebar::ItemKind;
        let Some(hidden) = self.side_item_toggle(item) else { return };
        match item.kind {
            ItemKind::Tag { .. } => self.pick_tag(item.name.clone(), hidden, cx),
            _ => self.pick_branch(item.name.clone(), hidden, cx),
        }
    }

    /// The ▾ menu: every preset by group, its colour and name, a check on the current one.
    pub fn open_preset_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((list, cur)) = self.preset_state() else { return };
        let weak = cx.weak_entity();
        let mut rows: Vec<MenuRow> = Vec::new();
        for (group, label) in [("builtin", "Built in"), ("all", "All repositories"), ("repo", "This repository")] {
            let entries: Vec<&PresetEntry> = list.iter().filter(|e| e.group == group).collect();
            if entries.is_empty() {
                continue;
            }
            if !rows.is_empty() {
                rows.push(MenuRow::Separator);
            }
            rows.push(MenuRow::header(format!("presets.group.{group}"), label));
            for e in entries {
                let (id, name) = (e.id().to_string(), e.name().to_string());
                let on = e.id() == cur.id();
                let tooltip = if on { format!("The graph shows \"{name}\"") } else { format!("Show the graph as \"{name}\"") };
                let (w, i) = (weak.clone(), id.clone());
                rows.push(
                    MenuRow::action(format!("preset.{id}"), name, if on { "check" } else { "" }, tooltip, move |_, cx| {
                        let i = i.clone();
                        w.update(cx, |v, cx| v.set_preset(&i, cx)).ok();
                    })
                    .swatch(preset_colour(e.preset["color"].as_str().unwrap_or("gray"))),
                );
            }
        }
        menu::open_below("preset-caret", rows, OpenOptions { label: Some("Branch presets".into()), ..Default::default() }, window, cx);
    }

    /// `fetchReveal` and open: the panel's rows from the core.
    pub fn open_reveal(&mut self, ix: usize, pinned: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(g) = tab.graph.clone() else { return };
        let Some(row) = g.rows.get(ix) else { return };
        let (key, count, fold) = match (&row.collapsed, &row.fold) {
            (Some(c), _) => (c.key.clone(), c.count, false),
            (_, Some(f)) => (f.key.clone(), f.count, true),
            _ => return,
        };
        if let Some(r) = tab.reveal.as_ref().filter(|r| r.key == key) {
            if pinned && !r.pinned {
                let tab = self.tabs.get_mut(self.active).expect("the tab");
                tab.reveal.as_mut().expect("just seen").pinned = true;
                cx.notify();
            }
            return;
        }
        let handle = tab.list.0.borrow().base_handle.clone();
        let (top, view_h) = (-handle.offset().y.as_f32(), handle.bounds().size.height.as_f32());
        let row_top = ix as f32 * crate::metrics::row_h();
        let below = reveal_below(row_top, top, crate::metrics::row_h(), count as f32, fold);
        let _ = view_h;
        let repo = tab.id;
        let limit = tab.limit;
        let (view, pin) = self.graph_options(repo);
        let Some(view) = view else { return };
        if cfg!(test) {
            return;
        }
        let api = self.api.clone();
        let asked = key.clone();
        let read = self.rt.spawn(async move { api.dispatch(Request::GraphReveal { repo, limit: Some(limit), pin, active: None, view, key: asked, below }).await });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(v)) = read.await else { return };
            if v.is_null() {
                return;
            }
            let Ok(payload) = serde_json::from_value::<gitvolt_graph::RevealPayload>(v) else { return };
            this.update(cx, |this, cx| {
                let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) else { return };
                tab.reveal = Some(Reveal { key, row: ix, below, pinned, branch: payload.branch.clone(), count: payload.count, rows: Rc::new(graph_rows(&payload.rows)), tags: payload.tags.iter().map(|t| (t.row, t.text.clone())).collect(), list: UniformListScrollHandle::new() });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn close_reveal(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active) else { return false };
        tab.reveal_hover = tab.reveal_hover.wrapping_add(1);
        if tab.reveal.take().is_some() {
            cx.notify();
            return true;
        }
        false
    }

    /// The pointer on a ⋮ or folded row: after a rest its panel opens, if Settings' hover-open is on (the default). Leaving
    /// closes one that isn't pinned.
    pub fn hover_reveal(&mut self, ix: usize, entered: bool, cx: &mut Context<Self>) {
        if self.settings["collapseHoverOpen"] == false {
            return;
        }
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        tab.reveal_hover = tab.reveal_hover.wrapping_add(1);
        let (generation, repo) = (tab.reveal_hover, tab.id);
        if !entered {
            return;
        }
        let timer = cx.background_executor().timer(Duration::from_millis(REVEAL_HOVER_MS));
        cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |this, cx| {
                if this.tabs.get(this.active).is_some_and(|t| t.id == repo && t.reveal_hover == generation) {
                    this.open_reveal(ix, false, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// The pointer left a ⋮ row or its panel: an unpinned panel closes, unless it went from one to the other.
    pub fn leave_reveal(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let (repo, generation) = (tab.id, tab.reveal_hover);
        let timer = cx.background_executor().timer(Duration::from_millis(120));
        cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |this, cx| {
                let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) else { return };
                if tab.reveal_hover == generation && !tab.reveal_over && tab.reveal.as_ref().is_some_and(|r| !r.pinned) {
                    tab.reveal = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// A reveal panel's row: selected in the graph if it shows there, else in the details panel alone.
    pub fn select_revealed(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        if let Some(i) = tab.graph.as_ref().and_then(|g| g.by_sha.get(&id).copied()) {
            tab.revealed = None;
            return self.select(i, cx);
        }
        let repo = tab.id;
        tab.selected = None;
        tab.picks = Default::default();
        tab.compare = None;
        tab.revealed = Some(id.clone());
        tab.commit = None;
        tab.files = None;
        cx.notify();
        self.read_commit(repo, id, usize::MAX, cx);
    }

    /// The hidden list: which branches or tags fork here, each its chip, tip and Show; which preset hides them, Show all.
    pub fn open_ghosts(&mut self, ghosts: Vec<Ghost>, at: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        const ID: &str = "ghost-pop";
        let name = self.preset_state().map_or(String::new(), |(_, c)| c.name().to_string());
        let now = crate::format::now();
        let weak = cx.weak_entity();
        let m = modal::Modal::new(ID, move |_, _| {
            let title = hidden_text(&ghosts).1;
            let (w, all) = (weak.clone(), ghosts.clone());
            let button = |id: SharedString, label: &'static str| div().id(id).flex_none().px(px(8.)).py(px(2.)).border_1().border_color(theme::section_border()).rounded(px(4.)).text_size(px(12.)).text_color(theme::text_normal()).cursor_pointer().hover(|d| d.bg(theme::hover_row()).text_color(theme::text_selected())).child(label);
            let rows = ghosts.iter().map(|g| {
                let lane = g.color as usize;
                let (w, branch, tag) = (weak.clone(), g.branch.clone(), g.tag);
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .py(px(4.))
                    .hover(|d| d.bg(theme::hover_row()))
                    .child(div().flex().flex_none().items_center().gap(px(3.)).max_w(px(110.)).px(px(6.)).py(px(1.)).border_1().border_color(theme::lane(lane)).rounded(px(3.)).text_color(theme::text_selected()).when(tag, |d| d.child(icon("tag", 11., theme::text_selected()))).child(div().truncate().child(g.branch.clone())))
                    .child(div().flex().flex_col().flex_1().min_w_0().child(div().truncate().child(g.summary.clone())).child(div().truncate().text_size(px(11.)).text_color(theme::text_secondary()).child(format!("{} · {}", g.author_name, crate::format::relative_time(g.author_time, now)))))
                    .child(button(format!("ghost-show:{}", g.branch).into(), "Show").on_click(move |_, window, cx| {
                        modal::close(ID, window, cx);
                        let b = branch.clone();
                        w.update(cx, |v, cx| if tag { v.pick_tag(b, true, cx) } else { v.pick_branch(b, true, cx) }).ok();
                    }))
            });
            div()
                .flex()
                .flex_col()
                .w(px(340.))
                .py(px(4.))
                .bg(theme::panel_bg1())
                .border_1()
                .border_color(theme::section_border())
                .rounded(px(6.))
                .shadow(crate::ui::shadow(theme::shadow_popup(), 6., 20.))
                .text_color(theme::text_normal())
                .child(div().px(px(10.)).pt(px(4.)).pb(px(6.)).text_size(px(12.)).text_color(theme::text_secondary()).child(title))
                .children(rows)
                .child(div().flex().items_center().justify_between().gap(px(8.)).mt(px(4.)).px(px(10.)).pt(px(6.)).pb(px(2.)).border_t_1().border_color(theme::section_border()).child(div().min_w_0().truncate().text_size(px(12.)).text_color(theme::text_secondary()).child(format!("Hidden by “{name}”"))).child(button("ghost-show-all".into(), "Show all").on_click(move |_, window, cx| {
                    modal::close(ID, window, cx);
                    let list = all.clone();
                    w.update(cx, |v, cx| v.show_ghosts(list, cx)).ok();
                })))
                .into_any_element()
        })
        .unframed()
        .width(340.)
        .anchored(Some(at));
        modal::open(m, window, cx);
    }

    /// The Graph header's pin picker: Default, None and every branch. Picking stores the repository's pin and reloads.
    pub fn open_pin_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        const AUTO: &str = "\u{0}auto";
        const OFF: &str = "\u{0}off";
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(g) = tab.graph.as_ref() else { return };
        let (repo, path) = (tab.id, tab.repo_path.clone());
        let setting = tab.repo_settings(&self.profile)["pin"].clone();
        let kind = setting["kind"].as_str().unwrap_or("auto").to_string();
        let current = if kind == "ref" { setting["name"].as_str().unwrap_or_default().to_string() } else if kind == "off" { OFF.to_string() } else { AUTO.to_string() };
        let short = |r: &str| crate::stacks::short_ref(r);
        let pair_label = |refs: &[String]| refs.iter().map(|r| short(r)).collect::<Vec<_>>().join(" ↔ ");
        let locals = tab.locals();
        let tip_of = |r: &str, branch: Option<&str>| {
            let pair: Vec<String> = if let Some(l) = locals.iter().find(|b| b.full_name == r) {
                match l.upstream.as_ref().filter(|_| !l.gone) {
                    Some(u) => vec![r.to_string(), u.clone()],
                    None => vec![r.to_string()],
                }
            } else {
                let mut tracking: Vec<&crate::sync::LocalBranch> = locals.iter().filter(|b| b.upstream.as_deref() == Some(r)).collect();
                tracking.sort_by_key(|b| (Some(b.name.as_str()) != branch, b.name.to_lowercase()));
                tracking.first().map_or(vec![r.to_string()], |t| vec![t.full_name.clone(), r.to_string()])
            };
            if pair.len() > 1 { format!("Pin {}: both stay at the left", pair_label(&pair)) } else { format!("Pin {} as the trunk", short(r)) }
        };
        let mut branches: Vec<crate::ui::picker::PickItem> = locals.iter().map(|b| crate::ui::picker::PickItem { id: b.full_name.clone().into(), label: b.name.clone().into(), detail: Some("local".into()), icon: Some("git-branch"), current: false, tooltip: Some(tip_of(&b.full_name, None).into()) }).collect();
        for (remote, list) in tab.remote_groups() {
            for (name, full, _) in list {
                branches.push(crate::ui::picker::PickItem { id: full.clone().into(), label: format!("{remote}/{name}").into(), detail: Some("remote".into()), icon: Some("git-branch"), current: false, tooltip: Some(tip_of(&full, Some(&name)).into()) });
            }
        }
        branches.sort_by_key(|b| b.label.to_lowercase());
        let auto_detail = (kind == "auto" && !g.pinned_refs.is_empty()).then(|| SharedString::from(pair_label(&g.pinned_refs)));
        let mut items = vec![
            crate::ui::picker::PickItem { id: AUTO.into(), label: "Default".into(), detail: auto_detail, icon: Some("rotate-ccw"), current: false, tooltip: Some("The repository's default branch on origin, else main or master".into()) },
            crate::ui::picker::PickItem { id: OFF.into(), label: "None".into(), detail: None, icon: Some("pin-off"), current: false, tooltip: Some("No trunk: every branch lays out freely".into()) },
        ];
        items.extend(branches);
        for i in &mut items {
            i.current = i.id.as_ref() == current;
        }
        let weak = cx.weak_entity();
        crate::ui::picker::open_below(
            "graph-pin",
            items,
            "Pin a branch as trunk",
            None,
            move |item, _, cx| {
                let pin = match item.id.as_ref() {
                    AUTO => json!({ "kind": "auto" }),
                    OFF => json!({ "kind": "off" }),
                    id => json!({ "kind": "ref", "name": id }),
                };
                let path = path.clone();
                weak.update(cx, |v, cx| {
                    v.update_repo(&path, move |rs| rs["pin"] = pin, cx);
                    v.reload(repo, cx);
                })
                .ok();
            },
            window,
            cx,
        );
    }
}

/// A reveal panel over the graph: its rows drawn as the graph's, its title on the side away from the row.
pub fn reveal_panel(tab: &Tab, r: &Reveal, cols: crate::model::Columns, view_h: f32, selected: Option<&str>, cx: &mut Context<GitVolt>) -> AnyElement {
    use crate::graph::{RowGeometry, paint_row};
    use crate::model::{Col};
    let handle = tab.list.0.borrow().base_handle.clone();
    let offset = handle.offset().y.as_f32();
    let (view_top, row_top) = (-offset, r.row as f32 * crate::metrics::row_h());
    let (top, height) = panel_box(r.below, row_top, view_top, view_top + view_h, crate::metrics::row_h(), r.rows.len() as f32);
    let list_h = height - crate::metrics::row_h();
    let fold = r.key.starts_with("fold:");
    let word = if r.count == 1 { "commit" } else { "commits" };
    let what = if fold { format!("{} {word} merged", r.count) } else { format!("{} hidden {word}", r.count) };
    let key = r.key.clone();
    let below = r.below;
    let title = div()
        .flex()
        .flex_none()
        .h(px(crate::metrics::row_h()))
        .items_center()
        .gap(px(6.))
        .px(px(crate::metrics::cell_pad()))
        .bg(theme::panel_bg1())
        .whitespace_nowrap()
        .child(div().truncate().font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).child(r.branch.clone()))
        .child(div().text_color(theme::text_secondary()).child(format!("· {what}")))
        .child(crate::ui::anchor("reveal-expand", div().id("reveal-expand").ml_auto().px(px(4.)).rounded(px(3.)).text_color(theme::blue()).cursor_pointer().hover(|d| d.underline()).on_click(cx.listener(move |this, _, _, cx| {
            let k = key.clone();
            this.expand_view(&k, cx);
        })).child("Expand in graph")));
    let rows = r.rows.clone();
    let tags = r.tags.clone();
    let selected = selected.map(str::to_string);
    let hidden = tab.hidden_columns.clone();
    let list = uniform_list(
        "reveal-list",
        rows.len(),
        cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
            let rows = rows.clone();
            range
                .map(|ix| {
                    let row = &rows[ix];
                    let on = selected.as_deref() == Some(row.id.as_ref());
                    let geometry = RowGeometry { chips_end: 0., graph_x: if hidden.contains(&Col::Labels) { 0. } else { cols.labels }, graph_w: cols.graph, clipped: cols.clipped, selected: on, head: false };
                    let painter = rows.clone();
                    let cell = |w: f32, color: Hsla, text: SharedString| div().flex().flex_none().w(px(w)).h(px(crate::metrics::row_h())).items_center().px(px(crate::metrics::cell_pad())).min_w_0().overflow_hidden().text_color(if on { theme::text_selected() } else { color }).when(on, |d| d.bg(theme::selected_row())).child(div().truncate().child(text));
                    let id = row.id.to_string();
                    let badges = tags.iter().filter(|t| t.0 as usize == ix).map(|t| div().flex_none().ml(px(6.)).px(px(5.)).border_1().border_color(theme::section_border()).rounded(px(3.)).text_size(px(10.)).line_height(px(14.)).child(t.1.clone()));
                    div()
                        .id(("reveal-row", ix))
                        .relative()
                        .flex()
                        .w_full()
                        .h(px(crate::metrics::row_h()))
                        .hover(|d| d.bg(theme::hover_row()))
                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.select_revealed(id.clone(), cx);
                        }))
                        .child(canvas(|_, _, _| {}, move |bounds, _, window, cx| paint_row(bounds, &painter[ix], ix, geometry, None, window, cx)).absolute().top_0().left_0().size_full())
                        .when(!hidden.contains(&Col::Labels), |d| d.child(div().flex_none().w(px(cols.labels))))
                        .child(div().flex_none().w(px(cols.graph)))
                        .child(cell(cols.message, theme::text_normal(), row.summary.clone()).children(badges))
                        .when(!hidden.contains(&Col::Author), |d| d.child(cell(cols.author, theme::text_disabled(), row.author.clone())))
                        .when(!hidden.contains(&Col::Date), |d| d.child(cell(cols.date, theme::text_disabled(), row.date.clone())))
                        .when(!hidden.contains(&Col::Sha), |d| d.child(cell(cols.sha, theme::text_disabled(), row.sha.clone()).font_family("JetBrains Mono").px(px(6.))))
                        .child(div().flex_1().h(px(crate::metrics::row_h())).when(on, |d| d.bg(theme::selected_row())))
                        .into_any_element()
                })
                .collect()
        }),
    )
    .track_scroll(&r.list)
    .h(px(list_h));
    deferred(
        div()
            .id("reveal-panel")
            .absolute()
            .left_0()
            .right_0()
            .top(px(top + offset))
            .h(px(height))
            .flex()
            .flex_col()
            .occlude()
            .bg(theme::panel_bg())
            .shadow(vec![BoxShadow { color: theme::section_border(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.), inset: false }, BoxShadow { color: theme::shadow_popup(), offset: point(px(0.), px(8.)), blur_radius: px(24.), spread_radius: px(0.), inset: false }])
            .on_hover(cx.listener(|this, entered: &bool, _, cx| {
                if let Some(tab) = this.tabs.get_mut(this.active) {
                    tab.reveal_over = *entered;
                }
                if !*entered {
                    this.leave_reveal(cx);
                }
            }))
            .when(below, |d| d.flex_col_reverse())
            .child(title)
            .child(list),
    )
    .with_priority(2)
    .into_any_element()
}

/// `presetColour`: a preset's colour; a name outside the palette shows gray.
pub fn preset_colour(name: &str) -> Hsla {
    theme::tag_group(name)
}

/// `hiddenText`: after the summary of a commit hidden branches or tags fork from.
pub fn hidden_text(ghosts: &[Ghost]) -> (String, String) {
    let count = ghosts.len();
    let tags = ghosts.iter().filter(|g| g.tag).count();
    let what = if tags == 0 { if count == 1 { "branch" } else { "branches" } } else if tags == count { if count == 1 { "tag" } else { "tags" } } else { "branches and tags" };
    (format!("⋮ {count} hidden"), format!("{count} hidden {what} {} here", if count == 1 { "starts" } else { "start" }))
}

/// The preset button, right of the sidebar's filter: a click moves to the next preset in the cycle, ▾ lists them all. Its icon
/// carries the current preset's colour as a dot.
pub fn preset_button(this: &GitVolt, cx: &mut Context<GitVolt>) -> AnyElement {
    let Some((list, cur)) = this.preset_state() else { return div().into_any_element() };
    let next = next_in_cycle(&list, cur.id());
    let tooltip = format!("Branches: {} · {}▾ all presets", cur.name(), next.map_or(String::new(), |n| format!("click for {} · ", n.name())));
    let has_next = next.is_some();
    let colour = preset_colour(cur.preset["color"].as_str().unwrap_or("gray"));
    tip(
        tooltip,
        div()
            .flex()
            .flex_none()
            .h(px(24.))
            .bg(theme::app_bg())
            .border_1()
            .border_color(theme::section_border())
            .rounded(px(4.))
            .overflow_hidden()
            .child(crate::ui::anchor(
                "preset-main",
                div()
                    .id("preset-main")
                    .relative()
                    .flex()
                    .w(px(26.))
                    .h_full()
                    .items_center()
                    .justify_center()
                    .when(has_next, |d| d.cursor_pointer().hover(|d| d.bg(theme::hover_row())))
                    .when(!has_next, |d| d.opacity(0.6))
                    .on_click(cx.listener(|this, _, _, cx| this.cycle_preset(cx)))
                    .child(icon("git-fork", 13., theme::text_secondary()))
                    .child(div().absolute().right(px(4.)).bottom(px(4.)).size(px(6.)).rounded_full().bg(colour).border_1().border_color(theme::app_bg())),
            ))
            .child(crate::ui::anchor(
                "preset-caret",
                div().id("preset-caret").flex().w(px(16.)).h_full().items_center().justify_center().border_l_1().border_color(theme::section_border()).cursor_pointer().hover(|d| d.bg(theme::hover_row())).on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.open_preset_menu(window, cx);
                })).child(icon("chevron-down", 11., theme::text_secondary())),
            )),
    )
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn p(shown: &[&str], hidden: &[&str]) -> Picks {
        let f = |a: &[&str]| a.iter().map(|s| s.to_string()).collect();
        Picks { shown: f(shown), hidden: f(hidden) }
    }

    #[test]
    fn all_is_there_and_the_cycle_wraps() {
        let list = preset_list(&json!({ "builtinPresets": [{ "id": "all", "inCycle": true }, { "id": "mine", "inCycle": false }] }), &Value::Null);
        assert_eq!(list.iter().map(PresetEntry::id).collect::<Vec<_>>(), ["all", "mine", "all-collapsed"]);
        assert_eq!(current_preset(&list, &Value::Null).id(), "all");
        assert_eq!(next_in_cycle(&list, "all").map(PresetEntry::id), Some("all-collapsed"), "mine isn't ticked");
        assert_eq!(next_in_cycle(&list, "all-collapsed").map(PresetEntry::id), Some("all"));
        assert_eq!(view_spec_of(&list[0], &Value::Null, &[], &Value::Null), None, "All asks for nothing");
        assert!(view_spec_of(&list[2], &Value::Null, &["branch:x".to_string()], &json!("hide")).is_some());
    }

    #[test]
    fn hiding_on_a_built_in_goes_to_the_repositorys_custom() {
        let repo = edit_picks(&json!({}), &Value::Null, false, |o| pick_one(o, "topic", false));
        assert_eq!(repo["branchPreset"], "custom");
        assert_eq!(repo["customPreset"]["name"], "Custom");
        assert_eq!(repo["branchPicks"]["custom"]["hidden"], json!(["topic"]));
        let repo = edit_picks(&json!({}), &repo, true, |o| pick_one(o, "v1", false));
        assert_eq!(repo["branchPicks"]["custom"]["hiddenTags"], json!(["v1"]));
        assert_eq!(repo["branchPicks"]["custom"]["hidden"], json!(["topic"]), "the branch lists stay");
    }

    #[test]
    fn a_folder_pick_decides_for_what_is_under_it() {
        assert!(folder_pick_hides(&p(&[], &["a/"]), "a/b/"));
        assert!(!folder_pick_hides(&p(&["a/b/"], &["a/"]), "a/b/"));
        assert!(!folder_pick_hides(&p(&[], &[]), "a/"));
        // Hiding one a folder hides only drops its exception.
        assert_eq!(pick_one(&p(&["a/x"], &["a/"]), "a/x", false), p(&[], &["a/"]));
        assert_eq!(folder_edit(&p(&["a/x"], &["a/y"]), "a/", false), p(&[], &["a/"]));
        // Showing drops the picks under it; a pick that hid it is enough.
        assert_eq!(folder_edit(&p(&[], &["a/"]), "a/", true), p(&[], &[]));
        assert_eq!(folder_edit(&p(&[], &[]), "a/", true), p(&["a/"], &[]), "the preset's rules hid it");
    }

    #[test]
    fn the_trunk_and_what_is_checked_out_stay() {
        let view = json!({ "branches": [{ "name": "x", "state": "hidden" }], "trunk": ["main"] });
        assert_eq!(cannot_hide(Some(&view), "main", None, &[]), Some("The trunk is always shown"));
        assert_eq!(cannot_hide(Some(&view), "t", Some("t"), &[]), Some("Checked out: always shown"));
        assert_eq!(cannot_hide(None, "main", None, &["refs/remotes/origin/main".to_string()]), Some("The trunk is always shown"));
        assert_eq!(cannot_hide(None, "t", None, &[]), None);
        assert_eq!(branch_state(Some(&view), "x").as_deref(), Some("hidden"));
    }

    #[test]
    fn a_runs_key_unfolds_its_branch() {
        assert_eq!(expand_key(&format!("branch:x@{}", "a".repeat(40))), "branch:x");
        assert_eq!(expand_key("branch:x"), "branch:x");
        assert_eq!(expand_key("fold:abc"), "fold:abc");
    }

    #[test]
    fn a_panel_opens_above_when_it_fits() {
        assert!(reveal_below(100., 0., 28., 3., true), "a fold's always below");
        assert!(!reveal_below(280., 0., 28., 3., false));
        assert!(reveal_below(28., 0., 28., 3., false));
        let (top, h) = panel_box(true, 28., 0., 280., 28., 100.);
        assert_eq!((top, h), (56., 224.));
        let (top, h) = panel_box(false, 280., 0., 800., 28., 3.);
        assert_eq!((top, h), (168., 112.));
    }
}
