//! The reveal panel's rows: a collapsed run or a folded merge laid out on the graph's own lanes.

use crate::input::GraphInput;
use crate::payload::RevealPayload;
use crate::walk::WalkResult;

/// The panel for `key` (`"branch:<unit>"` or `"fold:<merge sha>"`) under `input.view`, opening
/// `below` the row or above it. `None` for an unknown or stale key. Stub until lane R.
pub fn reveal(_input: &GraphInput, _walked: &WalkResult, _key: &str, _below: bool) -> Option<RevealPayload> {
    None
}
