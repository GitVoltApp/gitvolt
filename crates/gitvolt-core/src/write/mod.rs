//! Writes. Every write goes through `run_write`, one sequence the core owns:
//! 1 queue and lock, 2 preflight (read-only), 3 journal write-ahead, 4 snapshot, 5 autostash,
//! 6 run, 7 verify, 8 restore, 9 journal finalize, 10 events and caches. Reads never build a
//! write invocation (`WriteToken`).

pub(crate) mod commit;
pub(crate) mod branch;
// --- conflicted files ---
pub(crate) mod conflict;
pub(crate) mod files;
pub(crate) mod gitflow;
// --- modules ---
pub(crate) mod config;
pub(crate) mod names;
pub(crate) mod discard;
// --- delete a branch ---
pub(crate) mod branch_delete;
// --- checkout ---
pub(crate) mod checkout;
pub(crate) mod hooks;
pub(crate) mod index_lock;
pub(crate) mod patch;
pub(crate) mod precheck;
pub(crate) mod queue;
pub(crate) mod refs;
pub(crate) mod remotes;
pub(crate) mod rewrites;
// --- modules ---
pub(crate) mod reset;
pub(crate) mod restore;
pub mod lfs;
pub(crate) mod progress;
pub mod remote_output;
pub(crate) mod stage;
// --- push ---
pub(crate) mod sync;
// --- tags ---
pub(crate) mod tags;
pub(crate) mod worktree;
// --- integrate ---
pub mod integrate;
pub mod rebase;
pub mod irebase;
pub mod pick;
// --- cherry-pick and revert ---
pub(crate) mod sequence;
pub(crate) mod stage_patch;
pub(crate) mod submodules;
// --- stashes ---
pub(crate) mod stash;
#[cfg(any(test, feature = "testing"))]
pub mod test_intents;
#[cfg(test)]
mod tests;
#[cfg(test)]
#[cfg_attr(test, allow(dead_code))]
pub(crate) mod test_support;
mod token;
pub mod types;

pub(crate) use token::WriteToken;

use crate::api::{blocking, Api, RepoHandle};
use crate::error::{gix_err, GbError, GbErrorKind};
use crate::events::{AppEvent, ChangeKind, EventBus, OpKind, OpOutcome};
use crate::git::{GitInvocation, GitOutput};
use crate::journal::snapshot::{self, SnapshotCx};
use crate::journal::{EntryState, HeadState, JournalStore, KeptReason, NewEntry, PausedKind, PausedOp, RefMove, Snapshot, UndoKind};
use crate::ops::OpEntry;
use crate::write::queue::RepoWrites;
use crate::write::types::{Expect, WipListsPayload, WriteResult};
use gix::bstr::ByteSlice;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Queued writes run one at a time in click order. Immediate ones (stage, discards, …) act on
/// what's on screen: they take the lock directly, waiting at most for the running item's local
/// phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteClass {
    Queued,
    #[cfg_attr(not(any(test, feature = "testing")), allow(dead_code))] // the app's first: stage and discards
    Immediate,
}

/// How a write touches the staging undo log; `run_write` acts on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Staging {
    /// One stage or unstage: recorded as a step, `write-tree` before and after.
    Step,
    /// Leaves the index alone (saving a file, a worktree-only discard): the log stays.
    Keep,
    /// Staging undo and redo, which move steps themselves, and a conflict resolution, which
    /// records its own step (its path's stages and file, `journal::resolve_step`).
    Own,
    /// Moves HEAD or rewrites the index (commit, checkout, reset, stash, merge, a journal undo):
    /// the log is cleared. The default, so a new intent can't leave a stale log behind.
    Clear,
}

/// What a write needs before it runs, from its read-only `plan`.
#[derive(Debug, Default)]
pub(crate) struct Plan {
    /// Snapshot these first: `(paths, the untracked ones the op deletes)`. A directory
    /// stands for its files; the snapshot's own `paths` are what `run` acts on.
    pub snapshot: Option<(Vec<String>, Vec<String>)>,
    /// Autostash before running, decided by preflight.
    pub autostash: Option<crate::journal::autostash::AutostashSpec>,
}

/// What preflight read.
#[derive(Debug, Clone)]
pub(crate) struct Before {
    pub head: HeadState,
    /// `expect`'s refs, the intent's `refs()` and HEAD's branch, as they are now.
    pub refs: BTreeMap<String, Option<String>>,
    pub in_progress: Option<&'static str>,
}

/// An intent's view during `plan` (read-only).
#[allow(dead_code)] // `h` and `expect`: first readers: the stage and discard intents
pub(crate) struct Pre<'a> {
    pub api: &'a Api,
    pub h: &'a Arc<RepoHandle>,
    pub root: &'a Path,
    pub expect: &'a Expect,
    pub before: &'a Before,
}

// --- network phases ---
/// What a network phase works with: the queue's running slot only, never the write lock
/// or the watcher hold. git updates `refs/remotes/<r>/<b>` itself, at the end of the transfer,
/// under its own ref lock; nothing that can run meanwhile writes there.
#[allow(dead_code)] // `api`, `h`, `root`, `token`: first readers: pull
pub(crate) struct NetCx<'a> {
    pub api: &'a Api,
    pub h: &'a Arc<RepoHandle>,
    pub root: &'a Path,
    pub op: &'a OpEntry,
    pub token: WriteToken,
    out: mpsc::UnboundedSender<String>,
}

/// A network git command in `root`: never the `ext::` transport, no timeout, cancellable,
/// detached with askpass, its stderr streamed to Activity. It may lazy-fetch. A write
/// invocation, so a Cancel is SIGTERM to git's process group, then SIGKILL after the grace: git
/// removes its ref `.lock` files first.
fn network_invocation(token: &WriteToken, api: &Api, root: &Path, op: &OpEntry, out: mpsc::UnboundedSender<String>, args: Vec<OsString>) -> GitInvocation {
    let argv: Vec<OsString> = crate::netops::NO_EXT.iter().map(OsString::from).chain(args).collect();
    GitInvocation::network_write(token, root, argv).timeout(None).cancel(op.cancel.clone()).detach_terminal().stream_stderr(out).envs(api.net_env(op.id)).env("GIT_NO_LAZY_FETCH", "0")
}

#[allow(dead_code)] // `git`: first caller: pull
impl NetCx<'_> {
    pub(crate) fn git<I, S>(&self, args: I) -> GitInvocation
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        network_invocation(&self.token, self.api, self.root, self.op, self.out.clone(), args.into_iter().map(Into::into).collect())
    }

    pub(crate) fn output(&self) -> mpsc::UnboundedSender<String> {
        self.out.clone()
    }
}

/// One write operation (one request per operation; TypeScript only asks).
pub(crate) trait WriteIntent: Send + Sync {
    type Outcome: Serialize + Send;
    fn kind(&self) -> OpKind;
    /// The journal and Activity label (`commit "Fix x"`, `checkout feature/x`, …).
    fn label(&self) -> String;
    fn class(&self) -> WriteClass {
        WriteClass::Queued
    }
    /// How it's undone; `None` = not journaled.
    fn undo(&self) -> Option<UndoKind>;
    /// It may run hooks, sign or reach the network: no timeout, cancellable, trace2.
    fn runs_hooks(&self) -> bool {
        false
    }
    // --- hook tracing ---
    /// A hook-running intent gets a trace2 event file, so a failure names its hook. A rebase
    /// opts out (a 60-commit rebase writes ~650 KB of events): a hook that fails
    /// mid-rebase stops it (the pause shows that), and `pre-rebase` says so on stderr.
    fn traces_hooks(&self) -> bool {
        self.runs_hooks()
    }
    /// It may run during a merge or rebase (stage, discard per file, save, resolve).
    fn allowed_in_progress(&self) -> bool {
        false
    }
    /// It does nothing but settle a paused merge or rebase (`SettlePaused`). Any other
    /// write whose settle brought an autostash back fails with Stale before it acts.
    fn only_settles(&self) -> bool {
        false
    }
    /// Refs it may move besides HEAD's branch; verify compares them.
    fn refs(&self) -> Vec<String> {
        Vec::new()
    }
    /// What the user already confirmed (the clean-restore warning).
    fn confirm(&self) -> crate::write::types::Confirm {
        Default::default()
    }
    /// A network transfer before the local phases (pull's fetch): it runs after
    /// the queue slot is taken, outside the write lock and the watcher hold. A failure ends the
    /// write before preflight: nothing is journaled.
    fn transfer_first(&self, _net: &mut NetCx<'_>) -> impl Future<Output = Result<(), GbError>> + Send {
        async { Ok(()) }
    }
    /// It rewrites the branch it moves (rebase, amend): a pushed branch it rewrites may get a
    /// rewrite mark, so the next Push forces with the lease recorded now.
    fn rewrite(&self) -> Option<rewrites::RewriteKind> {
        None
    }
    /// The staging undo log's part in this write.
    #[allow(dead_code)] // read in `run_write`
    fn staging(&self) -> Staging {
        Staging::Clear
    }
    /// A done entry of the same label just below this one, whose `after` snapshot holds
    /// exactly what this one's `before` does (nothing journaled or changed in between), takes
    /// this write's `after` instead of the write getting an entry of its own (`Journal::coalesce`).
    fn coalesces(&self) -> bool {
        false
    }
    fn plan(&self, _pre: &Pre<'_>) -> impl Future<Output = Result<Plan, GbError>> + Send {
        async { Ok(Plan::default()) }
    }
    fn run(&self, cx: &mut WriteCx<'_>) -> impl Future<Output = Result<Self::Outcome, GbError>> + Send;
}

/// What an intent's `run` works with.
pub(crate) struct WriteCx<'a> {
    pub api: &'a Api,
    pub h: &'a Arc<RepoHandle>,
    /// The worktree (canonical).
    pub root: &'a Path,
    /// As the user saw it, carried forward by the queue.
    #[allow(dead_code)] // first readers: the stage and discard intents
    pub expect: &'a Expect,
    pub before: &'a Before,
    pub token: WriteToken,
    pub op: &'a OpEntry,
    /// The data dir's `tmp/` (0700).
    pub tmp: PathBuf,
    out: mpsc::UnboundedSender<String>,
    /// The intent `runs_hooks`: no timeout.
    hooks: bool,
    trace: Option<hooks::Trace2>,
    kinds: BTreeSet<ChangeKind>,
    /// Refs the write moved itself, with the value its CAS required them to have before.
    touched: BTreeMap<String, Option<String>>,
    /// The `before` snapshot step 4 took (the plan's paths). A destructive intent deletes only
    /// what it holds, never a path that appeared since.
    pub snapshot: Option<Snapshot>,
    /// The `after` snapshot (discards), kept only when the run succeeds.
    pub after: Option<Snapshot>,
    /// The run may have changed the working tree before it failed: a failed write keeps its
    /// entry (and `before` snapshot) only then, or when a ref or HEAD moved.
    pub partial: bool,
    /// Finalizing this write's entry leaves the Redo stack (an out-of-order undo
    /// independent of every redo entry).
    pub keep_redo: bool,
    /// The intent changed the journal itself (undo, redo, banners): announce it.
    pub journal_changed: bool,
    // --- the entry, the lock and holds travel with the write ---
    /// This write's journal entry, written ahead (`None`: not journaled, or not yet: step 3).
    journal: Option<(&'a crate::journal::JournalStore, u64)>,
    /// Steps 5–8 have the worktree stashed: `unlock` (and so `network`) must not run then
    /// (immediate writes would act on the stashed worktree while it waits).
    autostashed: bool,
    writes: Arc<RepoWrites>,
    /// The write lock, while held (`unlock` drops it for a network transfer).
    lock: Option<tokio::sync::OwnedMutexGuard<()>>,
    /// The watcher holds, while held (a transfer releases them too).
    holds: Vec<crate::watch::WatchHold>,
    /// The run stopped on conflicts (a merge or rebase): steps 8 and 9 leave its
    /// autostash and its entry waiting.
    pub paused: Option<Pause>,
    /// A repair step (`run_repair`) was stopped: the rest are skipped.
    repair_stopped: bool,
}

impl WriteCx<'_> {
    /// A write in this worktree: cancellable, detached, with askpass, its stderr streamed to
    /// Activity. Hook-running intents get no timeout and a trace2 file.
    pub(crate) fn git<I, S>(&self, args: I) -> GitInvocation
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let mut inv = GitInvocation::write(&self.token, self.root, args).cancel(self.op.cancel.clone()).detach_terminal().stream_stderr(self.out.clone()).envs(self.api.net_env(self.op.id));
        if self.hooks {
            inv = inv.timeout(None);
        }
        if let Some(t) = &self.trace {
            inv = inv.envs(t.env());
        }
        inv
    }

    /// An autostash step's invocation (push, apply, drop): `run_stash_step` runs it.
    pub(crate) fn git_stash<I, S>(&self, args: I) -> GitInvocation
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        GitInvocation::write(&self.token, self.root, args).detach_terminal().stream_stderr(self.out.clone())
    }

    // --- the put-back after an interrupted move ---
    /// A write that puts the worktree back after a failed or cancelled step (a reverse
    /// read-tree, `checkout::put_back`). `run_repair` runs it.
    pub(crate) fn git_repair<I, S>(&self, args: I) -> GitInvocation
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        GitInvocation::write(&self.token, self.root, args).detach_terminal().stream_stderr(self.out.clone())
    }

    /// Runs a repair step as an autostash step (`run_stash_step`, "Restoring files…"): the op's
    /// Cancel doesn't stop it (a Cancel may be what brought it here), but a Cancel pressed while
    /// it runs is a Stop, and `api.autostash_timeout` is a hard limit. Once one is stopped, the
    /// write keeps its entry (`partial`) and every later repair step is skipped:
    /// a hung smudge filter never holds the queue past the user's Stop.
    pub(crate) async fn run_repair(&mut self, inv: GitInvocation) -> Result<GitOutput, GbError> {
        if self.repair_stopped {
            return Err(GbError::other("Stopped restoring files"));
        }
        let (res, stopped) = self.run_stash_step(crate::events::StashStep::RestoringFiles, "restore files", inv).await;
        if stopped {
            self.repair_stopped = true;
            self.partial = true;
            return Err(GbError::other("Stopped restoring files"));
        }
        res
    }

    /// A repair step was stopped (`run_repair`).
    pub(crate) fn repair_stopped(&self) -> bool {
        self.repair_stopped
    }

    /// Runs an autostash step. A Cancel of the op doesn't stop it (the user's changes
    /// are restored whatever the run did), but a Cancel pressed while it runs is a Stop, and
    /// `api.autostash_timeout` (15 minutes) is a hard limit: both stop git's whole process group
    /// (SIGTERM, then SIGKILL), filters included. `true`: it was stopped.
    pub(crate) async fn run_stash_step(&mut self, step: crate::events::StashStep, message: &str, inv: GitInvocation) -> (Result<GitOutput, GbError>, bool) {
        let (api, op) = (self.api, self.op);
        let limit = api.autostash_timeout;
        let stop = tokio_util::sync::CancellationToken::new();
        let pressed = op.stop_requested.notified();
        tokio::pin!(pressed);
        pressed.as_mut().enable();
        api.bus.emit(AppEvent::OpStashStep { op: op.id, step: Some(step), message: message.to_string() });
        let started = std::time::Instant::now();
        let run = api.cli.run(inv.cancel(stop.clone()).timeout(Some(limit)));
        tokio::pin!(run);
        let res = loop {
            tokio::select! {
                r = &mut run => break r,
                _ = &mut pressed, if !stop.is_cancelled() => stop.cancel(),
            }
        };
        api.bus.emit(AppEvent::OpStashStep { op: op.id, step: None, message: message.to_string() });
        let stopped = res.is_err() && (stop.is_cancelled() || started.elapsed() >= limit);
        (res, stopped)
    }

    /// Runs it; a hook that exited non-zero makes the failure `HookFailed { hook }`.
    pub(crate) async fn run_git(&mut self, inv: GitInvocation) -> Result<GitOutput, GbError> {
        let res = self.api.cli.run(inv).await;
        res.map_err(|e| match &self.trace {
            Some(t) => hooks::hook_error(e, t),
            None => e,
        })
    }

    /// A ref CAS; verify records what really moved.
    pub(crate) async fn cas(&mut self, moves: &[RefMove], message: &str) -> Result<(), GbError> {
        for m in moves {
            self.touched.entry(m.name.clone()).or_insert_with(|| m.old.clone());
        }
        refs::cas(&self.api.cli, &self.token, &self.h.repo, self.root, moves, message).await
    }

    pub(crate) fn snapshots(&self) -> SnapshotCx<'_> {
        SnapshotCx { cli: &self.api.cli, token: &self.token, root: self.root, tmp: &self.tmp }
    }

    pub(crate) fn touch(&mut self, kind: ChangeKind) {
        self.kinds.insert(kind);
    }

    /// This write's journal entry, when it's journaled ("Stage all & commit" records its
    /// `index_before` there).
    pub(crate) fn entry(&self) -> Option<(&JournalStore, u64)> {
        self.journal
    }

    // --- journal helpers and the network step ---
    /// Edits this write's (pending) journal entry; nothing when it isn't journaled.
    #[allow(dead_code)] // replace `before`, a stash oid on redo
    pub(crate) fn edit_entry(&mut self, f: impl FnOnce(&mut crate::journal::JournalEntry)) -> Result<(), GbError> {
        let Some((store, id)) = self.journal else { return Ok(()) };
        store.update(|j| match j.entry_mut(id) {
            Some(e) => f(e),
            None => tracing::warn!(target: "gitvolt_core::write", "journal entry {id} is gone: this edit is lost"),
        })
    }

    /// The run found the operation is another kind (a checkout that moved the checked-out
    /// branch instead of HEAD is a Rewind).
    #[allow(dead_code)] // first reader: checkout
    pub(crate) fn set_undo(&mut self, undo: UndoKind) -> Result<(), GbError> {
        self.edit_entry(|e| e.undo = undo)
    }

    /// `branch.<name>.*` keys the run changed: undo and redo replay them. A
    /// change already made: a write that fails after it keeps its entry (`partial`).
    #[allow(dead_code)] // first readers: the test intents
    pub(crate) fn record_config(&mut self, changes: Vec<crate::journal::ConfigChange>) -> Result<(), GbError> {
        if changes.is_empty() {
            return Ok(());
        }
        self.partial = true;
        self.touch(ChangeKind::Config);
        self.edit_entry(|e| e.config.extend(changes))
    }

    /// A stash the run created or dropped. Like `record_config`, a write
    /// that fails after it keeps its entry (`partial`).
    #[allow(dead_code)] // first reader: stashes
    pub(crate) fn record_stash(&mut self, m: crate::journal::StashMove) -> Result<(), GbError> {
        self.partial = true;
        self.touch(ChangeKind::Stash);
        self.edit_entry(|e| e.stashes.push(m))
    }

    /// What Undo's toast adds after the label.
    #[allow(dead_code)] // first readers: the test intents; Delete Both
    pub(crate) fn set_note(&mut self, note: String) -> Result<(), GbError> {
        self.edit_entry(|e| e.note = Some(note))
    }

    /// A done Barrier under this write's entry: the run made a push.
    #[allow(dead_code)] // first reader: Delete Both
    pub(crate) fn barrier_below(&mut self, label: &str, kind: OpKind) -> Result<(), GbError> {
        let Some((store, id)) = self.journal else { return Ok(()) };
        let (head, now) = (self.before.head.clone(), self.api.now());
        store.update(|j| j.barrier_before(id, label.to_string(), kind, head, now))?;
        self.journal_changed = true;
        Ok(())
    }


    // --- network phases ---
    /// A network transfer starts (push): the write lock and the watcher hold go, so stage
    /// and discards run meanwhile. Only the queue's running slot is held. `relock` before any
    /// local step; `steps` relocks after `run` regardless. Never with an autostash out or a
    /// snapshot taken: stage and discards would then run against the stashed worktree.
    pub(crate) fn unlock(&mut self) {
        debug_assert!(!self.autostashed && self.snapshot.is_none(), "a write unlocked with an autostash or snapshot active");
        self.lock = None;
        self.holds.clear();
    }

    /// Takes the write lock and the watcher hold again after `unlock` (no-op when held).
    pub(crate) async fn relock(&mut self) {
        if self.lock.is_none() {
            self.lock = Some(self.writes.acquire().await);
            self.holds = self.api.watch_holds(&self.h.common_dir);
        }
    }

    /// One transfer outside the lock (a remote delete, a push): `inv` runs with the write
    /// lock and the watcher hold released, both taken again before it returns. It never runs
    /// after step 5 autostashed (immediate writes would act on the stashed worktree meanwhile):
    /// an intent that transfers doesn't autostash, or transfers before the lock (pull).
    #[allow(dead_code)] // first callers: remote delete, push
    pub(crate) async fn network(&mut self, inv: GitInvocation) -> Result<GitOutput, GbError> {
        self.unlock();
        let res = self.run_git(inv).await;
        self.relock().await;
        self.transferred(res)
    }

    /// `network`, keeping the write lock and the watcher hold: a transfer whose command writes
    /// the index too (`git submodule add` stages after its clone), which a stage or discard
    /// meanwhile would meet on `index.lock`, leaving it half done.
    pub(crate) async fn network_locked(&mut self, inv: GitInvocation) -> Result<GitOutput, GbError> {
        let res = self.run_git(inv).await;
        self.transferred(res)
    }

    fn transferred(&self, mut res: Result<GitOutput, GbError>) -> Result<GitOutput, GbError> {
        // --- network server output ---
        // The transfer's `remote:` lines: `opRemote` for Activity, and first in a failure's Details.
        remote_output::capture(self.api, self.op.id, &mut res);
        // A cancelled credential prompt is a Cancel, as for a fetch.
        res.map_err(|e| crate::netops::user_cancelled(e, self.op))
    }

    /// The remote the next transfer talks to, for its credential prompts.
    pub(crate) fn note_remote(&self, remote: &str, direction: gix::remote::Direction) {
        crate::netops::note_op_remotes(self.op, self.root, Some(&[remote.to_string()]), direction);
    }

    /// `git`, as a network command: `NO_EXT`, no timeout, lazy fetch allowed, trace2 for hooks.
    #[allow(dead_code)] // first caller: push
    pub(crate) fn net_git<I, S>(&self, args: I) -> GitInvocation
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let inv = network_invocation(&self.token, self.api, self.root, self.op, self.out.clone(), args.into_iter().map(Into::into).collect());
        match &self.trace {
            Some(t) => inv.envs(t.env()),
            None => inv,
        }
    }

    /// `git`, with stderr sent to `tx` instead of Activity (a progress tap forwards the rest).
    pub(crate) fn git_to<I, S>(&self, args: I, tx: mpsc::UnboundedSender<String>) -> GitInvocation
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.git(args).stream_stderr(tx)
    }

    /// The op's Activity output.
    pub(crate) fn output(&self) -> mpsc::UnboundedSender<String> {
        self.out.clone()
    }

    // --- refs a git command may move ---
    /// Refs git itself may move (a rebase's stacked branches), with their values now, read
    /// under the lock when the op runs: verify (and a pause's `refs_before`) records exactly these
    /// besides HEAD's branch.
    pub(crate) fn watch_refs(&mut self, refs: impl IntoIterator<Item = (String, Option<String>)>) {
        for (name, old) in refs {
            self.touched.entry(name).or_insert(old);
        }
    }

    /// A watched ref the op didn't move after all (someone else did): verify leaves it out.
    pub(crate) fn unwatch_ref(&mut self, name: &str) {
        self.touched.remove(name);
    }
}

/// HEAD as the journal records it.
pub(crate) fn head_state(repo: &gix::Repository) -> Result<HeadState, GbError> {
    let head = repo.head().map_err(gix_err)?;
    let branch = head.referent_name().and_then(|n| n.as_bstr().to_str().ok()?.strip_prefix("refs/heads/").map(str::to_string));
    Ok(HeadState { branch, oid: head.id().map(|id| id.to_string()) })
}

/// The operations that block writes; a bisect doesn't.
pub(crate) fn in_progress_name(s: gix::state::InProgress) -> Option<&'static str> {
    use gix::state::InProgress::*;
    match s {
        ApplyMailbox | ApplyMailboxRebase => Some("am"),
        CherryPick | CherryPickSequence => Some("cherry-pick"),
        Merge => Some("merge"),
        Rebase | RebaseInteractive => Some("rebase"),
        Revert | RevertSequence => Some("revert"),
        Bisect => None,
    }
}

fn read_all(repo: &gix::Repository, names: &BTreeSet<String>) -> Result<BTreeMap<String, Option<String>>, GbError> {
    names.iter().map(|n| Ok((n.clone(), refs::read_ref(repo, n)?))).collect()
}

/// Preflight, with gix only (no git process, so nothing can write).
async fn preflight(root: &Path, expect: &Expect, extra: Vec<String>, allowed_in_progress: bool) -> Result<Before, GbError> {
    let (root, expect) = (root.to_path_buf(), expect.clone());
    blocking(move || {
        let repo = gix::open(&root).map_err(gix_err)?;
        let in_progress = repo.state().and_then(in_progress_name);
        if let Some(what) = in_progress
            && !allowed_in_progress
        {
            return Err(GbError::in_progress(what));
        }
        let head = head_state(&repo)?;
        if expect.head.is_some() && head.oid != expect.head {
            return Err(GbError::ref_moved("HEAD"));
        }
        let mut names: BTreeSet<String> = expect.refs.keys().cloned().collect();
        names.extend(extra);
        if let Some(b) = &head.branch {
            names.insert(format!("refs/heads/{b}"));
        }
        let refs = read_all(&repo, &names)?;
        for (name, want) in &expect.refs {
            if refs.get(name) != Some(want) {
                return Err(GbError::ref_moved(name));
            }
        }
        Ok(Before { head, refs, in_progress })
    })
    .await
}

/// HEAD and the given refs (plus HEAD's branch) as they are now.
async fn observe(root: &Path, mut names: BTreeSet<String>) -> Result<(HeadState, BTreeMap<String, Option<String>>), GbError> {
    let root = root.to_path_buf();
    blocking(move || {
        let repo = gix::open(&root).map_err(gix_err)?;
        let head = head_state(&repo)?;
        if let Some(b) = &head.branch {
            names.insert(format!("refs/heads/{b}"));
        }
        Ok((head, read_all(&repo, &names)?))
    })
    .await
}

// --- the pause ---
/// A merge or rebase stopped on conflicts, set by the intent's `run`: steps 8 and 9
/// leave its autostash and its entry waiting for Commit, Continue or Abort.
#[derive(Debug, Clone)]
pub(crate) struct Pause {
    pub kind: PausedKind,
    /// What it integrates, as the user named it.
    pub target: String,
    // --- the target's oid ---
    /// What the target was when it stopped (a rebase's `onto`, a merge's `MERGE_HEAD`): settle
    /// judges completion against it.
    pub target_oid: Option<String>,
    /// `PausedOp::put_back`.
    pub put_back: Vec<(String, String)>,
    /// `PausedOp::picked`.
    pub picked: Vec<String>,
    /// An interactive rebase's session, for `PausedOp::irebase`.
    pub irebase: Option<crate::journal::IrebaseState>,
}

/// `SettlePaused`: runs nothing itself. A paused op that ended outside GitVolt
/// settles in the step that follows preflight (2b), since this write finds nothing in progress;
/// one still in progress is left waiting.
pub(crate) struct SettleIntent;

impl WriteIntent for SettleIntent {
    fn only_settles(&self) -> bool {
        true
    }
    type Outcome = ();
    fn kind(&self) -> OpKind {
        OpKind::Resolve
    }
    fn label(&self) -> String {
        "finish a merge or rebase".into()
    }
    fn undo(&self) -> Option<UndoKind> {
        None
    }
    fn allowed_in_progress(&self) -> bool {
        true
    }
    async fn run(&self, _cx: &mut WriteCx<'_>) -> Result<(), GbError> {
        Ok(())
    }
}

/// The write that may complete the paused op: its journal entry and the refs it moved itself.
struct Completing<'m> {
    entry: Option<u64>,
    moves: &'m [RefMove],
}

/// Settles the worktree's paused merge, rebase, cherry-pick or revert if it's no longer in
/// progress, at one of two points:
/// - right after preflight, when this write found no operation in progress (it ended outside
///   GitVolt): the pause settles against that state, before this write changes anything;
/// - step 7b, after verify, when this write started mid-operation (`completing`): the Commit or
///   Continue that ends it. Its own entry is absorbed into the pause only if it
///   made the completing commit itself and holds no snapshot.
///
/// It's completed only when HEAD, on its branch, is a merge commit whose first parent is the
/// old tip (a rebase: the branch moved onto its target; a cherry-pick or revert: the branch
/// gained only the pick's own commits, see `completed`); anything else is an abort outside
/// GitVolt, and the entry goes. Its autostash is restored either way.
/// A pick that failed part-way outside GitVolt leaves `.git/sequencer/`
/// with nothing in progress; settling drops it (`--quit`, the commits stay).
async fn settle_paused(cx: &mut WriteCx<'_>, completing: Option<Completing<'_>>) -> Settled {
    match settle_paused_inner(cx, completing).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(target: "gitvolt_core::write", "settling a paused operation: {e}");
            Settled::default()
        }
    }
}

/// What a settle did.
#[derive(Default)]
struct Settled {
    /// The error a Stop during the autostash's restore makes: it fails the write, as at step 8.
    stop: Option<GbError>,
    /// The paused op's label, when its autostash was restored (the worktree changed).
    restored: Option<String>,
}

async fn settle_paused_inner(cx: &mut WriteCx<'_>, completing: Option<Completing<'_>>) -> Result<Settled, GbError> {
    let store = cx.api.journal(cx.root)?;
    let Some(entry) = store.load()?.paused().cloned() else { return Ok(Settled::default()) };
    let Some(op) = entry.paused.clone() else { return Ok(Settled::default()) };
    let root = cx.root.to_path_buf();
    let still = blocking(move || Ok::<_, GbError>(gix::open(&root).map_err(gix_err)?.state().and_then(in_progress_name))).await?;
    if still.is_some() {
        return Ok(Settled::default());
    }
    // --- a pick's sequencer leftover ---
    let quit = match op.kind {
        PausedKind::CherryPick => Some("cherry-pick"),
        PausedKind::Revert => Some("revert"),
        _ => None,
    };
    // One whose todo still lists commits is the user's to go on with.
    if let Some(what) = quit
        && !crate::write::sequence::sequencer_has_todo(cx.root).await
    {
        crate::write::sequence::quit_leftover_sequencer(cx, what).await;
    }
    let (head_after, now) = observe(cx.root, op.refs_before.keys().cloned().collect()).await?;
    let completed = completed(cx, &entry, &op, &now, completing.is_some()).await;
    let (head_after, moves) = if completed {
        let moves: Vec<RefMove> = now
            .iter()
            .filter_map(|(name, new)| {
                let old = op.refs_before.get(name)?.clone();
                (old != *new).then(|| RefMove { name: name.clone(), old, new: new.clone() })
            })
            .collect();
        // A ref besides the branch counts only if the op moved it into the
        // new history (a stacked branch `--update-refs` moved). One that moved elsewhere during
        // the pause (a commit in another worktree) is someone else's, never this entry's.
        let branch_name = entry.head_before.branch.as_ref().map(|b| format!("refs/heads/{b}"));
        let tip = branch_name.as_ref().and_then(|b| now.get(b).cloned().flatten());
        let root = cx.root.to_path_buf();
        let moves = blocking(move || {
            let repo = gix::open(&root).map_err(gix_err)?;
            let tip = tip.and_then(|t| gix::ObjectId::from_hex(t.as_bytes()).ok());
            Ok(moves
                .into_iter()
                .filter(|m| {
                    Some(&m.name) == branch_name.as_ref()
                        || m.new.as_deref().and_then(|n| gix::ObjectId::from_hex(n.as_bytes()).ok()).zip(tip).is_some_and(|(n, t)| is_ancestor(&repo, n, t))
                })
                .collect::<Vec<_>>())
        })
        .await?;
        (head_after, moves)
    } else {
        (entry.head_before.clone(), Vec::new())
    };
    // The moves an interactive rebase's Continue recorded as it made them
    // (its chip deletes, the chips git's `update-ref` lines moved) are this entry's in either
    // verdict: a completion that left the branch where it was still deleted or moved them, and
    // the Undo must put them back.
    let mut moves = moves;
    for m in op.irebase.iter().flat_map(|s| s.moved.iter()) {
        if !moves.iter().any(|x| x.name == m.name) {
            moves.push(m.clone());
        }
    }
    let branch = entry.head_before.branch.as_ref().map(|b| format!("refs/heads/{b}"));
    let absorb = completing.filter(|_| completed && cx.snapshot.is_none() && cx.after.is_none()).and_then(|c| {
        let made_it = c.moves.iter().any(|m| Some(&m.name) == branch.as_ref() && m.new == head_after.oid);
        c.entry.filter(|_| made_it)
    });
    // A rebase that completed is a rewrite, whichever write ended it.
    if completed && op.kind == PausedKind::Rebase {
        rewrites::record(cx.api, &cx.h.common_dir, cx.root, Some(rewrites::RewriteKind::Rebase), &moves).await;
    }
    let owner = cx.api.owner()?;
    let (found, claimed) = store.update(|j| {
        // Only the entry this load saw, still paused (another instance may have
        // settled it meanwhile), and only a stash that still waits on it.
        if j.paused().map(|e| e.id) != Some(entry.id) {
            return (false, None);
        }
        if let Some(id) = absorb
            && j.entry_mut(id).is_some_and(|e| e.before.is_none() && e.after.is_none())
        {
            j.drop_entry(id);
        }
        j.settle(entry.id, head_after, moves);
        let claimed = op.autostash.and_then(|id| j.kept_mut(id)).filter(|k| k.reason == KeptReason::Paused).map(|k| {
            k.reason = KeptReason::Pending;
            k.owner = Some(owner);
            k.clone()
        });
        (true, claimed)
    })?;
    cx.journal_changed |= found;
    // The rebase is over (completed or aborted): its session goes.
    if found && let Some(s) = &op.irebase {
        crate::write::irebase::run::remove_session(std::path::Path::new(&s.dir));
    }
    let Some(k) = claimed else { return Ok(Settled::default()) };
    let stop = crate::journal::autostash::restore_paused(cx, k).await;
    Ok(Settled { stop, restored: Some(entry.label) })
}

/// Whether the paused op completed (rather than being aborted, perhaps followed by
/// other work, outside GitVolt). Judged on the branch's ref as it is now
/// (a completion followed by an outside checkout still counts), against the target's oid
/// recorded at pause time (a target that moved during the pause doesn't turn a completion into
/// an abort):
/// - a merge: the branch's tip is a merge commit whose parents are the old tip and the target;
/// - a rebase: the branch was rewritten onto the target. The target is an ancestor of the new
///   tip, and the old tip isn't (a merge or fast-forward of the target keeps it). Ended outside
///   GitVolt (`inside` false), a tip equal to the target is an abort and a reset; a Skip of every
///   commit in GitVolt leaves it there too, and that's a completion.
///   And every commit the new tip has beyond the target is a rewrite of one of
///   the old tip's. A rebase keeps each commit's author (email and time), even through a
///   conflict resolution, a message cleanup or a `prepare-commit-msg` hook; an abort, a reset and
///   new work makes commits of its own, so it counts as an abort. (A commit re-authored during
///   the pause also does: the entry goes and nothing is undone, never the reverse.)
async fn completed(cx: &WriteCx<'_>, entry: &crate::journal::JournalEntry, op: &PausedOp, now: &BTreeMap<String, Option<String>>, inside: bool) -> bool {
    let Some(branch) = entry.head_before.branch.clone() else { return false };
    let name = format!("refs/heads/{branch}");
    let (Some(Some(old)), Some(Some(new))) = (op.refs_before.get(&name).cloned(), now.get(&name).cloned()) else { return false };
    if old == new {
        return false;
    }
    let root = cx.root.to_path_buf();
    let (kind, target, target_oid, picked) = (op.kind, op.target.clone(), op.target_oid.clone(), op.picked.clone());
    // Commits made at an interactive rebase's Edit stop (its pieces: Continue's commit there).
    let made = op.irebase.as_ref().map(|s| s.made.clone()).unwrap_or_default();
    blocking(move || {
        let repo = gix::open(&root).map_err(gix_err)?;
        let oid = |hex: &str| gix::ObjectId::from_hex(hex.as_bytes()).map_err(gix_err);
        let (old, new) = (oid(&old)?, oid(&new)?);
        let target_oid = target_oid.as_deref().map(oid).transpose()?;
        Ok(match kind {
            PausedKind::Merge => {
                let parents: Vec<gix::ObjectId> = repo.find_commit(new).map_err(gix_err)?.parent_ids().map(|p| p.detach()).collect();
                // An entry without a target oid: any merge commit on the old tip.
                parents.len() >= 2 && parents[0] == old && target_oid.is_none_or(|t| parents[1..].contains(&t))
            }
            PausedKind::Rebase => {
                // Without one: the target ref as it is now.
                let Some(target) = target_oid.or_else(|| repo.rev_parse_single(target.as_str()).ok().map(|t| t.detach())) else { return Ok(false) };
                if !((inside || new != target) && is_ancestor(&repo, target, new) && !is_ancestor(&repo, old, new)) {
                    return Ok(false);
                }
                let mut theirs = authored(&repo, old, target)?;
                theirs.extend(authored_commits(&repo, &made)?);
                authored(&repo, new, target)?.is_subset(&theirs)
            }
            // --- picks ---
            // A cherry-pick or revert only adds commits on top of the old tip: completed once the
            // branch moved forward from it. An abort put it back (`old == new`, refused above).
            // And only with commits of the pick's own. An abort in a terminal and new
            // work there moves the branch forward too: that's an abort, and its Undo (a Rewind)
            // would delete that work.
            PausedKind::CherryPick => is_ancestor(&repo, old, new) && authored(&repo, new, old)?.is_subset(&authored_commits(&repo, &picked)?),
            // GitVolt's own Continue commits the panel's message, which the user
            // may have rewritten (the "This reverts commit" line gone): ended by it (`inside`),
            // no more new commits than the revert's own counts. Ended outside, each must say so.
            PausedKind::Revert => is_ancestor(&repo, old, new) && if inside { at_most(&repo, new, old, picked.len())? } else { reverts_only(&repo, new, old, &picked)? },
        })
    })
    .await
    .unwrap_or(false)
}

/// Per commit: the author's email and time. Not the message: a conflicted Continue commits with
/// `--cleanup=strip` (a `#` line goes) and `prepare-commit-msg` may edit it.
type Authored = BTreeSet<(Vec<u8>, String)>;

/// The commits `tip` has beyond `base`, as what a rebase keeps of each.
fn authored(repo: &gix::Repository, tip: gix::ObjectId, base: gix::ObjectId) -> Result<Authored, GbError> {
    let mut out = BTreeSet::new();
    for info in repo.rev_walk([tip]).with_hidden([base]).all().map_err(gix_err)? {
        let c = repo.find_commit(info.map_err(gix_err)?.id).map_err(gix_err)?;
        let a = c.author().map_err(gix_err)?;
        out.insert((a.email.to_vec(), a.time.to_string()));
    }
    Ok(out)
}

// --- a pick's completion ---
/// `authored` of the commits themselves: what git keeps of each when it cherry-picks it.
fn authored_commits(repo: &gix::Repository, oids: &[String]) -> Result<Authored, GbError> {
    let mut out = BTreeSet::new();
    for hex in oids {
        let c = repo.find_commit(gix::ObjectId::from_hex(hex.as_bytes()).map_err(gix_err)?).map_err(gix_err)?;
        let a = c.author().map_err(gix_err)?;
        out.insert((a.email.to_vec(), a.time.to_string()));
    }
    Ok(out)
}

/// The commits `tip` has beyond `base` are reverts of `oids`: no more of them than `oids`, and
/// each says "This reverts commit <one of them>" (git's message, kept through `--no-edit`). The
/// oid may be abbreviated, 7 hex digits or more (`revert.reference`'s style).
pub(crate) fn reverts_only(repo: &gix::Repository, tip: gix::ObjectId, base: gix::ObjectId, oids: &[String]) -> Result<bool, GbError> {
    let mut count = 0;
    for info in repo.rev_walk([tip]).with_hidden([base]).all().map_err(gix_err)? {
        count += 1;
        if count > oids.len() {
            return Ok(false);
        }
        let c = repo.find_commit(info.map_err(gix_err)?.id).map_err(gix_err)?;
        let message = c.message_raw().map_err(gix_err)?.to_string();
        if !reverts_one_of(&message, oids) {
            return Ok(false);
        }
    }
    Ok(true)
}
/// `tip` has no more than `n` commits beyond `base`.
fn at_most(repo: &gix::Repository, tip: gix::ObjectId, base: gix::ObjectId, n: usize) -> Result<bool, GbError> {
    let mut count = 0;
    for info in repo.rev_walk([tip]).with_hidden([base]).all().map_err(gix_err)? {
        info.map_err(gix_err)?;
        count += 1;
        if count > n {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `message` has a "This reverts commit <hex>" line whose hex, 7 digits or more, starts one of
/// `oids`.
fn reverts_one_of(message: &str, oids: &[String]) -> bool {
    message.match_indices("This reverts commit ").any(|(i, m)| {
        let hex = message[i + m.len()..].chars().take_while(|c| c.is_ascii_hexdigit()).collect::<String>().to_ascii_lowercase();
        hex.len() >= 7 && oids.iter().any(|o| o.to_ascii_lowercase().starts_with(&hex))
    })
}

/// `a` is `b` or one of its ancestors (gix: no git process, nothing in the command log).
pub(crate) fn is_ancestor(repo: &gix::Repository, a: gix::ObjectId, b: gix::ObjectId) -> bool {
    a == b || repo.merge_base(a, b).ok().flatten().is_some_and(|m| m.detach() == a)
}

fn forward_output(bus: EventBus, op: u64) -> (mpsc::UnboundedSender<String>, tokio::task::JoinHandle<()>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let task = tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            bus.emit(AppEvent::OpOutput { op, line: crate::redact::redact(&line) });
        }
    });
    (tx, task)
}

struct Ran<O> {
    result: Result<O, GbError>,
    moves: Vec<RefMove>,
    head_before: HeadState,
    head_after: HeadState,
    kinds: BTreeSet<ChangeKind>,
    journal_changed: bool,
    // The lock and holds come back, so an early failure keeps them until `run_write`
    // has read the lists.
    lock: Option<tokio::sync::OwnedMutexGuard<()>>,
    holds: Vec<crate::watch::WatchHold>,
}

impl<O> Ran<O> {
    fn failed(e: GbError, lock: Option<tokio::sync::OwnedMutexGuard<()>>, holds: Vec<crate::watch::WatchHold>) -> Self {
        Self { result: Err(e), moves: Vec::new(), head_before: HeadState::default(), head_after: HeadState::default(), kinds: BTreeSet::new(), journal_changed: false, lock, holds }
    }
}

/// Steps 2–9, under the lock (a network transfer in `run` may release it; it's taken again
/// before verify).
#[allow(clippy::too_many_arguments)]
async fn steps<I: WriteIntent>(api: &Api, h: &Arc<RepoHandle>, root: &Path, expect: &Expect, intent: &I, op: &OpEntry, out: mpsc::UnboundedSender<String>, writes: Arc<RepoWrites>, lock: tokio::sync::OwnedMutexGuard<()>, holds: Vec<crate::watch::WatchHold>) -> Ran<I::Outcome> {
    // 2. Preflight (read-only).
    let before = match preflight(root, expect, intent.refs(), intent.allowed_in_progress()).await {
        Ok(b) => b,
        Err(e) => return Ran::failed(e, Some(lock), holds),
    };
    let tmp = match api.tmp_dir() {
        Ok(t) => t,
        Err(e) => return Ran::failed(e, Some(lock), holds),
    };
    // The journal this write records in (its entry is written ahead at step 3), declared before
    // the cx, which borrows it.
    let store = match intent.undo() {
        Some(_) => match api.journal(root) {
            Ok(s) => Some(s),
            Err(e) => return Ran::failed(e, Some(lock), holds),
        },
        None => None,
    };
    let trace = if intent.traces_hooks() { hooks::Trace2::new(&tmp).ok() } else { None };
    // The cx exists from here, so a pause that ended outside GitVolt settles
    // before this write plans or changes anything. An early failure hands the lock back.
    let mut cx = WriteCx {
        api,
        h,
        root,
        expect,
        before: &before,
        token: WriteToken::mint(),
        op,
        tmp,
        out,
        hooks: intent.runs_hooks(),
        trace,
        kinds: BTreeSet::new(),
        touched: BTreeMap::new(),
        snapshot: None,
        after: None,
        partial: false,
        keep_redo: false,
        journal_changed: false,
        journal: None,
        autostashed: false,
        writes,
        lock: Some(lock),
        holds,
        paused: None,
        repair_stopped: false,
    };
    macro_rules! fail {
        ($e:expr) => {
            return Ran { journal_changed: cx.journal_changed, ..Ran::failed($e, cx.lock.take(), std::mem::take(&mut cx.holds)) }
        };
    }
    // 2b. No merge or rebase in progress: a paused one ended outside GitVolt. It settles
    //     against what preflight saw, so this write's own moves never land on its entry and its
    //     autostash is restored before this write runs. A Stop during that restore fails it, and
    //     so does the restore itself: the worktree changed under what the user saw,
    //     so they look, then retry.
    if before.in_progress.is_none() {
        let settled = settle_paused(&mut cx, None).await;
        if let Some(e) = settled.stop {
            fail!(e);
        }
        if let Some(label) = settled.restored
            && !intent.only_settles()
        {
            fail!(GbError::stale(format!("The {label} ended outside GitVolt and your changes from before it are back; check them, then try again")));
        }
    }
    // The intent's own plan and run are boxed (as `steps` is): each is a different future per
    // intent, and inline they'd sit in this one.
    let plan = match Box::pin(intent.plan(&Pre { api, h, root, expect, before: &before })).await {
        Ok(p) => p,
        Err(e) => fail!(e),
    };
    // Before anything runs: the autostash plan, and the clean-restore warning.
    let stash_plan = match &plan.autostash {
        Some(spec) => {
            let planned = crate::journal::autostash::plan(api, root, &cx.tmp, spec).await;
            match planned {
                Ok(p) => p,
                Err(e) => fail!(e),
            }
        }
        None => None,
    };
    if let Some(p) = &stash_plan
        && !p.conflicts.is_empty()
        && !intent.confirm().autostash
    {
        let target = p.target.clone().unwrap_or_default();
        fail!(GbError::new(GbErrorKind::Conflict, format!("Your changes conflict with {target}")).with_detail(crate::error::ErrorDetail::AutostashConflict { paths: p.conflicts.clone(), target }));
    }
    // 3. Journal, write-ahead. (Pending entries are recovered once, at the first open, never here.)
    let entry = match (&store, intent.undo()) {
        (Some(s), Some(undo)) => {
            // Stamped with this instance, so another instance's recovery leaves it alone.
            let owner = match api.owner() {
                Ok(o) => o,
                Err(e) => fail!(e),
            };
            let begun = s.update(|j| {
                let id = j.begin(NewEntry { label: intent.label(), kind: intent.kind(), head_before: before.head.clone(), undo }, api.now());
                if let Some(e) = j.entry_mut(id) {
                    e.owner = Some(owner);
                }
                id
            });
            match begun {
                Ok(id) => Some(id),
                Err(e) => fail!(e),
            }
        }
        _ => None,
    };
    cx.journal = store.as_ref().zip(entry);
    // 4. Snapshot, for working-tree-destructive intents.
    let mut ready = Ok(());
    if let Some((paths, untracked)) = &plan.snapshot {
        match snapshot::create(&cx.snapshots(), &intent.label(), paths, untracked).await {
            Ok(snap) => {
                if let (Some(s), Some(id)) = (&store, entry) {
                    let kept = snap.clone();
                    ready = s.update(|j| {
                        if let Some(e) = j.entry_mut(id) {
                            e.before = Some(kept);
                        }
                    });
                }
                cx.snapshot = Some(snap);
            }
            Err(e) => ready = Err(e),
        }
    }
    // 5. Autostash, written ahead: a `Pending` record (no oid yet) before the push,
    //    so a stash git stores is never without one. After the push: its oid, or the record
    //    goes. A stash git stored is restored at step 8 even when the push then failed,
    //    unless a Stop interrupted the push (it stays, `Stopped`, and the op doesn't run).
    let mut stash = None;
    if ready.is_ok()
        && let Some(p) = &stash_plan
    {
        let (saved, res) = crate::journal::autostash::save(&mut cx, p, &intent.label()).await;
        cx.autostashed = saved.is_some();
        stash = saved;
        if let Err(e) = res {
            ready = Err(e);
        }
        cx.journal_changed = true;
    }
    // A stage or unstage is one staging step; its `before` is the index as a tree now.
    let ran = ready.is_ok();
    let staging_before = if ran && intent.staging() == Staging::Step { crate::journal::staging::capture(&cx).await.unwrap_or(None) } else { None };
    // 6. Run.
    let mut result = match ready {
        Ok(()) => Box::pin(intent.run(&mut cx)).await,
        Err(e) => Err(e),
    };
    // A network transfer may have unlocked: every local step from here runs locked again.
    cx.relock().await;
    // 7. Verify: the observed old → new of each ref, never the intended one.
    let mut known = before.refs.clone();
    for (name, old) in &cx.touched {
        known.entry(name.clone()).or_insert_with(|| old.clone());
    }
    //    If it can't be read, nothing is guessed: the entry is kept, flagged non-undoable,
    //    so neither a "no moves" drop nor an undo to made-up values can follow.
    let (head_after, now, unverified) = match observe(root, known.keys().cloned().collect()).await {
        Ok((head, now)) => (head, now, false),
        Err(e) => {
            tracing::warn!(target: "gitvolt_core::write", "verify failed: {e}");
            (before.head.clone(), known.clone(), true)
        }
    };
    let moves: Vec<RefMove> = now
        .iter()
        .filter_map(|(name, new)| {
            let old = known.get(name)?.clone();
            (old != *new).then(|| RefMove { name: name.clone(), old, new: new.clone() })
        })
        .collect();
    // 7b. This write started mid-operation and may have ended it (Commit, Continue,
    //     Abort): the pause settles. A Stop restoring its autostash fails the write, as
    //     at step 8.
    if before.in_progress.is_some()
        && cx.paused.is_none()
        && let Some(e) = settle_paused(&mut cx, Some(Completing { entry, moves: &moves })).await.stop
    {
        result = Err(e);
    }
    // Record the step, or clear the log for a write that moved HEAD or rewrote the index.
    if ran {
        crate::journal::staging::after_run(&cx, intent.staging(), &intent.label(), staging_before, result.is_ok(), &head_after).await;
    }
    // 8. Restore the autostash, whatever the run did (unless a merge or rebase stopped on
    //    conflicts, which waits for Continue, Commit or Abort).
    //    A Cancel of the op doesn't stop it; a Stop does, and fails the write.
    if let Some(st) = stash.as_ref().filter(|s| s.restore) {
        if cx.paused.is_some() {
            // It waits for Commit, Continue or Abort.
            crate::journal::autostash::keep_paused(api, root, st);
        } else {
            let applied = match Box::pin(crate::journal::autostash::restore(&mut cx, &st.oid, &st.message, true)).await {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!(target: "gitvolt_core::write", "autostash restore failed: {e}");
                    crate::journal::autostash::Applied::Refused { index: false, message: e.message }
                }
            };
            crate::journal::autostash::settle(api, root, st, &applied);
            if let Some(e) = crate::journal::autostash::stop_error(&applied, &st.message) {
                result = Err(e);
            }
        }
    }
    cx.autostashed = false;
    // 9. Finalize: done, or dropped when nothing changed. A failure after a partial change still
    //    records what did change; one that changed nothing (no ref or HEAD moved, and the intent
    //    didn't flag a partial change) leaves no entry, not even a `before`-only or a barrier one.
    let after_snapshot = if result.is_ok() { cx.after.take() } else { None };
    // A kept autostash isn't a reason to keep the entry: it has its own banner (`Journal::kept`).
    let unchanged_failure = result.is_err() && !unverified && moves.is_empty() && head_after == before.head && !cx.partial;
    let mut journal_changed = cx.journal_changed;
    let keep_redo = cx.keep_redo;
    // A save on top of a save of the same file, with nothing changed in between, merges.
    let coalesce_into = match (&store, entry, &cx.snapshot, &after_snapshot) {
        (Some(s), Some(id), Some(before_snap), Some(_)) if intent.coalesces() && cx.paused.is_none() && !unverified => s.load().ok().and_then(|j| {
            let prev = j.below(id)?;
            let same = prev.label == intent.label()
                && prev.kind == intent.kind()
                && prev.undo == UndoKind::Restore
                && prev.blocked.is_none()
                && prev.head_after == head_after
                && prev.after.as_ref().is_some_and(|a| {
                    a.paths == before_snap.paths && a.untracked == before_snap.untracked && a.modes == before_snap.modes && snapshot::trees(root, a).ok().is_some_and(|t| snapshot::trees(root, before_snap).ok() == Some(t))
                });
            same.then_some(prev.id)
        }),
        _ => None,
    };
    // What a commit, checkout or stash changed beyond its snapshots, for the Undo
    // dropdown's dependency check (read now, while the objects are at hand).
    let tree_paths = match (&store, entry) {
        (Some(s), Some(id)) if cx.paused.is_none() && !unverified && !unchanged_failure && (head_after != before.head || intent.undo() == Some(UndoKind::Stash)) => match s.load().ok().and_then(|j| j.undo.iter().find(|e| e.id == id).cloned()) {
            Some(mut e) => {
                e.head_after = head_after.clone();
                crate::journal::history::tree_paths(&api.cli, root, &e).await
            }
            None => None,
        },
        _ => None,
    };
    if let (Some(s), Some(id)) = (&store, entry) {
        let finalized = match &cx.paused {
            // The entry waits, with what completion compares against (`known`: preflight's
            // refs plus the old value of every ref the write CAS-moved).
            Some(p) => {
                let paused = PausedOp { kind: p.kind, target: p.target.clone(), refs_before: known.clone(), autostash: stash.as_ref().map(|st| st.id), target_oid: p.target_oid.clone(), put_back: p.put_back.clone(), picked: p.picked.clone(), irebase: p.irebase.clone() };
                s.update(|j| {
                    if let Some(e) = j.entry_mut(id) {
                        e.state = EntryState::Paused;
                        e.head_after = head_after.clone();
                        e.paused = Some(paused);
                        e.owner = None;
                    }
                    true
                })
            }
            None => s.update(|j| {
                if unchanged_failure {
                    j.drop_entry(id);
                    return false;
                }
                if let Some(e) = j.entry_mut(id) {
                    e.head_after = head_after.clone();
                    e.refs = moves.clone();
                    e.after = after_snapshot;
                    e.owner = None;
                    e.tree_paths = tree_paths;
                    if unverified {
                        e.blocked = Some(crate::journal::UNVERIFIED.to_string());
                    }
                }
                if let Some(into) = coalesce_into {
                    return j.coalesce(id, into);
                }
                j.finalize_with(id, keep_redo)
            }),
        };
        journal_changed |= finalized.unwrap_or(false);
    }
    // A rewrite of a pushed branch records its lease; any move drops a mark it ended.
    let rewrite = if result.is_ok() && cx.paused.is_none() { intent.rewrite() } else { None };
    rewrites::record(api, &h.common_dir, root, rewrite, &moves).await;
    let kinds = std::mem::take(&mut cx.kinds);
    Ran { result, moves, head_before: before.head.clone(), head_after, kinds, journal_changed, lock: cx.lock.take(), holds: std::mem::take(&mut cx.holds) }
}

/// The sidebar's submodules of `root` changed with this write (a gitlink staged, removed or
/// checked out, `.gitmodules`, a submodule updated): `repoChanged` says `submodules`.
async fn submodules_changed(api: &Api, h: &RepoHandle, root: &Path) -> bool {
    let (repo, workdir, seen, root) = (h.repo.clone(), h.workdir.clone(), api.submodules.clone(), root.to_path_buf());
    crate::api::blocking(move || {
        let here = crate::platform::fs::canonicalize(&workdir)?;
        Ok(seen.check(&repo.to_thread_local(), &here, &[root]))
    })
    .await
    .unwrap_or(false)
}

/// `repoChanged` with what the write touched (and the lists' version), and
/// `refsUpdated` when a ref moved, to every tab of the repository. Always sent, whatever the
/// outcome: it's the refresh after every write.
fn announce<O>(api: &Api, writes: &RepoWrites, root: &Path, ran: &Ran<O>, version: Option<&str>, submodules: bool) {
    let mut kinds = ran.kinds.clone();
    kinds.extend([ChangeKind::Worktree, ChangeKind::Index]);
    if submodules {
        kinds.insert(ChangeKind::Submodules);
    }
    if !ran.moves.is_empty() {
        kinds.insert(ChangeKind::Refs);
    }
    if ran.head_before != ran.head_after {
        kinds.insert(ChangeKind::Head);
    }
    let wt = root.display().to_string();
    let versions: BTreeMap<String, String> = version.map(|v| (wt.clone(), v.to_string())).into_iter().collect();
    for id in writes.ids() {
        api.bus.emit(AppEvent::RepoChanged { repo: id, kinds: kinds.iter().copied().collect(), worktrees: vec![wt.clone()], versions: versions.clone() });
        if !ran.moves.is_empty() {
            api.bus.emit(AppEvent::RefsUpdated { repo: id });
        }
    }
}

/// Every write. Boxed, so a caller's future (`dispatch`'s match, an undo, a stash op) holds a
/// pointer, not the whole write: inline, the futures and the debug-build frames that move them
/// add up past a 2 MB thread stack.
pub(crate) async fn run_write<I: WriteIntent>(api: &Api, repo: u32, worktree: &str, expect: Expect, intent: I) -> Result<WriteResult<I::Outcome>, GbError> {
    Box::pin(run_write_inner(api, repo, worktree, expect, intent)).await
}

async fn run_write_inner<I: WriteIntent>(api: &Api, repo: u32, worktree: &str, expect: Expect, intent: I) -> Result<WriteResult<I::Outcome>, GbError> {
    let h = api.handle(repo)?;
    let writes = api.repo_writes(&h);
    let (kind, label) = (intent.kind(), intent.label());
    let op = api.ops.begin(kind, Some(repo), true);
    // 1. Queue and lock. A queued write joins the queue before anything awaits, so the queue's
    //    order is the click order. Cancelled while it waits, it leaves at once, quietly.
    let ticket = match intent.class() {
        WriteClass::Queued => Some(writes.queue.enqueue(&label, kind, op.id).cancel_on(op.cancel.clone())),
        WriteClass::Immediate => None,
    };
    // Then the worktree and the guard, before anything writes; an error drops the ticket, which
    // takes the item out of the queue.
    let root = api.worktree_dir(&h, worktree).await?;
    api.check_write(&h.common_dir)?;
    let slot = match ticket {
        Some(t) => Some(writes.queue.turn(t).await?),
        None => None,
    };
    let expect = match &slot {
        Some(s) => s.carry(&expect),
        None => expect,
    };
    // The op starts once the queue slot is taken, before any transfer; the lock and the
    // watcher hold come after `transfer_first`.
    api.bus.emit(AppEvent::OpStarted { op: op.id, kind, repo: Some(repo), label: label.clone(), interactive: true });
    let (out, forward) = forward_output(api.bus.clone(), op.id);
    // 0. A network transfer first (pull's fetch): the queue slot only.
    let first = {
        let mut net = NetCx { api, h: &h, root: &root, op: &op, token: WriteToken::mint(), out: out.clone() };
        intent.transfer_first(&mut net).await
    };
    let lock = writes.acquire().await;
    let holds = api.watch_holds(&h.common_dir);
    let mut ran = match first {
        Ok(()) => Box::pin(steps(api, &h, &root, &expect, &intent, &op, out, writes.clone(), lock, holds)).await,
        Err(e) => {
            drop(out);
            Ran::failed(e, Some(lock), holds)
        }
    };
    // `steps` always hands both back (a network step relocks); this only guards the lists'
    // read below.
    if ran.lock.is_none() {
        ran.lock = Some(writes.acquire().await);
        ran.holds = api.watch_holds(&h.common_dir);
    }
    let _ = forward.await;
    // 10. The fresh lists, after the write's last git command; the watcher absorbs them (and
    //     the refs as the write left them), so its own pass finds nothing new.
    let lists = crate::watch::read_and_keep_lists(&h.repo, &api.cli, &h.wip, &root).await.ok();
    let submodules = submodules_changed(api, &h, &root).await;
    for hold in &ran.holds {
        if let Some(l) = &lists {
            hold.absorb(root.clone(), l.digest);
        }
        hold.absorb_git();
    }
    ran.holds.clear();
    ran.lock = None;
    if let Some(slot) = slot {
        let mut carried = ran.moves.clone();
        if ran.head_before.oid != ran.head_after.oid {
            carried.push(RefMove { name: "HEAD".into(), old: ran.head_before.oid.clone(), new: ran.head_after.oid.clone() });
        }
        // A failure stops only the items queued behind it; a Cancel stops nothing.
        slot.finish(ran.result.as_ref().err(), &carried);
    }
    let wip = lists.map(|l| WipListsPayload { worktree: root.display().to_string(), version: l.version.clone(), staged: (*l.staged).clone(), unstaged: (*l.unstaged).clone() });
    announce(api, &writes, &root, &ran, wip.as_ref().map(|w| w.version.as_str()), submodules);
    // A write that ran is never reported as failed because the journal couldn't be read after it
    // (the user might retry a commit): the state falls back to an empty one.
    let journal = match api.journal_state(&root) {
        Ok(state) => {
            if ran.journal_changed {
                for id in writes.ids() {
                    api.bus.emit(AppEvent::JournalChanged { repo: id, worktree: root.display().to_string(), state: state.clone() });
                }
            }
            state
        }
        Err(e) => {
            tracing::warn!(target: "gitvolt_core::write", "journal state after {label}: {e}");
            crate::journal::Journal::empty(&root.display().to_string()).state(None)
        }
    };
    let (outcome, message) = match &ran.result {
        Ok(_) => (OpOutcome::Ok, None),
        Err(e) if e.kind == GbErrorKind::Cancelled => (OpOutcome::Cancelled, Some(e.message.clone())),
        Err(e) => (OpOutcome::Failed, Some(e.message.clone())),
    };
    api.bus.emit(AppEvent::OpFinished { op: op.id, kind, repo: Some(repo), outcome, message, command: None });
    let conflicted = wip.as_ref().is_some_and(|w| w.unstaged.files.iter().any(|f| f.status == "U"));
    let staging = crate::journal::staging::state(api, &root, conflicted);
    Ok(WriteResult { outcome: ran.result?, journal, staging, wip })
}
