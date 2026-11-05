use crate::app::GitVolt;
use gpui::*;

impl GitVolt {
    pub fn open_palette(&mut self, _prefix: &str, _window: &mut Window, _cx: &mut Context<Self>) {}
    pub fn go_to_clipboard(&mut self, _cx: &mut Context<Self>) {}
    pub fn open_about(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}
    pub fn tab_group_rows(&self, _ix: usize, _this: &WeakEntity<GitVolt>) -> Vec<crate::ui::menu::MenuRow> {
        vec![crate::ui::menu::MenuRow::pending("tab.newGroup", "Add to new group", "layers")]
    }
}
