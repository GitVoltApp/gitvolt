//! Where an account's API, web pages and avatars are (spec #4 §3.1). The harness replaces them
//! per host with its fake forge's (`ForgeConfig.overrides`, T10).

use gitvolt_core::forge::ForgeKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEndpoints {
    /// `https://gitlab.example.com/api/v4`, `https://api.github.com`, `https://codeberg.org/api/v1`.
    /// No trailing slash.
    pub api: String,
    /// `https://gitlab.example.com`, `https://github.com`.
    pub web: String,
    /// GitHub's avatar host; `None` for GitLab and Forgejo (their avatars are under `web`).
    pub avatars: Option<String>,
}

pub fn default_endpoints(kind: ForgeKind, host: &str) -> HostEndpoints {
    match kind {
        ForgeKind::GitLab => HostEndpoints { api: format!("https://{host}/api/v4"), web: format!("https://{host}"), avatars: None },
        ForgeKind::GitHub => HostEndpoints { api: "https://api.github.com".into(), web: "https://github.com".into(), avatars: Some("https://avatars.githubusercontent.com".into()) },
        ForgeKind::Forgejo => HostEndpoints { api: format!("https://{host}/api/v1"), web: format!("https://{host}"), avatars: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forgejo_is_on_its_own_host_under_api_v1() {
        assert_eq!(default_endpoints(ForgeKind::Forgejo, "forgejo.example.org:3000"), HostEndpoints { api: "https://forgejo.example.org:3000/api/v1".into(), web: "https://forgejo.example.org:3000".into(), avatars: None });
    }
}
