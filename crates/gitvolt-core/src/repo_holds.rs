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
//!
//! A watcher feeds the `WipCache` of the handle it was started for, so one runs only for a
//! handle in `Api::repos` (`watch_repo`) and goes with it. Lock order: `repos`, then `holds`,
//! then `watchers`.

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

/// Tests: runs in `revive` between its open and its insert.
#[cfg(test)]
type DuringOpen = Box<dyn FnOnce(&Api) + Send>;

/// The session's repositories and who holds them (`Api::holds`).
#[derive(Default)]
pub(crate) struct RepoHolds {
    inner: Mutex<Inner>,
    #[cfg(test)]
    during_open: Mutex<Option<DuringOpen>>,
}

impl RepoHolds {
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn held(&self, id: u32) -> bool {
        self.inner().holders.values().any(|&r| r == id)
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

    /// Every tab let go: the UI's startup reset, as `unwatch_all` (a reload's tabs are new, and
    /// hold theirs again as they open). Every handle goes, held before or not; ids stay.
    pub(crate) fn release_all(&self) {
        self.holds.inner().holders.clear();
        let ids: Vec<u32> = self.repos.lock().expect("repos poisoned").keys().copied().collect();
        for id in ids {
            self.let_go_unless_held(Some(id));
        }
    }

    fn let_go_unless_held(&self, id: Option<u32>) {
        let Some(id) = id else { return };
        // Checked under `repos`: a revive or a watch can't slip in between the check and the drop.
        let (h, w) = {
            let mut repos = self.repos.lock().expect("repos poisoned");
            if self.holds.held(id) {
                return;
            }
            (repos.remove(&id), self.watchers.lock().expect("watchers poisoned").remove(&id))
        };
        drop(w);
        // Outside the locks, and off the async worker: a big repository's caches take a moment
        // to free (when nothing else still has the handle).
        match (h, tokio::runtime::Handle::try_current()) {
            (Some(h), Ok(rt)) => drop(rt.spawn_blocking(move || drop(h))),
            (h, _) => drop(h),
        }
    }

    /// `id`, let go earlier this session, opened again (`lookup` calls it on a miss): the same
    /// workdir, fresh caches. `None` for an id this session never opened.
    pub(crate) fn revive(&self, id: u32) -> Option<Result<Arc<RepoHandle>, GbError>> {
        let k = self.holds.inner().known.get(&id).cloned()?;
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
        #[cfg(test)]
        {
            let during_open = self.holds.during_open.lock().unwrap().take();
            if let Some(f) = during_open {
                f(self);
            }
        }
        // Whether a tab holds it is read only now, under `repos`: one may have let go during the
        // open (it found nothing to drop), and the handle would stay for nobody.
        let mut repos = self.repos.lock().expect("repos poisoned");
        // Another request may have revived it meanwhile: one handle, theirs.
        if let Some(h) = repos.get(&id) {
            return Some(Ok(h.clone()));
        }
        if self.holds.held(id) {
            repos.insert(id, fresh.clone());
            // A watcher left for the id feeds another handle's cache (none should be).
            self.unwatch_repo(id);
        }
        Some(Ok(fresh))
    }
}

#[cfg(test)]
mod tests {
    use crate::api::Api;
    use crate::write::test_support::{api, call, repo};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

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

    /// A tab letting go while a request reopens its repository (`revive`'s open is slow): the
    /// handle isn't kept for nobody.
    #[tokio::test]
    async fn a_tab_letting_go_during_a_revive_leaves_no_handle() {
        let r = repo();
        let data = tempfile::tempdir().unwrap();
        let api = api(data.path());
        let id = open(&api, r.path()).await;
        hold(&api, "t1", id).await;
        release(&api, "t1").await;
        hold(&api, "t2", id).await; // a tab whose open raced the close: the next request revives it
        *api.holds.during_open.lock().unwrap() = Some(Box::new(|api: &Api| api.release_repo("t2")));
        call(&api, "queueState", json!({ "repo": id })).await.unwrap();
        assert!(api.holds.during_open.lock().unwrap().is_none(), "the tab let go during the open");
        assert!(!is_open(&api, id), "nobody holds it any more");
    }

    /// A late `watchRepo` from a tab that closed (an unheld revive) starts nothing: nothing would
    /// stop it, and it would feed a cache the next open of the repository doesn't have.
    #[tokio::test]
    async fn a_watch_for_a_repository_let_go_starts_no_watcher() {
        let r = repo();
        let data = tempfile::tempdir().unwrap();
        let api = api(data.path());
        let id = open(&api, r.path()).await;
        hold(&api, "t1", id).await;
        release(&api, "t1").await;
        call(&api, "watchRepo", json!({ "repo": id })).await.unwrap();
        assert!(api.watched_repos().is_empty());
        assert!(!is_open(&api, id));
    }

    /// Two requests reviving one held id at once (each opens it): one handle is kept, and both
    /// get it.
    #[tokio::test]
    async fn two_revives_at_once_keep_one_handle() {
        let r = repo();
        let data = tempfile::tempdir().unwrap();
        let api = api(data.path());
        let id = open(&api, r.path()).await;
        hold(&api, "t1", id).await;
        release(&api, "t1").await;
        hold(&api, "t2", id).await;
        let theirs = Arc::new(Mutex::new(None));
        let slot = theirs.clone();
        *api.holds.during_open.lock().unwrap() = Some(Box::new(move |api: &Api| *slot.lock().unwrap() = api.revive(id)));
        let mine = api.revive(id).unwrap().unwrap();
        let theirs = theirs.lock().unwrap().take().unwrap().unwrap();
        let kept = api.repos.lock().unwrap()[&id].clone();
        assert!(Arc::ptr_eq(&mine, &kept) && Arc::ptr_eq(&theirs, &kept));
    }

    /// A reopen (a config change) of a handle let go since, and opened again, doesn't replace
    /// the newer handle with one built from the old handle's caches.
    #[tokio::test]
    async fn a_reopen_of_a_handle_let_go_keeps_the_newer_one() {
        let r = repo();
        let data = tempfile::tempdir().unwrap();
        let api = api(data.path());
        let id = open(&api, r.path()).await;
        hold(&api, "t1", id).await;
        let old = api.handle(id).unwrap();
        release(&api, "t1").await;
        assert_eq!(open(&api, r.path()).await, id);
        hold(&api, "t2", id).await;
        let newer = api.repos.lock().unwrap()[&id].clone();
        api.reopen(id, &old).unwrap();
        assert!(Arc::ptr_eq(&api.repos.lock().unwrap()[&id], &newer));
        // The current handle's reopen still replaces it, caches kept.
        let fresh = api.reopen(id, &newer).unwrap();
        let now = api.repos.lock().unwrap()[&id].clone();
        assert!(Arc::ptr_eq(&now, &fresh) && Arc::ptr_eq(&now.wip, &newer.wip));
    }

    /// The UI's startup reset: a reloaded page's tabs are new, so every hold goes, and with it
    /// every handle (and watcher); the ids stay, and the tabs hold them again.
    #[tokio::test]
    async fn release_all_lets_go_of_every_repository() {
        let (a, b) = (repo(), repo());
        let data = tempfile::tempdir().unwrap();
        let api = api(data.path());
        let (ia, ib) = (open(&api, a.path()).await, open(&api, b.path()).await);
        hold(&api, "t1", ia).await; // b: opened, never held
        call(&api, "watchRepo", json!({ "repo": ia })).await.unwrap();
        api.dispatch(crate::api::Request::ReleaseAll).await.unwrap();
        assert_eq!(api.holds.holders_of(ia), 0);
        assert!(!is_open(&api, ia) && !is_open(&api, ib));
        assert!(api.watched_repos().is_empty());
        assert_eq!(open(&api, a.path()).await, ia);
        hold(&api, "t1", ia).await;
        assert!(is_open(&api, ia));
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
