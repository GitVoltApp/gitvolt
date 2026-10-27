//! Branch presets: the stored model, and the rule engine that decides each branch of a window
//! (its units, the trunk, the facts rules match on, and the decisions). The view transform
//! (`crate::view`) acts on the decisions.

mod facts;
mod glob;
mod model;
mod rules;
mod units;

pub(crate) use facts::{unit_facts, Facts, ReachSets, UnitFacts};
#[cfg(test)]
pub(crate) use glob::glob_match;
pub(crate) use glob::name_matches;
pub use model::{Author, BranchPicks, BranchPreset, CollapsePlacement, Matcher, PresetAction, PresetScope, PresetSlot, Rule};
pub(crate) use rules::{decide, tag_hidden};
pub(crate) use units::{branch_units, trunk_flags, BranchUnit, Decision};
