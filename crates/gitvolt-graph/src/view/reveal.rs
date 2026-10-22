//! The reveal panel's rows: a collapsed run or a folded merge laid out on the graph's own lanes.
//!
//! The graph is laid out again (stateless, on the same walk), then the panel's rows go through the
//! same `LayoutState` with the lanes it shares with the graph pinned to the graph's: the run's
//! line to its ⋮ row's lane, its parent to that commit's lane; the trunk to the merge's lane and
//! the merged branch to the first lane free beside it. Every lane left of those is reserved, so
//! whatever else the panel draws (branches merged into it) opens further right.

use crate::assemble::assemble_view;
use crate::input::GraphInput;
use crate::layout::{Half, LayoutNode, LayoutState, NodeKind, Parent, Segment};
use crate::payload::{RevealPayload, RevealTag, RowPayload};
use crate::presets::BranchUnit;
use crate::view::{branch_key, run_key, window_index, CommitState, ViewRows};
use crate::walk::{CommitMeta, WalkResult};
use crate::Oid;
use std::collections::{HashMap, HashSet};

/// The panel for `key` (a run's `"branch:<unit>@<newest sha>"`, or `"fold:<merge sha>"`) under
/// `input.view`, opening `below` the row or above it (a fold's always opens below). A unit's own
/// `"branch:<unit>"` reveals its first run (the newest). `None` for an unknown or stale key.
pub fn reveal(input: &GraphInput, walked: &WalkResult, key: &str, below: bool) -> Option<RevealPayload> {
    input.view.as_ref()?;
    let (graph, view) = assemble_view(input, walked).ok()?;
    let view = view?;
    let cx = Cx {
        commits: &walked.commits,
        index: window_index(&walked.commits),
        stashes: input.refs.stashes.iter().map(|s| s.id).collect(),
        view: &view,
        rows: graph.rows.iter().map(|r| (r.id.as_str(), r)).collect(),
        dots: graph.rows.iter().filter_map(|r| Some((r.collapsed.as_ref()?.key.as_str(), r))).collect(),
    };
    if let Some((name, newest)) = branch_key(key) {
        cx.run_panel(name, newest, below)
    } else {
        cx.fold_panel(key, key.strip_prefix("fold:")?)
    }
}

/// What a panel lays out: its rows (walk indexes, in order), the rows pinned to a graph lane, the
/// lanes reserved (`0..width`), and the lines running in from above (lane, the row they reach).
struct Panel {
    rows: Vec<usize>,
    pinned: HashMap<usize, u16>,
    width: u16,
    enter: Vec<(u16, usize)>,
    /// Trunk rows: only their own line runs on; what else they merged isn't the panel's.
    trunk: HashSet<usize>,
}

struct Cx<'a> {
    commits: &'a [CommitMeta],
    index: HashMap<Oid, usize>,
    stashes: HashSet<Oid>,
    view: &'a ViewRows,
    /// The graph's rows by id.
    rows: HashMap<&'a str, &'a RowPayload>,
    /// The graph's ⋮ rows by key.
    dots: HashMap<&'a str, &'a RowPayload>,
}

impl Cx<'_> {
    fn first_parent(&self, c: usize) -> Option<usize> {
        self.commits[c].parents.first().and_then(|p| self.index.get(p)).copied()
    }

    /// The graph row standing for a loaded commit: its own, or its run's ⋮ row.
    fn graph_row(&self, c: usize) -> Option<&RowPayload> {
        match self.view.plan.state[c] {
            CommitState::Visible => self.rows.get(self.commits[c].id.to_string().as_str()).copied(),
            CommitState::Run { run } => self.dots.get(self.run_key(run).as_str()).copied(),
            CommitState::Folded { .. } | CommitState::Hidden => None,
        }
    }

    fn run_key(&self, run: usize) -> String {
        let r = &self.view.plan.runs[run];
        run_key(&self.view.d.units[r.unit].name, &self.commits[r.newest].id)
    }

    /// Pins `from`'s first-parent chain, while it stays among `members`, to `lane`.
    fn pin_chain(&self, from: Option<usize>, members: &HashSet<usize>, lane: u16, pinned: &mut HashMap<usize, u16>) {
        let mut cur = from;
        while let Some(c) = cur.filter(|c| members.contains(c) && !pinned.contains_key(c)) {
            pinned.insert(c, lane);
            cur = self.first_parent(c);
        }
    }

    /// A collapsed run's panel: its middle commits, newest first; below the ⋮ row, then its oldest
    /// commit and that one's parent (tagged "parent"). The run's line keeps the ⋮ row's lane: above,
    /// it runs off the bottom into the ⋮ row; below, it comes in from the top, and so does the
    /// parent's lane when the graph has it running past the ⋮ row.
    fn run_panel(&self, name: &str, newest: Option<Oid>, below: bool) -> Option<RevealPayload> {
        let i = self.view.plan.runs.iter().position(|r| self.view.d.units[r.unit].name == name && newest.is_none_or(|n| self.commits[r.newest].id == n))?;
        let run = &self.view.plan.runs[i];
        let key = self.run_key(i);
        let dots = *self.dots.get(key.as_str())?;
        let l = dots.lane;
        let parent = self.first_parent(run.oldest);
        let p_lane = parent.and_then(|p| self.graph_row(p)).map(|r| r.lane);

        let mut rows = run.middle.clone();
        let mut tags = vec![];
        if below {
            rows.push(run.oldest);
            if let Some(p) = parent {
                tags.push(RevealTag { row: rows.len() as u32, text: "parent".into() });
                rows.push(p);
            }
        }
        // The run's own line: the parent keeps the lane it has in the graph.
        let members: HashSet<usize> = rows.iter().copied().filter(|&c| Some(c) != parent).collect();
        let start = self.first_parent(run.newest).filter(|c| members.contains(c)).or_else(|| run.middle.iter().copied().find(|&c| !self.stashes.contains(&self.commits[c].id)));
        let mut pinned = HashMap::new();
        self.pin_chain(start, &members, l, &mut pinned);

        let mut enter = vec![];
        if below {
            enter.extend(start.map(|s| (l, s)));
            if let (Some(p), Some(pl)) = (parent, p_lane)
                && (pl != l || pinned.contains_key(&run.oldest))
            {
                pinned.insert(p, pl);
                let runs_past = dots.segments.iter().map(|&s| Segment::unpack(s)).any(|s| match s.half {
                    Half::Full => s.from_lane == pl,
                    Half::Bottom => s.to_lane == pl,
                    Half::Top => false,
                });
                if pl != l && runs_past {
                    enter.push((pl, p));
                }
            }
        }
        let width = l.max(p_lane.unwrap_or(l)) + 1;
        let (rows, max_lanes) = self.lay_out(&Panel { rows, pinned, width, enter, trunk: HashSet::new() });
        Some(RevealPayload { key, branch: name.into(), count: run.middle.len() as u32, rows, tags, entry_lane: l, max_lanes })
    }

    /// A folded merge's panel, always below it: that stretch in full, in walk order. The trunk's
    /// commits from the merge's first parent down to the one the branch forked from (the last row)
    /// keep the merge's lane, tagged with the trunk's name; the merged branch takes the first lane
    /// free beside it at the merge's row; branches merged into it go further right. Both lanes come
    /// in from the top.
    fn fold_panel(&self, key: &str, sha: &str) -> Option<RevealPayload> {
        let merge = *self.index.get(&Oid::from_hex(sha)?)?;
        let fold = self.view.plan.folds.iter().find(|f| f.merge == merge)?;
        let row = *self.rows.get(self.commits[merge].id.to_string().as_str())?;
        let t = row.lane;
        let busy: HashSet<u16> = row.segments.iter().map(|&s| Segment::unpack(s)).flat_map(|s| [s.from_lane, s.to_lane]).collect();
        let b = (t + 1..u16::MAX).find(|l| !busy.contains(l))?;

        let folded: HashSet<usize> = fold.commits.iter().copied().collect();
        let outer: HashSet<usize> = fold.commits.iter().flat_map(|&c| self.commits[c].parents.iter().filter_map(|p| self.index.get(p).copied())).filter(|p| !folded.contains(p)).collect();
        let last = fold.commits.iter().copied().max()?;
        let bottom = outer.iter().copied().max().unwrap_or(last).max(last);
        let mut chain = vec![];
        let mut cur = self.first_parent(merge);
        while let Some(c) = cur.filter(|&c| c <= bottom) {
            chain.push(c);
            cur = self.first_parent(c);
        }
        // Down to the fork point; without one in the window, to the branch's oldest loaded commit.
        let end = chain.iter().rev().copied().find(|c| outer.contains(c));
        chain.retain(|&c| end.map_or(c < last, |f| c <= f));

        let mut rows: Vec<usize> = chain.iter().chain(&fold.commits).copied().collect();
        rows.sort_unstable();
        let mut pinned: HashMap<usize, u16> = chain.iter().map(|&c| (c, t)).collect();
        let second = self.commits[merge].parents.get(1).and_then(|p| self.index.get(p)).copied();
        let start = second.filter(|c| folded.contains(c)).or_else(|| fold.commits.iter().copied().find(|&c| !self.stashes.contains(&self.commits[c].id)));
        self.pin_chain(start, &folded, b, &mut pinned);
        let enter = chain.first().map(|&c| (t, c)).into_iter().chain(start.map(|s| (b, s))).collect();

        let trunk: HashSet<usize> = chain.into_iter().collect();
        let name = self.trunk_name(merge);
        let tags = rows.iter().enumerate().filter(|(_, c)| trunk.contains(c)).map(|(r, _)| RevealTag { row: r as u32, text: name.clone() }).collect();
        let (rows, max_lanes) = self.lay_out(&Panel { rows, pinned, width: b + 1, enter, trunk });
        Some(RevealPayload { key: key.into(), branch: fold.branch.clone(), count: fold.commits.len() as u32, rows, tags, entry_lane: b, max_lanes })
    }

    /// The display name of the trunk branch whose first-parent line holds `merge` (`origin/main`
    /// when only the remote one exists).
    fn trunk_name(&self, merge: usize) -> String {
        let on_line = |u: &&BranchUnit| {
            u.tips.iter().any(|t| {
                let mut cur = self.index.get(t).copied();
                while let Some(c) = cur.filter(|&c| c <= merge) {
                    if c == merge {
                        return true;
                    }
                    cur = self.first_parent(c);
                }
                false
            })
        };
        let d = &self.view.d;
        let unit = d.units.iter().zip(&d.trunk).filter(|(_, t)| **t).map(|(u, _)| u).find(on_line);
        match unit {
            Some(u) if u.has_local => u.name.clone(),
            Some(u) => u.refs.first().and_then(|r| r.strip_prefix("refs/remotes/")).unwrap_or(&u.name).to_string(),
            None => "trunk".into(),
        }
    }

    /// The panel's rows, laid out with its pins, reservation and lines from above.
    fn lay_out(&self, p: &Panel) -> (Vec<RowPayload>, u16) {
        let row_of: HashMap<usize, u32> = p.rows.iter().enumerate().map(|(r, &c)| (c, r as u32)).collect();
        let mut state = LayoutState::new(&vec![u32::MAX; usize::from(p.width)]);
        for (lane, c) in &p.enter {
            state.enter(*lane, row_of[c]);
        }
        let rows = p
            .rows
            .iter()
            .enumerate()
            .map(|(r, &ci)| {
                let c = &self.commits[ci];
                let mut parents = Vec::with_capacity(c.parents.len());
                for (k, id) in c.parents.iter().enumerate() {
                    let parent = match self.index.get(id).and_then(|i| row_of.get(i)) {
                        Some(&row) => Parent::Row(row),
                        None if k > 0 && p.trunk.contains(&ci) => continue,
                        None => Parent::Outside(*id),
                    };
                    if !parents.contains(&parent) {
                        parents.push(parent);
                    }
                }
                let kind = if self.stashes.contains(&c.id) {
                    NodeKind::Stash
                } else if c.parents.len() > 1 {
                    NodeKind::Merge
                } else {
                    NodeKind::Commit
                };
                let g = state.push(r as u32, &LayoutNode { parents, kind, pinned: p.pinned.get(&ci).copied(), time: c.committer_time });
                let segments = g.segments.iter().map(|s| s.pack()).collect();
                RowPayload::commit(c, kind, &g, segments)
            })
            .collect();
        (rows, state.max_lanes())
    }
}

#[cfg(test)]
mod tests {
    use crate::layout::{Half, Segment};
    use crate::presets::{Matcher, PresetAction, Rule};
    use crate::testkit::{commit, id, local, remote, request, worktree};
    use crate::view::ViewSpec;
    use crate::{build, build_reveal, CommitMeta, GraphPayload, GraphRequest, Oid, RefIn, RevealPayload, RevealTag, RowPayload};
    use std::collections::HashMap;

    fn c(n: u8, parents: &[u8], time: i64) -> CommitMeta {
        commit(n, parents, time, "ada@example.com", &format!("c{n}"))
    }

    fn collapse_x() -> ViewSpec {
        ViewSpec { rules: vec![Rule { action: PresetAction::Collapse, matcher: Matcher::NameMatches { patterns: "x".into() } }], otherwise: PresetAction::Show, ..ViewSpec::default() }
    }

    fn folding() -> ViewSpec {
        ViewSpec { otherwise: PresetAction::Show, fold_merged: true, ..ViewSpec::default() }
    }

    fn with(mut req: GraphRequest, spec: ViewSpec) -> GraphRequest {
        req.input.view = Some(spec);
        req
    }

    /// `base`, with `more` added or replacing the commit of the same id.
    fn amended(mut base: Vec<CommitMeta>, more: Vec<CommitMeta>) -> Vec<CommitMeta> {
        for m in more {
            base.retain(|x| x.id != m.id);
            base.push(m);
        }
        base
    }

    /// main: 1 ← 2 ← 3 at `main_time`; x from 2: 10..=15 (1000..=1500); `more`; x collapsed.
    fn run(main_time: i64, more: Vec<CommitMeta>) -> (GraphRequest, HashMap<Oid, CommitMeta>) {
        let mut cs = vec![c(1, &[], 100), c(2, &[1], 200), c(3, &[2], main_time)];
        cs.extend((10..=15).map(|n| c(n, &[if n == 10 { 2 } else { n - 1 }], i64::from(n) * 100)));
        let (req, commits) = request([local("main", 3), local("x", 15)], [worktree("/demo/x", 3, "main", true)], amended(cs, more), 100);
        (with(req, collapse_x()), commits)
    }

    fn ids(p: &RevealPayload) -> Vec<String> {
        p.rows.iter().map(|r| r.id.clone()).collect()
    }

    fn shas(ns: &[u8]) -> Vec<String> {
        ns.iter().map(|&n| id(n).to_string()).collect()
    }

    fn lanes(p: &RevealPayload) -> Vec<u16> {
        p.rows.iter().map(|r| r.lane).collect()
    }

    fn segments(p: &RevealPayload, row: usize) -> Vec<Segment> {
        p.rows[row].segments.iter().map(|&s| Segment::unpack(s)).collect()
    }

    fn graph_row<'a>(g: &'a GraphPayload, row_id: &str) -> &'a RowPayload {
        g.rows.iter().find(|r| r.id == row_id).unwrap()
    }

    #[test]
    fn the_panel_above_lines_up_with_the_dotted_lane() {
        let (req, commits) = run(2000, vec![]);
        let g = build(&req, &commits).unwrap();
        let dots = graph_row(&g, &format!("collapsed:x@{}", id(15)));
        let l = dots.lane;
        assert_eq!(l, 1);
        let key = format!("branch:x@{}", id(15));
        let p = build_reveal(&req, &commits, &key, false).unwrap();
        assert_eq!((p.key.clone(), p.branch.as_str(), p.count, p.entry_lane, p.tags.clone()), (key.clone(), "x", 4, l, vec![]));
        assert_eq!(build_reveal(&req, &commits, "branch:x", false), Some(p.clone()), "the branch's own key: its first run");
        assert_eq!(ids(&p), shas(&[14, 13, 12, 11]));
        assert_eq!(lanes(&p), [l; 4]);
        assert!((0..4).flat_map(|r| segments(&p, r)).all(|s| s.from_lane >= l && s.to_lane >= l), "nothing left of the run's lane");
        assert!(segments(&p, 0).iter().all(|s| s.half != Half::Top), "nothing enters from above");
        assert!(segments(&p, 3).iter().any(|s| s.half == Half::Bottom && s.to_lane == l && !s.dotted), "it runs off the bottom into the ⋮ row");
        assert_eq!(p.rows[0].color, dots.color);
    }

    #[test]
    fn the_panel_below_ends_with_the_oldest_and_its_parent() {
        let (req, commits) = run(2000, vec![]);
        let g = build(&req, &commits).unwrap();
        let p = build_reveal(&req, &commits, "branch:x", true).unwrap();
        assert_eq!(ids(&p), shas(&[14, 13, 12, 11, 10, 2]));
        assert_eq!(p.tags, [RevealTag { row: 5, text: "parent".into() }]);
        assert_eq!(p.count, 4, "the hidden commits only");
        let parent_lane = graph_row(&g, &id(2).to_string()).lane;
        assert_eq!(lanes(&p), [1, 1, 1, 1, 1, parent_lane]);
        assert!(segments(&p, 0).iter().any(|s| s.half == Half::Top && s.from_lane == 1), "the run's lane enters at the top");
        // main runs past the ⋮ row in the graph, so its lane runs from the top to the parent.
        for r in 0..5 {
            assert!(segments(&p, r).iter().any(|s| s.half == Half::Full && s.from_lane == parent_lane), "row {r}");
        }
        let into_parent: Vec<u16> = segments(&p, 5).iter().filter(|s| s.half == Half::Top).map(|s| s.from_lane).collect();
        assert_eq!(into_parent, [0, 1]);
        // main's tip under the ⋮ row: no line in its lane above the parent.
        let (req, commits) = run(1250, vec![]);
        let p = build_reveal(&req, &commits, "branch:x", true).unwrap();
        assert_eq!(lanes(&p), [1, 1, 1, 1, 1, 0]);
        assert!((0..5).flat_map(|r| segments(&p, r)).all(|s| s.from_lane == 1 && s.to_lane == 1));
    }

    #[test]
    fn a_branch_merged_into_the_run_gets_its_own_lane_right_of_it() {
        // y (20, 21) from x's 11, merged into x at 14; its ref deleted.
        let (req, commits) = run(2000, vec![c(20, &[11], 1150), c(21, &[20], 1160), c(14, &[13, 21], 1400)]);
        let p = build_reveal(&req, &commits, "branch:x", false).unwrap();
        assert_eq!(ids(&p), shas(&[14, 13, 12, 21, 20, 11]));
        assert_eq!(lanes(&p), [1, 1, 1, 2, 2, 1]);
        assert!(segments(&p, 0).iter().any(|s| s.half == Half::Bottom && s.to_lane == 2), "the merge line opens right of the run");
        let p = build_reveal(&req, &commits, "branch:x", true).unwrap();
        assert_eq!(lanes(&p), [1, 1, 1, 2, 2, 1, 1, 0]);
    }

    #[test]
    fn a_stale_key_reveals_nothing() {
        let (req, commits) = run(2000, vec![]);
        for key in ["branch:nope", "branch:main", "fold:nope", &format!("fold:{}", id(3)), &format!("branch:x@{}", id(14)), "x", ""] {
            assert_eq!(build_reveal(&req, &commits, key, false), None, "{key}");
        }
        let mut plain = req.clone();
        plain.input.view = None;
        assert_eq!(build_reveal(&plain, &commits, "branch:x", false), None, "no view");
        let mut expanded = req;
        expanded.input.view = Some(ViewSpec { expanded: vec!["branch:x".into()], ..collapse_x() });
        assert_eq!(build_reveal(&expanded, &commits, "branch:x", false), None, "expanded: no run left");
    }

    #[test]
    fn each_run_of_a_diverged_branch_reveals_its_own_commits() {
        // x's shared commits 10..=15 from main's 2, then local x 20..=25 and origin/x 30..=35.
        let mut cs = vec![c(1, &[], 100), c(2, &[1], 200), c(3, &[2], 300)];
        for (first, from) in [(10, 2), (20, 15), (30, 15)] {
            cs.extend((first..first + 6).map(|n| c(n, &[if n == first { from } else { n - 1 }], i64::from(n) * 100)));
        }
        let (req, commits) = request([local("main", 3), local("x", 25), remote("origin", "x", 35)], [worktree("/demo/x", 3, "main", true)], cs, 100);
        let req = with(req, collapse_x());
        for (newest, middle) in [(35, [34, 33, 32, 31]), (25, [24, 23, 22, 21]), (15, [14, 13, 12, 11])] {
            let key = format!("branch:x@{}", id(newest));
            let p = build_reveal(&req, &commits, &key, false).unwrap();
            assert_eq!((p.key.as_str(), p.count), (key.as_str(), 4));
            assert_eq!(ids(&p), shas(&middle));
        }
    }

    /// main: 1 ← 2 ← 3 ← M(30, merging x's 12) ← 31; x: 10 (from 2), 11, 12, between main's
    /// commits; `more`, `refs`; merged branches folded.
    fn folded(more: Vec<CommitMeta>, refs: Vec<RefIn>, limit: usize) -> (GraphRequest, HashMap<Oid, CommitMeta>) {
        let mut cs = vec![c(1, &[], 100), c(2, &[1], 200), c(10, &[2], 250), c(3, &[2], 300), c(11, &[10], 350), c(12, &[11], 400)];
        cs.push(commit(30, &[3, 12], 3000, "ada@example.com", "Merge branch 'x'"));
        cs.push(c(31, &[30], 3100));
        let (req, commits) = request([local("main", 31)].into_iter().chain(refs), [worktree("/demo/x", 31, "main", true)], amended(cs, more), limit);
        (with(req, folding()), commits)
    }

    fn fold_key() -> String {
        format!("fold:{}", id(30))
    }

    #[test]
    fn a_fold_panel_draws_trunk_and_branch_side_by_side() {
        let (req, commits) = folded(vec![], vec![], 100);
        let g = build(&req, &commits).unwrap();
        assert_eq!(graph_row(&g, &id(30).to_string()).lane, 0);
        // `below` doesn't matter: a fold's panel is always below.
        for below in [true, false] {
            let p = build_reveal(&req, &commits, &fold_key(), below).unwrap();
            assert_eq!((p.key.clone(), p.branch.as_str(), p.count, p.entry_lane, p.max_lanes), (fold_key(), "x", 3, 1, 2));
            assert_eq!(ids(&p), shas(&[12, 11, 3, 10, 2]));
            assert_eq!(lanes(&p), [1, 1, 0, 1, 0]);
            let first: Vec<(u16, Half)> = segments(&p, 0).iter().map(|s| (s.from_lane, s.half)).collect();
            assert_eq!(first, [(0, Half::Full), (1, Half::Top), (1, Half::Bottom)], "both lanes come in from the top");
            let fork: Vec<(u16, Half)> = segments(&p, 4).iter().map(|s| (s.from_lane, s.half)).collect();
            assert_eq!(fork, [(0, Half::Top), (1, Half::Top), (0, Half::Bottom)], "the branch curves into its fork point");
        }
    }

    #[test]
    fn trunk_rows_are_tagged_and_the_fork_point_is_last() {
        let (req, commits) = folded(vec![], vec![], 100);
        let p = build_reveal(&req, &commits, &fold_key(), true).unwrap();
        assert_eq!(p.tags, [RevealTag { row: 2, text: "main".into() }, RevealTag { row: 4, text: "main".into() }]);
        assert_eq!(p.rows.last().unwrap().id, id(2).to_string(), "c1, under the fork point, isn't part of it");
        // The fork point outside the window: down to the branch's oldest loaded commit.
        let (req, commits) = folded(vec![], vec![], 6);
        let p = build_reveal(&req, &commits, &fold_key(), true).unwrap();
        assert_eq!(ids(&p), shas(&[12, 11, 3, 10]));
        assert_eq!(p.tags, [RevealTag { row: 2, text: "main".into() }]);
    }

    #[test]
    fn branches_merged_into_the_merged_branch_take_further_lanes() {
        // y (20, 21) from x's 10, merged into x at 12.
        let (req, commits) = folded(vec![c(20, &[10], 260), c(21, &[20], 270), c(12, &[11, 21], 400)], vec![], 100);
        let p = build_reveal(&req, &commits, &fold_key(), true).unwrap();
        assert_eq!(p.count, 5);
        assert_eq!(ids(&p), shas(&[12, 11, 3, 21, 20, 10, 2]));
        assert_eq!(lanes(&p), [1, 1, 0, 2, 2, 1, 0]);
        assert_eq!(p.max_lanes, 3);
    }

    #[test]
    fn the_branch_lane_is_the_first_free_lane_at_the_merge_row() {
        // z, shown, forks from 2 and is newer than the merge: its line runs past it in lane 1.
        let (req, commits) = folded(vec![c(40, &[2], 3050)], vec![local("z", 40)], 100);
        let g = build(&req, &commits).unwrap();
        assert_eq!(graph_row(&g, &id(40).to_string()).lane, 1);
        let p = build_reveal(&req, &commits, &fold_key(), true).unwrap();
        assert_eq!(p.entry_lane, 2);
        assert_eq!(lanes(&p), [2, 2, 0, 2, 0]);
        assert!((0..p.rows.len()).flat_map(|r| segments(&p, r)).all(|s| s.from_lane != 1 && s.to_lane != 1), "the graph's lane 1 stays clear");
    }

    /// Random histories (branches forked anywhere, merged into main and each other, stashes, window
    /// cuts) under random presets: every ⋮ row and folded merge reveals a panel on its lanes.
    #[test]
    fn random_histories_reveal_every_run_and_fold() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let names = ["main", "a", "b", "c", "d"];
        let (mut runs, mut folds) = (0, 0);
        for _ in 0..60 {
            let mut cs = vec![c(1, &[], 100)];
            let mut tips: Vec<Option<u8>> = vec![Some(1), None, None, None, None];
            let mut remotes: Vec<Option<u8>> = vec![None; names.len()];
            let mut stashes = vec![];
            let mut plain = vec![1u8];
            for n in 2..=80u8 {
                let b = rnd(names.len());
                // A branch's remote one: left behind, then maybe moving on by itself (diverged).
                if b > 0 && rnd(6) == 0 {
                    remotes[b] = tips[b];
                }
                let on_remote = b > 0 && remotes[b].is_some() && rnd(2) == 0;
                let base = if on_remote { remotes[b].unwrap() } else { tips[b].unwrap_or_else(|| plain[rnd(plain.len())]) };
                let mut parents = vec![base];
                if rnd(5) == 0
                    && let Some(other) = tips[rnd(names.len())].filter(|&t| t != base)
                {
                    parents.push(other);
                }
                if rnd(15) == 0 {
                    stashes.push(n);
                    cs.push(c(n, &[base], i64::from(n) * 100));
                    continue;
                }
                cs.push(c(n, &parents, i64::from(n) * 100));
                plain.push(n);
                if on_remote {
                    remotes[b] = Some(n);
                    continue;
                }
                tips[b] = Some(n);
                if b > 0 && rnd(6) == 0 {
                    tips[b] = None;
                }
            }
            let mut refs: Vec<_> = names.iter().zip(&tips).filter_map(|(name, t)| Some(local(name, (*t)?))).collect();
            refs.extend(names.iter().zip(&remotes).filter_map(|(name, t)| Some(remote("origin", name, (*t)?))));
            let (mut req, store) = request(refs, [worktree("/demo/x", tips[0].unwrap(), "main", true)], cs, [20, 50, 100][rnd(3)]);
            req.input.refs.stashes = stashes.iter().enumerate().map(|(i, &n)| crate::StashIn { index: i, id: id(n), message: "On x: wip".into(), time: i64::from(n) * 100 }).collect();
            for _ in 0..3 {
                let actions = [PresetAction::Show, PresetAction::Collapse];
                let rules = names[1..].iter().map(|n| Rule { action: actions[rnd(2)], matcher: Matcher::NameMatches { patterns: (*n).into() } }).collect();
                let spec = ViewSpec { rules, otherwise: PresetAction::Collapse, fold_merged: rnd(2) == 0, ..ViewSpec::default() };
                let req = with(req.clone(), spec);
                let g = build(&req, &store).unwrap();
                for dots in g.rows.iter().filter_map(|r| Some((r, r.collapsed.as_ref()?))) {
                    let (row, collapsed) = dots;
                    for below in [false, true] {
                        let p = build_reveal(&req, &store, &collapsed.key, below).unwrap_or_else(|| panic!("{} below {below}", collapsed.key));
                        assert_eq!((p.entry_lane, p.count), (row.lane, collapsed.count));
                        if below {
                            assert!(p.rows.len() > p.count as usize);
                        } else {
                            assert_eq!(p.rows.len(), p.count as usize);
                            assert!(p.rows.iter().all(|r| r.lane >= row.lane), "nothing left of the run's lane");
                        }
                    }
                    runs += 1;
                }
                for (row, fold) in g.rows.iter().filter_map(|r| Some((r, r.fold.as_ref()?))) {
                    let p = build_reveal(&req, &store, &fold.key, true).unwrap_or_else(|| panic!("{}", fold.key));
                    assert!(p.entry_lane > row.lane);
                    assert_eq!(p.count, fold.count);
                    assert!(p.tags.iter().all(|t| p.rows[t.row as usize].lane == row.lane), "the trunk keeps the merge's lane");
                    folds += 1;
                }
            }
        }
        assert!(runs > 20 && folds > 20, "the cases cover runs ({runs}) and folds ({folds})");
    }
}
