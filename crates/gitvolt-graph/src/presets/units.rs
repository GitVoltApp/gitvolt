//! Branch units: a local branch together with its remote-tracking refs, or a remote-only
//! branch, keyed by the short branch name; and which of them are the trunk.

use crate::input::RefsIn;
use crate::payload::DecidedBy;
use crate::presets::PresetAction;
use crate::Oid;

/// One branch as a chip shows it: `refs/heads/<name>` and every `refs/remotes/<r>/<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BranchUnit {
    pub name: String,
    /// Full ref names.
    pub refs: Vec<String>,
    pub tips: Vec<Oid>,
    pub has_local: bool,
}

/// What a preset does with one unit, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Decision {
    pub action: PresetAction,
    pub by: DecidedBy,
    /// The first rule that matched, in rule order (for the settings page's counts); `None`:
    /// Everything else.
    pub rule: Option<u16>,
}

/// The repository's branch units. Stub until lane A.
pub(crate) fn branch_units(_refs: &RefsIn) -> Vec<BranchUnit> {
    vec![]
}

/// Per unit: is it a trunk branch (its refs include a `pinned` name, or its name matches one of
/// the repository's extra trunk `patterns`). Stub until lane A.
pub(crate) fn trunk_flags(units: &[BranchUnit], _pinned: &[String], _patterns: &[String]) -> Vec<bool> {
    vec![false; units.len()]
}
