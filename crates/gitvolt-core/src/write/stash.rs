//! Stashes. Identity is the stash commit's oid, never a list position:
//! `stash@{n}` is found by oid in `git stash list` (a read) under the lock, just before each
//! command, and checked again (`rev-parse`) so a stack that shifted is never acted on blindly.
//! - Push: `git stash push --include-untracked -m <message> [-- <paths>]`; journaled (undo
//!   applies it with `--index` and drops it; redo pushes again and records the new oid). A push
//!   of some paths takes a staged rename's both sides, and keeps only their changes (`narrow`).
//!   Paths are git's bytes throughout (a name needn't be UTF-8).
//! - A WIP list's Stash takes that list's part (`StashPart`): Unstaged's is git's push of the
//!   files with unstaged changes, their staged part then checked out again (`keep_staged`);
//!   Staged's is built here (`push_staged`), git's `--staged` taking the whole index.
//! - An apply with `--index` while other files are staged runs on a temp index (`index_aside`):
//!   git's own resets the whole index first, and refuses then, losing what was staged. An index
//!   part that's staged already is a no-op, and a staged part only goes into edited files by a
//!   3-way merge (`Route`, `merged`).
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
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
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

/// Which part of some paths' changes a WIP list's Stash takes: the list it was started from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum StashPart {
    /// Their staged changes; the unstaged edits stay in the files, unstaged.
    Staged,
    /// Their unstaged changes and untracked files; the staged changes stay staged.
    Unstaged,
}

impl StashPart {
    fn word(self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Unstaged => "unstaged",
        }
    }
}

pub(crate) struct StashPush {
    /// The whole WIP draft, or `WIP on <branch>` (`(no branch)`) when it was empty; for some
    /// paths, `WIP on <branch>: <path>` (`<path> and N more`), and the part (`(staged)`).
    pub message: String,
    /// Only these files and folders (a WIP file's or folder's Stash); `None`: every change.
    pub paths: Option<Vec<String>>,
    /// Only that part of the paths' changes; `None`: all of them.
    pub part: Option<StashPart>,
}

impl StashPush {
    /// Fills the empty message from HEAD, read before the run, the paths and the part.
    pub(crate) fn new(message: String, branch: Option<&str>, paths: Option<Vec<String>>, part: Option<StashPart>) -> Self {
        let on = format!("WIP on {}", branch.unwrap_or("(no branch)"));
        let message = match (&paths, message.trim().is_empty()) {
            (_, false) => message,
            (Some(p), true) if !p.is_empty() => {
                let named = match p.len() - 1 {
                    0 => format!("{on}: {}", p[0]),
                    n => format!("{on}: {} and {n} more", p[0]),
                };
                match part {
                    Some(part) => format!("{named} ({})", part.word()),
                    None => named,
                }
            }
            _ => on,
        };
        Self { message, paths, part }
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
    renames_of(cli, root, paths.iter().map(|p| p.as_bytes().to_vec()).collect()).await
}

/// `with_renames` of git's bytes.
async fn renames_of(cli: &GitCli, root: &Path, asked: Vec<Bytes>) -> Result<Vec<Bytes>, GbError> {
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

/// A push: the new stash's `(oid, subject)`, or `None` when there was nothing to stash. The
/// caller records it before `relist`. `part` is only for some paths (the plan refuses it alone).
async fn push(cx: &mut WriteCx<'_>, message: &str, paths: Option<&[String]>, part: Option<StashPart>) -> Result<Option<(String, String)>, GbError> {
    let scope = match paths {
        Some([]) => return Err(no_paths()),
        Some(p) => Some(with_renames(&cx.api.cli, cx.root, p).await?),
        None => None,
    };
    let asked = (paths, part);
    match (scope, part) {
        (Some(scope), Some(StashPart::Staged)) => push_staged(cx, message, &scope).await,
        (Some(scope), Some(StashPart::Unstaged)) => {
            // The files with unstaged changes only: a staged-only one isn't touched.
            let picked = unstaged_in(&cx.api.cli, cx.root, &scope).await?;
            if picked.is_empty() {
                return Ok(None);
            }
            let scope = renames_of(&cx.api.cli, cx.root, picked).await?;
            push_git(cx, message, Some(scope), asked).await
        }
        (scope, _) => push_git(cx, message, scope, asked).await,
    }
}

/// The files under `scope` with unstaged changes, and its untracked files (`stash -u`'s).
async fn unstaged_in(cli: &GitCli, root: &Path, scope: &[Bytes]) -> Result<Vec<Bytes>, GbError> {
    let read = async |args: &[&str]| {
        let args: Vec<OsString> = args.iter().map(OsString::from).chain(scope.iter().map(|p| crate::platform::osstr::from_vec(p.clone()))).collect();
        let out = cli.run(GitInvocation::new(root, args).env("GIT_LITERAL_PATHSPECS", "1")).await?;
        Ok::<_, GbError>(out.stdout.split(|b| *b == 0).filter(|s| !s.is_empty()).map(<[u8]>::to_vec).collect::<Vec<_>>())
    };
    // `diff`, not `diff-files`: a file whose stat alone changed isn't listed.
    let mut out = read(&["diff", "--name-only", "-z", "--no-renames", "--no-ext-diff", "--no-color", "--"]).await?;
    out.extend(read(&["ls-files", "-o", "--exclude-standard", "-z", "--"]).await?);
    out.retain(|p| within(p, scope));
    out.sort();
    out.dedup();
    Ok(out)
}

/// `git stash push --include-untracked -m <message> [-- <scope>]`, narrowed to the scope; an
/// Unstaged part then checks the staged one out again. A stash git stored before it failed (its
/// cleanup) is recorded here, and the error says so.
async fn push_git(cx: &mut WriteCx<'_>, message: &str, scope: Option<Vec<Bytes>>, asked: (Option<&[String]>, Option<StashPart>)) -> Result<Option<(String, String)>, GbError> {
    let (paths, part) = asked;
    let moved = |oid: String, message: String| StashMove { oid, message, created: true, without_index: false, paths: paths.map(<[String]>::to_vec), part };
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
    // An Unstaged part's staged files, which git rewrites twice: their permission bits.
    let bits = if part == Some(StashPart::Unstaged) { bits_of(cx.root, &staged.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>()) } else { Vec::new() };
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
        cx.record_stash(moved(oid, subject.clone()))?;
        let message = format!("git stored the stash \"{}\", then failed: {}. The changes are in the stash; some may still be in the files too", user_message(&subject), e.message);
        return Err(GbError { message, ..e });
    }
    let Some(scope) = &scope else { return Ok(Some((oid, subject))) };
    let stored = match narrow(cx, &oid, &subject, scope, &staged, &gone).await {
        Ok(narrowed) => narrowed,
        Err(e) => {
            tracing::warn!(target: "gitvolt_core::write", "the stash {oid} is kept as git made it, with the other staged changes: {e}");
            oid
        }
    };
    if part == Some(StashPart::Unstaged)
        && let Err(e) = keep_staged(cx, &stored, &staged, &gone, &bits).await
    {
        cx.record_stash(moved(stored, subject.clone()))?;
        let message = format!("The unstaged changes are stashed in \"{}\", but the staged ones couldn't be put back ({}). They're in the stash too: Undo puts everything back", user_message(&subject), e.message);
        return Err(GbError { message, ..e });
    }
    Ok(Some((stored, subject)))
}

/// An Unstaged part: git's push took the files' staged changes too (its `-- <paths>` resets them
/// to HEAD); they're checked out again from the stash's index part (it holds them, verified),
/// staged and in the files, their permission bits kept (`bits`). A staged deletion put back in
/// the index before the push (`gone`) is deleted again instead: git's own index part, kept when
/// narrowing fails, has that file. The stash holds them as its index part, which a Pop finds
/// staged already (`Route::Rebased`): it brings back the unstaged changes only.
async fn keep_staged(cx: &mut WriteCx<'_>, stash: &str, staged: &[Change], gone: &[Bytes], bits: &[(Bytes, u32)]) -> Result<(), GbError> {
    let paths: Vec<Bytes> = staged.iter().map(|(p, _)| p.clone()).filter(|p| !gone.contains(p)).collect();
    if !paths.is_empty() {
        let index = format!("{stash}^2");
        let inv = cx.git_stash(["checkout", "--no-overlay", index.as_str(), "--pathspec-from-file=-", "--pathspec-file-nul"]).env("GIT_LITERAL_PATHSPECS", "1").stdin(nul_bytes(&paths));
        cx.run_git(inv).await?;
        keep_bits(cx.root, bits);
    }
    if !gone.is_empty() {
        // git's cleanup wrote HEAD's file back: `rm` (no `-f`) deletes it only while it's HEAD's.
        let inv = cx.git_stash(["rm", "-q", "--ignore-unmatch", "--pathspec-from-file=-", "--pathspec-file-nul"]).env("GIT_LITERAL_PATHSPECS", "1").stdin(nul_bytes(gone));
        cx.run_git(inv).await?;
    }
    Ok(())
}

/// Each file's permission bits, read before it's written again: git writes a file anew with the
/// default ones, and a 0600 file must stay 0600.
fn bits_of(root: &Path, paths: &[Bytes]) -> Vec<(Bytes, u32)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        paths
            .iter()
            .filter_map(|p| {
                let m = std::fs::symlink_metadata(root.join(crate::platform::osstr::from_bytes(p))).ok()?;
                m.is_file().then(|| (p.clone(), m.permissions().mode() & 0o7777))
            })
            .collect()
    }
    #[cfg(not(unix))]
    {
        let _ = (root, paths);
        Vec::new()
    }
}

/// `bits_of`'s bits back on the files still there, but for the exec bit git wrote (a staged
/// mode change taken out or put back): best effort, each failure logged.
fn keep_bits(root: &Path, bits: &[(Bytes, u32)]) {
    #[cfg(unix)]
    for (p, was) in bits {
        use std::os::unix::fs::PermissionsExt;
        let path = root.join(crate::platform::osstr::from_bytes(p));
        let Ok(m) = std::fs::symlink_metadata(&path) else { continue };
        if !m.is_file() {
            continue;
        }
        let now = m.permissions().mode() & 0o7777;
        let want = match (now & 0o111 != 0, was & 0o111 != 0) {
            (exec, had) if exec == had => *was,
            (true, _) => was | ((was & 0o444) >> 2),
            (false, _) => was & !0o111,
        };
        if want != now
            && let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(want))
        {
            tracing::warn!(target: "gitvolt_core::write", "keeping the permissions of {}: {e}", path.display());
        }
    }
    #[cfg(not(unix))]
    let _ = (root, bits);
}

/// A file's `(mode, oid)` as git has it.
type Entry = (String, String);

/// Why one file's part can't be taken out of it, or put into it, alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Clash {
    /// Edits on or next to the same lines (`merge-file` conflicts).
    Lines,
    Binary,
    Link,
    /// A file on one side, a symbolic link on another.
    Kind,
    Submodule,
    /// One side adds or deletes the file, the other changes it.
    Whole,
}

fn pick<'a, T: PartialEq>(ours: &'a T, base: &'a T, theirs: &'a T) -> Option<&'a T> {
    if ours == base {
        Some(theirs)
    } else if theirs == base || ours == theirs {
        Some(ours)
    } else {
        None
    }
}

/// The 3-way merge of one file's entries (git's blobs, clean filters applied): `ours` with what
/// changed from `base` to `theirs`, the exec bit too. Contents merge with `git merge-file`, so
/// a change lands at its own lines, never at the nearest ones its context also matches, as a
/// patch's hunk may.
async fn merged(cx: &WriteCx<'_>, ours: Option<&Entry>, base: Option<&Entry>, theirs: Option<&Entry>) -> Result<Result<Option<Entry>, Clash>, GbError> {
    if let Some(e) = pick(&ours, &base, &theirs) {
        return Ok(Ok((*e).cloned()));
    }
    let (Some(o), Some(b), Some(t)) = (ours, base, theirs) else { return Ok(Err(Clash::Whole)) };
    let kind = |m: &str| match m {
        "160000" => 2,
        "120000" => 1,
        _ => 0,
    };
    if [o, b, t].iter().any(|e| kind(&e.0) == 2) {
        return Ok(Err(Clash::Submodule));
    }
    if kind(&o.0) != kind(&b.0) || kind(&t.0) != kind(&b.0) {
        return Ok(Err(Clash::Kind));
    }
    let Some(mode) = pick(&o.0, &b.0, &t.0) else { return Ok(Err(Clash::Kind)) };
    let oid = match pick(&o.1, &b.1, &t.1) {
        Some(oid) => oid.clone(),
        None if kind(&b.0) == 1 => return Ok(Err(Clash::Link)),
        None => match merge_file(cx, &o.1, &b.1, &t.1).await? {
            Ok(oid) => oid,
            Err(clash) => return Ok(Err(clash)),
        },
    };
    Ok(Ok(Some((mode.clone(), oid))))
}

/// `git merge-file -p` of three blobs (written out as git stores them), the result stored as a
/// blob as it is (no filters: it's on git's side of them already).
async fn merge_file(cx: &WriteCx<'_>, ours: &str, base: &str, theirs: &str) -> Result<Result<String, Clash>, GbError> {
    let dir = tempfile::Builder::new().prefix("stash-merge-").tempdir_in(&cx.tmp)?;
    let mut args: Vec<OsString> = ["merge-file", "-p", "-q"].into_iter().map(OsString::from).collect();
    for (name, oid) in [("ours", ours), ("base", base), ("theirs", theirs)] {
        let blob = cx.api.cli.run(GitInvocation::new(cx.root, ["cat-file", "blob", oid])).await?.stdout;
        let path = dir.path().join(name);
        std::fs::write(&path, blob)?;
        args.push(path.into_os_string());
    }
    match cx.api.cli.run(GitInvocation::new(cx.root, args)).await {
        Ok(out) => Ok(Ok(String::from_utf8_lossy(&cx.api.cli.run(cx.git_stash(["hash-object", "-w", "--stdin"]).stdin(out.stdout)).await?.stdout).trim().to_string())),
        // `error: Cannot merge binary files: …`; otherwise its exit code counts the conflicts.
        Err(e) if e.stderr.as_deref().unwrap_or_default().contains("binary") => Ok(Err(Clash::Binary)),
        Err(_) => Ok(Err(Clash::Lines)),
    }
}

/// A copy of the real index, for the files as git reads them (`files_now`) and their writes
/// (`write_files`); the real one isn't touched.
struct TempIndex {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
}

impl TempIndex {
    fn new(cx: &WriteCx<'_>) -> Result<Self, GbError> {
        let dir = tempfile::Builder::new().prefix("stash-files-").tempdir_in(&cx.tmp)?;
        let path = dir.path().join("index");
        let real = gix::open(cx.root).map_err(gix_err)?.index_path();
        if real.is_file() {
            std::fs::copy(&real, &path)?;
        }
        Ok(Self { _dir: dir, path })
    }
}

/// Each of `paths`' files as git reads it (its blob, clean filters applied, and mode), through
/// the temp index (its stat data fresh then); `None`: no file there.
async fn files_now(cx: &WriteCx<'_>, temp: &TempIndex, paths: &[Bytes]) -> Result<std::collections::HashMap<Bytes, Entry>, GbError> {
    // Removed, then added: hashed whatever their stat data says (the copy's own mtime would hide a
    // same-size edit made in the second the real index was written: git's racy check).
    cx.api.cli.run(cx.git_stash(["update-index", "--force-remove", "-z", "--stdin"]).env("GIT_INDEX_FILE", &temp.path).stdin(nul_bytes(paths))).await?;
    let there: Vec<Bytes> = paths.iter().filter(|p| std::fs::symlink_metadata(cx.root.join(crate::platform::osstr::from_bytes(p))).is_ok()).cloned().collect();
    cx.api.cli.run(cx.git_stash(["update-index", "--add", "-z", "--stdin"]).env("GIT_INDEX_FILE", &temp.path).stdin(nul_bytes(&there))).await?;
    let wanted: std::collections::HashSet<&[u8]> = paths.iter().map(Vec::as_slice).collect();
    let listed = cx.api.cli.run(GitInvocation::new(cx.root, ["ls-files", "-s", "-z"]).env("GIT_INDEX_FILE", &temp.path)).await?.stdout;
    // `<mode> <oid> <stage>\t<path>`.
    Ok(listed
        .split(|b| *b == 0)
        .filter_map(|r| {
            let tab = r.iter().position(|b| *b == b'\t')?;
            let path = &r[tab + 1..];
            if !wanted.contains(path) {
                return None;
            }
            let meta = String::from_utf8_lossy(&r[..tab]);
            let mut f = meta.split(' ');
            Some((path.to_vec(), (f.next()?.to_string(), f.next()?.to_string())))
        })
        .collect())
}

/// The files of `results` written as they say (`None`: deleted), filters and exec bits as git
/// writes them, through a two-way `read-tree -m -u` on the temp index: it refuses a file that
/// changed since `files_now` read it. The other permission bits are kept.
async fn write_files(cx: &mut WriteCx<'_>, temp: &TempIndex, results: &[Change]) -> Result<(), GbError> {
    let old = String::from_utf8_lossy(&cx.api.cli.run(cx.git_stash(["write-tree"]).env("GIT_INDEX_FILE", &temp.path)).await?.stdout).trim().to_string();
    let new = tree_with(cx, &old, &results.iter().collect::<Vec<_>>()).await?;
    let paths: Vec<Bytes> = results.iter().map(|(p, _)| p.clone()).collect();
    let bits = bits_of(cx.root, &paths);
    let inv = cx.git_stash(["read-tree", "-m", "-u", old.as_str(), new.as_str()]).env("GIT_INDEX_FILE", &temp.path);
    cx.run_git(inv).await?;
    keep_bits(cx.root, &bits);
    Ok(())
}

/// A Staged part: git's `stash push --staged -- <paths>` records the whole index and takes all
/// of it out of the files (as a patch, whose hunks may land on other lines), so it's built here,
/// of the scope only: the stash's index and worktree parts are the scope's staged changes, on
/// HEAD; each file is merged from its staged version to HEAD's, keeping its unstaged edits, and
/// unstaged. Every file is merged before anything changes: a clash refuses, nothing stashed.
/// The commits' identity and message are those of git's `stash create`, run first.
async fn push_staged(cx: &mut WriteCx<'_>, message: &str, scope: &[Bytes]) -> Result<Option<(String, String)>, GbError> {
    let (cli, root) = (&cx.api.cli, cx.root);
    let Ok(head) = read_text(cli, root, &["rev-parse", "--verify", "-q", "HEAD^{commit}"]).await else {
        return Err(GbError::new(GbErrorKind::InvalidInput, "You do not have the initial commit yet"));
    };
    if !raw_names(cli, root, &["ls-files", "-u", "-z"]).await?.is_empty() {
        return Err(GbError::new(GbErrorKind::InvalidInput, "Resolve the conflicted files first"));
    }
    let both: Vec<Pair> = pairs(cli, root, &["diff-index", "--cached", "-r", "-z", "--no-renames", &head]).await?.into_iter().filter(|(p, _, _)| within(p, scope)).collect();
    if both.is_empty() {
        return Ok(None);
    }
    let staged: Vec<Change> = both.iter().map(|(p, _, s)| (p.clone(), s.clone())).collect();
    let paths: Vec<Bytes> = staged.iter().map(|(p, _)| p.clone()).collect();
    let tree = tree_with(cx, &head, &staged.iter().collect::<Vec<_>>()).await?;
    // Each file without its staged part: merged from the staged version to HEAD's.
    let temp = TempIndex::new(cx)?;
    let files = files_now(cx, &temp, &paths).await?;
    let mut left: Vec<Change> = Vec::new();
    for (p, h, s) in &both {
        match merged(cx, files.get(p), s.as_ref(), h.as_ref()).await? {
            Ok(e) => left.push((p.clone(), e)),
            Err(clash) => return Err(refusal(cx, p, s.is_some(), h.is_some(), clash).await),
        }
    }

    // git's own stash commit, never stored: the identity and dates git chooses (its fallback
    // one too), and its message. Its index part is checked to be what was read.
    let (cli, root) = (&cx.api.cli, cx.root);
    let created = cx.run_git(cx.git(["stash", "create", message])).await?;
    let created = String::from_utf8_lossy(&created.stdout).trim().to_string();
    let parents = read_text(cli, root, &["show", "-s", "--format=%P", &created]).await?;
    let index = match parents.split(' ').collect::<Vec<_>>().as_slice() {
        [h, i, ..] if *h == head => i.to_string(),
        _ => return Err(GbError::stale("HEAD moved during the stash; nothing was stashed")),
    };
    let diff = async |to: &str| changes(cli, root, &["diff-tree", "-r", "-z", "--no-renames", &head, to]).await;
    if diff(&index).await?.into_iter().filter(|(p, _)| within(p, scope)).collect::<Vec<_>>() != staged {
        return Err(GbError::stale("The staged files changed during the stash; nothing was stashed"));
    }
    let i_commit = commit_like(cx, &index, &tree, &[&head]).await?;
    let stash = commit_like(cx, &created, &tree, &[&head, &i_commit]).await?;
    let w_parents = read_text(cli, root, &["show", "-s", "--format=%P", &stash]).await?;
    let w_tree = read_text(cli, root, &["rev-parse", &format!("{stash}^{{tree}}")]).await?;
    if diff(&i_commit).await? != staged || w_tree != tree || w_parents != format!("{head} {i_commit}") {
        return Err(GbError::other(format!("the stash {stash} doesn't hold the staged changes; nothing was stashed")));
    }
    let full = read_text(cli, root, &["show", "-s", "--format=%B", &created]).await?;
    let subject = full.lines().next().unwrap_or_default().trim().to_string();

    let inv = cx.git_stash(["stash", "store", "-m", subject.as_str(), stash.as_str()]);
    cx.run_git(inv).await?;
    cx.touch(ChangeKind::Stash);
    // From here a failure puts back what changed and drops the stash: nothing is stashed.
    if let Err(e) = write_files(cx, &temp, &left).await {
        unstash(cx, &stash).await;
        return Err(GbError { message: format!("The staged changes couldn't be taken out of the files ({}); nothing was stashed", e.message), ..e });
    }
    cx.partial = true;
    cx.touch(ChangeKind::Worktree);
    if let Err(e) = unstage(cx, &paths).await {
        let back: Vec<Change> = paths.iter().map(|p| (p.clone(), files.get(p).cloned())).collect();
        match write_files(cx, &temp, &back).await {
            Ok(()) => unstash(cx, &stash).await,
            // Both are in the stash: the files' unstaged edits, the index its staged changes.
            Err(put) => {
                tracing::warn!(target: "gitvolt_core::write", "putting the staged changes back in the files: {put}");
                cx.record_stash(StashMove { oid: stash, message: subject, created: true, without_index: false, paths: None, part: Some(StashPart::Staged) })?;
                return Err(GbError { message: format!("The staged changes are stashed and out of the files, but still staged ({}): unstage them, or Undo", e.message), ..e });
            }
        }
        return Err(GbError { message: format!("The staged changes couldn't be unstaged ({}); nothing was stashed", e.message), ..e });
    }
    cx.touch(ChangeKind::Index);
    Ok(Some((stash, subject)))
}

/// Why a Staged part's file refuses, as the user sees it. `staged`/`in_head`: the file is in the
/// index / in HEAD.
async fn refusal(cx: &WriteCx<'_>, path: &[u8], staged: bool, in_head: bool, clash: Clash) -> GbError {
    let name = String::from_utf8_lossy(path);
    let why = match clash {
        Clash::Lines => format!("{name} has unstaged edits on or next to the same lines"),
        Clash::Binary => format!("{name} is a binary file with unstaged edits"),
        Clash::Link => format!("{name} is a symbolic link with an unstaged change"),
        Clash::Kind => format!("{name} has an unstaged change between a file and a symbolic link"),
        Clash::Submodule => return GbError::new(GbErrorKind::Conflict, format!("Can't stash the staged changes alone: {name} is a submodule. Nothing was stashed")),
        Clash::Whole if !staged => format!("{name} is staged as deleted, but there's a file there again"),
        Clash::Whole if !in_head => {
            // A rename's new side, or a new file: the edits would have no file to stay in.
            let listed = raw_names(&cx.api.cli, cx.root, &["diff-index", "--cached", "-M", "--name-status", "-z", "HEAD"]).await.unwrap_or_default();
            // `<status>`, `<path>`, and a rename's (or copy's) new path.
            let (mut it, mut renamed) = (listed.iter(), false);
            while let (Some(status), Some(_)) = (it.next(), it.next()) {
                if status.starts_with(b"R") || status.starts_with(b"C") {
                    renamed |= it.next().is_some_and(|to| to == path) && status.starts_with(b"R");
                }
            }
            let what = if renamed { "renamed" } else { "a new file" };
            format!("{name} is {what} with unstaged edits, which would have no file to stay in")
        }
        Clash::Whole => format!("{name} is deleted, unstaged"),
    };
    GbError::new(GbErrorKind::Conflict, format!("Can't stash the staged changes alone: {why}. Stash its unstaged changes first, or stage them. Nothing was stashed"))
}

/// `git reset -q -- <paths>`, retrying a moment on another process's `index.lock`.
async fn unstage(cx: &mut WriteCx<'_>, paths: &[Bytes]) -> Result<(), GbError> {
    for tries in 1.. {
        let inv = cx.git_stash(["reset", "-q", "--pathspec-from-file=-", "--pathspec-file-nul"]).env("GIT_LITERAL_PATHSPECS", "1").stdin(nul_bytes(paths));
        match cx.run_git(inv).await {
            Ok(_) => return Ok(()),
            Err(_) if tries < 5 => tokio::time::sleep(std::time::Duration::from_millis(100 * tries)).await,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

/// The stash `oid` just stored goes again (on top, checked); one that stays is only logged.
async fn unstash(cx: &mut WriteCx<'_>, oid: &str) {
    if let Err(e) = drop_at(cx, 0, oid, false).await {
        tracing::warn!(target: "gitvolt_core::write", "the stash {oid} stays listed: {e}");
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

/// A change with both sides: the path, its old and its new `(mode, oid)` (`None`: absent).
type Pair = (Bytes, Option<Entry>, Option<Entry>);

/// Each file a raw diff (`diff-tree -r -z`, `diff-index -z`) lists as changed.
async fn changes(cli: &GitCli, root: &Path, args: &[&str]) -> Result<Vec<Change>, GbError> {
    Ok(pairs(cli, root, args).await?.into_iter().map(|(p, _, new)| (p, new)).collect())
}

/// `changes` with the old side too.
async fn pairs(cli: &GitCli, root: &Path, args: &[&str]) -> Result<Vec<Pair>, GbError> {
    let listed = raw_names(cli, root, args).await?;
    // `:<old mode> <new mode> <old oid> <new oid> <status>`, then the path.
    Ok(listed
        .chunks(2)
        .filter_map(|c| {
            let [meta, path] = c else { return None };
            let meta = String::from_utf8_lossy(meta);
            let f: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
            let (old_mode, mode, old, oid, status) = (f.first()?, f.get(1)?, f.get(2)?, f.get(3)?, f.get(4)?);
            let old = (!status.starts_with('A')).then(|| (old_mode.to_string(), old.to_string()));
            Some((path.clone(), old, (!status.starts_with('D')).then(|| (mode.to_string(), oid.to_string()))))
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
    #[cfg(test)]
    if tests::NARROW_FAILS.lock().unwrap().iter().any(|r| r.as_path() == cx.root) {
        return Err(GbError::other("narrowing fails (test)"));
    }
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
        if self.part.is_some() && self.paths.is_none() {
            return Err(GbError::new(GbErrorKind::InvalidInput, "Only some files' staged or unstaged changes can be stashed"));
        }
        refuse_stash_push_in_the_way(pre, "stash", self.paths.as_deref()).await?;
        Ok(Plan::default())
    }
    async fn run(&self, cx: &mut WriteCx<'_>) -> Result<StashPushOutcome, GbError> {
        let Some((oid, subject)) = push(cx, &self.message, self.paths.as_deref(), self.part).await? else { return Ok(StashPushOutcome::NothingToStash) };
        // Written ahead of the relist's drop.
        cx.record_stash(StashMove { oid: oid.clone(), message: subject.clone(), created: true, without_index: false, paths: self.paths.clone(), part: self.part })?;
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

/// How `apply --index` restores a stash with an index part, besides git's own route.
enum Route {
    Git,
    /// Its index part is staged already (an Unstaged part's stash, or a Staged part's popped
    /// after it): git's route would refuse it ("Conflicts in index"). A copy of the stash
    /// based on its index part is applied instead, the index part no change then: the files
    /// get the worktree part's changes over the index part's, and the index stays.
    Rebased(String),
    /// A staged part only (W is I), its files edited since (a Staged part's unstaged edits): git's
    /// merge refuses a file with local changes. Its changes go to the index as they are, and
    /// into each file by a 3-way merge (`merged`).
    Merge(Vec<Pair>),
}

async fn route(cx: &WriteCx<'_>, oid: &str) -> Result<Route, GbError> {
    let (cli, root) = (&cx.api.cli, cx.root);
    let tree = async |c: &str| read_text(cli, root, &["rev-parse", &format!("{oid}{c}^{{tree}}")]).await;
    let (b, i, w) = (tree("^1").await?, tree("^2").await?, tree("").await?);
    if b == i {
        return Ok(Route::Git);
    }
    let parents = read_text(cli, root, &["show", "-s", "--format=%P", oid]).await?;
    let parents: Vec<&str> = parents.split(' ').collect();
    let [_, index, untracked @ ..] = parents.as_slice() else { return Ok(Route::Git) };
    let part = raw_names(cli, root, &["diff-tree", "-r", "--name-only", "--no-renames", "-z", &b, &i]).await?;
    let differs = async |from: &str| raw_names(cli, root, &["diff-index", "--cached", "--name-only", "--no-renames", "-z", from]).await;
    if !differs(&i).await?.iter().any(|p| part.contains(p)) {
        let again = commit_like(cx, oid, &i, &[index]).await?;
        let parents: Vec<&str> = [*index, again.as_str()].into_iter().chain(untracked.iter().copied()).collect();
        return Ok(Route::Rebased(commit_like(cx, oid, &w, &parents).await?));
    }
    if w != i || !untracked.is_empty() || differs(&b).await?.iter().any(|p| part.contains(p)) {
        return Ok(Route::Git);
    }
    let edited = raw_names(cli, root, &["diff", "--name-only", "--no-renames", "--no-ext-diff", "--no-color", "-z"]).await?;
    if !edited.iter().any(|p| part.contains(p)) {
        return Ok(Route::Git);
    }
    Ok(Route::Merge(pairs(cli, root, &["diff-tree", "-r", "-z", "--no-renames", &b, &i]).await?))
}

/// `Route::Merge`: every file merged first (nothing changes on a clash), then the index part
/// staged and the files written; a failure writing them puts the index back.
async fn apply_merge(cx: &mut WriteCx<'_>, part: Vec<Pair>) -> Result<(), GbError> {
    let paths: Vec<Bytes> = part.iter().map(|(p, _, _)| p.clone()).collect();
    let temp = TempIndex::new(cx)?;
    let files = files_now(cx, &temp, &paths).await?;
    let mut out: Vec<Change> = Vec::new();
    for (p, b, i) in &part {
        match merged(cx, files.get(p), b.as_ref(), i.as_ref()).await? {
            Ok(e) => out.push((p.clone(), e)),
            Err(_) => return Err(GbError::new(GbErrorKind::Conflict, format!("The stash's staged changes don't go into {} as it is now: it has edits on or next to the same lines; nothing changed", String::from_utf8_lossy(p)))),
        }
    }
    let len = part.iter().find_map(|(_, b, i)| b.as_ref().or(i.as_ref()).map(|e| e.1.len())).unwrap_or(40);
    let info = |side: &dyn Fn(&Pair) -> Option<Entry>| {
        let mut info = Vec::new();
        for pair in &part {
            let (mode, oid) = side(pair).unwrap_or_else(|| ("0".into(), "0".repeat(len)));
            info.extend_from_slice(format!("{mode} {oid}\t").as_bytes());
            info.extend_from_slice(&pair.0);
            info.push(0);
        }
        info
    };
    let inv = cx.git_stash(["update-index", "-z", "--index-info"]).stdin(info(&|(_, _, i)| i.clone()));
    cx.run_git(inv).await?;
    cx.partial = true;
    cx.touch(ChangeKind::Index);
    if let Err(e) = write_files(cx, &temp, &out).await {
        let inv = cx.git_stash(["update-index", "-z", "--index-info"]).stdin(info(&|(_, b, _)| b.clone()));
        if let Err(back) = cx.run_git(inv).await {
            tracing::warn!(target: "gitvolt_core::write", "unstaging the stash's staged changes again: {back}");
        }
        return Err(GbError { message: format!("The stash's staged changes couldn't be written to the files ({}); nothing changed", e.message), ..e });
    }
    cx.touch(ChangeKind::Worktree);
    Ok(())
}

/// `git stash apply [--index] <oid>`; `Ok(None)` on a clean apply, `Ok(Some(files))` with
/// content conflicts; git refusing the index is `ApplyWithoutIndex` (nothing changed).
async fn apply(cx: &mut WriteCx<'_>, oid: &str, with_index: bool) -> Result<Option<u32>, GbError> {
    // git prints a merge's CONFLICT lines on stdout, which a failure doesn't keep: conflicts are
    // the unmerged paths the apply added.
    let unmerged_before = precheck::dirty(&cx.api.cli, cx.root).await?.conflicted;
    // With conflicts already, git refuses the apply: as it is.
    let route = if with_index && unmerged_before == 0 { route(cx, oid).await? } else { Route::Git };
    let target = match route {
        Route::Git => oid.to_string(),
        Route::Rebased(copy) => copy,
        Route::Merge(part) => return apply_merge(cx, part).await.map(|()| None),
    };
    let aside = if with_index && unmerged_before == 0 && target == oid { index_aside(cx, oid).await? } else { None };
    let mut args = vec!["stash", "apply"];
    if with_index {
        args.push("--index");
    }
    args.push(target.as_str());
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
                    cx.record_stash(StashMove { oid: self.oid.clone(), message: self.subject.clone(), created: false, without_index: self.without_index, paths: None, part: None })?;
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
        cx.record_stash(StashMove { oid: self.oid.clone(), message: self.subject.clone(), created: false, without_index: false, paths: None, part: None })?;
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
                let Some((oid, subject)) = push(cx, &message, m.paths.as_deref(), m.part).await? else { return Err(GbError::new(GbErrorKind::InvalidInput, "Nothing to stash")) };
                // Written ahead of the relist's drop.
                let new = |subject: String| StashMove { oid: oid.clone(), message: subject, created: true, without_index: false, paths: m.paths.clone(), part: m.part };
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

/// `stashPush`: an empty `message` names HEAD's branch, read first (and the paths and part, if any).
pub(crate) async fn stash_push(api: &Api, repo: u32, worktree: &str, message: String, paths: Option<Vec<String>>, part: Option<StashPart>) -> Result<WriteResult<StashPushOutcome>, GbError> {
    let h = api.handle(repo)?;
    let root = api.worktree_dir(&h, worktree).await?;
    let branch = blocking(move || crate::write::head_state(&gix::open(&root).map_err(gix_err)?)).await?.branch;
    run_write(api, repo, worktree, Default::default(), StashPush::new(message, branch.as_deref(), paths, part)).await
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

    /// Roots whose `narrow` fails (its fallback, git's own stash kept).
    pub(super) static NARROW_FAILS: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());

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
                    e.stashes.push(StashMove { oid, message: format!("On main: {}", &label[label.find('"').unwrap() + 1..label.len() - 1]), created, without_index: false, paths: None, part: None });
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
        e.stashes.push(StashMove { oid: "1a2b".into(), message: "On main: x".into(), created: false, without_index: false, paths: None, part: None });
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

    // --- a WIP list's part ---

    const TEN: &str = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";

    /// m.txt mixed (line 1 staged, line 8 edited further), s.txt staged, u.txt unstaged, n.txt
    /// untracked; d/x.txt mixed and d/y.txt untracked; o.txt staged, another file.
    fn mixed() -> TestRepo {
        let r = TestRepo::new();
        identity(&r);
        for f in ["m.txt", "s.txt", "u.txt", "o.txt", "d/x.txt"] {
            r.write(f, TEN);
        }
        r.git(&["add", "."]);
        r.git(&["commit", "-q", "-m", "one"]);
        for f in ["m.txt", "s.txt", "o.txt", "d/x.txt"] {
            r.write(f, &TEN.replacen("1\n", "one\n", 1));
        }
        r.git(&["add", "."]);
        for f in ["m.txt", "d/x.txt", "u.txt"] {
            r.write(f, &std::fs::read_to_string(r.path().join(f)).unwrap().replace("8\n", "eight\n"));
        }
        r.write("n.txt", "n\n");
        r.write("d/y.txt", "y\n");
        assert_eq!(r.git(&["status", "--porcelain", "-uall"]), "MM d/x.txt\nMM m.txt\nM  o.txt\nM  s.txt\n M u.txt\n?? d/y.txt\n?? n.txt");
        r
    }

    async fn stash_part(api: &crate::api::Api, id: u32, r: &TestRepo, paths: &[&str], part: &str) -> Result<serde_json::Value, crate::error::GbError> {
        send(api, json!({"method": "stashPush", "params": {"repo": id, "worktree": wt(r), "message": "", "paths": paths, "part": part}})).await
    }

    async fn pop_top(api: &crate::api::Api, id: u32, r: &TestRepo) {
        let out = send(api, json!({"method": "stashApply", "params": {"repo": id, "worktree": wt(r), "oid": top(r), "pop": true}})).await.unwrap();
        assert_eq!(out["outcome"]["status"], "applied");
    }

    /// The newest stash's index part and worktree part, each as `git diff --name-status`.
    fn parts(r: &TestRepo) -> (String, String) {
        (r.git(&["diff", "--name-status", "--no-renames", "stash@{0}^1", "stash@{0}^2"]), r.git(&["diff", "--name-status", "--no-renames", "stash@{0}^2", "stash@{0}"]))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_staged_stash_takes_the_staged_part_only() {
        let env = WriteEnv::new();
        let status = |r: &TestRepo| r.git(&["status", "--porcelain", "-uall"]);
        for (paths, left, held, name) in [
            // Mixed: the further edit stays, unstaged, on HEAD's line 1.
            (vec!["m.txt"], "MM d/x.txt\n M m.txt\nM  o.txt\nM  s.txt\n M u.txt\n?? d/y.txt\n?? n.txt", "M\tm.txt", "WIP on main: m.txt (staged)"),
            (vec!["s.txt"], "MM d/x.txt\nMM m.txt\nM  o.txt\n M u.txt\n?? d/y.txt\n?? n.txt", "M\ts.txt", "WIP on main: s.txt (staged)"),
            // A folder: its untracked file stays.
            (vec!["d"], " M d/x.txt\nMM m.txt\nM  o.txt\nM  s.txt\n M u.txt\n?? d/y.txt\n?? n.txt", "M\td/x.txt", "WIP on main: d (staged)"),
        ] {
            let r = mixed();
            let id = open(&env.api, &r).await;
            let before = RepoState::capture(&r);
            let out = stash_part(&env.api, id, &r, &paths, "staged").await.unwrap();
            assert_eq!(out["outcome"]["oid"].as_str(), Some(top(&r).as_str()));
            assert_eq!(status(&r), left, "{paths:?}: the others untouched");
            assert_eq!(parts(&r), (held.to_string(), String::new()), "{paths:?}: the staged part, as both index and worktree parts");
            assert!(r.try_git(&["rev-parse", "-q", "--verify", "stash@{0}^3"]).is_err(), "no untracked files");
            assert_eq!(r.git(&["rev-parse", "stash@{0}^1"]), r.git(&["rev-parse", "HEAD"]));
            assert_eq!(r.git(&["stash", "list", "--format=%gs"]), format!("On main: {name}"));
            let after = RepoState::capture(&r);
            if paths == ["m.txt"] {
                assert_eq!(std::fs::read_to_string(r.path().join("m.txt")).unwrap(), TEN.replace("8\n", "eight\n"));
            }
            undo_redo(&env.api, id, &r, &before, |s| {
                assert_eq!(s.files, after.files, "{paths:?}: redo takes the staged part again");
                assert_eq!(s.index, after.index);
                assert_eq!(parts(&r).0, held);
            })
            .await;
            let s = journal(&env.api, id, &r).await;
            send(&env.api, json!({"method": "undo", "params": {"repo": id, "worktree": wt(&r), "entry": s["undo"]["entry"]}})).await.unwrap();
            assert_eq!(RepoState::capture(&r), before, "{paths:?}: undo puts the staged part back, staged");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unstaged_stash_takes_the_unstaged_part_only() {
        let env = WriteEnv::new();
        let status = |r: &TestRepo| r.git(&["status", "--porcelain", "-uall"]);
        for (paths, left, name) in [
            // Mixed: the staged part stays staged, and in the file.
            (vec!["m.txt"], "MM d/x.txt\nM  m.txt\nM  o.txt\nM  s.txt\n M u.txt\n?? d/y.txt\n?? n.txt", "WIP on main: m.txt (unstaged)"),
            (vec!["u.txt"], "MM d/x.txt\nMM m.txt\nM  o.txt\nM  s.txt\n?? d/y.txt\n?? n.txt", "WIP on main: u.txt (unstaged)"),
            (vec!["n.txt"], "MM d/x.txt\nMM m.txt\nM  o.txt\nM  s.txt\n M u.txt\n?? d/y.txt", "WIP on main: n.txt (unstaged)"),
            (vec!["d"], "M  d/x.txt\nMM m.txt\nM  o.txt\nM  s.txt\n M u.txt\n?? n.txt", "WIP on main: d (unstaged)"),
        ] {
            let r = mixed();
            let id = open(&env.api, &r).await;
            let before = RepoState::capture(&r);
            let s_txt = std::fs::metadata(r.path().join("s.txt")).unwrap().modified().unwrap();
            stash_part(&env.api, id, &r, &paths, "unstaged").await.unwrap();
            assert_eq!(status(&r), left, "{paths:?}: the others untouched");
            assert_eq!(std::fs::metadata(r.path().join("s.txt")).unwrap().modified().unwrap(), s_txt, "a staged-only file isn't written");
            assert_eq!(r.git(&["stash", "list", "--format=%gs"]), format!("On main: {name}"));
            let after = RepoState::capture(&r);
            undo_redo(&env.api, id, &r, &before, |s| {
                assert_eq!(s.files, after.files, "{paths:?}: redo takes the unstaged part again");
                assert_eq!(s.index, after.index);
                assert_eq!(s.untracked, after.untracked);
            })
            .await;
            let s = journal(&env.api, id, &r).await;
            send(&env.api, json!({"method": "undo", "params": {"repo": id, "worktree": wt(&r), "entry": s["undo"]["entry"]}})).await.unwrap();
            assert_eq!(RepoState::capture(&r), before, "{paths:?}: undo puts the unstaged part back, unstaged");
        }
        // Nothing unstaged: nothing to stash, and a staged-only file in Staged's terms neither.
        let r = mixed();
        let id = open(&env.api, &r).await;
        let held = RepoState::capture(&r);
        assert_eq!(stash_part(&env.api, id, &r, &["s.txt"], "unstaged").await.unwrap()["outcome"], json!({"status": "nothingToStash"}));
        assert_eq!(stash_part(&env.api, id, &r, &["u.txt", "n.txt"], "staged").await.unwrap()["outcome"], json!({"status": "nothingToStash"}));
        assert_eq!(RepoState::capture(&r), held);
    }

    /// One mixed file's Staged and Unstaged stashes, pushed and popped in each order, give back
    /// exactly what it was: its staged part staged, its unstaged part unstaged.
    #[tokio::test(flavor = "multi_thread")]
    async fn both_parts_of_a_mixed_file_pop_back_in_either_order() {
        let env = WriteEnv::new();
        for pushed in [["staged", "unstaged"], ["unstaged", "staged"]] {
            for popped_first in ["staged", "unstaged"] {
                let r = mixed();
                let id = open(&env.api, &r).await;
                let before = RepoState::capture(&r);
                let mut oids = std::collections::HashMap::new();
                for part in pushed {
                    let out = stash_part(&env.api, id, &r, &["m.txt"], part).await.unwrap();
                    oids.insert(part, out["outcome"]["oid"].as_str().unwrap().to_string());
                }
                assert!(!r.git(&["status", "--porcelain"]).contains("m.txt"), "{pushed:?}: both parts stashed");
                for part in [popped_first, if popped_first == "staged" { "unstaged" } else { "staged" }] {
                    let out = send(&env.api, json!({"method": "stashApply", "params": {"repo": id, "worktree": wt(&r), "oid": oids[part], "pop": true}})).await;
                    assert_eq!(out.unwrap()["outcome"]["status"], "applied", "{pushed:?}, {part} popped");
                }
                assert_eq!(RepoState::capture(&r), before, "{pushed:?}, {popped_first} popped first");
            }
        }
    }

    /// A terminal's pop: a Staged stash pops back with `--index` onto unedited files (nothing
    /// else staged: git's resets the whole index first); an Unstaged one with `--index` while the
    /// index is its index part, else without it (git's `--index` refuses a staged part).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_terminals_pop_restores_each_part() {
        let env = WriteEnv::new();
        let r = TestRepo::new();
        identity(&r);
        r.write("a.txt", TEN);
        r.write("b.txt", TEN);
        r.git(&["add", "."]);
        r.git(&["commit", "-q", "-m", "one"]);
        r.write("a.txt", &TEN.replacen("1\n", "one\n", 1));
        r.git(&["add", "a.txt"]);
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        stash_part(&env.api, id, &r, &["a.txt"], "staged").await.unwrap();
        r.git(&["stash", "pop", "-q", "--index"]);
        assert_eq!(RepoState::capture(&r), before);
        r.write("a.txt", &TEN.replacen("1\n", "one\n", 1).replace("8\n", "eight\n"));
        r.write("b.txt", "b\n");
        r.write("n.txt", "n\n");
        let before = RepoState::capture(&r);
        stash_part(&env.api, id, &r, &["a.txt", "b.txt", "n.txt"], "unstaged").await.unwrap();
        // The index is the stash's index part whole: git skips it.
        r.git(&["stash", "pop", "-q", "--index"]);
        assert_eq!(RepoState::capture(&r), before);
        // Another file staged since: git's `--index` refuses ("Conflicts in index"), and without it
        // the stash comes back the same.
        stash_part(&env.api, id, &r, &["a.txt", "b.txt", "n.txt"], "unstaged").await.unwrap();
        r.write("c.txt", "c\n");
        r.git(&["add", "c.txt"]);
        let before = RepoState::capture(&r);
        assert!(r.try_git(&["stash", "apply", "-q", "--index"]).is_err());
        assert_eq!(RepoState::capture(&r), before, "git's refusal changed nothing");
        r.git(&["stash", "pop", "-q"]);
        assert_eq!(r.git(&["status", "--porcelain"]), "MM a.txt\n M b.txt\nA  c.txt\n?? n.txt");
    }

    /// A Staged stash popped onto its file's unstaged edits (git's merge refuses a file with local
    /// changes): its patch goes to the index and the file. Apply keeps the stash.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_staged_stash_applies_onto_the_files_unstaged_edits() {
        let env = WriteEnv::new();
        let r = mixed();
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        stash_part(&env.api, id, &r, &["m.txt"], "staged").await.unwrap();
        let oid = top(&r);
        let out = send(&env.api, json!({"method": "stashApply", "params": {"repo": id, "worktree": wt(&r), "oid": oid, "pop": false}})).await.unwrap();
        assert_eq!(out["outcome"]["status"], "applied");
        let mut now = RepoState::capture(&r);
        assert_eq!(top(&r), oid, "kept");
        (now.stashes, now.refs) = (before.stashes.clone(), before.refs.clone());
        assert_eq!(now, before);
    }

    /// A staged edit and an unstaged one on the same lines (or next to each other): the staged
    /// part can't come out of the file alone. Refused, nothing changed, no entry. (`ONE`: an
    /// edit of the same size in the same second as the staging, which only the content shows.)
    #[tokio::test(flavor = "multi_thread")]
    async fn a_staged_stash_over_overlapping_edits_is_refused() {
        let env = WriteEnv::new();
        for further in ["ONE\n2\n", "one\ntwo\n"] {
            let r = TestRepo::new();
            identity(&r);
            r.write("a.txt", TEN);
            r.write("b.txt", "b\n");
            r.git(&["add", "."]);
            r.git(&["commit", "-q", "-m", "one"]);
            r.write("a.txt", &TEN.replacen("1\n", "one\n", 1));
            r.write("b.txt", "b staged\n");
            r.git(&["add", "."]);
            r.write("a.txt", &TEN.replacen("1\n2\n", further, 1));
            let id = open(&env.api, &r).await;
            let held = RepoState::capture(&r);
            let e = stash_part(&env.api, id, &r, &["a.txt"], "staged").await.unwrap_err();
            assert_eq!(e.kind, GbErrorKind::Conflict, "{further:?}");
            assert_eq!(e.message, "Can't stash the staged changes alone: a.txt has unstaged edits on or next to the same lines. Stash its unstaged changes first, or stage them. Nothing was stashed");
            assert_eq!(RepoState::capture(&r), held, "{further:?}");
            assert!(journal(&env.api, id, &r).await["undo"].is_null());
            // Its unstaged part alone is fine.
            stash_part(&env.api, id, &r, &["a.txt"], "unstaged").await.unwrap();
            assert_eq!(r.git(&["status", "--porcelain"]), "M  a.txt\nM  b.txt");
            pop_top(&env.api, id, &r).await;
            assert_eq!(RepoState::capture(&r), held, "{further:?}: popped back");
        }
    }

    /// A staged rename is in Staged: both sides go (whichever is asked), and come back as the
    /// rename. Edited further, the edit would have no file to stay in: refused. From Unstaged,
    /// the edit goes and the rename stays staged.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_staged_rename_stashes_whole_from_staged() {
        let env = WriteEnv::new();
        let r = repo();
        r.write("a.txt", TEN);
        r.git(&["commit", "-q", "-am", "ten"]);
        r.git(&["mv", "a.txt", "moved.txt"]);
        r.write("b.txt", "b unstaged\n");
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        for asked in ["moved.txt", "a.txt"] {
            stash_part(&env.api, id, &r, &[asked], "staged").await.unwrap();
            assert_eq!(r.git(&["status", "--porcelain"]), " M b.txt", "{asked}");
            assert_eq!(parts(&r), ("D\ta.txt\nA\tmoved.txt".to_string(), String::new()));
            pop_top(&env.api, id, &r).await;
            assert_eq!(RepoState::capture(&r), before, "{asked}");
        }
        r.write("moved.txt", &TEN.replace("8\n", "eight\n"));
        let held = RepoState::capture(&r);
        let e = stash_part(&env.api, id, &r, &["moved.txt"], "staged").await.unwrap_err();
        assert_eq!(e.message, "Can't stash the staged changes alone: moved.txt is renamed with unstaged edits, which would have no file to stay in. Stash its unstaged changes first, or stage them. Nothing was stashed");
        assert_eq!(RepoState::capture(&r), held);
        stash_part(&env.api, id, &r, &["moved.txt"], "unstaged").await.unwrap();
        assert_eq!(r.git(&["status", "--porcelain"]), " M b.txt\nR  a.txt -> moved.txt");
        assert_eq!(parts(&r).1, "M\tmoved.txt", "the edit, over the staged rename");
        pop_top(&env.api, id, &r).await;
        assert_eq!(RepoState::capture(&r), held);
    }


    /// A staged hunk whose context also matches elsewhere: a patch (git's `--staged`, `apply`)
    /// lands on the nearest match once unstaged lines moved it, editing a committed line. The
    /// 3-way merge takes it out of, and puts it back into, its own lines.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_staged_part_comes_out_of_and_back_into_its_own_lines() {
        let env = WriteEnv::new();
        let filler = |from: usize, to: usize| (from..=to).map(|n| format!("{n}\n")).collect::<String>();
        let thirty: String = (0..30).map(|n| format!("added {n}\n")).collect();
        // Out: `p y q` at 1-3 matches the reversed hunk (y→x) nearer its place than its own lines.
        let out_head = format!("p\ny\nq\n{}p\nx\nq\n{}", filler(4, 20), filler(24, 30));
        // Back: `p x q` at 5-7 matches the hunk (x→y) nearer than its own lines.
        let back_head = format!("{}p\nx\nq\n{}p\nx\nq\n{}", filler(1, 4), filler(8, 20), filler(24, 30));
        for (head, at) in [(out_head, 3), (back_head, 10)] {
            let r = TestRepo::new();
            identity(&r);
            r.write("a.txt", &head);
            r.git(&["add", "."]);
            r.git(&["commit", "-q", "-m", "one"]);
            let lines: Vec<&str> = head.lines().collect();
            let with = |staged: bool, added: bool| {
                let mut l: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
                if staged {
                    l[21] = "y".into();
                }
                if added {
                    l.insert(at, thirty.trim_end().to_string());
                }
                l.iter().map(|s| format!("{s}\n")).collect::<String>()
            };
            r.write("a.txt", &with(true, false));
            r.git(&["add", "a.txt"]);
            r.write("a.txt", &with(true, true));
            let id = open(&env.api, &r).await;
            let before = RepoState::capture(&r);
            stash_part(&env.api, id, &r, &["a.txt"], "staged").await.unwrap();
            assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), with(false, true), "only the staged line goes");
            assert_eq!(r.git(&["diff", "--cached", "--name-only"]), "");
            pop_top(&env.api, id, &r).await;
            assert_eq!(RepoState::capture(&r), before, "and it comes back at its own line");
        }
    }

    /// The Unstaged part of a staged rename's new side whose old side is gone from disk: when
    /// narrowing fails and git's own stash is kept, the old side stays deleted (git's index part
    /// has it, as the push put it back first).
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unstaged_stash_keeps_a_staged_deletion_when_narrowing_fails() {
        let env = WriteEnv::new();
        let r = repo();
        r.write("a.txt", TEN);
        r.git(&["commit", "-q", "-am", "ten"]);
        r.git(&["mv", "a.txt", "moved.txt"]);
        r.write("moved.txt", &TEN.replace("8\n", "eight\n"));
        NARROW_FAILS.lock().unwrap().push(crate::platform::fs::canonicalize(r.path()).unwrap());
        let id = open(&env.api, &r).await;
        let held = RepoState::capture(&r);
        stash_part(&env.api, id, &r, &["moved.txt"], "unstaged").await.unwrap();
        assert_eq!(r.git(&["status", "--porcelain"]), "R  a.txt -> moved.txt");
        assert!(!r.path().join("a.txt").exists());
        pop_top(&env.api, id, &r).await;
        assert_eq!(RepoState::capture(&r), held);
    }

    /// A file's permission bits stay as they were through both parts' stashes and pops, and a
    /// staged mode change comes out of and back into a file with unstaged edits.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn both_parts_keep_a_files_permission_bits() {
        use std::os::unix::fs::PermissionsExt;
        let env = WriteEnv::new();
        let bits = |r: &TestRepo, f: &str| std::fs::metadata(r.path().join(f)).unwrap().permissions().mode() & 0o7777;
        let set = |r: &TestRepo, f: &str, m: u32| std::fs::set_permissions(r.path().join(f), std::fs::Permissions::from_mode(m)).unwrap();
        for part in ["staged", "unstaged"] {
            let r = mixed();
            set(&r, "m.txt", 0o600);
            let id = open(&env.api, &r).await;
            let before = RepoState::capture(&r);
            stash_part(&env.api, id, &r, &["m.txt"], part).await.unwrap();
            assert_eq!(bits(&r, "m.txt"), 0o600, "{part}");
            if part == "staged" {
                pop_top(&env.api, id, &r).await;
                assert_eq!(RepoState::capture(&r), before, "{part}");
            }
        }
        // A staged exec bit, with an unstaged edit: the edit stays, the bit goes and comes back.
        let r = mixed();
        set(&r, "s.txt", 0o750);
        r.git(&["add", "s.txt"]);
        r.write("s.txt", &TEN.replacen("1\n", "one\n", 1).replace("8\n", "eight\n"));
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        stash_part(&env.api, id, &r, &["s.txt"], "staged").await.unwrap();
        assert_eq!((bits(&r, "s.txt"), std::fs::read_to_string(r.path().join("s.txt")).unwrap()), (0o640, TEN.replace("8\n", "eight\n")));
        pop_top(&env.api, id, &r).await;
        assert_eq!(RepoState::capture(&r), before);
    }

    /// Each refusal says what's in the way.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_staged_stash_refusal_says_why() {
        let env = WriteEnv::new();
        let r = repo();
        r.write("new.txt", "new\n");
        r.git(&["add", "new.txt"]);
        r.write("new.txt", "new, edited\n");
        r.git(&["rm", "-q", "b.txt"]);
        r.write("b.txt", "back\n");
        let id = open(&env.api, &r).await;
        let held = RepoState::capture(&r);
        for (path, why) in [("new.txt", "new.txt is a new file with unstaged edits, which would have no file to stay in"), ("b.txt", "b.txt is staged as deleted, but there's a file there again")] {
            let e = stash_part(&env.api, id, &r, &[path], "staged").await.unwrap_err();
            assert_eq!(e.message, format!("Can't stash the staged changes alone: {why}. Stash its unstaged changes first, or stage them. Nothing was stashed"));
        }
        assert_eq!(RepoState::capture(&r), held);
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
