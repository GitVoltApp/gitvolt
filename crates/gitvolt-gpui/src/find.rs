use crate::app::GitVolt;
use gpui::*;

impl GitVolt {
    pub fn open_find(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}
    pub fn set_find_text(&mut self, _q: &str, _cx: &mut Context<Self>) {}
    pub fn focus_graph(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }
    pub fn focus_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }
}
