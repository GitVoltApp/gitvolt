//! The tab strip's state: the profile's `tabs`, `activeTab`, `closedTabs`, `tabGroups` and `savedGroups`
//! (`ui/src/app/tabs.ts`, `tabGroups.ts`), restored at the start and saved as they change; tab groups' chips and menu.

use crate::app::{GitVolt, OpenAs, icon};
use crate::theme;
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::menu::{MenuRow, Variant};
use crate::ui::modal::{self, Modal};
use crate::ui::{arm, controls};
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::rc::Rc;

/// `MAX_CLOSED` and what `closedTabs` holds.
#[derive(Debug, Clone, PartialEq)]
pub struct ClosedTab {
    pub path: String,
    pub alias: Option<String>,
    pub index: usize,
}

pub const GROUP_COLORS: [&str; 9] = ["blue", "purple", "cyan", "orange", "yellow", "pink", "green", "gray", "red"];

pub fn new_tab_state_id() -> String {
    crate::settings::themes::new_custom_id().replace("custom-", "tab-") + &format!("{:x}", crate::format::now() as u64 ^ next())
}

fn next() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(1);
    N.fetch_add(1, Ordering::Relaxed)
}

/// `colorLabel`: a group with no name goes by its colour.
pub fn color_label(c: &str) -> String {
    let mut s = c.chars();
    s.next().map_or(String::new(), |f| f.to_uppercase().chain(s).collect())
}

pub fn group_label(g: &Value) -> String {
    let name = g["name"].as_str().unwrap_or_default();
    if name.is_empty() { format!("{} group", color_label(g["color"].as_str().unwrap_or("gray"))) } else { name.to_string() }
}

/// `normalizeGroups`, on the tabs' ids in strip order: groups list only open tabs, in strip order, each in at most one;
/// empty ones go. Whether anything changed.
pub fn normalize_groups(groups: &mut Vec<Value>, order: &[String]) {
    let mut taken: Vec<String> = Vec::new();
    groups.retain_mut(|g| {
        let want: Vec<String> = g["tabs"].as_array().map_or(Vec::new(), |t| t.iter().filter_map(|x| x.as_str().map(str::to_string)).collect());
        let tabs: Vec<String> = order.iter().filter(|id| want.contains(id) && !taken.contains(id)).cloned().collect();
        taken.extend(tabs.iter().cloned());
        g["tabs"] = json!(tabs);
        !tabs.is_empty()
    });
}

/// Contiguity: each group's tabs, all at its first tab's place.
pub fn contiguous(order: &[String], groups: &[Value]) -> Vec<String> {
    let group_of = |id: &str| groups.iter().position(|g| g["tabs"].as_array().is_some_and(|t| t.iter().any(|x| x == id)));
    let mut out: Vec<String> = Vec::new();
    let mut done: Vec<usize> = Vec::new();
    for id in order {
        match group_of(id) {
            None => out.push(id.clone()),
            Some(g) if done.contains(&g) => {}
            Some(g) => {
                done.push(g);
                out.extend(order.iter().filter(|x| group_of(x) == Some(g)).cloned());
            }
        }
    }
    out
}

impl GitVolt {
    fn groups(&self) -> Vec<Value> {
        self.profile["tabGroups"].as_array().cloned().unwrap_or_default()
    }

    pub fn group_of_tab(&self, state_id: &str) -> Option<Value> {
        self.groups().into_iter().find(|g| g["tabs"].as_array().is_some_and(|t| t.iter().any(|x| x == state_id)))
    }

    pub fn next_group_color(&self) -> &'static str {
        let groups = self.groups();
        GROUP_COLORS.iter().find(|c| !groups.iter().any(|g| g["color"] == **c)).copied().unwrap_or(GROUP_COLORS[groups.len() % GROUP_COLORS.len()])
    }

    /// What the profile says of the tabs: saved as they change, so a restart finds them.
    pub fn save_tabs(&mut self, cx: &mut Context<Self>) {
        if self.profile.is_null() || self.restoring_tabs {
            return;
        }
        let order: Vec<String> = self.tabs.iter().map(|t| t.state_id.clone()).collect();
        let mut groups = self.groups();
        normalize_groups(&mut groups, &order);
        let want = contiguous(&order, &groups);
        if want != order {
            self.tabs.sort_by_key(|t| want.iter().position(|x| *x == t.state_id));
        }
        let active_id = self.tabs.get(self.active).map(|t| t.state_id.clone());
        let tabs: Vec<Value> = self.tabs.iter().map(|t| {
            if let Some(path) = &t.unopened {
                json!({ "id": t.state_id, "kind": "repo", "path": path, "alias": t.alias, "worktree": t.worktree })
            } else if t.is_open_tab() {
                json!({ "id": t.state_id, "kind": "open", "path": null, "alias": t.alias })
            } else {
                json!({ "id": t.state_id, "kind": "repo", "path": t.repo_path, "alias": t.alias, "worktree": t.worktree })
            }
        }).collect();
        let closed: Vec<Value> = self.closed_tabs.iter().map(|c| json!({ "path": c.path, "alias": c.alias, "index": c.index })).collect();
        self.update_profile(
            move |p| {
                p["tabs"] = json!(tabs);
                p["activeTab"] = json!(active_id);
                p["closedTabs"] = json!(closed);
                p["tabGroups"] = json!(groups);
            },
            cx,
        );
        self.fetch_if_due(cx);
    }

    /// The profile's tabs, in their order, each opened anew; one that can't be is left out.
    pub fn restore_tabs(&mut self, extra: Option<String>, cx: &mut Context<Self>) {
        let list = self.profile["tabs"].as_array().cloned().unwrap_or_default();
        let active = self.profile["activeTab"].as_str().map(str::to_string);
        self.closed_tabs = self.profile["closedTabs"].as_array().map_or(Vec::new(), |l| l.iter().filter_map(|c| Some(ClosedTab { path: c["path"].as_str()?.to_string(), alias: c["alias"].as_str().map(str::to_string), index: c["index"].as_u64().unwrap_or(0) as usize })).collect());
        let wanted: Vec<(String, String, Option<String>, Option<String>)> = list.iter().map(|t| (t["id"].as_str().unwrap_or_default().to_string(), t["kind"].as_str().unwrap_or("repo").to_string(), t["worktree"].as_str().or(t["path"].as_str()).map(str::to_string), t["alias"].as_str().map(str::to_string))).collect();
        if wanted.is_empty() && extra.is_none() {
            self.open_blank_tab(cx);
            return;
        }
        if cfg!(test) {
            return;
        }
        self.restoring_tabs = true;
        let api = self.api.clone();
        let paths: Vec<Option<String>> = wanted.iter().map(|w| w.2.clone()).collect();
        let extra2 = extra.clone();
        let read = self.rt.spawn(async move {
            let mut out = Vec::new();
            for path in paths {
                out.push(match path {
                    Some(path) => Some(crate::tabs::open_one(&api, path).await.map_err(|e| e.to_string())),
                    None => None,
                });
            }
            let extra = match extra2 {
                Some(path) => crate::tabs::open_one(&api, path).await.ok(),
                None => None,
            };
            (out, extra)
        });
        cx.spawn(async move |this, cx| {
            let Ok((opened, extra_opened)) = read.await else { return };
            this.update(cx, |this, cx| {
                let mut active_at = None;
                for ((state_id, kind, path, alias), found) in wanted.into_iter().zip(opened) {
                    match found {
                        Some(Ok(o)) => {
                            let at = this.tabs.len();
                            this.add_opened_tab(o, alias, Some(state_id.clone()), at, cx);
                        }
                        None if kind == "open" => {
                            this.open_blank_tab(cx);
                            let last = this.active;
                            this.tabs[last].state_id = state_id.clone();
                        }
                        // It stays in the strip and the profile, saying why, and comes back if the repository does.
                        Some(Err(error)) => {
                            let path = path.unwrap_or_default();
                            this.open_blank_tab(cx);
                            let last = this.active;
                            let name = std::path::Path::new(&path).file_name().map_or(path.clone(), |n| n.to_string_lossy().into_owned());
                            let tab = &mut this.tabs[last];
                            tab.state_id = state_id.clone();
                            tab.name = alias.clone().unwrap_or(name).into();
                            tab.alias = alias;
                            if let Some(screen) = tab.open_screen.as_mut() {
                                screen.error = Some(format!("Couldn't open {path}: {error}"));
                            }
                            tab.unopened = Some(path);
                        }
                        None => continue,
                    }
                    if active.as_deref() == Some(state_id.as_str()) {
                        active_at = Some(this.tabs.len() - 1);
                    }
                }
                if let Some(o) = extra_opened {
                    let existing = this.tabs.iter().position(|t| !t.is_open_tab() && crate::app::core_repo(t.id) == o.id && t.worktree == o.worktree);
                    active_at = Some(existing.unwrap_or_else(|| {
                        let at = this.tabs.len();
                        this.add_opened_tab(o, None, None, at, cx);
                        at
                    }));
                } else if let (Some(path), None) = (extra.as_ref(), active_at) {
                    this.open(path.clone(), cx);
                }
                this.restoring_tabs = false;
                if this.tabs.is_empty() {
                    this.open_blank_tab(cx);
                }
                this.active = active_at.unwrap_or(this.active).min(this.tabs.len().saturating_sub(1));
                this.save_tabs(cx);
                crate::bench::mark("tabs_restored");
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// `switchProfile`: every tab closes, asking first where there's unsaved work; the other profile's come up.
    pub fn switch_profile(&mut self, id: String, cx: &mut Context<Self>) {
        if self.ask_unsaved({
            let id = id.clone();
            move |v, cx| v.switch_profile(id.clone(), cx)
        }, cx) {
            return;
        }
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(gitvolt_core::api::Request::SwitchProfile { id }).await });
        cx.spawn(async move |this, cx| {
            let answer = read.await;
            this.update(cx, |this, cx| match answer {
                Ok(Ok(state)) => this.adopt_state(state, cx),
                Ok(Err(e)) => crate::ui::toast::show(e.message, cx),
                Err(e) => crate::ui::toast::show(e.to_string(), cx),
            })
            .ok();
        })
        .detach();
    }

    /// A profile's state becomes what's on screen.
    fn adopt_state(&mut self, state: Value, cx: &mut Context<Self>) {
        self.flush_drafts();
        // Tabs on repositories the next profile doesn't show stop being watched.
        let api = self.api.clone();
        let ids: Vec<u32> = self.tabs.iter().filter(|t| !t.is_open_tab()).map(|t| crate::app::core_repo(t.id)).collect();
        self.rt.spawn(async move {
            for repo in ids {
                api.dispatch(gitvolt_core::api::Request::UnwatchRepo { repo }).await.ok();
            }
        });
        self.tabs.clear();
        self.active = 0;
        self.profile = state["profile"].clone();
        self.settings = state["settings"].clone();
        self.profiles = state["profiles"].clone();
        self.colours_cache.borrow_mut().clear();
        self.apply_profile_layout();
        self.settings_changed(cx);
        self.restore_tabs(None, cx);
        cx.notify();
    }

    pub fn create_profile(&mut self, name: String, color: String, cx: &mut Context<Self>) {
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(gitvolt_core::api::Request::CreateProfile { name, color }).await });
        cx.spawn(async move |this, cx| {
            let answer = read.await;
            this.update(cx, |this, cx| match answer {
                Ok(Ok(meta)) => {
                    let id = meta["id"].as_str().unwrap_or_default().to_string();
                    if let Some(l) = this.profiles.as_array_mut() {
                        l.push(meta);
                    }
                    this.switch_profile(id, cx);
                }
                Ok(Err(e)) => crate::ui::toast::show(e.message, cx),
                Err(e) => crate::ui::toast::show(e.to_string(), cx),
            })
            .ok();
        })
        .detach();
    }

    pub fn delete_profile(&mut self, id: String, cx: &mut Context<Self>) {
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(gitvolt_core::api::Request::DeleteProfile { id }).await });
        cx.spawn(async move |this, cx| {
            let answer = read.await;
            this.update(cx, |this, cx| match answer {
                Ok(Ok(list)) => {
                    this.profiles = list;
                    cx.notify();
                }
                Ok(Err(e)) => crate::ui::toast::show(e.message, cx),
                Err(e) => crate::ui::toast::show(e.to_string(), cx),
            })
            .ok();
        })
        .detach();
    }

    /// The switcher's menu: each profile, New, Edit, Delete.
    pub fn profile_rows(&self, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let current = self.profile["id"].as_str().unwrap_or_default().to_string();
        let list = self.profiles.as_array().cloned().unwrap_or_default();
        let label = |p: &Value| {
            let n = p["name"].as_str().unwrap_or_default().trim();
            if n.is_empty() { "Unnamed".to_string() } else { n.to_string() }
        };
        let mut rows: Vec<MenuRow> = list.iter().map(|p| {
            let id = p["id"].as_str().unwrap_or_default().to_string();
            let on = id == current;
            let swatch = p["color"].as_str().and_then(theme::Col::parse).map_or(theme::blue(), theme::Col::hsla);
            let weak = this.clone();
            let id2 = id.clone();
            MenuRow::action(format!("profile.{id}"), label(p), "", "", move |_, cx| {
                let id = id2.clone();
                weak.update(cx, |v, cx| {
                    if v.profile["id"] != id.as_str() {
                        v.switch_profile(id, cx);
                    }
                }).ok();
            }).dot(swatch, on)
        }).collect();
        rows.push(MenuRow::Separator);
        let weak = this.clone();
        rows.push(MenuRow::action("profile.new", "New profile…", "plus", "Create a profile with its own tabs, recent repos and settings", move |window, cx| {
            weak.update(cx, |v, cx| v.open_profile_dialog(true, window, cx)).ok();
        }));
        let weak = this.clone();
        rows.push(MenuRow::action("profile.rename", "Edit profile…", "pencil", "Change this profile's name and color", move |window, cx| {
            weak.update(cx, |v, cx| v.open_profile_dialog(false, window, cx)).ok();
        }));
        let others: Vec<&Value> = list.iter().filter(|p| p["id"] != current.as_str()).collect();
        if !others.is_empty() {
            let sub: Vec<MenuRow> = others.into_iter().map(|p| {
                let (id, name) = (p["id"].as_str().unwrap_or_default().to_string(), label(p));
                let weak = this.clone();
                let n2 = name.clone();
                MenuRow::action(format!("profile.delete.{id}"), format!("{name}…"), "trash-2", format!("Delete {name} and its tabs"), move |window, cx| {
                    let (weak, id) = (weak.clone(), id.clone());
                    let req = arm::ArmRequest { body: "Its tabs, recent repositories and settings are removed. Your repositories on disk are not touched. This cannot be undone.".into(), ..arm::ArmRequest::danger(format!("Click again to delete {n2} and its tabs"), format!("Delete the profile “{n2}”?"), "Delete profile") };
                    arm::request(req, arm::Origin::current(cx), move |answer, _, cx| {
                        if answer.ok {
                            let id = id.clone();
                            weak.update(cx, |v, cx| v.delete_profile(id, cx)).ok();
                        }
                    }, window, cx);
                })
            }).collect();
            rows.push(MenuRow::submenu("profile.delete", "Delete profile", "trash-2", "Delete another profile (switch away before deleting the current one)", sub));
        }
        rows
    }

    /// New profile… / Edit profile….
    pub fn open_profile_dialog(&mut self, new: bool, window: &mut Window, cx: &mut Context<Self>) {
        const ID: &str = "profile-dialog";
        const COLORS: [&str; 8] = ["#4d88ff", "#2ece9d", "#f2ca33", "#f25d2e", "#d90171", "#8e00c2", "#15a0bf", "#7bd938"];
        let name = if new { String::new() } else { self.profile["name"].as_str().unwrap_or_default().to_string() };
        let color = Rc::new(std::cell::RefCell::new(if new { COLORS[1].to_string() } else { self.profile["color"].as_str().unwrap_or(COLORS[1]).to_string() }));
        let input = cx.new(|cx| {
            let mut i = TextInput::new(cx).enter_submits();
            i.set_text(&name, cx);
            i
        });
        let focus = input.read(cx).focus.clone();
        let weak = cx.weak_entity();
        let submit: crate::ui::Action = {
            let (input, color, weak) = (input.clone(), color.clone(), weak.clone());
            Rc::new(move |window, cx| {
                let name = input.read(cx).text().trim().to_string();
                let c = color.borrow().clone();
                modal::close(ID, window, cx);
                weak.update(cx, |v, cx| {
                    if new {
                        v.create_profile(name, c, cx);
                    } else {
                        v.update_profile(move |p| {
                            p["name"] = json!(name);
                            p["color"] = json!(c);
                        }, cx);
                        v.refresh_profile_meta(cx);
                    }
                }).ok();
            })
        };
        let s2 = submit.clone();
        let sub = cx.subscribe_in(&input, window, move |_, _, e: &InputEvent, window, cx| {
            if matches!(e, InputEvent::Submit { .. }) {
                // Out of this view's update: `submit` goes back into it.
                let s2 = s2.clone();
                window.defer(cx, move |window, cx| s2(window, cx));
            }
        });
        let view = input.clone();
        let mut m = Modal::new(ID, move |window, cx| {
            let mut swatches = div().flex().gap(px(8.));
            for c in COLORS {
                let (on, col) = (*color.borrow() == c, color.clone());
                swatches = swatches.child(crate::ui::anchor(format!("profile-color:{c}"), div().id(c).size(px(22.)).rounded_full().bg(theme::Col::parse(c).map_or(theme::blue(), theme::Col::hsla)).border_2().border_color(if on { theme::text_selected() } else { transparent_black() }).cursor_pointer().on_click(move |_, _, cx| {
                    *col.borrow_mut() = c.to_string();
                    cx.refresh_windows();
                })));
            }
            let focused = view.read(cx).is_focused(window);
            let s = submit.clone();
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(modal::title(if new { "New profile" } else { "Edit profile" }))
                .child(div().flex().flex_col().gap(px(4.)).child("Name").child(div().flex().items_center().h(px(28.)).px(px(8.)).rounded(px(4.)).bg(theme::app_bg()).border_1().border_color(if focused { theme::blue() } else { theme::section_border() }).text_color(theme::text_selected()).child(view.clone())))
                .child(div().text_size(px(12.)).text_color(theme::text_secondary()).child("Leave it empty to show just the color in the tab bar."))
                .child(swatches)
                .child(modal::actions_row().child(crate::ui::anchor("profile-cancel", controls::button("profile-cancel", "Cancel", controls::ButtonKind::Plain, true).on_click(|_, window, cx| modal::close(ID, window, cx)))).child(crate::ui::anchor("profile-save", controls::button("profile-save", if new { "Create" } else { "Save" }, controls::ButtonKind::Primary, true).on_click(move |_, window, cx| s(window, cx)))))
                .into_any_element()
        })
        .focus(focus);
        m.keep.push(sub);
        modal::open(m, window, cx);
    }

    /// The list's copy of this profile's name and colour.
    fn refresh_profile_meta(&mut self, _: &mut Context<Self>) {
        let (id, name, color) = (self.profile["id"].clone(), self.profile["name"].clone(), self.profile["color"].clone());
        if let Some(p) = self.profiles.as_array_mut().and_then(|l| l.iter_mut().find(|p| p["id"] == id)) {
            p["name"] = name;
            p["color"] = color;
        }
    }

    pub fn edit_groups(&mut self, change: impl FnOnce(&mut Vec<Value>), cx: &mut Context<Self>) {
        let mut groups = self.groups();
        change(&mut groups);
        self.profile["tabGroups"] = json!(groups);
        self.save_tabs(cx);
        cx.notify();
    }

    pub fn tab_new_group(&mut self, state_id: String, cx: &mut Context<Self>) {
        let color = self.next_group_color();
        let id = new_tab_state_id().replace("tab-", "group-");
        self.edit_groups(move |g| {
            for x in g.iter_mut() {
                if let Some(t) = x["tabs"].as_array_mut() {
                    t.retain(|y| y != state_id.as_str());
                }
            }
            g.push(json!({ "id": id, "name": "", "color": color, "collapsed": false, "tabs": [state_id] }));
        }, cx);
    }

    pub fn tab_add_to_group(&mut self, state_id: String, group: String, cx: &mut Context<Self>) {
        self.edit_groups(move |g| {
            for x in g.iter_mut() {
                if let Some(t) = x["tabs"].as_array_mut() {
                    t.retain(|y| y != state_id.as_str());
                }
            }
            if let Some(x) = g.iter_mut().find(|x| x["id"] == group.as_str())
                && let Some(t) = x["tabs"].as_array_mut()
            {
                t.push(json!(state_id));
            }
        }, cx);
    }

    pub fn tab_remove_from_group(&mut self, state_id: String, cx: &mut Context<Self>) {
        self.edit_groups(move |g| {
            for x in g.iter_mut() {
                if let Some(t) = x["tabs"].as_array_mut() {
                    t.retain(|y| y != state_id.as_str());
                }
            }
        }, cx);
    }

    /// What the tab's menu shows of groups.
    pub fn tab_group_rows(&self, ix: usize, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(ix) else { return Vec::new() };
        let state = tab.state_id.clone();
        let own = self.group_of_tab(&state);
        let others: Vec<Value> = self.groups().into_iter().filter(|g| Some(&g["id"]) != own.as_ref().map(|o| &o["id"])).collect();
        let mut rows = Vec::new();
        let (w, s) = (this.clone(), state.clone());
        rows.push(MenuRow::action("tab.newGroup", "Add to new group", "group", "Start a new tab group with this tab", move |_, cx| {
            let s = s.clone();
            w.update(cx, |v, cx| v.tab_new_group(s, cx)).ok();
        }));
        if !others.is_empty() {
            let sub = others.iter().map(|g| {
                let (w, s, id, label) = (this.clone(), state.clone(), g["id"].as_str().unwrap_or_default().to_string(), group_label(g));
                MenuRow::action(format!("tab.addToGroup.{id}"), label.clone(), "layers", format!("Move this tab to the end of {label}"), move |_, cx| {
                    let (s, id) = (s.clone(), id.clone());
                    w.update(cx, |v, cx| v.tab_add_to_group(s, id, cx)).ok();
                })
            }).collect();
            rows.push(MenuRow::submenu("tab.addToGroup", "Add to group", "layers", "Move this tab into a tab group", sub));
        }
        if let Some(o) = own {
            let (w, s) = (this.clone(), state);
            rows.push(MenuRow::action("tab.removeFromGroup", "Remove from group", "ungroup", format!("Take this tab out of {}", group_label(&o)), move |_, cx| {
                let s = s.clone();
                w.update(cx, |v, cx| v.tab_remove_from_group(s, cx)).ok();
            }));
        }
        rows
    }

    /// The tab bar's menu's saved groups.
    pub fn saved_group_rows(&self, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let saved = self.profile["savedGroups"].as_array().cloned().unwrap_or_default();
        if saved.is_empty() {
            return Vec::new();
        }
        let sub = saved.iter().map(|s| {
            let name = s["name"].as_str().filter(|n| !n.is_empty()).map_or_else(|| color_label(s["color"].as_str().unwrap_or("gray")), str::to_string);
            let n = s["tabs"].as_array().map_or(0, Vec::len);
            let id = s["id"].as_str().unwrap_or_default().to_string();
            let names: Vec<String> = s["tabs"].as_array().map_or(Vec::new(), |l| l.iter().map(|t| t["alias"].as_str().map_or_else(|| t["path"].as_str().unwrap_or_default().rsplit('/').next().unwrap_or_default().to_string(), str::to_string)).collect());
            let (w, w2, i1, i2, n2) = (this.clone(), this.clone(), id.clone(), id.clone(), name.clone());
            MenuRow::action(format!("tabbar.savedGroup.{id}"), format!("{name} ({n} tab{})", if n == 1 { "" } else { "s" }), "layers", format!("Reopen {name}: {}", names.join(", ")), move |_, cx| {
                let id = i1.clone();
                w.update(cx, |v, cx| v.reopen_saved_group(&id, cx)).ok();
            }).variants(vec![Variant { id: "delete".into(), label: None, icon: Some("trash-2"), tooltip: format!("Delete the saved group {name}").into(), disabled: None, run: Rc::new(move |window, cx| {
                let (w2, id) = (w2.clone(), i2.clone());
                let req = arm::ArmRequest { body: format!("Forgets {n2} and its {n} tab{}.", if n == 1 { "" } else { "s" }).into(), ..arm::ArmRequest::danger(format!("Click again to delete {n2}"), "Delete saved group?", "Delete") };
                arm::request(req, arm::Origin::current(cx), move |answer, _, cx| {
                    if answer.ok {
                        let id = id.clone();
                        w2.update(cx, |v, cx| v.update_profile(move |p| {
                            if let Some(l) = p["savedGroups"].as_array_mut() {
                                l.retain(|g| g["id"] != id.as_str());
                            }
                        }, cx)).ok();
                    }
                }, window, cx);
            }) }], None)
        }).collect();
        vec![MenuRow::submenu("tabbar.savedGroups", "Saved groups", "archive", "Groups closed with \"Save and close group\"", sub)]
    }

    fn group_id_tabs(&self, id: &str) -> Vec<usize> {
        let Some(g) = self.groups().into_iter().find(|g| g["id"] == id) else { return Vec::new() };
        let ids: Vec<String> = g["tabs"].as_array().map_or(Vec::new(), |t| t.iter().filter_map(|x| x.as_str().map(str::to_string)).collect());
        self.tabs.iter().enumerate().filter(|(_, t)| ids.contains(&t.state_id)).map(|(i, _)| i).collect()
    }

    /// `saveAndCloseGroup`: kept in the tab bar's menu.
    pub fn save_and_close_group(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(g) = self.groups().into_iter().find(|g| g["id"] == id) else { return };
        let tabs: Vec<Value> = self.group_id_tabs(id).iter().filter_map(|i| self.tabs.get(*i)).filter(|t| !t.is_open_tab()).map(|t| json!({ "path": t.repo_path, "alias": t.alias, "worktree": t.worktree })).collect();
        let saved = json!({ "id": g["id"], "name": g["name"], "color": g["color"], "tabs": tabs });
        self.update_profile(move |p| {
            if !p["savedGroups"].is_array() {
                p["savedGroups"] = json!([]);
            }
            p["savedGroups"].as_array_mut().expect("just made").push(saved);
        }, cx);
        self.close_group_tabs(id, cx);
    }

    pub fn close_group_tabs(&mut self, id: &str, cx: &mut Context<Self>) {
        for i in self.group_id_tabs(id).into_iter().rev() {
            self.close_tab(i, cx);
        }
    }

    pub fn ungroup(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = id.to_string();
        self.edit_groups(move |g| g.retain(|x| x["id"] != id.as_str()), cx);
    }

    fn reopen_saved_group(&mut self, id: &str, cx: &mut Context<Self>) {
        let saved = self.profile["savedGroups"].as_array().and_then(|l| l.iter().find(|g| g["id"] == id)).cloned();
        let Some(s) = saved else { return };
        let id = id.to_string();
        self.update_profile(move |p| {
            if let Some(l) = p["savedGroups"].as_array_mut() {
                l.retain(|g| g["id"] != id.as_str());
            }
        }, cx);
        // Opened one after the other, into a group of their own once they've their ids.
        self.pending_group = Some((json!({ "id": s["id"], "name": s["name"], "color": s["color"], "collapsed": false }), s["tabs"].as_array().map_or(0, Vec::len)));
        for t in s["tabs"].as_array().cloned().unwrap_or_default() {
            let path = t["worktree"].as_str().or(t["path"].as_str()).unwrap_or_default().to_string();
            self.open_as(OpenAs { path, in_tab: None, new_tab: true, alias: t["alias"].as_str().map(str::to_string), at: Some(usize::MAX), activate: true }, cx);
        }
    }

    /// A tab just opened: a saved group that's reopening takes it.
    pub fn take_into_pending_group(&mut self, state_id: &str, cx: &mut Context<Self>) {
        let Some((mut g, left)) = self.pending_group.take() else { return };
        let id = g["id"].as_str().unwrap_or_default().to_string();
        let mut groups = self.groups();
        match groups.iter_mut().find(|x| x["id"] == id.as_str()) {
            Some(x) => x["tabs"].as_array_mut().expect("a group's tabs").push(json!(state_id)),
            None => {
                g["tabs"] = json!([state_id]);
                groups.push(g.clone());
            }
        }
        self.profile["tabGroups"] = json!(groups);
        if left > 1 {
            self.pending_group = Some((g, left - 1));
        }
        self.save_tabs(cx);
    }

    /// `GroupMenu.tsx`: its name and colour, then New tab in group, Save and close group, Ungroup tabs and Delete group.
    pub fn open_group_menu(&mut self, id: String, anchor: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        const ID: &str = "tab-group-menu";
        let Some(g) = self.groups().into_iter().find(|g| g["id"] == id.as_str()) else { return };
        let name = cx.new(|cx| {
            let mut i = TextInput::new(cx).placeholder("Group name").enter_submits();
            i.set_text(g["name"].as_str().unwrap_or_default(), cx);
            i
        });
        let focus = name.read(cx).focus.clone();
        let weak = cx.weak_entity();
        let id0 = id.clone();
        // It runs inside this view's update: `this`, never a handle to the same view.
        let sub = cx.subscribe(&name, move |this, input, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Changed | InputEvent::Submit { .. }) {
                let (text, id) = (input.read(cx).text().to_string(), id0.clone());
                this.edit_groups(move |g| {
                    if let Some(x) = g.iter_mut().find(|x| x["id"] == id.as_str()) {
                        x["name"] = json!(text);
                    }
                }, cx);
            }
        });
        let bounds = crate::ui::Ui::get(cx).anchor(&anchor);
        let view = name.clone();
        let chosen = Rc::new(std::cell::RefCell::new(g["color"].as_str().unwrap_or("gray").to_string()));
        let mut m = Modal::new(ID, move |window, cx| {
            let mut swatches = div().flex().gap(px(6.));
            for c in GROUP_COLORS {
                let on = *chosen.borrow() == c;
                let (w, i, ch) = (weak.clone(), id.clone(), chosen.clone());
                swatches = swatches.child(crate::ui::tooltip::tip(color_label(c), crate::ui::anchor(format!("tg-colour:{c}"), div().id(c).size(px(18.)).rounded_full().bg(theme::tag_group(c)).border_2().border_color(if on { theme::text_selected() } else { transparent_black() }).cursor_pointer().on_click(move |_, _, cx| {
                    let i = i.clone();
                    *ch.borrow_mut() = c.to_string();
                    w.update(cx, |v, cx| v.edit_groups(move |g| {
                        if let Some(x) = g.iter_mut().find(|x| x["id"] == i.as_str()) {
                            x["color"] = json!(c);
                        }
                    }, cx)).ok();
                }))));
            }
            let row = |key: &'static str, ico: &'static str, label: &'static str| div().id(key).flex().items_center().gap(px(8.)).px(px(6.)).py(px(4.)).rounded(px(4.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).child(icon(ico, 14., theme::text_secondary())).child(label);
            let (w1, i1, w2, i2, w3, i3, w4, i4) = (weak.clone(), id.clone(), weak.clone(), id.clone(), weak.clone(), id.clone(), weak.clone(), id.clone());
            let focused = view.read(cx).is_focused(window);
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .w(px(220.))
                .child(div().flex().items_center().h(px(26.)).px(px(8.)).mb(px(4.)).rounded(px(4.)).bg(theme::app_bg()).border_1().border_color(if focused { theme::blue() } else { theme::section_border() }).text_color(theme::text_selected()).child(view.clone()))
                .child(swatches.mb(px(4.)))
                .child(crate::ui::anchor("tg-new-tab", row("tg-new-tab", "plus", "New tab in group").on_click(move |_, window, cx| {
                    modal::close(ID, window, cx);
                    let i = i1.clone();
                    w1.update(cx, |v, cx| {
                        let opened = v.open_blank_tab(cx);
                        let s = v.tabs.iter().find(|t| t.id == opened).map(|t| t.state_id.clone()).unwrap_or_default();
                        v.tab_add_to_group(s, i, cx);
                    }).ok();
                })))
                .child(crate::ui::anchor("tg-save", row("tg-save", "archive", "Save and close group").on_click(move |_, window, cx| {
                    modal::close(ID, window, cx);
                    let i = i2.clone();
                    w2.update(cx, |v, cx| v.save_and_close_group(&i, cx)).ok();
                })))
                .child(crate::ui::anchor("tg-ungroup", row("tg-ungroup", "ungroup", "Ungroup tabs").on_click(move |_, window, cx| {
                    modal::close(ID, window, cx);
                    let i = i3.clone();
                    w3.update(cx, |v, cx| v.ungroup(&i, cx)).ok();
                })))
                .child(crate::ui::anchor("tg-delete", row("tg-delete", "trash-2", "Delete group").on_click(move |_, window, cx| {
                    let (w, i) = (w4.clone(), i4.clone());
                    let req = arm::ArmRequest { body: "Closes its tabs.".into(), ..arm::ArmRequest::danger("Click again to delete the group and close its tabs", "Delete group?", "Delete") };
                    arm::request(req, arm::Origin::control("tg-delete"), move |answer, window, cx| {
                        if answer.ok {
                            modal::close(ID, window, cx);
                            let i = i.clone();
                            w.update(cx, |v, cx| v.close_group_tabs(&i, cx)).ok();
                        }
                    }, window, cx);
                })))
                .into_any_element()
        })
        .anchored(bounds)
        .width(250.)
        .focus(focus);
        m.keep.push(sub);
        modal::open(m, window, cx);
    }
}

pub struct Opened {
    pub id: u32,
    pub name: String,
    pub worktree: String,
    pub path: String,
}

pub async fn open_one(api: &gitvolt_core::api::Api, path: String) -> Result<Opened, gitvolt_core::error::GbError> {
    let v = api.dispatch(gitvolt_core::api::Request::OpenRepo { path }).await?;
    let id = v["id"].as_u64().unwrap_or(0) as u32;
    api.dispatch(gitvolt_core::api::Request::WatchRepo { repo: id }).await?;
    Ok(Opened { id, name: v["name"].as_str().unwrap_or_default().to_string(), worktree: v["worktree"].as_str().unwrap_or_default().to_string(), path: v["path"].as_str().unwrap_or_default().to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn groups_list_open_tabs_in_strip_order_and_empty_ones_go() {
        let order: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let mut g = vec![json!({ "id": "1", "tabs": ["c", "a", "gone"] }), json!({ "id": "2", "tabs": ["a"] }), json!({ "id": "3", "tabs": [] })];
        normalize_groups(&mut g, &order);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0]["tabs"], json!(["a", "c"]));
    }

    #[test]
    fn a_groups_tabs_stand_together_at_its_first() {
        let order: Vec<String> = ["a", "x", "b", "y"].iter().map(|s| s.to_string()).collect();
        let g = vec![json!({ "id": "1", "tabs": ["a", "b"] })];
        assert_eq!(contiguous(&order, &g), ["a", "b", "x", "y"]);
    }

    #[test]
    fn a_group_with_no_name_goes_by_its_colour() {
        assert_eq!(group_label(&json!({ "name": "", "color": "cyan" })), "Cyan group");
        assert_eq!(group_label(&json!({ "name": "Work", "color": "cyan" })), "Work");
    }
}
