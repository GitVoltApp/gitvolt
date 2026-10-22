//! What rules match a unit on, from the loaded window, and which units reach each commit.

use crate::input::WorktreeIn;
use crate::presets::BranchUnit;
use crate::walk::CommitMeta;
use crate::Oid;
use std::collections::{HashMap, HashSet};

/// One unit's facts, within the loaded commits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UnitFacts {
    pub trunk: bool,
    pub checked_out: bool,
    pub local: bool,
    /// The newest committer time among its tips in the window; `None` when none is loaded.
    pub tip_time: Option<i64>,
    pub merged: bool,
    /// Lower-cased author emails and names of the unit's own commits (reachable from it, not from the trunk).
    pub authors: HashSet<String>,
    pub mine: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Facts {
    pub units: Vec<UnitFacts>,
    pub reach: ReachSets,
}

/// Per commit (walk index): the units that reach it, as a bitset over unit indices (empty for
/// trunk-reachable commits), and whether a trunk branch reaches it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ReachSets {
    bits: Vec<Vec<u64>>,
    trunk: Vec<bool>,
}

impl ReachSets {
    /// `bits`: one word vector per commit (bit `u % 64` of word `u / 64` for unit `u`), empty for
    /// trunk-reachable commits; `trunk`: per commit, whether a trunk branch reaches it.
    pub(crate) fn new(bits: Vec<Vec<u64>>, trunk: Vec<bool>) -> Self {
        ReachSets { bits, trunk }
    }

    /// The units reaching `commit`, ascending.
    pub(crate) fn units_reaching(&self, commit: usize) -> impl Iterator<Item = usize> + '_ {
        self.bits.get(commit).into_iter().flatten().enumerate().flat_map(|(w, &word)| (0..64).filter(move |b| word >> b & 1 == 1).map(move |b| w * 64 + b))
    }

    pub(crate) fn trunk_reachable(&self, commit: usize) -> bool {
        self.trunk.get(commit).copied().unwrap_or(false)
    }
}

/// Every unit's facts and the reach sets, in one pass over the window. Stub until lane A: only
/// the trunk flags and whether a unit is local; no unit reaches any commit.
pub(crate) fn unit_facts(units: &[BranchUnit], trunk: &[bool], commits: &[CommitMeta], _index: &HashMap<Oid, usize>, _worktrees: &[WorktreeIn], _me: &[String], _now: i64) -> Facts {
    let units = units.iter().zip(trunk).map(|(u, &trunk)| UnitFacts { trunk, local: u.has_local, ..Default::default() }).collect();
    Facts { units, reach: ReachSets::new(vec![Vec::new(); commits.len()], vec![false; commits.len()]) }
}
