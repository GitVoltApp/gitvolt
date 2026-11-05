//! Forge accounts (spec #4 §3.2): any number per host per profile, kept in the profile (never
//! the token: that's in the token store), with what the UI shows about each.

use crate::error::{GbError, GbErrorKind};
use crate::forge::types::{ForgeKind, ForgeUser, TokenStorage};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ForgeAccount {
    /// Names the account in its profile, whatever the host (`RepoSettings::forge_account` keeps
    /// it): the host itself for the account a host had first, `<host>#<user id>` for the others
    /// (`new_account_id`). The token store and the forge cache name its entries by it
    /// (`AccountKey::keyring_account`), so it never changes: when a host's first account is
    /// removed, the others keep theirs. Empty in a profile from 1.5.0 or before (one account per
    /// host): `assign_account_ids` gives it the host on load.
    #[serde(default)]
    pub id: String,
    pub host: String,
    pub kind: ForgeKind,
    /// The token's user when the account was added (avatar and name in Settings).
    pub user: ForgeUser,
    pub storage: TokenStorage,
    /// GitLab's version (`19.1.0-ee`), read at add and once a day (spec #4 §3.2).
    // `default`: a profile file from an older or newer GitVolt still loads.
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    #[ts(type = "number")]
    pub version_checked_at: i64,
    #[serde(default)]
    #[ts(type = "number")]
    pub added_at: i64,
}

/// What the account's last requests said (no request is made to show it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum AccountStatus {
    Ok,
    RateLimited {
        #[ts(type = "number")]
        until: i64,
    },
    AuthFailed { message: String },
    Unreachable { message: String },
    /// The store has no token for it (the keyring was reset, the file deleted).
    TokenMissing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ForgeAccountView {
    pub account: ForgeAccount,
    pub status: AccountStatus,
}

/// The id a new account on `host` gets: the host itself while the host has no account (its token
/// then has 1.5.0's keyring name, so a downgrade still finds it), else `<host>#<user id>`.
pub fn new_account_id(accounts: &[ForgeAccount], host: &str, user_id: u64) -> String {
    let taken = |id: &str| accounts.iter().any(|a| a.id == id);
    if !accounts.iter().any(|a| a.host == host) && !taken(host) {
        return host.to_string();
    }
    let id = format!("{host}#{user_id}");
    if !taken(&id) {
        return id;
    }
    (2..).map(|n| format!("{host}#{user_id}-{n}")).find(|id| !taken(id)).expect("an unused id")
}

/// Gives every account an id on load (`ForgeAccount::id`); true when one changed. A profile from
/// 1.5.0 or before has none: each host's account gets the host, its token's name then. After a
/// downgrade saved the profile without them, a host's first account gets the host again and the
/// others `<host>#<user id>`, the names their tokens were saved under. Ids stay unique.
pub fn assign_account_ids(accounts: &mut [ForgeAccount]) -> bool {
    let mut changed = false;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for i in 0..accounts.len() {
        let a = &accounts[i];
        let valid = a.id == a.host || a.id.strip_prefix(a.host.as_str()).is_some_and(|rest| rest.starts_with('#'));
        if valid && !seen.contains(&a.id) {
            seen.insert(a.id.clone());
            continue;
        }
        // The host's own name only while no other account has it, here or further down.
        let host_free = !seen.contains(&a.host) && !accounts.iter().any(|b| b.id == b.host && b.host == a.host);
        // A development build named the others by the user id alone (`42`, token `<host>#42`).
        let bare_user = (!a.id.is_empty() && a.id.bytes().all(|b| b.is_ascii_digit())).then(|| format!("{}#{}", a.host, a.id)).filter(|id| !seen.contains(id));
        let id = if let Some(id) = bare_user {
            id
        } else if host_free {
            a.host.clone()
        } else {
            let base = format!("{}#{}", a.host, a.user.id);
            std::iter::once(base.clone()).chain((2..).map(|n| format!("{base}-{n}"))).find(|id| !seen.contains(id) && !accounts.iter().any(|b| &b.id == id)).expect("an unused id")
        };
        seen.insert(id.clone());
        accounts[i].id = id;
        changed = true;
    }
    changed
}

const HOST_ONLY: &str = "Enter the host only, like gitlab.example.com";

/// What the Host box holds, as a host: lowercase, without a scheme, a path or a query; a port
/// stays. Credentials, spaces and an scp-style URL are refused.
pub fn normalize_host(input: &str) -> Result<String, GbError> {
    let bad = || GbError::new(GbErrorKind::InvalidInput, HOST_ONLY);
    let s = input.trim();
    let s = s.split_once("://").map(|(_, rest)| rest).unwrap_or(s);
    let authority = s.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.to_ascii_lowercase();
    let (name, port) = match host.split_once(':') {
        Some((n, p)) => (n, Some(p)),
        None => (host.as_str(), None),
    };
    let name_ok = !name.is_empty() && !name.starts_with(['-', '.']) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    if !name_ok || !port_ok {
        return Err(bad());
    }
    Ok(host)
}

/// GitHub accounts are github.com's (GitHub Enterprise Server is out of scope, spec #4 §9); GitLab
/// and Forgejo accounts are on any host.
pub fn check_kind_host(kind: ForgeKind, host: &str) -> Result<(), GbError> {
    if kind == ForgeKind::GitHub && host != "github.com" {
        return Err(GbError::new(GbErrorKind::InvalidInput, "GitHub accounts are for github.com (GitHub Enterprise Server isn't supported)"));
    }
    Ok(())
}

/// The forge's "new token" page, prefilled with the write scope GitVolt needs (spec #4 §2).
pub fn token_page_url(kind: ForgeKind, host: &str) -> String {
    token_page_url_for(kind, host, false)
}

/// Like [`token_page_url`]; `classic` picks GitHub's classic-token page (no lifetime policy,
/// unlike orgs that cap fine-grained tokens). GitLab has no such choice. Forgejo's new-token form
/// takes nothing prefilled: the scopes (`read:user`, `write:repository`, `write:issue`) are picked there.
pub fn token_page_url_for(kind: ForgeKind, host: &str, classic: bool) -> String {
    const DESCRIPTION: &str = "name=GitVolt&description=GitVolt%20desktop%20client";
    match kind {
        ForgeKind::GitLab => format!("https://{host}/-/user_settings/personal_access_tokens?{DESCRIPTION}&scopes=api"),
        ForgeKind::GitHub if classic => "https://github.com/settings/tokens/new?description=GitVolt%20desktop%20client&scopes=repo,read:org".to_string(),
        ForgeKind::GitHub => format!("https://github.com/settings/personal-access-tokens/new?{DESCRIPTION}&pull_requests=write&issues=write&checks=read&statuses=read&metadata=read&profile=read"),
        ForgeKind::Forgejo => format!("https://{host}/user/settings/applications/tokens/new"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_box_accepts_a_url_and_keeps_only_the_host() {
        assert_eq!(normalize_host(" https://GitLab.Example.com/group/project ").unwrap(), "gitlab.example.com");
        assert_eq!(normalize_host("gitlab.example.com/").unwrap(), "gitlab.example.com");
        assert_eq!(normalize_host("http://gitlab.example.com:8443?x").unwrap(), "gitlab.example.com:8443");
        for bad in ["", "   ", "https://user:pw@gitlab.example.com", "git@gitlab.example.com:group/x.git", "gitlab example.com", "-gitlab.example.com", "gitlab.example.com:port"] {
            let e = normalize_host(bad).unwrap_err();
            assert_eq!((e.kind, e.message.as_str()), (GbErrorKind::InvalidInput, "Enter the host only, like gitlab.example.com"), "{bad:?}");
        }
        assert!(check_kind_host(ForgeKind::GitHub, "github.com").is_ok());
        assert!(check_kind_host(ForgeKind::GitLab, "gitlab.example.com").is_ok());
        assert!(check_kind_host(ForgeKind::Forgejo, "code.example.org").is_ok(), "Forgejo on any host");
        assert_eq!(check_kind_host(ForgeKind::GitHub, "github.example.com").unwrap_err().message, "GitHub accounts are for github.com (GitHub Enterprise Server isn't supported)");
    }

    #[test]
    fn create_token_pages_are_prefilled_with_write_scope() {
        assert_eq!(token_page_url(ForgeKind::GitLab, "gitlab.example.com"), "https://gitlab.example.com/-/user_settings/personal_access_tokens?name=GitVolt&description=GitVolt%20desktop%20client&scopes=api");
        assert_eq!(token_page_url_for(ForgeKind::GitHub, "github.com", true), "https://github.com/settings/tokens/new?description=GitVolt%20desktop%20client&scopes=repo,read:org");
        assert_eq!(token_page_url_for(ForgeKind::GitLab, "gitlab.example.com", true), token_page_url(ForgeKind::GitLab, "gitlab.example.com"));
        assert_eq!(token_page_url(ForgeKind::GitHub, "github.com"), "https://github.com/settings/personal-access-tokens/new?name=GitVolt&description=GitVolt%20desktop%20client&pull_requests=write&issues=write&checks=read&statuses=read&metadata=read&profile=read");
        assert_eq!(token_page_url(ForgeKind::Forgejo, "code.example.org:3000"), "https://code.example.org:3000/user/settings/applications/tokens/new");
    }

    #[test]
    fn an_account_saved_without_the_optional_fields_still_loads() {
        let profile = serde_json::json!({
            "version": 1, "id": "default", "name": "Default",
            "forgeAccounts": [{
                "host": "gitlab.example.com", "kind": "gitlab", "storage": "keyring",
                "user": {"id": 7, "username": "ada", "name": "Ada"},
                "somethingNewer": true,
            }],
        });
        let profile: crate::settings::Profile = serde_json::from_value(profile).unwrap();
        let a = profile.forge_accounts.into_iter().next().unwrap();
        assert_eq!(a.id, "", "1.5.0's account has no id until the store gives it one (`assign_account_ids`)");
        assert_eq!((a.version, a.version_checked_at, a.added_at), (None, 0, 0));
        assert_eq!((a.user.avatar_url, a.user.email, a.user.web_url.as_str()), (None, None, ""));
    }

    fn on(host: &str, id: &str, user: u64) -> ForgeAccount {
        let user = ForgeUser { id: user, username: format!("user{user}"), name: format!("User {user}"), avatar_url: None, web_url: String::new(), email: None, emails: vec![] };
        ForgeAccount { id: id.into(), host: host.into(), kind: ForgeKind::GitLab, user, storage: TokenStorage::Keyring, version: None, version_checked_at: 0, added_at: 0 }
    }

    #[test]
    fn a_hosts_first_account_is_named_by_the_host_and_the_others_by_their_user() {
        const H: &str = "gitlab.example.com";
        assert_eq!(new_account_id(&[], H, 7), H);
        assert_eq!(new_account_id(&[on("code.example.org", "code.example.org", 7)], H, 7), H, "another host's account doesn't count");
        assert_eq!(new_account_id(&[on(H, H, 7)], H, 8), "gitlab.example.com#8");
        // The host's first account was removed: the others keep their names, and a new one too.
        assert_eq!(new_account_id(&[on(H, "gitlab.example.com#8", 8)], H, 9), "gitlab.example.com#9");
        assert_eq!(new_account_id(&[on(H, H, 7), on(H, "gitlab.example.com#8", 8)], H, 8), "gitlab.example.com#8-2", "never a taken one");
    }

    #[test]
    fn loading_gives_every_account_a_unique_id() {
        const H: &str = "gitlab.example.com";
        // 1.5.0: one account per host, no ids.
        let mut old = vec![on(H, "", 7), on("code.example.org", "", 7)];
        assert!(assign_account_ids(&mut old));
        assert_eq!(old.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), [H, "code.example.org"]);
        assert!(!assign_account_ids(&mut old), "nothing more to do");
        // 1.5.0 saved two accounts on a host without their ids: never both the host's name.
        let mut stripped = vec![on(H, "", 7), on(H, "", 8)];
        assert!(assign_account_ids(&mut stripped));
        assert_eq!(stripped.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), [H, "gitlab.example.com#8"]);
        // Another account already has the host's name: the id-less one doesn't take it.
        let mut later = vec![on(H, "", 8), on(H, H, 7)];
        assert!(assign_account_ids(&mut later));
        assert_eq!(later.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["gitlab.example.com#8", H]);
        // A duplicate id is renamed.
        let mut twice = vec![on(H, H, 7), on(H, H, 8)];
        assert!(assign_account_ids(&mut twice));
        assert_eq!(twice.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), [H, "gitlab.example.com#8"]);
        // A development build's ids: the user id alone, `legacy:<host>`.
        let mut dev = vec![on(H, "legacy:gitlab.example.com", 7), on(H, "8", 8)];
        assert!(assign_account_ids(&mut dev));
        assert_eq!(dev.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), [H, "gitlab.example.com#8"]);
    }

    #[test]
    fn statuses_serialize_tagged() {
        assert_eq!(serde_json::to_value(AccountStatus::RateLimited { until: 5 }).unwrap(), serde_json::json!({"kind": "rateLimited", "until": 5}));
        assert_eq!(serde_json::to_value(AccountStatus::TokenMissing).unwrap(), serde_json::json!({"kind": "tokenMissing"}));
    }
}
