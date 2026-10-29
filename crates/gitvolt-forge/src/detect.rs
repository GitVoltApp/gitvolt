//! Which forge a host runs, asked without a token, for a host whose name doesn't say
//! (`git.example.dev`): the account form's forge follows it.
//! - Forgejo (and Gitea, whose API it keeps) answers `GET /api/v1/version` with its version, or,
//!   where only signed-in users may call the API, with a 401 or 403 whose JSON says so;
//! - GitLab answers `GET /api/v4/version` with its version, or 401 `{"message":"401 Unauthorized"}`.
//!
//! Anything else (a redirect, a page, no answer) is neither. No token, cookie or redirect is sent
//! or followed, and only the host the user typed is asked.

use crate::gravatar::USER_AGENT;
use gitvolt_core::forge::ForgeKind;
use serde_json::Value;
use std::time::Duration;

/// The most of an answer read: a version or an error message is far less.
const MAX_BODY: u64 = 16 * 1024;

/// `GET /api/v1/version`'s answer is a Forgejo's (or a Gitea's): its version, or the API refused
/// to a visitor who isn't signed in.
pub fn forgejo_answer(status: u16, body: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(body) else { return false };
    match status {
        200 => v["version"].is_string(),
        401 | 403 => v["message"].as_str().is_some_and(|m| m.to_ascii_lowercase().contains("signed in")),
        _ => false,
    }
}

/// `GET /api/v4/version`'s answer is a GitLab's.
pub fn gitlab_answer(status: u16, body: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(body) else { return false };
    match status {
        200 => v["version"].is_string(),
        401 => v["message"].as_str() == Some("401 Unauthorized"),
        _ => false,
    }
}

/// The forge at `base` (`https://host`, no trailing slash), asked over the network: blocking, call
/// it off the runtime. `None`: neither Forgejo nor GitLab answered as one.
pub fn detect(base: &str) -> Option<ForgeKind> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .user_agent(USER_AGENT)
        .build()
        .into();
    let ask = |path: &str| -> Option<(u16, String)> {
        let mut resp = agent.get(&format!("{base}{path}")).header("Accept", "application/json").call().ok()?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().with_config().limit(MAX_BODY).read_to_vec().unwrap_or_default();
        Some((status, String::from_utf8_lossy(&body).into_owned()))
    };
    probe(ask)
}

/// Forgejo's question, then GitLab's; `ask` gives a path's status and body, `None` when nothing
/// answered. A host that didn't answer the first (no such name, refused, timed out) isn't asked
/// the second: an unreachable host costs one timeout, not two.
fn probe(ask: impl Fn(&str) -> Option<(u16, String)>) -> Option<ForgeKind> {
    let (status, body) = ask("/api/v1/version")?;
    if forgejo_answer(status, &body) {
        return Some(ForgeKind::Forgejo);
    }
    ask("/api/v4/version").is_some_and(|(s, b)| gitlab_answer(s, &b)).then_some(ForgeKind::GitLab)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{Canned, TestServer};

    #[test]
    fn forgejo_answers_with_its_version_or_its_signed_in_only_refusal() {
        assert!(forgejo_answer(200, r#"{"version":"16.0.0-dev-798-568c49e1+gitea-1.22.0"}"#));
        assert!(forgejo_answer(200, r#"{"version":"29.0.0+dev-160-g55186ca39b"}"#), "a Gitea");
        assert!(forgejo_answer(403, r#"{"message":"Only signed in user is allowed to call APIs."}"#));
        assert!(!forgejo_answer(302, "Found"));
        assert!(!forgejo_answer(404, "Not found."));
        assert!(!forgejo_answer(403, r#"{"message":"Forbidden"}"#));
        assert!(!forgejo_answer(200, "<html>a page</html>"));
    }

    #[test]
    fn gitlab_answers_its_version_or_401_unauthorized() {
        assert!(gitlab_answer(401, r#"{"message":"401 Unauthorized"}"#));
        assert!(gitlab_answer(200, r#"{"version":"18.9.1-ee","revision":"x"}"#));
        assert!(!gitlab_answer(404, "Not found."));
        assert!(!gitlab_answer(401, r#"{"message":"Only signed in user is allowed to call APIs."}"#));
    }

    #[tokio::test]
    async fn asks_forgejos_version_then_gitlabs() {
        let forgejo = TestServer::start(|_, head| {
            if head.starts_with("get /api/v1/version ") { Canned::json(403, r#"{"message":"Only signed in user is allowed to call APIs."}"#) } else { Canned::json(404, "{}") }
        });
        let b = forgejo.base.clone();
        assert_eq!(tokio::task::spawn_blocking(move || detect(&b)).await.unwrap(), Some(ForgeKind::Forgejo));
        let heads = forgejo.heads.lock().unwrap().clone();
        assert!(heads.iter().all(|h| !h.contains("authorization") && !h.contains("cookie")), "no token, no cookie");

        let gitlab = TestServer::start(|_, head| {
            if head.starts_with("get /api/v4/version ") { Canned::json(401, r#"{"message":"401 Unauthorized"}"#) } else { Canned::json(302, "") }
        });
        let b = gitlab.base.clone();
        assert_eq!(tokio::task::spawn_blocking(move || detect(&b)).await.unwrap(), Some(ForgeKind::GitLab));

        let neither = TestServer::start(|_, _| Canned::json(404, "Not found."));
        let b = neither.base.clone();
        assert_eq!(tokio::task::spawn_blocking(move || detect(&b)).await.unwrap(), None);
    }

    #[test]
    fn a_host_that_doesnt_answer_isnt_asked_again() {
        let asked = std::cell::RefCell::new(Vec::new());
        let unreachable = |path: &str| -> Option<(u16, String)> {
            asked.borrow_mut().push(path.to_string());
            None
        };
        assert_eq!(probe(unreachable), None);
        assert_eq!(*asked.borrow(), ["/api/v1/version"], "no second timeout");
        // An answer that isn't Forgejo's goes on to GitLab's question.
        asked.borrow_mut().clear();
        let gitlab = |path: &str| -> Option<(u16, String)> {
            asked.borrow_mut().push(path.to_string());
            Some(if path == "/api/v4/version" { (401, r#"{"message":"401 Unauthorized"}"#.into()) } else { (404, "Not found.".into()) })
        };
        assert_eq!(probe(gitlab), Some(ForgeKind::GitLab));
        assert_eq!(*asked.borrow(), ["/api/v1/version", "/api/v4/version"]);
        assert_eq!(detect(&crate::test_server::closed_base()), None, "refused");
    }
}
