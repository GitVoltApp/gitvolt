//! A guard against quadratic preset views: a large window with many branches lays out in a
//! bounded time even in a debug build. It catches blowups, not micro-regressions.

use crate::presets::{Matcher, PresetAction, Rule};
use crate::testkit::{local, worktree};
use crate::view::ViewSpec;
use crate::walk::{walk, CommitMeta, WalkOptions};
use crate::{assemble, graph_tips, preview, GraphInput, Oid, RefIn};
use std::time::{Duration, Instant};

const UNITS: usize = 1000;
/// Commits per branch, and trunk commits between two forks.
const BRANCH_LEN: usize = 12;
const TRUNK_STEP: usize = 8;

fn oid(n: usize) -> Oid {
    let mut b = [0u8; 20];
    b[..8].copy_from_slice(&(n as u64 + 1).to_be_bytes());
    Oid(b)
}

/// Every branch forks from the trunk's tip of its time and holds `BRANCH_LEN` commits; the trunk
/// then moves on by `TRUNK_STEP` commits, the merge of every second branch among them (a trunk
/// of merges). Branch `i` is `merged/i` (even `i`), `hide/i` or `keep/i` (odd `i`, alternating),
/// so a half is merged, a quarter matches the hide rule and a quarter falls to Everything else.
fn window() -> (GraphInput, Vec<CommitMeta>) {
    let mut commits: Vec<CommitMeta> = Vec::new();
    let mut add = |parents: Vec<Oid>, email: &str, message: &str| {
        let n = commits.len();
        let t = 1_000_000 + n as i64 * 60;
        let name = email.split('@').next().unwrap_or(email).to_string();
        commits.push(CommitMeta::from_message(oid(n), parents, name, email.into(), t, t, message));
        oid(n)
    };
    let mut trunk = add(vec![], "root@example.com", "Initial commit");
    let mut refs: Vec<RefIn> = Vec::new();
    for i in 0..UNITS {
        let name = match i % 4 {
            0 | 2 => format!("merged/{i}"),
            1 => format!("hide/{i}"),
            _ => format!("keep/{i}"),
        };
        let author = format!("dev{}@example.com", i % 37);
        let mut tip = trunk;
        for k in 0..BRANCH_LEN {
            tip = add(vec![tip], &author, &format!("{name}: step {k}"));
        }
        refs.push(RefIn { target: tip, ..local(&name, 0) });
        for k in 0..TRUNK_STEP {
            trunk = if k == 0 && i % 2 == 0 { add(vec![trunk, tip], "lead@example.com", &format!("Merge branch '{name}'")) } else { add(vec![trunk], "lead@example.com", &format!("Trunk {i}.{k}")) };
        }
    }
    refs.push(RefIn { target: trunk, ..local("main", 0) });
    let mut wt = worktree("/demo/x", 0, "main", true);
    wt.head = Some(trunk);
    let input = GraphInput { refs: crate::RefsIn { refs, head: crate::HeadIn { branch: wt.branch.clone(), target: Some(trunk), detached: false, unborn: false }, ..Default::default() }, worktrees: vec![wt], workdir: "/demo/x".into(), ..Default::default() };
    commits.reverse();
    (input, commits)
}

#[test]
fn a_large_window_with_many_branches_stays_linear() {
    let (mut input, commits) = window();
    let total = commits.len();
    assert!(total >= 20_000, "{total}");
    let store: std::collections::HashMap<Oid, CommitMeta> = commits.into_iter().map(|c| (c.id, c)).collect();
    let tips = graph_tips(&input.refs, &input.worktrees);
    let walked = walk(&tips, &WalkOptions { limit: total, ..Default::default() }, |id| Ok::<_, ()>(store.get(&id).cloned())).unwrap();
    assert_eq!(walked.commits.len(), total);

    let started = Instant::now();
    let plain = assemble(&input, &walked).unwrap();
    let plain_time = started.elapsed();

    let rules = vec![Rule { action: PresetAction::Show, matcher: Matcher::Trunk }, Rule { action: PresetAction::Hide, matcher: Matcher::NameMatches { patterns: "hide/*".into() } }];
    input.view = Some(ViewSpec { rules, otherwise: PresetAction::Collapse, fold_merged: true, ghost_lines: true, ..Default::default() });
    let started = Instant::now();
    let g = assemble(&input, &walked).unwrap();
    let view_time = started.elapsed();
    let started = Instant::now();
    let p = preview(&input, &walked);
    let preview_time = started.elapsed();
    eprintln!("{total} commits, {UNITS} branches: plain {plain_time:?}, view {view_time:?}, preview {preview_time:?}");

    // The view did its work: folds, runs and ghosts, on far fewer rows.
    assert_eq!((p.folded, p.hidden, p.collapsed), (UNITS as u32 / 2, UNITS as u32 / 4, UNITS as u32 / 4), "{p:?}");
    assert_eq!(g.rows.iter().filter(|r| r.collapsed.is_some()).count(), UNITS / 4);
    assert!(g.rows.iter().any(|r| !r.ghosts.is_empty()));
    assert!(g.rows.len() < plain.rows.len() / 2, "{} of {}", g.rows.len(), plain.rows.len());

    assert!(view_time < Duration::from_secs(2), "assemble with a view took {view_time:?}");
    assert!(preview_time < Duration::from_secs(1), "preview took {preview_time:?}");
}
