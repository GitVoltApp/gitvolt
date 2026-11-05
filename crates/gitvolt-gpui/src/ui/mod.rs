//! The shared UI: what every feature builds on.
//!
//! - `input`: a text field and a textarea.

// What the later features build on is here before they use it.
#![allow(dead_code)]

pub mod input;

use gpui::*;

pub fn init(cx: &mut App) {
    input::bind_keys(cx);
}
