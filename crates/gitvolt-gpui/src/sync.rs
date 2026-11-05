//! The toolbar's writes (`ui/src/sync/*`, `ui/src/undo/feature.ts`, `ui/src/stash/*`, `ui/src/toolbar/*`):
//! Undo and Redo with their history, Fetch or Pull in the button's mode, Push with its panel and force with
//! lease, and Stash and Pop. Pure views of what each button says, so they're tested without a window.

use crate::app::{GitVolt, Tab};
use crate::ui::arm::{self, ArmRequest, Origin};
use crate::ui::controls::{self, ButtonKind, check_row};
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::menu::{self, MenuRow, Variant};
use crate::ui::modal;
use crate::ui::popover::{self, Choice, ChoiceRequest};
use crate::ui::toast::{self, Options, ToastAction};
use crate::write::{Write, WriteCtx, call};
use gitvolt_core::error::{GbError, GbErrorKind};
use gitvolt_core::journal::JournalState;
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::rc::Rc;

/// A local branch of the sidebar's read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalBranch {
    pub name: String,
    pub full_name: String,
    pub target: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub gone: bool,
    pub is_head: bool,
    pub checked_out: Option<String>,
    pub worktree: Option<String>,
    pub push_target: Option<String>,
    pub push_behind: Option<u32>,
    pub rewritten: Option<(String, String)>,
    pub tip_time: i64,
}

impl LocalBranch {
    pub fn of(v: &Value) -> Self {
        let text = |k: &str| v[k].as_str().unwrap_or_default().to_string();
        let opt = |k: &str| v[k].as_str().map(str::to_string);
        Self {
            name: text("name"),
            full_name: text("fullName"),
            target: text("target"),
            upstream: opt("upstream"),
            ahead: v["ahead"].as_u64().unwrap_or(0) as u32,
            behind: v["behind"].as_u64().unwrap_or(0) as u32,
            gone: v["gone"].as_bool() == Some(true),
            is_head: v["isHead"].as_bool() == Some(true),
            checked_out: opt("checkedOut"),
            worktree: opt("worktree"),
            push_target: opt("pushTarget"),
            push_behind: v["pushBehind"].as_u64().map(|n| n as u32),
            rewritten: v["rewritten"]["kind"].as_str().map(|k| (k.to_string(), v["rewritten"]["remote"].as_str().unwrap_or_default().to_string())),
            tip_time: v["tipTime"].as_i64().unwrap_or(0),
        }
    }

    fn upstream_short(&self) -> Option<String> {
        self.upstream.as_ref().map(|u| u.trim_start_matches("refs/remotes/").to_string())
    }
}

/// A remote and its branches: `(name, full ref, target)`.
pub type RemoteGroup = (String, Vec<(String, String, String)>);

impl Tab {
    pub fn locals(&self) -> Vec<LocalBranch> {
        self.sidebar["locals"].as_array().map_or(Vec::new(), |l| l.iter().map(LocalBranch::of).collect())
    }

    pub fn local_branch(&self, name: &str) -> Option<LocalBranch> {
        self.sidebar["locals"].as_array()?.iter().find(|b| b["name"] == name).map(LocalBranch::of)
    }

    pub fn remote_groups(&self) -> Vec<RemoteGroup> {
        self.sidebar["remotes"].as_array().map_or(Vec::new(), |r| {
            r.iter()
                .map(|g| {
                    let branches = g["branches"].as_array().map_or(Vec::new(), |b| b.iter().map(|x| (x["name"].as_str().unwrap_or_default().to_string(), x["fullName"].as_str().unwrap_or_default().to_string(), x["target"].as_str().unwrap_or_default().to_string())).collect());
                    (g["name"].as_str().unwrap_or_default().to_string(), branches)
                })
                .collect()
        })
    }

    /// The remote a new upstream defaults to.
    pub fn default_remote(&self) -> String {
        let names: Vec<String> = self.remote_groups().into_iter().map(|g| g.0).collect();
        if names.iter().any(|n| n == "origin") { "origin".into() } else { names.into_iter().next().unwrap_or_else(|| "origin".into()) }
    }

    /// The branch the tab's worktree has checked out, once the queued writes ran.
    pub fn head_branch(&self) -> Option<String> {
        let now = self.graph.as_ref().and_then(|g| g.worktree_branch(&self.worktree)).map(|b| b.to_string());
        self.head_moves.iter().fold(now, |h, m| match &m.1 {
            Some(b) => b.clone(),
            None => h,
        })
    }

    /// HEAD's commit, once the queued writes ran.
    pub fn head_sha(&self) -> Option<String> {
        let now = self.graph.as_ref().and_then(|g| match g.worktrees.iter().find(|w| w.path == self.worktree) {
            Some(w) => w.head.clone(),
            None => g.head_sha.clone(),
        });
        self.head_moves.iter().fold(now, |h, m| m.2.clone().or(h))
    }

    pub fn write_target(&self) -> WriteCtx {
        WriteCtx { repo: self.id, worktree: self.worktree.clone() }
    }
}

/// What a toolbar button says now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonView {
    pub label: String,
    pub tooltip: String,
    pub disabled: bool,
}

/// Fetch All, or one of the Pull modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    FetchAll,
    FfOrMerge,
    FfOnly,
    Rebase,
}

impl SyncMode {
    pub fn key(self) -> &'static str {
        match self {
            SyncMode::FetchAll => "fetchAll",
            SyncMode::FfOrMerge => "pullFfOrMerge",
            SyncMode::FfOnly => "pullFfOnly",
            SyncMode::Rebase => "pullRebase",
        }
    }

    pub fn of(key: &str) -> Self {
        match key {
            "pullFfOrMerge" => SyncMode::FfOrMerge,
            "pullFfOnly" => SyncMode::FfOnly,
            "pullRebase" => SyncMode::Rebase,
            _ => SyncMode::FetchAll,
        }
    }

    pub fn pull_mode(self) -> &'static str {
        match self {
            SyncMode::FfOnly => "ffOnly",
            SyncMode::Rebase => "rebase",
            _ => "ffOrMerge",
        }
    }

    fn text(self) -> &'static str {
        match self {
            SyncMode::FfOrMerge => "fast-forward if possible",
            SyncMode::FfOnly => "fast-forward only",
            SyncMode::Rebase => "rebase",
            SyncMode::FetchAll => "",
        }
    }
}

pub fn sync_view(mode: SyncMode, b: Option<&LocalBranch>, head: Option<&str>) -> ButtonView {
    let pull = |tooltip: String, disabled| ButtonView { label: "Pull".into(), tooltip, disabled };
    if mode == SyncMode::FetchAll {
        return ButtonView { label: "Fetch".into(), tooltip: "Fetch every remote".into(), disabled: false };
    }
    let (Some(b), Some(_)) = (b, head) else { return pull("HEAD is detached".into(), true) };
    let Some(up) = b.upstream_short() else { return pull(format!("{} has no upstream; set one from Push ▾", b.name), true) };
    pull(format!("Pull {up} into {} ({})", b.name, mode.text()), false)
}

pub fn rewrote_since_push(b: &LocalBranch) -> Option<&'static str> {
    let kind = b.rewritten.as_ref().filter(|_| b.push_target.is_some() && b.push_behind.unwrap_or(0) > 0)?;
    Some(if kind.0 == "rebase" { "rebased" } else { "amended" })
}

pub fn push_label(b: &LocalBranch) -> String {
    if let Some(rewrote) = rewrote_since_push(b) {
        let n = b.push_behind.unwrap_or(0);
        return format!("Push (force with lease: {rewrote}, replaces {n} {} on {})", if n == 1 { "commit" } else { "commits" }, b.rewritten.as_ref().map_or("", |r| r.1.as_str()));
    }
    match &b.push_target {
        Some(t) => format!("Push {} to {t}", b.name),
        None => format!("Push {} to a remote and track it", b.name),
    }
}

pub fn push_view(b: Option<&LocalBranch>, head: Option<&str>, remote: &str) -> (String, bool) {
    if head.is_none() {
        return ("HEAD is detached".into(), true);
    }
    match b {
        None => ("Push the current branch".into(), true),
        Some(b) if b.push_target.is_some() => (push_label(b), false),
        Some(b) => (format!("Push {} to {remote} and track it", b.name), false),
    }
}

/// Nothing to push (the menu hides Push): it pushes to its upstream, which has every commit it has.
pub fn nothing_to_push(b: &LocalBranch) -> bool {
    b.push_target.is_some() && b.upstream_short() == b.push_target && !b.gone && b.ahead == 0 && rewrote_since_push(b).is_none()
}

pub fn force_text(b: &LocalBranch) -> String {
    let n = b.push_behind.unwrap_or(0);
    let target = b.push_target.as_deref().unwrap_or_default();
    if n == 0 {
        return format!("It may replace commits on the server that aren't in {}. A push can't be undone.", b.name);
    }
    let what = if n == 1 { format!("1 commit on {target} that isn't in {}", b.name) } else { format!("{n} commits on {target} that aren't in {}", b.name) };
    format!("It replaces {what}. A push can't be undone.")
}

pub fn force_arm(target: &str, behind: Option<u32>) -> String {
    match behind.unwrap_or(0) {
        0 => format!("Click again to force push to {target}"),
        n => format!("Click again to force push: replaces {n} {}", if n == 1 { "commit" } else { "commits" }),
    }
}

fn commits_word(n: u64) -> String {
    format!("{n} {}", if n == 1 { "commit" } else { "commits" })
}

/// Undo or Redo's button.
pub fn undo_view(s: Option<&JournalState>, undo: bool, key: &str) -> (String, bool) {
    let (top, blocked) = match s {
        Some(s) if undo => (s.undo.as_ref(), s.undo_blocked.as_ref()),
        Some(s) => (s.redo.as_ref(), s.redo_blocked.as_ref()),
        None => (None, None),
    };
    if let Some(why) = blocked {
        return (why.clone(), true);
    }
    match top {
        Some(t) => (format!("{} {} ({key})", if undo { "Undo" } else { "Redo" }, t.label), false),
        None => ((if undo { "Nothing to undo" } else { "Nothing to redo" }).to_string(), true),
    }
}

/// §5.4's prompt: what moved since, and what undoing it does.
pub fn moved_text(label: &str, refs: &[Value]) -> String {
    let is_ref = |v: Option<&str>| v.is_some_and(|s| s.starts_with("refs/"));
    let short_ref = |n: &str| n.trim_start_matches("refs/heads/").trim_start_matches("refs/remotes/").trim_start_matches("refs/tags/").to_string();
    let short = |v: Option<&str>| match v {
        None => "nothing".to_string(),
        Some(s) if s.starts_with("refs/") => short_ref(s),
        Some(s) => s.chars().take(7).collect(),
    };
    refs.iter()
        .map(|r| {
            let name = r["name"].as_str().unwrap_or_default();
            let actual = r["actual"].as_str();
            let target = r["target"].as_str();
            let where_ = match actual {
                None => "it's gone".to_string(),
                Some(_) => format!("it's {} {}", if is_ref(actual) { "on" } else { "at" }, short(actual)),
            };
            let to = if r["stays"].as_bool() == Some(true) {
                format!("leaves {} there and restores the files over it", short_ref(name))
            } else if target.is_some() {
                format!("moves it to {}", short(target))
            } else {
                "deletes it".to_string()
            };
            let dropped = r["dropped"].as_u64().unwrap_or(0);
            let drops = if dropped > 0 { format!(" and drops the {} made since", commits_word(dropped)) } else { String::new() };
            format!("{} moved since {label} ({where_}, not {}). Undoing {to}{drops}.", short_ref(name), short(r["expected"].as_str()))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A row's tooltip's "touched": "a.txt, src/b.ts and 3 more".
pub fn touched_text(touched: &[String]) -> String {
    let shown = touched.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    if touched.len() > 3 { format!("{shown} and {} more", touched.len() - 3) } else { shown }
}

fn compact_age(ms: i64) -> String {
    let s = ((chrono::Utc::now().timestamp_millis() - ms).max(0) / 1000) as u64;
    match s {
        0..=4 => "now".into(),
        5..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        3600..=86399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86400),
    }
}

impl GitVolt {
    /// The same action on the same target isn't started twice.
    pub fn run_once(&mut self, repo: u32, key: String) -> bool {
        self.tabs.iter_mut().find(|t| t.id == repo).is_some_and(|t| t.in_flight.insert(key))
    }

    pub fn end_once(&mut self, repo: u32, key: &str) {
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == repo) {
            t.in_flight.remove(key);
        }
    }

    pub fn start_pending(&mut self, repo: u32, refs: &[String], action: &'static str) {
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == repo) {
            for r in refs {
                t.pending.insert(r.clone(), action);
            }
        }
    }

    pub fn end_pending(&mut self, repo: u32, refs: &[String], action: &'static str) {
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == repo) {
            for r in refs {
                if t.pending.get(r) == Some(&action) {
                    t.pending.remove(r);
                }
            }
        }
    }

    /// A push or fetch finished: with a warning from the server, a toast that stays, quoting it.
    pub fn show_server_result(&mut self, done: &str, problem: &str, server: &Value, op: u64, extra: Vec<ToastAction>, cx: &mut Context<Self>) {
        let lines = server["lines"].as_u64().unwrap_or(0);
        let mut actions = extra;
        if lines > 0 {
            let this = cx.weak_entity();
            actions.push(ToastAction {
                label: format!("Server output ({lines} {})", if lines == 1 { "line" } else { "lines" }).into(),
                run: Rc::new(move |window, cx| {
                    this.update(cx, |v, cx| v.open_activity(Some(op), window, cx)).ok();
                }),
            });
        }
        if let Some(w) = server["warning"].as_str() {
            toast::show_with(problem.to_string(), Options { warning: true, sticky: true, detail: Some(format!("“{w}”").into()), actions, ..Default::default() }, cx);
        } else {
            toast::show_with(done.to_string(), Options { actions, ..Default::default() }, cx);
        }
    }

    // ---- Fetch ----

    /// One fetch of the tab's repo. `background`: the scheduler's, which never prompts and shows nothing
    /// while it runs or when it succeeds. A user's that finds a background one running waits on it.
    pub fn run_fetch(&mut self, repo: u32, background: bool, remote: Option<String>, cx: &mut Context<Self>) {
        let key = format!("fetch|{}{}", if background { "bg:" } else { "" }, remote.as_deref().unwrap_or(""));
        if !self.run_once(repo, key.clone()) {
            return;
        }
        self.fetch_now(repo, background, remote, key, Vec::new(), cx);
    }

    fn fetch_now(&mut self, repo: u32, background: bool, remote: Option<String>, key: String, switched: Vec<gitvolt_core::askpass::SshOffer>, cx: &mut Context<Self>) {
        let body = match &remote {
            Some(r) => call("fetch", json!({ "repo": repo, "background": background, "remote": r })),
            None => call("fetch", json!({ "repo": repo, "background": background })),
        };
        if cfg!(test) {
            self.sent.borrow_mut().push(body);
            return;
        }
        let api = self.api.clone();
        let done = self.rt.spawn(async move { api.dispatch(crate::write::request_of(body)?).await });
        cx.spawn(async move |this, cx| {
            let answer = done.await.unwrap_or_else(|e| Err(GbError::other(e.to_string())));
            this.update(cx, |this, cx| {
                this.fetched(repo, background, remote, key, switched, answer, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fetched(&mut self, repo: u32, background: bool, remote: Option<String>, key: String, mut switched: Vec<gitvolt_core::askpass::SshOffer>, answer: Result<Value, GbError>, cx: &mut Context<Self>) {
        let name = self.tabs.iter().find(|t| t.id == repo).map_or(String::new(), |t| t.name.to_string());
        let full = remote.is_none();
        let stamp = |this: &mut Self| {
            if full && let Some(t) = this.tabs.iter_mut().find(|t| t.id == repo) {
                t.last_fetch = Some(std::time::Instant::now());
            }
        };
        match answer {
            Ok(out) => {
                let (done, reason) = (out["status"] == "done", out["reason"].as_str());
                if done || reason == Some("authRequired") {
                    self.failing_fetch.remove(&repo);
                }
                if done {
                    stamp(self);
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == repo)
                        && full
                    {
                        t.fetch_skipped = None;
                    }
                    let op = out["op"].as_u64().unwrap_or(0);
                    if !background && out["server"]["lines"].as_u64().unwrap_or(0) > 0 {
                        self.show_server_result(&format!("Fetched {name}"), &format!("Fetched {name}; the server reported a problem"), &out["server"], op, Vec::new(), cx);
                    } else if background && out["server"]["warning"].is_string() {
                        self.show_server_result("", &format!("Background fetch of {name}: the server reported a problem"), &out["server"], op, Vec::new(), cx);
                    }
                } else if reason == Some("authRequired") {
                    stamp(self);
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == repo) {
                        t.fetch_skipped = Some("Fetch skipped: authentication required".into());
                    }
                } else if !background {
                    let running = self.ops.ops.values().find(|o| o.kind == gitvolt_core::events::OpKind::Fetch && o.repo == Some(repo)).map(|o| o.op);
                    match running {
                        Some(op) => self.ops.show_op(op),
                        None => toast::show("A fetch is already running", cx),
                    }
                }
                self.end_once(repo, &key);
            }
            Err(e) => {
                let ctx = self.tabs.iter().find(|t| t.id == repo).map(Tab::write_target);
                if let (Some(ssh), Some(ctx)) = (crate::write::ssh_switches(&e), ctx.clone()) {
                    switched.extend(ssh.iter().cloned());
                    let (r, k) = (remote.clone(), key.clone());
                    self.switch_to_ssh(&ctx, ssh, move |this, cx| this.fetch_now(repo, background, r.clone(), k.clone(), switched.clone(), cx), cx);
                    return;
                }
                self.end_once(repo, &key);
                stamp(self);
                if e.kind == GbErrorKind::Cancelled {
                    if let (false, Some(ctx)) = (switched.is_empty(), ctx) {
                        self.toast_after_switch(&ctx, switched, &e, Some("Fetch failed"), cx);
                    }
                    return;
                }
                let text = if e.kind == GbErrorKind::AuthFailed { format!("Authentication failed ({})", e.message) } else { e.message.clone() };
                let started = self.failing_fetch.insert(repo);
                if !background && !switched.is_empty() {
                    if let Some(ctx) = ctx {
                        self.toast_after_switch(&ctx, switched, &e, Some("Fetch failed"), cx);
                    }
                } else if !background {
                    let this = cx.weak_entity();
                    toast::show_with(
                        format!("Fetch failed: {text}"),
                        Options {
                            error: true,
                            actions: vec![ToastAction {
                                label: "Activity log".into(),
                                run: Rc::new(move |window, cx| {
                                    this.update(cx, |v, cx| v.open_activity(None, window, cx)).ok();
                                }),
                            }],
                            ..Default::default()
                        },
                        cx,
                    );
                } else if started {
                    self.ops.push_error(format!("Fetch failed ({name}): {text}"));
                }
            }
        }
    }

    /// The Fetch/Pull button: in its mode.
    pub fn sync_default(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let (repo, ctx) = (tab.id, tab.write_target());
        match self.sync_mode {
            SyncMode::FetchAll => self.run_fetch(repo, false, None, cx),
            mode => self.pull(ctx, mode.pull_mode(), None, Origin::control("tb-sync"), cx),
        }
    }

    pub fn set_sync_mode(&mut self, mode: SyncMode, cx: &mut Context<Self>) {
        self.sync_mode = mode;
        self.save_layout();
        cx.notify();
    }

    // ---- Pull ----

    /// Pull `branch` (the checked-out one by default) in `mode`.
    pub fn pull(&mut self, ctx: WriteCtx, mode: &'static str, branch: Option<String>, origin: Origin, cx: &mut Context<Self>) {
        let repo = ctx.repo;
        let Some(tab) = self.tabs.iter().find(|t| t.id == repo) else { return };
        let name = branch.clone().or_else(|| tab.head_branch());
        let key = format!("pull|{}", name.as_deref().unwrap_or(""));
        if !self.run_once(repo, key.clone()) {
            return;
        }
        let refs: Vec<String> = name.as_ref().map(|n| format!("refs/heads/{n}")).into_iter().collect();
        self.start_pending(repo, &refs, "pull");
        let (worktree, send_branch) = (ctx.worktree.clone(), branch.clone());
        let done_ctx = ctx.clone();
        let done_origin = origin.clone();
        let w = Write::new(&ctx, move |asked| {
            let mut params = json!({ "repo": repo, "worktree": worktree, "mode": mode, "confirm": { "autostash": asked.autostash } });
            if let Some(b) = &send_branch {
                params["branch"] = json!(b);
            }
            call("pull", params)
        })
        .origin(origin)
        .on_success(move |this, out, cx| this.pulled(&done_ctx, out, done_origin.clone(), cx))
        .on_end(move |this, _, cx| {
            this.end_once(repo, &key);
            this.end_pending(repo, &refs, "pull");
            cx.notify();
        });
        self.write(w, cx);
    }

    fn pulled(&mut self, ctx: &WriteCtx, o: &Value, origin: Origin, cx: &mut Context<Self>) {
        let (branch, upstream, op) = (o["branch"].as_str().unwrap_or_default().to_string(), o["upstream"].as_str().unwrap_or_default().to_string(), o["op"].as_u64().unwrap_or(0));
        let r = &o["result"];
        let server = o["server"].clone();
        let commits = |k: &str| commits_word(r[k].as_u64().unwrap_or(0));
        let text = match r["status"].as_str() {
            Some("upToDate") => match r["ahead"].as_u64().unwrap_or(0) {
                0 => format!("{branch} is up to date"),
                n => format!("{branch} is up to date ({n} ahead: push)"),
            },
            Some("fastForward") => format!("Pulled {} into {branch} (fast-forward)", commits("commits")),
            Some("merged") => format!("Pulled {} into {branch} (merge)", commits("commits")),
            Some("rebased") => format!("Pulled {} into {branch} (rebase)", commits("commits")),
            Some("stopped") => {
                if let Some(w) = r["warning"].as_str() {
                    toast::show_with(w.to_string(), Options { warning: true, ..Default::default() }, cx);
                }
                return;
            }
            Some("diverged") => {
                let head = self.tabs.iter().find(|t| t.id == ctx.repo).and_then(Tab::head_branch);
                if head.as_deref() != Some(branch.as_str()) {
                    toast::show_with(format!("{branch} and {upstream} have diverged; check out {branch} first"), Options { error: true, ..Default::default() }, cx);
                    return;
                }
                let (ahead, behind, conflicts) = (r["ahead"].as_u64().unwrap_or(0), r["behind"].as_u64().unwrap_or(0), r["conflicts"].as_u64().unwrap_or(0));
                let req = ChoiceRequest {
                    title: format!("Pull {branch}?").into(),
                    body: Some(format!("{branch} and {upstream} have diverged ({ahead} ahead, {behind} behind).").into()),
                    note: (conflicts > 0).then(|| format!("Merging would conflict in {conflicts} {}.", if conflicts == 1 { "file" } else { "files" }).into()),
                    choices: vec![Choice { id: "rebase".into(), label: "Rebase".into(), primary: true, ..Default::default() }, Choice { id: "merge".into(), label: "Merge".into(), ..Default::default() }],
                };
                let this = cx.weak_entity();
                let ctx = ctx.clone();
                let at = self.origin_rect(&origin, cx);
                self.with_window(cx, move |window, cx| {
                    popover::ask_choice(
                        req,
                        at,
                        move |pick, _, cx| {
                            let Some(kind) = pick.filter(|p| p == "rebase" || p == "merge").map(|p| p.to_string()) else { return };
                            let (ctx, upstream, branch, server) = (ctx.clone(), upstream.clone(), branch.clone(), server.clone());
                            this.update(cx, |v, cx| v.integrate_upstream(ctx, kind, upstream, branch, behind, server, op, cx)).ok();
                        },
                        window,
                        cx,
                    );
                });
                return;
            }
            _ => return,
        };
        let problem = format!("{text}; the server reported a problem");
        self.show_server_result(&text, &problem, &server, op, Vec::new(), cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn integrate_upstream(&mut self, ctx: WriteCtx, kind: String, upstream: String, branch: String, behind: u64, server: Value, op: u64, cx: &mut Context<Self>) {
        let (repo, worktree, k) = (ctx.repo, ctx.worktree.clone(), kind.clone());
        let w = Write::new(&ctx, move |asked| call("integrate", json!({ "repo": repo, "worktree": worktree, "kind": k, "target": upstream, "confirm": { "autostash": asked.autostash } }))).on_success(move |this, out, cx| {
            if out["status"] == "done" {
                let text = format!("Pulled {} into {branch} ({kind})", commits_word(behind));
                let problem = format!("{text}; the server reported a problem");
                this.show_server_result(&text, &problem, &server, op, Vec::new(), cx);
            }
        });
        self.write(w, cx);
    }

    /// What an answer's popover is placed against.
    pub fn origin_rect(&self, origin: &Origin, cx: &App) -> Option<Bounds<Pixels>> {
        match origin {
            Origin::Control { key, .. } => crate::ui::Ui::get(cx).anchor(key),
            _ => None,
        }
    }

    // ---- Push ----

    /// Push: to the branch's target; with none, asks where.
    pub fn push_branch(&mut self, ctx: WriteCtx, b: LocalBranch, origin: Origin, cx: &mut Context<Self>) {
        if b.push_target.is_none() {
            return self.open_push_upstream(ctx, b, origin, cx);
        }
        self.send_push(ctx, b, PushOpts::default(), origin, cx);
    }

    fn lease_of(&self, repo: u32, target: Option<&str>) -> Option<String> {
        let tab = self.tabs.iter().find(|t| t.id == repo)?;
        let target = target?;
        tab.remote_groups().into_iter().find_map(|(g, branches)| branches.into_iter().find(|b| format!("{g}/{}", b.0) == target).map(|b| b.2))
    }

    pub fn send_push(&mut self, ctx: WriteCtx, b: LocalBranch, opts: PushOpts, origin: Origin, cx: &mut Context<Self>) {
        let repo = ctx.repo;
        let key = format!("push|{}", b.full_name);
        if !self.run_once(repo, key.clone()) {
            return;
        }
        let refs = vec![b.full_name.clone()];
        self.start_pending(repo, &refs, "push");
        let (worktree, send_b, send_opts) = (ctx.worktree.clone(), b.clone(), opts.clone());
        let (rejected_b, rejected_ctx, rejected_opts, rejected_origin) = (b.clone(), ctx.clone(), opts.clone(), origin.clone());
        let w = Write::new(&ctx, move |_| {
            let mut params = json!({ "repo": repo, "worktree": worktree, "branch": send_b.name, "expect": { "head": null, "refs": { send_b.full_name.clone(): send_b.target } } });
            if let Some((remote, branch)) = &send_opts.target {
                params["target"] = json!({ "remote": remote, "branch": branch });
            }
            if let Some(track) = send_opts.set_upstream {
                params["setUpstream"] = json!(track);
            }
            if let Some(oid) = &send_opts.lease {
                params["lease"] = json!({ "oid": oid });
            }
            call("push", params)
        })
        .origin(origin)
        .on_success(move |this, o, cx| {
            let dst = format!("{}/{}", o["remote"].as_str().unwrap_or_default(), o["dst"].as_str().unwrap_or_default());
            let branch = o["branch"].as_str().unwrap_or_default();
            let done = match o["forced"].as_str() {
                Some(kind) => format!("Force-pushed {branch} (with lease): it was {}", if kind == "rebase" { "rebased" } else { "amended" }),
                None => format!("Pushed {branch} to {dst}"),
            };
            let shown = if o["upToDate"].as_bool() == Some(true) { format!("{dst} is up to date") } else { done.clone() };
            let problem = format!("{done}; the server reported a problem");
            this.show_server_result(&shown, &problem, &o["server"], o["op"].as_u64().unwrap_or(0), Vec::new(), cx);
        })
        .handle(move |this, err, cx| {
            if err.kind != GbErrorKind::NonFastForward {
                return false;
            }
            this.push_rejected(rejected_ctx.clone(), rejected_b.clone(), err.command_id, rejected_opts.clone(), rejected_origin.clone(), cx);
            true
        })
        .on_end(move |this, _, cx| {
            this.end_once(repo, &key);
            this.end_pending(repo, &refs, "push");
            cx.notify();
        });
        self.write(w, cx);
    }

    /// A rejected push is a real choice: Pull them in first, or overwrite them.
    fn push_rejected(&mut self, ctx: WriteCtx, b: LocalBranch, command: Option<u64>, sent: PushOpts, origin: Origin, cx: &mut Context<Self>) {
        let now = self.tabs.iter().find(|t| t.id == ctx.repo).and_then(|t| t.local_branch(&b.name)).unwrap_or_else(|| b.clone());
        let theirs = sent.target.as_ref().map(|(r, n)| format!("{r}/{n}")).or_else(|| now.push_target.clone());
        let behind = if theirs == now.push_target { now.push_behind } else { None };
        let lease = self.lease_of(ctx.repo, theirs.as_deref());
        let n = behind.unwrap_or(0);
        let target = theirs.clone().unwrap_or_else(|| b.name.clone());
        let mut choices = vec![Choice { id: "pull".into(), label: "Pull".into(), primary: true, ..Default::default() }];
        if let Some(t) = &theirs {
            choices.push(Choice { id: "force".into(), label: "Force push…".into(), danger: true, arm: Some(force_arm(t, behind).into()), ..Default::default() });
        }
        choices.push(Choice { id: "details".into(), label: "Details".into(), ..Default::default() });
        let req = ChoiceRequest { title: format!("{target} has {} {} doesn't have", if n > 0 { commits_word(u64::from(n)) } else { "commits".into() }, b.name).into(), body: Some("Pull them in first, or overwrite them.".into()), note: None, choices };
        let this = cx.weak_entity();
        let at = self.origin_rect(&origin, cx);
        self.with_window(cx, move |window, cx| {
            popover::ask_choice(
                req,
                at,
                move |pick, window, cx| {
                    let (ctx, b, sent, origin, lease) = (ctx.clone(), b.clone(), sent.clone(), origin.clone(), lease.clone());
                    this.update(cx, |v, cx| match pick.as_deref() {
                        Some("pull") => {
                            let head = v.tabs.iter().find(|t| t.id == ctx.repo).and_then(Tab::head_branch);
                            let branch = (head.as_deref() != Some(b.name.as_str())).then(|| b.name.clone());
                            v.pull(ctx, "ffOnly", branch, origin, cx);
                        }
                        Some("force") => v.send_push(ctx, b, PushOpts { lease: Some(lease), ..sent }, origin, cx),
                        Some("details") => v.open_debug(command, window, cx),
                        _ => {}
                    })
                    .ok();
                },
                window,
                cx,
            );
        });
    }

    /// Push the checked-out branch.
    pub fn push_head(&mut self, origin: Origin, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let (ctx, b) = (tab.write_target(), tab.head_branch().and_then(|h| tab.local_branch(&h)));
        if let Some(b) = b {
            self.push_branch(ctx, b, origin, cx);
        }
    }

    /// Force push with a lease on the remote-tracking oid shown now, once armed.
    pub fn force_push(&mut self, ctx: WriteCtx, b: LocalBranch, origin: Origin, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = b.push_target.clone() else { return };
        let lease = self.lease_of(ctx.repo, Some(&target));
        let mut req = ArmRequest::danger(force_arm(&target, b.push_behind), format!("Force push {} to {target}?", b.name), "Force push");
        req.body = force_text(&b).into();
        let this = cx.weak_entity();
        let sent_origin = origin.clone();
        arm::request(
            req,
            origin,
            move |a, _, cx| {
                if a.ok {
                    let (ctx, b, lease, origin) = (ctx.clone(), b.clone(), lease.clone(), sent_origin.clone());
                    this.update(cx, |v, cx| v.send_push(ctx, b, PushOpts { lease: Some(lease), ..Default::default() }, origin, cx)).ok();
                }
            },
            window,
            cx,
        );
    }

    /// "Push main to [origin ▾] / [main]", ☑ Track it.
    pub fn open_push_upstream(&mut self, ctx: WriteCtx, b: LocalBranch, origin: Origin, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == ctx.repo) else { return };
        let mut remotes: Vec<String> = tab.remote_groups().into_iter().map(|g| g.0).collect();
        if remotes.is_empty() {
            toast::show_with(format!("{} has no upstream and this repository has no remote", b.name), Options { error: true, ..Default::default() }, cx);
            return;
        }
        remotes.sort_by_key(|r| r != "origin");
        let this = cx.weak_entity();
        // Dropped from where the push started: the button, or where the menu's row was.
        let anchor = match &origin {
            Origin::Control { key, .. } => crate::ui::Ui::get(cx).anchor(key),
            _ => {
                let at = crate::ui::menu::last_origin(cx);
                Some(Bounds::new(at, size(px(0.), px(0.))))
            }
        };
        self.with_window(cx, move |window, cx| push_panel(ctx, b, remotes, anchor, this, window, cx));
    }

    // ---- Undo and redo ----

    /// Undoes `entry`; `confirm`: "Undo anyway" with the refs as shown.
    fn undo_entry(&mut self, ctx: WriteCtx, entry: u64, confirm: Option<Value>, origin: Origin, cx: &mut Context<Self>) {
        let key = "undo".to_string();
        if !self.run_once(ctx.repo, key.clone()) {
            return;
        }
        let (repo, worktree, send_confirm) = (ctx.repo, ctx.worktree.clone(), confirm);
        let (done_ctx, done_origin) = (ctx.clone(), origin.clone());
        let w = Write::new(&ctx, move |asked| call("undo", json!({ "repo": repo, "worktree": worktree, "entry": entry, "confirm": send_confirm, "confirmAutostash": asked.autostash, "withoutIndex": asked.without_index, "confirmDiscard": asked.discard })))
            .origin(origin)
            .on_success(move |this, out, cx| this.undone(done_ctx.clone(), entry, out, done_origin.clone(), cx))
            .on_end(move |this, _, _| this.end_once(repo, &key));
        self.write(w, cx);
    }

    fn undone(&mut self, ctx: WriteCtx, entry: u64, out: &Value, origin: Origin, cx: &mut Context<Self>) {
        let label = out["label"].as_str().unwrap_or_default().to_string();
        if out["status"] == "moved" {
            let refs = out["refs"].as_array().cloned().unwrap_or_default();
            let body = moved_text(&label, &refs);
            let mut req = ArmRequest::danger(format!("Click again to undo {label} anyway"), format!("Undo {label}?"), "Undo anyway");
            req.body = body.clone().into();
            req.caption = Some(body.into());
            let confirm: serde_json::Map<String, Value> = refs.iter().map(|r| (r["name"].as_str().unwrap_or_default().to_string(), r["actual"].clone())).collect();
            let this = cx.weak_entity();
            let then_origin = origin.clone();
            self.with_window(cx, move |window, cx| {
                arm::request(
                    req,
                    origin,
                    move |a, _, cx| {
                        if a.ok {
                            let (ctx, confirm, origin) = (ctx.clone(), Value::Object(confirm.clone()), then_origin.clone());
                            this.update(cx, |v, cx| v.undo_entry(ctx, entry, Some(confirm), origin, cx)).ok();
                        }
                    },
                    window,
                    cx,
                );
            });
            return;
        }
        let note = match out["note"].as_str() {
            Some(n) => format!(" ({n})"),
            None => String::new(),
        };
        let stays = if note.is_empty() && out["kind"] == "pull" { " (the fetched remote branches stay)" } else { "" };
        let this = cx.weak_entity();
        let redo_ctx = ctx;
        toast::show_with(
            format!("Undid {label}{note}{stays}"),
            Options {
                actions: vec![ToastAction {
                    label: "Redo".into(),
                    run: Rc::new(move |_, cx| {
                        let ctx = redo_ctx.clone();
                        this.update(cx, |v, cx| v.redo(ctx, Origin::None, cx)).ok();
                    }),
                }],
                ..Default::default()
            },
            cx,
        );
    }

    /// Toolbar Undo, Ctrl+Z and the palette's.
    pub fn undo(&mut self, ctx: WriteCtx, origin: Origin, cx: &mut Context<Self>) {
        let Some(top) = self.ops.journal(ctx.repo, &ctx.worktree).filter(|s| s.undo_blocked.is_none()).and_then(|s| s.undo.as_ref()).map(|t| t.entry) else { return };
        self.undo_entry(ctx, top, None, origin, cx);
    }

    pub fn redo(&mut self, ctx: WriteCtx, origin: Origin, cx: &mut Context<Self>) {
        let Some(top) = self.ops.journal(ctx.repo, &ctx.worktree).filter(|s| s.redo_blocked.is_none()).and_then(|s| s.redo.as_ref()).map(|t| t.entry) else { return };
        let key = "undo".to_string();
        if !self.run_once(ctx.repo, key.clone()) {
            return;
        }
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let w = Write::new(&ctx, move |asked| call("redo", json!({ "repo": repo, "worktree": worktree, "entry": top, "confirmAutostash": asked.autostash, "withoutIndex": asked.without_index })))
            .origin(origin)
            .on_success(|_, out, cx| {
                if out["status"] == "done" {
                    toast::show(format!("Redid {}", out["label"].as_str().unwrap_or_default()), cx);
                }
            })
            .on_end(move |this, _, _| this.end_once(repo, &key));
        self.write(w, cx);
    }

    /// Ctrl+Z: in the WIP's list or its diff, the staging log's; else the journal's.
    pub fn undo_key(&mut self, redo: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let target = tab.write_target();
        let has_diff = tab.diff.is_some();
        if let Some(ctx) = self.staging_ctx() {
            return self.staging_undo(&ctx, redo, cx);
        }
        if has_diff {
            return;
        }
        if redo { self.redo(target, Origin::None, cx) } else { self.undo(target, Origin::None, cx) }
    }

    /// The keyboard's in the WIP's file list or the diff of its row.
    pub fn staging_ctx(&self) -> Option<WriteCtx> {
        let tab = self.tabs.get(self.active)?;
        matches!(tab.keys, crate::app::KeyArea::Files | crate::app::KeyArea::Diff).then(|| self.panel_ctx()).flatten()
    }

    /// The ▾ under Undo: the newest undoable entries, read when it opens.
    pub fn open_undo_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ctx) = self.tabs.get(self.active).map(Tab::write_target) else { return };
        let weak = cx.weak_entity();
        let options = || menu::OpenOptions { label: Some("Undo options".into()), ..Default::default() };
        if cfg!(test) {
            let rows = undo_history_rows(&[], self.ops.journal(ctx.repo, &ctx.worktree), &ctx, &weak);
            menu::open_below("tb-undo-caret", rows, options(), window, cx);
            return;
        }
        let api = self.api.clone();
        let body = call("journalHistory", json!({ "repo": ctx.repo, "worktree": ctx.worktree }));
        let read = self.rt.spawn(async move { api.dispatch(crate::write::request_of(body)?).await });
        // The press is still held when the read answers: its release can pick a row.
        cx.spawn_in(window, async move |this, cx| {
            let rows = read.await.ok().and_then(Result::ok).and_then(|v| v.as_array().cloned()).unwrap_or_default();
            this.update_in(cx, |v, window, cx| {
                let weak = cx.weak_entity();
                let rows = undo_history_rows(&rows, v.ops.journal(ctx.repo, &ctx.worktree), &ctx, &weak);
                menu::open_below("tb-undo-caret", rows, options(), window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// An older dropdown row: undone out of order, with no Redo.
    fn undo_from_history(&mut self, ctx: WriteCtx, entry: u64, origin: Origin, cx: &mut Context<Self>) {
        let key = "undo".to_string();
        if !self.run_once(ctx.repo, key.clone()) {
            return;
        }
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let (done_ctx, done_origin) = (ctx.clone(), origin.clone());
        let w = Write::new(&ctx, move |asked| call("undoEntry", json!({ "repo": repo, "worktree": worktree, "entry": entry, "confirmAutostash": asked.autostash })))
            .origin(origin)
            .on_success(move |this, out, cx| {
                if out["status"] == "moved" {
                    return this.undone(done_ctx.clone(), entry, out, done_origin.clone(), cx);
                }
                let note = out["note"].as_str().map_or(String::new(), |n| format!(" ({n})"));
                toast::show(format!("Undid {}{note}", out["label"].as_str().unwrap_or_default()), cx);
            })
            .on_end(move |this, _, _| this.end_once(repo, &key));
        self.write(w, cx);
    }

    // ---- Stash ----

    /// A full stash: the WIP draft is its message, cleared after a stash that happened.
    pub fn stash_push(&mut self, ctx: WriteCtx, cx: &mut Context<Self>) {
        let message = self.drafts.get(&ctx.worktree).map_or(String::new(), |d| d.message().trim().to_string());
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let done_worktree = worktree.clone();
        let w = Write::new(&ctx, move |_| call("stashPush", json!({ "repo": repo, "worktree": worktree, "message": message }))).on_success(move |this, out, cx| match out["status"].as_str() {
            Some("nothingToStash") => toast::show("No changes to stash", cx),
            Some("stashed") => this.clear_draft(repo, &done_worktree, cx),
            _ => {}
        });
        self.write(w, cx);
    }

    /// Apply or Pop, by the stash's oid; then the WIP row.
    pub fn apply_stash(&mut self, ctx: WriteCtx, oid: String, pop: bool, cx: &mut Context<Self>) {
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let w = Write::new(&ctx, move |asked| call("stashApply", json!({ "repo": repo, "worktree": worktree, "oid": oid, "pop": pop, "withoutIndex": asked.without_index }))).on_success(move |this, out, cx| {
            if out["status"] == "conflicts" {
                let n = out["files"].as_u64().unwrap_or(0);
                toast::show_with(format!("The stash conflicts in {n} {}; resolve them in Conflicted. The stash is kept.", if n == 1 { "file" } else { "files" }), Options { error: true, ..Default::default() }, cx);
            }
            this.reveal_wip(repo, cx);
        });
        self.write(w, cx);
    }

    /// Delete: it arms in place first.
    pub fn drop_stash(&mut self, ctx: WriteCtx, oid: String, name: Option<String>, origin: Origin, window: &mut Window, cx: &mut Context<Self>) {
        let arm_label = name.map_or("Click again to delete the stash".to_string(), |n| format!("Click again to delete the stash \"{n}\""));
        let mut req = ArmRequest::danger(arm_label, "Delete the stash?", "Delete");
        req.body = "You can undo this.".into();
        let this = cx.weak_entity();
        arm::request(
            req,
            origin,
            move |a, _, cx| {
                if a.ok {
                    let (repo, worktree, oid, ctx) = (ctx.repo, ctx.worktree.clone(), oid.clone(), ctx.clone());
                    this.update(cx, |v, cx| v.write(Write::new(&ctx, move |_| call("stashDrop", json!({ "repo": repo, "worktree": worktree, "oid": oid }))), cx)).ok();
                }
            },
            window,
            cx,
        );
    }

    /// What came back: the WIP row, once the graph that has it arrives.
    pub fn reveal_wip(&mut self, repo: u32, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) {
            tab.reveal_wip = true;
        }
        self.reload(repo, cx);
    }

    /// The draft's gone: both its boxes empty.
    pub fn clear_draft(&mut self, repo: u32, worktree: &str, cx: &mut Context<Self>) {
        self.drafts.remove(worktree);
        self.drafts_dirty += 1;
        self.flush_drafts();
        if let Some(row) = self.tabs.iter().find(|t| t.id == repo).and_then(|t| t.row_drafts.get(worktree).cloned()) {
            row.update(cx, |i, cx| i.set_text("", cx));
        }
        self.show_draft(repo, cx);
    }

    /// A kept stash's Apply or Restore, from a banner.
    pub fn apply_kept_stash(&mut self, ctx: WriteCtx, entry: u64, cx: &mut Context<Self>) {
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let w = Write::new(&ctx, move |asked| call("applyKeptStash", json!({ "repo": repo, "worktree": worktree, "entry": entry, "withoutIndex": asked.without_index, "confirmAutostash": asked.autostash }))).on_success(move |this, _, cx| this.reveal_wip(repo, cx));
        self.write(w, cx);
    }

    /// Dismisses a banner, the stash kept or dropped.
    pub fn dismiss_banner(&mut self, ctx: WriteCtx, entry: u64, drop_stash: bool, cx: &mut Context<Self>) {
        let body = call("dismissBanner", json!({ "repo": ctx.repo, "worktree": ctx.worktree, "entry": entry, "dropStash": drop_stash }));
        if cfg!(test) {
            self.sent.borrow_mut().push(body);
            return;
        }
        let api = self.api.clone();
        let done = self.rt.spawn(async move { api.dispatch(crate::write::request_of(body)?).await });
        cx.spawn(async move |this, cx| {
            let answer = done.await.unwrap_or_else(|e| Err(GbError::other(e.to_string())));
            this.update(cx, |this, cx| {
                match answer {
                    Ok(state) => {
                        if let Ok(state) = serde_json::from_value(state) {
                            this.ops.journals.insert((ctx.repo, ctx.worktree.clone()), state);
                        }
                    }
                    Err(e) => this.toast_error(&e, Some(ctx.repo), None, None, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushOpts {
    pub target: Option<(String, String)>,
    pub set_upstream: Option<bool>,
    /// A force push's lease: the oid shown (`Some(None)`: it must not exist).
    pub lease: Option<Option<String>>,
}

/// The ▾ under Undo's rows. The first is Undo itself; an older one is listed disabled with its reason unless
/// it's independent of every later one.
pub fn undo_history_rows(loaded: &[Value], s: Option<&JournalState>, ctx: &WriteCtx, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
    type Row = (u64, String, i64, Vec<String>, Option<String>);
    let mut rows: Vec<Row> = loaded
        .iter()
        .map(|r| (r["entry"].as_u64().unwrap_or(0), r["label"].as_str().unwrap_or_default().to_string(), r["atMs"].as_i64().unwrap_or(0), r["touched"].as_array().map_or(Vec::new(), |t| t.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()), r["blocked"].as_str().map(str::to_string)))
        .collect();
    if rows.is_empty()
        && let Some(top) = s.and_then(|s| s.undo.as_ref())
    {
        rows.push((top.entry, top.label.clone(), chrono::Utc::now().timestamp_millis(), Vec::new(), s.and_then(|s| s.undo_blocked.clone())));
    }
    if rows.is_empty() {
        let why = s.and_then(|s| s.undo_blocked.clone()).unwrap_or_else(|| "Nothing to undo".into());
        return vec![MenuRow::action("undo.history.none", "Nothing to undo", "undo-2", why.clone(), |_, _| {}).disabled(why)];
    }
    rows.into_iter()
        .enumerate()
        .map(|(i, (entry, label, at, touched, blocked))| {
            let when = compact_age(at);
            let touched = if touched.is_empty() { String::new() } else { format!("; touched {}", touched_text(&touched)) };
            let tooltip = format!("Undo {label}{} ({}){touched}", if i == 0 { "" } else { " out of order" }, if when == "now" { "just now".to_string() } else { format!("{when} ago") });
            let (weak, ctx) = (this.clone(), ctx.clone());
            MenuRow::action(format!("undo.history.{entry}"), label, "undo-2", tooltip, move |_, cx| {
                let ctx = ctx.clone();
                weak.update(cx, |v, cx| if i == 0 { v.undo(ctx, Origin::None, cx) } else { v.undo_from_history(ctx, entry, Origin::None, cx) }).ok();
            })
            .shortcut(when)
            .disabled_if(blocked)
        })
        .collect()
}

/// The Fetch/Pull ▾: the default action's choices; picking sets it and runs nothing.
pub fn sync_menu_rows(mode: SyncMode, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
    let mut rows = vec![MenuRow::note("Select a default pull/fetch operation to execute when clicking this button", 2)];
    for (m, label) in [(SyncMode::FetchAll, "Fetch All"), (SyncMode::FfOrMerge, "Pull (fast-forward if possible)"), (SyncMode::FfOnly, "Pull (fast-forward only)"), (SyncMode::Rebase, "Pull (rebase)")] {
        let weak = this.clone();
        rows.push(MenuRow::action(format!("fetch.{}", m.key()), label, "", label, move |_, cx| {
            weak.update(cx, |v, cx| v.set_sync_mode(m, cx)).ok();
        }).radio(m == mode));
    }
    rows
}

/// The Push ▾: the same-named remote branches to track, Other branch…, Force push (with lease)….
pub fn push_menu_rows(v: &GitVolt, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
    let Some(tab) = v.tabs.get(v.active) else { return Vec::new() };
    let Some(b) = tab.head_branch().and_then(|n| tab.local_branch(&n)) else { return Vec::new() };
    let ctx = tab.write_target();
    let mut rows = Vec::new();
    for (group, branches) in tab.remote_groups() {
        for (name, full, _) in branches.into_iter().filter(|x| x.0 == b.name) {
            let current = b.upstream.as_deref() == Some(full.as_str());
            let (weak, c, branch, g, n) = (this.clone(), ctx.clone(), b.name.clone(), group.clone(), name.clone());
            rows.push(MenuRow::action(format!("push.upstream.{group}"), format!("{group}/{name}"), if current { "circle-dot" } else { "circle" }, if current { format!("{} tracks {group}/{name}", b.name) } else { format!("Track {group}/{name} (pushes nothing)") }, move |_, cx| {
                let (c, branch, g, n) = (c.clone(), branch.clone(), g.clone(), n.clone());
                weak.update(cx, |v, cx| v.set_upstream(c, branch, Some((g, n)), cx)).ok();
            }));
        }
    }
    let (w1, c1, name) = (this.clone(), ctx.clone(), b.name.clone());
    rows.push(MenuRow::action("push.otherBranch", "Other branch…", "arrow-up-right", format!("Choose the remote branch {} tracks", b.name), move |_, cx| {
        let (c, name) = (c1.clone(), name.clone());
        w1.update(cx, |v, cx| v.pick_upstream(c, name, cx)).ok();
    }));
    rows.push(MenuRow::Separator);
    let (w2, target) = (this.clone(), b.push_target.clone());
    let force = MenuRow::action("push.force", "Force push (with lease)…", "shield-alert", format!("Replace {} with {}, only if nobody else pushed since your last fetch", target.as_deref().unwrap_or("the remote branch"), b.name), {
        let b = b.clone();
        move |window, cx| {
            let (ctx, b) = (ctx.clone(), b.clone());
            let origin = Origin::current(cx);
            w2.update(cx, |v, cx| v.force_push(ctx, b, origin, window, cx)).ok();
        }
    });
    rows.push(force.disabled_if(target.is_none().then(|| format!("{} has no push target", b.name))));
    rows
}

/// The Sync group's Pull and Push on a commit's or chip's menu.
pub fn sync_rows(v: &GitVolt, b: &LocalBranch, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
    let Some(tab) = v.tabs.get(v.active) else { return Vec::new() };
    let ctx = tab.write_target();
    let mut rows = Vec::new();
    if b.upstream.is_some() && !b.gone {
        let up = b.upstream_short().unwrap_or_default();
        let not_here = (!b.is_head).then(|| format!("{} isn't checked out: it can only be fast-forwarded", b.name));
        let run = |mode: &'static str| -> crate::ui::Action {
            let (weak, ctx, head, name) = (this.clone(), ctx.clone(), b.is_head, b.name.clone());
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let (ctx, branch) = (ctx.clone(), (!head).then(|| name.clone()));
                weak.update(cx, |v, cx| v.pull(ctx, mode, branch, Origin::None, cx)).ok();
            })
        };
        let plain = run("ffOnly");
        let variant = |id: &str, label: &str, tip: String, run: crate::ui::Action, off: Option<String>| Variant { id: id.to_string().into(), label: Some(label.to_string().into()), icon: None, tooltip: tip.into(), run, disabled: off.map(Into::into) };
        rows.push(
            MenuRow::action("sync.pull", "Pull", "git-pull-request", format!("Pull {up} into {} (fast-forward only)", b.name), move |w, cx| plain(w, cx)).variants(
                vec![
                    variant("ffOnly", "ff-only", "Fast-forward only".into(), run("ffOnly"), None),
                    variant("rebase", "rebase", format!("Rebase {} onto {up}", b.name), run("rebase"), not_here.clone()),
                    variant("merge", "merge", format!("Merge {up} into {}", b.name), run("ffOrMerge"), not_here),
                ],
                Some("ffOnly"),
            ),
        );
    }
    if !nothing_to_push(b) {
        let (w1, c1, b1) = (this.clone(), ctx.clone(), b.clone());
        let row = MenuRow::action("sync.push", "Push", "arrow-up-from-line", push_label(b), move |_, cx| {
            let (c, bb) = (c1.clone(), b1.clone());
            w1.update(cx, |v, cx| v.push_branch(c, bb, Origin::None, cx)).ok();
        });
        rows.push(match &b.push_target {
            Some(target) => {
                let (w2, c2, b2) = (this.clone(), ctx.clone(), b.clone());
                let force = Variant {
                    id: "sync.push.force".into(),
                    label: Some("force-with-lease".into()),
                    icon: None,
                    tooltip: format!("Replace {target} with {}, only if nobody else pushed since your last fetch", b.name).into(),
                    run: Rc::new(move |window, cx| {
                        let (c, bb) = (c2.clone(), b2.clone());
                        let origin = Origin::current(cx);
                        w2.update(cx, |v, cx| v.force_push(c, bb, origin, window, cx)).ok();
                    }),
                    disabled: None,
                };
                row.variants(vec![force], None)
            }
            None => row,
        });
    }
    rows
}

/// The panel that asks where: remote, branch name and Track it.
fn push_panel(ctx: WriteCtx, b: LocalBranch, remotes: Vec<String>, anchor: Option<Bounds<Pixels>>, this: WeakEntity<GitVolt>, window: &mut Window, cx: &mut App) {
    const ID: &str = "push-upstream";
    let input = cx.new(|cx| {
        let mut i = TextInput::new(cx);
        i.set_text(&b.name, cx);
        i
    });
    let remote = Rc::new(std::cell::RefCell::new(remotes[0].clone()));
    let track = Rc::new(std::cell::Cell::new(true));
    let submit: crate::ui::Action = {
        let (input, remote, track, this, ctx, b) = (input.clone(), remote.clone(), track.clone(), this, ctx, b.clone());
        Rc::new(move |window, cx| {
            let branch = input.read(cx).text().trim().to_string();
            if branch.is_empty() {
                return;
            }
            modal::close(ID, window, cx);
            let opts = PushOpts { target: Some((remote.borrow().clone(), branch)), set_upstream: Some(track.get()), lease: None };
            let (ctx, b) = (ctx.clone(), b.clone());
            this.update(cx, |v, cx| v.send_push(ctx, b, opts, Origin::None, cx)).ok();
        })
    };
    let handle = window.window_handle();
    let enter = cx.subscribe(&input, {
        let submit = submit.clone();
        move |_, e: &InputEvent, cx| match e {
            InputEvent::Submit { .. } => {
                let submit = submit.clone();
                handle.update(cx, move |_, window, cx| window.defer(cx, move |window, cx| submit(window, cx))).ok();
            }
            InputEvent::Changed => cx.refresh_windows(),
            _ => {}
        }
    });
    let focus = input.read(cx).focus.clone();
    let name = b.name;
    let stops = [cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true)];
    let mut m = modal::Modal::new(ID, move |_, cx| {
        let options: controls::SelectOptions<String> = Rc::new(remotes.iter().map(|r| (r.clone(), SharedString::from(r.clone()), None)).collect());
        let chosen = remote.borrow().clone();
        let set = remote.clone();
        let select = controls::select(
            "push-remote",
            chosen,
            options,
            None,
            Rc::new(move |v: &String, _: &mut Window, cx: &mut App| {
                *set.borrow_mut() = v.clone();
                cx.refresh_windows();
            }),
            cx,
        );
        let ok = !input.read(cx).text().trim().is_empty();
        let (s, s2, t, t2) = (submit.clone(), submit.clone(), track.clone(), track.clone());
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .child(format!("Push {name} to"))
                    .child(div().flex().flex_none().w(px(110.)).child(select))
                    .child("/")
                    .child(div().flex_1().min_w(px(220.)).h(px(28.)).px(px(7.)).flex().items_center().bg(crate::theme::app_bg()).border_1().border_color(crate::theme::blue()).rounded(px(4.)).text_color(crate::theme::text_selected()).child(input.clone())),
            )
            .child(controls::tabbable(
                check_row("push-track", track.get(), "Track it", move |on, _, cx| {
                    t.set(on);
                    cx.refresh_windows();
                })
                .border_1()
                .border_color(transparent_black())
                .rounded(px(3.))
                .ml(px(-1.)),
                &stops[0],
                move |_, cx| {
                    t2.set(!t2.get());
                    cx.refresh_windows();
                },
            ))
            .child(
                modal::actions_row()
                    .child(controls::tabbable(controls::button("push-cancel", "Cancel", ButtonKind::Quiet, true).on_click(|_, window, cx| modal::close(ID, window, cx)), &stops[1], |window, cx| modal::close(ID, window, cx)))
                    .child(controls::tabbable(controls::button("push-ok", "Push", ButtonKind::Positive, ok).when(ok, |d| d.on_click(move |_, window, cx| s(window, cx))), &stops[2], move |window, cx| s2(window, cx))),
            )
            .into_any_element()
    })
    .width(520.)
    .anchored(anchor)
    .focus(focus);
    m.keep.push(enter);
    modal::open(m, window, cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::testing::{Fixture, git};
    use std::prelude::v1::test;

    fn branch(name: &str) -> LocalBranch {
        LocalBranch { name: name.into(), full_name: format!("refs/heads/{name}"), target: "abc".into(), ..Default::default() }
    }

    #[test]
    fn the_sync_button_says_what_it_does() {
        assert_eq!(sync_view(SyncMode::FetchAll, None, None).label, "Fetch");
        assert_eq!(sync_view(SyncMode::FfOnly, None, None).tooltip, "HEAD is detached");
        let b = branch("main");
        assert_eq!(sync_view(SyncMode::Rebase, Some(&b), Some("main")).tooltip, "main has no upstream; set one from Push ▾");
        let up = LocalBranch { upstream: Some("refs/remotes/origin/main".into()), ..b };
        let v = sync_view(SyncMode::Rebase, Some(&up), Some("main"));
        assert_eq!((v.label.as_str(), v.tooltip.as_str(), v.disabled), ("Pull", "Pull origin/main into main (rebase)", false));
    }

    #[test]
    fn push_says_where_and_force_what_it_replaces() {
        assert_eq!(push_view(None, None, "origin"), ("HEAD is detached".to_string(), true));
        let b = branch("new");
        assert_eq!(push_view(Some(&b), Some("new"), "origin").0, "Push new to origin and track it");
        let t = LocalBranch { push_target: Some("origin/x".into()), push_behind: Some(2), rewritten: Some(("rebase".into(), "origin".into())), ..branch("x") };
        assert_eq!(push_label(&t), "Push (force with lease: rebased, replaces 2 commits on origin)");
        assert_eq!(force_arm("origin/x", Some(1)), "Click again to force push: replaces 1 commit");
        assert_eq!(force_arm("origin/x", None), "Click again to force push to origin/x");
        assert!(force_text(&t).starts_with("It replaces 2 commits on origin/x"));
        // Up to date with its upstream: the menu hides Push.
        let done = LocalBranch { upstream: Some("refs/remotes/origin/x".into()), push_target: Some("origin/x".into()), ..branch("x") };
        assert!(nothing_to_push(&done));
        assert!(!nothing_to_push(&LocalBranch { ahead: 1, ..done }));
    }

    #[test]
    fn a_moved_branch_says_what_undoing_does() {
        let refs = vec![json!({ "name": "refs/heads/main", "expected": "1111111aaa", "actual": "2222222bbb", "target": "3333333ccc", "dropped": 2 })];
        assert_eq!(moved_text("commit \"x\"", &refs), "main moved since commit \"x\" (it's at 2222222, not 1111111). Undoing moves it to 3333333 and drops the 2 commits made since.");
        assert_eq!(touched_text(&["a".into(), "b".into(), "c".into(), "d".into(), "e".into()]), "a, b, c and 2 more");
    }

    #[tokio::test]
    async fn undo_and_redo_of_a_commit_and_the_history_lists_it() {
        let f = Fixture::new().await;
        f.write("a.txt", "changed\n");
        let head = f.git(&["rev-parse", "HEAD"]);
        let (repo, wt) = (f.repo, f.worktree.clone());
        let r = f.ok(call("commit", json!({ "repo": repo, "worktree": wt, "summary": "Work", "description": "", "amend": false, "stageAll": true, "expect": { "head": head, "refs": {} } }))).await;
        let entry = r["journal"]["undo"]["entry"].as_u64().unwrap();
        assert!(r["journal"]["undo"]["label"].as_str().unwrap().contains("Work"));
        let hist = f.ok(call("journalHistory", json!({ "repo": repo, "worktree": wt }))).await;
        assert_eq!(hist[0]["entry"].as_u64(), Some(entry));
        let u = f.ok(call("undo", json!({ "repo": repo, "worktree": wt, "entry": entry, "confirm": null, "confirmAutostash": false, "withoutIndex": false, "confirmDiscard": false }))).await;
        assert_eq!(u["outcome"]["status"], "done");
        assert_eq!(f.git(&["rev-parse", "HEAD"]), head);
        let top = u["journal"]["redo"]["entry"].as_u64().unwrap();
        f.ok(call("redo", json!({ "repo": repo, "worktree": wt, "entry": top, "confirmAutostash": false, "withoutIndex": false }))).await;
        assert_ne!(f.git(&["rev-parse", "HEAD"]), head);
        assert_eq!(f.git(&["log", "-1", "--format=%s"]), "Work");
    }

    #[tokio::test]
    async fn a_commit_made_outside_makes_undo_ask_and_the_answer_goes_through() {
        let f = Fixture::new().await;
        f.write("a.txt", "changed\n");
        let head = f.git(&["rev-parse", "HEAD"]);
        let (repo, wt) = (f.repo, f.worktree.clone());
        let r = f.ok(call("commit", json!({ "repo": repo, "worktree": wt, "summary": "Work", "description": "", "amend": false, "stageAll": true, "expect": { "head": head, "refs": {} } }))).await;
        let entry = r["journal"]["undo"]["entry"].as_u64().unwrap();
        f.write("b.txt", "outside\n");
        f.git(&["add", "."]);
        f.git(&["commit", "-qm", "Outside"]);
        let body = |confirm: Value| call("undo", json!({ "repo": repo, "worktree": wt, "entry": entry, "confirm": confirm, "confirmAutostash": false, "withoutIndex": false, "confirmDiscard": false }));
        let asked = f.ok(body(Value::Null)).await;
        assert_eq!(asked["outcome"]["status"], "moved");
        let refs = asked["outcome"]["refs"].as_array().unwrap().clone();
        assert!(moved_text("x", &refs).contains("drops the 1 commit made since"), "{refs:?}");
        let confirm: serde_json::Map<String, Value> = refs.iter().map(|r| (r["name"].as_str().unwrap().to_string(), r["actual"].clone())).collect();
        f.ok(body(Value::Object(confirm))).await;
        assert_eq!(f.git(&["rev-parse", "HEAD"]), head);
    }

    #[tokio::test]
    async fn fetch_pull_and_push_against_a_bare_remote() {
        let f = Fixture::new().await;
        let bare = f.bare_origin();
        let (repo, wt) = (f.repo, f.worktree.clone());
        // Another clone moves the remote.
        let other = f.root.join("other");
        git(&f.root, &["clone", "-q", bare.to_str().unwrap(), other.to_str().unwrap()]);
        git(&other, &["config", "user.name", "Bob Example"]);
        git(&other, &["config", "user.email", "bob@example.com"]);
        std::fs::write(other.join("c.txt"), "c\n").unwrap();
        git(&other, &["add", "."]);
        git(&other, &["commit", "-qm", "From elsewhere"]);
        git(&other, &["push", "-q"]);
        let fetched = f.ok(call("fetch", json!({ "repo": repo, "background": false }))).await;
        assert_eq!(fetched["status"], "done");
        assert_eq!(f.git(&["rev-list", "--count", "HEAD..origin/main"]), "1");
        let pulled = f.ok(call("pull", json!({ "repo": repo, "worktree": wt, "mode": "ffOnly", "confirm": { "autostash": false } }))).await;
        assert_eq!(pulled["outcome"]["result"]["status"], "fastForward");
        assert_eq!(f.git(&["log", "-1", "--format=%s"]), "From elsewhere");
        // A push goes up; a new branch with a target tracks it.
        f.write("a.txt", "mine\n");
        f.git(&["commit", "-qam", "Mine"]);
        let oid = f.git(&["rev-parse", "HEAD"]);
        let pushed = f.ok(call("push", json!({ "repo": repo, "worktree": wt, "branch": "main", "expect": { "head": null, "refs": { "refs/heads/main": oid } } }))).await;
        assert_eq!(pushed["outcome"]["dst"], "main");
        assert_eq!(git(&bare, &["rev-parse", "main"]), oid);
        f.git(&["checkout", "-q", "-b", "feature/new"]);
        let oid = f.git(&["rev-parse", "HEAD"]);
        f.ok(call("push", json!({ "repo": repo, "worktree": wt, "branch": "feature/new", "target": { "remote": "origin", "branch": "feature/new" }, "setUpstream": true, "expect": { "head": null, "refs": { "refs/heads/feature/new": oid } } }))).await;
        assert_eq!(f.git(&["rev-parse", "--abbrev-ref", "@{u}"]), "origin/feature/new");
    }

    #[tokio::test]
    async fn a_rejected_push_is_non_fast_forward_and_force_with_the_lease_replaces() {
        let f = Fixture::new().await;
        let bare = f.bare_origin();
        let (repo, wt) = (f.repo, f.worktree.clone());
        f.git(&["commit", "-q", "--amend", "-m", "Rewritten"]);
        let oid = f.git(&["rev-parse", "HEAD"]);
        let tracking = f.git(&["rev-parse", "origin/main"]);
        let body = |lease: Value| call("push", json!({ "repo": repo, "worktree": wt, "branch": "main", "lease": lease, "expect": { "head": null, "refs": { "refs/heads/main": oid } } }));
        let e = f.send(body(Value::Null)).await.unwrap_err();
        assert_eq!(e.kind, GbErrorKind::NonFastForward);
        // A stale lease refuses it too.
        let stale = f.send(body(json!({ "oid": "0000000000000000000000000000000000000001" }))).await;
        assert!(stale.is_err());
        f.ok(body(json!({ "oid": tracking }))).await;
        assert_eq!(git(&bare, &["rev-parse", "main"]), oid);
    }

    #[tokio::test]
    async fn stash_push_pop_apply_and_drop() {
        let f = Fixture::new().await;
        let (repo, wt) = (f.repo, f.worktree.clone());
        f.write("a.txt", "changed\n");
        let r = f.ok(call("stashPush", json!({ "repo": repo, "worktree": wt, "message": "My draft" }))).await;
        assert_eq!(r["outcome"]["status"], "stashed");
        assert_eq!(f.git(&["status", "--porcelain"]), "");
        assert!(f.git(&["stash", "list"]).contains("My draft"));
        let oid = f.git(&["rev-parse", "stash@{0}"]);
        let r = f.ok(call("stashApply", json!({ "repo": repo, "worktree": wt, "oid": oid, "pop": false, "withoutIndex": false }))).await;
        assert_eq!(r["outcome"]["status"], "applied");
        assert_eq!(f.read("a.txt"), "changed\n");
        assert!(f.git(&["stash", "list"]).contains("My draft"), "Apply keeps it");
        f.git(&["checkout", "-q", "--", "a.txt"]);
        f.ok(call("stashApply", json!({ "repo": repo, "worktree": wt, "oid": oid, "pop": true, "withoutIndex": false }))).await;
        assert_eq!(f.git(&["stash", "list"]), "", "Pop deletes it");
        let again = f.ok(call("stashPush", json!({ "repo": repo, "worktree": wt, "message": "" }))).await;
        let oid = f.git(&["rev-parse", "stash@{0}"]);
        assert_eq!(again["outcome"]["status"], "stashed");
        f.ok(call("stashDrop", json!({ "repo": repo, "worktree": wt, "oid": oid }))).await;
        assert_eq!(f.git(&["stash", "list"]), "");
        let nothing = f.ok(call("stashPush", json!({ "repo": repo, "worktree": wt, "message": "" }))).await;
        assert_eq!(nothing["outcome"]["status"], "nothingToStash");
    }
}
