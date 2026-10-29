//! Forgejo's API v1 (Gitea's, which Forgejo keeps): identity, repositories, settings, forks,
//! avatars and pull requests. Its JSON is close to GitHub's REST, so this shares `github.rs`'s
//! pure helpers where the shapes match, with Forgejo's own words where they don't: review states
//! (`APPROVED`, `REQUEST_CHANGES`, `COMMENT`, `REQUEST_REVIEW`), merge styles (`Do`), a commit's
//! statuses as its checks, a draft as a `WIP:` title. No GraphQL, no rate limit by default, and
//! no avatar lookup by email. A thread is resolved through `pulls/comments/{id}/resolve` (an API
//! newer Forgejo has, forgejo/forgejo#14873; an older one answers 404). Line comments from the diff
//! are kept here as drafts and go in with their review, as one review POST with its comments (a
//! range is its first line and `extra_lines_count` lines after it, on one side): its API starts a
//! pending review only by posting a review,
//! which leaves an entry of its own in the PR's timeline. A pending review made on the web is read
//! and sent along.

use crate::avatar_cache::{image_at, payload_of, DiskAvatarCache, Lookup};
use crate::endpoints::HostEndpoints;
use crate::github::GitHubProvider;
use crate::http::{encode_component, ClientConfig, HttpClient, HttpResponse, Method};
use crate::known_names::{KnownNames, NameMatch};
use crate::time::unix_now;
use gitvolt_core::avatar::AvatarPayload;
use gitvolt_core::error::{GbError, GbErrorKind};
use gitvolt_core::forge::*;
use gitvolt_core::redact::Secret;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const FORGEJO_HEADERS: [(&str, &str); 1] = [("Accept", "application/json")];
/// Forgejo's default `MAX_RESPONSE_ITEMS`: a larger `limit` is cut to it.
pub const PER_PAGE: u32 = 50;
pub const OPEN_PAGES: usize = 5;
pub const FORK_PAGES: usize = 3;
pub const COMMENT_PAGES: usize = 5;
pub const LABEL_PAGES: usize = 3;
pub const TIMELINE_PAGES: usize = 3;
/// The open PRs whose statuses one list read asks for (one request each).
pub const PIPELINE_LOOKUPS: usize = 20;
/// Comments whose reactions one discussions read asks for (the comments carry no summary).
pub const REACTION_LOOKUPS: usize = 10;
pub const PEOPLE_SHOWN: usize = 20;
/// How long a failed ask of who the user is stands before it's asked again (`me_soft`).
pub const ME_RETRY: std::time::Duration = std::time::Duration::from_secs(600);
/// What Forgejo's default `WORK_IN_PROGRESS_PREFIXES` mark a draft with, case aside.
pub const WIP_PREFIXES: [&str; 2] = ["WIP:", "[WIP]"];

/// Where a review thread's comments sit, so a reply lands in it (Forgejo groups a pull
/// request's code comments by file and line; its API has no reply-to).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadAt {
    pub path: String,
    /// The commented line (a range's first) on the new side, else 0.
    pub new_line: u64,
    pub old_line: u64,
    /// A range's lines after the first (`extra_lines_count`; 0: one line).
    pub extra: u64,
    pub commit: Option<String>,
}

/// A line comment kept until its review is sent: this session's, by PR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDraft {
    pub id: u64,
    pub anchor: ReviewAnchor,
    pub body: String,
    /// The head it was written against.
    pub head: String,
}

pub struct ForgejoProvider {
    host: String,
    api_base: String,
    web: String,
    http: HttpClient,
    cache: Option<Arc<DiskAvatarCache>>,
    /// Lowercase email → avatar URL, from API data (a user's public email).
    learned: Mutex<HashMap<String, String>>,
    me: Mutex<Option<ForgeUser>>,
    me_failed: Mutex<Option<std::time::Instant>>,
    checks: crate::pipelines::PipelineCache,
    names: KnownNames,
    /// (repo, PR) → a review thread's first comment id → where it is (`reply`).
    threads: Mutex<HashMap<(String, u64), HashMap<u64, ThreadAt>>>,
    /// (repo, PR) → its review drafts not sent yet (`LocalDraft`).
    drafts: Mutex<HashMap<(String, u64), Vec<LocalDraft>>>,
    next_draft: std::sync::atomic::AtomicU64,
}

fn unreadable(host: &str, what: &str) -> GbError {
    GbError::other(format!("{host} sent a {what} GitVolt couldn't read"))
}

/// Forgejo's JSON → the normalized types (pure).
pub mod json {
    use super::{ThreadAt, WIP_PREFIXES};
    use crate::github::json::text;
    use crate::time::parse_rfc3339;
    use gitvolt_core::error::{GbError, GbErrorKind};
    use gitvolt_core::forge::*;
    use serde_json::Value;
    use std::collections::HashMap;

    pub fn text_of(v: &Value) -> Option<String> {
        text(v)
    }

    pub fn user(v: &Value) -> Option<ForgeUser> {
        let login = text(&v["login"]).or_else(|| text(&v["username"]))?;
        // A deleted user is Forgejo's "Ghost", id -1.
        let id = v["id"].as_u64().or_else(|| v["id"].as_i64().map(|_| 0))?;
        let email = text(&v["email"]);
        Some(ForgeUser { id, name: text(&v["full_name"]).unwrap_or_else(|| login.clone()), avatar_url: text(&v["avatar_url"]), web_url: v["html_url"].as_str().unwrap_or_default().to_string(), emails: email.iter().cloned().collect(), email, username: login })
    }

    pub fn users(v: &Value) -> Vec<ForgeUser> {
        v.as_array().into_iter().flatten().filter_map(user).collect()
    }

    pub fn project(host: &str, v: &Value) -> Option<ForgeProject> {
        Some(ForgeProject {
            kind: ForgeKind::Forgejo,
            id: v["id"].as_u64()?,
            host: host.to_string(),
            path: v["full_name"].as_str()?.to_string(),
            name: v["name"].as_str().unwrap_or_default().to_string(),
            owner: v["owner"]["login"].as_str().unwrap_or_default().to_string(),
            web_url: v["html_url"].as_str().unwrap_or_default().to_string(),
            // An empty repository names a default branch it doesn't have yet.
            default_branch: text(&v["default_branch"]).filter(|_| v["empty"].as_bool() != Some(true)),
            clone_https: v["clone_url"].as_str().unwrap_or_default().to_string(),
            clone_ssh: v["ssh_url"].as_str().unwrap_or_default().to_string(),
            fork_of: text(&v["parent"]["full_name"]).filter(|_| v["fork"].as_bool() != Some(false)),
            updated_at: v["updated_at"].as_str().and_then(parse_rfc3339),
            archived: v["archived"].as_bool().unwrap_or(false),
            owner_avatar_url: text(&v["owner"]["avatar_url"]),
        })
    }

    /// Forgejo's merge style (`Do`, `default_merge_style`) for a method.
    pub fn style_word(m: MergeMethod) -> &'static str {
        match m {
            MergeMethod::Merge => "merge",
            MergeMethod::Rebase => "rebase",
            // Rebase, then a merge commit: the semi-linear history GitLab's method names.
            MergeMethod::SemiLinear => "rebase-merge",
            MergeMethod::Squash => "squash",
            MergeMethod::FastForward => "fast-forward-only",
        }
    }

    /// The allowed merge styles, the repository's default first; a field the answer lacks takes
    /// Forgejo's default (all but fast-forward-only allowed).
    pub fn settings(v: &Value) -> ForgeProjectSettings {
        let allow = |k: &str, default: bool| v[k].as_bool().unwrap_or(default);
        let mut merge_methods: Vec<MergeMethod> = [
            ("allow_merge_commits", true, MergeMethod::Merge),
            ("allow_rebase", true, MergeMethod::Rebase),
            ("allow_rebase_explicit", true, MergeMethod::SemiLinear),
            ("allow_squash_merge", true, MergeMethod::Squash),
            ("allow_fast_forward_only_merge", false, MergeMethod::FastForward),
        ]
        .into_iter()
        .filter(|(k, d, _)| allow(k, *d))
        .map(|(_, _, m)| m)
        .collect();
        if let Some(i) = v["default_merge_style"].as_str().and_then(|d| merge_methods.iter().position(|m| style_word(*m) == d)) {
            let m = merge_methods.remove(i);
            merge_methods.insert(0, m);
        }
        let squash = if !merge_methods.contains(&MergeMethod::Squash) {
            SquashOption::Never
        } else if merge_methods == [MergeMethod::Squash] {
            SquashOption::Always
        } else {
            SquashOption::DefaultOff
        };
        ForgeProjectSettings { merge_methods, squash, delete_source_branch: v["default_delete_branch_after_merge"].as_bool().unwrap_or(false) }
    }

    /// A title Forgejo takes as a draft's (its default work-in-progress prefixes).
    pub fn wip_title(title: &str) -> bool {
        let t = title.trim_start();
        WIP_PREFIXES.iter().any(|p| t.get(..p.len()).is_some_and(|s| s.eq_ignore_ascii_case(p)))
    }

    /// `title` as a draft's (`WIP: …`) or a ready one's (every WIP prefix taken off).
    pub fn drafted(title: &str, draft: bool) -> String {
        let mut t = title.trim();
        while let Some(p) = WIP_PREFIXES.iter().find(|p| t.get(..p.len()).is_some_and(|s| s.eq_ignore_ascii_case(p))) {
            t = t[p.len()..].trim_start();
        }
        if draft { format!("WIP: {t}") } else { t.to_string() }
    }

    pub fn pr(v: &Value) -> Option<ForgeMr> {
        let title = v["title"].as_str()?.to_string();
        let merged = v["merged"].as_bool().unwrap_or(false) || v["merged_at"].is_string();
        let draft = v["draft"].as_bool() == Some(true) || wip_title(&title);
        let state = match v["state"].as_str()? {
            "open" if draft => MrState::Draft,
            "open" => MrState::Open,
            _ if merged => MrState::Merged,
            _ => MrState::Closed,
        };
        let requested = users(&v["requested_reviewers"]);
        let (labels, label_colors) = labels_of(&v["labels"]);
        Some(ForgeMr {
            number: v["number"].as_u64()?,
            title,
            state,
            author: user(&v["user"])?,
            // A deleted fork's head repository is null.
            source_project: v["head"]["repo"]["full_name"].as_str().unwrap_or_default().to_string(),
            source_branch: v["head"]["ref"].as_str()?.to_string(),
            target_project: v["base"]["repo"]["full_name"].as_str().unwrap_or_default().to_string(),
            target_branch: v["base"]["ref"].as_str()?.to_string(),
            head_sha: text(&v["head"]["sha"]),
            web_url: v["html_url"].as_str().unwrap_or_default().to_string(),
            pipeline: None,
            review: ReviewSummary {
                decision: if requested.is_empty() { ReviewDecision::None } else { ReviewDecision::ReviewRequired },
                approvals: 0,
                approvals_required: None,
                reviews: requested.into_iter().map(|u| ForgeReview { user: u, state: ReviewState::Pending, submitted_at: None, requested: false }).collect(),
            },
            // `mergeable` is false for a draft too, and while Forgejo checks: only an open,
            // ready one's says something about conflicts.
            conflicts: if state == MrState::Open { v["mergeable"].as_bool().map(|m| !m) } else { None },
            labels,
            label_colors,
            updated_at: v["updated_at"].as_str().and_then(parse_rfc3339).unwrap_or(0),
            stacked: v["body"].as_str().is_some_and(gitvolt_core::forge::stack::carries_stack_table),
            auto_merge: None,
        })
    }

    /// A commit's combined status (`/commits/{sha}/status`) as its pipeline: Forgejo's state is
    /// already the worst of its statuses. `None`: it has none.
    pub fn status(v: &Value, web_url: &str) -> Option<ForgePipeline> {
        if v["statuses"].as_array().is_none_or(|a| a.is_empty()) {
            return None;
        }
        let status = match v["state"].as_str() {
            Some("success") => PipelineStatus::Success,
            // Finished, with warnings: not a failure.
            Some("warning") => PipelineStatus::Success,
            Some("pending") => PipelineStatus::Pending,
            Some("skipped") => PipelineStatus::Skipped,
            _ => PipelineStatus::Failed,
        };
        Some(ForgePipeline { status, web_url: Some(web_url.to_string()) })
    }

    /// Each user's latest decisive review (a comment doesn't undo an approval; a dismissal does,
    /// back to their comment if they left one); a review request is pending until a review after
    /// it, a comment included. A re-request is a `REQUEST_REVIEW` entry in the reviews, so
    /// `requested_reviewers` adds only the users with no standing review. Forgejo keeps a reviewer
    /// there after a comment (only an approval or a change request answers the request), so that
    /// request can still be withdrawn; it has kept one after an approval too.
    pub fn review(reviews: &[Value], requested: &Value) -> ReviewSummary {
        let mut latest: Vec<ForgeReview> = Vec::new();
        // Each user's latest comment, for a dismissal to fall back to.
        let mut comments: Vec<(u64, Option<i64>)> = Vec::new();
        for r in reviews {
            let Some(u) = user(&r["user"]) else { continue };
            if r["dismissed"].as_bool() == Some(true) {
                latest.retain(|x| x.user.id != u.id);
                if let Some(&(_, at)) = comments.iter().find(|c| c.0 == u.id) {
                    latest.push(ForgeReview { user: u, state: ReviewState::Commented, submitted_at: at, requested: false });
                }
                continue;
            }
            let at = r["submitted_at"].as_str().and_then(parse_rfc3339);
            let state = match r["state"].as_str() {
                Some("APPROVED") => Some(ReviewState::Approved),
                Some("REQUEST_CHANGES") => Some(ReviewState::ChangesRequested),
                Some("REQUEST_REVIEW") => Some(ReviewState::Pending),
                Some("COMMENT") => {
                    comments.retain(|c| c.0 != u.id);
                    comments.push((u.id, at));
                    None
                }
                _ => continue,
            };
            match (latest.iter_mut().find(|x| x.user.id == u.id), state) {
                (Some(x), Some(s)) => {
                    x.state = s;
                    x.submitted_at = at;
                }
                (None, Some(s)) => latest.push(ForgeReview { user: u, state: s, submitted_at: at, requested: false }),
                (None, None) => latest.push(ForgeReview { user: u, state: ReviewState::Commented, submitted_at: at, requested: false }),
                (Some(x), None) if x.state == ReviewState::Pending => {
                    x.state = ReviewState::Commented;
                    x.submitted_at = at;
                }
                (Some(_), None) => {}
            }
        }
        for u in users(requested) {
            match latest.iter_mut().find(|x| x.user.id == u.id) {
                Some(x) => x.requested = x.state == ReviewState::Commented,
                None => latest.push(ForgeReview { user: u, state: ReviewState::Pending, submitted_at: None, requested: false }),
            }
        }
        let approvals = latest.iter().filter(|x| x.state == ReviewState::Approved).count() as u32;
        let decision = if latest.iter().any(|x| x.state == ReviewState::ChangesRequested) {
            ReviewDecision::ChangesRequested
        } else if approvals > 0 {
            ReviewDecision::Approved
        } else if latest.iter().any(|x| x.state == ReviewState::Pending) {
            ReviewDecision::ReviewRequired
        } else {
            ReviewDecision::None
        };
        ReviewSummary { decision, approvals, approvals_required: None, reviews: latest }
    }

    /// Why Forgejo won't merge it. Its `mergeable` is false with conflicts and while it checks
    /// them, which its API doesn't tell apart.
    pub fn merge_status(v: &Value) -> MergeStatus {
        let blocked = |r: &str| MergeStatus::Blocked { reason: r.to_string() };
        if v["merged"].as_bool() == Some(true) {
            return blocked("It's merged already");
        }
        if v["state"].as_str() == Some("closed") {
            return blocked("It's closed");
        }
        if v["draft"].as_bool() == Some(true) || v["title"].as_str().is_some_and(wip_title) {
            return blocked("Mark it ready first: it's a draft");
        }
        match v["mergeable"].as_bool() {
            Some(false) => blocked("It has conflicts, or Forgejo is still checking: rebase or merge the base branch, or refresh"),
            _ => MergeStatus::Mergeable,
        }
    }

    /// Update branch (a merge of the base, or a rebase), offered while it's open. A fork's head
    /// only when the token's user (`me`) may push to it: its author, or anyone when it allows
    /// maintainers' edits (`allow_maintainer_edit`); `me` unknown: offered, and Forgejo says why not.
    pub fn update_offer(v: &Value, me: Option<u64>) -> Option<BranchUpdateOffer> {
        if v["state"].as_str() != Some("open") || v["merged"].as_bool() == Some(true) {
            return None;
        }
        let fork = v["head"]["repo"]["full_name"] != v["base"]["repo"]["full_name"];
        let may_push = !fork || v["allow_maintainer_edit"].as_bool() == Some(true) || me.is_none_or(|me| v["user"]["id"].as_u64() == Some(me));
        may_push.then(|| BranchUpdateOffer { behind: None, forge_behind: false, kinds: vec![BranchUpdate::Merge, BranchUpdate::Rebase], in_progress: false })
    }

    /// A conversation or code comment.
    pub fn comment_note(v: &Value) -> Option<ForgeNote> {
        Some(ForgeNote {
            id: v["id"].as_u64()?.to_string(),
            author: user(&v["user"])?,
            body: v["body"].as_str().unwrap_or_default().to_string(),
            created_at: v["created_at"].as_str().and_then(parse_rfc3339).unwrap_or(0),
            system: false,
            position: None,
            body_html: None,
            reactions: Vec::new(),
            web_url: text(&v["html_url"]),
        })
    }

    /// A code comment's line: `position` on the new side, or `original_position` on the old one
    /// (Forgejo sets one of them).
    fn thread_at(c: &Value) -> Option<ThreadAt> {
        Some(ThreadAt { path: text(&c["path"])?, new_line: c["position"].as_u64().unwrap_or(0), old_line: c["original_position"].as_u64().unwrap_or(0), extra: c["extra_lines_count"].as_u64().unwrap_or(0), commit: text(&c["commit_id"]) })
    }

    /// Where a code comment is: a range ends `extra` lines after its first, which is its start.
    fn position(at: &ThreadAt, hunk: Option<&str>) -> DiffPosition {
        let line = |n: u64| (n > 0).then_some(n as u32);
        let last = |n: u64| line(n).map(|n| n + at.extra as u32);
        let start = |n: u64| if at.extra > 0 { line(n) } else { None };
        DiffPosition {
            path: at.path.clone(),
            old_path: None,
            line: last(at.new_line),
            old_line: last(at.old_line),
            snippet: hunk.and_then(crate::github::json::hunk_tail),
            start_line: start(at.new_line),
            start_old_line: start(at.old_line),
            head_sha: None,
            outdated: false,
        }
    }

    /// The conversation (`issue-<id>`), review summaries with text (`review-<id>`), and code
    /// comments as threads by file and line (`thread-<first comment id>`), oldest first; and where
    /// each thread is. A thread's resolution is its first comment's `resolver` (`resolve` changes
    /// it).
    pub fn discussions(comments: &[Value], reviews: &[Value], code_comments: &[Value]) -> (Vec<ForgeDiscussion>, HashMap<u64, ThreadAt>) {
        let one = |id: String, n: ForgeNote| ForgeDiscussion { id, notes: vec![n], resolvable: false, resolved: false, resolved_by: None, resolved_at: None };
        let mut out: Vec<ForgeDiscussion> = comments.iter().filter_map(|c| comment_note(c).map(|n| one(format!("issue-{}", n.id), n))).collect();
        for r in reviews {
            let body = r["body"].as_str().unwrap_or_default();
            if body.trim().is_empty() || matches!(r["state"].as_str(), Some("PENDING" | "REQUEST_REVIEW")) {
                continue;
            }
            let (Some(id), Some(author)) = (r["id"].as_u64(), user(&r["user"])) else { continue };
            let created_at = r["submitted_at"].as_str().and_then(parse_rfc3339).unwrap_or(0);
            out.push(one(format!("review-{id}"), ForgeNote { id: format!("review-{id}"), author, body: body.to_string(), created_at, system: false, position: None, body_html: None, reactions: Vec::new(), web_url: text(&r["html_url"]) }));
        }
        let mut sorted: Vec<&Value> = code_comments.iter().collect();
        sorted.sort_by_key(|c| (c["created_at"].as_str().and_then(parse_rfc3339).unwrap_or(0), c["id"].as_u64().unwrap_or(0)));
        let mut threads: Vec<(u64, ThreadAt, ForgeDiscussion)> = Vec::new();
        for c in sorted {
            let (Some(id), Some(mut note), Some(at)) = (c["id"].as_u64(), comment_note(c), thread_at(c)) else { continue };
            let same = |t: &ThreadAt| t.path == at.path && t.new_line == at.new_line && t.old_line == at.old_line;
            match threads.iter_mut().find(|(_, t, _)| same(t)) {
                Some((_, _, d)) => d.notes.push(note),
                None => {
                    note.position = Some(position(&at, c["diff_hunk"].as_str()));
                    let mut d = one(format!("thread-{id}"), note);
                    d.resolvable = true;
                    d.resolved_by = text(&c["resolver"]["login"]);
                    d.resolved = d.resolved_by.is_some() || c["resolver"].is_object();
                    threads.push((id, at, d));
                }
            }
        }
        let mut at = HashMap::new();
        for (root, t, d) in threads {
            at.insert(root, t);
            out.push(d);
        }
        out.sort_by_key(|d| d.notes.first().map_or(0, |n| n.created_at));
        (out, at)
    }

    /// Who reacted with what (`/reactions`: `content` and `user`), grouped, in GitHub's order
    /// (Forgejo's default reactions are the same eight).
    pub fn reaction_list(list: &[Value], me: Option<u64>) -> Vec<ForgeReaction> {
        group_reactions(list.iter().filter_map(|r| Some((r["content"].as_str()?, r["user"]["id"].as_u64()?, r["user"]["login"].as_str().unwrap_or_default()))), me, true)
    }

    pub fn label(v: &Value) -> Option<ForgeLabel> {
        Some(ForgeLabel { name: v["name"].as_str()?.to_string(), color: v["color"].as_str().and_then(label_color), description: text(&v["description"]) })
    }

    /// A review's event word.
    pub fn event_word(e: ReviewEvent) -> &'static str {
        match e {
            ReviewEvent::Comment => "COMMENT",
            ReviewEvent::Approve => "APPROVED",
            ReviewEvent::RequestChanges => "REQUEST_CHANGES",
        }
    }

    /// A review's POST body: the event, and the message unless empty (an approval needs none).
    pub fn review_body(review: &ReviewSubmit) -> Value {
        let mut b = serde_json::json!({ "event": event_word(review.event) });
        if !review.body.trim().is_empty() {
            b["body"] = review.body.clone().into();
        }
        b
    }

    /// A one-comment review that lands in the thread at `at` (Forgejo threads code comments by
    /// file and line).
    pub fn thread_reply_body(at: &ThreadAt, body: &str) -> Value {
        let mut b = serde_json::json!({ "event": "COMMENT", "body": "", "comments": [{ "path": at.path, "body": body, "new_position": at.new_line, "old_position": at.old_line }] });
        if let Some(c) = &at.commit {
            b["commit_id"] = c.clone().into();
        }
        b
    }

    // --- review comments ---
    /// The commits a PR's diff is between: the merge base Forgejo diffs from (the base branch's
    /// tip when it doesn't say) and the head.
    pub fn diff_refs(pr: &Value) -> Option<DiffRefs> {
        let base = text(&pr["merge_base"]).or_else(|| text(&pr["base"]["sha"]))?;
        Some(DiffRefs { start_sha: base.clone(), base_sha: base, head_sha: text(&pr["head"]["sha"])? })
    }

    /// A path as git prints it in a diff: quoted when it holds a `"`, a `\`, a control or a
    /// non-ASCII byte (`"caf\303\251.txt"`: C escapes, each UTF-8 byte in octal), else bare.
    /// With how many bytes of `p` it took (all of a bare one).
    fn git_path(p: &str) -> Option<(String, usize)> {
        use gix::bstr::ByteSlice;
        let (path, n) = gix::quote::ansi_c::undo(p.as_bytes().as_bstr()).ok()?;
        Some((path.to_str_lossy().into_owned(), n))
    }

    /// A `diff --git a/x b/y` header's two paths. Git quotes each half on its own (`git_path`);
    /// an unquoted pair, whose paths may hold spaces, is split where both halves name the same
    /// file, else at ` b/`.
    fn header_paths(rest: &str) -> (String, String) {
        let side = |p: String, prefix: &str| p.strip_prefix(prefix).map_or_else(|| p.clone(), str::to_string);
        let quoted = || -> Option<(String, String)> {
            let (a, b) = if rest.starts_with('"') {
                let (a, n) = git_path(rest)?;
                (a, git_path(rest[n..].strip_prefix(' ')?)?.0)
            } else {
                // A bare old path holds no `"`, so the quoted new one starts at the first ` "b/`.
                let (a, _) = rest.split_once(" \"b/")?;
                (a.to_string(), git_path(&rest[a.len() + 1..])?.0)
            };
            Some((side(a, "a/"), side(b, "b/")))
        };
        // A bare path holds no `"`.
        if rest.contains('"')
            && let Some(paths) = quoted()
        {
            return paths;
        }
        let n = rest.len().saturating_sub(1) / 2;
        if rest.len() % 2 == 1
            && rest.is_char_boundary(n)
            && rest.as_bytes()[n] == b' '
            && let (Some(a), Some(b)) = (rest[..n].strip_prefix("a/"), rest[n + 1..].strip_prefix("b/"))
            && a == b
        {
            return (a.to_string(), a.to_string());
        }
        match rest.split_once(" b/") {
            Some((a, b)) => (side(a.to_string(), "a/"), b.to_string()),
            None => (rest.to_string(), rest.to_string()),
        }
    }

    /// `/pulls/{n}.diff`, file by file, each with its commentable lines (from its first `@@`). A
    /// rename's paths are its `rename from` / `rename to`; a binary file has no line to comment.
    pub fn review_files(diff: &str) -> Vec<ReviewFile> {
        struct File { path: String, old_path: String, hunks: String, in_hunks: bool }
        let done = |f: File| ReviewFile { lines: gitvolt_core::forge::review::commentable_lines(&f.hunks), path: f.path, old_path: f.old_path, too_large: false };
        let mut out = Vec::new();
        let mut cur: Option<File> = None;
        for line in diff.split_inclusive('\n') {
            let bare = line.trim_end_matches(['\n', '\r']);
            if let Some(rest) = bare.strip_prefix("diff --git ") {
                out.extend(cur.take().map(done));
                let (old_path, path) = header_paths(rest);
                cur = Some(File { path, old_path, hunks: String::new(), in_hunks: false });
                continue;
            }
            let Some(f) = cur.as_mut() else { continue };
            if f.in_hunks || bare.starts_with("@@") {
                f.in_hunks = true;
                f.hunks.push_str(line);
            } else if let Some((p, _)) = bare.strip_prefix("rename from ").and_then(git_path) {
                f.old_path = p;
            } else if let Some((p, _)) = bare.strip_prefix("rename to ").and_then(git_path) {
                f.path = p;
            }
        }
        out.extend(cur.map(done));
        out
    }

    /// A thread's state from its first comment (what `resolve` / `unresolve` answer).
    pub fn thread_state(root: &Value) -> ThreadState {
        let resolved_by = text(&root["resolver"]["login"]);
        ThreadState { resolved: resolved_by.is_some() || root["resolver"].is_object(), resolved_by }
    }

    /// A pending review's code comment as a draft.
    pub fn draft(c: &Value) -> Option<ReviewDraft> {
        let at = thread_at(c)?;
        Some(ReviewDraft { id: c["id"].as_u64()?.to_string(), body: c["body"].as_str().unwrap_or_default().to_string(), position: Some(position(&at, c["diff_hunk"].as_str())), reply_to: None })
    }

    /// The first line and how many follow it (`extra_lines_count`), on the anchor's last line's
    /// side: a removed line's is the old one, any other's the new one. A range starting on the
    /// other side starts at that line's number on this side.
    pub fn line_span(anchor: &ReviewAnchor) -> (DiffSide, u32, u32) {
        let side = anchor.end.side();
        let on = |l: &ReviewLine| match side {
            DiffSide::Old => l.old_line,
            DiffSide::New => l.new_line,
        };
        let last = on(&anchor.end);
        let first = anchor.start.as_ref().map_or(last, |s| on(s).min(last));
        (side, first, last - first)
    }

    /// A code comment for a review POST: its first line by its side's number, and the lines after
    /// it (`extra_lines_count`, sent only for a range).
    pub fn line_comment(anchor: &ReviewAnchor, body: &str) -> Value {
        let (side, first, extra) = line_span(anchor);
        let (old, new) = match side {
            DiffSide::Old => (first, 0),
            DiffSide::New => (0, first),
        };
        let mut c = serde_json::json!({ "path": anchor.path, "body": body, "new_position": new, "old_position": old });
        if extra > 0 {
            c["extra_lines_count"] = extra.into();
        }
        c
    }

    /// A draft kept here (`LocalDraft`), as the drafts list shows it: `local-<n>`, on its line.
    pub fn local_draft(d: &super::LocalDraft) -> ReviewDraft {
        let a = &d.anchor;
        let (side, first, extra) = line_span(a);
        let at = |n: u32| (Some(n + extra), (extra > 0).then_some(n));
        let ((line, start_line), (old_line, start_old_line)) = match side {
            DiffSide::Old => ((None, None), at(first)),
            DiffSide::New => (at(first), (None, None)),
        };
        let position = DiffPosition { path: a.path.clone(), old_path: (a.old_path != a.path).then(|| a.old_path.clone()), line, old_line, snippet: None, start_line, start_old_line, head_sha: Some(d.head.clone()), outdated: false };
        ReviewDraft { id: format!("local-{}", d.id), body: d.body.clone(), position: Some(position), reply_to: None }
    }

    /// A review POST with its message (`review_body`) and code comments, on `commit`.
    pub fn review_with(review: &ReviewSubmit, commit: Option<&str>, comments: Vec<Value>) -> Value {
        let mut b = review_body(review);
        b["comments"] = comments.into();
        if let Some(c) = commit {
            b["commit_id"] = c.into();
        }
        b
    }
    // --- end review comments ---

    /// The merge POST's body: the style, the expected head, the branch deletion when asked
    /// (else the repository's default), and `auto` for "when the checks succeed".
    pub fn merge_body(opts: &MergeOptions, auto: bool) -> Value {
        let method = opts.method.unwrap_or(if opts.squash == Some(true) { MergeMethod::Squash } else { MergeMethod::Merge });
        let mut b = serde_json::json!({ "Do": style_word(method) });
        if let Some(sha) = &opts.expected_sha {
            b["head_commit_id"] = sha.clone().into();
        }
        if let Some(d) = opts.delete_source_branch {
            b["delete_branch_after_merge"] = d.into();
        }
        if auto {
            b["merge_when_checks_succeed"] = true.into();
        }
        b
    }

    /// The POST /pulls body. A branch from a fork is `owner:branch`; a draft is a `WIP:` title.
    pub fn create_body(target: &str, req: &CreateMr) -> Value {
        let owner = req.source.project.split('/').next().unwrap_or("");
        let head = if req.source.project == target || owner.is_empty() { req.source.branch.clone() } else { format!("{owner}:{}", req.source.branch) };
        serde_json::json!({ "title": drafted(&req.title, req.draft), "head": head, "base": req.target_branch, "body": req.description })
    }

    /// The auto-merge the issue timeline says is scheduled now: its last schedule, unless a
    /// cancel came after it.
    pub fn auto_merge(timeline: &[Value]) -> Option<AutoMerge> {
        merge_schedule(timeline).flatten()
    }

    /// The newest schedule or cancel among `events`: `Some(None)` cancelled, `None` neither there.
    pub fn merge_schedule(events: &[Value]) -> Option<Option<AutoMerge>> {
        let last = events.iter().rev().find(|e| matches!(e["type"].as_str(), Some("pull_scheduled_merge" | "pull_cancel_scheduled_merge")))?;
        Some((last["type"] == "pull_scheduled_merge").then(|| AutoMerge { enabled_by: user(&last["user"]), method: None }))
    }

    /// A page URL's `page` number.
    pub fn page_number(url: &str) -> Option<u64> {
        url.split(['?', '&']).find_map(|kv| kv.strip_prefix("page=")).and_then(|n| n.parse().ok())
    }

    /// The ids of label `names` among the repository's `labels`.
    pub fn label_ids(labels: &[Value], names: &[String], path: &str) -> Result<Vec<u64>, GbError> {
        names
            .iter()
            .map(|n| labels.iter().find(|l| l["name"].as_str() == Some(n.as_str())).and_then(|l| l["id"].as_u64()).ok_or_else(|| GbError::new(GbErrorKind::InvalidInput, format!("{path} has no label {n}"))))
            .collect()
    }
}

impl ForgejoProvider {
    pub fn new(host: &str, endpoints: &HostEndpoints, token: Secret, cache: Option<Arc<DiskAvatarCache>>) -> Self {
        let api_base = endpoints.api.trim_end_matches('/').to_string();
        let http = HttpClient::new(ClientConfig { host: host.into(), api_base: api_base.clone(), token: Some(token), headers: FORGEJO_HEADERS.to_vec(), timeout: crate::http::REQUEST_TIMEOUT });
        Self {
            host: host.into(),
            api_base,
            web: endpoints.web.trim_end_matches('/').into(),
            http,
            cache,
            learned: Mutex::default(),
            me: Mutex::new(None),
            me_failed: Mutex::new(None),
            checks: Default::default(),
            names: KnownNames::default(),
            threads: Mutex::default(),
            drafts: Mutex::default(),
            next_draft: std::sync::atomic::AtomicU64::new(1),
        }
    }

    pub fn http(&self) -> &HttpClient {
        &self.http
    }

    /// See `HttpClient::with_change_counter`.
    pub fn with_change_counter(mut self, changes: crate::http::ChangeCounter) -> Self {
        self.http = self.http.with_change_counter(changes);
        self
    }

    fn fresh<T>(value: T, r: &HttpResponse) -> Fresh<T> {
        Fresh { value, not_modified: r.not_modified, poll_interval_secs: r.poll_interval_secs, fetched_at: unix_now() }
    }

    /// The account's own avatar paths: uploads (`/avatars/…`) and by email hash (`/avatar/…`).
    fn avatar_bases(&self) -> [String; 2] {
        [format!("{}/avatars", self.web), format!("{}/avatar", self.web)]
    }

    fn learn(&self, u: &ForgeUser) {
        if let (Some(email), Some(url)) = (&u.email, &u.avatar_url) {
            let email = email.trim().to_ascii_lowercase();
            if !email.is_empty() {
                self.learned.lock().expect("learned avatars poisoned").insert(email, url.clone());
            }
        }
        self.names.learn(u);
    }

    async fn user_json(&self) -> Result<ForgeUser, GbError> {
        let r = self.http.get("/user").await?;
        let user = json::user(&r.json(&self.host)?).ok_or_else(|| unreadable(&self.host, "user"))?;
        self.learn(&user);
        Ok(user)
    }

    /// The token's user, asked once.
    async fn me(&self) -> Result<ForgeUser, GbError> {
        let cached = self.me.lock().expect("me poisoned").clone();
        if let Some(u) = cached {
            return Ok(u);
        }
        let u = self.user_json().await?;
        *self.me.lock().expect("me poisoned") = Some(u.clone());
        Ok(u)
    }

    /// The token's user's id, or `None` when it can't be asked; a failed ask isn't repeated
    /// before `ME_RETRY`.
    async fn me_soft(&self) -> Option<u64> {
        if self.me_failed.lock().expect("me poisoned").is_some_and(|t| t.elapsed() < ME_RETRY) {
            return None;
        }
        match self.me().await {
            Ok(u) => Some(u.id),
            Err(_) => {
                *self.me_failed.lock().expect("me poisoned") = Some(std::time::Instant::now());
                None
            }
        }
    }

    async fn pull_json(&self, project: &ForgeProject, number: u64) -> Result<Value, GbError> {
        self.http.get(&format!("{}/pulls/{number}", GitHubProvider::repo_url(&project.path)?)).await?.json(&self.host)
    }

    /// The PR as it is now (after a write), with its scheduled auto-merge.
    async fn pull(&self, project: &ForgeProject, number: u64) -> Result<ForgeMr, GbError> {
        self.http.expire_fresh();
        let mut mr = json::pr(&self.pull_json(project, number).await?).ok_or_else(|| unreadable(&self.host, "pull request"))?;
        mr.auto_merge = self.auto_merge(project, &mr).await;
        Ok(mr)
    }

    /// An open PR's scheduled auto-merge, from its timeline (the PR itself doesn't say). Best effort.
    async fn auto_merge(&self, project: &ForgeProject, mr: &ForgeMr) -> Option<AutoMerge> {
        if !matches!(mr.state, MrState::Open | MrState::Draft) {
            return None;
        }
        let repo = GitHubProvider::repo_url(&project.path).ok()?;
        let page = |p: u64| format!("{repo}/issues/{}/timeline?limit={PER_PAGE}&page={p}", mr.number);
        let first = self.http.get(&page(1)).await.ok()?;
        let last = first.last_page.as_deref().and_then(json::page_number).unwrap_or(1);
        // The newest schedule or cancel decides: from the last page back, `TIMELINE_PAGES` at most.
        for p in (2..=last).rev().take(TIMELINE_PAGES) {
            let events: Vec<Value> = self.http.get(&page(p)).await.ok()?.json(&self.host).ok()?;
            if let Some(state) = json::merge_schedule(&events) {
                return state;
            }
        }
        json::merge_schedule(&first.json::<Vec<Value>>(&self.host).ok()?).flatten()
    }

    /// A commit's statuses as one pipeline; a token that can't read them has none.
    async fn status_soft(&self, project: &ForgeProject, sha: &str) -> Result<(Option<ForgePipeline>, bool), GbError> {
        let repo = GitHubProvider::repo_url(&project.path)?;
        match self.http.get(&format!("{repo}/commits/{}/status", encode_component(sha))).await {
            Ok(r) => Ok((json::status(&r.json(&self.host)?, &format!("{}/commit/{sha}", project.web_url)), r.not_modified)),
            Err(e) if e.kind == GbErrorKind::NotFound || crate::http::is_forbidden(&e) => Ok((None, false)),
            Err(e) => Err(e),
        }
    }

    /// The open PRs for `filter`, newest activity first; `with_checks`: the first page, with the
    /// first PIPELINE_LOOKUPS PRs' statuses (the list), else every page without (the badges).
    async fn list_open(&self, project: &ForgeProject, filter: MrFilter, with_checks: bool) -> Result<Fresh<Vec<ForgeMr>>, GbError> {
        let repo = GitHubProvider::repo_url(&project.path)?;
        let r = self.http.get(&format!("{repo}/pulls?state=open&sort=recentupdate&limit={PER_PAGE}")).await?;
        let mut list: Vec<Value> = r.json(&self.host)?;
        if !with_checks {
            let mut next = r.next_page.clone();
            for _ in 1..OPEN_PAGES {
                let Some(p) = next.take() else { break };
                let more = self.http.get(&p).await?;
                list.extend(more.json::<Vec<Value>>(&self.host)?);
                next = more.next_page.clone();
            }
        }
        let me = match filter {
            MrFilter::All => None,
            _ => Some(self.me().await?.username),
        };
        let keep = |v: &Value| match (filter, me.as_deref()) {
            (MrFilter::Mine, Some(me)) => v["user"]["login"].as_str() == Some(me),
            (MrFilter::ReviewRequested, Some(me)) => v["requested_reviewers"].as_array().is_some_and(|a| a.iter().any(|u| u["login"].as_str() == Some(me))),
            _ => true,
        };
        let mut mrs: Vec<ForgeMr> = list.iter().filter(|v| keep(v)).filter_map(json::pr).collect();
        if with_checks {
            let now = unix_now();
            let shas: Vec<String> = mrs.iter().filter_map(|m| m.head_sha.clone()).filter(|s| self.checks.needs([s.as_str()], now)).take(PIPELINE_LOOKUPS).collect();
            for sha in shas {
                match self.status_soft(project, &sha).await {
                    Ok((p, _)) => self.checks.put(&sha, p, now),
                    Err(e) if e.kind == GbErrorKind::RateLimited => break,
                    Err(e) => tracing::debug!("statuses of {sha}: {}", e.message),
                }
            }
            for m in &mut mrs {
                m.pipeline = m.head_sha.as_deref().and_then(|sha| self.checks.get(sha));
            }
        }
        self.names.learn_mrs(&mrs);
        Ok(Self::fresh(mrs, &r))
    }

    /// Who reacted to the comments (their JSON says nothing of it): the first REACTION_LOOKUPS.
    async fn fill_reactions(&self, repo: &str, ds: &mut [ForgeDiscussion]) {
        if !ds.iter().any(|d| !d.id.starts_with("review-")) {
            return;
        }
        let me = self.me().await.ok().map(|u| u.id);
        let mut asked = 0;
        for d in ds.iter_mut().filter(|d| !d.id.starts_with("review-")) {
            for n in d.notes.iter_mut() {
                if asked == REACTION_LOOKUPS {
                    return;
                }
                asked += 1;
                let Ok(list) = self.http.get_pages(&format!("{repo}/issues/comments/{}/reactions", n.id), 1).await else { continue };
                n.reactions = json::reaction_list(&list, me);
            }
        }
    }

    /// Every comment of the PR's submitted reviews that has any.
    async fn code_comments(&self, repo: &str, number: u64, reviews: &[Value]) -> Result<Vec<Value>, GbError> {
        let mut out = Vec::new();
        for r in reviews.iter().filter(|r| r["comments_count"].as_u64().unwrap_or(0) > 0 && r["state"].as_str() != Some("PENDING")) {
            let Some(id) = r["id"].as_u64() else { continue };
            out.extend(self.http.get_pages(&format!("{repo}/pulls/{number}/reviews/{id}/comments"), 1).await?);
        }
        Ok(out)
    }

    /// Forgejo's reviewers and assignees are logins; GitVolt's are ids (`/users/search?uid=`).
    async fn logins(&self, ids: &[u64]) -> Result<Vec<String>, GbError> {
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let v: Value = self.http.get(&format!("/users/search?uid={id}")).await?.json(&self.host)?;
            let u = v["data"].as_array().and_then(|a| a.iter().find_map(json::user)).ok_or_else(|| GbError::new(GbErrorKind::InvalidInput, format!("{} has no user {id}", self.host)))?;
            out.push(u.username);
        }
        Ok(out)
    }

    async fn label_ids(&self, project: &ForgeProject, names: &[String]) -> Result<Vec<u64>, GbError> {
        let repo = GitHubProvider::repo_url(&project.path)?;
        let all = self.http.get_pages(&format!("{repo}/labels?limit={PER_PAGE}"), LABEL_PAGES).await?;
        json::label_ids(&all, names, &project.path)
    }

    /// The assignees after `change`, as the whole list Forgejo's edit takes.
    async fn change_assignees(&self, project: &ForgeProject, number: u64, change: &PeopleEdit) -> Result<(), GbError> {
        self.http.expire_fresh();
        let pr = self.pull_json(project, number).await?;
        let current = json::users(&pr["assignees"]);
        let ids = change.apply(&current.iter().map(|u| u.id).collect::<Vec<_>>());
        let mut logins = Vec::with_capacity(ids.len());
        for id in ids {
            match current.iter().find(|u| u.id == id) {
                Some(u) => logins.push(u.username.clone()),
                None => logins.extend(self.logins(&[id]).await?),
            }
        }
        let path = format!("{}/pulls/{number}", GitHubProvider::repo_url(&project.path)?);
        self.http.send_json(Method::Patch, &path, &json!({ "assignees": logins })).await?;
        Ok(())
    }

    async fn change_reviewers(&self, project: &ForgeProject, number: u64, change: &PeopleEdit) -> Result<(), GbError> {
        let path = format!("{}/pulls/{number}/requested_reviewers", GitHubProvider::repo_url(&project.path)?);
        if !change.remove.is_empty() {
            self.http.send_json(Method::Delete, &path, &json!({ "reviewers": self.logins(&change.remove).await? })).await?;
        }
        if !change.add.is_empty() {
            self.http.send_json(Method::Post, &path, &json!({ "reviewers": self.logins(&change.add).await? })).await?;
        }
        Ok(())
    }

    /// The create's follow-up calls in order: reviewers, assignees, labels. A failed one is
    /// recorded and doesn't stop the next; an empty one is skipped.
    async fn follow_up(&self, project: &ForgeProject, number: u64, req: &CreateMr, parts: &[CreatePart]) -> Vec<PartFailure> {
        let mut failed = Vec::new();
        for part in [CreatePart::Reviewers, CreatePart::Assignees, CreatePart::Labels] {
            if !parts.contains(&part) {
                continue;
            }
            let done = match part {
                CreatePart::Reviewers if !req.reviewers.is_empty() => self.change_reviewers(project, number, &PeopleEdit { add: req.reviewers.clone(), remove: vec![] }).await,
                CreatePart::Assignees if !req.assignees.is_empty() => self.change_assignees(project, number, &PeopleEdit { add: req.assignees.clone(), remove: vec![] }).await,
                CreatePart::Labels if !req.labels.is_empty() => self.set_labels(project, number, &req.labels, Method::Post).await,
                _ => Ok(()),
            };
            if let Err(e) = done {
                failed.push(PartFailure { part, message: e.message.strip_prefix(&format!("{}: ", self.host)).unwrap_or(&e.message).to_string() });
            }
        }
        failed
    }

    /// Adds (`Post`) or replaces (`Put`) the PR's labels, by name.
    async fn set_labels(&self, project: &ForgeProject, number: u64, names: &[String], how: Method) -> Result<(), GbError> {
        let ids = self.label_ids(project, names).await?;
        let path = format!("{}/issues/{number}/labels", GitHubProvider::repo_url(&project.path)?);
        self.http.send_json(how, &path, &json!({ "labels": ids })).await?;
        Ok(())
    }

    /// `opts` with its branch deletion decided: as asked, else the repository's default
    /// (`default_delete_branch_after_merge`), which Forgejo's merge API doesn't apply by itself.
    async fn deleting_as_set(&self, project: &ForgeProject, opts: &MergeOptions) -> Result<MergeOptions, GbError> {
        let delete = match opts.delete_source_branch {
            Some(d) => d,
            None => self.project_settings(project).await?.delete_source_branch,
        };
        Ok(MergeOptions { delete_source_branch: Some(delete), ..opts.clone() })
    }

    // --- review comments ---
    /// The user's pending review on the PR (Forgejo lists only its author's): its id and the
    /// commit it's on.
    async fn pending_review(&self, repo: &str, number: u64) -> Result<Option<(u64, Option<String>)>, GbError> {
        let me = self.me_soft().await;
        let reviews = self.http.get_pages(&format!("{repo}/pulls/{number}/reviews?limit={PER_PAGE}"), COMMENT_PAGES).await?;
        Ok(reviews
            .iter()
            .find(|r| r["state"].as_str() == Some("PENDING") && me.is_none_or(|m| r["user"]["id"].as_u64() == Some(m)))
            .and_then(|r| Some((r["id"].as_u64()?, crate::github::json::text(&r["commit_id"])))))
    }

    async fn review_comments(&self, repo: &str, number: u64, review: u64) -> Result<Vec<Value>, GbError> {
        self.http.get_pages(&format!("{repo}/pulls/{number}/reviews/{review}/comments"), COMMENT_PAGES).await
    }

    /// A published code comment would take the user's pending review with it (Forgejo files every
    /// code comment of the user's under that review): refused while there's one.
    async fn refuse_beside_pending(&self, repo: &str, number: u64) -> Result<(), GbError> {
        match self.pending_review(repo, number).await? {
            Some(_) => Err(GbError::new(GbErrorKind::InvalidInput, format!("On Forgejo this would publish your pending review on #{number} with it: add it to the review, or submit or discard the review first"))),
            None => Ok(()),
        }
    }
    // --- end review comments ---

    /// Where review thread `root` is, from the last discussions read (or read now).
    async fn thread(&self, project: &ForgeProject, number: u64, root: u64) -> Result<ThreadAt, GbError> {
        let key = (project.path.clone(), number);
        let known = |s: &Self| s.threads.lock().expect("threads poisoned").get(&key).and_then(|m| m.get(&root)).cloned();
        if let Some(at) = known(self) {
            return Ok(at);
        }
        self.discussions(project, number).await?;
        known(self).ok_or_else(|| GbError::new(GbErrorKind::NotFound, format!("Forgejo has no such review thread on #{number}")))
    }
}

/// A conversation comment's (`issue-…`) or code comment's (`thread-…`) URL: both are issue
/// comments to Forgejo. A review's summary (`review-…`) is neither.
pub fn comment_url(repo: &str, note: &NoteRef) -> Result<String, GbError> {
    if !note.discussion.starts_with("issue-") && !note.discussion.starts_with("thread-") {
        return Err(GbError::new(GbErrorKind::InvalidInput, "A review's summary can't be changed here: open it on Forgejo"));
    }
    if note.note.is_empty() || !note.note.bytes().all(|b| b.is_ascii_digit()) {
        return Err(GbError::new(GbErrorKind::InvalidInput, "Forgejo has no such comment"));
    }
    Ok(format!("{repo}/issues/comments/{}", note.note))
}

/// Forgejo's refusals of a merge, said plainly: the head moved, or it can't merge now (405).
pub fn merge_refused(number: u64, e: GbError) -> GbError {
    let lower = e.message.to_ascii_lowercase();
    if lower.contains("out of date") || lower.contains("does not match") || lower.contains("head commit") {
        return GbError::new(GbErrorKind::InvalidInput, format!("#{number} changed since it was loaded: refresh and try again"));
    }
    if e.message.contains("HTTP 405") {
        return GbError::new(GbErrorKind::InvalidInput, format!("Forgejo can't merge #{number} now: refresh to see why"));
    }
    e
}

impl ForgeProvider for ForgejoProvider {
    fn kind(&self) -> ForgeKind {
        ForgeKind::Forgejo
    }

    fn host(&self) -> &str {
        &self.host
    }

    fn export_responses(&self, project: &ForgeProject) -> Vec<StoredResponse> {
        let Ok(repo) = GitHubProvider::repo_url(&project.path) else { return Vec::new() };
        let base = self.http.url(&repo);
        self.http.export(|k| k.starts_with(&format!("{base}/pulls?")) || k == base)
    }

    fn import_responses(&self, entries: Vec<StoredResponse>) {
        self.http.import(entries);
    }

    fn take_request_stats(&self) -> RequestStats {
        self.http.take_stats()
    }

    /// Forgejo has none by default; one an instance sets is read from its headers as GitLab's.
    fn rate_limit(&self) -> RateLimitState {
        self.http.rate_limit()
    }

    fn expire_fresh(&self) {
        self.http.expire_fresh();
    }

    fn current_user(&self) -> ForgeFuture<'_, ForgeUser> {
        Box::pin(self.user_json())
    }

    /// A token's scopes can't be read back: whether it writes is `Unknown`, and a write it may
    /// not make is refused (403) when it's tried.
    fn check_token(&self) -> ForgeFuture<'_, TokenCheck> {
        Box::pin(async move { Ok(TokenCheck { user: self.user_json().await?, write: WriteAccess::Unknown }) })
    }

    /// `11.0.1+gitea-1.22.0`.
    fn version(&self) -> ForgeFuture<'_, Option<String>> {
        Box::pin(async move {
            let r = self.http.get("/version").await?;
            Ok(json::text_of(&r.json::<Value>(&self.host)?["version"]))
        })
    }

    fn project<'a>(&'a self, path: &'a str) -> ForgeFuture<'a, Fresh<ForgeProject>> {
        Box::pin(async move {
            let r = self.http.get(&GitHubProvider::repo_url(path)?).await?;
            let p = json::project(&self.host, &r.json(&self.host)?).ok_or_else(|| unreadable(&self.host, "repository"))?;
            Ok(Self::fresh(p, &r))
        })
    }

    fn project_settings<'a>(&'a self, project: &'a ForgeProject) -> ForgeFuture<'a, ForgeProjectSettings> {
        Box::pin(async move {
            let r = self.http.get(&GitHubProvider::repo_url(&project.path)?).await?;
            Ok(json::settings(&r.json(&self.host)?))
        })
    }

    fn forks<'a>(&'a self, project: &'a ForgeProject) -> ForgeFuture<'a, Vec<ForgeProject>> {
        Box::pin(async move {
            let path = format!("{}/forks?limit={PER_PAGE}", GitHubProvider::repo_url(&project.path)?);
            let mut forks: Vec<ForgeProject> = self.http.get_pages(&path, FORK_PAGES).await?.iter().filter_map(|v| json::project(&self.host, v)).collect();
            forks.sort_by_key(|f| std::cmp::Reverse(f.updated_at));
            Ok(forks)
        })
    }

    fn forks_page<'a>(&'a self, project: &'a ForgeProject, page: u32, per_page: u32) -> ForgeFuture<'a, ForkPage> {
        Box::pin(async move {
            let path = format!("{}/forks?page={page}&limit={per_page}", GitHubProvider::repo_url(&project.path)?);
            let r = self.http.get(&path).await?;
            let items: Vec<Value> = r.json(&self.host)?;
            Ok(ForkPage { forks: items.iter().filter_map(|v| json::project(&self.host, v)).collect(), next: r.next_page.is_some().then(|| page + 1) })
        })
    }

    /// The host's own avatar paths (or Gravatar's), checked by `avatar_fetch_url`, never with
    /// the token: `own_origin` is the API's, and avatars aren't under it.
    fn avatar_at<'a>(&'a self, url: &'a str) -> Option<ForgeFuture<'a, Option<AvatarPayload>>> {
        let bases = self.avatar_bases();
        let url = avatar_fetch_url(url, &[bases[0].as_str(), bases[1].as_str()])?;
        Some(Box::pin(async move { image_at(&self.http, self.cache.as_deref(), &url, &self.api_base).await }))
    }

    /// Forgejo can't look a picture up by email: only an email → avatar seen in API data (a
    /// user's public email), on the host's own avatar paths, fetched without the token.
    fn avatar_for_email<'a>(&'a self, email: &'a str) -> ForgeFuture<'a, Option<AvatarPayload>> {
        Box::pin(async move {
            let email = email.trim().to_ascii_lowercase();
            let bases = self.avatar_bases();
            let learned = self.learned.lock().expect("learned avatars poisoned").get(&email).cloned();
            let Some(url) = learned.and_then(|u| avatar_fetch_url(&u, &[bases[0].as_str(), bases[1].as_str()])).filter(|u| !is_gravatar_url(u)) else { return Ok(None) };
            if let Some(cache) = &self.cache {
                match cache.lookup(&email) {
                    Lookup::Found(p) => return Ok(Some(p)),
                    Lookup::Missing => return Ok(None),
                    Lookup::Unknown => {}
                }
            }
            Ok(match (self.http.get_image(&url, &self.api_base).await?, &self.cache) {
                (Some((ct, bytes)), Some(cache)) => cache.store_found(&email, &ct, &bytes),
                (Some((ct, bytes)), None) => payload_of(&ct, &bytes),
                (None, Some(cache)) => {
                    cache.store_missing(&email);
                    None
                }
                (None, None) => None,
            })
        })
    }

    /// Only `known_names` (free): the one person seen with exactly that name or login.
    fn avatar_for_name<'a>(&'a self, email: &'a str, name: &'a str) -> ForgeFuture<'a, Option<AvatarPayload>> {
        Box::pin(async move {
            let email = email.trim().to_ascii_lowercase();
            if email.is_empty() {
                return Ok(None);
            }
            if let Some(cache) = &self.cache
                && let Lookup::Found(p) = cache.lookup(&email)
            {
                return Ok(Some(p));
            }
            let NameMatch::One(Some(url)) = self.names.find(name) else { return Ok(None) };
            let bases = self.avatar_bases();
            let Some(url) = avatar_fetch_url(&url, &[bases[0].as_str(), bases[1].as_str()]).filter(|u| !is_gravatar_url(u)) else { return Ok(None) };
            self.learned.lock().expect("learned avatars poisoned").insert(email.clone(), url.clone());
            Ok(match (self.http.get_image(&url, &self.api_base).await?, &self.cache) {
                (Some((ct, bytes)), Some(cache)) => cache.store_found(&email, &ct, &bytes),
                (Some((ct, bytes)), None) => payload_of(&ct, &bytes),
                (None, _) => None,
            })
        })
    }

    fn open_mrs<'a>(&'a self, project: &'a ForgeProject, filter: MrFilter) -> ForgeFuture<'a, Fresh<Vec<ForgeMr>>> {
        Box::pin(self.list_open(project, filter, true))
    }

    fn open_mrs_light<'a>(&'a self, project: &'a ForgeProject, filter: MrFilter) -> ForgeFuture<'a, Fresh<Vec<ForgeMr>>> {
        Box::pin(self.list_open(project, filter, false))
    }

    /// Forgejo's list has no head filter: its open PR from the open list, else its most recently
    /// updated closed one.
    fn mr_for_branch<'a>(&'a self, project: &'a ForgeProject, source: &'a SourceRef) -> ForgeFuture<'a, Fresh<Option<ForgeMr>>> {
        Box::pin(async move {
            let ours = |m: &ForgeMr| m.source_branch == source.branch && m.source_project == source.project;
            let open = self.list_open(project, MrFilter::All, false).await?;
            if let Some(m) = open.value.iter().find(|m| ours(m)).cloned() {
                return Ok(open.map(|_| Some(m)));
            }
            let repo = GitHubProvider::repo_url(&project.path)?;
            let r = self.http.get(&format!("{repo}/pulls?state=closed&sort=recentupdate&limit={PER_PAGE}")).await?;
            let list: Vec<Value> = r.json(&self.host)?;
            Ok(Self::fresh(crate::github::json::pick_for_branch(list.iter().filter_map(json::pr).filter(|m| ours(m)).collect()), &r))
        })
    }

    fn mr_detail<'a>(&'a self, project: &'a ForgeProject, number: u64) -> ForgeFuture<'a, Fresh<ForgeMrDetail>> {
        Box::pin(async move {
            let repo = GitHubProvider::repo_url(&project.path)?;
            let r = self.http.get(&format!("{repo}/pulls/{number}")).await?;
            let v: Value = r.json(&self.host)?;
            let reviews = self.http.get_pages(&format!("{repo}/pulls/{number}/reviews?limit={PER_PAGE}"), COMMENT_PAGES).await?;
            let mut mr = json::pr(&v).ok_or_else(|| unreadable(&self.host, "pull request"))?;
            if let Some(sha) = mr.head_sha.clone() {
                mr.pipeline = self.status_soft(project, &sha).await?.0;
            }
            mr.review = json::review(&reviews, &v["requested_reviewers"]);
            mr.auto_merge = self.auto_merge(project, &mr).await;
            let fork = v["head"]["repo"]["full_name"] != v["base"]["repo"]["full_name"];
            let me = if fork { self.me_soft().await } else { None };
            let subscribed = match self.http.get(&format!("{repo}/issues/{number}/subscriptions/check")).await {
                Ok(s) => s.json::<Value>(&self.host).ok().and_then(|w| w["subscribed"].as_bool()),
                Err(_) => None,
            };
            let merged_sha = if mr.state == MrState::Merged { json::text_of(&v["merge_commit_sha"]) } else { None };
            let detail = ForgeMrDetail {
                description: v["body"].as_str().unwrap_or_default().to_string(),
                reviewers: mr.review.reviews.iter().map(|x| x.user.clone()).collect(),
                assignees: json::users(&v["assignees"]),
                merge_status: json::merge_status(&v),
                squash: None,
                delete_source_branch: None,
                // Forgejo says the merge base the diff is from.
                base_sha: json::text_of(&v["merge_base"]).or_else(|| json::text_of(&v["base"]["sha"])),
                subscribed,
                body_html: None,
                update: json::update_offer(&v, me),
                merged_sha,
                mr,
            };
            self.names.learn_detail(&detail);
            Ok(Fresh::new(detail, unix_now()))
        })
    }

    fn discussions<'a>(&'a self, project: &'a ForgeProject, number: u64) -> ForgeFuture<'a, Fresh<Vec<ForgeDiscussion>>> {
        Box::pin(async move {
            let repo = GitHubProvider::repo_url(&project.path)?;
            let comments = self.http.get_pages(&format!("{repo}/issues/{number}/comments?limit={PER_PAGE}"), COMMENT_PAGES).await?;
            let reviews = self.http.get_pages(&format!("{repo}/pulls/{number}/reviews?limit={PER_PAGE}"), COMMENT_PAGES).await?;
            let code = self.code_comments(&repo, number, &reviews).await?;
            let (mut ds, at) = json::discussions(&comments, &reviews, &code);
            self.threads.lock().expect("threads poisoned").insert((project.path.clone(), number), at);
            self.fill_reactions(&repo, &mut ds).await;
            self.names.learn_discussions(&ds);
            Ok(Fresh::new(ds, unix_now()))
        })
    }

    /// A review thread (`thread-<root>`) gets a one-comment review on its line, which Forgejo
    /// shows in that thread; anything else goes to the conversation.
    fn reply<'a>(&'a self, project: &'a ForgeProject, number: u64, note: &'a NewNote) -> ForgeFuture<'a, ForgeNote> {
        Box::pin(async move {
            let repo = GitHubProvider::repo_url(&project.path)?;
            let Some(root) = note.discussion.as_deref().and_then(|d| d.strip_prefix("thread-")).and_then(|r| r.parse::<u64>().ok()) else {
                let r = self.http.send_json(Method::Post, &format!("{repo}/issues/{number}/comments"), &json!({ "body": note.body })).await?;
                return json::comment_note(&r.json(&self.host)?).ok_or_else(|| unreadable(&self.host, "comment"));
            };
            let at = self.thread(project, number, root).await?;
            self.refuse_beside_pending(&repo, number).await?;
            let r = self.http.send_json(Method::Post, &format!("{repo}/pulls/{number}/reviews"), &json::thread_reply_body(&at, &note.body)).await?;
            let id = r.json::<Value>(&self.host)?["id"].as_u64().ok_or_else(|| unreadable(&self.host, "review"))?;
            let comments = self.http.get_pages(&format!("{repo}/pulls/{number}/reviews/{id}/comments"), 1).await?;
            comments.first().and_then(json::comment_note).ok_or_else(|| unreadable(&self.host, "review comment"))
        })
    }

    /// The comment's reactions, read first; then the one asked for added or removed, unless it's
    /// so already.
    fn react<'a>(&'a self, project: &'a ForgeProject, _number: u64, note: &'a NoteRef, name: &'a str, on: bool) -> ForgeFuture<'a, Vec<ForgeReaction>> {
        Box::pin(async move {
            let url = format!("{}/reactions", comment_url(&GitHubProvider::repo_url(&project.path)?, note)?);
            let me = self.me().await?;
            let mut list = self.http.get_pages(&url, 1).await?;
            let mine = list.iter().position(|r| r["content"].as_str() == Some(name) && r["user"]["id"].as_u64() == Some(me.id));
            match (on, mine) {
                (true, None) => {
                    let r = self.http.send_json(Method::Post, &url, &json!({ "content": name })).await?;
                    list.push(r.json(&self.host)?);
                }
                (false, Some(i)) => {
                    self.http.send_json(Method::Delete, &url, &json!({ "content": name })).await?;
                    list.remove(i);
                }
                _ => {}
            }
            Ok(json::reaction_list(&list, Some(me.id)))
        })
    }

    fn edit_note<'a>(&'a self, project: &'a ForgeProject, _number: u64, note: &'a NoteRef, body: &'a str) -> ForgeFuture<'a, ForgeNote> {
        Box::pin(async move {
            let r = self.http.send_json(Method::Patch, &comment_url(&GitHubProvider::repo_url(&project.path)?, note)?, &json!({ "body": body })).await?;
            json::comment_note(&r.json(&self.host)?).ok_or_else(|| unreadable(&self.host, "comment"))
        })
    }

    /// A conversation comment only: Forgejo's API answers a code comment's delete without deleting it.
    fn delete_note<'a>(&'a self, project: &'a ForgeProject, _number: u64, note: &'a NoteRef) -> ForgeFuture<'a, ()> {
        Box::pin(async move {
            if !note.discussion.starts_with("issue-") {
                return Err(GbError::new(GbErrorKind::InvalidInput, "Forgejo's API can't delete a review comment: delete it on Forgejo"));
            }
            self.http.delete(&comment_url(&GitHubProvider::repo_url(&project.path)?, note)?).await?;
            Ok(())
        })
    }

    fn approve<'a>(&'a self, project: &'a ForgeProject, number: u64) -> ForgeFuture<'a, ()> {
        Box::pin(async move {
            self.review(project, number, &ReviewSubmit { event: ReviewEvent::Approve, body: String::new() }).await?;
            Ok(())
        })
    }

    fn request_changes<'a>(&'a self, project: &'a ForgeProject, number: u64, body: &'a str) -> ForgeFuture<'a, ()> {
        Box::pin(async move {
            self.review(project, number, &ReviewSubmit { event: ReviewEvent::RequestChanges, body: body.to_string() }).await?;
            Ok(())
        })
    }

    /// One review: `POST /pulls/{n}/reviews` with the event and its message.
    fn review<'a>(&'a self, project: &'a ForgeProject, number: u64, review: &'a ReviewSubmit) -> ForgeFuture<'a, ReviewOutcome> {
        Box::pin(async move {
            let repo = GitHubProvider::repo_url(&project.path)?;
            self.http.send_json(Method::Post, &format!("{repo}/pulls/{number}/reviews"), &json::review_body(review)).await?;
            Ok(ReviewOutcome::default())
        })
    }

    /// `PUT` / `DELETE /issues/{n}/subscriptions/{login}`.
    fn set_subscribed<'a>(&'a self, project: &'a ForgeProject, number: u64, on: bool) -> ForgeFuture<'a, bool> {
        Box::pin(async move {
            let me = self.me().await?;
            let path = format!("{}/issues/{number}/subscriptions/{}", GitHubProvider::repo_url(&project.path)?, encode_component(&me.username));
            if on {
                self.http.send_json(Method::Put, &path, &json!({})).await?;
            } else {
                self.http.delete(&path).await?;
            }
            Ok(on)
        })
    }

    /// `pulls/comments/{root}/resolve` (or `unresolve`) on the thread's first comment. A Forgejo
    /// without that API answers 404, as for a comment that's gone: both are said.
    fn resolve<'a>(&'a self, project: &'a ForgeProject, _number: u64, discussion: &'a str, resolved: bool) -> ForgeFuture<'a, ThreadState> {
        Box::pin(async move {
            let root = discussion.strip_prefix("thread-").and_then(|r| r.parse::<u64>().ok()).ok_or_else(|| GbError::new(GbErrorKind::InvalidInput, "Only a review thread can be resolved"))?;
            let repo = GitHubProvider::repo_url(&project.path)?;
            let verb = if resolved { "resolve" } else { "unresolve" };
            let r = self.http.send_json(Method::Post, &format!("{repo}/pulls/comments/{root}/{verb}"), &json!({})).await.map_err(|e| {
                if e.kind == GbErrorKind::NotFound {
                    GbError::new(GbErrorKind::NotFound, format!("{} didn't {verb} the thread: its comment is gone, or this Forgejo is older than its API for it", self.host))
                } else {
                    e
                }
            })?;
            Ok(json::thread_state(&r.json(&self.host)?))
        })
    }

    // --- review comments ---
    fn review_diff<'a>(&'a self, project: &'a ForgeProject, number: u64) -> ForgeFuture<'a, ReviewDiff> {
        Box::pin(async move {
            let repo = GitHubProvider::repo_url(&project.path)?;
            let refs = json::diff_refs(&self.pull_json(project, number).await?).ok_or_else(|| unreadable(&self.host, "pull request"))?;
            let r = self.http.get(&format!("{repo}/pulls/{number}.diff")).await?;
            Ok(ReviewDiff { refs, files: json::review_files(&String::from_utf8_lossy(&r.body)) })
        })
    }

    fn review_drafts<'a>(&'a self, project: &'a ForgeProject, number: u64) -> ForgeFuture<'a, ReviewDrafts> {
        Box::pin(async move {
            let repo = GitHubProvider::repo_url(&project.path)?;
            let pending = self.pending_review(&repo, number).await?;
            let mut drafts: Vec<ReviewDraft> = match pending {
                Some((id, _)) => self.review_comments(&repo, number, id).await?.iter().filter_map(json::draft).collect(),
                None => Vec::new(),
            };
            let local: Vec<ReviewDraft> = self.drafts.lock().expect("drafts poisoned").get(&(project.path.clone(), number)).map(|l| l.iter().map(json::local_draft).collect()).unwrap_or_default();
            let pending_review = pending.map(|(id, _)| id.to_string()).or_else(|| (!local.is_empty()).then(|| "local".to_string()));
            drafts.extend(local);
            let refs = json::diff_refs(&self.pull_json(project, number).await?);
            Ok(ReviewDrafts { refs, drafts, pending_review, can_draft: true })
        })
    }

    /// Kept here until its review goes (`LocalDraft`): no request.
    fn add_draft<'a>(&'a self, project: &'a ForgeProject, number: u64, comment: &'a NewReviewComment) -> ForgeFuture<'a, ReviewDraft> {
        Box::pin(async move {
            let id = self.next_draft.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let d = LocalDraft { id, anchor: comment.anchor.clone(), body: comment.body.clone(), head: comment.refs.head_sha.clone() };
            let view = json::local_draft(&d);
            self.drafts.lock().expect("drafts poisoned").entry((project.path.clone(), number)).or_default().push(d);
            Ok(view)
        })
    }

    /// A draft kept here changes here; one of a pending review made on the web is its comment.
    fn edit_draft<'a>(&'a self, project: &'a ForgeProject, number: u64, id: &'a str, body: &'a str) -> ForgeFuture<'a, ReviewDraft> {
        Box::pin(async move {
            if let Some(n) = id.strip_prefix("local-").and_then(|n| n.parse::<u64>().ok()) {
                let mut drafts = self.drafts.lock().expect("drafts poisoned");
                let d = drafts.get_mut(&(project.path.clone(), number)).and_then(|l| l.iter_mut().find(|d| d.id == n)).ok_or_else(|| GbError::new(GbErrorKind::NotFound, "That draft is gone"))?;
                d.body = body.to_string();
                return Ok(json::local_draft(d));
            }
            let repo = GitHubProvider::repo_url(&project.path)?;
            let r = self.http.send_json(Method::Patch, &format!("{repo}/issues/comments/{}", encode_component(id)), &json!({ "body": body })).await?;
            let v: Value = r.json(&self.host)?;
            Ok(ReviewDraft { id: id.to_string(), body: v["body"].as_str().unwrap_or(body).to_string(), position: None, reply_to: None })
        })
    }

    /// A draft kept here just goes. One of a pending review made on the web is deleted on the
    /// forge (`DELETE …/reviews/{id}/comments/{comment}`), the review and its other comments kept;
    /// a Forgejo without that API (404 or 405) is said, nothing changed.
    fn delete_draft<'a>(&'a self, project: &'a ForgeProject, number: u64, id: &'a str) -> ForgeFuture<'a, ()> {
        Box::pin(async move {
            if let Some(n) = id.strip_prefix("local-").and_then(|n| n.parse::<u64>().ok()) {
                if let Some(l) = self.drafts.lock().expect("drafts poisoned").get_mut(&(project.path.clone(), number)) {
                    l.retain(|d| d.id != n);
                }
                return Ok(());
            }
            let repo = GitHubProvider::repo_url(&project.path)?;
            let Some((review, _)) = self.pending_review(&repo, number).await? else { return Ok(()) };
            self.http.delete(&format!("{repo}/pulls/{number}/reviews/{review}/comments/{}", encode_component(id))).await.map_err(|e| {
                if e.kind == GbErrorKind::NotFound || e.message.contains("HTTP 405") {
                    GbError::new(GbErrorKind::NotFound, format!("{} didn't delete the comment: it's gone, or this Forgejo can't delete one comment of a pending review. Discard the review, or delete the comment on the web", self.host))
                } else {
                    e
                }
            })?;
            Ok(())
        })
    }

    /// The review with the event, the message and the drafts written against the PR's head, as
    /// one POST (Forgejo files a pending review made on the web under it too); without such
    /// drafts, that pending review is submitted, or else the composer's review goes. Drafts
    /// written against an older head follow, one comment review per head on its commit: Forgejo
    /// places a review's comments on its one `commit_id`, and their lines are that commit's.
    /// Whatever wasn't sent stays.
    fn submit_review<'a>(&'a self, project: &'a ForgeProject, number: u64, review: &'a ReviewSubmit) -> ForgeFuture<'a, SubmitOutcome> {
        Box::pin(async move {
            let repo = GitHubProvider::repo_url(&project.path)?;
            let posted = !review.body.trim().is_empty();
            let key = (project.path.clone(), number);
            let local = self.drafts.lock().expect("drafts poisoned").remove(&key).unwrap_or_default();
            let keep = |left: Vec<LocalDraft>| {
                if !left.is_empty() {
                    self.drafts.lock().expect("drafts poisoned").entry(key.clone()).or_default().splice(0..0, left);
                }
            };
            let head = if local.is_empty() {
                None
            } else {
                match self.pull_json(project, number).await {
                    Ok(v) => crate::github::json::text(&v["head"]["sha"]),
                    Err(e) => {
                        keep(local);
                        return Err(e);
                    }
                }
            };
            let (current, older): (Vec<LocalDraft>, Vec<LocalDraft>) = local.into_iter().partition(|d| head.as_deref() == Some(d.head.as_str()));
            let comments = |ds: &[LocalDraft]| ds.iter().map(|d| json::line_comment(&d.anchor, &d.body)).collect::<Vec<_>>();
            let sent = if !current.is_empty() {
                self.http.send_json(Method::Post, &format!("{repo}/pulls/{number}/reviews"), &json::review_with(review, head.as_deref(), comments(&current))).await.map(|_| ())
            } else {
                match self.pending_review(&repo, number).await {
                    Ok(Some((id, _))) => self.http.send_json(Method::Post, &format!("{repo}/pulls/{number}/reviews/{id}"), &json::review_body(review)).await.map(|_| ()),
                    // Only drafts on older heads, and nothing to say: they're the review.
                    Ok(None) if review.event == ReviewEvent::Comment && !posted && !older.is_empty() => Ok(()),
                    Ok(None) => self.review(project, number, review).await.map(|_| ()),
                    Err(e) => Err(e),
                }
            };
            if let Err(e) = sent {
                keep(current.into_iter().chain(older).collect());
                return Err(e);
            }
            let mut heads: Vec<String> = Vec::new();
            for d in &older {
                if !heads.contains(&d.head) {
                    heads.push(d.head.clone());
                }
            }
            let mut left = older;
            for h in heads {
                let (these, rest): (Vec<LocalDraft>, Vec<LocalDraft>) = left.into_iter().partition(|d| d.head == h);
                let note = ReviewSubmit { event: ReviewEvent::Comment, body: String::new() };
                if let Err(e) = self.http.send_json(Method::Post, &format!("{repo}/pulls/{number}/reviews"), &json::review_with(&note, Some(&h), comments(&these))).await {
                    let n = these.len() + rest.len();
                    keep(these.into_iter().chain(rest).collect());
                    return Err(GbError::new(e.kind, format!("Your review went in, but {n} comment{} written against an older commit didn't, and stay pending: {}", if n == 1 { "" } else { "s" }, e.message)));
                }
                left = rest;
            }
            Ok(SubmitOutcome { body_posted: posted, event_sent: true, ..SubmitOutcome::default() })
        })
    }

    fn discard_review<'a>(&'a self, project: &'a ForgeProject, number: u64) -> ForgeFuture<'a, u32> {
        Box::pin(async move {
            let local = self.drafts.lock().expect("drafts poisoned").remove(&(project.path.clone(), number)).map_or(0, |l| l.len() as u32);
            let repo = GitHubProvider::repo_url(&project.path)?;
            let Some((id, _)) = self.pending_review(&repo, number).await? else { return Ok(local) };
            let count = self.review_comments(&repo, number, id).await?.len() as u32;
            self.http.delete(&format!("{repo}/pulls/{number}/reviews/{id}")).await?;
            Ok(local + count)
        })
    }

    /// A one-comment review, published at once: its thread.
    fn comment_now<'a>(&'a self, project: &'a ForgeProject, number: u64, comment: &'a NewReviewComment) -> ForgeFuture<'a, ForgeDiscussion> {
        Box::pin(async move {
            let repo = GitHubProvider::repo_url(&project.path)?;
            self.refuse_beside_pending(&repo, number).await?;
            let body = json::review_with(&ReviewSubmit { event: ReviewEvent::Comment, body: String::new() }, Some(&comment.refs.head_sha), vec![json::line_comment(&comment.anchor, &comment.body)]);
            let r = self.http.send_json(Method::Post, &format!("{repo}/pulls/{number}/reviews"), &body).await?;
            let id = r.json::<Value>(&self.host)?["id"].as_u64().ok_or_else(|| unreadable(&self.host, "review"))?;
            let comments = self.review_comments(&repo, number, id).await?;
            let (ds, at) = json::discussions(&[], &[], &comments);
            self.threads.lock().expect("threads poisoned").entry((project.path.clone(), number)).or_default().extend(at);
            ds.into_iter().next().ok_or_else(|| unreadable(&self.host, "review comment"))
        })
    }
    // --- end review comments ---

    /// Branch deletion as asked, else as the repository's "delete the branch after merging" says:
    /// Forgejo's API applies that setting only when the merge asks for it (it's the web form's
    /// default), so it's sent explicitly.
    fn merge<'a>(&'a self, project: &'a ForgeProject, number: u64, opts: &'a MergeOptions) -> ForgeFuture<'a, ForgeMr> {
        Box::pin(async move {
            let repo = GitHubProvider::repo_url(&project.path)?;
            let opts = self.deleting_as_set(project, opts).await?;
            self.http.send_json(Method::Post, &format!("{repo}/pulls/{number}/merge"), &json::merge_body(&opts, false)).await.map_err(|e| merge_refused(number, e))?;
            self.pull(project, number).await
        })
    }

    /// The merge with `merge_when_checks_succeed`: Forgejo merges at once when they have.
    fn set_auto_merge<'a>(&'a self, project: &'a ForgeProject, number: u64, opts: &'a MergeOptions) -> ForgeFuture<'a, ForgeMr> {
        Box::pin(async move {
            let repo = GitHubProvider::repo_url(&project.path)?;
            let opts = self.deleting_as_set(project, opts).await?;
            self.http.send_json(Method::Post, &format!("{repo}/pulls/{number}/merge"), &json::merge_body(&opts, true)).await.map_err(|e| merge_refused(number, e))?;
            self.pull(project, number).await
        })
    }

    fn cancel_auto_merge<'a>(&'a self, project: &'a ForgeProject, number: u64) -> ForgeFuture<'a, ForgeMr> {
        Box::pin(async move {
            self.http.delete(&format!("{}/pulls/{number}/merge", GitHubProvider::repo_url(&project.path)?)).await?;
            self.pull(project, number).await
        })
    }

    /// `POST /pulls/{n}/update?style=merge|rebase`, which Forgejo does before it answers. It
    /// takes no expected head: the head is checked just before.
    fn update_branch<'a>(&'a self, project: &'a ForgeProject, number: u64, how: BranchUpdate, expected_sha: Option<&'a str>) -> ForgeFuture<'a, ForgeMr> {
        Box::pin(async move {
            let style = match how {
                BranchUpdate::Merge => "merge",
                BranchUpdate::Rebase => "rebase",
                BranchUpdate::RebaseSkipCi => return Err(GbError::new(GbErrorKind::InvalidInput, "Forgejo can't rebase without running the checks")),
            };
            if let Some(sha) = expected_sha {
                self.http.expire_fresh();
                let now = self.pull_json(project, number).await?;
                if !now["head"]["sha"].as_str().is_some_and(|h| h.eq_ignore_ascii_case(sha)) {
                    return Err(GbError::new(GbErrorKind::InvalidInput, format!("#{number} changed since it was loaded: refresh and try again")));
                }
            }
            let path = format!("{}/pulls/{number}/update?style={style}", GitHubProvider::repo_url(&project.path)?);
            self.http.send_json(Method::Post, &path, &json!({})).await?;
            self.pull(project, number).await
        })
    }

    fn edit<'a>(&'a self, project: &'a ForgeProject, number: u64, edit: &'a MrEdit) -> ForgeFuture<'a, ForgeMr> {
        Box::pin(async move {
            if let Some(change) = edit.reviewers.as_ref().filter(|c| !c.is_empty()) {
                self.change_reviewers(project, number, change).await?;
            }
            if let Some(change) = edit.assignees.as_ref().filter(|c| !c.is_empty()) {
                self.change_assignees(project, number, change).await?;
            }
            if let Some(labels) = &edit.labels {
                self.set_labels(project, number, labels, Method::Put).await?;
            }
            let mut body = serde_json::Map::new();
            if let Some(t) = &edit.title {
                body.insert("title".into(), t.trim().into());
            }
            if let Some(d) = &edit.description {
                body.insert("body".into(), d.clone().into());
            }
            if !body.is_empty() {
                self.http.send_json(Method::Patch, &format!("{}/pulls/{number}", GitHubProvider::repo_url(&project.path)?), &Value::Object(body)).await?;
            }
            self.pull(project, number).await
        })
    }

    /// A draft is a `WIP:` title to Forgejo: the title gets it, or loses it.
    fn set_draft<'a>(&'a self, project: &'a ForgeProject, number: u64, draft: bool) -> ForgeFuture<'a, ForgeMr> {
        Box::pin(async move {
            self.http.expire_fresh();
            let v = self.pull_json(project, number).await?;
            let title = v["title"].as_str().ok_or_else(|| unreadable(&self.host, "pull request"))?;
            let next = json::drafted(title, draft);
            if next != title {
                self.http.send_json(Method::Patch, &format!("{}/pulls/{number}", GitHubProvider::repo_url(&project.path)?), &json!({ "title": next })).await?;
            }
            self.pull(project, number).await
        })
    }

    fn create_mr<'a>(&'a self, project: &'a ForgeProject, req: &'a CreateMr) -> ForgeFuture<'a, CreateOutcome> {
        Box::pin(async move {
            let repo = GitHubProvider::repo_url(&project.path)?;
            let r = self.http.send_json(Method::Post, &format!("{repo}/pulls"), &json::create_body(&project.path, req)).await?;
            let mut mr = json::pr(&r.json(&self.host)?).ok_or_else(|| unreadable(&self.host, "pull request"))?;
            let failed = self.follow_up(project, mr.number, req, &[CreatePart::Reviewers, CreatePart::Assignees, CreatePart::Labels]).await;
            if !failed.iter().any(|f| f.part == CreatePart::Labels) {
                mr.labels = req.labels.clone();
            }
            Ok(CreateOutcome { mr, failed })
        })
    }

    fn complete_create<'a>(&'a self, project: &'a ForgeProject, number: u64, req: &'a CreateMr, parts: &'a [CreatePart]) -> ForgeFuture<'a, Vec<PartFailure>> {
        Box::pin(async move { Ok(self.follow_up(project, number, req, parts).await) })
    }

    fn search_users<'a>(&'a self, _project: &'a ForgeProject, query: &'a str) -> ForgeFuture<'a, Vec<ForgeUser>> {
        Box::pin(async move {
            let v: Value = self.http.get(&format!("/users/search?q={}&limit={PEOPLE_SHOWN}", encode_component(query.trim()))).await?.json(&self.host)?;
            let users: Vec<ForgeUser> = json::users(&v["data"]).into_iter().take(PEOPLE_SHOWN).collect();
            self.names.learn_all(&users);
            Ok(users)
        })
    }

    fn labels<'a>(&'a self, project: &'a ForgeProject, query: &'a str) -> ForgeFuture<'a, Vec<ForgeLabel>> {
        Box::pin(async move {
            let repo = GitHubProvider::repo_url(&project.path)?;
            let q = query.trim().to_lowercase();
            let all = self.http.get_pages(&format!("{repo}/labels?limit={PER_PAGE}"), LABEL_PAGES).await?;
            Ok(all.iter().filter_map(json::label).filter(|l| l.name.to_lowercase().contains(&q)).collect())
        })
    }

    /// The first of Forgejo's template candidates the branch has (`FORGEJO_TEMPLATES`): the
    /// root's listing, then those of the candidate directories it has.
    fn mr_templates<'a>(&'a self, project: &'a ForgeProject, branch: &'a str) -> ForgeFuture<'a, Vec<MrTemplate>> {
        Box::pin(async move {
            use gitvolt_core::forge::create::{clip_template, template_name, FORGEJO_TEMPLATES};
            let repo = &GitHubProvider::repo_url(&project.path)?;
            let at = &encode_component(branch);
            let list = |dir: String| async move {
                let path = if dir.is_empty() { format!("{repo}/contents?ref={at}") } else { format!("{repo}/contents/{}?ref={at}", encode_component(&dir)) };
                match self.http.get(&path).await {
                    Ok(r) => r.json::<Vec<Value>>(&self.host),
                    Err(e) if e.kind == GbErrorKind::NotFound => Ok(Vec::new()),
                    Err(e) => Err(e),
                }
            };
            let root = list(String::new()).await?;
            let mut found: Vec<String> = root.iter().filter(|e| e["type"] == "file").filter_map(|e| e["path"].as_str().map(str::to_string)).collect();
            for dir in [".forgejo", ".gitea", ".github"] {
                if root.iter().any(|e| e["type"] == "dir" && e["name"] == dir) {
                    found.extend(list(dir.to_string()).await?.iter().filter(|e| e["type"] == "file").filter_map(|e| e["path"].as_str().map(str::to_string)));
                }
            }
            let Some(path) = FORGEJO_TEMPLATES.iter().find(|c| found.iter().any(|f| f == *c)) else { return Ok(Vec::new()) };
            let v: Value = self.http.get(&format!("{repo}/contents/{}?ref={at}", path.split('/').map(encode_component).collect::<Vec<_>>().join("/"))).await?.json(&self.host)?;
            let Some(bytes) = crate::github::json::try_decode_content(&v) else { return Ok(Vec::new()) };
            Ok(vec![MrTemplate { name: template_name(path), body: clip_template(&bytes), path: path.to_string() }])
        })
    }

    /// Points the PR at `target_branch`: `PATCH …/pulls/{n}` with `base`.
    fn retarget<'a>(&'a self, project: &'a ForgeProject, number: u64, target_branch: &'a str) -> ForgeFuture<'a, ForgeMr> {
        Box::pin(async move {
            let path = format!("{}/pulls/{number}", GitHubProvider::repo_url(&project.path)?);
            let r = self.http.send_json(Method::Patch, &path, &json!({ "base": target_branch })).await?;
            json::pr(&r.json(&self.host)?).ok_or_else(|| unreadable(&self.host, "pull request"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{json, ForgejoProvider, LocalDraft, ThreadAt};
    use crate::test_server::{Canned, TestServer};
    use gitvolt_core::forge::*;
    use serde_json::Value;
    use std::sync::{Arc, Mutex};

    const T0: &str = "2026-10-01T08:00:00Z";
    const T1: &str = "2026-10-02T08:00:00Z";
    const T2: &str = "2026-10-03T08:00:00Z";
    const WEB: &str = "https://forgejo.example.org";

    fn user(id: u64, login: &str) -> Value {
        json!({"id": id, "login": login, "full_name": "", "avatar_url": format!("{WEB}/avatars/{login}"), "html_url": format!("{WEB}/{login}")})
    }

    fn repo_json() -> Value {
        json!({"id": 9, "name": "widget", "full_name": "ada/widget", "owner": user(1, "ada"), "html_url": format!("{WEB}/ada/widget"), "default_branch": "main",
            "clone_url": format!("{WEB}/ada/widget.git"), "ssh_url": "git@forgejo.example.org:ada/widget.git", "fork": false, "parent": null, "archived": false, "updated_at": T1,
            "allow_merge_commits": true, "allow_rebase": true, "allow_rebase_explicit": false, "allow_squash_merge": true, "allow_fast_forward_only_merge": true,
            "default_merge_style": "squash", "default_delete_branch_after_merge": true})
    }

    fn pr_json(number: u64) -> Value {
        let (title, head, sha, author) = match number {
            4 => ("WIP: Spin faster", "spin", "def456", user(2, "grace")),
            5 => ("WIP: New thing", "feature", "abc999", user(1, "ada")),
            2 => ("Old work", "old", "aaa111", user(1, "ada")),
            _ => ("Add the widget", "feature", "abc123", user(1, "ada")),
        };
        json!({"id": 100 + number, "number": number, "title": title, "body": "It spins.", "state": if number == 2 { "closed" } else { "open" }, "merged": false, "merged_at": null,
            "user": author, "labels": [{"id": 7, "name": "bug", "color": "ee0701"}], "assignees": [user(1, "ada")], "requested_reviewers": [user(3, "joan")],
            "head": {"ref": head, "sha": sha, "repo": {"full_name": "ada/widget"}}, "base": {"ref": "main", "sha": "base111", "repo": {"full_name": "ada/widget"}},
            "merge_base": "base000", "mergeable": true, "html_url": format!("{WEB}/ada/widget/pulls/{number}"), "updated_at": T1})
    }

    fn code(id: u64, login_id: u64, path: &str, new_line: u64, old_line: u64, at: &str, resolver: Value) -> Value {
        let login = if login_id == 1 { "ada" } else { "grace" };
        json!({"id": id, "body": format!("c{id}"), "user": user(login_id, login), "resolver": resolver, "path": path, "position": new_line, "original_position": old_line,
            "commit_id": "abc123", "diff_hunk": "@@ -1 +1,2 @@\n a\n+b", "created_at": at, "html_url": format!("{WEB}/ada/widget/pulls/3#issuecomment-{id}")})
    }

    /// A PR diff with a change, a rename, a new file and a binary one.
    const DIFF: &str = "diff --git a/src/lib.rs b/src/lib.rs\nindex 1..2 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@\n keep\n-old\n+new\ndiff --git a/docs/a b.md b/docs/c d.md\nsimilarity index 90%\nrename from docs/a b.md\nrename to docs/c d.md\n--- a/docs/a b.md\n+++ b/docs/c d.md\n@@ -3 +3 @@\n-x\n+y\ndiff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+hello\ndiff --git a/logo.png b/logo.png\nBinary files a/logo.png and b/logo.png differ\n";

    /// Forgejo as far as these tests ask, for the repository `ada/widget`; `pending`: the user
    /// (ada) has a pending review (13) on #3.
    fn answer(line: &str, base: &str, pending: bool) -> Canned {
        let (method, path) = line.split_once(' ').unwrap_or_default();
        let path = path.trim_end().trim_end_matches(" http/1.1");
        let path = path.strip_prefix("/api/v1").unwrap_or(path);
        let r = "/repos/ada/widget";
        let j = |v: Value| Canned::json(200, &v.to_string());
        let pull = |p: &str| p.strip_prefix(&format!("{r}/pulls/")).and_then(|n| n.parse::<u64>().ok());
        match (method, path) {
            ("get", "/user") => j(user(1, "ada")),
            ("get", "/version") => j(json!({"version": "11.0.1+gitea-1.22.0"})),
            ("get", p) if p == r => j(repo_json()),
            ("get", p) if p == format!("{r}/pulls?state=open&sort=recentupdate&limit=50") => j(json!([pr_json(3)])).header("Link", &format!("<{base}{r}/pulls?state=open&sort=recentupdate&limit=50&page=2>; rel=\"next\"")),
            ("get", p) if p == format!("{r}/pulls?state=open&sort=recentupdate&limit=50&page=2") => j(json!([pr_json(4)])),
            ("get", p) if p == format!("{r}/pulls?state=closed&sort=recentupdate&limit=50") => j(json!([pr_json(2)])),
            ("get", p) if p == format!("{r}/commits/abc123/status") => j(json!({"state": "pending", "statuses": [{"status": "pending", "context": "ci"}]})),
            ("get", p) if p.starts_with(&format!("{r}/commits/")) => j(json!({"state": "", "statuses": []})),
            ("post", p) if p == format!("{r}/pulls/comments/21/resolve") => j(json!({"id": 21, "resolver": user(1, "ada")})),
            ("post", p) if p == format!("{r}/pulls/comments/21/unresolve") => j(json!({"id": 21, "resolver": null})),
            ("post", p) if p == format!("{r}/pulls/comments/99/resolve") => Canned::json(404, r#"{"message": "Not Found"}"#),
            ("get", p) if p == format!("{r}/pulls/3.diff") => Canned { status: 200, headers: vec![("Content-Type".into(), "text/plain".into())], body: DIFF.as_bytes().to_vec() },
            ("get", p) if pull(p).is_some() => j(pr_json(pull(p).unwrap())),
            ("patch", p) if pull(p).is_some() => j(pr_json(pull(p).unwrap())),
            ("get", p) if p == format!("{r}/pulls/3/reviews?limit=50") => {
                let mut list = vec![
                    json!({"id": 11, "user": user(2, "grace"), "state": "REQUEST_CHANGES", "body": "Needs tests", "comments_count": 2, "submitted_at": T1, "html_url": format!("{WEB}/ada/widget/pulls/3#r11")}),
                    json!({"id": 12, "user": user(1, "ada"), "state": "COMMENT", "body": "", "comments_count": 1, "submitted_at": T2}),
                ];
                list.extend(pending.then(|| json!({"id": 13, "user": user(1, "ada"), "state": "PENDING", "body": "", "comments_count": 2, "commit_id": "abc100"})));
                j(Value::Array(list))
            }
            ("get", p) if p == format!("{r}/pulls/3/reviews/13/comments") => j(json!([code(41, 1, "src/lib.rs", 2, 0, T2, Value::Null), code(42, 1, "README.md", 0, 3, T2, Value::Null)])),
            ("get", p) if p == format!("{r}/pulls/3/reviews/11/comments") => j(json!([code(21, 2, "src/lib.rs", 12, 0, T1, Value::Null), code(22, 2, "README.md", 0, 3, T1, user(1, "ada"))])),
            ("get", p) if p == format!("{r}/pulls/3/reviews/12/comments") => j(json!([code(23, 1, "src/lib.rs", 12, 0, T2, Value::Null)])),
            ("get", p) if p == format!("{r}/pulls/3/reviews/14/comments") => j(json!([code(24, 1, "src/lib.rs", 12, 0, T2, Value::Null)])),
            ("post", p) if p == format!("{r}/pulls/3/reviews") => j(json!({"id": 14})),
            ("get", p) if p == format!("{r}/issues/3/comments?limit=50") => j(json!([{"id": 31, "user": user(1, "ada"), "body": "Looks fine", "created_at": T0, "html_url": format!("{WEB}/ada/widget/pulls/3#issuecomment-31")}])),
            ("get", p) if p == format!("{r}/issues/comments/31/reactions") => j(json!([{"content": "+1", "user": user(1, "ada")}, {"content": "heart", "user": user(2, "grace")}])),
            ("get", p) if p.starts_with(&format!("{r}/issues/comments/")) && p.ends_with("/reactions") => j(json!([])),
            ("post", p) if p.ends_with("/reactions") => j(json!({"content": "rocket", "user": user(1, "ada")})),
            ("patch", p) if p == format!("{r}/issues/comments/31") => j(json!({"id": 31, "user": user(1, "ada"), "body": "Edited", "created_at": T0})),
            ("post", p) if p == format!("{r}/issues/3/comments") => j(json!({"id": 32, "user": user(1, "ada"), "body": "Hi", "created_at": T2})),
            // #3's timeline has three pages: an old schedule and its cancel first, the newest
            // schedule on the last.
            ("get", p) if p == format!("{r}/issues/3/timeline?limit=50&page=1") => j(json!([{"type": "pull_scheduled_merge", "user": user(1, "ada")}, {"type": "pull_cancel_scheduled_merge", "user": user(1, "ada")}]))
                .header("Link", &format!("<{base}{r}/issues/3/timeline?limit=50&page=2>; rel=\"next\", <{base}{r}/issues/3/timeline?limit=50&page=3>; rel=\"last\"")),
            ("get", p) if p == format!("{r}/issues/3/timeline?limit=50&page=2") => j(json!([{"type": "comment", "user": user(1, "ada")}])),
            ("get", p) if p == format!("{r}/issues/3/timeline?limit=50&page=3") => j(json!([{"type": "comment", "user": user(1, "ada")}, {"type": "pull_scheduled_merge", "user": user(2, "grace")}])),
            ("get", p) if p.ends_with("/timeline?limit=50&page=1") => j(json!([])),
            ("delete", p) if p == format!("{r}/pulls/3/reviews/13/comments/99") => Canned::json(405, r#"{"message": "Method Not Allowed"}"#),
            ("get", p) if p == format!("{r}/issues/3/subscriptions/check") => j(json!({"subscribed": true})),
            ("get", p) if p == format!("{r}/labels?limit=50") => j(json!([{"id": 7, "name": "bug", "color": "ee0701"}, {"id": 8, "name": "docs", "color": "#0075CA", "description": "Words"}])),
            ("get", "/users/search?uid=2") => j(json!({"ok": true, "data": [user(2, "grace")]})),
            ("get", "/users/search?uid=3") => j(json!({"ok": true, "data": [user(3, "joan")]})),
            ("get", "/users/search?q=gr&limit=20") => j(json!({"ok": true, "data": [user(2, "grace")]})),
            ("post", p) if p == format!("{r}/pulls") => Canned::json(201, &pr_json(5).to_string()),
            ("get", p) if p == format!("{r}/contents?ref=main") => j(json!([{"name": ".forgejo", "path": ".forgejo", "type": "dir"}, {"name": "README.md", "path": "README.md", "type": "file"}])),
            ("get", p) if p == format!("{r}/contents/.forgejo?ref=main") => j(json!([{"name": "pull_request_template.md", "path": ".forgejo/pull_request_template.md", "type": "file"}])),
            ("get", p) if p == format!("{r}/contents/.forgejo/pull_request_template.md?ref=main") => j(json!({"content": "IyMgV2h5DQo=", "encoding": "base64"})),
            ("get", p) if p == format!("{r}/forks?page=1&limit=2") => j(json!([repo_json()])).header("Link", &format!("<{base}{r}/forks?page=2&limit=2>; rel=\"next\"")),
            ("get", p) if p.starts_with("/avatars/") => Canned::json(404, "{}"),
            ("post" | "put" | "delete", _) => Canned::json(200, ""),
            _ => Canned::json(404, r#"{"message": "Not Found"}"#),
        }
    }

    fn served() -> (TestServer, ForgejoProvider, ForgeProject) {
        served_with(true)
    }

    fn served_with(pending: bool) -> (TestServer, ForgejoProvider, ForgeProject) {
        let base = Arc::new(Mutex::new(String::new()));
        let b = base.clone();
        let s = TestServer::start(move |_, head| answer(head.lines().next().unwrap_or_default(), &b.lock().unwrap(), pending));
        *base.lock().unwrap() = format!("{}/api/v1", s.base);
        let ep = crate::endpoints::HostEndpoints { api: format!("{}/api/v1", s.base), web: s.base.clone(), avatars: None };
        let p = ForgejoProvider::new("forgejo.example.org", &ep, gitvolt_core::redact::Secret::new("FAKE-forgejo-test-token"), None);
        let project = json::project("forgejo.example.org", &repo_json()).unwrap();
        (s, p, project)
    }

    /// Each request's method and path below the API (`/api/v1`).
    fn lines(s: &TestServer) -> Vec<String> {
        s.heads.lock().unwrap().iter().map(|h| h.lines().next().unwrap_or_default().trim_end().trim_end_matches(" http/1.1").replacen(" /api/v1/", " /", 1)).collect()
    }

    /// The writes sent (not GETs), each with its body.
    fn writes(s: &TestServer) -> Vec<(String, String)> {
        let bodies = s.bodies.lock().unwrap().clone();
        lines(s).into_iter().zip(bodies).filter(|(l, _)| !l.starts_with("get ")).collect()
    }

    fn body(b: &str) -> Value {
        serde_json::from_str(b).unwrap_or(Value::Null)
    }

    #[test]
    fn normalizes_a_user_and_a_repository_with_its_parent() {
        let u = json::user(&json!({"id": 2, "login": "grace", "full_name": "Grace", "email": "grace@example.com", "avatar_url": format!("{WEB}/avatars/g")})).unwrap();
        assert_eq!((u.name.as_str(), u.emails.clone(), u.avatar_url.as_deref()), ("Grace", vec!["grace@example.com".to_string()], Some("https://forgejo.example.org/avatars/g")));
        assert_eq!(json::user(&json!({"id": -1, "login": "Ghost"})).unwrap().id, 0, "a deleted user");
        let mut fork = repo_json();
        fork["fork"] = true.into();
        fork["parent"] = json!({"full_name": "upstream/widget"});
        let p = json::project("forgejo.example.org", &fork).unwrap();
        assert_eq!((p.kind, p.path.as_str(), p.owner.as_str(), p.fork_of.as_deref(), p.default_branch.as_deref()), (ForgeKind::Forgejo, "ada/widget", "ada", Some("upstream/widget"), Some("main")));
        assert_eq!((p.clone_ssh.as_str(), p.owner_avatar_url.as_deref(), p.updated_at), ("git@forgejo.example.org:ada/widget.git", Some("https://forgejo.example.org/avatars/ada"), Some(1_790_928_000)));
        let mut empty = repo_json();
        empty["empty"] = true.into();
        assert_eq!(json::project("forgejo.example.org", &empty).unwrap().default_branch, None);
    }

    #[test]
    fn settings_list_the_allowed_styles_with_the_default_first() {
        let s = json::settings(&repo_json());
        assert_eq!(s, ForgeProjectSettings { merge_methods: vec![MergeMethod::Squash, MergeMethod::Merge, MergeMethod::Rebase, MergeMethod::FastForward], squash: SquashOption::DefaultOff, delete_source_branch: true });
        let only = json::settings(&json!({"allow_merge_commits": false, "allow_rebase": false, "allow_rebase_explicit": false, "allow_squash_merge": true}));
        assert_eq!((only.merge_methods, only.squash), (vec![MergeMethod::Squash], SquashOption::Always));
        assert_eq!(json::settings(&json!({})).merge_methods, [MergeMethod::Merge, MergeMethod::Rebase, MergeMethod::SemiLinear, MergeMethod::Squash], "Forgejo's defaults");
    }

    #[test]
    fn a_wip_title_is_a_draft_and_drafting_adds_or_takes_the_prefix() {
        let m = json::pr(&pr_json(3)).unwrap();
        assert_eq!((m.number, m.state, m.source_branch.as_str(), m.target_project.as_str(), m.conflicts), (3, MrState::Open, "feature", "ada/widget", Some(false)));
        assert_eq!((m.labels.clone(), m.label_colors.get("bug").map(String::as_str)), (vec!["bug".to_string()], Some("#ee0701")));
        assert_eq!((m.review.decision, m.review.reviews[0].user.username.as_str()), (ReviewDecision::ReviewRequired, "joan"));
        let d = json::pr(&pr_json(4)).unwrap();
        assert_eq!((d.state, d.conflicts), (MrState::Draft, None));
        let mut flagged = pr_json(3);
        flagged["draft"] = true.into();
        assert_eq!(json::pr(&flagged).unwrap().state, MrState::Draft, "a newer Forgejo's `draft`");
        assert_eq!(json::pr(&pr_json(2)).unwrap().state, MrState::Closed);
        assert!(json::wip_title("[wip] x") && json::wip_title("  WIP: x") && !json::wip_title("Wipe it"));
        assert_eq!(json::drafted("Add it", true), "WIP: Add it");
        assert_eq!(json::drafted("[WIP] wip: Add it", false), "Add it");
        assert_eq!(json::drafted("WIP: Add it", true), "WIP: Add it");
    }

    #[test]
    fn a_commits_combined_status_is_its_pipeline() {
        let p = |state: &str| json::status(&json!({"state": state, "statuses": [{"status": state}]}), "u").map(|p| p.status);
        assert_eq!(p("success"), Some(PipelineStatus::Success));
        assert_eq!(p("pending"), Some(PipelineStatus::Pending));
        assert_eq!(p("failure"), Some(PipelineStatus::Failed));
        assert_eq!(p("error"), Some(PipelineStatus::Failed));
        assert_eq!(p("warning"), Some(PipelineStatus::Success));
        assert_eq!(json::status(&json!({"state": "", "statuses": []}), "u"), None);
        assert_eq!(json::status(&json!({"state": "success", "statuses": null}), "u"), None);
    }

    #[test]
    fn the_latest_decisive_review_per_user_decides() {
        let r = |id: u64, login: &str, state: &str| json!({"user": user(id, login), "state": state, "submitted_at": T1});
        let s = json::review(&[r(2, "grace", "REQUEST_CHANGES"), r(2, "grace", "COMMENT"), r(2, "grace", "APPROVED"), r(1, "ada", "COMMENT")], &json!([user(3, "joan")]));
        assert_eq!((s.decision, s.approvals), (ReviewDecision::Approved, 1));
        assert_eq!(s.reviews.iter().map(|x| (x.user.username.as_str(), x.state)).collect::<Vec<_>>(), [("grace", ReviewState::Approved), ("ada", ReviewState::Commented), ("joan", ReviewState::Pending)]);
        let mut dismissed = r(2, "grace", "APPROVED");
        dismissed["dismissed"] = true.into();
        assert_eq!(json::review(&[r(2, "grace", "APPROVED"), dismissed], &Value::Null).decision, ReviewDecision::None);
        assert_eq!(json::review(&[r(2, "grace", "APPROVED"), r(2, "grace", "REQUEST_REVIEW")], &Value::Null).decision, ReviewDecision::ReviewRequired, "asked again");
        assert_eq!(json::review(&[r(2, "grace", "REQUEST_CHANGES"), r(1, "ada", "PENDING")], &json!([])).decision, ReviewDecision::ChangesRequested);
        // Still in `requested_reviewers` after only commenting: reviewed, until asked again.
        let states = |s: ReviewSummary| (s.decision, s.reviews.iter().map(|x| x.state).collect::<Vec<_>>());
        assert_eq!(states(json::review(&[r(3, "joan", "REQUEST_REVIEW"), r(3, "joan", "COMMENT")], &json!([user(3, "joan")]))), (ReviewDecision::None, vec![ReviewState::Commented]));
        assert_eq!(states(json::review(&[r(3, "joan", "COMMENT")], &json!([user(3, "joan")]))), (ReviewDecision::None, vec![ReviewState::Commented]));
        assert_eq!(states(json::review(&[r(3, "joan", "COMMENT"), r(3, "joan", "REQUEST_REVIEW")], &json!([user(3, "joan")]))), (ReviewDecision::ReviewRequired, vec![ReviewState::Pending]));
        assert_eq!(states(json::review(&[r(3, "joan", "APPROVED"), r(3, "joan", "REQUEST_REVIEW"), r(3, "joan", "COMMENT")], &json!([user(3, "joan")]))), (ReviewDecision::None, vec![ReviewState::Commented]));
        // A commenter still listed can have the request withdrawn; an approver can't.
        let s = json::review(&[r(3, "joan", "COMMENT"), r(2, "grace", "APPROVED")], &json!([user(3, "joan"), user(2, "grace")]));
        assert_eq!(s.reviews.iter().map(|x| (x.user.username.as_str(), x.requested)).collect::<Vec<_>>(), [("joan", true), ("grace", false)]);
        // A dismissed approval falls back to the comment before it, not to nothing.
        let mut dropped = r(3, "joan", "APPROVED");
        dropped["dismissed"] = true.into();
        let s = json::review(&[r(3, "joan", "COMMENT"), r(3, "joan", "APPROVED"), dropped.clone()], &Value::Null);
        assert_eq!((s.decision, s.reviews[0].state, s.reviews[0].submitted_at.is_some()), (ReviewDecision::None, ReviewState::Commented, true));
        assert_eq!(states(json::review(&[r(3, "joan", "APPROVED"), dropped], &json!([user(3, "joan")]))), (ReviewDecision::ReviewRequired, vec![ReviewState::Pending]));
        // Approved, earlier ones dismissed, and still listed as requested: approved.
        let mut gone = r(3, "joan", "REQUEST_CHANGES");
        gone["dismissed"] = true.into();
        assert_eq!(states(json::review(&[gone, r(9, "bot", "PENDING"), r(3, "joan", "APPROVED")], &json!([user(3, "joan")]))), (ReviewDecision::Approved, vec![ReviewState::Approved]));
    }

    #[test]
    fn mergeability_and_the_update_offer_say_why_not() {
        let with = |extra: Value| {
            let mut v = pr_json(3);
            v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            v
        };
        assert_eq!(json::merge_status(&pr_json(3)), MergeStatus::Mergeable);
        assert_eq!(json::merge_status(&pr_json(4)), MergeStatus::Blocked { reason: "Mark it ready first: it's a draft".into() });
        assert!(matches!(json::merge_status(&with(json!({"mergeable": false}))), MergeStatus::Blocked { reason } if reason.starts_with("It has conflicts")));
        assert_eq!(json::merge_status(&with(json!({"state": "closed", "merged": true}))), MergeStatus::Blocked { reason: "It's merged already".into() });
        assert_eq!(json::update_offer(&pr_json(3), None).unwrap().kinds, [BranchUpdate::Merge, BranchUpdate::Rebase]);
        let fork = with(json!({"head": {"ref": "feature", "sha": "abc123", "repo": {"full_name": "grace/widget"}}}));
        assert!(json::update_offer(&fork, Some(2)).is_none(), "someone else's fork");
        assert!(json::update_offer(&fork, Some(1)).is_some(), "the author's own");
        let mut open = fork.clone();
        open["allow_maintainer_edit"] = true.into();
        assert!(json::update_offer(&open, Some(2)).is_some());
        assert!(json::update_offer(&pr_json(2), None).is_none(), "closed");
    }

    #[test]
    fn code_comments_thread_by_file_and_line_with_the_conversation_and_review_summaries() {
        let comments = [json!({"id": 31, "user": user(1, "ada"), "body": "Looks fine", "created_at": T0})];
        let reviews = [json!({"id": 11, "user": user(2, "grace"), "state": "REQUEST_CHANGES", "body": "Needs tests", "submitted_at": T1}), json!({"id": 13, "user": user(1, "ada"), "state": "PENDING", "body": "Draft"})];
        let code = [code(23, 1, "src/lib.rs", 12, 0, T2, Value::Null), code(21, 2, "src/lib.rs", 12, 0, T1, Value::Null), code(22, 2, "README.md", 0, 3, T1, user(1, "ada"))];
        let (ds, at) = json::discussions(&comments, &reviews, &code);
        assert_eq!(ds.iter().map(|d| (d.id.as_str(), d.notes.len())).collect::<Vec<_>>(), [("issue-31", 1), ("review-11", 1), ("thread-21", 2), ("thread-22", 1)]);
        let p = ds[2].notes[0].position.clone().unwrap();
        assert_eq!((p.path.as_str(), p.line, p.old_line, p.snippet.as_deref()), ("src/lib.rs", Some(12), None, Some(" a\n+b")));
        assert_eq!(ds[3].notes[0].position.as_ref().map(|p| (p.line, p.old_line)), Some((None, Some(3))));
        assert_eq!((ds[2].resolved, ds[3].resolved, ds[3].resolved_by.as_deref(), ds[3].resolvable), (false, true, Some("ada"), true));
        assert_eq!(at[&21], ThreadAt { path: "src/lib.rs".into(), new_line: 12, old_line: 0, extra: 0, commit: Some("abc123".into()) });
        assert_eq!(ds[0].notes[0].web_url, None);
    }

    #[test]
    fn bodies_carry_forgejos_words() {
        let opts = MergeOptions { method: Some(MergeMethod::SemiLinear), expected_sha: Some("abc".into()), delete_source_branch: Some(false), squash: None };
        assert_eq!(json::merge_body(&opts, false), json!({"Do": "rebase-merge", "head_commit_id": "abc", "delete_branch_after_merge": false}));
        assert_eq!(json::merge_body(&MergeOptions { squash: Some(true), ..Default::default() }, true), json!({"Do": "squash", "merge_when_checks_succeed": true}));
        assert_eq!(json::merge_body(&MergeOptions { method: Some(MergeMethod::FastForward), ..Default::default() }, false), json!({"Do": "fast-forward-only"}));
        assert_eq!(json::review_body(&ReviewSubmit { event: ReviewEvent::Approve, body: " ".into() }), json!({"event": "APPROVED"}));
        let req = |source: &str| CreateMr {
            source: SourceRef { project: source.into(), branch: "feature".into() }, target_branch: "main".into(), title: " Add it ".into(), description: "Why.".into(),
            draft: false, reviewers: vec![], assignees: vec![], labels: vec![], squash: None, delete_source_branch: None,
        };
        assert_eq!(json::create_body("ada/widget", &req("ada/widget")), json!({"title": "Add it", "head": "feature", "base": "main", "body": "Why."}));
        assert_eq!(json::create_body("ada/widget", &CreateMr { draft: true, ..req("grace/widget") })["head"], "grace:feature");
        assert_eq!(json::create_body("ada/widget", &CreateMr { draft: true, ..req("grace/widget") })["title"], "WIP: Add it");
    }

    #[test]
    fn the_timeline_says_whether_an_auto_merge_is_scheduled() {
        let e = |t: &str| json!({"type": t, "user": user(2, "grace")});
        assert_eq!(json::auto_merge(&[e("pull_scheduled_merge"), e("comment")]).and_then(|a| a.enabled_by).map(|u| u.username), Some("grace".into()));
        assert_eq!(json::auto_merge(&[e("pull_scheduled_merge"), e("pull_cancel_scheduled_merge")]), None);
        assert_eq!(json::auto_merge(&[e("comment")]), None);
    }

    #[tokio::test]
    async fn identity_version_and_the_repository() {
        let (s, p, project) = served();
        let check = p.check_token().await.unwrap();
        assert_eq!((check.user.username.as_str(), check.write), ("ada", WriteAccess::Unknown));
        assert_eq!(p.version().await.unwrap().as_deref(), Some("11.0.1+gitea-1.22.0"));
        assert_eq!(p.project("ada/widget").await.unwrap().value, project);
        assert_eq!(p.project_settings(&project).await.unwrap().merge_methods[0], MergeMethod::Squash);
        assert!(p.project("ada").await.is_err(), "not an owner/repository path");
        assert_eq!(p.rate_limit(), RateLimitState::default(), "no limit said");
        assert!(s.heads.lock().unwrap()[0].contains("authorization: bearer "), "the token as a bearer");
        assert_eq!(lines(&s), ["get /user", "get /version", "get /repos/ada/widget", "get /repos/ada/widget"]);
    }

    #[tokio::test]
    async fn the_open_list_follows_its_pages_and_reads_statuses() {
        let (s, p, project) = served();
        let all = p.open_mrs_light(&project, MrFilter::All).await.unwrap().value;
        assert_eq!(all.iter().map(|m| (m.number, m.state)).collect::<Vec<_>>(), [(3, MrState::Open), (4, MrState::Draft)]);
        let listed = p.open_mrs(&project, MrFilter::All).await.unwrap().value;
        assert_eq!(listed.iter().map(|m| (m.number, m.pipeline.clone())).collect::<Vec<_>>(), [(3, Some(ForgePipeline { status: PipelineStatus::Pending, web_url: Some(format!("{WEB}/ada/widget/commit/abc123")) }))]);
        assert_eq!(p.open_mrs_light(&project, MrFilter::Mine).await.unwrap().value.iter().map(|m| m.number).collect::<Vec<_>>(), [3]);
        assert!(p.open_mrs_light(&project, MrFilter::ReviewRequested).await.unwrap().value.is_empty(), "joan is asked, not ada");
        assert!(lines(&s).contains(&"get /repos/ada/widget/pulls?state=open&sort=recentupdate&limit=50&page=2".to_string()));
        assert!(lines(&s).contains(&"get /repos/ada/widget/commits/abc123/status".to_string()));
    }

    #[tokio::test]
    async fn a_branchs_pr_is_its_open_one_else_its_latest_closed_one() {
        let (_s, p, project) = served();
        let at = |b: &str| SourceRef { project: "ada/widget".into(), branch: b.into() };
        assert_eq!(p.mr_for_branch(&project, &at("feature")).await.unwrap().value.map(|m| m.number), Some(3));
        assert_eq!(p.mr_for_branch(&project, &at("old")).await.unwrap().value.map(|m| (m.number, m.state)), Some((2, MrState::Closed)));
        assert_eq!(p.mr_for_branch(&project, &at("nothing")).await.unwrap().value, None);
    }

    #[tokio::test]
    async fn the_detail_has_its_reviews_checks_auto_merge_and_subscription() {
        let (_s, p, project) = served();
        let d = p.mr_detail(&project, 3).await.unwrap().value;
        assert_eq!(d.mr.review.decision, ReviewDecision::ChangesRequested);
        assert_eq!(d.reviewers.iter().map(|u| u.username.as_str()).collect::<Vec<_>>(), ["grace", "ada", "joan"]);
        assert_eq!(d.mr.pipeline.map(|p| p.status), Some(PipelineStatus::Pending));
        assert_eq!(d.mr.auto_merge.and_then(|a| a.enabled_by).map(|u| u.username), Some("grace".into()));
        assert_eq!((d.subscribed, d.base_sha.as_deref(), d.merge_status, d.description.as_str()), (Some(true), Some("base000"), MergeStatus::Mergeable, "It spins."));
        assert_eq!((d.assignees.len(), d.update.map(|u| u.kinds.len()), d.merged_sha), (1, Some(2), None));
    }

    #[tokio::test]
    async fn discussions_and_replies_in_the_conversation_and_in_a_thread() {
        let (s, p, project) = served_with(false);
        let ds = p.discussions(&project, 3).await.unwrap().value;
        assert_eq!(ds.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["issue-31", "review-11", "thread-21", "thread-22"]);
        assert_eq!(ds[0].notes[0].reactions.iter().map(|r| (r.name.as_str(), r.mine)).collect::<Vec<_>>(), [("+1", true), ("heart", false)]);
        let n = p.reply(&project, 3, &NewNote { discussion: Some("thread-21".into()), body: "Done".into() }).await.unwrap();
        assert_eq!(n.id, "24");
        let n = p.reply(&project, 3, &NewNote { discussion: None, body: "Hi".into() }).await.unwrap();
        assert_eq!(n.id, "32");
        assert_eq!(writes(&s).into_iter().map(|(l, b)| (l, body(&b))).collect::<Vec<_>>(), [
            ("post /repos/ada/widget/pulls/3/reviews".to_string(), json!({"event": "COMMENT", "body": "", "commit_id": "abc123", "comments": [{"path": "src/lib.rs", "body": "Done", "new_position": 12, "old_position": 0}]})),
            ("post /repos/ada/widget/issues/3/comments".to_string(), json!({"body": "Hi"})),
        ]);
    }

    #[tokio::test]
    async fn reviews_merges_and_auto_merge_send_forgejos_words() {
        let (s, p, project) = served();
        p.approve(&project, 3).await.unwrap();
        p.request_changes(&project, 3, "Fix it").await.unwrap();
        p.review(&project, 3, &ReviewSubmit { event: ReviewEvent::Comment, body: "Note".into() }).await.unwrap();
        p.merge(&project, 3, &MergeOptions { method: Some(MergeMethod::Squash), expected_sha: Some("abc123".into()), delete_source_branch: Some(true), squash: None }).await.unwrap();
        let auto = p.set_auto_merge(&project, 3, &MergeOptions::default()).await.unwrap();
        assert!(auto.auto_merge.is_some(), "the timeline says so");
        p.cancel_auto_merge(&project, 3).await.unwrap();
        let r = "/repos/ada/widget/pulls/3";
        assert_eq!(writes(&s).into_iter().map(|(l, b)| (l, body(&b))).collect::<Vec<_>>(), [
            (format!("post {r}/reviews"), json!({"event": "APPROVED"})),
            (format!("post {r}/reviews"), json!({"event": "REQUEST_CHANGES", "body": "Fix it"})),
            (format!("post {r}/reviews"), json!({"event": "COMMENT", "body": "Note"})),
            (format!("post {r}/merge"), json!({"Do": "squash", "head_commit_id": "abc123", "delete_branch_after_merge": true})),
            (format!("post {r}/merge"), json!({"Do": "merge", "delete_branch_after_merge": true, "merge_when_checks_succeed": true})),
            (format!("delete {r}/merge"), Value::Null),
        ]);
    }

    #[tokio::test]
    async fn an_edit_sets_labels_by_id_people_by_login_and_a_draft_by_its_title() {
        let (s, p, project) = served();
        let edit = MrEdit { title: Some(" New title ".into()), description: None, labels: Some(vec!["docs".into()]), reviewers: Some(PeopleEdit { add: vec![2], remove: vec![3] }), assignees: Some(PeopleEdit { add: vec![2], remove: vec![] }) };
        p.edit(&project, 3, &edit).await.unwrap();
        p.set_draft(&project, 3, true).await.unwrap();
        p.set_draft(&project, 3, false).await.unwrap();
        assert!(p.edit(&project, 3, &MrEdit { labels: Some(vec!["nope".into()]), ..Default::default() }).await.unwrap_err().message.contains("has no label nope"));
        let r = "/repos/ada/widget";
        assert_eq!(writes(&s).into_iter().map(|(l, b)| (l, body(&b))).collect::<Vec<_>>(), [
            (format!("delete {r}/pulls/3/requested_reviewers"), json!({"reviewers": ["joan"]})),
            (format!("post {r}/pulls/3/requested_reviewers"), json!({"reviewers": ["grace"]})),
            (format!("patch {r}/pulls/3"), json!({"assignees": ["ada", "grace"]})),
            (format!("put {r}/issues/3/labels"), json!({"labels": [8]})),
            (format!("patch {r}/pulls/3"), json!({"title": "New title"})),
            (format!("patch {r}/pulls/3"), json!({"title": "WIP: Add the widget"})),
        ], "not a draft already: nothing to take off");
    }

    #[tokio::test]
    async fn a_branch_update_checks_the_head_first() {
        let (s, p, project) = served();
        p.update_branch(&project, 3, BranchUpdate::Rebase, Some("abc123")).await.unwrap();
        assert_eq!(p.update_branch(&project, 3, BranchUpdate::Merge, Some("0ld")).await.unwrap_err().message, "#3 changed since it was loaded: refresh and try again");
        assert!(p.update_branch(&project, 3, BranchUpdate::RebaseSkipCi, None).await.is_err());
        assert_eq!(writes(&s).into_iter().map(|(l, _)| l).collect::<Vec<_>>(), ["post /repos/ada/widget/pulls/3/update?style=rebase"]);
    }

    #[tokio::test]
    async fn a_create_then_adds_reviewers_assignees_and_labels() {
        let (s, p, project) = served();
        let req = CreateMr {
            source: SourceRef { project: "ada/widget".into(), branch: "feature".into() }, target_branch: "main".into(), title: "New thing".into(), description: "It spins.".into(),
            draft: true, reviewers: vec![2], assignees: vec![2], labels: vec!["bug".into()], squash: None, delete_source_branch: None,
        };
        let out = p.create_mr(&project, &req).await.unwrap();
        assert_eq!((out.mr.number, out.mr.state, out.failed), (5, MrState::Draft, vec![]));
        let r = "/repos/ada/widget";
        assert_eq!(writes(&s).into_iter().map(|(l, b)| (l, body(&b))).collect::<Vec<_>>(), [
            (format!("post {r}/pulls"), json!({"title": "WIP: New thing", "head": "feature", "base": "main", "body": "It spins."})),
            (format!("post {r}/pulls/5/requested_reviewers"), json!({"reviewers": ["grace"]})),
            (format!("patch {r}/pulls/5"), json!({"assignees": ["ada", "grace"]})),
            (format!("post {r}/issues/5/labels"), json!({"labels": [7]})),
        ]);
        let failed = p.complete_create(&project, 5, &CreateMr { labels: vec!["nope".into()], ..req }, &[CreatePart::Labels]).await.unwrap();
        assert_eq!(failed, [PartFailure { part: CreatePart::Labels, message: "ada/widget has no label nope".into() }]);
    }

    #[tokio::test]
    async fn labels_people_reactions_comments_retarget_and_subscriptions() {
        let (s, p, project) = served();
        assert_eq!(p.labels(&project, "D").await.unwrap(), [ForgeLabel { name: "docs".into(), color: Some("#0075ca".into()), description: Some("Words".into()) }]);
        assert_eq!(p.search_users(&project, " gr ").await.unwrap().iter().map(|u| u.username.as_str()).collect::<Vec<_>>(), ["grace"]);
        let note = NoteRef { discussion: "issue-31".into(), note: "31".into() };
        let after = p.react(&project, 3, &note, "rocket", true).await.unwrap();
        assert_eq!(after.iter().map(|r| (r.name.as_str(), r.mine)).collect::<Vec<_>>(), [("+1", true), ("heart", false), ("rocket", true)]);
        let after = p.react(&project, 3, &note, "+1", false).await.unwrap();
        assert_eq!(after.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["heart"]);
        p.react(&project, 3, &note, "heart", false).await.unwrap();
        assert_eq!(p.edit_note(&project, 3, &note, "Edited").await.unwrap().body, "Edited");
        p.delete_note(&project, 3, &note).await.unwrap();
        assert!(p.delete_note(&project, 3, &NoteRef { discussion: "thread-21".into(), note: "21".into() }).await.is_err());
        assert!(p.edit_note(&project, 3, &NoteRef { discussion: "review-11".into(), note: "review-11".into() }, "x").await.is_err());
        assert_eq!(p.retarget(&project, 3, "dev").await.unwrap().number, 3);
        assert!(!p.set_subscribed(&project, 3, false).await.unwrap());
        let c = "/repos/ada/widget/issues/comments/31";
        assert_eq!(writes(&s).into_iter().map(|(l, b)| (l, body(&b))).collect::<Vec<_>>(), [
            (format!("post {c}/reactions"), json!({"content": "rocket"})),
            (format!("delete {c}/reactions"), json!({"content": "+1"})),
            (format!("patch {c}"), json!({"body": "Edited"})),
            (format!("delete {c}"), Value::Null),
            ("patch /repos/ada/widget/pulls/3".to_string(), json!({"base": "dev"})),
            ("delete /repos/ada/widget/issues/3/subscriptions/ada".to_string(), Value::Null),
        ], "someone else's heart: nothing to remove");
    }

    #[tokio::test]
    async fn the_template_is_the_first_candidate_the_branch_has_and_forks_page() {
        let (_s, p, project) = served();
        let t = p.mr_templates(&project, "main").await.unwrap();
        assert_eq!(t, [MrTemplate { name: "Default".into(), path: ".forgejo/pull_request_template.md".into(), body: "## Why\n".into() }]);
        let page = p.forks_page(&project, 1, 2).await.unwrap();
        assert_eq!((page.forks.len(), page.next), (1, Some(2)));
    }

    #[tokio::test]
    async fn avatars_come_only_from_the_hosts_own_paths_without_the_token() {
        let (s, p, _project) = served();
        assert!(p.avatar_at("https://avatars.example.com/avatars/x.png").is_none());
        assert!(p.avatar_at(&format!("{}/api/v1/user", s.base)).is_none());
        assert_eq!(p.avatar_at(&format!("{}/avatars/abc", s.base)).unwrap().await.unwrap(), None);
        assert_eq!(p.avatar_for_email("nobody@example.com").await.unwrap(), None, "no lookup by email");
        let heads = s.heads.lock().unwrap().clone();
        assert_eq!(heads.len(), 1);
        assert!(heads[0].starts_with("get /avatars/abc ") && !heads[0].contains("authorization"), "{}", heads[0]);
    }

    #[test]
    fn the_pr_diff_splits_by_file_with_renames_and_its_commentable_lines() {
        let files = json::review_files(DIFF);
        let got: Vec<(&str, &str, usize)> = files.iter().map(|f| (f.path.as_str(), f.old_path.as_str(), f.lines.len())).collect();
        assert_eq!(got, [("src/lib.rs", "src/lib.rs", 3), ("docs/c d.md", "docs/a b.md", 2), ("new.txt", "new.txt", 1), ("logo.png", "logo.png", 0)]);
        assert_eq!(files[0].lines.iter().map(|l| (l.kind, l.old_line, l.new_line)).collect::<Vec<_>>(), [(LineKind::Context, 1, 1), (LineKind::Removed, 2, 2), (LineKind::Added, 3, 2)]);
        assert!(files.iter().all(|f| !f.too_large));
        assert_eq!(json::review_files(""), []);
    }

    /// Git quotes a path with a non-ASCII byte (octal escapes of its UTF-8) or a `"`, each half
    /// of a header on its own; spaces stay bare.
    #[test]
    fn the_pr_diff_reads_quoted_and_spaced_paths() {
        let paths = |diff: &str| json::review_files(diff).into_iter().map(|f| (f.path, f.old_path)).collect::<Vec<_>>();
        let one = |p: &str| vec![(p.to_string(), p.to_string())];
        assert_eq!(paths("diff --git a/src/lib.rs b/src/lib.rs\n"), one("src/lib.rs"));
        assert_eq!(paths("diff --git a/my file.txt b/my file.txt\n"), one("my file.txt"));
        assert_eq!(paths("diff --git a/a b/c.txt b/a b/c.txt\n"), one("a b/c.txt"), "a ` b/` inside the path");
        assert_eq!(paths("diff --git \"a/caf\\303\\251.txt\" \"b/caf\\303\\251.txt\"\n"), one("café.txt"));
        assert_eq!(paths("diff --git \"a/say \\\"hi\\\"\\tnow.txt\" \"b/say \\\"hi\\\"\\tnow.txt\"\n"), one("say \"hi\"\tnow.txt"));
        // Renames: the header's halves, then `rename from` / `rename to`, quoted alike.
        let rename = "diff --git a/old name.txt \"b/na\\303\\257ve.txt\"\nsimilarity index 90%\nrename from old name.txt\nrename to \"na\\303\\257ve.txt\"\n";
        assert_eq!(paths(rename), [("naïve.txt".to_string(), "old name.txt".to_string())]);
        assert_eq!(paths("diff --git \"a/na\\303\\257ve.txt\" b/plain.txt\n"), [("plain.txt".to_string(), "naïve.txt".to_string())]);
        assert_eq!(paths("diff --git a/x.txt b/y.txt\n"), [("y.txt".to_string(), "x.txt".to_string())]);
    }

    #[test]
    fn a_line_comment_names_a_removed_line_by_its_old_number_and_any_other_by_its_new_one() {
        let anchor = |kind, old_line, new_line| ReviewAnchor { path: "src/lib.rs".into(), old_path: "src/lib.rs".into(), start: None, end: ReviewLine { kind, old_line, new_line } };
        assert_eq!(json::line_comment(&anchor(LineKind::Removed, 2, 2), "x"), json!({"path": "src/lib.rs", "body": "x", "new_position": 0, "old_position": 2}));
        assert_eq!(json::line_comment(&anchor(LineKind::Added, 3, 2), "x")["new_position"], 2);
        assert_eq!(json::line_comment(&anchor(LineKind::Context, 1, 1), "x")["old_position"], 0);
    }

    fn new_comment(body: &str) -> NewReviewComment {
        NewReviewComment {
            anchor: ReviewAnchor { path: "src/lib.rs".into(), old_path: "src/lib.rs".into(), start: None, end: ReviewLine { kind: LineKind::Added, old_line: 3, new_line: 2 } },
            body: body.into(),
            refs: DiffRefs { base_sha: "base000".into(), start_sha: "base000".into(), head_sha: "abc123".into() },
        }
    }

    #[tokio::test]
    async fn the_review_diff_and_the_pending_reviews_drafts() {
        let (s, p, project) = served();
        let d = p.review_diff(&project, 3).await.unwrap();
        assert_eq!((d.refs.start_sha.as_str(), d.refs.head_sha.as_str()), ("base000", "abc123"), "from the merge base");
        assert_eq!(d.files.len(), 4);
        let r = p.review_drafts(&project, 3).await.unwrap();
        assert_eq!((r.pending_review.as_deref(), r.can_draft), (Some("13"), true));
        assert_eq!(r.drafts.iter().map(|d| (d.id.as_str(), d.position.as_ref().map(|p| (p.path.as_str(), p.line, p.old_line)))).collect::<Vec<_>>(), [
            ("41", Some(("src/lib.rs", Some(2), None))),
            ("42", Some(("README.md", None, Some(3)))),
        ]);
        assert!(lines(&s).contains(&"get /repos/ada/widget/pulls/3.diff".to_string()));
        let (_, p, project) = served_with(false);
        let r = p.review_drafts(&project, 3).await.unwrap();
        assert_eq!((r.pending_review, r.drafts.len()), (None, 0));
    }

    /// Drafts are kept here and go in with their review as one POST: no pending review is
    /// posted for them (Forgejo's would leave a timeline entry of its own).
    #[tokio::test]
    async fn drafts_stay_here_and_go_in_with_their_review_as_one() {
        let (s, p, project) = served_with(false);
        let d = p.add_draft(&project, 3, &new_comment("Rename this")).await.unwrap();
        assert_eq!((d.id.as_str(), d.position.as_ref().map(|p| (p.path.as_str(), p.line, p.head_sha.as_deref()))), ("local-1", Some(("src/lib.rs", Some(2), Some("abc123")))));
        let mut second = new_comment("Old line");
        second.anchor.end = ReviewLine { kind: LineKind::Removed, old_line: 5, new_line: 4 };
        p.add_draft(&project, 3, &second).await.unwrap();
        p.add_draft(&project, 3, &new_comment("Gone soon")).await.unwrap();
        assert_eq!(p.edit_draft(&project, 3, "local-1", "Rename it").await.unwrap().body, "Rename it");
        p.delete_draft(&project, 3, "local-3").await.unwrap();
        assert!(writes(&s).is_empty(), "nothing posted before the review");
        let r = p.review_drafts(&project, 3).await.unwrap();
        assert_eq!((r.pending_review.as_deref(), r.drafts.iter().map(|d| d.id.as_str()).collect::<Vec<_>>()), (Some("local"), vec!["local-1", "local-2"]));

        p.submit_review(&project, 3, &ReviewSubmit { event: ReviewEvent::RequestChanges, body: "See the notes".into() }).await.unwrap();
        assert_eq!(writes(&s).into_iter().map(|(l, b)| (l, body(&b))).collect::<Vec<_>>(), [
            ("post /repos/ada/widget/pulls/3/reviews".to_string(), json!({"event": "REQUEST_CHANGES", "body": "See the notes", "commit_id": "abc123", "comments": [
                {"path": "src/lib.rs", "body": "Rename it", "new_position": 2, "old_position": 0},
                {"path": "src/lib.rs", "body": "Old line", "new_position": 0, "old_position": 5},
            ]})),
        ]);
        assert_eq!(p.review_drafts(&project, 3).await.unwrap().drafts, [], "sent");
        p.add_draft(&project, 3, &new_comment("x")).await.unwrap();
        assert_eq!(p.discard_review(&project, 3).await.unwrap(), 1);
        assert_eq!(p.review_drafts(&project, 3).await.unwrap().pending_review, None);
    }

    /// A pending review made on the web: submitted as it is, or sent along with the drafts here.
    #[tokio::test]
    async fn a_pending_review_from_the_web_is_submitted_or_goes_along() {
        let (s, p, project) = served();
        p.submit_review(&project, 3, &ReviewSubmit { event: ReviewEvent::Comment, body: String::new() }).await.unwrap();
        p.add_draft(&project, 3, &new_comment("Also")).await.unwrap();
        p.submit_review(&project, 3, &ReviewSubmit { event: ReviewEvent::Approve, body: String::new() }).await.unwrap();
        assert_eq!(p.discard_review(&project, 3).await.unwrap(), 2, "its comments");
        assert_eq!(writes(&s).into_iter().map(|(l, b)| (l, body(&b))).collect::<Vec<_>>(), [
            ("post /repos/ada/widget/pulls/3/reviews/13".to_string(), json!({"event": "COMMENT"})),
            ("post /repos/ada/widget/pulls/3/reviews".to_string(), json!({"event": "APPROVED", "commit_id": "abc123", "comments": [{"path": "src/lib.rs", "body": "Also", "new_position": 2, "old_position": 0}]})),
            ("delete /repos/ada/widget/pulls/3/reviews/13".to_string(), Value::Null),
        ]);
    }

    /// A draft of a pending review made on the web is deleted on the forge, alone: the review
    /// and its other comments stay there, their ids too (nothing is kept here instead).
    #[tokio::test]
    async fn deleting_a_web_draft_deletes_that_comment_alone() {
        let (s, p, project) = served();
        p.delete_draft(&project, 3, "41").await.unwrap();
        assert_eq!(writes(&s).into_iter().map(|(l, _)| l).collect::<Vec<_>>(), ["delete /repos/ada/widget/pulls/3/reviews/13/comments/41"]);
        assert!(p.drafts.lock().unwrap().is_empty(), "nothing moved here");
        // A Forgejo without that API: said, and nothing else is done.
        let e = p.delete_draft(&project, 3, "99").await.unwrap_err();
        assert!(e.message.contains("can't delete one comment of a pending review"), "{}", e.message);
        assert_eq!(writes(&s).len(), 2);
    }

    /// Drafts written against an older head go in on their own commit, after the review: Forgejo
    /// places a review's comments on its one `commit_id`.
    #[tokio::test]
    async fn drafts_on_an_older_head_go_in_on_their_commit_after_the_review() {
        let (s, p, project) = served_with(false);
        let mut old = new_comment("Before the push");
        old.refs.head_sha = "old000".into();
        p.add_draft(&project, 3, &old).await.unwrap();
        p.add_draft(&project, 3, &new_comment("On the head")).await.unwrap();
        p.submit_review(&project, 3, &ReviewSubmit { event: ReviewEvent::Approve, body: String::new() }).await.unwrap();
        let sent: Vec<(String, Value, Value)> = writes(&s).into_iter().map(|(l, b)| { let v = body(&b); (l, v["event"].clone(), v["commit_id"].clone()) }).collect();
        assert_eq!(sent, [
            ("post /repos/ada/widget/pulls/3/reviews".to_string(), json!("APPROVED"), json!("abc123")),
            ("post /repos/ada/widget/pulls/3/reviews".to_string(), json!("COMMENT"), json!("old000")),
        ]);
        assert!(p.review_drafts(&project, 3).await.unwrap().drafts.is_empty());

        // Only older drafts and nothing to say: they're the review, one per head.
        let (s, p, project) = served_with(false);
        p.add_draft(&project, 3, &old).await.unwrap();
        p.submit_review(&project, 3, &ReviewSubmit { event: ReviewEvent::Comment, body: String::new() }).await.unwrap();
        assert_eq!(writes(&s).into_iter().map(|(_, b)| body(&b)["commit_id"].clone()).collect::<Vec<_>>(), [json!("old000")]);
    }

    /// The newest schedule or cancel decides, read from the timeline's last page back.
    #[tokio::test]
    async fn auto_merge_is_read_from_the_newest_timeline_events() {
        let (s, p, project) = served();
        let d = p.mr_detail(&project, 3).await.unwrap().value;
        assert_eq!(d.mr.auto_merge.as_ref().and_then(|a| a.enabled_by.as_ref()).map(|u| u.username.as_str()), Some("grace"));
        let pages: Vec<String> = lines(&s).into_iter().filter(|l| l.contains("/timeline")).collect();
        assert_eq!(pages, ["get /repos/ada/widget/issues/3/timeline?limit=50&page=1", "get /repos/ada/widget/issues/3/timeline?limit=50&page=3"], "the middle page isn't needed");
        assert_eq!(json::page_number("https://h/api/v1/x?limit=50&page=12"), Some(12));
        assert_eq!(json::merge_schedule(&[json!({"type": "comment"})]), None);
        assert_eq!(json::merge_schedule(&[json!({"type": "pull_scheduled_merge", "user": user(1, "ada")}), json!({"type": "pull_cancel_scheduled_merge"})]), Some(None));
    }

    /// A published code comment would publish the pending review with it: Comment now and a thread
    /// reply wait for it to be submitted or discarded.
    #[tokio::test]
    async fn comment_now_publishes_one_comment_unless_a_review_is_pending() {
        let (s, p, project) = served();
        let e = p.comment_now(&project, 3, &new_comment("Now")).await.unwrap_err();
        assert!(e.message.contains("pending review on #3"), "{}", e.message);
        let e = p.reply(&project, 3, &NewNote { discussion: Some("thread-21".into()), body: "Done".into() }).await.unwrap_err();
        assert!(e.message.contains("pending review on #3"), "{}", e.message);
        assert!(writes(&s).is_empty());

        let (s, p, project) = served_with(false);
        let d = p.comment_now(&project, 3, &new_comment("Now")).await.unwrap();
        assert_eq!((d.id.as_str(), d.notes[0].position.as_ref().map(|p| (p.path.as_str(), p.line))), ("thread-24", Some(("src/lib.rs", Some(12)))));
        assert_eq!(writes(&s).into_iter().map(|(l, b)| (l, body(&b)["event"].clone())).collect::<Vec<_>>(), [("post /repos/ada/widget/pulls/3/reviews".to_string(), json!("COMMENT"))]);
    }

    #[tokio::test]
    async fn a_thread_is_resolved_and_unresolved_through_its_first_comment() {
        let (s, p, project) = served();
        let ds = p.discussions(&project, 3).await.unwrap().value;
        assert!(ds.iter().filter(|d| d.id.starts_with("thread-")).all(|d| d.resolvable), "threads are resolvable");
        assert!(!ds.iter().find(|d| d.id == "issue-31").unwrap().resolvable);
        assert_eq!(p.resolve(&project, 3, "thread-21", true).await.unwrap(), ThreadState { resolved: true, resolved_by: Some("ada".into()) });
        assert_eq!(p.resolve(&project, 3, "thread-21", false).await.unwrap(), ThreadState { resolved: false, resolved_by: None });
        assert_eq!(writes(&s).into_iter().map(|(l, _)| l).collect::<Vec<_>>(), ["post /repos/ada/widget/pulls/comments/21/resolve", "post /repos/ada/widget/pulls/comments/21/unresolve"]);
        // A Forgejo without the API (or a comment that's gone) answers 404: said.
        let e = p.resolve(&project, 3, "thread-99", true).await.unwrap_err();
        assert!(e.message.contains("older than its API"), "{}", e.message);
        assert_eq!(p.resolve(&project, 3, "issue-31", true).await.unwrap_err().kind, gitvolt_core::error::GbErrorKind::InvalidInput);
    }

    /// Forgejo's merge API deletes the branch only when asked: the repository's default
    /// (`default_delete_branch_after_merge`, true here) is sent, unless the merge chose.
    #[tokio::test]
    async fn a_merge_deletes_the_branch_as_the_repository_says_unless_told() {
        let (s, p, project) = served();
        p.merge(&project, 3, &MergeOptions::default()).await.unwrap();
        p.merge(&project, 3, &MergeOptions { delete_source_branch: Some(false), ..Default::default() }).await.unwrap();
        let sent: Vec<Value> = writes(&s).into_iter().map(|(_, b)| body(&b)["delete_branch_after_merge"].clone()).collect();
        assert_eq!(sent, [json!(true), json!(false)]);
    }

    /// A range is its first line and the lines after it (`extra_lines_count`), on its last
    /// line's side, both ways: sent, and read back as a position whose start is the first line.
    #[test]
    fn a_range_is_its_first_line_and_the_lines_after_it() {
        let anchor = |start: Option<ReviewLine>, end: ReviewLine| ReviewAnchor { path: "src/lib.rs".into(), old_path: "src/lib.rs".into(), start, end };
        let add = |o, n| ReviewLine { kind: LineKind::Added, old_line: o, new_line: n };
        let del = |o, n| ReviewLine { kind: LineKind::Removed, old_line: o, new_line: n };
        let sent = |a: &ReviewAnchor| { let c = json::line_comment(a, "x"); (c["new_position"].clone(), c["old_position"].clone(), c["extra_lines_count"].clone()) };
        assert_eq!(sent(&anchor(Some(add(3, 4)), add(3, 7))), (json!(4), json!(0), json!(3)));
        assert_eq!(sent(&anchor(Some(del(5, 9)), del(8, 9))), (json!(0), json!(5), json!(3)));
        // From a removed line down to an added one: the new side, from the new line where the
        // removed one sat (the parser numbers it so).
        assert_eq!(sent(&anchor(Some(del(2, 2)), add(3, 3))), (json!(2), json!(0), json!(1)));
        assert_eq!(sent(&anchor(None, add(3, 7))), (json!(7), json!(0), Value::Null), "one line sends none");

        let c = json!({"id": 50, "user": user(1, "ada"), "body": "b", "path": "src/lib.rs", "position": 4, "original_position": 0, "extra_lines_count": 3, "created_at": T0});
        let d = json::draft(&c).unwrap();
        let p = d.position.unwrap();
        assert_eq!((p.start_line, p.line, p.start_old_line, p.old_line), (Some(4), Some(7), None, None));
        let local = json::local_draft(&LocalDraft { id: 1, anchor: anchor(Some(add(3, 4)), add(3, 7)), body: "x".into(), head: "abc123".into() }).position.unwrap();
        assert_eq!((local.start_line, local.line), (Some(4), Some(7)));
    }
}
