//! Builders for the presets' and the view transform's tests: commits, refs and worktrees by
//! number, a whole request, and reach sets by hand (so the view's tests don't need the rule
//! engine's pass).

use crate::input::{GraphInput, GraphRequest, HeadIn, RefIn, RefKind, RefsIn, WorktreeIn};
use crate::presets::ReachSets;
use crate::walk::CommitMeta;
use crate::Oid;
use std::collections::HashMap;

pub(crate) fn id(n: u8) -> Oid {
    Oid([n; 20])
}

/// Authored and committed at `time` by `email`, whose name is the part before the `@`.
pub(crate) fn commit(n: u8, parents: &[u8], time: i64, email: &str, message: &str) -> CommitMeta {
    let name = email.split('@').next().unwrap_or(email).to_string();
    CommitMeta::from_message(id(n), parents.iter().map(|&p| id(p)).collect(), name, email.into(), time, time, message)
}

pub(crate) fn local(name: &str, n: u8) -> RefIn {
    RefIn { full_name: format!("refs/heads/{name}"), short_name: name.into(), kind: RefKind::Local, target: id(n), upstream: None, upstream_mismatch: None, annotation: None }
}

pub(crate) fn remote(remote: &str, name: &str, n: u8) -> RefIn {
    RefIn { full_name: format!("refs/remotes/{remote}/{name}"), short_name: format!("{remote}/{name}"), kind: RefKind::Remote { remote: remote.into() }, ..local(name, n) }
}

pub(crate) fn tag(name: &str, n: u8) -> RefIn {
    RefIn { full_name: format!("refs/tags/{name}"), short_name: name.into(), kind: RefKind::Tag, ..local(name, n) }
}

/// `branch`: the short name of the branch checked out there.
pub(crate) fn worktree(path: &str, head: u8, branch: &str, is_main: bool) -> WorktreeIn {
    WorktreeIn { path: path.into(), canonical: path.into(), head: Some(id(head)), branch: Some(format!("refs/heads/{branch}")), is_main, bare: false, prunable: false, locked: false }
}

/// A request over `refs` and `worktrees`, built in the main worktree (HEAD is its; `/demo/x`
/// without one), and the store of `commits`.
pub(crate) fn request(refs: impl IntoIterator<Item = RefIn>, worktrees: impl IntoIterator<Item = WorktreeIn>, commits: impl IntoIterator<Item = CommitMeta>, limit: usize) -> (GraphRequest, HashMap<Oid, CommitMeta>) {
    let worktrees: Vec<WorktreeIn> = worktrees.into_iter().collect();
    let main = worktrees.iter().find(|w| w.is_main);
    let head = main.map(|w| HeadIn { branch: w.branch.clone(), target: w.head, detached: w.branch.is_none(), unborn: w.head.is_none() }).unwrap_or_default();
    let workdir = main.map_or_else(|| "/demo/x".to_string(), |w| w.canonical.clone());
    let input = GraphInput { refs: RefsIn { head, refs: refs.into_iter().collect(), ..Default::default() }, worktrees, workdir, ..Default::default() };
    (GraphRequest { input, limit }, commits.into_iter().map(|c| (c.id, c)).collect())
}

/// Reach sets by hand: per commit (walk index), the units reaching it; per commit, whether a
/// trunk branch does (its units are then dropped, as the real pass stores none).
pub(crate) fn reach(per_commit: &[&[usize]], trunk: &[bool]) -> ReachSets {
    let words = per_commit.iter().flat_map(|u| u.iter()).max().map_or(0, |&m| m / 64 + 1);
    let bits = per_commit
        .iter()
        .enumerate()
        .map(|(c, units)| {
            if trunk.get(c).copied().unwrap_or(false) {
                return Vec::new();
            }
            let mut w = vec![0u64; words];
            for &u in *units {
                w[u / 64] |= 1 << (u % 64);
            }
            w
        })
        .collect();
    ReachSets::new(bits, trunk.to_vec())
}

#[test]
fn reach_sets_built_by_hand_answer_per_commit() {
    let r = reach(&[&[0, 70], &[3], &[1]], &[false, false, true]);
    assert_eq!(r.units_reaching(0).collect::<Vec<_>>(), [0, 70]);
    assert_eq!(r.units_reaching(1).collect::<Vec<_>>(), [3]);
    assert_eq!(r.units_reaching(2).count(), 0, "a trunk-reachable commit stores no units");
    assert!(r.trunk_reachable(2) && !r.trunk_reachable(0));
    assert_eq!(r.units_reaching(9).count(), 0, "out of range: nothing");
}

#[test]
fn a_request_is_built_in_its_main_worktree() {
    let (req, commits) = request([local("main", 2), remote("origin", "main", 2), tag("v1", 1)], [worktree("/demo/x", 2, "main", true)], [commit(1, &[], 100, "ada@example.com", "Initial commit"), commit(2, &[1], 200, "ada@example.com", "Second")], 10);
    assert_eq!((req.input.workdir.as_str(), req.input.refs.head.branch.as_deref(), req.input.refs.head.target), ("/demo/x", Some("refs/heads/main"), Some(id(2))));
    assert_eq!(req.input.refs.refs[1].short_name, "origin/main");
    assert_eq!(commits[&id(1)].author_name, "ada");
    assert_eq!(crate::build(&req, &commits).unwrap().rows.len(), 2);
}
