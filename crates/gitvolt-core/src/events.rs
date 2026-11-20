//! Backend → frontend events. One broadcast bus; `gitvolt-app` forwards it as the
//! Tauri event `gb:event`, and `gitvolt-harness` forwards it on every WebSocket.
//!
//! `repoChanged` also carries `worktrees` (the canonical paths whose
//! status changed), and there are extra events: `opStarted` (an op's id and label),
//! `authResolved` (closes the prompt), `opProgress`, which carries only the op id, and
//! `openRequested` (a second launch's path).

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use ts_rs::TS;

/// The Tauri event name every `AppEvent` is emitted under (`TAURI_EVENT` in `ui/src/api/transport.ts`).
pub const TAURI_EVENT: &str = "gb:event";
/// A receiver more than this many events behind skips ahead (and logs a warning): it loses the
/// oldest, whatever they are.
const CAPACITY: usize = 256;
/// From this many events not yet read by the slowest receiver, the bus sheds the stream events
/// (`AppEvent::sheddable`), so a burst of them can't push an `opStarted` or `opFinished` out of
/// the channel: those only go when this many other events pile up behind them.
const SHED_AT: usize = CAPACITY / 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ChangeKind {
    Worktree,
    Index,
    Refs,
    Head,
    Stash,
    Config,
    /// A merge, rebase, cherry-pick or revert started or ended.
    State,
    /// A worktree's submodules as the sidebar lists them: a gitlink in its index, its
    /// `.gitmodules`, or a submodule's checked-out commit (`submodules::Seen`).
    Submodules,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum OpKind {
    Fetch,
    Clone,
    // Writes.
    Commit,
    Checkout,
    Branch,
    Reset,
    Stash,
    Discard,
    Stage,
    Pull,
    Push,
    Merge,
    Rebase,
    Undo,
    Redo,
    Worktree,
    Resolve,
    // Saving a working file.
    Save,
    // Restore a file from a commit.
    Restore,
    // Git LFS: fetch, pull, a file's download, track and untrack.
    Lfs,
    // Submodule update and sync.
    Submodule,
}

/// An autostash step (`OpStashStep`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum StashStep {
    /// `git stash push`: "Saving your changes…".
    Saving,
    /// `git stash apply` and its drop: "Restoring your changes…".
    Restoring,
    /// Putting back the files a failed or cancelled checkout rewrote: "Restoring files…".
    RestoringFiles,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum OpOutcome {
    Ok,
    Skipped,
    Failed,
    Cancelled,
}

/// A rebase's place: step `n` of `m`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProgressStep {
    pub n: u32,
    pub m: u32,
    /// The branch being rebased (short name), when known: the status bar's `Rebasing main (n/m)...`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum AppEvent {
    /// Debounced file-system change in the watched (active) repo. `worktrees`: canonical paths
    /// whose status (or WIP file lists) actually changed. `versions`: for those of them whose WIP
    /// lists the watcher keeps, the lists' new version (`FileListPayload::version`).
    RepoChanged {
        repo: u32,
        kinds: Vec<ChangeKind>,
        worktrees: Vec<String>,
        #[ts(type = "Record<string, string>")]
        #[serde(default)]
        versions: std::collections::BTreeMap<String, String>,
    },
    /// A fetch moved at least one ref.
    RefsUpdated { repo: u32 },
    OpStarted {
        #[ts(type = "number")]
        op: u64,
        kind: OpKind,
        repo: Option<u32>,
        /// Fetch: the repo name. Clone: the destination path.
        label: String,
        /// User-started (`true`) or GitVolt-started (`false`, the background fetch). The UI
        /// shows a background op nowhere but its activity log.
        interactive: bool,
    },
    OpProgress {
        #[ts(type = "number")]
        op: u64,
        phase: String,
        percent: Option<u8>,
        /// A rebase's `n` of `m`; absent for fetch and clone.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        step: Option<ProgressStep>,
    },
    /// A push's or fetch's `remote:` lines, classified: the UI keeps them in
    /// that op's Activity entry, whole.
    OpRemote {
        #[ts(type = "number")]
        op: u64,
        lines: Vec<crate::write::remote_output::RemoteLine>,
    },
    OpFinished {
        #[ts(type = "number")]
        op: u64,
        kind: OpKind,
        repo: Option<u32>,
        outcome: OpOutcome,
        message: Option<String>,
        /// The git command that ran, argv joined for display and redacted (no environment, no
        /// askpass secrets): the activity log shows it.
        command: Option<String>,
    },
    /// One redacted line of a write's hook or progress output: the UI appends it
    /// to that op's Activity entry.
    OpOutput {
        #[ts(type = "number")]
        op: u64,
        line: String,
    },
    /// A write's autostash step began (`step`), or ended (`None`). Once a step has
    /// run ~60 s, the slow-write status says "Saving your changes…" or "Restoring your
    /// changes…"; while one runs, the op's Cancel reads "Stop — your changes stay in stash
    /// <message>", and stops the step (a 15-minute limit stops it too).
    OpStashStep {
        #[ts(type = "number")]
        op: u64,
        step: Option<StashStep>,
        message: String,
    },
    /// A worktree's undo journal changed (a write, an undo, a banner): Undo/Redo and the banners
    /// follow.
    JournalChanged { repo: u32, worktree: String, state: crate::journal::JournalState },
    /// The repository's action queue changed: the status-bar chip follows.
    QueueChanged {
        repo: u32,
        running: Option<crate::write::types::QueueItem>,
        queued: Vec<crate::write::types::QueueItem>,
        stopped: Option<crate::write::types::QueueStop>,
    },
    /// A credential prompt is waiting for the user.
    AuthWaiting {
        #[ts(type = "number")]
        prompt: u64,
        #[ts(type = "number")]
        op: u64,
        repo: Option<u32>,
        text: String,
        secret: bool,
        /// An HTTPS prompt tied to one remote of the op: "Use SSH for <remote> instead".
        ssh: Option<crate::askpass::SshOffer>,
    },
    AuthResolved {
        #[ts(type = "number")]
        prompt: u64,
    },
    /// Another launch on this config dir handed over its launch path (absolute) and exited
    /// (the single-instance guard, `instance.rs`): the UI opens it in a tab.
    OpenRequested { path: String },
    /// The update check, download or install moved on (`updates.rs`): the status bar's pill and
    /// the update dialog follow.
    UpdateChanged { state: crate::updates::UpdateState },
}

impl AppEvent {
    /// A stream event: an op's progress (the next one supersedes it) or a line of its output.
    /// Losing one costs a frame of a progress bar or a line of Activity, where losing an
    /// `opStarted`, `opFinished` or a refresh would leave the UI wrong.
    fn sheddable(&self) -> bool {
        matches!(self, AppEvent::OpProgress { .. } | AppEvent::OpOutput { .. })
    }
}

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<AppEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CAPACITY);
        Self { tx }
    }

    /// Never blocks and never fails: with no subscribers the event is dropped, and so is a
    /// sheddable one while a receiver is `SHED_AT` behind. (Two emitters racing past the check
    /// overshoot it by one each, far from `CAPACITY`.)
    pub fn emit(&self, ev: AppEvent) {
        if ev.sheddable() && self.tx.len() >= SHED_AT {
            tracing::debug!("event bus backed up: an op's progress or output line dropped");
            return;
        }
        let _ = self.tx.send(ev);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.tx.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribers_receive_emitted_events() {
        let bus = EventBus::new();
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();
        bus.emit(AppEvent::RefsUpdated { repo: 3 });
        assert_eq!(a.recv().await.unwrap(), AppEvent::RefsUpdated { repo: 3 });
        assert_eq!(b.recv().await.unwrap(), AppEvent::RefsUpdated { repo: 3 });
    }

    /// A receiver that falls behind during a burst of progress and output still gets the op's
    /// start and end: the burst is shed, they aren't.
    #[test]
    fn a_progress_burst_never_pushes_out_an_ops_start_or_end() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        let started = AppEvent::OpStarted { op: 1, kind: OpKind::Fetch, repo: Some(1), label: "r".into(), interactive: true };
        let finished = AppEvent::OpFinished { op: 1, kind: OpKind::Fetch, repo: Some(1), outcome: OpOutcome::Ok, message: None, command: None };
        bus.emit(started.clone());
        for i in 0..10_000u32 {
            bus.emit(AppEvent::OpProgress { op: 1, phase: "Receiving objects".into(), percent: Some((i % 101) as u8), step: None });
            bus.emit(AppEvent::OpOutput { op: 1, line: format!("line {i}") });
        }
        bus.emit(AppEvent::RefsUpdated { repo: 1 });
        bus.emit(finished.clone());
        let mut got = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(ev) => got.push(ev),
                Err(broadcast::error::TryRecvError::Empty) => break,
                Err(e) => panic!("{e:?}"),
            }
        }
        assert_eq!(got.first(), Some(&started));
        assert_eq!(&got[got.len() - 2..], [AppEvent::RefsUpdated { repo: 1 }, finished]);
        assert_eq!(got.len(), SHED_AT + 2, "the stream filled half the channel, then was shed");
        // Read up, the stream flows again.
        bus.emit(AppEvent::OpOutput { op: 1, line: "after".into() });
        assert_eq!(rx.try_recv().unwrap(), AppEvent::OpOutput { op: 1, line: "after".into() });
    }

    #[test]
    fn emitting_without_subscribers_is_fine() {
        EventBus::new().emit(AppEvent::RefsUpdated { repo: 1 });
    }

    #[test]
    fn serializes_tagged_camel_case_and_round_trips() {
        let ev = AppEvent::RepoChanged { repo: 1, kinds: vec![ChangeKind::Refs, ChangeKind::Worktree], worktrees: vec!["/r".into()], versions: [("/r".to_string(), "00ff".to_string())].into() };
        assert_eq!(
            serde_json::to_value(&ev).unwrap(),
            serde_json::json!({"type": "repoChanged", "repo": 1, "kinds": ["refs", "worktree"], "worktrees": ["/r"], "versions": {"/r": "00ff"}})
        );
        let ev = AppEvent::AuthWaiting { prompt: 2, op: 5, repo: None, text: "Password: ".into(), secret: true, ssh: None };
        assert_eq!(
            serde_json::to_value(&ev).unwrap(),
            serde_json::json!({"type": "authWaiting", "prompt": 2, "op": 5, "repo": null, "text": "Password: ", "secret": true, "ssh": null})
        );
        assert_eq!(serde_json::to_value(AppEvent::OpenRequested { path: "/r".into() }).unwrap(), serde_json::json!({"type": "openRequested", "path": "/r"}));
        let back: AppEvent = serde_json::from_value(serde_json::json!({"type": "refsUpdated", "repo": 4})).unwrap();
        assert_eq!(back, AppEvent::RefsUpdated { repo: 4 });
    }

    #[test]
    fn write_op_kinds_and_op_output_serialize() {
        assert_eq!(serde_json::to_value(OpKind::Commit).unwrap(), "commit");
        assert_eq!(serde_json::to_value(OpKind::Undo).unwrap(), "undo");
        assert_eq!(serde_json::to_value(OpKind::Resolve).unwrap(), "resolve");
        let ev = AppEvent::OpOutput { op: 3, line: "lint: ok".into() };
        assert_eq!(serde_json::to_value(&ev).unwrap(), serde_json::json!({"type": "opOutput", "op": 3, "line": "lint: ok"}));
    }
}
