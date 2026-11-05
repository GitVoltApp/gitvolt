//! The forge's side of a stack (`forge/stack/`).

use crate::app::GitVolt;
use gitvolt_core::forge::types::ForgeMr;
use gpui::*;

/// `StackLine`: the card's line naming the stack an MR/PR is in.
pub fn stack_line(_tab: u32, _mr: &ForgeMr, _this: &WeakEntity<GitVolt>, _cx: &mut App) -> Option<AnyElement> {
    None
}

/// `StackPanel`.
pub fn stack_panel(_this: &mut GitVolt, _tab: u32, _kind: gitvolt_core::forge::ForgeKind, _mr: &ForgeMr, _cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    None
}

impl GitVolt {
    /// A stack's after-merge flow.
    pub fn stack_after_merge(&mut self, _tab: u32, _number: u64, _cx: &mut Context<Self>) {}
}
