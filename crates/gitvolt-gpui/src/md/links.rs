//! Where a link, a reference and an image go (`links.ts`, `images.ts`, `fileContext.ts`, `videos.ts`): pure, with what
//! it needs in a `LinkEnv`, so the forge views can give theirs.

use super::ast::{RefKind, RefNode};
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgeKind {
    Gitlab,
    Github,
    Forgejo,
}

/// `commit`: a commit id, or `worktree`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MdContext {
    /// An MR/PR view: its tab, by the core's repository id.
    Forge { repo: u32 },
    /// A file's rendered view.
    File { repo: u32, commit: String, path: String },
}

impl MdContext {
    pub fn repo(&self) -> u32 {
        match self {
            MdContext::Forge { repo } | MdContext::File { repo, .. } => *repo,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub path: String,
    /// Without a trailing `/`.
    pub web_url: String,
    pub host: String,
    pub default_branch: Option<String>,
    pub id: Option<u64>,
}

/// The full id of a loaded commit from its prefix.
pub type FullSha = Rc<dyn Fn(&str) -> Option<String>>;

/// What resolving needs (`LinkEnv`): the tab's forge, and the commits it has loaded.
#[derive(Clone, Default)]
pub struct LinkEnv {
    pub kind: Option<ForgeKind>,
    pub project: Option<Project>,
    pub full_sha: Option<FullSha>,
    /// The repository and remote `@user`'s card is looked up on.
    pub users: Option<(u32, String)>,
}

impl LinkEnv {
    fn sha(&self, prefix: &str) -> Option<String> {
        self.full_sha.as_ref().and_then(|f| f(prefix))
    }
}

/// What a link does on a plain click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    Mr { number: u64, web_url: String },
    Commit { sha: String, web_url: Option<String> },
    File { path: String, commit: String, anchor: Option<String> },
    Anchor { id: String },
    External { url: String },
    Inert,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSource {
    Forge { url: String },
    Remote { url: String, host: String },
    Repo { path: String, commit: String },
    Data { url: String },
    None,
}

/// `resolveRepoPath`: `href` relative to `from_file`, a normalized path from the repository's root, percent-decoded
/// here, once. `None`: it climbs above the root, names nothing, or a segment doesn't decode or decodes to a separator.
pub fn resolve_repo_path(from_file: &str, href: &str) -> Option<String> {
    let path = href.split('?').next().unwrap_or("");
    if path.is_empty() {
        return None;
    }
    let mut out: Vec<String> = if path.starts_with('/') {
        Vec::new()
    } else {
        let mut dirs: Vec<String> = from_file.split('/').filter(|s| !s.is_empty() && *s != ".").map(str::to_string).collect();
        if !from_file.is_empty() && !from_file.ends_with('/') {
            dirs.pop();
        }
        dirs
    };
    for raw in path.split('/') {
        let seg = percent_decode(raw)?;
        if seg.contains(['/', '\\', '\0']) {
            return None;
        }
        match seg.as_str() {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            _ => out.push(seg),
        }
    }
    (!out.is_empty()).then(|| out.join("/"))
}

/// `decodeURIComponent`: `None` for a malformed escape or what isn't UTF-8.
pub fn percent_decode(s: &str) -> Option<String> {
    if !s.contains('%') {
        return Some(s.to_string());
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            if !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn decode_or_same(s: &str) -> String {
    percent_decode(s).unwrap_or_else(|| s.to_string())
}

/// Browsers drop ASCII whitespace and controls inside a scheme: so does the check.
fn squashed(s: &str) -> String {
    s.chars().filter(|c| !matches!(*c as u32, 0..=0x20 | 0x7f)).collect()
}

fn scheme_of(s: &str) -> Option<String> {
    let colon = s.find(':')?;
    let name = &s[..colon];
    let mut chars = name.chars();
    chars.next().filter(char::is_ascii_alphabetic)?;
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')).then(|| name.to_ascii_lowercase())
}

fn network_path(s: &str) -> bool {
    let mut b = s.bytes();
    matches!((b.next(), b.next()), (Some(b'/' | b'\\'), Some(b'/' | b'\\')))
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn dash(kind: ForgeKind) -> &'static str {
    if kind == ForgeKind::Gitlab { "/-" } else { "" }
}

/// `https://github.com` for `https://github.com/o/r`; a GitLab instance may live under a path.
pub fn web_root(p: &Project) -> String {
    let tail = format!("/{}", p.path);
    if p.web_url.to_ascii_lowercase().ends_with(&tail.to_ascii_lowercase()) {
        p.web_url[..p.web_url.len() - tail.len()].to_string()
    } else {
        origin_of(&p.web_url).unwrap_or_else(|| p.web_url.clone())
    }
}

fn origin_of(url: &str) -> Option<String> {
    let at = url.find("://")?;
    let rest = &url[at + 3..];
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    Some(url[..at + 3 + end].to_string())
}

fn mr_url(kind: ForgeKind, web: &str, n: u64) -> String {
    let path = match kind {
        ForgeKind::Gitlab => "-/merge_requests",
        ForgeKind::Github => "pull",
        ForgeKind::Forgejo => "pulls",
    };
    format!("{web}/{path}/{n}")
}

fn issue_url(kind: ForgeKind, web: &str, n: u64) -> String {
    format!("{web}{}/issues/{n}", dash(kind))
}

fn commit_url(kind: ForgeKind, web: &str, sha: &str) -> String {
    format!("{web}{}/commit/{sha}", dash(kind))
}

fn blob_url(kind: ForgeKind, web: &str, branch: &str, path: &str) -> String {
    let base = if kind == ForgeKind::Forgejo { "/src/branch".to_string() } else { format!("{}/blob", dash(kind)) };
    let mut p = path;
    while let Some(r) = p.strip_prefix("./") {
        p = r;
    }
    format!("{web}{base}/{}/{}", encode_component(branch), p.trim_start_matches('/'))
}

/// `encodeURIComponent`.
pub fn encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn target_of_reference(env: &LinkEnv, node: &RefNode) -> LinkTarget {
    let (Some(kind), Some(p)) = (env.kind, env.project.as_ref()) else {
        if node.kind == RefKind::Commit
            && let Some(full) = node.sha.as_deref().and_then(|s| env.sha(s))
        {
            return LinkTarget::Commit { sha: full, web_url: None };
        }
        return LinkTarget::Inert;
    };
    match node.kind {
        RefKind::Mention => node.user.as_ref().map_or(LinkTarget::Inert, |u| LinkTarget::External { url: format!("{}/{u}", web_root(p)) }),
        RefKind::Commit => {
            let Some(sha) = &node.sha else { return LinkTarget::Inert };
            match env.sha(sha) {
                Some(full) => LinkTarget::Commit { sha: full, web_url: None },
                None => LinkTarget::Commit { sha: sha.clone(), web_url: Some(commit_url(kind, &p.web_url, sha)) },
            }
        }
        RefKind::Mr | RefKind::Issue => {
            let Some(n) = node.number else { return LinkTarget::Inert };
            let here = node.project.as_ref().is_none_or(|x| same(x, &p.path));
            let web = if here { p.web_url.clone() } else { format!("{}/{}", web_root(p), node.project.as_deref().unwrap_or_default()) };
            if node.kind == RefKind::Mr {
                return if here { LinkTarget::Mr { number: n, web_url: mr_url(kind, &web, n) } } else { LinkTarget::External { url: mr_url(kind, &web, n) } };
            }
            // `#n`: GitHub and Forgejo number PRs and issues together; GitLab's are issues.
            if kind != ForgeKind::Gitlab && here {
                return LinkTarget::Mr { number: n, web_url: issue_url(kind, &web, n) };
            }
            LinkTarget::External { url: issue_url(kind, &web, n) }
        }
    }
}

enum ForgePage {
    Mr(u64),
    Commit(String),
    Other,
}

fn hex_sha(s: &str) -> Option<String> {
    let s = s.split('/').next().unwrap_or("");
    ((7..=40).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_hexdigit())).then(|| s.to_ascii_lowercase())
}

fn number_then_end(s: &str) -> Option<u64> {
    let digits = s.split('/').next().unwrap_or("");
    (!digits.is_empty() && digits.bytes().all(|c| c.is_ascii_digit())).then(|| digits.parse().ok()).flatten()
}

fn forge_page(kind: ForgeKind, rest: &str) -> ForgePage {
    let (mr, commit) = match kind {
        ForgeKind::Gitlab => ("-/merge_requests/", "-/commit/"),
        ForgeKind::Github => ("pull/", "commit/"),
        ForgeKind::Forgejo => ("pulls/", "commit/"),
    };
    if let Some(n) = rest.strip_prefix(mr).and_then(number_then_end) {
        return ForgePage::Mr(n);
    }
    if let Some(sha) = rest.strip_prefix(commit).and_then(hex_sha) {
        return ForgePage::Commit(sha);
    }
    ForgePage::Other
}

/// This project's MR/PR or commit page, as an in-app target.
fn forge_target(env: &LinkEnv, url: &str) -> Option<LinkTarget> {
    let (kind, p) = (env.kind?, env.project.as_ref()?);
    let prefix = format!("{}/", p.web_url.to_ascii_lowercase());
    if !url.to_ascii_lowercase().starts_with(&prefix) {
        return None;
    }
    let rest = url[p.web_url.len() + 1..].split(['?', '#']).next().unwrap_or("");
    match forge_page(kind, rest) {
        ForgePage::Mr(n) => Some(LinkTarget::Mr { number: n, web_url: url.to_string() }),
        ForgePage::Commit(sha) => Some(match env.sha(&sha) {
            Some(full) => LinkTarget::Commit { sha: full, web_url: None },
            None => LinkTarget::Commit { sha, web_url: Some(url.to_string()) },
        }),
        ForgePage::Other => None,
    }
}

pub fn target_of_href(env: &LinkEnv, ctx: &MdContext, href: &str) -> LinkTarget {
    let raw = href.trim();
    if raw.is_empty() {
        return LinkTarget::Inert;
    }
    if let Some(id) = raw.strip_prefix('#') {
        let id = decode_or_same(id);
        return if id.is_empty() {
            LinkTarget::Inert
        } else {
            LinkTarget::Anchor { id: if id.starts_with("user-content-") { id } else { format!("user-content-{id}") } }
        };
    }
    let sq = squashed(raw);
    if network_path(&sq) {
        return LinkTarget::Inert;
    }
    match scheme_of(&sq).as_deref() {
        Some("mailto") => return LinkTarget::External { url: sq },
        Some("http" | "https") => return forge_target(env, &sq).unwrap_or(LinkTarget::External { url: sq }),
        Some(_) => return LinkTarget::Inert,
        None => {}
    }
    let (path, anchor) = match href.find('#') {
        Some(at) => (&href[..at], Some(&href[at + 1..])),
        None => (href, None),
    };
    let from = if let MdContext::File { path, .. } = ctx { path.as_str() } else { "" };
    let Some(path) = resolve_repo_path(from, path) else { return LinkTarget::Inert };
    let commit = if let MdContext::File { commit, .. } = ctx { commit.clone() } else { "worktree".to_string() };
    LinkTarget::File { path, commit, anchor: anchor.map(decode_or_same).filter(|a| !a.is_empty()) }
}

/// Ctrl+click's URL: the forge page when there is one, else the external URL.
pub fn browser_url_for(env: &LinkEnv, ctx: &MdContext, target: &LinkTarget) -> Option<String> {
    match target {
        LinkTarget::Mr { web_url, .. } => Some(web_url.clone()),
        LinkTarget::Commit { sha, web_url } => web_url.clone().or_else(|| Some(commit_url(env.kind?, &env.project.as_ref()?.web_url, sha))),
        LinkTarget::External { url } => Some(url.clone()),
        LinkTarget::File { path, anchor, .. } => {
            if !matches!(ctx, MdContext::Forge { .. }) {
                return None;
            }
            let (kind, p) = (env.kind?, env.project.as_ref()?);
            let url = blob_url(kind, &p.web_url, p.default_branch.as_deref().unwrap_or("HEAD"), path);
            Some(anchor.as_ref().map_or(url.clone(), |a| format!("{url}#{a}")))
        }
        LinkTarget::Anchor { .. } | LinkTarget::Inert => None,
    }
}

/// The hover tooltip: the full target first.
pub fn link_tooltip(env: &LinkEnv, ctx: &MdContext, target: &LinkTarget) -> Option<String> {
    match target {
        LinkTarget::Mr { number, .. } => Some(format!("Open {} in GitVolt", mr_ref(env.kind.unwrap_or(ForgeKind::Gitlab), *number))),
        LinkTarget::Commit { sha, web_url } => Some(web_url.clone().unwrap_or_else(|| format!("Select {} in the graph", &sha[..sha.len().min(7)]))),
        LinkTarget::File { path, .. } => {
            if matches!(ctx, MdContext::File { .. }) { Some(path.clone()) } else { browser_url_for(env, ctx, target) }
        }
        LinkTarget::Anchor { id } => Some(format!("Go to #{}", id.strip_prefix("user-content-").unwrap_or(id))),
        LinkTarget::External { url } => Some(url.clone()),
        LinkTarget::Inert => None,
    }
}

fn mr_ref(kind: ForgeKind, n: u64) -> String {
    if kind == ForgeKind::Gitlab { format!("!{n}") } else { format!("#{n}") }
}

const GITHUB_IMAGE_HOSTS: [&str; 4] = ["user-images.githubusercontent.com", "private-user-images.githubusercontent.com", "raw.githubusercontent.com", "avatars.githubusercontent.com"];

pub fn data_image(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    ["png", "gif", "jpeg", "webp"].iter().any(|k| {
        let head = format!("data:image/{k};base64,");
        lower.starts_with(&head) && s.len() > head.len() && s[head.len()..].bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'=') || c.is_ascii_whitespace())
    })
}

fn host_of(url: &str) -> Option<String> {
    let rest = &url[url.find("://")? + 3..];
    let auth = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    (!auth.contains('@') && !auth.is_empty()).then(|| auth.to_ascii_lowercase())
}

/// `forgeImageHost`: never on Forgejo, whose images load on a click like any host's.
fn forge_image_host(kind: Option<ForgeKind>, p: &Project, host: &str) -> bool {
    if kind == Some(ForgeKind::Forgejo) {
        return false;
    }
    if host == p.host.to_ascii_lowercase() || host_of(&p.web_url).as_deref() == Some(host) {
        return true;
    }
    kind == Some(ForgeKind::Github) && GITHUB_IMAGE_HOSTS.contains(&host)
}

/// `resolveImage`.
pub fn resolve_image(env: &LinkEnv, ctx: &MdContext, src: &str) -> ImageSource {
    let s = src.trim();
    if s.is_empty() {
        return ImageSource::None;
    }
    let sq = squashed(s);
    if network_path(&sq) {
        return ImageSource::None;
    }
    if sq.to_ascii_lowercase().starts_with("data:") {
        return if data_image(s) { ImageSource::Data { url: s.to_string() } } else { ImageSource::None };
    }
    let project = env.project.as_ref();
    if let Some(scheme) = scheme_of(&sq) {
        let Some(host) = host_of(&sq) else { return ImageSource::None };
        let web_origin = project.and_then(|p| origin_of(&p.web_url));
        let https = scheme == "https";
        if !https && !(scheme == "http" && web_origin.as_deref() == origin_of(&sq).as_deref() && web_origin.is_some()) {
            return ImageSource::None;
        }
        if project.is_some_and(|p| forge_image_host(env.kind, p, &host)) {
            return ImageSource::Forge { url: sq };
        }
        return if https { ImageSource::Remote { url: sq, host } } else { ImageSource::None };
    }
    if let MdContext::File { path, commit, .. } = ctx {
        let at = src.find('#').map_or(src, |k| &src[..k]);
        return match resolve_repo_path(path, at) {
            Some(path) => ImageSource::Repo { path, commit: commit.clone() },
            None => ImageSource::None,
        };
    }
    if let (Some(p), Some(ForgeKind::Gitlab)) = (project, env.kind)
        && let Some(id) = p.id
        && s.trim_start_matches('/').starts_with("uploads/")
    {
        return ImageSource::Forge { url: format!("{}/-/project/{id}/{}", web_root(p), s.trim_start_matches('/')) };
    }
    if let (Some(p), Some(ForgeKind::Forgejo)) = (project, env.kind)
        && let Some(uuid) = s.strip_prefix("/attachments/")
        && !uuid.is_empty()
        && uuid.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
        && p.web_url.starts_with("https://")
        && let (Some(origin), Some(host)) = (origin_of(&p.web_url), host_of(&p.web_url))
    {
        return ImageSource::Remote { url: format!("{origin}{s}"), host };
    }
    ImageSource::None
}

/// `isVideoSrc`: the extensions GitLab renders an image link to as a video.
pub fn is_video_src(src: &str) -> bool {
    let path = src.trim().split(['?', '#']).next().unwrap_or("");
    let lower = path.to_ascii_lowercase();
    [".mp4", ".m4v", ".mov", ".webm", ".ogv"].iter().any(|e| lower.ends_with(e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn file() -> MdContext {
        MdContext::File { repo: 1, commit: "abc".into(), path: "docs/guide/a.md".into() }
    }

    fn forge() -> LinkEnv {
        LinkEnv { users: None, kind: Some(ForgeKind::Github), project: Some(Project { path: "o/r".into(), web_url: "https://github.example/o/r".into(), host: "github.example".into(), default_branch: Some("main".into()), id: None }), full_sha: None }
    }

    #[test]
    fn a_relative_path_resolves_as_the_web_ones() {
        assert_eq!(resolve_repo_path("docs/a.md", "b.md").as_deref(), Some("docs/b.md"));
        assert_eq!(resolve_repo_path("docs/a.md", "../b.md").as_deref(), Some("b.md"));
        assert_eq!(resolve_repo_path("docs/a.md", "/b.md").as_deref(), Some("b.md"));
        assert_eq!(resolve_repo_path("docs/a.md", "./x/./y%20z.md?raw=1").as_deref(), Some("docs/x/y z.md"));
        assert_eq!(resolve_repo_path("a.md", "../b.md"), None, "above the root");
        assert_eq!(resolve_repo_path("a.md", "x%2Fy.md"), None, "a decoded separator");
        assert_eq!(resolve_repo_path("a.md", "%zz"), None, "doesn't decode");
    }

    #[test]
    fn links_go_where_the_table_says() {
        let env = LinkEnv::default();
        let ctx = file();
        assert_eq!(target_of_href(&env, &ctx, "#Top"), LinkTarget::Anchor { id: "user-content-Top".into() });
        assert_eq!(target_of_href(&env, &ctx, "https://a.example/x"), LinkTarget::External { url: "https://a.example/x".into() });
        assert_eq!(target_of_href(&env, &ctx, "mailto:a@b.example"), LinkTarget::External { url: "mailto:a@b.example".into() });
        assert_eq!(target_of_href(&env, &ctx, "javascript:alert(1)"), LinkTarget::Inert);
        assert_eq!(target_of_href(&env, &ctx, "java\tscript:alert(1)"), LinkTarget::Inert, "whitespace in a scheme");
        assert_eq!(target_of_href(&env, &ctx, "//evil.example/x"), LinkTarget::Inert);
        assert_eq!(target_of_href(&env, &ctx, "\\\\evil.example"), LinkTarget::Inert);
        assert_eq!(target_of_href(&env, &ctx, "../b.md#part"), LinkTarget::File { path: "docs/b.md".into(), commit: "abc".into(), anchor: Some("part".into()) });
        assert_eq!(target_of_href(&env, &ctx, ""), LinkTarget::Inert);
    }

    #[test]
    fn a_forge_page_opens_in_the_app() {
        let env = forge();
        let ctx = MdContext::Forge { repo: 1 };
        assert_eq!(target_of_href(&env, &ctx, "https://github.example/o/r/pull/5"), LinkTarget::Mr { number: 5, web_url: "https://github.example/o/r/pull/5".into() });
        assert_eq!(target_of_href(&env, &ctx, "https://github.example/o/r/commit/abcdef1"), LinkTarget::Commit { sha: "abcdef1".into(), web_url: Some("https://github.example/o/r/commit/abcdef1".into()) });
        assert_eq!(link_tooltip(&env, &ctx, &LinkTarget::Mr { number: 5, web_url: String::new() }).as_deref(), Some("Open #5 in GitVolt"));
    }

    #[test]
    fn references_need_a_forge_but_a_loaded_commit_does_not() {
        let r = RefNode { kind: RefKind::Issue, project: None, number: Some(4), sha: None, user: None, value: "#4".into() };
        assert_eq!(target_of_reference(&LinkEnv::default(), &r), LinkTarget::Inert);
        assert_eq!(target_of_reference(&forge(), &r), LinkTarget::Mr { number: 4, web_url: "https://github.example/o/r/issues/4".into() });
        let c = RefNode { kind: RefKind::Commit, project: None, number: None, sha: Some("abc1234".into()), user: None, value: "abc1234".into() };
        let env = LinkEnv { full_sha: Some(Rc::new(|p| Some(format!("{p}000")))), ..Default::default() };
        assert_eq!(target_of_reference(&env, &c), LinkTarget::Commit { sha: "abc1234000".into(), web_url: None });
    }

    #[test]
    fn images_are_guarded_as_the_web_ones() {
        let (env, ctx) = (LinkEnv::default(), file());
        assert_eq!(resolve_image(&env, &ctx, "../i.png"), ImageSource::Repo { path: "docs/i.png".into(), commit: "abc".into() });
        assert_eq!(resolve_image(&env, &ctx, "https://x.example/i.png"), ImageSource::Remote { url: "https://x.example/i.png".into(), host: "x.example".into() });
        assert_eq!(resolve_image(&env, &ctx, "http://x.example/i.png"), ImageSource::None, "not over http");
        assert_eq!(resolve_image(&env, &ctx, "https://u:p@x.example/i.png"), ImageSource::None, "no credentials");
        assert_eq!(resolve_image(&env, &ctx, "//x.example/i.png"), ImageSource::None);
        assert_eq!(resolve_image(&env, &ctx, "data:image/svg+xml;base64,AAAA"), ImageSource::None);
        assert!(matches!(resolve_image(&env, &ctx, "data:image/png;base64,AAAA"), ImageSource::Data { .. }));
        assert_eq!(resolve_image(&env, &ctx, "../../../../i.png"), ImageSource::None, "above the root");
        let gh = forge();
        assert_eq!(resolve_image(&gh, &MdContext::Forge { repo: 1 }, "https://raw.githubusercontent.com/o/r/a.png"), ImageSource::Forge { url: "https://raw.githubusercontent.com/o/r/a.png".into() });
        assert_eq!(resolve_image(&gh, &MdContext::Forge { repo: 1 }, "a.png"), ImageSource::None);
    }

    #[test]
    fn a_video_is_told_by_its_extension() {
        assert!(is_video_src("/uploads/a/clip.WEBM?x=1"));
        assert!(!is_video_src("clip.webm.png"));
    }
}
