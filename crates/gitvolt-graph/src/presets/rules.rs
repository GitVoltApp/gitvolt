//! Deciding each unit from a preset's rules, its picks and the session's expansions.

use crate::payload::DecidedBy;
use crate::presets::{BranchUnit, Decision, Facts, PresetAction};
use crate::view::ViewSpec;

/// One decision per unit. Stub until lane A: every unit is shown by Everything else.
pub(crate) fn decide(_spec: &ViewSpec, units: &[BranchUnit], _facts: &Facts) -> Vec<Decision> {
    units.iter().map(|_| Decision { action: PresetAction::Show, by: DecidedBy::Otherwise, rule: None }).collect()
}
