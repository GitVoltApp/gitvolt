//! Forge pages by URL template (`forge/urls.ts`), no API call. Only `https`, as the web has it, and only remotes the
//! repository already names.

use crate::app::Tab;
use gitvolt_core::forge::ForgeKind;

/// A remote resolved down to what forge URLs need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRemote {
    pub host: String,
    pub path: String,
    pub kind: ForgeKind,
}

/// `encodeURIComponent` per `/`-separated segment: a branch or path with `#` or spaces can't break the URL.
fn encode_segments(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for b in p.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' | b'/' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn is_full_sha(r: &str) -> bool {
    (r.len() == 40 || r.len() == 64) && r.bytes().all(|b| b.is_ascii_hexdigit())
}

fn base(r: &ProjectRemote, project: Option<&str>) -> String {
    format!("https://{}/{}", r.host, project.unwrap_or(&r.path))
}

/// A file's page with an optional line anchor: GitLab's range has one `L`, GitHub's and Forgejo's two.
pub fn file_url(r: &ProjectRemote, at: &str, file: &str, lines: Option<(u32, u32)>) -> String {
    let (at_enc, f) = (encode_segments(at), encode_segments(file));
    let url = match r.kind {
        ForgeKind::GitLab => format!("{}/-/blob/{at_enc}/{f}", base(r, None)),
        ForgeKind::Forgejo => format!("{}/src/{}/{at_enc}/{f}", base(r, None), if is_full_sha(at) { "commit" } else { "branch" }),
        ForgeKind::GitHub => format!("{}/blob/{at_enc}/{f}", base(r, None)),
    };
    let Some((a, b)) = lines else { return url };
    let end = match (a == b, r.kind) {
        (true, _) => String::new(),
        (false, ForgeKind::GitLab) => format!("-{b}"),
        (false, _) => format!("-L{b}"),
    };
    format!("{url}#L{a}{end}")
}

pub fn repo_url(r: &ProjectRemote) -> String {
    base(r, None)
}

pub fn branch_url(r: &ProjectRemote, branch: &str) -> String {
    let enc = encode_segments(branch);
    match r.kind {
        ForgeKind::Forgejo => format!("{}/src/branch/{enc}", base(r, None)),
        ForgeKind::GitLab => format!("{}/-/tree/{enc}", base(r, None)),
        ForgeKind::GitHub => format!("{}/tree/{enc}", base(r, None)),
    }
}

pub fn commit_url(r: &ProjectRemote, sha: &str) -> String {
    match r.kind {
        ForgeKind::GitLab => format!("{}/-/commit/{sha}", base(r, None)),
        _ => format!("{}/commit/{sha}", base(r, None)),
    }
}

pub fn merge_request_url(r: &ProjectRemote, project: Option<&str>, n: u64) -> String {
    let b = base(r, project);
    match r.kind {
        ForgeKind::Forgejo => format!("{b}/pulls/{n}"),
        ForgeKind::GitLab => format!("{b}/-/merge_requests/{n}"),
        ForgeKind::GitHub => format!("{b}/pull/{n}"),
    }
}

/// The commit message's `Open !n`: a whole reference label (`!42`, `group/sub/project!7`, `#12`), when it's an MR/PR on
/// this forge kind: GitLab's `!`, GitHub's and Forgejo's `#`. Its label and URL.
pub fn mr_ref_url(label: &str, r: &ProjectRemote) -> Option<String> {
    let at = label.find(['!', '#'])?;
    let project = (at > 0).then(|| &label[..at]);
    let n: u64 = label[at + 1..].parse().ok()?;
    ((label.as_bytes()[at] == b'!') == (r.kind == ForgeKind::GitLab)).then(|| merge_request_url(r, project, n))
}

/// `env.forge`: the remote the forge links point at. `remote` names one, else the first that parses; remotes arrive
/// `origin` first. No forge kind for its host: no links.
pub fn project_remote(tab: &Tab, overrides: &serde_json::Value, remote: Option<&str>) -> Option<ProjectRemote> {
    let list = tab.info["remotes"].as_array()?;
    let r = list.iter().filter(|r| remote.is_none_or(|n| r["name"].as_str() == Some(n))).find(|r| r["host"].as_str().is_some() && r["path"].as_str().is_some())?;
    let host = r["host"].as_str()?;
    let detected = r["hostKind"].as_str().unwrap_or_else(|| super::text::detect_host_kind(host));
    let kind = super::text::kind_of(super::text::effective_kind(host, detected, overrides))?;
    Some(ProjectRemote { host: host.to_string(), path: r["path"].as_str()?.to_string(), kind })
}

/// `upstreamOf`: `(remote, branch)` of the branch `sha` is on: the branches whose tip it is (HEAD's first), else the
/// branch it belongs to. A local one's is its configured upstream; a remote-tracking one is its own.
pub fn upstream_of(tab: &Tab, sha: &str) -> Option<(String, String)> {
    let g = tab.graph.as_ref()?;
    let i = g.rows.iter().position(|r| r.id.as_ref() == sha)?;
    let split = |full: &str, remote: &str| full.strip_prefix(&format!("refs/remotes/{remote}/")).map(|b| (remote.to_string(), b.to_string()));
    let configured = |name: &str| -> Option<Option<(String, String)>> {
        let side = tab.side.sections.iter().flat_map(|s| &s.items).find_map(|it| match &it.kind {
            crate::sidebar::ItemKind::Local { upstream, gone, .. } if it.name == name => Some((upstream.clone(), *gone)),
            _ => None,
        })?;
        Some(match side {
            (Some(u), false) => {
                let short = u.strip_prefix("refs/remotes/")?;
                let names: Vec<String> = tab.remote_groups().into_iter().map(|g| g.0).collect();
                let remote = names.iter().filter(|n| short.starts_with(&format!("{n}/"))).max_by_key(|n| n.len())?;
                Some((remote.clone(), short[remote.len() + 1..].to_string()))
            }
            _ => None,
        })
    };
    let same_named = |name: &str| -> Option<(String, String)> {
        let chips = g.rows.iter().flat_map(|r| &r.chips).filter(|c| !c.tag);
        let mut found: Vec<(String, String)> = chips.flat_map(|c| c.remotes.iter()).filter_map(|(remote, full)| split(full, remote)).filter(|(_, b)| b == name).collect();
        found.sort_by_key(|(r, _)| r != "origin");
        found.into_iter().next()
    };
    let mut chips: Vec<_> = g.rows[i].chips.iter().filter(|c| !c.tag).collect();
    chips.sort_by_key(|c| !c.head);
    for c in chips {
        let cfg = if c.local { configured(&c.name) } else { None };
        if let Some(Some(u)) = cfg {
            return Some(u);
        }
        if matches!(cfg, Some(None)) {
            continue;
        }
        let mut own: Vec<_> = c.remotes.iter().collect();
        own.sort_by_key(|(r, _)| r != "origin");
        if let Some(u) = own.first().and_then(|(r, f)| split(f, r)) {
            return Some(u);
        }
        if c.local
            && let Some(u) = same_named(&c.name)
        {
            return Some(u);
        }
    }
    let m = g.membership.get(i)?.as_ref()?;
    if let Some(name) = m.full_ref.strip_prefix("refs/heads/") {
        return match configured(name) {
            Some(u) => u,
            None => same_named(name),
        };
    }
    let r = g.rows.iter().flat_map(|r| &r.chips).flat_map(|c| c.remotes.iter()).find(|(_, f)| *f == m.full_ref)?;
    split(&r.1, &r.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(kind: ForgeKind) -> ProjectRemote {
        ProjectRemote { host: "forge.example.com".into(), path: "group/project".into(), kind }
    }

    #[test]
    fn file_links_differ_by_forge() {
        let f = |k| file_url(&at(k), "feat/a b", "src/x#1.rs", Some((3, 7)));
        assert_eq!(f(ForgeKind::GitLab), "https://forge.example.com/group/project/-/blob/feat/a%20b/src/x%231.rs#L3-7");
        assert_eq!(f(ForgeKind::GitHub), "https://forge.example.com/group/project/blob/feat/a%20b/src/x%231.rs#L3-L7");
        assert_eq!(f(ForgeKind::Forgejo), "https://forge.example.com/group/project/src/branch/feat/a%20b/src/x%231.rs#L3-L7");
        let sha = "a".repeat(40);
        assert_eq!(file_url(&at(ForgeKind::Forgejo), &sha, "x", Some((3, 3))), format!("https://forge.example.com/group/project/src/commit/{sha}/x#L3"));
    }

    #[test]
    fn other_pages() {
        assert_eq!(branch_url(&at(ForgeKind::GitLab), "main"), "https://forge.example.com/group/project/-/tree/main");
        assert_eq!(branch_url(&at(ForgeKind::Forgejo), "main"), "https://forge.example.com/group/project/src/branch/main");
        assert_eq!(commit_url(&at(ForgeKind::GitHub), "abc"), "https://forge.example.com/group/project/commit/abc");
        assert_eq!(merge_request_url(&at(ForgeKind::Forgejo), None, 4), "https://forge.example.com/group/project/pulls/4");
        assert_eq!(merge_request_url(&at(ForgeKind::GitLab), Some("o/p"), 4), "https://forge.example.com/o/p/-/merge_requests/4");
    }

    #[test]
    fn a_message_ref_is_an_mr_only_as_its_forge_writes_it() {
        assert_eq!(mr_ref_url("!42", &at(ForgeKind::GitLab)).as_deref(), Some("https://forge.example.com/group/project/-/merge_requests/42"));
        assert_eq!(mr_ref_url("#42", &at(ForgeKind::GitLab)), None);
        assert_eq!(mr_ref_url("!42", &at(ForgeKind::GitHub)), None);
        assert_eq!(mr_ref_url("o/p#42", &at(ForgeKind::GitHub)).as_deref(), Some("https://forge.example.com/o/p/pull/42"));
    }
}
