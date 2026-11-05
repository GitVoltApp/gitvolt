//! The diff's review cards.

use crate::app::GitVolt;
use gitvolt_core::forge::types::ReviewDraft;
use gitvolt_core::forge::ForgeKind;
use gpui::*;

pub fn draft_card(_: &mut GitVolt, _tab: u32, _kind: ForgeKind, d: &ReviewDraft, _outdated: bool, _cx: &mut Context<GitVolt>) -> AnyElement {
    div().child(d.body.clone()).into_any_element()
}
