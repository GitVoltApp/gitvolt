//! The operations the core announces on its event bus (`ui/src/app/ops.ts`, `queue/store.ts`,
//! `undo/store.ts`): what's running, the credential prompts, the action queue, each worktree's journal
//! and staging log, the background errors and the activity log. Fed once per event, whichever tab shows.

use gitvolt_core::askpass::SshOffer;
use gitvolt_core::events::{AppEvent, OpKind, OpOutcome, ProgressStep, StashStep};
use gitvolt_core::journal::JournalState;
use gitvolt_core::write::remote_output::RemoteLine;
use gitvolt_core::write::types::{QueueStatePayload, StagingUndoState};
use std::collections::HashMap;
use std::time::Instant;

pub const OUTPUT_CAP: usize = 64 * 1024;
const TRUNCATED: &str = "… output truncated at 64 KB";
pub const MAX_ERRORS: usize = 100;
pub const MAX_ACTIVITY: usize = 200;

/// A running operation, from `opStarted` until `opFinished`. `interactive`: the user started it.
/// `shown`: a user's Fetch found this background fetch running and waits on it instead.
#[derive(Debug, Clone)]
pub struct OpInfo {
    pub op: u64,
    pub kind: OpKind,
    pub repo: Option<u32>,
    pub label: String,
    pub percent: Option<u8>,
    pub interactive: bool,
    pub shown: bool,
    pub started: Instant,
    pub stash: Option<(StashStep, String, Instant)>,
    pub step: Option<ProgressStep>,
}

/// A credential prompt waiting for the user, from `authWaiting` until `authResolved`.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthPrompt {
    pub prompt: u64,
    pub op: u64,
    pub repo: Option<u32>,
    pub text: String,
    pub secret: bool,
    pub ssh: Option<SshOffer>,
}

/// One finished operation, for the activity log.
#[derive(Debug, Clone)]
pub struct ActivityEntry {
    pub op: u64,
    pub kind: OpKind,
    pub label: String,
    pub background: bool,
    pub duration_ms: u64,
    pub outcome: OpOutcome,
    pub message: Option<String>,
    pub command: Option<String>,
    pub output: Vec<String>,
    pub remote: Vec<RemoteLine>,
}

#[derive(Debug, Clone)]
pub struct BgError {
    pub message: String,
}

#[derive(Default)]
pub struct Ops {
    pub ops: HashMap<u64, OpInfo>,
    pub prompts: Vec<AuthPrompt>,
    /// Newest first.
    pub errors: Vec<BgError>,
    pub unread: usize,
    /// Newest first.
    pub activity: Vec<ActivityEntry>,
    outputs: HashMap<u64, (Vec<String>, usize)>,
    remotes: HashMap<u64, Vec<RemoteLine>>,
    pub queues: HashMap<u32, QueueStatePayload>,
    pub journals: HashMap<(u32, String), JournalState>,
    pub staging: HashMap<(u32, String), StagingUndoState>,
}

impl Ops {
    pub fn apply(&mut self, e: &AppEvent) {
        match e {
            AppEvent::OpStarted { op, kind, repo, label, interactive } => {
                self.ops.insert(*op, OpInfo { op: *op, kind: *kind, repo: *repo, label: label.clone(), percent: None, interactive: *interactive, shown: false, started: Instant::now(), stash: None, step: None });
            }
            AppEvent::OpProgress { op, percent, step, .. } => {
                if let Some(o) = self.ops.get_mut(op) {
                    o.percent = *percent;
                    if step.is_some() {
                        o.step.clone_from(step);
                    }
                }
            }
            AppEvent::OpStashStep { op, step, message } => {
                if let Some(o) = self.ops.get_mut(op) {
                    o.stash = step.map(|s| (s, message.clone(), Instant::now()));
                }
            }
            AppEvent::OpOutput { op, line } => {
                let (lines, size) = self.outputs.entry(*op).or_default();
                if lines.last().is_some_and(|l| l == TRUNCATED) {
                    return;
                }
                *size += line.len() + 1;
                lines.push(if *size > OUTPUT_CAP { TRUNCATED.to_string() } else { line.clone() });
            }
            AppEvent::OpRemote { op, lines } => self.remotes.entry(*op).or_default().extend(lines.iter().cloned()),
            AppEvent::OpFinished { op, kind, outcome, message, command, .. } => {
                let done = self.ops.remove(op);
                let output = self.outputs.remove(op).map_or(Vec::new(), |o| o.0);
                let remote = self.remotes.remove(op).unwrap_or_default();
                // A finished op's askpass child is gone, so a prompt of its would never close.
                self.prompts.retain(|p| p.op != *op);
                // A successful stage or unstage appears only among the commands.
                if *kind == OpKind::Stage && *outcome == OpOutcome::Ok {
                    return;
                }
                let entry = ActivityEntry {
                    op: *op,
                    kind: *kind,
                    label: done.as_ref().map_or(String::new(), |d| d.label.clone()),
                    background: done.as_ref().is_some_and(|d| !d.interactive),
                    duration_ms: done.as_ref().map_or(0, |d| d.started.elapsed().as_millis() as u64),
                    outcome: *outcome,
                    message: message.clone(),
                    command: command.clone(),
                    output,
                    remote,
                };
                self.activity.insert(0, entry);
                self.activity.truncate(MAX_ACTIVITY);
            }
            AppEvent::AuthWaiting { prompt, op, repo, text, secret, ssh } => {
                self.prompts.retain(|p| p.prompt != *prompt);
                self.prompts.push(AuthPrompt { prompt: *prompt, op: *op, repo: *repo, text: text.clone(), secret: *secret, ssh: ssh.clone() });
            }
            AppEvent::AuthResolved { prompt } => self.prompts.retain(|p| p.prompt != *prompt),
            AppEvent::QueueChanged { repo, running, queued, stopped } => {
                self.queues.insert(*repo, QueueStatePayload { running: running.clone(), queued: queued.clone(), stopped: stopped.clone() });
            }
            AppEvent::JournalChanged { repo, worktree, state } => {
                self.journals.insert((*repo, worktree.clone()), state.clone());
            }
            _ => {}
        }
    }

    pub fn push_error(&mut self, message: String) {
        self.errors.insert(0, BgError { message });
        self.errors.truncate(MAX_ERRORS);
        self.unread += 1;
    }

    pub fn journal(&self, repo: u32, worktree: &str) -> Option<&JournalState> {
        self.journals.get(&(repo, worktree.to_string()))
    }

    pub fn staging(&self, repo: u32, worktree: &str) -> Option<&StagingUndoState> {
        self.staging.get(&(repo, worktree.to_string()))
    }

    /// The user's running write (not a fetch or clone).
    pub fn user_write(&self) -> Option<&OpInfo> {
        self.ops.values().filter(|o| o.interactive && o.kind != OpKind::Fetch && o.kind != OpKind::Clone).min_by_key(|o| o.op)
    }

    /// The user's running fetch, or the background one a user's Fetch waits on.
    pub fn user_fetch(&self) -> Option<&OpInfo> {
        self.ops.values().filter(|o| o.kind == OpKind::Fetch && (o.interactive || o.shown)).min_by_key(|o| o.op)
    }

    pub fn clone_op(&self) -> Option<&OpInfo> {
        self.ops.values().filter(|o| o.kind == OpKind::Clone).min_by_key(|o| o.op)
    }

    pub fn show_op(&mut self, op: u64) {
        if let Some(o) = self.ops.get_mut(&op) {
            o.shown = true;
        }
    }

    /// An op of `kind` waits in `repo`'s queue.
    pub fn queued_kind(&self, repo: u32, kind: OpKind) -> bool {
        self.queues.get(&repo).is_some_and(|q| q.queued.iter().any(|i| i.kind == kind))
    }
}

/// `Running: push dev · 2 queued`, or `Stopped: push dev failed · 2 not run`.
pub fn chip_text(q: &QueueStatePayload) -> String {
    let n = q.queued.len();
    if let Some(s) = &q.stopped {
        return format!("Stopped: {} failed · {n} not run", s.label);
    }
    match &q.running {
        None => format!("{n} queued"),
        Some(r) if n > 0 => format!("Running: {} · {n} queued", r.label),
        Some(r) => format!("Running: {}", r.label),
    }
}

pub fn queue_idle(q: Option<&QueueStatePayload>) -> bool {
    q.is_none_or(|q| q.running.is_none() && q.queued.is_empty() && q.stopped.is_none())
}

/// A rebase's counter.
pub fn rebase_status(o: &OpInfo) -> Option<String> {
    if o.kind != OpKind::Rebase && o.kind != OpKind::Pull {
        return None;
    }
    let s = o.step.as_ref()?;
    Some(match &s.branch {
        Some(b) => format!("Rebasing {b} ({}/{})…", s.n, s.m),
        None => format!("Rebasing ({}/{})…", s.n, s.m),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitvolt_core::write::types::{QueueItem, QueueStop};
    use std::prelude::v1::test;

    fn started(op: u64, kind: OpKind, interactive: bool) -> AppEvent {
        AppEvent::OpStarted { op, kind, repo: Some(1), label: "fixture".into(), interactive }
    }

    fn finished(op: u64, kind: OpKind, outcome: OpOutcome) -> AppEvent {
        AppEvent::OpFinished { op, kind, repo: Some(1), outcome, message: None, command: Some("git x".into()) }
    }

    #[test]
    fn a_finished_op_leaves_an_activity_entry_with_its_output() {
        let mut o = Ops::default();
        o.apply(&started(1, OpKind::Commit, true));
        assert!(o.user_write().is_some());
        o.apply(&AppEvent::OpOutput { op: 1, line: "lint: ok".into() });
        o.apply(&finished(1, OpKind::Commit, OpOutcome::Ok));
        assert!(o.user_write().is_none());
        assert_eq!(o.activity[0].output, ["lint: ok"]);
        assert!(!o.activity[0].background);
    }

    #[test]
    fn a_successful_stage_is_not_in_the_activity_log_and_a_failed_one_is() {
        let mut o = Ops::default();
        o.apply(&started(1, OpKind::Stage, true));
        o.apply(&finished(1, OpKind::Stage, OpOutcome::Ok));
        assert!(o.activity.is_empty());
        o.apply(&started(2, OpKind::Stage, true));
        o.apply(&finished(2, OpKind::Stage, OpOutcome::Failed));
        assert_eq!(o.activity.len(), 1);
    }

    #[test]
    fn a_finished_op_takes_its_prompt_with_it() {
        let mut o = Ops::default();
        o.apply(&started(1, OpKind::Push, true));
        o.apply(&AppEvent::AuthWaiting { prompt: 7, op: 1, repo: Some(1), text: "Password: ".into(), secret: true, ssh: None });
        assert_eq!(o.prompts.len(), 1);
        o.apply(&finished(1, OpKind::Push, OpOutcome::Failed));
        assert!(o.prompts.is_empty());
    }

    #[test]
    fn the_output_stops_at_the_cap_once() {
        let mut o = Ops::default();
        o.apply(&started(1, OpKind::Commit, true));
        for _ in 0..3 {
            o.apply(&AppEvent::OpOutput { op: 1, line: "x".repeat(OUTPUT_CAP / 2) });
        }
        o.apply(&AppEvent::OpOutput { op: 1, line: "more".into() });
        o.apply(&finished(1, OpKind::Commit, OpOutcome::Ok));
        assert_eq!(o.activity[0].output.last().map(String::as_str), Some(TRUNCATED));
        assert_eq!(o.activity[0].output.iter().filter(|l| *l == TRUNCATED).count(), 1);
    }

    #[test]
    fn the_queue_chip_says_what_runs_and_what_stopped() {
        let item = |label: &str| QueueItem { id: 1, label: label.into(), kind: OpKind::Push, op: 1 };
        assert_eq!(chip_text(&QueueStatePayload { running: Some(item("push dev")), queued: vec![item("a"), item("b")], stopped: None }), "Running: push dev · 2 queued");
        assert_eq!(chip_text(&QueueStatePayload { running: None, queued: vec![item("a")], stopped: Some(QueueStop { label: "push dev".into(), message: "m".into() }) }), "Stopped: push dev failed · 1 not run");
        assert!(queue_idle(Some(&QueueStatePayload::default())));
    }
}
