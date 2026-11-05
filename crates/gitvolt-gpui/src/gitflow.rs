//! Git flow (`ui/src/gitflow/`): Init from the palette, a Git Flow sidebar panel once it's set up (its + starts a branch),
//! and Finish on a feature, release or hotfix branch's menu or the palette. Its settings live in the repository's git config.

use crate::app::{GitVolt, Tab};
use crate::refs::{branch_name_error, tag_name_error};
use crate::theme;
use crate::ui::controls::{self, ButtonKind};
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::menu::MenuRow;
use crate::ui::modal;
use crate::ui::toast::{self, Options};
use crate::write::{Write, WriteCtx, call};
use gitvolt_core::api::Request;
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const KINDS: [&str; 3] = ["feature", "release", "hotfix"];

pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "feature" => "Feature",
        "release" => "Release",
        _ => "Hotfix",
    }
}

/// `GitflowConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Flow {
    pub master: String,
    pub develop: String,
    pub feature: String,
    pub release: String,
    pub hotfix: String,
    pub version_tag: String,
}

impl Flow {
    pub fn of(v: &Value) -> Option<Self> {
        let s = |k: &str| v[k].as_str().map(str::to_string);
        Some(Self { master: s("master")?, develop: s("develop")?, feature: s("feature")?, release: s("release")?, hotfix: s("hotfix")?, version_tag: s("versionTag").unwrap_or_default() })
    }

    pub fn json(&self) -> Value {
        json!({ "master": self.master, "develop": self.develop, "feature": self.feature, "release": self.release, "hotfix": self.hotfix, "versionTag": self.version_tag })
    }

    pub fn prefix(&self, kind: &str) -> &str {
        match kind {
            "feature" => &self.feature,
            "release" => &self.release,
            _ => &self.hotfix,
        }
    }

    /// `classify`: a branch's kind and what follows its prefix (a release's version).
    pub fn classify(&self, name: &str) -> Option<(&'static str, String)> {
        KINDS.iter().find_map(|k| name.strip_prefix(self.prefix(k)).filter(|r| !r.is_empty()).map(|r| (*k, r.to_string())))
    }

    /// Where a new branch of `kind` starts: a hotfix from production, the others from development.
    pub fn base(&self, kind: &str) -> &str {
        if kind == "hotfix" { &self.master } else { &self.develop }
    }

    /// The branches Finish merges into, in order.
    pub fn targets(&self, kind: &str) -> Vec<String> {
        if kind == "feature" { vec![self.develop.clone()] } else { vec![self.master.clone(), self.develop.clone()] }
    }

    fn get(&self, field: usize) -> &str {
        [&self.master, &self.develop, &self.feature, &self.release, &self.hotfix, &self.version_tag][field]
    }

    fn set(&mut self, field: usize, value: String) {
        *[&mut self.master, &mut self.develop, &mut self.feature, &mut self.release, &mut self.hotfix, &mut self.version_tag][field] = value;
    }
}

/// A version number in a release's or hotfix's name: `3.0.0`, `v3.0.0`, `3.0`, `3.0.0-rc.1`.
fn version_of(rest: &str) -> Option<&str> {
    let v = rest.strip_prefix('v').unwrap_or(rest);
    let (core, tail) = match v.find(['-', '+']) {
        Some(i) => (&v[..i], &v[i + 1..]),
        None => (v, ""),
    };
    let numbers = !core.is_empty() && core.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    let suffix_ok = (v.len() == core.len()) || (!tail.is_empty() && tail.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'));
    (numbers && suffix_ok).then_some(v)
}

/// `suggestedTag`: the version a release or hotfix is named after, after the version tag prefix; `""` when its name isn't
/// a version; a feature has none.
pub fn suggested_tag(f: &Flow, kind: &str, rest: &str) -> Option<String> {
    if kind == "feature" {
        return None;
    }
    Some(version_of(rest).map_or(String::new(), |v| format!("{}{v}", f.version_tag)))
}

/// `defaultConfig`: `git flow init`'s defaults, with the production branch the repository has.
pub fn default_config(locals: &[String]) -> Flow {
    let master = ["main", "master"].iter().find(|b| locals.iter().any(|l| l == *b)).map_or("main", |b| *b);
    Flow { master: master.into(), develop: "develop".into(), feature: "feature/".into(), release: "release/".into(), hotfix: "hotfix/".into(), version_tag: String::new() }
}

/// `configError`: why the settings can't be used, as the core's check says.
pub fn config_error(c: &Flow) -> Option<String> {
    for (what, name) in [("production", &c.master), ("development", &c.develop)] {
        if let Some(why) = branch_name_error(name) {
            return Some(format!("The {what} branch: {why}"));
        }
    }
    if c.master == c.develop {
        return Some("The production and development branches must differ".to_string());
    }
    let prefixes: Vec<(&str, &str)> = KINDS.iter().map(|k| (*k, c.prefix(k))).collect();
    for (i, (what, prefix)) in prefixes.iter().enumerate() {
        if prefix.is_empty() {
            return Some(format!("Enter a {what} prefix"));
        }
        if let Some(why) = branch_name_error(&format!("{prefix}x")) {
            return Some(format!("The {what} prefix: {why}"));
        }
        if let Some((other, _)) = prefixes[..i].iter().find(|(_, p)| p.starts_with(prefix) || prefix.starts_with(p)) {
            return Some(format!("The {what} and {other} prefixes overlap"));
        }
        if let Some(branch) = [&c.master, &c.develop].iter().find(|b| b.starts_with(prefix)) {
            return Some(format!("The {what} prefix would make {branch} a {what} branch"));
        }
    }
    if !c.version_tag.is_empty()
        && let Some(why) = tag_name_error(&format!("{}1.0", c.version_tag))
    {
        return Some(format!("The version tag prefix: {why}"));
    }
    None
}

/// `startNameError`: what the core would refuse, said of the name typed; the prefix shows beside the box.
pub fn start_name_error(c: &Flow, kind: &str, name: &str, taken: &dyn Fn(&str) -> bool) -> Option<String> {
    if name.is_empty() {
        return Some(if kind == "feature" { "Enter a feature name".to_string() } else { "Enter a version".to_string() });
    }
    let full = format!("{}{name}", c.prefix(kind));
    if let Some(why) = branch_name_error(&full) {
        return Some(why);
    }
    if taken(&full) {
        return Some(format!("A branch named {full} already exists"));
    }
    if kind != "feature" {
        let tag = format!("{}{name}", c.version_tag);
        if tag_name_error(&tag).is_some() {
            return Some(format!("{tag} isn't a valid tag name"));
        }
    }
    None
}

fn list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        _ => format!("{} and {}", names[..names.len() - 1].join(", "), names[names.len() - 1]),
    }
}

/// `finishedText`: what a finish did, in one sentence.
pub fn finished_text(branch: &str, out: &Value) -> String {
    let merged: Vec<String> = out["merged"].as_array().map_or(Vec::new(), |m| m.iter().filter_map(|s| s.as_str().map(str::to_string)).collect());
    let mut done = Vec::new();
    if !merged.is_empty() {
        done.push(format!("merged into {}", list(&merged)));
    }
    if let Some(t) = out["tag"].as_str() {
        done.push(format!("tagged {t}"));
    }
    if out["deleted"] == true {
        done.push("deleted".to_string());
    }
    if done.is_empty() {
        return format!("{branch} is already finished");
    }
    let said = if done.len() > 1 && done.iter().any(|d| d.contains(" and ")) { format!("{}, and {}", done[..done.len() - 1].join(", "), done[done.len() - 1]) } else { list(&done) };
    format!("Finished {branch}: {said}")
}

impl Tab {
    /// The tab's git flow settings, once it's set up.
    pub fn flow(&self) -> Option<Flow> {
        Flow::of(&self.sidebar["gitflow"])
    }

    /// The checked-out branch, when it's one of git flow's.
    pub fn head_flow_branch(&self) -> Option<String> {
        let flow = self.flow()?;
        let head = self.head_branch()?;
        flow.classify(&head).map(|_| head)
    }
}

/// A field, its focus ring by whether it has the keyboard.
pub fn labelled(label: impl Into<SharedString>, input: &Entity<TextInput>, window: &Window, cx: &App) -> Div {
    let on = input.read(cx).is_focused(window);
    div().flex().flex_col().gap(px(4.)).min_w_0().child(label.into()).child(div().flex().h(px(30.2)).px(px(8.)).items_center().bg(theme::app_bg()).border_1().border_color(if on { theme::blue() } else { theme::section_border() }).rounded(px(4.)).text_color(theme::text_selected()).child(input.clone()))
}

fn err(text: impl Into<SharedString>) -> Div {
    div().text_color(theme::red()).child(text.into())
}

fn field_input(text: &str, placeholder: Option<&'static str>, cx: &mut App) -> Entity<TextInput> {
    cx.new(|cx| {
        let mut i = TextInput::new(cx);
        if let Some(p) = placeholder {
            i = i.placeholder(p);
        }
        i.set_text(text, cx);
        i
    })
}

impl GitVolt {
    /// Init's suggestion before it's set up, the current settings after.
    pub fn open_gitflow_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        const ID: &str = "gitflow-init";
        const LABELS: [&str; 6] = ["Production branch", "Development branch", "Feature prefix", "Release prefix", "Hotfix prefix", "Version tag prefix"];
        let Some(tab) = self.tabs.get(self.active) else { return };
        let locals: Vec<String> = tab.locals().into_iter().map(|b| b.name).collect();
        let current = tab.flow();
        let editing = current.is_some();
        let initial = current.unwrap_or_else(|| default_config(&locals));
        let ctx = tab.write_target();
        let inputs: Vec<Entity<TextInput>> = (0..6).map(|i| field_input(initial.get(i), (i == 5).then_some("None"), cx)).collect();
        let this = cx.weak_entity();
        let read = {
            let inputs = inputs.clone();
            move |cx: &App| {
                let mut c = Flow::default();
                for (i, input) in inputs.iter().enumerate() {
                    c.set(i, input.read(cx).text().to_string());
                }
                c
            }
        };
        let locals2 = locals.clone();
        let usable = {
            let (initial, locals) = (initial.clone(), locals.clone());
            move |c: &Flow| config_error(c).is_none() && locals.contains(&c.master) && (!editing || *c != initial)
        };
        let send: crate::ui::Action = {
            let (read, usable, ctx) = (read.clone(), usable.clone(), ctx.clone());
            Rc::new(move |window, cx| {
                let c = read(cx);
                if !usable(&c) {
                    return;
                }
                modal::close(ID, window, cx);
                let ctx = ctx.clone();
                this.update(cx, |v, cx| v.init_gitflow(ctx, c, editing, cx)).ok();
            })
        };
        let handle = window.window_handle();
        let keep: Vec<Subscription> = inputs
            .iter()
            .map(|input| {
                let send = send.clone();
                App::subscribe(cx, input, move |_, e: &InputEvent, cx| match e {
                    InputEvent::Submit { .. } => {
                        let send = send.clone();
                        handle.update(cx, move |_, window, cx| window.defer(cx, move |window, cx| send(window, cx))).ok();
                    }
                    InputEvent::Changed => cx.refresh_windows(),
                    _ => {}
                })
            })
            .collect();
        let focus = inputs[0].read(cx).focus.clone();
        let stops = [cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true)];
        let mut m = modal::Modal::new(ID, move |window, cx| {
            let c = read(cx);
            let error = config_error(&c);
            let missing = error.is_none() && !locals2.contains(&c.master);
            let creates = error.is_none() && !missing && !locals2.contains(&c.develop);
            let can = usable(&c);
            let (s1, s2) = (send.clone(), send.clone());
            let mut grid = div().flex().flex_wrap().gap_y(px(10.)).gap_x(px(12.));
            for (i, l) in LABELS.iter().enumerate() {
                grid = grid.child(div().w(px(206.)).child(labelled(*l, &inputs[i], window, cx)));
            }
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(modal::title(if editing { "Git flow settings" } else { "Set up git flow" }))
                .child(grid)
                .children(error.map(err))
                .when(missing, |d| d.child(err(format!("There's no branch {}: create it first", c.master))))
                .when(creates, |d| d.child(format!("{} doesn't exist yet: it's created at {}.", c.develop, c.master)))
                .child("Kept in this repository's git config, where the git flow command line reads them too.")
                .child(modal::actions_row().child(controls::tabbable(controls::button("form-cancel", "Cancel", ButtonKind::Plain, true).on_click(|_, window, cx| modal::close(ID, window, cx)), &stops[0], |window, cx| modal::close(ID, window, cx))).child(crate::ui::anchor("form-ok", controls::tabbable(controls::button("form-ok", if editing { "Save" } else { "Set up git flow" }, ButtonKind::Primary, can).when(can, |d| d.on_click(move |_, window, cx| s1(window, cx))), &stops[1], move |window, cx| s2(window, cx)))))
                .into_any_element()
        })
        .width(460.)
        .focus(focus);
        m.keep.extend(keep);
        modal::open(m, window, cx);
    }

    pub fn init_gitflow(&mut self, ctx: WriteCtx, config: Flow, editing: bool, cx: &mut Context<Self>) {
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let w = Write::new(&ctx, move |_| call("gitflowInit", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "config": config.json() }))).on_success(move |this, _, cx| {
            toast::show(if editing { "Git flow settings saved" } else { "Git flow is set up" }, cx);
            this.reload(repo, cx);
        });
        self.write(w, cx);
    }

    /// `Start a feature`: an ordinary create-and-checkout at its base branch's tip.
    pub fn open_gitflow_start(&mut self, kind: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        const ID: &str = "gitflow-start";
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(flow) = tab.flow() else { return };
        let locals: Vec<String> = tab.locals().into_iter().map(|b| b.name).collect();
        let ctx = tab.write_target();
        let chosen = Rc::new(Cell::new(kind));
        let name = field_input("", None, cx);
        let this = cx.weak_entity();
        let error_of = {
            let (flow, locals) = (flow.clone(), locals);
            move |kind: &str, n: &str| start_name_error(&flow, kind, n, &|b| locals.iter().any(|l| l == b))
        };
        let send: crate::ui::Action = {
            let (name, chosen, error_of, ctx) = (name.clone(), chosen.clone(), error_of.clone(), ctx);
            Rc::new(move |window, cx| {
                let n = name.read(cx).text().to_string();
                if error_of(chosen.get(), &n).is_some() {
                    return;
                }
                modal::close(ID, window, cx);
                let (ctx, kind) = (ctx.clone(), chosen.get());
                this.update(cx, |v, cx| v.start_gitflow(ctx, kind, n, cx)).ok();
            })
        };
        let handle = window.window_handle();
        let s = send.clone();
        let keep = App::subscribe(cx, &name, move |_, e: &InputEvent, cx| match e {
            InputEvent::Submit { .. } => {
                let send = s.clone();
                handle.update(cx, move |_, window, cx| window.defer(cx, move |window, cx| send(window, cx))).ok();
            }
            InputEvent::Changed => cx.refresh_windows(),
            _ => {}
        });
        let focus = name.read(cx).focus.clone();
        let stops = [cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true)];
        let mut m = modal::Modal::new(ID, move |window, cx| {
            let k = chosen.get();
            let n = name.read(cx).text().to_string();
            let error = error_of(k, &n);
            let (s1, s2) = (send.clone(), send.clone());
            let radios = KINDS.iter().enumerate().map(|(i, each)| {
                let (c1, c2, each) = (chosen.clone(), chosen.clone(), *each);
                controls::tabbable(
                    div().id(("gitflow-kind", i)).flex().items_center().gap(px(6.)).cursor_pointer().border_1().border_color(transparent_black()).rounded(px(3.)).on_click(move |_, _, cx| {
                        c1.set(each);
                        cx.refresh_windows();
                    }).child(div().size(px(14.)).rounded_full().border_1().border_color(if k == each { theme::blue() } else { theme::section_border() }).flex().items_center().justify_center().when(k == each, |d| d.child(div().size(px(8.)).rounded_full().bg(theme::blue())))).child(kind_label(each)),
                    &stops[i],
                    move |_, cx| {
                        c2.set(each);
                        cx.refresh_windows();
                    },
                )
            });
            let prefix = flow.prefix(k).to_string();
            let focused = name.read(cx).is_focused(window);
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(modal::title(format!("Start a {}", kind_label(k).to_lowercase())))
                .child(div().flex().gap(px(16.)).children(radios))
                .child(div().flex().flex_col().gap(px(4.)).child(if k == "feature" { "Name" } else { "Version" }).child(div().flex().h(px(30.2)).items_center().bg(theme::app_bg()).border_1().border_color(if focused { theme::blue() } else { theme::section_border() }).rounded(px(4.)).child(div().flex_none().pl(px(8.)).text_color(theme::text_secondary()).child(prefix)).child(div().flex().flex_1().min_w_0().pl(px(1.)).pr(px(8.)).text_color(theme::text_selected()).child(name.clone()))))
                .when_some(error.clone().filter(|_| !n.is_empty()), |d, e| d.child(err(e)))
                .child(format!("Starts from {} and checks the new branch out.", flow.base(k)))
                .child(modal::actions_row().child(controls::tabbable(controls::button("form-cancel", "Cancel", ButtonKind::Plain, true).on_click(|_, window, cx| modal::close(ID, window, cx)), &stops[3], |window, cx| modal::close(ID, window, cx))).child(crate::ui::anchor("form-ok", controls::tabbable(controls::button("form-ok", format!("Start {}", kind_label(k).to_lowercase()), ButtonKind::Primary, error.is_none()).when(error.is_none(), |d| d.on_click(move |_, window, cx| s1(window, cx))), &stops[4], move |window, cx| s2(window, cx)))))
                .into_any_element()
        })
        .width(460.)
        .focus(focus);
        m.keep.push(keep);
        modal::open(m, window, cx);
    }

    fn start_gitflow(&mut self, ctx: WriteCtx, kind: &str, name: String, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == ctx.repo) else { return };
        let Some(flow) = tab.flow() else { return };
        let base = flow.base(kind).to_string();
        let Some(sha) = tab.local_branch(&base).map(|b| b.target) else {
            return toast::show_with(format!("There's no branch {base} to start from"), Options { error: true, ..Default::default() }, cx);
        };
        self.create_branch(ctx, sha, Some(format!("refs/heads/{base}")), format!("{}{name}", flow.prefix(kind)), true, cx);
    }

    /// Finish's options: the tag a release or hotfix gets, its message, whether the branch goes.
    pub fn open_gitflow_finish(&mut self, branch: String, window: &mut Window, cx: &mut Context<Self>) {
        const ID: &str = "gitflow-finish";
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(flow) = tab.flow() else { return };
        let Some((kind, rest)) = flow.classify(&branch) else { return };
        let suggested = suggested_tag(&flow, kind, &rest);
        let tagged = suggested.is_some();
        let ctx = tab.write_target();
        let repo = ctx.repo;
        let tags = tab.sidebar["tags"].clone();
        let tag = field_input(suggested.as_deref().unwrap_or_default(), Some("3.0.0"), cx);
        let message = field_input("", Some("None: a lightweight tag"), cx);
        let message_edited = Rc::new(Cell::new(false));
        let keep_branch = Rc::new(Cell::new(false));
        let kept = Rc::new(RefCell::new(None::<String>));
        let this = cx.weak_entity();
        let targets = flow.targets(kind);
        // An earlier finish that stopped before development tagged production already: that tag stays.
        if tagged && !cfg!(test) {
            let (api, b, t, m, k, tags) = (self.api.clone(), branch.clone(), tag.clone(), message.clone(), kept.clone(), tags);
            let read = self.rt.spawn(async move { api.dispatch(Request::GitflowFinishedTag { repo: crate::app::core_repo(repo), branch: b }).await });
            cx.spawn(async move |_, cx| {
                let Ok(Ok(v)) = read.await else { return };
                let Some(found) = v.as_str().map(str::to_string) else { return };
                cx.update(|cx| {
                    let text = tags.as_array().and_then(|l| l.iter().find(|x| x["name"] == found.as_str())).and_then(|x| x["annotation"]["message"].as_str()).unwrap_or_default().to_string();
                    t.update(cx, |i, cx| i.set_text(&found, cx));
                    m.update(cx, |i, cx| i.set_text(&text, cx));
                    *k.borrow_mut() = Some(found);
                    cx.refresh_windows();
                });
            })
            .detach();
        }
        let shown_message = {
            let (tag, message, edited) = (tag.clone(), message.clone(), message_edited.clone());
            move |cx: &App| {
                let t = tag.read(cx).text().to_string();
                if edited.get() { message.read(cx).text().to_string() } else if t.is_empty() { String::new() } else { format!("{} {t}", kind_label(kind)) }
            }
        };
        let tag_error = {
            let (tag, kept) = (tag.clone(), kept.clone());
            move |cx: &App| -> Option<String> {
                if !tagged || kept.borrow().is_some() {
                    return None;
                }
                let t = tag.read(cx).text();
                if t.is_empty() { Some("Enter a tag name".to_string()) } else { tag_name_error(t) }
            }
        };
        let send: crate::ui::Action = {
            let (tag, tag_error, shown, keep, kept, ctx, b) = (tag.clone(), tag_error.clone(), shown_message.clone(), keep_branch.clone(), kept.clone(), ctx, branch.clone());
            Rc::new(move |window, cx| {
                if tag_error(cx).is_some() {
                    return;
                }
                let t = tagged.then(|| tag.read(cx).text().to_string());
                let m = (tagged && kept.borrow().is_none()).then(|| shown(cx)).filter(|m| !m.trim().is_empty());
                modal::close(ID, window, cx);
                let (ctx, b, keep) = (ctx.clone(), b.clone(), keep.get());
                this.update(cx, |v, cx| v.finish_gitflow(ctx, b, t, m, keep, cx)).ok();
            })
        };
        let handle = window.window_handle();
        let keep_subs: Vec<Subscription> = [(&tag, false), (&message, true)]
            .into_iter()
            .map(|(input, is_message)| {
                let (send, edited) = (send.clone(), message_edited.clone());
                App::subscribe(cx, input, move |_, e: &InputEvent, cx| match e {
                    InputEvent::Submit { .. } => {
                        let send = send.clone();
                        handle.update(cx, move |_, window, cx| window.defer(cx, move |window, cx| send(window, cx))).ok();
                    }
                    InputEvent::Changed => {
                        if is_message {
                            edited.set(true);
                        }
                        cx.refresh_windows();
                    }
                    _ => {}
                })
            })
            .collect();
        let focus = if tagged { tag.read(cx).focus.clone() } else { cx.focus_handle().tab_stop(true) };
        let stops = [cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true)];
        let (name, master, develop) = (branch, flow.master.clone(), flow.develop.clone());
        let mut m = modal::Modal::new(ID, move |window, cx| {
            let t = tag.read(cx).text().to_string();
            let error = tag_error(cx);
            let was_kept = kept.borrow().clone();
            // The message follows the tag until it's edited.
            if !message_edited.get() && was_kept.is_none() {
                let text = shown_message(cx);
                if message.read(cx).text() != text {
                    message.update(cx, |i, cx| i.set_text(&text, cx));
                }
            }
            let tag_step = match &was_kept {
                Some(k) => format!("keeps {master}'s tag {k}"),
                None if !t.is_empty() && error.is_none() => format!("tags {master} {t}"),
                None => format!("tags {master}"),
            };
            let mut steps = vec![format!("merges it into {}", targets.join(", then "))];
            if tagged {
                steps.push(tag_step);
            }
            if !keep_branch.get() {
                steps.push("deletes it".to_string());
            }
            let (s1, s2, k1, k2) = (send.clone(), send.clone(), keep_branch.clone(), keep_branch.clone());
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(modal::title(format!("Finish {name}")))
                .child(format!("Finishing {}, then leaves you on {develop}.", steps.join(", ")))
                .when(tagged, |d| d.child(labelled("Tag", &tag, window, cx)).when_some(error.clone().filter(|_| !t.is_empty()), |d, e| d.child(err(e))).child(labelled("Tag message", &message, window, cx)))
                .child(controls::tabbable(
                    controls::check_row("gitflow-delete", !keep_branch.get(), format!("Delete {name}"), move |on, _, cx| {
                        k1.set(!on);
                        cx.refresh_windows();
                    })
                    .border_1()
                    .border_color(transparent_black())
                    .rounded(px(3.))
                    .ml(px(-1.)),
                    &stops[0],
                    move |_, cx| {
                        k2.set(!k2.get());
                        cx.refresh_windows();
                    },
                ))
                .child(modal::actions_row().child(controls::tabbable(controls::button("form-cancel", "Cancel", ButtonKind::Plain, true).on_click(|_, window, cx| modal::close(ID, window, cx)), &stops[1], |window, cx| modal::close(ID, window, cx))).child(crate::ui::anchor("form-ok", controls::tabbable(controls::button("form-ok", format!("Finish {}", kind_label(kind).to_lowercase()), ButtonKind::Primary, error.is_none()).when(error.is_none(), |d| d.on_click(move |_, window, cx| s1(window, cx))), &stops[2], move |window, cx| s2(window, cx)))))
                .into_any_element()
        })
        .width(460.)
        .focus(focus);
        m.keep.extend(keep_subs);
        modal::open(m, window, cx);
    }

    /// One write. A merge that would conflict is left for the user to make; Finish again then does what's left.
    pub fn finish_gitflow(&mut self, ctx: WriteCtx, branch: String, tag: Option<String>, tag_message: Option<String>, keep: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == ctx.repo) else { return };
        let Some(flow) = tab.flow() else { return };
        let Some((kind, _)) = flow.classify(&branch) else { return };
        let mut names = vec![branch.clone()];
        names.extend(flow.targets(kind));
        let mut refs = serde_json::Map::new();
        for n in names {
            if let Some(b) = tab.local_branch(&n) {
                refs.insert(format!("refs/heads/{n}"), json!(b.target));
            }
        }
        let (repo, worktree, b) = (ctx.repo, ctx.worktree.clone(), branch.clone());
        let c2 = ctx.clone();
        let w = Write::new(&ctx, move |_| {
            let mut params = json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "branch": b, "keep": keep, "expect": { "head": null, "refs": refs } });
            if let Some(t) = &tag {
                params["tag"] = json!(t);
            }
            if let Some(m) = &tag_message {
                params["tagMessage"] = json!(m);
            }
            call("gitflowFinish", params)
        })
        .on_success(move |_, out, cx| {
            if out["status"] == "done" {
                return toast::show(finished_text(&branch, out), cx);
            }
            let into = out["into"].as_str().unwrap_or_default().to_string();
            let files = out["files"].as_array().map_or(0, Vec::len);
            let merged: Vec<String> = out["merged"].as_array().map_or(Vec::new(), |m| m.iter().filter_map(|s| s.as_str().map(str::to_string)).collect());
            let mut before = Vec::new();
            if !merged.is_empty() {
                before.push(format!("merged into {}", list(&merged)));
            }
            if let Some(t) = out["tag"].as_str() {
                before.push(format!("tagged {t}"));
            }
            let so_far = if before.is_empty() { String::new() } else { format!("Done so far: {}. ", list(&before)) };
            let (weak, c, target, b) = (cx.weak_entity(), c2.clone(), into.clone(), branch.clone());
            let action = toast::ToastAction {
                label: format!("Merge into {into}").into(),
                run: Rc::new(move |_, cx| {
                    let (c, target, b) = (c.clone(), target.clone(), b.clone());
                    weak.update(cx, |v, cx| v.merge_into(c, target, b, cx)).ok();
                }),
            };
            toast::show_with(format!("Merging {branch} into {into} will conflict in {files} {}", if files == 1 { "file" } else { "files" }), Options { warning: true, sticky: true, detail: Some(format!("{so_far}Make that merge and resolve the conflicts, then finish {branch} again.").into()), actions: vec![action], ..Default::default() }, cx);
        });
        self.write(w, cx);
    }

    /// The merge a finish left: check `into` out, then the usual Merge.
    fn merge_into(&mut self, ctx: WriteCtx, into: String, branch: String, cx: &mut Context<Self>) {
        let on_it = self.tabs.iter().find(|t| t.id == ctx.repo).and_then(Tab::head_branch).as_deref() == Some(into.as_str());
        if on_it {
            return self.start_integrate(ctx, "merge", branch, into, false, crate::ui::arm::Origin::None, cx);
        }
        let target = json!({ "kind": "branch", "name": into });
        let expect = json!({ "head": null, "refs": {} });
        let (c2, i2) = (ctx.clone(), into.clone());
        let then: crate::write::After = Rc::new(move |this: &mut GitVolt, cx: &mut Context<GitVolt>| {
            this.start_integrate(c2.clone(), "merge", branch.clone(), i2.clone(), false, crate::ui::arm::Origin::None, cx);
        });
        self.checkout(ctx, target, expect, None, crate::ui::arm::Origin::None, Some(then), cx);
    }

    /// `Finish feature x…`, in a branch's menu.
    pub fn gitflow_finish_row(&self, name: &str, this: &WeakEntity<GitVolt>) -> Option<MenuRow> {
        let tab = self.tabs.get(self.active)?;
        let flow = tab.flow()?;
        let (kind, rest) = flow.classify(name)?;
        let label = kind_label(kind).to_lowercase();
        let tooltip = if kind == "feature" { format!("Merge {name} into {} and delete it", flow.develop) } else { format!("Merge {name} into {} and {}, tag it, and delete it", flow.master, flow.develop) };
        let (weak, branch) = (this.clone(), name.to_string());
        Some(MenuRow::action("gitflow.finish", format!("Finish {label} {rest}…"), "flag", tooltip, move |window, cx| {
            let branch = branch.clone();
            weak.update(cx, |v, cx| v.open_gitflow_finish(branch, window, cx)).ok();
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn flow() -> Flow {
        default_config(&["master".to_string()])
    }

    #[test]
    fn a_branch_is_classified_by_its_prefix() {
        let f = flow();
        assert_eq!(f.classify("feature/x"), Some(("feature", "x".to_string())));
        assert_eq!(f.classify("release/1.0"), Some(("release", "1.0".to_string())));
        assert_eq!(f.classify("feature/"), None);
        assert_eq!(f.classify("master"), None);
        assert_eq!(f.base("hotfix"), "master");
        assert_eq!(f.base("feature"), "develop");
        assert_eq!(f.targets("release"), ["master", "develop"]);
    }

    #[test]
    fn a_tag_is_suggested_from_a_version() {
        let mut f = flow();
        assert_eq!(suggested_tag(&f, "feature", "x"), None);
        assert_eq!(suggested_tag(&f, "release", "3.0.0").as_deref(), Some("3.0.0"));
        assert_eq!(suggested_tag(&f, "release", "spring").as_deref(), Some(""));
        assert_eq!(suggested_tag(&f, "hotfix", "3.0.0-rc.1").as_deref(), Some("3.0.0-rc.1"));
        f.version_tag = "v".into();
        assert_eq!(suggested_tag(&f, "release", "v3.0").as_deref(), Some("v3.0"));
    }

    #[test]
    fn the_settings_are_checked_as_the_core_does() {
        let mut f = flow();
        assert_eq!(config_error(&f), None);
        f.develop = f.master.clone();
        assert_eq!(config_error(&f).as_deref(), Some("The production and development branches must differ"));
        let mut f = flow();
        f.release = "feature/x".into();
        assert_eq!(config_error(&f).as_deref(), Some("The release and feature prefixes overlap"));
        let mut f = flow();
        f.feature = "dev".into();
        assert_eq!(config_error(&f).as_deref(), Some("The feature prefix would make develop a feature branch"));
    }

    #[test]
    fn a_start_name_says_what_the_core_would_refuse() {
        let f = flow();
        let none = |_: &str| false;
        assert_eq!(start_name_error(&f, "feature", "", &none).as_deref(), Some("Enter a feature name"));
        assert_eq!(start_name_error(&f, "release", "", &none).as_deref(), Some("Enter a version"));
        assert_eq!(start_name_error(&f, "feature", "ok", &none), None);
        assert_eq!(start_name_error(&f, "feature", "ok", &|b| b == "feature/ok").as_deref(), Some("A branch named feature/ok already exists"));
    }

    #[test]
    fn a_finish_says_what_it_did() {
        assert_eq!(finished_text("release/1", &json!({ "merged": ["main", "develop"], "tag": "1", "deleted": true })), "Finished release/1: merged into main and develop, tagged 1, and deleted");
        assert_eq!(finished_text("feature/x", &json!({ "merged": ["develop"], "tag": null, "deleted": true })), "Finished feature/x: merged into develop and deleted");
        assert_eq!(finished_text("feature/x", &json!({ "merged": [], "deleted": false })), "feature/x is already finished");
    }
}
