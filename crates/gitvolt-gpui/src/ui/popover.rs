//! Popovers (`ui/ConfirmDialog.tsx`, `ChoiceDialog.tsx`, `arm.css`): anchored where the action started,
//! no backdrop. `confirm` is `arm::request`'s fallback when there's no control to arm; `ask_choice` is
//! a real choice, more than one way forward. Cancel, Esc, a press outside or a newer one answers none.

use super::Ui;
use super::arm::{ArmAnswer, ArmOption, ArmRequest, Tone};
use super::controls::{self, ButtonKind, check_row};
use super::place::{self, Rect};
use crate::theme;
use gpui::prelude::*;
use gpui::*;
use std::time::Instant;

/// One answer. `danger`: it arms in place before it goes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Choice {
    pub id: SharedString,
    pub label: SharedString,
    pub primary: bool,
    pub danger: bool,
    pub arm: Option<SharedString>,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChoiceRequest {
    pub title: SharedString,
    pub body: Option<SharedString>,
    pub note: Option<SharedString>,
    pub choices: Vec<Choice>,
}

type Shown = (SharedString, Option<SharedString>, Option<SharedString>, Option<Tone>, Option<ArmOption>, bool, bool);
type ThenChoice = Box<dyn FnOnce(Option<SharedString>, &mut Window, &mut App)>;

enum Kind {
    Confirm { req: ArmRequest, then: Option<super::arm::Then>, hints: bool, checked: bool },
    Choice { req: ChoiceRequest, then: Option<ThenChoice> },
}

pub struct Popover {
    kind: Kind,
    anchor: Option<Rect>,
    at: Instant,
    /// The button the keyboard's on, of `buttons`.
    focused: usize,
    bounds: Option<Bounds<Pixels>>,
    /// A press on its risky choice armed it: the row's overlay.
    armed_choice: Option<SharedString>,
}

/// `(id, label, kind)`s in the row's order: Cancel first, the safe answer last.
fn buttons(p: &Popover) -> Vec<(SharedString, SharedString, ButtonKind, bool)> {
    match &p.kind {
        Kind::Confirm { req, .. } => {
            let kind = match req.tone {
                Tone::Danger | Tone::Warn => ButtonKind::Danger,
                Tone::Positive => ButtonKind::Positive,
            };
            vec![("cancel".into(), "Cancel".into(), ButtonKind::Quiet, true), ("confirm".into(), req.confirm_label.clone(), kind, true)]
        }
        Kind::Choice { req, .. } => {
            let mut out = vec![("cancel".into(), SharedString::from("Cancel"), ButtonKind::Quiet, true)];
            for c in req.choices.iter().rev() {
                let kind = if c.danger { ButtonKind::Danger } else { ButtonKind::Positive };
                out.push((c.id.clone(), c.label.clone(), kind, !c.disabled));
            }
            out
        }
    }
}

pub fn confirm(req: ArmRequest, hints: bool, anchor: Option<Bounds<Pixels>>, then: impl FnOnce(ArmAnswer, &mut Window, &mut App) + 'static, window: &mut Window, cx: &mut App) {
    finish(None, window, cx);
    let checked = req.option.as_ref().is_some_and(|o| o.checked);
    // The risky answer isn't where the keyboard starts: Enter on it would be one key away from harm.
    let go = hints && req.tone == Tone::Positive;
    let popover = Popover { kind: Kind::Confirm { req, then: Some(Box::new(then)), hints, checked }, anchor: anchor.map(Rect::of), at: Instant::now(), focused: usize::from(go), bounds: None, armed_choice: None };
    Ui::get_mut(cx).popover = Some(popover);
    cx.refresh_windows();
}

/// Asks. `choices` go safe first: the safe one takes the keyboard, so Enter goes; `then` gets the
/// chosen id, or `None`.
pub fn ask_choice(req: ChoiceRequest, anchor: Option<Bounds<Pixels>>, then: impl FnOnce(Option<SharedString>, &mut Window, &mut App) + 'static, window: &mut Window, cx: &mut App) {
    finish(None, window, cx);
    super::arm::disarm(window, cx);
    let safe = req.choices.iter().position(|c| c.primary && !c.disabled).or_else(|| req.choices.iter().position(|c| !c.disabled && !c.danger));
    let focused = safe.map_or(0, |i| req.choices.len() - i);
    let popover = Popover { kind: Kind::Choice { req, then: Some(Box::new(then)) }, anchor: anchor.map(Rect::of), at: Instant::now(), focused, bounds: None, armed_choice: None };
    Ui::get_mut(cx).popover = Some(popover);
    cx.refresh_windows();
}

fn finish(answer: Option<SharedString>, window: &mut Window, cx: &mut App) {
    let Some(mut p) = Ui::get_mut(cx).popover.take() else { return };
    cx.refresh_windows();
    match &mut p.kind {
        Kind::Confirm { then, checked, .. } => {
            let ok = answer.as_ref().is_some_and(|a| a.as_ref() == "confirm");
            if let Some(then) = then.take() {
                then(ArmAnswer { ok, checked: ok && *checked }, window, cx);
            }
        }
        Kind::Choice { then, .. } => {
            let answer = answer.filter(|a| a.as_ref() != "cancel");
            if let Some(then) = then.take() {
                then(answer, window, cx);
            }
        }
    }
}

/// A button pressed. A risky choice arms first.
fn press(id: SharedString, by_key: bool, window: &mut Window, cx: &mut App) {
    let Some(p) = Ui::get(cx).popover.as_ref() else { return };
    // Only a fresh press after it opened answers it (the settle guard).
    let settle = if by_key { super::arm::KEY_SETTLE } else { super::arm::CLICK_SETTLE };
    if id.as_ref() != "cancel" && p.at.elapsed() < settle {
        return;
    }
    if let Kind::Choice { req, .. } = &p.kind
        && let Some(c) = req.choices.iter().find(|c| c.id == id)
    {
        if c.disabled {
            return;
        }
        if c.danger && p.armed_choice.as_ref() != Some(&id) {
            let label = c.arm.clone().unwrap_or_else(|| SharedString::from(format!("Click again to {}", lower_first(&c.label))));
            if let Some(p) = Ui::get_mut(cx).popover.as_mut() {
                p.armed_choice = Some(id);
                p.at = Instant::now();
                ARMED_LABEL.with(|l| *l.borrow_mut() = label);
            }
            cx.refresh_windows();
            return;
        }
    }
    finish(Some(id), window, cx);
}

thread_local! {
    static ARMED_LABEL: std::cell::RefCell<SharedString> = const { std::cell::RefCell::new(SharedString::new_static("")) };
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().chain(c).collect(),
        None => String::new(),
    }
}

pub(super) fn on_key(k: &Keystroke, window: &mut Window, cx: &mut App) -> bool {
    let Some(p) = Ui::get(cx).popover.as_ref() else { return false };
    let list = buttons(p);
    let focused = p.focused.min(list.len().saturating_sub(1));
    match k.key.as_str() {
        "escape" => {
            if p.armed_choice.is_some() {
                if let Some(p) = Ui::get_mut(cx).popover.as_mut() {
                    p.armed_choice = None;
                }
                cx.refresh_windows();
            } else {
                finish(None, window, cx);
            }
            true
        }
        "enter" | "space" => {
            if !k.modifiers.modified() {
                let id = list[focused].0.clone();
                press(id, true, window, cx);
            }
            true
        }
        "tab" | "left" | "right" => {
            let back = k.key == "left" || (k.key == "tab" && k.modifiers.shift);
            let n = list.len();
            let mut at = focused;
            for _ in 0..n {
                at = if back { (at + n - 1) % n } else { (at + 1) % n };
                if list[at].3 {
                    break;
                }
            }
            if let Some(p) = Ui::get_mut(cx).popover.as_mut() {
                p.focused = at;
                p.armed_choice = None;
            }
            cx.refresh_windows();
            true
        }
        _ => {
            // Every other key stays here too: nothing acts behind it. Zoom went past the router.
            true
        }
    }
}

pub(super) fn on_press(e: &MouseDownEvent, window: &mut Window, cx: &mut App) {
    let Some(p) = Ui::get(cx).popover.as_ref() else { return };
    if p.bounds.is_some_and(|b| b.contains(&e.position)) {
        return;
    }
    finish(None, window, cx);
}

pub(super) fn element(_: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let p = Ui::get(cx).popover.as_ref()?;
    let anchor = p.anchor;
    let list = buttons(p);
    let focused = p.focused.min(list.len().saturating_sub(1));
    let armed = p.armed_choice.clone();
    let (title, body, note, tone, option, hints, checked): Shown = match &p.kind {
        Kind::Confirm { req, hints, checked, .. } => (req.title.clone(), (!req.body.is_empty()).then(|| req.body.clone()), None, Some(req.tone), req.option.clone(), *hints, *checked),
        Kind::Choice { req, .. } => (req.title.clone(), req.body.clone(), req.note.clone(), None, None, false, false),
    };
    let go_focus = hints && tone == Some(Tone::Positive);
    let mut row = div().flex().items_center().justify_end().gap(px(6.)).mt(px(2.));
    if hints {
        row = row.child(
            div()
                .mr_auto()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(11.))
                .text_color(theme::text_secondary())
                .when(go_focus, |d| d.child(hint_key("⏎")).child("go"))
                .child(hint_key("Esc"))
                .child("cancel"),
        );
    }
    for (i, (id, label, kind, enabled)) in list.into_iter().enumerate() {
        let click_id = id.clone();
        let mut b = controls::button(ElementId::Name(SharedString::from(format!("popover-{id}"))), label, kind, enabled)
            .px(px(12.))
            .py(px(4.))
            .on_click(move |_, window, cx| press(click_id.clone(), false, window, cx));
        if i == focused {
            b = b.border_color(theme::text_selected());
        }
        if armed.as_ref() == Some(&id) {
            let text = ARMED_LABEL.with(|l| l.borrow().clone());
            let color = theme::red();
            // A border paints over its children: the button's would show through the label.
            b = b.relative().border_color(transparent_black()).child(
                div()
                    .absolute()
                    .top(px(-1.))
                    .bottom(px(-1.))
                    .right(px(-1.))
                    .min_w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(10.))
                    .whitespace_nowrap()
                    .border_1()
                    .border_color(color)
                    .rounded(px(4.))
                    .bg(theme::mix(color, 0.22, theme::panel_bg1()))
                    .text_color(theme::mix(color, 0.35, theme::text_selected()))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(12.5))
                    .child(text),
            );
        }
        row = row.child(super::anchor(format!("popover:{id}"), b));
    }
    let card = div()
        .id("popover")
        .occlude()
        .flex()
        .flex_col()
        .gap(px(8.))
        .min_w(px(260.))
        .max_w(px(380.))
        .px(px(14.))
        .py(px(12.))
        .bg(theme::panel_bg1())
        .border_1()
        .border_color(theme::section_border())
        .rounded(px(8.))
        .shadow(super::shadow(theme::shadow_popup(), 8., 24.))
        .text_color(theme::text_normal())
        .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).child(title))
        .when_some(body, |d, t| d.child(div().text_size(px(12.5)).text_color(theme::text_secondary()).child(t)))
        .when_some(note, |d, t| d.child(div().mt(px(8.)).child(t)))
        .when_some(option, |d, o| d.child(check_row("popover-option", checked, o.label, |on, _, cx| set_option(on, cx))))
        .child(row);
    Some(
        place::placed(card, move |size, viewport, _| {
            let (x, y) = place::place_popover(anchor, size.width.as_f32(), size.height.as_f32(), viewport.width.as_f32(), viewport.height.as_f32());
            point(px(x), px(y))
        })
        .on_placed(|b, cx| {
            if let Some(p) = Ui::get_mut(cx).popover.as_mut() {
                p.bounds = Some(b);
            }
        })
        .into_any_element(),
    )
}

fn set_option(on: bool, cx: &mut App) {
    if let Some(Popover { kind: Kind::Confirm { checked, .. }, .. }) = Ui::get_mut(cx).popover.as_mut() {
        *checked = on;
        cx.refresh_windows();
    }
}

fn hint_key(text: &'static str) -> Div {
    div().px(px(5.)).border_1().border_color(theme::section_border()).rounded(px(3.)).font_family(crate::app::MONO_FONT).text_size(px(11.)).child(text)
}

#[cfg(test)]
pub fn is_open(cx: &App) -> bool {
    Ui::get(cx).popover.is_some()
}

#[cfg(test)]
pub fn age_for_test(cx: &mut App) {
    if let Some(p) = Ui::get_mut(cx).popover.as_mut() {
        p.at -= std::time::Duration::from_secs(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::prelude::v1::test;
    use std::rc::Rc;

    struct Host;
    impl Render for Host {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            super::super::begin_frame(window, cx);
            div().size_full().relative().children(super::super::overlays(window, cx))
        }
    }

    fn open(cx: &mut TestAppContext) -> VisualTestContext {
        cx.update(super::super::init);
        let window = cx.update(|cx| cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| Host))).unwrap();
        let v = VisualTestContext::from_window(window.into(), cx);
        v.run_until_parked();
        v
    }

    fn request() -> ChoiceRequest {
        ChoiceRequest {
            title: "Push rejected".into(),
            body: None,
            note: None,
            choices: vec![
                Choice { id: "rebase".into(), label: "Rebase".into(), primary: true, ..Default::default() },
                Choice { id: "force".into(), label: "Force push".into(), danger: true, ..Default::default() },
            ],
        }
    }

    #[gpui::test]
    fn enter_takes_the_safe_answer_and_esc_none(cx: &mut TestAppContext) {
        let mut cx = open(cx);
        let got: Rc<RefCell<Vec<Option<String>>>> = Rc::default();
        let log = got.clone();
        cx.update(|window, cx| ask_choice(request(), None, move |a, _, _| log.borrow_mut().push(a.map(|s| s.to_string())), window, cx));
        cx.run_until_parked();
        cx.update(|_, cx| age_for_test(cx));
        cx.simulate_keystrokes("enter");
        assert_eq!(*got.borrow(), [Some("rebase".to_string())]);
        let log = got.clone();
        cx.update(|window, cx| ask_choice(request(), None, move |a, _, _| log.borrow_mut().push(a.map(|s| s.to_string())), window, cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("escape");
        assert_eq!(got.borrow()[1], None);
        assert!(!cx.update(|_, cx| is_open(cx)));
    }

    #[gpui::test]
    fn a_press_outside_cancels(cx: &mut TestAppContext) {
        let mut cx = open(cx);
        let got: Rc<RefCell<Vec<bool>>> = Rc::default();
        let log = got.clone();
        cx.update(|window, cx| confirm(ArmRequest::danger("x", "Sure?", "Do it"), true, None, move |a, _, _| log.borrow_mut().push(a.ok), window, cx));
        cx.run_until_parked();
        cx.simulate_click(point(px(1.), px(1.)), Modifiers::none());
        assert_eq!(*got.borrow(), [false]);
    }
}
