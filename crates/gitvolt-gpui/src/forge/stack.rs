//! The forge's side of a stack (`forge/stack/`).

use crate::app::GitVolt;
use gitvolt_core::forge::types::ForgeMr;
use gpui::*;

/// `StackLine`: the card's line naming the stack an MR/PR is in.
pub fn stack_line(_tab: u32, _mr: &ForgeMr, _this: &WeakEntity<GitVolt>, _cx: &mut App) -> Option<AnyElement> {
    None
}
