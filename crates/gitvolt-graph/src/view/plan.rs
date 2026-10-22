//! Which commits a preset shows, collapses into a ⋮ row, folds into a trunk merge, and hides: one
//! pass over the window in walk order (children first), then one over the commits' reach sets.

use crate::payload::{BranchState, DecidedBy};
use crate::presets::{BranchUnit, CollapsePlacement, Decision, PresetAction, ReachSets};
use crate::view::ViewSpec;
use crate::walk::CommitMeta;
use crate::Oid;
use std::collections::{HashMap, HashSet};

/// A collapsed run needs at least this many commits between its newest and its oldest.
pub(crate) const MIN_COLLAPSED: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommitState {
    /// A row (a run's newest and oldest commits are rows too).
    Visible,
    /// In the middle of collapsed run `run` (an index into `ViewPlan::runs`): its ⋮ row stands
    /// for it.
    Run { run: usize },
    /// Folded into the trunk merge at walk index `merge`.
    Folded { merge: usize },
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Run {
    pub unit: usize,
    pub newest: usize,
    /// In walk order, stashes on them included.
    pub middle: Vec<usize>,
    pub oldest: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fold {
    pub merge: usize,
    /// In walk order.
    pub commits: Vec<usize>,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ViewPlan {
    pub state: Vec<CommitState>,
    /// One per collapsed line of history (a unit can have several), in unit order, then walk
    /// order of their newest commits.
    pub runs: Vec<Run>,
    /// In walk order of their merges.
    pub folds: Vec<Fold>,
    pub unit_state: Vec<BranchState>,
}

/// What `plan` reads from the window, besides the spec.
pub(crate) struct Window<'a> {
    pub commits: &'a [CommitMeta],
    pub index: &'a HashMap<Oid, usize>,
    pub units: &'a [BranchUnit],
    pub decisions: &'a [Decision],
    pub reach: &'a ReachSets,
    /// Per commit: on a trunk branch's first-parent line.
    pub trunk_fp: &'a [bool],
    /// Detached worktree HEADs (walk indexes): always rows.
    pub detached_heads: &'a [usize],
    /// Every worktree's HEAD (walk indexes): a WIP row docks there, so a run never hides one.
    pub worktree_heads: &'a [usize],
    /// Per unit: checked out in a worktree (its Show unfolds it, as an explicit one does).
    pub checked_out: &'a [bool],
    /// Per commit: a stash, which goes wherever its base commit goes.
    pub stash: &'a [bool],
}

/// Decides every commit's state (Ruling 2, 5, 6):
/// - Shown units (and the trunk), detached HEADs and commits no branch reaches keep their commits
///   visible, down through every parent but a trunk merge's folded ones.
/// - With `fold_merged`, a merge on a trunk branch's first-parent line folds the commits only its
///   other parents reach (the oldest such merge holds a commit two of them reach), unless a shown
///   or collapsed branch needs them; a unit whose tips are all folded is `Folded`.
/// - A commit only one collapsed unit reaches (hidden units aside) is in one of its runs, one per
///   line of history (`lines`: diverged local and remote branches collapse apart, and no tip is
///   inside a run); one that two non-hidden units reach stays visible; the rest is hidden.
/// - A run keeps its newest and oldest commits as rows; one with fewer than `MIN_COLLAPSED`
///   commits between them, or one hiding a worktree's HEAD, isn't collapsed.
pub(crate) fn plan(w: &Window, spec: &ViewSpec) -> ViewPlan {
    let n = w.commits.len();
    let mut tips_at: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (u, unit) in w.units.iter().enumerate() {
        for t in &unit.tips {
            if let Some(&c) = w.index.get(t)
                && !tips_at[c].contains(&u)
            {
                tips_at[c].push(u);
            }
        }
    }
    let unfolded: HashSet<Oid> = spec.expanded.iter().filter_map(|k| Oid::from_hex(k.strip_prefix("fold:")?)).collect();
    let mut is_detached = vec![false; n];
    for &c in w.detached_heads {
        is_detached[c] = true;
    }

    // `shown`: a shown branch (or the trunk, a detached HEAD, no branch at all) keeps it;
    // `kept`: a collapsed branch needs it (so no merge folds it); `branch`: some branch reaches
    // it; `fold`: the oldest trunk merge folding it.
    let mut shown = vec![false; n];
    let mut kept = vec![false; n];
    let mut branch = vec![false; n];
    let mut fold: Vec<Option<usize>> = vec![None; n];
    for c in 0..n {
        if w.trunk_fp[c] || is_detached[c] {
            shown[c] = true;
        }
        for &u in &tips_at[c] {
            branch[c] = true;
            let d = &w.decisions[u];
            let explicit = d.by != DecidedBy::Otherwise || w.checked_out[u];
            if d.action == PresetAction::Show && (explicit || fold[c].is_none()) {
                shown[c] = true;
            }
        }
        for &u in &tips_at[c] {
            if w.decisions[u].action == PresetAction::Collapse && (fold[c].is_none() || shown[c]) {
                kept[c] = true;
            }
        }
        if !branch[c] && !w.stash[c] {
            shown[c] = true;
        }
        if w.stash[c] {
            continue;
        }
        let commit = &w.commits[c];
        let fold_here = spec.fold_merged && w.trunk_fp[c] && !unfolded.contains(&commit.id);
        for (k, p) in commit.parents.iter().enumerate() {
            let Some(&p) = w.index.get(p) else { continue };
            branch[p] |= branch[c];
            if fold_here && k > 0 {
                fold[p] = fold[p].max(Some(c));
            } else {
                shown[p] |= shown[c];
                kept[p] |= kept[c];
                if !shown[c] {
                    fold[p] = fold[p].max(fold[c]);
                }
            }
        }
    }

    let hidden = |u: usize| w.decisions[u].action == PresetAction::Hide;
    let mut state = vec![CommitState::Hidden; n];
    // The collapsed unit a commit is a run member of: `Run` or `Visible` once its line is known.
    let mut member: Vec<Option<usize>> = vec![None; n];
    for c in (0..n).filter(|&c| !w.stash[c]) {
        state[c] = if shown[c] {
            CommitState::Visible
        } else if let Some(merge) = fold[c] {
            if kept[c] { CommitState::Visible } else { CommitState::Folded { merge } }
        } else {
            let mut keeping = w.reach.units_reaching(c).filter(|&u| !hidden(u));
            match (keeping.next(), keeping.next()) {
                (None, _) => CommitState::Hidden,
                (Some(u), None) if w.decisions[u].action == PresetAction::Collapse => {
                    member[c] = Some(u);
                    CommitState::Visible
                }
                _ => CommitState::Visible,
            }
        };
    }

    let mut is_head = vec![false; n];
    for &c in w.worktree_heads {
        is_head[c] = true;
    }
    let mut runs = Vec::new();
    for line in lines(w, &member) {
        let (newest, oldest) = (line[0], line[line.len() - 1]);
        let middle = line.get(1..line.len() - 1).unwrap_or_default();
        if middle.len() < MIN_COLLAPSED || middle.iter().any(|&c| is_head[c]) {
            continue;
        }
        for &c in middle {
            state[c] = CommitState::Run { run: runs.len() };
        }
        runs.push(Run { unit: member[newest].expect("a line's commits are members"), newest, middle: middle.to_vec(), oldest });
    }
    keep_lines_downward(w, &mut state, &mut runs, spec.placement == CollapsePlacement::PullTogether);

    // Stashes follow their base: into its run's middle (on the oldest too, which a pulled-together
    // run moves up past the stash), its fold, or out of sight.
    let mut added = vec![false; runs.len()];
    for c in (0..n).filter(|&c| w.stash[c]) {
        let base = w.commits[c].parents.first().and_then(|p| w.index.get(p)).copied();
        let into = match base.map(|b| (b, state[b])) {
            None => None,
            Some((b, CommitState::Visible)) => runs.iter().position(|r| r.oldest == b),
            Some((_, CommitState::Run { run })) => Some(run),
            Some((_, s)) => {
                state[c] = s;
                continue;
            }
        };
        state[c] = match into {
            Some(run) => {
                runs[run].middle.push(c);
                added[run] = true;
                CommitState::Run { run }
            }
            None => CommitState::Visible,
        };
    }
    for (r, _) in runs.iter_mut().zip(&added).filter(|(_, a)| **a) {
        r.middle.sort_unstable();
    }

    let mut held: HashMap<usize, Vec<usize>> = HashMap::new();
    for (c, s) in state.iter().enumerate() {
        if let CommitState::Folded { merge } = *s {
            held.entry(merge).or_default().push(c);
        }
    }
    let mut merges: Vec<usize> = held.keys().copied().collect();
    merges.sort_unstable();
    let folds = merges
        .into_iter()
        .map(|merge| {
            let commits = held.remove(&merge).unwrap_or_default();
            let branch = fold_branch(w, &tips_at, merge, &commits);
            Fold { merge, commits, branch }
        })
        .collect();

    let mut unit_state: Vec<BranchState> = w
        .decisions
        .iter()
        .map(|d| match d.action {
            PresetAction::Show => BranchState::Shown,
            PresetAction::Hide => BranchState::Hidden,
            PresetAction::Collapse => BranchState::Collapsed,
        })
        .collect();
    for (u, s) in unit_state.iter_mut().enumerate().filter(|(u, _)| !hidden(*u)) {
        let mut tips = w.units[u].tips.iter().filter_map(|t| w.index.get(t)).peekable();
        if tips.peek().is_some() && tips.all(|&c| matches!(state[c], CommitState::Folded { .. })) {
            *s = BranchState::Folded;
        }
    }
    ViewPlan { state, runs, folds, unit_state }
}

/// A collapsed unit's run members split into lines of history, each in walk order: its first
/// commit is the line's newest, its last the oldest. A line starts at each of the unit's tips and
/// wherever one tip's first-parent line reaches another's (the fork point of diverged local and
/// remote branches, a local branch behind its remote, a pull that merged the remote's commit),
/// and follows first parents down to the next start; a branch merged into a line (reached through
/// a second parent only) goes with the line that merged it. So no tip is ever inside another
/// line, and a unit with one tip is one line. Units in order, their lines in walk order of their
/// starts.
fn lines(w: &Window, member: &[Option<usize>]) -> Vec<Vec<usize>> {
    let mut by_unit: HashMap<usize, Vec<usize>> = HashMap::new();
    for (c, u) in member.iter().enumerate() {
        if let Some(u) = *u {
            by_unit.entry(u).or_default().push(c);
        }
    }
    let mut units: Vec<usize> = by_unit.keys().copied().collect();
    units.sort_unstable();
    let first_parent = |c: usize| w.commits[c].parents.first().and_then(|p| w.index.get(p)).copied();
    let mut out = Vec::new();
    for unit in units {
        let mine = |c: usize| member[c] == Some(unit);
        let mut tips: Vec<usize> = w.units[unit].tips.iter().filter_map(|t| w.index.get(t).copied()).filter(|&c| mine(c)).collect();
        tips.sort_unstable();
        tips.dedup();
        // Each tip's first-parent line (`on_line`), newest tip first, down to where it meets a line
        // already drawn. A commit one line reaches from another (where they fork, or one merged
        // into the other) starts a line, so a line's ⋮ row never reaches into another's middle.
        let mut on_line: HashMap<usize, usize> = HashMap::new();
        for (k, &t) in tips.iter().enumerate() {
            let mut cur = Some(t);
            while let Some(c) = cur.filter(|&c| mine(c) && !on_line.contains_key(&c)) {
                on_line.insert(c, k);
                cur = first_parent(c);
            }
        }
        let mut starts: HashSet<usize> = tips.iter().copied().collect();
        for (&c, &k) in &on_line {
            let crossing = w.commits[c].parents.iter().filter_map(|p| w.index.get(p)).filter(|p| on_line.get(p).is_some_and(|&j| j != k));
            starts.extend(crossing);
        }
        // In walk order, a commit no line has reached yet starts one (a start, or a stray with no
        // member child); each passes its line on to its first parent on the same tip line, and to
        // any parent on no tip line.
        let mut line_of: HashMap<usize, usize> = HashMap::new();
        for &c in &by_unit[&unit] {
            let l = match line_of.get(&c) {
                Some(&l) => l,
                None => {
                    out.push(Vec::new());
                    out.len() - 1
                }
            };
            out[l].push(c);
            for (k, p) in w.commits[c].parents.iter().enumerate() {
                let Some(&p) = w.index.get(p) else { continue };
                let follows = match on_line.get(&p) {
                    None => true,
                    Some(pl) => k == 0 && on_line.get(&c) == Some(pl),
                };
                if follows && mine(p) && !starts.contains(&p) && !line_of.contains_key(&p) {
                    line_of.insert(p, l);
                }
            }
        }
    }
    out
}

/// A ⋮ row sits right under its run's newest commit, so a row reaching into another run's middle
/// from below that run's ⋮ row would draw its line upward (lines of one branch merged into each
/// other in a way `lines` doesn't split): that run isn't collapsed. Never the case for a branch
/// with one line.
fn keep_lines_downward(w: &Window, state: &mut [CommitState], runs: &mut Vec<Run>, pull: bool) {
    // Runs are in unit order: one run per unit means one line each.
    if runs.windows(2).all(|p| p[0].unit != p[1].unit) {
        return;
    }
    let mut kept = vec![true; runs.len()];
    loop {
        let pulled: HashMap<usize, usize> = if pull { runs.iter().zip(&kept).filter(|(_, k)| **k).map(|(r, _)| (r.oldest, r.newest)).collect() } else { HashMap::new() };
        // A row's place: a commit's own (a pulled-together oldest's under its ⋮ row), a run's ⋮ row's.
        let place = |c: usize| match state[c] {
            CommitState::Visible => Some(pulled.get(&c).map_or((c, 0), |&newest| (newest, 2))),
            CommitState::Run { run } => Some((runs[run].newest, 1)),
            CommitState::Folded { .. } | CommitState::Hidden => None,
        };
        let upward = (0..state.len()).find_map(|c| {
            let from = place(c)?;
            let own = match state[c] {
                CommitState::Run { run } => Some(run),
                _ => None,
            };
            w.commits[c].parents.iter().filter_map(|p| w.index.get(p)).find_map(|&p| match state[p] {
                CommitState::Run { run } if Some(run) != own && place(p)? <= from => Some(run),
                _ => None,
            })
        });
        let Some(r) = upward else { break };
        kept[r] = false;
        for &c in &runs[r].middle {
            state[c] = CommitState::Visible;
        }
    }
    if kept.iter().all(|k| *k) {
        return;
    }
    let mut next = 0;
    let renumber: Vec<usize> = kept.iter().map(|&k| if k { next += 1; next - 1 } else { usize::MAX }).collect();
    for s in state.iter_mut() {
        if let CommitState::Run { run } = s {
            *run = renumber[*run];
        }
    }
    let mut keep = kept.iter();
    runs.retain(|_| *keep.next().expect("one per run"));
}

/// A fold's branch: the unit whose tip is the merge's second parent, else one whose tip is among
/// the folded commits; else the one the merge message names; else "merged branch".
fn fold_branch(w: &Window, tips_at: &[Vec<usize>], merge: usize, commits: &[usize]) -> String {
    let second = w.commits[merge].parents.get(1).and_then(|p| w.index.get(p)).copied();
    let unit = second.and_then(|c| tips_at[c].first()).or_else(|| commits.iter().find_map(|&c| tips_at[c].first()));
    match unit {
        Some(&u) => w.units[u].name.clone(),
        None => merged_branch_name(&w.commits[merge].summary).unwrap_or_else(|| "merged branch".into()),
    }
}

/// The branch a merge's summary names: `Merge branch 'x'` (and `… into 'main'`), `Merge pull
/// request #n from owner/x`, `Merge remote-tracking branch 'origin/x'`.
pub(crate) fn merged_branch_name(summary: &str) -> Option<String> {
    let quoted = |rest: &str| rest.strip_prefix('\'').and_then(|r| r.split_once('\'')).map(|(name, _)| name.to_string());
    let after_slash = |s: &str| s.split_once('/').map(|(_, b)| b.to_string()).filter(|b| !b.is_empty());
    if let Some(rest) = summary.strip_prefix("Merge branch ") {
        return quoted(rest).filter(|b| !b.is_empty());
    }
    if let Some(rest) = summary.strip_prefix("Merge remote-tracking branch ") {
        return quoted(rest).and_then(|r| after_slash(&r));
    }
    if let Some(rest) = summary.strip_prefix("Merge pull request #") {
        let (_, from) = rest.split_once(" from ")?;
        return after_slash(from.split_whitespace().next()?);
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::presets::PresetAction::{Collapse, Hide, Show};
    use crate::testkit::{commit, id, reach};

    /// A window by hand: commits in walk order (children first), units as (name, tip numbers),
    /// decisions, and per commit (walk index) the units reaching it and whether the trunk does.
    pub(crate) struct Hand {
        pub commits: Vec<CommitMeta>,
        pub index: HashMap<Oid, usize>,
        pub units: Vec<BranchUnit>,
        pub decisions: Vec<Decision>,
        pub reach: ReachSets,
        pub trunk_fp: Vec<bool>,
        pub detached: Vec<usize>,
        pub heads: Vec<usize>,
        pub checked_out: Vec<bool>,
        pub stash: Vec<bool>,
    }

    pub(crate) fn unit(name: &str, tips: &[u8]) -> BranchUnit {
        BranchUnit { name: name.into(), refs: vec![format!("refs/heads/{name}")], tips: tips.iter().map(|&t| id(t)).collect(), has_local: true }
    }

    pub(crate) fn by_rule(action: PresetAction) -> Decision {
        Decision { action, by: DecidedBy::Rule { index: 0 }, rule: Some(0) }
    }

    pub(crate) fn otherwise(action: PresetAction) -> Decision {
        Decision { action, by: DecidedBy::Otherwise, rule: None }
    }

    pub(crate) fn trunk() -> Decision {
        Decision { action: Show, by: DecidedBy::Trunk, rule: None }
    }

    impl Hand {
        /// `units[0]` is the trunk: its first-parent line from its tips is `trunk_fp`, and every
        /// commit it reaches is trunk-reachable. Reach sets come from the units' tips.
        pub(crate) fn new(commits: Vec<CommitMeta>, units: Vec<BranchUnit>, decisions: Vec<Decision>) -> Hand {
            let index: HashMap<Oid, usize> = commits.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
            let n = commits.len();
            let parents = |c: usize| -> Vec<usize> { commits[c].parents.iter().filter_map(|p| index.get(p).copied()).collect() };
            let reaching = |u: &BranchUnit| -> Vec<bool> {
                let mut r = vec![false; n];
                let mut stack: Vec<usize> = u.tips.iter().filter_map(|t| index.get(t).copied()).collect();
                while let Some(c) = stack.pop() {
                    if !std::mem::replace(&mut r[c], true) {
                        stack.extend(parents(c));
                    }
                }
                r
            };
            let trunk_reach = units.first().map(reaching).unwrap_or_else(|| vec![false; n]);
            let per_unit: Vec<Vec<bool>> = units.iter().map(reaching).collect();
            let per_commit: Vec<Vec<usize>> = (0..n).map(|c| (0..units.len()).filter(|&u| per_unit[u][c]).collect()).collect();
            let per_commit_refs: Vec<&[usize]> = per_commit.iter().map(Vec::as_slice).collect();
            let mut trunk_fp = vec![false; n];
            for t in units.first().map(|u| u.tips.clone()).unwrap_or_default() {
                let mut cur = index.get(&t).copied();
                while let Some(c) = cur.filter(|&c| !trunk_fp[c]) {
                    trunk_fp[c] = true;
                    cur = commits[c].parents.first().and_then(|p| index.get(p)).copied();
                }
            }
            let units_len = units.len();
            Hand { reach: reach(&per_commit_refs, &trunk_reach), index, units, decisions, trunk_fp, detached: vec![], heads: vec![], checked_out: vec![false; units_len], stash: vec![false; n], commits }
        }

        pub(crate) fn window(&self) -> Window<'_> {
            Window {
                commits: &self.commits,
                index: &self.index,
                units: &self.units,
                decisions: &self.decisions,
                reach: &self.reach,
                trunk_fp: &self.trunk_fp,
                detached_heads: &self.detached,
                worktree_heads: &self.heads,
                checked_out: &self.checked_out,
                stash: &self.stash,
            }
        }

        pub(crate) fn plan(&self, spec: &ViewSpec) -> ViewPlan {
            plan(&self.window(), spec)
        }

        pub(crate) fn at(&self, n: u8) -> usize {
            self.index[&id(n)]
        }

        /// The states of commits `ns`, by number.
        pub(crate) fn states(&self, p: &ViewPlan, ns: &[u8]) -> Vec<CommitState> {
            ns.iter().map(|&n| p.state[self.at(n)]).collect()
        }
    }

    pub(crate) fn c(n: u8, parents: &[u8], message: &str) -> CommitMeta {
        commit(n, parents, i64::from(n) * 100, "ada@example.com", message)
    }

    /// A linear branch from `from`: commits `first..=last`, newest last.
    pub(crate) fn chain(from: u8, first: u8, last: u8) -> Vec<CommitMeta> {
        (first..=last).map(|n| c(n, &[if n == first { from } else { n - 1 }], &format!("c{n}"))).collect()
    }

    /// Walk order: newest (highest number) first.
    pub(crate) fn newest_first(mut cs: Vec<CommitMeta>) -> Vec<CommitMeta> {
        cs.sort_by_key(|c| std::cmp::Reverse(c.committer_time));
        cs
    }

    fn spec() -> ViewSpec {
        ViewSpec::default()
    }

    fn folding() -> ViewSpec {
        ViewSpec { fold_merged: true, ..ViewSpec::default() }
    }

    use CommitState::{Folded, Hidden, Run as InRun, Visible};

    /// main: 1 ← 2 ← 3; x from 2: 10..=(9 + len).
    fn branch_off_main(len: u8, action: PresetAction) -> Hand {
        let mut cs = vec![c(1, &[], "Initial commit"), c(2, &[1], "c2"), c(3, &[2], "c3")];
        cs.extend(chain(2, 10, 9 + len));
        Hand::new(newest_first(cs), vec![unit("main", &[3]), unit("x", &[9 + len])], vec![trunk(), by_rule(action)])
    }

    #[test]
    fn a_hidden_branch_drops_out_and_its_fork_point_stays() {
        let h = branch_off_main(3, Hide);
        let p = h.plan(&spec());
        assert_eq!(h.states(&p, &[10, 11, 12]), [Hidden; 3]);
        assert_eq!(h.states(&p, &[1, 2, 3]), [Visible; 3]);
        assert_eq!(p.unit_state, [BranchState::Shown, BranchState::Hidden]);
        assert!(p.runs.is_empty() && p.folds.is_empty());
    }

    #[test]
    fn a_collapsed_branch_keeps_its_newest_and_oldest() {
        let h = branch_off_main(6, Collapse);
        let p = h.plan(&spec());
        assert_eq!(p.runs, [Run { unit: 1, newest: h.at(15), middle: [14, 13, 12, 11].map(|n| h.at(n)).to_vec(), oldest: h.at(10) }]);
        assert_eq!(h.states(&p, &[15, 14, 11, 10]), [Visible, InRun { run: 0 }, InRun { run: 0 }, Visible]);
        assert_eq!(p.unit_state[1], BranchState::Collapsed);
    }

    /// main: 1 ← 2 ← 3; x's shared commits 10..=15 from 2, then its local branch 20..=25 and its
    /// remote one 30..=35 (newer), both from 15.
    fn diverged() -> Hand {
        let mut cs = vec![c(1, &[], "Initial commit"), c(2, &[1], "c2"), c(3, &[2], "c3")];
        cs.extend(chain(2, 10, 15));
        cs.extend(chain(15, 20, 25));
        cs.extend(chain(15, 30, 35));
        Hand::new(newest_first(cs), vec![unit("main", &[3]), unit("x", &[25, 35])], vec![trunk(), by_rule(Collapse)])
    }

    #[test]
    fn diverged_local_and_remote_branches_collapse_apart() {
        let h = diverged();
        let p = h.plan(&spec());
        let run = |newest: u8, middle: [u8; 4], oldest: u8| Run { unit: 1, newest: h.at(newest), middle: middle.map(|n| h.at(n)).to_vec(), oldest: h.at(oldest) };
        assert_eq!(p.runs, [run(35, [34, 33, 32, 31], 30), run(25, [24, 23, 22, 21], 20), run(15, [14, 13, 12, 11], 10)], "each side, then the shared part");
        assert_eq!(h.states(&p, &[35, 30, 25, 20, 15, 10]), [Visible; 6], "both tips, each run's ends");
        assert_eq!(h.states(&p, &[34, 24, 14]), [InRun { run: 0 }, InRun { run: 1 }, InRun { run: 2 }]);
        // Sides too short to collapse stay rows; the shared part still collapses.
        let mut cs = vec![c(1, &[], "Initial commit"), c(2, &[1], "c2"), c(3, &[2], "c3")];
        cs.extend(chain(2, 10, 15));
        cs.extend(chain(15, 20, 21));
        cs.extend(chain(15, 30, 31));
        let h = Hand::new(newest_first(cs), vec![unit("main", &[3]), unit("x", &[21, 31])], vec![trunk(), by_rule(Collapse)]);
        let p = h.plan(&spec());
        assert_eq!(p.runs.iter().map(|r| (r.newest, r.oldest)).collect::<Vec<_>>(), [(h.at(15), h.at(10))]);
        assert_eq!(h.states(&p, &[31, 30, 21, 20]), [Visible; 4]);
    }

    #[test]
    fn a_local_branch_behind_its_remote_keeps_its_tip_a_row() {
        // x: 10..=15 from 2, its remote one commit ahead at 16.
        let mut cs = vec![c(1, &[], "Initial commit"), c(2, &[1], "c2"), c(3, &[2], "c3")];
        cs.extend(chain(2, 10, 16));
        let h = Hand::new(newest_first(cs), vec![unit("main", &[3]), unit("x", &[15, 16])], vec![trunk(), by_rule(Collapse)]);
        let p = h.plan(&spec());
        assert_eq!(p.runs, [Run { unit: 1, newest: h.at(15), middle: [14, 13, 12, 11].map(|n| h.at(n)).to_vec(), oldest: h.at(10) }]);
        assert_eq!(h.states(&p, &[16, 15, 10]), [Visible; 3]);
        // Ahead of it instead: the remote's tip is a row, and starts a line of its own.
        let mut cs = vec![c(1, &[], "Initial commit"), c(2, &[1], "c2"), c(3, &[2], "c3")];
        cs.extend(chain(2, 10, 19));
        let h = Hand::new(newest_first(cs), vec![unit("main", &[3]), unit("x", &[19, 13])], vec![trunk(), by_rule(Collapse)]);
        let p = h.plan(&spec());
        assert_eq!(p.runs.iter().map(|r| (r.newest, r.oldest)).collect::<Vec<_>>(), [(h.at(19), h.at(14))], "13, 12, 11, 10: too few");
        assert_eq!(h.states(&p, &[14, 13, 12, 11, 10]), [Visible; 5]);
    }

    #[test]
    fn a_line_merged_into_another_splits_where_it_is_merged() {
        // The remote line: 20..=23 from x's shared 15, merged into the local line at 33 (30..=37),
        // then on to 40..=44, newer than the local tip.
        let mut cs = vec![c(1, &[], "Initial commit"), c(2, &[1], "c2"), c(3, &[2], "c3")];
        cs.extend(chain(2, 10, 15));
        cs.extend(chain(15, 20, 23));
        cs.extend(chain(15, 30, 37));
        let at = cs.len() - 5;
        cs[at] = c(33, &[32, 23], "Merge remote-tracking branch 'origin/x' into x");
        cs.extend(chain(23, 40, 44));
        let h = Hand::new(newest_first(cs), vec![unit("main", &[3]), unit("x", &[37, 44])], vec![trunk(), by_rule(Collapse)]);
        let p = h.plan(&spec());
        let ends: Vec<(usize, usize)> = p.runs.iter().map(|r| (r.newest, r.oldest)).collect();
        assert_eq!(ends, [(h.at(44), h.at(40)), (h.at(37), h.at(30)), (h.at(15), h.at(10))], "23, where the local line merged it, starts one of its own (too short)");
        assert_eq!(h.states(&p, &[23, 22, 21, 20]), [Visible; 4]);
    }

    #[test]
    fn a_run_whose_middle_a_row_below_its_dots_reaches_stays_rows() {
        // The remote line: 20..=23 from x's shared 15, then 60..=63, the newest. A side branch
        // (30, 31) from its 22 is merged into the local line (40..=47) at 44; the side branch's
        // 30 ends up the local run's oldest row, below the remote's ⋮ row, and its parent 22 would
        // be in that run.
        let mut cs = vec![c(1, &[], "Initial commit"), c(2, &[1], "c2"), c(3, &[2], "c3")];
        cs.extend(chain(2, 10, 15));
        cs.extend(chain(15, 20, 23));
        cs.extend(chain(22, 30, 31));
        cs.extend(chain(15, 40, 47));
        let at = cs.len() - 4;
        cs[at] = c(44, &[43, 31], "Merge branch 'side' into x");
        cs.extend(chain(23, 60, 63));
        let h = Hand::new(newest_first(cs), vec![unit("main", &[3]), unit("x", &[47, 63])], vec![trunk(), by_rule(Collapse)]);
        let p = h.plan(&spec());
        let ends: Vec<(usize, usize)> = p.runs.iter().map(|r| (r.newest, r.oldest)).collect();
        assert_eq!(ends, [(h.at(47), h.at(30)), (h.at(15), h.at(10))]);
        assert_eq!(h.states(&p, &[63, 62, 61, 60, 23, 22, 21, 20]), [Visible; 8]);
        assert_eq!(h.states(&p, &[44, 31, 11]), [InRun { run: 0 }, InRun { run: 0 }, InRun { run: 1 }], "renumbered");
    }

    #[test]
    fn a_short_branch_is_not_collapsed() {
        let h = branch_off_main(4, Collapse);
        let p = h.plan(&spec());
        assert!(p.runs.is_empty());
        assert_eq!(h.states(&p, &[10, 11, 12, 13]), [Visible; 4]);
        assert_eq!(p.unit_state[1], BranchState::Collapsed, "still collapsed by its decision");
    }

    /// x: 10..=15 from 2; y on top of x's 13: 20..=25.
    fn stacked(x: PresetAction, y: PresetAction) -> Hand {
        let mut cs = vec![c(1, &[], "Initial commit"), c(2, &[1], "c2"), c(3, &[2], "c3")];
        cs.extend(chain(2, 10, 15));
        cs.extend(chain(13, 20, 25));
        Hand::new(newest_first(cs), vec![unit("main", &[3]), unit("x", &[15]), unit("y", &[25])], vec![trunk(), by_rule(x), by_rule(y)])
    }

    #[test]
    fn commits_shared_by_two_collapsed_branches_stay_visible() {
        let h = stacked(Collapse, Collapse);
        let p = h.plan(&spec());
        assert_eq!(h.states(&p, &[10, 11, 12, 13]), [Visible; 4], "x and y both need them");
        assert_eq!(p.runs.iter().map(|r| r.unit).collect::<Vec<_>>(), [2], "x's own three (14, 15) are too few");
        assert_eq!(h.states(&p, &[14, 15]), [Visible; 2]);
        assert_eq!(p.runs[0].middle, [24, 23, 22, 21].map(|n| h.at(n)));
    }

    #[test]
    fn a_collapsed_branch_under_a_shown_branch_keeps_the_shared_commits() {
        let h = stacked(Collapse, Show);
        let p = h.plan(&spec());
        assert_eq!(h.states(&p, &[10, 11, 12, 13]), [Visible; 4], "y needs them");
        assert!(p.runs.is_empty(), "x keeps only 14 and 15 of its own");
        // A hidden branch on top doesn't keep them: x collapses all six.
        let h = stacked(Collapse, Hide);
        let p = h.plan(&spec());
        assert_eq!(p.runs[0].middle, [14, 13, 12, 11].map(|n| h.at(n)));
        assert_eq!(h.states(&p, &[20, 25]), [Hidden; 2]);
    }

    /// main: 1 ← 2 ← M(30) merging x (10..=12 from 1) ← 31.
    fn merged(x_tip: Option<u8>, x: Decision, message: &str) -> Hand {
        let mut cs = vec![c(1, &[], "Initial commit"), c(2, &[1], "c2"), c(30, &[2, 12], message), c(31, &[30], "c31")];
        cs.extend(chain(1, 10, 12));
        let mut units = vec![unit("main", &[31])];
        let mut decisions = vec![trunk()];
        if let Some(t) = x_tip {
            units.push(unit("feature/x", &[t]));
            decisions.push(x);
        }
        Hand::new(newest_first(cs), units, decisions)
    }

    #[test]
    fn a_merged_branch_folds_into_its_trunk_merge() {
        let h = merged(Some(12), otherwise(Show), "Merge branch 'other'");
        let p = h.plan(&folding());
        assert_eq!(h.states(&p, &[10, 11, 12]), [Folded { merge: h.at(30) }; 3]);
        assert_eq!(p.folds, [Fold { merge: h.at(30), commits: [12, 11, 10].map(|n| h.at(n)).to_vec(), branch: "feature/x".into() }], "named from the ref");
        assert_eq!(p.unit_state, [BranchState::Shown, BranchState::Folded]);
        assert_eq!(h.states(&merged(Some(12), otherwise(Show), "m").plan(&spec()), &[10, 11, 12]), [Visible; 3], "nothing folds without the option");
    }

    #[test]
    fn a_deleted_merged_branch_is_named_from_the_merge_message() {
        for (message, name) in [
            ("Merge branch 'feature/trade-ui'", "feature/trade-ui"),
            ("Merge branch 'feature/trade-ui' into 'main'", "feature/trade-ui"),
            ("Merge pull request #42 from octo/feature/trade-ui", "feature/trade-ui"),
            ("Merge remote-tracking branch 'origin/feature/trade-ui'", "feature/trade-ui"),
            ("Fold in the trade screen", "merged branch"),
        ] {
            let p = merged(None, trunk(), message).plan(&folding());
            assert_eq!(p.folds.iter().map(|f| f.branch.as_str()).collect::<Vec<_>>(), [name], "{message}");
        }
    }

    #[test]
    fn a_branch_merged_twice_belongs_to_the_older_merge() {
        // main: 1 ← 2 ← M1(30, merging 11) ← M2(31, merging 13); x: 10 ← 11 ← 12 ← 13.
        let mut cs = vec![c(1, &[], "Initial commit"), c(2, &[1], "c2"), c(30, &[2, 11], "Merge branch 'x'"), c(31, &[30, 13], "Merge branch 'x'")];
        cs.extend(chain(1, 10, 13));
        let h = Hand::new(newest_first(cs), vec![unit("main", &[31])], vec![trunk()]);
        let p = h.plan(&folding());
        let held: Vec<(usize, Vec<usize>)> = p.folds.iter().map(|f| (f.merge, f.commits.clone())).collect();
        assert_eq!(held, [(h.at(31), vec![h.at(13), h.at(12)]), (h.at(30), vec![h.at(11), h.at(10)])]);
    }

    #[test]
    fn everything_else_show_does_not_unfold_but_an_explicit_show_does() {
        let p = merged(Some(12), otherwise(Show), "m").plan(&folding());
        assert_eq!(p.unit_state[1], BranchState::Folded);
        // A rule's Show unfolds what the ref reaches; the merge keeps the rest.
        let h = merged(Some(11), by_rule(Show), "m");
        let p = h.plan(&folding());
        assert_eq!(p.unit_state[1], BranchState::Shown);
        assert_eq!(h.states(&p, &[10, 11, 12]), [Visible, Visible, Folded { merge: h.at(30) }]);
        assert_eq!(p.folds[0].commits, [h.at(12)]);
        // All of it: no fold left.
        let p = merged(Some(12), by_rule(Show), "m").plan(&folding());
        assert!(p.folds.is_empty());
        // Checked out counts as explicit.
        let mut h = merged(Some(12), otherwise(Show), "m");
        h.checked_out[1] = true;
        assert!(h.plan(&folding()).folds.is_empty());
    }

    #[test]
    fn fold_beats_collapse() {
        let h = merged(Some(12), by_rule(Collapse), "m");
        let p = h.plan(&folding());
        assert!(p.runs.is_empty());
        assert_eq!(p.folds[0].commits.len(), 3);
        assert_eq!(p.unit_state[1], BranchState::Folded);
    }

    #[test]
    fn a_collapsed_branch_forked_from_folded_commits_keeps_its_fork_point() {
        // y (20..=25) forks from x's 11, x merged at 30: 10 and 11 stay for y, 12 folds.
        let mut cs = vec![c(1, &[], "Initial commit"), c(2, &[1], "c2"), c(30, &[2, 12], "m"), c(31, &[30], "c31")];
        cs.extend(chain(1, 10, 12));
        cs.extend(chain(11, 20, 25));
        let h = Hand::new(newest_first(cs), vec![unit("main", &[31]), unit("y", &[25])], vec![trunk(), by_rule(Collapse)]);
        let p = h.plan(&folding());
        assert_eq!(h.states(&p, &[10, 11, 12]), [Visible, Visible, Folded { merge: h.at(30) }]);
        assert_eq!((p.runs[0].newest, p.runs[0].oldest), (h.at(25), h.at(20)));
    }

    #[test]
    fn a_branch_merged_into_a_non_trunk_branch_stays() {
        // y merges x (10..=12) at 20, y not the trunk.
        let mut cs = vec![c(1, &[], "Initial commit"), c(2, &[1], "c2"), c(20, &[2, 12], "Merge branch 'x' into y")];
        cs.extend(chain(1, 10, 12));
        let h = Hand::new(newest_first(cs), vec![unit("main", &[2]), unit("y", &[20])], vec![trunk(), otherwise(Show)]);
        let p = h.plan(&folding());
        assert!(p.folds.is_empty());
        assert_eq!(h.states(&p, &[10, 11, 12, 20]), [Visible; 4]);
    }

    #[test]
    fn stash_on_a_collapsed_commit_goes_into_the_run_and_lines_stay_continuous() {
        // A stash (40) on x's 12; one (41) on x's oldest; one (42) on its newest.
        let mut cs = vec![c(1, &[], "Initial commit"), c(2, &[1], "c2"), c(3, &[2], "c3"), c(40, &[12], "On x: wip"), c(41, &[10], "On x: old"), c(42, &[15], "On x: new")];
        cs.extend(chain(2, 10, 15));
        let mut h = Hand::new(newest_first(cs), vec![unit("main", &[3]), unit("x", &[15])], vec![trunk(), by_rule(Collapse)]);
        for n in [40, 41, 42] {
            let i = h.at(n);
            h.stash[i] = true;
        }
        let p = h.plan(&spec());
        assert_eq!(h.states(&p, &[40, 41, 42]), [InRun { run: 0 }, InRun { run: 0 }, Visible]);
        let mut middle = [14, 13, 12, 11, 40, 41].map(|n| h.at(n));
        middle.sort_unstable();
        assert_eq!(p.runs[0].middle, middle);
        // The rows' lines: `view::rows`' `a_stash_in_a_run_keeps_every_line_continuous`.
    }

    #[test]
    fn a_detached_head_on_a_hidden_branch_keeps_its_commit() {
        let mut h = branch_off_main(3, Hide);
        h.detached = vec![h.at(11)];
        let p = h.plan(&spec());
        assert_eq!(h.states(&p, &[10, 11, 12]), [Visible, Visible, Hidden]);
    }

    #[test]
    fn a_run_never_hides_a_worktree_head() {
        let mut h = branch_off_main(6, Collapse);
        h.heads = vec![h.at(12)];
        assert!(h.plan(&spec()).runs.is_empty());
    }

    #[test]
    fn an_expanded_fold_or_branch_shows_in_full() {
        let h = merged(Some(12), otherwise(Show), "m");
        let p = h.plan(&ViewSpec { expanded: vec![format!("fold:{}", id(30))], ..folding() });
        assert!(p.folds.is_empty());
        assert_eq!(h.states(&p, &[10, 11, 12]), [Visible; 3]);
        // A branch's expansion is the rule engine's: it decides the unit Show/Expanded.
        let h = branch_off_main(6, Collapse);
        let mut h2 = branch_off_main(6, Collapse);
        h2.decisions[1] = Decision { action: Show, by: DecidedBy::Expanded, rule: Some(0) };
        assert_eq!(h.plan(&spec()).runs.len(), 1);
        assert!(h2.plan(&spec()).runs.is_empty());
        assert!(h2.plan(&spec()).state.iter().all(|s| *s == Visible));
    }

    #[test]
    fn tag_only_history_stays() {
        // 20 ← 21 reached by no branch (a tag's), on hidden x's 10.
        let mut cs = vec![c(1, &[], "Initial commit"), c(2, &[1], "c2"), c(3, &[2], "c3"), c(20, &[10], "c20"), c(21, &[20], "c21")];
        cs.extend(chain(2, 10, 12));
        let h = Hand::new(newest_first(cs), vec![unit("main", &[3]), unit("x", &[12])], vec![trunk(), by_rule(Hide)]);
        let p = h.plan(&spec());
        assert_eq!(h.states(&p, &[20, 21, 10, 11, 12]), [Visible, Visible, Visible, Hidden, Hidden]);
    }
}
