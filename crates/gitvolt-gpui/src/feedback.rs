//! What's going on (`ui/src/statusbar`, `ui/src/queue`, `ui/src/banner`, `ui/src/app/ActivityModal.tsx`,
//! `ui/src/tabs/NotificationsBell.tsx`, `ui/src/auth`): the status bar's items, the action queue's chip, the
//! worktree's banners, the bell and the activity log, the credential prompt, and the background fetch.

use crate::app::{GitVolt, Tab, icon};
use crate::commit_box::spinner;
use crate::ops::{ActivityEntry, chip_text, queue_idle, rebase_status};
use crate::theme;
use crate::ui::arm::{self, ArmRequest, Origin};
use crate::ui::banner::{Banner, BannerAction};
use crate::ui::controls::{self, ButtonKind, check_row};
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::menu::{self, MenuRow, OpenOptions};
use crate::ui::modal::{self, Modal};
use crate::ui::toast;
use crate::ui::tooltip::tip;
use crate::write::{WriteCtx, call};
use gitvolt_core::events::{OpKind, OpOutcome, StashStep};
use gitvolt_core::journal::{self, BannerKind};
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::rc::Rc;
use std::time::Duration;

/// How long a user's fetch or write runs on its button alone before the bar shows it, with Cancel.
pub const SLOW_MS: Duration = Duration::from_millis(2000);
/// How long an autostash step runs before the status names it.
pub const SLOW_STASH: Duration = Duration::from_secs(60);
/// `fetchIntervalSecs` as it runs: its default when unread; 0 turns it off.
pub fn fetch_interval(secs: Option<u64>) -> Option<Duration> {
    match secs.unwrap_or(FETCH_DEFAULT_SECS) {
        0 => None,
        n => Some(Duration::from_secs(n.clamp(FETCH_MIN_SECS, FETCH_MAX_SECS))),
    }
}

const FETCH_DEFAULT_SECS: u64 = 60;
const FETCH_MIN_SECS: u64 = 60;
const FETCH_MAX_SECS: u64 = 86_400;

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// A running write's words, for the bar.
pub fn write_text(o: &crate::ops::OpInfo) -> String {
    if let Some((step, _, at)) = &o.stash
        && at.elapsed() >= SLOW_STASH
    {
        return match step {
            StashStep::Saving => "Saving your changes…",
            StashStep::Restoring => "Restoring your changes…",
            StashStep::RestoringFiles => "Restoring files…",
        }
        .to_string();
    }
    if let Some(r) = rebase_status(o) {
        return r;
    }
    if o.kind == OpKind::Commit { "Committing…".to_string() } else { format!("{}…", capitalized(&o.label)) }
}

/// §6.4 and §5.1's words.
pub fn banner_text(b: &journal::Banner) -> String {
    let stash = b.stash_message.as_deref().unwrap_or_default();
    match b.kind {
        BannerKind::AutostashRefused => format!("Your changes from before {} are in stash \"{stash}\": they conflict with {}.", b.label, b.target.as_deref().unwrap_or("it")),
        BannerKind::AutostashConflicts => format!("Your restored changes conflict in {} {}; resolve them in Conflicted. The stash is kept until you drop it.", b.files, if b.files == 1 { "file" } else { "files" }),
        BannerKind::AutostashPartial => format!("Your changes from before {} were partly restored; the stash still has everything.", b.label),
        BannerKind::AutostashStopped => format!("{} didn't run: saving your changes was stopped. They're in stash \"{stash}\".", capitalized(&b.label)),
        BannerKind::AbortedWork => format!("Your edits from the stop are in stash \"{stash}\": the rebase was aborted.{}", b.target.as_ref().map_or(String::new(), |t| format!(" Your commits from the stop are on {t}."))),
        BannerKind::AbortInterrupted => format!("GitVolt stopped before it could {}: the abort may not have run; your work is in the working tree or the stash \"{stash}\"{}.", b.label, b.target.as_ref().map_or(String::new(), |t| format!(", and your commits from the stop are on {t}"))),
        BannerKind::Recovery => format!("GitVolt stopped during {}. Your changes are safe in {}.", b.label, if b.snapshot { "a snapshot" } else { "the autostash" }),
    }
}

/// The Activity tab's rows, newest first: a run of identical quiet background ones shares a row.
pub fn activity_rows(activity: &[ActivityEntry], errors_only: bool, hide_background: bool, focus: Option<u64>) -> Vec<Vec<usize>> {
    let quiet = |e: &ActivityEntry| e.background && e.outcome != OpOutcome::Failed;
    let same = |a: &ActivityEntry, b: &ActivityEntry| quiet(a) && quiet(b) && a.kind == b.kind && a.label == b.label && a.outcome == b.outcome && a.command == b.command && a.message.is_none() && b.message.is_none() && a.output.is_empty() && b.output.is_empty() && a.remote.is_empty() && b.remote.is_empty();
    let mut rows: Vec<Vec<usize>> = Vec::new();
    for (i, e) in activity.iter().enumerate() {
        let focused = Some(e.op) == focus;
        if !focused && errors_only && e.outcome != OpOutcome::Failed {
            continue;
        }
        if !focused && hide_background && quiet(e) {
            continue;
        }
        match rows.last_mut() {
            Some(prev) if !focused && Some(activity[prev[0]].op) != focus && same(&activity[prev[0]], e) => prev.push(i),
            _ => rows.push(vec![i]),
        }
    }
    rows
}

/// One entry as plain text.
pub fn entry_text(e: &ActivityEntry) -> String {
    let outcome = format!("{:?}", e.outcome).to_lowercase();
    let kind = format!("{:?}", e.kind).to_lowercase();
    let head = format!("{} · {kind} · {} · {} ms · {outcome}", if e.label.is_empty() { "(no repo)" } else { &e.label }, if e.background { "background" } else { "user" }, e.duration_ms);
    let mut parts = vec![head];
    parts.extend(e.command.clone());
    parts.extend(e.message.clone());
    if !e.remote.is_empty() {
        parts.push(e.remote.iter().map(|l| format!("server: {}", l.text)).collect::<Vec<_>>().join("\n"));
    }
    if !e.output.is_empty() {
        parts.push(e.output.join("\n"));
    }
    parts.join("\n")
}

#[derive(Default)]
pub struct ActivityShown {
    pub activity: ActivityUi,
    pub ops: ActivityOps,
}

#[derive(Default)]
pub struct ActivityOps {
    pub activity: Vec<ActivityEntry>,
}

/// The modal's own state.
#[derive(Default, Clone)]
pub struct ActivityUi {
    pub commands_view: bool,
    pub errors_only: bool,
    pub show_background: bool,
    pub focus_op: Option<u64>,
    pub focus_command: Option<u64>,
    pub commands: Vec<Value>,
    pub open_rows: std::collections::HashSet<u64>,
}

const ACTIVITY_ID: &str = "activity-log";
const AUTH_ID: &str = "auth-prompt";

impl GitVolt {
    /// Runs a bar's timer: when an op's been going long enough for the bar to say so.
    pub fn after_op_started(&mut self, cx: &mut Context<Self>) {
        let timer = cx.background_executor().timer(SLOW_MS + Duration::from_millis(50));
        cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
    }

    /// A credential prompt came or went: its dialog follows, one at a time.
    pub fn sync_auth_modal(&mut self, cx: &mut Context<Self>) {
        let first = self.ops.prompts.first().cloned();
        let shown = self.auth_shown;
        if shown == first.as_ref().map(|p| p.prompt) {
            return;
        }
        self.auth_shown = first.as_ref().map(|p| p.prompt);
        let this = cx.weak_entity();
        self.with_window(cx, move |window, cx| {
            modal::close(AUTH_ID, window, cx);
            if let Some(p) = first {
                open_auth(p, this, window, cx);
            }
        });
    }

    /// The app's one background fetch, each interval of the settings', of the shown tab: a hidden one
    /// does no work, and between fetches nothing wakes.
    pub fn start_fetch_schedule(&mut self, cx: &mut Context<Self>) {
        if cfg!(test) || std::env::var_os("GITVOLT_NO_BACKGROUND_FETCH").is_some() {
            return;
        }
        let api = self.api.clone();
        let state = self.rt.spawn(async move { api.dispatch(gitvolt_core::api::Request::LoadState).await });
        cx.spawn(async move |this, cx| {
            let saved = state.await.ok().and_then(Result::ok).and_then(|v| v["settings"]["fetchIntervalSecs"].as_u64());
            let Some(interval) = fetch_interval(saved) else { return };
            loop {
                cx.background_executor().timer(interval).await;
                let live = this.update(cx, |v, cx| {
                    let Some(tab) = v.tabs.get(v.active) else { return };
                    // A fetch by hand this interval counts.
                    if tab.last_fetch.is_none_or(|at| at.elapsed() + Duration::from_secs(1) >= interval) {
                        v.run_fetch(tab.id, true, None, cx);
                    }
                });
                if live.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// Cancels the queue's running op, drops an item, resumes or clears.
    fn queue_rows(&self, repo: u32, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(q) = self.ops.queues.get(&repo) else { return Vec::new() };
        let mut rows = Vec::new();
        let action = |id: String, label: String, glyph: &'static str, tooltip: String, body: Value| {
            let weak = this.clone();
            MenuRow::action(id, label, glyph, tooltip, move |_, cx| {
                let body = body.clone();
                weak.update(cx, |v, cx| v.fire(body, cx)).ok();
            })
        };
        if let Some(r) = &q.running {
            rows.push(action("queue.running".into(), format!("Cancel {}", r.label), "circle-stop", "Stop the running operation".into(), call("cancelOp", json!({ "op": r.op }))));
        }
        for item in &q.queued {
            let tip = if q.stopped.is_some() { "Not run: drop it from the queue" } else { "Queued: drop it from the queue" };
            rows.push(action(format!("queue.{}", item.id), format!("Remove {}", item.label), "x", tip.into(), call("queueRemove", json!({ "repo": repo, "id": item.id }))));
        }
        if q.stopped.is_some() {
            rows.push(MenuRow::Separator);
            rows.push(action("queue.resume".into(), "Resume".into(), "play", "Run the items that didn't run, each against the repository as it is now".into(), call("queueResume", json!({ "repo": repo }))));
            rows.push(action("queue.clear".into(), "Clear".into(), "list-x", "Drop the items that didn't run".into(), call("queueClear", json!({ "repo": repo }))));
        }
        rows
    }

    /// The Debug window, at this command.
    pub fn open_debug(&mut self, command: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
        self.activity.commands_view = true;
        self.activity.focus_command = command;
        self.activity.focus_op = None;
        self.read_commands(cx);
        self.show_activity(window, cx);
    }

    /// The activity log, at this op with its server output open.
    pub fn open_activity(&mut self, op: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
        self.activity.commands_view = false;
        self.activity.focus_op = op;
        if let Some(op) = op {
            self.activity.open_rows.insert(op);
        }
        self.show_activity(window, cx);
    }

    fn read_commands(&mut self, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(gitvolt_core::api::Request::CommandLog).await });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(v)) = read.await {
                this.update(cx, |this, cx| {
                    this.activity.commands = v.as_array().cloned().unwrap_or_default();
                    this.activity.commands.reverse();
                    cx.notify();
                    cx.refresh_windows();
                })
                .ok();
            }
        })
        .detach();
    }

    fn show_activity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if modal::is_open(ACTIVITY_ID, cx) {
            cx.refresh_windows();
            return;
        }
        let this = cx.weak_entity();
        let size = window.viewport_size();
        self.sync_activity_shown();
        let shown = self.activity_shown.clone();
        let m = Modal::new(ACTIVITY_ID, move |window, cx| activity_view(&shown.borrow(), this.clone(), window, cx))
        .width(size.width.as_f32() * 0.8);
        modal::open(m, window, cx);
    }

    /// What the open log draws: overlays build while the app is being updated, so it can't be read then.
    pub fn sync_activity_shown(&mut self) {
        *self.activity_shown.borrow_mut() = ActivityShown { activity: self.activity.clone(), ops: ActivityOps { activity: self.ops.activity.clone() } };
    }

    pub fn activity_open(&self, cx: &App) -> bool {
        modal::is_open(ACTIVITY_ID, cx)
    }

    /// The tab's banners, from its worktree's journal.
    pub fn banner_list(&self, tab: &Tab, cx: &mut Context<Self>) -> Vec<Banner> {
        let Some(state) = self.ops.journal(tab.id, &tab.worktree) else { return Vec::new() };
        let ctx = WriteCtx { repo: tab.id, worktree: tab.worktree.clone() };
        state.banners.iter().map(|b| self.banner_of(b, &ctx, cx)).collect()
    }

    fn banner_of(&self, b: &journal::Banner, ctx: &WriteCtx, cx: &mut Context<Self>) -> Banner {
        let weak = cx.weak_entity();
        let entry = b.entry;
        let restore = b.kind == BannerKind::Recovery && b.snapshot;
        let applies = matches!(b.kind, BannerKind::AutostashRefused | BannerKind::AutostashPartial | BannerKind::AutostashStopped | BannerKind::AbortedWork | BannerKind::AbortInterrupted) || (b.kind == BannerKind::Recovery && !b.snapshot && b.stash.is_some());
        let mut actions = Vec::new();
        if applies || restore {
            let (w, c) = (weak.clone(), ctx.clone());
            actions.push(BannerAction {
                label: if restore { "Restore" } else { "Apply" }.into(),
                run: Rc::new(move |_, cx| {
                    let c = c.clone();
                    w.update(cx, |v, cx| v.apply_kept_stash(c, entry, cx)).ok();
                }),
            });
        }
        if let Some(sha) = b.stash.clone() {
            let w = weak.clone();
            actions.push(BannerAction {
                label: "Show".into(),
                run: Rc::new(move |_, cx| {
                    let sha = sha.clone();
                    w.update(cx, |v, cx| {
                        if v.in_graph(&sha) {
                            v.go_to_sha(&sha, cx);
                        } else {
                            v.show_toast("Not in the loaded history", cx);
                        }
                    })
                    .ok();
                }),
            });
        }
        if b.can_drop {
            let (w, c, binary) = (weak.clone(), ctx.clone(), b.binary);
            actions.push(BannerAction {
                label: "Drop stash".into(),
                run: Rc::new(move |window, cx| {
                    let (w2, c2) = (w.clone(), c.clone());
                    if binary {
                        let mut req = ArmRequest::danger("Click again to drop the stash, your only copy of the binary files", "Drop the stash?", "Drop stash");
                        req.body = "Some conflicted files are binary: only the current version of those is kept, and the stash is the only copy of yours.".into();
                        arm::request(
                            req,
                            Origin::None,
                            move |a, _, cx| {
                                if a.ok {
                                    let c = c2.clone();
                                    w2.update(cx, |v, cx| v.dismiss_banner(c, entry, true, cx)).ok();
                                }
                            },
                            window,
                            cx,
                        );
                    } else {
                        w2.update(cx, |v, cx| v.dismiss_banner(c2, entry, true, cx)).ok();
                    }
                }),
            });
        }
        let (w, c, label, stash_kept) = (weak, ctx.clone(), b.label.clone(), b.stash.is_some());
        actions.push(BannerAction {
            label: "×".into(),
            run: Rc::new(move |window, cx| {
                let (w2, c2) = (w.clone(), c.clone());
                if restore {
                    let mut req = ArmRequest::danger("Click again to dismiss: the snapshot is lost", "Dismiss without restoring?", "Dismiss");
                    req.body = format!("The snapshot is the only copy of your changes from before {label}. Once dismissed, GitVolt can't restore them.").into();
                    arm::request(
                        req,
                        Origin::None,
                        move |a, _, cx| {
                            if a.ok {
                                let c = c2.clone();
                                w2.update(cx, |v, cx| v.dismiss_banner(c, entry, false, cx)).ok();
                            }
                        },
                        window,
                        cx,
                    );
                } else {
                    w2.update(cx, |v, cx| v.dismiss_banner(c2, entry, false, cx)).ok();
                }
                let _ = stash_kept;
            }),
        });
        let accent = if b.kind == BannerKind::Recovery { theme::blue() } else { theme::orange() };
        Banner { id: format!("banner-{entry}").into(), text: banner_text(b).into(), accent, actions }
    }
}

/// The bell's menu: background errors, the activity log, Clear.
pub fn bell_rows(v: &GitVolt, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
    let activity = {
        let weak = this.clone();
        MenuRow::action("bell.activity", "Activity log", "history", "Every finished fetch and clone, with git's message", move |window, cx| {
            weak.update(cx, |v, cx| v.open_activity(None, window, cx)).ok();
        })
    };
    if v.ops.errors.is_empty() {
        return vec![MenuRow::action("bell.none", "No notifications", "bell-off", "Background errors (a failed background fetch) appear here", |_, _| {}).disabled("Nothing yet"), MenuRow::Separator, activity];
    }
    let mut rows: Vec<MenuRow> = v.ops.errors.iter().enumerate().map(|(i, e)| MenuRow::action(format!("bell.{i}"), e.message.clone(), "triangle-alert", e.message.clone(), |_, _| {})).collect();
    let weak = this.clone();
    rows.push(MenuRow::Separator);
    rows.push(activity);
    rows.push(MenuRow::action("bell.clear", "Clear notifications", "bell-off", "Forget these notifications", move |_, cx| {
        weak.update(cx, |v, cx| {
            v.ops.errors.clear();
            v.ops.unread = 0;
            cx.notify();
        })
        .ok();
    }));
    rows
}

/// The tab bar's bell, with its count.
pub fn bell(v: &GitVolt, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let unread = v.ops.unread;
    let open = menu::is_open_on("bell", cx);
    tip(
        "Notifications: background errors",
        crate::ui::anchor(
            "bell",
            div()
                .id("bell")
                .relative()
                .flex()
                .flex_none()
                .w(px(34.))
                .h_full()
                .items_center()
                .justify_center()
                .when(open, |d| d.bg(theme::hover_row()))
                .hover(|d| d.bg(theme::hover_row()))
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.ops.unread = 0;
                    let weak = cx.weak_entity();
                    let rows = bell_rows(this, &weak);
                    menu::open_below("bell", rows, OpenOptions { label: Some("Notifications".into()), ..Default::default() }, window, cx);
                }))
                .child(icon("bell", 16., theme::text_secondary()))
                .when(unread > 0, |d| d.child(div().absolute().top(px(4.)).right(px(4.)).min_w(px(14.)).h(px(14.)).px(px(3.)).rounded(px(7.)).bg(theme::red()).text_color(theme::text_on_accent()).text_size(px(10.)).flex().items_center().justify_center().child(unread.to_string()))),
        ),
    )
}

fn link(id: &'static str, label: impl Into<SharedString>, cx: &mut Context<GitVolt>, f: impl Fn(&mut GitVolt, &mut Context<GitVolt>) + 'static) -> Stateful<Div> {
    div().id(id).ml(px(6.)).text_color(theme::blue()).cursor_pointer().hover(|d| d.underline()).on_click(cx.listener(move |this, _, _, cx| f(this, cx))).child(label.into())
}

/// The middle of the status bar: the queue's chip, then the one thing it says.
pub fn status_items(v: &GitVolt, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    let mut out: Vec<AnyElement> = Vec::new();
    let tab = v.tabs.get(v.active);
    let repo = tab.map(|t| t.id);
    if let Some(repo) = repo
        && !queue_idle(v.ops.queues.get(&repo))
        && let Some(q) = v.ops.queues.get(&repo)
    {
        let stopped = q.stopped.is_some();
        out.push(
            tip(
                "The action queue: what runs now and what's next",
                crate::ui::anchor(
                    "queue-chip",
                    div()
                        .id("queue-chip")
                        .flex()
                        .h_full()
                        .items_center()
                        .px(px(1.))
                        .rounded(px(3.))
                        .whitespace_nowrap()
                        .when(stopped, |d| d.text_color(theme::orange()))
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::hover_row()))
                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            let weak = cx.weak_entity();
                            let rows = this.queue_rows(repo, &weak);
                            menu::open_below("queue-chip", rows, OpenOptions { label: Some("Action queue".into()), ..Default::default() }, window, cx);
                        }))
                        .child(chip_text(q)),
                ),
            )
            .into_any_element(),
        );
    }
    let item = || div().flex().flex_none().h_full().items_center().gap(px(4.)).px(px(1.)).whitespace_nowrap();
    if let Some(p) = v.ops.prompts.first() {
        let op = p.op;
        out.push(item().text_color(theme::text_selected()).child("Waiting for authentication…").child(link("sb-auth-cancel", "Cancel", cx, move |this, cx| this.cancel_op(op, cx))).into_any_element());
    } else if let Some(o) = v.ops.clone_op() {
        let op = o.op;
        let text = format!("Cloning…{}", o.percent.map_or(String::new(), |p| format!(" {p}%")));
        out.push(item().child(spinner("sb-spin-clone", 12., theme::text_secondary())).child(text).child(link("sb-clone-cancel", "Cancel", cx, move |this, cx| this.cancel_op(op, cx))).into_any_element());
    } else if let Some(o) = v.ops.user_fetch().filter(|o| o.started.elapsed() >= SLOW_MS) {
        let op = o.op;
        let text = format!("Fetching {}…{}", o.label, o.percent.map_or(String::new(), |p| format!(" {p}%")));
        out.push(item().child(spinner("sb-spin-fetch", 12., theme::text_secondary())).child(text).child(link("sb-fetch-cancel", "Cancel", cx, move |this, cx| this.cancel_op(op, cx))).into_any_element());
    } else if let Some(o) = v.ops.user_write().filter(|o| rebase_status(o).is_some() || o.started.elapsed() >= SLOW_MS) {
        let op = o.op;
        let cancel = match &o.stash {
            Some((_, message, _)) => format!("Stop — your changes stay in stash {message}"),
            None => "Cancel".to_string(),
        };
        out.push(item().child(spinner("sb-spin-write", 12., theme::text_secondary())).child(write_text(o)).child(link("sb-write-cancel", cancel, cx, move |this, cx| this.cancel_op(op, cx))).into_any_element());
    }
    if let Some(why) = tab.and_then(|t| t.fetch_skipped.clone()) {
        out.push(item().text_color(theme::status_modified()).child(icon("triangle-alert", 12., theme::status_modified())).child(why).into_any_element());
    }
    out
}

fn activity_view(v: &ActivityShown, weak: WeakEntity<GitVolt>, window: &Window, _: &mut App) -> AnyElement {
    let height = window.viewport_size().height.as_f32() * 0.8;
    let commands_view = v.activity.commands_view;
    let tab_button = |id: &'static str, label: &'static str, on: bool, go_commands: bool, weak: WeakEntity<GitVolt>| {
        div()
            .id(id)
            .px(px(12.))
            .py(px(4.))
            .rounded(px(4.))
            .cursor_pointer()
            .when(on, |d| d.bg(theme::selected_row()).text_color(theme::text_selected()))
            .when(!on, |d| d.text_color(theme::text_secondary()).hover(|d| d.bg(theme::hover_row()).text_color(theme::text_normal())))
            .on_click(move |_, _, cx| {
                weak.update(cx, |v, cx| {
                    v.activity.commands_view = go_commands;
                    if go_commands {
                        v.read_commands(cx);
                    }
                    cx.refresh_windows();
                })
                .ok();
            })
            .child(label)
    };
    let close = div().id("activity-close").flex().size(px(22.)).items_center().justify_center().rounded(px(3.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).on_click(|_, window, cx| modal::close(ACTIVITY_ID, window, cx)).child(icon("x", 14., theme::text_secondary()));
    let head = div().flex().flex_none().items_center().gap(px(8.)).child(div().flex().flex_1().min_w_0().gap(px(2.)).child(tab_button("activity-tab", "Activity", !commands_view, false, weak.clone())).child(tab_button("commands-tab", "Commands", commands_view, true, weak.clone()))).child(close);
    let body = if commands_view { commands_body(v) } else { activity_body(v, weak) };
    div().flex().flex_col().gap(px(10.)).h(px(height - 32.)).child(head).child(body).into_any_element()
}

fn activity_body(v: &ActivityShown, weak: WeakEntity<GitVolt>) -> AnyElement {
    let a = &v.activity;
    let rows = activity_rows(&v.ops.activity, a.errors_only, !a.show_background, a.focus_op);
    let shown: usize = rows.iter().map(Vec::len).sum();
    let total = v.ops.activity.len();
    let check = |id: &'static str, label: &'static str, on: bool, f: fn(&mut ActivityUi, bool), weak: WeakEntity<GitVolt>| {
        check_row(id, on, label, move |next, _, cx| {
            weak.update(cx, |v, cx| {
                f(&mut v.activity, next);
                cx.refresh_windows();
            })
            .ok();
        })
    };
    let all_text = rows.iter().flatten().map(|i| entry_text(&v.ops.activity[*i])).collect::<Vec<_>>().join("\n\n");
    let toolbar = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(16.))
        .min_h(px(28.))
        .child(check("act-errors", "Errors only", a.errors_only, |a, on| a.errors_only = on, weak.clone()))
        .child(check("act-background", "Hide background", !a.show_background, |a, on| a.show_background = !on, weak.clone()))
        .child(div().flex_1().text_color(theme::text_secondary()).child(format!("{shown} of {total}")))
        .child(controls::button("act-copy-all", "Copy all", ButtonKind::Plain, shown > 0).when(shown > 0, |d| {
            d.on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(all_text.clone()));
                toast::show("Copied", cx);
            })
        }));
    let empty = if total == 0 {
        "No activity yet: every finished operation appears here"
    } else if a.errors_only {
        "No failures"
    } else {
        "Only background activity: untick Hide background to see it"
    };
    let mut list = div().id("activity-list").flex().flex_col().flex_1().min_h_0().overflow_y_scroll().border_t_1().border_color(theme::section_border());
    if rows.is_empty() {
        list = list.child(div().p(px(8.)).text_color(theme::text_secondary()).child(empty));
    }
    for run in rows {
        let e = &v.ops.activity[run[0]];
        let open = e.outcome == OpOutcome::Failed || a.open_rows.contains(&e.op) || a.focus_op == Some(e.op);
        let color = match e.outcome {
            OpOutcome::Failed => theme::red(),
            OpOutcome::Ok => theme::green(),
            _ => theme::text_secondary(),
        };
        let has_detail = run.len() > 1 || e.command.is_some() || e.message.is_some() || !e.remote.is_empty() || !e.output.is_empty();
        let (op, w) = (e.op, weak.clone());
        let pill = |t: String| div().px(px(5.)).rounded(px(3.)).bg(theme::control_bg()).text_size(px(11.)).text_color(theme::text_secondary()).child(t);
        let mut row = div()
            .id(("activity-row", e.op))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(6.))
            .h(px(26.))
            .flex_none()
            .when(has_detail, |d| {
                d.cursor_pointer().hover(|d| d.bg(theme::hover_row())).on_click(move |_, _, cx| {
                    w.update(cx, |v, cx| {
                        if !v.activity.open_rows.remove(&op) {
                            v.activity.open_rows.insert(op);
                        }
                        cx.refresh_windows();
                    })
                    .ok();
                })
            })
            .child(div().size(px(8.)).rounded_full().bg(color))
            .child(div().flex_1().min_w_0().truncate().text_color(theme::text_normal()).child(if e.label.is_empty() { "(no repo)".to_string() } else { e.label.clone() }))
            .child(pill(format!("{:?}", e.kind).to_lowercase()))
            .child(pill(if e.background { "background".into() } else { "user".into() }));
        if matches!(e.outcome, OpOutcome::Cancelled | OpOutcome::Skipped) {
            row = row.child(pill(format!("{:?}", e.outcome).to_lowercase()));
        }
        if run.len() > 1 {
            row = row.child(pill(format!("×{}", run.len())));
        }
        row = row.child(div().flex_none().text_size(px(11.)).text_color(theme::text_secondary()).child(if e.duration_ms < 1000 { format!("{} ms", e.duration_ms) } else { format!("{:.1} s", e.duration_ms as f64 / 1000.) }));
        list = list.child(row);
        if open && has_detail {
            let mono = || div().font_family(crate::diff::view::FONT).text_size(px(12.)).px(px(22.)).py(px(2.));
            let mut detail = div().flex().flex_col().flex_none();
            if let Some(c) = &e.command {
                detail = detail.child(mono().text_color(theme::text_secondary()).child(format!("$ {c}")));
            }
            if let Some(m) = &e.message {
                detail = detail.child(mono().text_color(if e.outcome == OpOutcome::Failed { theme::red() } else { theme::text_normal() }).child(m.clone()));
            }
            let lines = e.remote.iter().filter(|l| !format!("{:?}", l.kind).to_lowercase().contains("boilerplate")).count();
            if lines > 0 {
                detail = detail.child(div().px(px(22.)).pt(px(4.)).text_size(px(11.)).text_color(theme::text_secondary()).child(format!("Server output ({lines} {})", if lines == 1 { "line" } else { "lines" })));
                for l in &e.remote {
                    detail = detail.child(mono().text_color(theme::text_normal()).child(l.text.clone()));
                }
            }
            if !e.output.is_empty() {
                detail = detail.child(div().px(px(22.)).pt(px(4.)).text_size(px(11.)).text_color(theme::text_secondary()).child(format!("Output ({} {})", e.output.len(), if e.output.len() == 1 { "line" } else { "lines" })));
                detail = detail.child(mono().text_color(theme::text_normal()).child(e.output.join("\n")));
            }
            list = list.child(detail);
        }
    }
    div().flex().flex_col().flex_1().min_h_0().gap(px(8.)).child(toolbar).child(list).into_any_element()
}

fn commands_body(v: &ActivityShown) -> AnyElement {
    let mut list = div().id("commands-list").flex().flex_col().flex_1().min_h_0().overflow_y_scroll().border_t_1().border_color(theme::section_border());
    for c in &v.activity.commands {
        let id = c["id"].as_u64().unwrap_or(0);
        let focused = v.activity.focus_command == Some(id);
        let failed = c["ok"] == false;
        let text = c["args"].as_array().map_or_else(|| c["command"].as_str().unwrap_or_default().to_string(), |a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" "));
        let stderr = c["stderr"].as_str().filter(|s| !s.is_empty() && (focused || failed)).map(str::to_string);
        list = list.child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .px(px(6.))
                .py(px(2.))
                .when(focused, |d| d.bg(theme::selected_row()))
                .child(div().flex().gap(px(8.)).child(div().size(px(8.)).mt(px(5.)).flex_none().rounded_full().bg(if failed { theme::red() } else { theme::green() })).child(div().flex_1().min_w_0().font_family(crate::diff::view::FONT).text_size(px(12.)).text_color(theme::text_normal()).child(format!("$ git {text}"))))
                .when_some(stderr, |d, s| d.child(div().pl(px(16.)).font_family(crate::diff::view::FONT).text_size(px(12.)).text_color(theme::red()).child(s))),
        );
    }
    list.into_any_element()
}

/// The credential prompt: one at a time; the credential helper decides whether anything is remembered.
fn open_auth(p: crate::ops::AuthPrompt, this: WeakEntity<GitVolt>, window: &mut Window, cx: &mut App) {
    let input = cx.new(|cx| TextInput::new(cx).secret(p.secret).claim_escape());
    let id = p.prompt;
    type Answer = Rc<dyn Fn(Option<String>, &mut App)>;
    let answer: Answer = {
        let this = this.clone();
        Rc::new(move |value, cx| {
            this.update(cx, |v, cx| v.answer_auth(id, value, cx)).ok();
        })
    };
    let submit = {
        let (input, answer) = (input.clone(), answer.clone());
        Rc::new(move |cx: &mut App| {
            let value = input.read(cx).text().to_string();
            answer(Some(value), cx);
        })
    };
    let sub = cx.subscribe(&input, {
        let (submit, answer) = (submit.clone(), answer.clone());
        move |_, e: &InputEvent, cx| match e {
            InputEvent::Submit { .. } => submit(cx),
            InputEvent::Escape => answer(None, cx),
            _ => {}
        }
    });
    let focus = input.read(cx).focus.clone();
    let text = p.text.trim().to_string();
    let ssh = p.ssh.clone();
    let on_close = answer.clone();
    let mut m = Modal::new(AUTH_ID, move |_, cx| {
        let _ = &cx;
        let (cancel, ok) = (answer.clone(), submit.clone());
        let use_ssh = this.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().flex().items_center().gap(px(8.)).child(icon("key-round", 16., theme::text_selected())).child(modal::title("Authentication required")))
            .child(div().mt(px(6.)).text_color(theme::text_secondary()).child(text.clone()))
            .when_some(ssh.clone(), |d, offer| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(px(6.))
                        .px(px(10.))
                        .py(px(8.))
                        .bg(theme::app_bg())
                        .border_1()
                        .border_color(theme::section_border())
                        .rounded(px(6.))
                        .child(div().text_color(theme::text_secondary()).child("This remote uses HTTPS, so git needs a username and password."))
                        .child(controls::button("auth-ssh", format!("Use SSH for {} instead", offer.remote), ButtonKind::Plain, true).on_click(move |_, _, cx| {
                            use_ssh.update(cx, |v, cx| v.choose_ssh(id, cx)).ok();
                        }))
                        .child(div().font_family(crate::diff::view::FONT).text_size(px(11.)).text_color(theme::text_secondary()).child(offer.url)),
                )
            })
            .child(div().h(px(30.)).px(px(8.)).flex().items_center().bg(theme::app_bg()).border_1().border_color(theme::blue()).rounded(px(4.)).text_color(theme::text_selected()).child(input.clone()))
            .child(div().text_size(px(11.)).text_color(theme::text_secondary()).child("Remembering credentials is up to your git credential helper."))
            .child(
                modal::actions_row()
                    .child(controls::button("auth-cancel", "Cancel", ButtonKind::Plain, true).on_click(move |_, _, cx| cancel(None, cx)))
                    .child(controls::button("auth-submit", "Submit", ButtonKind::Primary, true).on_click(move |_, _, cx| ok(cx))),
            )
            .into_any_element()
    })
    .focus(focus)
    .on_close(move |_, cx| {
        // Esc or a press on the backdrop, while it's still waiting: the prompt's answered with nothing.
        on_close(None, cx);
    });
    m.keep.push(sub);
    modal::open(m, window, cx);
}

impl GitVolt {
    fn answer_auth(&mut self, prompt: u64, answer: Option<String>, cx: &mut Context<Self>) {
        // The core's `authResolved` closes it; a token never appears anywhere else.
        if !self.ops.prompts.iter().any(|p| p.prompt == prompt) {
            return;
        }
        self.ops.prompts.retain(|p| p.prompt != prompt);
        self.sync_auth_modal(cx);
        let body = call("authAnswer", json!({ "prompt": prompt, "answer": answer }));
        if cfg!(test) {
            self.sent.borrow_mut().push(json!({ "method": "authAnswer", "prompt": prompt, "answered": body["params"]["answer"].is_string() }));
            return;
        }
        self.fire(body, cx);
        cx.notify();
    }

    fn choose_ssh(&mut self, prompt: u64, cx: &mut Context<Self>) {
        self.ops.prompts.retain(|p| p.prompt != prompt);
        self.sync_auth_modal(cx);
        self.fire(call("authUseSsh", json!({ "prompt": prompt })), cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitvolt_core::journal::Banner as JBanner;
    use std::prelude::v1::test;
    use std::time::Instant;

    fn entry(op: u64, background: bool, outcome: OpOutcome) -> ActivityEntry {
        ActivityEntry { op, kind: OpKind::Fetch, label: "fixture".into(), background, duration_ms: 5, outcome, message: None, command: Some("git fetch".into()), output: Vec::new(), remote: Vec::new() }
    }

    #[test]
    fn quiet_background_runs_share_a_row_and_hide_by_default() {
        let list = vec![entry(5, true, OpOutcome::Ok), entry(4, true, OpOutcome::Ok), entry(3, false, OpOutcome::Ok), entry(2, true, OpOutcome::Failed)];
        assert_eq!(activity_rows(&list, false, false, None), vec![vec![0, 1], vec![2], vec![3]]);
        assert_eq!(activity_rows(&list, false, true, None), vec![vec![2], vec![3]]);
        assert_eq!(activity_rows(&list, true, false, None), vec![vec![3]]);
        // The focused op always shows, on a row of its own.
        assert_eq!(activity_rows(&list, true, true, Some(5)), vec![vec![0], vec![3]]);
    }

    #[test]
    fn a_banner_says_what_happened_to_the_changes() {
        let b = JBanner { entry: 1, kind: BannerKind::AutostashRefused, label: "checkout x".into(), stash: Some("abc".into()), stash_message: Some("gitvolt".into()), target: Some("x".into()), snapshot: false, files: 0, can_drop: false, binary: false };
        assert_eq!(banner_text(&b), "Your changes from before checkout x are in stash \"gitvolt\": they conflict with x.");
        let c = JBanner { kind: BannerKind::AutostashConflicts, files: 1, ..b.clone() };
        assert!(banner_text(&c).contains("conflict in 1 file;"));
        let r = JBanner { kind: BannerKind::Recovery, snapshot: true, ..b };
        assert_eq!(banner_text(&r), "GitVolt stopped during checkout x. Your changes are safe in a snapshot.");
    }

    #[test]
    fn a_write_says_itself_and_a_long_autostash_says_so() {
        let op = |kind, label: &str| crate::ops::OpInfo { op: 1, kind, repo: None, label: label.into(), phase: None, percent: None, interactive: true, shown: false, started: Instant::now(), stash: None, step: None };
        assert_eq!(write_text(&op(OpKind::Commit, "x")), "Committing…");
        assert_eq!(write_text(&op(OpKind::Push, "push main")), "Push main…");
        let mut slow = op(OpKind::Checkout, "checkout x");
        slow.stash = Some((StashStep::Saving, "m".into(), Instant::now() - SLOW_STASH));
        assert_eq!(write_text(&slow), "Saving your changes…");
    }

    #[test]
    fn an_entry_copies_as_text() {
        let e = ActivityEntry { message: Some("boom".into()), output: vec!["line".into()], ..entry(1, false, OpOutcome::Failed) };
        let t = entry_text(&e);
        assert!(t.starts_with("fixture · fetch · user · 5 ms · failed"));
        assert!(t.contains("git fetch\nboom\nline"));
    }
}
