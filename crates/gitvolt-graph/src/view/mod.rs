//! The view transform: which commits a preset shows, collapses, folds and hides, before the
//! lane layout; the settings page's live summary; and where a hidden commit went.

#[cfg(test)]
mod perf_tests;
mod plan;
mod reveal;
mod rows;
mod spec;

pub(crate) use plan::{CommitState, ViewPlan};
pub use reveal::reveal;
pub(crate) use rows::ViewRows;
pub(crate) use spec::{branch_key, run_key};
pub use spec::ViewSpec;

use crate::input::{GraphInput, RefKind};
use crate::payload::{BranchState, PresetPreview};
use crate::pins::{fork_of, trunk_names};
use crate::presets::{branch_units, decide, tag_hidden, trunk_flags, unit_facts, BranchUnit, Decision, Facts, PresetAction};
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
    /// The tags hand picks hide (`tag_hidden`), in the refs' order.
    pub hidden_tags: Vec<HiddenTag>,
}

/// A hidden tag: its short name (its `"tag:<name>"` expansion's), full name and commit.
pub(crate) struct HiddenTag {
    pub name: String,
    pub full_name: String,
    pub target: Oid,
}

impl Decided {
    /// The full names of the hidden units' refs and of the hidden tags: they have no chip, and a
    /// worktree on a hidden branch has no WIP row.
    pub(crate) fn hidden_refs(&self) -> HashSet<&str> {
        let units = self.units.iter().zip(&self.decisions).filter(|(_, d)| d.action == PresetAction::Hide).flat_map(|(u, _)| u.refs.iter().map(String::as_str));
        units.chain(self.hidden_tags.iter().map(|t| t.full_name.as_str())).collect()
    }
}

/// Runs the rule engine on `spec` over the window, and the hand picks on its tags.
pub(crate) fn decide_window(input: &GraphInput, spec: &ViewSpec, commits: &[CommitMeta], index: &HashMap<Oid, usize>) -> Decided {
    let units = branch_units(&input.refs);
    let fork = fork_of(&input.refs, &input.forks);
    let trunk = trunk_flags(&units, &trunk_names(input, fork.as_ref()), &spec.trunk_patterns);
    let facts = unit_facts(&units, &trunk, commits, index, &input.worktrees, input.open_head().branch.as_deref(), &spec.me);
    let decisions = decide(spec, &units, &facts);
    let hidden_tags = input
        .refs
        .refs
        .iter()
        .filter(|r| r.kind == RefKind::Tag && tag_hidden(spec, &r.short_name))
        .map(|r| HiddenTag { name: r.short_name.clone(), full_name: r.full_name.clone(), target: r.target })
        .collect();
    Decided { units, trunk, facts, decisions, hidden_tags }
}

/// Per commit: a hidden tag reaches it, and no other ref but a branch does (a shown tag, a compare
/// head, `refs/remotes/<r>/HEAD`). `plan` hides such a commit when no branch reaches it either:
/// what only hidden tags keep (an archived branch's commits) goes with them.
fn kept_by_hidden_tags_only(input: &GraphInput, d: &Decided, commits: &[CommitMeta], index: &HashMap<Oid, usize>) -> Vec<bool> {
    let n = commits.len();
    if d.hidden_tags.is_empty() {
        return vec![false; n];
    }
    let branches: HashSet<&str> = d.units.iter().flat_map(|u| u.refs.iter().map(String::as_str)).collect();
    let hidden: HashSet<&str> = d.hidden_tags.iter().map(|t| t.full_name.as_str()).collect();
    let (mut by_hidden, mut by_other) = (vec![false; n], vec![false; n]);
    for r in input.refs.refs.iter().filter(|r| !branches.contains(r.full_name.as_str())) {
        if let Some(&c) = index.get(&r.target) {
            if hidden.contains(r.full_name.as_str()) { by_hidden[c] = true } else { by_other[c] = true }
        }
    }
    for c in input.refs.compare_heads.iter().filter_map(|h| index.get(h)) {
        by_other[*c] = true;
    }
    // Walk order: children first.
    for c in 0..n {
        for p in commits[c].parents.iter().filter_map(|p| index.get(p)) {
            by_hidden[*p] |= by_hidden[c];
            by_other[*p] |= by_other[c];
        }
    }
    by_hidden.iter().zip(&by_other).map(|(h, o)| *h && !o).collect()
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
    // A worktree on a hidden branch has no WIP row (`assemble`).
    let hidden = d.hidden_refs();
    let heads: Vec<usize> = input.worktrees.iter().filter(|w| !w.branch.as_deref().is_some_and(|b| hidden.contains(b))).filter_map(|w| at(&w.head)).collect();
    let branches: HashSet<&str> = input.worktrees.iter().filter_map(|w| w.branch.as_deref()).collect();
    let checked_out: Vec<bool> = d.units.iter().map(|u| u.refs.iter().any(|r| branches.contains(r.as_str()))).collect();
    let mut stash = vec![false; n];
    for s in &input.refs.stashes {
        if let Some(&c) = index.get(&s.id) {
            stash[c] = true;
        }
    }
    let tag_hidden = kept_by_hidden_tags_only(input, d, commits, index);
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
        tag_hidden: &tag_hidden,
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

/// What holds `sha`, a loaded commit that isn't a row under `input.view`: its collapsed run's key
/// (`"branch:<unit>@<newest sha>"`, whose `expanded` key is `"branch:<unit>"`), `"fold:<merge
/// sha>"`, `"branch:<hidden unit>"` for the hidden unit with the newest tip reaching it, or, when
/// no branch does, `"tag:<hidden tag>"` for the hidden tag on the newest commit reaching it. `None`
/// when it is a row or isn't loaded.
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
        CommitState::Run { run } => {
            let r = &plan.runs[run];
            Some(run_key(&d.units[r.unit].name, &commits[r.newest].id))
        }
        CommitState::Folded { merge } => Some(format!("fold:{}", commits[merge].id)),
        CommitState::Hidden => {
            // A stash hides with its base commit.
            let target = if stashes.contains(&commits[ci].id) { commits[ci].parents.first().and_then(|p| index.get(p)).copied()? } else { ci };
            let hidden = d.facts.reach.units_reaching(target).filter(|&u| d.decisions[u].action == PresetAction::Hide);
            if let Some(u) = hidden.max_by_key(|&u| (newest_tip(&d.units[u], commits, index).map(|t| commits[t].committer_time), Reverse(u))) {
                return Some(format!("branch:{}", d.units[u].name));
            }
            // No branch reaches it: hidden tags keep it, the one on the newest commit names it.
            let tag = d.hidden_tags.iter().filter(|t| index.get(&t.target).is_some_and(|&c| reaches(commits, index, c, target)));
            let tag = tag.max_by_key(|t| (index.get(&t.target).map(|&c| commits[c].committer_time), Reverse(&t.name)))?;
            Some(format!("tag:{}", tag.name))
        }
    }
}

/// Whether walk index `to` is `from` or one of its ancestors in the window.
fn reaches(commits: &[CommitMeta], index: &HashMap<Oid, usize>, from: usize, to: usize) -> bool {
    let mut seen = HashSet::from([from]);
    let mut todo = vec![from];
    while let Some(c) = todo.pop() {
        if c == to {
            return true;
        }
        // Walk order puts a commit before its parents: nothing past `to` leads back to it.
        todo.extend(commits[c].parents.iter().filter_map(|p| index.get(p).copied()).filter(|&p| p <= to && seen.insert(p)));
    }
    false
}

/// A unit's newest tip in the window (by committer time).
pub(crate) fn newest_tip(unit: &BranchUnit, commits: &[CommitMeta], index: &HashMap<Oid, usize>) -> Option<usize> {
    unit.tips.iter().filter_map(|t| index.get(t).copied()).max_by_key(|&c| (commits[c].committer_time, Reverse(c)))
}

pub(crate) fn window_index(commits: &[CommitMeta]) -> HashMap<Oid, usize> {
    commits.iter().enumerate().map(|(i, c)| (c.id, i)).collect()
}
