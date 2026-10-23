//! Settings › Git identity (read-only): who commits are made as, and whether they're signed.

use crate::error::GbError;
use crate::git::{GitCli, GitInvocation};
use crate::write::commit::CommitIdentity;
use serde::Serialize;
use std::path::Path;
use ts_rs::TS;

/// What Settings › Git identity shows (read-only): who commits are made as, and whether they're signed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitIdentityInfo {
    pub identity: Option<CommitIdentity>,
    pub signing: Option<SigningInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SigningInfo {
    /// `openpgp` (git's default), `ssh` or `x509`.
    pub format: String,
    pub key: Option<String>,
}

/// With a worktree, git's own resolution there. Without one, the same reads in an empty directory
/// under the temp dir, with the temp dir as the ceiling so no surrounding repository's config is
/// read: global and system config plus the profile's extra config.
pub(crate) async fn git_identity(cli: &GitCli, root: Option<&Path>) -> Result<GitIdentityInfo, GbError> {
    let (dir, ceiling) = match root {
        Some(r) => (r.to_path_buf(), None),
        None => {
            let temp = std::env::temp_dir();
            let dir = temp.join("gitvolt-identity");
            std::fs::create_dir_all(&dir).map_err(|e| GbError::other(format!("couldn't create {}: {e}", dir.display())))?;
            (dir, Some(temp))
        }
    };
    let inv = |args: &[&str]| {
        let inv = GitInvocation::new(&dir, args.iter().copied());
        match &ceiling {
            Some(c) => inv.env("GIT_CEILING_DIRECTORIES", c),
            None => inv,
        }
    };
    let identity = crate::write::commit::identity_in(cli, inv(&["var", "GIT_AUTHOR_IDENT"])).await?;
    // An unset key exits 1 and a malformed value 128: neither is worth failing the card for.
    let config = |args: &[&str]| {
        let inv = inv(args).ok_exit(1);
        async move { cli.run(inv).await.ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty()) }
    };
    let signing = if config(&["config", "--get", "--type=bool", "commit.gpgSign"]).await.as_deref() == Some("true") {
        let format = config(&["config", "--get", "gpg.format"]).await.unwrap_or_else(|| "openpgp".into());
        Some(SigningInfo { format, key: config(&["config", "--get", "user.signingKey"]).await })
    } else {
        None
    };
    Ok(GitIdentityInfo { identity, signing })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestRepo;
    use std::sync::Arc;

    /// A runner whose environment names nobody: the tests' own `GIT_AUTHOR_*` would win over config.
    fn cli() -> GitCli {
        let env = [("GIT_CONFIG_GLOBAL", "/dev/null"), ("GIT_CONFIG_NOSYSTEM", "1"), ("LC_ALL", "C")].into_iter().map(|(k, v)| (k.into(), v.into())).collect();
        GitCli::new(Arc::new(crate::log::CommandLog::new(100))).with_env(env).with_command_hook(Arc::new(|cmd: &mut std::process::Command| {
            for var in ["GIT_AUTHOR_NAME", "GIT_AUTHOR_EMAIL", "GIT_COMMITTER_NAME", "GIT_COMMITTER_EMAIL", "EMAIL"] {
                cmd.env_remove(var);
            }
        }))
    }

    fn robin() -> Option<CommitIdentity> {
        Some(CommitIdentity { name: "Robin Example".into(), email: "robin@example.com".into() })
    }

    #[tokio::test]
    async fn repo_local_identity_and_signing_are_read() {
        let r = TestRepo::new();
        r.git(&["config", "user.name", "Robin Example"]);
        r.git(&["config", "user.email", "robin@example.com"]);
        let info = git_identity(&cli(), Some(r.path())).await.unwrap();
        assert_eq!(info.identity, robin());
        assert_eq!(info.signing, None, "gpgSign unset");
        r.git(&["config", "commit.gpgSign", "true"]);
        r.git(&["config", "gpg.format", "ssh"]);
        r.git(&["config", "user.signingKey", "/keys/robin.pub"]);
        let info = git_identity(&cli(), Some(r.path())).await.unwrap();
        assert_eq!(info.signing, Some(SigningInfo { format: "ssh".into(), key: Some("/keys/robin.pub".into()) }));
    }

    #[tokio::test]
    async fn without_a_repo_only_global_and_profile_config_count() {
        let other = TestRepo::new();
        other.git(&["config", "user.name", "Someone Else"]);
        other.git(&["config", "user.email", "else@example.com"]);
        let dir = tempfile::tempdir().unwrap();
        let inc = dir.path().join("profile.gitconfig");
        std::fs::write(&inc, "[user]\n\tname = Robin Example\n\temail = robin@example.com\n").unwrap();
        let cli = cli();
        cli.set_include_path(Some(inc));
        assert_eq!(git_identity(&cli, None).await.unwrap().identity, robin());
    }
}
