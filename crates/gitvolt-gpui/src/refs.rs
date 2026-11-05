//! A branch's, tag's and commit's writes (`ui/src/branches/*`, `ui/src/tags/*`, `ui/src/integrate/*`,
//! `ui/src/pick/*`): checkout, create, rename, delete, set upstream, reset, merge, rebase, fast-forward,
//! cherry-pick, revert, and tags. The menus' rows call these; `BranchDrag` drops a chip on a chip.

use crate::app::{GitVolt, Tab};
use crate::model::{Chip, GraphView};
use crate::ui::arm::{self, ArmOption, ArmRequest, Origin, Tone};
use crate::ui::picker::PickItem;
use crate::ui::menu::{self, MenuRow, Variant};
use crate::ui::modal::{self, PromptRequest};
use crate::ui::popover::{self, Choice, ChoiceRequest};
use crate::ui::toast::{self, Options};
use crate::write::{Write, WriteCtx, call};
use gitvolt_core::error::{ErrorDetail, GbError};
use gpui::*;
use serde_json::{Value, json};
use std::rc::Rc;

/// git's branch-name rules, word for word the core's.
pub fn branch_name_error(name: &str) -> Option<String> {
    let err = |s: &str| Some(s.to_string());
    if name.is_empty() {
        return err("Enter a branch name");
    }
    if name == "HEAD" {
        return err("HEAD isn't a branch name");
    }
    if name.starts_with('-') {
        return err("A branch name can't start with -");
    }
    if name.chars().any(|c| (c as u32) <= 0x20 || c as u32 == 0x7f || "~^:?*[\\".contains(c)) {
        return err("A branch name can't contain spaces or ~ ^ : ? * [ \\");
    }
    if name.contains("..") {
        return err("A branch name can't contain ..");
    }
    if name == "@" || name.contains("@{") {
        return err("A branch name can't be @ or contain @{");
    }
    if name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        return err("A branch name can't have an empty part between slashes");
    }
    if name.ends_with('.') {
        return err("A branch name can't end with .");
    }
    if name.split('/').any(|c| c.starts_with('.') || c.ends_with(".lock")) {
        return err("No part of a branch name can start with . or end with .lock");
    }
    None
}

pub fn tag_name_error(name: &str) -> Option<String> {
    branch_name_error(name).map(|e| e.replace("branch", "tag"))
}

fn files_word(n: usize) -> String {
    format!("{n} file{}", if n == 1 { "" } else { "s" })
}

fn commits_word(n: u64) -> String {
    format!("{n} {}", if n == 1 { "commit" } else { "commits" })
}

/// Whether commit `a` is in `b`'s history, from the loaded rows; `None`: they can't tell.
pub fn is_ancestor(g: &GraphView, a: &str, b: &str) -> Option<bool> {
    if a == b {
        return Some(true);
    }
    let (ia, ib) = (*g.by_sha.get(a)?, *g.by_sha.get(b)?);
    let reach = crate::model::branch_rows(g, ib);
    if reach.contains(&ia) {
        return Some(true);
    }
    if g.truncated { None } else { Some(false) }
}

#[derive(Clone)]
pub struct IntegrateEnv {
    pub head_branch: Option<String>,
    pub head_sha: Option<String>,
    pub busy: Option<String>,
}

/// The Integrate group for a branch chip `Y` with HEAD on `X`: only what can apply. A `Y` already in `X`'s
/// history gets only the fast-forward; any other only the merge and the rebase; a `Y` checked out in a
/// worktree can't be fast-forwarded. When the rows can't tell, all show and the core checks after the click.
pub fn integrate_rows(sha: &str, y: &Chip, checked_out: bool, g: &GraphView, env: &IntegrateEnv, this: &WeakEntity<GitVolt>, ctx: &WriteCtx) -> Vec<MenuRow> {
    let x = env.head_branch.clone().unwrap_or_else(|| "HEAD".into());
    if y.local && y.name.as_ref() == x {
        return Vec::new();
    }
    let local = y.local.then(|| y.name.to_string());
    let target = local.clone().or_else(|| y.remotes.first().map(|r| r.1.trim_start_matches("refs/remotes/").to_string())).unwrap_or_else(|| y.name.to_string());
    let head = env.head_sha.as_deref();
    let in_x = head.and_then(|h| is_ancestor(g, sha, h));
    let mut rows = Vec::new();
    let go = |kind: &'static str, ff: bool| {
        let (weak, c, t, xx) = (this.clone(), ctx.clone(), target.clone(), x.clone());
        move |_: &mut Window, cx: &mut App| {
            let (c, t, xx) = (c.clone(), t.clone(), xx.clone());
            let origin = Origin::current(cx);
            weak.update(cx, |v, cx| v.start_integrate(c, kind, t, xx, ff, origin, cx)).ok();
        }
    };
    if let Some(l) = &local
        && in_x != Some(false)
        && head != Some(sha)
        && !checked_out
    {
        let (weak, c, name, to) = (this.clone(), ctx.clone(), l.clone(), x.clone());
        rows.push(
            MenuRow::action("integrate.ff", format!("Fast-forward {l} to {x}"), "fast-forward", format!("Move {l} up to {x}"), move |_, cx| {
                let (c, name, to) = (c.clone(), name.clone(), to.clone());
                weak.update(cx, |v, cx| v.fast_forward(c, name, to, cx)).ok();
            })
            .disabled_if(env.busy.clone()),
        );
    }
    if in_x == Some(true) {
        return rows;
    }
    let ff = head.is_some_and(|h| h != sha && is_ancestor(g, h, sha) == Some(true));
    rows.push(if ff {
        MenuRow::action("integrate.merge", format!("Fast-forward {x} to {target}"), "git-merge", format!("Moves {x} up to {target}; no merge commit is made"), go("merge", true)).disabled_if(env.busy.clone())
    } else {
        MenuRow::action("integrate.merge", format!("Merge {target} into {x}"), "git-merge", format!("git merge {target} on {x}"), go("merge", false)).disabled_if(env.busy.clone())
    });
    rows.push(MenuRow::action("integrate.rebase", format!("Rebase {x} onto {target}"), "git-pull-request-arrow", format!("git rebase {target} on {x}"), go("rebase", false)).disabled_if(env.busy.clone()));
    rows
}

impl Tab {
    /// "merge" or "rebase" in the tab's worktree, for "Finish or abort the ..".
    pub fn in_progress_name(&self) -> Option<&'static str> {
        let g = self.graph.as_ref()?;
        Some(match g.in_progress.get(&self.worktree)? {
            gitvolt_graph::InProgress::Merge { .. } => "merge",
            gitvolt_graph::InProgress::Rebase { .. } => "rebase",
            gitvolt_graph::InProgress::CherryPick { .. } => "cherry-pick",
            gitvolt_graph::InProgress::Revert { .. } => "revert",
            gitvolt_graph::InProgress::Other { .. } => "operation",
        })
    }

    pub fn busy_reason(&self) -> Option<String> {
        self.in_progress_name().map(|n| format!("Finish or abort the {n} first"))
    }
}

impl GitVolt {
    fn head_move(&mut self, repo: u32, branch: Option<Option<String>>, sha: Option<String>) -> u64 {
        self.next_move += 1;
        let id = self.next_move;
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == repo) {
            t.head_moves.push((id, branch, sha));
        }
        id
    }

    fn end_move(&mut self, repo: u32, id: u64) {
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == repo) {
            t.head_moves.retain(|m| m.0 != id);
        }
    }

    /// One checkout; the core resolves the case when it runs. Diverged asks Reset or Cancel. `then`: HEAD
    /// landed.
    #[allow(clippy::too_many_arguments)]
    pub fn checkout(&mut self, ctx: WriteCtx, target: Value, expect: Value, on_diverged: Option<&'static str>, origin: Origin, then: Option<crate::write::After>, cx: &mut Context<Self>) {
        let repo = ctx.repo;
        let ref_of = match target["kind"].as_str() {
            Some("branch") => Some(format!("refs/heads/{}", target["name"].as_str().unwrap_or_default())),
            Some("remote") => Some(format!("refs/remotes/{}/{}", target["remote"].as_str().unwrap_or_default(), target["branch"].as_str().unwrap_or_default())),
            _ => None,
        };
        let refs: Vec<String> = ref_of.clone().into_iter().collect();
        self.start_pending(repo, &refs, "checkout");
        let branch = match target["kind"].as_str() {
            Some("detached") => Some(None),
            Some("branch") => Some(target["name"].as_str().map(str::to_string)),
            _ => Some(target["local"].as_str().or(target["branch"].as_str()).map(str::to_string)),
        };
        let sha = ref_of.as_ref().and_then(|r| expect["refs"][r].as_str().map(str::to_string)).or_else(|| target["oid"].as_str().map(str::to_string));
        let mv = self.head_move(repo, branch, sha);
        let (worktree, t, e) = (ctx.worktree.clone(), target.clone(), expect.clone());
        let (d_ctx, d_target, d_expect, d_origin, d_then) = (ctx.clone(), target, expect, origin.clone(), then.clone());
        let w = Write::new(&ctx, move |asked| call("checkout", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "target": t, "onDiverged": on_diverged, "expect": e, "confirmAutostash": asked.autostash })))
            .origin(origin)
            .handle(|this, err, cx| this.offer_worktree(err, cx))
            .on_success(move |this, out, cx| {
                if out["status"] == "diverged" {
                    let (ctx, target, expect, origin, then) = (d_ctx.clone(), d_target.clone(), d_expect.clone(), d_origin.clone(), d_then.clone());
                    this.ask_diverged(ctx, target, expect, out, origin, then, cx);
                } else if let Some(then) = &d_then {
                    then(this, cx);
                }
            })
            .on_end(move |this, _, cx| {
                this.end_move(repo, mv);
                this.end_pending(repo, &refs, "checkout");
                cx.notify();
            });
        self.write(w, cx);
    }

    /// "feature/x is checked out in ../shop-feature-x": said, with no way to switch yet.
    fn offer_worktree(&mut self, err: &GbError, cx: &mut Context<Self>) -> bool {
        let Some(ErrorDetail::CheckedOutElsewhere { branch, .. }) = &err.detail else { return false };
        let path = self.tabs.get(self.active).and_then(|t| t.graph.as_ref()).and_then(|g| g.worktrees.iter().find(|w| w.branch.as_deref() == Some(format!("refs/heads/{branch}").as_str())).map(|w| w.path.clone()));
        let Some(path) = path else { return false };
        let link = |label: &'static str, this: WeakEntity<GitVolt>, path: String| toast::ToastAction {
            label: label.into(),
            run: Rc::new(move |_, cx| {
                let path = path.clone();
                this.update(cx, |v, cx| if label == "Switch to it" { v.switch_worktree(path, cx) } else { v.open_worktree_tab(path, cx) }).ok();
            }),
        };
        let this = cx.weak_entity();
        toast::show_with(err.message.clone(), Options { error: true, actions: vec![link("Switch to it", this.clone(), path.clone()), link("Open in a new tab", this, path)], ..Default::default() }, cx);
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn ask_diverged(&mut self, ctx: WriteCtx, target: Value, expect: Value, out: &Value, origin: Origin, then: Option<crate::write::After>, cx: &mut Context<Self>) {
        let (local, remote, ahead, behind) = (out["local"].as_str().unwrap_or_default().to_string(), out["remote"].as_str().unwrap_or_default().to_string(), out["ahead"].as_u64().unwrap_or(0), out["behind"].as_u64().unwrap_or(0));
        let body = format!("{local} and {remote} have diverged ({ahead} ahead, {behind} behind).");
        let arm_label = if ahead > 0 { format!("Click again to reset (drops {ahead} local {})", if ahead == 1 { "commit" } else { "commits" }) } else { "Click again to reset".to_string() };
        let mut req = ArmRequest::danger(arm_label, "Branches have diverged", "Reset to remote");
        req.body = body.clone().into();
        req.caption = Some(body.into());
        let refs = json!({ format!("refs/heads/{local}"): out["localOid"], format!("refs/remotes/{remote}"): out["remoteOid"] });
        let head = expect["head"].clone();
        let this = cx.weak_entity();
        let again = origin.clone();
        self.with_window(cx, move |window, cx| {
            arm::request(
                req,
                origin,
                move |a, _, cx| {
                    if a.ok {
                        let (ctx, target, refs, head, origin, then) = (ctx.clone(), target.clone(), refs.clone(), head.clone(), again.clone(), then.clone());
                        this.update(cx, |v, cx| v.checkout(ctx, target, json!({ "head": head, "refs": refs }), Some("reset"), origin, then, cx)).ok();
                    }
                },
                window,
                cx,
            );
        });
    }

    /// A chip's double-click: its local branch, else its remote one. Tags aren't branches.
    pub fn checkout_chip(&mut self, sha: &str, chip: &Chip, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        if chip.tag || (!chip.local && chip.remotes.is_empty()) {
            return;
        }
        let ctx = tab.write_target();
        let head = tab.head_sha();
        if chip.local {
            if tab.head_branch().as_deref() == Some(chip.name.as_ref()) {
                return;
            }
            let full = format!("refs/heads/{}", chip.name);
            self.checkout(ctx, json!({ "kind": "branch", "name": chip.name.as_ref() }), json!({ "head": head, "refs": { full: sha } }), None, Origin::None, None, cx);
        } else {
            let r = chip.remotes.iter().find(|x| x.0 == "origin").or(chip.remotes.first());
            let Some((remote, full)) = r else { return };
            let branch = full.trim_start_matches(&format!("refs/remotes/{remote}/")).to_string();
            self.checkout(ctx, json!({ "kind": "remote", "remote": remote, "branch": branch }), json!({ "head": head, "refs": { full.clone(): sha } }), None, Origin::None, None, cx);
        }
    }

    /// The branch picker and the sidebar: a local branch by name.
    pub fn checkout_local(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(b) = tab.local_branch(name) else { return };
        if tab.head_branch().as_deref() == Some(name) {
            return;
        }
        let (ctx, head) = (tab.write_target(), tab.head_sha());
        self.checkout(ctx, json!({ "kind": "branch", "name": name }), json!({ "head": head, "refs": { b.full_name.clone(): b.target } }), None, Origin::None, None, cx);
    }

    /// A remote branch: tracking it locally.
    pub fn checkout_remote(&mut self, remote: &str, branch: &str, target: &str, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let (ctx, head) = (tab.write_target(), tab.head_sha());
        let full = format!("refs/remotes/{remote}/{branch}");
        self.checkout(ctx, json!({ "kind": "remote", "remote": remote, "branch": branch }), json!({ "head": head, "refs": { full: target } }), None, Origin::None, None, cx);
    }

    /// Create: a dialog with Check out on.
    /// Create branch here and the toolbar's Branch: the inline input in the commit's row, the dialog where there's no row.
    pub fn create_branch_at(&mut self, sha: String, start_ref: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.open_branch_editor(&sha, start_ref.clone(), cx) {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else { return };
        let (ctx, locals) = (tab.write_target(), tab.locals());
        let title = match &start_ref {
            Some(r) => format!("Create a branch from {}", r.trim_start_matches("refs/heads/").trim_start_matches("refs/remotes/")),
            None => format!("Create a branch at {}", sha.chars().take(7).collect::<String>()),
        };
        let this = cx.weak_entity();
        let validate: modal::Validate = Rc::new(move |v| branch_name_error(v).or_else(|| locals.iter().any(|b| b.name == v).then(|| format!("A branch named {v} already exists"))));
        let req = PromptRequest { title: title.into(), label: "Branch name".into(), initial: String::new(), confirm_label: "Create branch".into(), validate: Some(validate), require_change: false, checkbox: Some(("Check out".into(), true)) };
        modal::prompt(
            req,
            move |answer, _, cx| {
                let Some(answer) = answer else { return };
                let (ctx, sha, start_ref) = (ctx.clone(), sha.clone(), start_ref.clone());
                this.update(cx, |v, cx| v.create_branch(ctx, sha, start_ref, answer.value, answer.checked, cx)).ok();
            },
            window,
            cx,
        );
    }

    pub fn create_branch(&mut self, ctx: WriteCtx, sha: String, start_ref: Option<String>, name: String, checkout: bool, cx: &mut Context<Self>) {
        let repo = ctx.repo;
        let mv = checkout.then(|| self.head_move(repo, Some(Some(name.clone())), Some(sha.clone())));
        let worktree = ctx.worktree.clone();
        let full = format!("refs/heads/{name}");
        let w = Write::new(&ctx, move |asked| call("createBranch", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "name": name, "start": sha, "startRef": start_ref, "checkout": checkout, "expect": { "head": null, "refs": { full.clone(): null } }, "confirmAutostash": asked.autostash }))).on_end(move |this, _, cx| {
            if let Some(mv) = mv {
                this.end_move(repo, mv);
            }
            cx.notify();
        });
        self.write(w, cx);
    }

    pub fn rename_branch(&mut self, name: String, oid: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let (ctx, locals) = (tab.write_target(), tab.locals());
        let from = name.clone();
        let validate: modal::Validate = Rc::new(move |v| if v == from { None } else { branch_name_error(v).or_else(|| locals.iter().any(|b| b.name == v).then(|| format!("A branch named {v} already exists"))) });
        let req = PromptRequest { title: format!("Rename {name}").into(), label: "New name".into(), initial: name.clone(), confirm_label: "Rename".into(), validate: Some(validate), require_change: true, checkbox: None };
        let this = cx.weak_entity();
        modal::prompt(
            req,
            move |answer, _, cx| {
                let Some(answer) = answer else { return };
                let (repo, worktree, from, to, oid, ctx) = (ctx.repo, ctx.worktree.clone(), name.clone(), answer.value, oid.clone(), ctx.clone());
                this.update(cx, |v, cx| {
                    let (a, b) = (format!("refs/heads/{from}"), format!("refs/heads/{to}"));
                    v.write(Write::new(&ctx, move |_| call("renameBranch", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "from": from, "to": to, "expect": { "head": null, "refs": { a.clone(): oid, b.clone(): null } } }))), cx);
                })
                .ok();
            },
            window,
            cx,
        );
    }

    /// Set upstream, to a remote branch or none.
    pub fn set_upstream(&mut self, ctx: WriteCtx, branch: String, upstream: Option<(String, String)>, cx: &mut Context<Self>) {
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let w = Write::new(&ctx, move |_| {
            let up = upstream.as_ref().map(|(r, b)| json!({ "remote": r, "branch": b }));
            call("setUpstream", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "branch": branch, "upstream": up }))
        });
        self.write(w, cx);
    }

    /// The picker: a remote branch, None, or one the next push creates. A menu here, not a searchable list.
    /// Set upstream, and Push's Change upstream: a remote branch, or None, in the searchable picker where the menu was.
    pub fn pick_upstream(&mut self, ctx: WriteCtx, branch: String, cx: &mut Context<Self>) {
        const NONE: &str = "\u{0}none";
        const NEW: &str = "\u{0}new";
        let Some(tab) = self.tabs.iter().find(|t| t.id == ctx.repo) else { return };
        let current = tab.local_branch(&branch).and_then(|b| b.upstream);
        let groups = tab.remote_groups();
        let remotes: Vec<String> = groups.iter().map(|g| g.0.clone()).collect();
        let mut listed: Vec<(PickItem, bool)> = groups
            .iter()
            .flat_map(|(g, b)| {
                let current = current.clone();
                let branch = branch.clone();
                b.iter().map(move |x| {
                    let label = format!("{g}/{}", x.0);
                    (PickItem { id: format!("{g}\u{0}{}", x.0).into(), label: label.clone().into(), icon: Some("git-branch"), current: current.as_deref() == Some(format!("refs/remotes/{g}/{}", x.0).as_str()), tooltip: Some(format!("Track {label}").into()), ..Default::default() }, x.0 == branch)
                })
            })
            .collect();
        listed.sort_by_key(|i| !i.1);
        let mut items = vec![
            PickItem { id: NONE.into(), label: "None".into(), detail: Some("Track nothing".into()), icon: Some("x"), current: current.is_none(), tooltip: Some(format!("{branch} tracks no remote branch").into()) },
            PickItem { id: NEW.into(), label: "A branch the next push creates…".into(), detail: Some("Type its name".into()), icon: Some("git-branch"), current: false, tooltip: Some("Name a remote branch that doesn't exist yet".into()) },
        ];
        items.extend(listed.into_iter().map(|i| i.0));
        let this = cx.weak_entity();
        let placeholder = format!("Upstream of {branch}");
        self.with_window(cx, move |window, cx| {
            // Where the menu it was chained off is: it takes that menu's place.
            let at = crate::ui::menu::last_origin(cx);
            let anchor = Bounds::new(point(at.x, at.y - px(4.)), size(px(0.), px(0.)));
            let on_pick = move |item: &PickItem, window: &mut Window, cx: &mut App| {
                let (ctx, branch, remotes) = (ctx.clone(), branch.clone(), remotes.clone());
                match item.id.as_ref() {
                    NEW => {
                        this.update(cx, |v, cx| v.ask_new_upstream(ctx, branch, remotes, window, cx)).ok();
                    }
                    NONE => {
                        this.update(cx, |v, cx| v.set_upstream(ctx, branch, None, cx)).ok();
                    }
                    id => {
                        let Some((remote, name)) = id.split_once('\u{0}') else { return };
                        let up = Some((remote.to_string(), name.to_string()));
                        this.update(cx, |v, cx| v.set_upstream(ctx, branch, up, cx)).ok();
                    }
                }
            };
            crate::ui::picker::open(crate::ui::picker::PickRequest { key: None, anchor, items, placeholder: placeholder.into(), toggle: None, on_pick: Rc::new(on_pick) }, window, cx);
        });
    }

    fn ask_new_upstream(&mut self, ctx: WriteCtx, branch: String, remotes: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let split = {
            let remotes = remotes.clone();
            move |v: &str| -> Option<(String, String)> {
                let mut sorted = remotes.clone();
                sorted.sort_by_key(|r| std::cmp::Reverse(r.len()));
                let r = sorted.into_iter().find(|r| v.starts_with(&format!("{r}/")) && v.len() > r.len() + 1)?;
                Some((r.clone(), v[r.len() + 1..].to_string()))
            }
        };
        let check = split.clone();
        let validate: modal::Validate = Rc::new(move |v| match check(v) {
            Some((_, b)) => branch_name_error(&b),
            None => Some("Start with a remote name, e.g. origin/".to_string()),
        });
        let req = PromptRequest { title: format!("Track a new remote branch with {branch}").into(), label: "Remote branch".into(), initial: format!("{}/{branch}", remotes.first().map_or("origin", String::as_str)), confirm_label: "Set upstream".into(), validate: Some(validate), require_change: false, checkbox: None };
        let this = cx.weak_entity();
        modal::prompt(
            req,
            move |answer, _, cx| {
                let Some(up) = answer.and_then(|a| split(&a.value)) else { return };
                let (ctx, branch) = (ctx.clone(), branch.clone());
                this.update(cx, |v, cx| v.set_upstream(ctx, branch, Some(up), cx)).ok();
            },
            window,
            cx,
        );
    }

    /// Soft and mixed never confirm; hard asks when there's something to discard (the core decides).
    pub fn reset_to(&mut self, sha: String, mode: &'static str, origin: Origin, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let (ctx, head) = (tab.write_target(), tab.head_sha());
        let repo = ctx.repo;
        let mv = self.head_move(repo, None, Some(sha.clone()));
        let worktree = ctx.worktree.clone();
        let w = Write::new(&ctx, move |asked| call("reset", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "to": sha, "mode": mode, "expect": { "head": head, "refs": {} }, "discard": asked.discard })))
            .origin(origin)
            .on_end(move |this, _, cx| {
                this.end_move(repo, mv);
                cx.notify();
            });
        self.write(w, cx);
    }

    /// The confirmation and the write. Both asks once; a local-only delete that comes back unmerged asks,
    /// then forces.
    pub fn delete_branch(&mut self, ctx: WriteCtx, plan: DeletePlan, force: bool, origin: Origin, window: &mut Window, cx: &mut Context<Self>) {
        let both = plan.local.is_some() && plan.remote.is_some();
        if let (Some(remote), false) = (&plan.remote, force) {
            let name = format!("{}/{}", remote.0, remote.1);
            let body = if both {
                let note = plan.unpushed.map_or(String::new(), |n| format!(", and {} has {} that {} on it", plan.branch, commits_word(n), if n == 1 { "isn't" } else { "aren't" }));
                format!("Deleting {name} can't be undone{note}. You can undo deleting the local branch.")
            } else {
                format!("It goes from {}: this can't be undone.", remote.0)
            };
            let arm_label = if both {
                let note = plan.unpushed.map_or(String::new(), |n| format!(" ({} not on {name})", commits_word(n)));
                format!("Click again to delete {} and {name}{note}", plan.branch)
            } else {
                format!("Click again to delete {name} (can't be undone)")
            };
            let mut req = ArmRequest::danger(arm_label, if both { format!("Delete {} and {name}?", plan.branch) } else { format!("Delete {name}?") }, "Delete");
            req.body = body.into();
            let this = cx.weak_entity();
            let again = origin.clone();
            arm::request(
                req,
                origin,
                move |a, _, cx| {
                    if a.ok {
                        let (ctx, plan, origin) = (ctx.clone(), plan.clone(), again.clone());
                        this.update(cx, |v, cx| v.send_delete(ctx, plan, both, origin, cx)).ok();
                    }
                },
                window,
                cx,
            );
            return;
        }
        self.send_delete(ctx, plan, force, origin, cx);
    }

    pub fn send_delete(&mut self, ctx: WriteCtx, plan: DeletePlan, force: bool, origin: Origin, cx: &mut Context<Self>) {
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let mut refs = Vec::new();
        if plan.local.is_some() {
            refs.push(format!("refs/heads/{}", plan.branch));
        }
        if let Some(r) = &plan.remote {
            refs.push(format!("refs/remotes/{}/{}", r.0, r.1));
        }
        self.start_pending(repo, &refs, "delete");
        // A local delete may come back unmerged: the row's menu stays open, so that question arms the row.
        let hold = if force { None } else { arm::hold_row(cx) };
        let p = plan.clone();
        let (d_ctx, d_plan, d_origin) = (ctx.clone(), plan, origin.clone());
        let w = Write::new(&ctx, move |_| {
            let remote = p.remote.as_ref().map(|r| json!({ "remote": r.0, "branch": r.1, "lease": r.2 }));
            let expect = match &p.local {
                Some(oid) => json!({ "head": null, "refs": { format!("refs/heads/{}", p.branch): oid } }),
                None => json!({ "head": null, "refs": {} }),
            };
            call("deleteBranch", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "branch": p.branch, "local": p.local.is_some(), "remote": remote, "force": force, "expect": expect }))
        })
        .origin(origin)
        .on_success(move |this, out, cx| {
            if out["status"] != "unmerged" {
                return;
            }
            let (branch, commits, into) = (out["branch"].as_str().unwrap_or_default(), out["commits"].as_u64().unwrap_or(0), out["into"].as_str().unwrap_or_default());
            let word = commits_word(commits);
            let mut req = ArmRequest::danger(format!("Click again to delete {branch}: {word} not in {into}"), format!("Delete {branch}?"), "Delete");
            req.body = format!("{branch} has {word} that {} in {into}. You can undo this.", if commits == 1 { "isn't" } else { "aren't" }).into();
            let (ctx, plan, origin) = (d_ctx.clone(), d_plan.clone(), d_origin.clone());
            let weak = cx.weak_entity();
            let again = origin.clone();
            this.with_window(cx, move |window, cx| {
                arm::request(
                    req,
                    origin,
                    move |a, _, cx| {
                        if a.ok {
                            let (ctx, plan, origin) = (ctx.clone(), plan.clone(), again.clone());
                            weak.update(cx, |v, cx| v.send_delete(ctx, plan, true, origin, cx)).ok();
                        }
                    },
                    window,
                    cx,
                );
            });
        })
        .on_end(move |this, _, cx| {
            this.end_pending(repo, &refs, "delete");
            this.with_window(cx, move |window, cx| arm::release_row(hold, window, cx));
            cx.notify();
        });
        self.write(w, cx);
    }

    /// Merge or rebase: the preview first; it asks only for predicted conflicts, a stack or a refused rebase.
    #[allow(clippy::too_many_arguments)]
    pub fn start_integrate(&mut self, ctx: WriteCtx, kind: &'static str, target: String, x: String, ff_only: bool, origin: Origin, cx: &mut Context<Self>) {
        if cfg!(test) {
            self.sent.borrow_mut().push(call("integratePreview", json!({ "kind": kind, "target": target })));
            return;
        }
        let api = self.api.clone();
        let body = call("integratePreview", json!({ "repo": crate::app::core_repo(ctx.repo), "worktree": ctx.worktree, "kind": kind, "target": target }));
        let read = self.rt.spawn(async move { api.dispatch(crate::write::request_of(body)?).await });
        let hold = arm::hold_row(cx);
        cx.spawn(async move |this, cx| {
            let preview = read.await.unwrap_or_else(|e| Err(GbError::other(e.to_string())));
            this.update(cx, |this, cx| {
                match preview {
                    Ok(p) => this.previewed(ctx, kind, target, x, ff_only, p, origin, cx),
                    Err(e) => this.toast_error(&e, Some(ctx.repo), None, None, cx),
                }
                // Armed first, then the hold goes: the row stays armed in its open menu.
                this.with_window(cx, move |window, cx| arm::release_row(hold, window, cx));
            })
            .ok();
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn previewed(&mut self, ctx: WriteCtx, kind: &'static str, target: String, x: String, ff_only: bool, p: Value, origin: Origin, cx: &mut Context<Self>) {
        let rebase = kind == "rebase";
        if !rebase && p["merged"].as_bool() == Some(true) {
            toast::show(format!("{target} is already merged into {x}"), cx);
            return;
        }
        let stacked = if rebase { p["stacked"].as_array().cloned().unwrap_or_default() } else { Vec::new() };
        let update_refs = rebase.then(|| p["updateRefsDefault"].as_bool() == Some(true));
        if let Some(refused) = p["lossyMerge"].as_str().filter(|_| rebase) {
            let req = ChoiceRequest { title: format!("Rebase {x} onto {target}?").into(), body: Some(refused.to_string().into()), note: None, choices: vec![Choice { id: "go".into(), label: "Rebase".into(), primary: true, disabled: true, ..Default::default() }] };
            self.with_window(cx, move |window, cx| popover::ask_choice(req, None, |_, _, _| {}, window, cx));
            return;
        }
        let conflicts = p["conflicts"].as_array().map_or(0, Vec::len);
        if conflicts > 0 || !stacked.is_empty() {
            let verb = if rebase { format!("Rebasing {x} onto {target}") } else { format!("Merging {target} into {x}") };
            let conflict_text = (conflicts > 0).then(|| format!("{verb} will conflict in {}.", files_word(conflicts)));
            let replayed = match p["ahead"].as_u64().filter(|_| rebase) {
                Some(n) if n > 0 => format!("{}, ", commits_word(n)),
                _ => String::new(),
            };
            let arm_label = match (&conflict_text, rebase) {
                (Some(_), true) => format!("Click again to rebase onto {target} ({replayed}conflicts in {})", files_word(conflicts)),
                (Some(_), false) => format!("Click again to merge {target} (conflicts in {})", files_word(conflicts)),
                (None, _) => format!("Click again to rebase {x} onto {target}"),
            };
            let mut req = ArmRequest::new(if conflict_text.is_some() { Tone::Warn } else { Tone::Positive }, arm_label, if rebase { format!("Rebase {x} onto {target}?") } else { format!("Merge {target} into {x}?") }, if rebase { "Rebase" } else { "Merge" });
            req.body = conflict_text.clone().unwrap_or_default().into();
            req.caption = conflict_text.map(Into::into);
            if !stacked.is_empty() {
                let n = stacked.len();
                let away: Vec<String> = stacked.iter().filter(|s| s["worktree"].is_string()).map(|s| format!("{} is checked out in {}", s["name"].as_str().unwrap_or_default(), s["worktree"].as_str().unwrap_or_default())).collect();
                req.option = Some(ArmOption { label: format!("Also move {n} stacked {}", if n == 1 { "branch" } else { "branches" }).into(), checked: p["updateRefsDefault"].as_bool() == Some(true), detail: Some(stacked.iter().filter_map(|s| s["name"].as_str()).collect::<Vec<_>>().join(", ").into()), note: (!away.is_empty()).then(|| format!("{}: git leaves it where it is.", away.join("; ")).into()) });
            }
            let this = cx.weak_entity();
            let again = origin.clone();
            let has_stack = !stacked.is_empty();
            self.with_window(cx, move |window, cx| {
                arm::request(
                    req,
                    origin,
                    move |a, _, cx| {
                        if !a.ok {
                            return;
                        }
                        let (ctx, target, x, origin) = (ctx.clone(), target.clone(), x.clone(), again.clone());
                        let refs = if has_stack { Some(a.checked) } else { update_refs };
                        this.update(cx, |v, cx| v.send_integrate(ctx, kind, target, x, ff_only, refs, origin, cx)).ok();
                    },
                    window,
                    cx,
                );
            });
            return;
        }
        self.send_integrate(ctx, kind, target, x, ff_only, update_refs, origin, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn send_integrate(&mut self, ctx: WriteCtx, kind: &'static str, target: String, x: String, ff_only: bool, update_refs: Option<bool>, origin: Origin, cx: &mut Context<Self>) {
        let (repo, worktree, t) = (ctx.repo, ctx.worktree.clone(), target.clone());
        let w = Write::new(&ctx, move |asked| {
            let mut params = json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "kind": kind, "target": t, "confirm": { "autostash": asked.autostash } });
            if let Some(u) = update_refs {
                params["updateRefs"] = json!(u);
            }
            if ff_only {
                params["ffOnly"] = json!(true);
            }
            call("integrate", params)
        })
        .origin(origin)
        .on_success(move |this, o, cx| match o["status"].as_str() {
            Some("upToDate") => toast::show(if kind == "merge" { format!("{target} is already merged into {x}") } else { format!("{x} is up to date") }, cx),
            Some("stopped") => this.toast_rebase_outcome(o, cx),
            Some("done") => {
                let text = if o["fastForward"].as_bool() == Some(true) {
                    format!("Fast-forwarded {x} to {target}")
                } else if kind == "merge" {
                    format!("Merged {target} into {x}")
                } else {
                    format!("Rebased {x} onto {target}")
                };
                toast::show(text, cx);
            }
            _ => {}
        });
        self.write(w, cx);
    }

    /// "Fast-forward Y to X".
    pub fn fast_forward(&mut self, ctx: WriteCtx, branch: String, to: String, cx: &mut Context<Self>) {
        let (repo, worktree, b, t) = (ctx.repo, ctx.worktree.clone(), branch.clone(), to.clone());
        let w = Write::new(&ctx, move |_| call("fastForward", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "branch": b, "to": t, "expect": { "head": null, "refs": {} } }))).on_success(move |_, o, cx| toast::show(if o["status"] == "upToDate" { format!("{branch} is up to date") } else { format!("Fast-forwarded {branch} to {to}") }, cx));
        self.write(w, cx);
    }

    /// Cherry-pick or revert one commit.
    /// Cherry-pick or revert `commits` (newest first) on `branch`; the autostash's question is the write's.
    pub fn start_sequence(&mut self, pick: bool, commits: Vec<crate::selection::CommitRef>, no_commit: bool, branch: String, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let (ctx, head) = (tab.write_target(), tab.head_sha());
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let method = if pick { "cherryPick" } else { "revert" };
        let oids: Vec<String> = commits.iter().map(|c| c.oid.clone()).collect();
        let count = commits.len();
        let what = if let [one] = commits.as_slice() { one.oid.chars().take(crate::model::SHORT_SHA).collect::<String>() } else { format!("{count} commits") };
        let w = Write::new(&ctx, move |asked| call(method, json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "oids": oids, "noCommit": no_commit, "expect": { "head": head, "refs": {} }, "confirm": { "autostash": asked.autostash } }))).on_success(move |this, o, cx| {
            let verb = if pick { "Cherry-picked" } else { "Reverted" };
            let committed = o["committed"].as_bool() == Some(true);
            let warn = |m: String, cx: &mut Context<GitVolt>| toast::show_with(m, Options { warning: true, ..Default::default() }, cx);
            if o["status"] == "done" {
                if committed {
                    toast::show(if pick { format!("{verb} {what} onto {branch}") } else { format!("{verb} {what}") }, cx);
                } else {
                    toast::show(format!("{verb} {what} without committing: the changes are staged"), cx);
                }
            } else {
                let at = o["at"].as_str().map_or(String::new(), |a| format!(" at {}", a.chars().take(crate::model::SHORT_SHA).collect::<String>()));
                if let (true, Some(e)) = (committed, o["error"].as_str()) {
                    let op = if pick { "cherry-pick" } else { "revert" };
                    let e = e.trim_start_matches("error: ").trim_start_matches("fatal: ").trim_end_matches('.');
                    warn(format!("The {op} couldn't commit{at}: {e}. Its changes are staged."), cx);
                } else if !committed {
                    let rest = if count > 1 { format!(": {} of {count} applied, the rest weren't", o["applied"].as_u64().unwrap_or(0)) } else { String::new() };
                    warn(format!("Stopped on conflicts in {}{at}{rest}", files_word(o["files"].as_u64().unwrap_or(0) as usize)), cx);
                }
            }
            if !committed {
                this.reveal_wip(repo, cx);
            }
        });
        self.write(w, cx);
    }

    /// Create tag here: the inline input; where there's no row, a dialog, then its message for an annotated one.
    pub fn create_tag_at(&mut self, sha: String, annotated: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.open_tag_editor(&sha, annotated, cx) {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else { return };
        let ctx = tab.write_target();
        let names: Vec<String> = tab.sidebar["tags"].as_array().map_or(Vec::new(), |t| t.iter().filter_map(|x| x["name"].as_str().map(str::to_string)).collect());
        let validate: modal::Validate = Rc::new(move |v| tag_name_error(v).or_else(|| names.iter().any(|n| n == v).then(|| format!("A tag named {v} already exists"))));
        let short: String = sha.chars().take(crate::model::SHORT_SHA).collect();
        let req = PromptRequest { title: format!("Create a tag at {short}").into(), label: "Tag name".into(), initial: String::new(), confirm_label: if annotated { "Next" } else { "Create tag" }.into(), validate: Some(validate), require_change: false, checkbox: None };
        let this = cx.weak_entity();
        modal::prompt(
            req,
            move |answer, window, cx| {
                let Some(name) = answer.map(|a| a.value) else { return };
                if !annotated {
                    let (ctx, sha) = (ctx.clone(), sha.clone());
                    this.update(cx, |v, cx| v.create_tag(ctx, sha, name, None, cx)).ok();
                    return;
                }
                let validate: modal::Validate = Rc::new(|v| v.trim().is_empty().then(|| "Enter a tag message".to_string()));
                let req = PromptRequest { title: format!("Message for {name}").into(), label: "Tag message".into(), initial: String::new(), confirm_label: "Create tag".into(), validate: Some(validate), require_change: false, checkbox: None };
                let (this, ctx, sha) = (this.clone(), ctx.clone(), sha.clone());
                modal::prompt(
                    req,
                    move |message, _, cx| {
                        let Some(message) = message.map(|m| m.value) else { return };
                        let (ctx, sha, name) = (ctx.clone(), sha.clone(), name.clone());
                        this.update(cx, |v, cx| v.create_tag(ctx, sha, name, Some(message), cx)).ok();
                    },
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub fn create_tag(&mut self, ctx: WriteCtx, sha: String, name: String, message: Option<String>, cx: &mut Context<Self>) {
        let (repo, worktree, n) = (ctx.repo, ctx.worktree.clone(), name.clone());
        let plain = message.is_none();
        let w = Write::new(&ctx, move |_| call("createTag", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "name": n, "target": sha, "message": message }))).on_success(move |_, _, cx| toast::show(if plain { format!("Created tag {name}") } else { format!("Created annotated tag {name}") }, cx));
        self.write(w, cx);
    }

    /// Local goes at once; a remote delete is a push that can't be undone, so it arms first.
    pub fn delete_tag(&mut self, tag: String, local: bool, remote: Option<String>, origin: Origin, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let ctx = tab.write_target();
        let Some(r) = remote.clone() else { return self.send_delete_tag(ctx, tag, local, None, cx) };
        let arm_label = if local { format!("Click again to delete {tag} here and from {r} (the remote delete can't be undone)") } else { format!("Click again to delete {tag} from {r} (can't be undone)") };
        let mut req = ArmRequest::danger(arm_label, if local { format!("Delete {tag} here and from {r}?") } else { format!("Delete {tag} from {r}?") }, "Delete");
        req.body = if local { format!("Deleting it from {r} can't be undone. You can undo deleting the local tag.") } else { format!("It goes from {r}: this can't be undone.") }.into();
        let this = cx.weak_entity();
        arm::request(
            req,
            origin,
            move |a, _, cx| {
                if a.ok {
                    let (ctx, tag, remote) = (ctx.clone(), tag.clone(), remote.clone());
                    this.update(cx, |v, cx| v.send_delete_tag(ctx, tag, local, remote, cx)).ok();
                }
            },
            window,
            cx,
        );
    }

    fn send_delete_tag(&mut self, ctx: WriteCtx, tag: String, local: bool, remote: Option<String>, cx: &mut Context<Self>) {
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let done = match (&remote, local) {
            (Some(r), true) => format!("Deleted {tag} here and from {r}"),
            (Some(r), false) => format!("Deleted {tag} from {r}"),
            (None, _) => format!("Deleted tag {tag}"),
        };
        let w = Write::new(&ctx, move |_| call("deleteTag", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "name": tag, "local": local, "remote": remote }))).on_success(move |_, _, cx| toast::show(done.clone(), cx));
        self.write(w, cx);
    }

    /// Push one tag, or every tag.
    pub fn push_tags(&mut self, remote: String, tag: Option<String>, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let ctx = tab.write_target();
        let (repo, worktree, r, t) = (ctx.repo, ctx.worktree.clone(), remote.clone(), tag.clone());
        let w = Write::new(&ctx, move |_| call("pushTags", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "remote": r, "tag": t }))).on_success(move |this, o, cx| {
            let done = match (o["upToDate"].as_bool() == Some(true), &tag) {
                (true, Some(t)) => format!("{remote} already has {t}"),
                (true, None) => format!("{remote} already has every tag"),
                (false, Some(t)) => format!("Pushed {t} to {remote}"),
                (false, None) => format!("Pushed all tags to {remote}"),
            };
            let problem = format!("{done}; the server reported a problem");
            this.show_server_result(&done, &problem, &o["server"], o["op"].as_u64().unwrap_or(0), Vec::new(), cx);
        });
        self.write(w, cx);
    }

    /// Restore a file from a commit: it asks over the file's own changes (the core decides).
    pub fn restore_file(&mut self, sha: String, path: String, origin: Origin, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let ctx = tab.write_target();
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let w = Write::new(&ctx, move |asked| call("restoreFile", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "sha": sha, "path": path, "confirm": asked.discard }))).origin(origin);
        self.write(w, cx);
    }

    /// The menu of a local branch chip dropped on another: Merge A into B, Rebase B onto A; when B isn't
    /// checked out, each checks it out first.
    pub fn branch_drop_rows(&self, from_row: usize, from: &Chip, onto_row: usize, onto: &Chip, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let Some(g) = tab.graph.as_ref() else { return Vec::new() };
        if !from.local || !onto.local || from.name == onto.name {
            return Vec::new();
        }
        let (Some(a_row), Some(b_row)) = (g.rows.get(from_row), g.rows.get(onto_row)) else { return Vec::new() };
        let (a, b) = (from.name.to_string(), onto.name.to_string());
        let (a_sha, b_sha) = (a_row.id.to_string(), b_row.id.to_string());
        let here = tab.head_branch().as_deref() == Some(b.as_str());
        let busy = tab.busy_reason();
        let ctx = tab.write_target();
        let local = tab.local_branch(&b);
        let away = local.as_ref().and_then(|l| l.checked_out.clone()).filter(|w| *w != tab.worktree);
        let env = IntegrateEnv { head_branch: Some(b.clone()), head_sha: Some(b_sha.clone()), busy: busy.clone() };
        let mut rows = integrate_rows(&a_sha, from, false, g, &env, this, &ctx);
        rows.retain(|r| !matches!(r, MenuRow::Action(x) if x.id.as_ref() == "integrate.ff"));
        if rows.is_empty() && is_ancestor(g, &a_sha, &b_sha) == Some(true) {
            rows.push(MenuRow::action("integrate.merge", format!("Merge {a} into {b}"), "git-merge", format!("git merge {a} on {b}"), |_, _| {}).disabled(busy.clone().unwrap_or_else(|| format!("{b} already contains {a}"))));
        }
        rows.extend(self.rebase_onto_rows(&a_sha, from, Some((b.clone(), b_sha.clone())), this));
        if here {
            return rows;
        }
        let where_ = away.filter(|_| busy.is_none()).map(|w| format!("{b} is checked out in {w}"));
        rows.into_iter()
            .map(|r| {
                let MenuRow::Action(mut a) = r else { return r };
                if let Some(why) = &where_ {
                    a.disabled = Some(why.clone().into());
                    return MenuRow::Action(a);
                }
                if a.disabled.is_some() {
                    return MenuRow::Action(a);
                }
                a.tooltip = format!("{}. Checks out {b} first.", a.tooltip).into();
                let run = a.run.clone();
                let (weak, ctx, name, tip, head) = (this.clone(), ctx.clone(), b.clone(), b_sha.clone(), tab.head_sha());
                a.run = Rc::new(move |_, cx| {
                    let (run, ctx, name, tip, head) = (run.clone(), ctx.clone(), name.clone(), tip.clone(), head.clone());
                    weak.update(cx, |v, cx| {
                        let full = format!("refs/heads/{name}");
                        let then: crate::write::After = Rc::new(move |this, cx| {
                            let run = run.clone();
                            this.with_window(cx, move |window, cx| run(window, cx));
                        });
                        v.checkout(ctx, json!({ "kind": "branch", "name": name }), json!({ "head": head, "refs": { full: tip } }), None, Origin::None, Some(then), cx);
                    })
                    .ok();
                });
                MenuRow::Action(a)
            })
            .collect()
    }
}

/// What a Delete takes: the local branch's oid, and `(remote, branch, lease)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletePlan {
    pub branch: String,
    pub local: Option<String>,
    pub remote: Option<(String, String, String)>,
    pub unpushed: Option<u64>,
}

/// What a clicked label stands for, from the loaded sidebar.
pub fn delete_plan(tab: &Tab, chip: &Chip) -> (DeletePlan, Option<String>, Option<String>) {
    let local = chip.local.then(|| tab.local_branch(&chip.name)).flatten();
    let groups = tab.remote_groups();
    let remote_of = |full: &str| groups.iter().find_map(|(g, b)| b.iter().find(|x| x.1 == full).map(|x| (g.clone(), x.0.clone(), x.2.clone())));
    let found = match &local {
        Some(l) => l.upstream.as_deref().filter(|_| !l.gone).and_then(remote_of).or_else(|| remote_of(&format!("refs/remotes/origin/{}", l.name))),
        None => chip.remotes.first().and_then(|r| remote_of(&r.1)).or_else(|| remote_of(&format!("refs/remotes/origin/{}", chip.name))),
    };
    let unpushed = local.as_ref().filter(|l| found.as_ref().is_some_and(|f| l.upstream.as_deref() == Some(format!("refs/remotes/{}/{}", f.0, f.1).as_str())) && !l.gone && l.ahead > 0).map(|l| u64::from(l.ahead));
    let n = local.as_ref().map_or(chip.name.to_string(), |l| l.name.clone());
    let local_why = match &local {
        None => Some("No local branch".to_string()),
        Some(l) => match &l.checked_out {
            Some(w) if *w == tab.worktree => Some(format!("Can't delete {n}: it's checked out")),
            Some(w) => Some(format!("Can't delete {n}: it's checked out in {w}")),
            None => None,
        },
    };
    let remote_why = found.is_none().then(|| "No remote branch".to_string());
    (DeletePlan { branch: n, local: local.map(|l| l.target), remote: found, unpushed }, local_why, remote_why)
}

/// `Delete | Local | Remote | Both |`: the clicked control decides.
pub fn delete_row(tab: &Tab, chip: &Chip, this: &WeakEntity<GitVolt>) -> Option<MenuRow> {
    let (plan, local_why, remote_why) = delete_plan(tab, chip);
    if plan.local.is_none() && plan.remote.is_none() {
        return None;
    }
    let ctx = tab.write_target();
    let go = |local: bool, remote: bool| -> crate::ui::Action {
        let (weak, ctx, plan) = (this.clone(), ctx.clone(), plan.clone());
        Rc::new(move |window: &mut Window, cx: &mut App| {
            let origin = Origin::current(cx);
            let p = DeletePlan { local: plan.local.clone().filter(|_| local), remote: plan.remote.clone().filter(|_| remote), ..plan.clone() };
            let ctx = ctx.clone();
            weak.update(cx, |v, cx| v.delete_branch(ctx, p, false, origin, window, cx)).ok();
        })
    };
    let remote_name = plan.remote.as_ref().map_or(String::new(), |r| format!("{}/{}", r.0, r.1));
    let (has_local, has_remote) = (plan.local.is_some(), plan.remote.is_some());
    let can_local = has_local && local_why.is_none();
    let variant = |id: &str, label: &str, tooltip: String, run: crate::ui::Action, off: Option<String>| Variant { id: id.to_string().into(), label: Some(label.to_string().into()), icon: None, tooltip: tooltip.into(), run, disabled: off.map(Into::into) };
    let mut variants = Vec::new();
    if has_local {
        variants.push(variant("local", "Local", format!("Delete the local branch {} (you can undo this)", plan.branch), go(true, false), local_why.clone()));
    }
    if has_remote {
        variants.push(variant("remote", "Remote", format!("Delete {remote_name} from its remote (a push: can't be undone)"), go(false, true), None));
    }
    if has_local && has_remote {
        variants.push(variant("both", "Both", "Delete the remote branch, then the local one".to_string(), go(true, true), local_why.clone()));
    }
    let remote_only = !can_local && has_remote;
    let why = if remote_only || can_local { None } else { local_why.or(remote_why) };
    let run = if remote_only { go(false, true) } else { go(true, false) };
    let tooltip = if remote_only { format!("Delete {remote_name} from {}", plan.remote.as_ref().map_or("", |r| r.0.as_str())) } else { format!("Delete the local branch {}", plan.branch) };
    Some(MenuRow::action("branch.delete", "Delete", "trash-2", tooltip, move |w, cx| run(w, cx)).variants(variants, Some(if remote_only { "remote" } else { "local" })).disabled_if(why))
}

/// A chip being dragged onto another.
#[derive(Debug, Clone, PartialEq)]
pub struct BranchDrag {
    pub row: usize,
    pub chip: Chip,
    pub from: Point<Pixels>,
    pub at: Point<Pixels>,
    pub active: bool,
}

const DRAG_THRESHOLD: f32 = 4.;

impl GitVolt {
    pub fn start_branch_drag(&mut self, row: usize, chip: Chip, at: Point<Pixels>) {
        if !chip.local {
            return;
        }
        self.branch_drag = Some(BranchDrag { row, chip, from: at, at, active: false });
    }

    pub fn on_branch_drag(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(d) = self.branch_drag.as_mut() else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            self.branch_drag = None;
            cx.notify();
            return;
        }
        d.at = e.position;
        let dist = (e.position.x - d.from.x).as_f32().hypot((e.position.y - d.from.y).as_f32());
        if !d.active && dist < DRAG_THRESHOLD {
            return;
        }
        d.active = true;
        cx.notify();
    }

    /// The chip under `at`.
    fn chip_at(&self, at: Point<Pixels>, cx: &App) -> Option<(usize, Chip)> {
        let tab = self.tabs.get(self.active)?;
        let g = tab.graph.as_ref()?;
        let ui = crate::ui::Ui::get(cx);
        g.rows.iter().enumerate().find_map(|(i, r)| r.chips.iter().find(|c| ui.anchor(&format!("chip:{i}:{}", c.name)).is_some_and(|b| b.contains(&at))).map(|c| (i, c.clone())))
    }

    /// The release: over another local branch's chip, its menu.
    pub fn end_branch_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.branch_drag.take() else { return };
        cx.notify();
        if !d.active {
            return;
        }
        let Some((row, onto)) = self.chip_at(d.at, cx) else { return };
        let weak = cx.weak_entity();
        let rows = self.branch_drop_rows(d.row, &d.chip, row, &onto, &weak);
        if !rows.is_empty() {
            menu::open_context(rows, d.at, window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::testing::{Fixture, git};
    use std::prelude::v1::test;

    #[test]
    fn names_are_checked_as_git_does() {
        assert_eq!(branch_name_error("feature/x"), None);
        assert_eq!(branch_name_error("").as_deref(), Some("Enter a branch name"));
        assert_eq!(branch_name_error("a b").as_deref(), Some("A branch name can't contain spaces or ~ ^ : ? * [ \\"));
        assert_eq!(branch_name_error("a..b").as_deref(), Some("A branch name can't contain .."));
        assert_eq!(branch_name_error("x/.y").as_deref(), Some("No part of a branch name can start with . or end with .lock"));
        assert_eq!(tag_name_error("-x").as_deref(), Some("A tag name can't start with -"));
    }

    #[tokio::test]
    async fn create_checkout_rename_and_delete_a_branch() {
        let f = Fixture::new().await;
        let (repo, wt) = (f.repo, f.worktree.clone());
        let head = f.git(&["rev-parse", "HEAD"]);
        f.ok(call("createBranch", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "name": "feature/x", "start": head, "startRef": "refs/heads/main", "checkout": true, "expect": { "head": null, "refs": { "refs/heads/feature/x": null } }, "confirmAutostash": false }))).await;
        assert_eq!(f.git(&["branch", "--show-current"]), "feature/x");
        f.ok(call("checkout", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "target": { "kind": "branch", "name": "main" }, "onDiverged": null, "expect": { "head": head, "refs": { "refs/heads/main": head } }, "confirmAutostash": false }))).await;
        assert_eq!(f.git(&["branch", "--show-current"]), "main");
        f.ok(call("checkout", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "target": { "kind": "detached", "oid": head }, "onDiverged": null, "expect": { "head": head, "refs": {} }, "confirmAutostash": false }))).await;
        assert_eq!(f.git(&["branch", "--show-current"]), "");
        f.ok(call("checkout", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "target": { "kind": "branch", "name": "main" }, "onDiverged": null, "expect": { "head": head, "refs": {} }, "confirmAutostash": false }))).await;
        f.ok(call("renameBranch", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "from": "feature/x", "to": "feature/y", "expect": { "head": null, "refs": { "refs/heads/feature/x": head, "refs/heads/feature/y": null } } }))).await;
        assert!(f.git(&["branch"]).contains("feature/y"));
        // Unmerged: the core says so first, and force takes it, and undo brings it back.
        f.git(&["checkout", "-q", "feature/y"]);
        f.write("n.txt", "n\n");
        f.git(&["add", "."]);
        f.git(&["commit", "-qm", "Unmerged"]);
        f.git(&["checkout", "-q", "main"]);
        let tip = f.git(&["rev-parse", "feature/y"]);
        let del = |force: bool| call("deleteBranch", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "branch": "feature/y", "local": true, "remote": null, "force": force, "expect": { "head": null, "refs": { "refs/heads/feature/y": tip } } }));
        let asked = f.ok(del(false)).await;
        assert_eq!(asked["outcome"]["status"], "unmerged");
        assert!(f.git(&["branch"]).contains("feature/y"), "nothing ran");
        let r = f.ok(del(true)).await;
        assert!(!f.git(&["branch"]).contains("feature/y"));
        let entry = r["journal"]["undo"]["entry"].as_u64().unwrap();
        f.ok(call("undo", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "entry": entry, "confirm": null, "confirmAutostash": false, "withoutIndex": false, "confirmDiscard": false }))).await;
        assert!(f.git(&["branch"]).contains("feature/y"));
    }

    #[tokio::test]
    async fn checking_out_over_conflicting_changes_asks_about_the_autostash() {
        let f = Fixture::new().await;
        let (repo, wt) = (f.repo, f.worktree.clone());
        f.git(&["checkout", "-q", "-b", "other"]);
        f.write("a.txt", "other's\n");
        f.git(&["commit", "-qam", "Other"]);
        f.git(&["checkout", "-q", "main"]);
        f.write("a.txt", "mine\n");
        let (head, tip) = (f.git(&["rev-parse", "HEAD"]), f.git(&["rev-parse", "other"]));
        let body = |confirm: bool| call("checkout", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "target": { "kind": "branch", "name": "other" }, "onDiverged": null, "expect": { "head": head, "refs": { "refs/heads/other": tip } }, "confirmAutostash": confirm }));
        let e = f.send(body(false)).await.unwrap_err();
        let q = crate::write::question(&e, Default::default()).expect("the clean-restore question");
        assert_eq!(q.flag, crate::write::Flag::Autostash);
        f.ok(body(true)).await;
        assert_eq!(f.git(&["branch", "--show-current"]), "other");
        assert!(!f.git(&["stash", "list"]).is_empty(), "the changes are in a stash");
    }

    #[tokio::test]
    async fn reset_soft_mixed_and_hard_with_its_question() {
        let f = Fixture::new().await;
        let (repo, wt) = (f.repo, f.worktree.clone());
        let (top, base) = (f.git(&["rev-parse", "HEAD"]), f.git(&["rev-parse", "HEAD~1"]));
        let body = |to: &str, mode: &str, head: &str, discard: bool| call("reset", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "to": to, "mode": mode, "expect": { "head": head, "refs": {} }, "discard": discard }));
        f.ok(body(&base, "soft", &top, false)).await;
        assert!(f.git(&["status", "--porcelain"]).starts_with('M'), "soft keeps it staged");
        f.ok(body(&top, "mixed", &base, false)).await;
        assert_eq!(f.git(&["rev-parse", "HEAD"]), top);
        f.write("a.txt", "dirty\n");
        let e = f.send(body(&base, "hard", &top, false)).await.unwrap_err();
        let q = crate::write::question(&e, Default::default()).expect("the reset question");
        assert_eq!(q.req.tone, Tone::Danger);
        assert!(q.req.arm.contains("discard changes"), "{}", q.req.arm);
        f.ok(body(&base, "hard", &top, true)).await;
        assert_eq!(f.git(&["rev-parse", "HEAD"]), base);
        assert_eq!(f.git(&["status", "--porcelain"]), "");
    }

    #[tokio::test]
    async fn merge_rebase_fast_forward_and_the_preview() {
        let f = Fixture::new().await;
        let (repo, wt) = (f.repo, f.worktree.clone());
        f.git(&["checkout", "-q", "-b", "side"]);
        f.write("s.txt", "s\n");
        f.git(&["add", "."]);
        f.git(&["commit", "-qm", "Side"]);
        f.git(&["checkout", "-q", "main"]);
        // Behind: Fast-forward main to side.
        let p = f.ok(call("integratePreview", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "kind": "merge", "target": "side" }))).await;
        assert_eq!(p["conflicts"].as_array().map(Vec::len), Some(0));
        let body = |kind: &str, target: &str, ff: bool| {
            let mut params = json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "kind": kind, "target": target, "confirm": { "autostash": false } });
            if ff {
                params["ffOnly"] = json!(true);
            }
            call("integrate", params)
        };
        let r = f.ok(body("merge", "side", true)).await;
        assert_eq!(r["outcome"]["status"], "done");
        assert_eq!(f.git(&["log", "-1", "--format=%s"]), "Side");
        // Merge makes a merge commit once they've diverged.
        f.git(&["checkout", "-q", "-b", "two", "HEAD~1"]);
        f.write("t.txt", "t\n");
        f.git(&["add", "."]);
        f.git(&["commit", "-qm", "Two"]);
        f.git(&["checkout", "-q", "main"]);
        f.ok(body("merge", "two", false)).await;
        assert_eq!(f.git(&["log", "-1", "--format=%p"]).split(' ').count(), 2);
        // Rebase replays.
        f.git(&["checkout", "-q", "-b", "three", "HEAD~2"]);
        f.write("u.txt", "u\n");
        f.git(&["add", "."]);
        f.git(&["commit", "-qm", "Three"]);
        f.ok(body("rebase", "main", false)).await;
        assert_eq!(f.git(&["log", "-1", "--format=%s"]), "Three");
        assert_eq!(f.git(&["merge-base", "--is-ancestor", "main", "HEAD"]), "");
        // Fast-forward another branch in place.
        f.git(&["checkout", "-q", "main"]);
        f.git(&["branch", "behind", "HEAD~1"]);
        let main = f.git(&["rev-parse", "HEAD"]);
        f.ok(call("fastForward", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "branch": "behind", "to": "main", "expect": { "head": null, "refs": {} } }))).await;
        assert_eq!(f.git(&["rev-parse", "behind"]), main);
    }

    #[tokio::test]
    async fn a_merge_that_conflicts_stops_and_abort_puts_it_back() {
        let f = Fixture::new().await;
        let (repo, wt) = (f.repo, f.worktree.clone());
        f.git(&["checkout", "-q", "-b", "side"]);
        f.write("a.txt", "side\n");
        f.git(&["commit", "-qam", "Side"]);
        f.git(&["checkout", "-q", "main"]);
        f.write("a.txt", "main\n");
        f.git(&["commit", "-qam", "Main"]);
        let head = f.git(&["rev-parse", "HEAD"]);
        let p = f.ok(call("integratePreview", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "kind": "merge", "target": "side" }))).await;
        assert_eq!(p["conflicts"].as_array().map(Vec::len), Some(1));
        let r = f.ok(call("integrate", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "kind": "merge", "target": "side", "confirm": { "autostash": false } }))).await;
        assert_eq!(r["outcome"]["status"], "stopped");
        assert!(f.git(&["status", "--porcelain"]).contains("UU a.txt"));
        f.ok(call("mergeAbort", json!({ "repo": crate::app::core_repo(repo), "worktree": wt }))).await;
        assert_eq!(f.git(&["rev-parse", "HEAD"]), head);
        assert_eq!(f.git(&["status", "--porcelain"]), "");
    }

    #[tokio::test]
    async fn cherry_pick_revert_and_no_commit() {
        let f = Fixture::new().await;
        let (repo, wt) = (f.repo, f.worktree.clone());
        f.git(&["checkout", "-q", "-b", "side", "HEAD~1"]);
        f.write("s.txt", "s\n");
        f.git(&["add", "."]);
        f.git(&["commit", "-qm", "Side"]);
        let pick = f.git(&["rev-parse", "HEAD"]);
        f.git(&["checkout", "-q", "main"]);
        let head = f.git(&["rev-parse", "HEAD"]);
        let body = |method: &str, oid: &str, no_commit: bool, head: &str| call(method, json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "oids": [oid], "noCommit": no_commit, "expect": { "head": head, "refs": {} }, "confirm": { "autostash": false } }));
        let r = f.ok(body("cherryPick", &pick, false, &head)).await;
        assert_eq!(r["outcome"]["status"], "done");
        assert_eq!(f.git(&["log", "-1", "--format=%s"]), "Side");
        let head = f.git(&["rev-parse", "HEAD"]);
        f.ok(body("revert", &head, false, &head)).await;
        assert!(f.git(&["log", "-1", "--format=%s"]).starts_with("Revert"));
        let head = f.git(&["rev-parse", "HEAD"]);
        let r = f.ok(body("cherryPick", &pick, true, &head)).await;
        assert_eq!(r["outcome"]["committed"], false);
        assert_eq!(f.git(&["rev-parse", "HEAD"]), head, "without committing");
        assert!(f.git(&["status", "--porcelain"]).contains("A  s.txt"));
    }

    #[tokio::test]
    async fn tags_create_annotated_delete_and_push() {
        let f = Fixture::new().await;
        let bare = f.bare_origin();
        let (repo, wt) = (f.repo, f.worktree.clone());
        let head = f.git(&["rev-parse", "HEAD"]);
        let make = |name: &str, message: Value| call("createTag", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "name": name, "target": head, "message": message }));
        f.ok(make("v1", Value::Null)).await;
        f.ok(make("v2", json!("A message"))).await;
        assert_eq!(f.git(&["cat-file", "-t", "v1"]), "commit");
        assert_eq!(f.git(&["cat-file", "-t", "v2"]), "tag");
        f.ok(call("pushTags", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "remote": "origin", "tag": "v1" }))).await;
        assert!(git(&bare, &["tag"]).contains("v1"));
        f.ok(call("pushTags", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "remote": "origin", "tag": null }))).await;
        assert!(git(&bare, &["tag"]).contains("v2"));
        f.ok(call("deleteTag", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "name": "v1", "local": true, "remote": "origin" }))).await;
        assert!(!f.git(&["tag"]).contains("v1"));
        assert!(!git(&bare, &["tag"]).contains("v1"));
    }

    #[tokio::test]
    async fn set_upstream_and_a_remote_delete_with_its_lease() {
        let f = Fixture::new().await;
        let bare = f.bare_origin();
        let (repo, wt) = (f.repo, f.worktree.clone());
        f.git(&["branch", "x"]);
        f.git(&["push", "-q", "origin", "x"]);
        f.ok(call("setUpstream", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "branch": "x", "upstream": { "remote": "origin", "branch": "x" } }))).await;
        assert_eq!(f.git(&["rev-parse", "--abbrev-ref", "x@{u}"]), "origin/x");
        f.ok(call("setUpstream", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "branch": "x", "upstream": null }))).await;
        assert!(!f.git(&["config", "--list"]).contains("branch.x.merge"));
        let tracking = f.git(&["rev-parse", "origin/x"]);
        f.ok(call("deleteBranch", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "branch": "x", "local": false, "remote": { "remote": "origin", "branch": "x", "lease": tracking }, "force": false, "expect": { "head": null, "refs": {} } }))).await;
        assert!(!git(&bare, &["branch"]).contains('x'));
    }

    #[tokio::test]
    async fn a_file_restores_from_a_commit_and_asks_over_its_own_changes() {
        let f = Fixture::new().await;
        let (repo, wt) = (f.repo, f.worktree.clone());
        let old = f.git(&["rev-parse", "HEAD~1"]);
        f.write("b.txt", "dirty\n");
        let body = |confirm: bool| call("restoreFile", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "sha": old, "path": "b.txt", "confirm": confirm }));
        let e = f.send(body(false)).await.unwrap_err();
        assert!(crate::write::question(&e, Default::default()).is_some_and(|q| q.flag == crate::write::Flag::Discard));
        f.ok(body(true)).await;
        assert_eq!(f.read("b.txt"), "b\n");
    }
}
