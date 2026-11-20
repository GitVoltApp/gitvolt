//! A graph read's reply as a change to the payload the UI already holds: the rows as splices
//! (inserted and removed runs, by position in the old rows) and the rows kept in place whose
//! content changed, with everything else in the payload (labels, head, lanes, the view) sent
//! whole. Computed as `diff(old, new)` of two full payloads, so `apply(old, diff(old, new))` is
//! `new` whatever changed between them.

use crate::payload::{GraphPayload, RowPayload};
use serde::Serialize;
use std::collections::HashMap;
use ts_rs::TS;

/// A `graph` read's reply when the UI names the version it holds (`since`).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum GraphReply {
    /// The whole payload: the first read, or `since` is unknown or too old.
    Full(GraphFull),
    /// `since`'s payload, changed into this version's.
    Delta(GraphDelta),
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GraphFull {
    pub version: u32,
    pub graph: GraphPayload,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GraphDelta {
    pub version: u32,
    /// The version it changes.
    pub since: u32,
    /// How many rows the new payload has.
    pub rows: u32,
    /// In order, none overlapping: the old rows between two splices are kept.
    pub splices: Vec<RowSplice>,
    /// Kept rows whose content changed (a line through them moved, a new count), by their
    /// position in the new rows.
    pub patched: Vec<RowPatch>,
    /// The new payload but its rows (empty).
    pub rest: GraphPayload,
}

/// `remove` old rows from old row `at` on, and `insert` these in their place.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RowSplice {
    pub at: u32,
    pub remove: u32,
    pub insert: Vec<RowPayload>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RowPatch {
    pub at: u32,
    pub row: RowPayload,
}

/// `new` as a change to `old` (version `since`).
pub fn diff(old: &GraphPayload, new: &GraphPayload, since: u32, version: u32) -> GraphDelta {
    let at: HashMap<&str, usize> = old.rows.iter().enumerate().map(|(i, r)| (r.id.as_str(), i)).collect();
    let matched: Vec<Option<usize>> = new.rows.iter().map(|r| at.get(r.id.as_str()).copied()).collect();
    let (mut splices, mut patched) = (vec![], vec![]);
    // Between two kept rows (and before the first, after the last): one splice, if anything there
    // changed.
    let (mut i0, mut j0) = (0, 0);
    let mut gap = |i: usize, j: usize, i0: usize, j0: usize| {
        if i > i0 || j > j0 {
            splices.push(RowSplice { at: i0 as u32, remove: (i - i0) as u32, insert: new.rows[j0..j].to_vec() });
        }
    };
    for (j, i) in kept(&matched) {
        gap(i, j, i0, j0);
        if old.rows[i] != new.rows[j] {
            patched.push(RowPatch { at: j as u32, row: new.rows[j].clone() });
        }
        (i0, j0) = (i + 1, j + 1);
    }
    gap(old.rows.len(), new.rows.len(), i0, j0);
    GraphDelta { version, since, rows: new.rows.len() as u32, splices, patched, rest: without_rows(new) }
}

/// The rows that stay, as (new position, old position): the longest run of rows in both whose
/// order is the same (every row, when rows only came or went).
fn kept(matched: &[Option<usize>]) -> Vec<(usize, usize)> {
    // `tails[k]`: the row ending the run of length k + 1 that ends lowest in the old rows.
    let mut tails: Vec<usize> = vec![];
    let mut prev = vec![usize::MAX; matched.len()];
    for (j, m) in matched.iter().enumerate() {
        let Some(i) = *m else { continue };
        let k = tails.partition_point(|&t| matched[t].is_some_and(|ti| ti < i));
        if k > 0 {
            prev[j] = tails[k - 1];
        }
        if k == tails.len() {
            tails.push(j);
        } else {
            tails[k] = j;
        }
    }
    let mut out = vec![];
    let mut j = tails.last().copied();
    while let Some(x) = j {
        out.push((x, matched[x].expect("a kept row is matched")));
        j = (prev[x] != usize::MAX).then_some(prev[x]);
    }
    out.reverse();
    out
}

/// Every field but the rows. Spelled out, so a new field can't be left out.
fn without_rows(g: &GraphPayload) -> GraphPayload {
    let GraphPayload { rows: _, labels, max_lanes, pinned_refs, head, truncated, open_worktree, worktrees, in_progress, fork, view } = g;
    GraphPayload {
        rows: vec![],
        labels: labels.clone(),
        max_lanes: *max_lanes,
        pinned_refs: pinned_refs.clone(),
        head: head.clone(),
        truncated: *truncated,
        open_worktree: open_worktree.clone(),
        worktrees: worktrees.clone(),
        in_progress: in_progress.clone(),
        fork: fork.clone(),
        view: view.clone(),
    }
}

/// `old` changed by `delta`; `None` when `delta` doesn't fit `old` (not the payload it was made
/// against).
pub fn apply(old: &GraphPayload, delta: &GraphDelta) -> Option<GraphPayload> {
    let mut rows = Vec::with_capacity(delta.rows as usize);
    let mut i = 0;
    for s in &delta.splices {
        let (at, end) = (s.at as usize, s.at as usize + s.remove as usize);
        if at < i || end > old.rows.len() {
            return None;
        }
        rows.extend_from_slice(&old.rows[i..at]);
        rows.extend_from_slice(&s.insert);
        i = end;
    }
    rows.extend_from_slice(&old.rows[i..]);
    if rows.len() != delta.rows as usize {
        return None;
    }
    for p in &delta.patched {
        let r = rows.get_mut(p.at as usize).filter(|r| r.id == p.row.id)?;
        *r = p.row.clone();
    }
    Some(GraphPayload { rows, ..without_rows(&delta.rest) })
}

#[cfg(test)]
mod props;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{commit, local, request, worktree};
    use crate::build;
    use crate::layout::{GraphRow, NodeKind};
    use crate::walk::CommitMeta;

    fn json(g: &GraphPayload) -> serde_json::Value {
        serde_json::to_value(g).unwrap()
    }

    #[test]
    fn a_commit_on_top_is_one_inserted_row_and_its_parent_patched() {
        let history = [commit(1, &[], 100, "ada@example.com", "Initial"), commit(2, &[1], 200, "ada@example.com", "Second"), commit(3, &[2], 300, "ada@example.com", "Third")];
        let (req, commits) = request([local("main", 2)], [worktree("/demo/x", 2, "main", true)], history.clone(), 10);
        let old = build(&req, &commits).unwrap();
        let (req, commits) = request([local("main", 3)], [worktree("/demo/x", 3, "main", true)], history, 10);
        let new = build(&req, &commits).unwrap();
        let d = diff(&old, &new, 1, 2);
        assert_eq!((d.since, d.version, d.rows), (1, 2, 3));
        assert_eq!(d.splices.len(), 1);
        assert_eq!((d.splices[0].at, d.splices[0].remove, d.splices[0].insert.len()), (0, 0, 1));
        assert_eq!(d.splices[0].insert[0].summary, "Third");
        // The old tip gains the line from above: patched; the root is as it was.
        assert_eq!(d.patched.iter().map(|p| (p.at, p.row.summary.as_str())).collect::<Vec<_>>(), [(1, "Second")]);
        assert!(d.rest.rows.is_empty());
        assert_eq!(json(&apply(&old, &d).unwrap()), json(&new));
    }

    #[test]
    fn an_equal_payload_is_an_empty_delta() {
        let (req, commits) = request([local("main", 2)], [worktree("/demo/x", 2, "main", true)], [commit(1, &[], 100, "ada@example.com", "Initial"), commit(2, &[1], 200, "ada@example.com", "Second")], 10);
        let g = build(&req, &commits).unwrap();
        let d = diff(&g, &g, 4, 5);
        assert!(d.splices.is_empty() && d.patched.is_empty());
        assert_eq!(json(&apply(&g, &d).unwrap()), json(&g));
    }

    #[test]
    fn rows_moving_past_each_other_are_removed_and_inserted() {
        let (old, new) = (with(&["a", "b", "c", "d", "e"]), with(&["x", "d", "b", "c", "e", "y"]));
        let d = diff(&old, &new, 1, 2);
        // b, c and e stay (the longest run in order); a and d go, x, d and y come.
        let kept = old.rows.len() - d.splices.iter().map(|s| s.remove as usize).sum::<usize>();
        assert_eq!(kept, 3);
        assert_eq!(json(&apply(&old, &d).unwrap()), json(&new));
    }

    #[test]
    fn a_delta_against_another_payload_is_refused() {
        let d = diff(&with(&["a", "b"]), &with(&["a", "c"]), 1, 2);
        assert!(apply(&with(&["a"]), &d).is_none(), "too short");
        let mut patched = diff(&with(&["a", "b"]), &with(&["a", "b"]), 1, 2);
        patched.patched.push(RowPatch { at: 1, row: row("z") });
        assert!(apply(&with(&["a", "b"]), &patched).is_none(), "a patch names its row");
    }

    fn row(id: &str) -> RowPayload {
        let c = CommitMeta::from_message(crate::Oid::NULL, vec![], "Mara Quill".into(), "mara@example.com".into(), 100, 100, id);
        RowPayload { id: id.into(), ..RowPayload::commit(&c, NodeKind::Commit, &GraphRow { lane: 0, color: 0, segments: vec![], stub: None, tail: None }, vec![]) }
    }

    fn with(ids: &[&str]) -> GraphPayload {
        GraphPayload { rows: ids.iter().map(|i| row(i)).collect(), ..empty() }
    }

    pub(super) fn empty() -> GraphPayload {
        GraphPayload {
            rows: vec![],
            labels: vec![],
            max_lanes: 1,
            pinned_refs: vec![],
            head: Default::default(),
            truncated: false,
            open_worktree: None,
            worktrees: vec![],
            in_progress: Default::default(),
            fork: None,
            view: None,
        }
    }
}
