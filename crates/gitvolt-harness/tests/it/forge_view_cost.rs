//! What an open MR/PR view, a hover card and a burst of avatar lookups cost, on the big seed of
//! `forge_poll_cost`: the fake forge's request log, grouped by endpoint.
//!
//! The tables: `cargo test -p gitvolt-harness --test it forge_view_cost:: -- --nocapture`.

use crate::forge_poll_cost::{ok, open_mr, poll, print, setup, take, Setup, Step};
use gitvolt_core::forge::*;
use gitvolt_core::redact::Secret;
use gitvolt_forge::endpoints::HostEndpoints;
use gitvolt_forge::github::GitHubProvider;
use gitvolt_forge::gitlab::GitLabProvider;
use gitvolt_harness::fake_forge::github_pulls::{FakeComment, FakeReaction, FakeReviewComment};
use gitvolt_harness::fake_forge::gitlab_mrs::{FakeAward, FakeDiscussion, FakeNote};
use gitvolt_harness::fake_forge::*;
use serde_json::json;

/// Requests whose endpoint key has `part`.
fn count(step: &Step, part: &str) -> usize {
    step.by.iter().filter(|(k, _)| k.contains(part)).map(|(_, (n, _))| n).sum()
}

/// MR/PR 105 gets a conversation: a comment with a reaction, and a review thread (GitHub) or a
/// discussion whose note has an award (GitLab).
fn with_conversation(s: &Setup, kind: &str) {
    let mut seed = s.h.forge.current_seed();
    if kind == "gitlab" {
        let m = seed.gitlab.merge_requests.iter_mut().find(|m| m.iid == 105).unwrap();
        let note = FakeNote { id: 9001, author: "grace".into(), body: "Why this way?".into(), created_at: "2026-10-03T08:00:00Z".into(), awards: vec![FakeAward { name: "thumbsup".into(), user: "ada".into(), ..Default::default() }], ..Default::default() };
        m.discussions = vec![FakeDiscussion { id: "d9001".into(), notes: vec![note], resolvable: true, ..Default::default() }];
    } else {
        let p = seed.github.pulls.iter_mut().find(|p| p.number == 105).unwrap();
        p.comments = vec![FakeComment { id: 7001, user: "hubot".into(), body: "Looks good.".into(), created_at: "2026-10-03T08:00:00Z".into(), reactions: vec![FakeReaction { id: 1, user: "octocat".into(), content: "+1".into() }] }];
        p.review_comments = vec![FakeReviewComment { id: 7002, user: "hubot".into(), body: "Why this way?".into(), created_at: "2026-10-03T08:00:00Z".into(), path: "src/lib.rs".into(), line: Some(3), side: "RIGHT".into(), diff_hunk: "@@ -1,3 +1,3 @@\n a\n b\n-c\n+d".into(), ..Default::default() }];
    }
    s.h.forge.seed(seed);
}

/// One idle minute with MR/PR 105's view open: the full poll (badges, list, the view's detail and
/// discussion) and, with `review`, the review session's refresh.
async fn idle_minute(s: &Setup, review: bool) {
    poll(s, "timer", "all", Some(105)).await;
    if review {
        ok(&s.h.api, json!({"method": "forgeReviewDrafts", "params": {"repo": s.id, "number": 105}})).await;
    }
}

/// Per idle minute, with the view open (and with a review session), once it has settled.
#[tokio::test(flavor = "multi_thread")]
async fn an_open_mr_view_costs_a_few_304s_an_idle_minute() {
    for kind in ["gitlab", "github"] {
        let s = setup(kind, false).await;
        with_conversation(&s, kind);
        poll(&s, "activate", "all", None).await;
        poll(&s, "timer", "all", None).await;
        open_mr(&s, 105).await;
        idle_minute(&s, false).await;
        take(&s, "settling");
        idle_minute(&s, false).await;
        let view = take(&s, "an idle minute, MR/PR view open");
        ok(&s.h.api, json!({"method": "forgeReviewDrafts", "params": {"repo": s.id, "number": 105}})).await;
        idle_minute(&s, true).await;
        take(&s, "settling");
        idle_minute(&s, true).await;
        let review = take(&s, "an idle minute, view open, review session");
        let steps = [view, review];
        print(&format!("{kind}: an open view"), &steps);
        let [view, review] = &steps;
        // Every request a 304 (GraphQL aside), and no GraphQL nor labels while nothing changed:
        // the threads and awards are asked again after THREADS_REFRESH / AWARDS_REFRESH.
        let graphql = |st: &Step| count(st, "graphql");
        for st in &steps {
            assert_eq!((graphql(st), count(st, "/labels")), (0, 0), "{kind}, {}: {:?}", st.name, st.by);
            assert_eq!(st.not_modified, st.total, "{kind}, {}: {:?}", st.name, st.by);
        }
        let (view_max, review_max) = if kind == "gitlab" { (4, 5) } else { (7, 7) };
        assert!(view.total <= view_max && review.total <= review_max, "{kind}: {} and {}", view.total, review.total);
        if kind == "github" {
            assert_eq!(count(review, "/reviews"), 1, "the reviews once a minute, one representation: {:?}", review.by);
            assert_eq!(review.by.get("GET /repos/octo-org/widget/pulls/:n").map(|(n, _)| *n), Some(1), "the PR once: {:?}", review.by);
        }
    }
}

/// A hover card over an MR/PR the list shows (its pipeline known from the list).
#[tokio::test(flavor = "multi_thread")]
async fn a_hover_card_reads_the_mr_and_its_review_only() {
    for kind in ["gitlab", "github"] {
        let s = setup(kind, false).await;
        poll(&s, "activate", "all", None).await;
        poll(&s, "timer", "all", None).await;
        take(&s, "polls");
        s.h.forge.advance();
        ok(&s.h.api, json!({"method": "forgeMrCard", "params": {"repo": s.id, "number": 110}})).await;
        let hover = take(&s, "a hover card");
        let steps = [hover];
        print(&format!("{kind}: a hover card"), &steps);
        let hover = &steps[0];
        assert_eq!((hover.total, count(hover, "graphql"), count(hover, "/commits/"), count(hover, "/labels")), (2, 0, 0, 0), "{kind}: the MR/PR and its review only: {:?}", hover.by);
    }
}

/// Long descriptions put the open list over a megabyte: the steady poll still revalidates it
/// (one 304), the badges' and the list's reads one request.
#[tokio::test(flavor = "multi_thread")]
async fn a_list_over_a_megabyte_is_revalidated_not_read_again() {
    for kind in ["gitlab", "github"] {
        let s = setup(kind, false).await;
        let long = "A long description. ".repeat(1600);
        let mut seed = s.h.forge.current_seed();
        seed.gitlab.merge_requests.iter_mut().for_each(|m| m.description = long.clone());
        seed.github.pulls.iter_mut().for_each(|p| p.body = long.clone());
        s.h.forge.seed(seed);
        poll(&s, "activate", "all", None).await;
        poll(&s, "timer", "all", None).await;
        take(&s, "first polls");
        poll(&s, "timer", "all", None).await;
        let steady = take(&s, "steady poll, a list over 1 MB");
        let steps = [steady];
        print(&format!("{kind}: a big list"), &steps);
        assert_eq!((steps[0].total, steps[0].not_modified), (1, 1), "{kind}: {:?}", steps[0].by);
    }
}

fn github(f: &FakeForge) -> GitHubProvider {
    GitHubProvider::new(GITHUB_HOST, &HostEndpoints { api: f.github_api(), web: f.github_web(), avatars: Some(f.github_avatars()) }, Secret::new(GITHUB_TOKEN), None).with_change_counter(f.change_counter())
}

fn gitlab(f: &FakeForge) -> GitLabProvider {
    GitLabProvider::new(GITLAB_HOST, &HostEndpoints { api: f.gitlab_api(), web: f.gitlab_web(), avatars: None }, Secret::new(GITLAB_TOKEN), None).with_change_counter(f.change_counter())
}

/// Commit authors nobody knows, more than the ETag cache keeps entries.
const AUTHORS: usize = 300;

/// 300 unknown commit authors looked up between two polls: the poll after still revalidates its
/// list (a 304), its validator not pushed out by the lookups.
#[tokio::test(flavor = "multi_thread")]
async fn an_avatar_burst_leaves_the_polls_validators() {
    let f = FakeForge::start().await;
    let p = github(&f);
    let w = p.project("octo-org/widget").await.unwrap().value;
    p.open_mrs(&w, MrFilter::All).await.unwrap();
    for i in 0..AUTHORS {
        let _ = p.avatar_for_email_in(&w, &format!("author{i}@example.com")).await;
    }
    f.clear_requests();
    f.advance();
    p.open_mrs(&w, MrFilter::All).await.unwrap();
    let github_after: Vec<(String, u16)> = f.requests().into_iter().filter(|r| r.path.ends_with("/pulls")).map(|r| (r.path, r.status)).collect();

    let g = gitlab(&f);
    let project = g.project("group/project").await.unwrap().value;
    g.open_mrs(&project, MrFilter::All).await.unwrap();
    for i in 0..AUTHORS {
        let _ = g.avatar_for_email(&format!("author{i}@example.com")).await;
    }
    f.clear_requests();
    f.advance();
    g.open_mrs(&project, MrFilter::All).await.unwrap();
    let gitlab_after: Vec<(String, u16)> = f.requests().into_iter().filter(|r| r.path.ends_with("/merge_requests")).map(|r| (r.path, r.status)).collect();
    eprintln!("\n=== after {AUTHORS} avatar lookups ===\n    github list: {github_after:?}\n    gitlab list: {gitlab_after:?}");
    assert!(github_after.iter().all(|(_, s)| *s == 304), "GitHub: {github_after:?}");
    assert!(gitlab_after.iter().all(|(_, s)| *s == 304), "GitLab: {gitlab_after:?}");
}
