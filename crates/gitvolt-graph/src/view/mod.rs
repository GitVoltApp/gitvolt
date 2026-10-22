//! The view transform: which commits a preset shows, collapses, folds and hides, before the
//! lane layout; the settings page's live summary; and where a hidden commit went.

mod plan;
mod reveal;
mod rows;
mod spec;

pub(crate) use plan::{CommitState, ViewPlan};
pub use reveal::reveal;
pub(crate) use rows::ViewRows;
pub use spec::ViewSpec;

use crate::input::GraphInput;
use crate::payload::{BranchState, PresetPreview};
use crate::pins::{fork_of, trunk_names};
use crate::presets::{branch_units, decide, trunk_flags, unit_facts, BranchUnit, Decision, Facts, PresetAction};
use crate::walk::{CommitMeta, WalkResult};
use crate::Oid;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

/// The rule engine's answer over a window: the units, which are the trunk, their facts (with the
/// reach sets) and the decisions.
pub(crate) struct Decided {
    pub units: Vec<BranchUnit>,
    pub trunk: Vec<bool>,
    pub facts: Facts,
    pub decisions: Vec<Decision>,
}

/// Runs the rule engine on `spec` over the window.
pub(crate) fn decide_window(input: &GraphInput, spec: &ViewSpec, commits: &[CommitMeta], index: &HashMap<Oid, usize>) -> Decided {
    let units = branch_units(&input.refs);
    let fork = fork_of(&input.refs, &input.forks);
    let trunk = trunk_flags(&units, &trunk_names(input, fork.as_ref()), &spec.trunk_patterns);
    let facts = unit_facts(&units, &trunk, commits, index, &input.worktrees, &spec.me, spec.now);
    let decisions = decide(spec, &units, &facts);
    Decided { units, trunk, facts, decisions }
}

/// `plan` over the window, with what it reads from the worktrees, the stashes and the trunk.
pub(crate) fn plan_window(input: &GraphInput, spec: &ViewSpec, commits: &[CommitMeta], index: &HashMap<Oid, usize>, d: &Decided) -> ViewPlan {
    let n = commits.len();
    let mut trunk_fp = vec![false; n];
    for unit in d.units.iter().zip(&d.trunk).filter(|(_, t)| **t).map(|(u, _)| u) {
        for t in &unit.tips {
            let mut cur = index.get(t).copied();
            while let Some(c) = cur.filter(|&c| !trunk_fp[c]) {
                trunk_fp[c] = true;
                cur = commits[c].parents.first().and_then(|p| index.get(p)).copied();
            }
        }
    }
    let at = |id: &Option<Oid>| id.as_ref().and_then(|h| index.get(h)).copied();
    let head = &input.refs.head;
    let detached: Vec<usize> = input.worktrees.iter().filter(|w| w.branch.is_none()).filter_map(|w| at(&w.head)).chain(head.detached.then(|| at(&head.target)).flatten()).collect();
    let heads: Vec<usize> = input.worktrees.iter().filter_map(|w| at(&w.head)).collect();
    let branches: HashSet<&str> = input.worktrees.iter().filter_map(|w| w.branch.as_deref()).collect();
    let checked_out: Vec<bool> = d.units.iter().map(|u| u.refs.iter().any(|r| branches.contains(r.as_str()))).collect();
    let mut stash = vec![false; n];
    for s in &input.refs.stashes {
        if let Some(&c) = index.get(&s.id) {
            stash[c] = true;
        }
    }
    let window = plan::Window {
        commits,
        index,
        units: &d.units,
        decisions: &d.decisions,
        reach: &d.facts.reach,
        trunk_fp: &trunk_fp,
        detached_heads: &detached,
        worktree_heads: &heads,
        checked_out: &checked_out,
        stash: &stash,
    };
    plan::plan(&window, spec)
}

/// The settings page's live summary of `input.view` over the window: per rule how many branches
/// it decides (Everything else last), how many are shown, collapsed and hidden, and how many
/// merges hold a folded branch. All zeros without a view.
pub fn preview(input: &GraphInput, walked: &WalkResult) -> PresetPreview {
    let Some(spec) = &input.view else { return PresetPreview::default() };
    let index = window_index(&walked.commits);
    let d = decide_window(input, spec, &walked.commits, &index);
    preview_of(spec, &d, &plan_window(input, spec, &walked.commits, &index, &d))
}

pub(crate) fn preview_of(spec: &ViewSpec, d: &Decided, plan: &ViewPlan) -> PresetPreview {
    let mut rules = vec![0u32; spec.rules.len() + 1];
    for decision in &d.decisions {
        let i = decision.rule.map_or(spec.rules.len(), usize::from).min(spec.rules.len());
        rules[i] += 1;
    }
    let count = |s: BranchState| plan.unit_state.iter().filter(|&&x| x == s).count() as u32;
    PresetPreview { rules, shown: count(BranchState::Shown), collapsed: count(BranchState::Collapsed), hidden: count(BranchState::Hidden), folded: plan.folds.len() as u32 }
}

/// The `expanded` key that would make `sha`, a loaded commit that isn't a row under
/// `input.view`, a row: `"branch:<collapsed unit>"`, `"fold:<merge sha>"`, or
/// `"branch:<hidden unit>"` for the hidden unit with the newest tip reaching it. `None` when it is
/// a row or isn't loaded.
pub fn hidden_owner(input: &GraphInput, walked: &WalkResult, sha: &str) -> Option<String> {
    let spec = input.view.as_ref()?;
    let commits = &walked.commits;
    let index = window_index(commits);
    let ci = *index.get(&Oid::from_hex(sha)?)?;
    let d = decide_window(input, spec, commits, &index);
    let plan = plan_window(input, spec, commits, &index, &d);
    let stashes: HashSet<Oid> = input.refs.stashes.iter().map(|s| s.id).collect();
    hidden_owner_of(commits, &index, &d, &plan, &stashes, ci)
}

pub(crate) fn hidden_owner_of(commits: &[CommitMeta], index: &HashMap<Oid, usize>, d: &Decided, plan: &ViewPlan, stashes: &HashSet<Oid>, ci: usize) -> Option<String> {
    match plan.state[ci] {
        CommitState::Visible => None,
        CommitState::Run { unit } => Some(format!("branch:{}", d.units[unit].name)),
        CommitState::Folded { merge } => Some(format!("fold:{}", commits[merge].id)),
        CommitState::Hidden => {
            // A stash hides with its base commit.
            let target = if stashes.contains(&commits[ci].id) { commits[ci].parents.first().and_then(|p| index.get(p)).copied()? } else { ci };
            let hidden = d.facts.reach.units_reaching(target).filter(|&u| d.decisions[u].action == PresetAction::Hide);
            let u = hidden.max_by_key(|&u| (newest_tip(&d.units[u], commits, index).map(|t| commits[t].committer_time), Reverse(u)))?;
            Some(format!("branch:{}", d.units[u].name))
        }
    }
}

/// A unit's newest tip in the window (by committer time).
pub(crate) fn newest_tip(unit: &BranchUnit, commits: &[CommitMeta], index: &HashMap<Oid, usize>) -> Option<usize> {
    unit.tips.iter().filter_map(|t| index.get(t).copied()).max_by_key(|&c| (commits[c].committer_time, Reverse(c)))
}

pub(crate) fn window_index(commits: &[CommitMeta]) -> HashMap<Oid, usize> {
    commits.iter().enumerate().map(|(i, c)| (c.id, i)).collect()
}
