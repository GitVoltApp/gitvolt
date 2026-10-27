//! The rows a preset's plan leaves, as `assemble` lays them out: which commits are rows, where
//! each run's ⋮ row goes, the layout parents that skip what isn't a row, and the payload's extras
//! (the ⋮ rows' counts, the folded merges' marks, the hidden branches forking from each row and
//! every branch's state).

use crate::layout::{LayoutNode, NodeKind, Parent, LANE_COLORS};
use crate::payload::{BranchView, CollapsedRow, DecidedBy, FoldMark, GhostBranch, ViewPayload};
use crate::presets::PresetAction;
use crate::view::{newest_tip, run_key, CommitState, Decided, ViewPlan, ViewSpec};
use crate::walk::CommitMeta;
use crate::Oid;
use std::collections::{HashMap, HashSet};

/// A preset's plan over the window, ready for `assemble`.
pub(crate) struct ViewRows {
    pub d: Decided,
    pub plan: ViewPlan,
    /// A run's newest commit → the run.
    run_at: HashMap<usize, usize>,
    /// The runs' oldest commits moved up under their ⋮ row (`PullTogether`).
    pulled: HashSet<usize>,
    /// A merge → its fold.
    fold_at: HashMap<usize, usize>,
    /// The hidden units that fork from a row (its stub's), newest tip first.
    ghosts: HashMap<usize, Vec<usize>>,
}

impl ViewRows {
    pub(crate) fn new(spec: &ViewSpec, d: Decided, plan: ViewPlan, commits: &[CommitMeta], index: &HashMap<Oid, usize>) -> ViewRows {
        let run_at = plan.runs.iter().enumerate().map(|(i, r)| (r.newest, i)).collect();
        let pulled = plan.runs.iter().filter(|r| r.pulled).map(|r| r.oldest).collect();
        let fold_at = plan.folds.iter().enumerate().map(|(i, f)| (f.merge, i)).collect();
        let ghosts = if spec.ghost_lines { ghost_units(&d, &plan, spec.ghost_picks, commits, index) } else { HashMap::new() };
        ViewRows { d, plan, run_at, pulled, fold_at, ghosts }
    }

    /// Whether the commit is a row at its own place in the walk (a pulled-together run's oldest
    /// is a row too, placed by `run_at_newest`'s caller).
    pub(crate) fn in_place(&self, ci: usize) -> bool {
        self.plan.state[ci] == CommitState::Visible && !self.pulled.contains(&ci)
    }

    /// The run whose newest commit `ci` is: its ⋮ row follows, then its oldest commit when pulled
    /// together.
    pub(crate) fn run_at_newest(&self, ci: usize) -> Option<(usize, Option<usize>)> {
        let &r = self.run_at.get(&ci)?;
        let oldest = self.plan.runs[r].oldest;
        Some((r, self.pulled.contains(&oldest).then_some(oldest)))
    }

    /// The row standing for a loaded commit: its own, or its run's ⋮ row; `None` when it's folded
    /// or hidden.
    fn row_of(&self, ci: usize, row_of_commit: &[u32], collapsed_row: &[u32]) -> Option<u32> {
        match self.plan.state[ci] {
            CommitState::Visible => Some(row_of_commit[ci]),
            CommitState::Run { run } => Some(collapsed_row[run]),
            CommitState::Folded { .. } | CommitState::Hidden => None,
        }
    }

    /// A row's layout parents: each parent's row (a run's middle: its ⋮ row), once; a folded or
    /// hidden one dropped; one outside the window running off the bottom. A folded merge's tail
    /// last, ending in the row under it, unless the merge is the last of the window's `rows` (a
    /// tail there would read as more of the branch loaded below).
    pub(crate) fn parents(&self, ci: usize, commits: &[CommitMeta], index: &HashMap<Oid, usize>, row_of_commit: &[u32], collapsed_row: &[u32], rows: u32) -> Vec<Parent> {
        let mut out = Vec::with_capacity(commits[ci].parents.len());
        for p in &commits[ci].parents {
            let parent = match index.get(p) {
                Some(&pi) => match self.row_of(pi, row_of_commit, collapsed_row) {
                    Some(r) => Parent::Row(r),
                    None => continue,
                },
                None => Parent::Outside(*p),
            };
            if !out.contains(&parent) {
                out.push(parent);
            }
        }
        let below = row_of_commit[ci] + 1;
        if self.fold_at.contains_key(&ci) && below < rows && !out.is_empty() {
            out.push(Parent::Tail(below));
        }
        out
    }

    /// Whether hidden branches fork from the commit (it gets a stub, unless it's row 0).
    pub(crate) fn has_ghosts(&self, ci: usize) -> bool {
        self.ghosts.contains_key(&ci)
    }

    /// Run `r`'s ⋮ node: its dotted line down to the oldest commit, then the lines of whatever
    /// else its middle commits have as parents.
    pub(crate) fn collapsed_node(&self, r: usize, commits: &[CommitMeta], index: &HashMap<Oid, usize>, row_of_commit: &[u32], collapsed_row: &[u32]) -> LayoutNode {
        let run = &self.plan.runs[r];
        let mut parents = vec![Parent::Row(row_of_commit[run.oldest])];
        for &m in &run.middle {
            for p in &commits[m].parents {
                let parent = match index.get(p) {
                    Some(&pi) if pi == run.oldest || self.plan.state[pi] == (CommitState::Run { run: r }) => continue,
                    Some(&pi) => match self.row_of(pi, row_of_commit, collapsed_row) {
                        Some(row) => Parent::Row(row),
                        None => continue,
                    },
                    None => Parent::Outside(*p),
                };
                if !parents.contains(&parent) {
                    parents.push(parent);
                }
            }
        }
        LayoutNode { parents, kind: NodeKind::Collapsed, pinned: None, time: commits[run.middle[0]].committer_time, stub: false }
    }

    /// Run `r`'s ⋮ row: its key (`run_key`), how many commits it holds and by how many authors (by
    /// email).
    pub(crate) fn collapsed_row(&self, r: usize, commits: &[CommitMeta]) -> CollapsedRow {
        let run = &self.plan.runs[r];
        let branch = self.d.units[run.unit].name.clone();
        let authors: HashSet<String> = run.middle.iter().map(|&m| commits[m].author_email.to_lowercase()).collect();
        CollapsedRow { key: run_key(&branch, &commits[run.newest].id), branch, count: run.middle.len() as u32, authors: authors.len() as u32 }
    }

    /// The mark of a merge holding a folded branch, whose tail the layout put in lane `tail`.
    pub(crate) fn fold_mark(&self, ci: usize, commits: &[CommitMeta], tail: Option<u16>) -> Option<FoldMark> {
        let f = &self.plan.folds[*self.fold_at.get(&ci)?];
        Some(FoldMark { key: format!("fold:{}", commits[ci].id), branch: f.branch.clone(), count: f.commits.len() as u32, tail_lane: tail })
    }

    /// The hidden branches forking from a row, all in the colour of its stub's lane (`stub`), or of
    /// the lane right of the row's without one (row 0).
    pub(crate) fn ghosts(&self, ci: usize, lane: u16, stub: Option<u16>, commits: &[CommitMeta], index: &HashMap<Oid, usize>) -> Vec<GhostBranch> {
        let Some(units) = self.ghosts.get(&ci) else { return vec![] };
        let color = (usize::from(stub.unwrap_or(lane + 1)) % usize::from(LANE_COLORS)) as u8;
        units
            .iter()
            .map(|&u| {
                let tip = newest_tip(&self.d.units[u], commits, index).map(|t| &commits[t]);
                GhostBranch {
                    branch: self.d.units[u].name.clone(),
                    color,
                    tip: tip.map(|t| t.id.to_string()).unwrap_or_default(),
                    summary: tip.map(|t| t.summary.clone()).unwrap_or_default(),
                    author_name: tip.map(|t| t.author_name.clone()).unwrap_or_default(),
                    author_time: tip.map_or(0, |t| t.author_time),
                }
            })
            .collect()
    }

    /// Every unit's state, sorted by name, the trunk's names and the hidden tags'.
    pub(crate) fn payload(&self) -> ViewPayload {
        let mut branches: Vec<BranchView> = self
            .d
            .units
            .iter()
            .enumerate()
            .map(|(u, unit)| BranchView { name: unit.name.clone(), refs: unit.refs.clone(), state: self.plan.unit_state[u], by: self.d.decisions[u].by })
            .collect();
        branches.sort_by(|a, b| a.name.cmp(&b.name));
        let mut trunk: Vec<String> = self.d.units.iter().zip(&self.d.trunk).filter(|(_, t)| **t).map(|(u, _)| u.name.clone()).collect();
        trunk.sort();
        let mut hidden_tags: Vec<String> = self.d.hidden_tags.iter().map(|t| t.name.clone()).collect();
        hidden_tags.sort();
        ViewPayload { branches, trunk, hidden_tags }
    }
}

/// Per row, the units a preset's rules hide (a pick too with `picks`) that fork from it: those
/// reaching a hidden child of it. Newest tip first.
fn ghost_units(d: &Decided, plan: &ViewPlan, picks: bool, commits: &[CommitMeta], index: &HashMap<Oid, usize>) -> HashMap<usize, Vec<usize>> {
    let ghostly: Vec<bool> = d
        .decisions
        .iter()
        .map(|x| x.action == PresetAction::Hide && (matches!(x.by, DecidedBy::Rule { .. } | DecidedBy::Otherwise) || (picks && x.by == DecidedBy::Picked)))
        .collect();
    let mut out: HashMap<usize, Vec<usize>> = HashMap::new();
    for (h, _) in plan.state.iter().enumerate().filter(|(_, s)| **s == CommitState::Hidden) {
        for p in &commits[h].parents {
            let Some(&p) = index.get(p).filter(|&&p| plan.state[p] == CommitState::Visible) else { continue };
            for u in d.facts.reach.units_reaching(h).filter(|&u| ghostly[u]) {
                let units = out.entry(p).or_default();
                if !units.contains(&u) {
                    units.push(u);
                }
            }
        }
    }
    let tip_time: Vec<Option<i64>> = d.units.iter().map(|u| newest_tip(u, commits, index).map(|t| commits[t].committer_time)).collect();
    for units in out.values_mut() {
        units.sort_by_key(|&u| (std::cmp::Reverse(tip_time[u]), u));
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::layout::{Half, NodeKind, Segment};
    use crate::payload::{BranchState, DecidedBy, GraphPayload};
    use crate::presets::{BranchPicks, CollapsePlacement, Matcher, PresetAction, Rule};
    use crate::testkit::{commit, id, local, remote, request, tag, worktree};
    use crate::view::ViewSpec;
    use crate::{build, build_hidden_owner, build_preview, CommitMeta, GraphRequest, Oid, PresetPreview, StashIn};
    use std::collections::HashMap;

    fn c(n: u8, parents: &[u8], time: i64, message: &str) -> CommitMeta {
        commit(n, parents, time, "ada@example.com", message)
    }

    fn rule(action: PresetAction, patterns: &str) -> Rule {
        Rule { action, matcher: Matcher::NameMatches { patterns: patterns.into() } }
    }

    fn view(rules: Vec<Rule>) -> ViewSpec {
        ViewSpec { rules, otherwise: PresetAction::Show, ghost_lines: true, ..ViewSpec::default() }
    }

    fn with(mut req: GraphRequest, spec: ViewSpec) -> GraphRequest {
        req.input.view = Some(spec);
        req
    }

    fn summaries(g: &GraphPayload) -> Vec<&str> {
        g.rows.iter().map(|r| if r.kind == NodeKind::Collapsed { "⋮" } else { r.summary.as_str() }).collect()
    }

    fn segments(g: &GraphPayload, row: usize) -> Vec<Segment> {
        g.rows[row].segments.iter().map(|&s| Segment::unpack(s)).collect()
    }

    /// A branch from main's 2 whose commits `first..=last` are at `n * 100`, `x<n>`.
    fn branch(name: &str, first: u8, last: u8, from: u8) -> Vec<CommitMeta> {
        (first..=last).map(|n| c(n, &[if n == first { from } else { n - 1 }], i64::from(n) * 100, &format!("{name}{n}"))).collect()
    }

    /// main: 1 ← 2 ← 3 (at 1250); x from 2: 10..=15 (1000..=1500), its 13 by bea.
    fn history() -> (GraphRequest, HashMap<Oid, CommitMeta>) {
        let mut cs = vec![c(1, &[], 100, "Initial commit"), c(2, &[1], 200, "c2"), c(3, &[2], 1250, "c3")];
        cs.extend(branch("x", 10, 15, 2));
        cs[3 + 3] = commit(13, &[12], 1300, "bea@example.com", "x13");
        request([local("main", 3), local("x", 15)], [worktree("/demo/x", 3, "main", true)], cs, 100)
    }

    #[test]
    fn no_view_is_todays_payload() {
        let (req, commits) = history();
        let plain = build(&req, &commits).unwrap();
        let json = serde_json::to_value(&plain).unwrap();
        assert!(json.get("view").is_none());
        for row in json["rows"].as_array().unwrap() {
            for key in ["collapsed", "fold", "ghosts"] {
                assert!(row.get(key).is_none(), "{key}");
            }
        }
        let viewed = build(&with(req, view(vec![])), &commits).unwrap();
        assert_eq!(viewed.rows, plain.rows, "a view showing everything lays out the same rows");
        assert!(viewed.view.is_some());
    }

    #[test]
    fn keep_date_order_leaves_the_oldest_at_its_date() {
        let (req, commits) = history();
        let g = build(&with(req, view(vec![rule(PresetAction::Collapse, "x")])), &commits).unwrap();
        assert_eq!(summaries(&g), ["x15", "⋮", "c3", "x10", "c2", "Initial commit"]);
        let dotted = g.rows[1].lane;
        assert_eq!((g.rows[0].lane, g.rows[3].lane), (dotted, dotted));
        assert!(segments(&g, 2).iter().any(|s| s.from_lane == dotted && s.half == Half::Full && s.dotted), "the dotted lane runs past c3");
        let row = g.rows[1].collapsed.as_ref().unwrap();
        assert_eq!((row.key.clone(), row.branch.as_str(), row.count, row.authors), (format!("branch:x@{}", id(15)), "x", 4, 2));
        assert_eq!((g.rows[1].id.clone(), g.rows[1].parents.len()), (format!("collapsed:x@{}", id(15)), 0));
        assert_eq!(g.labels.iter().find(|l| l.name == "x").unwrap().row, 0);
    }

    #[test]
    fn pull_together_makes_three_adjacent_rows() {
        let (req, commits) = history();
        let spec = ViewSpec { placement: CollapsePlacement::PullTogether, ..view(vec![rule(PresetAction::Collapse, "x")]) };
        let g = build(&with(req, spec), &commits).unwrap();
        assert_eq!(summaries(&g), ["x15", "⋮", "x10", "c3", "c2", "Initial commit"]);
    }

    #[test]
    fn a_checked_out_branch_collapses_under_its_wip() {
        let (mut req, commits) = history();
        req.input.worktrees.push(worktree("/demo/wt-x", 15, "x", false));
        req.input.wip = vec![(1, Default::default())];
        let g = build(&with(req, view(vec![rule(PresetAction::Collapse, "x")])), &commits).unwrap();
        assert_eq!(summaries(&g), ["// WIP", "x15", "⋮", "c3", "x10", "c2", "Initial commit"]);
        assert_eq!(g.view.unwrap().branches.iter().find(|b| b.name == "x").unwrap().state, BranchState::Collapsed);
    }

    fn picked(hidden: &[&str]) -> ViewSpec {
        ViewSpec { picks: BranchPicks { hidden: hidden.iter().map(|s| s.to_string()).collect(), ..Default::default() }, ..view(vec![]) }
    }

    fn wips(g: &GraphPayload) -> Vec<&str> {
        g.rows.iter().filter_map(|r| Some(r.wip.as_ref()?.worktree_path.as_str())).collect()
    }

    #[test]
    fn a_branch_checked_out_elsewhere_hides_by_hand_with_its_wip() {
        let (mut req, commits) = history();
        req.input.worktrees.push(worktree("/demo/wt-x", 15, "x", false));
        req.input.wip = vec![(0, Default::default()), (1, Default::default())];
        let g = build(&with(req.clone(), picked(&["x"])), &commits).unwrap();
        assert_eq!(summaries(&g), ["// WIP", "c3", "c2", "Initial commit"], "x's commits and its worktree's WIP go");
        assert_eq!(wips(&g), ["/demo/x"]);
        assert_eq!(g.max_lanes, 1, "no lane left behind");
        assert!(g.labels.iter().all(|l| l.name != "x"));
        assert_eq!(g.worktrees.iter().map(|w| w.path.as_str()).collect::<Vec<_>>(), ["/demo/x", "/demo/wt-x"], "the worktree is still listed");
        assert_eq!(g.view.unwrap().branches.iter().find(|b| b.name == "x").map(|b| (b.state, b.by)), Some((BranchState::Hidden, DecidedBy::Picked)));
        // Shown again, both come back; a rule doesn't hide it.
        for spec in [picked(&[]), view(vec![rule(PresetAction::Hide, "x")])] {
            let g = build(&with(req.clone(), spec), &commits).unwrap();
            assert_eq!(wips(&g), ["/demo/x", "/demo/wt-x"]);
            assert_eq!(summaries(&g)[..3], ["// WIP", "// WIP", "x15"]);
        }
        // The tab showing x's worktree keeps its branch, whatever the pick.
        req.input.active = Some("/demo/wt-x".into());
        let g = build(&with(req, picked(&["x"])), &commits).unwrap();
        assert_eq!(wips(&g), ["/demo/wt-x", "/demo/x"]);
        assert_eq!(summaries(&g)[..2], ["// WIP", "x15"]);
        assert_eq!(g.view.unwrap().branches.iter().find(|b| b.name == "x").map(|b| (b.state, b.by)), Some((BranchState::Shown, DecidedBy::CheckedOut)));

        // One whose HEAD stays a row (main's tip) loses its WIP row too.
        let (mut req, commits) = history();
        req.input.refs.refs.push(local("y", 3));
        req.input.worktrees.push(worktree("/demo/wt-y", 3, "y", false));
        req.input.wip = vec![(1, Default::default())];
        assert_eq!(wips(&build(&with(req.clone(), picked(&[])), &commits).unwrap()), ["/demo/wt-y"]);
        let g = build(&with(req, picked(&["y"])), &commits).unwrap();
        assert!(wips(&g).is_empty() && g.rows.iter().any(|r| r.summary == "c3"));
    }

    #[test]
    fn a_hidden_branch_has_no_chip_on_a_commit_that_stays() {
        // deploy/1 (local and remote) and deploy/2 (remote only) on main's commits, beside a tag
        // and a branch whose upstream is origin/deploy/1.
        let (mut req, commits) = history();
        let keep = crate::RefIn { upstream: Some("refs/remotes/origin/deploy/1".into()), ..local("keep", 2) };
        req.input.refs.refs.extend([local("deploy/1", 2), remote("origin", "deploy/1", 2), remote("origin", "deploy/2", 3), tag("v1", 2), keep]);
        let names = |g: &GraphPayload| g.labels.iter().map(|l| (l.name.clone(), l.remotes.len())).collect::<Vec<_>>();
        let named = |list: &[(&str, usize)]| list.iter().map(|&(n, r)| (n.to_string(), r)).collect::<Vec<_>>();
        let plain = build(&req, &commits).unwrap();
        assert_eq!(names(&plain), named(&[("x", 0), ("main", 0), ("deploy/2", 1), ("deploy/1", 1), ("keep", 1), ("v1", 0)]));
        // By a rule or by hand.
        for spec in [view(vec![rule(PresetAction::Hide, "deploy/*")]), picked(&["deploy/1", "deploy/2"])] {
            let req = with(req.clone(), spec);
            let g = build(&req, &commits).unwrap();
            assert_eq!(names(&g), named(&[("x", 0), ("main", 0), ("keep", 0), ("v1", 0)]), "no chip, local or remote; the rest unchanged");
            assert_eq!(g.rows, plain.rows, "the same rows, and no stubs: none of their commits is hidden");
            assert_eq!(build_hidden_owner(&req, &commits, &id(2).to_string()), None, "its tip is a row to go to");
        }
        // The trunk can't be hidden: its chip stays.
        assert!(build(&with(req, picked(&["main"])), &commits).unwrap().labels.iter().any(|l| l.name == "main"));
    }

    fn tags_picked(shown: &[&str], hidden: &[&str]) -> ViewSpec {
        let names = |l: &[&str]| l.iter().map(|s| s.to_string()).collect();
        ViewSpec { picks: BranchPicks { shown_tags: names(shown), hidden_tags: names(hidden), ..Default::default() }, ..view(vec![]) }
    }

    /// `history`, and an archived branch from main's 2 that only tags keep: 20 (at 600), tagged
    /// `kept/20`, and 21 (700), tagged `archive/old`. Also `archive/on-x` on x's 14, `v1` on main's
    /// tip, beside a branch `v1`.
    fn archived() -> (GraphRequest, HashMap<Oid, CommitMeta>) {
        let (mut req, mut commits) = history();
        for cm in [c(20, &[2], 600, "old20"), c(21, &[20], 700, "old21")] {
            commits.insert(cm.id, cm);
        }
        req.input.refs.refs.extend([tag("archive/old", 21), tag("kept/20", 20), tag("archive/on-x", 14), tag("v1", 3), local("v1", 3)]);
        (req, commits)
    }

    fn tag_chips(g: &GraphPayload) -> Vec<&str> {
        g.labels.iter().filter(|l| l.tag).map(|l| l.name.as_str()).collect()
    }

    #[test]
    fn a_hidden_tag_drops_its_chip_and_the_commits_only_it_keeps() {
        let (req, commits) = archived();
        let plain = build(&with(req.clone(), view(vec![])), &commits).unwrap();
        assert_eq!(summaries(&plain), ["x15", "x14", "x13", "c3", "x12", "x11", "x10", "old21", "old20", "c2", "Initial commit"]);
        assert_eq!(tag_chips(&plain), ["archive/on-x", "v1", "archive/old", "kept/20"]);

        // The folder: its tags' chips go, and old21, which only archive/old keeps; old20 stays for
        // kept/20, and x14 for x.
        let req_archive = with(req.clone(), tags_picked(&[], &["archive/"]));
        let g = build(&req_archive, &commits).unwrap();
        assert_eq!(summaries(&g), ["x15", "x14", "x13", "c3", "x12", "x11", "x10", "old20", "c2", "Initial commit"]);
        assert_eq!(tag_chips(&g), ["v1", "kept/20"]);
        assert_eq!(g.view.as_ref().unwrap().hidden_tags, ["archive/old", "archive/on-x"]);
        assert!(g.rows.iter().all(|r| r.ghosts.is_empty()), "no stubs: tags are hidden by hand");
        // Both tags hidden: the archived branch goes whole, with no line left behind.
        let g = build(&with(req.clone(), tags_picked(&[], &["archive/", "kept/20"])), &commits).unwrap();
        assert_eq!(summaries(&g), ["x15", "x14", "x13", "c3", "x12", "x11", "x10", "c2", "Initial commit"]);
        assert_eq!(g.max_lanes, 2);
        // A tag shown by hand inside the hidden folder stays, and so does what it keeps.
        let g = build(&with(req.clone(), tags_picked(&["archive/old"], &["archive/", "kept/20"])), &commits).unwrap();
        assert_eq!(tag_chips(&g), ["v1", "archive/old"]);
        assert!(summaries(&g).contains(&"old21") && summaries(&g).contains(&"old20"));

        // On the trunk's tip: only its chip goes, and the branch v1 keeps its own.
        let g = build(&with(req, tags_picked(&[], &["v1"])), &commits).unwrap();
        assert_eq!(summaries(&g), summaries(&plain));
        assert_eq!(tag_chips(&g), ["archive/on-x", "archive/old", "kept/20"]);
        assert!(g.labels.iter().any(|l| l.name == "v1" && l.local.is_some()));
    }

    #[test]
    fn a_hidden_branch_keeps_its_tagged_commits_hidden_and_a_detached_head_keeps_a_hidden_tags() {
        // As before tags could hide: a shown tag doesn't bring back a hidden branch's commits.
        let (req, commits) = archived();
        let g = build(&with(req.clone(), picked(&["x"])), &commits).unwrap();
        assert!(!summaries(&g).contains(&"x14") && !tag_chips(&g).contains(&"archive/on-x"));
        // HEAD detached at old21: the commit stays a row, without archive/old's chip.
        let (mut req, commits) = archived();
        req.input.refs.head = crate::input::HeadIn { branch: None, target: Some(id(21)), detached: true, unborn: false };
        req.input.worktrees[0].branch = None;
        req.input.worktrees[0].head = Some(id(21));
        let g = build(&with(req, tags_picked(&[], &["archive/", "kept/20"])), &commits).unwrap();
        assert!(summaries(&g).contains(&"old21") && summaries(&g).contains(&"old20"));
        assert!(g.labels.iter().all(|l| l.name != "archive/old"));
    }

    #[test]
    fn hidden_owner_names_the_hidden_tag_and_its_expansion_shows_it() {
        let (req, commits) = archived();
        let spec = tags_picked(&[], &["archive/", "kept/20"]);
        let req = with(req, spec.clone());
        // Both hidden tags reach 20: archive/old's commit is the newer.
        assert_eq!(build_hidden_owner(&req, &commits, &id(21).to_string()).as_deref(), Some("tag:archive/old"));
        assert_eq!(build_hidden_owner(&req, &commits, &id(20).to_string()).as_deref(), Some("tag:archive/old"));
        assert_eq!(build_hidden_owner(&req, &commits, &id(14).to_string()), None, "a row: only its chip went");
        let expanded = with(req.clone(), ViewSpec { expanded: vec!["tag:archive/old".into()], ..spec });
        let g = build(&expanded, &commits).unwrap();
        assert!(summaries(&g).contains(&"old21") && summaries(&g).contains(&"old20"));
        assert_eq!(tag_chips(&g), ["v1", "archive/old"]);
        assert_eq!(g.view.unwrap().hidden_tags, ["archive/on-x", "kept/20"]);
        assert_eq!(build_hidden_owner(&expanded, &commits, &id(21).to_string()), None);
    }

    #[test]
    fn a_stash_in_a_run_keeps_every_line_continuous() {
        let (mut req, mut commits) = history();
        // On x's 12 (a middle commit), on its oldest, and on its newest.
        for (n, base, time) in [(40, 12, 1210), (41, 10, 1600), (42, 15, 1700)] {
            commits.insert(id(n), c(n, &[base], time, "On x: wip"));
        }
        req.input.refs.stashes = [(40, 1210), (41, 1600), (42, 1700)].into_iter().enumerate().map(|(i, (n, time))| StashIn { index: i, id: id(n), message: "On x: wip".into(), time }).collect();
        for placement in [CollapsePlacement::KeepDateOrder, CollapsePlacement::PullTogether] {
            let spec = ViewSpec { placement, ..view(vec![rule(PresetAction::Collapse, "x")]) };
            // `build` checks every line's continuity (the `testing` checks run in unit tests).
            let g = build(&with(req.clone(), spec), &commits).unwrap();
            assert_eq!(g.rows.iter().filter(|r| r.kind == NodeKind::Stash).count(), 1, "only the newest's stash is a row");
            assert_eq!(g.rows.iter().find_map(|r| r.collapsed.as_ref()).unwrap().count, 6);
        }
    }

    #[test]
    fn a_run_cut_by_the_window_collapses_with_what_is_loaded() {
        // x: 10..=19 from main's 2, all newer than main; the window holds x's 7 newest.
        let mut cs = vec![c(1, &[], 100, "Initial commit"), c(2, &[1], 200, "c2"), c(3, &[2], 300, "c3")];
        cs.extend(branch("x", 10, 19, 2));
        let (req, commits) = request([local("main", 3), local("x", 19)], [worktree("/demo/x", 3, "main", true)], cs, 7);
        let mut req = with(req, view(vec![rule(PresetAction::Collapse, "x")]));
        let g = build(&req, &commits).unwrap();
        assert!(g.truncated);
        assert_eq!(summaries(&g), ["x19", "⋮", "x13"]);
        assert_eq!(g.rows[1].collapsed.as_ref().unwrap().count, 5);
        assert!(segments(&g, 2).iter().any(|s| s.half == Half::Bottom), "the oldest loaded runs off the bottom");
        // Loading more changes the count, not the rows above.
        req.limit = 9;
        let more = build(&req, &commits).unwrap();
        assert_eq!(summaries(&more)[..3], ["x19", "⋮", "x11"]);
        assert_eq!(more.rows[1].collapsed.as_ref().unwrap().count, 7);
        assert_eq!((more.rows[0].lane, more.rows[1].lane), (g.rows[0].lane, g.rows[1].lane));
    }

    /// main: 1 ← 2 ← M(30, merging x's 12) ← 31; x: 10..=12 from 1, its ref deleted.
    fn merged() -> (GraphRequest, HashMap<Oid, CommitMeta>) {
        let mut cs = vec![c(1, &[], 100, "Initial commit"), c(2, &[1], 200, "c2"), c(30, &[2, 12], 3000, "Merge branch 'x'"), c(31, &[30], 3100, "c31")];
        cs.extend(branch("x", 10, 12, 1));
        request([local("main", 31)], [worktree("/demo/x", 31, "main", true)], cs, 100)
    }

    /// `merged`, with x's ref at 12, and y (20..=25) and z (26) from main's 2.
    fn merged_and_more() -> (GraphRequest, HashMap<Oid, CommitMeta>) {
        let (mut req, mut commits) = merged();
        commits.extend(branch("y", 20, 25, 2).into_iter().chain(branch("z", 26, 26, 2)).map(|c| (c.id, c)));
        req.input.refs.refs.extend([local("x", 12), local("y", 25), local("z", 26)]);
        (req, commits)
    }

    #[test]
    fn a_folded_merge_keeps_its_trunk_line_and_a_tail() {
        let (req, commits) = merged();
        let g = build(&with(req, ViewSpec { fold_merged: true, ..view(vec![]) }), &commits).unwrap();
        assert_eq!(summaries(&g), ["c31", "Merge branch 'x'", "c2", "Initial commit"]);
        let merge = &g.rows[1];
        assert_eq!(merge.kind, NodeKind::Merge);
        assert_eq!(segments(&g, 1).iter().filter(|s| s.half == Half::Bottom).map(|s| (s.to_lane, s.dotted)).collect::<Vec<_>>(), [(0, false), (1, false)], "down its own lane, and the tail's curve");
        let fold = merge.fold.as_ref().unwrap();
        assert_eq!((fold.key.clone(), fold.branch.as_str(), fold.count, fold.tail_lane), (format!("fold:{}", id(30)), "x", 3, Some(1)));
        let end: Vec<(u16, Half, bool)> = segments(&g, 2).iter().filter(|s| s.from_lane == 1).map(|s| (s.to_lane, s.half, s.dotted)).collect();
        assert_eq!(end, [(1, Half::Top, true)], "ending dotted in the row under it");
        assert!(segments(&g, 3).iter().all(|s| s.from_lane == 0 && s.to_lane == 0));
        assert_eq!(g.max_lanes, 2, "the lane x itself would take");
    }

    #[test]
    fn a_fold_on_the_last_loaded_row_has_no_tail_until_more_loads() {
        // (e) The window ends at the merge (x's 12 loaded, folded): a tail would read as more of x
        // loaded below.
        let (req, commits) = merged();
        let mut req = with(req, ViewSpec { fold_merged: true, ..view(vec![]) });
        req.limit = 3;
        let g = build(&req, &commits).unwrap();
        assert_eq!(summaries(&g), ["c31", "Merge branch 'x'"]);
        let fold = g.rows[1].fold.as_ref().unwrap();
        assert_eq!(fold.tail_lane, None);
        assert_eq!(segments(&g, 1).iter().filter(|s| s.half == Half::Bottom).map(|s| s.to_lane).collect::<Vec<_>>(), [0], "only its trunk line, off the bottom");
        assert_eq!(g.max_lanes, 1);
        req.limit = 100;
        assert_eq!(build(&req, &commits).unwrap().rows[1].fold.as_ref().unwrap().tail_lane, Some(1), "Load older commits brings it back");
    }

    #[test]
    fn a_fold_tail_routes_around_a_pulled_together_run() {
        // (f) x merged into main at 30; y (20..=25 from main's 1, newer than the merge) collapsed
        // and pulled together: its oldest's line passes the merge in lane 1, so the tail takes 2.
        let (mut req, mut commits) = merged();
        commits.extend(branch("y", 20, 25, 1).into_iter().map(|c| (c.id, c)));
        for n in 20..=25u8 {
            commits.get_mut(&id(n)).unwrap().committer_time = 3010 + i64::from(n);
        }
        req.input.refs.refs.push(local("y", 25));
        let spec = ViewSpec { fold_merged: true, placement: CollapsePlacement::PullTogether, ..view(vec![rule(PresetAction::Collapse, "y")]) };
        let g = build(&with(req, spec), &commits).unwrap();
        assert_eq!(summaries(&g), ["c31", "y25", "⋮", "y20", "Merge branch 'x'", "c2", "Initial commit"]);
        assert_eq!(g.rows[3].lane, 1);
        assert_eq!(g.rows[4].fold.as_ref().unwrap().tail_lane, Some(2));
        assert!(segments(&g, 4).iter().any(|s| s.from_lane == 1 && s.half == Half::Full), "y's line passes the merge");
    }

    #[test]
    fn the_view_payload_lists_every_branch_state() {
        let (req, commits) = merged_and_more();
        let spec = ViewSpec { fold_merged: true, picks: BranchPicks { hidden: vec!["z".into()], ..Default::default() }, ..view(vec![rule(PresetAction::Collapse, "y")]) };
        let g = build(&with(req, spec), &commits).unwrap();
        let v = g.view.unwrap();
        let states: Vec<(&str, BranchState, DecidedBy)> = v.branches.iter().map(|b| (b.name.as_str(), b.state, b.by)).collect();
        assert_eq!(
            states,
            [
                ("main", BranchState::Shown, DecidedBy::Trunk),
                ("x", BranchState::Folded, DecidedBy::Otherwise),
                ("y", BranchState::Collapsed, DecidedBy::Rule { index: 0 }),
                ("z", BranchState::Hidden, DecidedBy::Picked),
            ]
        );
        assert_eq!(v.trunk, ["main"]);
        assert_eq!(v.branches[0].refs, ["refs/heads/main"]);
        assert!(g.labels.iter().all(|l| l.name != "x" && l.name != "z"), "no chips on commits that aren't rows");
    }

    #[test]
    fn the_trunk_keeps_lane_zero_under_a_view() {
        let (req, commits) = history();
        for spec in [view(vec![rule(PresetAction::Collapse, "x")]), view(vec![rule(PresetAction::Hide, "x")])] {
            let g = build(&with(req.clone(), spec), &commits).unwrap();
            assert_eq!(g.pinned_refs, ["refs/heads/main"]);
            let trunk: Vec<u16> = g.rows.iter().filter(|r| ["c3", "c2", "Initial commit"].contains(&r.summary.as_str())).map(|r| r.lane).collect();
            assert_eq!(trunk, [0, 0, 0]);
        }
    }

    #[test]
    fn hidden_owner_names_the_run_the_fold_or_the_hidden_branch() {
        let (req, commits) = history();
        let req = with(req, view(vec![rule(PresetAction::Collapse, "x")]));
        assert_eq!(build_hidden_owner(&req, &commits, &id(12).to_string()), Some(format!("branch:x@{}", id(15))), "its run's key");
        assert_eq!(build_hidden_owner(&req, &commits, &id(15).to_string()), None, "a row");
        assert_eq!(build_hidden_owner(&req, &commits, &id(99).to_string()), None, "not loaded");
        let (fold_req, fold_commits) = merged();
        let fold_req = with(fold_req, ViewSpec { fold_merged: true, ..view(vec![]) });
        assert_eq!(build_hidden_owner(&fold_req, &fold_commits, &id(11).to_string()), Some(format!("fold:{}", id(30))));
        // Hidden by two branches: the one with the newer tip, w (16) on top of x.
        let (mut req, mut commits) = history();
        commits.insert(id(16), c(16, &[15], 1600, "w16"));
        req.input.refs.refs.push(local("w", 16));
        let req = with(req, view(vec![rule(PresetAction::Hide, "x, w")]));
        assert_eq!(build_hidden_owner(&req, &commits, &id(12).to_string()).as_deref(), Some("branch:w"));
    }

    /// main: 1 ← 2 ← 3; x's shared commits 10..=15 from 2, then local x 20..=25 and origin/x
    /// 30..=35, both from 15; x collapsed.
    fn diverged() -> (GraphRequest, HashMap<Oid, CommitMeta>) {
        let mut cs = vec![c(1, &[], 100, "Initial commit"), c(2, &[1], 200, "c2"), c(3, &[2], 300, "c3")];
        cs.extend(branch("x", 10, 15, 2).into_iter().chain(branch("x", 20, 25, 15)).chain(branch("x", 30, 35, 15)));
        let (req, commits) = request([local("main", 3), local("x", 25), remote("origin", "x", 35)], [worktree("/demo/x", 3, "main", true)], cs, 100);
        (with(req, view(vec![rule(PresetAction::Collapse, "x")])), commits)
    }

    #[test]
    fn diverged_local_and_remote_branches_get_a_run_each() {
        let (req, commits) = diverged();
        let g = build(&req, &commits).unwrap();
        assert_eq!(summaries(&g), ["x35", "⋮", "x30", "x25", "⋮", "x20", "x15", "⋮", "x10", "c3", "c2", "Initial commit"]);
        let keys: Vec<String> = g.rows.iter().filter_map(|r| Some(r.collapsed.as_ref()?.key.clone())).collect();
        assert_eq!(keys, [35, 25, 15].map(|n| format!("branch:x@{}", id(n))));
        assert_eq!(g.labels.iter().find(|l| l.local.as_deref() == Some("refs/heads/x")).map(|l| l.row), Some(3), "the local tip is a row with its chip");
        assert!(g.labels.iter().any(|l| l.row == 0 && l.remotes.iter().any(|r| r.full_name == "refs/remotes/origin/x")), "and the remote's tip");
        // Each hidden commit names its own run; expanding the branch unfolds all three.
        for (hidden, run) in [(33, 35), (22, 25), (12, 15)] {
            assert_eq!(build_hidden_owner(&req, &commits, &id(hidden).to_string()), Some(format!("branch:x@{}", id(run))));
        }
        let mut spec = req.input.view.clone().unwrap();
        spec.expanded = vec!["branch:x".into()];
        let g = build(&with(req, spec), &commits).unwrap();
        assert!(g.rows.iter().all(|r| r.kind != NodeKind::Collapsed));
        assert_eq!(g.rows.len(), 21);
    }

    /// main: 1 ← 2 ← 3; x's 10 and 11 from 2, origin/x at 12 merging them, then local x on to 15
    /// and 16, which merges 13 ← 14 (built on 11); y at 16 too.
    fn lines_sharing_a_commit() -> (GraphRequest, HashMap<Oid, CommitMeta>) {
        let mut cs = vec![c(1, &[], 100, "Initial commit"), c(2, &[1], 200, "c2"), c(3, &[2], 300, "c3")];
        cs.extend([c(10, &[2], 1000, "x10"), c(11, &[2], 1100, "x11"), c(12, &[10, 11], 1200, "x12"), c(13, &[11], 1300, "x13"), c(14, &[13], 1400, "x14")]);
        cs.extend([c(15, &[12], 1500, "x15"), c(16, &[15, 14], 1600, "x16")]);
        request([local("main", 3), local("x", 16), remote("origin", "x", 12), local("y", 16)], [worktree("/demo/x", 3, "main", true)], cs, 100)
    }

    #[test]
    fn a_pulled_together_oldest_stays_at_its_date_when_a_row_below_reaches_it() {
        // With y hidden, x has 16 and everything under it to itself: 15, 14 and 13 collapse into a
        // ⋮ row down to 11, which origin/x's 12 merged. Pulled up under the ⋮ row, 11 would sit
        // above 12, whose line to it would run on down past the last row instead: 11 stays at its
        // date, as with Keep date order.
        let (req, commits) = lines_sharing_a_commit();
        let collapse = rule(PresetAction::Collapse, "x");
        for hide in [picked(&["y"]), view(vec![rule(PresetAction::Hide, "y")])] {
            let spec = ViewSpec { rules: hide.rules.iter().cloned().chain([collapse.clone()]).collect(), ..hide };
            for placement in [CollapsePlacement::PullTogether, CollapsePlacement::KeepDateOrder] {
                let g = build(&with(req.clone(), ViewSpec { placement, ..spec.clone() }), &commits).unwrap_or_else(|e| panic!("{placement:?}: {e}"));
                assert_eq!(summaries(&g), ["x16", "⋮", "x12", "x11", "x10", "c3", "c2", "Initial commit"], "{placement:?}");
                assert_eq!(g.rows[1].collapsed.as_ref().map(|r| r.count), Some(3));
            }
        }
        // Shown, y keeps every commit a row.
        assert_eq!(build(&with(req, view(vec![collapse])), &commits).unwrap().rows.len(), 10);
    }

    /// main: 1 ← 2 ← 3 (at 900); x (10..=12) and w (20..=21) from 2, newer than 3.
    fn two_off_main() -> (GraphRequest, HashMap<Oid, CommitMeta>) {
        let mut cs = vec![c(1, &[], 100, "Initial commit"), c(2, &[1], 200, "c2"), c(3, &[2], 900, "c3")];
        cs.extend(branch("x", 10, 12, 2).into_iter().chain(branch("w", 20, 21, 2)));
        request([local("main", 3), local("x", 12), local("w", 21)], [worktree("/demo/x", 3, "main", true)], cs, 100)
    }

    fn ghosts_on_c2(g: &GraphPayload) -> Vec<(&str, u8)> {
        let row = g.rows.iter().find(|r| r.summary == "c2").unwrap();
        row.ghosts.iter().map(|gh| (gh.branch.as_str(), gh.color)).collect()
    }

    #[test]
    fn a_stub_marks_where_hidden_branches_fork() {
        let (req, commits) = two_off_main();
        let g = build(&with(req, view(vec![rule(PresetAction::Hide, "x")])), &commits).unwrap();
        assert_eq!(summaries(&g), ["w21", "w20", "c3", "c2", "Initial commit"]);
        // w's line passes c3 in lane 1: the stub starts beside it, in lane 2, and so is coloured.
        assert_eq!(g.rows[3].stub_lane, Some(2));
        assert_eq!(ghosts_on_c2(&g), [("x", 2)]);
        let start: Vec<(u16, Half, bool)> = segments(&g, 2).iter().filter(|s| s.from_lane == 2).map(|s| (s.to_lane, s.half, s.dotted)).collect();
        assert_eq!(start, [(2, Half::Bottom, true)]);
        assert!(segments(&g, 3).iter().any(|s| (s.from_lane, s.to_lane, s.half, s.dotted) == (2, 0, Half::Top, false)), "curving into c2");
        let ghost = &g.rows[3].ghosts[0];
        assert_eq!((ghost.tip.clone(), ghost.summary.as_str(), ghost.author_name.as_str(), ghost.author_time), (id(12).to_string(), "x12", "ada", 1200));
        assert!(g.rows.iter().filter(|r| r.summary != "c2").all(|r| r.ghosts.is_empty() && r.stub_lane.is_none()));
    }

    #[test]
    fn hidden_branches_on_one_commit_share_one_stub_and_its_colour() {
        let (req, commits) = two_off_main();
        let g = build(&with(req, view(vec![rule(PresetAction::Hide, "x, w")])), &commits).unwrap();
        assert_eq!(summaries(&g), ["c3", "c2", "Initial commit"]);
        assert_eq!(ghosts_on_c2(&g), [("w", 1), ("x", 1)], "newest tip first, in the stub's colour");
        // (h2) Ten from one commit still draw one stub.
        let (mut req, mut commits) = two_off_main();
        for n in 0..10u8 {
            commits.insert(id(40 + n), c(40 + n, &[2], 300 + i64::from(n), &format!("bump {n}")));
            req.input.refs.refs.push(local(&format!("dependabot/{n}"), 40 + n));
        }
        let g = build(&with(req, view(vec![rule(PresetAction::Hide, "x, w, dependabot/*")])), &commits).unwrap();
        assert_eq!(summaries(&g), ["c3", "c2", "Initial commit"]);
        assert_eq!(g.rows[1].ghosts.len(), 12);
        assert_eq!(g.rows[1].stub_lane, Some(1));
        let starts = segments(&g, 0).iter().filter(|s| s.half == Half::Bottom && s.dotted).count();
        assert_eq!((starts, g.max_lanes), (1, 2), "one stub, one lane");
    }

    #[test]
    fn no_stub_on_the_first_row() {
        // v, hidden, forks from c3, the first row: listed there, but with no row above for a stub.
        let (mut req, mut commits) = two_off_main();
        commits.insert(id(50), c(50, &[3], 950, "v50"));
        req.input.refs.refs.push(local("v", 50));
        let g = build(&with(req, view(vec![rule(PresetAction::Hide, "x, w, v")])), &commits).unwrap();
        assert_eq!(summaries(&g), ["c3", "c2", "Initial commit"]);
        assert_eq!((g.rows[0].ghosts.len(), g.rows[0].stub_lane), (1, None));
        assert_eq!(g.rows[0].ghosts[0].color, 1, "the lane right of its own");
    }

    #[test]
    fn no_ghosts_for_branches_hidden_by_hand() {
        let (req, commits) = two_off_main();
        let spec = ViewSpec { picks: BranchPicks { hidden: vec!["x".into()], ..Default::default() }, ..view(vec![rule(PresetAction::Hide, "w")]) };
        let g = build(&with(req, spec), &commits).unwrap();
        assert_eq!(ghosts_on_c2(&g), [("w", 1)]);
    }

    #[test]
    fn ghosts_for_branches_hidden_by_hand_with_the_option() {
        // x by name, the d/ folder (and so d/v, made in it) by hand; w by a rule.
        let (mut req, mut commits) = two_off_main();
        commits.insert(id(30), c(30, &[2], 1500, "v30"));
        req.input.refs.refs.push(local("d/v", 30));
        let picks = BranchPicks { hidden: vec!["x".into(), "d/".into()], ..Default::default() };
        let spec = ViewSpec { picks, ..view(vec![rule(PresetAction::Hide, "w")]) };
        let on = build(&with(req.clone(), ViewSpec { ghost_picks: true, ..spec.clone() }), &commits).unwrap();
        assert_eq!(summaries(&on), ["c3", "c2", "Initial commit"]);
        assert_eq!(ghosts_on_c2(&on), [("w", 1), ("d/v", 1), ("x", 1)], "newest tip first, in the stub's colour");
        let off = build(&with(req, spec), &commits).unwrap();
        assert_eq!(ghosts_on_c2(&off), [("w", 1)]);
    }

    #[test]
    fn no_ghosts_when_the_option_is_off() {
        let (req, commits) = two_off_main();
        let spec = ViewSpec { ghost_lines: false, ..view(vec![rule(PresetAction::Hide, "x, w")]) };
        let g = build(&with(req, spec), &commits).unwrap();
        assert!(g.rows.iter().all(|r| r.ghosts.is_empty()));
    }

    /// Random histories (branches forked anywhere, merged into main and into each other, stashes,
    /// worktrees, window cuts) under random presets, also with the linked worktree's branch and
    /// another hidden by hand: every graph's lines stay continuous (`build` runs the continuity
    /// check in tests) and no WIP loses its commit.
    #[test]
    fn random_histories_lay_out_under_any_preset() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rnd = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let names = ["main", "a", "b", "c", "d", "e"];
        let (mut runs, mut folds, mut ghosts, mut split, mut tails, mut stubs) = (0, 0, 0, 0, 0, 0);
        for _ in 0..400 {
            let mut commits = vec![c(1, &[], 100, "Initial commit")];
            let mut tips: Vec<Option<u8>> = vec![Some(1), None, None, None, None, None];
            let mut remotes: Vec<Option<u8>> = vec![None; names.len()];
            let mut last = 0;
            let mut stashes = vec![];
            let mut plain = vec![1u8];
            for n in 2..=70u8 {
                // Often the same branch again, so its lines grow long enough to collapse.
                let b = if rnd(2) == 0 { last } else { rnd(names.len()) };
                last = b;
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
                if rnd(12) == 0 {
                    stashes.push(n);
                    commits.push(c(n, &[base], i64::from(n) * 100, "On x: wip"));
                    continue;
                }
                commits.push(c(n, &parents, i64::from(n) * 100, &format!("{}{n}", names[b])));
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
            let mut wts = vec![worktree("/demo/x", tips[0].unwrap(), "main", true)];
            if let Some(i) = (1..names.len()).find(|&i| tips[i].is_some()) {
                wts.push(worktree("/demo/wt", tips[i].unwrap(), names[i], false));
            }
            let limit = [10, 30, 100][rnd(3)];
            let (mut req, store) = request(refs, wts, commits, limit);
            req.input.wip = (0..req.input.worktrees.len()).map(|w| (w, Default::default())).collect();
            req.input.refs.stashes = stashes.iter().enumerate().map(|(i, &n)| StashIn { index: i, id: id(n), message: "On x: wip".into(), time: i64::from(n) * 100 }).collect();
            for _ in 0..4 {
                let actions = [PresetAction::Show, PresetAction::Hide, PresetAction::Collapse];
                let rules = names[1..].iter().map(|n| rule(actions[rnd(3)], n)).collect();
                let spec = ViewSpec {
                    otherwise: actions[rnd(3)],
                    fold_merged: rnd(2) == 0,
                    placement: [CollapsePlacement::KeepDateOrder, CollapsePlacement::PullTogether][rnd(2)],
                    ..view(rules)
                };
                // Also with the linked worktree's branch hidden by hand (its WIP goes with it), and
                // another branch: what it shares with a collapsed one is then that one's alone. Their
                // stubs, too, half the time.
                let linked = req.input.worktrees.get(1).and_then(|w| w.branch.as_deref()?.strip_prefix("refs/heads/"));
                let hidden = linked.into_iter().chain([names[1 + rnd(names.len() - 1)]]).map(String::from).collect();
                let hiding = ViewSpec { picks: BranchPicks { hidden, ..Default::default() }, ghost_picks: rnd(2) == 0, ..spec.clone() };
                for (spec, hides) in [(&hiding, true), (&spec, false)] {
                    let g = build(&with(req.clone(), spec.clone()), &store).unwrap_or_else(|e| panic!("{e}\n{spec:?}"));
                    let ids: Vec<&str> = g.rows.iter().map(|r| r.id.as_str()).collect();
                    for w in g.rows.iter().filter(|r| r.kind == NodeKind::Wip) {
                        assert!(ids.contains(&w.parents[0].as_str()), "a WIP docks on a row: {spec:?}");
                        assert!(!hides || w.id != "wip:/demo/wt", "a hidden branch's worktree has no WIP row: {spec:?}");
                    }
                    if hides {
                        continue;
                    }
                    runs += g.rows.iter().filter(|r| r.collapsed.is_some()).count();
                    let mut branches = std::collections::HashSet::new();
                    split += g.rows.iter().filter_map(|r| r.collapsed.as_ref()).filter(|c| !branches.insert(&c.branch)).count();
                    folds += g.rows.iter().filter(|r| r.fold.is_some()).count();
                    ghosts += g.rows.iter().map(|r| r.ghosts.len()).sum::<usize>();
                    tails += g.rows.iter().filter(|r| r.fold.as_ref().is_some_and(|f| f.tail_lane.is_some())).count();
                    stubs += g.rows.iter().filter(|r| r.stub_lane.is_some()).count();
                }
            }
        }
        assert!(runs > 20 && folds > 20 && ghosts > 20 && split > 5 && tails > 20 && stubs > 20, "the cases cover runs ({runs}), branches with several ({split}), folds ({folds}), their tails ({tails}), hidden branches ({ghosts}) and stubs ({stubs})");
    }

    #[test]
    fn preview_counts_rules_and_states() {
        let (mut req, commits) = merged_and_more();
        req.input.refs.refs.push(local("v", 26));
        let spec = ViewSpec { fold_merged: true, ..view(vec![rule(PresetAction::Collapse, "y"), rule(PresetAction::Hide, "z"), rule(PresetAction::Hide, "q")]) };
        let p = build_preview(&with(req.clone(), spec), &commits);
        // Everything else: main (the trunk), x (folded) and v (shown).
        assert_eq!(p, PresetPreview { rules: vec![1, 1, 0, 3], shown: 2, collapsed: 1, hidden: 1, folded: 1 });
        assert_eq!(p.rules.iter().sum::<u32>(), 5, "every branch counted once");
        assert_eq!(build_preview(&req, &commits), PresetPreview::default(), "no view: nothing");
    }
}
