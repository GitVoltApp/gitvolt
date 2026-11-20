//! `git status --porcelain=v2 -z` parsing (see git-status(1), "Porcelain Format Version 2").

use crate::error::GbError;
use crate::git::{GitCli, GitInvocation};
use crate::snapshot::WipCache;
use crate::write::queue::RefreshPermit;
use futures_util::future::{BoxFuture, FutureExt, Shared};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub use gitvolt_graph::WipCounts;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Ordinary,
    Renamed,
    Unmerged,
    Untracked,
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: String,
    pub orig_path: Option<String>,
    pub index: char,
    pub worktree: char,
    pub kind: EntryKind,
    /// A submodule's own state (porcelain v2's `S<c><m><u>` field); `None` for any other path.
    pub submodule: Option<SubmoduleFlags>,
}

/// What `git status` says about a submodule: `S<c><m><u>`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SubmoduleFlags {
    /// `C`: checked out at another commit than the index records ("new commits").
    pub commit: bool,
    /// `M`: tracked changes inside it ("modified content").
    pub modified: bool,
    /// `U`: untracked files inside it ("untracked content").
    pub untracked: bool,
}

/// `S<c><m><u>` as flags; `N...` (not a submodule) is `None`.
fn submodule_flags(field: &str) -> Option<SubmoduleFlags> {
    let b = field.as_bytes();
    (b.len() == 4 && b[0] == b'S').then(|| SubmoduleFlags { commit: b[1] == b'C', modified: b[2] == b'M', untracked: b[3] == b'U' })
}

fn xy(field: &str) -> (char, char) {
    let mut c = field.chars();
    (c.next().unwrap_or('.'), c.next().unwrap_or('.'))
}

pub fn parse_porcelain_v2(out: &[u8]) -> Vec<StatusEntry> {
    let mut entries = Vec::new();
    let mut tokens = out.split(|b| *b == 0);
    while let Some(tok) = tokens.next() {
        if tok.is_empty() {
            continue;
        }
        let line = String::from_utf8_lossy(tok);
        let entry = match tok[0] {
            b'1' => {
                let f: Vec<&str> = line.splitn(9, ' ').collect();
                (f.len() == 9).then(|| {
                    let (x, y) = xy(f[1]);
                    StatusEntry { path: f[8].to_string(), orig_path: None, index: x, worktree: y, kind: EntryKind::Ordinary, submodule: submodule_flags(f[2]) }
                })
            }
            b'2' => {
                let f: Vec<&str> = line.splitn(10, ' ').collect();
                let orig = tokens.next().map(|o| String::from_utf8_lossy(o).into_owned());
                (f.len() == 10).then(|| {
                    let (x, y) = xy(f[1]);
                    StatusEntry { path: f[9].to_string(), orig_path: orig, index: x, worktree: y, kind: EntryKind::Renamed, submodule: submodule_flags(f[2]) }
                })
            }
            b'u' => {
                let f: Vec<&str> = line.splitn(11, ' ').collect();
                (f.len() == 11).then(|| {
                    let (x, y) = xy(f[1]);
                    StatusEntry { path: f[10].to_string(), orig_path: None, index: x, worktree: y, kind: EntryKind::Unmerged, submodule: submodule_flags(f[2]) }
                })
            }
            b'?' => Some(StatusEntry { path: line[2..].to_string(), orig_path: None, index: '?', worktree: '?', kind: EntryKind::Untracked, submodule: None }),
            b'!' => Some(StatusEntry { path: line[2..].to_string(), orig_path: None, index: '!', worktree: '!', kind: EntryKind::Ignored, submodule: None }),
            _ => None, // `# ...` headers
        };
        entries.extend(entry);
    }
    entries
}

/// Raw `git status --porcelain=v2 -z --untracked-files=all` output (the watcher digests it).
pub async fn status_raw(cli: &GitCli, worktree: &Path) -> Result<Vec<u8>, GbError> {
    let out = cli
        .run(GitInvocation::new(worktree, ["status", "--porcelain=v2", "-z", "--untracked-files=all"]))
        .await?;
    Ok(out.stdout)
}

/// The git dir of a linked worktree (its `.git` is a file); `None` for the main one.
fn linked_git_dir(worktree: &Path) -> Option<std::path::PathBuf> {
    let dot = worktree.join(".git");
    if dot.is_dir() {
        return None;
    }
    let text = std::fs::read_to_string(&dot).ok()?;
    let dir = text.lines().find_map(|l| l.strip_prefix("gitdir:")).map(str::trim)?;
    Some(worktree.join(dir))
}

/// A linked worktree whose checkout is still being written (`git worktree add`): its index
/// isn't there yet, so a status now would call every HEAD file deleted and every file already on
/// disk untracked. Such a worktree has no counts yet; its index appearing (watched in its git
/// dir) triggers the read.
pub fn checkout_pending(worktree: &Path) -> bool {
    linked_git_dir(worktree).is_some_and(|g| !g.join("index").exists() && gix::open(worktree).is_ok_and(|r| r.head_id().is_ok()))
}

/// A linked worktree's index is locked (a git command is writing it, often briefly): its counts
/// are left as they were, and the index's rename when the lock goes triggers the read.
pub fn index_locked(worktree: &Path) -> bool {
    linked_git_dir(worktree).is_some_and(|g| g.join("index.lock").exists())
}

/// A digest of the entries of `worktree`'s index (path, blob, mode, stage and flags), without
/// their stat info: what a refresh rewrites leaves it as it was, so an index change that keeps it
/// can't change what status says. `None` when the index can't be read.
pub(crate) fn index_digest(worktree: &Path) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let git_dir = linked_git_dir(worktree).unwrap_or_else(|| worktree.join(".git"));
    let index = gix::index::File::at(git_dir.join("index"), gix::hash::Kind::Sha1, false, Default::default()).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for e in index.entries() {
        e.path(&index).hash(&mut h);
        e.id.as_bytes().hash(&mut h);
        (e.mode.bits(), e.flags.bits()).hash(&mut h);
    }
    Some(h.finish())
}

/// A terminal's multi-step command (a rebase, `git am`, a cherry-pick or revert of several
/// commits) may be between two of its steps in this worktree: a refresh holding `index.lock`
/// just then would make its next step fail, so reads leave the index alone meanwhile.
pub(crate) fn sequence_running(worktree: &Path) -> bool {
    let git_dir = linked_git_dir(worktree).unwrap_or_else(|| worktree.join(".git"));
    ["rebase-merge", "rebase-apply", "sequencer"].iter().any(|d| git_dir.join(d).is_dir())
}

/// [`status_raw`], refreshing the index's stat info as `git status` does in a terminal while
/// `permit` allows (and no multi-step command is under way there, `sequence_running`). A
/// refresh that can't take `index.lock` at once is skipped by git, without an error.
pub(crate) async fn status_raw_refreshing(cli: &GitCli, worktree: &Path, permit: Option<&RefreshPermit<'_>>) -> Result<Vec<u8>, GbError> {
    let inv = GitInvocation::new(worktree, ["status", "--porcelain=v2", "-z", "--untracked-files=all"]);
    let inv = match permit {
        Some(p) if !sequence_running(worktree) => inv.refresh_index(p),
        _ => inv,
    };
    Ok(cli.run(inv).await?.stdout)
}

/// A status read anyone may wait on (`WipCache::read_status`): the raw output, and the index's
/// entries just before it (`index_digest`).
pub(crate) type SharedRead = Shared<BoxFuture<'static, Result<(Arc<Vec<u8>>, Option<u64>), GbError>>>;

/// The status read running per worktree, by its stamp (`WipCache::read_status`).
#[derive(Default)]
pub(crate) struct StatusReads(Mutex<HashMap<PathBuf, (u64, SharedRead)>>);

impl std::fmt::Debug for StatusReads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StatusReads")
    }
}

/// One `WipCache::read_status`.
pub(crate) struct StatusRead {
    /// The stamp taken when it started.
    pub stamp: u64,
    pub raw: Arc<Vec<u8>>,
    /// The index's entries just before it started (`index_digest`).
    pub index: Option<u64>,
    /// What the cache held before it (or, for a caller that shared it, what the first of them
    /// put).
    pub before: Option<crate::snapshot::WipEntry>,
}

impl WipCache {
    /// A refresh permit for this repository's reads, while no write runs (see `gated`).
    pub(crate) fn refresh_permit(&self) -> Option<RefreshPermit<'static>> {
        self.gate.as_ref().and_then(RefreshPermit::try_read)
    }

    /// The stamp of the last status read started: a read stamped above it starts later.
    pub(crate) fn now(&self) -> u64 {
        self.seq.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// `root`'s status for the cache's readers (graph builds, the watcher, a first `fileList`),
    /// kept in the cache (`put_raw`); `refresh`: refreshing the index if it may. One read
    /// serves everyone who asks while it runs, if it started after their `since` (a `now()`
    /// taken when their question arose: what changed before then, it sees). Otherwise a new
    /// one starts, for later callers to share.
    pub(crate) async fn read_status(&self, cli: &GitCli, root: &Path, since: u64, refresh: bool) -> Result<StatusRead, GbError> {
        let key = crate::platform::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let (stamp, read) = {
            let mut reads = self.reads.0.lock().expect("wip cache poisoned");
            match reads.get(&key) {
                Some((stamp, read)) if *stamp > since => (*stamp, read.clone()),
                _ => {
                    let stamp = self.stamp();
                    let permit = if refresh { self.refresh_permit() } else { None };
                    let (cli, root) = (cli.clone(), root.to_path_buf());
                    let read = async move {
                        let index = index_digest(&root);
                        let raw = status_raw_refreshing(&cli, &root, permit.as_ref()).await?;
                        Ok((Arc::new(raw), index))
                    }
                    .boxed()
                    .shared();
                    reads.insert(key.clone(), (stamp, read.clone()));
                    (stamp, read)
                }
            }
        };
        let res = read.await;
        {
            let mut reads = self.reads.0.lock().expect("wip cache poisoned");
            if reads.get(&key).is_some_and(|(s, _)| *s == stamp) {
                reads.remove(&key);
            }
        }
        let (raw, index) = res?;
        let before = self.put_raw(root, stamp, raw.clone());
        Ok(StatusRead { stamp, raw, index, before })
    }
}

pub async fn status(cli: &GitCli, worktree: &Path) -> Result<Vec<StatusEntry>, GbError> {
    Ok(parse_porcelain_v2(&status_raw(cli, worktree).await?))
}

/// Past this many bytes of paths, [`status_of`] reads the whole status: `status` takes its
/// pathspecs on the command line only (no `--pathspec-from-file`), which Windows caps at 32K.
const STATUS_OF_ARGV: usize = 8 * 1024;

/// [`status`] of `paths` only (literal pathspecs: a folder's with every file under it), so a
/// write on a few files costs what they do, not a scan of the whole worktree. A rename shows as
/// one only when both its paths are among them. Many paths: the whole status.
pub async fn status_of(cli: &GitCli, worktree: &Path, paths: &[String]) -> Result<Vec<StatusEntry>, GbError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    if paths.iter().map(|p| p.len() + 1).sum::<usize>() > STATUS_OF_ARGV {
        return status(cli, worktree).await;
    }
    let args = ["status", "--porcelain=v2", "-z", "--untracked-files=all", "--"].into_iter().map(String::from).chain(paths.iter().cloned());
    let out = cli.run(GitInvocation::new(worktree, args).env("GIT_LITERAL_PATHSPECS", "1")).await?;
    Ok(parse_porcelain_v2(&out.stdout))
}

pub fn summarize(entries: &[StatusEntry]) -> WipCounts {
    let mut c = WipCounts::default();
    for e in entries {
        match e.kind {
            EntryKind::Ignored => {}
            EntryKind::Unmerged => c.conflicted += 1,
            EntryKind::Untracked => c.added += 1,
            _ if e.index == 'D' || e.worktree == 'D' => c.deleted += 1,
            _ if e.index == 'A' => c.added += 1,
            EntryKind::Renamed if e.index == 'R' => c.renamed += 1,
            _ => c.modified += 1,
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use crate::log::CommandLog;
    use crate::testing::{isolated_git_env, TestRepo};
    use std::sync::Arc;

    fn cli() -> super::super::git::GitCli {
        super::super::git::GitCli::new(Arc::new(CommandLog::new(10))).with_env(isolated_git_env())
    }

    #[cfg(unix)] // file names Windows forbids (*, ?, :, newlines)
    #[tokio::test]
    async fn parses_weird_paths_and_renames() {
        let r = TestRepo::new();
        for i in 0..4 {
            r.commit(&format!("c{i}"));
        }
        r.write("file_0.txt", "changed\n");
        r.git(&["mv", "file_1.txt", "renamed file.txt"]);
        std::fs::remove_file(r.path().join("file_2.txt")).unwrap();
        r.write("dir with space/ünï.txt", "new\n");
        r.git(&["add", "dir with space/ünï.txt"]);
        r.write("a\nb.txt", "untracked\n");

        let mut entries = super::status(&cli(), r.path()).await.unwrap();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["a\nb.txt", "dir with space/ünï.txt", "file_0.txt", "file_2.txt", "renamed file.txt"]);

        let renamed = entries.iter().find(|e| e.path == "renamed file.txt").unwrap();
        assert_eq!(renamed.kind, super::EntryKind::Renamed);
        assert_eq!(renamed.orig_path.as_deref(), Some("file_1.txt"));
        assert_eq!(renamed.index, 'R');

        let untracked = entries.iter().find(|e| e.path == "a\nb.txt").unwrap();
        assert_eq!(untracked.kind, super::EntryKind::Untracked);

        assert_eq!(super::summarize(&entries), super::WipCounts { modified: 1, added: 2, deleted: 1, renamed: 1, conflicted: 0 });
    }

    #[tokio::test]
    async fn counts_conflicts() {
        let r = TestRepo::new();
        r.commit("base");
        r.switch_new("other");
        r.write("file_0.txt", "theirs\n");
        r.git(&["commit", "-qam", "theirs"]);
        r.switch("main");
        r.write("file_0.txt", "ours\n");
        r.git(&["commit", "-qam", "ours"]);
        assert!(r.try_git(&["merge", "-q", "other"]).is_err());
        let entries = super::status(&cli(), r.path()).await.unwrap();
        assert_eq!(super::summarize(&entries).conflicted, 1);
        assert_eq!(entries[0].kind, super::EntryKind::Unmerged);
    }

    #[tokio::test]
    async fn clean_tree_is_empty() {
        let r = TestRepo::new();
        r.commit("c");
        assert!(super::summarize(&super::status(&cli(), r.path()).await.unwrap()).is_empty());
    }

    /// A plain `git status` refreshes stat info and rewrites `.git/index` whenever it can take
    /// the (optional) lock, even though nothing tracked actually changed. A read without a
    /// refresh permit runs with `GIT_OPTIONAL_LOCKS=0` (see git.rs), so `status()` here must
    /// leave the index file untouched byte for byte, not just logically unchanged.
    #[tokio::test]
    async fn status_without_a_permit_does_not_rewrite_the_index() {
        let r = TestRepo::new();
        r.commit("c");
        let index_path = r.path().join(".git/index");
        let before_bytes = std::fs::read(&index_path).unwrap();
        let before_mtime = std::fs::metadata(&index_path).unwrap().modified().unwrap();

        // Touch a tracked file's mtime, without touching its content, far enough in the future
        // to defeat racy-git's same-second heuristic. A plain `git status` would re-stat it,
        // find the content unchanged, and still rewrite `.git/index` with the refreshed stat
        // info (a write GitVolt must never make).
        let tracked = r.path().join("file_0.txt");
        let f = std::fs::File::options().write(true).open(&tracked).unwrap();
        f.set_modified(before_mtime + std::time::Duration::from_secs(120)).unwrap();

        super::status(&cli(), r.path()).await.unwrap();

        let after_bytes = std::fs::read(&index_path).unwrap();
        let after_mtime = std::fs::metadata(&index_path).unwrap().modified().unwrap();
        assert_eq!(before_bytes, after_bytes, ".git/index bytes must be unchanged by status()");
        assert_eq!(before_mtime, after_mtime, ".git/index mtime must be unchanged by status()");
    }

    /// Moves `file_0.txt`'s mtime back a few minutes, its content untouched: stat-dirty, and
    /// clean again once a refresh records the new mtime (in the past, so never racy).
    fn stat_dirty(r: &TestRepo) {
        let f = std::fs::File::options().write(true).open(r.path().join("file_0.txt")).unwrap();
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(300)).unwrap();
    }

    fn permit(gate: &Arc<tokio::sync::RwLock<()>>) -> crate::write::queue::RefreshPermit<'static> {
        crate::write::queue::RefreshPermit::try_read(gate).unwrap()
    }

    /// With a permit, status writes back the stat info it re-hashed, as in a terminal: what git
    /// compares by stat alone (`diff-files`) is clean afterwards. Only the stat info changes.
    #[tokio::test]
    async fn a_permitted_status_refreshes_the_index() {
        let r = TestRepo::new();
        r.commit("c");
        stat_dirty(&r);
        assert_eq!(r.git(&["diff-files", "--name-only"]), "file_0.txt", "stat-dirty");
        let entries = r.git(&["ls-files", "-s"]);
        let gate = Arc::default();
        let raw = super::status_raw_refreshing(&cli(), r.path(), Some(&permit(&gate))).await.unwrap();
        assert!(super::parse_porcelain_v2(&raw).is_empty(), "nothing changed");
        assert_eq!(r.git(&["diff-files", "--name-only"]), "", "refreshed");
        assert_eq!(r.git(&["ls-files", "-s"]), entries, "the entries are as they were");
    }

    /// `index.lock` held (a terminal's `git commit` part-way): git skips the refresh, and the
    /// status still answers, without an error.
    #[tokio::test]
    async fn a_refresh_is_skipped_without_error_while_index_lock_is_held() {
        let r = TestRepo::new();
        r.commit("c");
        stat_dirty(&r);
        let lock = r.path().join(".git/index.lock");
        std::fs::write(&lock, b"").unwrap();
        let index = std::fs::read(r.path().join(".git/index")).unwrap();
        let gate = Arc::default();
        let raw = super::status_raw_refreshing(&cli(), r.path(), Some(&permit(&gate))).await.unwrap();
        assert!(super::parse_porcelain_v2(&raw).is_empty());
        assert_eq!(std::fs::read(r.path().join(".git/index")).unwrap(), index, "not refreshed");
        assert!(lock.exists(), "the lock is its owner's");
        std::fs::remove_file(&lock).unwrap();
    }

    /// No permit while a write holds the repository (it waits for a refresh already running),
    /// and no refresh in a worktree mid-rebase, where a terminal's next step may need the lock.
    #[tokio::test]
    async fn no_refresh_while_a_write_runs_or_a_sequence_is_under_way() {
        let writes = crate::write::queue::RepoWrites::new(|_, _| {});
        let reading = crate::write::queue::RefreshPermit::try_read(&writes.refresh_gate);
        assert!(reading.is_some());
        let write = writes.acquire();
        tokio::pin!(write);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(50), &mut write).await.is_err(), "a write waits for a refresh running");
        drop(reading);
        let guard = write.await;
        assert!(crate::write::queue::RefreshPermit::try_read(&writes.refresh_gate).is_none(), "none while a write runs");
        drop(guard);
        assert!(crate::write::queue::RefreshPermit::try_read(&writes.refresh_gate).is_some());

        let r = TestRepo::new();
        r.commit("c");
        stat_dirty(&r);
        std::fs::create_dir(r.path().join(".git/rebase-merge")).unwrap();
        let index = std::fs::read(r.path().join(".git/index")).unwrap();
        let gate = Arc::default();
        super::status_raw_refreshing(&cli(), r.path(), Some(&permit(&gate))).await.unwrap();
        assert_eq!(std::fs::read(r.path().join(".git/index")).unwrap(), index, "not refreshed mid-rebase");
    }

    /// A refresh leaves the entries' digest as it was; staging changes it.
    #[tokio::test]
    async fn the_index_digest_ignores_stat_info() {
        let r = TestRepo::new();
        r.commit("c");
        let before = super::index_digest(r.path()).unwrap();
        stat_dirty(&r);
        let gate = Arc::default();
        super::status_raw_refreshing(&cli(), r.path(), Some(&permit(&gate))).await.unwrap();
        assert_eq!(super::index_digest(r.path()), Some(before), "refreshed: the same entries");
        r.write("file_0.txt", "staged\n");
        r.git(&["add", "file_0.txt"]);
        assert_ne!(super::index_digest(r.path()), Some(before));
    }

    /// Reads of one worktree asked for at once share one `git status`; a read already running
    /// that started before the caller's question doesn't serve it.
    #[tokio::test]
    async fn reads_of_a_worktree_share_a_status_started_late_enough() {
        let r = TestRepo::new();
        r.commit("c");
        let log = Arc::new(CommandLog::new(100));
        let cli = super::super::git::GitCli::new(log.clone()).with_env(isolated_git_env());
        let statuses = || log.entries().iter().filter(|e| e.args.iter().any(|a| a == "status")).count();
        let wip = crate::snapshot::WipCache::default();
        let since = wip.now();
        let (a, b) = futures_util::future::join(wip.read_status(&cli, r.path(), since, true), wip.read_status(&cli, r.path(), since, true)).await;
        assert_eq!(a.unwrap().stamp, b.unwrap().stamp, "the same read");
        assert_eq!(statuses(), 1);
        let first = wip.read_status(&cli, r.path(), wip.now(), true);
        let later = async {
            tokio::task::yield_now().await;
            // Asked after the first read started: whatever changed meanwhile must be seen.
            wip.read_status(&cli, r.path(), wip.now(), true).await
        };
        let (a, b) = futures_util::future::join(first, later).await;
        assert!(a.unwrap().stamp < b.unwrap().stamp, "a read of its own");
        assert_eq!(statuses(), 3);
        assert!(wip.get(r.path()).is_some(), "kept in the cache");
    }

    #[test]
    fn ignores_headers_and_handles_unmerged_record() {
        let h = "0".repeat(40);
        let raw = format!("# branch.oid {h}\0# branch.head main\0u UU N... 100644 100644 100644 100644 {h} {h} {h} x y.txt\0");
        let e = super::parse_porcelain_v2(raw.as_bytes());
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].path, "x y.txt");
        assert_eq!(e[0].kind, super::EntryKind::Unmerged);
        assert_eq!(e[0].submodule, None);
    }

    #[test]
    fn reads_a_submodules_state_field() {
        let h = "0".repeat(40);
        let raw = format!("1 .M SCMU 160000 160000 160000 {h} {h} vendor/lib\x001 .M S.M. 160000 160000 160000 {h} {h} vendor/other\x001 .M N... 100644 100644 100644 {h} {h} a.txt\0");
        let e = super::parse_porcelain_v2(raw.as_bytes());
        assert_eq!(e[0].submodule, Some(super::SubmoduleFlags { commit: true, modified: true, untracked: true }));
        assert_eq!(e[1].submodule, Some(super::SubmoduleFlags { commit: false, modified: true, untracked: false }));
        assert_eq!(e[2].submodule, None);
    }
}
