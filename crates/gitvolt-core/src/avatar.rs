//! Avatar lookup (spec §4.2). Implemented by `gitvolt-forge` (Gravatar in #1, forge APIs in #4)
//! and injected into `Api`, so core never depends on an HTTP client.

use crate::error::GbError;
use serde::Serialize;
use std::future::Future;
use std::pin::Pin;
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AvatarPayload {
    pub mime: String,
    pub base64: String,
}

/// GitHub's noreply addresses: `noreply@github.com` (the committer of every commit made in
/// GitHub's web UI) and `[<id>+]<login>@users.noreply.github.com`. They never receive mail, so
/// no Gravatar account can be verified for one: asking Gravatar is a round trip for nothing. Only
/// a GitHub account can show one (by the user id in the address).
pub fn is_github_noreply(email: &str) -> bool {
    let email = email.trim().to_ascii_lowercase();
    email == GITHUB_WEB_FLOW || email.ends_with(NOREPLY_DOMAIN)
}

/// The committer of commits made in GitHub's web UI: everyone's, so no one person's picture.
pub const GITHUB_WEB_FLOW: &str = "noreply@github.com";
const NOREPLY_DOMAIN: &str = "@users.noreply.github.com";
/// GitHub's user ids are 9 digits today; longer is not an id.
const MAX_ID_DIGITS: usize = 12;

/// The user id in a `<id>+<login>@users.noreply.github.com` address (the domain in any case):
/// ASCII digits without a leading zero, then a login of 1–39 letters, digits or hyphens, so a
/// second `+` part is refused. The id is all an account-less lookup sends
/// (`AvatarProvider::github_noreply_avatar`).
pub fn github_noreply_id(email: &str) -> Option<u64> {
    let email = email.trim();
    let at = email.len().checked_sub(NOREPLY_DOMAIN.len())?;
    let (local, domain) = (email.get(..at)?, email.get(at..)?);
    let (id, login) = local.split_once('+')?;
    let ok = domain.eq_ignore_ascii_case(NOREPLY_DOMAIN)
        && (1..=MAX_ID_DIGITS).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_digit())
        && !id.starts_with('0')
        && (1..=39).contains(&login.len())
        && login.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if ok { id.parse().ok() } else { None }
}

pub type AvatarFuture<'a> = Pin<Box<dyn Future<Output = Result<Option<AvatarPayload>, GbError>> + Send + 'a>>;

pub trait AvatarProvider: Send + Sync {
    /// `Ok(None)`: there's no avatar for this email (the UI shows initials).
    fn avatar<'a>(&'a self, email: &'a str) -> AvatarFuture<'a>;

    /// The "load avatars" setting (spec §14.1): off answers every lookup with `None`, without a
    /// request. Providers that always answer from a local source ignore it.
    fn set_enabled(&self, _on: bool) {}

    /// The last step for a `<id>+<login>@users.noreply.github.com` email, when nothing else found
    /// an avatar: GitHub's public picture for that user id, by an anonymous request that sends
    /// only the id (`github_noreply_id`). Off with "load avatars", like `avatar`; the caller also
    /// checks the forge-avatars setting.
    fn github_noreply_avatar<'a>(&'a self, _email: &'a str) -> AvatarFuture<'a> {
        Box::pin(async { Ok(None) })
    }
}

#[cfg(test)]
mod tests {
    use super::github_noreply_id;

    #[test]
    fn the_noreply_id_is_digits_only_before_a_single_login() {
        assert_eq!(github_noreply_id("583231+octocat@users.noreply.github.com"), Some(583231));
        assert_eq!(github_noreply_id(" 583231+Octo-Cat@Users.NoReply.GitHub.COM "), Some(583231), "any case, trimmed");
        for not in [
            "octocat@users.noreply.github.com",
            "+octocat@users.noreply.github.com",
            "58a231+octocat@users.noreply.github.com",
            "-5+octocat@users.noreply.github.com",
            "+5+octocat@users.noreply.github.com",
            "0583231+octocat@users.noreply.github.com",
            "1234567890123+octocat@users.noreply.github.com",
            "99999999999999999999999+octocat@users.noreply.github.com",
            "583231+@users.noreply.github.com",
            "583231+octo+cat@users.noreply.github.com",
            "583231+octo@cat@users.noreply.github.com",
            "583231+octocat@noreply.github.com",
            "583231+octocat@users.noreply.github.com.example",
            "583231+octocat@example.com",
            "noreply@github.com",
            "",
        ] {
            assert_eq!(github_noreply_id(not), None, "{not}");
        }
    }
}
