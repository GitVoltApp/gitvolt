//! The small controls: a dialog's buttons, `select` (`ui/Select.tsx`: the app's own menu, no native
//! popup), `switch` (`ui/Switch.tsx`), `check_box`, `keycaps` (`ui/keycaps.css`), and a clearable
//! filter box (`ui/ClearableInput.tsx`).

use super::input::TextInput;
use super::menu::{self, MenuRow, OpenOptions};
use super::tooltip::tip;
use crate::app::icon;
use crate::theme;
use gpui::prelude::*;
use gpui::*;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Plain,
    Primary,
    /// Green outlined, filled on hover.
    Positive,
    /// Red; it arms before it goes.
    Danger,
    /// The quiet Cancel of a popover.
    Quiet,
}

/// `.modal-actions button`.
pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, kind: ButtonKind, enabled: bool) -> Stateful<Div> {
    let base = div().id(id).flex().flex_none().items_center().justify_center().px(px(14.)).py(px(5.)).rounded(px(4.)).border_1().whitespace_nowrap().child(label.into());
    let base = match kind {
        ButtonKind::Plain => base.bg(theme::control_bg()).border_color(theme::section_border()).text_color(theme::text_normal()).when(enabled, |d| d.hover(|d| d.bg(theme::hover_row()).text_color(theme::text_selected()))),
        // `.modal-actions button[type="submit"]:disabled`: no blue.
        ButtonKind::Primary if !enabled => base.bg(theme::panel_bg1()).border_color(theme::section_border()).text_color(theme::text_selected()),
        ButtonKind::Primary => base.bg(theme::blue()).border_color(theme::blue()).text_color(theme::text_on_accent()),
        ButtonKind::Positive => base.border_color(theme::green()).text_color(theme::green()).when(enabled, |d| d.hover(|d| d.bg(theme::green()).text_color(theme::text_on_accent()))),
        ButtonKind::Danger => base.border_color(theme::red()).text_color(theme::mix(theme::red(), 0.6, theme::text_selected())).when(enabled, |d| d.hover(|d| d.bg(theme::mix(theme::red(), 0.2, transparent_black())))),
        ButtonKind::Quiet => base.border_color(transparent_black()).text_color(theme::text_secondary()).when(enabled, |d| d.hover(|d| d.bg(theme::hover_row()).text_color(theme::text_selected()))),
    };
    base.when(enabled, |d| d.cursor_pointer()).when(!enabled, |d| d.opacity(0.5))
}

/// `(value, label, tooltip)`s.
pub type SelectOptions<T> = Rc<Vec<(T, SharedString, Option<SharedString>)>>;
pub type OnSelect<T> = Rc<dyn Fn(&T, &mut Window, &mut App)>;

/// A dropdown: its options in the app's menu, under the button, the chosen one checked and started
/// on. Enter or Space and the arrows open it where the owner routes them; a press opens it at once.
pub fn select<T: Clone + PartialEq + 'static>(key: impl Into<SharedString>, value: T, options: SelectOptions<T>, shown: Option<SharedString>, on_change: OnSelect<T>, cx: &App) -> AnyElement {
    let key: SharedString = key.into();
    let open = menu::is_open_on(&key, cx);
    let current = shown.or_else(|| options.iter().find(|o| o.0 == value).map(|o| o.1.clone())).unwrap_or_default();
    let (k, options, chosen) = (key.clone(), options.clone(), value);
    super::anchor(
        key.clone(),
        div()
            .id(ElementId::Name(key))
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .pl(px(8.))
            .pr(px(10.))
            .py(px(5.))
            .text_color(theme::text_selected())
            .bg(theme::app_bg())
            .border_1()
            .border_color(if open { theme::blue() } else { theme::section_border() })
            .rounded(px(4.))
            .hover(|d| d.bg(theme::hover_row()))
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                let rows = options
                    .iter()
                    .enumerate()
                    .map(|(i, (v, label, tip))| {
                        let (v, on_change) = (v.clone(), on_change.clone());
                        MenuRow::action(format!("select.{i}"), label.clone(), if v == chosen { "check" } else { "" }, tip.clone().unwrap_or_else(|| label.clone()), move |window, cx| on_change(&v, window, cx))
                    })
                    .collect();
                let initial = options.iter().position(|o| o.0 == chosen).map(|i| SharedString::from(format!("select.{i}")));
                menu::open_below(k.clone(), rows, OpenOptions { initial, ..Default::default() }, window, cx);
            })
            .child(div().min_w_0().truncate().child(current))
            .child(icon("chevron-down", 14., theme::text_secondary())),
    )
    .into_any_element()
}

/// A switch's track and knob (`.switch-track`).
pub fn switch_track(checked: bool, disabled: bool) -> Div {
    div()
        .relative()
        .flex_none()
        .w(px(30.))
        .h(px(17.))
        .rounded(px(9.))
        .bg(if checked { theme::switch_on() } else { theme::switch_off() })
        .when(disabled, |d| d.opacity(0.55))
        .child(div().absolute().top(px(2.)).left(px(if checked { 15. } else { 2. })).size(px(13.)).rounded_full().bg(theme::text_on_accent()))
}

/// An on/off toggle, with its label and a line under it; a click on either flips it.
pub fn switch(id: impl Into<ElementId>, checked: bool, label: impl Into<SharedString>, description: Option<SharedString>, disabled: bool, on_change: impl Fn(bool, &mut Window, &mut App) + 'static) -> Stateful<Div> {
    let track = switch_track(checked, disabled);
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.))
        .min_w_0()
        .px(px(4.))
        .py(px(6.))
        .rounded(px(6.))
        .when(!disabled, |d| d.cursor_pointer().hover(|d| d.bg(theme::control_bg())).on_click(move |_, window, cx| on_change(!checked, window, cx)))
        .child(track)
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().text_size(px(12.5)).text_color(if disabled { theme::text_secondary() } else { theme::text_normal() }).child(label.into()))
                .when_some(description, |d, t| d.child(div().truncate().text_size(px(11.5)).text_color(theme::text_disabled()).child(t))),
        )
}

/// The box of a checkbox: green with a tick when it's on (`.ctx-arm-box`).
pub fn check_box(checked: bool, size_px: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size_px))
        .mt(px(2.))
        .border_1()
        .border_color(if checked { native_accent() } else { native_border() })
        .rounded(px(2.))
        .bg(if checked { native_accent() } else { native_ground() })
        .when(checked, |d| d.child(icon("check", size_px - 2., native_mark())))
}

/// What Chromium draws a native checkbox and radio with, in the colour scheme of the theme.
pub fn native_accent() -> Hsla {
    rgb(if theme::light_now() { 0x0075ff } else { 0x99c8ff }).into()
}

pub fn native_border() -> Hsla {
    rgb(if theme::light_now() { 0x767676 } else { 0x858585 }).into()
}

pub fn native_ground() -> Hsla {
    rgb(if theme::light_now() { 0xffffff } else { 0x3b3b3b }).into()
}

fn native_mark() -> Hsla {
    rgb(if theme::light_now() { 0xffffff } else { 0x3b3b3b }).into()
}

/// A labelled checkbox row (`.modal-check`).
pub fn check_row(id: impl Into<ElementId>, checked: bool, label: impl Into<SharedString>, on_change: impl Fn(bool, &mut Window, &mut App) + 'static) -> Stateful<Div> {
    div().id(id).flex().items_center().gap(px(6.)).cursor_pointer().on_click(move |_, window, cx| on_change(!checked, window, cx)).child(check_box(checked, 13.).mt_0()).child(label.into())
}

/// A control Tab reaches in a dialog: Enter or Space runs it, and it's outlined while it has the keyboard.
pub fn tabbable(el: Stateful<Div>, focus: &FocusHandle, run: impl Fn(&mut Window, &mut App) + 'static) -> Stateful<Div> {
    el.track_focus(focus)
        .focus(|s| s.border_color(theme::blue()))
        .on_key_down(move |e: &KeyDownEvent, window, cx| {
            let m = &e.keystroke.modifiers;
            if matches!(e.keystroke.key.as_str(), "enter" | "space") && !m.control && !m.alt && !m.shift && !m.platform {
                cx.stop_propagation();
                run(window, cx);
            }
        })
}

/// A chord as keycaps: `Ctrl+Shift+T` draws three.
pub fn keycaps(chord: &str) -> Div {
    let mut row = div().flex().flex_none().gap(px(3.));
    for key in split_chord(chord) {
        row = row.child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .min_w(px(18.))
                .px(px(6.))
                .py(px(3.))
                .text_size(px(11.))
                .line_height(px(11.))
                .border_1()
                .border_b_2()
                .border_color(theme::section_border())
                .rounded(px(4.))
                .bg(theme::app_bg())
                .text_color(theme::text_selected())
                .child(key),
        );
    }
    row
}

/// One keycap holding a whole chord (`.sc-key`).
pub fn keycap_one(text: &str) -> Div {
    div().flex().items_center().justify_center().min_w(px(18.)).px(px(6.)).py(px(3.)).text_size(px(11.)).line_height(px(11.)).border_1().border_b_2().border_color(theme::section_border()).rounded(px(4.)).bg(theme::app_bg()).text_color(theme::text_secondary()).child(text.to_string())
}

/// `keycaps`: a trailing `+` key stays a key.
pub fn split_chord(chord: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = chord;
    while let Some(i) = rest[..rest.char_indices().last().map_or(0, |c| c.0)].find('+') {
        out.push(rest[..i].to_string());
        rest = &rest[i + 1..];
    }
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
    out
}

/// A filter box: its field with a clear (×) inside its right edge, there while it has text. The space
/// for it is always reserved, and the button never takes the focus.
pub fn clearable(id: impl Into<ElementId>, input: &Entity<TextInput>, on_clear: impl Fn(&mut Window, &mut App) + 'static, window: &Window, cx: &App) -> Stateful<Div> {
    let field = input.read(cx);
    let (empty, focused) = (field.text().is_empty(), field.is_focused(window));
    let handle = field.focus.clone();
    let id: ElementId = id.into();
    div()
        .id(id.clone())
        .relative()
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .h(px(26.))
        .pl(px(8.))
        .pr(px(24.))
        .bg(theme::app_bg())
        .border_1()
        .border_color(if focused { theme::blue() } else { theme::section_border() })
        .rounded(px(4.))
        .text_color(theme::text_selected())
        .child(input.clone())
        .when(!empty, |d| {
            d.child(
                div()
                    .absolute()
                    .right(px(3.))
                    .top_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .child(tip(
                        "Clear filter",
                        div()
                            .id(ElementId::NamedChild(std::sync::Arc::new(id), "clear".into()))
                            .flex()
                            .p(px(2.))
                            .rounded(px(3.))
                            .cursor_pointer()
                            .hover(|d| d.bg(theme::hover_row()))
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                                window.focus(&handle, cx);
                            })
                            .on_click(move |_, window, cx| on_clear(window, cx))
                            .child(icon("x", 12., theme::text_secondary())),
                    )),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_chord_splits_into_its_keys() {
        assert_eq!(split_chord("Ctrl+Shift+T"), ["Ctrl", "Shift", "T"]);
        assert_eq!(split_chord("Ctrl++"), ["Ctrl", "+"]);
        assert_eq!(split_chord("F7"), ["F7"]);
    }
}
