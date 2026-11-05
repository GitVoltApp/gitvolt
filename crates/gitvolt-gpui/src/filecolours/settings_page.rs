//! Settings › File colours (`fileColours/settings/`): the scopes in order, each with its colour and patterns; the
//! counts over the open commit's or WIP files and the worktree's; try a path; share as JSON.

use crate::app::{GitVolt, icon};
use crate::settings::frame::input_box;
use crate::settings::pages::Def;
use crate::settings::themes::{parse_share, to_share};
use crate::theme::{self, Col};
use crate::ui::menu::{self, MenuRow, OpenOptions};
use crate::ui::{arm, colour, toast};
use gitvolt_core::api::Request;
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::rc::Rc;

#[derive(Default)]
pub struct PageState {
    pub open: Option<String>,
    /// `(scope id, pattern index or None for a new one)`: the chip being edited.
    pub chip: Option<(String, Option<usize>)>,
    /// A scope being dragged: `(index, moved)`.
    pub drag: Option<(usize, bool)>,
    /// The worktree's tracked files, and the worktree they're of.
    pub repo_files: Option<(String, Rc<Vec<String>>)>,
    pub loading_files: bool,
}

const STARTERS: [(&str, &str, &[&str], bool); 3] = [("Tests", "green", &["**/tests/**", "**/test/**", "**/__tests__/**", "*.test.*", "*.spec.*", "*_test.*", "*.snap"], false), ("Generated", "gray", &["**/generated/**", "*.generated.*", "*.pb.*", "*.min.js", "*.lock", "package-lock.json"], true), ("Docs", "blue", &["docs/**", "*.md", "*.rst", "*.adoc"], false)];

pub fn defs() -> Vec<Def> {
    Vec::new()
}

/// `crypto.randomUUID`'s place: an id only this list reads.
fn new_scope_id() -> String {
    crate::settings::themes::new_custom_id().replace("custom-", "scope-") + &format!("{:x}", crate::format::now() as u64)
}

/// `scopesToJson`.
pub fn scopes_to_json(scopes: &[Value]) -> String {
    let list: Vec<Value> = scopes.iter().map(|s| json!({ "name": s["name"], "tint": s["tint"], "patterns": s["patterns"], "startCollapsed": s["startCollapsed"] })).collect();
    to_share("file-colours", 1, &[("scopes", json!(list))])
}

/// `parseScopesJson`: `None` for not JSON, another kind, a newer version, no scopes array, or too big.
pub fn parse_scopes(text: &str) -> Option<Vec<Value>> {
    let (body, _) = parse_share(text, "file-colours", 1)?;
    let list = body.get("scopes")?.as_array()?;
    if list.len() > 200 {
        return None;
    }
    let mut out = Vec::new();
    for r in list {
        let name = r["name"].as_str().unwrap_or_default().trim().to_string();
        let patterns: Vec<String> = r["patterns"].as_array().map_or(Vec::new(), |p| p.iter().filter_map(|x| x.as_str()).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).take(200).collect());
        if name.is_empty() || patterns.is_empty() {
            continue;
        }
        let tint = r["tint"].as_str().filter(|t| super::tint_hex(t).is_some()).unwrap_or("gray");
        out.push(json!({ "name": name, "tint": tint, "patterns": patterns, "startCollapsed": r["startCollapsed"] == true }));
    }
    Some(out)
}

/// `appendScopes`: new ids; a taken name takes a suffix.
pub fn append_scopes(scopes: &[Value], incoming: &[Value]) -> Vec<Value> {
    let mut out = scopes.to_vec();
    for s in incoming {
        let taken: Vec<String> = out.iter().filter_map(|x| x["name"].as_str().map(str::to_string)).collect();
        let mut v = s.clone();
        v["name"] = json!(crate::settings::themes::unique_name(s["name"].as_str().unwrap_or_default(), &taken));
        v["id"] = json!(new_scope_id());
        out.push(v);
    }
    out
}

/// `repoLabel`: the repository's folder name.
pub fn repo_label(common_dir: &str) -> String {
    let mut parts: Vec<&str> = common_dir.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    let mut last = parts.pop().unwrap_or_default();
    if last == ".git" {
        last = parts.pop().unwrap_or_default();
    }
    let trimmed = last.strip_suffix(".git").unwrap_or(last);
    if trimmed.is_empty() { common_dir.to_string() } else { trimmed.to_string() }
}

fn plural(n: usize) -> String {
    format!("{n} scope{}", if n == 1 { "" } else { "s" })
}

impl GitVolt {
    fn scopes_dir(&self) -> Option<String> {
        self.tabs.get(self.active).filter(|t| !t.is_open_tab()).and_then(|t| t.info["commonDir"].as_str().map(str::to_string))
    }

    fn scopes(&self) -> Vec<Value> {
        self.scopes_dir().and_then(|d| self.profile["fileColours"][&d].as_array().cloned()).unwrap_or_default()
    }

    fn set_scopes(&mut self, next: Vec<Value>, cx: &mut Context<Self>) {
        let Some(dir) = self.scopes_dir() else { return };
        self.update_profile(
            move |p| {
                if !p["fileColours"].is_object() {
                    p["fileColours"] = json!({});
                }
                p["fileColours"][dir] = json!(next);
            },
            cx,
        );
    }

    fn change_scope(&mut self, id: &str, change: impl FnOnce(&mut Value), cx: &mut Context<Self>) {
        let mut list = self.scopes();
        if let Some(s) = list.iter_mut().find(|s| s["id"] == id) {
            change(s);
        }
        self.set_scopes(list, cx);
    }

    /// `pasteScopes`: appends the scopes in `text`; anything else only says so.
    pub fn paste_file_colours(&mut self, text: &str, cx: &mut Context<Self>) {
        match parse_scopes(text) {
            Some(incoming) if !incoming.is_empty() => {
                let n = incoming.len();
                let next = append_scopes(&self.scopes(), &incoming);
                self.set_scopes(next, cx);
                toast::show(format!("Added {}", plural(n)), cx);
            }
            _ => toast::show("That isn't GitVolt file colours", cx),
        }
    }

    /// The worktree's tracked files, once per worktree while the page shows.
    fn read_repo_files(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active).filter(|t| !t.is_open_tab()) else { return };
        let (repo, worktree) = (tab.id, tab.worktree.clone());
        let page = &mut self.settings_ui.file_colours;
        if cfg!(test) || page.loading_files || page.repo_files.as_ref().is_some_and(|f| f.0 == worktree) {
            return;
        }
        page.loading_files = true;
        let api = self.api.clone();
        let wt = worktree.clone();
        let read = self.rt.spawn(async move { api.dispatch(Request::WorktreeFiles { repo, worktree: wt }).await });
        cx.spawn(async move |this, cx| {
            let answer = read.await;
            this.update(cx, |v, cx| {
                let page = &mut v.settings_ui.file_colours;
                page.loading_files = false;
                if let Ok(Ok(list)) = answer {
                    let files = list.as_array().map_or(Vec::new(), |l| l.iter().filter_map(|x| x.as_str().map(str::to_string)).collect());
                    page.repo_files = Some((worktree, Rc::new(files)));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn scope_drag(&mut self, y: Pixels, pressed: bool, cx: &mut Context<Self>) {
        let Some((from, _)) = self.settings_ui.file_colours.drag else { return };
        if !pressed {
            self.settings_ui.file_colours.drag = None;
            return;
        }
        let n = self.scopes().len();
        let ui = crate::ui::Ui::get(cx);
        let Some(to) = (0..n).find(|i| *i != from && ui.anchor(&format!("fc-row:{i}")).is_some_and(|b| y >= b.top() && y < b.bottom())) else { return };
        let mut list = self.scopes();
        let s = list.remove(from);
        list.insert(to, s);
        self.settings_ui.file_colours.drag = Some((to, true));
        self.set_scopes(list, cx);
    }
}

fn fc_button(id: impl Into<ElementId>, label: impl Into<SharedString>, enabled: bool) -> Stateful<Div> {
    div().id(id).flex().flex_none().px(px(10.)).py(px(4.)).rounded(px(4.)).border_1().border_color(theme::section_border()).bg(theme::panel_bg1()).text_color(theme::text_normal()).when(!enabled, |d| d.opacity(0.5)).when(enabled, |d| d.cursor_pointer().hover(|d| d.bg(theme::hover_row()))).child(label.into())
}

fn count_text(changed: Option<usize>, repo: Option<usize>) -> String {
    match (changed, repo) {
        (Some(c), Some(r)) => format!("{c} changed · {r} in repo"),
        _ => String::new(),
    }
}

fn chips(this: &mut GitVolt, scope: &Value, open: bool, window: &Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let id = scope["id"].as_str().unwrap_or_default().to_string();
    let patterns: Vec<String> = scope["patterns"].as_array().map_or(Vec::new(), |p| p.iter().filter_map(|x| x.as_str().map(str::to_string)).collect());
    let at = this.settings_ui.file_colours.chip.clone().filter(|c| c.0 == id).map(|c| c.1);
    let mut row = div().flex().flex_1().min_w_0().flex_wrap().items_center().gap(px(6.));
    let edit_field = |this: &mut GitVolt, i: Option<usize>, value: &str, window: &Window, cx: &mut Context<GitVolt>| {
        let key = format!("fc-chip:{id}:{i:?}");
        let (sid, plist) = (id.clone(), patterns.clone());
        let input = this.settings_field(&key, value, "", Rc::new(move |v, text, cx| {
            let t = text.trim().to_string();
            v.settings_ui.file_colours.chip = None;
            let mut list = plist.clone();
            match i {
                None if !t.is_empty() && !list.contains(&t) => list.push(t),
                None => return cx.notify(),
                Some(n) if t.is_empty() => {
                    list.remove(n);
                }
                Some(n) if t != list[n] && !list.contains(&t) => list[n] = t,
                Some(_) => return cx.notify(),
            }
            v.change_scope(&sid, move |s| s["patterns"] = json!(list), cx);
        }), None, window, cx);
        let f = input.read(cx).focus.clone();
        if !f.contains_focused(window, cx) && this.focus_next_frame.is_none() {
            this.focus_next_frame = Some(f);
        }
        div().flex().flex_none().w(px(140.)).child(input_box(ElementId::Name(key.into()), input, false, window, cx).h(px(22.)))
    };
    for (i, p) in patterns.iter().enumerate() {
        if open && at == Some(Some(i)) {
            row = row.child(edit_field(this, Some(i), p, window, cx));
            continue;
        }
        let valid = super::compile(p).is_some();
        let (sid, sid2, plist) = (id.clone(), id.clone(), patterns.clone());
        let chip = div().id(ElementId::Name(format!("fc-chip-{id}-{i}").into())).flex().flex_none().items_center().gap(px(4.)).px(px(6.)).py(px(1.)).text_size(px(12.)).rounded(px(3.)).bg(theme::app_bg()).border_1().border_color(if valid { theme::section_border() } else { theme::status_deleted() }).text_color(theme::text_normal());
        row = row.child(crate::ui::tooltip::tip(if valid { "" } else { "Not a valid pattern" }, if open {
            chip.child(div().id(ElementId::Name(format!("fc-chip-text-{id}-{i}").into())).cursor_pointer().on_click(cx.listener(move |v, _, _, cx| {
                v.settings_ui.file_colours.chip = Some((sid.clone(), Some(i)));
                cx.notify();
            })).child(p.clone())).child(div().id(ElementId::Name(format!("fc-chip-x-{id}-{i}").into())).text_size(px(10.)).text_color(theme::text_secondary()).cursor_pointer().hover(|d| d.text_color(theme::text_selected())).on_click(cx.listener(move |v, _, _, cx| {
                let mut list = plist.clone();
                list.remove(i);
                v.change_scope(&sid2, move |s| s["patterns"] = json!(list), cx);
            })).child("✕"))
        } else {
            chip.font_family("monospace").child(p.clone())
        }));
    }
    if open {
        if at == Some(None) {
            row = row.child(edit_field(this, None, "", window, cx));
        } else {
            let sid = id.clone();
            row = row.child(crate::ui::anchor(format!("fc-add-pattern:{id}"), div().id(ElementId::Name(format!("fc-add-{id}").into())).text_color(theme::link()).cursor_pointer().on_click(cx.listener(move |v, _, _, cx| {
                v.settings_ui.file_colours.chip = Some((sid.clone(), None));
                cx.notify();
            })).child("+ pattern")));
        }
    }
    row.into_any_element()
}

fn tint_swatches(scope: &Value, cx: &mut Context<GitVolt>) -> AnyElement {
    let id = scope["id"].as_str().unwrap_or_default().to_string();
    let value = scope["tint"].as_str().unwrap_or_default().to_string();
    let mut row = div().flex().items_center().gap(px(6.));
    for (name, hex) in super::TINT_PRESETS {
        let sid = id.clone();
        let on = value == name;
        row = row.child(crate::ui::tooltip::tip(name, crate::ui::anchor(format!("fc-tint:{id}:{name}"), div().id(ElementId::Name(format!("fc-tint-{id}-{name}").into())).size(px(20.)).rounded(px(4.)).border_2().border_color(if on { theme::text_selected() } else { transparent_black() }).bg(Col::of_hex(hex).hsla()).cursor_pointer().on_click(cx.listener(move |v, _, _, cx| {
            let sid = sid.clone();
            v.change_scope(&sid, move |s| s["tint"] = json!(name), cx);
        })))));
    }
    let custom = !super::TINT_PRESETS.iter().any(|(n, _)| *n == value);
    let hex = super::tint_hex(&value).map_or("#8b949e".to_string(), |h| Col::of_hex(h).css());
    let weak = cx.weak_entity();
    let sid = id.clone();
    let swatch = colour::swatch(format!("fc-custom:{id}"), &hex, "Custom colour", false, 20., Rc::new(|_, _, _| {}), Rc::new(move |v, _, cx| {
        let (v, sid) = (v.to_lowercase(), sid.clone());
        weak.update(cx, |t, cx| t.change_scope(&sid, move |s| s["tint"] = json!(v), cx)).ok();
    }), Rc::new(|_, _| {}), cx);
    row.child(div().relative().flex().when(custom, |d| d.rounded(px(4.)).border_2().border_color(theme::text_selected())).child(swatch).when(!custom, |d| d.child(div().absolute().top_0().left_0().size_full().flex().items_center().justify_center().font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).child("+")))).into_any_element()
}

fn scope_row(this: &mut GitVolt, i: usize, scope: &Value, counts: (Option<usize>, Option<usize>), window: &Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let id = scope["id"].as_str().unwrap_or_default().to_string();
    let name = scope["name"].as_str().unwrap_or_default().to_string();
    let open = this.settings_ui.file_colours.open.as_deref() == Some(id.as_str());
    let dot = scope["tint"].as_str().and_then(super::tint_hex).map(|h| Col::of_hex(h).hsla());
    let chips_el = chips(this, scope, open, window, cx);
    let toggle = {
        let id = id.clone();
        cx.listener(move |v: &mut GitVolt, _: &ClickEvent, _: &mut Window, cx: &mut Context<GitVolt>| {
            let page = &mut v.settings_ui.file_colours;
            page.open = if page.open.as_deref() == Some(id.as_str()) { None } else { Some(id.clone()) };
            page.chip = None;
            cx.notify();
        })
    };
    let name_el: AnyElement = if open {
        let key = format!("fc-name:{id}");
        let (sid, was) = (id.clone(), name.clone());
        let input = this.settings_field(&key, &name, "", Rc::new(move |v, text, cx| {
            let n = text.trim().to_string();
            if !n.is_empty() && n != was {
                v.change_scope(&sid, move |s| s["name"] = json!(n), cx);
            }
        }), None, window, cx);
        div().flex().flex_none().w(px(140.)).child(input_box(ElementId::Name(key.into()), input, false, window, cx).h(px(22.))).into_any_element()
    } else {
        crate::ui::anchor(format!("fc-name:{name}"), div().id(ElementId::Name(format!("fc-name-{id}").into())).flex_none().w(px(140.)).truncate().py(px(2.)).text_color(theme::text_selected()).cursor_pointer().on_click(toggle).child(name.clone())).into_any_element()
    };
    let (del_id, del_name) = (id.clone(), name.clone());
    let del_key: SharedString = format!("fc-del:{name}").into();
    let dk = del_key.clone();
    let delete = crate::ui::tooltip::tip(format!("Delete {name}"), crate::ui::anchor(del_key, div().id(ElementId::Name(format!("fc-x-{id}").into())).flex().flex_none().size(px(20.)).items_center().justify_center().rounded(px(3.)).text_color(theme::text_secondary()).cursor_pointer().hover(|d| d.bg(theme::hover_row()).text_color(theme::text_selected())).on_click(cx.listener(move |_, _, window, cx| {
        let (weak, id, name) = (cx.weak_entity(), del_id.clone(), del_name.clone());
        let req = arm::ArmRequest { body: "Its files stop being coloured.".into(), ..arm::ArmRequest::danger(format!("Delete {name}"), format!("Delete {name}?"), format!("Delete {name}")) };
        arm::request(req, arm::Origin::control(dk.clone()), move |answer, _, cx| {
            if answer.ok {
                let id = id.clone();
                weak.update(cx, |v, cx| {
                    let list: Vec<Value> = v.scopes().into_iter().filter(|s| s["id"] != id.as_str()).collect();
                    v.set_scopes(list, cx);
                }).ok();
            }
        }, window, cx);
    })).child("✕")));
    let dragging = this.settings_ui.file_colours.drag.is_some_and(|d| d.0 == i && d.1);
    let collapsed = scope["startCollapsed"] == true;
    let mut row = div().flex().flex_col().when(i > 0, |d| d.border_t_1().border_color(theme::section_border())).when(dragging, |d| d.bg(theme::selected_row())).child(
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(10.))
            .py(px(8.))
            .min_h(px(28.))
            .child(crate::ui::tooltip::tip(format!("Reorder {name}"), div().id(ElementId::Name(format!("fc-handle-{id}").into())).flex().flex_none().size(px(20.)).items_center().justify_center().rounded(px(3.)).cursor_grab().hover(|d| d.bg(theme::hover_row())).on_mouse_down(MouseButton::Left, cx.listener(move |v, _, _, cx| {
                cx.stop_propagation();
                v.settings_ui.file_colours.drag = Some((i, false));
            })).child(icon("grip-vertical", 14., theme::text_secondary()))))
            .child(div().flex_none().size(px(18.)).rounded(px(4.)).when_some(dot, |d, c| d.bg(c)))
            .child(name_el)
            .child(chips_el)
            .when(collapsed, |d| d.child(div().flex_none().text_size(px(11.)).text_color(theme::text_secondary()).child("starts collapsed")))
            .child(div().flex_none().min_w(px(150.)).flex().justify_end().text_size(px(12.)).text_color(theme::text_secondary()).child(count_text(counts.0, counts.1)))
            .child(delete),
    );
    if open {
        let sid = id.clone();
        let tints = tint_swatches(scope, cx);
        row = row.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(14.))
                .pr(px(10.))
                .pb(px(10.))
                .pl(px(58.))
                .child(div().text_color(theme::text_secondary()).child("Colour"))
                .child(tints)
                .child(crate::ui::anchor(format!("fc-collapsed:{id}"), crate::ui::controls::switch(ElementId::Name(format!("fc-collapsed-{id}").into()), collapsed, "Folders start collapsed in tree view", None, false, {
                    let weak = cx.weak_entity();
                    move |on, _, cx| {
                        let sid = sid.clone();
                        weak.update(cx, |v, cx| v.change_scope(&sid, move |s| s["startCollapsed"] = json!(on), cx)).ok();
                    }
                }))),
        );
    }
    crate::ui::anchor(format!("fc-row:{i}"), row).into_any_element()
}

pub fn body(this: &mut GitVolt, window: &mut Window, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    let Some(dir) = this.scopes_dir() else {
        return vec![div().mt(px(16.)).text_color(theme::text_secondary()).child("Open a repository to set its file colours.").into_any_element()];
    };
    this.read_repo_files(cx);
    let scopes = this.scopes();
    let matcher = super::Matcher::new(&json!(scopes));
    let counted = |paths: &[String]| -> Vec<usize> {
        let mut n = vec![0; scopes.len()];
        for p in paths {
            if let Ok(i) = usize::try_from(matcher.scope_of(p))
                && let Some(c) = n.get_mut(i)
            {
                *c += 1;
            }
        }
        n
    };
    let changed: Vec<String> = this.tabs.get(this.active).and_then(|t| t.files.as_ref()).map_or(Vec::new(), |f| f.changes.iter().map(|c| c.path.to_string()).collect());
    let changed_n = counted(&changed);
    let repo_n = this.settings_ui.file_colours.repo_files.as_ref().map(|(_, f)| counted(f));
    let mut page = div().flex().flex_col().gap(px(12.)).mt(px(16.));
    if scopes.is_empty() {
        let mut row = div().flex().items_center().gap(px(8.)).p(px(12.)).bg(theme::panel_bg()).border_1().border_color(theme::section_border()).rounded(px(6.)).child(div().text_color(theme::text_secondary()).child("No scopes yet. Start from one:"));
        for (i, (name, tint, patterns, collapsed)) in STARTERS.iter().enumerate() {
            row = row.child(crate::ui::anchor(format!("fc-starter:{name}"), fc_button(("fc-starter", i), *name, true).on_click(cx.listener(move |v, _, _, cx| {
                let mut list = v.scopes();
                list.push(json!({ "id": new_scope_id(), "name": name, "tint": tint, "patterns": patterns, "startCollapsed": collapsed }));
                v.set_scopes(list, cx);
            }))));
        }
        page = page.child(row);
    } else {
        let mut list = div().flex().flex_col().bg(theme::panel_bg()).border_1().border_color(theme::section_border()).rounded(px(6.));
        for (i, s) in scopes.iter().enumerate() {
            list = list.child(scope_row(this, i, s, (Some(changed_n[i]), repo_n.as_ref().map(|r| r[i])), window, cx));
        }
        page = page.child(list.on_mouse_move(cx.listener(|v, e: &MouseMoveEvent, _, cx| v.scope_drag(e.position.y, e.pressed_button == Some(MouseButton::Left), cx))).on_mouse_up(MouseButton::Left, cx.listener(|v, _, _, _| v.settings_ui.file_colours.drag = None)));
    }
    page = page.child(crate::ui::anchor("fc-new", div().id("fc-new").flex_none().w_auto().py(px(2.)).text_color(theme::link()).cursor_pointer().on_click(cx.listener(|v, _, _, cx| {
        let list = v.scopes();
        let tint = super::next_tint(&list);
        let taken: Vec<String> = list.iter().filter_map(|s| s["name"].as_str().map(str::to_string)).collect();
        let id = new_scope_id();
        let mut next = list;
        next.push(json!({ "id": id, "name": crate::settings::themes::unique_name("New scope", &taken), "tint": tint, "patterns": [], "startCollapsed": false }));
        v.settings_ui.file_colours.open = Some(id);
        v.set_scopes(next, cx);
    })).child("+ New scope")));
    // Try a path.
    let weak = cx.weak_entity();
    let try_input = this.settings_field("fc-try", "", "Try a path, e.g. src/api/user.test.ts", Rc::new(|_, _, _| {}), Some(Rc::new(|_, _, cx| cx.notify())), window, cx);
    let _ = weak;
    let path = try_input.read(cx).text().trim().to_string();
    let answer = if path.is_empty() {
        String::new()
    } else {
        match matcher.explain(&path).and_then(|(i, pat)| Some((scopes.get(i)?, pat))) {
            Some((s, pat)) => {
                let at = s["patterns"].as_array().and_then(|p| p.iter().position(|x| x.as_str() == Some(pat.as_str()))).map_or(0, |n| n + 1);
                format!("→ {} (pattern {at}, {pat})", s["name"].as_str().unwrap_or_default())
            }
            None => "→ No scope".to_string(),
        }
    };
    page = page.child(div().child(input_box("fc-try-box", try_input, false, window, cx)).child(div().min_h(px(18.)).pt(px(4.)).text_size(px(12.)).text_color(theme::text_secondary()).child(answer)));
    // Share.
    let others: Vec<(String, usize, Vec<Value>)> = this.profile["fileColours"].as_object().map_or(Vec::new(), |m| m.iter().filter(|(d, l)| **d != dir && l.as_array().is_some_and(|x| !x.is_empty())).map(|(d, l)| (d.clone(), l.as_array().map_or(0, Vec::len), l.as_array().cloned().unwrap_or_default())).collect());
    let has = !scopes.is_empty();
    let copy_list = scopes.clone();
    let others_empty = others.is_empty();
    let share = div()
        .flex()
        .flex_wrap()
        .gap(px(8.))
        .child(crate::ui::anchor("fc-copy", fc_button("fc-copy", "Copy as JSON", has).when(has, |d| d.on_click(cx.listener(move |_, _, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(scopes_to_json(&copy_list)));
            toast::show(format!("Copied {}", plural(copy_list.len())), cx);
        })))))
        .child(crate::ui::anchor("fc-paste", fc_button("fc-paste", "Paste scopes", true).on_click(cx.listener(|v, _, _, cx| {
            let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
            v.paste_file_colours(&text, cx);
        }))))
        .child(crate::ui::tooltip::tip(if others_empty { "No other repository has scopes" } else { "" }, crate::ui::anchor("fc-from", fc_button("fc-from", "Copy from another repository…", !others_empty).when(!others_empty, |d| d.on_mouse_down(MouseButton::Left, cx.listener(move |_, _, window, cx| {
            cx.stop_propagation();
            let weak = cx.weak_entity();
            let rows: Vec<MenuRow> = others.iter().map(|(d, n, list)| {
                let (weak, list, n) = (weak.clone(), list.clone(), *n);
                MenuRow::action(format!("fc.from.{d}"), format!("{} ({})", repo_label(d), plural(n)), "check", format!("Add {}'s scopes to this repository", repo_label(d)), move |_, cx| {
                    let list = list.clone();
                    weak.update(cx, |v, cx| {
                        let next = append_scopes(&v.scopes(), &list);
                        v.set_scopes(next, cx);
                        toast::show(format!("Added {}", plural(n)), cx);
                    }).ok();
                })
            }).collect();
            menu::open_below("fc-from", rows, OpenOptions { label: Some("Copy from another repository".into()), ..Default::default() }, window, cx);
        }))))));
    vec![page.child(share).into_any_element()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn scopes_go_out_as_json_and_come_back_checked() {
        let s = vec![json!({ "id": "x", "name": "Tests", "tint": "green", "patterns": ["*.rs"], "startCollapsed": true })];
        let text = scopes_to_json(&s);
        assert!(text.starts_with("{\n  \"gitvolt\": \"file-colours\""), "{text}");
        let back = parse_scopes(&text).unwrap();
        assert_eq!(back[0]["name"], "Tests");
        assert_eq!(back[0]["startCollapsed"], true);
        assert_eq!(parse_scopes("{\"gitvolt\":\"theme\",\"version\":1}"), None);
        let odd = parse_scopes("{\"gitvolt\":\"file-colours\",\"version\":1,\"scopes\":[{\"name\":\"\",\"patterns\":[\"a\"]},{\"name\":\"b\",\"tint\":\"nope\",\"patterns\":[\" c \",\"\"]}]}").unwrap();
        assert_eq!(odd.len(), 1);
        assert_eq!(odd[0]["tint"], "gray");
        assert_eq!(odd[0]["patterns"], json!(["c"]));
    }

    #[test]
    fn appended_scopes_get_free_names_and_new_ids() {
        let s = vec![json!({ "id": "x", "name": "Tests" })];
        let out = append_scopes(&s, &[json!({ "name": "Tests", "tint": "blue", "patterns": ["a"] })]);
        assert_eq!(out[1]["name"], "Tests (2)");
        assert_ne!(out[1]["id"], "x");
    }

    #[test]
    fn a_repositorys_label_is_its_folder() {
        assert_eq!(repo_label("/h/orbital/.git"), "orbital");
        assert_eq!(repo_label("/srv/orbital.git"), "orbital");
        assert_eq!(repo_label("/h/orbital/"), "orbital");
    }
}
