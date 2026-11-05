//! Settings › Branch presets (`presets/settings/`): the list on the left, the selected preset's editor on the right, and
//! `edit.ts`'s edits over the settings and the open repository's settings.

use super::{ALL_ID, CUSTOM_ID, PresetEntry, current_preset, preset_colour, preset_list};
use crate::app::{GitVolt, icon};
use crate::settings::frame::input_box;
use crate::settings::pages::Def;
use crate::settings::themes::{parse_share, to_share, unique_name};
use crate::theme;
use crate::ui::controls::{self, OnSelect};
use crate::ui::{arm, toast};
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::rc::Rc;

type OnAction = Rc<dyn Fn(&mut GitVolt, &str, &mut Context<GitVolt>)>;
type SetRule = Rc<dyn Fn(&mut GitVolt, Value, &mut Context<GitVolt>)>;

const GROUP_COLORS: [&str; 9] = ["blue", "purple", "cyan", "orange", "yellow", "pink", "green", "gray", "red"];
const MATCHERS: [(&str, &str); 8] = [("trunk", "is the trunk"), ("checkedOut", "is checked out"), ("commitsBy", "has commits by"), ("nameMatches", "name matches"), ("remote", "is remote"), ("local", "is local"), ("olderThan", "last commit older than"), ("mergedIntoTrunk", "is merged into the trunk")];

#[derive(Default)]
pub struct PageState {
    pub selected: Option<String>,
    pub preview: Option<(String, Value)>,
    pub asked: String,
    /// A row or rule being dragged.
    pub drag: Option<(&'static str, usize)>,
}

pub fn defs() -> Vec<Def> {
    vec![Def { id: "branchPresets", label: "Branch presets", page: "branch-presets", group: "Presets", keywords: "hide collapse fold branches preset view filter cycle mine custom rules picked ghost lines stub hidden fork", description: "Named views of the graph: their rules, colours and which ones the button steps through.".into(), control: crate::settings::pages::Ctl::OpenPage, binding: crate::settings::pages::Binding::None }]
}

fn new_id() -> String {
    crate::settings::themes::new_custom_id().replace("custom-", "preset-") + &format!("{:x}", crate::format::now() as u64)
}

/// `homeOf`.
fn home_of(settings: &Value, repo: &Value, id: &str) -> Option<&'static str> {
    if id == ALL_ID || super::is_builtin(id) {
        return Some("builtin");
    }
    if settings["branchPresets"].as_array().is_some_and(|l| l.iter().any(|p| p["id"] == id)) {
        return Some("all");
    }
    if repo["branchPresets"].as_array().is_some_and(|l| l.iter().any(|p| p["id"] == id)) {
        return Some("repo");
    }
    (repo["customPreset"]["id"] == id).then_some("custom")
}

/// `parsePresetJson`, the fields it checks.
pub fn parse_preset(text: &str) -> Option<Value> {
    let (b, _) = parse_share(text, "branch-preset", 1)?;
    let name = b.get("name")?.as_str()?.trim().to_string();
    let color = b.get("color")?.as_str()?;
    if name.is_empty() || !GROUP_COLORS.contains(&color) {
        return None;
    }
    let is_action = |v: &Value| matches!(v.as_str(), Some("show" | "hide" | "collapse"));
    let otherwise = b.get("otherwise")?;
    if !is_action(otherwise) {
        return None;
    }
    let rules = b.get("rules")?.as_array()?;
    for r in rules {
        if !is_action(&r["action"]) || !MATCHERS.iter().any(|m| r["matcher"]["kind"] == m.0) {
            return None;
        }
    }
    Some(json!({ "name": name, "color": color, "rules": rules, "otherwise": otherwise, "foldMerged": b.get("foldMerged").cloned().unwrap_or(json!(false)), "ghostLines": b.get("ghostLines").cloned().unwrap_or(json!(false)), "ghostPicks": b.get("ghostPicks").cloned().unwrap_or(json!(false)) }))
}

/// `presetToJson`.
pub fn preset_to_json(p: &Value) -> String {
    to_share("branch-preset", 1, &[("name", p["name"].clone()), ("color", p["color"].clone()), ("rules", p["rules"].clone()), ("otherwise", p["otherwise"].clone()), ("foldMerged", p["foldMerged"].clone()), ("ghostLines", p["ghostLines"].clone()), ("ghostPicks", p["ghostPicks"].clone())])
}

impl GitVolt {
    fn repo_doc(&self) -> Value {
        match self.tabs.get(self.active).filter(|t| !t.is_open_tab()) {
            Some(t) => {
                let r = &self.profile["repos"][&t.repo_path];
                if r.is_object() { r.clone() } else { json!({}) }
            }
            None => Value::Null,
        }
    }

    /// `save`: what an edit changed, then the graphs whose view it changed.
    fn presets_apply(&mut self, settings: Value, repo: Value, cx: &mut Context<Self>) {
        if settings != self.settings {
            self.set_settings(move |s| *s = settings, cx);
        }
        if let Some(path) = self.tabs.get(self.active).filter(|t| !t.is_open_tab()).map(|t| t.repo_path.clone())
            && repo.is_object()
            && repo != self.repo_doc()
        {
            self.update_repo(&path, move |r| *r = repo, cx);
        }
        self.reload_active(cx);
    }

    fn preset_names(&self) -> Vec<String> {
        preset_list(&self.settings, &self.repo_doc()).iter().map(|e| e.name().to_string()).collect()
    }

    /// `updatePreset`: where it lives; renaming or moving Custom makes it a regular preset.
    fn preset_edit(&mut self, id: &str, change: impl FnOnce(&mut Value), cx: &mut Context<Self>) {
        let (mut settings, mut repo) = (self.settings.clone(), self.repo_doc());
        match home_of(&settings, &repo, id) {
            Some("all") => {
                if let Some(p) = settings["branchPresets"].as_array_mut().and_then(|l| l.iter_mut().find(|p| p["id"] == id)) {
                    change(p);
                }
            }
            Some("repo") => {
                let Some(list) = repo["branchPresets"].as_array_mut() else { return };
                let Some(at) = list.iter().position(|p| p["id"] == id) else { return };
                change(&mut list[at]);
                if list[at]["scope"] == "all" {
                    let p = list.remove(at);
                    push_all(&mut settings, p);
                }
            }
            Some("custom") => {
                let before = repo["customPreset"].clone();
                let mut after = before.clone();
                change(&mut after);
                if after["name"] == before["name"] && after["scope"] == before["scope"] {
                    repo["customPreset"] = after;
                } else {
                    let fresh = new_id();
                    after["id"] = json!(fresh);
                    let picks = repo["branchPicks"].as_object_mut().and_then(|m| m.remove(CUSTOM_ID));
                    if let Some(picks) = picks {
                        repo["branchPicks"][&fresh] = picks;
                    }
                    if repo["branchPreset"] == CUSTOM_ID {
                        repo["branchPreset"] = json!(fresh);
                    }
                    repo.as_object_mut().map(|m| m.remove("customPreset"));
                    if after["scope"] == "all" {
                        push_all(&mut settings, after);
                    } else {
                        after["scope"] = json!("repo");
                        push_to(&mut repo, after);
                    }
                    self.settings_ui.presets.selected = Some(fresh);
                }
            }
            _ => return,
        }
        self.presets_apply(settings, repo, cx);
    }

    fn preset_set_cycle(&mut self, entry: &PresetEntry, on: bool, cx: &mut Context<Self>) {
        let id = entry.id().to_string();
        let (mut settings, mut repo) = (self.settings.clone(), self.repo_doc());
        match home_of(&settings, &repo, &id) {
            Some("builtin") => {
                let mut slots: Vec<Value> = preset_list(&settings, &Value::Null).iter().filter(|e| e.group == "builtin").map(|e| json!({ "id": e.id(), "inCycle": e.in_cycle })).collect();
                if let Some(s) = slots.iter_mut().find(|s| s["id"] == id.as_str()) {
                    s["inCycle"] = json!(on);
                }
                settings["builtinPresets"] = json!(slots);
                self.presets_apply(settings, repo, cx);
            }
            Some("custom") => {
                repo["customPreset"]["inCycle"] = json!(on);
                self.presets_apply(settings, repo, cx);
            }
            _ => self.preset_edit(&id, |p| p["inCycle"] = json!(on), cx),
        }
    }

    /// `newPreset`: for all repositories, in the cycle, showing everything until it gets rules.
    fn preset_new(&mut self, cx: &mut Context<Self>) {
        let mut settings = self.settings.clone();
        let used: Vec<String> = settings["branchPresets"].as_array().map_or(Vec::new(), |l| l.iter().filter_map(|p| p["color"].as_str().map(str::to_string)).collect());
        let color = GROUP_COLORS.iter().find(|c| **c != "gray" && !used.iter().any(|u| u == *c)).unwrap_or(&"blue");
        let id = new_id();
        push_all(&mut settings, json!({ "id": id, "name": unique_name("New preset", &self.preset_names()), "color": color, "scope": "all", "rules": [], "otherwise": "show", "foldMerged": false, "ghostLines": true, "ghostPicks": false, "inCycle": true }));
        self.settings_ui.presets.selected = Some(id);
        let repo = self.repo_doc();
        self.presets_apply(settings, repo, cx);
    }

    fn preset_duplicate(&mut self, entry: &PresetEntry, cx: &mut Context<Self>) {
        let (mut settings, mut repo) = (self.settings.clone(), self.repo_doc());
        let id = new_id();
        let mut copy = entry.preset.clone();
        copy["id"] = json!(id);
        copy["name"] = json!(unique_name(entry.name(), &self.preset_names()));
        copy["inCycle"] = json!(true);
        if matches!(home_of(&settings, &repo, entry.id()), Some("repo" | "custom")) {
            copy["scope"] = json!("repo");
            push_to(&mut repo, copy);
        } else {
            copy["scope"] = json!("all");
            push_all(&mut settings, copy);
        }
        self.settings_ui.presets.selected = Some(id);
        self.presets_apply(settings, repo, cx);
    }

    /// `deletePreset`: with the open repository's picks for it; a repository on it falls back to All.
    fn preset_delete(&mut self, id: &str, cx: &mut Context<Self>) {
        let (mut settings, mut repo) = (self.settings.clone(), self.repo_doc());
        let home = home_of(&settings, &repo, id);
        if home == Some("all")
            && let Some(l) = settings["branchPresets"].as_array_mut()
        {
            l.retain(|p| p["id"] != id);
        }
        if repo.is_object() {
            if let Some(m) = repo["branchPicks"].as_object_mut() {
                m.remove(id);
            }
            if home == Some("repo")
                && let Some(l) = repo["branchPresets"].as_array_mut()
            {
                l.retain(|p| p["id"] != id);
            }
            if home == Some("custom") {
                repo.as_object_mut().map(|m| m.remove("customPreset"));
            }
            if repo["branchPreset"] == id {
                repo.as_object_mut().map(|m| m.remove("branchPreset"));
            }
        }
        self.settings_ui.presets.selected = None;
        self.presets_apply(settings, repo, cx);
    }

    /// `importInto`.
    pub fn paste_preset(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(mut p) = parse_preset(text) else {
            toast::show("That isn't a GitVolt branch preset", cx);
            return;
        };
        let name = unique_name(p["name"].as_str().unwrap_or_default(), &self.preset_names());
        let id = new_id();
        p["name"] = json!(name);
        p["id"] = json!(id);
        p["scope"] = json!("all");
        p["inCycle"] = json!(true);
        let mut settings = self.settings.clone();
        push_all(&mut settings, p);
        self.settings_ui.presets.selected = Some(id);
        let repo = self.repo_doc();
        self.presets_apply(settings, repo, cx);
        toast::show(format!("Added {name}"), cx);
    }

    fn preset_move(&mut self, group: &'static str, from: usize, to: usize, cx: &mut Context<Self>) {
        let (mut settings, mut repo) = (self.settings.clone(), self.repo_doc());
        let swap = |l: &mut Vec<Value>| {
            if from < l.len() && to < l.len() {
                let x = l.remove(from);
                l.insert(to, x);
            }
        };
        match group {
            "builtin" => {
                let mut slots: Vec<Value> = preset_list(&settings, &Value::Null).iter().filter(|e| e.group == "builtin").map(|e| json!({ "id": e.id(), "inCycle": e.in_cycle })).collect();
                swap(&mut slots);
                settings["builtinPresets"] = json!(slots);
            }
            "all" => {
                if let Some(l) = settings["branchPresets"].as_array_mut() {
                    swap(l);
                }
            }
            _ => {
                if let Some(l) = repo["branchPresets"].as_array_mut() {
                    swap(l);
                }
            }
        }
        self.presets_apply(settings, repo, cx);
    }

    /// The live counts, asked 150 ms after the last edit, never polled; the last answer stays up meanwhile.
    fn ask_preset_preview(&mut self, entry: &PresetEntry, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active).filter(|t| !t.is_open_tab()) else { return };
        let repo_id = tab.id;
        let limit = self.settings["commitLimit"].as_u64().unwrap_or(2000) as u32;
        let rs = self.repo_doc();
        let p = &entry.preset;
        let picks = &rs["branchPicks"][entry.id()];
        let spec = json!({ "rules": p["rules"], "otherwise": p["otherwise"], "picks": if picks.is_object() { picks.clone() } else { json!({ "shown": [], "hidden": [] }) }, "foldMerged": p["foldMerged"], "ghostLines": p["ghostLines"], "ghostPicks": p["ghostPicks"], "trunkPatterns": rs.get("trunkPatterns").filter(|t| t.is_array()).cloned().unwrap_or(json!([])), "expanded": [], "placement": self.settings["collapsePlacement"] });
        let key = format!("{}|{}|{limit}|{}", entry.id(), spec, rs["pin"]);
        if self.settings_ui.presets.asked == key || cfg!(test) {
            return;
        }
        self.settings_ui.presets.asked = key.clone();
        let id = entry.id().to_string();
        let api = self.api.clone();
        let pin = rs.get("pin").filter(|p| p.is_object()).cloned();
        let timer = cx.background_executor().timer(std::time::Duration::from_millis(150));
        cx.spawn(async move |this, cx| {
            timer.await;
            let Ok(Some(rt)) = this.update(cx, |v, _| (v.settings_ui.presets.asked == key).then(|| v.rt.clone())) else { return };
            let body = crate::write::call("branchPresetPreview", json!({ "repo": crate::app::core_repo(repo_id), "limit": limit, "pin": pin, "view": spec }));
            let read = rt.spawn(async move { match crate::write::request_of(body) { Ok(req) => api.dispatch(req).await, Err(e) => Err(e) } });
            let answer = read.await;
            this.update(cx, |v, cx| {
                if let Ok(Ok(p)) = answer {
                    v.settings_ui.presets.preview = Some((id, p));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}

fn push_all(settings: &mut Value, p: Value) {
    if !settings["branchPresets"].is_array() {
        settings["branchPresets"] = json!([]);
    }
    settings["branchPresets"].as_array_mut().expect("just made").push(p);
}

fn push_to(repo: &mut Value, p: Value) {
    if !repo["branchPresets"].is_array() {
        repo["branchPresets"] = json!([]);
    }
    repo["branchPresets"].as_array_mut().expect("just made").push(p);
}

fn link(id: impl Into<ElementId>, label: &'static str) -> Stateful<Div> {
    div().id(id).py(px(2.)).text_color(theme::link()).cursor_pointer().child(label)
}

fn hint(text: &str) -> Div {
    div().text_size(px(12.)).font_weight(FontWeight::NORMAL).text_color(theme::text_secondary()).child(text.to_string())
}

fn section_head(title: &str, note: Option<&str>) -> Div {
    div().flex().items_baseline().gap(px(6.)).mb(px(6.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).child(title.to_string()).children(note.map(hint))
}

/// Show / Hide / Collapse.
fn action_choice(key: String, value: &str, read_only: bool, on: OnAction, cx: &mut Context<GitVolt>) -> Div {
    let mut seg = div().flex().flex_none().rounded(px(4.)).border_1().border_color(theme::section_border()).overflow_hidden();
    for (a, text) in [("show", "Show"), ("hide", "Hide"), ("collapse", "Collapse")] {
        let on_now = value == a;
        let handler = on.clone();
        seg = seg.child(crate::ui::anchor(format!("{key}:{a}"), div().id(ElementId::Name(format!("{key}-{a}").into())).px(px(8.)).py(px(3.)).when(on_now, |d| d.bg(theme::selected_row()).text_color(match a { "show" => theme::green(), "hide" => theme::red(), _ => theme::blue() })).when(!on_now, |d| d.text_color(theme::text_secondary())).when(!read_only, |d| d.cursor_pointer().hover(|d| d.bg(theme::hover_row()))).on_click(cx.listener(move |v, _, _, cx| {
            if !read_only {
                handler(v, a, cx);
            }
        })).child(text)));
    }
    seg
}

fn count_text(n: Option<u64>) -> String {
    n.map_or(String::new(), |n| format!("{n} branch{}", if n == 1 { "" } else { "es" }))
}

#[allow(clippy::too_many_arguments)]
fn rule_row(this: &mut GitVolt, pid: &str, i: usize, rule: &Value, count: Option<u64>, read_only: bool, window: &Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let n = i + 1;
    let set_rule = |pid: String, i: usize| -> SetRule {
        Rc::new(move |v, next, cx| {
            let pid = pid.clone();
            v.preset_edit(&pid, move |p| p["rules"][i] = next, cx);
        })
    };
    let set = set_rule(pid.to_string(), i);
    let kind = rule["matcher"]["kind"].as_str().unwrap_or_default().to_string();
    let (s1, r1) = (set.clone(), rule.clone());
    let action = action_choice(format!("rule:{n}"), rule["action"].as_str().unwrap_or_default(), read_only, Rc::new(move |v, a, cx| {
        let mut next = r1.clone();
        next["action"] = json!(a);
        s1(v, next, cx);
    }), cx);
    let label = MATCHERS.iter().find(|m| m.0 == kind).map_or(kind.clone(), |m| m.1.to_string());
    let mut what = div().flex().flex_1().min_w_0().flex_wrap().items_center().gap(px(6.));
    if read_only {
        what = what.child(label);
    } else {
        let options: Vec<(String, SharedString, Option<SharedString>)> = MATCHERS.iter().map(|m| (m.0.to_string(), m.1.into(), None)).collect();
        let (weak, s2, r2) = (cx.weak_entity(), set.clone(), rule.clone());
        let on_change: OnSelect<String> = Rc::new(move |k, _, cx| {
            let mut next = r2.clone();
            next["matcher"] = match k.as_str() {
                x if r2["matcher"]["kind"] == x => r2["matcher"].clone(),
                "commitsBy" => json!({ "kind": k, "author": { "kind": "me" } }),
                "nameMatches" => json!({ "kind": k, "patterns": "" }),
                "olderThan" => json!({ "kind": k, "days": 90 }),
                _ => json!({ "kind": k }),
            };
            let s2 = s2.clone();
            weak.update(cx, |v, cx| s2(v, next, cx)).ok();
        });
        what = what.child(div().flex().flex_none().w(px(190.)).child(controls::select(format!("rule-kind:{n}"), kind.clone(), Rc::new(options), None, on_change, cx)));
    }
    match kind.as_str() {
        "checkedOut" => what = what.child(hint("in any worktree; always shown, even when a rule hides it")),
        "nameMatches" => {
            let patterns = rule["matcher"]["patterns"].as_str().unwrap_or_default().to_string();
            if read_only {
                what = what.child(div().font_family("monospace").child(patterns));
            } else {
                let (s3, r3) = (set.clone(), rule.clone());
                let input = this.settings_field(&format!("rule-pat:{pid}:{i}"), &patterns, "dependabot/*, renovate/*", Rc::new(move |v, text, cx| {
                    let t = text.trim().to_string();
                    if r3["matcher"]["patterns"] != t.as_str() {
                        let mut next = r3.clone();
                        next["matcher"]["patterns"] = json!(t);
                        s3(v, next, cx);
                    }
                }), None, window, cx);
                what = what.child(input_box(ElementId::Name(format!("rule-pat-{n}").into()), input, false, window, cx).h(px(24.)));
            }
        }
        "olderThan" => {
            let days = rule["matcher"]["days"].as_u64().unwrap_or(90).to_string();
            if read_only {
                what = what.child(days);
            } else {
                let (s3, r3) = (set.clone(), rule.clone());
                let input = this.settings_field(&format!("rule-days:{pid}:{i}"), &days, "", Rc::new(move |v, text, cx| {
                    if let Ok(d) = text.trim().parse::<f64>().map(|d| d.round())
                        && d >= 0.
                        && r3["matcher"]["days"].as_f64() != Some(d)
                    {
                        let mut next = r3.clone();
                        next["matcher"]["days"] = json!(d as u64);
                        s3(v, next, cx);
                    }
                }), None, window, cx);
                what = what.child(div().flex().flex_none().w(px(60.)).child(input_box(ElementId::Name(format!("rule-days-{n}").into()), input, false, window, cx).h(px(24.))));
            }
            what = what.child("days").child(hint("in the loaded commits"));
        }
        "commitsBy" => {
            let who = rule["matcher"]["author"]["who"].as_str().unwrap_or_default().to_string();
            let person = rule["matcher"]["author"]["kind"] == "person";
            if read_only {
                what = what.child(if person { who } else { "me".to_string() });
            } else {
                let (weak, s3, r3, w3) = (cx.weak_entity(), set.clone(), rule.clone(), who.clone());
                let on_change: OnSelect<String> = Rc::new(move |k, _, cx| {
                    let mut next = r3.clone();
                    next["matcher"]["author"] = if k == "me" { json!({ "kind": "me" }) } else { json!({ "kind": "person", "who": w3 }) };
                    let s3 = s3.clone();
                    weak.update(cx, |v, cx| s3(v, next, cx)).ok();
                });
                let options: Vec<(String, SharedString, Option<SharedString>)> = vec![("me".to_string(), "me".into(), None), ("person".to_string(), "someone".into(), None)];
                what = what.child(div().flex().flex_none().w(px(100.)).child(controls::select(format!("rule-who:{n}"), if person { "person" } else { "me" }.to_string(), Rc::new(options), None, on_change, cx)));
                if person {
                    let (s3, r3) = (set.clone(), rule.clone());
                    let input = this.settings_field(&format!("rule-person:{pid}:{i}"), &who, "name or email", Rc::new(move |v, text, cx| {
                        let t = text.trim().to_string();
                        if r3["matcher"]["author"]["who"] != t.as_str() {
                            let mut next = r3.clone();
                            next["matcher"]["author"] = json!({ "kind": "person", "who": t });
                            s3(v, next, cx);
                        }
                    }), None, window, cx);
                    what = what.child(input_box(ElementId::Name(format!("rule-person-{n}").into()), input, false, window, cx).h(px(24.)));
                }
            }
            what = what.child(hint("in the loaded commits"));
        }
        _ => {}
    }
    let pid2 = pid.to_string();
    let drag_pid = pid.to_string();
    let grip = if read_only { div().flex_none().size(px(20.)).into_any_element() } else {
        div().id(ElementId::Name(format!("rule-grip-{n}").into())).flex().flex_none().size(px(20.)).items_center().justify_center().rounded(px(3.)).cursor_grab().hover(|d| d.bg(theme::hover_row())).on_mouse_down(MouseButton::Left, cx.listener(move |v, _, _, cx| {
            cx.stop_propagation();
            v.settings_ui.presets.drag = Some(("rule", i));
            let _ = &drag_pid;
        })).child(icon("grip-vertical", 14., theme::text_secondary())).into_any_element()
    };
    crate::ui::anchor(
        format!("bp-rule:{i}"),
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .py(px(4.))
            .child(grip)
            .child(div().flex_none().w(px(14.)).text_size(px(12.)).text_color(theme::text_secondary()).child(n.to_string()))
            .child(action)
            .child(what)
            .child(div().flex_none().min_w(px(66.)).flex().justify_end().text_size(px(12.)).text_color(theme::text_secondary()).child(count_text(count)))
            .child(if read_only { div().flex_none().size(px(20.)).into_any_element() } else {
                crate::ui::anchor(format!("rule-x:{i}"), div().id(ElementId::Name(format!("rule-x-{n}").into())).flex().flex_none().size(px(20.)).items_center().justify_center().rounded(px(3.)).text_color(theme::text_secondary()).cursor_pointer().hover(|d| d.bg(theme::hover_row()).text_color(theme::text_selected())).on_click(cx.listener(move |v, _, _, cx| {
                    let pid2 = pid2.clone();
                    v.preset_edit(&pid2, move |p| {
                        if let Some(l) = p["rules"].as_array_mut() {
                            l.remove(i);
                        }
                    }, cx);
                })).child("✕")).into_any_element()
            }),
    )
    .into_any_element()
}

fn list_row(entry: &PresetEntry, selected: bool, group_index: Option<(&'static str, usize)>, cx: &mut Context<GitVolt>) -> AnyElement {
    let id = entry.id().to_string();
    let custom = id == CUSTOM_ID;
    let (sel_id, e2, tick) = (id.clone(), entry.clone(), entry.in_cycle);
    crate::ui::anchor(
        format!("bp-item:{}", entry.name()),
        div()
            .id(ElementId::Name(format!("bp-item-{id}").into()))
            .flex()
            .items_center()
            .gap(px(6.))
            .min_h(px(26.))
            .pl(px(2.))
            .pr(px(6.))
            .rounded(px(4.))
            .cursor_pointer()
            .when(selected, |d| d.bg(theme::selected_row()))
            .when(!selected, |d| d.hover(|d| d.bg(theme::hover_row())))
            .on_click(cx.listener(move |v, _, _, cx| {
                v.settings_ui.presets.selected = Some(sel_id.clone());
                cx.notify();
            }))
            .child(match group_index {
                Some((g, i)) => div().id(ElementId::Name(format!("bp-grip-{id}").into())).flex().flex_none().size(px(20.)).items_center().justify_center().rounded(px(3.)).cursor_grab().hover(|d| d.bg(theme::hover_row())).on_mouse_down(MouseButton::Left, cx.listener(move |v, _, _, cx| {
                    cx.stop_propagation();
                    v.settings_ui.presets.drag = Some((g, i));
                })).child(icon("grip-vertical", 14., theme::text_secondary())).into_any_element(),
                None => div().flex_none().size(px(20.)).into_any_element(),
            })
            .child(div().flex_none().size(px(9.)).rounded_full().bg(preset_colour(entry.preset["color"].as_str().unwrap_or("gray"))))
            .child(div().flex_1().min_w_0().truncate().py(px(2.)).when(custom, |d| d.italic()).text_color(if selected { theme::text_selected() } else { theme::text_normal() }).child(entry.name().to_string()))
            .when(entry.read_only, |d| d.child(crate::ui::tooltip::tip("Built in: Duplicate it to change it", icon("lock", 11., theme::text_secondary()))))
            .child(crate::ui::anchor(format!("bp-tick:{}", entry.name()), div().id(ElementId::Name(format!("bp-tick-{id}").into())).flex_none().on_click(cx.listener(move |v, _, _, cx| {
                cx.stop_propagation();
                v.preset_set_cycle(&e2, !tick, cx);
            })).child(controls::check_box(tick, 13.).mt_0()))),
    )
    .into_any_element()
}

pub fn body(this: &mut GitVolt, window: &mut Window, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    let repo = this.repo_doc();
    let has_repo = repo.is_object();
    let entries = preset_list(&this.settings, &repo);
    let selected = this.settings_ui.presets.selected.clone();
    let entry = selected.as_deref().and_then(|s| entries.iter().find(|e| e.id() == s)).unwrap_or_else(|| current_preset(&entries, &repo)).clone();
    let p = entry.preset.clone();
    let pid = entry.id().to_string();
    let read_only = entry.read_only;
    let repo_name = this.tabs.get(this.active).filter(|t| !t.is_open_tab()).map(|t| t.name.to_string());
    if has_repo {
        this.ask_preset_preview(&entry, cx);
    }
    let preview = this.settings_ui.presets.preview.clone().filter(|x| x.0 == pid).map(|x| x.1);
    // The left column.
    let mut side = div().flex().flex_col().flex_none().gap(px(10.)).w(px(236.)).px(px(8.)).py(px(12.)).border_r_1().border_color(theme::section_border());
    let group = |title: &str, detail: Option<&str>, head: bool| div().flex().justify_between().items_baseline().mb(px(4.)).px(px(6.)).font_features(crate::glyphs::spacing(0.06)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_secondary()).child(div().flex().child(title.to_uppercase()).children(detail.map(|d| div().font_weight(FontWeight::NORMAL).child(format!(" · {d}"))))).when(head, |d| d.child(div().font_features(crate::glyphs::spacing(0.)).text_size(px(10.)).font_weight(FontWeight::NORMAL).child("in the cycle")));
    for (g, title) in [("builtin", "Built in"), ("all", "All repositories"), ("repo", "This repository")] {
        if g == "repo" && !has_repo {
            continue;
        }
        let list: Vec<&PresetEntry> = entries.iter().filter(|e| e.group == g).collect();
        let mut col = div().child(group(title, if g == "repo" { repo_name.as_deref() } else { None }, g == "builtin"));
        let mut rows = div().flex().flex_col();
        for (i, e) in list.iter().enumerate() {
            let draggable = e.id() != CUSTOM_ID;
            rows = rows.child(list_row(e, e.id() == pid, draggable.then_some((g, i)), cx));
        }
        if list.is_empty() {
            rows = rows.child(div().px(px(6.)).py(px(2.)).text_size(px(12.)).text_color(theme::text_secondary()).child("None yet"));
        }
        col = col.child(rows);
        side = side.child(col);
    }
    side = side
        .child(div().flex().gap(px(14.)).px(px(6.)).child(crate::ui::anchor("bp-new", link("bp-new", "+ New preset").on_click(cx.listener(|v, _, _, cx| v.preset_new(cx))))).child(crate::ui::anchor("bp-paste", link("bp-paste", "Paste preset").on_click(cx.listener(|v, _, _, cx| {
            let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
            v.paste_preset(&text, cx);
        })))))
        .child(div().mt_auto().px(px(6.)).pt(px(10.)).border_t_1().border_color(theme::section_border()).text_size(px(11.)).line_height(relative(1.45)).text_color(theme::text_secondary()).child("Drag to set the order. The button's click steps through the ticked presets; the ▾ menu always lists them all."));
    // The editor.
    let mut editor = div().flex().flex_col().flex_1().min_w_0().gap(px(14.)).px(px(16.)).pt(px(14.)).pb(px(16.));
    let label = |t: &str| div().mb(px(4.)).text_size(px(12.)).text_color(theme::text_secondary()).child(t.to_string());
    let name_field: AnyElement = if read_only {
        div().flex().items_center().h(px(28.)).px(px(8.)).w_full().rounded(px(4.)).bg(theme::app_bg()).border_1().border_color(theme::section_border()).text_color(theme::text_selected()).child(entry.name().to_string()).into_any_element()
    } else {
        let id2 = pid.clone();
        let input = this.settings_field(&format!("bp-name:{pid}"), entry.name(), "", Rc::new(move |v, text, cx| {
            let name = text.trim().to_string();
            let id = id2.clone();
            if !name.is_empty() && preset_list(&v.settings, &v.repo_doc()).iter().find(|e| e.id() == id).is_some_and(|e| e.name() != name) {
                v.preset_edit(&id, move |p| p["name"] = json!(name), cx);
            }
        }), None, window, cx);
        div().flex().w(px(220.)).child(input_box("bp-name", input, false, window, cx).h(px(26.))).into_any_element()
    };
    let scope_el: AnyElement = if read_only {
        div().h(px(26.)).flex().items_center().text_color(theme::text_secondary()).child("Built in, everywhere").into_any_element()
    } else {
        let mut seg = div().flex().rounded(px(4.)).border_1().border_color(theme::section_border()).overflow_hidden();
        let on_scope = p["scope"].as_str().unwrap_or("all").to_string();
        let mut choices: Vec<(&'static str, String)> = vec![("all", "All repositories".to_string())];
        if let Some(n) = &repo_name {
            choices.push(("repo", format!("{n} only")));
        }
        for (s, text) in choices {
            let (on, id2) = (on_scope == s, pid.clone());
            seg = seg.child(crate::ui::anchor(format!("bp-scope:{s}"), div().id(ElementId::Name(format!("bp-scope-{s}").into())).px(px(8.)).py(px(3.)).when(on, |d| d.bg(theme::selected_row()).text_color(theme::text_selected())).when(!on, |d| d.cursor_pointer().hover(|d| d.bg(theme::hover_row()))).on_click(cx.listener(move |v, _, _, cx| {
                if !on {
                    v.preset_scope(&id2, s, cx);
                }
            })).child(text)));
        }
        seg.into_any_element()
    };
    editor = editor.child(div().flex().flex_wrap().items_end().gap(px(16.)).child(div().flex_1().min_w(px(180.)).child(label("Name")).child(name_field)).child(div().child(label("Available in")).child(scope_el)));
    let mut colours = div().flex().gap(px(8.));
    for c in GROUP_COLORS {
        let (on, id2) = (p["color"] == c, pid.clone());
        colours = colours.child(crate::ui::tooltip::tip(c, crate::ui::anchor(format!("bp-colour:{c}"), div().id(ElementId::Name(format!("bp-colour-{c}").into())).relative().size(px(20.)).rounded_full().bg(preset_colour(c)).border_2().border_color(if on { theme::text_selected() } else { preset_colour(c) }).child(div().absolute().top_0().left_0().size_full().rounded_full().border_1().border_color(theme::panel_bg())).when(!read_only, |d| d.cursor_pointer()).on_click(cx.listener(move |v, _, _, cx| {
            if !read_only && !on {
                v.preset_edit(&id2, move |p| p["color"] = json!(c), cx);
            }
        })))));
    }
    editor = editor.child(div().child(label("Colour")).child(colours));
    // Rules.
    let rules = p["rules"].as_array().cloned().unwrap_or_default();
    let counts = preview.as_ref().and_then(|v| v["rules"].as_array()).filter(|c| c.len() == rules.len() + 1).cloned();
    let mut rule_list = div().flex().flex_col().border_1().border_color(theme::section_border()).rounded(px(6.));
    for (i, r) in rules.iter().enumerate() {
        rule_list = rule_list.child(rule_row(this, &pid, i, r, counts.as_ref().and_then(|c| c[i].as_u64()), read_only, window, cx));
    }
    let id3 = pid.clone();
    rule_list = rule_list.child(div().flex().items_center().gap(px(8.)).min_h(px(30.)).pl(px(4.)).pr(px(8.)).py(px(4.)).when(!rules.is_empty(), |d| d.border_t_1().border_color(theme::section_border())).child(div().flex_none().size(px(20.))).child(div().flex_none().w(px(14.))).child(action_choice("rule:else".into(), p["otherwise"].as_str().unwrap_or("show"), read_only, Rc::new(move |v, a, cx| {
        let (id, a) = (id3.clone(), a.to_string());
        v.preset_edit(&id, move |p| p["otherwise"] = json!(a), cx);
    }), cx)).child(div().flex_1().min_w_0().child("Everything else")).child(div().flex_none().min_w(px(66.)).flex().justify_end().text_size(px(12.)).text_color(theme::text_secondary()).child(count_text(counts.as_ref().and_then(|c| c[rules.len()].as_u64())))).child(div().flex_none().size(px(20.))));
    let mut rules_section = div().child(section_head("Rules", Some("top to bottom; the first that matches a branch decides"))).child(rule_list.on_mouse_move(cx.listener({
        let pid = pid.clone();
        move |v, e: &MouseMoveEvent, _, cx| v.bp_rule_drag(&pid, e, cx)
    })).on_mouse_up(MouseButton::Left, cx.listener(|v, _, _, _| v.settings_ui.presets.drag = None)));
    if !read_only {
        let id4 = pid.clone();
        rules_section = rules_section.child(crate::ui::anchor("bp-add-rule", link("bp-add-rule", "+ Add rule").on_click(cx.listener(move |v, _, _, cx| {
            let id = id4.clone();
            v.preset_edit(&id, |p| {
                if !p["rules"].is_array() {
                    p["rules"] = json!([]);
                }
                p["rules"].as_array_mut().expect("just made").push(json!({ "action": "hide", "matcher": { "kind": "nameMatches", "patterns": "" } }));
            }, cx);
        }))));
    }
    editor = editor.child(rules_section);
    // Picked by hand.
    if let (false, Some(name)) = (read_only, repo_name.as_deref()) {
        let picks = repo["branchPicks"][&pid].clone();
        let list = |k: &str| picks[k].as_array().map_or(Vec::new(), |a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect::<Vec<_>>());
        let mut section = div().child(section_head("Picked by hand", Some("alt+click a branch or tag chip, or a sidebar folder; these come before the rules")));
        let mut any = false;
        for (title, branch, tag) in [("Always hidden:", "hidden", "hiddenTags"), ("Always shown:", "shown", "shownTags")] {
            let (b, t) = (list(branch), list(tag));
            if b.is_empty() && t.is_empty() {
                continue;
            }
            any = true;
            let mut chips = div().flex().flex_wrap().items_center().gap(px(6.)).mb(px(4.)).child(hint(title));
            for (which, names, is_tag) in [(branch, b, false), (tag, t, true)] {
                for n in names {
                    let (id5, n2) = (pid.clone(), n.clone());
                    chips = chips.child(div().flex().items_center().gap(px(4.)).px(px(6.)).py(px(1.)).text_size(px(12.)).rounded(px(3.)).bg(theme::app_bg()).border_1().border_color(theme::section_border()).when(is_tag, |d| d.child(icon("tag", 11., theme::text_secondary()))).child(n.clone()).child(div().id(ElementId::Name(format!("bp-forget-{which}-{n}").into())).text_size(px(10.)).text_color(theme::text_secondary()).cursor_pointer().on_click(cx.listener(move |v, _, _, cx| {
                        let Some(path) = v.tabs.get(v.active).map(|t| t.repo_path.clone()) else { return };
                        let (id, n) = (id5.clone(), n2.clone());
                        v.update_repo(&path, move |r| {
                            if let Some(l) = r["branchPicks"][&id][which].as_array_mut() {
                                l.retain(|x| x != n.as_str());
                            }
                        }, cx);
                        v.reload_active(cx);
                    })).child("✕")));
                }
            }
            section = section.child(chips);
        }
        if !any {
            section = section.child(hint(&format!("None in {name}.")));
        }
        editor = editor.child(section);
    }
    // Options.
    let opt = |id: &'static str, key: &'static str, label: &'static str, desc: Option<&'static str>, on: bool, off: bool, pid: String, cx: &mut Context<GitVolt>| {
        let weak = cx.weak_entity();
        crate::ui::anchor(format!("bp-opt:{key}"), controls::switch(id, on, label, desc.map(SharedString::from), off, move |v, _, cx| {
            let pid = pid.clone();
            weak.update(cx, |t, cx| t.preset_edit(&pid, move |p| p[key] = json!(v), cx)).ok();
        }))
    };
    let ghost = p["ghostLines"] == true;
    editor = editor.child(div().child(section_head("Options", None)).child(div().flex().flex_col().gap(px(6.)).child(opt("bp-fold", "foldMerged", "Fold branches merged into the trunk", Some("into their merge commit; hover it to see them"), p["foldMerged"] == true, read_only, pid.clone(), cx)).child(opt("bp-ghost", "ghostLines", "Mark where hidden branches fork", Some("those the rules hide; hover the mark to see them"), ghost, read_only, pid.clone(), cx)).child(div().pl(px(40.)).child(opt("bp-ghost-picks", "ghostPicks", "Also for branches and tags hidden by hand", None, p["ghostPicks"] == true, read_only || !ghost, pid.clone(), cx)))));
    // The summary.
    let num = |k: &str| preview.as_ref().and_then(|v| v[k].as_u64()).map_or("–".to_string(), |n| n.to_string());
    editor = editor.child(match &repo_name {
        Some(n) => div().px(px(10.)).py(px(8.)).bg(theme::panel_bg1()).border_1().border_color(theme::section_border()).rounded(px(4.)).text_size(px(12.)).text_color(theme::text_secondary()).child(format!("In {n} now: {} shown · {} collapsed · {} hidden · {} merged branches folded", num("shown"), num("collapsed"), num("hidden"), num("folded"))),
        None => hint("Open a repository to see what this preset decides in it."),
    });
    let (e2, copy, del_id, del_name, del_key) = (entry.clone(), p.clone(), pid.clone(), entry.name().to_string(), "bp-delete");
    editor = editor.child(
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(crate::ui::anchor("bp-duplicate", crate::settings::frame::plain_button("bp-duplicate", "Duplicate", true).on_click(cx.listener(move |v, _, _, cx| v.preset_duplicate(&e2, cx)))))
            .child(crate::ui::anchor("bp-copy", crate::settings::frame::plain_button("bp-copy", "Copy as JSON", true).on_click(cx.listener(move |_, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(preset_to_json(&copy)));
                toast::show(format!("Copied {}", copy["name"].as_str().unwrap_or_default()), cx);
            }))))
            .child(div().flex_1())
            .when(!read_only, |d| {
                d.child(crate::ui::anchor(del_key, crate::settings::frame::plain_button("bp-delete", "Delete preset", true).text_color(theme::status_deleted()).on_click(cx.listener(move |_, _, window, cx| {
                    let (weak, id) = (cx.weak_entity(), del_id.clone());
                    let req = arm::ArmRequest { body: "Repositories on it go back to All.".into(), ..arm::ArmRequest::danger(format!("Delete {del_name}"), format!("Delete {del_name}?"), format!("Delete {del_name}")) };
                    arm::request(req, arm::Origin::control(del_key), move |answer, _, cx| {
                        if answer.ok {
                            let id = id.clone();
                            weak.update(cx, |v, cx| v.preset_delete(&id, cx)).ok();
                        }
                    }, window, cx);
                }))))
            }),
    );
    let page = div().flex().mt(px(16.)).bg(theme::panel_bg()).border_1().border_color(theme::section_border()).rounded(px(6.)).overflow_hidden().child(side.on_mouse_move(cx.listener(move |v, e: &MouseMoveEvent, _, cx| v.bp_list_drag(e, cx))).on_mouse_up(MouseButton::Left, cx.listener(|v, _, _, _| v.settings_ui.presets.drag = None))).child(editor);
    vec![page.into_any_element()]
}

impl GitVolt {
    /// "Available in": all repositories, or the open one only, at the end of the list it goes to.
    fn preset_scope(&mut self, id: &str, scope: &str, cx: &mut Context<Self>) {
        let (mut settings, mut repo) = (self.settings.clone(), self.repo_doc());
        if home_of(&settings, &repo, id) == Some("all") && scope == "repo" && repo.is_object() {
            let Some(l) = settings["branchPresets"].as_array_mut() else { return };
            let Some(at) = l.iter().position(|p| p["id"] == id) else { return };
            let mut p = l.remove(at);
            p["scope"] = json!("repo");
            push_to(&mut repo, p);
            return self.presets_apply(settings, repo, cx);
        }
        let scope = scope.to_string();
        self.preset_edit(id, move |p| p["scope"] = json!(scope), cx);
    }

    fn bp_list_drag(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some((g, from)) = self.settings_ui.presets.drag.filter(|d| d.0 != "rule") else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            self.settings_ui.presets.drag = None;
            return;
        }
        let entries = preset_list(&self.settings, &self.repo_doc());
        let group: Vec<String> = entries.iter().filter(|x| x.group == g && x.id() != CUSTOM_ID).map(|x| x.name().to_string()).collect();
        let ui = crate::ui::Ui::get(cx);
        let Some(to) = (0..group.len()).find(|i| *i != from && ui.anchor(&format!("bp-item:{}", group[*i])).is_some_and(|b| e.position.y >= b.top() && e.position.y < b.bottom())) else { return };
        self.settings_ui.presets.drag = Some((g, to));
        self.preset_move(g, from, to, cx);
    }

    fn bp_rule_drag(&mut self, pid: &str, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(("rule", from)) = self.settings_ui.presets.drag else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            self.settings_ui.presets.drag = None;
            return;
        }
        let ui = crate::ui::Ui::get(cx);
        let n = preset_list(&self.settings, &self.repo_doc()).iter().find(|x| x.id() == pid).and_then(|x| x.preset["rules"].as_array().map(Vec::len)).unwrap_or(0);
        let Some(to) = (0..n).find(|i| *i != from && ui.anchor(&format!("bp-rule:{i}")).is_some_and(|b| e.position.y >= b.top() && e.position.y < b.bottom())) else { return };
        self.settings_ui.presets.drag = Some(("rule", to));
        self.preset_edit(pid, move |p| {
            if let Some(l) = p["rules"].as_array_mut() {
                let r = l.remove(from);
                l.insert(to, r);
            }
        }, cx);
    }

    /// "Edit presets…": Settings › Branch presets, on the shown tab's current preset.
    pub fn open_presets_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo_doc();
        let list = preset_list(&self.settings, &repo);
        self.settings_ui.presets.selected = Some(current_preset(&list, &repo).id().to_string());
        self.settings_ui.page = "branch-presets".into();
        self.show_settings(None, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_shared_preset_goes_out_and_comes_back_checked() {
        let p = json!({ "name": "Mine", "color": "blue", "rules": [{ "action": "hide", "matcher": { "kind": "remote" } }], "otherwise": "show", "foldMerged": true, "ghostLines": false, "ghostPicks": false });
        let back = parse_preset(&preset_to_json(&p)).unwrap();
        assert_eq!(back["name"], "Mine");
        assert_eq!(back["foldMerged"], true);
        assert_eq!(parse_preset("{\"gitvolt\":\"branch-preset\",\"version\":1,\"name\":\"x\",\"color\":\"nope\",\"rules\":[],\"otherwise\":\"show\"}"), None);
        assert_eq!(parse_preset("{\"gitvolt\":\"branch-preset\",\"version\":1,\"name\":\"x\",\"color\":\"red\",\"rules\":[{\"action\":\"x\",\"matcher\":{\"kind\":\"remote\"}}],\"otherwise\":\"show\"}"), None);
        assert_eq!(parse_preset("nope"), None);
    }

    #[test]
    fn a_home_goes_by_where_a_preset_lives() {
        let s = json!({ "branchPresets": [{ "id": "a" }] });
        let r = json!({ "branchPresets": [{ "id": "b" }], "customPreset": { "id": "custom" } });
        assert_eq!(home_of(&s, &r, "a"), Some("all"));
        assert_eq!(home_of(&s, &r, "b"), Some("repo"));
        assert_eq!(home_of(&s, &r, "zzz"), None);
    }
}
