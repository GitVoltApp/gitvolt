//! Paste a forge page or an MR's number (`ui/src/goto`): a page of a project an open tab has, or a number on the active one.
//! A URL is only read here: nothing fetches it, and only a click on the toast's Open opens it.

use super::text::{forge_name, mr_long_noun, mr_ref};
use super::urls::{ProjectRemote, merge_request_url};
use crate::app::GitVolt;
use gitvolt_core::api::Request;
use gitvolt_core::forge::ForgeKind;
use gpui::*;

/// A forge page's project: `host[:port]` in lower case, and its path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRef {
    pub host: String,
    pub project: String,
    pub forge: ForgeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForgeTarget {
    /// A forge URL (`url`, `at`), or `!12` / `#12` / `group/project!12` (`sigil`).
    Mr { number: u64, url: Option<String>, at: Option<ProjectRef>, project: Option<String>, sigil: Option<char> },
    Commit { sha: String, url: String, at: ProjectRef },
    /// The ref and the path, decoded segments: a ref may have slashes.
    File { ref_path: Vec<String>, lines: Option<(u32, u32)>, url: String, at: ProjectRef },
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(n) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(n);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// `#L12`, `#L12-20`, `#L12-L20`; reversed ranges are put in order.
fn anchor_lines(hash: &str) -> Option<(u32, u32)> {
    let rest = hash.strip_prefix("#L").or(hash.strip_prefix("#l"))?;
    let (a, b) = rest.split_once('-').map_or((rest, None), |(a, b)| (a, Some(b)));
    let a: u32 = a.parse().ok()?;
    let b: u32 = match b {
        Some(b) => b.trim_start_matches(['L', 'l']).parse().ok()?,
        None => a,
    };
    (a >= 1 && b >= 1).then(|| (a.min(b), a.max(b)))
}

fn hex_sha(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn number_after(rest: &str, prefix: &str) -> Option<u64> {
    let tail = rest.strip_prefix(prefix)?;
    tail.split('/').next()?.parse().ok()
}

fn sha_after(rest: &str, prefix: &str) -> Option<String> {
    let tail = rest.strip_prefix(prefix)?;
    let s = tail.split('/').next()?;
    hex_sha(s).then(|| s.to_lowercase())
}

/// The page below a project's URL (`forgePageOf`).
enum Page {
    Mr(u64),
    Commit(String),
    Blob(String),
}

fn page_of(kind: ForgeKind, rest: &str) -> Option<Page> {
    let (mr, commit) = match kind {
        ForgeKind::GitLab => ("-/merge_requests/", "-/commit/"),
        ForgeKind::GitHub => ("pull/", "commit/"),
        ForgeKind::Forgejo => ("pulls/", "commit/"),
    };
    if let Some(n) = number_after(rest, mr) {
        return Some(Page::Mr(n));
    }
    if let Some(s) = sha_after(rest, commit) {
        return Some(Page::Commit(s));
    }
    let blob = match kind {
        ForgeKind::GitLab => rest.strip_prefix("-/blob/"),
        ForgeKind::GitHub => rest.strip_prefix("blob/"),
        ForgeKind::Forgejo => ["src/branch/", "src/commit/", "src/tag/"].iter().find_map(|p| rest.strip_prefix(p)),
    }?;
    // The ref, and at least one path segment.
    blob.split_once('/').is_some_and(|(a, b)| !a.is_empty() && !b.is_empty()).then(|| Page::Blob(blob.to_string()))
}

/// What an `http(s)` URL names on a forge; `None` when it's no such page.
pub fn classify_url(s: &str) -> Option<ForgeTarget> {
    let scheme_end = s.find("://")?;
    if !matches!(s[..scheme_end].to_ascii_lowercase().as_str(), "http" | "https") {
        return None;
    }
    let after = &s[scheme_end + 3..];
    let (no_hash, hash) = after.split_once('#').map_or((after, ""), |(a, h)| (a, h));
    let (no_query, query) = no_hash.split_once('?').map_or((no_hash, ""), |(a, q)| (a, q));
    let (authority, path) = no_query.split_once('/').unwrap_or((no_query, ""));
    let host = authority.rsplit('@').next().unwrap_or(authority).to_lowercase();
    let segs: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let dash = segs.iter().position(|p| *p == "-").filter(|d| *d > 0);
    let cut = dash.unwrap_or(2);
    if segs.len() <= cut {
        return None;
    }
    let third = segs.get(2).copied().unwrap_or_default();
    let forgejo = super::text::detect_host_kind(&host) == "forgejo" || third == "pulls" || third == "src";
    let forge = if dash.is_some() {
        ForgeKind::GitLab
    } else if forgejo {
        ForgeKind::Forgejo
    } else {
        ForgeKind::GitHub
    };
    let at = ProjectRef { host, project: segs[..cut].iter().map(|p| decode(p)).collect::<Vec<_>>().join("/"), forge };
    let rest = segs[cut..].join("/");
    // An MR's own commit.
    let in_mr = match forge {
        ForgeKind::GitLab => {
            number_after(&rest, "-/merge_requests/").and_then(|_| query.split('&').find_map(|kv| kv.strip_prefix("commit_id=")).filter(|c| hex_sha(c)).map(str::to_lowercase))
        }
        ForgeKind::GitHub | ForgeKind::Forgejo => {
            let prefix = if forge == ForgeKind::GitHub { "pull/" } else { "pulls/" };
            rest.strip_prefix(prefix).and_then(|t| t.split_once("/commits/")).filter(|(n, _)| n.bytes().all(|b| b.is_ascii_digit())).and_then(|(_, c)| {
                let c = c.split('/').next().unwrap_or_default();
                hex_sha(c).then(|| c.to_lowercase())
            })
        }
    };
    let url = s.to_string();
    if let Some(sha) = in_mr {
        return Some(ForgeTarget::Commit { sha, url, at });
    }
    match page_of(forge, &rest)? {
        Page::Mr(number) => Some(ForgeTarget::Mr { number, url: Some(url), project: Some(at.project.clone()), at: Some(at), sigil: None }),
        Page::Commit(sha) => Some(ForgeTarget::Commit { sha, url, at }),
        Page::Blob(p) => Some(ForgeTarget::File { ref_path: p.split('/').map(decode).collect(), lines: anchor_lines(&format!("#{hash}")), url, at }),
    }
}

/// `!12`, `#12`, `group/project!12`.
pub fn classify_short(s: &str) -> Option<ForgeTarget> {
    let at = s.rfind(['!', '#'])?;
    let digits = &s[at + 1..];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let project = &s[..at];
    let word = |w: &str| !w.is_empty() && w.chars().all(|c| c.is_alphanumeric() || "_.-".contains(c));
    if !project.is_empty() && !(project.contains('/') && project.split('/').all(word)) {
        return None;
    }
    Some(ForgeTarget::Mr { number: digits.parse().ok()?, url: None, at: None, project: (!project.is_empty()).then(|| project.to_string()), sigil: s[at..].chars().next() })
}

/// The first split of `parts` whose tail is a tracked file: the ref and the path.
pub fn split_ref_path(parts: &[String], known: &std::collections::HashSet<&str>) -> Option<(String, String)> {
    (1..parts.len()).find_map(|i| {
        let path = parts[i..].join("/");
        known.contains(path.as_str()).then(|| (parts[..i].join("/"), path))
    })
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn without_scheme(u: &str) -> &str {
    u.split_once("://").map_or(u, |p| p.1).trim_end_matches('/')
}

/// The toast's way to the page; the core opens only `http`, `https` and `mailto`.
fn say(message: String, url: Option<String>, this: &WeakEntity<GitVolt>, cx: &mut App) {
    let actions = url.into_iter().map(|u| {
        let w = this.clone();
        crate::ui::toast::ToastAction {
            label: "Open".into(),
            run: std::rc::Rc::new(move |_: &mut Window, cx: &mut App| {
                let u = u.clone();
                w.update(cx, |v, _| v.forge_open_url(u)).ok();
            }),
        }
    });
    crate::ui::toast::show_with(message, crate::ui::toast::Options { actions: actions.collect(), ..Default::default() }, cx);
}

#[derive(PartialEq, Clone, Copy)]
enum PageKind {
    Mr,
    Commit,
    File,
}

impl GitVolt {
    /// `matchTab`: the index of the tab whose forge project this exactly is; failing that, by its remotes. A tab not shown yet has
    /// nothing loaded and isn't looked in. `near`: a tab whose remote was the project, for the message.
    fn tab_of(&self, at: &ProjectRef, page: PageKind) -> (Option<usize>, Option<usize>) {
        let wanted = format!("{}/{}", at.host, at.project);
        let project_of = |t: &crate::app::Tab| t.forge.project.as_ref().filter(|_| t.forge.kind.is_some()).map(|p| without_scheme(&p.web_url).to_string());
        let mut order: Vec<usize> = vec![self.active];
        order.extend((0..self.tabs.len()).filter(|i| *i != self.active));
        order.retain(|i| *i < self.tabs.len());
        if let Some(i) = order.iter().copied().find(|i| project_of(&self.tabs[*i]).is_some_and(|p| same(&p, &wanted))) {
            return (Some(i), None);
        }
        let has_remote = |t: &crate::app::Tab| t.info["remotes"].as_array().is_some_and(|l| l.iter().any(|r| same(r["host"].as_str().unwrap_or_default(), &at.host) && same(r["path"].as_str().unwrap_or_default(), &at.project)));
        let remote = order.iter().copied().find(|i| {
            let t = &self.tabs[*i];
            match page {
                PageKind::Commit => has_remote(t),
                PageKind::File => t.forge.kind.is_none() && has_remote(t),
                PageKind::Mr => false,
            }
        });
        (remote, order.iter().copied().find(|i| has_remote(&self.tabs[*i])))
    }

    fn not_open(&self, at: &ProjectRef, near: Option<usize>) -> String {
        match near.map(|i| &self.tabs[i]) {
            Some(t) if t.forge.project.is_some() => format!("{} isn't open here ({}'s project is {})", at.project, t.name, t.forge.project.as_ref().map_or("", |p| p.path.as_str())),
            Some(t) => format!("No {} account for {}", forge_name(at.forge), t.name),
            None => format!("{} isn't open here", at.project),
        }
    }

    pub fn go_to_forge(&mut self, target: ForgeTarget, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        match target {
            ForgeTarget::Mr { number, url, at: Some(at), .. } => {
                let r = mr_ref(at.forge, number);
                let (tab, near) = self.tab_of(&at, PageKind::Mr);
                if let Some(i) = tab {
                    let project = self.tabs[i].forge.project.as_ref().map_or(String::new(), |p| p.path.clone());
                    return self.go_to_mr(i, number, project, url, cx);
                }
                let message = match near.map(|i| &self.tabs[i]) {
                    Some(t) if t.forge.project.is_none() => format!("No {} account for {}: {r} opens in the browser", forge_name(at.forge), t.name),
                    Some(t) => format!("{r} belongs to {}, which isn't open here ({}'s project is {})", at.project, t.name, t.forge.project.as_ref().map_or("", |p| p.path.as_str())),
                    None => format!("{r} belongs to {}, which isn't open here", at.project),
                };
                say(message, url, &weak, cx);
            }
            ForgeTarget::Mr { number, project, sigil, .. } => self.go_to_short_mr(number, project, sigil, cx),
            ForgeTarget::Commit { sha, url, at } => {
                let (tab, near) = self.tab_of(&at, PageKind::Commit);
                let Some(i) = tab else { return say(self.not_open(&at, near), Some(url), &weak, cx) };
                self.set_active_tab(i, cx);
                self.go_to_commit_from_forge(sha, url, cx);
            }
            ForgeTarget::File { ref_path, lines, url, at } => {
                let (tab, near) = self.tab_of(&at, PageKind::File);
                let Some(i) = tab else { return say(self.not_open(&at, near), Some(url), &weak, cx) };
                self.go_to_forge_file(i, ref_path, lines, url, cx);
            }
        }
    }

    fn set_active_tab(&mut self, i: usize, cx: &mut Context<Self>) {
        if i != self.active {
            self.active = i;
            self.save_tabs(cx);
            cx.notify();
        }
    }

    /// `!12` / `#12`: the active repository's forge, if its kind writes them that way.
    fn go_to_short_mr(&mut self, number: u64, project: Option<String>, sigil: Option<char>, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get(self.active) else { return };
        let weak = cx.weak_entity();
        let Some((kind, p)) = t.forge.kind.zip(t.forge.project.clone()) else {
            return say(format!("{} has no GitLab, GitHub or Forgejo project for {}{number}", t.name, sigil.unwrap_or('#')), None, &weak, cx);
        };
        if (kind == ForgeKind::GitLab) != (sigil == Some('!')) {
            return;
        }
        let root = p.web_url[..p.web_url.len().saturating_sub(p.path.len())].trim_end_matches('/').to_string();
        let wanted = project.unwrap_or_else(|| p.path.clone());
        let forge = ProjectRemote { host: p.host.clone(), path: p.path.clone(), kind };
        let url = merge_request_url(&forge, Some(&wanted), number);
        if same(&wanted, &p.path) {
            return self.go_to_mr(self.active, number, p.path, Some(url), cx);
        }
        let at = ProjectRef { host: without_scheme(&root).split('/').next().unwrap_or_default().to_lowercase(), project: wanted.clone(), forge: kind };
        match self.tab_of(&at, PageKind::Mr).0 {
            Some(i) => {
                let project = self.tabs[i].forge.project.as_ref().map_or(String::new(), |p| p.path.clone());
                self.go_to_mr(i, number, project, Some(url), cx);
            }
            None => say(format!("{} belongs to {wanted}, which isn't open here", mr_ref(kind, number)), Some(url), &weak, cx),
        }
    }

    /// Opened only once the forge says it's an MR/PR (GitHub numbers issues alike): never the browser on its own.
    fn go_to_mr(&mut self, i: usize, number: u64, project: String, url: Option<String>, cx: &mut Context<Self>) {
        self.set_active_tab(i, cx);
        let Some(t) = self.tabs.get(i) else { return };
        let tab = t.id;
        let Some(kind) = t.forge.kind else { return };
        if t.forge.known_mr(number).is_some() || t.forge.details.contains_key(&number) {
            return self.forge_open_mr(tab, number, cx);
        }
        let r = mr_ref(kind, number);
        let (api, rt) = (self.api.clone(), self.rt.clone());
        let req = Request::ForgeMrDetail { repo: crate::app::core_repo(tab), number };
        cx.spawn(async move |this, cx| {
            let answer = rt.spawn(async move { api.dispatch(req).await }).await;
            this.update(cx, |v, cx| {
                let weak = cx.weak_entity();
                match answer {
                    Ok(Ok(_)) => v.forge_open_mr(tab, number, cx),
                    Ok(Err(e)) if e.kind == gitvolt_core::error::GbErrorKind::NotFound => say(format!("{r} isn't a {} in {project}", mr_long_noun(kind)), url, &weak, cx),
                    Ok(Err(e)) => say(format!("Couldn't read {r}: {}", e.message), url, &weak, cx),
                    Err(e) => say(format!("Couldn't read {r}: {e}"), url, &weak, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// A Markdown file's `!12`: its view when the forge says it's an MR/PR, else the page (`#n` may be an issue).
    pub fn forge_open_or_browse(&mut self, number: u64, url: String, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get(self.active) else { return };
        let tab = t.id;
        if t.forge.kind.is_none() {
            return self.forge_open_url(url);
        }
        if t.forge.known_mr(number).is_some() {
            return self.forge_open_mr(tab, number, cx);
        }
        let (api, rt) = (self.api.clone(), self.rt.clone());
        let req = Request::ForgeMrDetail { repo: crate::app::core_repo(tab), number };
        cx.spawn(async move |this, cx| {
            let answer = rt.spawn(async move { api.dispatch(req).await }).await;
            this.update(cx, |v, cx| match answer {
                Ok(Ok(_)) => v.forge_open_mr(tab, number, cx),
                _ => v.forge_open_url(url),
            })
            .ok();
        })
        .detach();
    }

    fn go_to_forge_file(&mut self, i: usize, parts: Vec<String>, lines: Option<(u32, u32)>, url: String, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get(i) else { return };
        let (tab, name, worktree, branch) = (t.id, t.name.clone(), t.worktree.clone(), t.graph.as_ref().map(|g| g.head_branch.to_string()).filter(|b| b != "detached"));
        let (api, rt) = (self.api.clone(), self.rt.clone());
        let req = Request::WorktreeFiles { repo: crate::app::core_repo(tab), worktree };
        cx.spawn(async move |this, cx| {
            let files = rt.spawn(async move { api.dispatch(req).await }).await;
            this.update(cx, |v, cx| {
                let weak = cx.weak_entity();
                let list: Vec<String> = match files {
                    Ok(Ok(l)) => serde_json::from_value(l).unwrap_or_default(),
                    _ => Vec::new(),
                };
                let known: std::collections::HashSet<&str> = list.iter().map(String::as_str).collect();
                let Some((at, path)) = split_ref_path(&parts, &known) else {
                    return say(format!("No file {} in {name}", parts[1.min(parts.len())..].join("/")), Some(url.clone()), &weak, cx);
                };
                let Some(i) = v.tabs.iter().position(|t| t.id == tab) else { return };
                v.set_active_tab(i, cx);
                v.forge_close_mr(tab, cx);
                v.open_worktree_file(&path, cx);
                if let Some((line, _)) = lines {
                    v.with_diff(cx, |d| d.pending_line = Some(line.saturating_sub(1)));
                }
                // The working copy shows, whatever ref the page was of.
                if branch.as_ref().is_some_and(|b| *b != at) {
                    crate::ui::toast::show(format!("Showing the working copy of {path}, not {at}'s"), cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn go_to_commit_from_forge(&mut self, sha: String, url: String, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let (repo, name, api, id) = (tab.id, tab.name.clone(), self.api.clone(), sha.clone());
        let located = self.rt.spawn(async move { api.dispatch(Request::LocateCommit { repo: crate::app::core_repo(repo), sha: id }).await });
        cx.spawn(async move |this, cx| {
            let v = match located.await {
                Ok(Ok(v)) => v,
                _ => serde_json::Value::Null,
            };
            this.update(cx, |this, cx| {
                let weak = cx.weak_entity();
                let shown = if sha.len() <= 12 { sha.clone() } else { sha[..7].to_string() };
                if v["found"].as_bool() != Some(true) {
                    return say(format!("No commit {shown} in {name}"), Some(url.clone()), &weak, cx);
                }
                let full = v["id"].as_str().unwrap_or(&sha).to_string();
                this.forge_close_mr(repo, cx);
                if this.in_graph(&full) {
                    return this.go_to_sha(&full, cx);
                }
                if let Some(limit) = v["limit"].as_u64() {
                    return this.load_deep(repo, limit as u32, Some(full), cx);
                }
                say(format!("{shown} isn't in {name}'s graph (deeper than 10,000 commits, or on no branch)"), Some(url.clone()), &weak, cx);
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn it_reads_a_page_of_each_forge() {
        let gl = classify_url("https://gitlab.example.com/group/sub/project/-/merge_requests/12/diffs").unwrap();
        assert!(matches!(&gl, ForgeTarget::Mr { number: 12, at: Some(a), .. } if a.project == "group/sub/project" && a.forge == ForgeKind::GitLab && a.host == "gitlab.example.com"));
        let gh = classify_url("https://github.com/octo-org/widget/pull/7").unwrap();
        assert!(matches!(&gh, ForgeTarget::Mr { number: 7, at: Some(a), .. } if a.project == "octo-org/widget" && a.forge == ForgeKind::GitHub));
        let fj = classify_url("https://forge.example.com:8443/team/gadget/pulls/3").unwrap();
        assert!(matches!(&fj, ForgeTarget::Mr { number: 3, at: Some(a), .. } if a.forge == ForgeKind::Forgejo && a.host == "forge.example.com:8443"));
        assert_eq!(classify_url("https://example.com/just/a/page"), None);
        assert_eq!(classify_url("https://example.com/a"), None);
        assert_eq!(classify_url("ftp://gitlab.example.com/a/-/merge_requests/1"), None);
    }

    #[test]
    fn an_mrs_own_commit_is_a_commit() {
        let sha = "abcdef1234567";
        assert!(matches!(classify_url(&format!("https://gitlab.example.com/g/p/-/merge_requests/2/diffs?commit_id={sha}")), Some(ForgeTarget::Commit { sha: s, .. }) if s == sha));
        assert!(matches!(classify_url(&format!("https://github.com/o/r/pull/2/commits/{sha}")), Some(ForgeTarget::Commit { sha: s, .. }) if s == sha));
        assert!(matches!(classify_url(&format!("https://forgejo.example.com/o/r/pulls/2/commits/{sha}")), Some(ForgeTarget::Commit { sha: s, .. }) if s == sha));
        assert!(matches!(classify_url(&format!("https://github.com/o/r/commit/{sha}")), Some(ForgeTarget::Commit { .. })));
    }

    #[test]
    fn file_pages_keep_their_lines_and_decode() {
        let Some(ForgeTarget::File { ref_path, lines, .. }) = classify_url("https://gitlab.example.com/g/p/-/blob/feat/x/src/a%20b.rs#L20-12") else { panic!() };
        assert_eq!(ref_path, ["feat", "x", "src", "a b.rs"]);
        assert_eq!(lines, Some((12, 20)));
        let Some(ForgeTarget::File { lines, .. }) = classify_url("https://github.com/o/r/blob/main/a.rs#L3-L5") else { panic!() };
        assert_eq!(lines, Some((3, 5)));
        let Some(ForgeTarget::File { ref_path, .. }) = classify_url("https://forgejo.example.com/o/r/src/branch/main/a.rs") else { panic!() };
        assert_eq!(ref_path, ["main", "a.rs"]);
    }

    #[test]
    fn short_references() {
        assert!(matches!(classify_short("!12"), Some(ForgeTarget::Mr { number: 12, sigil: Some('!'), project: None, .. })));
        assert!(matches!(classify_short("group/project#3"), Some(ForgeTarget::Mr { number: 3, sigil: Some('#'), project: Some(p), .. }) if p == "group/project"));
        assert_eq!(classify_short("word!12"), None);
        assert_eq!(classify_short("!x"), None);
    }

    #[test]
    fn the_first_split_naming_a_tracked_file_wins() {
        let parts: Vec<String> = ["feat", "x", "src", "a.rs"].iter().map(|s| s.to_string()).collect();
        let known = std::collections::HashSet::from(["src/a.rs"]);
        assert_eq!(split_ref_path(&parts, &known), Some(("feat/x".to_string(), "src/a.rs".to_string())));
        assert_eq!(split_ref_path(&parts, &std::collections::HashSet::new()), None);
    }
}
