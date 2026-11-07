//! The one toast (`ui/Toast.tsx`, `toastStore.ts`): plain notices for 4 s at the bottom; a warning or
//! anything with links for 8 s at the top, below the toolbar; sticky until dismissed. Hovering pauses
//! its countdown.
//!
//! `toast::show("Copied", cx)`, or `toast::show_with("Couldn't", Options { error: true, detail, actions, .. }, cx)`.

use super::Ui;
use crate::theme;
use gpui::prelude::*;
use gpui::*;
use std::time::{Duration, Instant};

pub const TOAST: Duration = Duration::from_millis(4000);
pub const LONG_TOAST: Duration = Duration::from_millis(8000);
/// Below the title bar, tabs and toolbar, where a top toast sits.
const TOP: f32 = 30. + 34. + 44. + 8.;

#[derive(Clone)]
pub struct ToastAction {
    pub label: SharedString,
    pub run: super::Action,
}

#[derive(Clone, Default)]
pub struct Options {
    pub actions: Vec<ToastAction>,
    pub sticky: bool,
    pub warning: bool,
    pub error: bool,
    pub detail: Option<SharedString>,
}

/// The one duration policy: no caller passes a duration.
pub fn duration(o: &Options) -> Option<Duration> {
    if o.sticky {
        None
    } else if o.error || o.warning || !o.actions.is_empty() {
        Some(LONG_TOAST)
    } else {
        Some(TOAST)
    }
}

#[derive(Default)]
pub struct ToastState {
    pub message: Option<SharedString>,
    pub options: Options,
    generation: u64,
    left: Option<Duration>,
    started: Option<Instant>,
    held: bool,
}

pub fn show(message: impl Into<SharedString>, cx: &mut App) {
    show_with(message, Options::default(), cx);
}

/// A toast that says something failed.
pub fn show_error(message: impl Into<SharedString>, cx: &mut App) {
    show_with(message, Options { error: true, ..Default::default() }, cx);
}

pub fn show_with(message: impl Into<SharedString>, options: Options, cx: &mut App) {
    let left = duration(&options);
    let t = &mut Ui::get_mut(cx).toast;
    t.message = Some(message.into());
    t.options = options;
    t.held = false;
    run_timer(left, cx);
    cx.refresh_windows();
}

fn run_timer(left: Option<Duration>, cx: &mut App) {
    let t = &mut Ui::get_mut(cx).toast;
    t.generation += 1;
    t.left = left;
    t.started = Some(Instant::now());
    let Some(left) = left else { return };
    let generation = t.generation;
    let timer = cx.background_executor().timer(left);
    cx.spawn(async move |cx| {
        timer.await;
        cx.update(|cx| {
            if Ui::get(cx).toast.generation == generation {
                dismiss(cx);
            }
        });
    })
    .detach();
}

pub fn dismiss(cx: &mut App) {
    let t = &mut Ui::get_mut(cx).toast;
    t.generation += 1;
    t.left = None;
    if t.message.take().is_some() {
        t.options = Options::default();
        cx.refresh_windows();
    }
}

/// Esc on a sticky toast. Whether there was one.
pub(super) fn dismiss_sticky(cx: &mut App) -> bool {
    let t = &Ui::get(cx).toast;
    if t.message.is_some() && t.options.sticky {
        dismiss(cx);
        return true;
    }
    false
}

fn hold(hovered: bool, cx: &mut App) {
    let t = &mut Ui::get_mut(cx).toast;
    if t.left.is_none() || t.message.is_none() || t.held == hovered {
        return;
    }
    t.held = hovered;
    if hovered {
        t.generation += 1;
        let spent = t.started.map_or(Duration::ZERO, |s| s.elapsed());
        t.left = t.left.map(|l| l.saturating_sub(spent));
    } else {
        let left = t.left;
        run_timer(left, cx);
    }
}

pub fn message(cx: &App) -> Option<SharedString> {
    Ui::get(cx).toast.message.clone()
}

pub(super) fn element(_: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let t = &Ui::get(cx).toast;
    let message = t.message.clone()?;
    let o = t.options.clone();
    let warning = o.warning;
    // Warnings and anything that invites input show at the top; plain notices at the bottom.
    let top = warning || !o.actions.is_empty();
    let body = div()
        .id("toast")
        .occlude()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(8.))
        .max_w(px(720.))
        .px(px(12.))
        .py(px(6.))
        .rounded(px(4.))
        .bg(theme::panel_bg1())
        .border_1()
        .border_color(if warning { theme::orange() } else { theme::section_border() })
        .text_color(theme::text_selected())
        .on_hover(|hovered, _, cx| hold(*hovered, cx))
        .child(message)
        .children(o.actions.iter().enumerate().map(|(i, a)| {
            let run = a.run.clone();
            let key = format!("toast-action:{}", a.label);
            crate::ui::anchor(key, div()
                .id(("toast-action", i))
                .text_color(theme::link())
                .cursor_pointer()
                .hover(|d| d.underline())
                .on_click(move |_, window, cx| {
                    dismiss(cx);
                    run(window, cx);
                })
                .child(a.label.clone()))
        }))
        .when(o.sticky, |d| {
            d.child(div().id("toast-dismiss").flex().items_center().rounded(px(3.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).on_click(|_, _, cx| dismiss(cx)).child(crate::app::icon("x", 14., theme::text_secondary())))
        })
        .when_some(o.detail, |d, text| d.child(div().w_full().text_size(px(12.)).text_color(theme::text_secondary()).child(text)));
    let row = div().absolute().left_0().right_0().flex().justify_center().child(body);
    Some(if top { row.top(px(TOP)) } else { row.bottom(px(24.)) }.into_any_element())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn the_duration_follows_what_it_says() {
        assert_eq!(duration(&Options::default()), Some(TOAST));
        assert_eq!(duration(&Options { error: true, ..Options::default() }), Some(LONG_TOAST));
        assert_eq!(duration(&Options { warning: true, ..Options::default() }), Some(LONG_TOAST));
        assert_eq!(duration(&Options { sticky: true, ..Options::default() }), None);
    }

    #[gpui::test]
    fn it_goes_when_its_time_is_up(cx: &mut TestAppContext) {
        cx.update(super::super::init);
        cx.update(|cx| show("Copied", cx));
        assert!(cx.update(|cx| message(cx)).is_some());
        cx.executor().advance_clock(TOAST + Duration::from_millis(1));
        cx.run_until_parked();
        assert!(cx.update(|cx| message(cx)).is_none());
    }
}
