//! Who "me" is for a repository: its `user.email` and the emails of the active profile's forge
//! accounts. Branch presets' "has commits by me" matches against these.

use crate::api::{Api, RepoHandle};
use crate::git::GitInvocation;
use std::path::PathBuf;
use std::time::SystemTime;

/// What `user.email` was read under: the repository's config file and the profile's extra
/// gitconfig. The cache is dropped when either changed (the watcher's config change shows up
/// as a new stamp), so a read costs a `stat`, and a git process only after a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigStamp {
    config: Option<(SystemTime, u64)>,
    include: Option<PathBuf>,
}

impl Api {
    /// The repository's `user.email` (through `GitCli`, so the profile's extra gitconfig applies)
    /// and every email of the active profile's forge accounts, lower-cased and de-duplicated.
    #[allow(dead_code)] // the graph reads call it from the next task
    pub(crate) async fn me_emails(&self, h: &RepoHandle) -> Vec<String> {
        let stamp = ConfigStamp {
            config: std::fs::metadata(h.common_dir.join("config")).ok().and_then(|m| Some((m.modified().ok()?, m.len()))),
            include: self.cli.include_path(),
        };
        let cached = h.me_email.lock().expect("me_email poisoned").clone().filter(|(s, _)| *s == stamp);
        let repo_email = match cached {
            Some((_, email)) => email,
            None => {
                // An unset email exits 1: no email, not an error.
                let out = self.cli.run(GitInvocation::new(&h.workdir, ["config", "--get", "user.email"]).ok_exit(1)).await;
                let email = out.ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|e| !e.is_empty());
                *h.me_email.lock().expect("me_email poisoned") = Some((stamp, email.clone()));
                email
            }
        };
        let mut out: Vec<String> = Vec::new();
        let accounts = self.store.active_profile().forge_accounts;
        for e in repo_email.into_iter().chain(accounts.into_iter().flat_map(|a| a.user.emails)) {
            let e = e.trim().to_lowercase();
            if !e.is_empty() && !out.contains(&e) {
                out.push(e);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::api::Api;
    use crate::forge::accounts::ForgeAccount;
    use crate::forge::types::{ForgeKind, ForgeUser, TokenStorage};
    use crate::git::GitCli;
    use crate::log::CommandLog;
    use crate::testing::{isolated_git_env, TestRepo};
    use std::sync::Arc;

    fn api() -> Api {
        Api::new(GitCli::new(Arc::new(CommandLog::new(100))).with_env(isolated_git_env()), None)
    }

    async fn handle(api: &Api, r: &TestRepo) -> Arc<crate::api::RepoHandle> {
        let id = api.open_repo(&r.path().display().to_string()).await.unwrap().id;
        api.handle(id).unwrap()
    }

    #[tokio::test]
    async fn me_is_the_repo_email_and_the_accounts_emails() {
        let r = TestRepo::new();
        r.git(&["config", "user.email", "robin@example.com"]);
        let api = api();
        let p = api.store().active_profile().id;
        let user = ForgeUser { id: 1, username: "robin".into(), name: "Robin".into(), avatar_url: None, web_url: String::new(), email: None, emails: vec!["Robin@Example.com".into(), "r@example.com".into()] };
        api.store().set_forge_accounts(&p, vec![ForgeAccount { host: "gitlab.example.com".into(), kind: ForgeKind::GitLab, user, storage: TokenStorage::File, version: None, version_checked_at: 0, added_at: 0 }]).unwrap();
        let h = handle(&api, &r).await;
        assert_eq!(api.me_emails(&h).await, ["robin@example.com", "r@example.com"]);
    }

    #[tokio::test]
    async fn me_follows_a_config_change() {
        let r = TestRepo::new();
        r.git(&["config", "user.email", "robin@example.com"]);
        let api = api();
        let h = handle(&api, &r).await;
        assert_eq!(api.me_emails(&h).await, ["robin@example.com"]);
        // A longer value, so the stamp differs even on a coarse-grained clock.
        r.git(&["config", "user.email", "robin.other@example.com"]);
        assert_eq!(api.me_emails(&h).await, ["robin.other@example.com"]);
    }
}
