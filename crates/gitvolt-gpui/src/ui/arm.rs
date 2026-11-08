//! Arm in place (`AGENTS.md`, `ui/arm/*`): the first click arms the control, its label saying what a
//! second click does (in red if destructive), drawn over its own space so nothing moves; the second
//! click runs it. A click anywhere else, Esc, the control going away or another control arming
//! cancels. There is no timer.
//!
//! ```ignore
//! ui::anchor("toolbar.discard", button.on_click(|_, window, cx| {
//!     arm::request(arm::ArmRequest::danger("Click again to discard 5 files", "Discard 5 files?", "Discard"),
//!         Origin::control("toolbar.discard"), |answer, window, cx| { if answer.ok { .. } }, window, cx);
//! }))
//! ```
//! A menu row's `run` passes `Origin::current(cx)`: the row itself arms, and the menu stays open. With
//! no control to arm (a keyboard shortcut, `Origin::None`), it asks in a popover.

use super::Ui;
use super::place::{self, Rect};
use crate::theme;
use gpui::prelude::*;
use gpui::*;
use std::time::{Duration, Instant};

/// A confirm must be a new gesture, this long after arming: a double click's second press never
/// confirms. A minimum, not a timer; the armed state doesn't expire.
pub const CLICK_SETTLE: Duration = Duration::from_millis(200);
pub const KEY_SETTLE: Duration = Duration::from_millis(350);

/// The anchors an armed label stays inside (`bounds`).
const BOUNDS: &str = "arm-bounds:";

/// A container an armed label inside it doesn't grow out of, as the web's nearest scroll box
/// (`grow.ts`'s `armBounds`): a panel's body.
pub fn bounds(key: &str, child: impl IntoElement) -> super::tooltip::Anchor {
    super::anchor(format!("{BOUNDS}{key}"), child)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Danger,
    Positive,
    Warn,
}

impl Tone {
    pub fn color(self) -> Hsla {
        match self {
            Tone::Danger => theme::red(),
            Tone::Positive => theme::green(),
            Tone::Warn => theme::orange(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArmOption {
    pub label: SharedString,
    pub checked: bool,
    pub detail: Option<SharedString>,
    pub note: Option<SharedString>,
}

/// One confirmation: `arm` is the armed control's label; `title`, `body`, `confirm_label` the
/// popover's, when there's no control to arm.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmRequest {
    pub arm: SharedString,
    pub tone: Tone,
    pub caption: Option<SharedString>,
    pub title: SharedString,
    pub body: SharedString,
    pub confirm_label: SharedString,
    pub option: Option<ArmOption>,
    /// An icon button's own icon: the armed label keeps it.
    pub icon: Option<&'static str>,
}

impl ArmRequest {
    pub fn new(tone: Tone, arm: impl Into<SharedString>, title: impl Into<SharedString>, confirm_label: impl Into<SharedString>) -> Self {
        Self { arm: arm.into(), tone, caption: None, title: title.into(), body: SharedString::default(), confirm_label: confirm_label.into(), option: None, icon: None }
    }

    pub fn icon(mut self, name: &'static str) -> Self {
        self.icon = Some(name);
        self
    }

    pub fn danger(arm: impl Into<SharedString>, title: impl Into<SharedString>, confirm_label: impl Into<SharedString>) -> Self {
        Self::new(Tone::Danger, arm, title, confirm_label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmAnswer {
    pub ok: bool,
    pub checked: bool,
}

/// What a confirmation arms.
#[derive(Debug, Clone, PartialEq)]
pub enum Origin {
    /// A control under `ui::anchor(key, ..)`. `cover`: another anchor whose box the overlay covers
    /// (a row's buttons); `grow_right`: which side a long label grows to first.
    Control { key: SharedString, cover: Option<SharedString>, grow_right: Option<bool> },
    /// The menu row that's running.
    MenuRow,
    /// Nothing to arm: a popover, centred, with hints.
    None,
}

impl Origin {
    pub fn control(key: impl Into<SharedString>) -> Self {
        Origin::Control { key: key.into(), cover: None, grow_right: None }
    }

    /// The row when a menu is running one; else nothing.
    pub fn current(cx: &App) -> Self {
        if Ui::get(cx).arm.running_row { Origin::MenuRow } else { Origin::None }
    }
}

pub struct Armed {
    pub req: ArmRequest,
    pub origin: Origin,
    pub at: Instant,
    pub checked: bool,
    /// The overlay's box, while it's drawn.
    pub(crate) overlay: Option<Bounds<Pixels>>,
    then: Option<Then>,
}

/// What a confirm answers to.
pub type Then = Box<dyn FnOnce(ArmAnswer, &mut Window, &mut App)>;

#[derive(Default)]
pub struct ArmState {
    pub armed: Option<Armed>,
    /// A menu is running a row: `Origin::current` is that row.
    pub(crate) running_row: bool,
    /// The running row's action asks later: its menu stays open meanwhile.
    pub(crate) held: bool,
}

/// Arms `origin`; `then` answers on the second click (`ok`) or once it's disarmed (`!ok`). Only one
/// control is armed app-wide: an armed one answers `!ok` first. With no control, asks in a popover.
pub fn request(req: ArmRequest, origin: Origin, then: impl FnOnce(ArmAnswer, &mut Window, &mut App) + 'static, window: &mut Window, cx: &mut App) {
    disarm(window, cx);
    // A control with an option has no room for it in place, and a missing one has no place at all.
    let usable = match &origin {
        Origin::Control { key, .. } => req.option.is_none() && Ui::get(cx).anchor(key).is_some(),
        Origin::MenuRow => Ui::get(cx).menu.is_some(),
        Origin::None => false,
    };
    if !usable {
        super::popover::confirm(req, matches!(origin, Origin::None), origin_bounds(&origin, cx), then, window, cx);
        return;
    }
    let checked = req.option.as_ref().is_some_and(|o| o.checked);
    Ui::get_mut(cx).arm.armed = Some(Armed { req, origin, at: Instant::now(), checked, overlay: None, then: Some(Box::new(then)) });
    super::tooltip::hide(cx);
    cx.refresh_windows();
}

/// For a menu row's action that may ask after an `await`: the menu stays open until `release_row`, so
/// the question can arm the row. What it returns goes to `release_row`.
pub fn hold_row(cx: &mut App) -> Option<u64> {
    let ui = Ui::get_mut(cx);
    if !ui.arm.running_row {
        return None;
    }
    ui.arm.held = true;
    ui.menu.as_ref().map(|m| m.seq)
}

/// The hold's over: the menu closes unless the row armed meanwhile.
pub fn release_row(hold: Option<u64>, window: &mut Window, cx: &mut App) {
    if hold.is_some() && Ui::get(cx).menu.as_ref().map(|m| m.seq) == hold && !armed_on_row(cx) {
        super::menu::close(window, cx);
    }
}

fn origin_bounds(origin: &Origin, cx: &App) -> Option<Bounds<Pixels>> {
    match origin {
        Origin::Control { key, .. } => Ui::get(cx).anchor(key),
        _ => None,
    }
}

fn answer(ok: bool, window: &mut Window, cx: &mut App) {
    let Some(mut a) = Ui::get_mut(cx).arm.armed.take() else { return };
    cx.refresh_windows();
    if let Some(then) = a.then.take() {
        then(ArmAnswer { ok, checked: ok && a.checked }, window, cx);
    }
}

/// The armed control's second click: a fresh press, past the settle guard. Whether it confirmed.
pub fn confirm_click(window: &mut Window, cx: &mut App) -> bool {
    let ui = Ui::get(cx);
    let Some(a) = ui.arm.armed.as_ref() else { return false };
    let fresh = ui.last_press.is_some_and(|p| p.duration_since(a.at) >= CLICK_SETTLE);
    if fresh {
        answer(true, window, cx);
    }
    fresh
}

/// The armed control's Enter or Space: past the longer settle, so a held key's repeats never confirm.
pub fn confirm_key(window: &mut Window, cx: &mut App) -> bool {
    let Some(a) = Ui::get(cx).arm.armed.as_ref() else { return false };
    let fresh = a.at.elapsed() >= KEY_SETTLE;
    if fresh {
        answer(true, window, cx);
    }
    fresh
}

/// Cancels what's armed; its action does nothing.
pub fn disarm(window: &mut Window, cx: &mut App) {
    answer(false, window, cx);
}

pub fn set_option(checked: bool, cx: &mut App) {
    if let Some(a) = Ui::get_mut(cx).arm.armed.as_mut() {
        a.checked = checked;
        cx.refresh_windows();
    }
}

pub fn is_armed(cx: &App) -> bool {
    Ui::get(cx).arm.armed.is_some()
}

/// The key of the control that's armed, if one is.
pub fn armed_control(cx: &App) -> Option<SharedString> {
    match &Ui::get(cx).arm.armed.as_ref()?.origin {
        Origin::Control { key, .. } => Some(key.clone()),
        _ => None,
    }
}

/// Where the armed label is drawn, while it is: for the real-input flows.
pub fn overlay(cx: &App) -> Option<Bounds<Pixels>> {
    Ui::get(cx).arm.armed.as_ref()?.overlay
}

pub(super) fn is_armed_in_place(cx: &App) -> bool {
    Ui::get(cx).arm.armed.as_ref().is_some_and(|a| matches!(a.origin, Origin::Control { .. }))
}

pub fn armed_on_row(cx: &App) -> bool {
    Ui::get(cx).arm.armed.as_ref().is_some_and(|a| a.origin == Origin::MenuRow)
}

/// A press outside the armed control disarms; the press goes on to what it hit.
pub(super) fn on_press(e: &MouseDownEvent, window: &mut Window, cx: &mut App) {
    let ui = Ui::get(cx);
    let Some(a) = ui.arm.armed.as_ref() else { return };
    // A menu row's own press is the menu's to judge.
    if a.origin == Origin::MenuRow {
        return;
    }
    if a.overlay.is_some_and(|o| o.contains(&e.position)) {
        return;
    }
    disarm(window, cx);
}

/// The control went away: the thing it would act on changed.
pub(super) fn check_anchor(window: &mut Window, cx: &mut App) {
    let ui = Ui::get(cx);
    let Some(Armed { origin: Origin::Control { key, .. }, .. }) = ui.arm.armed.as_ref() else { return };
    if !ui.last_anchors.contains_key(key) {
        window.defer(cx, disarm);
    }
}

pub(super) fn element(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let ui = Ui::get(cx);
    let a = ui.arm.armed.as_ref()?;
    let Origin::Control { key, cover, grow_right } = &a.origin else { return None };
    let bounds = cover.as_ref().and_then(|c| ui.anchor(c)).or_else(|| ui.anchor(key))?;
    let covering = cover.is_some();
    let (label, tone, caption, glyph) = (a.req.arm.clone(), a.req.tone, a.req.caption.clone(), a.req.icon);
    let grow_right = *grow_right;
    let needed = super::text_width(&label, 12.5, FontWeight::SEMIBOLD, cx) + 22. + if glyph.is_some() { 19. } else { 0. };
    let vw = window.viewport_size().width.as_f32();
    let box_ = Rect::of(bounds);
    let width = bounds.size.width.as_f32();
    let containers = ui.anchors_starting(&[BOUNDS]).into_iter().map(|(_, b)| Rect::of(b));
    let (grow, max_w) = place::arm_placement((box_.left, box_.right), place::arm_bounds(box_, containers, vw), needed, grow_right);
    let w = needed.max(width).min(max_w);
    let left = if grow { box_.left } else { box_.right - w };
    let h = bounds.size.height.as_f32();
    let color = tone.color();
    let overlay = div()
        .id("arm-overlay")
        .absolute()
        .left(px(left))
        .top(px(box_.top))
        .w(px(w))
        .h(px(h))
        .occlude()
        .flex()
        .items_center()
        .gap(px(6.))
        .when(covering, |d| d.justify_center())
        .px(px(10.))
        .border_1()
        .border_color(color)
        .rounded(px(4.))
        .bg(theme::mix(color, 0.22, theme::panel_bg()))
        .hover(move |d| d.bg(theme::mix(color, 0.32, theme::panel_bg())))
        .text_color(theme::mix(color, 0.35, theme::text_selected()))
        .text_size(px(12.5))
        .font_weight(FontWeight::SEMIBOLD)
        .overflow_hidden()
        .whitespace_nowrap()
        .cursor_pointer()
        .on_click(|_, window, cx| {
            confirm_click(window, cx);
        })
        .children(glyph.map(|g| crate::app::icon(g, 13., theme::mix(color, 0.35, theme::text_selected()))))
        .child(label);
    Ui::get_mut(cx).arm.armed.as_mut()?.overlay = Some(Bounds::new(point(px(left), px(box_.top)), size(px(w), px(h))));
    let shown = div().absolute().top_0().left_0().size_0().child(overlay);
    Some(match caption {
        Some(text) => {
            // `max-width: 320px` of a box as wide as its text, on the side the control grew from. A parent of no size would squeeze it to a letter a line.
            let cap_w = (super::text_width(&text, 12., FontWeight::NORMAL, cx) + 22.).min(320.);
            let cap_left = if grow { left } else { left + w - cap_w };
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_0()
                .child(shown)
                .child(div().absolute().left(px(cap_left)).top(px(box_.bottom + 6.)).w(px(cap_w)).px(px(10.)).py(px(6.)).bg(theme::panel_bg1()).border_1().border_color(theme::section_border()).rounded(px(6.)).shadow(vec![BoxShadow { color: theme::shadow_popup(), offset: point(px(0.), px(6.)), blur_radius: px(20.), spread_radius: px(0.), inset: false }]).text_color(theme::mix(color, 0.45, theme::text_selected())).text_size(px(12.)).child(text))
                .into_any_element()
        }
        None => shown.into_any_element(),
    })
}

/// For a test: a press now, so a confirm counts as fresh.
#[cfg(test)]
pub fn age_for_test(cx: &mut App) {
    if let Some(a) = Ui::get_mut(cx).arm.armed.as_mut() {
        a.at -= Duration::from_secs(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;
    use std::rc::Rc;

    struct Host {
        answers: Rc<std::cell::RefCell<Vec<bool>>>,
    }
    impl Render for Host {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            super::super::begin_frame(window, cx);
            let answers = self.answers.clone();
            div()
                .size_full()
                .relative()
                .child(super::super::anchor(
                    "discard",
                    div().id("discard").w(px(60.)).h(px(24.)).on_click(move |_, window, cx| {
                        let answers = answers.clone();
                        request(ArmRequest::danger("Click again to discard 5 files", "Discard?", "Discard"), Origin::control("discard"), move |a, _, _| answers.borrow_mut().push(a.ok), window, cx);
                    }),
                ))
                .children(super::super::overlays(window, cx))
        }
    }

    fn open(cx: &mut TestAppContext) -> (VisualTestContext, Rc<std::cell::RefCell<Vec<bool>>>) {
        cx.update(super::super::init);
        let answers: Rc<std::cell::RefCell<Vec<bool>>> = Rc::default();
        let host = answers.clone();
        let window = cx.update(|cx| cx.open_window(WindowOptions::default(), move |_, cx| cx.new(|_| Host { answers: host }))).unwrap();
        let v = VisualTestContext::from_window(window.into(), cx);
        v.run_until_parked();
        (v, answers)
    }

    #[gpui::test]
    fn a_second_click_confirms_after_the_settle(cx: &mut TestAppContext) {
        let (mut cx, answers) = open(cx);
        let at = point(px(10.), px(10.));
        cx.simulate_click(at, Modifiers::none());
        cx.run_until_parked();
        assert!(cx.update(|_, cx| is_armed(cx)));
        // A double click's second press is too soon, and doesn't confirm.
        cx.simulate_click(at, Modifiers::none());
        assert!(cx.update(|_, cx| is_armed(cx)), "still armed");
        cx.update(|_, cx| age_for_test(cx));
        cx.simulate_click(at, Modifiers::none());
        assert_eq!(*answers.borrow(), [true]);
        assert!(!cx.update(|_, cx| is_armed(cx)));
    }

    #[gpui::test]
    fn esc_or_a_click_elsewhere_cancels(cx: &mut TestAppContext) {
        let (mut cx, answers) = open(cx);
        let at = point(px(10.), px(10.));
        cx.simulate_click(at, Modifiers::none());
        cx.run_until_parked();
        cx.simulate_keystrokes("escape");
        assert_eq!(*answers.borrow(), [false]);
        cx.simulate_click(at, Modifiers::none());
        cx.run_until_parked();
        cx.simulate_click(point(px(300.), px(300.)), Modifiers::none());
        assert_eq!(*answers.borrow(), [false, false]);
    }
}
