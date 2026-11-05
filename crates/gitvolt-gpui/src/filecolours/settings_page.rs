//! Settings › File colours (`fileColours/settings/`).

use crate::app::GitVolt;
use crate::settings::pages::Def;
use gpui::*;

#[derive(Default)]
pub struct PageState {}

pub fn defs() -> Vec<Def> {
    Vec::new()
}

pub fn body(_: &mut GitVolt, _: &mut Window, _: &mut Context<GitVolt>) -> Vec<AnyElement> {
    Vec::new()
}

impl GitVolt {
    pub fn paste_file_colours(&mut self, _: &str, _: &mut Context<Self>) {}
}
