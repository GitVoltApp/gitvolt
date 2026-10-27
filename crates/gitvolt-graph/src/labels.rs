//! The ref labels (chips) on the graph's rows (§8.5), moved from the core's snapshot.rs.

use crate::input::{HeadIn, RefIn, RefKind, RefsIn, WorktreeIn};
use crate::payload::{HostKind, RefLabel, RemoteRefLabel};
use crate::Oid;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// `here`: the canonical open (active) worktree; `head`: its HEAD; `hidden`: the full names of the
/// refs a preset hides (branches, and tags hidden by hand), which have no chip even on a commit
/// that is a row.
pub(crate) fn build_labels(refs: &RefsIn, head: &HeadIn, worktrees: &[WorktreeIn], here: &str, index: &HashMap<Oid, usize>, row_of_commit: &[u32], hidden: &HashSet<&str>) -> Vec<RefLabel> {
    let checked_out_elsewhere: HashMap<&str, String> = worktrees.iter().filter(|w| w.canonical != here).filter_map(|w| w.branch.as_deref().map(|b| (b, w.path.clone()))).collect();
    let checked_out: HashMap<&str, String> = worktrees.iter().filter_map(|w| w.branch.as_deref().map(|b| (b, w.path.clone()))).collect();
    // A commit a preset hides, folds or collapses has no row, and no chips.
    let row_of = |id: &Oid| index.get(id).map(|&i| row_of_commit[i]).filter(|&r| r != crate::assemble::NO_ROW);
    let host_name = |remote: &str| refs.remote_host_names.get(remote).cloned();
    let host = |remote: &str| refs.remote_hosts.get(remote).copied().unwrap_or(HostKind::Generic);

    let mut labels = Vec::new();
    let mut merged: HashSet<&str> = HashSet::new();
    let shown = |r: &&RefIn| !hidden.contains(r.full_name.as_str());
    for local in refs.refs.iter().filter(|r| r.kind == RefKind::Local).filter(shown) {
        let Some(row) = row_of(&local.target) else { continue };
        let mut remotes = Vec::new();
        for rr in refs.refs.iter().filter(shown) {
            let RefKind::Remote { remote } = &rr.kind else { continue };
            let branch_part = &rr.short_name[remote.len() + 1..];
            let is_counterpart = branch_part == local.short_name || local.upstream.as_deref() == Some(rr.full_name.as_str());
            if rr.target == local.target && is_counterpart {
                remotes.push(RemoteRefLabel { full_name: rr.full_name.clone(), remote: remote.clone(), host: host_name(remote), host_kind: host(remote) });
                merged.insert(rr.full_name.as_str());
            }
        }
        labels.push(RefLabel {
            row,
            name: local.short_name.clone(),
            local: Some(local.full_name.clone()),
            remotes,
            tag: false,
            is_head: head.branch.as_deref() == Some(local.full_name.as_str()),
            worktree: checked_out_elsewhere.get(local.full_name.as_str()).cloned(),
            checked_out: checked_out.get(local.full_name.as_str()).cloned(),
            upstream_mismatch: local.upstream_mismatch.clone(),
            annotation: None,
        });
    }
    // Remote-only refs: one label per (commit, branch name), so the same branch on several
    // remotes at the same commit is ONE chip with one icon per remote (§8.5).
    let mut remote_only: HashMap<(Oid, &str), usize> = HashMap::new();
    for rr in refs.refs.iter().filter(shown) {
        let RefKind::Remote { remote } = &rr.kind else { continue };
        if merged.contains(rr.full_name.as_str()) {
            continue;
        }
        let Some(row) = row_of(&rr.target) else { continue };
        // Only the branch part (`p/janderson/foo`, not `origin/p/janderson/foo`): the remote
        // icons already mark it as remote, and `remotes[].full_name` keeps each full name for
        // the tooltip.
        let branch = &rr.short_name[remote.len() + 1..];
        let remote_label = RemoteRefLabel { full_name: rr.full_name.clone(), remote: remote.clone(), host: host_name(remote), host_kind: host(remote) };
        match remote_only.get(&(rr.target, branch)) {
            Some(&i) => labels[i].remotes.push(remote_label),
            None => {
                remote_only.insert((rr.target, branch), labels.len());
                labels.push(RefLabel { row, name: branch.to_string(), local: None, remotes: vec![remote_label], tag: false, is_head: false, worktree: None, checked_out: None, upstream_mismatch: None, annotation: None });
            }
        }
    }
    for t in refs.refs.iter().filter(|r| r.kind == RefKind::Tag).filter(shown) {
        if let Some(row) = row_of(&t.target) {
            labels.push(RefLabel { row, name: t.short_name.clone(), local: None, remotes: vec![], tag: true, is_head: false, worktree: None, checked_out: None, upstream_mismatch: None, annotation: t.annotation.clone() });
        }
    }
    // A worktree whose HEAD is detached has no branch chip to carry its icon: it gets a chip of
    // its own at its HEAD, named by its folder (as the sidebar names it), so the commits only it
    // reaches say why they're in the graph. `checked_out` is its path, `worktree` too when it isn't
    // the open one. With the main worktree alone there's nothing to tell apart: plain `HEAD`.
    let usable: Vec<&WorktreeIn> = worktrees.iter().filter(|w| !w.bare && !w.prunable).collect();
    let mut head_labelled = false;
    if usable.len() > 1 {
        for w in usable.iter().filter(|w| w.branch.is_none()) {
            let Some(row) = w.head.as_ref().and_then(row_of) else { continue };
            let is_here = w.canonical == here;
            head_labelled |= is_here;
            let path = Some(w.path.clone());
            labels.push(RefLabel { row, name: worktree_name(&w.path), local: None, remotes: vec![], tag: false, is_head: is_here, worktree: if is_here { None } else { path.clone() }, checked_out: path, upstream_mismatch: None, annotation: None });
        }
    }
    if head.detached
        && !head_labelled
        && let Some(row) = head.target.as_ref().and_then(row_of)
    {
        labels.push(RefLabel { row, name: "HEAD".into(), local: None, remotes: vec![], tag: false, is_head: true, worktree: None, checked_out: None, upstream_mismatch: None, annotation: None });
    }
    // HEAD's branch, then locals: a published one (its remote at the same commit, `main` with
    // `origin/main`) before a local-only one, and one checked out in another worktree (a scratch
    // branch) last, then other worktrees' detached HEADs; then remote-only branches, then tags.
    // Ties keep the refs' order (the worktrees' for detached HEADs).
    let priority = |l: &RefLabel| match () {
        _ if l.is_head => (0, 0),
        _ if l.local.is_some() => (1, if l.worktree.is_some() { 2 } else if l.remotes.is_empty() { 1 } else { 0 }),
        _ if l.checked_out.is_some() => (1, 3),
        _ if !l.tag => (2, 0),
        _ => (3, 0),
    };
    labels.sort_by_key(|l| (l.row, priority(l)));
    labels
}

/// A worktree's name in the graph: its folder's (the sidebar's and the WIP row's).
pub(crate) fn worktree_name(path: &str) -> String {
    Path::new(path).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
}
