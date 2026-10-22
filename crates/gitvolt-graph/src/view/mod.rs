//! The view transform: which commits a preset shows, collapses, folds and hides, before the
//! lane layout; the settings page's live summary; and where a hidden commit went.

mod reveal;
mod spec;

pub use reveal::reveal;
pub use spec::ViewSpec;

use crate::input::GraphInput;
use crate::payload::PresetPreview;
use crate::walk::WalkResult;

/// The settings page's live summary of `input.view` over the window: per rule how many branches
/// it decides, and how many are shown, collapsed, hidden and folded. Stub until lane B.
pub fn preview(_input: &GraphInput, _walked: &WalkResult) -> PresetPreview {
    PresetPreview::default()
}

/// The `expanded` key that would make `sha`, a loaded commit that isn't a row under
/// `input.view`, a row: `"branch:<collapsed unit>"`, `"fold:<merge sha>"`, or
/// `"branch:<hidden unit>"` for the hidden unit with the newest tip reaching it. `None` when it is
/// a row or isn't loaded. Stub until lane B.
pub fn hidden_owner(_input: &GraphInput, _walked: &WalkResult, _sha: &str) -> Option<String> {
    None
}
