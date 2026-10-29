//! The Forgejo provider's pull requests against the fake Forgejo, which keeps Forgejo's own rules
//! (a message for `PENDING` and Request changes, one timeline entry per review POST, a code
//! comment `DELETE /issues/comments` keeps, a branch deleted only when asked).

use gitvolt_core::error::GbErrorKind;
use gitvolt_core::forge::*;
use gitvolt_core::redact::Secret;
use gitvolt_forge::endpoints::HostEndpoints;
use gitvolt_forge::forgejo::ForgejoProvider;
use gitvolt_harness::fake_forge::forgejo::{FakeCodeComment, FakeReview, DEV_PR, FORGEJO_HOST, FORGEJO_TOKEN};
use gitvolt_harness::fake_forge::*;

fn provider(f: &FakeForge) -> ForgejoProvider {
    ForgejoProvider::new(FORGEJO_HOST, &HostEndpoints { api: f.forgejo_api(), web: f.forgejo_web(), avatars: None }, Secret::new(FORGEJO_TOKEN), None).with_change_counter(f.change_counter())
}

async fn gadget(p: &ForgejoProvider) -> ForgeProject {
    p.project("team/gadget").await.unwrap().value
}

fn pull(f: &FakeForge, n: u64) -> forgejo::FakePull {
    f.current_seed().forgejo.pulls.into_iter().find(|p| p.number == n).unwrap()
}

/// The `review` entries a PR's timeline has: one per review POST, as Forgejo leaves them.
fn review_entries(f: &FakeForge, n: u64) -> usize {
    pull(f, n).timeline.iter().filter(|e| e["type"] == "review").count()
}

fn comment(path: &str, line: u32, body: &str, head: &str) -> NewReviewComment {
    NewReviewComment {
        anchor: ReviewAnchor { path: path.into(), old_path: path.into(), start: None, end: ReviewLine { kind: LineKind::Added, old_line: 1, new_line: line } },
        body: body.into(),
        refs: DiffRefs { base_sha: "a".repeat(40), start_sha: "a".repeat(40), head_sha: head.into() },
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_account_the_version_and_the_repository() {
    let f = FakeForge::start().await;
    let p = provider(&f);
    let check = p.check_token().await.unwrap();
    assert_eq!((check.user.username.as_str(), check.write), ("ada", WriteAccess::Unknown));
    assert_eq!(p.version().await.unwrap().as_deref(), Some("11.0.1+gitea-1.22.0"));
    let g = gadget(&p).await;
    assert_eq!((g.kind, g.path.as_str(), g.default_branch.as_deref()), (ForgeKind::Forgejo, "team/gadget", Some("main")));
    let s = p.project_settings(&g).await.unwrap();
    assert!(s.delete_source_branch, "the seed's repository deletes the branch after merging");
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_open_prs_with_their_checks_and_drafts() {
    let f = FakeForge::start().await;
    let p = provider(&f);
    let g = gadget(&p).await;
    let list = p.open_mrs(&g, MrFilter::All).await.unwrap().value;
    assert_eq!(list.iter().map(|m| (m.number, m.state)).collect::<Vec<_>>(), [(DEV_PR, MrState::Open), (5, MrState::Draft)]);
    assert_eq!(list[0].pipeline.as_ref().map(|p| p.status), Some(PipelineStatus::Success));
    assert_eq!(list[1].pipeline.as_ref().map(|p| p.status), Some(PipelineStatus::Pending));
    let found = p.mr_for_branch(&g, &SourceRef { project: "team/gadget".into(), branch: "dev".into() }).await.unwrap().value;
    assert_eq!(found.map(|m| m.number), Some(DEV_PR));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_view_has_its_reviews_conversation_and_threads() {
    let f = FakeForge::start().await;
    let p = provider(&f);
    let g = gadget(&p).await;
    let d = p.mr_detail(&g, DEV_PR).await.unwrap().value;
    assert_eq!(d.mr.review.decision, ReviewDecision::ReviewRequired);
    assert_eq!(d.mr.labels, ["backend"]);
    assert!(d.mr.auto_merge.is_none());
    let ds = p.discussions(&g, DEV_PR).await.unwrap().value;
    assert_eq!(ds.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["issue-31", "thread-42"]);
    let at = ds[1].notes[0].position.as_ref().unwrap();
    assert_eq!((at.path.as_str(), at.line, ds[1].resolvable), ("README.md", Some(2), true));
    let diff = p.review_diff(&g, DEV_PR).await.unwrap();
    assert_eq!((diff.refs.start_sha.clone(), diff.files.len(), diff.files[0].lines.len()), ("a".repeat(40), 1, 3));
}

/// Drafts stay in GitVolt and go in with their review as one POST: the timeline gains one review
/// entry, not one per draft, and the comments sit on the head they were written against.
#[tokio::test(flavor = "multi_thread")]
async fn a_review_with_drafts_is_one_timeline_entry() {
    let f = FakeForge::start().await;
    let p = provider(&f);
    let g = gadget(&p).await;
    let head = "d".repeat(40);
    p.add_draft(&g, DEV_PR, &comment("README.md", 2, "Why two lines?", &head)).await.unwrap();
    p.add_draft(&g, DEV_PR, &comment("README.md", 3, "And a third?", &head)).await.unwrap();
    assert_eq!(review_entries(&f, DEV_PR), 0, "nothing posted for the drafts");
    p.submit_review(&g, DEV_PR, &ReviewSubmit { event: ReviewEvent::Comment, body: String::new() }).await.unwrap();
    assert_eq!(review_entries(&f, DEV_PR), 1);
    let mine = pull(&f, DEV_PR).reviews.into_iter().find(|r| r.author == "ada").unwrap();
    assert_eq!((mine.state.as_str(), mine.comments.len(), mine.commit_id.as_str()), ("COMMENT", 2, head.as_str()));
    // Forgejo still lists ada as requested; GitVolt shows the review.
    assert_eq!(pull(&f, DEV_PR).requested_reviewers, ["ada"]);
    let review = p.mr_detail(&g, DEV_PR).await.unwrap().value.mr.review;
    let ada = review.reviews.iter().find(|r| r.user.username == "ada").unwrap();
    assert_eq!((review.decision, ada.state), (ReviewDecision::None, ReviewState::Commented));
}

/// Forgejo's own rules hold: Request changes needs a message, comments or not; nobody approves
/// their own PR.
#[tokio::test(flavor = "multi_thread")]
async fn forgejo_refuses_what_it_refuses() {
    let f = FakeForge::start().await;
    let p = provider(&f);
    let g = gadget(&p).await;
    p.add_draft(&g, DEV_PR, &comment("README.md", 2, "x", &"d".repeat(40))).await.unwrap();
    let e = p.submit_review(&g, DEV_PR, &ReviewSubmit { event: ReviewEvent::RequestChanges, body: String::new() }).await.unwrap_err();
    assert_eq!(e.kind, GbErrorKind::InvalidInput);
    assert!(e.message.contains("requires a body"), "{}", e.message);
    assert_eq!(p.review_drafts(&g, DEV_PR).await.unwrap().drafts.len(), 1, "a refused review keeps its drafts");
    p.submit_review(&g, DEV_PR, &ReviewSubmit { event: ReviewEvent::RequestChanges, body: "Explain the lines".into() }).await.unwrap();
    assert_eq!(pull(&f, DEV_PR).reviews.iter().filter(|r| r.author == "ada").map(|r| r.state.as_str()).collect::<Vec<_>>(), ["REQUEST_CHANGES"]);
    assert!(p.approve(&g, 5).await.unwrap_err().message.contains("approve your own pull"));
}

/// A pending review started on the web: one draft is deleted on the forge alone, the rest stay
/// there and go in with the review.
#[tokio::test(flavor = "multi_thread")]
async fn a_web_pending_review_loses_one_draft_and_keeps_the_rest() {
    let f = FakeForge::start().await;
    let mut seed = f.current_seed();
    let code = |id, line| FakeCodeComment { id, author: "ada".into(), path: "README.md".into(), new_line: line, body: format!("draft {id}"), commit_id: "d".repeat(40), ..Default::default() };
    let dev = seed.forgejo.pulls.iter_mut().find(|p| p.number == DEV_PR).unwrap();
    dev.reviews.push(FakeReview { id: 60, author: "ada".into(), state: "PENDING".into(), body: "started on the web".into(), commit_id: "d".repeat(40), comments: vec![code(61, 2), code(62, 3)], ..Default::default() });
    f.seed(seed);
    let p = provider(&f);
    let g = gadget(&p).await;
    let pending = p.review_drafts(&g, DEV_PR).await.unwrap();
    assert_eq!((pending.pending_review.as_deref(), pending.drafts.iter().map(|d| d.id.as_str()).collect::<Vec<_>>()), (Some("60"), vec!["61", "62"]));
    p.delete_draft(&g, DEV_PR, "61").await.unwrap();
    let left = p.review_drafts(&g, DEV_PR).await.unwrap();
    assert_eq!(left.drafts.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["62"], "the other keeps its id");
    p.submit_review(&g, DEV_PR, &ReviewSubmit { event: ReviewEvent::Comment, body: String::new() }).await.unwrap();
    let review = pull(&f, DEV_PR).reviews.into_iter().find(|r| r.id == 60).unwrap();
    assert_eq!((review.state.as_str(), review.comments.iter().map(|c| c.id).collect::<Vec<_>>()), ("COMMENT", vec![62]));
}

/// The repository deletes the branch after merging: GitVolt asks for it, since Forgejo's merge
/// API doesn't apply the setting itself. Auto-merge is scheduled, read back and cancelled.
#[tokio::test(flavor = "multi_thread")]
async fn merges_delete_the_branch_and_auto_merge_is_scheduled_and_cancelled() {
    let f = FakeForge::start().await;
    let p = provider(&f);
    let g = gadget(&p).await;
    let mr = p.merge(&g, DEV_PR, &MergeOptions { expected_sha: Some("d".repeat(40)), ..Default::default() }).await.unwrap();
    assert_eq!(mr.state, MrState::Merged);
    let merged = pull(&f, DEV_PR);
    assert_eq!((merged.merged_with.as_str(), merged.branch_deleted), ("merge", true));

    // #5's checks are pending: scheduled, not merged.
    let scheduled = p.set_auto_merge(&g, 5, &MergeOptions::default()).await.unwrap();
    assert_eq!(scheduled.auto_merge.and_then(|a| a.enabled_by).map(|u| u.username), Some("ada".into()));
    assert!(!pull(&f, 5).merged);
    let cancelled = p.cancel_auto_merge(&g, 5).await.unwrap();
    assert!(cancelled.auto_merge.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_resolves_and_unresolves_through_its_first_comment() {
    let f = FakeForge::start().await;
    let p = provider(&f);
    let g = gadget(&p).await;
    assert_eq!(p.resolve(&g, DEV_PR, "thread-42", true).await.unwrap(), ThreadState { resolved: true, resolved_by: Some("ada".into()) });
    assert!(p.discussions(&g, DEV_PR).await.unwrap().value.iter().find(|d| d.id == "thread-42").unwrap().resolved);
    assert_eq!(p.resolve(&g, DEV_PR, "thread-42", false).await.unwrap(), ThreadState { resolved: false, resolved_by: None });

    // A Forgejo before forgejo/forgejo#14873: said.
    let mut seed = f.current_seed();
    seed.forgejo.no_resolve_api = true;
    f.seed(seed);
    let e = p.resolve(&g, DEV_PR, "thread-42", true).await.unwrap_err();
    assert!(e.message.contains("older than its API"), "{}", e.message);
}
