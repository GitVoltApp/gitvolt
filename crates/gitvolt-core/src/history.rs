//! File History and Blame: read-only `git log --follow` and `git blame`. Neither
//! writes to the repository (the API's read audit dispatches both). An open history's pages
//! come from one streaming walk (`HistoryWalks`).

use crate::api::Api;
use crate::blob::check_relative;
use crate::error::{GbError, GbErrorKind};
use crate::git::{GitInvocation, GitStream, LOCAL_TIMEOUT};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ts_rs::TS;

/// The most rows one page may ask for (the UI asks for 200).
pub const HISTORY_PAGE_MAX: u32 = 1000;
/// One record per commit: a record separator, then NUL-separated fields.
const LOG_FORMAT: &str = "--format=%x1e%H%x00%P%x00%an%x00%ae%x00%at%x00%s";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileHistoryRow {
    pub sha: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    #[ts(type = "number")]
    pub time: i64,
    pub summary: String,
    /// The file's path at this commit (a rename's new path).
    pub path: String,
    /// `A`, `M`, `D`, `R`, `C` or `T`; empty for a commit that shows no change of its own (a merge).
    pub status: String,
    /// A rename's or copy's source: the file's path in the commits below this one.
    pub old_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileHistoryPage {
    pub rows: Vec<FileHistoryRow>,
    /// More rows follow these.
    pub more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BlameHunk {
    pub sha: String,
    /// The first line (1-based) in the file at the blamed commit.
    pub start: u32,
    pub lines: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BlameCommit {
    pub sha: String,
    pub author: String,
    pub email: String,
    #[ts(type = "number")]
    pub time: i64,
    pub summary: String,
    /// `git blame` stopped here (the history's root, or a shallow clone's edge).
    pub boundary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BlamePayload {
    pub hunks: Vec<BlameHunk>,
    pub commits: Vec<BlameCommit>,
}

fn option_like(rev: &str) -> GbError {
    GbError::new(GbErrorKind::InvalidInput, format!("not a revision: {rev}"))
}

/// How long an open history's walk waits for its next page before it's stopped.
pub(crate) const WALK_IDLE: Duration = Duration::from_secs(60);
/// The most walks kept between pages at once (one per open File History); the least recently
/// used goes first.
const WALKS_MAX: usize = 8;
/// What `fill` reads from git at a time.
const READ_CHUNK: usize = 64 * 1024;

/// One open history: the arguments as the UI sends them, so `fileHistoryClose` finds it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WalkKey {
    repo: u32,
    worktree: String,
    path: String,
    rev: Option<String>,
}

/// One `git log --follow`, read a page at a time. `--skip` loses `--follow`'s rename and a new
/// walk per page re-walked every row above it (16.6 ms at row 1,800, against 3.3 ms for the
/// first page), so a history's pages come from one walk, which git runs only as far ahead as
/// the pipe holds.
struct HistoryWalk {
    stream: GitStream,
    /// Bytes read but not parsed: the record still coming in.
    tail: Vec<u8>,
    /// The followed path where the walk is (`parse_follow_records`).
    cur: String,
    rows: VecDeque<FileHistoryRow>,
    /// Rows handed out: the `skip` of the page it serves next.
    served: usize,
    done: bool,
}

impl HistoryWalk {
    async fn start(api: &Api, root: &std::path::Path, path: &str, rev: &str) -> Result<Self, GbError> {
        let args = ["log", "--no-show-signature", "--follow", "-M", "--name-status", "-z", LOG_FORMAT, "--end-of-options", rev, "--", path];
        let stream = api.cli.stream(GitInvocation::new(root, args).env("GIT_LITERAL_PATHSPECS", "1")).await?;
        Ok(Self { stream, tail: Vec::new(), cur: path.to_string(), rows: VecDeque::new(), served: 0, done: false })
    }

    /// Reads until `n` rows are in or the walk has ended (then git's failure, if it failed).
    async fn fill(&mut self, n: usize) -> Result<(), GbError> {
        let mut buf = vec![0u8; READ_CHUNK];
        while self.rows.len() < n && !self.done {
            let k = self.stream.read(&mut buf).await.map_err(|e| GbError::new(GbErrorKind::Io, format!("git log: {e}")))?;
            if k == 0 {
                self.done = true;
                let last = std::mem::take(&mut self.tail);
                self.rows.extend(parse_follow_records(&last, &mut self.cur));
                self.stream.finish().await?;
                break;
            }
            self.tail.extend_from_slice(&buf[..k]);
            // Every record before the last separator is whole.
            if let Some(cut) = self.tail.iter().rposition(|&b| b == 0x1e).filter(|&c| c > 0) {
                let rest = self.tail.split_off(cut);
                let whole = std::mem::replace(&mut self.tail, rest);
                self.rows.extend(parse_follow_records(&whole, &mut self.cur));
            }
        }
        Ok(())
    }

    /// The page after the rows served: up to `limit` rows, and whether more follow.
    async fn page(&mut self, limit: usize) -> Result<FileHistoryPage, GbError> {
        self.fill(limit + 1).await?;
        let rows: Vec<_> = self.rows.drain(..limit.min(self.rows.len())).collect();
        self.served += rows.len();
        Ok(FileHistoryPage { rows, more: !self.rows.is_empty() })
    }
}

struct Parked {
    walk: HistoryWalk,
    /// Which park this is: an idle timer stops the walk only if it wasn't used since.
    ticket: u64,
    at: Instant,
}

/// The open histories' walks, parked between pages. A walk ends with its history
/// (`fileHistoryClose`), after `WALK_IDLE` unused, or when `WALKS_MAX` newer ones push it out.
#[derive(Clone)]
pub(crate) struct HistoryWalks {
    parked: Arc<Mutex<HashMap<WalkKey, Parked>>>,
    tickets: Arc<AtomicU64>,
    idle: Duration,
}

impl Default for HistoryWalks {
    fn default() -> Self {
        Self::with_idle(WALK_IDLE)
    }
}

impl HistoryWalks {
    fn with_idle(idle: Duration) -> Self {
        Self { parked: Arc::default(), tickets: Arc::default(), idle }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<WalkKey, Parked>> {
        self.parked.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `key`'s walk, if it's at row `at`; one elsewhere is stopped.
    fn take(&self, key: &WalkKey, at: usize) -> Option<HistoryWalk> {
        let parked = self.lock().remove(key)?;
        (parked.walk.served == at).then_some(parked.walk)
    }

    fn park(&self, key: WalkKey, walk: HistoryWalk) {
        let ticket = self.tickets.fetch_add(1, Ordering::Relaxed);
        let evicted = {
            let mut map = self.lock();
            map.insert(key.clone(), Parked { walk, ticket, at: Instant::now() });
            let oldest = (map.len() > WALKS_MAX).then(|| map.iter().min_by_key(|(_, p)| p.at).map(|(k, _)| k.clone())).flatten();
            oldest.and_then(|k| map.remove(&k))
        };
        drop(evicted);
        let parked = Arc::downgrade(&self.parked);
        let idle = self.idle;
        tokio::spawn(async move {
            tokio::time::sleep(idle).await;
            let Some(parked) = parked.upgrade() else { return };
            let stale = {
                let mut map = parked.lock().unwrap_or_else(|e| e.into_inner());
                if map.get(&key).is_some_and(|p| p.ticket == ticket) { map.remove(&key) } else { None }
            };
            drop(stale);
        });
    }

    /// Stops the walk of the history opened with these arguments.
    pub(crate) fn close(&self, repo: u32, worktree: String, path: String, rev: Option<String>) {
        let gone = self.lock().remove(&WalkKey { repo, worktree, path, rev });
        drop(gone);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }
}

/// One page of `path`'s history from `rev` (the worktree's HEAD when `None`), newest first.
/// `-M` so a user's `diff.renames=false` can't hide the rename the paths follow;
/// literal pathspecs so `[`, `*` or `:` in a name are just characters. The first page starts a
/// walk (a reopened history sees the branch as it is now); the next ones go on with it, or with
/// a new one cut at `skip` when it's gone (closed, idle, another page asked for).
pub(crate) async fn file_history(api: &Api, repo: u32, worktree: &str, path: String, rev: Option<String>, skip: u32, limit: u32) -> Result<FileHistoryPage, GbError> {
    check_relative(&path)?;
    if let Some(r) = rev.as_deref().filter(|r| r.starts_with('-')) {
        return Err(option_like(r));
    }
    let limit = limit.clamp(1, HISTORY_PAGE_MAX) as usize;
    let skip = skip as usize;
    let key = WalkKey { repo, worktree: worktree.to_string(), path, rev };
    let parked = if skip > 0 { api.history_walks.take(&key, skip) } else { None };
    let mut walk = match parked {
        Some(w) => w,
        None => {
            let h = api.handle(repo)?;
            let root = api.worktree_dir(&h, worktree).await?;
            let mut w = HistoryWalk::start(api, &root, &key.path, key.rev.as_deref().unwrap_or("HEAD")).await?;
            tokio::time::timeout(LOCAL_TIMEOUT, w.fill(skip)).await.map_err(|_| walk_timeout())??;
            let n = skip.min(w.rows.len());
            w.rows.drain(..n);
            w.served = skip;
            w
        }
    };
    let page = tokio::time::timeout(LOCAL_TIMEOUT, walk.page(limit)).await.map_err(|_| walk_timeout())??;
    if page.more {
        api.history_walks.park(key, walk);
    }
    Ok(page)
}

fn walk_timeout() -> GbError {
    GbError::other(format!("git log timed out after {}s", LOCAL_TIMEOUT.as_secs()))
}

/// `LOG_FORMAT` records with `--name-status -z`: after the fields (and git's `\n`), the followed
/// file's `<status>\0<path>\0`, or `<R|C><score>\0<old>\0<new>\0`; a merge has none. `path` is
/// the file's path where the walk starts, carried down to records with no change of their own.
#[cfg(test)]
pub(crate) fn parse_follow_log(out: &[u8], path: &str) -> Vec<FileHistoryRow> {
    parse_follow_records(out, &mut path.to_string())
}

/// `parse_follow_log` for whole records read so far, `cur` carried from one call to the next.
fn parse_follow_records(out: &[u8], cur: &mut String) -> Vec<FileHistoryRow> {
    let text = String::from_utf8_lossy(out);
    let mut rows = Vec::new();
    for rec in text.split('\x1e').filter(|r| !r.trim().is_empty()) {
        let mut f = rec.split('\0');
        let (Some(sha), Some(parents), Some(author), Some(email), Some(time), Some(summary)) = (f.next(), f.next(), f.next(), f.next(), f.next(), f.next()) else {
            continue;
        };
        let change: Vec<&str> = f.map(|s| s.trim_start_matches('\n')).filter(|s| !s.is_empty()).collect();
        let (status, here, old_path) = match change.as_slice() {
            [st, old, new, ..] if st.starts_with('R') || st.starts_with('C') => (st[..1].to_string(), new.to_string(), Some(old.to_string())),
            [st, p, ..] => (st[..1].to_string(), p.to_string(), None),
            _ => (String::new(), cur.clone(), None),
        };
        *cur = old_path.clone().unwrap_or_else(|| here.clone());
        rows.push(FileHistoryRow {
            sha: sha.to_string(),
            parents: parents.split(' ').filter(|p| !p.is_empty()).map(String::from).collect(),
            author: author.to_string(),
            email: email.to_string(),
            time: time.parse().unwrap_or(0),
            summary: summary.to_string(),
            path: here,
            status,
            old_path,
        });
    }
    rows
}

/// `path` at `rev`, line by line. The UI caches it per (rev, path). `--incremental` names each
/// line group once (`--porcelain` repeats a header per line and every line's text: 4.5 MB for a
/// 50,000-line file, against 11 KB).
pub(crate) async fn blame(api: &Api, repo: u32, worktree: &str, rev: String, path: String) -> Result<BlamePayload, GbError> {
    check_relative(&path)?;
    if rev.starts_with('-') {
        return Err(option_like(&rev));
    }
    let h = api.handle(repo)?;
    let root = api.worktree_dir(&h, worktree).await?;
    let out = api.cli.run(GitInvocation::new(&root, ["blame", "--incremental", rev.as_str(), "--", path.as_str()])).await?;
    Ok(parse_blame_incremental(&out.stdout))
}

/// One `--incremental` entry: `<sha> <orig> <final> <count>`, its commit's `key value` lines the
/// first time the commit comes up, and `filename <path>` last.
struct BlameEntry {
    sha: String,
    orig: u32,
    start: u32,
    lines: u32,
    file: String,
}

/// `git blame --incremental`: entries in the order blame settles them, which `--porcelain` would
/// print in line order after merging the neighbours from one commit's file that run on in both
/// files (git's `blame_coalesce`). Sorted and merged the same way, they're `parse_blame`'s
/// result for the same blame, commits in the order their first lines come.
pub(crate) fn parse_blame_incremental(out: &[u8]) -> BlamePayload {
    let text = String::from_utf8_lossy(out);
    let mut entries: Vec<BlameEntry> = Vec::new();
    let mut info: HashMap<String, BlameCommit> = HashMap::new();
    let mut open: Option<BlameEntry> = None;
    for line in text.split('\n') {
        let mut parts = line.split(' ');
        let first = parts.next().unwrap_or("");
        if open.is_none() {
            let nums: Vec<u32> = parts.filter_map(|p| p.parse().ok()).collect();
            if (first.len() == 40 || first.len() == 64) && first.bytes().all(|b| b.is_ascii_hexdigit()) && nums.len() == 3 {
                open = Some(BlameEntry { sha: first.to_string(), orig: nums[0], start: nums[1], lines: nums[2], file: String::new() });
            }
            continue;
        }
        let Some(e) = open.as_mut() else { continue };
        let c = info.entry(e.sha.clone()).or_insert_with(|| BlameCommit { sha: e.sha.clone(), ..BlameCommit::default() });
        if line == "boundary" {
            c.boundary = true;
            continue;
        }
        match line.split_once(' ') {
            Some(("author", v)) => c.author = v.to_string(),
            Some(("author-mail", v)) => c.email = v.trim_start_matches('<').trim_end_matches('>').to_string(),
            Some(("author-time", v)) => c.time = v.parse().unwrap_or(0),
            Some(("summary", v)) => c.summary = v.to_string(),
            Some(("filename", v)) => {
                e.file = v.to_string();
                entries.extend(open.take());
            }
            _ => {}
        }
    }
    entries.sort_by_key(|e| e.start);
    let mut merged: Vec<BlameEntry> = Vec::with_capacity(entries.len());
    for e in entries {
        match merged.last_mut() {
            Some(m) if m.sha == e.sha && m.file == e.file && m.orig + m.lines == e.orig && m.start + m.lines == e.start => m.lines += e.lines,
            _ => merged.push(e),
        }
    }
    let mut commits = Vec::new();
    let hunks = merged
        .into_iter()
        .map(|e| {
            if let Some(c) = info.remove(&e.sha) {
                commits.push(c);
            }
            BlameHunk { sha: e.sha, start: e.start, lines: e.lines }
        })
        .collect();
    BlamePayload { hunks, commits }
}

/// `git blame --porcelain`: a header `<sha> <orig> <final> <count>` opens each group (`<count>`
/// only on a group's first line), followed by `key value` lines the first time a commit is seen,
/// and a TAB-prefixed content line per line. The reference `parse_blame_incremental` is held to.
#[cfg(test)]
pub(crate) fn parse_blame(out: &[u8]) -> BlamePayload {
    let text = String::from_utf8_lossy(out);
    let (mut hunks, mut commits) = (Vec::new(), Vec::<BlameCommit>::new());
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut current: Option<usize> = None;
    for line in text.split('\n') {
        if line.starts_with('\t') {
            continue;
        }
        let mut parts = line.split(' ');
        let first = parts.next().unwrap_or("");
        if (first.len() == 40 || first.len() == 64) && first.bytes().all(|b| b.is_ascii_hexdigit()) {
            let nums: Vec<u32> = parts.filter_map(|p| p.parse().ok()).collect();
            let ci = *index.entry(first.to_string()).or_insert_with(|| {
                commits.push(BlameCommit { sha: first.to_string(), ..BlameCommit::default() });
                commits.len() - 1
            });
            current = Some(ci);
            if let (Some(&start), Some(&lines)) = (nums.get(1), nums.get(2)) {
                hunks.push(BlameHunk { sha: first.to_string(), start, lines });
            }
            continue;
        }
        let Some(ci) = current else { continue };
        let c = &mut commits[ci];
        if line == "boundary" {
            c.boundary = true;
            continue;
        }
        match line.split_once(' ') {
            Some(("author", v)) => c.author = v.to_string(),
            Some(("author-mail", v)) => c.email = v.trim_start_matches('<').trim_end_matches('>').to_string(),
            Some(("author-time", v)) => c.time = v.parse().unwrap_or(0),
            Some(("summary", v)) => c.summary = v.to_string(),
            _ => {}
        }
    }
    BlamePayload { hunks, commits }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::write::{open, send, wt, WriteEnv};
    use crate::testing::{fixtures, TestRepo};
    use serde_json::{json, Value};

    async fn page(env: &WriteEnv, id: u32, r: &TestRepo, path: &str, rev: Option<&str>, skip: u32, limit: u32) -> Value {
        send(&env.api, json!({"method": "fileHistory", "params": {"repo": id, "worktree": wt(r), "path": path, "rev": rev, "skip": skip, "limit": limit}})).await.unwrap()
    }
    fn col(p: &Value, field: &str) -> Vec<String> {
        p["rows"].as_array().unwrap().iter().map(|r| r[field].as_str().unwrap_or("").to_string()).collect()
    }
    fn sha_of(r: &TestRepo, subject: &str) -> String {
        r.git(&["log", "--all", "--format=%H", "-F", &format!("--grep={subject}"), "-1"])
    }

    #[tokio::test]
    async fn history_follows_the_rename_with_each_rows_path() {
        let r = TestRepo::new();
        fixtures::file_history(&r);
        let env = WriteEnv::new();
        let id = open(&env.api, &r).await;
        let p = page(&env, id, &r, "src/story.txt", None, 0, 200).await;
        assert_eq!(col(&p, "summary"), ["Sharpen the opening", "Move the story under src", "Add the middle", "Start the story"]);
        assert_eq!(col(&p, "path"), ["src/story.txt", "src/story.txt", "story.txt", "story.txt"]);
        assert_eq!(col(&p, "status"), ["M", "R", "M", "A"]);
        assert_eq!(p["rows"][1]["oldPath"], "story.txt");
        assert_eq!(p["rows"][0]["author"], "Ada Lovelace");
        assert_eq!(p["rows"][0]["email"], "ada@example.com");
        assert_eq!(p["rows"][2]["parents"].as_array().unwrap().len(), 1);
        assert_eq!(p["more"], false);
    }

    /// Git's `--skip` drops `--follow`'s rename; pages are cut here instead.
    #[tokio::test]
    async fn pages_cross_the_rename_and_say_when_theres_more() {
        let r = TestRepo::new();
        fixtures::file_history(&r);
        let env = WriteEnv::new();
        let id = open(&env.api, &r).await;
        let a = page(&env, id, &r, "src/story.txt", None, 0, 2).await;
        assert_eq!(col(&a, "summary"), ["Sharpen the opening", "Move the story under src"]);
        assert_eq!(a["more"], true);
        let b = page(&env, id, &r, "src/story.txt", None, 2, 2).await;
        assert_eq!(col(&b, "summary"), ["Add the middle", "Start the story"]);
        assert_eq!(col(&b, "path"), ["story.txt", "story.txt"]);
        assert_eq!(b["more"], false);
        let c = page(&env, id, &r, "src/story.txt", None, 9, 2).await;
        assert_eq!(c["rows"].as_array().unwrap().len(), 0);
        assert_eq!(c["more"], false);
    }

    #[tokio::test]
    async fn rev_starts_the_walk_at_that_commit() {
        let r = TestRepo::new();
        fixtures::file_history(&r);
        let env = WriteEnv::new();
        let id = open(&env.api, &r).await;
        let middle = sha_of(&r, "Add the middle");
        let p = page(&env, id, &r, "story.txt", Some(&middle), 0, 200).await;
        assert_eq!(col(&p, "summary"), ["Add the middle", "Start the story"]);
    }

    /// `[draft]` is a glob class that would also match `notes d.txt`.
    #[tokio::test]
    async fn a_path_with_glob_characters_is_taken_literally() {
        let r = TestRepo::new();
        r.write("notes [draft].txt", "1\n");
        r.commit_all_as("Draft one", "Ada Lovelace", "ada@example.com");
        r.write("notes d.txt", "d\n");
        r.commit_all_as("Other notes", "Ada Lovelace", "ada@example.com");
        r.write("notes [draft].txt", "2\n");
        r.commit_all_as("Draft two", "Ada Lovelace", "ada@example.com");
        let env = WriteEnv::new();
        let id = open(&env.api, &r).await;
        let p = page(&env, id, &r, "notes [draft].txt", None, 0, 200).await;
        assert_eq!(col(&p, "summary"), ["Draft two", "Draft one"]);
        let head = r.git(&["rev-parse", "HEAD"]);
        let b = send(&env.api, json!({"method": "blame", "params": {"repo": id, "worktree": wt(&r), "rev": head, "path": "notes [draft].txt"}})).await.unwrap();
        assert_eq!(b["hunks"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_rev_or_path_that_looks_like_an_option_is_refused() {
        let r = TestRepo::new();
        fixtures::file_history(&r);
        let env = WriteEnv::new();
        let id = open(&env.api, &r).await;
        let e = send(&env.api, json!({"method": "fileHistory", "params": {"repo": id, "worktree": wt(&r), "path": "src/story.txt", "rev": "--all", "skip": 0, "limit": 5}})).await.unwrap_err();
        assert_eq!(e.kind, crate::error::GbErrorKind::InvalidInput);
        let e = send(&env.api, json!({"method": "blame", "params": {"repo": id, "worktree": wt(&r), "rev": "HEAD", "path": "../x"}})).await.unwrap_err();
        assert_eq!(e.kind, crate::error::GbErrorKind::InvalidInput);
    }

    /// Each line group carries its commit; hunks are in file order.
    #[tokio::test]
    async fn blame_groups_lines_by_commit_with_their_authors() {
        let r = TestRepo::new();
        fixtures::file_history(&r);
        let env = WriteEnv::new();
        let id = open(&env.api, &r).await;
        let head = r.git(&["rev-parse", "HEAD"]);
        let b = send(&env.api, json!({"method": "blame", "params": {"repo": id, "worktree": wt(&r), "rev": head, "path": "src/story.txt"}})).await.unwrap();
        let summary = |sha: &str| b["commits"].as_array().unwrap().iter().find(|c| c["sha"] == sha).unwrap()["summary"].as_str().unwrap().to_string();
        let hunks: Vec<(String, u64, u64)> = b["hunks"].as_array().unwrap().iter().map(|h| (summary(h["sha"].as_str().unwrap()), h["start"].as_u64().unwrap(), h["lines"].as_u64().unwrap())).collect();
        let s = |x: &str| x.to_string();
        assert_eq!(hunks, [(s("Sharpen the opening"), 1, 1), (s("Start the story"), 2, 1), (s("Add the middle"), 3, 2), (s("Start the story"), 5, 1), (s("Move the story under src"), 6, 1), (s("Start the story"), 7, 2)]);
        assert_eq!(b["commits"].as_array().unwrap().len(), 4);
        let start = b["commits"].as_array().unwrap().iter().find(|c| c["summary"] == "Start the story").unwrap();
        assert_eq!(start["author"], "Ada Lovelace");
        assert_eq!(start["email"], "ada@example.com");
        assert!(start["time"].as_i64().unwrap() > 0);
    }

    /// An older commit is blamed at its own path.
    #[tokio::test]
    async fn blame_at_an_older_commit_reads_its_path() {
        let r = TestRepo::new();
        fixtures::file_history(&r);
        let env = WriteEnv::new();
        let id = open(&env.api, &r).await;
        let middle = sha_of(&r, "Add the middle");
        let b = send(&env.api, json!({"method": "blame", "params": {"repo": id, "worktree": wt(&r), "rev": middle, "path": "story.txt"}})).await.unwrap();
        let spans: Vec<(u64, u64)> = b["hunks"].as_array().unwrap().iter().map(|h| (h["start"].as_u64().unwrap(), h["lines"].as_u64().unwrap())).collect();
        assert_eq!(spans, [(1, 2), (3, 2), (5, 4)]);
    }

    /// A file renamed twice, edited on a branch merged back between edits on main: the walk
    /// crosses both renames and the merge.
    fn renamed_and_merged(r: &TestRepo) {
        let lines = |tag: &str| (1..=40).map(|i| format!("line {i} {tag}\n")).collect::<String>();
        r.write("a.txt", &lines("one"));
        r.commit_all_as("Start a", "Ada Lovelace", "ada@example.com");
        for i in 0..5 {
            r.write("a.txt", &lines(&format!("a{i}")));
            r.commit_all_as(&format!("Edit a {i}"), "Ada Lovelace", "ada@example.com");
        }
        r.git(&["mv", "a.txt", "b.txt"]);
        r.commit_all_as("Rename a to b", "Grace Hopper", "grace@example.com");
        r.switch_new("side");
        let mut side = lines("b-side");
        side.push_str("side tail\n");
        r.write("b.txt", &side);
        r.commit_all_as("Side edit of b", "Grace Hopper", "grace@example.com");
        r.switch("main");
        r.write("other.txt", "unrelated\n");
        r.commit_all_as("Unrelated", "Lars Tidemann", "lars@example.com");
        r.merge("side", "Merge side");
        for i in 0..4 {
            let mut t = std::fs::read_to_string(r.path().join("b.txt")).unwrap();
            t.push_str(&format!("main tail {i}\n"));
            r.write("b.txt", &t);
            r.commit_all_as(&format!("Edit b {i}"), "Lars Tidemann", "lars@example.com");
        }
        r.git(&["mv", "b.txt", "c.txt"]);
        r.commit_all_as("Rename b to c", "Ada Lovelace", "ada@example.com");
        r.write("c.txt", &lines("c"));
        r.commit_all_as("Edit c", "Ada Lovelace", "ada@example.com");
    }

    /// Pages of one walk are the full walk cut in pages: across both renames (each row's path)
    /// and the merge. Between pages, the walk is parked; after the last, it's gone.
    #[tokio::test]
    async fn pages_of_one_walk_equal_the_full_walk_across_renames_and_a_merge() {
        let r = TestRepo::new();
        renamed_and_merged(&r);
        let env = WriteEnv::new();
        let id = open(&env.api, &r).await;
        let full = page(&env, id, &r, "c.txt", None, 0, 1000).await;
        assert_eq!(full["more"], false);
        let all = full["rows"].as_array().unwrap().clone();
        assert!(all.len() > 12, "{all:?}");
        assert_eq!(env.api.history_walks.len(), 0);
        let paths: std::collections::BTreeSet<_> = all.iter().map(|r| r["path"].as_str().unwrap().to_string()).collect();
        assert_eq!(paths.into_iter().collect::<Vec<_>>(), ["a.txt", "b.txt", "c.txt"]);
        // As git itself walks it, in one go.
        let raw = r.git(&["log", "--follow", "-M", "--format=%H", "--", "c.txt"]);
        assert_eq!(all.iter().map(|r| r["sha"].as_str().unwrap()).collect::<Vec<_>>(), raw.lines().collect::<Vec<_>>());
        for size in [1u32, 3, 4] {
            let mut paged = Vec::new();
            loop {
                let p = page(&env, id, &r, "c.txt", None, paged.len() as u32, size).await;
                paged.extend(p["rows"].as_array().unwrap().iter().cloned());
                if p["more"] == false {
                    break;
                }
                assert_eq!(env.api.history_walks.len(), 1, "parked between pages");
            }
            assert_eq!(paged, all, "pages of {size}");
            assert_eq!(env.api.history_walks.len(), 0, "the walk ends with its last page");
        }
    }

    /// A page asked for out of turn (no walk there: closed, idle, another history's) starts a
    /// new walk, cut at `skip`, still across the renames.
    #[tokio::test]
    async fn a_page_with_no_walk_at_its_row_walks_again_from_the_start() {
        let r = TestRepo::new();
        renamed_and_merged(&r);
        let env = WriteEnv::new();
        let id = open(&env.api, &r).await;
        let all = page(&env, id, &r, "c.txt", None, 0, 1000).await["rows"].as_array().unwrap().clone();
        let first = page(&env, id, &r, "c.txt", None, 0, 2).await;
        assert_eq!(first["rows"].as_array().unwrap()[..], all[..2]);
        let later = page(&env, id, &r, "c.txt", None, 9, 3).await;
        assert_eq!(later["rows"].as_array().unwrap()[..], all[9..12]);
        // The parked walk was at row 2, not 9: replaced by the new one, now at row 12.
        assert_eq!(env.api.history_walks.len(), 1);
        let next = page(&env, id, &r, "c.txt", None, 12, 3).await;
        assert_eq!(next["rows"].as_array().unwrap()[..], all[12..(12 + 3).min(all.len())]);
    }

    #[tokio::test]
    async fn closing_the_history_stops_its_walk() {
        let r = TestRepo::new();
        renamed_and_merged(&r);
        let env = WriteEnv::new();
        let id = open(&env.api, &r).await;
        page(&env, id, &r, "c.txt", None, 0, 2).await;
        assert_eq!(env.api.history_walks.len(), 1);
        // Another history's close leaves it.
        send(&env.api, json!({"method": "fileHistoryClose", "params": {"repo": id, "worktree": wt(&r), "path": "b.txt"}})).await.unwrap();
        assert_eq!(env.api.history_walks.len(), 1);
        send(&env.api, json!({"method": "fileHistoryClose", "params": {"repo": id, "worktree": wt(&r), "path": "c.txt"}})).await.unwrap();
        assert_eq!(env.api.history_walks.len(), 0);
        let log = env.api.cli.log().entries();
        let walk = log.iter().rev().find(|e| e.args.iter().any(|a| a == "--follow")).unwrap();
        assert!(walk.ok && walk.exit_code.is_none(), "{walk:?}");
    }

    #[tokio::test]
    async fn an_idle_walk_is_stopped() {
        let r = TestRepo::new();
        renamed_and_merged(&r);
        let env = WriteEnv::new();
        open(&env.api, &r).await;
        let walks = HistoryWalks::with_idle(Duration::from_millis(50));
        let mut walk = HistoryWalk::start(&env.api, r.path(), "c.txt", "HEAD").await.unwrap();
        assert!(walk.page(2).await.unwrap().more);
        let key = WalkKey { repo: 1, worktree: wt(&r), path: "c.txt".into(), rev: None };
        walks.park(key.clone(), walk);
        assert_eq!(walks.len(), 1);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(walks.len(), 0);
        assert!(walks.take(&key, 2).is_none());
    }

    /// A file with lines moved within it, lines copied in from another file, and a rename.
    fn moved_and_copied(r: &TestRepo) {
        let block = |tag: &str, n: usize| (1..=n).map(|i| format!("{tag} {i}\n")).collect::<String>();
        r.write("a.txt", &(block("intro", 6) + &block("middle", 10) + &block("outro", 6)));
        r.write("lib.txt", &block("library function body line", 12));
        r.commit_all_as("Start", "Ada Lovelace", "ada@example.com");
        // The middle moves to the end.
        r.write("a.txt", &(block("intro", 6) + &block("outro", 6) + &block("middle", 10)));
        r.commit_all_as("Move the middle down", "Grace Hopper", "grace@example.com");
        // Most of lib.txt is copied in, between edits.
        let mut t = block("intro", 3) + "edited\n" + &block("intro", 6)[24..] + &block("library function body line", 12)[..300] + &block("outro", 6) + &block("middle", 10);
        t.push_str("closing\n");
        r.write("a.txt", &t);
        r.commit_all_as("Copy the library in", "Lars Tidemann", "lars@example.com");
        r.git(&["mv", "a.txt", "src.txt"]);
        let t = std::fs::read_to_string(r.path().join("src.txt")).unwrap().replacen("middle 4", "middle four", 1);
        r.write("src.txt", &t);
        r.commit_all_as("Rename to src", "Ada Lovelace", "ada@example.com");
    }

    /// `--incremental` is parsed to what `--porcelain` gives, hunk for hunk and commit for
    /// commit: with git's defaults and with move and copy detection (more, smaller entries to
    /// merge), at each commit.
    #[test]
    fn incremental_blame_equals_porcelain_blame() {
        let r = TestRepo::new();
        moved_and_copied(&r);
        fixtures::file_history(&r);
        let revs = r.git(&["log", "--format=%H"]);
        let mut compared = 0;
        for rev in revs.lines() {
            let files = r.git(&["ls-tree", "--name-only", "-r", rev]);
            for file in files.lines().filter(|f| ["src.txt", "a.txt", "src/story.txt", "story.txt"].contains(f)) {
                for flags in [&[][..], &["-M"][..], &["-C", "-C", "-M"][..]] {
                    let run = |mode: &str| {
                        let mut args = vec!["blame", mode];
                        args.extend_from_slice(flags);
                        args.extend([rev, "--", file]);
                        let out = std::process::Command::new("git").current_dir(r.path()).args(&args).output().unwrap();
                        assert!(out.status.success(), "{args:?}");
                        out.stdout
                    };
                    let porcelain = parse_blame(&run("--porcelain"));
                    assert!(!porcelain.hunks.is_empty());
                    assert_eq!(parse_blame_incremental(&run("--incremental")), porcelain, "{rev} {file} {flags:?}");
                    compared += 1;
                }
            }
        }
        assert!(compared >= 20, "{compared}");
    }

    /// A merge shows no change of its own: its row keeps the path of the rows above it.
    #[test]
    fn a_record_without_a_change_keeps_the_current_path() {
        let out = b"\x1em1\0p1 p2\0Ada\0a@x\x001\0Merge\0\x1ec1\0p1\0Ada\0a@x\x002\0Edit\0\nM\0f.txt\0\x1ec0\0\0Ada\0a@x\x003\0Add\0\nR100\0old.txt\0f.txt\0";
        let rows = parse_follow_log(out, "f.txt");
        assert_eq!(rows.iter().map(|r| (r.path.as_str(), r.status.as_str())).collect::<Vec<_>>(), [("f.txt", ""), ("f.txt", "M"), ("f.txt", "R")]);
        assert_eq!(rows[0].parents, ["p1", "p2"]);
        assert!(rows[2].parents.is_empty());
        assert_eq!(rows[2].old_path.as_deref(), Some("old.txt"));
    }
}
