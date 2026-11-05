//! The commit box, at the bottom of the WIP panel (`ui/src/commit/*`): the draft's summary and description
//! as one text box, Amend, who the commit is made as, and the button. An operation in progress takes the top
//! of it: its status, and Continue, Skip and Abort.

use crate::app::{GitVolt, Tab, icon};
use crate::theme;
use crate::ui::arm::{self, ArmRequest, Origin};
use crate::ui::tooltip::tip;
use crate::wip::{WIP_DRAFT_COUNTER_FROM, WIP_DRAFT_WARN_FROM, commit_view_of, resolve_first};
use crate::write::{Write, call};
use gitvolt_graph::InProgress;
use gpui::prelude::*;
use gpui::*;
use serde_json::json;
use std::time::{Duration, Instant};

/// A write past this says so, with Cancel.
const STILL_WORKING: Duration = Duration::from_secs(30);
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
fn merged_name(message: &str) -> Option<String> {
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

pub fn operation_view(p: &InProgress, branch: Option<&str>) -> Operation {
    let finish_where = "Finish this in a terminal, or abort it there.".to_string();
    match p {
        InProgress::Merge { merge_head, message, conflicted } => {
            let y = merged_name(message).unwrap_or_else(|| short(merge_head));
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
            }
        }
        InProgress::Rebase { onto, head_name, step, total, stopped_at, conflicted, message, .. } => Operation {
            kind: OpName::Rebase,
            title: format!("Rebasing {} onto {} (step {step} of {total})", branch_of(head_name), short(onto)),
            detail: stopped_at.as_ref().map(|s| format!("Stopped at {}", short(s))),
            hint: if *conflicted > 0 { resolve_first(*conflicted as usize) } else { "No conflicted files left: it is paused. Continue to go on.".into() },
            conflicted: *conflicted,
            primary: Some("Continue rebase".into()),
            skip: true,
            message: without_comments(message),
            stop: format!("rebase:{}:{step}", stopped_at.clone().unwrap_or_default()),
        },
        InProgress::CherryPick { head, message, conflicted } => Operation {
            kind: OpName::CherryPick,
            title: head.as_ref().map_or("A cherry-pick is in progress".to_string(), |h| format!("Cherry-picking {}", short(h))),
            detail: None,
            hint: if *conflicted > 0 { resolve_first(*conflicted as usize) } else { "No conflicted files left: Continue to commit the cherry-pick.".into() },
            conflicted: *conflicted,
            primary: Some("Continue cherry-pick".into()),
            skip: true,
            message: without_comments(message),
            stop: format!("pick:{}", head.clone().unwrap_or_default()),
        },
        InProgress::Revert { head, message, conflicted } => Operation {
            kind: OpName::Revert,
            title: head.as_ref().map_or("A revert is in progress".to_string(), |h| format!("Reverting {}", short(h))),
            detail: None,
            hint: if *conflicted > 0 { resolve_first(*conflicted as usize) } else { "No conflicted files left: Continue to commit the revert.".into() },
            conflicted: *conflicted,
            primary: Some("Continue revert".into()),
            skip: true,
            message: without_comments(message),
            stop: format!("revert:{}", head.clone().unwrap_or_default()),
        },
        InProgress::Other { what } => Operation { kind: OpName::Other, title: format!("{} {what} is in progress", if what.starts_with(['a', 'e', 'i', 'o', 'u']) { "An" } else { "A" }), detail: None, hint: finish_where, conflicted: 0, primary: None, skip: false, message: String::new(), stop: format!("other:{what}") },
    }
}

impl Tab {
    /// The selected WIP worktree's operation in progress.
    pub fn operation(&self) -> Option<Operation> {
        let worktree = self.wip_worktree()?;
        let g = self.graph.as_ref()?;
        let p = g.in_progress.get(worktree)?;
        Some(operation_view(p, g.worktree_branch(worktree).as_deref()))
    }
}

fn button_base(id: impl Into<ElementId>) -> Stateful<Div> {
    div().id(id).flex().items_center().justify_center().gap(px(6.)).px(px(12.)).py(px(4.)).rounded(px(4.)).border_1().whitespace_nowrap()
}

/// `.commit-positive`: filled green, or a plain outline while it can't act.
fn positive(id: impl Into<ElementId>, label: impl Into<SharedString>, blocked: bool) -> Stateful<Div> {
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
fn neutral(id: impl Into<ElementId>, label: impl Into<SharedString>, blocked: bool) -> Stateful<Div> {
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
    let operation = tab.operation();
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
    let amend = operation.is_none().then(|| {
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
    let row = div().flex().flex_none().items_center().gap(px(8.)).min_h(px(20.)).children(amend).child(identity);
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
            .child(div().font_weight(FontWeight::SEMIBOLD).text_color(theme::text_normal()).when(panel_h <= 640., |d| d.truncate()).child(op.title.clone()))
            .when_some(op.detail.clone(), |d, t| d.child(div().truncate().text_color(theme::text_secondary()).child(t)))
            .child(div().text_color(if op.conflicted > 0 { theme::orange() } else { theme::text_secondary() }).child(op.hint.clone()))
    });
    let op_actions = operation.as_ref().map(|op| operation_actions(op, committing || integrating, cx));
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

fn operation_actions(op: &Operation, blocked: bool, cx: &mut Context<GitVolt>) -> Option<impl IntoElement + use<>> {
    op.primary.as_ref()?;
    let name = op.name();
    let kind = op.kind;
    let abort_tip = if kind == OpName::Merge { "Put the branch back as it was before the merge".to_string() } else { format!("Put the branch back as it was before the {name}") };
    let skip = op.skip.then(|| {
        tip(
            if kind == OpName::Rebase { "Drop the stopped commit and go on".to_string() } else { format!("Drop this commit's {name} and go on") },
            neutral("op-skip", "Skip", blocked).flex_1().when(!blocked, |d| d.on_click(cx.listener(move |this, _, _, cx| this.operation_control(kind, "skip", None, Origin::None, cx)))),
        )
    });
    let arm_label = format!("Click again to abort the {name}");
    let body = format!("The branch goes back to how it was before the {name}.");
    let abort = tip(
        abort_tip,
        danger("op-abort", format!("Abort {name}"), blocked).flex_1().when(!blocked, |d| {
            d.on_click(cx.listener(move |_, _, window, cx| {
                let mut req = ArmRequest::danger(arm_label.clone(), format!("Abort the {name}?"), format!("Abort {name}"));
                req.body = body.clone().into();
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
    Some(crate::ui::anchor("op-abort-anchor", div().flex().flex_none().gap(px(6.)).children(skip).child(abort)))
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
        let ctx = crate::write::WriteCtx { repo, worktree: worktree.clone() };
        let w = Write::new(&ctx, move |_| match kind {
            OpName::Merge => call("mergeAbort", json!({ "repo": repo, "worktree": worktree })),
            OpName::Rebase => call("rebaseControl", json!({ "repo": repo, "worktree": worktree, "action": action, "message": message })),
            _ => call("pickControl", json!({ "repo": repo, "worktree": worktree, "action": action, "message": message })),
        })
        .origin(origin)
        .on_success(move |this, out, cx| {
            this.toast_rebase_outcome(out, cx);
            this.reload(repo, cx);
        })
        .on_end(move |this, _, cx| {
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
        let o = operation_view(&p, Some("refs/heads/main"));
        assert_eq!(o.title, "Merging feature/x into main");
        assert_eq!(o.hint, "Resolve 2 conflicted files first");
        assert_eq!(o.primary.as_deref(), Some("Commit and merge"));
        assert!(!o.skip);
    }

    #[test]
    fn a_stops_comment_block_goes_and_other_hash_lines_stay() {
        assert_eq!(without_comments("Fix #1\n\n# Conflicts:\n#\tA\n"), "Fix #1\n");
        assert_eq!(without_comments("# heading\nbody"), "# heading\nbody");
    }
}
