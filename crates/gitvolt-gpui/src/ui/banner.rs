//! A banner (`ui/src/banner`): a line between the toolbar and the panels, its accent on the left, and
//! its links. Nothing raises one yet; the writes will (autostash, recovery).

use crate::theme;
use gpui::prelude::*;
use gpui::*;

#[derive(Clone)]
pub struct BannerAction {
    pub label: SharedString,
    pub run: super::Action,
}

#[derive(Clone)]
pub struct Banner {
    pub id: SharedString,
    pub text: SharedString,
    /// `--orange` (autostash), `--blue` (recovery).
    pub accent: Hsla,
    pub actions: Vec<BannerAction>,
}

pub fn banners(list: &[Banner]) -> Option<AnyElement> {
    if list.is_empty() {
        return None;
    }
    Some(
        div()
            .flex()
            .flex_col()
            .flex_none()
            .border_b_1()
            .border_color(theme::section_border())
            .children(list.iter().map(|b| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(6.))
                    .border_l_3()
                    .border_color(b.accent)
                    .bg(theme::panel_bg1())
                    .text_color(theme::text_normal())
                    .text_size(px(12.))
                    .child(div().flex_1().min_w_0().child(b.text.clone()))
                    .children(b.actions.iter().enumerate().map(|(i, a)| {
                        let run = a.run.clone();
                        div()
                            .id(ElementId::NamedInteger(b.id.clone(), i as u64))
                            .px(px(4.))
                            .cursor_pointer()
                            .text_color(theme::link())
                            .on_click(move |_, window, cx| run(window, cx))
                            .child(a.label.clone())
                    }))
            }))
            .into_any_element(),
    )
}
