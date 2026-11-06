//! Modal dialogs (`auth.css`'s `.modal`, `app/modalKeys.ts`, `ui/PromptDialog.tsx`): over a backdrop;
//! the topmost has the keyboard. A dialog takes focus in its own subtree, outside the app's key
//! context, so no shortcut acts on the tab behind it. Esc or a press on the backdrop closes it, and
//! focus goes back where it was.
//!
//! ```ignore
//! modal::open(Modal::new("about", |window, cx| div().child("..").into_any_element()), window, cx);
//! modal::prompt(PromptRequest { title: "Rename".into(), .. }, |answer, window, cx| { .. }, window, cx);
//! ```

use super::Ui;
use super::controls::{self, ButtonKind, check_row};
use super::input::{InputEvent, TextInput};
use crate::theme;
use gpui::prelude::*;
use gpui::*;
use std::rc::Rc;

actions!(modal, [CloseModal, FocusNext, FocusPrev]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", CloseModal, Some("Modal")),
        KeyBinding::new("tab", FocusNext, Some("Modal")),
        KeyBinding::new("shift-tab", FocusPrev, Some("Modal")),
        KeyBinding::new("tab", FocusNext, Some("OpenScreen")),
        KeyBinding::new("shift-tab", FocusPrev, Some("OpenScreen")),
    ]);
}

pub type Build = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

pub struct Modal {
    pub id: SharedString,
    pub width: f32,
    /// Its box's whole style: `false` for one that draws its own (the palette).
    pub framed: bool,
    /// How far down the window it sits, for one that's not centred (the palette's 12vh).
    pub top: Option<f32>,
    /// Dropped from where its action started, in the anchored-popover family: no shade behind it, a press outside cancels.
    pub anchored: Option<super::place::Rect>,
    build: Build,
    on_close: Option<super::Action>,
    focus: Option<FocusHandle>,
    /// On the dialog's box: what Tab has to stay inside, whichever control `focus` is.
    trap: Option<FocusHandle>,
    return_to: Option<FocusHandle>,
    /// The chord that opened it closes it (the palette's Ctrl+P).
    pub toggle: Option<String>,
    /// Keeps what the dialog's entities need alive for as long as it's open.
    pub keep: Vec<Subscription>,
}

impl Modal {
    pub fn new(id: impl Into<SharedString>, build: impl Fn(&mut Window, &mut App) -> AnyElement + 'static) -> Self {
        Self { id: id.into(), width: 420., framed: true, top: None, anchored: None, build: Rc::new(build), on_close: None, focus: None, trap: None, return_to: None, toggle: None, keep: Vec::new() }
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn unframed(mut self) -> Self {
        self.framed = false;
        self
    }

    pub fn anchored(mut self, anchor: Option<Bounds<Pixels>>) -> Self {
        self.anchored = Some(anchor.map_or(super::place::Rect::at(0., 0.), super::place::Rect::of));
        self
    }

    pub fn top(mut self, top: f32) -> Self {
        self.top = Some(top);
        self
    }

    pub fn toggle(mut self, chord: String) -> Self {
        self.toggle = Some(chord);
        self
    }

    pub fn on_close(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(f));
        self
    }

    /// Where the keyboard goes when it opens: a field of the dialog.
    pub fn focus(mut self, handle: FocusHandle) -> Self {
        self.focus = Some(handle);
        self
    }
}

/// Opens it on top.
pub fn open(mut modal: Modal, window: &mut Window, cx: &mut App) {
    super::menu::close(window, cx);
    super::tooltip::hide(cx);
    modal.return_to = window.focused(cx);
    modal.trap = Some(cx.focus_handle());
    let handle = modal.focus.get_or_insert_with(|| cx.focus_handle()).clone();
    window.focus(&handle, cx);
    Ui::get_mut(cx).modals.push(modal);
    cx.refresh_windows();
}

/// Closes the one with `id`, wherever in the stack.
pub fn close(id: &str, window: &mut Window, cx: &mut App) {
    let ui = Ui::get_mut(cx);
    let Some(at) = ui.modals.iter().rposition(|m| m.id.as_ref() == id) else { return };
    let modal = ui.modals.remove(at);
    let top = at == ui.modals.len();
    if top && let Some(back) = &modal.return_to {
        window.focus(back, cx);
    }
    cx.refresh_windows();
    if let Some(f) = &modal.on_close {
        f(window, cx);
    }
}

fn close_top(window: &mut Window, cx: &mut App) {
    if let Some(id) = Ui::get(cx).modals.last().map(|m| m.id.clone()) {
        close(&id, window, cx);
    }
}

/// The topmost dialog's own chord closes it. Whether it did.
pub(super) fn toggle_key(combo: &str, window: &mut Window, cx: &mut App) -> bool {
    if Ui::get(cx).modals.last().and_then(|m| m.toggle.as_deref()) == Some(combo) {
        close_top(window, cx);
        return true;
    }
    false
}

/// For the real-input flows: what's open, bottom first.
pub fn ids(cx: &App) -> Vec<String> {
    Ui::get(cx).modals.iter().map(|m| m.id.to_string()).collect()
}

pub fn is_open(id: &str, cx: &App) -> bool {
    Ui::get(cx).modals.iter().any(|m| m.id.as_ref() == id)
}

type Shown = (SharedString, f32, bool, Option<f32>, Option<super::place::Rect>, Build, FocusHandle, FocusHandle);

pub(super) fn elements(window: &mut Window, cx: &mut App) -> Vec<AnyElement> {
    let items: Vec<Shown> = Ui::get(cx).modals.iter().map(|m| (m.id.clone(), m.width, m.framed, m.top, m.anchored, m.build.clone(), m.focus.clone().expect("set on opening"), m.trap.clone().expect("set on opening"))).collect();
    items
        .into_iter()
        .map(|(id, width, framed, top, anchored, build, focus, trap)| {
            let content = build(window, cx);
            let body = div().id(ElementId::Name(SharedString::from(format!("{id}-dialog")))).role(Role::Dialog).track_focus(&trap).occlude().child(crate::ui::spin::cover()).flex().flex_col().flex_none().when(anchored.is_none(), |d| d.w(px(width))).when(anchored.is_some(), |d| d.max_w(px(width))).max_w(window.viewport_size().width - px(32.));
            let body = if framed {
                body.gap(px(12.))
                    .when(anchored.is_none(), |d| d.px(px(20.)).py(px(16.)).bg(theme::panel_bg()))
                    .when(anchored.is_some(), |d| d.px(px(14.)).py(px(12.)).bg(theme::panel_bg1()))
                    .border_1()
                    .border_color(theme::section_border())
                    .rounded(px(8.))
                    .shadow(super::shadow(theme::shadow_modal(), 10., 32.))
                    .text_color(theme::text_normal())
            } else {
                body
            };
            let body = body.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(content);
            if let Some(anchor) = anchored {
                let placed = super::place::placed(body, move |size, vp, _| {
                    let (x, y) = super::place::place_popover(Some(anchor), size.width.as_f32(), size.height.as_f32(), vp.width.as_f32(), vp.height.as_f32());
                    point(px(x), px(y))
                });
                return div()
                    .id(ElementId::Name(SharedString::from(format!("{id}-backdrop"))))
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .occlude()
            .child(crate::ui::spin::cover())
                    .key_context("Modal")
                    .track_focus(&focus)
                    .on_action(|_: &CloseModal, window, cx| close_top(window, cx))
                    .on_action({
                        let root = trap.clone();
                        move |_: &FocusNext, window, cx| step_focus(&root, false, window, cx)
                    })
                    .on_action({
                        let root = trap.clone();
                        move |_: &FocusPrev, window, cx| step_focus(&root, true, window, cx)
                    })
                    .on_mouse_down(MouseButton::Left, |_, window, cx| close_top(window, cx))
                    .child(placed)
                    .into_any_element();
            }
            div()
                .id(ElementId::Name(SharedString::from(format!("{id}-backdrop"))))
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
            .child(crate::ui::spin::cover())
                .flex()
                .justify_center()
                .bg(theme::backdrop())
                .key_context("Modal")
                .track_focus(&focus)
                .when_some(top, |d, t| d.items_start().pt(px(t)))
                .when(top.is_none(), |d| d.items_center())
                .on_action(|_: &CloseModal, window, cx| close_top(window, cx))
                .on_action({
                    let root = trap.clone();
                    move |_: &FocusNext, window, cx| step_focus(&root, false, window, cx)
                })
                .on_action({
                    let root = trap.clone();
                    move |_: &FocusPrev, window, cx| step_focus(&root, true, window, cx)
                })
                .on_mouse_down(MouseButton::Left, |_, window, cx| close_top(window, cx))
                .child(body)
                .into_any_element()
        })
        .collect()
}

/// A dialog's title (`.modal h2`).
/// Tab inside a dialog: the window's tab stops hold what's behind it too, which a dialog traps out, as the web's.
pub fn step_focus(root: &FocusHandle, back: bool, window: &mut Window, cx: &mut App) {
    for _ in 0..256 {
        if back {
            window.focus_prev(cx);
        } else {
            window.focus_next(cx);
        }
        if root.contains_focused(window, cx) {
            return;
        }
    }
    window.focus(root, cx);
}

pub fn title(text: impl Into<SharedString>) -> Div {
    div().flex().items_center().gap(px(8.)).mb(px(9.)).text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).child(text.into())
}

/// `.modal-actions`.
pub fn actions_row() -> Div {
    div().flex().justify_end().gap(px(8.))
}

pub type Validate = Rc<dyn Fn(&str) -> Option<String>>;
type OnAnswer = Rc<dyn Fn(Option<PromptAnswer>, &mut Window, &mut App)>;

/// A one-field question.
#[derive(Clone, Default)]
pub struct PromptRequest {
    pub title: SharedString,
    pub label: SharedString,
    pub initial: String,
    pub confirm_label: SharedString,
    /// Live: why the value can't be used.
    pub validate: Option<Validate>,
    /// The confirm stays disabled while the value equals `initial`.
    pub require_change: bool,
    pub checkbox: Option<(SharedString, bool)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptAnswer {
    pub value: String,
    pub checked: bool,
}

/// Asks; `None` on Cancel, Esc or a backdrop click.
pub fn prompt(req: PromptRequest, then: impl Fn(Option<PromptAnswer>, &mut Window, &mut App) + 'static, window: &mut Window, cx: &mut App) {
    const ID: &str = "prompt";
    let then: OnAnswer = Rc::new(then);
    let input = cx.new(|cx| {
        let mut i = TextInput::new(cx);
        i.set_text(&req.initial, cx);
        i.select_all_text(cx);
        i
    });
    let checked = Rc::new(std::cell::Cell::new(req.checkbox.as_ref().is_some_and(|c| c.1)));
    let answered = Rc::new(std::cell::Cell::new(false));
    let stops = [cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true)];
    let tried = Rc::new(std::cell::Cell::new(false));

    let blocked = {
        let req = req.clone();
        Rc::new(move |value: &str| req.validate.as_ref().and_then(|v| v(value)).is_some() || (req.require_change && value == req.initial))
    };
    let submit: super::Action = {
        let (input, checked, then, answered, tried, blocked) = (input.clone(), checked.clone(), then.clone(), answered.clone(), tried.clone(), blocked.clone());
        Rc::new(move |window, cx| {
            tried.set(true);
            let value = input.read(cx).text().to_string();
            cx.refresh_windows();
            if blocked(&value) {
                return;
            }
            answered.set(true);
            close(ID, window, cx);
            then(Some(PromptAnswer { value, checked: checked.get() }), window, cx);
        })
    };
    // An Enter in the field submits; `cx.subscribe` runs after the key's own update, with no window of
    // its own, so the answer waits for the window's next turn.
    let handle = window.window_handle();
    let enter = cx.subscribe(&input, {
        let submit = submit.clone();
        move |_, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Submit { .. }) {
                let submit = submit.clone();
                handle.update(cx, move |_, window, cx| window.defer(cx, move |window, cx| submit(window, cx))).ok();
            } else if matches!(e, InputEvent::Changed) {
                cx.refresh_windows();
            }
        }
    });
    let focus = input.read(cx).focus.clone();
    let on_close = {
        let (then, answered) = (then, answered);
        move |window: &mut Window, cx: &mut App| {
            if !answered.get() {
                then(None, window, cx);
            }
        }
    };
    let mut modal = Modal::new(ID, {
        let (input, checked, submit, tried) = (input, checked, submit, tried);
        move |_, cx| {
            let value = input.read(cx).text().to_string();
            let error = req.validate.as_ref().and_then(|v| v(&value));
            let unchanged = value == req.initial;
            let blocked = error.is_some() || (req.require_change && unchanged);
            let show_error = error.filter(|_| !value.is_empty() && (tried.get() || !unchanged));
            let focused = true;
            let (submit2, submit3) = (submit.clone(), submit.clone());
            let checked2 = checked.clone();
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(title(req.title.clone()))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(req.label.clone())
                        .child(div().h(px(30.)).px(px(8.)).flex().items_center().bg(theme::app_bg()).border_1().border_color(if focused { theme::blue() } else { theme::section_border() }).rounded(px(4.)).text_color(theme::text_selected()).child(input.clone())),
                )
                .when_some(show_error, |d, e| d.child(div().text_color(theme::red()).child(e)))
                .when_some(req.checkbox.clone(), |d, (label, _)| {
                    let toggled = checked2.clone();
                    let row = check_row("prompt-check", checked.get(), label, move |on, _, cx| {
                        toggled.set(on);
                        cx.refresh_windows();
                    });
                    d.child(controls::tabbable(row.border_1().border_color(transparent_black()).rounded(px(3.)).ml(px(-1.)), &stops[0], move |_, cx| {
                        checked2.set(!checked2.get());
                        cx.refresh_windows();
                    }))
                })
                .child(
                    actions_row()
                        .child(controls::tabbable(controls::button("prompt-cancel", "Cancel", ButtonKind::Plain, true).on_click(|_, window, cx| close(ID, window, cx)), &stops[1], |window, cx| close(ID, window, cx)))
                        .child(controls::tabbable(controls::button("prompt-ok", req.confirm_label.clone(), ButtonKind::Primary, !blocked).when(!blocked, |d| d.on_click(move |_, window, cx| submit2(window, cx))), &stops[2], move |window, cx| submit3(window, cx))),
                )
                .into_any_element()
        }
    })
    .focus(focus)
    .on_close(on_close);
    modal.keep.push(enter);
    open(modal, window, cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::prelude::v1::test;

    struct Host {
        focus: FocusHandle,
        keys: Rc<RefCell<u32>>,
    }
    actions!(modal_test, [AppKey]);
    impl Render for Host {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            super::super::begin_frame(window, cx);
            let keys = self.keys.clone();
            div()
                .size_full()
                .relative()
                .child(div().size_full().key_context("App").track_focus(&self.focus).on_action(move |_: &AppKey, _, _| *keys.borrow_mut() += 1))
                .children(super::super::overlays(window, cx))
        }
    }

    fn open_host(cx: &mut TestAppContext) -> (VisualTestContext, Rc<RefCell<u32>>) {
        cx.update(|cx| {
            super::super::init(cx);
            cx.bind_keys([KeyBinding::new("x", AppKey, Some("App"))]);
        });
        let keys: Rc<RefCell<u32>> = Rc::default();
        let host = keys.clone();
        let window = cx
            .update(|cx| {
                cx.open_window(WindowOptions::default(), move |window, cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus, cx);
                    cx.new(|_| Host { focus, keys: host })
                })
            })
            .unwrap();
        let v = VisualTestContext::from_window(window.into(), cx);
        v.run_until_parked();
        (v, keys)
    }

    #[gpui::test]
    fn a_dialog_keeps_the_apps_keys_out_and_esc_closes_it(cx: &mut TestAppContext) {
        let (mut cx, keys) = open_host(cx);
        cx.simulate_keystrokes("x");
        assert_eq!(*keys.borrow(), 1);
        cx.update(|window, cx| open(Modal::new("about", |_, _| div().child("About").into_any_element()), window, cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("x");
        assert_eq!(*keys.borrow(), 1, "not behind it");
        cx.simulate_keystrokes("escape");
        assert!(!cx.update(|_, cx| is_open("about", cx)));
        cx.run_until_parked();
        cx.simulate_keystrokes("x");
        assert_eq!(*keys.borrow(), 2, "focus went back");
    }

    #[gpui::test]
    fn a_prompt_validates_and_answers(cx: &mut TestAppContext) {
        let (mut cx, _) = open_host(cx);
        let got: Rc<RefCell<Vec<Option<String>>>> = Rc::default();
        let log = got.clone();
        cx.update(|window, cx| {
            let req = PromptRequest { title: "Rename".into(), label: "Name".into(), initial: "old".into(), confirm_label: "Rename".into(), validate: Some(Rc::new(|v| v.contains(' ').then(|| "No spaces".to_string()))), require_change: true, checkbox: None };
            prompt(req, move |a, _, _| log.borrow_mut().push(a.map(|a| a.value)), window, cx);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(got.borrow().is_empty(), "unchanged: nothing to do");
        cx.simulate_input("a b");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(got.borrow().is_empty(), "invalid");
        cx.simulate_keystrokes("ctrl-a");
        cx.simulate_input("new");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(*got.borrow(), [Some("new".to_string())]);
    }

    #[gpui::test]
    fn tab_reaches_a_prompts_checkbox_and_buttons(cx: &mut TestAppContext) {
        let (mut cx, _) = open_host(cx);
        let got: Rc<RefCell<Vec<Option<PromptAnswer>>>> = Rc::default();
        let log = got.clone();
        cx.update(|window, cx| {
            let req = PromptRequest { title: "New".into(), label: "Name".into(), initial: "a".into(), confirm_label: "Make".into(), checkbox: Some(("Check out".into(), true)), ..Default::default() };
            prompt(req, move |a, _, _| log.borrow_mut().push(a), window, cx);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("tab space");
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(got.borrow().is_empty(), "Enter on the checkbox toggles it: Space and Enter both did");
        cx.simulate_keystrokes("space tab tab enter");
        cx.run_until_parked();
        assert_eq!(*got.borrow(), [Some(PromptAnswer { value: "a".into(), checked: false })]);
    }
}
