//! Which tabs hold which open repository, so a handle no tab holds goes (`Api::repos`).
//!
//! Without it every handle opened stays until the app quits, closed tabs included: with many
//! repositories open over a session, the core grows by a few MB per repository (its object
//! store, status, graph walk and find snapshot) and never gives it back. A tab holds the
//! repository it shows (`HoldRepo`, by the tab's id) and lets go when it closes (`ReleaseRepo`);
//! the last one to let go takes the handle, with its caches and its watcher.
//!
//! Two rules keep that safe without the UI having to count anything right:
//! - A repository keeps its id for the whole session (`id_for`): a reopen gives the same one, so
//!   whatever the UI or a running op still holds under that id stays valid.
//! - A request for an id that was let go reopens it (`revive`, from `Api::lookup`): kept while a
//!   tab holds it (a tab whose open raced another's close), else only for that request (a
//!   Compare's cleanup after its tab closed), so handles nobody holds don't pile up again. A
//!   write still running keeps the handle it took (an `Arc`) to the end.

use crate::api::{Api, RepoHandle, config_stamp, open_options};
use crate::error::GbError;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

/// A repository opened this session: enough to open it again under the same id.
#[derive(Clone)]
struct Known {
    workdir: PathBuf,
    name: String,
    common_dir: PathBuf,
}

#[derive(Default)]
struct Inner {
    known: HashMap<u32, Known>,
    /// Tab id → the repository it shows (one each).
    holders: HashMap<String, u32>,
}

/// The session's repositories and who holds them (`Api::holds`).
#[derive(Default)]
pub(crate) struct RepoHolds(Mutex<Inner>);

impl RepoHolds {
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The id the repository at `common_dir` had earlier this session.
    pub(crate) fn id_for(&self, common_dir: &Path) -> Option<u32> {
        self.inner().known.iter().find(|(_, k)| k.common_dir == common_dir).map(|(&id, _)| id)
    }

    /// Handle `id` was just opened.
    pub(crate) fn opened(&self, id: u32, h: &RepoHandle) {
        self.inner().known.insert(id, Known { workdir: h.workdir.clone(), name: h.name.clone(), common_dir: h.common_dir.clone() });
    }

    /// How many tabs hold `id`.
    #[cfg(test)]
    pub(crate) fn holders_of(&self, id: u32) -> usize {
        self.inner().holders.values().filter(|&&r| r == id).count()
    }
}

impl Api {
    /// Tab `holder` shows repository `repo`, and no other: a tab moved to another repository
    /// lets go of the one it showed.
    pub(crate) fn hold_repo(&self, holder: String, repo: u32) {
        let before = self.holds.inner().holders.insert(holder, repo).filter(|&r| r != repo);
        self.let_go_unless_held(before);
    }

    /// Tab `holder` closed. Nothing happens for a holder that holds nothing.
    pub(crate) fn release_repo(&self, holder: &str) {
        let before = self.holds.inner().holders.remove(holder);
        self.let_go_unless_held(before);
    }

    fn let_go_unless_held(&self, id: Option<u32>) {
        let Some(id) = id.filter(|id| !self.holds.inner().holders.values().any(|v| v == id)) else { return };
        self.unwatch_repo(id);
        let h = self.repos.lock().expect("repos poisoned").remove(&id);
        // Dropped here, outside the lock: a big repository's caches take a moment to free.
        drop(h);
    }

    /// `id`, let go earlier this session, opened again (`lookup` calls it on a miss): the same
    /// workdir, fresh caches. `None` for an id this session never opened.
    pub(crate) fn revive(&self, id: u32) -> Option<Result<Arc<RepoHandle>, GbError>> {
        let (k, held) = {
            let inner = self.holds.inner();
            (inner.known.get(&id).cloned()?, inner.holders.values().any(|&r| r == id))
        };
        // Before the open, as `open_repo`: a config change made while it reads still shows.
        let config = config_stamp(&k.common_dir);
        let repo = match gix::ThreadSafeRepository::open_opts(&k.workdir, open_options()) {
            Ok(repo) => repo,
            Err(e) => return Some(Err(crate::error::gix_err(e))),
        };
        let fresh = Arc::new(RepoHandle {
            repo,
            workdir: k.workdir,
            name: k.name,
            common_dir: k.common_dir,
            wip: Arc::new(crate::snapshot::WipCache::watched_only()),
            snapshot: Mutex::new(None),
            walk: Arc::default(),
            me_email: Mutex::default(),
            config,
        });
        if !held {
            return Some(Ok(fresh));
        }
        // Another request may have revived it meanwhile: one handle, theirs.
        let mut repos = self.repos.lock().expect("repos poisoned");
        Some(Ok(repos.entry(id).or_insert(fresh).clone()))
    }
}

#[cfg(test)]
mod tests {
    use crate::api::Api;
    use crate::write::test_support::{api, call, repo};
    use serde_json::json;

    async fn open(api: &Api, dir: &std::path::Path) -> u32 {
        let s = call(api, "openRepo", json!({ "path": dir.display().to_string() })).await.unwrap();
        s["id"].as_u64().unwrap() as u32
    }

    async fn hold(api: &Api, holder: &str, repo: u32) {
        call(api, "holdRepo", json!({ "holder": holder, "repo": repo })).await.unwrap();
    }

    async fn release(api: &Api, holder: &str) {
        call(api, "releaseRepo", json!({ "holder": holder })).await.unwrap();
    }

    fn is_open(api: &Api, id: u32) -> bool {
        api.repos.lock().unwrap().contains_key(&id)
    }

    #[tokio::test]
    async fn the_last_tab_to_let_go_takes_the_handle_and_a_reopen_keeps_the_id() {
        let r = repo();
        let data = tempfile::tempdir().unwrap();
        let api = api(data.path());
        let id = open(&api, r.path()).await;
        hold(&api, "t1", id).await;
        hold(&api, "t2", id).await; // a second tab on the same repository (another worktree)
        release(&api, "t1").await;
        assert!(is_open(&api, id), "another tab still holds it");
        release(&api, "t2").await;
        assert!(!is_open(&api, id), "nobody holds it");
        assert_eq!(open(&api, r.path()).await, id);
        assert!(is_open(&api, id));
    }

    #[tokio::test]
    async fn a_request_for_an_id_let_go_opens_it_again() {
        let r = repo();
        let data = tempfile::tempdir().unwrap();
        let api = api(data.path());
        let id = open(&api, r.path()).await;
        hold(&api, "t1", id).await;
        release(&api, "t1").await;
        assert!(!is_open(&api, id));
        // A late request nobody holds it for (a Compare's cleanup after its tab closed):
        // answered, and the handle goes with it.
        call(&api, "queueState", json!({ "repo": id })).await.unwrap();
        assert!(!is_open(&api, id));
        // A tab that holds it again (its open raced another tab's close): kept.
        hold(&api, "t2", id).await;
        call(&api, "queueState", json!({ "repo": id })).await.unwrap();
        assert!(is_open(&api, id));
        // An id this session never opened is still an error.
        assert!(call(&api, "queueState", json!({ "repo": id + 100 })).await.is_err());
    }

    #[tokio::test]
    async fn a_tab_moved_to_another_repository_lets_go_of_the_first() {
        let (a, b) = (repo(), repo());
        let data = tempfile::tempdir().unwrap();
        let api = api(data.path());
        let (ia, ib) = (open(&api, a.path()).await, open(&api, b.path()).await);
        assert_ne!(ia, ib);
        hold(&api, "t1", ia).await;
        hold(&api, "t1", ia).await; // held again by the same tab: still one holder
        assert_eq!(api.holds.holders_of(ia), 1);
        hold(&api, "t1", ib).await;
        assert!(!is_open(&api, ia));
        assert!(is_open(&api, ib));
        release(&api, "nobody").await; // a tab that held nothing
        assert!(is_open(&api, ib));
    }

    #[tokio::test]
    async fn letting_go_stops_the_repository_s_watcher() {
        let r = repo();
        let data = tempfile::tempdir().unwrap();
        let api = api(data.path());
        let id = open(&api, r.path()).await;
        hold(&api, "t1", id).await;
        call(&api, "watchRepo", json!({ "repo": id })).await.unwrap();
        assert_eq!(api.watched_repos(), vec![id]);
        release(&api, "t1").await;
        assert!(api.watched_repos().is_empty());
    }
}
