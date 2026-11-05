//! The MR/PR view (`mrview/`): the tab's left flyout, over the graph or docked beside it. It shows what's loaded at once, loads the rest,
//! and the poller keeps it fresh. What a body says goes through `crate::md`.

pub mod actions;
pub mod checkout;
pub mod compare;
pub mod header;
pub mod merge;
pub mod people;
pub mod system_note;
pub mod thread;

use super::section::state_icon;
use super::store::Refreshing;
use super::text::{mr_name, mr_ref};
use crate::app::{GitVolt, Tab, core_repo, icon};
use crate::md::field::{FieldEvent, FieldOptions, MarkdownField, PeopleSource};
use crate::md::links::{LinkEnv, Project};
use crate::md::view::{Content, MdEvent, MdOptions, MdView};
use crate::theme;
use crate::ui::tooltip::tip;
use gitvolt_core::forge::types::{ForgeMrDetail, ForgeMr};
use gitvolt_core::forge::ForgeKind;
use gpui::prelude::*;
use gpui::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub const FLYOUT_MIN: f32 = 360.;
pub const FLYOUT_DEFAULT: f32 = 560.;
pub const FLYOUT_MAX: f32 = 760.;
pub const CENTER_KEEP: f32 = 240.;
pub const DOCK_DEFAULT: f32 = 480.;
pub const DOCK_KEEP: f32 = 640.;

/// `flyoutWidth`: within its bounds and leaving `CENTER_KEEP` beside it.
pub fn flyout_width(preferred: Option<f32>, room: f32) -> f32 {
    let want = preferred.unwrap_or(FLYOUT_DEFAULT).clamp(FLYOUT_MIN, FLYOUT_MAX);
    want.min(room - CENTER_KEEP).max(FLYOUT_MIN.min(room)).round()
}

pub fn can_dock(room: f32) -> bool {
    room - DOCK_KEEP >= FLYOUT_MIN
}

/// `dockWidth`.
pub fn dock_width(preferred: Option<f32>, room: f32) -> f32 {
    let want = preferred.unwrap_or(DOCK_DEFAULT).clamp(FLYOUT_MIN, FLYOUT_MAX);
    want.min(room - DOCK_KEEP).max(FLYOUT_MIN).round()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    None,
    Review,
    Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityTab {
    Activity,
    Comments,
    Diff,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum RangeState {
    #[default]
    Loading,
    Ready { commits: Option<Vec<(String, String)>>, files: u64, added: u64, deleted: u64 },
    None,
}

/// A view's own state: it outlives closing the view, so a half-written reply stays for the session.
pub struct MrUi {
    pub mode: Mode,
    pub busy: Option<actions::Busy>,
    pub scroll: ScrollHandle,
    pub tab: ActivityTab,
    pub comment: Option<Entity<MarkdownField>>,
    pub replies: HashMap<String, Entity<MarkdownField>>,
    pub reply_open: HashSet<String>,
    pub reply_busy: Option<String>,
    pub note_edit: HashMap<String, Entity<MarkdownField>>,
    pub note_busy: Option<String>,
    pub bodies: HashMap<String, (String, Entity<MdView>)>,
    pub folds: HashMap<String, bool>,
    pub reaction_picker: Option<thread::PickerAt>,
    pub stats: RangeState,
    pub stats_for: Option<(String, String, u64)>,
    pub commits_open: bool,
    pub comparing: bool,
    pub rechecking: bool,
    pub missed: bool,
    pub review: actions::ReviewUi,
    pub edit: Option<actions::EditUi>,
    pub merge: merge::MergeUi,
    pub picker: Option<people::Pop>,
}

impl Default for MrUi {
    fn default() -> Self {
        Self { mode: Mode::None, busy: None, scroll: ScrollHandle::new(), tab: ActivityTab::Activity, comment: None, replies: HashMap::new(), reply_open: HashSet::new(), reply_busy: None, note_edit: HashMap::new(), note_busy: None, bodies: HashMap::new(), folds: HashMap::new(), reaction_picker: None, stats: RangeState::Loading, stats_for: None, commits_open: false, comparing: false, rechecking: false, missed: false, review: Default::default(), edit: None, merge: Default::default(), picker: None }
    }
}

/// The project a body's links resolve against.
pub fn link_env(tab: &Tab) -> LinkEnv {
    let Some(kind) = tab.forge.kind else { return LinkEnv::default() };
    let md_kind = match kind {
        ForgeKind::GitLab => crate::md::links::ForgeKind::Gitlab,
        ForgeKind::GitHub => crate::md::links::ForgeKind::Github,
        ForgeKind::Forgejo => crate::md::links::ForgeKind::Forgejo,
    };
    let project = tab.forge.project.as_ref().map(|p| Project { path: p.path.clone(), web_url: p.web_url.trim_end_matches('/').to_string(), host: p.host.clone(), default_branch: p.default_branch.clone(), id: Some(p.id) });
    let graph = tab.graph.clone();
    let users = tab.forge.remote.clone().map(|r| (crate::app::core_repo(tab.id), r));
    LinkEnv { users, kind: Some(md_kind), project, full_sha: Some(std::rc::Rc::new(move |prefix: &str| graph.as_ref()?.by_sha.keys().find(|s| s.starts_with(prefix)).map(|s| s.to_string()))) }
}

pub fn flavor_of(kind: ForgeKind) -> crate::md::ast::Flavor {
    match kind {
        ForgeKind::GitLab => crate::md::ast::Flavor::Gitlab,
        ForgeKind::GitHub => crate::md::ast::Flavor::Github,
        ForgeKind::Forgejo => crate::md::ast::Flavor::Forgejo,
    }
}

impl GitVolt {
    /// `openMrView`: the tab's view of this MR/PR, on top. Already open on it: left as it is.
    pub fn forge_open_mr(&mut self, tab: u32, number: u64, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        if t.forge.open_mr == Some(number) {
            cx.notify();
            return;
        }
        if let Some(old) = t.forge.open_mr.take()
            && let Some(ui) = t.mr_views.get_mut(&old)
        {
            ui.bodies.clear();
        }
        t.create = None;
        t.forge.open_mr = Some(number);
        t.mr_views.entry(number).or_default();
        // A beat of grace: arrowing down the sidebar's list opens each row in turn; only the one it rests on loads.
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(120)).await;
            weak.update(cx, |v, cx| {
                if v.tabs.iter().find(|t| t.id == tab).is_some_and(|t| t.forge.open_mr == Some(number)) {
                    v.forge_refresh_mr(tab, number, cx);
                    v.review_resume(tab, number, cx);
                }
            })
            .ok();
        })
        .detach();
        self.forge_after_open(tab, number, cx);
        cx.notify();
    }

    fn forge_after_open(&mut self, tab: u32, number: u64, cx: &mut Context<Self>) {
        self.mr_recount(tab, number, cx);
        self.merge_load_settings(tab, cx);
    }

    /// Closes the tab's view; a half-written reply stays for the session.
    pub fn forge_close_mr(&mut self, tab: u32, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        if let Some(n) = t.forge.open_mr.take()
            && let Some(ui) = t.mr_views.get_mut(&n)
        {
            ui.bodies.clear();
            ui.picker = None;
            ui.reaction_picker = None;
        }
        self.mr_release_compare_ref(tab, cx);
        self.focus_next_frame = Some(self.focus.clone());
        cx.notify();
    }

    pub fn mr_ui(&mut self, tab: u32, number: u64) -> Option<&mut MrUi> {
        self.tabs.iter_mut().find(|t| t.id == tab)?.mr_views.get_mut(&number)
    }

    /// The Markdown field of a comment or reply, made the first time.
    pub fn mr_field(&mut self, tab: u32, number: u64, key: FieldKey, cx: &mut Context<Self>) -> Option<Entity<MarkdownField>> {
        let t = self.tabs.iter().find(|t| t.id == tab)?;
        let kind = t.forge.kind?;
        let existing = {
            let ui = t.mr_views.get(&number)?;
            match &key {
                FieldKey::Comment => ui.comment.clone(),
                FieldKey::Reply(d) => ui.replies.get(d).cloned(),
                FieldKey::NoteEdit(n) => ui.note_edit.get(n).cloned(),
            }
        };
        if existing.is_some() {
            return existing;
        }
        let (label, placeholder) = match &key {
            FieldKey::Comment => ("Write a comment", "Write a comment"),
            FieldKey::Reply(_) => ("Reply", "Write a reply"),
            FieldKey::NoteEdit(_) => ("Edit comment", ""),
        };
        let md = MdOptions::for_comment(core_repo(tab), link_env(t), flavor_of(kind), Default::default());
        let people = self.mr_people_source(tab, number);
        let opts = FieldOptions { md, label: label.into(), placeholder: placeholder.into(), rows: 4, grow_preview: false, people: Some(people) };
        let (api, rt, retired) = (self.api.clone(), self.rt.clone(), self.retired_images.clone());
        let field = cx.new(|cx| MarkdownField::new(opts, api, rt, retired, cx));
        let sub_key = key.clone();
        cx.subscribe(&field, move |this, _, e: &FieldEvent, cx| match e {
            FieldEvent::Submit => this.mr_field_submit(tab, number, &sub_key, cx),
            FieldEvent::Changed => cx.notify(),
            FieldEvent::Blurred => {}
        })
        .detach();
        let ui = self.mr_ui(tab, number)?;
        match key {
            FieldKey::Comment => ui.comment = Some(field.clone()),
            FieldKey::Reply(d) => {
                ui.replies.insert(d, field.clone());
            }
            FieldKey::NoteEdit(n) => {
                ui.note_edit.insert(n, field.clone());
            }
        }
        Some(field)
    }

    /// `@username`: the MR/PR's own people first, then what the forge's search finds.
    pub fn mr_people_source(&self, tab: u32, number: u64) -> PeopleSource {
        use crate::md::complete::Person;
        let mut participants: Vec<Person> = Vec::new();
        if let Some(t) = self.tabs.iter().find(|t| t.id == tab) {
            let mut add = |u: &gitvolt_core::forge::types::ForgeUser| {
                if !participants.iter().any(|p| p.username == u.username) {
                    participants.push(Person { id: u.id, username: u.username.clone(), name: u.name.clone() });
                }
            };
            if let Some(d) = t.forge.details.get(&number) {
                add(&d.mr.author);
                d.reviewers.iter().for_each(&mut add);
                d.assignees.iter().for_each(&mut add);
            }
            for d in t.forge.discussions.get(&number).into_iter().flatten() {
                d.notes.iter().for_each(|n| add(&n.author));
            }
        }
        let remote = self.tabs.iter().find(|t| t.id == tab).and_then(|t| t.forge.remote.clone());
        let (api, rt) = (self.api.clone(), self.rt.clone());
        let search: Option<crate::md::field::PeopleSearch> = remote.map(|remote| {
            std::rc::Rc::new(move |q: String, cx: &mut App| {
                let (api, rt, remote) = (api.clone(), rt.clone(), remote.clone());
                cx.spawn(async move |_| {
                    let users = super::ask::<Vec<gitvolt_core::forge::types::ForgeUser>>(api, rt, gitvolt_core::api::Request::ForgeSearchUsers { repo: core_repo(tab), remote, query: q }).await.unwrap_or_default();
                    users.into_iter().map(|u| Person { id: u.id, username: u.username, name: u.name }).collect()
                })
            }) as crate::md::field::PeopleSearch
        });
        PeopleSource { participants, search, avatar: None }
    }

    /// A rendered body, made once per note and its text; what it links to acts through the app.
    pub fn mr_body(&mut self, tab: u32, number: u64, key: &str, text: &str, cx: &mut Context<Self>) -> Option<Entity<MdView>> {
        self.mr_body_on(tab, number, key, text, None, cx)
    }

    /// The same, for a note whose suggestions replace known lines.
    pub fn mr_body_on(&mut self, tab: u32, number: u64, key: &str, text: &str, base: Option<crate::md::SuggestionBase>, cx: &mut Context<Self>) -> Option<Entity<MdView>> {
        let t = self.tabs.iter().find(|t| t.id == tab)?;
        if let Some((seen, view)) = t.mr_views.get(&number)?.bodies.get(key)
            && seen == text
        {
            return Some(view.clone());
        }
        let kind = t.forge.kind?;
        let mut opts = MdOptions::for_comment(core_repo(tab), link_env(t), flavor_of(kind), Default::default());
        opts.base = base;
        let (api, rt, retired) = (self.api.clone(), self.rt.clone(), self.retired_images.clone());
        let content: Arc<str> = Arc::from(text);
        let view = cx.new(|cx| {
            let mut v = MdView::new(opts, api, rt, retired, cx);
            v.set_content(Content::Text(content), false, cx);
            v
        });
        cx.subscribe(&view, move |this, _, e: &MdEvent, cx| this.mr_md_event(tab, e, cx)).detach();
        let ui = self.mr_ui(tab, number)?;
        ui.bodies.insert(key.to_string(), (text.to_string(), view.clone()));
        Some(view)
    }

    fn mr_md_event(&mut self, tab: u32, e: &MdEvent, cx: &mut Context<Self>) {
        match e {
            MdEvent::OpenFile { path, commit, .. } => {
                if commit == "worktree" {
                    self.open_worktree_file(path, cx);
                } else {
                    self.open_path_at(path, commit, false, cx);
                }
            }
            MdEvent::ShowCommit { sha } => self.go_to_commit(sha.clone().into(), cx),
            MdEvent::OpenMr { number, .. } => self.forge_open_mr(tab, *number, cx),
            MdEvent::TooLarge => cx.notify(),
            MdEvent::Zoom(_) => {}
        }
    }

    pub fn mr_dock_width(&self, room: f32) -> Option<f32> {
        let t = self.tabs.get(self.active)?;
        t.forge.open_mr?;
        if self.prefs["flyoutDock"]["docked"].as_bool() != Some(true) || !can_dock(room) || t.diff.is_some() || t.history.is_some() {
            return None;
        }
        Some(dock_width(self.prefs["flyoutDock"]["width"].as_f64().map(|w| w as f32), room))
    }

    pub fn mr_toggle_dock(&mut self, cx: &mut Context<Self>) {
        let on = self.prefs["flyoutDock"]["docked"].as_bool() == Some(true);
        self.prefs["flyoutDock"]["docked"] = serde_json::json!(!on);
        self.save_layout();
        cx.notify();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FieldKey {
    Comment,
    Reply(String),
    NoteEdit(String),
}

/// A flyout's frame around the view: the header (title, dock, refresh, browser, ×), and its scrolling body.
pub fn mr_flyout(this: &mut GitVolt, room: f32, window: &mut Window, cx: &mut Context<GitVolt>) -> Option<(AnyElement, bool)> {
    if this.tabs.get(this.active)?.create.is_some() {
        return crate::forge::create::create_flyout(this, room, window, cx).map(|f| (f, false));
    }
    let t = this.tabs.get(this.active)?;
    let tab = t.id;
    let number = t.forge.open_mr?;
    let kind = t.forge.kind.unwrap_or(ForgeKind::GitLab);
    if t.diff.is_some() || t.history.is_some() {
        // A file on top: the view waits under it.
        return None;
    }
    let r = mr_ref(kind, number);
    let label = format!("{} {r}", mr_name(kind));
    let detail: Option<ForgeMrDetail> = t.forge.details.get(&number).cloned();
    let mr: Option<ForgeMr> = detail.as_ref().map(|d| d.mr.clone()).or_else(|| t.forge.known_mr(number).cloned());
    let error = t.forge.detail_errors.get(&number).cloned();
    let stale = super::store::stale_text(&t.forge, super::poll::now_ms());
    let refreshing = t.forge.refreshing.contains(&Refreshing::Mr(number));
    let dock_pref = this.prefs["flyoutDock"]["docked"].as_bool() == Some(true);
    let docked = this.mr_dock_width(room).is_some();
    let width = if docked { this.mr_dock_width(room).unwrap_or(DOCK_DEFAULT) } else { flyout_width(this.profile["flyoutWidth"].as_f64().map(|w| w as f32), room) };
    let dockable = can_dock(room);
    let web_url = mr.as_ref().map(|m| m.web_url.clone());
    let title = mr.as_ref().map(|m| m.title.clone()).unwrap_or_default();
    let head_btn = |id: &'static str| div().id(id).flex().flex_none().size(px(24.)).items_center().justify_center().rounded(px(4.)).cursor_pointer().hover(|d| d.bg(theme::hover_row()));
    let spin = |n: &'static str, key: &'static str| icon(n, 14., theme::text_secondary()).with_animation(key, Animation::new(std::time::Duration::from_secs(1)).repeat(), |s, d| s.with_transformation(Transformation::rotate(percentage(d))));
    let dock_tip = if docked || dock_pref && !dockable { "Undock (float over the graph)" } else if dockable { "Dock beside the graph" } else { "The window is too narrow to dock" };
    let mut head = div()
        .flex()
        .flex_none()
        .items_start()
        .gap(px(6.))
        .pt(px(8.))
        .pb(px(8.))
        .pl(px(12.))
        .pr(px(8.))
        .border_b_1()
        .border_color(theme::section_border())
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_start()
                .gap(px(6.))
                .pt(px(2.))
                .text_size(px(14.))
                .line_height(px(18.9))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text_selected())
                .children(mr.as_ref().map(|m| div().mt(px(2.)).flex_none().child(state_icon(m.state, 14.))))
                .child(div().flex_1().min_w_0().child(format!("{r} {}", crate::md::emoji::emojify(&title)))),
        );
    head = head.child(tip(dock_tip, crate::ui::anchor("mr-dock", head_btn("mr-dock").when(dockable || docked, |d| d.on_click(cx.listener(|v, _, _, cx| v.mr_toggle_dock(cx)))).child(icon(if docked { "arrow-right-to-line" } else { "arrow-left-to-line" }, 14., theme::text_secondary())))));
    if let Some(url) = web_url {
        let refresh_icon: AnyElement = if refreshing { spin("refresh-cw", "mr-refreshing").into_any_element() } else { icon("refresh-cw", 14., theme::text_secondary()).into_any_element() };
        head = head
            .child(tip(format!("Refresh {r}"), crate::ui::anchor("mr-refresh", head_btn("mr-refresh").on_click(cx.listener(move |v, _, _, cx| v.forge_refresh_mr_now(tab, number, cx))).child(refresh_icon))))
            .child(tip("Open in browser", crate::ui::anchor("mr-browser", head_btn("mr-browser").on_click(cx.listener(move |v, _, _, _| v.forge_open_url(url.clone()))).child(icon("external-link", 14., theme::text_secondary())))));
    }
    head = head.child(tip("Close (Esc)", crate::ui::anchor("mr-close", head_btn("mr-close").on_click(cx.listener(move |v, _, _, cx| v.forge_close_mr(tab, cx))).child(icon("x", 14., theme::text_secondary())))));
    let mut body = div().id(ElementId::Name(format!("mr-body:{number}").into())).flex().flex_col().flex_1().min_h_0().overflow_y_scroll().gap(px(12.)).pt(px(12.)).pb(px(12.)).pl(px(12.)).pr(px(12.));
    if let Some(ui) = this.tabs.get(this.active).and_then(|t| t.mr_views.get(&number)) {
        body = body.track_scroll(&ui.scroll);
    }
    if let Some(s) = stale {
        body = body.child(div().text_size(px(11.)).text_color(theme::orange()).child(s));
    }
    match &mr {
        None => {
            body = body.child(div().text_color(theme::text_secondary()).child(error.map_or("Loading…".to_string(), |e| format!("Couldn't load {label}: {e}"))));
        }
        Some(m) => {
            if let Some(e) = &error {
                body = body.child(div().text_size(px(11.)).text_color(theme::orange()).child(format!("Couldn't refresh {r}: {e}")));
            }
            body = body.child(header::mr_header(this, tab, kind, m, detail.as_ref(), window, cx));
            let editing = this.tabs.get(this.active).and_then(|t| t.mr_views.get(&number)).is_some_and(|u| u.mode == Mode::Edit) && matches!(m.state, gitvolt_core::forge::types::MrState::Open | gitvolt_core::forge::types::MrState::Draft);
            if !editing {
                // Its place in the column's gaps, with or without a stack.
                body = body.child(super::stack::stack_panel(this, tab, kind, m, cx).unwrap_or_else(|| div().into_any_element()));
            }
            body = body.children(actions::mr_forms(this, tab, kind, m, detail.as_ref(), window, cx));
            if !editing {
                body = body.children(merge::merge_box(this, tab, kind, m, detail.as_ref(), cx));
                body = body.child(thread::description(this, tab, kind, m, detail.as_ref(), cx));
                body = body.child(thread::activity(this, tab, kind, m, detail.as_ref(), window, cx));
                if let Some(field) = this.mr_field(tab, number, FieldKey::Comment, cx) {
                    body = body.child(div().px(px(12.)).py(px(10.)).rounded(px(8.)).border_1().border_color(theme::section_border()).bg(theme::panel_bg1()).child(thread::reply_box(this, tab, kind, number, None, field, None, cx)));
                }
            }
        }
    }
    let bar = this.tabs.get(this.active).and_then(|t| t.mr_views.get(&number)).map(|u| scrollbar(&u.scroll));
    let frame = div()
        .id("mr-flyout")
        .key_context("MrView")
        .occlude()
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        .w(px(width))
        .h_full()
        .min_h_0()
        .bg(theme::panel_bg1())
        .text_color(theme::text_normal())
        .border_r_1()
        .border_color(theme::section_border())
        .when(!docked, |d| d.shadow(vec![BoxShadow { color: theme::shadow_popup(), offset: point(px(4.), px(0.)), blur_radius: px(16.), spread_radius: px(0.), inset: false }]))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_key_down(cx.listener(move |v, e: &KeyDownEvent, window, cx| {
            if e.keystroke.key == "escape" && !e.keystroke.modifiers.modified() {
                v.mr_escape(tab, number, window, cx);
                cx.stop_propagation();
            }
        }))
        .child(head)
        .child(body)
        .children(bar)
        .child(
            div()
                .id("mr-resize")
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(-3.))
                .w(px(6.))
                .cursor(CursorStyle::ResizeLeftRight)
                .on_mouse_down(MouseButton::Left, cx.listener(move |v, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if e.click_count == 2 {
                        v.mr_set_width(None, docked, cx);
                        return;
                    }
                    v.mr_drag = Some((e.position.x.as_f32(), width, docked));
                })),
        )
        .into_any_element();
    Some((frame, docked))
}

/// The body's scrollbar: the thumb where it is. The wheel moves the body; the thumb says where.
fn scrollbar(handle: &ScrollHandle) -> AnyElement {
    let (offset, max, view) = (-handle.offset().y.as_f32(), handle.max_offset().y.as_f32(), handle.bounds().size.height.as_f32());
    if max <= 0. || view <= 0. {
        return div().into_any_element();
    }
    let total = view + max;
    let h = (view * view / total).max(20.);
    let top = (offset / max).clamp(0., 1.) * (view - h);
    div().absolute().right(px(1.)).bottom_0().top(px(39.)).w(px(8.)).child(div().absolute().top(px(top)).w_full().h(px(h)).rounded(px(4.)).bg(theme::scroll_thumb())).into_any_element()
}

impl GitVolt {
    pub fn mr_set_width(&mut self, w: Option<f32>, docked: bool, cx: &mut Context<Self>) {
        if docked {
            self.prefs["flyoutDock"]["width"] = serde_json::json!(w);
            self.save_layout();
        } else {
            self.update_profile(move |p| p["flyoutWidth"] = serde_json::json!(w), cx);
        }
        cx.notify();
    }

    /// Esc: an open form or popup first, else the view; docked, the view's a pane, and Esc closes it only from inside (it's here).
    pub fn mr_escape(&mut self, tab: u32, number: u64, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ui) = self.mr_ui(tab, number) {
            if ui.reaction_picker.take().is_some() || ui.picker.take().is_some() {
                cx.notify();
                return;
            }
            if ui.mode == Mode::Edit {
                ui.mode = Mode::None;
                ui.edit = None;
                cx.notify();
                return;
            }
        }
        self.forge_close_mr(tab, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn its_width_keeps_the_graph_usable() {
        assert_eq!(flyout_width(None, 1359.), 560.);
        assert_eq!(flyout_width(Some(2000.), 1359.), 760.);
        assert_eq!(flyout_width(None, 700.), 460.);
        assert_eq!(flyout_width(None, 300.), 300.);
        assert!(can_dock(1000.));
        assert!(!can_dock(999.));
        assert_eq!(dock_width(None, 1359.), 480.);
        assert_eq!(dock_width(Some(760.), 1100.), 460.);
    }
}
