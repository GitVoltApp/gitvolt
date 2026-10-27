//! What each remote's tags are. git keeps no remote-tracking refs for tags, so after a fetch the
//! core asks every remote it fetched for its tag list (`git ls-remote --tags <remote>`, through
//! the same askpass, environment and redaction as the fetch), and a tag push or delete through
//! GitVolt updates the list it changed, without another request. The graph's tag chips show it
//! (`gitvolt_graph::RefLabel::tag_remotes`).
//!
//! Kept per repository, by canonical common dir, so every tab of it shares one: in memory, and in
//! `<data>/remote-tags/<first 16 hex of sha256(common dir)>.json` (0600, in a 0700 directory),
//! beside the rewrite marks, so a restart shows the last known state at once, as git keeps
//! remote-tracking branches across one. It's a cache: a lost or corrupt file only means "unknown"
//! until the next fetch.

use crate::api::{Api, RepoHandle};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// One remote's tags: name → the commit it peels to (full hex).
pub(crate) type TagList = BTreeMap<String, String>;
/// Each remote whose tags are known → its tags.
pub(crate) type Known = BTreeMap<String, TagList>;

/// `git ls-remote --tags` output as tag → commit: an annotated tag's peeled line (`<tag>^{}`)
/// wins over its own (the tag object's), whichever order they come in.
pub(crate) fn parse_ls_remote(out: &str) -> TagList {
    let mut tags = TagList::new();
    let mut peeled: Vec<(String, String)> = Vec::new();
    for line in out.lines() {
        let Some((oid, name)) = line.split_once('\t') else { continue };
        let Some(name) = name.trim_end().strip_prefix("refs/tags/") else { continue };
        if oid.len() < 40 || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        match name.strip_suffix("^{}") {
            Some(tag) => peeled.push((tag.to_string(), oid.to_ascii_lowercase())),
            None => {
                tags.entry(name.to_string()).or_insert_with(|| oid.to_ascii_lowercase());
            }
        }
    }
    for (tag, oid) in peeled {
        tags.insert(tag, oid);
    }
    tags
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    version: u32,
    remotes: Known,
}

const VERSION: u32 = 1;

/// Every repository's known remote tags this session (`Api::remote_tags`).
#[derive(Default)]
pub(crate) struct RemoteTags {
    repos: Mutex<HashMap<PathBuf, Known>>,
}

fn file_of(data_dir: &Path, common_dir: &Path) -> PathBuf {
    let common = crate::platform::fs::canonicalize(common_dir).unwrap_or_else(|_| common_dir.to_path_buf());
    let name = &crate::journal::JournalStore::hex(&Sha256::digest(common.as_os_str().as_encoded_bytes()))[..16];
    data_dir.join("remote-tags").join(format!("{name}.json"))
}

fn load(path: &Path) -> Known {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<File>(&b).ok()).filter(|f| f.version == VERSION).map(|f| f.remotes).unwrap_or_default()
}

fn save(data_dir: &Path, path: &Path, known: &Known) -> Result<(), crate::error::GbError> {
    crate::paths::private_dir(data_dir)?;
    if let Some(dir) = path.parent() {
        crate::paths::private_dir(dir)?;
    }
    let bytes = serde_json::to_vec(&File { version: VERSION, remotes: known.clone() }).map_err(|e| crate::error::GbError::other(format!("remote tags: {e}")))?;
    crate::journal::write_private(path, &bytes)
}

impl RemoteTags {
    fn with<T>(&self, data_dir: &Path, common_dir: &Path, f: impl FnOnce(&mut Known) -> T) -> (T, bool) {
        let mut repos = self.repos.lock().unwrap_or_else(|e| e.into_inner());
        let path = file_of(data_dir, common_dir);
        let known = repos.entry(common_dir.to_path_buf()).or_insert_with(|| load(&path));
        let before = known.clone();
        let out = f(known);
        let changed = *known != before;
        if changed && let Err(e) = save(data_dir, &path, known) {
            tracing::warn!("couldn't save the remotes' tags to {}: {}", path.display(), e.message);
        }
        (out, changed)
    }

    /// The repository's known remote tags (read from its file the first time).
    pub(crate) fn get(&self, data_dir: &Path, common_dir: &Path) -> Known {
        self.with(data_dir, common_dir, |k| k.clone()).0
    }

    /// Runs `f` on them, saving them if it changed them: whether it did.
    pub(crate) fn update(&self, data_dir: &Path, common_dir: &Path, f: impl FnOnce(&mut Known)) -> bool {
        self.with(data_dir, common_dir, f).1
    }
}

/// The remotes `git fetch --all` fetches, in its order: each one with a URL and without
/// `remote.<name>.skipFetchAll`.
pub(crate) fn fetch_all_remotes(workdir: &Path) -> Vec<String> {
    let Ok(repo) = gix::open(workdir) else { return Vec::new() };
    let config = repo.config_snapshot();
    crate::remotes::remote_names_in_order(&repo)
        .into_iter()
        .filter(|r| !config.boolean(format!("remote.{r}.skipFetchAll").as_str()).unwrap_or(false))
        .filter(|r| crate::remotes::remote_url(&repo, r, gix::remote::Direction::Fetch).is_some())
        .collect()
}

/// Each local tag (or only `only`) and the commit it peels to.
pub(crate) fn local_tags(root: &Path, only: Option<&str>) -> Vec<(String, String)> {
    let Ok(repo) = gix::open(root) else { return Vec::new() };
    let Ok(platform) = repo.references() else { return Vec::new() };
    let Ok(refs) = platform.prefixed(b"refs/tags/".as_slice()) else { return Vec::new() };
    refs.flatten()
        .filter_map(|mut r| {
            let name = r.name().as_bstr().to_string().strip_prefix("refs/tags/")?.to_string();
            if only.is_some_and(|o| o != name) {
                return None;
            }
            let commit = r.peel_to_id().ok()?.detach().to_string();
            Some((name, commit))
        })
        .collect()
}

impl Api {
    /// Repository `h`'s known remote tags, for the graph (`BuildOptions::remote_tags`).
    pub(crate) fn known_remote_tags(&self, h: &RepoHandle) -> Known {
        self.remote_tags.get(&self.data_dir, &h.common_dir)
    }

    /// After a fetch of `remotes`: each one's tag list, from `git ls-remote --tags`, run as the
    /// fetch runs git (no `ext::`, detached, askpass) and cancelled with it (`cancel`). It never
    /// prompts: it runs under an op of its own that isn't interactive, so a remote whose
    /// credentials nothing supplies without asking (the fetch just asked) stays as it was. A
    /// failed listing keeps that remote's last known list, and is only logged: it never fails
    /// the fetch. Whether anything changed.
    pub(crate) async fn refresh_remote_tags(&self, h: &RepoHandle, repo: u32, remotes: &[String], cancel: &CancellationToken) -> bool {
        let quiet = self.ops.begin(crate::events::OpKind::Fetch, Some(repo), false);
        let mut listed = Vec::new();
        for remote in remotes {
            if cancel.is_cancelled() {
                break;
            }
            let args = crate::netops::NO_EXT.into_iter().chain(["ls-remote", "--tags", "--end-of-options", remote.as_str()]);
            let inv = crate::git::GitInvocation::new(&h.workdir, args).timeout(None).cancel(cancel.clone()).detach_terminal().envs(self.net_env(quiet.id));
            match self.cli.run(inv).await {
                Ok(out) => listed.push((remote.clone(), parse_ls_remote(&String::from_utf8_lossy(&out.stdout)))),
                Err(e) => tracing::warn!("couldn't list {remote}'s tags after a fetch: {}", crate::redact::redact(&e.message)),
            }
        }
        self.remote_tags.update(&self.data_dir, &h.common_dir, |known| known.extend(listed))
    }

    /// A tag push or delete through GitVolt succeeded: on `remote`, each tag now peels to its
    /// commit (`None`: it's gone). Only a remote whose list is known: an unknown one stays
    /// unknown, since one tag doesn't tell the rest. Whether anything changed.
    pub(crate) fn note_remote_tags(&self, common_dir: &Path, remote: &str, tags: Vec<(String, Option<String>)>) -> bool {
        self.remote_tags.update(&self.data_dir, common_dir, |known| {
            let Some(list) = known.get_mut(remote) else { return };
            for (tag, commit) in tags {
                match commit {
                    Some(c) => list.insert(tag, c),
                    None => list.remove(&tag),
                };
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestRepo;
    use crate::write::test_support::{api, call, open, repo, wt};
    use serde_json::{json, Value};

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";
    const C: &str = "3333333333333333333333333333333333333333";

    #[test]
    fn ls_remote_lines_parse_with_an_annotated_tags_peeled_commit() {
        let out = format!("{A}\trefs/tags/v1\n{B}\trefs/tags/v2\n{C}\trefs/tags/v2^{{}}\n{C}\trefs/heads/main\nnot a line\n{A}\trefs/tags/nested/v3\n");
        let tags = parse_ls_remote(&out);
        assert_eq!(tags, TagList::from([("v1".into(), A.into()), ("v2".into(), C.into()), ("nested/v3".into(), A.into())]));
        // The peeled line first (no order is promised): the peeled commit still wins.
        assert_eq!(parse_ls_remote(&format!("{C}\trefs/tags/v2^{{}}\n{B}\trefs/tags/v2\n")), TagList::from([("v2".into(), C.into())]));
        assert!(parse_ls_remote("").is_empty());
    }

    #[test]
    fn known_tags_persist_per_repository_and_a_corrupt_file_is_unknown() {
        let data = tempfile::tempdir().unwrap();
        let (one, two) = (data.path().join("one.git"), data.path().join("two.git"));
        let store = RemoteTags::default();
        assert!(store.get(data.path(), &one).is_empty());
        assert!(store.update(data.path(), &one, |k| { k.insert("origin".into(), TagList::from([("v1".into(), A.into())])); }));
        assert!(!store.update(data.path(), &one, |k| { k.insert("origin".into(), TagList::from([("v1".into(), A.into())])); }), "the same list: no change");
        assert!(store.get(data.path(), &two).is_empty());
        // A new session reads it back.
        let again = RemoteTags::default();
        assert_eq!(again.get(data.path(), &one)["origin"]["v1"], A);
        std::fs::write(file_of(data.path(), &one), b"{not json").unwrap();
        assert!(RemoteTags::default().get(data.path(), &one).is_empty());
    }

    /// main: one ← two, pushed. origin has v1 (lightweight) and v2 (annotated) at two, and a v4 at
    /// one; here v4 is at two, and v3 is only here. Returns (repo, one, two).
    fn tagged() -> (TestRepo, String, String) {
        let r = repo();
        let one = r.git(&["rev-parse", "HEAD"]);
        r.write("b.txt", "b\n");
        r.git(&["add", "b.txt"]);
        r.git(&["commit", "-q", "-m", "two"]);
        let two = r.git(&["rev-parse", "HEAD"]);
        r.add_origin();
        r.push("main");
        r.git(&["tag", "v1"]);
        r.tag("v2", "HEAD");
        r.git(&["tag", "v3"]);
        r.git(&["push", "-q", "origin", "v1", "v2", &format!("{one}:refs/tags/v4")]);
        r.git(&["tag", "v4"]);
        (r, one, two)
    }

    /// Each tag chip's `tagRemotes` on origin: (tag, state, commit), or (tag, "unknown", null).
    async fn chips(api: &Api, id: u32) -> Vec<(String, String, Value)> {
        let g = call(api, "graph", json!({ "repo": id, "limit": null })).await.unwrap();
        let mut out: Vec<(String, String, Value)> = g["labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|l| l["tag"] == true)
            .map(|l| {
                let name = l["name"].as_str().unwrap().to_string();
                match l.get("tagRemotes") {
                    None => (name, "unknown".into(), Value::Null),
                    Some(rs) => {
                        let o = rs.as_array().unwrap().iter().find(|r| r["remote"] == "origin").unwrap();
                        (name, o["state"].as_str().unwrap().to_string(), o.get("commit").cloned().unwrap_or(Value::Null))
                    }
                }
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    fn ls_remotes(api: &Api) -> usize {
        api.cli.log().entries().into_iter().filter(|e| e.args.iter().any(|a| a == "ls-remote")).count()
    }

    #[tokio::test]
    async fn a_fetch_lists_each_remotes_tags_and_the_chips_say_same_other_or_missing() {
        let data = tempfile::tempdir().unwrap();
        let (r, one, _) = tagged();
        let api = api(data.path());
        let id = open(&api, &r).await;
        let unknown = |t: &str| (t.to_string(), "unknown".to_string(), Value::Null);
        assert_eq!(chips(&api, id).await, [unknown("v1"), unknown("v2"), unknown("v3"), unknown("v4")], "before a fetch: unknown");
        call(&api, "fetch", json!({ "repo": id, "background": true })).await.unwrap();
        let same = |t: &str| (t.to_string(), "same".to_string(), Value::Null);
        // v2 is annotated: compared by the commit it peels to, on both sides.
        assert_eq!(chips(&api, id).await, [same("v1"), same("v2"), ("v3".into(), "missing".into(), Value::Null), ("v4".into(), "other".into(), json!(one))]);
        assert_eq!(ls_remotes(&api), 1);
        // A new session (the same data dir) knows them before its first fetch.
        let again = crate::write::test_support::api(data.path());
        let id2 = open(&again, &r).await;
        assert_eq!(chips(&again, id2).await[0], same("v1"));
    }

    /// The tag listing runs after the fetch; when it fails, the fetch still succeeds and the tags
    /// stay unknown.
    #[tokio::test]
    async fn a_failing_tag_listing_never_fails_the_fetch() {
        let data = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let (r, _, _) = tagged();
        // `ls-remote origin` outside any repository: "'origin' does not appear to be a git repository".
        let away = elsewhere.path().to_path_buf();
        let hook: crate::git::CommandHook = std::sync::Arc::new(move |cmd: &mut std::process::Command| {
            if cmd.get_args().any(|a| a == "ls-remote") {
                cmd.current_dir(&away);
            }
        });
        let cli = crate::git::GitCli::new(std::sync::Arc::new(crate::log::CommandLog::new(100))).with_env(crate::testing::isolated_git_env()).with_command_hook(hook);
        let api = Api::new(cli, None).with_data_dir(data.path().to_path_buf());
        let id = open(&api, &r).await;
        let out = call(&api, "fetch", json!({ "repo": id, "background": false })).await.unwrap();
        assert_eq!(out["status"], "done");
        assert_eq!(ls_remotes(&api), 1, "it was asked");
        assert!(chips(&api, id).await.iter().all(|c| c.1 == "unknown"));
    }

    /// Pull's own fetch lists its remote's tags too.
    #[tokio::test]
    async fn a_pull_lists_its_remotes_tags() {
        let data = tempfile::tempdir().unwrap();
        let (r, _, _) = tagged();
        let api = api(data.path());
        let id = open(&api, &r).await;
        let expect = json!({ "head": null, "refs": {} });
        call(&api, "pull", json!({ "repo": id, "worktree": wt(r.path()), "branch": "main", "mode": "ffOnly", "expect": expect, "confirm": { "autostash": false } })).await.unwrap();
        assert_eq!(ls_remotes(&api), 1);
        assert!(chips(&api, id).await.contains(&("v1".into(), "same".into(), Value::Null)));
    }

    /// A tag push or delete updates a known remote's list at once, with no request of its own; an
    /// unknown remote stays unknown.
    #[tokio::test]
    async fn pushing_and_deleting_a_tag_update_what_is_known_without_asking_again() {
        let data = tempfile::tempdir().unwrap();
        let (r, _, two) = tagged();
        let api = api(data.path());
        let id = open(&api, &r).await;
        call(&api, "pushTags", json!({ "repo": id, "worktree": wt(r.path()), "remote": "origin", "tag": "v3" })).await.unwrap();
        assert!(chips(&api, id).await.iter().all(|c| c.1 == "unknown"), "one tag doesn't tell the rest");
        call(&api, "fetch", json!({ "repo": id, "background": true })).await.unwrap();
        let asked = ls_remotes(&api);
        r.git(&["tag", "v5"]);
        call(&api, "pushTags", json!({ "repo": id, "worktree": wt(r.path()), "remote": "origin", "tag": "v5" })).await.unwrap();
        assert!(chips(&api, id).await.contains(&("v5".into(), "same".into(), Value::Null)));
        call(&api, "deleteTag", json!({ "repo": id, "worktree": wt(r.path()), "name": "v1", "local": false, "remote": "origin" })).await.unwrap();
        assert!(chips(&api, id).await.contains(&("v1".into(), "missing".into(), Value::Null)));
        // Every tag: the remote then has each one here at its commit (a different v4 is refused).
        r.git(&["tag", "-d", "v4"]);
        r.git(&["tag", "v6"]);
        call(&api, "pushTags", json!({ "repo": id, "worktree": wt(r.path()), "remote": "origin" })).await.unwrap();
        let known = api.known_remote_tags(&api.handle(id).unwrap());
        assert_eq!((known["origin"]["v1"].as_str(), known["origin"]["v6"].as_str()), (two.as_str(), two.as_str()));
        assert_eq!(ls_remotes(&api), asked, "no listing of its own");
    }
}
