//! Forgejo's API v1, the subset GitVolt's provider uses: identity, the repository, pull requests
//! (list, view, diff, statuses, timeline), the conversation, reviews with their code comments,
//! merging and auto-merge, and resolving threads. It keeps Forgejo's own rules where GitVolt has
//! to work around them:
//! - a review event needs a message as `preparePullReviewType` says (`PENDING` and
//!   `REQUEST_CHANGES` always, `COMMENT` unless it carries comments; nobody approves or rejects
//!   their own pull request);
//! - a review POST files its comments under the user's pending review when there's one, and every
//!   review POST (`PENDING` too) leaves a `review` entry in the timeline;
//! - `DELETE /issues/comments/{id}` answers 204 for a code comment and keeps it;
//! - a merge deletes the head branch only when `delete_branch_after_merge` asks;
//! - a submitted review answers the reviewer's review request.

use super::{FakeLabel, FakeProject, FakeRequest, FakeToken, FakeUser, ForgeState, Reply};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const FORGEJO_HOST: &str = "forgejo.example.com";
pub const FORGEJO_TOKEN: &str = "forgejo-FAKE-e2e-ada";
/// The PR the default seed has from the `dev` branch (the sync fixture's).
pub const DEV_PR: u64 = 12;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ForgejoSeed {
    /// `GET /version`'s answer.
    pub version: String,
    pub tokens: Vec<FakeToken>,
    pub repos: Vec<FakeProject>,
    /// Users who appear without a token of their own.
    pub users: Vec<FakeUser>,
    pub labels: Vec<FakeLabel>,
    pub pulls: Vec<FakePull>,
    /// The next id a comment, review or code comment made through the API gets.
    pub next_id: u64,
    /// A Forgejo without `POST pulls/comments/{id}/resolve` (before forgejo/forgejo#14873): 404.
    pub no_resolve_api: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FakePull {
    pub number: u64,
    /// The base repository's `owner/repo`.
    pub repo: String,
    pub head_ref: String,
    pub base_ref: String,
    pub title: String,
    pub body: String,
    /// `open`, `closed`.
    pub state: String,
    pub merged: bool,
    /// The style (`Do`) the fake merged it with.
    pub merged_with: String,
    /// The merge asked to delete the head branch.
    pub branch_deleted: bool,
    pub author: String,
    pub head_sha: String,
    pub base_sha: String,
    pub merge_base: String,
    pub labels: Vec<String>,
    pub requested_reviewers: Vec<String>,
    pub assignees: Vec<String>,
    pub mergeable: bool,
    /// The head's combined status (`success`, `pending`, `failure`); empty: none.
    pub status: String,
    /// `/pulls/{n}.diff`.
    pub diff: String,
    pub comments: Vec<FakeComment>,
    pub reviews: Vec<FakeReview>,
    /// The issue timeline's events, oldest first, as Forgejo sends them.
    pub timeline: Vec<Value>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FakeComment {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FakeReview {
    pub id: u64,
    pub author: String,
    /// `APPROVED`, `REQUEST_CHANGES`, `COMMENT`, `PENDING`.
    pub state: String,
    pub body: String,
    pub commit_id: String,
    pub submitted_at: String,
    pub comments: Vec<FakeCodeComment>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FakeCodeComment {
    pub id: u64,
    pub author: String,
    pub path: String,
    /// The commented line (a range's first) on the new side, else 0.
    pub new_line: u64,
    pub old_line: u64,
    pub extra_lines_count: u64,
    pub body: String,
    pub commit_id: String,
    pub resolver: Option<String>,
    pub created_at: String,
}

const T: &str = "2026-10-01T10:00:00Z";

pub fn default_users(base: &str) -> Vec<FakeUser> {
    let u = |id, login: &str, name: &str| FakeUser { id, username: login.into(), name: name.into(), email: Some(format!("{login}@forgejo.example.com")), avatar_url: Some(format!("{base}/forgejo/avatars/{login}")) };
    vec![u(7, "ada", "Ada Lovelace"), u(8, "grace", "Grace Hopper"), u(9, "joan", "Joan Clarke")]
}

pub fn default_seed(base: &str) -> ForgejoSeed {
    let users = default_users(base);
    let readme = "diff --git a/README.md b/README.md\nindex 1111111..2222222 100644\n--- a/README.md\n+++ b/README.md\n@@ -1 +1,3 @@\n Readme\n+Second line\n+Third line\n";
    let dev = FakePull {
        number: DEV_PR,
        repo: "team/gadget".into(),
        head_ref: "dev".into(),
        base_ref: "main".into(),
        title: "Dev work".into(),
        body: "Adds the dev work.".into(),
        state: "open".into(),
        author: "grace".into(),
        head_sha: "d".repeat(40),
        base_sha: "b".repeat(40),
        merge_base: "a".repeat(40),
        labels: vec!["backend".into()],
        requested_reviewers: vec!["ada".into()],
        mergeable: true,
        status: "success".into(),
        diff: readme.into(),
        comments: vec![FakeComment { id: 31, author: "grace".into(), body: "Looks good overall.".into(), created_at: T.into() }],
        reviews: vec![FakeReview {
            id: 41,
            author: "joan".into(),
            state: "COMMENT".into(),
            body: String::new(),
            commit_id: "d".repeat(40),
            submitted_at: T.into(),
            comments: vec![FakeCodeComment { id: 42, author: "joan".into(), path: "README.md".into(), new_line: 2, body: "Say what the second line is for.".into(), commit_id: "d".repeat(40), created_at: T.into(), ..Default::default() }],
        }],
        timeline: vec![json!({"type": "comment", "user": {"id": 8, "login": "grace", "full_name": "Grace Hopper"}})],
        updated_at: T.into(),
        ..Default::default()
    };
    let draft = FakePull {
        number: 5,
        repo: "team/gadget".into(),
        head_ref: "diverged".into(),
        base_ref: "main".into(),
        title: "WIP: Explore caching".into(),
        state: "open".into(),
        author: "ada".into(),
        head_sha: "e".repeat(40),
        base_sha: "b".repeat(40),
        merge_base: "a".repeat(40),
        status: "pending".into(),
        updated_at: "2026-09-28T10:00:00Z".into(),
        ..Default::default()
    };
    ForgejoSeed {
        version: "11.0.1+gitea-1.22.0".into(),
        tokens: vec![FakeToken { token: FORGEJO_TOKEN.into(), user: users[0].clone(), scopes: vec!["write:repository".into(), "write:issue".into(), "read:user".into()], fine_grained: false }],
        repos: vec![FakeProject { id: 901, path: "team/gadget".into(), default_branch: Some("main".into()), updated_at: T.into(), settings: json!({"default_delete_branch_after_merge": true}), ..Default::default() }],
        users,
        labels: vec![FakeLabel { name: "backend".into(), color: "0075ca".into(), description: None }, FakeLabel { name: "docs".into(), color: "a2eeef".into(), description: Some("Words".into()) }],
        pulls: vec![dev, draft],
        next_id: 100,
        no_resolve_api: false,
    }
}

fn user_of(st: &ForgejoSeed, login: &str) -> FakeUser {
    st.tokens.iter().map(|t| &t.user).chain(st.users.iter()).find(|u| u.username == login).cloned().unwrap_or(FakeUser { username: login.into(), name: login.into(), ..Default::default() })
}

fn user_json(u: &FakeUser, base: &str) -> Value {
    json!({ "id": u.id, "login": u.username, "full_name": u.name, "email": u.email, "avatar_url": u.avatar_url, "html_url": format!("{base}/forgejo/{}", u.username) })
}

fn login_json(st: &ForgejoSeed, login: &str, base: &str) -> Value {
    user_json(&user_of(st, login), base)
}

pub fn repo_json(p: &FakeProject, base: &str) -> Value {
    let (owner, name) = p.path.split_once('/').unwrap_or(("", p.path.as_str()));
    let mut v = json!({
        "id": p.id, "name": name, "full_name": p.path, "owner": { "id": 0, "login": owner },
        "html_url": format!("{base}/forgejo/{}", p.path), "default_branch": p.default_branch,
        "clone_url": p.http_url.clone().unwrap_or_else(|| format!("https://{FORGEJO_HOST}/{}.git", p.path)),
        "ssh_url": p.ssh_url.clone().unwrap_or_else(|| format!("git@{FORGEJO_HOST}:{}.git", p.path)),
        "fork": p.fork_of.is_some(), "parent": p.fork_of.as_ref().map(|f| json!({ "full_name": f })), "archived": p.archived, "updated_at": p.updated_at,
        "allow_merge_commits": true, "allow_rebase": true, "allow_rebase_explicit": false, "allow_squash_merge": true, "allow_fast_forward_only_merge": false,
        "default_merge_style": "merge", "default_delete_branch_after_merge": false,
    });
    if let (Some(obj), Some(extra)) = (v.as_object_mut(), p.settings.as_object()) {
        for (k, x) in extra {
            obj.insert(k.clone(), x.clone());
        }
    }
    v
}

fn pull_json(st: &ForgejoSeed, p: &FakePull, base: &str) -> Value {
    let label = |n: &String| st.labels.iter().find(|l| &l.name == n).map(|l| json!({ "id": label_id(st, n), "name": l.name, "color": l.color }));
    json!({
        "id": 1000 + p.number, "number": p.number, "title": p.title, "body": p.body, "state": p.state, "merged": p.merged,
        "merged_at": p.merged.then_some(&p.updated_at), "user": login_json(st, &p.author, base),
        "labels": p.labels.iter().filter_map(label).collect::<Vec<_>>(),
        "assignees": p.assignees.iter().map(|a| login_json(st, a, base)).collect::<Vec<_>>(),
        "requested_reviewers": p.requested_reviewers.iter().map(|a| login_json(st, a, base)).collect::<Vec<_>>(),
        "head": { "ref": p.head_ref, "sha": p.head_sha, "repo": { "full_name": p.repo } },
        "base": { "ref": p.base_ref, "sha": p.base_sha, "repo": { "full_name": p.repo } },
        "merge_base": p.merge_base, "mergeable": p.mergeable && !p.merged && p.state == "open",
        "html_url": format!("{base}/forgejo/{}/pulls/{}", p.repo, p.number), "updated_at": p.updated_at, "allow_maintainer_edit": false,
    })
}

fn label_id(st: &ForgejoSeed, name: &str) -> u64 {
    st.labels.iter().position(|l| l.name == name).map_or(0, |i| i as u64 + 1)
}

fn comment_json(st: &ForgejoSeed, c: &FakeComment, repo: &str, number: u64, base: &str) -> Value {
    json!({ "id": c.id, "user": login_json(st, &c.author, base), "body": c.body, "created_at": c.created_at, "html_url": format!("{base}/forgejo/{repo}/pulls/{number}#issuecomment-{}", c.id) })
}

fn review_json(st: &ForgejoSeed, r: &FakeReview, base: &str) -> Value {
    json!({ "id": r.id, "user": login_json(st, &r.author, base), "state": r.state, "body": r.body, "commit_id": r.commit_id, "submitted_at": (r.state != "PENDING").then_some(&r.submitted_at), "comments_count": r.comments.len(), "dismissed": false })
}

fn code_json(st: &ForgejoSeed, c: &FakeCodeComment, review: u64, base: &str) -> Value {
    json!({
        "id": c.id, "user": login_json(st, &c.author, base), "body": c.body, "path": c.path, "position": c.new_line, "original_position": c.old_line,
        "extra_lines_count": c.extra_lines_count, "commit_id": c.commit_id, "diff_hunk": "@@ -1 +1,3 @@\n Readme\n+Second line", "created_at": c.created_at,
        "resolver": c.resolver.as_ref().map(|r| login_json(st, r, base)), "pull_request_review_id": review,
    })
}

/// `items` paged by the request's `limit` (default 30) and `page`, with `Link` `next` and `last`
/// and `X-Total-Count`, as Forgejo pages. `url` is absolute, without a query.
fn paged(items: Vec<Value>, r: &FakeRequest, url: &str) -> Reply {
    let limit = r.query.get("limit").and_then(|v| v.parse::<usize>().ok()).filter(|n| *n > 0).unwrap_or(30);
    let page = r.query.get("page").and_then(|v| v.parse::<usize>().ok()).filter(|n| *n > 0).unwrap_or(1);
    let last = items.len().div_ceil(limit).max(1);
    let keep: Vec<(&String, &String)> = r.query.iter().filter(|(k, _)| k.as_str() != "page").collect();
    let at = |p: usize| {
        let q: Vec<String> = keep.iter().map(|(k, v)| format!("{k}={v}")).chain([format!("page={p}")]).collect();
        format!("{url}?{}", q.join("&"))
    };
    let total = items.len();
    let slice: Vec<Value> = items.into_iter().skip((page - 1).saturating_mul(limit)).take(limit).collect();
    let mut links = Vec::new();
    if page < last {
        links.push(format!("<{}>; rel=\"next\"", at(page + 1)));
    }
    links.push(format!("<{}>; rel=\"last\"", at(last)));
    Reply::json(Value::Array(slice)).header("Link", &links.join(", ")).header("X-Total-Count", &total.to_string())
}

fn not_found() -> Reply {
    Reply::status(404, json!({ "message": "Not Found" }))
}

fn refuse(message: impl Into<String>) -> Reply {
    Reply::status(422, json!({ "message": message.into() }))
}

fn body_of(r: &FakeRequest) -> Value {
    serde_json::from_slice(r.body).unwrap_or(Value::Null)
}

fn now_id(st: &mut ForgejoSeed) -> u64 {
    st.next_id += 1;
    st.next_id
}

/// Forgejo's `preparePullReviewType`: the event, or why it's refused.
fn review_event(event: &str, body: &str, has_comments: bool, author: &str, me: &str) -> Result<String, String> {
    let event = event.to_ascii_uppercase();
    let empty = body.trim().is_empty();
    match event.as_str() {
        "APPROVED" if author == me => Err("approve your own pull is not allowed".into()),
        "APPROVED" => Ok(event),
        "REQUEST_CHANGES" if author == me => Err("reject your own pull is not allowed".into()),
        "REQUEST_CHANGES" | "PENDING" | "" if empty => Err(format!("review event {} requires a body", if event.is_empty() { "PENDING" } else { &event })),
        "COMMENT" if empty && !has_comments => Err("review event COMMENT requires a body or a comment".into()),
        "REQUEST_CHANGES" | "COMMENT" => Ok(event),
        _ => Ok("PENDING".into()),
    }
}

pub(crate) fn route(state: &mut ForgeState, r: &FakeRequest) -> Reply {
    let segs: Vec<&str> = r.segments.iter().map(String::as_str).collect();
    // The web side: avatars, without a token.
    if let ["avatars", ..] = segs.as_slice() {
        return Reply::png();
    }
    let Some(api) = segs.strip_prefix(&["api", "v1"]) else { return not_found() };
    let st = &mut state.seed.forgejo;
    let Some(me) = r.token.as_ref().map(|t| t.user.username.clone()) else {
        return Reply::status(401, json!({ "message": "Only signed in user is allowed to call APIs." }));
    };
    let base = r.base;
    let api_url = format!("{base}/forgejo/api/v1");
    match (r.method, api) {
        ("GET", ["version"]) => Reply::json(json!({ "version": st.version })),
        ("GET", ["user"]) => Reply::json(login_json(st, &me, base)),
        ("GET", ["users", "search"]) => {
            let all: Vec<FakeUser> = st.tokens.iter().map(|t| t.user.clone()).chain(st.users.iter().cloned()).collect();
            let hit = |u: &FakeUser| match (r.query.get("uid"), r.query.get("q")) {
                (Some(id), _) => id.parse::<u64>().ok() == Some(u.id),
                (_, Some(q)) => u.username.contains(q.as_str()),
                _ => true,
            };
            let mut seen = Vec::new();
            let data: Vec<Value> = all.iter().filter(|u| hit(u) && !seen.contains(&u.username) && { seen.push(u.username.clone()); true }).map(|u| user_json(u, base)).collect();
            Reply::json(json!({ "ok": true, "data": data }))
        }
        (_, ["repos", o, n, rest @ ..]) => {
            let path = format!("{o}/{n}");
            let Some(repo) = st.repos.iter().find(|p| p.path == path).cloned() else { return not_found() };
            repo_route(st, r, &me, &repo, rest, &format!("{api_url}/repos/{path}"))
        }
        _ => not_found(),
    }
}

fn repo_route(st: &mut ForgejoSeed, r: &FakeRequest, me: &str, repo: &FakeProject, rest: &[&str], url: &str) -> Reply {
    let base = r.base;
    let pull_at = |st: &ForgejoSeed, n: &str| st.pulls.iter().position(|p| p.repo == repo.path && n.parse::<u64>().ok() == Some(p.number));
    match (r.method, rest) {
        ("GET", []) => Reply::json(repo_json(repo, base)),
        ("GET", ["forks"]) => paged(Vec::new(), r, &format!("{url}/forks")),
        ("GET", ["labels"]) => Reply::json(Value::Array(st.labels.iter().map(|l| json!({ "id": label_id(st, &l.name), "name": l.name, "color": l.color, "description": l.description })).collect())),
        ("GET", ["contents"]) => Reply::json(json!([])),
        ("GET", ["contents", ..]) => not_found(),
        ("GET", ["commits", sha, "status"]) => {
            let status = st.pulls.iter().find(|p| p.head_sha == *sha).map(|p| p.status.clone()).unwrap_or_default();
            let statuses = if status.is_empty() { json!([]) } else { json!([{ "status": status, "context": "ci", "target_url": format!("{base}/forgejo/{}/actions", repo.path) }]) };
            Reply::json(json!({ "state": status, "statuses": statuses }))
        }
        ("GET", ["pulls"]) => {
            let want = r.query.get("state").map(String::as_str).unwrap_or("open");
            let mut list: Vec<&FakePull> = st.pulls.iter().filter(|p| p.repo == repo.path && (want == "all" || p.state == want)).collect();
            list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            let items = list.iter().map(|p| pull_json(st, p, base)).collect();
            paged(items, r, &format!("{url}/pulls"))
        }
        // `/pulls/{n}.diff` arrives as one segment.
        ("GET", ["pulls", n]) if n.ends_with(".diff") => match pull_at(st, n.trim_end_matches(".diff")) {
            Some(i) => Reply { status: 200, headers: Vec::new(), body: st.pulls[i].diff.clone().into_bytes(), content_type: "text/plain" },
            None => not_found(),
        },
        ("GET", ["pulls", n]) => pull_at(st, n).map_or_else(not_found, |i| Reply::json(pull_json(st, &st.pulls[i], base))),
        ("PATCH", ["pulls", n]) => {
            let Some(i) = pull_at(st, n) else { return not_found() };
            let b = body_of(r);
            let p = &mut st.pulls[i];
            if let Some(t) = b["title"].as_str() {
                p.title = t.into();
            }
            if let Some(t) = b["body"].as_str() {
                p.body = t.into();
            }
            if let Some(t) = b["base"].as_str() {
                p.base_ref = t.into();
            }
            Reply::json(pull_json(st, &st.pulls[i], base))
        }
        ("POST", ["pulls", n, "merge"]) => merge(st, r, me, n, &repo.path),
        ("DELETE", ["pulls", n, "merge"]) => {
            let Some(i) = pull_at(st, n) else { return not_found() };
            let who = login_json(st, me, base);
            st.pulls[i].timeline.push(json!({ "type": "pull_cancel_scheduled_merge", "user": who }));
            Reply::no_content()
        }
        ("GET", ["pulls", n, "reviews"]) => {
            let Some(i) = pull_at(st, n) else { return not_found() };
            // Another user's pending review isn't listed.
            let items = st.pulls[i].reviews.iter().filter(|x| x.state != "PENDING" || x.author == me).map(|x| review_json(st, x, base)).collect();
            paged(items, r, &format!("{url}/pulls/{n}/reviews"))
        }
        ("POST", ["pulls", n, "reviews"]) => create_review(st, r, me, n, &repo.path),
        ("POST", ["pulls", n, "reviews", id]) => submit_review(st, r, me, n, id, &repo.path),
        ("DELETE", ["pulls", n, "reviews", id]) => {
            let Some(i) = pull_at(st, n) else { return not_found() };
            let before = st.pulls[i].reviews.len();
            st.pulls[i].reviews.retain(|x| !(x.id.to_string() == *id && x.author == me));
            if st.pulls[i].reviews.len() == before { not_found() } else { Reply::no_content() }
        }
        ("GET", ["pulls", n, "reviews", id, "comments"]) => {
            let Some(i) = pull_at(st, n) else { return not_found() };
            match st.pulls[i].reviews.iter().find(|x| x.id.to_string() == *id && (x.state != "PENDING" || x.author == me)) {
                Some(x) => Reply::json(Value::Array(x.comments.iter().map(|c| code_json(st, c, x.id, base)).collect())),
                None => not_found(),
            }
        }
        ("DELETE", ["pulls", n, "reviews", id, "comments", c]) => {
            let Some(i) = pull_at(st, n) else { return not_found() };
            let Some(x) = st.pulls[i].reviews.iter_mut().find(|x| x.id.to_string() == *id && x.author == me) else { return not_found() };
            let before = x.comments.len();
            x.comments.retain(|k| k.id.to_string() != *c);
            if x.comments.len() == before { not_found() } else { Reply::no_content() }
        }
        ("POST", ["pulls", "comments", id, verb @ ("resolve" | "unresolve")]) if !st.no_resolve_api => {
            let on = *verb == "resolve";
            for p in st.pulls.iter_mut().filter(|p| p.repo == repo.path) {
                let all: Vec<(u64, String, u64, u64)> = p.reviews.iter().flat_map(|x| x.comments.iter().map(|c| (c.id, c.path.clone(), c.new_line, c.old_line))).collect();
                let Some((_, path, new, old)) = all.iter().find(|(cid, ..)| cid.to_string() == *id).cloned() else { continue };
                // The thread's first comment holds the mark.
                let root = all.iter().filter(|(_, p, n, o)| *p == path && *n == new && *o == old).map(|(cid, ..)| *cid).min().unwrap_or(0);
                for x in p.reviews.iter_mut() {
                    for c in x.comments.iter_mut().filter(|c| c.id == root) {
                        c.resolver = on.then(|| me.to_string());
                    }
                }
                let (rv, c) = p.reviews.iter().find_map(|x| x.comments.iter().find(|c| c.id == root).map(|c| (x.id, c.clone()))).expect("the root is there");
                return Reply::json(code_json(st, &c, rv, base));
            }
            not_found()
        }
        ("GET", ["issues", n, "comments"]) => {
            let Some(i) = pull_at(st, n) else { return not_found() };
            let p = &st.pulls[i];
            let items = p.comments.iter().map(|c| comment_json(st, c, &p.repo, p.number, base)).collect();
            paged(items, r, &format!("{url}/issues/{n}/comments"))
        }
        ("POST", ["issues", n, "comments"]) => {
            let Some(i) = pull_at(st, n) else { return not_found() };
            let id = now_id(st);
            let c = FakeComment { id, author: me.into(), body: body_of(r)["body"].as_str().unwrap_or_default().into(), created_at: "2026-10-05T10:00:00Z".into() };
            st.pulls[i].comments.push(c.clone());
            let p = &st.pulls[i];
            Reply::status(201, comment_json(st, &c, &p.repo, p.number, base))
        }
        ("PATCH", ["issues", "comments", id]) => {
            let body = body_of(r)["body"].as_str().unwrap_or_default().to_string();
            for p in st.pulls.iter_mut() {
                if let Some(c) = p.comments.iter_mut().find(|c| c.id.to_string() == *id) {
                    c.body = body;
                    let (c, repo, n) = (c.clone(), p.repo.clone(), p.number);
                    return Reply::json(comment_json(st, &c, &repo, n, base));
                }
                if let Some(c) = p.reviews.iter_mut().flat_map(|x| x.comments.iter_mut()).find(|c| c.id.to_string() == *id) {
                    c.body = body;
                    let c = c.clone();
                    return Reply::json(json!({ "id": c.id, "user": login_json(st, &c.author, base), "body": c.body, "created_at": c.created_at }));
                }
            }
            not_found()
        }
        // Forgejo deletes a conversation comment; a code comment it answers 204 for and keeps.
        ("DELETE", ["issues", "comments", id]) => {
            for p in st.pulls.iter_mut() {
                p.comments.retain(|c| c.id.to_string() != *id);
            }
            Reply::no_content()
        }
        ("GET", ["issues", "comments", _, "reactions"]) => Reply::json(json!([])),
        ("POST", ["issues", "comments", _, "reactions"]) => Reply::status(201, json!({ "content": body_of(r)["content"], "user": login_json(st, me, base) })),
        ("GET", ["issues", n, "timeline"]) => {
            let Some(i) = pull_at(st, n) else { return not_found() };
            paged(st.pulls[i].timeline.clone(), r, &format!("{url}/issues/{n}/timeline"))
        }
        ("GET", ["issues", _, "subscriptions", "check"]) => Reply::json(json!({ "subscribed": false })),
        ("PUT" | "DELETE", ["issues", _, "subscriptions", _]) => Reply::status(201, json!({})),
        _ => not_found(),
    }
}

/// `POST /pulls/{n}/reviews`: its comments go under the user's pending review (started when
/// there's none), then the review is submitted with the event, or stays pending; either way a
/// `review` entry joins the timeline.
fn create_review(st: &mut ForgejoSeed, r: &FakeRequest, me: &str, n: &str, repo: &str) -> Reply {
    let Some(i) = st.pulls.iter().position(|p| p.repo == repo && n.parse::<u64>().ok() == Some(p.number)) else { return not_found() };
    let b = body_of(r);
    let comments = b["comments"].as_array().cloned().unwrap_or_default();
    let body = b["body"].as_str().unwrap_or_default().to_string();
    let event = match review_event(b["event"].as_str().unwrap_or_default(), &body, !comments.is_empty(), &st.pulls[i].author, me) {
        Ok(e) => e,
        Err(m) => return refuse(m),
    };
    let commit = b["commit_id"].as_str().map_or_else(|| st.pulls[i].head_sha.clone(), str::to_string);
    let mut made = Vec::new();
    for c in &comments {
        let id = now_id(st);
        made.push(FakeCodeComment {
            id,
            author: me.into(),
            path: c["path"].as_str().unwrap_or_default().into(),
            new_line: c["new_position"].as_u64().unwrap_or(0),
            old_line: c["old_position"].as_u64().unwrap_or(0),
            extra_lines_count: c["extra_lines_count"].as_u64().unwrap_or(0),
            body: c["body"].as_str().unwrap_or_default().into(),
            commit_id: commit.clone(),
            created_at: "2026-10-05T10:00:00Z".into(),
            resolver: None,
        });
    }
    let p = &mut st.pulls[i];
    let pending = p.reviews.iter().position(|x| x.author == me && x.state == "PENDING");
    let at = match pending {
        Some(at) => at,
        None => {
            let id = st.next_id + 1;
            st.next_id = id;
            let p = &mut st.pulls[i];
            p.reviews.push(FakeReview { id, author: me.into(), state: "PENDING".into(), commit_id: commit.clone(), ..Default::default() });
            p.reviews.len() - 1
        }
    };
    let p = &mut st.pulls[i];
    let x = &mut p.reviews[at];
    x.comments.extend(made);
    x.state = event;
    x.body = body.clone();
    x.commit_id = commit;
    x.submitted_at = "2026-10-05T10:00:00Z".into();
    let x = x.clone();
    let who = login_json(st, me, r.base);
    let p = &mut st.pulls[i];
    p.timeline.push(json!({ "type": "review", "body": body, "review_id": x.id, "user": who }));
    // An approval or a change request answers the user's review request; a comment doesn't.
    if x.state == "APPROVED" || x.state == "REQUEST_CHANGES" {
        p.requested_reviewers.retain(|u| u != me);
    }
    Reply::json(review_json(st, &x, r.base))
}

/// `POST /pulls/{n}/reviews/{id}`: a pending review of the user's, submitted with the event.
fn submit_review(st: &mut ForgejoSeed, r: &FakeRequest, me: &str, n: &str, id: &str, repo: &str) -> Reply {
    let Some(i) = st.pulls.iter().position(|p| p.repo == repo && n.parse::<u64>().ok() == Some(p.number)) else { return not_found() };
    let author = st.pulls[i].author.clone();
    let Some(x) = st.pulls[i].reviews.iter_mut().find(|x| x.id.to_string() == id && x.author == me) else { return not_found() };
    if x.state != "PENDING" {
        return refuse("only a pending review can be submitted");
    }
    let b = body_of(r);
    let body = b["body"].as_str().unwrap_or_default().to_string();
    let event = match review_event(b["event"].as_str().unwrap_or_default(), &body, !x.comments.is_empty(), &author, me) {
        Ok(e) if e == "PENDING" => return refuse("review stay pending"),
        Ok(e) => e,
        Err(m) => return refuse(m),
    };
    x.state = event;
    x.body = body.clone();
    x.submitted_at = "2026-10-05T10:00:00Z".into();
    let x = x.clone();
    let who = login_json(st, me, r.base);
    let p = &mut st.pulls[i];
    p.timeline.push(json!({ "type": "review", "body": body, "review_id": x.id, "user": who }));
    // An approval or a change request answers the user's review request; a comment doesn't.
    if x.state == "APPROVED" || x.state == "REQUEST_CHANGES" {
        p.requested_reviewers.retain(|u| u != me);
    }
    Reply::json(review_json(st, &x, r.base))
}

/// `POST /pulls/{n}/merge`: merged now, or scheduled (`merge_when_checks_succeed` while its
/// checks haven't passed). The head branch goes only when `delete_branch_after_merge` says so.
fn merge(st: &mut ForgejoSeed, r: &FakeRequest, me: &str, n: &str, repo: &str) -> Reply {
    let Some(i) = st.pulls.iter().position(|p| p.repo == repo && n.parse::<u64>().ok() == Some(p.number)) else { return not_found() };
    let b = body_of(r);
    let who = login_json(st, me, r.base);
    let p = &mut st.pulls[i];
    if p.merged || p.state != "open" {
        return Reply::status(405, json!({ "message": "The pull request is closed or merged" }));
    }
    if b["head_commit_id"].as_str().is_some_and(|h| h != p.head_sha) {
        return Reply::status(409, json!({ "message": "head out of date" }));
    }
    if b["merge_when_checks_succeed"].as_bool() == Some(true) && p.status != "success" {
        p.timeline.push(json!({ "type": "pull_scheduled_merge", "user": who }));
        return Reply::status(201, json!({}));
    }
    if !p.mergeable || p.title.starts_with("WIP:") {
        return Reply::status(405, json!({ "message": "Please try again later" }));
    }
    p.merged = true;
    p.state = "closed".into();
    p.merged_with = b["Do"].as_str().unwrap_or("merge").into();
    p.branch_deleted = b["delete_branch_after_merge"].as_bool() == Some(true);
    p.timeline.push(json!({ "type": "merge_pull", "user": who }));
    Reply::json(json!({}))
}
