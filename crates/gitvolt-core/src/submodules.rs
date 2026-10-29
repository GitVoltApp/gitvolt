//! Submodules, for repositories that use them: which worktrees use them (the UI offers its
//! submodule actions only there), `git submodule status`, for the "Open in a tab" picker, and the
//! sidebar's Submodules section (`sidebar_items`). The writes (update, sync, add, remove) are
//! `write::submodules`.

use crate::api::Api;
use crate::error::{gix_err, GbError};
use crate::git::GitInvocation;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use ts_rs::TS;

/// One line of `git submodule status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SubmoduleStatus {
    /// Checked out at the recorded commit (` `).
    Current,
    /// Checked out at another commit (`+`).
    NewCommits,
    /// Not initialised (`-`).
    Uninitialized,
    /// Conflicted in the index (`U`).
    Conflicted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SubmoduleInfo {
    /// Relative to the worktree.
    pub path: String,
    /// The commit checked out, or the one recorded when it isn't initialised.
    pub commit: String,
    pub status: SubmoduleStatus,
}

/// A submodule in the sidebar's Submodules section: a gitlink of the worktree's index, what
/// `.gitmodules` says of it, and what's checked out there.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SubmoduleItem {
    /// Relative to the worktree.
    pub path: String,
    /// Its name in `.gitmodules` (`submodule.<name>`); the path when it has no entry there.
    pub name: String,
    /// `.gitmodules`' URL, credentials removed; `None` without an entry.
    pub url: Option<String>,
    /// The commit the index records (our side while it's conflicted).
    pub recorded: String,
    /// The commit checked out in it; `None` when it isn't initialised.
    pub head: Option<String>,
    /// The branch checked out in it, when it's on one.
    pub branch: Option<String>,
    pub status: SubmoduleStatus,
}

#[cfg(test)]
thread_local! {
    /// The repositories `open` opened on this thread.
    static OPENED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// `gix::open` of `dir` (counted in tests: the sidebar's read opens nothing in a repository
/// without submodules).
fn open(dir: &Path) -> Option<gix::Repository> {
    #[cfg(test)]
    OPENED.with(|n| n.set(n.get() + 1));
    gix::open(dir).ok()
}

/// The repository checked out at `path` in `root`: `gix::open` of that folder alone (never a
/// discovery up to the superproject), and never through a symlink.
pub(crate) fn open_checkout(root: &Path, path: &str) -> Option<gix::Repository> {
    let dir = crate::blob::safe_join(root, path).ok()?;
    if !std::fs::symlink_metadata(&dir).ok()?.is_dir() {
        return None;
    }
    open(&dir)
}

/// The `sidebar` read's submodules of each checked-out worktree in `roots` (canonical paths),
/// keyed by path; a worktree without any isn't listed. `repo` is the handle already open on
/// `here`, one of `roots`. Cheap without submodules: `here`'s gitlinks come from `repo`'s index
/// (gix keeps it, read again only once it changes), and a linked worktree is opened only when it
/// has a `.gitmodules` (so its gitlinks without one, an embedded repository staged by hand,
/// aren't listed). Each submodule's checkout is opened for its HEAD. `seen` records what was
/// listed. Call it off the runtime.
pub fn sidebar_by_worktree(repo: &gix::Repository, here: &Path, roots: Vec<PathBuf>, seen: &Seen) -> BTreeMap<String, Vec<SubmoduleItem>> {
    let mut out = BTreeMap::new();
    for root in roots {
        let items = worktree_items(repo, here, &root);
        seen.note(&root, &items);
        if !items.is_empty() {
            out.insert(root.display().to_string(), items);
        }
    }
    out
}

/// One worktree's submodules, as `sidebar_by_worktree` reads them.
fn worktree_items(repo: &gix::Repository, here: &Path, root: &Path) -> Vec<SubmoduleItem> {
    if root == here {
        sidebar_items(repo, root)
    } else if has_gitmodules(root) {
        open(root).map(|r| sidebar_items(&r, root)).unwrap_or_default()
    } else {
        Vec::new()
    }
}

fn has_gitmodules(root: &Path) -> bool {
    std::fs::symlink_metadata(root.join(".gitmodules")).is_ok()
}

/// What the sidebar last listed of each worktree's submodules (a digest per canonical root, none
/// when it listed none), so `repoChanged` says `submodules` only when that changed: a gitlink in
/// the index, `.gitmodules`, or a submodule's checked-out commit.
#[derive(Debug, Default)]
pub struct Seen(Mutex<HashMap<PathBuf, u64>>);

impl Seen {
    /// Records `items` as `root`'s listing; `true` when it differs from the one before.
    fn note(&self, root: &Path, items: &[SubmoduleItem]) -> bool {
        let digest = (!items.is_empty()).then(|| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            items.hash(&mut h);
            h.finish()
        });
        let mut seen = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let before = match digest {
            Some(d) => seen.insert(root.to_path_buf(), d),
            None => seen.remove(root),
        };
        before != digest
    }

    /// Whether the submodules of any of `roots` (`repo` open on `here`, as for
    /// `sidebar_by_worktree`) changed since they were last listed or checked; records them as
    /// they are now. A worktree without a `.gitmodules` that had none isn't read at all, so an
    /// ordinary repository's index and file changes cost a stat each. Call it off the runtime.
    pub fn check(&self, repo: &gix::Repository, here: &Path, roots: &[PathBuf]) -> bool {
        let mut changed = false;
        for root in roots {
            let listed = self.0.lock().unwrap_or_else(|e| e.into_inner()).contains_key(root);
            if listed || has_gitmodules(root) {
                changed |= self.note(root, &worktree_items(repo, here, root));
            }
        }
        changed
    }
}

/// The sidebar's submodules of the worktree `repo` is opened on (`root`), by path. In process,
/// no git spawned: the index's gitlinks, `.gitmodules`, and each checkout's HEAD. Empty when it
/// uses none. Reads the index: call it off the runtime.
pub fn sidebar_items(repo: &gix::Repository, root: &Path) -> Vec<SubmoduleItem> {
    let Ok(index) = repo.index_or_empty() else { return Vec::new() };
    // Path → recorded commit and whether it's conflicted; a conflict's side 2 ("ours") is recorded.
    let mut links: std::collections::BTreeMap<String, (String, bool)> = std::collections::BTreeMap::new();
    for e in index.entries().iter().filter(|e| e.mode == gix::index::entry::Mode::COMMIT) {
        let stage = e.stage_raw();
        let slot = links.entry(e.path(&index).to_string()).or_insert_with(|| (e.id.to_string(), false));
        if stage != 0 {
            slot.1 = true;
            if stage == 2 {
                slot.0 = e.id.to_string();
            }
        }
    }
    if links.is_empty() {
        return Vec::new();
    }
    let declared: Vec<(String, String, Option<String>)> = repo
        .submodules()
        .ok()
        .flatten()
        .map(|subs| subs.filter_map(|s| Some((s.path().ok()?.to_string(), s.name().to_string(), s.url().ok().map(|u| crate::redact::redact(&u.to_bstring().to_string()))))).collect())
        .unwrap_or_default();
    links
        .into_iter()
        .map(|(path, (recorded, conflicted))| {
            let entry = declared.iter().find(|(p, _, _)| *p == path);
            let checkout = open_checkout(root, &path);
            let head = checkout.as_ref().and_then(|r| r.head_id().ok()).map(|id| id.to_string());
            let branch = checkout.as_ref().and_then(|r| r.head_name().ok().flatten()).map(|n| n.shorten().to_string());
            let status = if conflicted {
                SubmoduleStatus::Conflicted
            } else {
                match &head {
                    None => SubmoduleStatus::Uninitialized,
                    Some(h) if *h == recorded => SubmoduleStatus::Current,
                    Some(_) => SubmoduleStatus::NewCommits,
                }
            };
            SubmoduleItem { name: entry.map_or_else(|| path.clone(), |(_, n, _)| n.clone()), url: entry.and_then(|(_, _, u)| u.clone()), path, recorded, head, branch, status }
        })
        .collect()
}

/// The gitlinks in `root`'s index: each path, with the commit it records.
pub(crate) fn gitlinks(root: &Path) -> Result<Vec<(String, String)>, GbError> {
    let repo = gix::open(root).map_err(gix_err)?;
    let index = repo.index_or_empty().map_err(gix_err)?;
    Ok(index
        .entries()
        .iter()
        .filter(|e| e.mode == gix::index::entry::Mode::COMMIT)
        .map(|e| (e.path(&index).to_string(), e.id.to_string()))
        .collect())
}

/// A worktree that uses submodules (a `.gitmodules` file or a gitlink in its index).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SubmoduleSupport {
    /// Gitlinks `.gitmodules` doesn't map (an embedded repository added without one): git's
    /// submodule commands refuse the whole worktree while there's one.
    pub unmapped: Vec<String>,
}

/// Whether the worktree at `root` uses submodules, and which of its gitlinks git's submodule
/// commands can't handle. `None`: it uses none. Reads the index: call it off the runtime.
pub fn support(root: &Path) -> Option<SubmoduleSupport> {
    let repo = gix::open(root).ok()?;
    let links = gitlinks(root).ok()?;
    if links.is_empty() && !root.join(".gitmodules").is_file() {
        return None;
    }
    let mapped: Vec<String> = repo.submodules().ok().flatten().map(|subs| subs.filter_map(|s| s.path().ok().map(|p| p.to_string())).collect()).unwrap_or_default();
    Some(SubmoduleSupport { unmapped: links.into_iter().map(|(p, _)| p).filter(|p| !mapped.contains(p)).collect() })
}

/// `git submodule status` lines, matched to the gitlinks' `paths` (the line's text after the
/// commit is the path, then git's ` (describe)` when there is one): a state character, the
/// commit, the path. A line no gitlink matches is left out.
pub fn parse_status(out: &str, paths: &[String]) -> Vec<SubmoduleInfo> {
    out.lines()
        .filter_map(|line| {
            let status = match line.chars().next()? {
                ' ' => SubmoduleStatus::Current,
                '+' => SubmoduleStatus::NewCommits,
                '-' => SubmoduleStatus::Uninitialized,
                'U' => SubmoduleStatus::Conflicted,
                _ => return None,
            };
            let (commit, rest) = line[1..].split_once(' ')?;
            let path = paths.iter().filter(|p| rest == p.as_str() || rest.strip_prefix(p.as_str()).is_some_and(|d| d.starts_with(" ("))).max_by_key(|p| p.len())?;
            Some(SubmoduleInfo { path: path.clone(), commit: commit.to_string(), status })
        })
        .collect()
}

impl Api {
    /// `git submodule status` in `worktree`, one entry per gitlink of its index. A read: git
    /// takes no optional lock.
    pub(crate) async fn submodules(&self, repo: u32, worktree: &str) -> Result<Vec<SubmoduleInfo>, GbError> {
        let h = self.handle(repo)?;
        let root = self.worktree_dir(&h, worktree).await?;
        let r = root.clone();
        let paths: Vec<String> = crate::api::blocking(move || gitlinks(&r)).await?.into_iter().map(|(p, _)| p).collect();
        let out = self.cli.run(GitInvocation::new(&root, ["submodule", "status"])).await?;
        Ok(parse_status(&String::from_utf8_lossy(&out.stdout), &paths))
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{fixtures, TestRepo};
    use serde_json::json;

    #[test]
    fn parses_each_state_and_matches_the_gitlinks_paths() {
        let (a, b) = ("a".repeat(40), "b".repeat(40));
        // `vendor/x (old)` is a path of its own: the longest gitlink that fits wins, and a line no
        // gitlink matches is left out.
        let out = format!(" {a} vendor/parser (heads/main)\n+{b} vendor/two words\n-{a} vendor/theme\nU{b} vendor/x (old) (v1.0-2-gabc)\n+{b} elsewhere/unknown\n");
        let paths: Vec<String> = ["vendor/parser", "vendor/two words", "vendor/theme", "vendor/x", "vendor/x (old)"].map(String::from).to_vec();
        let list = parse_status(&out, &paths);
        let got: Vec<(&str, &str, SubmoduleStatus)> = list.iter().map(|s| (s.path.as_str(), s.commit.as_str(), s.status)).collect();
        assert_eq!(got, vec![
            ("vendor/parser", a.as_str(), SubmoduleStatus::Current),
            ("vendor/two words", b.as_str(), SubmoduleStatus::NewCommits),
            ("vendor/theme", a.as_str(), SubmoduleStatus::Uninitialized),
            ("vendor/x (old)", b.as_str(), SubmoduleStatus::Conflicted),
        ]);
    }

    #[tokio::test]
    async fn lists_each_submodule_with_its_state_and_writes_nothing() {
        use crate::write::test_support::{api, open, wt};
        let r = TestRepo::new();
        fixtures::submodules(&r);
        let parser = r.path().join("vendor/parser");
        r.git_in(&parser, &["checkout", "-q", "HEAD~1"]);
        let older = r.git_in(&parser, &["rev-parse", "HEAD"]);
        let theme = r.git(&["rev-parse", "HEAD:vendor/theme"]);
        let index = std::fs::read(r.path().join(".git/index")).unwrap();
        let data = tempfile::tempdir().unwrap();
        let api = api(data.path());
        let id = open(&api, &r).await;
        let list = api.submodules(id, &wt(r.path())).await.unwrap();
        assert_eq!(list, vec![
            SubmoduleInfo { path: "vendor/parser".into(), commit: older, status: SubmoduleStatus::NewCommits },
            SubmoduleInfo { path: "vendor/theme".into(), commit: theme, status: SubmoduleStatus::Uninitialized },
        ]);
        assert_eq!(std::fs::read(r.path().join(".git/index")).unwrap(), index, "a read");
        let info = api.dispatch(crate::api::Request::RepoInfo { repo: id }).await.unwrap();
        assert_eq!(info["submodules"], json!({ wt(r.path()): {"unmapped": []} }));
    }

    #[test]
    fn the_sidebar_lists_each_submodule_with_its_url_checkout_and_state() {
        let plain = TestRepo::new();
        plain.commit("one");
        assert_eq!(sidebar_items(&gix::open(plain.path()).unwrap(), plain.path()), vec![]);

        let r = TestRepo::new();
        fixtures::submodules(&r);
        let parser = r.path().join("vendor/parser");
        let recorded = r.git(&["rev-parse", "HEAD:vendor/parser"]);
        r.git_in(&parser, &["checkout", "-q", "HEAD~1"]);
        let older = r.git_in(&parser, &["rev-parse", "HEAD"]);
        let theme = r.git(&["rev-parse", "HEAD:vendor/theme"]);
        let url = |name: &str| Some(r.root().join(name).display().to_string());
        let list = sidebar_items(&gix::open(r.path()).unwrap(), r.path());
        assert_eq!(list, vec![
            SubmoduleItem { path: "vendor/parser".into(), name: "vendor/parser".into(), url: url("parser-src"), recorded: recorded.clone(), head: Some(older), branch: None, status: SubmoduleStatus::NewCommits },
            SubmoduleItem { path: "vendor/theme".into(), name: "vendor/theme".into(), url: url("theme-src"), recorded: theme, head: None, branch: None, status: SubmoduleStatus::Uninitialized },
        ]);

        // Back on its branch at the recorded commit: current, and the branch is named.
        r.git_in(&parser, &["checkout", "-q", "main"]);
        let parser_item = sidebar_items(&gix::open(r.path()).unwrap(), r.path()).remove(0);
        assert_eq!((parser_item.head.as_deref(), parser_item.branch.as_deref(), parser_item.status), (Some(recorded.as_str()), Some("main"), SubmoduleStatus::Current));
    }

    #[tokio::test]
    async fn the_sidebar_payload_carries_the_submodules() {
        let r = TestRepo::new();
        fixtures::submodules(&r);
        r.git(&["branch", "plain"]);
        let linked = r.add_worktree("plain", "plain");
        let repo = gix::ThreadSafeRepository::open(r.path()).unwrap();
        let s = crate::shelldata::sidebar(&repo, r.path(), &Default::default()).await.unwrap();
        // Keyed by canonical worktree path; the linked worktree, its submodules not checked out,
        // lists them too, uninitialised.
        let key = |p: &std::path::Path| crate::platform::fs::canonicalize(p).unwrap().display().to_string();
        assert_eq!(s.submodules.keys().cloned().collect::<Vec<_>>(), { let mut k = vec![key(r.path()), key(&linked)]; k.sort(); k });
        assert_eq!(s.submodules[&key(r.path())].iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), ["vendor/parser", "vendor/theme"]);
        assert!(s.submodules[&key(&linked)].iter().all(|s| s.status == SubmoduleStatus::Uninitialized));
    }

    /// Without submodules, the sidebar's read opens no repository: the open handle's index says
    /// so for its worktree, and a linked one without a `.gitmodules` isn't opened.
    #[test]
    fn the_sidebar_opens_nothing_without_submodules() {
        let opened = || OPENED.with(|n| n.get());
        let key = |p: &std::path::Path| crate::platform::fs::canonicalize(p).unwrap();
        let plain = TestRepo::new();
        plain.commit("one");
        plain.git(&["branch", "other"]);
        let linked = plain.add_worktree("other", "other");
        let repo = gix::open(plain.path()).unwrap();
        let before = opened();
        assert!(sidebar_by_worktree(&repo, &key(plain.path()), vec![key(plain.path()), key(&linked)], &Seen::default()).is_empty());
        assert_eq!(opened(), before, "nothing opened");

        // With submodules: the open handle for its own worktree, then the linked one and each
        // checkout of both (the linked one's, not initialised, are empty folders).
        let r = TestRepo::new();
        fixtures::submodules(&r);
        r.git(&["branch", "plain"]);
        let linked = r.add_worktree("plain", "plain");
        let repo = gix::open(r.path()).unwrap();
        let before = opened();
        let s = sidebar_by_worktree(&repo, &key(r.path()), vec![key(r.path()), key(&linked)], &Seen::default());
        assert_eq!(s.len(), 2);
        assert_eq!(opened() - before, 1 + 2 + 2);
    }

    /// `Seen` reports a change of what the sidebar listed (a gitlink, `.gitmodules`, a
    /// checkout's HEAD) once, and nothing else; without submodules, it reads nothing.
    #[test]
    fn seen_reports_a_change_of_what_the_sidebar_listed() {
        let r = TestRepo::new();
        fixtures::submodules(&r);
        let root = crate::platform::fs::canonicalize(r.path()).unwrap();
        let seen = Seen::default();
        // A fresh handle each time, so its index is read as it is now.
        let check = || seen.check(&gix::open(&root).unwrap(), &root, std::slice::from_ref(&root));
        sidebar_by_worktree(&gix::open(&root).unwrap(), &root, vec![root.clone()], &seen);
        assert!(!check(), "as listed");
        r.write("notes.txt", "n\n");
        r.git(&["add", "notes.txt"]);
        assert!(!check(), "an ordinary file staged");
        r.git(&["rm", "-q", "--cached", "vendor/parser"]);
        assert!(check(), "a gitlink unstaged");
        assert!(!check(), "reported once");
        r.git(&["reset", "-q", "--", "vendor/parser"]);
        assert!(check(), "and staged again");
        // (The parser's URL is its config's, which `--init` wrote; the theme isn't initialised.)
        r.git(&["config", "-f", ".gitmodules", "submodule.vendor/theme.url", "../elsewhere"]);
        assert!(check(), ".gitmodules' URL");
        r.git_in(&r.path().join("vendor/parser"), &["checkout", "-q", "HEAD~1"]);
        assert!(check(), "a checkout's HEAD");
        assert!(!check());

        let plain = TestRepo::new();
        plain.commit("one");
        let root = crate::platform::fs::canonicalize(plain.path()).unwrap();
        let repo = gix::open(&root).unwrap();
        sidebar_by_worktree(&repo, &root, vec![root.clone()], &seen);
        plain.write("notes.txt", "n\n");
        plain.git(&["add", "notes.txt"]);
        let before = OPENED.with(|n| n.get());
        assert!(!seen.check(&repo, &root, std::slice::from_ref(&root)));
        assert_eq!(OPENED.with(|n| n.get()), before);
    }

    #[test]
    fn a_worktree_uses_submodules_with_a_gitmodules_file_or_a_gitlink() {
        let plain = TestRepo::new();
        plain.commit("one");
        assert_eq!(support(plain.path()), None);

        let r = TestRepo::new();
        fixtures::submodules(&r);
        assert_eq!(support(r.path()), Some(SubmoduleSupport::default()));

        // An embedded repository staged without a `.gitmodules`: a gitlink git's submodule
        // commands refuse.
        let e = TestRepo::new();
        e.commit("one");
        let inner = e.path().join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        e.git_in(&inner, &["init", "-q", "-b", "main"]);
        std::fs::write(inner.join("x.txt"), "x\n").unwrap();
        e.git_in(&inner, &["add", "x.txt"]);
        e.git_in(&inner, &["commit", "-q", "-m", "inner"]);
        e.git(&["add", "inner"]);
        assert!(!e.path().join(".gitmodules").exists());
        assert_eq!(support(e.path()), Some(SubmoduleSupport { unmapped: vec!["inner".into()] }));
    }

    /// Per worktree: a linked worktree whose branch has submodules, beside a main one without.
    #[tokio::test]
    async fn repo_info_names_each_worktree_that_uses_submodules() {
        let r = TestRepo::new();
        r.commit("one");
        r.switch_new("with-subs");
        r.write(".gitmodules", "[submodule \"vendor/x\"]\n\tpath = vendor/x\n\turl = ../x-src\n");
        r.git(&["add", ".gitmodules"]);
        r.git(&["commit", "-q", "-m", "gitmodules"]);
        r.switch("main");
        let linked = r.add_worktree("subs", "with-subs");
        let repo = gix::ThreadSafeRepository::open(r.path()).unwrap();
        let info = crate::shelldata::repo_info(&repo, r.path()).await.unwrap();
        let canonical = crate::platform::fs::canonicalize(&linked).unwrap().display().to_string();
        assert_eq!(info.submodules.into_iter().collect::<Vec<_>>(), vec![(canonical, SubmoduleSupport::default())]);
    }
}
