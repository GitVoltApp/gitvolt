//! Stashes. Identity is the stash commit's oid, never a list position:
//! `stash@{n}` is found by oid in `git stash list` (a read) under the lock, just before each
//! command, and checked again (`rev-parse`) so a stack that shifted is never acted on blindly.
//! - Push: `git stash push --include-untracked -m <message> [-- <paths>]`; journaled (undo
//!   applies it with `--index` and drops it; redo pushes again and records the new oid). A push
//!   of some paths takes a staged rename's both sides, and keeps only their changes (`narrow`).
//!   Paths are git's bytes throughout (a name needn't be UTF-8).
//! - An apply with `--index` while other files are staged runs on a temp index (`index_aside`):
//!   git's own resets the whole index first, and refuses then, losing what was staged.
//! - Apply: `git stash apply --index <oid>` (or without `--index` once confirmed); not journaled.
//! - Pop: snapshot `before`, apply, and drop on a clean apply; journaled (undo restores
//!   `before` and stores the stash back; redo pops again). A conflicting pop keeps the stash,
//!   as git's own does.
//! - Drop: journaled (undo `git stash store -m <subject> <oid>`; redo drops again).

use crate::api::{blocking, Api};
use crate::error::{gix_err, ErrorDetail, GbError, GbErrorKind};
use crate::events::{ChangeKind, OpKind};
use crate::git::{GitCli, GitInvocation};
use crate::journal::{snapshot, JournalEntry, StashMove, UndoKind};
use crate::write::types::WriteResult;
use crate::write::{precheck, run_write, Plan, Pre, WriteCx, WriteIntent};
use serde::Serialize;
use std::path::Path;
use ts_rs::TS;

/// `On main: Fix x` → `Fix x`; `WIP on main: 1a2b3c4 x` → `1a2b3c4 x`.
pub(crate) fn user_message(subject: &str) -> &str {
    for prefix in ["On ", "WIP on "] {
        if let Some(rest) = subject.strip_prefix(prefix)
            && let Some((_, msg)) = rest.split_once(": ")
        {
            return msg;
        }
    }
    subject
}

fn gone() -> GbError {
    GbError::new(GbErrorKind::NotFound, "That stash is gone")
}

/// `(oid, reflog subject)` of every stash, newest first (a read).
pub(crate) async fn stash_list(cli: &GitCli, root: &Path) -> Result<Vec<(String, String)>, GbError> {
    let out = cli.run(GitInvocation::new(root, ["stash", "list", "--format=%H%x00%gs"])).await?;
    Ok(String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.split_once('\0').map(|(o, s)| (o.to_string(), s.to_string()))).collect())
}

/// The subject of the stash `oid` (its newest listing), or "That stash is gone".
async fn subject_of(cli: &GitCli, root: &Path, oid: &str) -> Result<String, GbError> {
    stash_list(cli, root).await?.into_iter().find(|(o, _)| o == oid).map(|(_, s)| s).ok_or_else(gone)
}

/// What `stash@{n}` is now (a read).
async fn at(cli: &GitCli, root: &Path, n: usize) -> Option<String> {
    let at = format!("stash@{{{n}}}");
    let out = cli.run(GitInvocation::new(root, ["rev-parse", "--verify", "-q", at.as_str()])).await.ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `git stash drop stash@{n}`, once `stash@{n}` is checked to be `oid` just before. git's
/// `Dropped stash@{n} (<oid>)` is checked after: another process's stash dropped
/// in between is stored back at once, and the drop refused. `cancellable: false`: the op's
/// Cancel can't stop it midway (the multi-line push's drop/store pair).
pub(crate) async fn drop_at(cx: &mut WriteCx<'_>, n: usize, oid: &str, cancellable: bool) -> Result<(), GbError> {
    if at(&cx.api.cli, cx.root, n).await.as_deref() != Some(oid) {
        return Err(GbError::stale("The stash list changed; nothing was dropped"));
    }
    let at = format!("stash@{{{n}}}");
    let args = ["stash", "drop", at.as_str()];
    let inv = if cancellable { cx.git(args) } else { cx.git_stash(args) };
    let out = cx.run_git(inv).await?;
    cx.touch(ChangeKind::Stash);
    match dropped_oid(&String::from_utf8_lossy(&out.stdout)) {
        Some(other) if other != oid => {
            // Its reflog line is gone with it: the commit's subject is the same line (`On x: …`).
            let subject = cx.api.cli.run(GitInvocation::new(cx.root, ["show", "-s", "--format=%s", other.as_str()])).await.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
            let inv = cx.git_stash(["stash", "store", "-m", subject.as_str(), other.as_str()]);
            if let Err(e) = cx.run_git(inv).await {
                tracing::warn!(target: "gitvolt_core::write", "storing back the stash {other} dropped by mistake: {e}");
            }
            Err(GbError::stale("The stash list changed; nothing was dropped"))
        }
        _ => Ok(()),
    }
}

/// `Dropped refs/stash@{0} (1a2b…)` → `1a2b…`.
fn dropped_oid(printed: &str) -> Option<String> {
    printed.lines().find(|l| l.starts_with("Dropped "))?.rsplit_once('(')?.1.split_once(')').map(|(o, _)| o.trim().to_string())
}

/// `git stash drop stash@{n}`, `n` found by oid and checked again just before.
pub(crate) async fn drop_oid(cx: &mut WriteCx<'_>, oid: &str) -> Result<(), GbError> {
    let n = stash_list(&cx.api.cli, cx.root).await?.iter().position(|(o, _)| o == oid).ok_or_else(gone)?;
    drop_at(cx, n, oid, true).await
}

/// A stash the user popped or dropped: a kept record for it (an autostash's banner) would
/// now offer Apply of a stash that's gone, so it goes too. Best effort: the
/// banner is also hidden at read time while its stash isn't listed (`Api::journal_state`).
fn forget_kept(cx: &mut WriteCx<'_>, oid: &str) {
    match cx.api.journal(cx.root).and_then(|s| s.update(|j| j.forget_kept_stash(oid))) {
        Ok(gone) => cx.journal_changed |= gone,
        Err(e) => tracing::warn!(target: "gitvolt_core::write", "clearing the kept record of stash {oid}: {e}"),
    }
}

/// After a failed drop: the move recorded ahead goes when the stash is still listed (nothing
/// was dropped). `true`: it went.
async fn unrecord_if_listed(cx: &mut WriteCx<'_>, oid: &str) -> bool {
    if subject_of(&cx.api.cli, cx.root, oid).await.is_err() {
        return false;
    }
    if let Err(e) = cx.edit_entry(|e| e.stashes.retain(|m| m.oid != oid)) {
        tracing::warn!(target: "gitvolt_core::write", "unrecording the stash {oid}: {e}");
    }
    true
}

/// NUL-separated names from a read.
async fn names(cli: &GitCli, root: &Path, args: &[&str]) -> Result<Vec<String>, GbError> {
    let out = cli.run(GitInvocation::new(root, args.iter().copied())).await?;
    Ok(out.stdout.split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect())
}

/// A `stash push -u` (the Stash button, a file's Stash, the redo of one) is refused where
/// HEAD's file would be written over a repository (`precheck::stash_push_in_the_way`).
pub(crate) async fn refuse_stash_push_in_the_way(pre: &Pre<'_>, what: &str, paths: Option<&[String]>) -> Result<(), GbError> {
    let tracked = precheck::dirty(&pre.api.cli, pre.root).await?.tracked;
    // The check reads paths as text: a rename's other side whose name isn't UTF-8 isn't checked.
    let scope = match paths {
        Some(p) => Some(with_renames(&pre.api.cli, pre.root, p).await?.into_iter().filter_map(|p| String::from_utf8(p).ok()).collect::<Vec<_>>()),
        None => None,
    };
    match Box::pin(precheck::stash_push_in_the_way(&pre.api.cli, pre.root, &tracked, scope.as_deref())).await?.first() {
        Some(p) => Err(precheck::repository_in_the_way(p, what)),
        None => Ok(()),
    }
}

/// An apply of `oid` (apply, pop, the undo of a push, the redo of a pop, a
/// kept stash's Apply) is refused where a file of the stash would be written over a repository
/// (`precheck::stash_apply_in_the_way`).
pub(crate) async fn refuse_stash_apply_in_the_way(pre: &Pre<'_>, oid: &str, what: &str) -> Result<(), GbError> {
    match Box::pin(precheck::stash_apply_in_the_way(&pre.api.cli, pre.root, oid)).await?.first() {
        Some(p) => Err(precheck::repository_in_the_way(p, what)),
        None => Ok(()),
    }
}

/// The paths a stash's apply writes: its worktree and index changes, and its untracked files.
async fn stash_paths(cli: &GitCli, root: &Path, oid: &str) -> Result<Vec<String>, GbError> {
    let (base, index, third) = (format!("{oid}^1"), format!("{oid}^2"), format!("{oid}^3"));
    let mut paths = names(cli, root, &["diff", "--no-renames", "--name-only", "-z", &base, oid]).await?;
    paths.extend(names(cli, root, &["diff", "--no-renames", "--name-only", "-z", &base, &index]).await?);
    // `-q`: a stash without untracked files has no third parent.
    if cli.run(GitInvocation::new(root, ["rev-parse", "--verify", "-q", third.as_str()])).await.is_ok() {
        paths.extend(names(cli, root, &["ls-tree", "-r", "-z", "--name-only", &third]).await?);
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum StashPushOutcome {
    Stashed { oid: String },
    /// git found nothing to stash: no entry, no stash; the toast says so.
    NothingToStash,
}

pub(crate) struct StashPush {
    /// The whole WIP draft, or `WIP on <branch>` (`(no branch)`) when it was empty; for some
    /// paths, `WIP on <branch>: <path>` (`<path> and N more`).
    pub message: String,
    /// Only these files and folders (a WIP file's or folder's Stash); `None`: every change.
    pub paths: Option<Vec<String>>,
}

impl StashPush {
    /// Fills the empty message from HEAD, read before the run, and the paths.
    pub(crate) fn new(message: String, branch: Option<&str>, paths: Option<Vec<String>>) -> Self {
        let on = format!("WIP on {}", branch.unwrap_or("(no branch)"));
        let message = match (&paths, message.trim().is_empty()) {
            (_, false) => message,
            (Some(p), true) if !p.is_empty() => match p.len() - 1 {
                0 => format!("{on}: {}", p[0]),
                n => format!("{on}: {} and {n} more", p[0]),
            },
            _ => on,
        };
        Self { message, paths }
    }
}

/// A path as git's bytes: a name needn't be UTF-8.
type Bytes = Vec<u8>;

/// `p` is `scope`'s path or under one of its folders.
fn within(p: &[u8], scope: &[Bytes]) -> bool {
    scope.iter().any(|s| p == s.as_slice() || p.strip_prefix(s.as_slice()).is_some_and(|rest| rest.first() == Some(&b'/')))
}

/// NUL-separated names from a read, as git's bytes.
async fn raw_names(cli: &GitCli, root: &Path, args: &[&str]) -> Result<Vec<Bytes>, GbError> {
    let out = cli.run(GitInvocation::new(root, args.iter().copied())).await?;
    Ok(out.stdout.split(|b| *b == 0).filter(|s| !s.is_empty()).map(<[u8]>::to_vec).collect())
}

/// Paths for `--pathspec-from-file=- --pathspec-file-nul`.
fn nul_bytes(paths: &[Bytes]) -> Vec<u8> {
    paths.iter().flat_map(|p| p.iter().copied().chain([0])).collect()
}

/// A read's output, its last newline dropped.
async fn read_text(cli: &GitCli, root: &Path, args: &[&str]) -> Result<String, GbError> {
    Ok(String::from_utf8_lossy(&cli.run(GitInvocation::new(root, args.iter().copied())).await?.stdout).trim().to_string())
}

/// `paths` and the other side of each staged rename with a side in them: a stash of one side
/// alone would leave half the rename staged (a deletion, or a copy).
async fn with_renames(cli: &GitCli, root: &Path, paths: &[String]) -> Result<Vec<Bytes>, GbError> {
    let asked: Vec<Bytes> = paths.iter().map(|p| p.as_bytes().to_vec()).collect();
    let mut out = asked.clone();
    // No HEAD: git refuses the stash, and there's no rename.
    if cli.run(GitInvocation::new(root, ["rev-parse", "--verify", "-q", "HEAD"])).await.is_err() {
        return Ok(out);
    }
    let mut it = raw_names(cli, root, &["diff-index", "--cached", "-M", "--name-status", "-z", "HEAD"]).await?.into_iter();
    while let Some(status) = it.next() {
        let Some(a) = it.next() else { break };
        if !status.starts_with(b"R") {
            continue;
        }
        let Some(b) = it.next() else { break };
        if within(&a, &asked) || within(&b, &asked) {
            for p in [a, b] {
                if !within(&p, &out) {
                    out.push(p);
                }
            }
        }
    }
    Ok(out)
}

/// `git stash push --include-untracked -m <message> [-- <paths>]`: the new stash's `(oid,
/// subject)`, or `None` when git found nothing to stash. The caller records it before `relist`.
/// A stash git stored before it failed (its cleanup) is recorded here, and the error says so.
async fn push(cx: &mut WriteCx<'_>, message: &str, paths: Option<&[String]>) -> Result<Option<(String, String)>, GbError> {
    let scope = match paths {
        Some([]) => return Err(no_paths()),
        Some(p) => Some(with_renames(&cx.api.cli, cx.root, p).await?),
        None => None,
    };
    // What's staged in scope, read just before git's push (its index commit has the whole index).
    let staged: Vec<Change> = match &scope {
        Some(scope) => changes(&cx.api.cli, cx.root, &["diff-index", "--cached", "-r", "-z", "--no-renames", "HEAD"]).await?.into_iter().filter(|(p, _)| within(p, scope)).collect(),
        None => Vec::new(),
    };
    // git's `add -- <paths>` refuses a path neither the index nor the worktree has: a staged
    // deletion goes back in the index first (an unstaged one then), and `narrow` stashes it staged.
    let gone: Vec<Bytes> = staged.iter().filter(|(p, s)| s.is_none() && std::fs::symlink_metadata(cx.root.join(crate::platform::osstr::from_bytes(p))).is_err()).map(|(p, _)| p.clone()).collect();
    if !gone.is_empty() {
        let inv = cx.git(["reset", "-q", "--pathspec-from-file=-", "--pathspec-file-nul"]).env("GIT_LITERAL_PATHSPECS", "1").stdin(nul_bytes(&gone));
        cx.run_git(inv).await?;
    }
    let before = crate::journal::autostash::stash_oid(cx.api, cx.root).await?;
    let mut args = vec!["stash", "push", "--include-untracked", "-m", message];
    if scope.is_some() {
        args.extend(["--pathspec-from-file=-", "--pathspec-file-nul"]);
    }
    let mut inv = cx.git(args);
    if let Some(scope) = &scope {
        inv = inv.env("GIT_LITERAL_PATHSPECS", "1").stdin(nul_bytes(scope));
    }
    let ran = cx.run_git(inv).await;
    let after = crate::journal::autostash::stash_oid(cx.api, cx.root).await;
    let stored = match &after {
        Ok(a) if *a != before => a.clone(),
        _ => None,
    };
    let Some(oid) = stored else {
        // Nothing stored: the deletions are staged again, as they were.
        if !gone.is_empty() {
            stage_deletions(cx, &gone).await?;
        }
        ran?;
        after?;
        return Ok(None);
    };
    cx.partial = true;
    for k in [ChangeKind::Stash, ChangeKind::Worktree, ChangeKind::Index] {
        cx.touch(k);
    }
    let subject = subject_of(&cx.api.cli, cx.root, &oid).await?;
    if let Err(e) = ran {
        // git stored the stash, then failed (cleaning the files up): it's recorded, so Undo can
        // apply it, and the error says where the changes are.
        cx.record_stash(StashMove { oid, message: subject.clone(), created: true, without_index: false, paths: paths.map(<[String]>::to_vec) })?;
        let message = format!("git stored the stash \"{}\", then failed: {}. The changes are in the stash; some may still be in the files too", user_message(&subject), e.message);
        return Err(GbError { message, ..e });
    }
    let Some(scope) = &scope else { return Ok(Some((oid, subject))) };
    match narrow(cx, &oid, &subject, scope, &staged, &gone).await {
        Ok(narrowed) => Ok(Some((narrowed, subject))),
        Err(e) => {
            tracing::warn!(target: "gitvolt_core::write", "the stash {oid} is kept as git made it, with the other staged changes: {e}");
            Ok(Some((oid, subject)))
        }
    }
}

/// Stages `gone`'s deletions again, retrying a moment on another process's `index.lock`.
async fn stage_deletions(cx: &mut WriteCx<'_>, gone: &[Bytes]) -> Result<(), GbError> {
    for tries in 1.. {
        let inv = cx.git_stash(["rm", "--cached", "-q", "--ignore-unmatch", "--pathspec-from-file=-", "--pathspec-file-nul"]).env("GIT_LITERAL_PATHSPECS", "1").stdin(nul_bytes(gone));
        match cx.run_git(inv).await {
            Ok(_) => return Ok(()),
            Err(_) if tries < 5 => tokio::time::sleep(std::time::Duration::from_millis(100 * tries)).await,
            Err(e) => {
                let names: Vec<_> = gone.iter().map(|p| String::from_utf8_lossy(p)).collect();
                return Err(GbError { message: format!("The deletion of {} is no longer staged; stage it again ({})", names.join(", "), e.message), ..e });
            }
        }
    }
    unreachable!()
}

fn no_paths() -> GbError {
    GbError::new(GbErrorKind::InvalidInput, "No files to stash")
}

/// A change `changes` lists: the path, and its new `(mode, oid)` (`None`: deleted).
type Change = (Bytes, Option<(String, String)>);

/// Each file a raw diff (`diff-tree -r -z`, `diff-index -z`) lists as changed.
async fn changes(cli: &GitCli, root: &Path, args: &[&str]) -> Result<Vec<Change>, GbError> {
    let listed = raw_names(cli, root, args).await?;
    // `:<old mode> <new mode> <old oid> <new oid> <status>`, then the path.
    Ok(listed
        .chunks(2)
        .filter_map(|c| {
            let [meta, path] = c else { return None };
            let meta = String::from_utf8_lossy(meta);
            let f: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
            let (mode, oid, status) = (f.get(1)?, f.get(3)?, f.get(4)?);
            Some((path.clone(), (!status.starts_with('D')).then(|| (mode.to_string(), oid.to_string()))))
        })
        .collect())
}

/// `base`'s tree with `changes` made, through a temp index.
async fn tree_with(cx: &WriteCx<'_>, base: &str, changes: &[&Change]) -> Result<String, GbError> {
    let dir = tempfile::Builder::new().prefix("stash-").tempdir_in(&cx.tmp)?;
    let temp = dir.path().join("index");
    let mut info = Vec::new();
    for (p, s) in changes {
        let meta = match s {
            Some((mode, oid)) => format!("{mode} {oid}\t"),
            None => format!("0 {}\t", "0".repeat(base.len())),
        };
        info.extend_from_slice(meta.as_bytes());
        info.extend_from_slice(p);
        info.push(0);
    }
    cx.api.cli.run(cx.git_stash(["read-tree", base]).env("GIT_INDEX_FILE", &temp)).await?;
    cx.api.cli.run(cx.git_stash(["update-index", "--replace", "-z", "--index-info"]).env("GIT_INDEX_FILE", &temp).stdin(info)).await?;
    let out = cx.api.cli.run(cx.git_stash(["write-tree"]).env("GIT_INDEX_FILE", &temp)).await?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `git commit-tree` of `tree` with `of`'s message, author and committer: the identity and
/// dates git chose for the stash (its fallback one too, when none is configured), kept.
async fn commit_like(cx: &WriteCx<'_>, of: &str, tree: &str, parents: &[&str]) -> Result<String, GbError> {
    let message = cx.api.cli.run(GitInvocation::new(cx.root, ["show", "-s", "--format=%B", of])).await?.stdout;
    let mut who = cx.api.cli.run(GitInvocation::new(cx.root, ["show", "-s", "--date=raw", "--format=%an%x00%ae%x00%ad%x00%cn%x00%ce%x00%cd", of])).await?.stdout;
    if who.last() == Some(&b'\n') {
        who.pop();
    }
    let fields: Vec<&[u8]> = who.split(|b| *b == 0).collect();
    let [an, ae, ad, cn, ce, cd] = fields.as_slice() else { return Err(GbError::other(format!("unreadable identity on {of}"))) };
    let mut args = vec!["commit-tree", "--no-gpg-sign", tree, "-F", "-"];
    for p in parents {
        args.extend(["-p", p]);
    }
    let mut inv = cx.git_stash(args).stdin(message);
    for (k, v) in [("GIT_AUTHOR_NAME", an), ("GIT_AUTHOR_EMAIL", ae), ("GIT_AUTHOR_DATE", ad), ("GIT_COMMITTER_NAME", cn), ("GIT_COMMITTER_EMAIL", ce), ("GIT_COMMITTER_DATE", cd)] {
        inv = inv.env(k, crate::platform::osstr::from_vec(v.to_vec()));
    }
    let out = cx.api.cli.run(inv).await?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// git records the whole index in a stash of some paths, and W on top of it: the other staged
/// files would be in the stash too. Unless it already holds just the scope's changes, the stash
/// is made again from its base: I with `staged` (the scope's index just before the push, a
/// staged deletion git saw unstaged too), W with the scope's changes, its untracked files'
/// commit kept. It's checked against git's before it replaces it (stored on top, git's then
/// dropped: both are listed between the two, never neither); any difference keeps git's.
/// Returns the stash listed now.
async fn narrow(cx: &mut WriteCx<'_>, oid: &str, subject: &str, scope: &[Bytes], staged: &[Change], gone: &[Bytes]) -> Result<String, GbError> {
    let parents = read_text(&cx.api.cli, cx.root, &["show", "-s", "--format=%P", oid]).await?;
    let parents: Vec<&str> = parents.split(' ').collect();
    let [base, index, untracked @ ..] = parents.as_slice() else { return Ok(oid.to_string()) };
    let diff = async |cx: &WriteCx<'_>, to: &str| changes(&cx.api.cli, cx.root, &["diff-tree", "-r", "-z", "--no-renames", base, to]).await;
    let (w, i) = (diff(cx, oid).await?, diff(cx, index).await?);
    if gone.is_empty() && w.iter().chain(&i).all(|(p, _)| within(p, scope)) {
        return Ok(oid.to_string());
    }
    let w: Vec<&Change> = w.iter().filter(|(p, _)| within(p, scope)).collect();
    // git's index part for the scope is what was staged, but for the deletions put back first.
    let i: Vec<&Change> = i.iter().filter(|(p, _)| within(p, scope)).collect();
    let expected: Vec<&Change> = staged.iter().filter(|(p, _)| !gone.contains(p)).collect();
    if i != expected {
        return Err(GbError::other("the index changed during the stash"));
    }
    let i_commit = commit_like(cx, index, &tree_with(cx, base, &staged.iter().collect::<Vec<_>>()).await?, &[base]).await?;
    let w_parents: Vec<&str> = [*base, i_commit.as_str()].into_iter().chain(untracked.iter().copied()).collect();
    let narrowed = commit_like(cx, oid, &tree_with(cx, base, &w).await?, &w_parents).await?;
    // Checked before git's goes: W and I hold exactly the scope's changes (nothing else differs
    // from the base), and the untracked files are git's.
    let (w_now, i_now) = (diff(cx, &narrowed).await?, diff(cx, &i_commit).await?);
    let parents_now = read_text(&cx.api.cli, cx.root, &["show", "-s", "--format=%P", &narrowed]).await?;
    if w_now.iter().collect::<Vec<_>>() != w || i_now.as_slice() != staged || parents_now != w_parents.join(" ") {
        return Err(GbError::other(format!("the narrowed stash {narrowed} doesn't hold what git's does")));
    }
    let inv = cx.git_stash(["stash", "store", "-m", subject, narrowed.as_str()]);
    cx.run_git(inv).await?;
    cx.touch(ChangeKind::Stash);
    if let Err(e) = drop_at(cx, 1, oid, false).await {
        tracing::warn!(target: "gitvolt_core::write", "git's own stash {oid} stays listed: {e}");
    }
    Ok(narrowed)
}

/// git folds a multi-line `-m` into one reflog line (`On main: Fix x the details`); the list
/// shows the summary only, so the stash is stored again under `On main: Fix x` and the folded
/// line dropped. The stash commit keeps the whole message. Returns its subject now.
///
/// A store of the oid already on top logs nothing, so it's a drop, then a store, neither
/// cancellable. A pending kept-stash record covers the gap between them: a crash raises its
/// Recovery banner, and so does a store that fails. The push succeeded whatever happens here:
/// when the stash isn't on top any more (another process pushed) or the drop is refused, the
/// folded line stays.
async fn relist(cx: &mut WriteCx<'_>, oid: &str, subject: &str, message: &str, label: &str) -> String {
    let summary = message.lines().next().unwrap_or_default().trim();
    let want = format!("{}{summary}", &subject[..subject.len() - user_message(subject).len()]);
    if !message.trim_end().contains('\n') || want == subject {
        return subject.to_string();
    }
    match stash_list(&cx.api.cli, cx.root).await {
        Ok(list) if list.first().is_some_and(|(o, _)| o == oid) => {}
        _ => return subject.to_string(),
    }
    let guard = match guard(cx, oid, summary, label) {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!(target: "gitvolt_core::write", "the stash {oid} keeps its folded line: {e}");
            return subject.to_string();
        }
    };
    if let Err(e) = drop_at(cx, 0, oid, false).await {
        let listed = subject_of(&cx.api.cli, cx.root, oid).await;
        settle_guard(cx, guard, listed.is_ok());
        tracing::warn!(target: "gitvolt_core::write", "the stash {oid} keeps its folded line: {e}");
        return listed.unwrap_or_else(|_| subject.to_string());
    }
    let inv = cx.git_stash(["stash", "store", "-m", want.as_str(), oid]);
    let stored = cx.run_git(inv).await;
    cx.touch(ChangeKind::Stash);
    if let Err(e) = &stored {
        tracing::warn!(target: "gitvolt_core::write", "the stash {oid} couldn't be listed again: {e}");
    }
    let listed = subject_of(&cx.api.cli, cx.root, oid).await;
    settle_guard(cx, guard, listed.is_ok());
    listed.unwrap_or_else(|_| subject.to_string())
}

/// A `Pending` kept-stash record for `oid`, owned by this instance (no banner while it runs).
fn guard(cx: &mut WriteCx<'_>, oid: &str, message: &str, label: &str) -> Result<u64, GbError> {
    let owner = cx.api.owner()?;
    let k = crate::journal::KeptStash {
        id: 0,
        oid: Some(oid.to_string()),
        stash_before: None,
        message: message.to_string(),
        label: label.to_string(),
        target: None,
        reason: crate::journal::KeptReason::Pending,
        created_ms: cx.api.now(),
        owner: Some(owner),
    };
    cx.api.journal(cx.root)?.update(|j| j.keep(k))
}

/// The guard goes when the stash is listed again; otherwise its Recovery banner shows now.
fn settle_guard(cx: &mut WriteCx<'_>, id: u64, listed: bool) {
    let res = cx.api.journal(cx.root).and_then(|s| {
        s.update(|j| {
            if listed {
                j.kept.retain(|k| k.id != id);
            } else if let Some(k) = j.kept_mut(id) {
                k.reason = crate::journal::KeptReason::Interrupted;
                k.owner = None;
            }
        })
    });
    if let Err(e) = res {
        tracing::warn!(target: "gitvolt_core::write", "the stash record {id}: {e}");
    }
    cx.journal_changed = true;
}

impl WriteIntent for StashPush {
    type Outcome = StashPushOutcome;
    fn kind(&self) -> OpKind {
        OpKind::Stash
    }
    fn label(&self) -> String {
        format!("stash \"{}\"", self.message.lines().next().unwrap_or_default().trim())
    }
    fn undo(&self) -> Option<UndoKind> {
        Some(UndoKind::Stash)
    }
    /// `stash push -u` ends in a `reset --hard` that writes HEAD's file over a directory in its
    /// place, deleting a repository in it whole: refused first.
    async fn plan(&self, pre: &Pre<'_>) -> Result<Plan, GbError> {
        if self.paths.as_ref().is_some_and(Vec::is_empty) {
            return Err(no_paths());
        }
        refuse_stash_push_in_the_way(pre, "stash", self.paths.as_deref()).await?;
        Ok(Plan::default())
    }
    async fn run(&self, cx: &mut WriteCx<'_>) -> Result<StashPushOutcome, GbError> {
        let Some((oid, subject)) = push(cx, &self.message, self.paths.as_deref()).await? else { return Ok(StashPushOutcome::NothingToStash) };
        // Written ahead of the relist's drop.
        cx.record_stash(StashMove { oid: oid.clone(), message: subject.clone(), created: true, without_index: false, paths: self.paths.clone() })?;
        let now = relist(cx, &oid, &subject, &self.message, &self.label()).await;
        if now != subject {
            cx.edit_entry(|e| {
                if let Some(m) = e.stashes.iter_mut().find(|m| m.oid == oid) {
                    m.message = now;
                }
            })?;
        }
        Ok(StashPushOutcome::Stashed { oid })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum StashApplyOutcome {
    Applied,
    /// The files land in Conflicted; the stash is kept.
    Conflicts { files: u32 },
}

fn without_index() -> GbError {
    GbError::new(GbErrorKind::Conflict, "git couldn't restore what was staged").with_detail(ErrorDetail::ApplyWithoutIndex)
}

/// git's `apply --index` resets the whole index to HEAD before its merge, which then refuses
/// over any staged change ("Index was not unstashed"), the staged changes lost. When the stash
/// has an index part and something is staged, the apply runs on a temp index instead, a copy of
/// the real one reset to HEAD (its stat data kept), and only the stash's paths are taken back
/// (`take_back`): the other files' staged state is never touched. A path of the stash that's
/// staged now is git's refusal: the question to apply without the index, nothing changed.
struct Aside {
    _dir: tempfile::TempDir,
    index: std::path::PathBuf,
    /// The stash's tracked paths: its index and worktree changes.
    paths: Vec<Bytes>,
}

async fn index_aside(cx: &WriteCx<'_>, oid: &str) -> Result<Option<Aside>, GbError> {
    let (cli, root) = (&cx.api.cli, cx.root);
    let tree = |c: &str| format!("{oid}{c}^{{tree}}");
    if read_text(cli, root, &["rev-parse", &tree("^1")]).await? == read_text(cli, root, &["rev-parse", &tree("^2")]).await? {
        return Ok(None);
    }
    let staged = raw_names(cli, root, &["diff-index", "--cached", "--name-only", "--no-renames", "-z", "HEAD"]).await?;
    if staged.is_empty() {
        return Ok(None);
    }
    let (base, index) = (format!("{oid}^1"), format!("{oid}^2"));
    let mut paths = raw_names(cli, root, &["diff-tree", "-r", "--name-only", "--no-renames", "-z", &base, oid]).await?;
    paths.extend(raw_names(cli, root, &["diff-tree", "-r", "--name-only", "--no-renames", "-z", &base, &index]).await?);
    paths.sort();
    paths.dedup();
    if paths.iter().any(|p| staged.contains(p)) {
        return Err(without_index());
    }
    let dir = tempfile::Builder::new().prefix("stash-apply-").tempdir_in(&cx.tmp)?;
    let index = dir.path().join("index");
    let real = gix::open(root).map_err(gix_err)?.index_path();
    if real.is_file() {
        std::fs::copy(&real, &index)?;
    }
    cli.run(cx.git_stash(["reset", "-q"]).env("GIT_INDEX_FILE", &index)).await?;
    Ok(Some(Aside { _dir: dir, index, paths }))
}

/// The stash's paths as the apply left them in the temp index (conflict stages too), written
/// to the real one; nothing else in it changes.
async fn take_back(cx: &WriteCx<'_>, aside: &Aside, zero: &str) -> Result<(), GbError> {
    let listed = cx.api.cli.run(GitInvocation::new(cx.root, ["ls-files", "-s", "-z"]).env("GIT_INDEX_FILE", &aside.index)).await?.stdout;
    let mut info = Vec::new();
    for p in &aside.paths {
        info.extend_from_slice(format!("0 {zero}\t").as_bytes());
        info.extend_from_slice(p);
        info.push(0);
    }
    // `<mode> <oid> <stage>\t<path>`, as `--index-info` takes it.
    for rec in listed.split(|b| *b == 0) {
        if let Some(tab) = rec.iter().position(|b| *b == b'\t')
            && aside.paths.iter().any(|p| p.as_slice() == &rec[tab + 1..])
        {
            info.extend_from_slice(rec);
            info.push(0);
        }
    }
    cx.api.cli.run(cx.git_stash(["update-index", "-z", "--index-info"]).stdin(info)).await?;
    Ok(())
}

/// `git stash apply [--index] <oid>`; `Ok(None)` on a clean apply, `Ok(Some(files))` with
/// content conflicts; git refusing the index is `ApplyWithoutIndex` (nothing changed).
async fn apply(cx: &mut WriteCx<'_>, oid: &str, with_index: bool) -> Result<Option<u32>, GbError> {
    // git prints a merge's CONFLICT lines on stdout, which a failure doesn't keep: conflicts are
    // the unmerged paths the apply added.
    let unmerged_before = precheck::dirty(&cx.api.cli, cx.root).await?.conflicted;
    // With conflicts already, git refuses the apply: as it is.
    let aside = if with_index && unmerged_before == 0 { index_aside(cx, oid).await? } else { None };
    let mut args = vec!["stash", "apply"];
    if with_index {
        args.push("--index");
    }
    args.push(oid);
    let mut inv = cx.git(args);
    if let Some(a) = &aside {
        inv = inv.env("GIT_INDEX_FILE", &a.index);
    }
    let res = cx.run_git(inv).await;
    cx.touch(ChangeKind::Worktree);
    cx.touch(ChangeKind::Index);
    let refused = |e: &GbError| {
        let stderr = e.stderr.as_deref().unwrap_or_default();
        stderr.contains("Conflicts in index") || stderr.contains("without --index")
    };
    if let Some(a) = &aside {
        // Taken back after a merge git ran: clean, or with conflicts (unmerged in the temp index).
        let merged = match &res {
            Ok(_) => true,
            Err(e) if refused(e) => false,
            Err(_) => cx.api.cli.run(GitInvocation::new(cx.root, ["ls-files", "-u", "-z"]).env("GIT_INDEX_FILE", &a.index)).await.is_ok_and(|o| !o.stdout.is_empty()),
        };
        if merged {
            cx.partial = true;
            take_back(cx, a, &"0".repeat(oid.len())).await?;
        }
    }
    let e = match res {
        Ok(_) => {
            cx.partial = true;
            return Ok(None);
        }
        Err(e) => e,
    };
    if with_index && refused(&e) {
        return Err(without_index());
    }
    // From here the apply may have written some of it.
    cx.partial = true;
    let now = precheck::dirty(&cx.api.cli, cx.root).await?.conflicted;
    if now > unmerged_before { Ok(Some(now - unmerged_before)) } else { Err(e) }
}

pub(crate) struct StashApply {
    pub oid: String,
    pub pop: bool,
    pub without_index: bool,
    /// The stash's subject, read by the request (the label).
    pub subject: String,
}

impl WriteIntent for StashApply {
    type Outcome = StashApplyOutcome;
    fn kind(&self) -> OpKind {
        OpKind::Stash
    }
    fn label(&self) -> String {
        format!("{} stash \"{}\"", if self.pop { "pop" } else { "apply" }, user_message(&self.subject))
    }
    /// Apply isn't journaled (the stash is kept); pop is.
    fn undo(&self) -> Option<UndoKind> {
        self.pop.then_some(UndoKind::Stash)
    }
    async fn plan(&self, pre: &Pre<'_>) -> Result<Plan, GbError> {
        subject_of(&pre.api.cli, pre.root, &self.oid).await?;
        refuse_stash_apply_in_the_way(pre, &self.oid, if self.pop { "pop" } else { "apply" }).await?;
        if !self.pop {
            return Ok(Plan::default());
        }
        let paths = stash_paths(&pre.api.cli, pre.root, &self.oid).await?;
        let present_untracked = precheck::untracked_among(&pre.api.cli, pre.root, &paths).await?;
        Ok(Plan { snapshot: Some((paths, present_untracked)), ..Plan::default() })
    }
    async fn run(&self, cx: &mut WriteCx<'_>) -> Result<StashApplyOutcome, GbError> {
        match apply(cx, &self.oid, !self.without_index).await? {
            Some(files) => Ok(StashApplyOutcome::Conflicts { files }),
            None => {
                if self.pop {
                    // Written ahead of the drop.
                    cx.record_stash(StashMove { oid: self.oid.clone(), message: self.subject.clone(), created: false, without_index: self.without_index, paths: None })?;
                    if let Err(e) = drop_oid(cx, &self.oid).await {
                        unrecord_if_listed(cx, &self.oid).await;
                        return Err(e);
                    }
                    forget_kept(cx, &self.oid);
                    // The pop is done: a failed `after` only costs its undo.
                    match after_snapshot(cx, &self.label()).await {
                        Ok(after) => cx.after = Some(after),
                        Err(e) => {
                            tracing::warn!(target: "gitvolt_core::write", "{}: the after snapshot failed: {e}", self.label());
                            cx.edit_entry(|e| e.blocked = Some(NO_AFTER.to_string()))?;
                        }
                    }
                }
                Ok(StashApplyOutcome::Applied)
            }
        }
    }
}

/// Why a pop whose `after` snapshot failed can't be undone.
const NO_AFTER: &str = "Can't be undone: GitVolt couldn't record what the pop left";

/// What a clean pop left in its `before` snapshot's paths: an undo autostashes only what
/// changed since (`changed_since`), as a Restore's does.
async fn after_snapshot(cx: &WriteCx<'_>, label: &str) -> Result<crate::journal::Snapshot, GbError> {
    let paths = cx.snapshot.as_ref().map(|s| s.paths.clone()).unwrap_or_default();
    snapshot::create(&cx.snapshots(), label, &paths, &[]).await
}

pub(crate) struct StashDrop {
    pub oid: String,
    pub subject: String,
}

impl WriteIntent for StashDrop {
    type Outcome = ();
    fn kind(&self) -> OpKind {
        OpKind::Stash
    }
    fn label(&self) -> String {
        format!("drop stash \"{}\"", user_message(&self.subject))
    }
    fn undo(&self) -> Option<UndoKind> {
        Some(UndoKind::Stash)
    }
    async fn run(&self, cx: &mut WriteCx<'_>) -> Result<(), GbError> {
        // Written ahead of the drop: a crash between them leaves a banner.
        cx.record_stash(StashMove { oid: self.oid.clone(), message: self.subject.clone(), created: false, without_index: false, paths: None })?;
        if let Err(e) = drop_oid(cx, &self.oid).await {
            if unrecord_if_listed(cx, &self.oid).await {
                // Nothing changed: no entry.
                cx.partial = false;
            }
            return Err(e);
        }
        forget_kept(cx, &self.oid);
        Ok(())
    }
}

/// `git stash store -m <subject> <oid>`: the stash returns at the top of the list.
async fn store(cx: &mut WriteCx<'_>, m: &StashMove) -> Result<(), GbError> {
    let inv = cx.git(["stash", "store", "-m", m.message.as_str(), m.oid.as_str()]);
    cx.run_git(inv).await?;
    cx.touch(ChangeKind::Stash);
    Ok(())
}

/// Edits the entry being undone or redone (an undo is a write of its own, without an entry).
fn edit(cx: &mut WriteCx<'_>, id: u64, f: impl FnOnce(&mut JournalEntry)) -> Result<(), GbError> {
    cx.journal_changed = true;
    cx.api.journal(cx.root)?.update(|j| match j.entry_mut(id) {
        Some(e) => f(e),
        None => tracing::warn!(target: "gitvolt_core::write", "journal entry {id} is gone: this edit is lost"),
    })
}

/// `UndoKind::Stash`, told apart by what the entry recorded:
/// - created: undo applies it (`--index`) and drops it; redo pushes again (the new oid);
/// - dropped, no `before`: undo stores it back; redo drops it;
/// - `before` (a pop): undo restores `before`, then stores the stash back if the pop dropped
///   it; redo pops again, snapshotting first.
///
/// `without_index`: "Apply without restoring what was staged?" was confirmed: the
/// push's undo and the pop's redo ask it as Apply does. A pop made without the index redoes
/// without it.
pub(crate) async fn undo_stash(cx: &mut WriteCx<'_>, entry: &JournalEntry, undo: bool, label: &str, without_index: bool) -> Result<(), GbError> {
    let moved = entry.stashes.first().cloned();
    match (moved, &entry.before) {
        // The stash itself is applied (a push of some paths too: what it holds, whatever
        // changed while it was made), with `--index` around any other staged files (`apply`).
        (Some(m), _) if m.created => {
            if undo {
                subject_of(&cx.api.cli, cx.root, &m.oid).await?;
                if apply(cx, &m.oid, !without_index).await?.is_some() {
                    return Err(GbError::new(GbErrorKind::Conflict, "The stash's changes conflict; it was kept"));
                }
                drop_oid(cx, &m.oid).await
            } else {
                // The whole message (summary and description) from the old stash commit, which
                // is still there (dangling); the reflog subject is only its first line.
                let full = cx.api.cli.run(GitInvocation::new(cx.root, ["show", "-s", "--format=%B", m.oid.as_str()])).await.map(|o| String::from_utf8_lossy(&o.stdout).trim_end().to_string()).unwrap_or_else(|_| m.message.clone());
                let message = user_message(&full).to_string();
                let Some((oid, subject)) = push(cx, &message, m.paths.as_deref()).await? else { return Err(GbError::new(GbErrorKind::InvalidInput, "Nothing to stash")) };
                // Written ahead of the relist's drop.
                let new = |subject: String| StashMove { oid: oid.clone(), message: subject, created: true, without_index: false, paths: m.paths.clone() };
                edit(cx, entry.id, |e| {
                    if let Some(s) = e.stashes.first_mut() {
                        *s = new(subject.clone());
                    }
                })?;
                let now = relist(cx, &oid, &subject, &message, label).await;
                if now == subject {
                    return Ok(());
                }
                edit(cx, entry.id, |e| {
                    if let Some(s) = e.stashes.first_mut() {
                        *s = new(now);
                    }
                })
            }
        }
        (moved, Some(before)) => {
            if undo {
                // The stash goes back first, beyond a Cancel: a restore that
                // then fails or is cancelled leaves the changes in the stash, never in neither.
                // Already listed (a retried undo): not stored twice.
                if let Some(m) = &moved
                    && subject_of(&cx.api.cli, cx.root, &m.oid).await.is_err()
                {
                    let inv = cx.git_stash(["stash", "store", "-m", m.message.as_str(), m.oid.as_str()]);
                    cx.run_git(inv).await?;
                    cx.touch(ChangeKind::Stash);
                }
                snapshot::restore(&cx.snapshots(), before).await?;
                cx.touch(ChangeKind::Worktree);
                cx.touch(ChangeKind::Index);
                Ok(())
            } else {
                let m = moved.ok_or_else(|| GbError::new(GbErrorKind::InvalidInput, "That pop kept its stash; nothing to redo"))?;
                let oid = m.oid.clone();
                let without = without_index || m.without_index;
                subject_of(&cx.api.cli, cx.root, &oid).await?;
                let paths = stash_paths(&cx.api.cli, cx.root, &oid).await?;
                let untracked = precheck::untracked_among(&cx.api.cli, cx.root, &paths).await?;
                let snap = snapshot::create(&cx.snapshots(), label, &paths, &untracked).await?;
                if apply(cx, &oid, !without).await?.is_some() {
                    edit(cx, entry.id, |e| e.before = Some(snap))?;
                    return Err(GbError::new(GbErrorKind::Conflict, "The stash's changes conflict; it was kept"));
                }
                drop_oid(cx, &oid).await?;
                let after = snapshot::create(&cx.snapshots(), label, &snap.paths, &[]).await;
                if let Err(e) = &after {
                    tracing::warn!(target: "gitvolt_core::write", "redo {label}: the after snapshot failed: {e}");
                }
                edit(cx, entry.id, |e| {
                    e.before = Some(snap);
                    e.blocked = after.is_err().then(|| NO_AFTER.to_string());
                    e.after = after.ok();
                    if let Some(s) = e.stashes.first_mut() {
                        s.without_index = without;
                    }
                })
            }
        }
        (Some(m), None) => {
            if undo {
                store(cx, &m).await
            } else {
                drop_oid(cx, &m.oid).await
            }
        }
        (None, None) => Ok(()),
    }
}

// --- The requests ---

/// The subject of `oid` in `worktree`'s list, read before the write (its label).
async fn request_subject(api: &Api, repo: u32, worktree: &str, oid: &str) -> Result<String, GbError> {
    let h = api.handle(repo)?;
    let root = api.worktree_dir(&h, worktree).await?;
    subject_of(&api.cli, &root, oid).await
}

/// `stashPush`: an empty `message` names HEAD's branch, read first (and the paths, if any).
pub(crate) async fn stash_push(api: &Api, repo: u32, worktree: &str, message: String, paths: Option<Vec<String>>) -> Result<WriteResult<StashPushOutcome>, GbError> {
    let h = api.handle(repo)?;
    let root = api.worktree_dir(&h, worktree).await?;
    let branch = blocking(move || crate::write::head_state(&gix::open(&root).map_err(gix_err)?)).await?.branch;
    run_write(api, repo, worktree, Default::default(), StashPush::new(message, branch.as_deref(), paths)).await
}

/// `stashApply` (Apply, or Pop).
pub(crate) async fn stash_apply(api: &Api, repo: u32, worktree: &str, oid: String, pop: bool, without_index: bool) -> Result<WriteResult<StashApplyOutcome>, GbError> {
    let subject = request_subject(api, repo, worktree, &oid).await?;
    run_write(api, repo, worktree, Default::default(), StashApply { oid, pop, without_index, subject }).await
}

/// `stashDrop` (Delete).
pub(crate) async fn stash_drop(api: &Api, repo: u32, worktree: &str, oid: String) -> Result<WriteResult<()>, GbError> {
    let subject = request_subject(api, repo, worktree, &oid).await?;
    run_write(api, repo, worktree, Default::default(), StashDrop { oid, subject }).await
}

#[cfg(test)]
mod tests {
    use super::user_message;
    use crate::error::GbErrorKind;
    use crate::testing::state::RepoState;
    use crate::testing::write::{identity, open, send, wt, WriteEnv};
    use crate::testing::TestRepo;
    use serde_json::json;

    fn repo() -> TestRepo {
        let r = TestRepo::new();
        identity(&r);
        r.write("a.txt", "a\n");
        r.write("b.txt", "b\n");
        r.git(&["add", "."]);
        r.git(&["commit", "-q", "-m", "one"]);
        r
    }

    /// a staged, b unstaged, c untracked.
    fn dirty(r: &TestRepo) {
        r.write("a.txt", "a staged\n");
        r.git(&["add", "a.txt"]);
        r.write("b.txt", "b unstaged\n");
        r.write("c.txt", "c untracked\n");
    }

    fn top(r: &TestRepo) -> String {
        r.git(&["rev-parse", "refs/stash"])
    }

    async fn journal(api: &crate::api::Api, id: u32, r: &TestRepo) -> serde_json::Value {
        send(api, json!({"method": "journalState", "params": {"repo": id, "worktree": wt(r)}})).await.unwrap()
    }

    async fn undo_redo(api: &crate::api::Api, id: u32, r: &TestRepo, before: &RepoState, after_check: impl Fn(&RepoState)) {
        let s = journal(api, id, r).await;
        send(api, json!({"method": "undo", "params": {"repo": id, "worktree": wt(r), "entry": s["undo"]["entry"]}})).await.unwrap();
        assert_eq!(&RepoState::capture(r), before, "undo");
        let s = journal(api, id, r).await;
        send(api, json!({"method": "redo", "params": {"repo": id, "worktree": wt(r), "entry": s["redo"]["entry"]}})).await.unwrap();
        after_check(&RepoState::capture(r));
    }

    #[test]
    fn user_messages_drop_gits_prefix() {
        assert_eq!(user_message("On main: Fix x"), "Fix x");
        assert_eq!(user_message("WIP on feature/x: 1a2b3c4 Subject"), "1a2b3c4 Subject");
        assert_eq!(user_message("On (no branch): x"), "x");
        assert_eq!(user_message("plain"), "plain");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_named_stash_round_trips_with_its_split() {
        let env = WriteEnv::new();
        let r = repo();
        dirty(&r);
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        let out = send(&env.api, json!({"method": "stashPush", "params": {"repo": id, "worktree": wt(&r), "message": "Fix x\n\nthe details"}})).await.unwrap();
        assert_eq!(out["outcome"]["status"], "stashed");
        assert_eq!(r.git(&["stash", "list", "--format=%gs"]), "On main: Fix x");
        assert_eq!(r.git(&["show", "-s", "--format=%B", "refs/stash"]), "On main: Fix x\n\nthe details", "the stash commit keeps the whole message");
        assert!(!r.path().join("c.txt").exists(), "untracked files are stashed too");
        assert_eq!(journal(&env.api, id, &r).await["undo"]["label"], "stash \"Fix x\"");
        undo_redo(&env.api, id, &r, &before, |after| {
            assert_eq!(after.stashes.lines().count(), 1, "re-pushed");
            assert!(after.stashes.ends_with("On main: Fix x"));
            assert!(after.untracked.is_empty());
        })
        .await;
        // Redo recorded the new oid: undo again applies that one.
        let s = journal(&env.api, id, &r).await;
        send(&env.api, json!({"method": "undo", "params": {"repo": id, "worktree": wt(&r), "entry": s["undo"]["entry"]}})).await.unwrap();
        assert_eq!(RepoState::capture(&r), before);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_message_names_the_branch_and_nothing_to_stash_is_no_entry() {
        let env = WriteEnv::new();
        let r = repo();
        let id = open(&env.api, &r).await;
        let out = send(&env.api, json!({"method": "stashPush", "params": {"repo": id, "worktree": wt(&r), "message": ""}})).await.unwrap();
        assert_eq!(out["outcome"], json!({"status": "nothingToStash"}));
        assert!(journal(&env.api, id, &r).await["undo"].is_null());
        dirty(&r);
        send(&env.api, json!({"method": "stashPush", "params": {"repo": id, "worktree": wt(&r), "message": "  "}})).await.unwrap();
        assert_eq!(r.git(&["stash", "list", "--format=%gs"]), "On main: WIP on main");
        r.git(&["stash", "pop", "-q", "--index"]);
        r.git(&["switch", "-q", "--detach", "HEAD"]);
        send(&env.api, json!({"method": "stashPush", "params": {"repo": id, "worktree": wt(&r), "message": ""}})).await.unwrap();
        assert!(r.git(&["stash", "list", "-1", "--format=%gs"]).ends_with(": WIP on (no branch)"));
    }

    // --- some paths only ---

    /// `dirty`, and: e staged (another file's staged change), d/x unstaged, d/y untracked.
    fn dirtier(r: &TestRepo) {
        r.write("d/x.txt", "x\n");
        r.write("e.txt", "e\n");
        r.git(&["add", "."]);
        r.git(&["commit", "-q", "-m", "two"]);
        dirty(r);
        r.write("e.txt", "e staged\n");
        r.git(&["add", "e.txt"]);
        r.write("d/x.txt", "x unstaged\n");
        r.write("d/y.txt", "y untracked\n");
    }

    async fn stash_some(api: &crate::api::Api, id: u32, r: &TestRepo, paths: &[&str]) -> serde_json::Value {
        send(api, json!({"method": "stashPush", "params": {"repo": id, "worktree": wt(r), "message": "", "paths": paths}})).await.unwrap()
    }

    /// The files the newest stash holds: its worktree side's and its untracked files'.
    fn stashed(r: &TestRepo) -> Vec<String> {
        let mut out: Vec<String> = r.git(&["diff", "--name-only", "--no-renames", "stash@{0}^1", "stash@{0}"]).lines().map(String::from).collect();
        if let Ok(u) = r.try_git(&["ls-tree", "-r", "--name-only", "stash@{0}^3"]) {
            out.extend(u.lines().map(String::from));
        }
        out.sort();
        out
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stash_of_some_files_leaves_the_others_and_their_staged_state() {
        let env = WriteEnv::new();
        let r = repo();
        dirtier(&r);
        let id = open(&env.api, &r).await;
        let status = || r.git(&["status", "--porcelain", "-uall"]);
        let before = RepoState::capture(&r);
        // A staged file (its unstaged edit too), an unstaged one, an untracked one, a folder.
        for (paths, gone, left, name) in [
            (vec!["a.txt"], vec!["a.txt"], " M b.txt\n M d/x.txt\nM  e.txt\n?? c.txt\n?? d/y.txt", "WIP on main: a.txt"),
            (vec!["b.txt"], vec!["b.txt"], "M  a.txt\n M d/x.txt\nM  e.txt\n?? c.txt\n?? d/y.txt", "WIP on main: b.txt"),
            (vec!["c.txt"], vec!["c.txt"], "M  a.txt\n M b.txt\n M d/x.txt\nM  e.txt\n?? d/y.txt", "WIP on main: c.txt"),
            (vec!["d"], vec!["d/x.txt", "d/y.txt"], "M  a.txt\n M b.txt\nM  e.txt\n?? c.txt", "WIP on main: d"),
            (vec!["a.txt", "c.txt"], vec!["a.txt", "c.txt"], " M b.txt\n M d/x.txt\nM  e.txt\n?? d/y.txt", "WIP on main: a.txt and 1 more"),
        ] {
            let out = stash_some(&env.api, id, &r, &paths).await;
            assert_eq!(out["outcome"]["status"], "stashed", "{paths:?}");
            assert_eq!(out["outcome"]["oid"].as_str(), Some(top(&r).as_str()));
            assert_eq!(status(), left, "{paths:?}: the others untouched, staged ones still staged");
            assert_eq!(stashed(&r), gone, "{paths:?}: the stash holds those only");
            assert_eq!(r.git(&["stash", "list", "--format=%gs"]), format!("On main: {name}"));
            assert_eq!(journal(&env.api, id, &r).await["undo"]["label"], format!("stash \"{name}\""));
            undo_redo(&env.api, id, &r, &before, |after| {
                assert_eq!(after.stashes.lines().count(), 1, "re-pushed");
                assert_eq!(stashed(&r), gone, "{paths:?}: redo stashes those only");
            })
            .await;
            let s = journal(&env.api, id, &r).await;
            send(&env.api, json!({"method": "undo", "params": {"repo": id, "worktree": wt(&r), "entry": s["undo"]["entry"]}})).await.unwrap();
            assert_eq!(RepoState::capture(&r), before, "{paths:?}: undo applies it back, staged as it was");
        }
    }

    /// A staged rename's other side goes with it, whichever side is asked: never half a rename.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stash_of_a_renamed_file_takes_both_sides() {
        let env = WriteEnv::new();
        let r = repo();
        r.git(&["mv", "a.txt", "moved.txt"]);
        r.write("b.txt", "b unstaged\n");
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        assert_eq!(r.git(&["status", "--porcelain"]), " M b.txt\nR  a.txt -> moved.txt");
        stash_some(&env.api, id, &r, &["moved.txt"]).await;
        assert_eq!(r.git(&["status", "--porcelain"]), " M b.txt", "a.txt is back, moved.txt gone");
        assert_eq!(stashed(&r), ["a.txt", "moved.txt"]);
        assert_eq!(r.git(&["stash", "list", "--format=%gs"]), "On main: WIP on main: moved.txt");
        undo_redo(&env.api, id, &r, &before, |_| {}).await;
        assert_eq!(r.git(&["status", "--porcelain"]), " M b.txt");
        r.git(&["stash", "pop", "-q", "--index"]);
        assert_eq!(RepoState::capture(&r).index, before.index);
        stash_some(&env.api, id, &r, &["a.txt"]).await;
        assert_eq!(r.git(&["status", "--porcelain"]), " M b.txt", "from the old side too");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stash_of_unchanged_files_is_nothing_to_stash() {
        let env = WriteEnv::new();
        let r = repo();
        r.write("b.txt", "b unstaged\n");
        let id = open(&env.api, &r).await;
        let held = RepoState::capture(&r);
        for paths in [vec!["a.txt"], vec!["gone.txt"]] {
            let out = stash_some(&env.api, id, &r, &paths).await;
            assert_eq!(out["outcome"], json!({"status": "nothingToStash"}), "{paths:?}");
        }
        assert!(journal(&env.api, id, &r).await["undo"].is_null());
        let e = send(&env.api, json!({"method": "stashPush", "params": {"repo": id, "worktree": wt(&r), "message": "", "paths": []}})).await.unwrap_err();
        assert_eq!(serde_json::to_value(&e).unwrap()["kind"], "InvalidInput", "an empty list never stashes everything");
        assert_eq!(RepoState::capture(&r), held);
    }

    /// A staged deletion whose file is gone (`git rm`) is stashed staged, and comes back so.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stash_of_a_staged_deletion_keeps_it_staged() {
        let env = WriteEnv::new();
        let r = repo();
        r.git(&["rm", "-q", "a.txt"]);
        r.write("b.txt", "b unstaged\n");
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        assert_eq!(stash_some(&env.api, id, &r, &["a.txt"]).await["outcome"]["status"], "stashed");
        assert_eq!(r.git(&["status", "--porcelain"]), " M b.txt");
        assert_eq!(r.git(&["diff", "--name-status", "stash@{0}^1", "stash@{0}^2"]), "D\ta.txt", "staged in the stash");
        undo_redo(&env.api, id, &r, &before, |_| {}).await;
        assert_eq!(r.git(&["status", "--porcelain"]), " M b.txt");
    }

    /// Pop and Apply never change the staged state of the files outside the stash: git's own
    /// `apply --index` resets the whole index and then refuses over a staged change.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pop_keeps_the_other_files_staged() {
        let env = WriteEnv::new();
        let pop = async |r: &TestRepo, id: u32| send(&env.api, json!({"method": "stashApply", "params": {"repo": id, "worktree": wt(r), "oid": top(r), "pop": true}})).await;
        // Some paths, the other file staged all along.
        let r = repo();
        dirty(&r);
        r.git(&["add", "b.txt"]);
        r.write("a.txt", "a staged and more\n");
        let id = open(&env.api, &r).await;
        let held = RepoState::capture(&r);
        stash_some(&env.api, id, &r, &["a.txt"]).await;
        assert_eq!(r.git(&["status", "--porcelain"]), "M  b.txt\n?? c.txt");
        pop(&r, id).await.unwrap();
        assert_eq!(RepoState::capture(&r), held, "a's staged part staged again, b still staged");
        // A whole stash with a staged part, another file staged since.
        let r = repo();
        dirty(&r);
        r.git(&["stash", "push", "-q", "--include-untracked"]);
        r.write("e.txt", "e staged since\n");
        r.git(&["add", "e.txt"]);
        let id = open(&env.api, &r).await;
        pop(&r, id).await.unwrap();
        assert_eq!(r.git(&["status", "--porcelain"]), "M  a.txt\n M b.txt\nA  e.txt\n?? c.txt");
        assert!(r.git(&["stash", "list"]).is_empty());
        // A path of the stash that's staged now: git's refusal, the question, nothing changed.
        let r = repo();
        dirty(&r);
        r.git(&["stash", "push", "-q", "--include-untracked"]);
        r.write("a.txt", "a staged since\n");
        r.git(&["add", "a.txt"]);
        let id = open(&env.api, &r).await;
        let held = RepoState::capture(&r);
        let e = pop(&r, id).await.unwrap_err();
        assert_eq!(serde_json::to_value(&e).unwrap()["detail"], json!({"kind": "applyWithoutIndex"}));
        assert_eq!(RepoState::capture(&r), held);
    }

    /// Undo applies the stash itself: a `before` that's older than what was stashed (an editor's
    /// autosave in between) is never restored over it.
    #[tokio::test(flavor = "multi_thread")]
    async fn undoing_a_stash_of_some_paths_applies_what_it_holds() {
        let env = WriteEnv::new();
        let r = repo();
        r.write("a.txt", "older\n");
        let older = r.git(&["stash", "create"]);
        r.write("a.txt", "autosaved\n");
        r.write("b.txt", "b staged\n");
        r.git(&["add", "b.txt"]);
        let id = open(&env.api, &r).await;
        stash_some(&env.api, id, &r, &["a.txt"]).await;
        let root = std::fs::canonicalize(r.path()).unwrap();
        let stale = crate::journal::Snapshot { commit: older, paths: vec!["a.txt".into()], untracked: vec![], modes: Default::default() };
        env.api.journal(&root).unwrap().update(|j| j.undo.last_mut().unwrap().before = Some(stale)).unwrap();
        let s = journal(&env.api, id, &r).await;
        send(&env.api, json!({"method": "undo", "params": {"repo": id, "worktree": wt(&r), "entry": s["undo"]["entry"]}})).await.unwrap();
        assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "autosaved\n");
        assert_eq!(r.git(&["status", "--porcelain"]), " M a.txt\nM  b.txt");
    }

    /// Narrowed with no identity configured: the stash keeps the one git chose for it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stash_of_some_paths_narrows_without_an_identity() {
        use crate::git::GitCli;
        use std::sync::Arc;
        const IDENT: [&str; 5] = ["GIT_AUTHOR_NAME", "GIT_AUTHOR_EMAIL", "GIT_COMMITTER_NAME", "GIT_COMMITTER_EMAIL", "EMAIL"];
        let env: Vec<_> = crate::testing::isolated_git_env().into_iter().filter(|(k, _)| !IDENT.iter().any(|i| k == i)).collect();
        // Nobody named, but what a command is given itself.
        let cli = GitCli::new(Arc::new(crate::log::CommandLog::new(100))).with_env(env).with_command_hook(Arc::new(|cmd: &mut std::process::Command| {
            let given: Vec<String> = cmd.get_envs().filter(|(_, v)| v.is_some()).map(|(k, _)| k.to_string_lossy().into_owned()).collect();
            for var in IDENT.into_iter().filter(|v| !given.iter().any(|g| g == v)) {
                cmd.env_remove(var);
            }
        }));
        let data = tempfile::tempdir().unwrap();
        let api = crate::api::Api::new(cli, None).with_data_dir(data.path().to_path_buf());
        let r = TestRepo::new();
        r.write("a.txt", "a\n");
        r.write("b.txt", "b\n");
        r.git(&["add", "."]);
        r.git(&["commit", "-q", "-m", "one"]);
        r.write("a.txt", "a edited\n");
        r.write("b.txt", "b staged\n");
        r.git(&["add", "b.txt"]);
        let id = open(&api, &r).await;
        stash_some(&api, id, &r, &["a.txt"]).await;
        assert_eq!(stashed(&r), ["a.txt"], "narrowed");
        assert_eq!(r.git(&["show", "-s", "--format=%an <%ae>", "stash@{0}"]), "git stash <git@stash>");
    }

    /// A name that isn't UTF-8 is stashed as itself: the narrowed stash never holds a made-up one.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stash_of_a_folder_keeps_names_that_arent_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let name = std::path::Path::new(std::ffi::OsStr::from_bytes(b"d/caf\xe9.txt"));
        let env = WriteEnv::new();
        let r = repo();
        r.write("d/x.txt", "x\n");
        std::fs::write(r.path().join(name), "base\n").unwrap();
        r.git(&["add", "."]);
        r.git(&["commit", "-q", "-m", "two"]);
        std::fs::write(r.path().join(name), "edited\n").unwrap();
        r.write("b.txt", "b staged\n");
        r.git(&["add", "b.txt"]);
        let id = open(&env.api, &r).await;
        stash_some(&env.api, id, &r, &["d"]).await;
        assert_eq!(std::fs::read(r.path().join(name)).unwrap(), b"base\n");
        assert_eq!(r.git(&["status", "--porcelain"]), "M  b.txt");
        r.git(&["stash", "pop", "-q"]);
        assert_eq!(std::fs::read(r.path().join(name)).unwrap(), b"edited\n");
        assert_eq!(std::fs::read_dir(r.path().join("d")).unwrap().count(), 2, "no other name made up");
    }

    /// git stores the stash, then fails cleaning the file up (a clean filter that fails once a
    /// stash is stored: the cleanup's `add`): the stash is recorded (Undo applies it) and the
    /// error says where the changes are.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stash_git_stored_before_failing_is_recorded() {
        let env = WriteEnv::new();
        let r = repo();
        let stored = r.path().join(".git/logs/refs/stash");
        r.git(&["config", "filter.flaky.clean", &format!("[ -e {} ] && exit 1; cat", stored.display())]);
        r.git(&["config", "filter.flaky.required", "true"]);
        std::fs::write(r.path().join(".git/info/attributes"), "a.txt filter=flaky\n").unwrap();
        r.write("a.txt", "a edited\n");
        let id = open(&env.api, &r).await;
        let e = send(&env.api, json!({"method": "stashPush", "params": {"repo": id, "worktree": wt(&r), "message": "", "paths": ["a.txt"]}})).await.unwrap_err();
        assert!(e.message.starts_with("git stored the stash \"WIP on main: a.txt\", then failed"), "{}", e.message);
        assert_eq!(r.git(&["stash", "list", "--format=%gs"]), "On main: WIP on main: a.txt");
        assert_eq!(journal(&env.api, id, &r).await["undo"]["label"], "stash \"WIP on main: a.txt\"");
    }

    /// A path with glob characters is taken literally.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stash_of_some_paths_takes_them_literally() {
        let env = WriteEnv::new();
        let r = repo();
        r.write("[ab].txt", "glob\n");
        r.write("b.txt", "b unstaged\n");
        let id = open(&env.api, &r).await;
        stash_some(&env.api, id, &r, &["[ab].txt"]).await;
        assert_eq!(stashed(&r), ["[ab].txt"]);
        assert_eq!(r.git(&["status", "--porcelain"]), " M b.txt");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn apply_keeps_the_stash_and_isnt_journaled() {
        let env = WriteEnv::new();
        let r = repo();
        dirty(&r);
        r.git(&["stash", "push", "-q", "--include-untracked", "-m", "keep me"]);
        let id = open(&env.api, &r).await;
        let oid = top(&r);
        let out = send(&env.api, json!({"method": "stashApply", "params": {"repo": id, "worktree": wt(&r), "oid": oid, "pop": false}})).await.unwrap();
        assert_eq!(out["outcome"], json!({"status": "applied"}));
        assert_eq!(r.git(&["diff", "--cached", "--name-only"]), "a.txt", "the index came back");
        assert_eq!(top(&r), oid, "kept");
        assert!(journal(&env.api, id, &r).await["undo"].is_null());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refused_index_asks_and_without_index_applies() {
        let env = WriteEnv::new();
        let r = repo();
        r.write("a.txt", "a staged\n");
        r.git(&["add", "a.txt"]);
        r.git(&["stash", "push", "-q", "-m", "s"]);
        // The index now conflicts with the stash's index part.
        r.write("a.txt", "a other\n");
        r.git(&["add", "a.txt"]);
        r.git(&["commit", "-q", "-m", "moves a"]);
        let id = open(&env.api, &r).await;
        let oid = top(&r);
        let apply = |without: bool| json!({"method": "stashApply", "params": {"repo": id, "worktree": wt(&r), "oid": oid, "pop": false, "withoutIndex": without}});
        let e = send(&env.api, apply(false)).await.unwrap_err();
        assert_eq!(serde_json::to_value(&e).unwrap()["detail"], json!({"kind": "applyWithoutIndex"}));
        let out = send(&env.api, apply(true)).await.unwrap();
        assert_eq!(out["outcome"]["status"], "conflicts", "the content conflicts land in Conflicted");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pop_round_trips_and_a_conflicting_pop_keeps_the_stash() {
        pop_round_trip().await;
    }

    /// The stack guard: the pop round trip (a write, its undo and redo, each through `dispatch`)
    /// fits a 2 MB thread, a spawned thread's and a tokio worker's default, in a debug build and
    /// whatever RUST_MIN_STACK says. An overflow aborts the run: the write path's futures have
    /// grown inline again (see `run_write`, `dispatch`).
    #[test]
    fn the_pop_round_trip_fits_a_2_mb_stack() {
        const STACK: usize = 2 * 1024 * 1024;
        let run = || tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(STACK).build().unwrap().block_on(pop_round_trip());
        std::thread::Builder::new().stack_size(STACK).spawn(run).unwrap().join().unwrap();
    }

    async fn pop_round_trip() {
        let env = WriteEnv::new();
        let r = repo();
        dirty(&r);
        r.git(&["stash", "push", "-q", "--include-untracked", "-m", "to pop"]);
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        let oid = top(&r);
        send(&env.api, json!({"method": "stashApply", "params": {"repo": id, "worktree": wt(&r), "oid": oid, "pop": true}})).await.unwrap();
        assert!(r.git(&["stash", "list"]).is_empty());
        assert_eq!(journal(&env.api, id, &r).await["undo"]["label"], "pop stash \"to pop\"");
        let after = RepoState::capture(&r);
        undo_redo(&env.api, id, &r, &before, |s| assert_eq!(s, &after)).await;

        let c = repo();
        c.write("a.txt", "a stashed\n");
        c.git(&["stash", "push", "-q", "-m", "conflicts"]);
        c.write("a.txt", "a committed\n");
        c.git(&["commit", "-q", "-am", "moves a"]);
        let cid = open(&env.api, &c).await;
        let coid = top(&c);
        let out = send(&env.api, json!({"method": "stashApply", "params": {"repo": cid, "worktree": wt(&c), "oid": coid, "pop": true, "withoutIndex": true}})).await.unwrap();
        assert_eq!(out["outcome"], json!({"status": "conflicts", "files": 1}));
        assert_eq!(top(&c), coid, "a conflicting pop keeps the stash, as git's own does");
    }

    /// A pop's undo overwrites P: an edit made since the pop asks first, as a Restore's does,
    /// and an edit outside P doesn't.
    #[tokio::test(flavor = "multi_thread")]
    async fn undoing_a_pop_asks_before_overwriting_an_edit_made_since() {
        let env = WriteEnv::new();
        let r = repo();
        dirty(&r);
        r.git(&["stash", "push", "-q", "--include-untracked", "-m", "to pop"]);
        let id = open(&env.api, &r).await;
        let oid = top(&r);
        send(&env.api, json!({"method": "stashApply", "params": {"repo": id, "worktree": wt(&r), "oid": oid, "pop": true}})).await.unwrap();
        r.write("b.txt", "b edited since\n");
        let s = journal(&env.api, id, &r).await;
        let held = RepoState::capture(&r);
        let e = send(&env.api, json!({"method": "undo", "params": {"repo": id, "worktree": wt(&r), "entry": s["undo"]["entry"]}})).await.unwrap_err();
        assert_eq!(serde_json::to_value(&e).unwrap()["detail"]["kind"], "autostashConflict");
        assert_eq!(serde_json::to_value(&e).unwrap()["detail"]["paths"], json!(["b.txt"]));
        assert_eq!(RepoState::capture(&r), held, "asking changes nothing");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drop_round_trips_with_the_list_line_unchanged() {
        let env = WriteEnv::new();
        let r = repo();
        for m in ["first", "second"] {
            r.write("a.txt", &format!("{m}\n"));
            r.git(&["stash", "push", "-q", "-m", m]);
        }
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        let second = top(&r);
        send(&env.api, json!({"method": "stashDrop", "params": {"repo": id, "worktree": wt(&r), "oid": second}})).await.unwrap();
        assert_eq!(r.git(&["stash", "list", "--format=%gs"]), "On main: first");
        assert_eq!(journal(&env.api, id, &r).await["undo"]["label"], "drop stash \"second\"");
        undo_redo(&env.api, id, &r, &before, |s| assert_eq!(s.stashes.lines().count(), 1)).await;
    }

    /// A kept autostash's banner goes when the user pops or drops that stash from
    /// the stash menus (the record too), and doesn't show while its stash isn't listed.
    #[tokio::test(flavor = "multi_thread")]
    async fn popping_or_dropping_a_kept_autostash_clears_its_banner() {
        use crate::journal::{KeptReason, KeptStash};
        for method in ["pop", "drop", "outside"] {
            let env = WriteEnv::new();
            let r = repo();
            r.write("a.txt", "kept\n");
            r.git(&["stash", "push", "-q", "-m", "autostash before checkout x"]);
            let oid = top(&r);
            let root = crate::platform::fs::canonicalize(r.path()).unwrap();
            let store = env.api.journal(&root).unwrap();
            let record = KeptStash { id: 0, oid: Some(oid.clone()), stash_before: None, message: "autostash before checkout x".into(), label: "checkout x".into(), target: Some("x".into()), reason: KeptReason::Refused, created_ms: 0, owner: None };
            store.update(|j| j.keep(record.clone())).unwrap();
            let id = open(&env.api, &r).await;
            assert_eq!(journal(&env.api, id, &r).await["banners"][0]["stash"].as_str(), Some(oid.as_str()), "{method}");
            match method {
                "pop" => send(&env.api, json!({"method": "stashApply", "params": {"repo": id, "worktree": wt(&r), "oid": oid, "pop": true}})).await.map(drop).unwrap(),
                "drop" => send(&env.api, json!({"method": "stashDrop", "params": {"repo": id, "worktree": wt(&r), "oid": oid}})).await.map(drop).unwrap(),
                _ => drop(r.git(&["stash", "drop", "-q"])),
            }
            assert_eq!(journal(&env.api, id, &r).await["banners"], json!([]), "{method}");
            let left = store.load().unwrap().kept.len();
            assert_eq!(left, usize::from(method == "outside"), "{method}: the menus clear the record; an outside drop only hides it");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_outside_stash_between_showing_and_dropping_drops_the_right_one() {
        let env = WriteEnv::new();
        let r = repo();
        r.write("a.txt", "shown\n");
        r.git(&["stash", "push", "-q", "-m", "shown"]);
        let id = open(&env.api, &r).await;
        let shown = top(&r);
        r.write("a.txt", "outside\n");
        r.git(&["stash", "push", "-q", "-m", "outside"]);
        send(&env.api, json!({"method": "stashDrop", "params": {"repo": id, "worktree": wt(&r), "oid": shown}})).await.unwrap();
        assert_eq!(r.git(&["stash", "list", "--format=%gs"]), "On main: outside");
        let before = RepoState::capture(&r);
        let e = send(&env.api, json!({"method": "stashApply", "params": {"repo": id, "worktree": wt(&r), "oid": shown, "pop": true}})).await.unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (GbErrorKind::NotFound, "That stash is gone"));
        assert_eq!(RepoState::capture(&r), before);
    }


    #[test]
    fn the_dropped_oid_is_read_from_gits_line() {
        assert_eq!(super::dropped_oid("Dropped stash@{0} (ba3ceed98797445279ae20220118f8b396323d70)\n").as_deref(), Some("ba3ceed98797445279ae20220118f8b396323d70"));
        assert_eq!(super::dropped_oid(""), None);
    }

    /// The drop/store pair of a multi-line push. A store that fails leaves the stash
    /// out of the list: the push still succeeded, and the stash's banner shows at once.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_relist_whose_store_fails_keeps_the_push_and_raises_a_banner() {
        let env = WriteEnv::new();
        let r = repo();
        dirty(&r);
        // The 3rd refs/stash transaction (push, drop, store) is refused.
        let count = r.path().join(".git/stash-tx");
        r.hook("reference-transaction", &format!("#!/bin/sh\n[ \"$1\" = prepared ] || exit 0\ngrep -q ' refs/stash$' || exit 0\nn=$(cat {c} 2>/dev/null || echo 0); n=$((n+1)); echo $n > {c}\n[ $n -ne 3 ]\n", c = crate::platform::fs::to_git_path(&count)));
        let id = open(&env.api, &r).await;
        let out = send(&env.api, json!({"method": "stashPush", "params": {"repo": id, "worktree": wt(&r), "message": "Fix x\n\nbody"}})).await.unwrap();
        assert_eq!(out["outcome"]["status"], "stashed");
        let oid = out["outcome"]["oid"].as_str().unwrap().to_string();
        assert_eq!(r.git(&["stash", "list"]), "", "the store was refused");
        let s = journal(&env.api, id, &r).await;
        assert_eq!(s["undo"]["label"], "stash \"Fix x\"");
        let b = s["banners"].as_array().unwrap();
        assert_eq!(b.len(), 1, "{b:?}");
        assert_eq!((b[0]["kind"].as_str(), b[0]["stash"].as_str()), (Some("recovery"), Some(oid.as_str())));
        // Its Apply brings the changes back, split included.
        let entry = b[0]["entry"].as_u64().unwrap();
        send(&env.api, json!({"method": "applyKeptStash", "params": {"repo": id, "worktree": wt(&r), "entry": entry}})).await.unwrap();
        assert_eq!(r.git(&["diff", "--cached", "--name-only"]), "a.txt");
        assert!(r.path().join("c.txt").exists());
    }

    /// A stash op that stopped after recording its stash. A recorded stash that's
    /// no longer listed but still exists gets a Recovery banner at the next start; one still
    /// listed gets none.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_crashed_stash_op_whose_stash_is_unlisted_raises_a_banner_at_the_next_start() {
        use crate::journal::{HeadState, NewEntry, Owner, StashMove, UndoKind};
        let (first, data) = WriteEnv::new().into_parts();
        let r = repo();
        dirty(&r);
        r.git(&["stash", "push", "-q", "--include-untracked", "-m", "lost"]);
        let lost = top(&r);
        r.git(&["stash", "drop", "-q"]);
        r.write("b.txt", "kept\n");
        r.git(&["stash", "push", "-q", "-m", "kept"]);
        let kept = top(&r);
        let root = crate::platform::fs::canonicalize(r.path()).unwrap();
        let dead = Owner { pid: 1, start: 0, instance: u64::MAX };
        let now = first.now();
        first
            .journal(&root)
            .unwrap()
            .update(|j| {
                for (oid, label, created) in [(lost.clone(), "stash \"lost\"", true), (kept.clone(), "drop stash \"kept\"", false)] {
                    let id = j.begin(NewEntry { label: label.into(), kind: crate::events::OpKind::Stash, head_before: HeadState::default(), undo: UndoKind::Stash }, now);
                    let e = j.entry_mut(id).unwrap();
                    e.owner = Some(dead.clone());
                    e.stashes.push(StashMove { oid, message: format!("On main: {}", &label[label.find('"').unwrap() + 1..label.len() - 1]), created, without_index: false, paths: None });
                }
            })
            .unwrap();
        drop(first);
        let api = crate::api::Api::new(crate::git::GitCli::new(std::sync::Arc::new(crate::log::CommandLog::new(10))).with_env(crate::testing::isolated_git_env()), None).with_data_dir(data.path().to_path_buf());
        let id = open(&api, &r).await;
        let s = journal(&api, id, &r).await;
        let b = s["banners"].as_array().unwrap();
        assert_eq!(b.len(), 1, "{b:?}");
        assert_eq!((b[0]["kind"].as_str(), b[0]["stash"].as_str(), b[0]["stashMessage"].as_str()), (Some("recovery"), Some(lost.as_str()), Some("lost")));
        assert!(s["undo"].is_null());
        r.git(&["stash", "drop", "-q"]);
        send(&api, json!({"method": "applyKeptStash", "params": {"repo": id, "worktree": wt(&r), "entry": b[0]["entry"]}})).await.unwrap();
        assert_eq!(r.git(&["diff", "--cached", "--name-only"]), "a.txt", "the lost stash came back");
    }

    /// A conflicting pop has no `after`. Its undo refuses while a path is unmerged,
    /// then asks before overwriting the user's resolution.
    #[tokio::test(flavor = "multi_thread")]
    async fn undoing_a_conflicting_pop_keeps_a_resolution_made_since() {
        let env = WriteEnv::new();
        let c = repo();
        c.write("a.txt", "a stashed\n");
        c.git(&["stash", "push", "-q", "-m", "conflicts"]);
        c.write("a.txt", "a committed\n");
        c.git(&["commit", "-q", "-am", "moves a"]);
        let id = open(&env.api, &c).await;
        let oid = top(&c);
        let out = send(&env.api, json!({"method": "stashApply", "params": {"repo": id, "worktree": wt(&c), "oid": oid, "pop": true, "withoutIndex": true}})).await.unwrap();
        assert_eq!(out["outcome"]["status"], "conflicts");
        let undo = || json!({"method": "undo", "params": {"repo": id, "worktree": wt(&c), "entry": 0}});
        let entry = journal(&env.api, id, &c).await["undo"]["entry"].clone();
        let mut req = undo();
        req["params"]["entry"] = entry;
        let e = send(&env.api, req.clone()).await.unwrap_err();
        assert_eq!(e.message, "Resolve or abort the conflicts first");
        c.write("a.txt", "a resolved\n");
        c.git(&["add", "a.txt"]);
        let held = RepoState::capture(&c);
        let e = send(&env.api, req).await.unwrap_err();
        assert_eq!(serde_json::to_value(&e).unwrap()["detail"]["kind"], "autostashConflict");
        assert_eq!(RepoState::capture(&c), held, "the resolution is kept");
    }

    /// A stash with line 1 staged, under a commit that changed line 4: git refuses its index
    /// part (the hunk's context moved), but its changes merge cleanly.
    fn refused_index_repo() -> TestRepo {
        let r = TestRepo::new();
        identity(&r);
        r.write("a.txt", "1\n2\n3\n4\n5\n6\n7\n8\n");
        r.git(&["add", "."]);
        r.git(&["commit", "-q", "-m", "one"]);
        r.write("a.txt", "one\n2\n3\n4\n5\n6\n7\n8\n");
        r.git(&["add", "a.txt"]);
        r
    }

    fn move_line_4(r: &TestRepo) {
        r.write("a.txt", "1\n2\n3\nfour\n5\n6\n7\n8\n");
        r.git(&["commit", "-q", "-am", "four"]);
    }

    /// The undo of a push asks "Apply without restoring what was staged?" as Apply
    /// does, and applies without the index once confirmed.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pushs_undo_can_apply_without_the_index() {
        let env = WriteEnv::new();
        let r = refused_index_repo();
        let id = open(&env.api, &r).await;
        send(&env.api, json!({"method": "stashPush", "params": {"repo": id, "worktree": wt(&r), "message": "s"}})).await.unwrap();
        move_line_4(&r);
        let entry = journal(&env.api, id, &r).await["undo"]["entry"].clone();
        let undo = |without: bool| json!({"method": "undo", "params": {"repo": id, "worktree": wt(&r), "entry": entry, "withoutIndex": without}});
        let e = send(&env.api, undo(false)).await.unwrap_err();
        assert_eq!(serde_json::to_value(&e).unwrap()["detail"], json!({"kind": "applyWithoutIndex"}));
        send(&env.api, undo(true)).await.unwrap();
        assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "one\n2\n3\nfour\n5\n6\n7\n8\n");
        assert_eq!(r.git(&["stash", "list"]), "");
    }

    /// A pop made without the index redoes without it, without asking again.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pop_made_without_the_index_redoes_without_it() {
        let env = WriteEnv::new();
        let r = refused_index_repo();
        r.git(&["stash", "push", "-q", "-m", "s"]);
        move_line_4(&r);
        let id = open(&env.api, &r).await;
        let oid = top(&r);
        let pop = |without: bool| json!({"method": "stashApply", "params": {"repo": id, "worktree": wt(&r), "oid": oid, "pop": true, "withoutIndex": without}});
        let e = send(&env.api, pop(false)).await.unwrap_err();
        assert_eq!(serde_json::to_value(&e).unwrap()["detail"], json!({"kind": "applyWithoutIndex"}));
        assert!(journal(&env.api, id, &r).await["undo"].is_null(), "a refused index leaves no entry");
        send(&env.api, pop(true)).await.unwrap();
        let after = RepoState::capture(&r);
        let s = journal(&env.api, id, &r).await;
        send(&env.api, json!({"method": "undo", "params": {"repo": id, "worktree": wt(&r), "entry": s["undo"]["entry"]}})).await.unwrap();
        let s = journal(&env.api, id, &r).await;
        send(&env.api, json!({"method": "redo", "params": {"repo": id, "worktree": wt(&r), "entry": s["redo"]["entry"]}})).await.unwrap();
        assert_eq!(RepoState::capture(&r), after);
    }


    /// A crashed pop keeps its Restore banner, and its stash banner is raised
    /// once: dismissed, it doesn't come back at the next start.
    #[test]
    fn a_resolved_stash_banner_isnt_raised_again() {
        use crate::journal::{HeadState, Journal, NewEntry, Snapshot, StashMove, UndoKind};
        let mut j = Journal::empty("/r");
        let id = j.begin(NewEntry { label: "pop stash \"x\"".into(), kind: crate::events::OpKind::Stash, head_before: HeadState::default(), undo: UndoKind::Stash }, 1);
        let e = j.entry_mut(id).unwrap();
        e.before = Some(Snapshot { commit: "c0ffee".into(), paths: vec!["a.txt".into()], untracked: Vec::new(), ..Default::default() });
        e.stashes.push(StashMove { oid: "1a2b".into(), message: "On main: x".into(), created: false, without_index: false, paths: None });
        j.recover_unless(|_| false);
        j.resolve_stash_moves(&[], |_| true);
        assert_eq!(j.kept.len(), 1);
        assert_eq!(j.recovery.len(), 1, "the Restore banner stays");
        j.kept.clear();
        j.recover_unless(|_| false);
        j.resolve_stash_moves(&[], |_| true);
        assert!(j.kept.is_empty(), "dismissed: not raised again");
    }

    /// The redo of a multi-line push has no pending entry; its guard covers the
    /// drop/store pair. A refused store keeps the redo, records the new oid and shows its banner.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_redone_push_whose_relist_store_fails_raises_a_banner() {
        let env = WriteEnv::new();
        let r = repo();
        dirty(&r);
        // refs/stash transactions: push 1, drop 2, store 3; undo's drop 4; redo's push 5, drop 6,
        // store 7 (refused).
        let count = r.path().join(".git/stash-tx");
        r.hook("reference-transaction", &format!("#!/bin/sh\n[ \"$1\" = prepared ] || exit 0\ngrep -q ' refs/stash$' || exit 0\nn=$(cat {c} 2>/dev/null || echo 0); n=$((n+1)); echo $n > {c}\n[ $n -ne 7 ]\n", c = crate::platform::fs::to_git_path(&count)));
        let id = open(&env.api, &r).await;
        let out = send(&env.api, json!({"method": "stashPush", "params": {"repo": id, "worktree": wt(&r), "message": "Fix x\n\nbody"}})).await.unwrap();
        assert_eq!(out["outcome"]["status"], "stashed");
        assert_eq!(r.git(&["stash", "list", "--format=%gs"]), "On main: Fix x");
        let s = journal(&env.api, id, &r).await;
        send(&env.api, json!({"method": "undo", "params": {"repo": id, "worktree": wt(&r), "entry": s["undo"]["entry"]}})).await.unwrap();
        let s = journal(&env.api, id, &r).await;
        send(&env.api, json!({"method": "redo", "params": {"repo": id, "worktree": wt(&r), "entry": s["redo"]["entry"]}})).await.unwrap();
        assert_eq!(std::fs::read_to_string(&count).unwrap().trim(), "7", "the store was the one refused");
        assert_eq!(r.git(&["stash", "list"]), "");
        let s = journal(&env.api, id, &r).await;
        assert_eq!(s["undo"]["label"], "stash \"Fix x\"");
        let b = s["banners"].as_array().unwrap();
        assert_eq!(b.len(), 1, "{b:?}");
        let again = b[0]["stash"].as_str().unwrap().to_string();
        assert_eq!(b[0]["kind"], "recovery");
        let root = crate::platform::fs::canonicalize(r.path()).unwrap();
        let entry = env.api.journal(&root).unwrap().load().unwrap().undo_top().cloned().unwrap();
        assert_eq!(entry.stashes[0].oid, again, "the entry records the redo's oid");
        send(&env.api, json!({"method": "applyKeptStash", "params": {"repo": id, "worktree": wt(&r), "entry": b[0]["entry"]}})).await.unwrap();
        assert_eq!(r.git(&["diff", "--cached", "--name-only"]), "a.txt");
        assert!(r.path().join("c.txt").exists());
    }

    // --- Repo safety ---

    /// A repository to clone from, with one commit.
    fn upstream() -> TestRepo {
        let s = TestRepo::new();
        identity(&s);
        s.write("s.txt", "s\n");
        s.git(&["add", "."]);
        s.git(&["commit", "-q", "-m", "s"]);
        s
    }

    /// `git clone` of `from` at `at`, with a local-only commit (what the user would lose).
    fn embed(r: &TestRepo, from: &TestRepo, at: &str) {
        r.git(&["clone", "-q", &from.path().display().to_string(), at]);
        let sm = r.path().join(at);
        for args in [&["config", "user.name", "Ada Lovelace"][..], &["config", "user.email", "ada@example.com"]] {
            r.git_in(&sm, args);
        }
        std::fs::write(sm.join("local.txt"), "local\n").unwrap();
        r.git_in(&sm, &["add", "local.txt"]);
        r.git_in(&sm, &["commit", "-q", "-m", "local-only"]);
    }

    fn stash_push(id: u32, r: &TestRepo, message: &str) -> serde_json::Value {
        json!({"method": "stashPush", "params": {"repo": id, "worktree": wt(r), "message": message}})
    }

    /// HEAD has the file `b.txt`; the user replaced it by a folder holding a clone.
    /// `stash push -u` skips the clone, then its `reset --hard` writes the file over the folder,
    /// clone and all: refused, nothing changed.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stash_never_deletes_a_clone_in_place_of_a_tracked_file() {
        let sub = upstream();
        let r = repo();
        std::fs::remove_file(r.path().join("b.txt")).unwrap();
        embed(&r, &sub, "b.txt/lib");
        let env = WriteEnv::new();
        let id = open(&env.api, &r).await;
        let shown = RepoState::capture(&r);
        let e = send(&env.api, stash_push(id, &r, "s")).await.unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (GbErrorKind::InvalidInput, "b.txt/lib is a repository in the way of the stash: move it first"));
        assert_eq!(RepoState::capture(&r), shown);
        assert!(r.path().join("b.txt/lib/.git").is_dir());
        assert_eq!(r.git(&["stash", "list"]), "");
    }

    /// HEAD has a populated gitlink `sm`; the stash replaced it by the file `sm`.
    /// Popping it would write the file over the clone (a local-only commit inside): refused.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pop_never_deletes_a_populated_submodule_where_the_stash_has_a_file() {
        let sub = upstream();
        let r = repo();
        embed(&r, &sub, "sm");
        r.git(&["add", "sm"]);
        r.git(&["commit", "-q", "-m", "gitlink"]);
        r.git(&["rm", "-q", "--cached", "sm"]);
        let aside = r.root().join("sm-aside");
        std::fs::rename(r.path().join("sm"), &aside).unwrap();
        r.write("sm", "file\n");
        r.git(&["add", "sm"]);
        r.git(&["stash", "push", "-q", "-m", "s"]);
        let _ = std::fs::remove_dir(r.path().join("sm"));
        std::fs::rename(&aside, r.path().join("sm")).unwrap();
        assert_eq!(r.git(&["status", "--porcelain"]), "", "clean");
        let env = WriteEnv::new();
        let id = open(&env.api, &r).await;
        let shown = RepoState::capture(&r);
        for pop in [true, false] {
            let e = send(&env.api, json!({"method": "stashApply", "params": {"repo": id, "worktree": wt(&r), "oid": top(&r), "pop": pop}})).await.unwrap_err();
            let what = if pop { "pop" } else { "apply" };
            assert_eq!((e.kind, e.message.as_str()), (GbErrorKind::InvalidInput, format!("sm is a repository in the way of the {what}: move it first").as_str()));
        }
        assert_eq!(RepoState::capture(&r), shown);
        assert!(r.path().join("sm/.git").is_dir());
    }

    /// The stash turned the tracked folder `vendor/` into the file `vendor`; since the
    /// push, an ignored clone sits at `vendor/lib`. Undoing the push (an apply) would delete it,
    /// ignored or not: refused, and the stash stays.
    #[tokio::test(flavor = "multi_thread")]
    async fn undoing_a_push_never_deletes_an_ignored_clone_under_a_replaced_folder() {
        let sub = upstream();
        let r = repo();
        r.write("vendor/x", "vx\n");
        r.write(".gitignore", "vendor/lib/\n");
        r.git(&["add", "."]);
        r.git(&["commit", "-q", "-m", "vendor"]);
        r.git(&["rm", "-q", "-r", "vendor"]);
        r.write("vendor", "file\n");
        r.git(&["add", "vendor"]);
        let env = WriteEnv::new();
        let id = open(&env.api, &r).await;
        send(&env.api, stash_push(id, &r, "s")).await.unwrap();
        assert_eq!(std::fs::read_to_string(r.path().join("vendor/x")).unwrap(), "vx\n");
        embed(&r, &sub, "vendor/lib");
        assert_eq!(r.git(&["status", "--porcelain"]), "", "the clone is ignored");
        let shown = RepoState::capture(&r);
        let s = journal(&env.api, id, &r).await;
        let e = send(&env.api, json!({"method": "undo", "params": {"repo": id, "worktree": wt(&r), "entry": s["undo"]["entry"]}})).await.unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (GbErrorKind::InvalidInput, "vendor/lib is a repository in the way of the undo: move it first"));
        assert_eq!(RepoState::capture(&r), shown);
        assert!(r.path().join("vendor/lib/.git").is_dir());
        assert_eq!(r.git(&["stash", "list"]).lines().count(), 1);
    }
}
