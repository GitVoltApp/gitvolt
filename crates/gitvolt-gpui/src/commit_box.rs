//! The commit box, at the bottom of the WIP panel (`ui/src/commit/*`): the draft's summary and description
//! as one text box, Amend, who the commit is made as, and the button. An operation in progress takes the top
//! of it: its status, and Continue, Skip and Abort.

use crate::app::{GitVolt, Tab, icon};
use crate::theme;
use crate::ui::arm::{self, ArmRequest, Origin};
use crate::ui::tooltip::tip;
use crate::wip::{WIP_DRAFT_COUNTER_FROM, WIP_DRAFT_WARN_FROM, commit_view_of, resolve_first};
use crate::write::{Write, call};
use gitvolt_core::journal::PausedInfo;
use gitvolt_graph::InProgress;
use gpui::prelude::*;
use gpui::*;
use serde_json::json;
use std::time::{Duration, Instant};

/// A write past this says so, with Cancel.
pub const STILL_WORKING: Duration = Duration::from_secs(30);
const LINE_H: f32 = 18.2;

/// What the box shows for an operation in progress (`conflicts/inProgress.ts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    pub kind: OpName,
    pub title: String,
    pub detail: Option<String>,
    pub hint: String,
    pub conflicted: u32,
    /// The primary button; `None`: GitVolt has no control for it.
    pub primary: Option<String>,
    pub skip: bool,
    /// The message git keeps for it.
    pub message: String,
    pub stop: String,
    /// A rebase's stopped commit: its message fills the box when git wrote none.
    pub stopped_at: Option<String>,
    /// At an interactive rebase's Edit stop: the commit git made there.
    pub edit_stop: Option<String>,
    /// An Edit stop of a rebase started outside GitVolt: the core refuses Commit there.
    pub edit_elsewhere: bool,
    /// GitVolt's Edit stop is "about to commit": HEAD went back to the commit's parent, this one, its changes
    /// staged, its message the box's.
    pub edit_base: Option<String>,
    /// At that stop: the paths the commit adds, and whether the index differs from the commit.
    pub edit_added: Vec<String>,
    pub edit_changed: bool,
    /// GitVolt's own interactive rebase: its Abort keeps the work done at the stop that can't be had again.
    pub keeps_work: bool,
    pub caution: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpName {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Other,
}

impl Operation {
    pub fn is_merge(&self) -> bool {
        self.kind == OpName::Merge
    }

    fn name(&self) -> &'static str {
        match self.kind {
            OpName::Merge => "merge",
            OpName::Rebase => "rebase",
            OpName::CherryPick => "cherry-pick",
            OpName::Revert => "revert",
            OpName::Other => "operation",
        }
    }
}

fn short(oid: &str) -> String {
    oid.chars().take(7).collect()
}

fn branch_of(name: &str) -> &str {
    name.trim_start_matches("refs/heads/")
}

/// The branch a merge's default message names.
pub fn merged_name(message: &str) -> Option<String> {
    let rest = message.strip_prefix("Merge ")?;
    let rest = rest.strip_prefix("remote-tracking ").unwrap_or(rest).strip_prefix("branch '")?;
    Some(rest.split('\'').next()?.to_string())
}

fn without_comments(message: &str) -> String {
    let text = message.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    let at = lines.iter().enumerate().position(|(i, l)| *l == "# Conflicts:" && lines[i + 1..].iter().all(|x| x.starts_with('#') || x.trim().is_empty()));
    match at {
        None => lines.join("\n"),
        Some(at) => format!("{}\n", lines[..at].join("\n").trim_end_matches('\n')),
    }
}

/// "Amending in a terminal" at the "about to commit" stop: HEAD is the commit's parent there.
pub const AMEND_IN_TERMINAL: &str = "Amending in a terminal here would fold this commit into its parent.";
pub const COMMIT_OR_DISCARD: &str = "Commit or discard your changes first";

pub fn not_staged(file: &str) -> String {
    format!("{file} from this commit isn't staged. Stage it, or discard it, then Continue.")
}

pub fn operation_view(p: &InProgress, paused: Option<&PausedInfo>, branch: Option<&str>, subject_of: &dyn Fn(&str) -> Option<String>, name_at: &dyn Fn(&str) -> Option<String>) -> Operation {
    let at = |sha: &str, verb: &str| match subject_of(sha) {
        Some(subject) => format!("{verb} {} {subject}", short(sha)),
        None => format!("{verb} {}", short(sha)),
    };
    let finish_where = "Finish it in a terminal.".to_string();
    let plain = Operation { kind: OpName::Other, title: String::new(), detail: None, hint: String::new(), conflicted: 0, primary: None, skip: false, message: String::new(), stop: String::new(), stopped_at: None, edit_stop: None, edit_elsewhere: false, edit_base: None, edit_added: Vec::new(), edit_changed: false, keeps_work: false, caution: None };
    match p {
        InProgress::Merge { merge_head, message, conflicted } => {
            let y = paused.map(|p| p.target.clone()).or_else(|| merged_name(message)).unwrap_or_else(|| short(merge_head));
            Operation {
                kind: OpName::Merge,
                title: format!("Merging {y} into {}", branch.map_or("HEAD", branch_of)),
                detail: None,
                hint: if *conflicted > 0 { resolve_first(*conflicted as usize) } else { "No conflicted files left: commit to finish the merge.".into() },
                conflicted: *conflicted,
                primary: Some("Commit and merge".into()),
                skip: false,
                message: String::new(),
                stop: format!("merge:{merge_head}"),
                ..plain
            }
        }
        InProgress::Rebase { onto, head_name, step, total, stopped_at, conflicted, message, edit_stop, edit_base, edit_added, edit_changed, edit_conflict, message_failed, gitvolt } => {
            let onto_name = paused.map(|p| p.target.clone()).or_else(|| name_at(onto)).unwrap_or_else(|| short(onto));
            // An Edit stop: its message is HEAD's, the commit's subject.
            let edit = edit_stop.as_ref().filter(|_| *conflicted == 0).cloned();
            let elsewhere = edit.is_some() && !*gitvolt;
            let base = edit_base.clone().filter(|_| edit.is_some() && !elsewhere);
            let first_line = message.split('\n').next().unwrap_or_default().to_string();
            let subject = edit.as_ref().map_or(String::new(), |e| subject_of(e).unwrap_or_else(|| first_line.clone()));
            let editing = format!("Editing {} {subject}", edit.as_deref().map(short).unwrap_or_default()).trim_end().to_string();
            // A new message a hook refused; an Edit row whose pick conflicted: git won't stop for it again.
            let failed = message_failed.as_ref().map(|f| format!("The new message wasn't applied: {}. Type it again to retry, or Continue to keep the old one.", f.trim_end_matches('.')));
            let still = if *edit_conflict { "This commit was set to Edit: make any other changes now, then Continue." } else { "No conflicted files left: it is paused. Continue to go on." };
            let hint = if elsewhere {
                "Finish this rebase where you started it.".to_string()
            } else if base.is_some() {
                format!("{editing}: its changes are staged. Change them, commit in pieces, or just Continue.")
            } else if let Some(f) = failed {
                f
            } else if edit.is_some() {
                "Change it, then Continue.".to_string()
            } else if *conflicted > 0 {
                resolve_first(*conflicted as usize)
            } else {
                still.to_string()
            };
            let detail = if base.is_some() {
                None
            } else if let Some(e) = &edit {
                Some(format!("Stopped to edit {} {first_line}", short(e)).trim_end().to_string())
            } else {
                stopped_at.as_ref().map(|s| at(s, "Stopped at"))
            };
            Operation {
                kind: OpName::Rebase,
                title: format!("Rebasing {} onto {onto_name} (step {step} of {total})", branch_of(head_name)),
                detail,
                hint,
                conflicted: *conflicted,
                primary: Some("Continue rebase".into()),
                skip: edit.is_none(),
                message: without_comments(message),
                stop: format!("rebase:{}:{step}", edit.clone().or_else(|| stopped_at.clone()).unwrap_or_default()),
                stopped_at: stopped_at.clone(),
                edit_elsewhere: elsewhere,
                edit_added: if base.is_some() { edit_added.clone() } else { Vec::new() },
                edit_changed: base.is_some() && *edit_changed,
                keeps_work: paused.is_some_and(|p| p.irebase),
                caution: base.is_some().then(|| AMEND_IN_TERMINAL.to_string()),
                edit_stop: edit,
                edit_base: base,
                ..plain
            }
        }
        InProgress::CherryPick { head, message, conflicted } => Operation {
            kind: OpName::CherryPick,
            title: head.as_ref().map_or("A cherry-pick is in progress".to_string(), |h| at(h, "Cherry-picking")),
            detail: None,
            hint: if *conflicted > 0 { resolve_first(*conflicted as usize) } else { "No conflicted files left: Continue to commit the cherry-pick.".into() },
            conflicted: *conflicted,
            primary: Some("Continue cherry-pick".into()),
            skip: true,
            message: without_comments(message),
            stop: format!("cherry-pick:{}", head.clone().unwrap_or_default()),
            ..plain
        },
        InProgress::Revert { head, message, conflicted } => Operation {
            kind: OpName::Revert,
            title: head.as_ref().map_or("A revert is in progress".to_string(), |h| at(h, "Reverting")),
            detail: None,
            hint: if *conflicted > 0 { resolve_first(*conflicted as usize) } else { "No conflicted files left: Continue to commit the revert.".into() },
            conflicted: *conflicted,
            primary: Some("Continue revert".into()),
            skip: true,
            message: without_comments(message),
            stop: format!("revert:{}", head.clone().unwrap_or_default()),
            ..plain
        },
        InProgress::Other { what } => Operation { kind: OpName::Other, title: format!("{} {what} is in progress", if what.starts_with(['a', 'e', 'i', 'o', 'u']) { "An" } else { "A" }), detail: None, hint: finish_where, stop: what.clone(), ..plain },
    }
}

impl Tab {
    /// The selected WIP worktree's operation in progress.
    pub fn operation(&self) -> Option<Operation> {
        let worktree = self.wip_worktree()?;
        let g = self.graph.as_ref()?;
        let p = g.in_progress.get(worktree)?;
        let row = |sha: &str| g.by_sha.get(sha).and_then(|i| g.rows.get(*i));
        let subject = |sha: &str| row(sha).map(|r| r.summary.to_string());
        let name = |sha: &str| {
            let chips: Vec<_> = row(sha)?.chips.iter().filter(|c| !c.tag).collect();
            chips.iter().find(|c| c.local).map(|c| c.name.to_string()).or_else(|| chips.iter().flat_map(|c| c.remotes.first()).next().map(|r| r.1.trim_start_matches("refs/remotes/").to_string()))
        };
        Some(operation_view(p, self.paused.as_ref(), g.worktree_branch(worktree).as_deref(), &subject, &name))
    }

    /// Who the box belongs to, at an Edit stop as elsewhere.
    pub fn box_state(&self) -> BoxState {
        let operation = self.operation();
        let op = operation.clone().filter(|o| !o.is_merge());
        let head = self.wip_worktree().and_then(|w| self.wip_head(w));
        let edit = op.as_ref().and_then(|o| o.edit_stop.as_ref());
        let elsewhere = op.as_ref().is_some_and(|o| o.edit_elsewhere);
        let edit_staged = edit.is_some() && op.as_ref().is_some_and(|o| o.edit_base.is_some()) && !elsewhere;
        let head_left = edit.is_some_and(|e| head.as_deref() != Some(e.as_str()));
        let edit_moved = head_left && !elsewhere && !edit_staged;
        let made_commits = if edit_staged { head != op.as_ref().and_then(|o| o.edit_base.clone()) } else { head_left };
        let box_op = if edit_moved || edit_staged { None } else { op };
        let owns = operation.is_some() && !edit_moved && !(edit_staged && made_commits);
        BoxState { operation, box_op, edit_staged, edit_moved, head_left, made_commits, owns }
    }
}

/// `CommitBox`'s reading of the operation in progress.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxState {
    pub operation: Option<Operation>,
    /// The operation whose message and primary button the box is: not at an Edit stop that commits normally.
    pub box_op: Option<Operation>,
    pub edit_staged: bool,
    pub edit_moved: bool,
    pub head_left: bool,
    pub made_commits: bool,
    /// The operation owns the box: an amend ends.
    pub owns: bool,
}

impl BoxState {
    /// The box holds the stop's message, not the WIP draft.
    pub fn op_box(&self) -> bool {
        self.box_op.is_some() || self.edit_staged
    }
}

/// Abort's arm-in-place confirm: what it throws away, or keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeptWork {
    Commits,
    Work,
}

pub fn abort_confirm(op: &Operation, name: &str, resolved: usize, kept: Option<KeptWork>) -> (String, String) {
    let back = format!("The branch goes back to how it was before the {name}.");
    let rebase = op.kind == OpName::Rebase;
    if rebase && op.edit_stop.is_some() && kept == Some(KeptWork::Commits) {
        return ("Click again to abort: new commits from the stop are kept on a branch".into(), format!("{back} New commits from the stop are kept on a branch."));
    }
    if rebase && op.edit_stop.is_some() && kept == Some(KeptWork::Work) {
        return ("Click again to abort: new work from the stop is kept".into(), format!("{back} New work from the stop is kept."));
    }
    if rebase && op.edit_stop.is_none() && resolved > 0 && op.keeps_work {
        return ("Click again to abort: changes you made by hand are kept".into(), format!("{back} Changes you made by hand are kept in a stash."));
    }
    if rebase && op.edit_stop.is_none() && resolved > 0 {
        return ("Click again to abort: discards the conflict resolution so far".into(), format!("{back} The conflict resolution so far is discarded."));
    }
    let arm = if resolved > 0 { format!("Click again to abort: undoes {resolved} {} resolved", if resolved == 1 { "file" } else { "files" }) } else { format!("Click again to abort the {name}") };
    (arm, back)
}

/// What Abort keeps: commits made at the stop, else changes there; both, or not knowing which, is "your work".
pub fn kept_work(state: &BoxState, op: &Operation, staged: usize, unstaged: usize, conflicted: usize, tracked_left: usize) -> Option<KeptWork> {
    op.edit_stop.as_ref()?;
    if state.edit_staged {
        let left = tracked_left + conflicted > 0 || op.edit_changed;
        match (state.made_commits, left) {
            (true, false) => Some(KeptWork::Commits),
            (true, true) | (false, true) => Some(KeptWork::Work),
            (false, false) => None,
        }
    } else if state.head_left || staged > 0 || unstaged + conflicted > 0 {
        Some(KeptWork::Work)
    } else {
        None
    }
}

/// A rebase, cherry-pick or revert's primary button. An unedited or emptied box sends no message: git
/// commits with its own. A description with no summary would be lost, so it waits for one.
pub fn continue_button(op: &Operation, summary: &str, description: &str) -> crate::wip::CommitView {
    let reason = if op.primary.is_none() {
        Some(op.hint.clone())
    } else if op.conflicted > 0 {
        Some(crate::wip::resolve_first(op.conflicted as usize))
    } else if summary.trim().is_empty() && !description.trim().is_empty() {
        Some("Write a commit summary".to_string())
    } else {
        None
    };
    crate::wip::CommitView { label: op.primary.clone().unwrap_or_else(|| "Commit".to_string()), reason, stage_all: false }
}

fn button_base(id: impl Into<ElementId>) -> Stateful<Div> {
    div().id(id).flex().items_center().justify_center().gap(px(6.)).px(px(12.)).py(px(4.)).rounded(px(4.)).border_1().whitespace_nowrap()
}

/// `.commit-positive`: filled green, or a plain outline while it can't act.
pub(crate) fn positive(id: impl Into<ElementId>, label: impl Into<SharedString>, blocked: bool) -> Stateful<Div> {
    let border = theme::mix(theme::stats_added(), 0.6, theme::green());
    let hover = theme::mix(theme::stats_added(), 0.88, theme::green());
    let base = button_base(id).child(label.into());
    if blocked {
        base.text_color(theme::text_secondary()).bg(theme::panel_bg1()).border_color(theme::section_border()).font_weight(FontWeight::MEDIUM)
    } else {
        base.cursor_pointer().text_color(theme::text_on_accent()).bg(theme::stats_added()).border_color(border).font_weight(FontWeight::SEMIBOLD).hover(move |d| d.bg(hover))
    }
}

/// `.commit-neutral`.
pub(crate) fn neutral(id: impl Into<ElementId>, label: impl Into<SharedString>, blocked: bool) -> Stateful<Div> {
    let base = button_base(id).child(label.into()).border_color(theme::section_border()).bg(theme::control_bg());
    if blocked { base.text_color(theme::text_disabled()) } else { base.cursor_pointer().text_color(theme::text_normal()).hover(|d| d.bg(theme::control_hover_bg())) }
}

/// `.commit-danger`.
fn danger(id: impl Into<ElementId>, label: impl Into<SharedString>, blocked: bool) -> Stateful<Div> {
    let base = button_base(id).child(label.into());
    if blocked {
        base.text_color(theme::text_disabled()).border_color(theme::section_border())
    } else {
        base.cursor_pointer().text_color(theme::mix(theme::red(), 0.6, theme::text_selected())).border_color(theme::red()).hover(|d| d.bg(theme::mix(theme::red(), 0.2, transparent_black())))
    }
}

/// `.spin`: turns only while the write it stands for runs.
pub fn spinner(id: impl Into<ElementId>, size_px: f32, color: Hsla) -> impl IntoElement {
    icon("loader-circle", size_px, color).with_animation(id, Animation::new(Duration::from_secs(1)).repeat(), |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))))
}

/// How many rows the description shows: its text's, within what the panel's height allows.
pub fn description_rows(text: &str, chars_per_line: usize, panel_h: f32) -> usize {
    let (min, max) = if panel_h <= 480. {
        (1, 2)
    } else if panel_h <= 640. {
        (2, 4)
    } else {
        (4, 8)
    };
    let per = chars_per_line.max(1);
    let lines: usize = text.split('\n').map(|l| l.chars().count().div_ceil(per).max(1)).sum();
    lines.clamp(min, max)
}

pub fn commit_box(v: &GitVolt, tab: &Tab, panel_h: f32, window: &Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let view = commit_view_of(tab, cx);
    let state = tab.box_state();
    let operation = state.operation.clone();
    let repo = tab.id;
    let integrating = v.integrating(repo);
    let committing = tab.wip.committing;
    let blocked = view.reason.is_some() || committing || integrating;
    let worktree = tab.wip_worktree().unwrap_or_default().to_string();
    let head = tab.wip_head(&worktree);
    let n = tab.wip.summary.read(cx).text().chars().count();
    let description_text = tab.wip.description.read(cx).text().to_string();
    let rows = description_rows(&description_text, 48, panel_h);
    let focused = tab.wip.summary.read(cx).is_focused(window) || tab.wip.description.read(cx).is_focused(window);
    let fields = div()
        .flex()
        .flex_col()
        .flex_none()
        .border_1()
        .border_color(if focused { theme::focus_outline() } else { theme::wip_input_border() })
        .rounded(px(6.))
        .bg(theme::wip_input_bg())
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .pr(px(8.))
                .child(div().flex_1().min_w_0().h(px(32.)).pt(px(8.)).pb(px(4.)).pl(px(8.)).text_size(px(15.6)).line_height(px(20.3)).text_color(theme::text_selected()).child(tab.wip.summary.clone()))
                .when(n > WIP_DRAFT_COUNTER_FROM, |d| d.child(div().flex_none().text_size(px(11.)).text_color(if n > WIP_DRAFT_WARN_FROM { theme::orange() } else { theme::text_secondary() }).child(n.to_string()))),
        )
        .child(div().flex_none().h(px(rows as f32 * LINE_H + 10.)).pt(px(2.)).pb(px(8.)).px(px(8.)).line_height(px(LINE_H)).text_color(theme::text_normal()).child(tab.wip.description.clone()));
    let amend_on = tab.wip.amend.is_some();
    let can_amend = head.is_some() && !committing;
    let shows_amend = operation.is_none() || state.edit_moved || (state.edit_staged && state.made_commits);
    let amend = shows_amend.then(|| {
        tip(
            "Amend the previous commit",
            div()
                .id("commit-amend")
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .text_color(theme::text_secondary())
                .when(can_amend, |d| d.cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.toggle_amend(cx))))
                .when(!can_amend, |d| d.opacity(0.5))
                .child(crate::ui::controls::check_box(amend_on, 13.).mt_0().ml(px(4.)).mr(px(3.)))
                .child("Amend"),
        )
    });
    let identity = match &tab.wip.identity {
        None => div().flex_1().into_any_element(),
        Some(None) => div().flex().flex_1().min_w_0().items_center().justify_end().gap(px(6.)).text_size(px(12.)).text_color(theme::orange()).child(icon("triangle-alert", 13., theme::orange())).child("No git identity set: commits will fail").into_any_element(),
        Some(Some((name, email))) => div().flex_1().min_w_0().pb(px(3.)).child(tip(
            format!("Commits as {name} <{email}> (git config user.name, user.email)"),
            div()
                .flex()
                .min_w_0()
                .items_center()
                .justify_end()
                .gap(px(6.))
                .text_size(px(12.))
                .text_color(theme::text_secondary())
                .child(crate::details::avatar(&SharedString::from(name.clone()), email, 16.))
                .child(div().flex_none().text_color(theme::text_normal()).child(name.clone()))
                .child(div().min_w_0().truncate().child(email.clone())),
        )).into_any_element(),
    };
    let row = div().flex().flex_none().items_center().gap(px(8.)).min_h(px(if amend.is_some() { 20. } else { 18. })).children(amend).child(identity);
    let label = view.label.clone();
    let tooltip = view.reason.clone().unwrap_or_else(|| format!("{label} ({})", crate::commands::display_chord("Mod+Enter")));
    let button = tip(
        tooltip,
        positive("commit-button", label, blocked)
            .w_full()
            .min_h(px(30.))
            .py(px(6.))
            .when(committing, |d| d.child(spinner("commit-spin", 13., theme::text_secondary())))
            .when(!blocked, |d| d.on_click(cx.listener(|this, _, _, cx| this.submit_commit(cx)))),
    );
    let status = operation.as_ref().map(|op| {
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(2.))
            .px(px(8.))
            .py(px(6.))
            .border_l_3()
            .border_color(theme::orange())
            .rounded(px(4.))
            .bg(theme::panel_bg())
            .text_size(px(12.))
            .line_height(px(17.))
            .child(div().font_weight(FontWeight::SEMIBOLD).text_color(theme::text_normal()).when(panel_h <= 640., |d| d.truncate()).child(op.title.clone()))
            .when_some(op.detail.clone(), |d, t| d.child(div().truncate().text_color(theme::text_secondary()).child(t)))
            .child(div().text_color(if op.conflicted > 0 { theme::orange() } else { theme::text_secondary() }).child(op.hint.clone()))
            .when_some(op.caution.clone(), |d, t| d.child(div().text_color(theme::text_secondary()).child(t)))
    });
    let op_actions = operation.as_ref().map(|op| operation_actions(tab, op, &state, committing || integrating, window, cx));
    let still = tab.wip.committing_since.filter(|at| at.elapsed() >= STILL_WORKING).map(|_| {
        let op = v.ops.user_write().map(|o| o.op);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .text_size(px(12.))
            .text_color(theme::text_secondary())
            .child(spinner("commit-still", 12., theme::text_secondary()))
            .child("Still working…")
            .when_some(op, |d, op| {
                d.child(tip(
                    "Stop it: nothing is lost, and a rebase stays paused where it was",
                    div().id("commit-still-cancel").ml_auto().text_color(theme::blue()).cursor_pointer().hover(|d| d.underline()).on_click(cx.listener(move |this, _, _, cx| this.cancel_op(op, cx))).child("Cancel"),
                ))
            })
    });
    div()
        .id("commit-box")
        .flex()
        .flex_col()
        .flex_none()
        .gap(px(6.))
        .p(px(8.))
        .border_t_1()
        .border_color(theme::section_border())
        .bg(theme::panel_bg1())
        .children(status)
        .child(fields)
        .child(row)
        .child(button)
        .children(op_actions.flatten())
        .children(still)
        .into_any_element()
}

fn operation_actions(tab: &Tab, op: &Operation, state: &BoxState, blocked: bool, window: &Window, cx: &mut Context<GitVolt>) -> Option<impl IntoElement + use<>> {
    op.primary.as_ref()?;
    let name = op.name();
    let kind = op.kind;
    let (staged, unstaged, conflicted) = tab.files.as_ref().map_or((0, 0, 0), crate::details::Files::counts);
    let unstaged_files = tab.files.as_ref().map_or(Vec::new(), |f| f.section_files(crate::wip::Which::Unstaged));
    // Tracked changes left unstaged, or a file the commit adds left untracked, hold Continue up; any other
    // untracked file doesn't.
    let tracked_left = unstaged_files.iter().filter(|f| f.status != 'A' && f.status != 'U').count();
    let kept = kept_work(state, op, staged, unstaged, conflicted, tracked_left);
    // Continue under the box: an Edit stop where the box commits.
    let summary = tab.wip.summary.read(cx).text().to_string();
    let cont = if state.edit_staged {
        let leftover = unstaged_files.iter().find(|f| f.status == 'A' && op.edit_added.iter().any(|p| p == f.path.as_ref())).map(|f| f.path.to_string());
        let reason = if tab.wip.staging > 0 {
            Some(crate::wip::STAGING_BUSY.to_string())
        } else if tab.wip.amend.is_some() {
            Some("Untick Amend first".to_string())
        } else if tracked_left + conflicted > 0 {
            Some(COMMIT_OR_DISCARD.to_string())
        } else if let Some(file) = leftover {
            Some(not_staged(&file))
        } else if staged > 0 && summary.trim().is_empty() {
            Some("Write a commit summary".to_string())
        } else {
            None
        };
        let tip_text = if staged > 0 { format!("Commit {staged} {} with this message, then go on with the rebase", if staged == 1 { "file" } else { "files" }) } else { "Go on with the rebase".to_string() };
        let message = if staged > 0 { tab.wip.stop.as_ref().and_then(crate::wip::StopBox::edited_message) } else { None };
        Some((reason, tip_text, message))
    } else if state.edit_moved {
        Some(((staged + unstaged + conflicted > 0).then(|| COMMIT_OR_DISCARD.to_string()), "Go on with the rebase".to_string(), None))
    } else {
        None
    };
    let cont = cont.map(|(reason, tip_text, message)| {
        let off = blocked || reason.is_some();
        tip(
            reason.unwrap_or(tip_text),
            positive("op-continue", "Continue rebase", off).flex_1().when(!off, |d| d.on_click(cx.listener(move |this, _, _, cx| this.operation_control(OpName::Rebase, "continue", message.clone(), Origin::None, cx)))),
        )
    });
    let abort_tip = if kind == OpName::Merge { "Put the branch back as it was before the merge".to_string() } else { format!("Put the branch back as it was before the {name}") };
    let skip = op.skip.then(|| {
        tip(
            if kind == OpName::Rebase { "Drop the stopped commit and go on".to_string() } else { format!("Drop this commit's {name} and go on") },
            neutral("op-skip", "Skip", blocked).flex_1().when(!blocked, |d| d.on_click(cx.listener(move |this, _, _, cx| this.operation_control(kind, "skip", None, Origin::None, cx)))),
        )
    });
    // An armed Abort counts what it throws away: a different count disarms it.
    let resolved = if op.edit_stop.is_some() { 0 } else { staged };
    let (arm_label, body) = abort_confirm(op, name, resolved, kept);
    let armed_key = format!("{resolved}:{kept:?}");
    if arm::armed_control(cx).is_some_and(|k| k == "op-abort-anchor") && tab.wip.abort_armed_at.as_deref().is_some_and(|k| k != armed_key) {
        window.defer(cx, arm::disarm);
    }
    let repo = tab.id;
    let abort = tip(
        abort_tip,
        danger("op-abort", format!("Abort {name}"), blocked).flex_1().when(!blocked, |d| {
            d.on_click(cx.listener(move |this, _, window, cx| {
                let mut req = ArmRequest::danger(arm_label.clone(), format!("Abort the {name}?"), format!("Abort {name}"));
                req.body = body.clone().into();
                if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) {
                    tab.wip.abort_armed_at = Some(armed_key.clone());
                }
                let weak = cx.weak_entity();
                arm::request(
                    req,
                    Origin::control("op-abort-anchor"),
                    move |a, _, cx| {
                        if a.ok {
                            weak.update(cx, |v, cx| v.operation_control(kind, "abort", None, Origin::None, cx)).ok();
                        }
                    },
                    window,
                    cx,
                );
            }))
        }),
    );
    Some(crate::ui::anchor("op-abort-anchor", div().flex().flex_none().gap(px(6.)).children(cont).children(skip).child(abort)))
}

impl GitVolt {
    /// Skip, Abort or Continue of the box's operation.
    pub fn operation_control(&mut self, kind: OpName, action: &'static str, message: Option<String>, origin: Origin, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let Some(worktree) = tab.wip_worktree().map(str::to_string) else { return };
        if tab.wip.committing {
            return;
        }
        let repo = tab.id;
        tab.wip.committing = true;
        tab.wip.committing_since = Some(Instant::now());
        self.remind_still_working(cx);
        let aborted_merge = (kind == OpName::Merge).then(|| worktree.clone());
        let ended = aborted_merge.clone();
        if let Some(w) = aborted_merge.clone() {
            self.merge_aborting.insert(w);
        }
        let ctx = crate::write::WriteCtx { repo, worktree: worktree.clone() };
        let w = Write::new(&ctx, move |_| match kind {
            OpName::Merge => call("mergeAbort", json!({ "repo": repo, "worktree": worktree })),
            OpName::Rebase => call("rebaseControl", json!({ "repo": repo, "worktree": worktree, "action": action, "message": message })),
            _ => call("pickControl", json!({ "repo": repo, "worktree": worktree, "action": action, "message": message })),
        })
        .origin(origin)
        .on_success(move |this, out, cx| {
            this.toast_rebase_outcome(out, cx);
            if let Some(w) = aborted_merge.clone() {
                this.restore_draft_after_abort(repo, w, cx);
            }
            this.reload(repo, cx);
        })
        .on_end(move |this, _, cx| {
            if let Some(w) = &ended {
                this.merge_aborting.remove(w);
            }
            if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) {
                tab.wip.committing = false;
                tab.wip.committing_since = None;
            }
            cx.notify();
        });
        self.write(w, cx);
    }

    /// Asks the core to stop an operation.
    pub fn cancel_op(&mut self, op: u64, cx: &mut Context<Self>) {
        let body = call("cancelOp", json!({ "op": op }));
        self.fire(body, cx);
    }

    /// A request whose answer is the events it brings.
    pub fn fire(&mut self, body: serde_json::Value, _: &mut Context<Self>) {
        if cfg!(test) {
            self.sent.borrow_mut().push(body);
            return;
        }
        let api = self.api.clone();
        self.rt.spawn(async move {
            if let Ok(req) = crate::write::request_of(body) {
                let _ = api.dispatch(req).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn the_description_grows_with_its_text_within_the_panels_room() {
        assert_eq!(description_rows("", 48, 900.), 4);
        assert_eq!(description_rows(&"a\n".repeat(5), 48, 900.), 6);
        assert_eq!(description_rows(&"a\n".repeat(20), 48, 900.), 8);
        assert_eq!(description_rows("", 48, 600.), 2);
        assert_eq!(description_rows(&"a\n".repeat(20), 48, 400.), 2);
        assert_eq!(description_rows(&"x".repeat(100), 48, 900.), 4);
    }

    #[test]
    fn a_merge_says_what_it_merges_and_its_box_commits_it() {
        let p = InProgress::Merge { merge_head: "abcdef123".into(), message: "Merge branch 'feature/x' into main\n".into(), conflicted: 2 };
        let o = operation_view(&p, None, Some("refs/heads/main"), &|_| None, &|_| None);
        assert_eq!(o.title, "Merging feature/x into main");
        assert_eq!(o.hint, "Resolve 2 conflicted files first");
        assert_eq!(o.primary.as_deref(), Some("Commit and merge"));
        assert!(!o.skip);
    }

    #[test]
    fn a_rebase_names_what_it_stopped_at_and_what_it_goes_onto() {
        let p = InProgress::Rebase { onto: "abcdef123".into(), head_name: "refs/heads/main".into(), step: 1, total: 2, stopped_at: Some("1234567890".into()), conflicted: 1, message: "Mine\n".into(), edit_stop: None, edit_base: None, edit_added: vec![], edit_changed: false, edit_conflict: false, message_failed: None, gitvolt: false };
        let o = operation_view(&p, None, None, &|_| Some("Fix x".into()), &|_| Some("topic".into()));
        assert_eq!(o.title, "Rebasing main onto topic (step 1 of 2)");
        assert_eq!(o.detail.as_deref(), Some("Stopped at 1234567 Fix x"));
    }

    fn edit_rebase(gitvolt: bool, base: Option<&str>) -> InProgress {
        InProgress::Rebase { onto: "abcdef123".into(), head_name: "refs/heads/main".into(), step: 2, total: 5, stopped_at: Some("1234567890".into()), conflicted: 0, message: "Mine\n\nBody\n".into(), edit_stop: Some("1234567890".into()), edit_base: base.map(str::to_string), edit_added: vec!["new.txt".into()], edit_changed: false, edit_conflict: false, message_failed: None, gitvolt }
    }

    #[test]
    fn an_own_edit_stop_is_about_to_commit_and_has_no_skip() {
        let o = operation_view(&edit_rebase(true, Some("base0000")), None, None, &|_| Some("Mine".into()), &|_| None);
        assert_eq!(o.hint, "Editing 1234567 Mine: its changes are staged. Change them, commit in pieces, or just Continue.");
        assert_eq!(o.detail, None);
        assert!(!o.skip);
        assert_eq!(o.caution.as_deref(), Some(AMEND_IN_TERMINAL));
        assert_eq!(o.edit_added, ["new.txt"]);
    }

    #[test]
    fn a_terminals_edit_stop_stays_continues_and_says_so() {
        let o = operation_view(&edit_rebase(false, None), None, None, &|_| None, &|_| None);
        assert!(o.edit_elsewhere);
        assert_eq!(o.hint, "Finish this rebase where you started it.");
        assert_eq!(o.detail.as_deref(), Some("Stopped to edit 1234567 Mine"));
        let own = operation_view(&edit_rebase(true, None), None, None, &|_| None, &|_| None);
        assert_eq!(own.hint, "Change it, then Continue.");
    }

    #[test]
    fn a_refused_message_is_said_and_a_conflicted_edit_row_keeps_its_stop() {
        let InProgress::Rebase { onto, head_name, step, total, stopped_at, message, .. } = edit_rebase(true, None) else { unreachable!() };
        let p = InProgress::Rebase { onto, head_name, step, total, stopped_at, conflicted: 0, message, edit_stop: None, edit_base: None, edit_added: vec![], edit_changed: false, edit_conflict: true, message_failed: Some("hook said no.".into()), gitvolt: true };
        assert_eq!(operation_view(&p, None, None, &|_| None, &|_| None).hint, "The new message wasn't applied: hook said no. Type it again to retry, or Continue to keep the old one.");
    }

    #[test]
    fn aborts_say_what_they_keep_or_lose() {
        let plain = operation_view(&InProgress::Merge { merge_head: "a".into(), message: String::new(), conflicted: 1 }, None, None, &|_| None, &|_| None);
        assert_eq!(abort_confirm(&plain, "merge", 0, None).0, "Click again to abort the merge");
        assert_eq!(abort_confirm(&plain, "merge", 2, None).0, "Click again to abort: undoes 2 files resolved");
        let edit = operation_view(&edit_rebase(true, Some("b")), None, None, &|_| None, &|_| None);
        assert_eq!(abort_confirm(&edit, "rebase", 0, Some(KeptWork::Commits)).0, "Click again to abort: new commits from the stop are kept on a branch");
        assert_eq!(abort_confirm(&edit, "rebase", 0, Some(KeptWork::Work)).0, "Click again to abort: new work from the stop is kept");
        let mut stop = operation_view(&InProgress::Rebase { onto: "a".into(), head_name: "h".into(), step: 1, total: 2, stopped_at: None, conflicted: 1, message: String::new(), edit_stop: None, edit_base: None, edit_added: vec![], edit_changed: false, edit_conflict: false, message_failed: None, gitvolt: false }, None, None, &|_| None, &|_| None);
        assert_eq!(abort_confirm(&stop, "rebase", 1, None).0, "Click again to abort: discards the conflict resolution so far");
        stop.keeps_work = true;
        assert_eq!(abort_confirm(&stop, "rebase", 1, None).0, "Click again to abort: changes you made by hand are kept");
    }

    #[test]
    fn continue_waits_for_conflicts_and_for_a_summary_over_a_description() {
        let mut op = Operation { kind: OpName::Rebase, title: String::new(), detail: None, hint: String::new(), conflicted: 2, primary: Some("Continue rebase".into()), skip: true, message: String::new(), stop: String::new(), stopped_at: None, edit_stop: None, edit_elsewhere: false, edit_base: None, edit_added: Vec::new(), edit_changed: false, keeps_work: false, caution: None };
        assert_eq!(continue_button(&op, "s", "").reason.as_deref(), Some("Resolve 2 conflicted files first"));
        op.conflicted = 0;
        assert_eq!(continue_button(&op, "", "d").reason.as_deref(), Some("Write a commit summary"));
        let v = continue_button(&op, "", "");
        assert_eq!((v.label.as_str(), v.reason), ("Continue rebase", None), "an empty box: git's message");
    }

    #[test]
    fn a_stops_comment_block_goes_and_other_hash_lines_stay() {
        assert_eq!(without_comments("Fix #1\n\n# Conflicts:\n#\tA\n"), "Fix #1\n");
        assert_eq!(without_comments("# heading\nbody"), "# heading\nbody");
    }
}
