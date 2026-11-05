//! A one-line text field: what's typed goes to the view's `on_key_down`. No caret blink: an idle app
//! costs nothing. No IME yet.

use crate::theme;
use gpui::prelude::*;
use gpui::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Field {
    pub text: String,
    /// All of it is selected: the next character replaces it.
    pub selected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Typed {
    /// Not for a text field.
    None,
    Changed,
    Enter,
}

impl Field {
    pub fn key(&mut self, k: &Keystroke) -> Typed {
        if k.modifiers.control || k.modifiers.alt || k.modifiers.platform {
            return Typed::None;
        }
        match k.key.as_str() {
            "enter" => return Typed::Enter,
            "backspace" => {
                if self.selected {
                    self.text.clear();
                } else {
                    self.text.pop();
                }
                self.selected = false;
                return Typed::Changed;
            }
            _ => {}
        }
        let Some(c) = k.key_char.as_ref().filter(|c| !c.chars().any(char::is_control)) else { return Typed::None };
        if self.selected {
            self.text.clear();
        }
        self.selected = false;
        self.text.push_str(c);
        Typed::Changed
    }

    pub fn paste(&mut self, text: &str) {
        if self.selected {
            self.text.clear();
        }
        self.selected = false;
        self.text.push_str(text.split(['\n', '\r']).next().unwrap_or_default());
    }

    /// Its text and a caret at the end, or the whole of it selected.
    pub fn view(&self) -> Div {
        let text = div().when(self.selected, |d| d.bg(theme::selection_bg())).child(self.text.clone());
        div().flex().items_center().min_w_0().child(text).when(!self.selected, |d| d.child(div().flex_none().w(px(1.)).h(px(14.)).bg(theme::text_selected())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn key(s: &'static str) -> Keystroke {
        Keystroke::parse(s).unwrap()
    }

    #[test]
    fn typing_replaces_a_selection_then_appends_and_backspace_takes_one() {
        let mut f = Field { text: "100".into(), selected: true };
        let mut typed = key("5");
        typed.key_char = Some("5".into());
        assert_eq!(f.key(&typed), Typed::Changed);
        assert_eq!(f.text, "5");
        assert_eq!(f.key(&typed), Typed::Changed);
        assert_eq!(f.text, "55");
        assert_eq!(f.key(&key("backspace")), Typed::Changed);
        assert_eq!(f.text, "5");
        assert_eq!(f.key(&key("enter")), Typed::Enter);
        assert_eq!(f.key(&key("ctrl-a")), Typed::None);
    }

    #[test]
    fn a_paste_keeps_one_line() {
        let mut f = Field::default();
        f.paste("ab\ncd");
        assert_eq!(f.text, "ab");
    }
}
