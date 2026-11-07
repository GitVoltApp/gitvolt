//! The view's actions (`MrActions.tsx`, `EditMr.tsx`, `PendingReview.tsx`): Approve and Review… in the APPROVALS box, Check out, Edit and ⋯ on the status
//! line, and their forms under the header. A write's failure says why where it was started.

use super::header::{card_btn, mr_button, mr_primary};
use super::{Mode, thread};
use crate::app::{GitVolt, core_repo, icon};
use crate::forge::review::{self, Sent, is_pending, pending_text, retry_note, retry_plan};
use crate::forge::search::{self, SearchOption, Source};
use crate::forge::text::{forge_name, mr_ref};
use crate::md::field::{FieldEvent, FieldOptions, MarkdownField};
use crate::md::view::MdOptions;
use crate::theme;
use crate::ui::arm::{self, ArmRequest, Origin, Tone};
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::menu::{self, MenuRow};
use crate::ui::tooltip::tip;
use gitvolt_core::api::Request;
use gitvolt_core::forge::types::{ForgeMr, ForgeMrDetail, MrEdit, MrState, ReviewDecision, ReviewState};
use gitvolt_core::forge::ForgeKind;
use gpui::prelude::*;
use gpui::*;
use serde_json::json;
use std::rc::Rc;

const GITLAB_CHANGES_NOTE: &str = "Posts your comment and withdraws your approval; a GitLab with review states also marks your review Changes requested";
const FORGEJO_CHANGES_NOTE: &str = "Forgejo needs a message to request changes";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    Approve,
    CheckingOut,
}

#[derive(Debug, Clone)]
pub struct PeopleWrite {
    pub number: u64,
    pub edit: MrEdit,
    pub failure: String,
    pub role: super::people::Role,
    pub before: Vec<gitvolt_core::forge::types::ForgeUser>,
    pub who_id: u64,
    pub adding: bool,
}

const MODES: [(&str, &str, &str); 3] = [("comment", "Comment", "Leave a comment"), ("approve", "Approve", "A comment with your approval (optional)"), ("requestChanges", "Request changes", "What should change?")];

#[derive(Default)]
pub struct ReviewUi {
    pub event: Option<&'static str>,
    pub input: Option<Entity<TextInput>>,
    pub busy: bool,
    pub partial: Option<String>,
    pub sent: Option<Sent>,
}

pub struct EditUi {
    pub title: Entity<TextInput>,
    pub description: Entity<MarkdownField>,
    pub labels: Vec<String>,
    pub colors: std::collections::HashMap<String, Option<String>>,
    pub busy: bool,
    pub open_labels: bool,
}

/// What a review that went in says (the toast): `published`, the pending comments it sent with it. `fallback`: a GitLab with no Changes requested state took it as a comment and a withdrawn approval.
pub fn review_said(event: &str, r: &str, fallback: bool, published: Option<u32>) -> String {
    let sent = published.filter(|p| *p > 0).map(|p| if p == 1 { "1 comment".to_string() } else { format!("{p} comments") });
    if event == "comment" {
        return sent.map_or(format!("Commented on {r}"), |s| format!("Sent your review of {r} with {s}"));
    }
    let and = sent.as_ref().map_or(String::new(), |s| format!("Published {s} and "));
    if event == "approve" {
        return if sent.is_some() { format!("{and}approved {r}") } else { format!("Approved {r}") };
    }
    if fallback {
        return format!("{} on {r}. This GitLab has no Changes requested state, so your approval was withdrawn instead.", if sent.is_some() { format!("{and}commented") } else { "Commented".to_string() });
    }
    if sent.is_some() { format!("{and}requested changes on {r}") } else { format!("Requested changes on {r}") }
}

pub fn partial_text(kind: ForgeKind, published: u32, error: &str) -> String {
    let went = match published {
        0 => "Your pending review was published".to_string(),
        1 => "Your comment was published".to_string(),
        n => format!("Your {n} comments were published"),
    };
    format!("{went}, but {} refused the rest: {error}", forge_name(kind))
}

impl GitVolt {
    fn mr_approve(&mut self, tab: u32, kind: ForgeKind, number: u64, origin: Origin, window: &mut Window, cx: &mut Context<Self>) {
        let r = mr_ref(kind, number);
        let weak = cx.weak_entity();
        let req = ArmRequest::new(Tone::Positive, "Click again to approve", format!("Approve {r}?"), "Approve");
        arm::request(
            req,
            origin,
            move |answer, _, cx| {
                if !answer.ok {
                    return;
                }
                weak.update(cx, |v, cx| {
                    if let Some(ui) = v.mr_ui(tab, number) {
                        ui.busy = Some(Busy::Approve);
                    }
                    cx.notify();
                    let r2 = r.clone();
                    v.forge_ask::<serde_json::Value>(Request::ForgeApprove { repo: core_repo(tab), number }, cx, move |v, answer, cx| {
                        if let Some(ui) = v.mr_ui(tab, number) {
                            ui.busy = None;
                        }
                        match answer {
                            Ok(_) => {
                                v.forge_written(tab, cx);
                                v.forge_refresh_mr(tab, number, cx);
                                crate::ui::toast::show(format!("Approved {r2}"), cx);
                            }
                            Err(e) => crate::ui::toast::show_error(format!("Couldn't approve {r2}: {e}"), cx),
                        }
                    });
                })
                .ok();
            },
            window,
            cx,
        );
    }

    /// To the review: the pending review panel while one is pending, else the composer.
    fn mr_to_review(&mut self, tab: u32, number: u64, window: &mut Window, cx: &mut Context<Self>) {
        let pending = self.tabs.iter().find(|t| t.id == tab).is_some_and(|t| is_pending(t.forge.review.as_ref().filter(|s| s.number == number)));
        let (field, _) = self.mr_composer(tab, number, cx);
        let Some(ui) = self.mr_ui(tab, number) else { return };
        ui.mode = if !pending && ui.mode == Mode::Review { Mode::None } else { Mode::Review };
        if ui.mode == Mode::Review {
            window.focus(&field.read(cx).focus.clone(), cx);
            self.review_resume(tab, number, cx);
        }
        cx.notify();
    }

    pub fn mr_composer(&mut self, tab: u32, number: u64, cx: &mut Context<Self>) -> (Entity<TextInput>, &'static str) {
        let existing = self.mr_ui(tab, number).and_then(|u| u.review.input.clone());
        let input = existing.unwrap_or_else(|| {
            let i = cx.new(TextInput::multiline);
            let submit = i.clone();
            cx.subscribe(&submit, move |v, _, e: &InputEvent, cx| match e {
                InputEvent::Submit { ctrl: true, .. } => v.mr_review_send(tab, number, cx),
                InputEvent::Changed => cx.notify(),
                _ => {}
            })
            .detach();
            if let Some(ui) = self.mr_ui(tab, number) {
                ui.review.input = Some(i.clone());
            }
            i
        });
        let event = self.mr_ui(tab, number).and_then(|u| u.review.event).unwrap_or("comment");
        (input, event)
    }

    fn mr_review_send(&mut self, tab: u32, number: u64, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let kind = t.forge.kind.unwrap_or(ForgeKind::GitLab);
        let session = t.forge.review.as_ref().filter(|s| s.number == number).cloned();
        let pending = session.as_ref().map_or(0, |s| s.drafts.len());
        let with_pending = pending > 0 || session.as_ref().is_some_and(|s| s.pending_review.is_some());
        let (input, event) = self.mr_composer(tab, number, cx);
        let text = input.read(cx).text().to_string();
        let required = kind == ForgeKind::Forgejo && event == "requestChanges";
        let Some(ui) = self.mr_ui(tab, number) else { return };
        if ui.review.busy || !(event == "approve" || !text.trim().is_empty() || (pending > 0 && !required)) {
            return;
        }
        let sent = ui.review.sent;
        ui.review.busy = true;
        ui.review.partial = None;
        let (plan_event, plan_body) = retry_plan(event, &text, sent);
        let r = mr_ref(kind, number);
        let failure = match event {
            "comment" => format!("Couldn't comment on {r}"),
            "approve" => format!("Couldn't approve {r}"),
            _ => format!("Couldn't request changes on {r}"),
        };
        let review = json!({ "event": plan_event, "body": plan_body });
        cx.notify();
        let done = move |v: &mut GitVolt, message: String, cx: &mut Context<GitVolt>| {
            if let Some(ui) = v.mr_ui(tab, number) {
                ui.review.sent = None;
                ui.mode = Mode::None;
                if let Some(i) = ui.review.input.clone() {
                    i.update(cx, |i, cx| i.set_text("", cx));
                }
            }
            crate::ui::toast::show(message, cx);
        };
        if with_pending {
            self.review_submit(tab, review, cx, move |v, out, cx| {
                if let Some(ui) = v.mr_ui(tab, number) {
                    ui.review.busy = false;
                }
                match out {
                    Err(e) => crate::ui::toast::show_error(format!("{failure}: {e}"), cx),
                    Ok(o) => {
                        if let Some(err) = &o.event_error {
                            let now = Sent { body_posted: o.body_posted || sent.is_some_and(|s| s.body_posted), event_sent: o.event_sent || sent.is_some_and(|s| s.event_sent) };
                            let said = format!("{}{}", partial_text(kind, o.published, err), retry_note(Some(now)));
                            if let Some(ui) = v.mr_ui(tab, number) {
                                ui.review.sent = Some(now);
                                ui.review.partial = Some(said);
                            }
                        } else {
                            done(v, review_said(event, &r, o.fallback, Some(o.published)), cx);
                        }
                        v.forge_refresh_mr(tab, number, cx);
                    }
                }
            });
            return;
        }
        let Ok(review) = serde_json::from_value(review) else { return };
        self.forge_ask::<gitvolt_core::forge::types::ReviewOutcome>(Request::ForgeReview { repo: core_repo(tab), number, review }, cx, move |v, out, cx| {
            if let Some(ui) = v.mr_ui(tab, number) {
                ui.review.busy = false;
            }
            match out {
                Ok(o) => {
                    v.forge_written(tab, cx);
                    v.forge_refresh_mr(tab, number, cx);
                    done(v, review_said(event, &r, o.fallback, None), cx);
                }
                Err(e) => crate::ui::toast::show_error(format!("{failure}: {e}"), cx),
            }
        });
    }

    pub fn mr_open_edit(&mut self, tab: u32, number: u64, labels: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some(kind) = t.forge.kind else { return };
        let Some(mr) = t.forge.details.get(&number).map(|d| &d.mr).or_else(|| t.forge.known_mr(number)).cloned() else { return };
        let description = t.forge.details.get(&number).map(|d| d.description.clone()).unwrap_or_default();
        let has_detail = t.forge.details.contains_key(&number);
        let md = MdOptions::for_comment(core_repo(tab), super::link_env(t), super::flavor_of(kind), Default::default());
        let people = self.mr_people_source(tab, number);
        let opts = FieldOptions { md, label: "Description".into(), placeholder: "".into(), rows: 10, grow_preview: false, people: Some(people) };
        let (api, rt, retired) = (self.api.clone(), self.rt.clone(), self.retired_images.clone());
        let field = cx.new(|cx| {
            let mut f = MarkdownField::new(opts, api, rt, retired, cx);
            f.set_text(&description, cx);
            f.set_disabled(!has_detail, cx);
            f
        });
        cx.subscribe(&field, move |v, _, e: &FieldEvent, cx| match e {
            FieldEvent::Submit => v.mr_edit_save(tab, number, cx),
            FieldEvent::Changed => cx.notify(),
            FieldEvent::Blurred => {}
        })
        .detach();
        let title = cx.new(|cx| {
            let mut i = TextInput::new(cx).enter_submits();
            i.set_text(&mr.title, cx);
            i
        });
        cx.subscribe(&title, move |v, _, e: &InputEvent, cx| match e {
            InputEvent::Submit { .. } => v.mr_edit_save(tab, number, cx),
            InputEvent::Changed => cx.notify(),
            _ => {}
        })
        .detach();
        if !labels {
            window.focus(&title.read(cx).focus.clone(), cx);
        }
        let Some(ui) = self.mr_ui(tab, number) else { return };
        ui.edit = Some(EditUi { title, description: field, labels: mr.labels.clone(), colors: Default::default(), busy: false, open_labels: labels });
        ui.mode = Mode::Edit;
        ui.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    fn mr_edit_values(&self, tab: u32, number: u64, cx: &App) -> Option<MrEdit> {
        let t = self.tabs.iter().find(|t| t.id == tab)?;
        let mr = t.forge.details.get(&number).map(|d| &d.mr).or_else(|| t.forge.known_mr(number))?;
        let e = t.mr_views.get(&number)?.edit.as_ref()?;
        let title = e.title.read(cx).text().trim().to_string();
        let description = e.description.read(cx).text(cx);
        Some(MrEdit {
            title: (title != mr.title).then_some(title),
            description: t.forge.details.get(&number).filter(|d| d.description != description).map(|_| description),
            labels: (e.labels != mr.labels).then(|| e.labels.clone()),
            reviewers: None,
            assignees: None,
        })
    }

    fn mr_edit_save(&mut self, tab: u32, number: u64, cx: &mut Context<Self>) {
        let Some(edit) = self.mr_edit_values(tab, number, cx) else { return };
        if edit.title.is_none() && edit.description.is_none() && edit.labels.is_none() {
            return;
        }
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some(kind) = t.forge.kind else { return };
        let Some(ui) = self.mr_ui(tab, number) else { return };
        let Some(e) = ui.edit.as_mut() else { return };
        if e.busy || e.title.read(cx).text().trim().is_empty() {
            return;
        }
        e.busy = true;
        cx.notify();
        let new_description = edit.description.clone();
        let r = mr_ref(kind, number);
        self.forge_ask::<ForgeMr>(Request::ForgeEditMr { repo: core_repo(tab), number, edit }, cx, move |v, answer, cx| {
            if let Some(e) = v.mr_ui(tab, number).and_then(|u| u.edit.as_mut()) {
                e.busy = false;
            }
            match answer {
                Err(e) => crate::ui::toast::show_error(format!("Couldn't edit {r}: {e}"), cx),
                Ok(m) => {
                    v.forge_written(tab, cx);
                    v.forge_put_mr(tab, m, cx);
                    if let (Some(d), Some(t)) = (new_description, v.tabs.iter_mut().find(|t| t.id == tab))
                        && let Some(detail) = t.forge.details.get_mut(&number)
                    {
                        detail.description = d;
                    }
                    if let Some(ui) = v.mr_ui(tab, number) {
                        ui.mode = Mode::None;
                        ui.edit = None;
                    }
                }
            }
        });
    }

    fn mr_subscribe(&mut self, tab: u32, kind: ForgeKind, number: u64, on: bool, cx: &mut Context<Self>) {
        let put = move |v: &mut GitVolt, s: bool| {
            if let Some(d) = v.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.forge.details.get_mut(&number)) {
                d.subscribed = Some(s);
            }
        };
        put(self, on);
        let r = mr_ref(kind, number);
        let failure = if on { format!("Couldn't subscribe to {r}") } else { format!("Couldn't unsubscribe from {r}") };
        self.forge_ask::<bool>(Request::ForgeSetSubscribed { repo: core_repo(tab), number, on }, cx, move |v, answer, cx| match answer {
            Ok(s) => {
                v.forge_written(tab, cx);
                put(v, s);
            }
            Err(e) => {
                put(v, !on);
                crate::ui::toast::show_error(format!("{failure}: {e}"), cx);
            }
        });
    }

    /// Ctrl+Shift+A, Ctrl+Shift+M and Ctrl+Alt+U: while the button would take a click. Read by the command table.
    pub fn mr_lend_merge(&mut self, tab: u32, number: u64, merge: bool, update: bool) {
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
            t.forge.lent_merge = merge.then_some(number);
            t.forge.lent_update = update.then_some(number);
        }
    }

    /// The commands' way into the open view.
    pub fn mr_key(&mut self, which: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get(self.active) else { return };
        let tab = t.id;
        let Some(number) = t.forge.open_mr else { return };
        let Some(kind) = t.forge.kind else { return };
        let detail = t.forge.details.get(&number).cloned();
        let Some(mr) = detail.as_ref().map(|d| d.mr.clone()).or_else(|| t.forge.known_mr(number).cloned()) else { return };
        match which {
            "approve" => self.mr_approve(tab, kind, number, Origin::None, window, cx),
            "merge" => {
                let mode = if mr.auto_merge.is_some() { super::merge::MergeMode::Cancel } else if mr.pipeline.as_ref().is_some_and(|p| matches!(p.status, gitvolt_core::forge::types::PipelineStatus::Running | gitvolt_core::forge::types::PipelineStatus::Pending)) { super::merge::MergeMode::Auto } else { super::merge::MergeMode::Merge };
                self.merge_run(tab, kind, &mr, detail.as_ref(), mode, Origin::None, window, cx);
            }
            "update" => {
                let t = self.tabs.get(self.active);
                let saved = t.and_then(|t| self.profile["repos"][t.repo_path.as_str()]["branchUpdate"].as_str()).map(str::to_string);
                let methods = t.and_then(|t| t.forge.remote.as_ref().and_then(|x| t.forge.settings.get(x))).map(|s| s.merge_methods.clone()).unwrap_or_default();
                let saved = saved.as_deref().and_then(|s| serde_json::from_value(json!(s)).ok());
                let kinds = detail.as_ref().and_then(|d| d.update.as_ref()).map(|o| o.kinds.clone()).unwrap_or_default();
                if let Some(main) = super::merge::default_update(kind, &kinds, saved, &methods) {
                    self.merge_update(tab, kind, &mr, main, true, cx);
                }
            }
            "browser" => self.forge_open_url(mr.web_url),
            "refresh" => self.forge_refresh_mr_now(tab, number, cx),
            _ => {}
        }
    }
}

/// Check out, Edit and the ⋯ menu, at the right end of the status line.
pub fn status_actions(this: &mut GitVolt, tab: u32, kind: ForgeKind, mr: &ForgeMr, detail: Option<&ForgeMrDetail>, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    let number = mr.number;
    let live = matches!(mr.state, MrState::Open | MrState::Draft);
    let Some(t) = this.tabs.iter().find(|t| t.id == tab) else { return Vec::new() };
    let Some(ui) = t.mr_views.get(&number) else { return Vec::new() };
    let busy = ui.busy;
    let (label, disabled) = super::checkout::checkout_state(t, mr);
    let checking = busy == Some(Busy::CheckingOut);
    let mut out = Vec::new();
    let enabled = disabled.is_none() && !checking;
    let m = mr.clone();
    let btn = mr_button("mr-checkout", enabled).child(if checking { "Checking out…".to_string() } else { label }).when(enabled, |d| {
        d.on_click(cx.listener(move |v, _, _, cx| {
            if let Some(ui) = v.mr_ui(tab, number) {
                ui.busy = Some(Busy::CheckingOut);
            }
            v.mr_checkout(tab, m.clone(), cx);
            cx.notify();
        }))
    });
    out.push(tip(disabled.unwrap_or_else(|| format!("Check out {}", mr.source_branch)), crate::ui::anchor("mr-checkout", btn)).into_any_element());
    if live {
        let on = busy.is_none();
        out.push(crate::ui::anchor("mr-edit", mr_button("mr-edit", on).child(icon("pencil", 13., theme::text_normal())).child("Edit").when(on, |d| {
            d.on_click(cx.listener(move |v, _, window, cx| {
                let editing = v.mr_ui(tab, number).is_some_and(|u| u.mode == Mode::Edit);
                if editing {
                    if let Some(ui) = v.mr_ui(tab, number) {
                        ui.mode = Mode::None;
                        ui.edit = None;
                    }
                    cx.notify();
                } else {
                    v.mr_open_edit(tab, number, false, window, cx);
                }
            }))
        })).into_any_element());
    }
    let key = "mr-more";
    let (m2, sub, weak) = (mr.clone(), detail.and_then(|d| d.subscribed), cx.weak_entity());
    out.push(crate::ui::anchor(key, mr_button(key, busy.is_none()).px(px(7.)).child(icon("ellipsis-vertical", 14., theme::text_normal())).on_mouse_down(MouseButton::Left, cx.listener(move |_, _, window, cx| {
        cx.stop_propagation();
        let r = mr_ref(kind, number);
        let mut rows = Vec::new();
        if matches!(m2.state, MrState::Open | MrState::Draft) {
            let draft = m2.state == MrState::Draft;
            let (w, m3) = (weak.clone(), m2.clone());
            rows.push(MenuRow::action("mr.draft", if draft { "Mark as ready" } else { "Mark as draft" }, if draft { "git-pull-request" } else { "git-pull-request-draft" }, if draft { format!("Mark {r} as ready for review") } else { format!("Mark {r} as a draft") }, move |_, cx| {
                let m = m3.clone();
                w.update(cx, |v, cx| v.mr_toggle_draft(tab, kind, m, cx)).ok();
            }));
        }
        if let Some(on) = sub {
            let w = weak.clone();
            rows.push(MenuRow::action("mr.subscribe", if on { "Unsubscribe" } else { "Subscribe" }, if on { "bell-off" } else { "bell" }, if on { format!("Stop getting {}'s notifications about {r}", forge_name(kind)) } else { format!("Get {}'s notifications about {r} (email and in-app)", forge_name(kind)) }, move |_, cx| {
                w.update(cx, |v, cx| v.mr_subscribe(tab, kind, number, !on, cx)).ok();
            }));
        }
        let url = m2.web_url.clone();
        rows.push(MenuRow::action("mr.copyLink", "Copy link", "link", format!("Copy {r}'s web address"), move |_, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
            crate::ui::toast::show("Copied the link", cx);
        }));
        menu::open_below(key, rows, Default::default(), window, cx);
    }))).into_any_element());
    out
}

/// Approve and Review…: small icon buttons at the top right of the APPROVALS box, for open and draft MRs/PRs.
pub fn review_buttons(this: &mut GitVolt, tab: u32, kind: ForgeKind, mr: &ForgeMr, detail: Option<&ForgeMrDetail>, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    if !matches!(mr.state, MrState::Open | MrState::Draft) {
        return Vec::new();
    }
    let number = mr.number;
    let Some(t) = this.tabs.iter().find(|t| t.id == tab) else { return Vec::new() };
    let Some(ui) = t.mr_views.get(&number) else { return Vec::new() };
    let r = mr_ref(kind, number);
    let me = t.forge.me.clone();
    let approved = me.is_some() && detail.is_some_and(|d| d.mr.review.reviews.iter().any(|x| Some(&x.user.username) == me.as_ref() && x.state == ReviewState::Approved));
    let busy = ui.busy;
    let pending = is_pending(t.forge.review.as_ref().filter(|s| s.number == number));
    let expanded = ui.mode == Mode::Review || pending;
    let spin = busy == Some(Busy::Approve);
    let glyph: AnyElement = if spin { crate::commit_box::spinner("mr-approve-spin", 13., theme::green()).into_any_element() } else { icon(if approved { "circle-check" } else { "check" }, 13., theme::green()).into_any_element() };
    let can = !approved && busy.is_none();
    let approve_btn = card_btn("mr-approve").when(approved, |d| d.bg(theme::mix(theme::green(), 0.24, theme::control_bg())).border_color(theme::mix(theme::green(), 0.6, transparent_black()))).when(!approved && busy.is_some(), |d| d.opacity(0.55)).hover(|d| d.bg(theme::mix(theme::green(), 0.16, theme::control_bg())).border_color(theme::mix(theme::green(), 0.5, transparent_black()))).child(glyph).when(can, |d| {
        d.on_click(cx.listener(move |v, _, window, cx| v.mr_approve(tab, kind, number, Origin::control("mr-fact-reviews"), window, cx)))
    });
    let approve_tip = if approved { format!("You approved {r}") } else { format!("Approve {r}") };
    let review_btn = card_btn("mr-review").px(px(3.)).gap(px(1.)).when(expanded, |d| d.bg(theme::mix(theme::orange(), 0.16, theme::control_bg())).border_color(theme::mix(theme::orange(), 0.5, transparent_black()))).child(icon("message-square-warning", 13., theme::orange())).child(icon("chevron-down", 9., theme::orange())).when(busy.is_none(), |d| {
        d.on_click(cx.listener(move |v, _, window, cx| v.mr_to_review(tab, number, window, cx)))
    });
    vec![
        div().flex().flex_none().gap(px(4.)).child(tip(approve_tip, crate::ui::anchor("mr-approve", approve_btn))).child(tip(format!("Review {r}: comment, approve or request changes"), crate::ui::anchor("mr-review", review_btn))).into_any_element(),
    ]
}

/// The Edit form, or the review composer in its pending review panel, under the header.
pub fn mr_forms(this: &mut GitVolt, tab: u32, kind: ForgeKind, mr: &ForgeMr, detail: Option<&ForgeMrDetail>, window: &mut Window, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let number = mr.number;
    let live = matches!(mr.state, MrState::Open | MrState::Draft);
    let t = this.tabs.iter().find(|t| t.id == tab)?;
    let session = t.forge.review.clone().filter(|s| s.number == number && is_pending(Some(s)));
    let mode = t.mr_views.get(&number)?.mode;
    if live && mode == Mode::Edit {
        return Some(edit_form(this, tab, kind, mr, detail, window, cx));
    }
    if session.is_none() && !(live && mode == Mode::Review) {
        return None;
    }
    let composer = review_composer(this, tab, kind, number, session.is_some(), window, cx);
    let Some(s) = session else { return Some(composer) };
    // Your pending review: a box as the merge box's, its header a band.
    let n = s.drafts.len();
    let what = pending_text(n);
    let head = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .py(px(8.))
        .border_b_1()
        .border_color(theme::section_border())
        .rounded_t(px(8.))
        .bg(theme::mix(theme::yellow(), 0.10, theme::panel_bg1()))
        .child(icon("message-square", 14., theme::yellow()))
        .child(div().flex().flex_1().min_w_0().gap(px(4.)).text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).child(if n > 0 { "Your review".to_string() } else { "Your pending review".to_string() }).when(n > 0, |d| d.child(div().font_weight(FontWeight::NORMAL).text_color(theme::text_normal()).child(format!("· {n} pending")))))
        .child(tip(format!("Delete {what} from {}", forge_name(kind)), crate::ui::anchor("mr-pending-discard", mr_button("mr-pending-discard", true).px(px(8.)).py(px(2.)).child(icon("trash-2", 12., theme::text_normal())).child("Discard").on_click(cx.listener(move |v, _, window, cx| v.review_discard(tab, kind, Origin::control("mr-pending-discard"), window, cx))))));
    let threads = t_discussions(this, tab, number);
    let placed = review::place_review(&s, &threads);
    let mut rows: Vec<(String, gitvolt_core::forge::types::ReviewDraft, bool)> = Vec::new();
    for items in placed.by_path.values() {
        for it in items {
            if let review::PlacedItem::Draft { draft, at } = it
                && let Some(pos) = &draft.position
            {
                rows.push((thread::note_where(pos), draft.clone(), at.outdated));
            }
        }
    }
    let mut body = div().flex().flex_col().gap(px(8.)).px(px(12.)).pb(px(12.));
    if !rows.is_empty() {
        let mut list = div().flex().flex_col().gap(px(2.)).text_size(px(12.));
        for (i, (where_, draft, outdated)) in rows.into_iter().enumerate() {
            let (m2, pos) = (mr.clone(), draft.position.clone());
            let first = draft.body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_string();
            list = list.child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(8.))
                    .min_w_0()
                    .child(div().id(("mr-pending-where", i)).flex_none().max_w(relative(0.6)).truncate().font_family(crate::app::MONO_FONT).text_size(px(11.)).text_color(theme::link()).cursor_pointer().hover(|d| d.underline()).child(where_).on_click(cx.listener(move |v, _, _, cx| {
                        if let Some(p) = &pos {
                            v.mr_open_note_file(tab, kind, &m2, p, cx);
                        }
                    })))
                    .when(outdated, |d| d.child(div().flex_none().px(px(5.)).rounded(px(3.)).text_size(px(10.)).text_color(theme::orange()).child("Outdated")))
                    .child(div().flex_1().min_w_0().truncate().text_color(theme::text_secondary()).child(first)),
            );
        }
        body = body.child(list);
    }
    for d in &placed.unplaced {
        body = body.child(crate::forge::review::zones::draft_card(this, tab, kind, d, false, cx));
    }
    body = body.child(composer);
    Some(div().id("mr-pending").flex().flex_col().gap(px(8.)).flex_none().rounded(px(8.)).border_1().border_color(theme::mix(theme::yellow(), 0.45, theme::section_border())).bg(theme::panel_bg1()).child(head).child(body).into_any_element())
}

fn t_discussions(this: &GitVolt, tab: u32, number: u64) -> Vec<gitvolt_core::forge::types::ForgeDiscussion> {
    this.tabs.iter().find(|t| t.id == tab).and_then(|t| t.forge.discussions.get(&number).cloned()).unwrap_or_default()
}

pub fn review_composer(this: &mut GitVolt, tab: u32, kind: ForgeKind, number: u64, keep: bool, window: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let (input, event) = this.mr_composer(tab, number, cx);
    let t = this.tabs.iter().find(|t| t.id == tab);
    let session = t.and_then(|t| t.forge.review.clone()).filter(|s| s.number == number);
    let pending = session.as_ref().map_or(0, |s| s.drafts.len());
    let with_pending = pending > 0 || session.as_ref().is_some_and(|s| s.pending_review.is_some());
    let ui = t.and_then(|t| t.mr_views.get(&number));
    let (busy, partial) = ui.map_or((false, None), |u| (u.review.busy, u.review.partial.clone()));
    let text = input.read(cx).text().to_string();
    let required = kind == ForgeKind::Forgejo && event == "requestChanges";
    let ready = !busy && (event == "approve" || !text.trim().is_empty() || (pending > 0 && !required));
    let mode = MODES.iter().find(|m| m.0 == event).copied().unwrap_or(MODES[0]);
    let placeholder = if pending > 0 && event != "approve" && !required { "A summary (optional)" } else { mode.2 };
    input.update(cx, |i, _| i.set_placeholder(placeholder));
    let mut modes = div().flex().flex_none().border_1().border_color(theme::section_border()).rounded(px(6.)).overflow_hidden().bg(theme::control_bg());
    for (i, (e, label, _)) in MODES.iter().copied().enumerate() {
        let on = e == event;
        let color = match (on, e) {
            (true, "approve") => theme::green(),
            (true, "requestChanges") => theme::orange(),
            (true, _) => theme::text_selected(),
            _ => theme::text_secondary(),
        };
        modes = modes.child(crate::ui::anchor(format!("mr-review-mode:{e}"), div().id(("mr-review-mode", i)).flex().items_center().px(px(10.)).py(px(3.)).text_size(px(12.)).text_color(color).cursor_pointer().when(i > 0, |d| d.border_l_1().border_color(theme::section_border())).when(on, |d| d.bg(theme::selected_row())).hover(|d| d.bg(theme::control_hover_bg())).child(label).on_click(cx.listener(move |v, _, _, cx| {
            if let Some(ui) = v.mr_ui(tab, number) {
                ui.review.event = Some(e);
                ui.review.sent = None;
            }
            cx.notify();
        }))));
    }
    let focused = input.read(cx).is_focused(window);
    let area = div().id("mr-review-text").min_h(px(62.)).max_h(px(240.)).px(px(8.)).py(px(5.)).rounded(px(6.)).border_1().border_color(if focused { theme::blue() } else { theme::wip_input_border() }).bg(theme::wip_input_bg()).text_color(theme::text_normal()).text_size(px(12.)).child(input.clone());
    let cancel_on = !(keep && text.is_empty());
    let clear = input.clone();
    let cancel = mr_button("mr-review-cancel", cancel_on).child("Cancel").when(cancel_on, |d| {
        d.on_click(cx.listener(move |v, _, _, cx| {
            clear.update(cx, |i, cx| i.set_text("", cx));
            if !keep && let Some(ui) = v.mr_ui(tab, number) {
                ui.mode = Mode::None;
            }
            cx.notify();
        }))
    });
    let submit = mr_primary("mr-review-send", ready).when(ready && event == "requestChanges", |d| d.bg(theme::button_warn()).border_color(theme::button_warn()).text_color(theme::button_warn_ink())).child(if busy { "Sending…" } else { mode.1 }).when(ready, |d| d.on_click(cx.listener(move |v, _, _, cx| v.mr_review_send(tab, number, cx))));
    let mut notes = div().flex().flex_col().flex_1().min_w(px(180.)).gap(px(4.)).pt(px(3.));
    let note = |s: String| div().text_size(px(11.)).text_color(theme::text_secondary()).child(s);
    if with_pending {
        notes = notes.child(note(if pending == 0 { format!("Sends your pending review on {}", forge_name(kind)) } else { format!("Sends your {} with it", if pending == 1 { "pending comment".to_string() } else { format!("{pending} pending comments") }) }));
    }
    if kind == ForgeKind::GitLab && event == "requestChanges" {
        notes = notes.child(note(GITLAB_CHANGES_NOTE.to_string()));
    }
    if required && pending > 0 {
        notes = notes.child(note(FORGEJO_CHANGES_NOTE.to_string()));
    }
    if let Some(p) = partial {
        notes = notes.child(note(p));
    }
    div()
        .id("mr-review-form")
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(div().flex().child(modes))
        .child(area)
        .child(div().flex().flex_wrap().items_start().gap_x(px(12.)).gap_y(px(4.)).child(notes).child(div().flex().flex_none().ml_auto().gap(px(6.)).child(crate::ui::anchor("mr-review-cancel", cancel)).child(crate::ui::anchor("mr-review-send", submit))))
        .into_any_element()
}

fn edit_form(this: &mut GitVolt, tab: u32, kind: ForgeKind, mr: &ForgeMr, detail: Option<&ForgeMrDetail>, window: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let number = mr.number;
    let edit_values = this.mr_edit_values(tab, number, cx);
    let t = this.tabs.iter().find(|t| t.id == tab);
    let (Some(t), Some(values)) = (t, edit_values) else { return div().into_any_element() };
    let Some(e) = t.mr_views.get(&number).and_then(|u| u.edit.as_ref()) else { return div().into_any_element() };
    let (title, description, labels, colors, busy, open_labels) = (e.title.clone(), e.description.clone(), e.labels.clone(), e.colors.clone(), e.busy, e.open_labels);
    let remote = t.forge.remote.clone();
    let changed = values.title.is_some() || values.description.is_some() || values.labels.is_some();
    let can_save = changed && !title.read(cx).text().trim().is_empty() && !busy;
    let focused = title.read(cx).is_focused(window);
    let field_label = |s: &'static str| div().text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_secondary()).child(s);
    let title_box = div().id("mr-edit-title").flex().items_center().px(px(9.)).h(px(32.)).rounded(px(6.)).border_1().border_color(if focused { theme::blue() } else { theme::wip_input_border() }).bg(theme::wip_input_bg()).text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).child(title);
    // Reviewers and assignees change at once, as in the view's cards; labels go with Save.
    let rows = edit_rows(this, tab, kind, mr, detail, &labels, &colors, remote, open_labels, window, cx);
    let cancel = mr_button("mr-edit-cancel", true).px(px(14.)).py(px(5.)).text_size(px(13.)).child("Cancel").on_click(cx.listener(move |v, _, _, cx| {
        if let Some(ui) = v.mr_ui(tab, number) {
            ui.mode = Mode::None;
            ui.edit = None;
        }
        cx.notify();
    }));
    let save = mr_primary("mr-edit-save", can_save).px(px(14.)).py(px(5.)).text_size(px(13.)).child(if busy { "Saving…" } else { "Save" }).when(can_save, |d| d.on_click(cx.listener(move |v, _, _, cx| v.mr_edit_save(tab, number, cx))));
    div()
        .id("mr-edit-form")
        .flex()
        .flex_col()
        .gap(px(10.))
        .on_key_down(cx.listener(move |v, e: &KeyDownEvent, _, cx| {
            if e.keystroke.key == "escape" && !e.keystroke.modifiers.modified() {
                if let Some(ui) = v.mr_ui(tab, number) {
                    ui.mode = Mode::None;
                    ui.edit = None;
                }
                cx.stop_propagation();
                cx.notify();
            }
        }))
        .child(div().flex().flex_col().gap(px(4.)).child(field_label("Title")).child(title_box))
        .child(div().flex().flex_col().gap(px(4.)).child(field_label("Description")).child(description))
        .child(rows)
        .child(div().flex().justify_end().gap(px(8.)).mt(px(6.)).pt(px(10.)).pb(px(12.)).child(crate::ui::anchor("mr-edit-cancel", cancel)).child(crate::ui::anchor("mr-edit-save", save)))
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn edit_rows(this: &mut GitVolt, tab: u32, kind: ForgeKind, mr: &ForgeMr, detail: Option<&ForgeMrDetail>, labels: &[String], colors: &std::collections::HashMap<String, Option<String>>, remote: Option<String>, open_labels: bool, window: &mut Window, cx: &mut Context<GitVolt>) -> Div {
    use super::people::{Role, label_pill, person_chip, rm_button};
    let number = mr.number;
    let row = |k: &'static str, body: Div| div().flex().items_center().gap(px(8.)).min_h(px(40.)).px(px(10.)).py(px(6.)).child(div().flex_none().w(px(92.)).text_size(px(12.)).text_color(theme::text_secondary()).child(k)).child(body.flex().flex_wrap().flex_1().items_center().gap(px(6.)).min_w_0());
    let add_btn = |id: String, name: String| div().id(ElementId::Name(id.into())).flex().flex_none().items_center().gap(px(4.)).h(px(24.)).px(px(8.)).rounded(px(12.)).border_1().border_dashed().border_color(theme::mix(theme::text_selected(), 0.18, transparent_black())).text_size(px(12.)).text_color(theme::text_secondary()).cursor_pointer().hover(|d| d.border_color(theme::blue()).text_color(theme::text_normal())).child(icon("plus", 12., theme::text_secondary())).child(name);
    let mut card = div().flex().flex_col().flex_none().rounded(px(8.)).border_1().border_color(theme::section_border());
    for (role, label) in [(Role::Reviewers, "Reviewers"), (Role::Assignees, "Assignees")] {
        let list = detail.map(|d| if role == Role::Reviewers { d.reviewers.clone() } else { d.assignees.clone() }).unwrap_or_default();
        let mut body = div();
        for u in &list {
            let who = u.clone();
            let rm = rm_button(format!("mr-edit-rm-{}-{}", role.name(), u.username), &format!("Remove {}", u.name), move |v, cx| v.mr_change_people(tab, kind, number, role, None, Some(who.clone()), Vec::new(), cx), cx);
            body = body.child(person_chip(u, &u.name, Some(rm)));
        }
        let key = format!("mr-edit-add-{}", role.name());
        let (w, taken, remote2, api, rt, open_key) = (cx.weak_entity(), list.clone(), remote.clone(), this.api.clone(), this.rt.clone(), key.clone());
        body = body.child(crate::ui::anchor(key.clone(), add_btn(key, "Add".to_string()).on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            let (Some(remote), Some(at)) = (remote2.clone(), crate::ui::Ui::get(cx).anchor(&open_key)) else { return };
            let w = w.clone();
            let on_pick = Rc::new(move |o: &SearchOption, cx: &mut App| {
                let Some(user) = o.user.clone() else { return };
                w.update(cx, |v, cx| v.mr_change_people(tab, kind, number, role, Some(user), None, Vec::new(), cx)).ok();
            });
            search::open(format!("mr-edit-search-{}", role.name()).into(), label.into(), at, false, Source::People { repo: core_repo(tab), remote }, taken.iter().map(|u| u.id.to_string()).collect(), api.clone(), rt.clone(), on_pick, window, cx);
        })));
        card = card.child(row(label, body).when(role == Role::Assignees, |d| d.border_t_1().border_color(theme::section_border())));
    }
    let mut body = div();
    for l in labels {
        let color = colors.get(l).cloned().flatten().or_else(|| mr.label_colors.get(l).cloned());
        let name = l.clone();
        let rm = rm_button(format!("mr-edit-rm-label-{l}"), &format!("Remove {l}"), move |v, _| {
            if let Some(e) = v.mr_ui(tab, number).and_then(|u| u.edit.as_mut()) {
                e.labels.retain(|x| *x != name);
            }
        }, cx);
        body = body.child(label_pill(l, color.as_deref(), Some(rm)));
    }
    let key = "mr-edit-add-label";
    let (w, taken, remote2, api, rt) = (cx.weak_entity(), labels.to_vec(), remote.clone(), this.api.clone(), this.rt.clone());
    let limit = if kind == ForgeKind::GitHub { 300 } else { 50 };
    let open_search = move |window: &mut Window, cx: &mut App| {
        let (Some(remote), Some(at)) = (remote2.clone(), crate::ui::Ui::get(cx).anchor(key)) else { return };
        let w = w.clone();
        let on_pick = Rc::new(move |o: &SearchOption, cx: &mut App| {
            let (name, color) = (o.label.clone(), o.color.clone());
            w.update(cx, |v, cx| {
                if let Some(e) = v.mr_ui(tab, number).and_then(|u| u.edit.as_mut()) {
                    e.colors.insert(name.clone(), color.clone());
                    if !e.labels.contains(&name) {
                        e.labels.push(name.clone());
                    }
                }
                cx.notify();
            })
            .ok();
        });
        search::open("mr-edit-search-labels".into(), "Labels".into(), at, false, Source::Labels { repo: core_repo(tab), remote, limit }, taken.clone(), api.clone(), rt.clone(), on_pick, window, cx);
    };
    if open_labels && crate::ui::Ui::get(cx).anchor(key).is_some() {
        if let Some(e) = this.mr_ui(tab, number).and_then(|u| u.edit.as_mut()) {
            e.open_labels = false;
        }
        let opener = open_search.clone();
        let app: &mut App = cx;
        window.defer(app, move |window, cx| opener(window, cx));
    } else if open_labels {
        cx.notify();
    }
    let opener = open_search;
    body = body.child(crate::ui::anchor(key, add_btn(key.to_string(), "Add".to_string()).on_mouse_down(MouseButton::Left, move |_, window, cx| {
        cx.stop_propagation();
        opener(window, cx);
    })));
    card = card.child(row("Labels", body).border_t_1().border_color(theme::section_border()));
    let _ = ReviewDecision::None;
    card
}
