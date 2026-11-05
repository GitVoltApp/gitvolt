//! The overlay: `SettingsOverlay.tsx`, `Nav.tsx`, `SearchResults.tsx`, `SettingRow.tsx`, `fields.tsx`, `settings.css`.

use super::pages::{self, Binding, Ctl, Def, NumberSpec, Page, Scope};
use crate::app::{GitVolt, icon};
use crate::theme;
use crate::ui::controls::{self, OnSelect};
use crate::ui::modal::{CloseModal, FocusNext, FocusPrev};
use crate::ui::tooltip::tip;
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::ops::Range;
use std::rc::Rc;

/// A control button (`.setting-control button`).
pub fn plain_button(id: impl Into<ElementId>, label: impl Into<SharedString>, enabled: bool) -> Stateful<Div> {
    div().id(id).flex().flex_none().items_center().px(px(12.)).py(px(5.)).rounded(px(4.)).border_1().border_color(theme::section_border()).bg(theme::panel_bg1()).text_color(if enabled { theme::text_selected() } else { theme::text_disabled() }).when(enabled, |d| d.cursor_pointer().hover(|d| d.bg(theme::hover_row()))).child(label.into())
}

pub fn input_box(id: impl Into<ElementId>, input: Entity<crate::ui::input::TextInput>, invalid: bool, window: &Window, cx: &App) -> Stateful<Div> {
    let focused = input.read(cx).is_focused(window);
    div().id(id).flex().flex_1().min_w_0().items_center().h(px(28.)).px(px(8.)).rounded(px(4.)).bg(theme::app_bg()).border_1().border_color(if invalid { theme::status_deleted() } else if focused { theme::blue() } else { theme::section_border() }).text_color(theme::text_selected()).overflow_hidden().child(input)
}

pub fn marked(text: &str, ranges: &[Range<usize>]) -> AnyElement {
    if ranges.is_empty() {
        return div().child(text.to_string()).into_any_element();
    }
    let style = HighlightStyle { background_color: Some(Hsla { a: 0.3, ..theme::yellow() }), ..Default::default() };
    StyledText::new(text.to_string()).with_highlights(ranges.iter().map(|r| (r.clone(), style))).into_any_element()
}

fn scope_label(scope: Scope, this: &GitVolt) -> String {
    match scope {
        Scope::App => "GitVolt".into(),
        Scope::Profile => {
            let name = this.profile["name"].as_str().unwrap_or_default().trim();
            if name.is_empty() { "Profile".into() } else { format!("Profile · {name}") }
        }
        Scope::Repo => format!("Repository · {}", this.tabs.get(this.active).filter(|t| !t.is_open_tab()).map_or(String::new(), |t| t.name.to_string())),
    }
}

/// `Binding`'s value.
pub fn bound(this: &GitVolt, b: &Binding) -> Value {
    match b {
        Binding::None => Value::Null,
        Binding::App(k, _, _) => this.settings[*k].clone(),
        Binding::Profile(k, _) => this.profile[*k].clone(),
        Binding::Repo(k, _, _) => this.tabs.get(this.active).map_or(Value::Null, |t| this.profile["repos"][&t.repo_path][*k].clone()),
        Binding::Local(k, _) => this.prefs[*k].clone(),
    }
}

pub fn default_of(b: &Binding) -> Value {
    match b {
        Binding::None => Value::Null,
        Binding::App(_, d, _) | Binding::Profile(_, d) | Binding::Repo(_, d, _) | Binding::Local(_, d) => (d)(),
    }
}

pub fn modified(this: &GitVolt, def: &Def) -> bool {
    if matches!(def.binding, Binding::None) {
        return false;
    }
    if def.id == "fetchInterval" {
        return crate::feedback::clamp_fetch_interval(this.settings["fetchIntervalSecs"].as_f64().unwrap_or(60.)) != 60;
    }
    bound(this, &def.binding) != default_of(&def.binding)
}

/// Writes through the binding, and runs what the change has to do.
pub fn set_bound(this: &mut GitVolt, b: &Binding, value: Value, cx: &mut Context<GitVolt>) {
    match b {
        Binding::None => {}
        Binding::App(k, _, after) => {
            let k = *k;
            this.set_settings(move |s| s[k] = value, cx);
            if let Some(after) = after {
                after(this, cx);
            }
        }
        Binding::Profile(k, _) => {
            let k = *k;
            this.update_profile(move |p| p[k] = value, cx);
        }
        Binding::Repo(k, _, after) => {
            let k = *k;
            let Some(path) = this.tabs.get(this.active).map(|t| t.repo_path.clone()) else { return };
            this.update_repo(&path, move |r| r[k] = value, cx);
            if let Some(after) = after {
                after(this, cx);
            }
        }
        Binding::Local(k, _) => {
            let k = *k;
            this.prefs[k] = value;
            this.save_layout();
            this.settings_changed(cx);
            cx.notify();
        }
    }
}

fn reset(this: &mut GitVolt, def: &Def, cx: &mut Context<GitVolt>) {
    let d = default_of(&def.binding);
    set_bound(this, &def.binding, d, cx);
}

/// `resetAppSettings`: every setting on an app-scope page that has a binding.
pub fn reset_app_settings(this: &mut GitVolt, cx: &mut Context<GitVolt>) {
    for def in pages::defs().iter() {
        if pages::page(def.page).is_some_and(|p| p.scope == Scope::App) && modified(this, def) {
            reset(this, def, cx);
        }
    }
}

fn number_control(this: &mut GitVolt, def: &'static Def, spec: NumberSpec, window: &Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let value = (spec.get)(this);
    let shown = spec.shown(value);
    let id = def.id;
    let commit: super::Commit = Rc::new(move |v, text, cx| {
        let def = pages::def(id).expect("a setting");
        let Ctl::Number(spec) = &def.control else { return };
        let now = (spec.get)(v);
        if let Some(n) = spec.committed(text, now)
            && n != now
        {
            (spec.set)(v, n, cx);
        }
        // Back to the applied value when what's typed isn't one.
        let applied = spec.shown((spec.get)(v));
        if let Some(f) = v.settings_ui.fields.get_mut(id) {
            f.shown = "\0".into();
            let _ = applied;
        }
        cx.notify();
    });
    let input = this.settings_field(id, &shown, spec.placeholder, commit, None, window, cx);
    let focus = input.read(cx).focus.clone();
    let step_button = |dir: f64, cx: &mut Context<GitVolt>| {
        let at_limit = spec.at_limit(dir, value);
        let name = if dir > 0. { "plus" } else { "minus" };
        let label = format!("{} {}", if dir > 0. { "Increase" } else { "Decrease" }, pages::lower_first(def.label));
        tip(
            label,
            crate::ui::anchor(
                format!("setting-step:{id}:{name}"),
                div()
                    .id(ElementId::Name(format!("{id}-{name}").into()))
                    .flex()
                    .flex_none()
                    .w(px(26.))
                    .items_center()
                    .justify_center()
                    .when(!at_limit, |d| d.cursor_pointer().hover(|d| d.bg(theme::hover_row())))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.number_press(id, dir, cx);
                    }))
                    .child(icon(name, 12., if at_limit { theme::text_disabled() } else { theme::text_secondary() })),
            ),
        )
    };
    let minus = step_button(-1., cx);
    let plus = step_button(1., cx);
    let is_focused = focus.is_focused(window);
    div()
        .id(ElementId::Name(format!("{id}-number").into()))
        .flex()
        .flex_none()
        .w(px(132.))
        .h(px(28.))
        .rounded(px(4.))
        .bg(theme::app_bg())
        .border_1()
        .border_color(if is_focused { theme::blue() } else { theme::section_border() })
        .text_color(theme::text_selected())
        .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
            let k = &e.keystroke;
            if matches!(k.key.as_str(), "up" | "down") && !k.modifiers.control && !k.modifiers.alt {
                cx.stop_propagation();
                this.number_step(id, if k.key == "up" { 1. } else { -1. }, if k.modifiers.shift { 10. } else { 1. }, cx);
            }
        }))
        .on_scroll(cx.listener(move |this, e: &ScrollEvent, window, cx| {
            let focused = this.settings_ui.fields.get(id).is_some_and(|f| f.input.read(cx).is_focused(window));
            let dy = e.delta.pixel_delta(px(16.)).y.as_f32();
            if focused && dy != 0. && !e.modifiers.control {
                cx.stop_propagation();
                this.number_step(id, if dy > 0. { 1. } else { -1. }, 1., cx);
            }
        }))
        .child(minus)
        .child(div().flex().flex_1().min_w_0().items_center().justify_center().overflow_hidden().child(input))
        .child(plus)
        .into_any_element()
}

impl GitVolt {
    /// A step of a number field: applied at once, clamped; what's typed is where it starts.
    pub fn number_step(&mut self, id: &'static str, dir: f64, times: f64, cx: &mut Context<Self>) {
        let Some(def) = pages::def(id) else { return };
        let Ctl::Number(spec) = &def.control else { return };
        let now = (spec.get)(self);
        let typed = self.settings_ui.fields.get(id).map_or(String::new(), |f| f.input.read(cx).text().to_string());
        let start = spec.blank_start.map_or(spec.min, |f| f(self));
        let next = spec.stepped(&typed, now, dir, times, start);
        if next != now {
            (spec.set)(self, next, cx);
        }
        if let Some(f) = self.settings_ui.fields.get_mut(id) {
            let text = spec.shown(next);
            f.shown = text.clone();
            f.input.clone().update(cx, |i, cx| i.set_text(&text, cx));
        }
        cx.notify();
    }

    /// A press on − or +: a step now, then repeats until the release.
    fn number_press(&mut self, id: &'static str, dir: f64, cx: &mut Context<Self>) {
        self.number_step(id, dir, 1., cx);
        self.settings_ui.hold += 1;
        let hold = self.settings_ui.hold;
        cx.spawn(async move |this, cx| {
            let mut delay = 400;
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(delay)).await;
                delay = 60;
                let go = this.update(cx, |v, cx| {
                    if v.settings_ui.hold != hold {
                        return false;
                    }
                    let Some(def) = pages::def(id) else { return false };
                    let Ctl::Number(spec) = &def.control else { return false };
                    let now = (spec.get)(v);
                    if spec.at_limit(dir, now) {
                        return false;
                    }
                    v.number_step(id, dir, 1., cx);
                    true
                });
                if !matches!(go, Ok(true)) {
                    return;
                }
            }
        })
        .detach();
    }

    /// What's at a release: a held button stops, drags end.
    pub fn settings_release(&mut self, cx: &mut Context<Self>) {
        self.settings_ui.hold += 1;
        self.settings_ui.scroll_drag = None;
        self.settings_ui.float_drag = None;
        self.settings_ui.file_colours.drag = None;
        self.settings_ui.presets.drag = None;
        self.end_lane_drag(cx);
    }
}

fn switch_control(on: bool, def: &'static Def, cx: &mut Context<GitVolt>) -> AnyElement {
    crate::ui::anchor(
        format!("setting:{}", def.id),
        div()
            .id(ElementId::Name(format!("input-{}", def.id).into()))
            .flex()
            .flex_none()
            .p(px(4.))
            .rounded(px(6.))
            .cursor_pointer()
            .hover(|d| d.bg(theme::control_bg()))
            .on_click(cx.listener(move |this, _, _, cx| {
                set_bound(this, &def.binding, json!(!on), cx);
            }))
            .child(controls::switch_track(on, false)),
    )
    .into_any_element()
}

fn select_control(this: &GitVolt, def: &'static Def, options: Vec<(Value, String)>, cx: &mut Context<GitVolt>) -> AnyElement {
    let value = bound(this, &def.binding);
    let weak = cx.weak_entity();
    let opts: controls::SelectOptions<Value> = Rc::new(options.into_iter().map(|(v, l)| (v, SharedString::from(l), None)).collect());
    let on_change: OnSelect<Value> = Rc::new(move |v, _, cx| {
        let v = v.clone();
        weak.update(cx, |this, cx| set_bound(this, &def.binding, v, cx)).ok();
    });
    let shown = if opts.iter().any(|o| o.0 == value) { None } else { opts.first().map(|o| o.1.clone()) };
    let chosen = if opts.iter().any(|o| o.0 == value) { value } else { opts.first().map_or(Value::Null, |o| o.0.clone()) };
    div().flex().flex_none().w(px(240.)).child(controls::select(format!("setting:{}", def.id), chosen, opts, shown, on_change, cx)).into_any_element()
}

/// A setting's row.
pub fn setting_row(this: &mut GitVolt, def: &'static Def, first: bool, words: &[String], window: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let is_modified = modified(this, def);
    let flash = this.settings_ui.flash.as_deref() == Some(def.id);
    let control: AnyElement = match &def.control {
        Ctl::Switch => switch_control(bound(this, &def.binding) == json!(true), def, cx),
        Ctl::Select(options) => {
            let list = options(this);
            select_control(this, def, list, cx)
        }
        Ctl::Number(spec) => number_control(this, def, *spec, window, cx),
        Ctl::Custom(f) => f(this, def, window, cx),
        Ctl::OpenPage => plain_button(ElementId::Name(format!("open-{}", def.id).into()), "Open page", true).on_click(cx.listener(move |this, _, _, cx| {
            this.settings_ui.page = def.page.to_string();
            cx.notify();
        })).into_any_element(),
    };
    let font_row = def.id.ends_with("FontFamily");
    let (label, desc) = (crate::settings::search::highlights(def.label, words), crate::settings::search::highlights(&def.description, words));
    let restore = tip(
        "Restore the default",
        div()
            .id(ElementId::Name(format!("reset-{}", def.id).into()))
            .flex()
            .flex_none()
            .size(px(18.))
            .items_center()
            .justify_center()
            .rounded(px(3.))
            .when(!is_modified, |d| d.invisible())
            .cursor_pointer()
            .hover(|d| d.bg(theme::hover_row()))
            .on_click(cx.listener(move |this, _, _, cx| reset(this, def, cx)))
            .child(icon("rotate-ccw", 12., theme::text_secondary())),
    );
    let text = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(div().flex().items_center().gap(px(4.)).min_h(px(22.)).child(div().text_color(theme::text_selected()).child(marked(def.label, &label))).child(restore))
        .when(!def.description.is_empty(), |d| d.child(div().mt(px(2.)).text_size(px(12.)).text_color(theme::text_secondary()).child(marked(&def.description, &desc))));
    let (text, control) = if font_row { super::fonts::font_row_parts(this, def, text, control, window, cx) } else { (text, control) };
    crate::ui::anchor(
        format!("setting-row:{}", def.id),
        div()
            .id(ElementId::Name(format!("setting-{}", def.id).into()))
            .flex()
            .items_center()
            .gap(px(16.))
            .px(px(14.))
            .py(px(10.))
            .when(font_row, |d| d.items_start())
            .when(!first, |d| d.border_t_1().border_color(theme::section_border()))
            .when(flash, |d| d.bg(theme::selected_row()))
            .child(text)
            .child(div().flex().flex_none().w(px(if font_row { 320. } else { 380. })).min_h(px(28.)).items_center().justify_end().flex_wrap().gap(px(8.)).child(control)),
    )
    .into_any_element()
}

pub fn group_title(title: &str, hint: Option<&str>) -> Div {
    div().mb(px(6.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_secondary()).child(title.to_uppercase()).when_some(hint, |d, h| d.child(div().font_weight(FontWeight::NORMAL).child(format!(" · {h}"))).flex().flex_row())
}

pub fn card() -> Div {
    div().flex().flex_col().bg(theme::panel_bg()).border_1().border_color(theme::section_border()).rounded(px(6.))
}

/// `PageGroups`: a page's settings as rows, in titled groups, in registration order; `skip` leaves groups out.
pub fn page_groups(this: &mut GitVolt, page_id: &str, skip: &[&str], window: &mut Window, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    let mut groups: Vec<(&'static str, Vec<&'static Def>)> = Vec::new();
    for d in pages::defs().iter().filter(|d| d.page == page_id && !skip.contains(&d.group)) {
        match groups.iter_mut().find(|g| g.0 == d.group) {
            Some(g) => g.1.push(d),
            None => groups.push((d.group, vec![d])),
        }
    }
    groups
        .into_iter()
        .map(|(group, defs)| {
            let rows: Vec<AnyElement> = defs.iter().enumerate().map(|(i, d)| setting_row(this, d, i == 0, &[], window, cx)).collect();
            div().mt(px(20.)).child(group_title(group, None)).child(card().children(rows)).into_any_element()
        })
        .collect()
}

fn page_view(this: &mut GitVolt, page: &'static Page, window: &mut Window, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    let summary = (page.summary)(this);
    let mut out = vec![page_head(page.label, Some(summary))];
    match page.body {
        Some(body) => out.extend(body(this, window, cx)),
        None => out.extend(page_groups(this, page.id, &[], window, cx)),
    }
    out
}

fn page_head(title: impl Into<SharedString>, summary: Option<String>) -> AnyElement {
    div().mr(px(32.)).child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).child(title.into())).when_some(summary, |d, s| d.child(div().mt(px(4.)).text_color(theme::text_secondary()).child(s))).into_any_element()
}

fn results_view(this: &mut GitVolt, query: &str, listed: &[&'static Page], window: &mut Window, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    let words = super::search::words(query);
    let q = query.trim();
    let mut groups: Vec<(&'static Page, &'static str, Vec<&'static Def>)> = Vec::new();
    let mut n = 0;
    for page in listed {
        for d in pages::defs().iter().filter(|d| d.page == page.id && super::search::matches(&words, d.label, &d.description, d.keywords)) {
            n += 1;
            match groups.last_mut() {
                Some(g) if g.0.id == page.id && g.1 == d.group => g.2.push(d),
                _ => groups.push((page, d.group, vec![d])),
            }
        }
    }
    let title = match n {
        0 => format!("No settings match \"{q}\""),
        1 => format!("1 setting matches \"{q}\""),
        n => format!("{n} settings match \"{q}\""),
    };
    let mut out = vec![page_head(title, (n > 0).then(|| "Change them here, or open their page.".to_string()))];
    for (page, group, defs) in groups {
        let first = defs[0].id;
        let crumb = div()
            .flex()
            .items_baseline()
            .justify_between()
            .gap(px(12.))
            .mb(px(6.))
            .text_size(px(11.))
            .text_color(theme::text_secondary())
            .child(div().flex().child(format!("{} › ", scope_label(page.scope, this))).child(div().font_weight(FontWeight::SEMIBOLD).text_color(theme::text_normal()).child(page.label)).child(format!(" › {group}")))
            .child(div().id(ElementId::Name(format!("crumb-{first}").into())).text_color(theme::link()).cursor_pointer().hover(|d| d.underline()).on_click(cx.listener(move |this, _, window, cx| this.show_settings(Some(first), window, cx))).child("Open page"));
        let rows: Vec<AnyElement> = defs.iter().enumerate().map(|(i, d)| setting_row(this, d, i == 0, &words, window, cx)).collect();
        out.push(div().mt(px(20.)).child(crumb).child(card().children(rows)).into_any_element());
    }
    out
}

fn nav(this: &GitVolt, listed: &[&'static Page], current: &str, counts: Option<&[(&'static str, usize)]>, cx: &mut Context<GitVolt>) -> AnyElement {
    let mut list = div().id("settings-nav").flex().flex_col().gap(px(10.)).flex_1().min_h_0().overflow_y_scroll();
    let mut index = 0;
    for scope in [Scope::App, Scope::Profile, Scope::Repo] {
        let pages: Vec<&&'static Page> = listed.iter().filter(|p| p.scope == scope).collect();
        if pages.is_empty() {
            continue;
        }
        let mut group = div().flex().flex_col().gap(px(1.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .py(px(4.))
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text_secondary())
                .when(scope == Scope::Profile, |d| d.child(div().flex_none().size(px(7.)).rounded_full().bg(crate::profiles::profile_color(this))))
                .child(div().truncate().child(scope_label(scope, this))),
        );
        for p in pages {
            let count = counts.map(|c| c.iter().find(|x| x.0 == p.id).map_or(0, |x| x.1));
            let is_current = p.id == current;
            let cursor = this.settings_ui.nav_cursor == Some(index);
            let id = p.id;
            index += 1;
            group = group.child(crate::ui::anchor(
                format!("settings-nav:{id}"),
                div()
                    .id(ElementId::Name(format!("settings-nav-{id}").into()))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .when(count == Some(0), |d| d.opacity(0.45))
                    .when(is_current, |d| d.bg(theme::selected_row()).text_color(theme::text_selected()))
                    .when(!is_current, |d| d.text_color(theme::text_normal()).hover(|d| d.bg(theme::hover_row())))
                    .when(cursor, |d| d.border_2().border_color(theme::focus_outline()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings_set_page(id, cx);
                    }))
                    .child(icon(p.icon, 14., theme::sidebar_icon()))
                    .child(div().flex_1().min_w_0().truncate().child(p.label))
                    .when_some(count.filter(|c| *c > 0), |d, c| d.child(div().flex_none().text_size(px(11.)).text_color(theme::count_blue()).child(c.to_string()))),
            ));
        }
        list = list.child(group);
    }
    list.into_any_element()
}

impl GitVolt {
    pub fn settings_set_page(&mut self, id: &str, cx: &mut Context<Self>) {
        self.settings_ui.page = id.to_string();
        self.settings_ui.flash = None;
        self.settings_ui.scroll = ScrollHandle::new();
        if let Some(s) = self.settings_ui.search.clone() {
            s.update(cx, |i, cx| i.set_text("", cx));
        }
        cx.notify();
    }

    /// The pages this tab shows: Repository's only with a repository.
    pub fn settings_pages(&self) -> Vec<&'static Page> {
        let has_repo = self.tabs.get(self.active).is_some_and(|t| !t.is_open_tab());
        pages::pages().iter().filter(|p| p.scope != Scope::Repo || has_repo).collect()
    }

    /// The overlay's own keys, while no field has them.
    fn settings_key(&mut self, k: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let listed = self.settings_pages();
        let m = &k.modifiers;
        if m.control || m.alt || m.platform {
            return false;
        }
        let at = self.settings_ui.nav_cursor.or_else(|| listed.iter().position(|p| p.id == self.settings_ui.page)).unwrap_or(0);
        let to = match k.key.as_str() {
            "down" => Some(at + 1),
            "up" => Some(at.saturating_sub(1)),
            "home" => Some(0),
            "end" => Some(listed.len() - 1),
            _ => None,
        };
        if let Some(to) = to {
            self.settings_ui.nav_cursor = Some(to.min(listed.len() - 1));
            cx.notify();
            return true;
        }
        if matches!(k.key.as_str(), "enter" | "space") && let Some(i) = self.settings_ui.nav_cursor {
            let id = listed[i.min(listed.len() - 1)].id;
            self.settings_set_page(id, cx);
            return true;
        }
        if let Some(c) = k.key_char.as_deref().filter(|c| c.chars().count() == 1 && k.key != "enter" && k.key != "tab") {
            let search = self.settings_search(cx);
            let text = format!("{}{c}", search.read(cx).text());
            search.update(cx, |i, cx| i.set_text(&text, cx));
            let f = search.read(cx).focus.clone();
            window.focus(&f, cx);
            cx.notify();
            return true;
        }
        false
    }

    /// The chords it owns while open (`app_key` asks first). Every other chord stops here.
    pub fn settings_chord(&mut self, combo: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        use crate::commands::resolve_chord;
        if combo == resolve_chord("Mod+,") {
            self.close_settings(window, cx);
        } else if combo == resolve_chord("Mod+F") {
            let search = self.settings_search(cx);
            search.update(cx, |i, cx| i.select_all_text(cx));
            let f = search.read(cx).focus.clone();
            window.focus(&f, cx);
        } else if combo == resolve_chord("Mod+V") && !crate::ui::input::is_typing(window) {
            self.settings_paste(cx);
        } else {
            return false;
        }
        true
    }
}

pub const SETTINGS_W: f32 = 1120.;

/// The overlay. `None`: it's closed.
pub fn settings_overlay(this: &mut GitVolt, window: &mut Window, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    if !this.settings_ui.open {
        return None;
    }
    if this.settings_ui.floating {
        return Some(super::themes::floating_editor(this, window, cx));
    }
    let viewport = window.viewport_size();
    let (vw, vh) = (viewport.width.as_f32(), viewport.height.as_f32());
    let listed = this.settings_pages();
    let page = listed.iter().copied().find(|p| p.id == this.settings_ui.page).or_else(|| listed.iter().copied().find(|p| p.id == "general")).unwrap_or(listed[0]);
    let query = this.settings_query(cx);
    let searching = !query.trim().is_empty();
    let search = this.settings_search(cx);
    let counts: Option<Vec<(&'static str, usize)>> = searching.then(|| {
        let words = super::search::words(&query);
        listed.iter().map(|p| (p.id, pages::defs().iter().filter(|d| d.page == p.id && super::search::matches(&words, d.label, &d.description, d.keywords)).count())).collect()
    });
    let body: Vec<AnyElement> = if searching { results_view(this, &query, &listed, window, cx) } else { page_view(this, page, window, cx) };
    let focus = this.settings_ui.focus.get_or_insert_with(|| cx.focus_handle()).clone();
    let keycap = (query.is_empty()).then(|| div().absolute().right(px(6.)).top(px(6.)).child(controls::keycap_one(&crate::commands::display_chord("Mod+F"))));
    let weak = cx.weak_entity();
    let search_box = {
        let field = search.read(cx);
        let (empty, focused) = (field.text().is_empty(), field.is_focused(window));
        let w2 = weak.clone();
        div()
            .relative()
            .child(
                div()
                    .id("settings-search")
                    .relative()
                    .flex()
                    .items_center()
                    .h(px(30.))
                    .pl(px(8.))
                    .pr(px(24.))
                    .rounded(px(4.))
                    .bg(theme::app_bg())
                    .border_1()
                    .border_color(if focused { theme::blue() } else { theme::section_border() })
                    .text_color(theme::text_selected())
                    .child(search.clone())
                    .when(!empty, |d| {
                        d.child(div().id("settings-search-clear").absolute().right(px(3.)).top(px(4.)).p(px(2.)).rounded(px(3.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).on_click(move |_, _, cx| {
                            w2.update(cx, |v, cx| {
                                if let Some(s) = v.settings_ui.search.clone() {
                                    s.update(cx, |i, cx| i.set_text("", cx));
                                }
                                cx.notify();
                            })
                            .ok();
                        }).child(icon("x", 12., theme::text_secondary())))
                    }),
            )
            .children(keycap)
    };
    let scroll = this.settings_ui.scroll.clone();
    let panel = div()
        .id("settings-dialog")
        .occlude()
            .child(crate::ui::spin::cover())
        .relative()
        .flex()
        .flex_none()
        .w(px(SETTINGS_W.min(vw - 48.)))
        .h(px(vh - 48.))
        .bg(theme::panel_bg1())
        .border_1()
        .border_color(theme::section_border())
        .rounded(px(8.))
        .overflow_hidden()
        .shadow(crate::ui::shadow(theme::shadow_modal(), 10., 32.))
        .text_color(theme::text_normal())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap(px(10.))
                .w(px(if vw <= 760. { 180. } else { 220. }))
                .h_full()
                .min_h_0()
                .px(px(8.))
                .py(px(12.))
                .bg(theme::panel_bg())
                .border_r_1()
                .border_color(theme::section_border())
                .child(search_box)
                .child(nav(this, &listed, if searching { "" } else { page.id }, counts.as_deref(), cx)),
        )
        .child(
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(div().id("settings-body").size_full().overflow_y_scroll().track_scroll(&scroll).px(px(28.)).pt(px(18.)).pb(px(28.)).children(body))
                .child(body_scrollbar(this, cx)),
        )
        .child(tip(
            "Close (Esc)",
            crate::ui::anchor(
                "settings-close",
                div().id("settings-close").absolute().top(px(12.)).right(px(12.)).flex().size(px(22.)).items_center().justify_center().rounded(px(3.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).on_click(cx.listener(|this, _, window, cx| this.close_settings(window, cx))).child(icon("x", 14., theme::text_secondary())),
            ),
        ));
    Some(
        div()
            .id("settings-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .child(crate::ui::spin::cover())
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::backdrop())
            .key_context("Modal")
            .track_focus(&focus)
            .on_action(cx.listener(|this, _: &CloseModal, window, cx| {
                let query = this.settings_query(cx);
                if query.is_empty() {
                    this.close_settings(window, cx);
                } else if let Some(s) = this.settings_ui.search.clone() {
                    s.update(cx, |i, cx| i.set_text("", cx));
                    cx.notify();
                }
            }))
            .on_action(|_: &FocusNext, window, cx| window.focus_next(cx))
            .on_action(|_: &FocusPrev, window, cx| window.focus_prev(cx))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_settings(window, cx)))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if crate::ui::input::is_typing(window) {
                    return;
                }
                if this.settings_key(&e.keystroke, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(panel)
            .child(release_listener(cx))
            .into_any_element(),
    )
}

/// Drags and a held button end wherever the release lands, and follow the pointer wherever it goes.
pub fn release_listener(cx: &mut Context<GitVolt>) -> AnyElement {
    let (up, moved) = (cx.weak_entity(), cx.weak_entity());
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    up.update(cx, |this, cx| this.settings_release(cx)).ok();
                }
            });
            window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture && e.pressed_button.is_some() {
                    moved.update(cx, |this, cx| this.on_settings_drag(e, cx)).ok();
                }
            });
        },
    )
    .absolute()
    .size_0()
    .into_any_element()
}

/// The slim thumb over the page's right edge.
fn body_scrollbar(this: &GitVolt, cx: &mut Context<GitVolt>) -> AnyElement {
    let handle = this.settings_ui.scroll.clone();
    let (view, offset, max) = (handle.bounds().size.height.as_f32(), -handle.offset().y.as_f32(), handle.max_offset().y.as_f32());
    if max <= 0. || view <= 0. {
        return div().into_any_element();
    }
    let content = view + max;
    let thumb_h = (view * view / content).max(24.);
    let top = (view - thumb_h) * (offset / max);
    div()
        .id("settings-thumb")
        .absolute()
        .top(px(top))
        .right(px(0.))
        .w(px(10.))
        .h(px(thumb_h))
        .child(div().mx(px(2.)).my(px(2.)).h(px(thumb_h - 4.)).rounded(px(4.)).bg(if this.settings_ui.scroll_drag.is_some() { theme::scroll_thumb_hover() } else { theme::scroll_thumb() }).hover(|d| d.bg(theme::scroll_thumb_hover())))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            this.settings_ui.scroll_drag = Some((e.position.y.as_f32(), offset));
            cx.notify();
        }))
        .into_any_element()
}

impl GitVolt {
    fn on_settings_drag(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        self.on_lane_drag(e, cx);
        self.on_float_drag(e, cx);
        let Some((from, start)) = self.settings_ui.scroll_drag else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            self.settings_ui.scroll_drag = None;
            return;
        }
        let handle = self.settings_ui.scroll.clone();
        let (view, max) = (handle.bounds().size.height.as_f32(), handle.max_offset().y.as_f32());
        if max <= 0. {
            return;
        }
        let thumb = (view * view / (view + max)).max(24.);
        let per = max / (view - thumb).max(1.);
        let to = (start + (e.position.y.as_f32() - from) * per).clamp(0., max);
        handle.set_offset(point(handle.offset().x, px(-to)));
        cx.notify();
    }
}
