//! Stacks (`ui/src/stacks/`): local branches B1 → … → Bn, each tip an ancestor of the next on its first-parent line, B1 ahead
//! of a base: a branch chip's Push stack and Rebase stack onto <base>.

use crate::app::{GitVolt, Tab};
use crate::irebase::model::OpenRebase;
use crate::model::{Chip, GraphView};
use crate::sync::{LocalBranch, PushOpts, nothing_to_push, rewrote_since_push};
use crate::ui::arm::{self, ArmRequest, Origin, Tone};
use crate::ui::menu::MenuRow;
use crate::ui::popover::{self, Choice, ChoiceRequest};
use crate::ui::toast::{self, Options, ToastAction};
use crate::write::{Write, WriteCtx, call};
use gitvolt_core::error::{GbError, GbErrorKind};
use gpui::*;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

const HEADS: &str = "refs/heads/";
const TRUNKS: [&str; 3] = ["main", "master", "trunk"];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stack {
    /// Short names, bottom (next to the base) → top. Two or more.
    pub branches: Vec<String>,
    /// The full ref it sits on.
    pub base: String,
    /// Local branches built on a member that aren't on this straight path: a rebase leaves them on the old commits.
    pub left_behind: Vec<String>,
    /// The loaded graph cuts through this stack.
    pub partial: bool,
}

/// `shortRef`.
pub fn short_ref(r: &str) -> String {
    r.trim_start_matches("refs/heads/").trim_start_matches("refs/remotes/").to_string()
}

/// `joinNames`.
pub fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        _ => format!("{} and {}", names[..names.len() - 1].join(", "), names[names.len() - 1]),
    }
}

fn labels(g: &GraphView) -> impl Iterator<Item = (usize, &Chip)> {
    g.rows.iter().enumerate().flat_map(|(i, r)| r.chips.iter().map(move |c| (i, c)))
}

/// `findBase`: the base's row, and its branch name (`main` for `origin/main`).
fn find_base(g: &GraphView, base: &str) -> Option<(usize, String)> {
    for (row, c) in labels(g) {
        if c.local && format!("{HEADS}{}", c.name) == base {
            return Some((row, base[HEADS.len()..].to_string()));
        }
        if let Some((remote, full)) = c.remotes.iter().find(|r| r.1 == base) {
            return Some((row, full.trim_start_matches(&format!("refs/remotes/{remote}/")).to_string()));
        }
    }
    None
}

/// `detectStacks`: every stack on `base` in the loaded graph: one per straight path, newest top first. Linear: rows come
/// children before parents.
pub fn detect_stacks(g: &GraphView, base: &str) -> Vec<Stack> {
    let Some((base_row, base_name)) = find_base(g, base) else { return Vec::new() };
    let n = g.rows.len();
    let mut in_base = vec![false; n];
    in_base[base_row] = true;
    for i in base_row..n {
        if in_base[i] {
            for p in &g.rows[i].parents {
                if let Some(j) = g.by_sha.get(p) {
                    in_base[*j] = true;
                }
            }
        }
    }
    let mut on_row: HashMap<usize, Vec<String>> = HashMap::new();
    let mut row_of: HashMap<String, usize> = HashMap::new();
    for (row, c) in labels(g) {
        let name = c.name.to_string();
        if c.tag || !c.local || in_base[row] || name == base_name || row_of.contains_key(&name) {
            continue;
        }
        row_of.insert(name.clone(), row);
        on_row.entry(row).or_default().push(name);
    }
    for list in on_row.values_mut() {
        list.sort();
    }
    // below[i]: the nearest member strictly below row i on its first-parent line.
    let mut below: Vec<Option<String>> = vec![None; n];
    for i in (0..n).rev() {
        let Some(j) = g.rows[i].parents.first().and_then(|p| g.by_sha.get(p)).copied() else { continue };
        if in_base[j] {
            continue;
        }
        below[i] = on_row.get(&j).and_then(|l| l.last().cloned()).or_else(|| below[j].clone());
    }
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    let mut roots = Vec::new();
    let mut rows_sorted: Vec<&usize> = on_row.keys().collect();
    rows_sorted.sort();
    for row in rows_sorted {
        let list = &on_row[row];
        for (k, name) in list.iter().enumerate() {
            let parent = if k > 0 { Some(list[k - 1].clone()) } else { below[*row].clone() };
            match parent {
                None => roots.push(name.clone()),
                Some(p) => children.entry(p).or_default().push(name.clone()),
            }
        }
    }
    let newest = |a: &String, b: &String| row_of[a].cmp(&row_of[b]).then_with(|| a.cmp(b));
    for c in children.values_mut() {
        c.sort_by(newest);
    }
    roots.sort_by(newest);
    fn walk(name: &str, path: &[String], children: &HashMap<String, Vec<String>>, base: &str, out: &mut Vec<Stack>) {
        let mut next = path.to_vec();
        next.push(name.to_string());
        match children.get(name) {
            Some(kids) if !kids.is_empty() => {
                for k in kids {
                    walk(k, &next, children, base, out);
                }
            }
            _ if next.len() >= 2 => out.push(Stack { branches: next, base: base.to_string(), ..Default::default() }),
            _ => {}
        }
    }
    let mut stacks = Vec::new();
    for r in &roots {
        walk(r, &[], &children, base, &mut stacks);
    }
    fn subtree(name: &str, children: &HashMap<String, Vec<String>>, out: &mut Vec<String>) {
        out.push(name.to_string());
        for k in children.get(name).map_or(&[][..], Vec::as_slice) {
            subtree(k, children, out);
        }
    }
    for s in &mut stacks {
        let on: HashSet<&String> = s.branches.iter().collect();
        let mut left = Vec::new();
        for m in &s.branches {
            for k in children.get(m).map_or(&[][..], Vec::as_slice) {
                if !on.contains(k) {
                    subtree(k, &children, &mut left);
                }
            }
        }
        s.left_behind = left;
        s.partial = s.branches.iter().any(|name| g.rows[row_of[name]].parents.first().is_some_and(|p| !g.by_sha.contains_key(p)));
    }
    stacks.sort_by(|a, b| newest(a.branches.last().expect("two or more"), b.branches.last().expect("two or more")));
    stacks
}

/// `stackFor`: the stack a branch's menu acts on: at or below a fork, the one topped by the checked-out branch, else the
/// newest top's.
pub fn stack_for<'a>(stacks: &'a [Stack], branch: &str, head: Option<&str>) -> Option<&'a Stack> {
    let through: Vec<&Stack> = stacks.iter().filter(|s| s.branches.iter().any(|b| b == branch)).collect();
    through.iter().find(|s| s.branches.last().map(String::as_str) == head).copied().or(through.first().copied())
}

/// `stackBase`: a remote's default branch, origin first; else `<remote>/main|master|trunk`; else a local trunk.
pub fn stack_base(g: &GraphView, remotes: &[(String, Option<String>)]) -> Option<String> {
    let mut present = HashSet::new();
    let mut names: Vec<String> = remotes.iter().map(|r| r.0.clone()).collect();
    for (_, c) in labels(g) {
        if c.local {
            present.insert(format!("{HEADS}{}", c.name));
        }
        for (r, full) in &c.remotes {
            present.insert(full.clone());
            if !names.contains(r) {
                names.push(r.clone());
            }
        }
    }
    names.sort_by_key(|n| (n != "origin", n.clone()));
    for name in &names {
        if let Some(d) = remotes.iter().find(|r| &r.0 == name).and_then(|r| r.1.as_ref())
            && present.contains(d)
        {
            return Some(d.clone());
        }
    }
    for name in &names {
        for b in TRUNKS {
            let r = format!("refs/remotes/{name}/{b}");
            if present.contains(&r) {
                return Some(r);
            }
        }
    }
    TRUNKS.iter().map(|b| format!("{HEADS}{b}")).find(|r| present.contains(r))
}

#[derive(Debug, Clone, PartialEq)]
pub enum StepResult {
    Pushed,
    Forced,
    Published,
    UpToDate,
}

/// `pushSummary`: the one toast for a Push stack.
pub fn push_summary(steps: &[(String, StepResult)], failed: Option<(&str, bool, &str)>, rest: &[String], remote: &str) -> (String, Option<String>, bool) {
    let of = |k: StepResult| steps.iter().filter(|s| s.1 == k).map(|s| s.0.clone()).collect::<Vec<_>>();
    let sent: Vec<String> = steps.iter().filter(|s| s.1 != StepResult::UpToDate).map(|s| s.0.clone()).collect();
    if let Some((name, cancelled, message)) = failed {
        let mut parts = Vec::new();
        if !sent.is_empty() {
            parts.push(format!("Pushed {}", join_names(&sent)));
        }
        parts.push(format!("{name} {}", if cancelled { "was cancelled" } else { "was refused" }));
        if !rest.is_empty() {
            parts.push(format!("not pushed: {}", join_names(rest)));
        }
        return (parts.join("; "), (!cancelled).then(|| message.to_string()), !cancelled);
    }
    if sent.is_empty() {
        return ("The stack is up to date".to_string(), None, false);
    }
    let (forced, published) = (of(StepResult::Forced), of(StepResult::Published));
    let mut notes = Vec::new();
    if !forced.is_empty() {
        notes.push(format!("force with lease: {}", join_names(&forced)));
    }
    if !published.is_empty() {
        notes.push(format!("new on {remote}: {}", join_names(&published)));
    }
    let tail = if notes.is_empty() { String::new() } else { format!(" ({})", notes.join("; ")) };
    (format!("Pushed {}{tail}", join_names(&sent)), None, false)
}

/// `stackPlan`: Rebase stack as an interactive rebase: every row Pick, but rows already in the base: Drop; a lower member
/// whose tip row, or a row below it, is still replayed moves with its row; the rest stay.
pub fn stack_plan(stack: &Stack, plan: &Value) -> Value {
    let rows = plan["rows"].as_array().cloned().unwrap_or_default();
    let index: HashMap<&str, usize> = rows.iter().enumerate().map(|(i, r)| (r["oid"].as_str().unwrap_or_default(), i)).collect();
    let goes = |i: usize| rows[i]["upstream"] == true;
    let replayed = |at: &str| index.get(at).is_some_and(|i| (*i..rows.len()).any(|k| !goes(k)));
    let lower: HashSet<&String> = stack.branches[..stack.branches.len() - 1].iter().collect();
    let out_rows: Vec<Value> = rows.iter().enumerate().map(|(i, r)| json!({ "oid": r["oid"], "action": if goes(i) { "drop" } else { "pick" } })).collect();
    let chips: Vec<Value> = plan["chips"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|c| {
            let (branch, at) = (c["branch"].as_str().unwrap_or_default().to_string(), c["at"].as_str().unwrap_or_default().to_string());
            if lower.contains(&branch) && replayed(&at) { json!({ "branch": branch, "at": { "kind": "row", "oid": at } }) } else { json!({ "branch": branch, "at": { "kind": "stay" } }) }
        })
        .collect();
    json!({ "branch": stack.branches.last(), "base": stack.base, "expect": plan["expect"], "rows": out_rows, "chips": chips })
}

fn commits_word(n: usize) -> String {
    format!("{n} {}", if n == 1 { "commit" } else { "commits" })
}

/// `rebaseConfirm`: the armed row's label and caption: what moves, what conflicts, what stays behind.
pub fn rebase_confirm(stack: &Stack, req: &Value, conflicting: usize) -> (String, Option<String>, bool) {
    let base = short_ref(&stack.base);
    let chips = req["chips"].as_array().map_or(&[][..], Vec::as_slice);
    let kind_of = |b: &str| chips.iter().find(|c| c["branch"] == b).map(|c| c["at"]["kind"].as_str().unwrap_or_default().to_string());
    let lower = &stack.branches[..stack.branches.len() - 1];
    let in_base: Vec<String> = lower.iter().filter(|n| kind_of(n).as_deref() == Some("stay")).cloned().collect();
    let unplanned: Vec<String> = lower.iter().filter(|n| kind_of(n).is_none()).cloned().collect();
    let moving = stack.branches.len() - in_base.len() - unplanned.len();
    let what = format!("{moving} {}", if moving == 1 { "branch" } else { "branches" });
    let one = |names: &[String]| names.len() == 1;
    let others: Vec<String> = chips.iter().filter(|c| c["at"]["kind"] == "stay").filter_map(|c| c["branch"].as_str()).filter(|b| !stack.branches.iter().any(|m| m == b)).map(str::to_string).collect();
    let shown = if others.len() > 2 { format!("{} +{} more", others[..2].join(", "), others.len() - 2) } else { join_names(&others) };
    let mut notes = Vec::new();
    if conflicting > 0 {
        notes.push(format!("{} will conflict.", commits_word(conflicting)));
    }
    if !in_base.is_empty() {
        notes.push(format!("{} {} already in {base} and {}.", join_names(&in_base), if one(&in_base) { "is" } else { "are" }, if one(&in_base) { "stays where it is" } else { "stay where they are" }));
    }
    if !unplanned.is_empty() {
        notes.push(format!("{} {}.", join_names(&unplanned), if one(&unplanned) { "stays where it is" } else { "stay where they are" }));
    }
    if !others.is_empty() {
        notes.push(format!("{shown} {}.", if one(&others) { "stays where it is" } else { "stay where they are" }));
    }
    if !stack.left_behind.is_empty() {
        notes.push(format!("{} {} on the old commits.", join_names(&stack.left_behind), if one(&stack.left_behind) { "stays" } else { "stay" }));
    }
    let tail = if conflicting > 0 { format!(" (conflicts at {})", commits_word(conflicting)) } else { String::new() };
    (format!("Click again to rebase {what} onto {base}{tail}"), (!notes.is_empty()).then(|| notes.join(" ")), conflicting > 0)
}

impl Tab {
    /// The stacks of the loaded graph, on its base.
    pub fn stack_of(&self, branch: &str) -> Option<Stack> {
        let g = self.graph.as_ref()?;
        let remotes: Vec<(String, Option<String>)> = self.sidebar["remotes"].as_array().map_or(Vec::new(), |r| r.iter().map(|x| (x["name"].as_str().unwrap_or_default().to_string(), x["defaultBranch"].as_str().map(str::to_string))).collect());
        let base = stack_base(g, &remotes)?;
        let stacks = detect_stacks(g, &base);
        let head = self.head_branch();
        stack_for(&stacks, branch, head.as_deref()).cloned()
    }
}

impl GitVolt {
    /// "Push stack" on a member's chip: stacked, the repository has a remote, a member has something to push.
    pub fn push_stack_row(&self, name: &str, this: &WeakEntity<GitVolt>) -> Option<MenuRow> {
        let tab = self.tabs.get(self.active)?;
        let stack = tab.stack_of(name)?;
        if tab.remote_groups().is_empty() {
            return None;
        }
        let locals: Vec<LocalBranch> = stack.branches.iter().map(|n| tab.local_branch(n)).collect::<Option<_>>()?;
        if locals.iter().all(nothing_to_push) {
            return None;
        }
        let main = tab.graph.as_ref().and_then(|g| g.worktrees.iter().find(|w| w.is_main)).map_or(String::new(), |w| w.path.clone());
        let away = locals.iter().find_map(|b| b.worktree.as_ref().map(|w| (b.name.clone(), crate::manage::worktree_shown(&main, w))));
        let forced: Vec<String> = locals.iter().filter(|b| rewrote_since_push(b).is_some()).map(|b| b.name.clone()).collect();
        let tail = if forced.is_empty() { String::new() } else { format!(" (force with lease: {})", join_names(&forced)) };
        let why = tab.busy_reason().or_else(|| away.map(|(b, w)| format!("{b} is checked out in {w}")));
        let weak = this.clone();
        let tooltip = format!("Push {}, bottom first{tail}", stack.branches.join(" → "));
        Some(
            MenuRow::action("stack.push", "Push stack", "arrow-up-from-line", tooltip, move |_, cx| {
                let stack = stack.clone();
                weak.update(cx, |v, cx| v.push_stack(stack, cx)).ok();
            })
            .disabled_if(why),
        )
    }

    /// `pushStack`: each member's Push, bottom first; the first failure stops it, and one toast says which pushed.
    pub fn push_stack(&mut self, stack: Stack, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let ctx = tab.write_target();
        let names: Vec<String> = tab.remote_groups().into_iter().map(|g| g.0).collect();
        let own = names.iter().filter(|n| stack.base.starts_with(&format!("refs/remotes/{n}/"))).max_by_key(|n| n.len()).cloned();
        let remote = own.unwrap_or_else(|| tab.default_remote());
        self.push_stack_step(ctx, stack, remote, 0, Vec::new(), cx);
    }

    fn push_stack_step(&mut self, ctx: WriteCtx, stack: Stack, remote: String, i: usize, mut steps: Vec<(String, StepResult)>, cx: &mut Context<Self>) {
        let Some(name) = stack.branches.get(i).cloned() else {
            return self.push_stack_report(&ctx, &steps, None, &[], &remote, cx);
        };
        let rest: Vec<String> = stack.branches[i + 1..].to_vec();
        let Some(b) = self.tabs.iter().find(|t| t.id == ctx.repo).and_then(|t| t.local_branch(&name)) else {
            let e = GbError::new(GbErrorKind::NotFound, format!("{name} no longer exists"));
            return self.push_stack_report(&ctx, &steps, Some((name, e)), &rest, &remote, cx);
        };
        if nothing_to_push(&b) {
            steps.push((name, StepResult::UpToDate));
            return self.push_stack_step(ctx, stack, remote, i + 1, steps, cx);
        }
        let publish = b.push_target.is_none();
        let (repo, worktree, send_name, full, target, r2) = (ctx.repo, ctx.worktree.clone(), name.clone(), b.full_name.clone(), b.target.clone(), remote.clone());
        let body = move |_: crate::write::Asked| {
            let mut opts = json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "branch": send_name, "expect": { "head": null, "refs": { full.clone(): target } } });
            if publish {
                opts["target"] = json!({ "remote": r2, "branch": send_name });
                opts["setUpstream"] = json!(true);
            }
            call("push", opts)
        };
        let (c1, s1, r1, st1, n1) = (ctx.clone(), stack.clone(), remote.clone(), steps.clone(), name.clone());
        let (c2, st2, r3, n2, rest2) = (ctx.clone(), steps.clone(), remote.clone(), name, rest);
        let w = Write::new(&ctx, body)
            .on_success(move |this, out, cx| {
                let mut steps = st1.clone();
                let result = if out["upToDate"] == true { StepResult::UpToDate } else if out["forced"] == true { StepResult::Forced } else if publish { StepResult::Published } else { StepResult::Pushed };
                steps.push((n1.clone(), result));
                this.push_stack_step(c1.clone(), s1.clone(), r1.clone(), i + 1, steps, cx);
            })
            .handle(move |this, e, cx| {
                this.push_stack_report(&c2, &st2, Some((n2.clone(), e.clone())), &rest2, &r3, cx);
                true
            });
        self.write(w, cx);
    }

    fn push_stack_report(&mut self, ctx: &WriteCtx, steps: &[(String, StepResult)], failed: Option<(String, GbError)>, rest: &[String], remote: &str, cx: &mut Context<Self>) {
        let cancelled = failed.as_ref().is_some_and(|f| f.1.kind == GbErrorKind::Cancelled);
        let (message, detail, error) = push_summary(steps, failed.as_ref().map(|f| (f.0.as_str(), cancelled, f.1.message.as_str())), rest, remote);
        let mut actions = Vec::new();
        if let Some((name, e)) = failed.as_ref().filter(|_| !cancelled)
            && matches!(e.kind, GbErrorKind::NonFastForward | GbErrorKind::RefMoved)
            && let Some(b) = self.tabs.iter().find(|t| t.id == ctx.repo).and_then(|t| t.local_branch(name))
        {
            let (weak, ctx) = (cx.weak_entity(), ctx.clone());
            actions.push(ToastAction {
                label: format!("Push {name}…").into(),
                run: Rc::new(move |_, cx| {
                    let (ctx, b) = (ctx.clone(), b.clone());
                    weak.update(cx, |v, cx| v.send_push(ctx, b, PushOpts::default(), Origin::None, cx)).ok();
                }),
            });
        }
        toast::show_with(message, Options { error, detail: detail.map(Into::into), actions, ..Default::default() }, cx);
    }

    /// "Rebase stack onto <base>" on any member's chip: hidden when it already sits on the base; greyed during another
    /// operation, while a member is checked out elsewhere, or while HEAD isn't the top.
    pub fn rebase_stack_row(&self, name: &str, this: &WeakEntity<GitVolt>) -> Option<MenuRow> {
        let tab = self.tabs.get(self.active)?;
        let g = tab.graph.as_ref()?;
        let stack = tab.stack_of(name)?;
        let base = short_ref(&stack.base);
        let base_tip = g.rows.iter().position(|r| r.chips.iter().any(|c| c.remotes.iter().any(|x| x.1 == stack.base) || (c.local && format!("{HEADS}{}", c.name) == stack.base))).map(|i| g.rows[i].id.to_string());
        let bottom = tab.local_branch(&stack.branches[0]).map(|b| b.target);
        if let (Some(t), Some(b)) = (&base_tip, &bottom)
            && crate::refs::is_ancestor(g, t, b) == Some(true)
        {
            return None;
        }
        let main = g.worktrees.iter().find(|w| w.is_main).map_or(String::new(), |w| w.path.clone());
        let away = stack.branches.iter().filter_map(|n| tab.local_branch(n)).find_map(|b| b.worktree.as_ref().map(|w| (b.name.clone(), crate::manage::worktree_shown(&main, w))));
        let top = stack.branches.last().cloned().unwrap_or_default();
        let why = tab
            .busy_reason()
            .or_else(|| stack.partial.then(|| "Load more history first".to_string()))
            .or_else(|| away.map(|(b, w)| format!("{b} is checked out in {w}")))
            .or_else(|| (tab.head_branch().as_deref() != Some(top.as_str())).then(|| format!("Check out {top} first")));
        let weak = this.clone();
        let tooltip = format!("Rebase {} onto {base}, moving every branch", stack.branches.join(" → "));
        Some(
            MenuRow::action("stack.rebase", format!("Rebase stack onto {base}"), "git-pull-request-arrow", tooltip, move |window, cx| {
                let stack = stack.clone();
                let origin = Origin::current(cx);
                let hold = arm::hold_row(cx);
                weak.update(cx, |v, cx| v.rebase_stack(stack, origin, hold, window, cx)).ok();
            })
            .disabled_if(why),
        )
    }

    /// `rebaseStack`: the plan, the refusals, the confirm; then one interactive rebase moving every member, one Undo for all.
    pub fn rebase_stack(&mut self, stack: Stack, origin: Origin, hold: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let ctx = tab.write_target();
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let top = stack.branches.last().cloned().unwrap_or_default();
        let base = short_ref(&stack.base);
        let plan_body = call("rebasePlan", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "branch": top, "base": stack.base }));
        let _ = window;
        if cfg!(test) {
            self.sent.borrow_mut().push(plan_body);
            return;
        }
        let api = self.api.clone();
        let read = self.rt.spawn(async move {
            let plan = api.dispatch(crate::write::request_of(plan_body)?).await?;
            let req_rows = plan["rows"].as_array().map_or(Vec::new(), |rows| rows.iter().map(|r| json!({ "oid": r["oid"], "action": if r["upstream"] == true { "drop" } else { "pick" } })).collect::<Vec<_>>());
            let predict = call("predictRebase", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "base": plan["base"].as_str().map_or(Value::Null, |_| plan["base"].clone()), "rows": req_rows }));
            let p = match crate::write::request_of(predict) {
                Ok(req) => api.dispatch(req).await.ok(),
                Err(_) => None,
            };
            Ok::<_, GbError>((plan, p))
        });
        cx.spawn(async move |this, cx| {
            let answer = read.await.unwrap_or_else(|e| Err(GbError::other(e.to_string())));
            let Some(window_handle) = this.update(cx, |v, _| v.window).ok().flatten() else { return };
            window_handle
                .update(cx, |_, window, cx| {
                    this.update(cx, |v, cx| v.rebase_stack_read(stack, ctx, origin, hold, answer, base, window, cx)).ok();
                })
                .ok();
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn rebase_stack_read(&mut self, stack: Stack, ctx: WriteCtx, origin: Origin, hold: Option<u64>, answer: Result<(Value, Option<Value>), GbError>, base: String, window: &mut Window, cx: &mut Context<Self>) {
        let top = stack.branches.last().cloned().unwrap_or_default();
        let (plan, predicted) = match answer {
            Ok(x) => x,
            Err(e) => {
                arm::release_row(hold, window, cx);
                return toast::show_with(e.message, Options { error: true, ..Default::default() }, cx);
            }
        };
        let merges = plan["merges"].as_u64().unwrap_or(0);
        if merges > 0 {
            // Never flatten without the editor's warning.
            arm::release_row(hold, window, cx);
            let weak = cx.weak_entity();
            let (branch, b2) = (top, stack.base.clone());
            let word = if merges == 1 { "commit" } else { "commits" };
            let req = ChoiceRequest { title: format!("Rebase the stack onto {base}?").into(), body: Some(format!("This would flatten {merges} merge {word}: use the interactive rebase editor.").into()), note: None, choices: vec![Choice { id: "editor".into(), label: "Open the editor".into(), primary: true, ..Default::default() }, Choice { id: "cancel".into(), label: "Cancel".into(), ..Default::default() }] };
            popover::ask_choice(
                req,
                None,
                move |picked, _, cx| {
                    if picked.as_deref() == Some("editor") {
                        let opts = OpenRebase { branch: branch.clone(), base: b2.clone(), preset: None };
                        weak.update(cx, |v, cx| v.open_rebase_editor(opts, cx)).ok();
                    }
                },
                window,
                cx,
            );
            return;
        }
        let rows = plan["rows"].as_array().cloned().unwrap_or_default();
        if !rows.is_empty() && rows.iter().all(|r| r["upstream"] == true) {
            arm::release_row(hold, window, cx);
            return toast::show(format!("{top} is already in {base}"), cx);
        }
        if plan["behind"].as_u64() == Some(0) {
            arm::release_row(hold, window, cx);
            return toast::show(format!("The stack is already on {base}"), cx);
        }
        let req = stack_plan(&stack, &plan);
        let conflicting = predicted.as_ref().filter(|p| p.get("off").is_none_or(Value::is_null)).and_then(|p| p["rows"].as_array()).map_or(0, |r| r.iter().filter(|x| x["conflicts"].as_array().is_some_and(|c| !c.is_empty())).count());
        let (label, caption, warn) = rebase_confirm(&stack, &req, conflicting);
        let mut armed = ArmRequest::new(if warn { Tone::Warn } else { Tone::Positive }, label, format!("Rebase the stack onto {base}?"), "Rebase");
        armed.body = caption.clone().unwrap_or_default().into();
        armed.caption = caption.map(Into::into);
        let weak = cx.weak_entity();
        let (again, moving) = (origin.clone(), req["chips"].as_array().map_or(0, |c| c.iter().filter(|x| x["at"]["kind"] == "row").count()) + 1);
        arm::request(
            armed,
            origin,
            move |a, _, cx| {
                if a.ok {
                    let (ctx, req, base, origin) = (ctx.clone(), req.clone(), base.clone(), again.clone());
                    weak.update(cx, |v, cx| v.send_stack_rebase(ctx, req, moving, base, origin, cx)).ok();
                }
            },
            window,
            cx,
        );
        arm::release_row(hold, window, cx);
    }

    fn send_stack_rebase(&mut self, ctx: WriteCtx, req: Value, moving: usize, base: String, origin: Origin, cx: &mut Context<Self>) {
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let w = Write::new(&ctx, move |asked| call("interactiveRebase", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "branch": req["branch"], "base": req["base"], "expect": req["expect"], "rows": req["rows"], "chips": req["chips"], "confirm": { "autostash": asked.autostash } })))
            .on_success(move |_, out, cx| match out["status"].as_str() {
                Some("done") => toast::show_with(format!("Rebased {moving} branches onto {base}"), Options { warning: out["warning"].is_string(), detail: out["warning"].as_str().map(|w| w.to_string().into()), ..Default::default() }, cx),
                Some("upToDate") => toast::show(format!("The stack is already on {base}"), cx),
                Some("stopped") => {
                    if let Some(why) = out["reason"].as_str().or(out["message"].as_str()) {
                        toast::show_with(why.to_string(), Options { warning: true, ..Default::default() }, cx);
                    }
                }
                _ => {}
            })
            .origin(origin);
        self.write(w, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RowView;
    use gitvolt_graph::NodeKind;
    use std::prelude::v1::test;

    fn local(name: &str) -> Chip {
        Chip { name: name.to_string().into(), head: false, local: true, remote: false, tag: false, remotes: Vec::new() }
    }

    fn row(id: &str, parents: &[&str], chips: Vec<Chip>) -> RowView {
        RowView { id: id.to_string().into(), kind: NodeKind::Commit, lane: 0, color: 0, segments: Vec::new(), summary: SharedString::default(), author: SharedString::default(), date: SharedString::default(), time: 0, sha: SharedString::default(), initials: SharedString::default(), avatar: 0, chips, worktree: None, email: String::new(), parents: parents.iter().map(|p| p.to_string()).collect(), stash_branch: None, mr_refs: Vec::new(), body: SharedString::default(), wip: None, collapsed: None, fold: None, ghosts: Vec::new() }
    }

    fn graph(rows: Vec<RowView>) -> GraphView {
        let by_sha = rows.iter().enumerate().map(|(i, r)| (r.id.to_string(), i)).collect();
        GraphView { rows, max_lanes: 1, head_row: None, head_branch: SharedString::default(), truncated: false, by_sha, head_sha: None, worktrees: Vec::new(), in_progress: Default::default(), membership: Vec::new(), pinned_refs: Vec::new(), view: None, fork: None }
    }

    fn stacked() -> GraphView {
        graph(vec![row("c", &["b"], vec![local("top")]), row("x", &["a"], vec![local("fork")]), row("b", &["a"], vec![local("mid")]), row("a", &["m"], vec![local("low")]), row("m", &[], vec![local("main")])])
    }

    #[test]
    fn a_stack_is_a_straight_path_from_the_base_and_a_fork_gives_one_per_line() {
        let s = detect_stacks(&stacked(), "refs/heads/main");
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].branches, ["low", "mid", "top"]);
        assert_eq!(s[0].left_behind, ["fork"]);
        assert_eq!(s[1].branches, ["low", "fork"]);
        assert_eq!(stack_for(&s, "low", Some("fork")).map(|x| x.branches.last().cloned()), Some(Some("fork".to_string())));
        assert_eq!(stack_for(&s, "low", None).map(|x| x.branches.last().cloned()), Some(Some("top".to_string())));
        assert_eq!(stack_for(&s, "main", None), None);
    }

    #[test]
    fn a_single_branch_is_no_stack_and_the_base_is_found_as_the_web_does() {
        let g = graph(vec![row("a", &["m"], vec![local("one")]), row("m", &[], vec![local("main")])]);
        assert!(detect_stacks(&g, "refs/heads/main").is_empty());
        assert_eq!(stack_base(&g, &[]).as_deref(), Some("refs/heads/main"));
        let origin = Chip { name: "main".into(), head: false, local: false, remote: true, tag: false, remotes: vec![("origin".into(), "refs/remotes/origin/main".into())] };
        let g = graph(vec![row("m", &[], vec![origin])]);
        assert_eq!(stack_base(&g, &[("origin".into(), Some("refs/remotes/origin/main".into()))]).as_deref(), Some("refs/remotes/origin/main"));
    }

    #[test]
    fn a_cut_off_stack_is_partial() {
        let g = graph(vec![row("c", &["b"], vec![local("top")]), row("b", &["gone"], vec![local("low")]), row("m", &[], vec![local("main")])]);
        let s = detect_stacks(&g, "refs/heads/main");
        assert_eq!(s.len(), 1);
        assert!(s[0].partial);
    }

    #[test]
    fn the_summary_says_who_pushed_and_where_it_stopped() {
        let steps = vec![("a".to_string(), StepResult::Pushed), ("b".to_string(), StepResult::Forced), ("c".to_string(), StepResult::Published)];
        assert_eq!(push_summary(&steps, None, &[], "origin").0, "Pushed a, b and c (force with lease: b; new on origin: c)");
        assert_eq!(push_summary(&[("a".to_string(), StepResult::UpToDate)], None, &[], "origin").0, "The stack is up to date");
        let (m, d, e) = push_summary(&steps[..1], Some(("b", false, "no")), &["c".to_string()], "origin");
        assert_eq!(m, "Pushed a; b was refused; not pushed: c");
        assert_eq!(d.as_deref(), Some("no"));
        assert!(e);
    }

    #[test]
    fn a_plan_moves_lower_members_and_drops_what_the_base_has() {
        let stack = Stack { branches: vec!["low".into(), "top".into()], base: "refs/heads/main".into(), ..Default::default() };
        let plan = json!({ "expect": {}, "rows": [{ "oid": "b", "upstream": false }, { "oid": "a", "upstream": true }], "chips": [{ "branch": "low", "at": "a" }, { "branch": "other", "at": "b" }] });
        let req = stack_plan(&stack, &plan);
        assert_eq!(req["rows"][1]["action"], "drop");
        assert_eq!(req["chips"][0]["at"]["kind"], "stay", "nothing below it's replayed");
        assert_eq!(req["chips"][1]["at"]["kind"], "stay", "not a member");
        let (label, caption, warn) = rebase_confirm(&stack, &req, 1);
        assert_eq!(label, "Click again to rebase 1 branch onto main (conflicts at 1 commit)");
        assert!(warn);
        assert!(caption.unwrap().contains("low is already in main"));
    }
}
