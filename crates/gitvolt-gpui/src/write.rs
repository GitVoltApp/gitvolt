//! One write (`ui/src/write/client.ts`): a request through `Api::dispatch`, so the core's pipeline, queue,
//! journal and hooks apply. Its answer's journal, staging log and WIP lists are applied at once; a question
//! the core asks (autostash, a hard reset's losses..) arms the control that started it and sends again; any
//! other failure goes to the error toast, with Retry, Refresh and Remove stale lock.
//!
//! Requests are built as the web UI's `api/client.ts` builds them: `{"method", "params"}` JSON, which
//! `Request` deserializes.

use crate::app::GitVolt;
use crate::ui::arm::{self, ArmRequest, Origin, Tone};
use crate::ui::toast::{self, Options, ToastAction};
use gitvolt_core::api::Request;
use gitvolt_core::askpass::SshOffer;
use gitvolt_core::error::{ErrorDetail, GbError, GbErrorKind};
use gpui::*;
use serde_json::{Value, json};
use std::rc::Rc;

/// Where a write runs: the repo and the worktree (canonical, as the core says it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteCtx {
    pub repo: u32,
    pub worktree: String,
}

/// The questions a write's request has had answered "yes"; each is asked at most once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Asked {
    pub autostash: bool,
    pub without_index: bool,
    pub discard: bool,
    pub markers: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    Autostash,
    WithoutIndex,
    Discard,
    Markers,
}

impl Flag {
    fn set(self, mut asked: Asked) -> Asked {
        match self {
            Flag::Autostash => asked.autostash = true,
            Flag::WithoutIndex => asked.without_index = true,
            Flag::Discard => asked.discard = true,
            Flag::Markers => asked.markers = true,
        }
        asked
    }
}

pub type Send = Rc<dyn Fn(Asked) -> Value>;
pub type OnSuccess = Rc<dyn Fn(&mut GitVolt, &Value, &mut Context<GitVolt>)>;
pub type OnHandle = Rc<dyn Fn(&mut GitVolt, &GbError, &mut Context<GitVolt>) -> bool>;
pub type OnEnd = Rc<dyn Fn(&mut GitVolt, bool, &mut Context<GitVolt>)>;
/// What runs after something, in the view.
pub type After = Rc<dyn Fn(&mut GitVolt, &mut Context<GitVolt>)>;

pub struct Write {
    pub ctx: WriteCtx,
    pub send: Send,
    pub origin: Origin,
    pub on_success: Option<OnSuccess>,
    pub handle: Option<OnHandle>,
    pub on_end: Option<OnEnd>,
}

impl Write {
    pub fn new(ctx: &WriteCtx, send: impl Fn(Asked) -> Value + 'static) -> Self {
        Self { ctx: ctx.clone(), send: Rc::new(send), origin: Origin::None, on_success: None, handle: None, on_end: None }
    }

    /// Its questions arm this control.
    pub fn origin(mut self, origin: Origin) -> Self {
        self.origin = origin;
        self
    }

    pub fn on_success(mut self, f: impl Fn(&mut GitVolt, &Value, &mut Context<GitVolt>) + 'static) -> Self {
        self.on_success = Some(Rc::new(f));
        self
    }

    /// A failure the caller has its own answer for: whether it took it.
    pub fn handle(mut self, f: impl Fn(&mut GitVolt, &GbError, &mut Context<GitVolt>) -> bool + 'static) -> Self {
        self.handle = Some(Rc::new(f));
        self
    }

    /// Once it's over, however it ended; whether it succeeded.
    pub fn on_end(mut self, f: impl Fn(&mut GitVolt, bool, &mut Context<GitVolt>) + 'static) -> Self {
        self.on_end = Some(Rc::new(f));
        self
    }
}

/// A request, as `client.ts` sends it.
pub fn call(method: &str, params: Value) -> Value {
    json!({ "method": method, "params": params })
}

pub fn request_of(call: Value) -> Result<Request, GbError> {
    serde_json::from_value(call).map_err(|e| GbError::other(format!("A request that can't be sent: {e}")))
}

/// A confirmation's words for a failure that's a question.
#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    pub req: ArmRequest,
    pub flag: Flag,
}

/// The question a failure asks, answered by sending again with its flag set; `None`: not one.
pub fn question(err: &GbError, asked: Asked) -> Option<Question> {
    let q = |tone, title: String, body: String, confirm: &str, arm: String, caption: Option<String>, flag| {
        let mut req = ArmRequest::new(tone, arm, title, confirm.to_string());
        req.body = body.into();
        req.caption = caption.map(Into::into);
        Some(Question { req, flag })
    };
    match err.detail.as_ref()? {
        ErrorDetail::AutostashConflict { paths, target } if !asked.autostash => {
            let first = paths.first().cloned().unwrap_or_default();
            let more = if paths.len() > 1 { format!(" (and {} more)", paths.len() - 1) } else { String::new() };
            let body = format!("Your changes to {first}{more} conflict with {target}. They'll be kept in a stash you can apply afterwards.");
            q(Tone::Warn, "Your changes conflict".into(), body.clone(), "Continue", "Click again to continue: your changes go to a stash".into(), Some(body), Flag::Autostash)
        }
        ErrorDetail::ApplyWithoutIndex if !asked.without_index => q(Tone::Warn, "Apply without restoring what was staged?".into(), "git couldn't restore what was staged: the changes go in unstaged.".into(), "Apply", "Click again to apply it all unstaged".into(), Some("git couldn't restore what was staged.".into()), Flag::WithoutIndex),
        ErrorDetail::ResetDiscards { branch, files, .. } if !asked.discard => {
            let arm = format!("Click again to reset {branch} and discard changes to {files} {}", if *files == 1 { "file" } else { "files" });
            q(Tone::Danger, "Discard changes?".into(), err.message.clone(), "Reset", arm, Some(err.message.clone()), Flag::Discard)
        }
        ErrorDetail::MarkersRemain { path } if !asked.markers => q(Tone::Warn, "Conflict markers remain".into(), format!("{path} still has conflict markers. Mark it resolved anyway?"), "Mark resolved", "Click again to mark it resolved with its conflict markers".into(), Some(format!("{path} still has conflict markers.")), Flag::Markers),
        ErrorDetail::MarkersRemainIn { paths } if !asked.markers => {
            let n = paths.len();
            let which = if n == 1 { format!("{} still has", paths[0]) } else { format!("{n} files still have") };
            let names = if n <= 3 { paths.join(", ") } else { format!("{} and {} more", paths[..3].join(", "), n - 3) };
            q(Tone::Warn, "Conflict markers remain".into(), format!("{which} conflict markers ({names}). Mark them resolved anyway?"), "Mark resolved", format!("Click again: {which} conflict markers"), Some(format!("With markers: {names}.")), Flag::Markers)
        }
        ErrorDetail::DiscardEdits { path } if !asked.discard => q(Tone::Danger, "Discard your edits?".into(), format!("The side you chose replaces {path}, and Undo can't bring the edits back."), "Discard edits", format!("Click again to replace your edits to {path}"), Some("Undo can't bring the edits back.".into()), Flag::Discard),
        ErrorDetail::RestoreOverChanges { path } if !asked.discard => q(Tone::Warn, "Replace your changes?".into(), format!("{path} has changes of its own. Restoring replaces them; you can undo this."), "Restore", format!("Click again to replace your changes to {path}"), Some(format!("{path} has changes of its own.")), Flag::Discard),
        ErrorDetail::SubmoduleOrphans { paths, commits } if !asked.discard => {
            let them = if *commits == 1 { "its commit".to_string() } else { format!("{commits} commits") };
            let arm = if paths.len() == 1 { format!("Click again to update {}, leaving {them} behind", paths[0]) } else { format!("Click again to update, leaving {them} behind") };
            q(Tone::Danger, "Leave commits behind?".into(), err.message.clone(), "Update", arm, Some("They stay only in the reflog.".into()), Flag::Discard)
        }
        ErrorDetail::UndoStoppedPick { op, arm } if !asked.discard => q(Tone::Danger, format!("Undo the stopped {op}?"), err.message.clone(), "Undo", arm.to_string(), Some("Its changes are discarded, including anything you resolved since.".into()), Flag::Discard),
        _ => None,
    }
}

/// The remotes a cancelled op's prompt switched to SSH.
pub fn ssh_switches(err: &GbError) -> Option<Vec<SshOffer>> {
    if err.kind != GbErrorKind::Cancelled {
        return None;
    }
    match err.detail.as_ref()? {
        ErrorDetail::UseSsh { remotes } if !remotes.is_empty() => Some(remotes.clone()),
        _ => None,
    }
}

/// Title and message for a notice, or `None` for what never notifies (a cancel).
pub fn describe_error(err: &GbError) -> Option<(String, String)> {
    let title = match err.kind {
        GbErrorKind::Cancelled => return None,
        GbErrorKind::AuthFailed => "Authentication failed",
        GbErrorKind::NonFastForward => "Branch has diverged",
        GbErrorKind::Conflict => "Merge conflict",
        GbErrorKind::DirtyWorktree => "Uncommitted changes in the way",
        GbErrorKind::IndexLocked => "Repository is locked",
        GbErrorKind::RefMoved => "Branch changed outside GitVolt",
        GbErrorKind::NotFound => "Not found",
        GbErrorKind::InvalidInput => "Invalid input",
        GbErrorKind::GitTooOld => "git is too old",
        GbErrorKind::Io => "File system error",
        GbErrorKind::Other => "Something went wrong",
        GbErrorKind::HookFailed => "A hook failed",
        GbErrorKind::InProgress => "Operation in progress",
        GbErrorKind::Stale => "Changed since it was shown",
        GbErrorKind::RateLimited => "Rate limited",
        GbErrorKind::Network => "Couldn't reach the server",
    };
    let (title, message) = match err.kind {
        GbErrorKind::RefMoved => (title.to_string(), "Branch changed outside GitVolt — refresh and retry".to_string()),
        GbErrorKind::IndexLocked => {
            let held = matches!(err.detail, Some(ErrorDetail::IndexLock { .. }));
            (title.to_string(), if held { format!("{}\nAnother git program may be running.", err.message) } else { format!("{}\nAnother git process may be running. If none is, remove .git/index.lock yourself.", err.message) })
        }
        GbErrorKind::HookFailed => match &err.detail {
            Some(ErrorDetail::Hook { hook }) => (format!("{hook} hook failed"), err.message.clone()),
            _ => (title.to_string(), err.message.clone()),
        },
        GbErrorKind::InProgress => (err.message.clone(), "Finish or abort it first".to_string()),
        GbErrorKind::Stale => (err.message.clone(), "Refreshed: try again".to_string()),
        GbErrorKind::InvalidInput if err.message.contains(" is a repository in the way of the ") => ("A repository is in the way".to_string(), in_the_way(&err.message)),
        _ => (title.to_string(), err.message.clone()),
    };
    Some((title, message))
}

fn in_the_way(message: &str) -> String {
    let Some((path, rest)) = message.split_once(" is a repository in the way of the ") else { return message.to_string() };
    let op = rest.strip_suffix(": move it first").unwrap_or(rest);
    format!("{path} can't be replaced during the {op}: move it first")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Retry,
    Refresh,
    RemoveLock,
    Copy,
}

/// The kind's suggested action, in the toast's first link.
pub fn slots(kind: GbErrorKind) -> &'static [Slot] {
    match kind {
        GbErrorKind::AuthFailed | GbErrorKind::HookFailed | GbErrorKind::Network => &[Slot::Retry],
        GbErrorKind::NonFastForward | GbErrorKind::Conflict | GbErrorKind::DirtyWorktree | GbErrorKind::RefMoved | GbErrorKind::Stale => &[Slot::Refresh],
        GbErrorKind::IndexLocked => &[Slot::RemoveLock, Slot::Retry],
        GbErrorKind::Io | GbErrorKind::Other => &[Slot::Copy],
        _ => &[],
    }
}

impl GitVolt {
    /// Sends a write.
    pub fn write(&mut self, w: Write, cx: &mut Context<Self>) {
        self.attempt(Rc::new(w), Asked::default(), Vec::new(), cx);
    }

    fn attempt(&mut self, w: Rc<Write>, asked: Asked, switched: Vec<SshOffer>, cx: &mut Context<Self>) {
        let body = (w.send)(asked);
        if cfg!(test) {
            self.sent.borrow_mut().push(body);
            self.unanswered.push((w, asked, switched));
            return;
        }
        let api = self.api.clone();
        let done = self.rt.spawn(async move { api.dispatch(request_of(body)?).await });
        cx.spawn(async move |this, cx| {
            let answer = done.await.unwrap_or_else(|e| Err(GbError::other(e.to_string())));
            this.update(cx, |this, cx| this.answered(w, asked, switched, answer, cx)).ok();
        })
        .detach();
    }

    /// For a test: answers the oldest write sent.
    #[cfg(test)]
    pub fn answer_write(&mut self, answer: Result<Value, GbError>, cx: &mut Context<Self>) {
        let (w, asked, switched) = self.unanswered.remove(0);
        self.answered(w, asked, switched, answer, cx);
    }

    fn answered(&mut self, w: Rc<Write>, asked: Asked, switched: Vec<SshOffer>, answer: Result<Value, GbError>, cx: &mut Context<Self>) {
        match answer {
            Ok(v) => {
                self.apply_result(&w.ctx, &v, cx);
                if let Some(f) = &w.on_success {
                    f(self, &v["outcome"], cx);
                }
                if let Some(f) = &w.on_end {
                    f(self, true, cx);
                }
                cx.notify();
            }
            Err(err) => self.failed(w, asked, switched, err, cx),
        }
    }

    fn failed(&mut self, w: Rc<Write>, asked: Asked, mut switched: Vec<SshOffer>, err: GbError, cx: &mut Context<Self>) {
        if let Some(ssh) = ssh_switches(&err) {
            switched.extend(ssh.iter().cloned());
            self.switch_to_ssh(&w.ctx.clone(), ssh, move |this, cx| this.attempt(w.clone(), asked, switched.clone(), cx), cx);
            return;
        }
        if let Some(q) = question(&err, asked) {
            let origin = w.origin.clone();
            let this = cx.weak_entity();
            let window = self.window;
            cx.defer(move |cx| {
                let Some(window) = window else { return };
                window
                    .update(cx, |_, window, cx| {
                        arm::request(
                            q.req,
                            origin,
                            move |answer, _, cx| {
                                this.update(cx, |this, cx| {
                                    if answer.ok {
                                        this.attempt(w.clone(), q.flag.set(asked), switched.clone(), cx);
                                    } else if let Some(f) = &w.on_end {
                                        f(this, false, cx);
                                    }
                                })
                                .ok();
                            },
                            window,
                            cx,
                        );
                    })
                    .ok();
            });
            return;
        }
        // A stale undo, redo or banner changed nothing, so no journalChanged comes.
        if err.kind == GbErrorKind::Stale {
            self.load_journal(w.ctx.repo, w.ctx.worktree.clone(), cx);
        }
        let handled = w.handle.as_ref().is_some_and(|f| f(self, &err, cx));
        if let Some(f) = &w.on_end {
            f(self, false, cx);
        }
        if handled {
            return;
        }
        if !switched.is_empty() {
            self.toast_after_switch(&w.ctx.clone(), switched, &err, None, cx);
            return;
        }
        // A retry never re-sends a destructive discard the user confirmed for an earlier attempt.
        let this = cx.weak_entity();
        let retry: crate::ui::Action = Rc::new({
            let (this, w) = (this.clone(), w.clone());
            move |_, cx| {
                let w = w.clone();
                this.update(cx, |v, cx| v.attempt(w, Asked { discard: false, ..asked }, Vec::new(), cx)).ok();
            }
        });
        let repo = w.ctx.repo;
        let refresh: crate::ui::Action = Rc::new({
            let this = this.clone();
            move |_, cx| {
                this.update(cx, |v, cx| v.refresh_repo(repo, cx)).ok();
            }
        });
        self.toast_error(&err, Some(repo), Some(retry), Some(refresh), cx);
    }

    /// A user's action failed: the toast, with the kind's suggested link, then Details.
    pub fn toast_error(&mut self, err: &GbError, repo: Option<u32>, retry: Option<crate::ui::Action>, refresh: Option<crate::ui::Action>, cx: &mut Context<Self>) {
        let Some((title, message)) = describe_error(err) else { return };
        let this = cx.weak_entity();
        let mut first = None;
        for slot in slots(err.kind) {
            first = match slot {
                Slot::Retry => retry.clone().map(|run| ToastAction { label: "Retry".into(), run }),
                Slot::Refresh => refresh.clone().map(|run| ToastAction { label: "Refresh".into(), run }),
                Slot::RemoveLock => match (&err.detail, repo) {
                    (Some(ErrorDetail::IndexLock { path, mtime_ms, ino, dev }), Some(repo)) => {
                        let (path, mtime_ms, ino, dev) = (path.to_string(), *mtime_ms, *ino, *dev);
                        let this = this.clone();
                        Some(ToastAction {
                            label: "Remove stale lock".into(),
                            run: Rc::new(move |window, cx| {
                                let (this, path) = (this.clone(), path.clone());
                                let label = lock_label(&path);
                                let mut req = ArmRequest::danger(format!("Click again to remove {label}"), format!("Remove {label}?"), "Remove");
                                req.body = "Do this only if no other git program is running.".into();
                                req.caption = Some("Only if no other git program is running.".into());
                                arm::request(
                                    req,
                                    Origin::None,
                                    move |a, _, cx| {
                                        if a.ok {
                                            this.update(cx, |v, cx| v.remove_lock(repo, path.clone(), mtime_ms, ino, dev, cx)).ok();
                                        }
                                    },
                                    window,
                                    cx,
                                );
                            }),
                        })
                    }
                    _ => None,
                },
                Slot::Copy => None,
            };
            if first.is_some() {
                break;
            }
        }
        let first = first.unwrap_or_else(|| {
            let text = [Some(title.clone()), Some(message.clone()), err.stderr.clone()].into_iter().flatten().collect::<Vec<_>>().join("\n");
            ToastAction {
                label: "Copy error".into(),
                run: Rc::new(move |_, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    toast::show("Copied", cx);
                }),
            }
        });
        let command = err.command_id.filter(|_| err.kind != GbErrorKind::InvalidInput && err.kind != GbErrorKind::GitTooOld);
        let details = ToastAction {
            label: "Details".into(),
            run: Rc::new(move |window, cx| {
                this.update(cx, |v, cx| v.open_debug(command, window, cx)).ok();
            }),
        };
        toast::show_with(format!("{title}: {message}"), Options { error: true, actions: vec![first, details], ..Default::default() }, cx);
    }

    fn remove_lock(&mut self, repo: u32, path: String, mtime_ms: i64, ino: u64, dev: u64, cx: &mut Context<Self>) {
        let api = self.api.clone();
        let label = lock_label(&path);
        let body = call("removeIndexLock", json!({ "repo": repo, "path": path, "mtimeMs": mtime_ms, "ino": ino, "dev": dev }));
        if cfg!(test) {
            self.sent.borrow_mut().push(body);
            return;
        }
        let done = self.rt.spawn(async move { api.dispatch(request_of(body)?).await });
        cx.spawn(async move |this, cx| {
            let answer = done.await.unwrap_or_else(|e| Err(GbError::other(e.to_string())));
            this.update(cx, |this, cx| match answer {
                Ok(_) => toast::show(format!("Removed {label}"), cx),
                Err(e) => this.toast_error(&e, Some(repo), None, None, cx),
            })
            .ok();
        })
        .detach();
    }

    /// A write's answer: the journal, the staging log and the fresh WIP lists at once, so the UI never
    /// waits on the file watcher.
    pub fn apply_result(&mut self, ctx: &WriteCtx, v: &Value, cx: &mut Context<Self>) {
        if let Ok(j) = serde_json::from_value(v["journal"].clone()) {
            self.ops.journals.insert((ctx.repo, ctx.worktree.clone()), j);
        }
        if let Ok(s) = serde_json::from_value(v["staging"].clone()) {
            self.ops.staging.insert((ctx.repo, ctx.worktree.clone()), s);
        }
        if v["wip"].is_object() {
            let wip = &v["wip"];
            self.put_wip(ctx.repo, wip["worktree"].as_str().unwrap_or(&ctx.worktree).to_string(), &wip["unstaged"], &wip["staged"], cx);
        }
    }

    /// Asks the core for a worktree's journal, for Undo, Redo and the banners.
    pub fn load_journal(&mut self, repo: u32, worktree: String, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let api = self.api.clone();
        let (r, w) = (repo, worktree.clone());
        let read = self.rt.spawn(async move { api.dispatch(Request::JournalState { repo: r, worktree: w }).await });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(v)) = read.await
                && let Ok(state) = serde_json::from_value(v)
            {
                this.update(cx, |this, cx| {
                    this.ops.journals.insert((repo, worktree), state);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Refresh: reads what the view shows again.
    pub fn refresh_repo(&mut self, repo: u32, cx: &mut Context<Self>) {
        self.reload(repo, cx);
    }

    /// Points each remote at its SSH URL, one write each, then `then`.
    pub fn switch_to_ssh(&mut self, ctx: &WriteCtx, remotes: Vec<SshOffer>, then: impl Fn(&mut Self, &mut Context<Self>) + 'static, cx: &mut Context<Self>) {
        let then: crate::write::After = Rc::new(then);
        self.switch_next(ctx.clone(), Rc::new(remotes), 0, then, cx);
    }

    fn switch_next(&mut self, ctx: WriteCtx, remotes: Rc<Vec<SshOffer>>, at: usize, then: crate::write::After, cx: &mut Context<Self>) {
        let Some(r) = remotes.get(at).cloned() else {
            then(self, cx);
            return;
        };
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let next = ctx.clone();
        let w = Write::new(&ctx, move |_| call("switchRemoteToSsh", json!({ "repo": repo, "worktree": worktree, "remote": r.remote, "url": r.url }))).on_success(move |this, _, cx| this.switch_next(next.clone(), remotes.clone(), at + 1, then.clone(), cx));
        self.write(w, cx);
    }

    /// The run after the switch failed or was cancelled: says the remote now uses SSH, with Switch back.
    pub fn toast_after_switch(&mut self, ctx: &WriteCtx, remotes: Vec<SshOffer>, err: &GbError, title: Option<&str>, cx: &mut Context<Self>) {
        let names = remotes.iter().map(|r| r.remote.as_str()).collect::<Vec<_>>().join(", ");
        let this = cx.weak_entity();
        let back = ToastAction {
            label: "Switch back to HTTPS".into(),
            run: Rc::new({
                let (this, ctx, remotes) = (this.clone(), ctx.clone(), remotes.clone());
                move |_, cx| {
                    let (ctx, remotes) = (ctx.clone(), remotes.clone());
                    this.update(cx, |v, cx| v.switch_back(ctx, remotes, 0, cx)).ok();
                }
            }),
        };
        let Some((t, message)) = describe_error(err) else {
            let verb = if remotes.len() == 1 { "uses" } else { "use" };
            toast::show_with(format!("Cancelled; {names} now {verb} SSH"), Options { warning: true, actions: vec![back], ..Default::default() }, cx);
            return;
        };
        let command = err.command_id;
        let details = ToastAction {
            label: "Details".into(),
            run: Rc::new(move |window, cx| {
                this.update(cx, |v, cx| v.open_debug(command, window, cx)).ok();
            }),
        };
        toast::show_with(format!("{} after switching {names} to SSH: {message}", title.map_or(t, str::to_string)), Options { error: true, actions: vec![back, details], ..Default::default() }, cx);
    }

    fn switch_back(&mut self, ctx: WriteCtx, remotes: Vec<SshOffer>, at: usize, cx: &mut Context<Self>) {
        let Some(r) = remotes.get(at).cloned() else {
            let names = remotes.iter().map(|r| r.remote.as_str()).collect::<Vec<_>>().join(", ");
            toast::show(format!("{names} {} HTTPS again", if remotes.len() == 1 { "uses" } else { "use" }), cx);
            return;
        };
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let next = ctx.clone();
        let w = Write::new(&ctx, move |_| call("switchRemoteBack", json!({ "repo": repo, "worktree": worktree, "remote": r.remote }))).on_success(move |this, _, cx| this.switch_back(next.clone(), remotes.clone(), at + 1, cx));
        self.write(w, cx);
    }

    /// What a write from the active tab targets.
    pub fn write_ctx(&self) -> Option<WriteCtx> {
        let tab = self.tabs.get(self.active)?;
        Some(WriteCtx { repo: tab.id, worktree: tab.worktree.clone() })
    }
}

/// The lock's path inside the repository.
pub fn lock_label(path: &str) -> String {
    path.rfind("/.git/").map_or("index.lock".to_string(), |at| path[at + 1..].to_string())
}

/// Tests' stand-ins: a real repository under `target/bench/work/tmp`, deleted afterwards, with fictional
/// names and no access to the user's git config, and an `Api` over it. Plain `#[tokio::test]`s: GPUI's
/// scheduler can't await the runtime.
#[cfg(test)]
pub mod testing {
    use gitvolt_core::api::{Api, Request};
    use gitvolt_core::git::GitCli;
    use gitvolt_core::log::CommandLog;
    use serde_json::Value;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT: AtomicU32 = AtomicU32::new(0);

    pub struct Fixture {
        pub root: PathBuf,
        pub path: PathBuf,
        pub api: Arc<Api>,
        pub repo: u32,
        pub worktree: String,
    }

    pub fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git").args(args).current_dir(dir).env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1").output().expect("git runs");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim_end().to_string()
    }

    fn scratch() -> PathBuf {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).expect("the workspace");
        let root = workspace.join("target/bench/work/tmp/gpui-tests").join(format!("{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&root).expect("a scratch folder");
        root.canonicalize().expect("canonical")
    }

    impl Fixture {
        /// A repository with two commits on `main`: `a.txt` (ten lines) and `b.txt`.
        pub async fn new() -> Self {
            let root = scratch();
            let path = root.join("work");
            std::fs::create_dir_all(&path).expect("a repo folder");
            git(&path, &["init", "-q", "-b", "main"]);
            git(&path, &["config", "user.name", "Ada Fixture"]);
            git(&path, &["config", "user.email", "ada@example.com"]);
            let lines: String = (1..=10).map(|i| format!("line {i}\n")).collect();
            std::fs::write(path.join("a.txt"), lines).expect("a file");
            std::fs::write(path.join("b.txt"), "b\n").expect("a file");
            git(&path, &["add", "."]);
            git(&path, &["commit", "-qm", "Base"]);
            std::fs::write(path.join("b.txt"), "b\nb2\n").expect("a file");
            git(&path, &["add", "."]);
            git(&path, &["commit", "-qm", "Second"]);
            let env: Vec<(OsString, OsString)> = vec![("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()), ("GIT_CONFIG_NOSYSTEM".into(), "1".into())];
            let api = Arc::new(Api::new(GitCli::new(Arc::new(CommandLog::new(100))).with_env(env), None).with_data_dir(root.join("data")));
            let opened = api.dispatch(Request::OpenRepo { path: path.to_string_lossy().into_owned() }).await.expect("it opens");
            let repo = opened["id"].as_u64().expect("an id") as u32;
            let worktree = opened["worktree"].as_str().expect("a worktree").to_string();
            Self { root, path, api, repo, worktree }
        }

        pub fn git(&self, args: &[&str]) -> String {
            git(&self.path, args)
        }

        pub fn write(&self, file: &str, text: &str) {
            let at = self.path.join(file);
            if let Some(dir) = at.parent() {
                std::fs::create_dir_all(dir).expect("a folder");
            }
            std::fs::write(at, text).expect("a file");
        }

        pub fn read(&self, file: &str) -> String {
            std::fs::read_to_string(self.path.join(file)).expect("a file")
        }

        /// Sends what the UI would, as it would.
        pub async fn send(&self, body: Value) -> Result<Value, gitvolt_core::error::GbError> {
            self.api.dispatch(super::request_of(body)?).await
        }

        pub async fn ok(&self, body: Value) -> Value {
            let what = body.to_string();
            self.send(body).await.unwrap_or_else(|e| panic!("{what}: {}", e.message))
        }

        /// A bare repository under the fixture as `origin`, `main` pushed.
        pub fn bare_origin(&self) -> PathBuf {
            let bare = self.root.join("origin.git");
            std::fs::create_dir_all(&bare).expect("a folder");
            git(&bare, &["init", "-q", "--bare", "-b", "main"]);
            self.git(&["remote", "add", "origin", bare.to_str().expect("a path")]);
            self.git(&["push", "-q", "-u", "origin", "main"]);
            bare
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn err(kind: GbErrorKind, detail: Option<ErrorDetail>) -> GbError {
        GbError { kind, message: "m".into(), command_id: None, stderr: None, detail }
    }

    #[test]
    fn a_question_is_asked_once() {
        let e = err(GbErrorKind::DirtyWorktree, Some(ErrorDetail::AutostashConflict { paths: vec!["a".into(), "b".into()], target: "main".into() }));
        let q = question(&e, Asked::default()).expect("a question");
        assert_eq!(q.flag, Flag::Autostash);
        assert_eq!(q.req.arm.as_ref(), "Click again to continue: your changes go to a stash");
        assert!(q.req.body.contains("a (and 1 more)"));
        assert_eq!(question(&e, Asked { autostash: true, ..Default::default() }), None);
        assert_eq!(question(&err(GbErrorKind::Other, None), Asked::default()), None);
    }

    #[test]
    fn a_hard_reset_names_its_losses_and_is_red() {
        let e = err(GbErrorKind::DirtyWorktree, Some(ErrorDetail::ResetDiscards { branch: "main".into(), to: "abc".into(), files: 4 }));
        let q = question(&e, Asked::default()).expect("a question");
        assert_eq!(q.req.tone, Tone::Danger);
        assert_eq!(q.req.arm.as_ref(), "Click again to reset main and discard changes to 4 files");
    }

    #[test]
    fn errors_read_as_the_web_uis_and_a_cancel_says_nothing() {
        assert_eq!(describe_error(&err(GbErrorKind::Cancelled, None)), None);
        assert_eq!(describe_error(&err(GbErrorKind::RefMoved, None)).unwrap().1, "Branch changed outside GitVolt — refresh and retry");
        assert_eq!(describe_error(&err(GbErrorKind::HookFailed, Some(ErrorDetail::Hook { hook: "pre-commit".into() }))).unwrap().0, "pre-commit hook failed");
        let way = GbError { message: "sub is a repository in the way of the checkout: move it first".into(), ..err(GbErrorKind::InvalidInput, None) };
        assert_eq!(describe_error(&way).unwrap(), ("A repository is in the way".to_string(), "sub can't be replaced during the checkout: move it first".to_string()));
        assert_eq!(slots(GbErrorKind::IndexLocked), [Slot::RemoveLock, Slot::Retry]);
        assert_eq!(lock_label("/r/.git/index.lock"), ".git/index.lock");
    }

    #[test]
    fn a_request_reads_as_the_web_uis() {
        let r = request_of(call("stage", json!({ "repo": 1, "worktree": "/w", "paths": ["a"] }))).unwrap();
        assert!(matches!(r, Request::Stage { repo: 1, .. }));
        assert!(request_of(call("nonsense", json!({}))).is_err());
    }
}
