//! The interactive rebase (`ui/src/irebase/`): the editor in the graph's place, its menu rows and the palette's entry.
//! One session per tab: it outlives the view, since Start closes the view and a RefMoved answer brings it back with the
//! plan as it was. Writes go through `Request::InteractiveRebase`, so the core's todo rules, journal and hooks apply.

pub mod model;
pub mod view;

use crate::app::{GitVolt, Tab};
use crate::ui::arm::{self, ArmRequest, Origin};
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::menu::MenuRow;
use crate::ui::toast;
use crate::write::{Write, WriteCtx, call};
use gitvolt_core::error::{GbError, GbErrorKind};
use gpui::prelude::*;
use gpui::*;
use model::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;

/// How many plan changes Undo goes back.
const UNDO_LIMIT: usize = 200;
/// Pointer travel before a press becomes a drag: less is a click.
pub const DRAG_THRESHOLD: f32 = 4.;

pub fn short(oid: &str) -> String {
    oid.chars().take(7).collect()
}

/// Conflict prediction as the editor shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PredictionView {
    pub pending: bool,
    /// Each conflicting row's paths.
    pub by_row: HashMap<String, Vec<String>>,
    /// "Prediction is off for ranges over 300 commits", "Couldn't predict conflicts", ..
    pub note: Option<String>,
}

/// `predictionView`: the core's answer.
pub fn prediction_view(v: &Value) -> PredictionView {
    if let Some(off) = v["off"].as_str() {
        return PredictionView { pending: false, by_row: HashMap::new(), note: Some(off.to_string()) };
    }
    let mut by_row = HashMap::new();
    for r in v["rows"].as_array().map_or(&[][..], Vec::as_slice) {
        let conflicts: Vec<String> = r["conflicts"].as_array().map_or(Vec::new(), |c| c.iter().filter_map(|s| s.as_str().map(str::to_string)).collect());
        if !conflicts.is_empty() {
            by_row.insert(r["oid"].as_str().unwrap_or_default().to_string(), conflicts);
        }
    }
    PredictionView { pending: false, by_row, note: None }
}

/// A row drag: the group it carries and where it would land.
pub struct RowDrag {
    pub from: usize,
    pub group: Vec<usize>,
    pub oids: Vec<String>,
    pub home: usize,
    pub to: usize,
    pub dy: f32,
    pub heights: Vec<f32>,
    pub start: Point<Pixels>,
    pub active: bool,
    /// The pressed row and the keys held: a release without a drag is a click.
    pub oid: String,
    pub shift: bool,
    pub ctrl: bool,
}

/// A chip dragged to another row.
pub struct ChipDrag {
    pub branch: String,
    pub start: Point<Pixels>,
    pub at: Point<Pixels>,
    pub over: Option<String>,
    pub active: bool,
}

pub struct AddBranch {
    pub row: String,
    pub input: Entity<TextInput>,
    pub error: Option<String>,
}

pub struct RebaseSession {
    pub ctx: WriteCtx,
    pub opened: OpenRebase,
    pub state: EditorState,
    pub prediction: PredictionView,
    predicted_key: String,
    predict_seq: u64,
    /// RefMoved on Start: the ref that moved; the editor offers Reload.
    pub moved: Option<String>,
    /// The row whose message is open for editing.
    pub editing: Option<String>,
    pub summary: Entity<TextInput>,
    pub description: Entity<TextInput>,
    /// Start's write is running, the view closed: an entry point must not bring the editor back over it.
    pub running: bool,
    pub view_open: bool,
    /// Each branch's lane colour index in the graph when the editor opened.
    pub colors: HashMap<String, usize>,
    pub past: Vec<Plan>,
    pub future: Vec<Plan>,
    pub focus: FocusHandle,
    pub scroll: ScrollHandle,
    pub row_drag: Option<RowDrag>,
    pub chip_drag: Option<ChipDrag>,
    pub add_branch: Option<AddBranch>,
    /// A chip's full name, while the pointer's on it, and the "+N" list.
    pub expanded: Option<String>,
    pub more_open: Option<String>,
    /// The graph's selection to put back on close.
    restore: Option<String>,
}

impl RebaseSession {
    pub fn chip_color(&self, branch: &str) -> Option<Hsla> {
        self.colors.get(branch).map(|c| crate::theme::lane(*c))
    }

    /// Edits the plan. A change to it (not the selection alone) is one Undo step, and clears Redo.
    pub fn edit(&mut self, f: impl FnOnce(&mut EditorState)) {
        let before = Plan { rows: self.state.rows.clone(), chips: self.state.chips.clone() };
        f(&mut self.state);
        if before.rows != self.state.rows || before.chips != self.state.chips {
            self.past.push(before);
            if self.past.len() > UNDO_LIMIT {
                self.past.remove(0);
            }
            self.future.clear();
        }
    }

    /// Undo or Redo of the last plan change; the selection stays. False: nothing to do, or a message editor is open
    /// (its draft is never dropped: finish it first).
    pub fn step(&mut self, back: bool) -> bool {
        if self.editing.is_some() {
            return false;
        }
        let from = if back { &mut self.past } else { &mut self.future };
        let Some(to) = from.pop() else { return false };
        let now = Plan { rows: std::mem::replace(&mut self.state.rows, to.rows), chips: std::mem::replace(&mut self.state.chips, to.chips) };
        if back { self.future.push(now) } else { self.past.push(now) }
        true
    }
}

pub const FINISH_MESSAGE: &str = "Finish editing the message first";

pub fn flatten_warning(n: u32) -> String {
    format!("{n} merge {} will be flattened into a straight line", if n == 1 { "commit" } else { "commits" })
}

impl Tab {
    /// The editor, while it's on screen.
    pub fn rebase_shown(&self) -> Option<&RebaseSession> {
        self.rebase.as_ref().filter(|s| s.view_open)
    }

    pub fn rebase_shown_mut(&mut self) -> Option<&mut RebaseSession> {
        self.rebase.as_mut().filter(|s| s.view_open)
    }

    /// HEAD's branch once queued writes ran, and its upstream (`origin/main`). `None` on a detached HEAD.
    pub fn rebase_head(&self) -> Option<(String, Option<String>)> {
        let branch = self.head_branch()?;
        let upstream = self.local_branch(&branch).filter(|b| !b.gone).and_then(|b| b.upstream.map(|u| u.trim_start_matches("refs/remotes/").to_string()));
        Some((branch, upstream))
    }

    /// The operation in progress in the tab's worktree, by git's name.
    pub fn rebase_op(&self) -> Option<String> {
        self.busy_reason()
    }

    fn lane_colors(&self) -> HashMap<String, usize> {
        let mut out = HashMap::new();
        let Some(g) = self.graph.as_ref() else { return out };
        for r in &g.rows {
            for c in r.chips.iter().filter(|c| !c.tag) {
                if c.local {
                    out.entry(c.name.to_string()).or_insert(r.color as usize);
                }
                for (_, full) in &c.remotes {
                    out.entry(full.trim_start_matches("refs/remotes/").to_string()).or_insert(r.color as usize);
                }
            }
        }
        out
    }
}

impl GitVolt {
    fn rebase_tab(&mut self, repo: u32) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == repo)
    }

    /// Runs `f` on the shown session, then what follows from a change.
    pub fn with_rebase(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut RebaseSession, &mut Context<Self>)) {
        let Some(s) = self.tabs.get_mut(self.active).and_then(Tab::rebase_shown_mut) else { return };
        f(s, cx);
        self.rebase_changed(cx);
        cx.notify();
    }

    /// The plan may have changed: its prediction, once it pauses. A pending one keeps the previous marks.
    fn rebase_changed(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let repo = tab.id;
        let Some(s) = tab.rebase.as_mut() else { return };
        let key = prediction_key(&s.state);
        if key == s.predicted_key {
            return;
        }
        s.predicted_key = key;
        s.predict_seq += 1;
        s.prediction.pending = true;
        let seq = s.predict_seq;
        if cfg!(test) {
            return;
        }
        let timer = cx.background_executor().timer(Duration::from_millis(250));
        cx.spawn(async move |this, cx| {
            timer.await;
            let Ok(request) = this.update(cx, |this, _| {
                let s = this.rebase_tab(repo)?.rebase.as_ref().filter(|s| s.predict_seq == seq)?;
                let rows = to_request(&s.state).0;
                Some(call("predictRebase", json!({ "repo": s.ctx.repo, "worktree": s.ctx.worktree, "base": s.state.base.oid, "rows": rows })))
            }) else {
                return;
            };
            let Some(body) = request else { return };
            let Ok(api) = this.update(cx, |this, _| (this.api.clone(), this.rt.clone())) else { return };
            let read = api.1.spawn(async move { api.0.dispatch(crate::write::request_of(body)?).await });
            let answer = read.await.unwrap_or_else(|e| Err(GbError::other(e.to_string())));
            this.update(cx, |this, cx| {
                let Some(s) = this.rebase_tab(repo).and_then(|t| t.rebase.as_mut()).filter(|s| s.predict_seq == seq) else { return };
                s.prediction = match answer {
                    Ok(v) => prediction_view(&v),
                    Err(_) => PredictionView { pending: false, by_row: HashMap::new(), note: Some("Couldn't predict conflicts".to_string()) },
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Opens the editor: `branch`, checked out in the tab's worktree, onto `base`, rows preset as `preset` says. The open
    /// session is never silently replaced. While Start's write runs, the tab is only focused. An edited plan comes back
    /// as it is: for the same target quietly, for another with a toast to finish or cancel it first.
    pub fn open_rebase_editor(&mut self, opts: OpenRebase, cx: &mut Context<Self>) {
        // It takes the open file's place.
        let again = opts.clone();
        if self.ask_unsaved(move |v, cx| v.open_rebase_editor(again.clone(), cx), cx) {
            return;
        }
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let ctx = tab.write_target();
        let repo = tab.id;
        if let Some(open) = tab.rebase.as_mut() {
            let same = open.ctx.worktree == ctx.worktree && open.opened.same_target(&opts);
            let was = format!("{} onto {}", open.opened.branch, open.opened.base);
            if open.running {
                if !same {
                    self.show_toast(format!("Rebasing {was}: wait for it to finish"), cx);
                }
                return;
            }
            if dirty(&open.state) {
                open.view_open = true;
                if !same {
                    self.show_toast(format!("Finish or cancel the interactive rebase of {was} first"), cx);
                }
                cx.notify();
                return;
            }
        }
        if cfg!(test) {
            self.sent.borrow_mut().push(call("rebasePlan", json!({ "repo": repo, "worktree": ctx.worktree, "branch": opts.branch, "base": opts.base })));
            return;
        }
        let api = self.api.clone();
        let body = call("rebasePlan", json!({ "repo": repo, "worktree": ctx.worktree, "branch": opts.branch, "base": opts.base }));
        let read = self.rt.spawn(async move { api.dispatch(crate::write::request_of(body)?).await });
        cx.spawn(async move |this, cx| {
            let answer = read.await.unwrap_or_else(|e| Err(GbError::other(e.to_string())));
            this.update(cx, |this, cx| match answer {
                Err(e) => toast::show_with(format!("Couldn't open the interactive rebase: {}", e.message), toast::Options { error: true, ..Default::default() }, cx),
                Ok(plan) => this.show_rebase_plan(repo, ctx, opts, &plan, cx),
            })
            .ok();
        })
        .detach();
    }

    /// The plan's in: the session, and the view in the graph's place, with the sidebar away.
    pub fn show_rebase_plan(&mut self, repo: u32, ctx: WriteCtx, opts: OpenRebase, plan: &Value, cx: &mut Context<Self>) {
        if plan["rows"].as_array().is_none_or(Vec::is_empty) {
            return self.show_toast(format!("{} has no commits to rebase onto {}", opts.branch, opts.base), cx);
        }
        let state = from_plan(plan, opts.preset.as_ref());
        let summary = cx.new(|cx| TextInput::new(cx).placeholder("Summary").placeholder_color(crate::theme::wip_input_placeholder()).claim_escape().claim_arrows());
        let description = cx.new(|cx| TextInput::multiline(cx).placeholder("Description").placeholder_color(crate::theme::wip_input_placeholder()).claim_escape().claim_arrows());
        cx.subscribe(&summary, move |this, _, e: &InputEvent, cx| this.rebase_message_event(repo, true, e, cx)).detach();
        cx.subscribe(&description, move |this, _, e: &InputEvent, cx| this.rebase_message_event(repo, false, e, cx)).detach();
        let focus = cx.focus_handle();
        let Some(tab) = self.rebase_tab(repo) else { return };
        let restore = tab.selected.and_then(|i| tab.graph.as_ref()?.rows.get(i)).map(|r| r.id.to_string());
        let colors = tab.lane_colors();
        // The editor takes the graph's place, and a row it selects shows in the details panel.
        tab.diff = None;
        tab.merge = None;
        tab.history = None;
        tab.rebase = Some(RebaseSession { ctx, opened: opts, state, prediction: PredictionView::default(), predicted_key: String::new(), predict_seq: 0, moved: None, editing: None, summary, description, running: false, view_open: true, colors, past: Vec::new(), future: Vec::new(), focus: focus.clone(), scroll: ScrollHandle::new(), row_drag: None, chip_drag: None, add_branch: None, expanded: None, more_open: None, restore });
        self.focus_next_frame = Some(focus);
        self.rebase_changed(cx);
        cx.notify();
    }

    /// The view goes; the graph's selection comes back.
    fn close_rebase_view(&mut self, repo: u32, cx: &mut Context<Self>) {
        let Some(tab) = self.rebase_tab(repo) else { return };
        let restore = tab.rebase.as_mut().and_then(|s| {
            s.view_open = false;
            s.restore.take()
        });
        if let Some(sha) = restore {
            let at = tab.graph.as_ref().and_then(|g| g.by_sha.get(&sha).copied());
            let current = tab.selected;
            if let Some(i) = at.filter(|i| Some(*i) != current) {
                self.select(i, cx);
            }
        }
        let focus = self.focus.clone();
        self.focus_next_frame = Some(focus);
        cx.notify();
    }

    /// Cancel and Esc: an edited plan arms in place first.
    pub fn cancel_rebase(&mut self, origin: Origin, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let repo = tab.id;
        let Some(s) = tab.rebase_shown() else { return };
        if dirty(&s.state) {
            let mut req = ArmRequest::danger("Click again to discard your rebase plan", "Discard your rebase plan?", "Discard");
            req.body = "The changes you made in the editor are lost.".into();
            let weak = cx.weak_entity();
            arm::request(
                req,
                origin,
                move |a, _, cx| {
                    if a.ok {
                        weak.update(cx, |v, cx| v.drop_rebase(repo, cx)).ok();
                    }
                },
                window,
                cx,
            );
            return;
        }
        self.drop_rebase(repo, cx);
    }

    fn drop_rebase(&mut self, repo: u32, cx: &mut Context<Self>) {
        self.close_rebase_view(repo, cx);
        if let Some(tab) = self.rebase_tab(repo) {
            tab.rebase = None;
        }
    }

    /// Start Rebase: back to the graph at once, where the commit panel shows the progress or the stop. RefMoved brings
    /// the editor back with Reload; a declined question or a failure, with the plan as it was.
    pub fn start_rebase(&mut self, origin: Origin, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let repo = tab.id;
        let Some(s) = tab.rebase_shown_mut() else { return };
        let g = grouping(&s.state.rows);
        if !problems(&s.state, &g).is_empty() {
            return;
        }
        let (rows, chips) = to_request(&s.state);
        let (ctx, branch, base, expect) = (s.ctx.clone(), s.state.branch.clone(), s.state.base.name.clone(), s.state.expect.clone());
        s.moved = None;
        s.editing = None;
        s.running = true;
        self.close_rebase_view(repo, cx);
        let (done_branch, done_base) = (branch.clone(), base.clone());
        let worktree = ctx.worktree.clone();
        let w = Write::new(&ctx, move |asked| call("interactiveRebase", json!({ "repo": repo, "worktree": worktree, "branch": branch, "base": base, "expect": expect, "rows": rows, "chips": chips, "confirm": { "autostash": asked.autostash } })))
            .origin(origin)
            .on_success(move |this, out, cx| {
                if let Some(tab) = this.rebase_tab(repo) {
                    tab.rebase = None;
                }
                let warning = out["warning"].as_str().map(str::to_string);
                match out["status"].as_str() {
                    Some("done") | Some("upToDate") => {
                        let text = if out["status"] == "done" { format!("Rebased {done_branch} onto {done_base}") } else { format!("{done_branch} is unchanged") };
                        match warning {
                            Some(w) => toast::show_with(text, toast::Options { warning: true, detail: Some(w.into()), ..Default::default() }, cx),
                            None => toast::show(text, cx),
                        }
                    }
                    _ => this.toast_rebase_outcome(out, cx),
                }
                this.reload(repo, cx);
            })
            .handle(move |this, err, cx| {
                if err.kind != GbErrorKind::RefMoved {
                    return false;
                }
                if let Some(s) = this.rebase_tab(repo).and_then(|t| t.rebase.as_mut()) {
                    s.moved = Some(err.message.clone());
                }
                cx.notify();
                true
            })
            .on_end(move |this, ok, cx| {
                if ok {
                    return;
                }
                // Nothing was written: back to the editor, the plan as it was.
                if let Some(s) = this.rebase_tab(repo).and_then(|t| t.rebase.as_mut()) {
                    s.running = false;
                    s.view_open = true;
                    let focus = s.focus.clone();
                    this.focus_next_frame = Some(focus);
                }
                cx.notify();
            });
        self.write(w, cx);
    }

    /// Reload: the new plan, keeping what it can of the edited one. Undo can't go back past it.
    pub fn reload_rebase(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let repo = tab.id;
        let Some(s) = tab.rebase_shown() else { return };
        let body = call("rebasePlan", json!({ "repo": repo, "worktree": s.ctx.worktree, "branch": s.opened.branch, "base": s.opened.base }));
        if cfg!(test) {
            self.sent.borrow_mut().push(body);
            return;
        }
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(crate::write::request_of(body)?).await });
        cx.spawn(async move |this, cx| {
            let answer = read.await.unwrap_or_else(|e| Err(GbError::other(e.to_string())));
            this.update(cx, |this, cx| match answer {
                Err(e) => toast::show_with(format!("Couldn't reload the plan: {}", e.message), toast::Options { error: true, ..Default::default() }, cx),
                Ok(plan) => {
                    if let Some(s) = this.rebase_tab(repo).and_then(|t| t.rebase.as_mut()) {
                        let same_base = plan["baseOid"].as_str() == Some(s.state.base.oid.as_str());
                        s.state = reload(&s.state, &plan);
                        s.past.clear();
                        s.future.clear();
                        s.moved = None;
                        if !same_base {
                            s.prediction = PredictionView::default();
                        }
                    }
                    this.rebase_changed(cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Sets `oids`' action, from the dropdown or the keys: Reword opens its message at once, as does a Squash of the
    /// bottom row that made it the Reword target; a refused one says why.
    pub fn rebase_action(&mut self, oids: Vec<String>, action: RowAction, opens: Option<String>, cx: &mut Context<Self>) {
        let Some(s) = self.tabs.get_mut(self.active).and_then(Tab::rebase_shown_mut) else { return };
        let mut next = s.state.clone();
        let result = choose_action(&mut next, &oids, action);
        match result {
            Err(why) => return self.show_toast(why, cx),
            Ok(reword) => {
                s.edit(|st| *st = next);
                let open = reword.or(if action == RowAction::Reword { opens } else { None });
                if let Some(oid) = open {
                    self.start_rebase_message(oid, cx);
                }
            }
        }
        self.rebase_changed(cx);
        cx.notify();
    }

    /// A row's message opens in place, focused, the caret at the summary's end.
    pub fn start_rebase_message(&mut self, oid: String, cx: &mut Context<Self>) {
        let Some(s) = self.tabs.get_mut(self.active).and_then(Tab::rebase_shown_mut) else { return };
        let g = grouping(&s.state.rows);
        let draft = crate::wip::Draft::of_message(&target_message(&s.state, &oid, &g));
        s.editing = Some(oid);
        let (summary, description) = (s.summary.clone(), s.description.clone());
        summary.update(cx, |i, cx| i.set_text(&draft.summary, cx));
        description.update(cx, |i, cx| i.set_text(&draft.description, cx));
        self.focus_next_frame = Some(summary.read(cx).focus.clone());
        cx.notify();
    }

    /// Only Save writes the plan: closing any other way leaves the previous message.
    pub fn finish_rebase_message(&mut self, save: bool, cx: &mut Context<Self>) {
        let Some(s) = self.tabs.get_mut(self.active).and_then(Tab::rebase_shown_mut) else { return };
        let Some(oid) = s.editing.take() else { return };
        if save {
            let draft = crate::wip::Draft { summary: s.summary.read(cx).text().to_string(), description: s.description.read(cx).text().to_string() };
            let text = draft.message();
            s.edit(|st| edit_message(st, &oid, &text));
        }
        let focus = s.focus.clone();
        self.focus_next_frame = Some(focus);
        self.rebase_changed(cx);
        cx.notify();
    }

    fn rebase_message_event(&mut self, repo: u32, is_summary: bool, e: &InputEvent, cx: &mut Context<Self>) {
        let Some(s) = self.rebase_tab(repo).and_then(|t| t.rebase.as_ref()) else { return };
        let (summary, description) = (s.summary.clone(), s.description.clone());
        match e {
            InputEvent::Submit { ctrl: true, .. } => self.finish_rebase_message(true, cx),
            InputEvent::Submit { ctrl: false, .. } | InputEvent::Arrow { down: true, .. } if is_summary => {
                let focus = description.read(cx).focus.clone();
                description.update(cx, |i, cx| {
                    i.buffer.move_to(0);
                    cx.notify();
                });
                self.focus_next_frame = Some(focus);
                cx.notify();
            }
            InputEvent::Arrow { down: false, .. } if !is_summary => {
                let at = summary.read(cx).text().len();
                let focus = summary.read(cx).focus.clone();
                summary.update(cx, |i, cx| {
                    i.buffer.move_to(at);
                    cx.notify();
                });
                self.focus_next_frame = Some(focus);
                cx.notify();
            }
            InputEvent::Escape => self.finish_rebase_message(false, cx),
            _ => {}
        }
    }

    /// The selection moved: what it's about shows in the details panel.
    pub fn rebase_select(&mut self, oid: &str, shift: bool, ctrl: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let shown = tab.selected.and_then(|i| tab.graph.as_ref()?.rows.get(i)).map(|r| r.id.to_string());
        let Some(s) = tab.rebase_shown_mut() else { return };
        let before = s.state.selected.clone();
        select(&mut s.state, oid, shift, ctrl);
        let changed = s.state.selected != before;
        let focus = focus_of(&s.state, &before, shown.as_deref());
        if changed && !s.state.selected.is_empty() && let Some(oid) = focus {
            self.rebase_inspect(&oid, cx);
        }
        cx.notify();
    }

    fn rebase_inspect(&mut self, oid: &str, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let at = tab.graph.as_ref().and_then(|g| g.by_sha.get(oid).copied());
        if let Some(i) = at.filter(|i| tab.selected != Some(*i)) {
            self.select(i, cx);
        }
    }

    /// A click on the base row: no row selected, the base in the details panel.
    pub fn rebase_inspect_base(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.tabs.get_mut(self.active).and_then(Tab::rebase_shown_mut) else { return };
        s.state.selected.clear();
        s.state.anchor = None;
        let oid = s.state.base.oid.clone();
        self.rebase_inspect(&oid, cx);
        cx.notify();
    }

    /// The editor's keys, before the app's own: none of them while typing in the message editor.
    pub fn rebase_key(&mut self, k: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if crate::ui::input::is_typing(window) {
            return false;
        }
        let Some(s) = self.tabs.get(self.active).and_then(Tab::rebase_shown) else { return false };
        let state = &s.state;
        let m = &k.modifiers;
        let ctrl = m.control || m.platform;
        match k.key.as_str() {
            "up" | "down" => {
                let down = k.key == "down";
                if ctrl {
                    self.with_rebase(cx, |s, _| s.edit(|st| move_selected(st, down)));
                } else {
                    let at = state.selected.last().or(state.anchor.as_ref()).and_then(|o| state.rows.iter().position(|r| r.oid == *o));
                    let n = state.rows.len() as i64;
                    let next = match at {
                        Some(i) => (i as i64 + if down { 1 } else { -1 }).clamp(0, n - 1),
                        None => if down { 0 } else { n - 1 },
                    };
                    if let Some(oid) = state.rows.get(next as usize).map(|r| r.oid.clone()) {
                        self.rebase_select(&oid, m.shift, false, cx);
                    }
                }
                true
            }
            "enter" if !ctrl && state.selected.len() == 1 => {
                let oid = state.selected[0].clone();
                self.start_rebase_message(oid, cx);
                true
            }
            key if !ctrl && !m.alt && !state.selected.is_empty() => {
                let Some(action) = RowAction::of_key(&key.to_lowercase()) else { return false };
                let oids = state.selected.clone();
                let opens = (oids.len() == 1).then(|| oids[0].clone());
                self.rebase_action(oids, action, opens, cx);
                true
            }
            _ => false,
        }
    }

    /// The press drags: the root sends it here, for a row or a chip.
    pub fn on_rebase_drag(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let Some(s) = tab.rebase.as_mut().filter(|s| s.view_open) else { return };
        if s.row_drag.is_none() && s.chip_drag.is_none() {
            return;
        }
        if e.pressed_button != Some(MouseButton::Left) {
            s.row_drag = None;
            s.chip_drag = None;
            cx.notify();
            return;
        }
        if let Some(d) = s.row_drag.as_mut() {
            let dy = (e.position.y - d.start.y).as_f32();
            let dx = (e.position.x - d.start.x).as_f32();
            if !d.active && (dx * dx + dy * dy).sqrt() < DRAG_THRESHOLD {
                return;
            }
            d.active = true;
            d.dy = dy;
            d.to = group_target(&d.heights, &d.group, d.from, dy);
            cx.notify();
        }
        if let Some(d) = s.chip_drag.as_mut() {
            let (dx, dy) = ((e.position.x - d.start.x).as_f32(), (e.position.y - d.start.y).as_f32());
            if !d.active && (dx * dx + dy * dy).sqrt() < DRAG_THRESHOLD {
                return;
            }
            d.active = true;
            d.at = e.position;
            let oids: Vec<String> = s.state.rows.iter().map(|r| r.oid.clone()).collect();
            let ui = crate::ui::Ui::get(cx);
            let over = oids.into_iter().find(|o| ui.anchor(&format!("irow:{o}")).is_some_and(|b| b.contains(&e.position)));
            d.over = over;
            cx.notify();
        }
    }

    /// The release: a drag lands, a press that didn't drag is a click.
    pub fn end_rebase_drag(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let Some(s) = tab.rebase.as_mut().filter(|s| s.view_open) else { return };
        if let Some(d) = s.row_drag.take() {
            cx.notify();
            if d.active {
                // A scattered group dropped where it was still closes up.
                if d.to != d.home || d.group.len() > 1 {
                    let (oids, to) = (d.oids, d.to);
                    self.with_rebase(cx, |s, _| s.edit(|st| move_rows(st, &oids, to)));
                }
            } else {
                self.rebase_select(&d.oid, d.shift, d.ctrl, cx);
            }
            return;
        }
        if let Some(d) = s.chip_drag.take() {
            cx.notify();
            if d.active && let Some(over) = d.over {
                let branch = d.branch;
                self.with_rebase(cx, |s, _| s.edit(|st| move_chip(st, &branch, &over)));
            }
        }
    }

    /// The menu of a chip: Delete branch, armed in place; Remove from this plan's changes or Restore; Copy branch name.
    pub fn rebase_chip_rows(&self, branch: &str, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let mut manage = Vec::new();
        let chip = self.tabs.get(self.active).and_then(Tab::rebase_shown).and_then(|s| s.state.chips.iter().find(|c| c.branch == branch)).filter(|c| c.locked.is_none());
        if let Some(c) = chip {
            let change = chip_change(c);
            if change != ChipChange::Deleted {
                let added = change == ChipChange::Added;
                let (weak, b) = (this.clone(), branch.to_string());
                let tooltip = if added { format!("Don't create {branch}") } else { format!("Delete {branch} once the rebase completes") };
                manage.push(MenuRow::action("irebase.chip.delete", "Delete branch", "trash-2", tooltip, move |window, cx| {
                    let label = if added { format!("Click again to drop {b}") } else { format!("Click again to delete {b}") };
                    let mut req = ArmRequest::danger(label, format!("Delete {b}?"), "Delete");
                    req.body = if added { "It is not created." } else { "It is deleted once the rebase completes." }.into();
                    let (weak, b) = (weak.clone(), b.clone());
                    arm::request(
                        req,
                        Origin::current(cx),
                        move |a, _, cx| {
                            if a.ok {
                                weak.update(cx, |v, cx| v.with_rebase(cx, |s, _| s.edit(|st| delete_chip(st, &b)))).ok();
                            }
                        },
                        window,
                        cx,
                    );
                }));
            }
            let revert = |label: String, tooltip: String| {
                let (weak, b) = (this.clone(), branch.to_string());
                MenuRow::action("irebase.chip.revert", label, "rotate-ccw", tooltip, move |_, cx| {
                    let b = b.clone();
                    weak.update(cx, |v, cx| v.with_rebase(cx, |s, _| s.edit(|st| revert_chip(st, &b)))).ok();
                })
            };
            if change == ChipChange::Deleted {
                manage.push(revert("Restore".to_string(), format!("Keep {branch}: it isn't deleted")));
            } else if change == ChipChange::Moved {
                manage.push(revert("Remove from this plan's changes".to_string(), format!("Put {branch} back on {}", c.origin.as_deref().map(short).unwrap_or_default())));
            }
        }
        let name = branch.to_string();
        let copy = vec![MenuRow::action("irebase.chip.copy", "Copy branch name", "copy", format!("Copy \"{branch}\""), move |_, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(name.clone()));
            toast::show("Copied", cx);
        })];
        crate::ui::menu::grouped(vec![manage, copy])
    }

    // ---- The entry points ----

    fn rebase_busy(tab: &Tab) -> Option<String> {
        tab.rebase_op()
    }

    /// `Interactive rebase X onto Y`, in a branch's Integrate group: hidden on WIP and stash rows, when Y is HEAD's branch
    /// or at HEAD, and when HEAD is already in Y's history. `as_head`: worked out as if this branch and commit were HEAD (a
    /// chip dropped on another).
    pub fn rebase_onto_rows(&self, sha: &str, chip: &crate::model::Chip, as_head: Option<(String, String)>, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let Some(g) = tab.graph.as_ref() else { return Vec::new() };
        let (head, head_sha) = match as_head {
            Some((b, s)) => (Some(b), Some(s)),
            None => (tab.head_branch(), tab.head_sha()),
        };
        let busy = Self::rebase_busy(tab);
        let x = head.clone().or_else(|| busy.as_ref().map(|_| "HEAD".to_string()));
        let Some(x) = x else { return Vec::new() };
        if chip.name.as_ref() == x || head_sha.as_deref() == Some(sha) {
            return Vec::new();
        }
        if head_sha.as_deref().and_then(|h| crate::refs::is_ancestor(g, h, sha)) == Some(true) {
            return Vec::new();
        }
        let target = if chip.local { chip.name.to_string() } else { chip.remotes.first().map_or(chip.name.to_string(), |r| r.1.trim_start_matches("refs/remotes/").to_string()) };
        let (weak, branch, base) = (this.clone(), head.clone(), target.clone());
        vec![MenuRow::action("irebase.onto", format!("Interactive rebase {x} onto {target}"), "list-ordered", format!("Reorder, reword, squash or drop {x}'s commits, then put them on {target}"), move |_, cx| {
            if let Some(branch) = branch.clone() {
                let opts = OpenRebase { branch, base: base.clone(), preset: None };
                weak.update(cx, |v, cx| v.open_rebase_editor(opts, cx)).ok();
            }
        })
        .disabled_if(busy)]
    }

    /// "Interactive rebase after this commit" and "Drop commit" of a graph row.
    pub fn rebase_commit_rows(&self, sha: &str, parents: &[String], this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let Some(g) = tab.graph.as_ref() else { return Vec::new() };
        let head = tab.head_branch();
        let busy = Self::rebase_busy(tab);
        let Some(x) = head.clone().or_else(|| busy.as_ref().map(|_| "HEAD".to_string())) else { return Vec::new() };
        let Some(head_sha) = tab.head_sha() else { return Vec::new() };
        let in_head = crate::refs::is_ancestor(g, sha, &head_sha);
        let mut rows = Vec::new();
        if sha != head_sha && in_head != Some(false) {
            let (weak, branch, base) = (this.clone(), head.clone(), sha.to_string());
            rows.push(
                MenuRow::action("irebase.fromHere", "Interactive rebase after this commit", "list-ordered", format!("Edit {x}'s commits after {}", short(sha)), move |_, cx| {
                    if let Some(branch) = branch.clone() {
                        let opts = OpenRebase { branch, base: base.clone(), preset: None };
                        weak.update(cx, |v, cx| v.open_rebase_editor(opts, cx)).ok();
                    }
                })
                .disabled_if(busy.clone()),
            );
        }
        if in_head != Some(false) && parents.len() == 1 {
            let (weak, branch, oid, base) = (this.clone(), head, sha.to_string(), parents[0].to_string());
            let x2 = x.clone();
            rows.push(
                MenuRow::action("irebase.drop", "Drop commit", "trash-2", format!("Rebase {x} without this commit"), move |window, cx| {
                    let mut req = ArmRequest::danger(format!("Click again to drop {}", short(&oid)), format!("Drop {}?", short(&oid)), "Drop");
                    req.body = format!("{x2} is rebased without it. You can undo this.").into();
                    let origin = Origin::current(cx);
                    let (weak, branch, oid, base) = (weak.clone(), branch.clone(), oid.clone(), base.clone());
                    arm::request(
                        req,
                        origin,
                        move |a, _, cx| {
                            if a.ok
                                && let Some(branch) = branch
                            {
                                weak.update(cx, |v, cx| v.drop_commit(branch, oid, base, cx)).ok();
                            }
                        },
                        window,
                        cx,
                    );
                })
                .disabled_if(busy),
            );
        }
        rows
    }

    /// `fromOldest`: a selection a squash or rebase can start from: on HEAD's branch, no merge, the oldest not a root.
    fn oldest_selection(&self, commits: &[crate::selection::CommitRef]) -> Option<(String, String, Vec<String>)> {
        let tab = self.tabs.get(self.active)?;
        let g = tab.graph.as_ref()?;
        let head = tab.head_sha()?;
        let branch = tab.head_branch().or_else(|| Self::rebase_busy(tab).map(|_| "HEAD".to_string()))?;
        if commits.len() < 2 || commits.iter().any(|c| c.merge || crate::refs::is_ancestor(g, &c.oid, &head) == Some(false)) {
            return None;
        }
        let oldest = commits.last()?;
        let row = g.by_sha.get(&oldest.oid).and_then(|i| g.rows.get(*i))?;
        let [parent] = row.parents.as_slice() else { return None };
        Some((branch, parent.clone(), commits.iter().map(|c| c.oid.clone()).collect()))
    }

    /// The selection's "Squash N commits" and "Squash N commits interactively…".
    pub fn squash_rows(&self, commits: &[crate::selection::CommitRef], this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let Some((branch, base, oids)) = self.oldest_selection(commits) else { return Vec::new() };
        let busy = Self::rebase_busy(tab);
        let n = oids.len();
        let into = short(&oids[n - 1]);
        let row = |interactive: bool| {
            let (weak, branch, base, oids) = (this.clone(), branch.clone(), base.clone(), oids.clone());
            move |_: &mut Window, cx: &mut App| {
                let (branch, base, oids) = (branch.clone(), base.clone(), oids.clone());
                weak.update(cx, |v, cx| v.squash_selection(branch, oids, base, interactive, cx)).ok();
            }
        };
        vec![
            MenuRow::action("irebase.squash", format!("Squash {n} commits"), "combine", format!("Squash the {n} commits into {into}, their messages merged"), row(false)).disabled_if(busy.clone()),
            MenuRow::action("irebase.squashInteractive", format!("Squash {n} commits interactively…"), "list-ordered", "Open the interactive rebase with them set to squash", row(true)).disabled_if(busy),
        ]
    }

    /// The selection's "Interactive rebase from here": from its oldest commit up.
    pub fn selection_from_here_rows(&self, commits: &[crate::selection::CommitRef], this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let Some((branch, base, oids)) = self.oldest_selection(commits) else { return Vec::new() };
        let oldest = short(&oids[oids.len() - 1]);
        let weak = this.clone();
        let tooltip = format!("Edit {branch}'s commits from {oldest} up");
        vec![
            MenuRow::action("irebase.selectionFromHere", "Interactive rebase from here", "list-ordered", tooltip, move |_, cx| {
                let opts = OpenRebase { branch: branch.clone(), base: base.clone(), preset: None };
                weak.update(cx, |v, cx| v.open_rebase_editor(opts, cx)).ok();
            })
            .disabled_if(Self::rebase_busy(tab)),
        ]
    }

    /// `squashSelection`: the selected commits (newest first) fold into the oldest, their messages merged. Direct, one
    /// interactive rebase with no editor; `interactive` opens the editor with the same preset.
    pub fn squash_selection(&mut self, branch: String, oids: Vec<String>, base: String, interactive: bool, cx: &mut Context<Self>) {
        let n = oids.len();
        let Some(oldest) = oids.last().cloned() else { return };
        let mut preset = Preset { rows: HashMap::new(), gather: Some(oldest.clone()) };
        for o in &oids[..n - 1] {
            preset.rows.insert(o.clone(), RowAction::Squash);
        }
        if interactive {
            return self.open_rebase_editor(OpenRebase { branch, base, preset: Some(preset) }, cx);
        }
        self.run_preset(branch, base, preset, "squash", format!("Squashed {n} commits into {}", short(&oldest)), cx);
    }

    /// The Drop row: the plan from the commit's parent with only that commit dropped, run with no editor.
    pub fn drop_commit(&mut self, branch: String, oid: String, base: String, cx: &mut Context<Self>) {
        let mut preset = Preset::default();
        preset.rows.insert(oid.clone(), RowAction::Drop);
        self.run_preset(branch, base, preset, "drop", format!("Dropped {}", short(&oid)), cx);
    }

    /// Runs `preset` on the plan from `base` without the editor.
    pub fn run_preset(&mut self, branch: String, base: String, preset: Preset, verb: &'static str, done: String, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let ctx = tab.write_target();
        let repo = ctx.repo;
        let body = call("rebasePlan", json!({ "repo": repo, "worktree": ctx.worktree, "branch": branch, "base": base }));
        if cfg!(test) {
            self.sent.borrow_mut().push(body);
            return;
        }
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(crate::write::request_of(body)?).await });
        cx.spawn(async move |this, cx| {
            let answer = read.await.unwrap_or_else(|e| Err(GbError::other(e.to_string())));
            this.update(cx, |this, cx| {
                let plan = match answer {
                    Ok(p) => p,
                    Err(e) => return toast::show_with(format!("Couldn't {verb}: {}", e.message), toast::Options { error: true, ..Default::default() }, cx),
                };
                let state = from_plan(&plan, Some(&preset));
                let g = grouping(&state.rows);
                if let Some(why) = problems(&state, &g).into_iter().next() {
                    return toast::show_with(why, toast::Options { error: true, ..Default::default() }, cx);
                }
                let (rows, chips) = to_request(&state);
                let worktree = ctx.worktree.clone();
                let (send_branch, send_base, expect) = (branch.clone(), base.clone(), state.expect.clone());
                let done = done.clone();
                let w = Write::new(&ctx, move |asked| call("interactiveRebase", json!({ "repo": repo, "worktree": worktree, "branch": send_branch, "base": send_base, "expect": expect, "rows": rows, "chips": chips, "confirm": { "autostash": asked.autostash } }))).on_success(move |this, out, cx| {
                    if out["status"] == "done" {
                        toast::show(done.clone(), cx);
                    }
                    this.reload(repo, cx);
                });
                this.write(w, cx);
            })
            .ok();
        })
        .detach();
    }

    /// The palette's "Interactive rebase…": HEAD's branch onto its upstream; with none, a toast says where to start it.
    pub fn palette_rebase(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        if let Some(why) = Self::rebase_busy(tab) {
            return self.show_toast(why, cx);
        }
        let Some((branch, upstream)) = tab.rebase_head() else { return };
        let Some(base) = upstream else {
            return self.show_toast(format!("{branch} has no upstream: open \"Interactive rebase {branch} onto …\" from a branch's menu"), cx);
        };
        self.open_rebase_editor(OpenRebase { branch, base, preset: None }, cx);
    }

    /// Ctrl+Z and Ctrl+Shift+Z in the editor: the plan's.
    pub fn rebase_undo(&mut self, back: bool, cx: &mut Context<Self>) {
        self.with_rebase(cx, |s, _| {
            s.step(back);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::testing::Fixture;
    use std::prelude::v1::test;

    #[test]
    fn a_prediction_reads_its_conflicts_or_why_there_are_none() {
        let v = prediction_view(&json!({ "rows": [{ "oid": "a", "conflicts": ["x.txt"] }, { "oid": "b", "conflicts": [] }], "off": null }));
        assert_eq!(v.by_row.get("a").map(Vec::len), Some(1));
        assert!(!v.by_row.contains_key("b"));
        assert_eq!(prediction_view(&json!({ "rows": [], "off": "Superseded" })).note.as_deref(), Some("Superseded"));
    }

    /// A fixture: five commits on `main` above `Second`.
    async fn fixture() -> (Fixture, String) {
        let f = Fixture::new().await;
        let base = f.git(&["rev-parse", "HEAD"]);
        for i in 1..=5 {
            f.write(&format!("f{i}.txt"), &format!("{i}\n"));
            f.git(&["add", "."]);
            f.git(&["commit", "-qm", &format!("Commit {i}")]);
        }
        (f, base)
    }

    fn subjects(f: &Fixture) -> Vec<String> {
        f.git(&["log", "--format=%s", "-6"]).lines().map(str::to_string).collect()
    }

    #[tokio::test]
    async fn a_plan_goes_through_the_core_as_the_editor_says() {
        let (f, base) = fixture().await;
        let plan = f.ok(call("rebasePlan", json!({ "repo": f.repo, "worktree": f.worktree, "branch": "main", "base": base }))).await;
        let mut s = from_plan(&plan, None);
        let oid = |s: &EditorState, sub: &str| s.rows.iter().find(|r| r.summary == sub).unwrap().oid.clone();
        assert_eq!(s.rows.len(), 5);
        // Reorder, squash, reword, drop.
        let (c5, c4, c3, c2) = (oid(&s, "Commit 5"), oid(&s, "Commit 4"), oid(&s, "Commit 3"), oid(&s, "Commit 2"));
        move_rows(&mut s, std::slice::from_ref(&c5), 2);
        set_actions(&mut s, std::slice::from_ref(&c2), RowAction::Drop);
        set_actions(&mut s, std::slice::from_ref(&c3), RowAction::Reword);
        edit_message(&mut s, &c3, "Reworded three");
        set_actions(&mut s, std::slice::from_ref(&c4), RowAction::Squash);
        let (rows, chips) = to_request(&s);
        let body = call("interactiveRebase", json!({ "repo": f.repo, "worktree": f.worktree, "branch": s.branch, "base": s.base.name, "expect": s.expect, "rows": rows, "chips": chips, "confirm": { "autostash": false } }));
        let out = f.ok(body).await;
        assert_eq!(out["outcome"]["status"], "done");
        assert_eq!(subjects(&f), ["Reworded three", "Commit 5", "Commit 1", "Second", "Base"]);
        let body = f.git(&["log", "-1", "--format=%B", "HEAD"]);
        assert!(body.starts_with("Reworded three"), "{body}");
        assert!(!f.path.join("f2.txt").exists(), "the dropped commit's file went");
        assert!(f.path.join("f4.txt").exists(), "the squashed one's stayed");
    }

    #[tokio::test]
    async fn an_edit_stop_is_about_to_commit_and_continue_goes_on() {
        let (f, base) = fixture().await;
        let plan = f.ok(call("rebasePlan", json!({ "repo": f.repo, "worktree": f.worktree, "branch": "main", "base": base }))).await;
        let mut s = from_plan(&plan, None);
        let c3 = s.rows.iter().find(|r| r.summary == "Commit 3").unwrap().oid.clone();
        set_actions(&mut s, std::slice::from_ref(&c3), RowAction::Edit);
        let (rows, chips) = to_request(&s);
        let out = f.ok(call("interactiveRebase", json!({ "repo": f.repo, "worktree": f.worktree, "branch": s.branch, "base": s.base.name, "expect": s.expect, "rows": rows, "chips": chips, "confirm": { "autostash": false } }))).await;
        assert_eq!(out["outcome"]["status"], "stopped");
        let graph = f.api.graph(f.repo, Some(100), None, None, None, None).await.unwrap();
        let progress = serde_json::to_value(&graph).unwrap()["inProgress"][&f.worktree].clone();
        let op = progress.clone();
        assert_eq!(op["kind"], "rebase");
        assert!(op["editBase"].is_string(), "GitVolt's stop has HEAD on the parent: {op}");
        assert_eq!(f.git(&["diff", "--cached", "--name-only"]), "f3.txt", "its changes staged");
        let out = f.ok(call("rebaseControl", json!({ "repo": f.repo, "worktree": f.worktree, "action": "continue", "message": null }))).await;
        assert_eq!(out["outcome"]["status"], "done");
        assert_eq!(subjects(&f)[..3], ["Commit 5", "Commit 4", "Commit 3"]);
    }

    fn open_plan(window: &WindowHandle<GitVolt>, vcx: &mut VisualTestContext) {
        window
            .update(vcx, |v, _, cx| {
                let row = |o: &str| json!({ "oid": o, "summary": o, "message": format!("{o}\n"), "authorName": "x", "authorEmail": "x@example.com" });
                let plan = json!({ "branch": "main", "base": "t", "baseOid": "b", "baseSummary": "B", "baseChips": [], "merges": 0, "rows": [row("c"), row("b1"), row("a")], "chips": [], "branches": [], "expect": {} });
                let ctx = WriteCtx { repo: 1, worktree: "/w".into() };
                v.show_rebase_plan(1, ctx, OpenRebase { branch: "main".into(), base: "t".into(), preset: None }, &plan, cx);
            })
            .unwrap();
        vcx.run_until_parked();
    }

    #[gpui::test]
    fn the_keys_walk_the_rows_set_their_actions_and_esc_cancels(cx: &mut TestAppContext) {
        let (window, mut vcx, _rt) = crate::app::tests::with_rows(cx, 1);
        open_plan(&window, &mut vcx);
        let read = |window: &WindowHandle<GitVolt>, vcx: &mut VisualTestContext| window.read_with(vcx, |v, _| v.tabs[0].rebase.as_ref().map(|s| (s.state.selected.clone(), s.state.rows.iter().map(|r| r.action).collect::<Vec<_>>(), s.state.rows.iter().map(|r| r.oid.clone()).collect::<Vec<_>>()))).unwrap();
        vcx.simulate_keystrokes("down");
        assert_eq!(read(&window, &mut vcx).unwrap().0, ["c"]);
        vcx.simulate_keystrokes("down d");
        let (sel, actions, _) = read(&window, &mut vcx).unwrap();
        assert_eq!(sel, ["b1"]);
        assert_eq!(actions[1], RowAction::Drop);
        vcx.simulate_keystrokes("shift-up");
        assert_eq!(read(&window, &mut vcx).unwrap().0.len(), 2, "a range");
        vcx.simulate_keystrokes("down ctrl-up");
        assert_eq!(read(&window, &mut vcx).unwrap().2, ["c", "a", "b1"], "the selected one moved a place");
        vcx.simulate_keystrokes("ctrl-z");
        assert_eq!(read(&window, &mut vcx).unwrap().2, ["c", "b1", "a"], "the plan's undo");
        // An edited plan arms its Cancel first; a press of Esc again disarms it.
        vcx.simulate_keystrokes("escape");
        vcx.run_until_parked();
        assert!(read(&window, &mut vcx).is_some(), "still open");
        assert!(vcx.update(|_, cx| crate::ui::arm::is_armed(cx)));
        vcx.simulate_keystrokes("escape");
        vcx.run_until_parked();
        assert!(!vcx.update(|_, cx| crate::ui::arm::is_armed(cx)));
        // Reset, then Esc closes at once.
        window.update(&mut vcx, |v, _, cx| v.with_rebase(cx, |s, _| s.edit(reset))).unwrap();
        vcx.run_until_parked();
        vcx.simulate_keystrokes("escape");
        vcx.run_until_parked();
        assert!(read(&window, &mut vcx).is_none());
    }

    #[gpui::test]
    fn a_drag_past_the_threshold_moves_and_a_press_without_one_selects(cx: &mut TestAppContext) {
        let (window, mut vcx, _rt) = crate::app::tests::with_rows(cx, 1);
        open_plan(&window, &mut vcx);
        window
            .update(&mut vcx, |v, _, cx| {
                let s = v.tabs[0].rebase.as_mut().unwrap();
                s.row_drag = Some(RowDrag { from: 0, group: vec![0], oids: vec!["c".into()], home: 0, to: 0, dy: 0., heights: vec![30., 30., 30.], start: point(px(0.), px(0.)), active: false, oid: "c".into(), shift: false, ctrl: false });
                v.end_rebase_drag(cx);
                assert_eq!(v.tabs[0].rebase.as_ref().unwrap().state.selected, ["c"], "a click");
                let s = v.tabs[0].rebase.as_mut().unwrap();
                s.row_drag = Some(RowDrag { from: 0, group: vec![0], oids: vec!["c".into()], home: 0, to: 0, dy: 0., heights: vec![30., 30., 30.], start: point(px(0.), px(0.)), active: false, oid: "c".into(), shift: false, ctrl: false });
                let e = MouseMoveEvent { position: point(px(0.), px(2.)), pressed_button: Some(MouseButton::Left), modifiers: Modifiers::default() };
                v.on_rebase_drag(&e, cx);
                assert!(!v.tabs[0].rebase.as_ref().unwrap().row_drag.as_ref().unwrap().active, "under the threshold");
                let e = MouseMoveEvent { position: point(px(0.), px(40.)), ..e };
                v.on_rebase_drag(&e, cx);
                v.end_rebase_drag(cx);
                let oids: Vec<String> = v.tabs[0].rebase.as_ref().unwrap().state.rows.iter().map(|r| r.oid.clone()).collect();
                assert_eq!(oids, ["b1", "c", "a"]);
            })
            .unwrap();
    }

    #[gpui::test]
    fn undo_and_redo_walk_the_plan_and_the_selection_stays(cx: &mut TestAppContext) {
        let (window, mut vcx, _rt) = crate::app::tests::with_rows(cx, 1);
        window
            .update(&mut vcx, |v, _, cx| {
                let plan = json!({ "branch": "main", "base": "t", "baseOid": "b", "baseSummary": "B", "baseChips": [], "merges": 0, "rows": [{ "oid": "a", "summary": "A", "message": "A\n", "authorName": "x", "authorEmail": "x@example.com" }], "chips": [], "branches": [], "expect": {} });
                let ctx = WriteCtx { repo: 1, worktree: "/w".into() };
                v.show_rebase_plan(1, ctx, OpenRebase { branch: "main".into(), base: "t".into(), preset: None }, &plan, cx);
                let s = v.tabs[0].rebase.as_mut().unwrap();
                s.state.selected = vec!["a".into()];
                s.edit(|st| set_actions(st, &["a".to_string()], RowAction::Drop));
                assert_eq!(s.past.len(), 1);
                assert!(s.step(true));
                assert_eq!(s.state.rows[0].action, RowAction::Pick);
                assert_eq!(s.state.selected, ["a"]);
                assert!(s.step(false));
                assert_eq!(s.state.rows[0].action, RowAction::Drop);
                s.editing = Some("a".into());
                assert!(!s.step(true), "a message editor holds it");
            })
            .unwrap();
    }
}
