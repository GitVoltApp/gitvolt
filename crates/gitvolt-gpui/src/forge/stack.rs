//! The forge's side of a stack (`forge/stack/`): the chain through an MR/PR, what's left after its bottom merged, the Stack panel, and
//! creating a stack's MRs/PRs. The chain is shape only and only shown; a retarget, a write, also needs a Stack table for evidence.

use super::mrview::header::mr_button;
use super::text::{mr_noun, mr_ref};
use crate::app::{GitVolt, Tab, core_repo};
use crate::stacks::{Drop, Stack, join_names};
use crate::theme;
use crate::ui::arm::Origin;
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::menu::MenuRow;
use crate::ui::modal;
use crate::ui::toast::{self, Options};
use crate::ui::tooltip::tip;
use gitvolt_core::api::Request;
use gitvolt_core::forge::ForgeKind;
use gitvolt_core::forge::stack::{StackMode, StackSync, StackView};
use gitvolt_core::forge::types::{CreateMr, CreateOutcome, ForgeMr, MrState, SourceRef};
use gpui::prelude::*;
use gpui::*;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

/// GitLab's native stacks hold at most 20; a longer walk is a loop.
pub const MAX_CHAIN: usize = 20;

pub fn is_open(m: &ForgeMr) -> bool {
    matches!(m.state, MrState::Open | MrState::Draft)
}

pub fn state_word(s: MrState) -> &'static str {
    match s {
        MrState::Open => "Open",
        MrState::Draft => "Draft",
        MrState::Merged => "Merged",
        MrState::Merging => "Merging",
        MrState::Closed => "Closed",
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MrChain {
    /// Bottom → top.
    pub mrs: Vec<ForgeMr>,
    pub index: usize,
}

pub type MrOf = Box<dyn Fn(&str) -> Option<ForgeMr>>;
pub type StackOf = Box<dyn Fn(&str) -> Option<Stack>>;
pub type TipOf = Box<dyn Fn(&str) -> Option<String>>;

/// What the chain and after-merge read, built once per pass.
pub struct StackEnv {
    pub mr_of: MrOf,
    pub locals: BTreeSet<String>,
    pub default_branch: String,
    /// The stack base as a forge branch.
    pub base: Option<String>,
    pub stack_of: StackOf,
    pub tip_of: TipOf,
}

impl StackEnv {
    fn trunk(&self, b: &str) -> bool {
        b == self.default_branch || self.base.as_deref() == Some(b)
    }
}

/// `stackEdge`: `above` sits on `below`: it targets `below`'s source in the same project, `below` isn't a trunk's, and goes onto a trunk or a member.
pub fn stack_edge(below: &ForgeMr, above: &ForgeMr, env: &StackEnv, member: &dyn Fn(&str) -> bool) -> bool {
    if above.target_branch != below.source_branch || above.target_project != below.target_project || env.trunk(&below.source_branch) {
        return false;
    }
    if !env.trunk(&below.target_branch) && !member(&below.target_branch) {
        return false;
    }
    env.locals.contains(&below.source_branch) || (is_open(above) && member(&above.source_branch))
}

pub fn stacked_edge(below: &ForgeMr, above: &ForgeMr) -> bool {
    below.stacked || above.stacked
}

/// `mrChain`: the stack through `mr`; `None`: just `mr`.
pub fn mr_chain(mr: &ForgeMr, env: &StackEnv) -> Option<MrChain> {
    let mut seen: HashSet<String> = HashSet::from([mr.source_branch.clone()]);
    let mut down: Vec<ForgeMr> = Vec::new();
    let mut above = mr.clone();
    while !env.trunk(&above.target_branch) && !seen.contains(&above.target_branch) && down.len() + 1 < MAX_CHAIN {
        let Some(below) = (env.mr_of)(&above.target_branch) else { break };
        if !stack_edge(&below, &above, env, &|_| true) {
            break;
        }
        seen.insert(below.source_branch.clone());
        down.insert(0, below.clone());
        above = below;
    }
    // The walk ended off the base: nothing below `mr` is a stack.
    if down.first().is_some_and(|m| !env.trunk(&m.target_branch)) {
        for m in &down {
            seen.remove(&m.source_branch);
        }
        down.clear();
    }
    let mut members: HashSet<String> = down.iter().map(|m| m.source_branch.clone()).collect();
    members.insert(mr.source_branch.clone());
    let mut up: Vec<ForgeMr> = Vec::new();
    let mut top = mr.clone();
    while down.len() + 1 + up.len() < MAX_CHAIN {
        let member = |b: &str| members.contains(b);
        let next = env.locals.iter().filter(|b| !env.trunk(b) && !seen.contains(*b)).filter_map(|b| (env.mr_of)(b)).filter(|m| is_open(m) && m.target_project == mr.target_project && stack_edge(&top, m, env, &member)).max_by_key(|m| m.updated_at);
        let Some(next) = next else { break };
        seen.insert(next.source_branch.clone());
        members.insert(next.source_branch.clone());
        up.push(next.clone());
        top = next;
    }
    let index = down.len();
    let mrs: Vec<ForgeMr> = down.into_iter().chain([mr.clone()]).chain(up).collect();
    (mrs.len() >= 2).then_some(MrChain { mrs, index })
}

/// `Stack 2 of 3 (below: !1, above: !3)`.
pub fn stack_line_text(c: &MrChain, kind: ForgeKind) -> String {
    let below = c.index.checked_sub(1).and_then(|i| c.mrs.get(i)).map(|m| format!("below: {}", mr_ref(kind, m.number)));
    let above = c.mrs.get(c.index + 1).map(|m| format!("above: {}", mr_ref(kind, m.number)));
    format!("Stack {} of {} ({})", c.index + 1, c.mrs.len(), [below, above].into_iter().flatten().collect::<Vec<_>>().join(", "))
}

#[derive(Debug, Clone, PartialEq)]
pub struct AfterMerge {
    pub merged: ForgeMr,
    pub next: ForgeMr,
    pub target: String,
    pub retarget: bool,
    /// The local branches to rebase, bottom → top.
    pub branches: Vec<String>,
    /// Drop points, best first.
    pub drop_from: Vec<String>,
}

fn after_make(merged: &ForgeMr, next: &ForgeMr, branches: Vec<String>, env: &StackEnv) -> AfterMerge {
    let mut drop_from: Vec<String> = merged.head_sha.iter().cloned().collect();
    if let Some(t) = (env.tip_of)(&merged.source_branch).filter(|t| !drop_from.contains(t)) {
        drop_from.push(t);
    }
    AfterMerge { merged: merged.clone(), next: next.clone(), target: merged.target_branch.clone(), retarget: next.target_branch != merged.target_branch, branches, drop_from }
}

/// `afterMerge`: what's left to do after the stack `branch` is in had its bottom merged.
pub fn after_merge(branch: &str, env: &StackEnv) -> Option<AfterMerge> {
    let stack = (env.stack_of)(branch);
    let members: Vec<String> = stack.as_ref().map_or_else(|| vec![branch.to_string()], |s| s.branches.clone());
    let member = |b: &str| members.iter().any(|m| m == b);
    for (i, m) in members.iter().enumerate() {
        let Some(next) = (env.mr_of)(m).filter(is_open) else { continue };
        if let Some(merged) = (env.mr_of)(&next.target_branch)
            && merged.state == MrState::Merged
            && stacked_edge(&merged, &next)
            && stack_edge(&merged, &next, env, &member)
        {
            return Some(after_make(&merged, &next, members[i..].to_vec(), env));
        }
    }
    let s = stack.as_ref().filter(|s| s.branches.len() >= 2)?;
    let merged = (env.mr_of)(&s.branches[0]).filter(|m| m.state == MrState::Merged)?;
    let next = (env.mr_of)(&s.branches[1]).filter(is_open)?;
    // The forge moved `next` onto the merged one's target: test the edge as it was before.
    let before = if next.target_branch == merged.target_branch { ForgeMr { target_branch: merged.source_branch.clone(), ..next.clone() } } else { next.clone() };
    if !stack_edge(&merged, &before, env, &member) {
        return None;
    }
    let a = after_make(&merged, &next, s.branches[1..].to_vec(), env);
    (!a.retarget || stacked_edge(&merged, &next)).then_some(a)
}

pub fn first_after_merge(env: &StackEnv, pick: &dyn Fn(&AfterMerge) -> bool) -> Option<AfterMerge> {
    env.locals.iter().find_map(|b| after_merge(b, env).filter(|a| pick(a)))
}

pub fn after_merge_label(a: &AfterMerge, kind: ForgeKind) -> String {
    if a.retarget { format!("Retarget {} and rebase the stack", mr_ref(kind, a.next.number)) } else { format!("Rebase the stack without the merged {}", mr_ref(kind, a.merged.number)) }
}

/// The forge branch a stack's base ref names.
pub fn base_branch(base: &str, remote: &str) -> String {
    if let Some(b) = base.strip_prefix(&format!("refs/remotes/{remote}/")).or(base.strip_prefix("refs/heads/")) {
        return b.to_string();
    }
    base.strip_prefix("refs/remotes/").and_then(|r| r.split_once('/')).map_or(base, |p| p.1).to_string()
}

#[derive(Debug, Clone, PartialEq)]
pub enum MemberPlan {
    Create { branch: String, target: String, title: String, description: String },
    Retarget { branch: String, target: String, mr: Box<ForgeMr> },
    Ok { branch: String, target: String, mr: Box<ForgeMr> },
    Merged { branch: String, target: String, mr: Box<ForgeMr> },
}

impl MemberPlan {
    fn branch(&self) -> &str {
        match self {
            MemberPlan::Create { branch, .. } | MemberPlan::Retarget { branch, .. } | MemberPlan::Ok { branch, .. } | MemberPlan::Merged { branch, .. } => branch,
        }
    }

    fn target(&self) -> &str {
        match self {
            MemberPlan::Create { target, .. } | MemberPlan::Retarget { target, .. } | MemberPlan::Ok { target, .. } | MemberPlan::Merged { target, .. } => target,
        }
    }
}

pub fn member_plans(view: &StackView) -> Vec<MemberPlan> {
    view.members
        .iter()
        .map(|m| {
            let (branch, target) = (m.branch.clone(), m.target_branch.clone());
            match &m.mr {
                Some(mr) if mr.state == MrState::Merged => MemberPlan::Merged { branch, target, mr: Box::new(mr.clone()) },
                Some(mr) if is_open(mr) && mr.target_branch == m.target_branch => MemberPlan::Ok { branch, target, mr: Box::new(mr.clone()) },
                Some(mr) if is_open(mr) => MemberPlan::Retarget { branch, target, mr: Box::new(mr.clone()) },
                _ => {
                    let title = m.prefill.as_ref().map(|p| p.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| m.branch.clone());
                    MemberPlan::Create { branch, target, title, description: m.prefill.as_ref().map_or(String::new(), |p| p.description.clone()) }
                }
            }
        })
        .collect()
}

fn count(n: usize, kind: ForgeKind) -> String {
    format!("{n} {}{}", mr_noun(kind), if n == 1 { "" } else { "s" })
}

pub fn submit_label(plans: &[MemberPlan], kind: ForgeKind) -> Option<String> {
    let c = plans.iter().filter(|p| matches!(p, MemberPlan::Create { .. })).count();
    let r = plans.iter().filter(|p| matches!(p, MemberPlan::Retarget { .. })).count();
    match (c, r) {
        (0, 0) => None,
        (c, 0) => Some(format!("Create {}", count(c, kind))),
        (0, r) => Some(format!("Retarget {}", count(r, kind))),
        (c, r) => Some(format!("Create {} and retarget {r}", count(c, kind))),
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CreateReport {
    pub created: Vec<u64>,
    pub retargeted: Vec<u64>,
    /// The branch, whether a create failed (else a retarget), and why.
    pub failed: Option<(String, bool, String)>,
    pub rest: Vec<String>,
    /// Number 0: the whole sync failed.
    pub table: Vec<(u64, String)>,
}

/// The one toast for a Create stack: its message, detail, and whether it's an error or a warning.
pub fn create_summary(r: &CreateReport, kind: ForgeKind) -> (String, Option<String>, bool, bool) {
    let refs = |ns: &[u64]| join_names(&ns.iter().map(|n| mr_ref(kind, *n)).collect::<Vec<_>>());
    let mut parts: Vec<String> = Vec::new();
    if !r.created.is_empty() {
        parts.push(format!("Created {}", refs(&r.created)));
    }
    if !r.retargeted.is_empty() {
        parts.push(format!("{} {}", if r.created.is_empty() { "Retargeted" } else { "retargeted" }, refs(&r.retargeted)));
    }
    if let Some((b, create, why)) = &r.failed {
        parts.push(format!("{b}'s {} wasn't {}: {why}", mr_noun(kind), if *create { "created" } else { "retargeted" }));
    }
    if !r.rest.is_empty() {
        parts.push(format!("not created: {}", join_names(&r.rest)));
    }
    let detail: Vec<String> = r.table.iter().map(|(n, m)| if *n != 0 { format!("Couldn't update the stack table in {}: {m}", mr_ref(kind, *n)) } else { format!("Couldn't update the stack tables: {m}") }).collect();
    let warning = !detail.is_empty();
    (parts.join("; "), warning.then(|| detail.join(" ")), r.failed.is_some(), warning)
}

/// The tab's env; `None`: no forge target yet.
pub fn stack_env_of(t: &Tab) -> Option<StackEnv> {
    let f = &t.forge;
    let project = f.project.as_ref()?;
    let remote = f.remote.clone()?;
    f.kind?;
    let mut by_ref = f.history.clone();
    by_ref.extend(f.by_ref.iter().map(|(k, v)| (k.clone(), v.clone())));
    let upstreams = f.upstreams.clone();
    let locals: BTreeSet<String> = t.locals().into_iter().map(|l| l.name).collect();
    let tips: HashMap<String, String> = t.locals().into_iter().map(|l| (l.name, l.target)).collect();
    let mut remotes: HashMap<String, Vec<String>> = HashMap::new();
    for (g, branches) in t.remote_groups() {
        for b in branches {
            remotes.entry(b.0.clone()).or_default().push(format!("refs/remotes/{g}/{}", b.0));
        }
    }
    let local_set = locals.clone();
    let r2 = remote.clone();
    let mr_of = Box::new(move |branch: &str| -> Option<ForgeMr> {
        for full in remotes.get(branch).into_iter().flatten() {
            if let Some(m) = by_ref.get(full) {
                return Some(m.clone());
            }
        }
        if local_set.contains(branch)
            && let Some(m) = upstreams.get(&format!("refs/heads/{branch}")).and_then(|u| by_ref.get(u))
        {
            return Some(m.clone());
        }
        by_ref.get(&format!("refs/remotes/{r2}/{branch}")).cloned()
    });
    let stacks: Vec<Stack> = t.all_stacks();
    let head = t.head_branch();
    let base = stacks.first().map(|s| base_branch(&s.base, &remote)).or_else(|| t.graph_stack_base().map(|b| base_branch(&b, &remote)));
    let stack_of = Box::new(move |b: &str| crate::stacks::stack_for(&stacks, b, head.as_deref()).cloned());
    Some(StackEnv { mr_of, locals, default_branch: project.default_branch.clone().unwrap_or_else(|| "main".to_string()), base, stack_of, tip_of: Box::new(move |b| tips.get(b).cloned()) })
}

/// `afterMergeBlocked`: why it can't run now.
pub fn after_merge_blocked(t: &Tab, a: &AfterMerge) -> Option<String> {
    if let Some(op) = t.in_progress_name() {
        return Some(format!("Finish or abort the {op} first"));
    }
    let top = a.branches.last()?;
    let locals = t.locals();
    if let Some((name, w)) = a.branches.iter().filter_map(|n| locals.iter().find(|l| l.name == *n)).find_map(|b| Some((b.name.clone(), b.worktree.clone()?))) {
        return Some(format!("{name} is checked out in {}", crate::manage::worktree_shown(&t.repo_path, &w)));
    }
    (t.head_branch().as_ref() != Some(top)).then(|| format!("Check out {top} first"))
}

/// `StackLine`: the card's line. Copied from the frame, since a hover card can't read the app.
pub fn stack_line(tab: u32, mr: &ForgeMr, _this: &WeakEntity<GitVolt>, _cx: &mut App) -> Option<AnyElement> {
    let text = STACK_LINES.with_borrow(|l| l.as_ref().filter(|(t, _)| *t == tab).and_then(|(_, m)| m.get(&mr.number).cloned()))?;
    Some(div().text_size(px(11.)).text_color(theme::text_secondary()).child(text).into_any_element())
}

thread_local! {
    static STACK_LINES: RefCell<Option<(u32, HashMap<u64, String>)>> = const { RefCell::new(None) };
}

/// The lines for the MRs/PRs the sidebar and the badges can hover; made when the store changes.
pub fn refresh_stack_lines(t: &Tab) {
    let Some((env, kind)) = stack_env_of(t).zip(t.forge.kind) else {
        STACK_LINES.with_borrow_mut(|l| *l = None);
        return;
    };
    let mut lines = HashMap::new();
    let all = t.forge.list.iter().flat_map(|l| l.mrs.iter()).chain(t.forge.by_ref.values());
    for m in all.take(200) {
        if !lines.contains_key(&m.number)
            && let Some(c) = mr_chain(m, &env)
        {
            lines.insert(m.number, stack_line_text(&c, kind));
        }
    }
    STACK_LINES.with_borrow_mut(|l| *l = Some((t.id, lines)));
}

/// `StackPanel`.
pub fn stack_panel(this: &mut GitVolt, tab: u32, kind: ForgeKind, mr: &ForgeMr, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let t = this.tabs.iter().find(|t| t.id == tab)?;
    let env = stack_env_of(t)?;
    let chain = mr_chain(mr, &env);
    let after = after_merge(&mr.source_branch, &env);
    if chain.is_none() && after.is_none() {
        return None;
    }
    let mut col = div().flex().flex_col().child(div().mb(px(4.)).font_weight(FontWeight::BOLD).child("Stack"));
    if let Some(c) = &chain {
        let mut list = div().flex().flex_col().gap(px(2.));
        for (i, m) in c.mrs.iter().enumerate() {
            let text = format!("{} {}", mr_ref(kind, m.number), crate::md::emoji::emojify(&m.title));
            let number = m.number;
            let name: AnyElement = if i == c.index {
                div().min_w_0().truncate().font_weight(FontWeight::SEMIBOLD).child(text).into_any_element()
            } else {
                crate::ui::anchor(format!("mr-stack:{number}"), div().id(("mr-stack", number as usize)).min_w_0().truncate().text_color(theme::text_normal()).cursor_pointer().hover(|d| d.underline()).child(text).on_click(cx.listener(move |v, _, _, cx| v.forge_open_mr(tab, number, cx)))).into_any_element()
            };
            list = list.child(div().flex().items_baseline().gap(px(8.)).child(name).child(div().ml_auto().flex_none().text_color(theme::text_secondary()).child(state_word(m.state))));
        }
        col = col.child(list);
    }
    if let Some(a) = after {
        let blocked = after_merge_blocked(t, &a);
        let label = after_merge_label(&a, kind);
        let on = blocked.is_none();
        let btn = crate::ui::anchor(
            "mr-stack-after",
            mr_button("mr-stack-after", on).child(label).when(on, |d| {
                d.on_click(cx.listener(move |v, _, window, cx| {
                    v.stack_retarget_and_rebase(tab, a.clone(), Origin::control("mr-stack-after"), window, cx);
                }))
            }),
        );
        col = col.child(div().flex().flex_col().mt(px(8.)).child(div().flex().child(btn)).children(blocked.map(|b| div().mt(px(4.)).text_color(theme::text_secondary()).child(b))));
    }
    Some(col.into_any_element())
}

impl GitVolt {
    /// A stack's prompt after its bottom merged, once per merged MR/PR a session.
    pub fn stack_after_merge(&mut self, tab: u32, _number: u64, cx: &mut Context<Self>) {
        self.stack_prompt(tab, cx);
    }

    /// `promptFor`: from the store's changes. Its action recomputes first: the toast may be old.
    pub fn stack_prompt(&mut self, tab: u32, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some((env, kind)) = stack_env_of(t).zip(t.forge.kind) else { return };
        let asked = t.forge.stack_prompted.clone();
        let Some(a) = first_after_merge(&env, &|x| !asked.contains(&x.merged.number)) else { return };
        let number = a.merged.number;
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
            t.forge.stack_prompted.insert(number);
        }
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let merged = mr_ref(kind, number);
        let Some(top) = a.branches.last().cloned() else { return };
        if t.head_branch().as_deref() != Some(top.as_str()) {
            let what = if a.retarget { format!("retarget {} and rebase the stack", mr_ref(kind, a.next.number)) } else { "rebase the stack".to_string() };
            toast::show_with(format!("{merged} was merged: check out {top} to {what}"), Options { sticky: true, ..Default::default() }, cx);
        } else {
            let label = if a.retarget { format!("Retarget the next {} and rebase the stack", mr_noun(kind)) } else { format!("Rebase the stack without {merged}") };
            let w = cx.weak_entity();
            let action = toast::ToastAction {
                label: label.into(),
                run: Rc::new(move |window: &mut Window, cx: &mut App| {
                    w.update(cx, |v, cx| v.stack_rerun(tab, number, window, cx)).ok();
                }),
            };
            toast::show_with(format!("{merged} was merged."), Options { sticky: true, actions: vec![action], ..Default::default() }, cx);
        }
    }

    fn stack_rerun(&mut self, tab: u32, number: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some((env, kind)) = stack_env_of(t).zip(t.forge.kind) else { return toast::show_error("Couldn't retarget: the repository isn't ready", cx) };
        let Some(a) = first_after_merge(&env, &|x| x.merged.number == number) else { return toast::show(format!("{} no longer has a stack to retarget and rebase", mr_ref(kind, number)), cx) };
        if let Some(why) = after_merge_blocked(t, &a) {
            return toast::show_error(why, cx);
        }
        self.stack_retarget_and_rebase(tab, a, Origin::None, window, cx);
    }

    /// `retargetAndRebase`: retarget first; fetch; rebase without the merged commits; push; the Stack tables. Each failure stops what's after it.
    pub fn stack_retarget_and_rebase(&mut self, tab: u32, a: AfterMerge, origin: Origin, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        // One at a time per merged MR/PR: the button stays enabled for its armed second click.
        if !t.forge.stack_running.insert(a.merged.number) {
            return;
        }
        let Some(kind) = t.forge.kind else { return };
        let _ = window;
        if a.retarget {
            let next = mr_ref(kind, a.next.number);
            let (number, target) = (a.next.number, a.target.clone());
            self.forge_ask::<ForgeMr>(Request::ForgeRetarget { repo: core_repo(tab), number, target: target.clone() }, cx, move |v, answer, cx| match answer {
                Ok(_) => {
                    v.forge_written(tab, cx);
                    v.stack_fetch_step(tab, a.clone(), kind, origin.clone(), cx);
                }
                Err(e) => {
                    toast::show_error(format!("Couldn't retarget {next} to {target}: {e}"), cx);
                    v.stack_done(tab, a.merged.number);
                }
            });
        } else {
            self.stack_fetch_step(tab, a, kind, origin, cx);
        }
    }

    fn stack_done(&mut self, tab: u32, number: u64) {
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
            t.forge.stack_running.remove(&number);
        }
    }

    fn stack_fetch_step(&mut self, tab: u32, a: AfterMerge, kind: ForgeKind, origin: Origin, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some(remote) = t.forge.remote.clone() else { return };
        let on_remote = t.remote_groups().into_iter().any(|(g, b)| g == remote && b.iter().any(|x| x.0 == a.target));
        let base = if on_remote { format!("refs/remotes/{remote}/{}", a.target) } else { format!("refs/heads/{}", a.target) };
        if !on_remote {
            return self.stack_rebase_step(tab, a, kind, base, origin, cx);
        }
        let ctx = t.write_target();
        let repo = core_repo(tab);
        let did = a.retarget.then(|| format!("Retargeted {} to {}", mr_ref(kind, a.next.number), a.target));
        let not_rebased = did.as_ref().map_or("The stack wasn't rebased".to_string(), |d| format!("{d}; the stack wasn't rebased"));
        let (r2, r3, b2) = (remote.clone(), remote, base);
        let (a2, a3, o2, nr) = (a.clone(), a, origin, not_rebased);
        let w = crate::write::Write::new(&ctx, move |_| crate::write::call("fetch", serde_json::json!({ "repo": repo, "background": false, "remote": r2 })))
            .on_success(move |v, out, cx| {
                if out["status"].as_str() == Some("done") {
                    let (a, b, o) = (a2.clone(), b2.clone(), o2.clone());
                    v.reload_then(tab, cx, move |v, cx| v.stack_rebase_step(tab, a, kind, b, o, cx));
                } else {
                    let reason = if out["reason"].as_str() == Some("busy") { "another fetch was running" } else { "it needs your credentials" };
                    toast::show_error(format!("{nr}: fetching {r3} was skipped ({reason})"), cx);
                    v.stack_done(tab, a2.merged.number);
                }
            })
            .on_end(move |v, ok, _| {
                if !ok {
                    v.stack_done(tab, a3.merged.number);
                }
            });
        self.write(w, cx);
    }

    fn stack_rebase_step(&mut self, tab: u32, a: AfterMerge, kind: ForgeKind, base: String, origin: Origin, cx: &mut Context<Self>) {
        let (api, repo) = (self.api.clone(), core_repo(tab));
        let (head, b2) = (a.merged.head_sha.clone(), base.clone());
        let job = self.rt.spawn(async move {
            let head = head?;
            let m = api.dispatch(Request::MergeBase { repo, a: head.clone(), b: b2 }).await.ok()?;
            Some(m.as_str() == Some(head.as_str()))
        });
        cx.spawn(async move |this, cx| {
            // A merge-commit or fast-forward merge leaves the merged head in the base: nothing to drop. A squash doesn't.
            let head_in_base = job.await.ok().flatten().unwrap_or(false);
            let Some(handle) = this.update(cx, |v, _| v.window).ok().flatten() else { return };
            handle
                .update(cx, |_, window, cx| {
                    this.update(cx, |v, cx| {
                        let stack = Stack { branches: a.branches.clone(), base: base.clone(), left_behind: Vec::new(), partial: false };
                        let drop = (!head_in_base && !a.drop_from.is_empty()).then(|| Drop { from: a.drop_from.clone(), branch: a.merged.source_branch.clone() });
                        let (a2, s2) = (a.clone(), stack.clone());
                        let then: crate::stacks::AfterRebase = Rc::new(move |v, status, cx| v.stack_after_rebase(tab, a2.clone(), kind, s2.clone(), status, head_in_base, cx));
                        v.rebase_stack_then(stack, origin, None, drop, Some(then), window, cx);
                    })
                    .ok();
                })
                .ok();
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn stack_after_rebase(&mut self, tab: u32, a: AfterMerge, kind: ForgeKind, stack: Stack, status: &str, head_in_base: bool, cx: &mut Context<Self>) {
        let did = a.retarget.then(|| format!("Retargeted {} to {}", mr_ref(kind, a.next.number), a.target));
        let top = a.branches.last().cloned().unwrap_or_default();
        match status {
            "done" | "upToDate" => {
                let up_to_date = status == "upToDate";
                let a2 = a.clone();
                self.reload_then(tab, cx, move |v, cx| {
                    let a3 = a2.clone();
                    let d2 = did.clone();
                    let after: crate::stacks::AfterPush = Rc::new(move |v, failed, cx| {
                        if failed {
                            return v.stack_done(tab, a3.merged.number);
                        }
                        v.stack_sync_step(tab, a3.clone(), kind, d2.clone(), up_to_date, cx);
                    });
                    v.push_stack_then(stack.clone(), after, cx);
                });
            }
            "cancelled" | "editor" => {
                self.stack_done(tab, a.merged.number);
                if let Some(d) = did {
                    let way = if head_in_base { "Rebase stack".to_string() } else { after_merge_label(&AfterMerge { retarget: false, ..a }, kind) };
                    toast::show(format!("{d}; the stack wasn't rebased. Use “{way}” on {top}"), cx);
                }
            }
            "failed" => {
                self.stack_done(tab, a.merged.number);
                if let Some(d) = did {
                    toast::show_error(format!("{d}; the stack wasn't rebased"), cx);
                }
            }
            // It stopped on a conflict: the commit panel takes over.
            _ => self.stack_done(tab, a.merged.number),
        }
    }

    fn stack_sync_step(&mut self, tab: u32, a: AfterMerge, kind: ForgeKind, did: Option<String>, up_to_date: bool, cx: &mut Context<Self>) {
        let mut branches = vec![a.merged.source_branch.clone()];
        branches.extend(a.branches.iter().cloned());
        let target = a.target.clone();
        let number = a.merged.number;
        self.forge_ask::<StackSync>(Request::ForgeSyncStack { repo: core_repo(tab), branches, base: target.clone() }, cx, move |v, answer, cx| {
            v.stack_done(tab, number);
            let line = match answer {
                Ok(s) if s.failed.is_empty() => None,
                Ok(s) => Some(s.failed.iter().map(|f| format!("Couldn't update the stack table in {}: {}", mr_ref(kind, f.number), f.message)).collect::<Vec<_>>().join(" ")),
                Err(e) => Some(format!("Couldn't update the stack tables: {e}")),
            };
            let opts = Options { warning: line.is_some(), detail: line.clone().map(Into::into), ..Default::default() };
            if up_to_date && did.is_none() && line.is_none() {
                return;
            }
            let message = match (&did, up_to_date) {
                (Some(d), true) => format!("{d}; the stack was already on {target}"),
                (None, true) => format!("The stack is already on {target}"),
                (Some(d), false) => format!("{d}, then rebased and pushed the stack"),
                (None, false) => format!("Rebased the stack onto {target} and pushed it"),
            };
            toast::show_with(message, opts, cx);
            v.forge_written(tab, cx);
        });
    }

    /// The Integrate row while there's something to do, greyed as it's blocked.
    pub fn stack_after_merge_rows(&self, name: &str, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(t) = self.tabs.get(self.active) else { return Vec::new() };
        let Some((env, kind)) = stack_env_of(t).zip(t.forge.kind) else { return Vec::new() };
        let Some(a) = after_merge(name, &env) else { return Vec::new() };
        let merged = mr_ref(kind, a.merged.number);
        let point = if a.retarget { format!("point {} at {}, ", mr_ref(kind, a.next.number), a.target) } else { String::new() };
        let tooltip = format!("{merged} was merged: {point}rebase {} onto {} without its commits, and push", join_names(&a.branches), a.target);
        let (w, tab, blocked, label) = (this.clone(), t.id, after_merge_blocked(t, &a), after_merge_label(&a, kind));
        vec![MenuRow::action("stack.forge.afterMerge", label, "layers", tooltip, move |window, cx| {
            let (a, origin) = (a.clone(), Origin::MenuRow);
            w.update(cx, |v, cx| v.stack_retarget_and_rebase(tab, a, origin, window, cx)).ok();
        }).disabled_if(blocked)]
    }

    /// `Create stack MRs…`: hidden without a project, or once every member has an open one on the right target.
    pub fn stack_create_rows(&self, name: &str, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(t) = self.tabs.get(self.active) else { return Vec::new() };
        let Some(stack) = t.stack_of(name) else { return Vec::new() };
        let (Some(env), Some(kind), Some(remote)) = (stack_env_of(t), t.forge.kind, t.forge.remote.clone()) else { return Vec::new() };
        let base = base_branch(&stack.base, &remote);
        let want = |i: usize| if i == 0 { base.clone() } else { stack.branches[i - 1].clone() };
        if stack.branches.iter().enumerate().all(|(i, b)| (env.mr_of)(b).is_some_and(|m| is_open(&m) && m.target_branch == want(i))) {
            return Vec::new();
        }
        let noun = mr_noun(kind);
        let unpushed = stack.branches.iter().any(|n| t.local_branch(n).is_none_or(|b| !b.push_target.as_deref().is_some_and(|p| p.starts_with(&format!("{remote}/"))) || !crate::sync::nothing_to_push(&b)));
        let why = t.in_progress_name().map(|op| format!("Finish or abort the {op} first")).or_else(|| stack.partial.then(|| "Load more history first".to_string())).or_else(|| unpushed.then(|| "Push the stack first".to_string()));
        let w = this.clone();
        vec![MenuRow::action("stack.forge.create", format!("Create stack {noun}s…"), "git-pull-request-create", format!("One {noun} per branch, each targeting the branch below it"), move |window, cx| {
            let stack = stack.clone();
            w.update(cx, |v, cx| v.open_create_stack(stack, window, cx)).ok();
        }).disabled_if(why)]
    }

    /// `CreateStackDialog`: a modal, as the web's; Enter in a title never creates, only the button or Ctrl+Enter.
    pub fn open_create_stack(&mut self, stack: Stack, window: &mut Window, cx: &mut Context<Self>) {
        const ID: &str = "create-stack";
        let Some(t) = self.tabs.get(self.active) else { return };
        let tab = t.id;
        let Some(kind) = t.forge.kind else { return };
        let Some(remote) = t.forge.remote.clone() else { return toast::show_error("This repository has no forge project: add an account in Settings › Accounts", cx) };
        let noun = mr_noun(kind);
        let state: Rc<RefCell<CreateStackUi>> = Rc::new(RefCell::new(CreateStackUi::default()));
        let (api, rt) = (self.api.clone(), self.rt.clone());
        let req = Request::ForgeStack { repo: core_repo(tab), branches: stack.branches.clone(), base: base_branch(&stack.base, &remote), base_ref: stack.base.clone() };
        let (s2, weak) = (state.clone(), cx.weak_entity());
        let handle = window.window_handle();
        cx.spawn(async move |_, cx| {
            let answer = super::ask::<StackView>(api, rt, req).await;
            handle
                .update(cx, |_, window, cx| {
                    match answer {
                        Err(e) => s2.borrow_mut().error = Some(e),
                        Ok(view) => {
                            let plans = member_plans(&view);
                            let mut titles = HashMap::new();
                            for p in &plans {
                                if let MemberPlan::Create { branch, title, .. } = p {
                                    let input = cx.new(TextInput::new);
                                    input.update(cx, |i, cx| i.set_text(title, cx));
                                    titles.insert(branch.clone(), input);
                                }
                            }
                            let first = plans.iter().find_map(|p| titles.get(p.branch()).map(|i: &Entity<TextInput>| i.read(cx).focus.clone()));
                            let mut keep = Vec::new();
                            for input in titles.values() {
                                let (s3, w2) = (s2.clone(), weak.clone());
                                keep.push(cx.subscribe(input, move |_, e: &InputEvent, cx| match e {
                                    InputEvent::Submit { ctrl: true, .. } => {
                                        let (s, w) = (s3.clone(), w2.clone());
                                        cx.defer(move |cx| {
                                            create_stack_submit(&s, &w, cx);
                                        });
                                    }
                                    InputEvent::Changed => cx.refresh_windows(),
                                    _ => {}
                                }));
                            }
                            let mut s = s2.borrow_mut();
                            s.keep = keep;
                            s.titles = titles;
                            s.plans = plans;
                            s.view = Some(view);
                            drop(s);
                            if let Some(f) = first {
                                f.focus(window, cx);
                            }
                        }
                    }
                    cx.refresh_windows();
                })
                .ok();
        })
        .detach();
        let (s3, w3, stop_a, stop_b, stop_c) = (state.clone(), cx.weak_entity(), cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true));
        state.borrow_mut().tab = tab;
        state.borrow_mut().kind = Some(kind);
        state.borrow_mut().remote = remote;
        let m = modal::Modal::new(ID, move |_window, cx| {
            let s = s3.borrow();
            let plans = &s.plans;
            let title_of = |b: &str, cx: &App| s.titles.get(b).map(|i| i.read(cx).text().to_string()).unwrap_or_default();
            let merged = plans.iter().any(|p| matches!(p, MemberPlan::Merged { .. }));
            let empty = plans.iter().any(|p| matches!(p, MemberPlan::Create { branch, .. } if title_of(branch, cx).trim().is_empty()));
            let label = submit_label(plans, kind);
            let can = label.is_some() && !merged && !empty;
            let intro: String = match &s.view {
                Some(v) if v.mode == StackMode::Native => format!("Each {noun} targets the branch below it. GitLab shows them as a stack."),
                Some(_) => format!("Each {noun} targets the branch below it. GitVolt keeps a Stack table in every description up to date."),
                None if s.error.is_none() => format!("Loading the stack's {noun}s…"),
                None => String::new(),
            };
            let mut list = div().flex().flex_col().gap(px(6.)).my(px(8.));
            for (i, p) in plans.iter().enumerate() {
                let trailing: AnyElement = match p {
                    MemberPlan::Create { branch, .. } => s.titles.get(branch).map_or(div().into_any_element(), |input| crate::ui::anchor(format!("stack-title:{branch}"), div().id(("stack-title", i)).px(px(8.)).py(px(4.)).rounded(px(4.)).border_1().border_color(theme::wip_input_border()).bg(theme::wip_input_bg()).child(input.clone())).into_any_element()),
                    MemberPlan::Ok { mr, .. } => div().text_color(theme::text_secondary()).child(format!("{} is open", mr_ref(kind, mr.number))).into_any_element(),
                    MemberPlan::Retarget { mr, .. } => div().text_color(theme::text_secondary()).child(format!("{}: retarget from {}", mr_ref(kind, mr.number), mr.target_branch)).into_any_element(),
                    MemberPlan::Merged { mr, .. } => div().text_color(theme::red()).child(format!("{} is merged: rebase the stack first", mr_ref(kind, mr.number))).into_any_element(),
                };
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().flex_none().w(px(20.)).text_right().text_color(theme::text_disabled()).child((i + 1).to_string()))
                        .child(tip(format!("{} → {}", p.branch(), p.target()), div().flex_1().min_w_0().truncate().font_family(crate::app::MONO_FONT).child(format!("{} → {}", p.branch(), p.target()))))
                        .child(div().flex().flex_col().flex_1().min_w_0().child(trailing)),
                );
            }
            let has_create = plans.iter().any(|p| matches!(p, MemberPlan::Create { .. }));
            let (d3, draft) = (s3.clone(), s.draft);
            let (s4, w4) = (s3.clone(), w3.clone());
            div()
                .flex()
                .flex_col()
                .min_h(px(200.))
                .child(modal::title(format!("Create stack {noun}s")))
                .child(div().mt(px(8.)).text_color(theme::text_secondary()).child(intro))
                .child(div().min_h(px(18.)).text_color(theme::red()).children(s.error.clone()))
                .child(list)
                .when(has_create, |d| {
                    d.child(crate::ui::controls::switch("stack-drafts", draft, "Create as drafts", None, false, move |v, _, cx| {
                        d3.borrow_mut().draft = v;
                        cx.refresh_windows();
                    }))
                })
                .child(
                    modal::actions_row()
                        .child(crate::ui::controls::tabbable(crate::ui::controls::button("form-cancel", "Cancel", crate::ui::controls::ButtonKind::Plain, true).on_click(move |_, window, cx| modal::close(ID, window, cx)), &stop_a, move |window, cx| modal::close(ID, window, cx)))
                        .child({
                            let (s5, w5) = (s4.clone(), w4.clone());
                            let run = move |window: &mut Window, cx: &mut App| {
                                if create_stack_submit(&s5, &w5, cx) {
                                    modal::close(ID, window, cx);
                                }
                            };
                            let r2 = run.clone();
                            crate::ui::anchor("stack-submit", crate::ui::controls::tabbable(crate::ui::controls::button("form-ok", label.unwrap_or_else(|| format!("Create stack {noun}s")), crate::ui::controls::ButtonKind::Primary, can).when(can, |d| d.on_click(move |_, window, cx| r2(window, cx))), &stop_b, run))
                        }),
                )
                .into_any_element()
        })
        .width(560.);
        let _ = stop_c;
        modal::open(m, window, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn run_create_stack(&mut self, tab: u32, kind: ForgeKind, remote: String, view: StackView, plans: Vec<MemberPlan>, titles: HashMap<String, String>, draft: bool, cx: &mut Context<Self>) {
        toast::show_with(format!("Creating the stack's {}s…", mr_noun(kind)), Options { sticky: true, ..Default::default() }, cx);
        let (api, rt) = (self.api.clone(), self.rt.clone());
        let repo = core_repo(tab);
        cx.spawn(async move |this, cx| {
            let mut r = CreateReport::default();
            for (i, p) in plans.iter().enumerate() {
                let step = match p {
                    MemberPlan::Retarget { mr, target, .. } => super::ask::<ForgeMr>(api.clone(), rt.clone(), Request::ForgeRetarget { repo, number: mr.number, target: target.clone() }).await.map(|m| r.retargeted.push(m.number)).map_err(|e| (false, e)),
                    MemberPlan::Create { branch, target, title, description } => {
                        let req = CreateMr { source: SourceRef { project: view.project.clone(), branch: branch.clone() }, target_branch: target.clone(), title: titles.get(branch).unwrap_or(title).trim().to_string(), description: description.clone(), draft, reviewers: Vec::new(), assignees: Vec::new(), labels: Vec::new(), squash: None, delete_source_branch: None };
                        super::ask::<CreateOutcome>(api.clone(), rt.clone(), Request::ForgeCreateMr { repo, remote: remote.clone(), req }).await.map(|o| r.created.push(o.mr.number)).map_err(|e| (true, e))
                    }
                    _ => Ok(()),
                };
                if let Err((create, why)) = step {
                    r.failed = Some((p.branch().to_string(), create, why));
                    r.rest = plans[i + 1..].iter().filter(|q| matches!(q, MemberPlan::Create { .. } | MemberPlan::Retarget { .. })).map(|q| q.branch().to_string()).collect();
                    break;
                }
            }
            if view.mode == StackMode::Managed && (!r.created.is_empty() || !r.retargeted.is_empty()) {
                let branches = view.members.iter().map(|m| m.branch.clone()).collect();
                let base = view.members.first().map_or(String::new(), |m| m.target_branch.clone());
                match super::ask::<StackSync>(api, rt, Request::ForgeSyncStack { repo, branches, base }).await {
                    Ok(s) => r.table = s.failed.iter().map(|f| (f.number, f.message.clone())).collect(),
                    Err(e) => r.table = vec![(0, e)],
                }
            }
            this.update(cx, |v, cx| {
                let (message, detail, error, warning) = create_summary(&r, kind);
                toast::show_with(message, Options { error, warning: warning && !error, detail: detail.map(Into::into), ..Default::default() }, cx);
                v.forge_written(tab, cx);
            })
            .ok();
        })
        .detach();
    }
}

#[derive(Default)]
struct CreateStackUi {
    tab: u32,
    kind: Option<ForgeKind>,
    remote: String,
    view: Option<StackView>,
    plans: Vec<MemberPlan>,
    titles: HashMap<String, Entity<TextInput>>,
    draft: bool,
    error: Option<String>,
    busy: bool,
    keep: Vec<Subscription>,
}

/// Starts the create, when it can; a second press does nothing.
fn create_stack_submit(s: &Rc<RefCell<CreateStackUi>>, w: &WeakEntity<GitVolt>, cx: &mut App) -> bool {
    let (tab, kind, remote, view, plans, titles, draft) = {
        let st = s.borrow();
        let Some(view) = st.view.clone() else { return false };
        let Some(kind) = st.kind else { return false };
        let titles: HashMap<String, String> = st.titles.iter().map(|(k, i)| (k.clone(), i.read(cx).text().to_string())).collect();
        let merged = st.plans.iter().any(|p| matches!(p, MemberPlan::Merged { .. }));
        let empty = st.plans.iter().any(|p| matches!(p, MemberPlan::Create { branch, .. } if titles.get(branch).is_none_or(|t| t.trim().is_empty())));
        if st.busy || merged || empty || submit_label(&st.plans, kind).is_none() {
            return false;
        }
        (st.tab, kind, st.remote.clone(), view, st.plans.clone(), titles, st.draft)
    };
    s.borrow_mut().busy = true;
    w.update(cx, |v, cx| v.run_create_stack(tab, kind, remote, view, plans, titles, draft, cx)).ok();
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn mr(n: u64, src: &str, dst: &str, state: &str, stacked: bool) -> ForgeMr {
        let mut v: ForgeMr = serde_json::from_value(serde_json::json!({
            "number": n, "title": format!("T{n}"), "state": state, "sourceBranch": src, "targetBranch": dst, "sourceProject": "g/p", "targetProject": "g/p",
            "author": {"id": 1, "username": "ada", "name": "Ada", "avatarUrl": null, "email": null, "webUrl": ""},
            "webUrl": "", "updatedAt": n as i64, "createdAt": 0, "labels": [], "labelColors": {}, "headSha": format!("{n}{n}{n}"), "review": {"decision": "none", "approvals": 0, "approvalsRequired": null, "reviews": []}, "pipeline": null, "conflicts": null, "stacked": stacked,
        })).unwrap_or_else(|e| panic!("{e}"));
        v.stacked = stacked;
        v
    }

    fn env(mrs: Vec<ForgeMr>, locals: &[&str], stack: Option<Vec<&str>>) -> StackEnv {
        let by: HashMap<String, ForgeMr> = mrs.into_iter().map(|m| (m.source_branch.clone(), m)).collect();
        let s = stack.map(|b| Stack { branches: b.iter().map(|x| x.to_string()).collect(), base: "refs/remotes/origin/main".into(), left_behind: Vec::new(), partial: false });
        StackEnv {
            mr_of: Box::new(move |b| by.get(b).cloned()),
            locals: locals.iter().map(|x| x.to_string()).collect(),
            default_branch: "main".into(),
            base: Some("main".into()),
            stack_of: Box::new(move |b| s.clone().filter(|s| s.branches.iter().any(|x| x == b))),
            tip_of: Box::new(|b| Some(format!("tip-{b}"))),
        }
    }

    #[test]
    fn the_chain_goes_down_to_the_base_and_up_through_local_branches() {
        let (a, b, c) = (mr(1, "a", "main", "open", false), mr(2, "b", "a", "open", false), mr(3, "c", "b", "open", false));
        let e = env(vec![a.clone(), b.clone(), c], &["a", "b", "c"], None);
        let chain = mr_chain(&b, &e).unwrap();
        assert_eq!(chain.mrs.iter().map(|m| m.number).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(chain.index, 1);
        assert_eq!(stack_line_text(&chain, ForgeKind::GitLab), "Stack 2 of 3 (below: !1, above: !3)");
    }

    #[test]
    fn a_walk_that_ends_off_the_base_isnt_a_stack() {
        let (a, b) = (mr(1, "a", "release", "open", false), mr(2, "b", "a", "open", false));
        let e = env(vec![a, b.clone()], &["a", "b"], None);
        assert_eq!(mr_chain(&b, &e), None);
    }

    #[test]
    fn after_a_merge_a_retarget_needs_evidence() {
        let (a, b, c) = (mr(1, "a", "main", "merged", false), mr(2, "b", "a", "open", false), mr(3, "c", "b", "open", false));
        let e = env(vec![a, b, c], &["a", "b", "c"], Some(vec!["a", "b", "c"]));
        assert_eq!(after_merge("c", &e), None);
        let (a, b, c) = (mr(1, "a", "main", "merged", true), mr(2, "b", "a", "open", true), mr(3, "c", "b", "open", true));
        let e = env(vec![a, b, c], &["a", "b", "c"], Some(vec!["a", "b", "c"]));
        let got = after_merge("c", &e).unwrap();
        assert!(got.retarget);
        assert_eq!(got.target, "main");
        assert_eq!(got.branches, ["b", "c"]);
        assert_eq!(got.drop_from, ["111", "tip-a"]);
        assert_eq!(after_merge_label(&got, ForgeKind::GitHub), "Retarget #2 and rebase the stack");
    }

    #[test]
    fn a_forge_that_retargeted_leaves_a_rebase_only() {
        let (a, b) = (mr(1, "a", "main", "merged", false), mr(2, "b", "main", "open", false));
        let e = env(vec![a, b], &["a", "b"], Some(vec!["a", "b"]));
        let got = after_merge("b", &e).unwrap();
        assert!(!got.retarget);
        assert_eq!(after_merge_label(&got, ForgeKind::GitLab), "Rebase the stack without the merged !1");
    }

    #[test]
    fn plans_and_their_button() {
        let plans = vec![MemberPlan::Create { branch: "a".into(), target: "main".into(), title: "A".into(), description: String::new() }, MemberPlan::Retarget { branch: "b".into(), target: "a".into(), mr: Box::new(mr(2, "b", "main", "open", false)) }];
        assert_eq!(submit_label(&plans, ForgeKind::GitLab).as_deref(), Some("Create 1 MR and retarget 1"));
        assert_eq!(submit_label(&plans[..1], ForgeKind::GitHub).as_deref(), Some("Create 1 PR"));
        assert_eq!(submit_label(&[], ForgeKind::GitHub), None);
        assert_eq!(base_branch("refs/remotes/origin/main", "origin"), "main");
        assert_eq!(base_branch("refs/heads/dev", "origin"), "dev");
        assert_eq!(base_branch("refs/remotes/up/x/y", "origin"), "x/y");
    }

    #[test]
    fn the_summary_says_what_happened() {
        let r = CreateReport { created: vec![1], retargeted: vec![2], failed: Some(("c".into(), true, "no".into())), rest: vec!["d".into()], table: vec![(0, "x".into())] };
        let (m, d, error, warning) = create_summary(&r, ForgeKind::GitLab);
        assert_eq!(m, "Created !1; retargeted !2; c's MR wasn't created: no; not created: d");
        assert_eq!(d.as_deref(), Some("Couldn't update the stack tables: x"));
        assert!(error && warning);
    }
}
