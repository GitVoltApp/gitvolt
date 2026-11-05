//! The shared UI: what every feature builds on. `init` once, `begin_frame` at the top of the root's
//! `render`, `overlays` as the root's last children. State that floats over the window lives in the
//! `Ui` global, so any view (or a menu row's closure) can open or close it without a handle.
//!
//! - `input`: a text field and a textarea.
//! - `tooltip`: `tip(text, child)`, hover cards.
//! - `menu`: context and dropdown menus.
//! - `arm`: arm in place; `popover`: confirm and choice; `modal`: dialogs, `prompt`.
//! - `controls`: buttons, select, switch, checkbox, keycaps.
//! - `toast`, `banner`.
//! - `fuzzy`: `fuzzysort`'s ranking.

// What the later features build on is here before they use it.
#![allow(dead_code)]

pub mod arm;
pub mod banner;
pub mod controls;
pub mod fuzzy;
pub mod input;
pub mod menu;
pub mod modal;
pub mod place;
pub mod popover;
pub mod toast;
pub mod tooltip;

use gpui::prelude::*;
use gpui::*;
use std::collections::HashMap;

/// What a row, a link or a dialog runs.
pub type Action = std::rc::Rc<dyn Fn(&mut Window, &mut App)>;

pub type AppKeys = std::rc::Rc<dyn Fn(&Keystroke, &mut Window, &mut App) -> bool>;

/// The floating state of the window.
#[derive(Default)]
pub struct Ui {
    pub tips: tooltip::Tips,
    pub menu: Option<menu::MenuState>,
    pub menu_seq: u64,
    pub arm: arm::ArmState,
    pub popover: Option<popover::Popover>,
    pub modals: Vec<modal::Modal>,
    pub toast: toast::ToastState,
    /// Where the anchors (`anchor`, `tip`) are this frame and were last frame.
    anchors: HashMap<SharedString, Bounds<Pixels>>,
    last_anchors: HashMap<SharedString, Bounds<Pixels>>,
    /// When the last press went down: a confirm must be a fresh one (`arm`).
    pub last_press: Option<std::time::Instant>,
    /// The mouse press in progress: where, and whether it's the right button.
    pub(crate) held: Option<((f32, f32), bool)>,
    /// A press on this menu's opener closed it: its own handler mustn't reopen it.
    pub(crate) swallow_anchor: Option<SharedString>,
    /// The UI font, for sizing what floats before it's laid out.
    font: Option<Font>,
    measurer: Option<std::rc::Rc<WindowTextSystem>>,
    /// The app layer's shortcuts: whether it took the key. Set by the app's view.
    pub app_keys: Option<AppKeys>,
    /// Ends the keystroke interceptor with the app.
    _keys: Option<Subscription>,
}

impl Global for Ui {}

impl Ui {
    pub fn get(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    pub fn get_mut(cx: &mut App) -> &mut Self {
        cx.global_mut::<Self>()
    }

    pub(crate) fn held_press(&self) -> Option<((f32, f32), bool)> {
        self.held
    }

    /// The width of `text` in the UI font.
    pub(crate) fn measure(&self, text: &str, size: f32, weight: FontWeight) -> f32 {
        self.measurer.as_ref().map_or(text.chars().count() as f32 * size * 0.6, |m| {
            let mut font = self.font.clone().unwrap_or_else(|| gpui::font(".SystemUIFont"));
            font.weight = weight;
            let run = TextRun { len: text.len(), font, color: gpui::black(), background_color: None, underline: None, strikethrough: None };
            m.shape_line(SharedString::from(text.to_string()), px(size), &[run], None).width.as_f32().ceil()
        })
    }

    /// Where an anchor was last painted.
    pub fn anchor(&self, key: &str) -> Option<Bounds<Pixels>> {
        self.anchors.get(key).or_else(|| self.last_anchors.get(key)).copied()
    }

    /// Something floats that wants the keyboard: the app's shortcuts stay out of it.
    pub fn captures_keys(&self) -> bool {
        self.menu.is_some() || self.popover.is_some() || !self.modals.is_empty()
    }
}

pub fn init(cx: &mut App) {
    input::bind_keys(cx);
    modal::bind_keys(cx);
    cx.set_global(Ui::default());
    let keys = cx.intercept_keystrokes(|e, window, cx| route_key(&e.keystroke, window, cx));
    Ui::get_mut(cx)._keys = Some(keys);
}

/// The key router's layers (`ui/keyRouter.ts`): an open menu takes every key but zoom; then an armed
/// control's Esc; a popover; a shown tooltip's Esc; a sticky toast's Esc. Below that, the view's own.
fn route_key(k: &Keystroke, window: &mut Window, cx: &mut App) {
    if is_zoom(k) {
        return;
    }
    if Ui::get(cx).menu.is_some() {
        menu::on_key(k, window, cx);
        cx.stop_propagation();
        return;
    }
    let plain_esc = k.key == "escape" && !k.modifiers.modified();
    if plain_esc && arm::is_armed_in_place(cx) {
        arm::disarm(window, cx);
        cx.stop_propagation();
        return;
    }
    if Ui::get(cx).popover.is_some() {
        if popover::on_key(k, window, cx) {
            cx.stop_propagation();
        }
        return;
    }
    if plain_esc && Ui::get(cx).tips.shown() {
        tooltip::hide(cx);
        cx.stop_propagation();
        return;
    }
    if !Ui::get(cx).modals.is_empty() {
        if let Some(combo) = crate::commands::combo_of(k)
            && modal::toggle_key(&combo, window, cx)
        {
            cx.stop_propagation();
        }
        return;
    }
    if plain_esc && toast::dismiss_sticky(cx) {
        cx.stop_propagation();
        return;
    }
    if let Some(keys) = Ui::get(cx).app_keys.clone()
        && keys(k, window, cx)
    {
        cx.stop_propagation();
    }
}

/// A box shadow: `0 y blur color`.
pub fn shadow(color: Hsla, y: f32, blur: f32) -> Vec<BoxShadow> {
    vec![BoxShadow { color, offset: point(px(0.), px(y)), blur_radius: px(blur), spread_radius: px(0.), inset: false }]
}

fn is_zoom(k: &Keystroke) -> bool {
    k.modifiers.control && !k.modifiers.alt && matches!(k.key.as_str(), "=" | "+" | "-" | "0")
}

/// The UI font's family, for what's sized before it's laid out.
pub fn set_font(family: SharedString, cx: &mut App) {
    Ui::get_mut(cx).font = Some(gpui::font(family));
}

/// The next frame's anchors begin here; what armed control or tooltip lost its anchor goes.
pub fn begin_frame(window: &mut Window, cx: &mut App) {
    let ui = Ui::get_mut(cx);
    if ui.measurer.is_none() {
        ui.measurer = Some(window.text_system().clone());
    }
    ui.last_anchors = std::mem::take(&mut ui.anchors);
    arm::check_anchor(window, cx);
    tooltip::check_anchor(cx);
}

/// Registers `key`'s bounds this frame.
pub(crate) fn set_anchor(key: SharedString, bounds: Bounds<Pixels>, cx: &mut App) {
    Ui::get_mut(cx).anchors.insert(key, bounds);
}

/// Everything that floats, in the web UI's z-order: the toast, dialogs (800), menus (900), tooltips
/// and cards (1000), the armed overlay (1010). Goes last in the root.
pub fn overlays(window: &mut Window, cx: &mut App) -> Vec<AnyElement> {
    let mut out = Vec::new();
    out.extend(toast::element(window, cx));
    out.extend(modal::elements(window, cx));
    out.extend(popover::element(window, cx));
    out.extend(menu::element(window, cx));
    out.extend(tooltip::element(window, cx));
    out.extend(arm::element(window, cx));
    out.push(canvas(|_, _, _| {}, |_, _, window, _| listen(window)).absolute().top_0().left_0().size_0().into_any_element());
    out
}

/// The window-wide listeners, painted last so their capture phase runs after every target's.
fn listen(window: &mut Window) {
    window.on_mouse_event(|e: &MouseMoveEvent, phase, window, cx| {
        if phase == DispatchPhase::Capture {
            tooltip::after_move(e.position, window, cx);
            menu::on_move(e, window, cx);
        }
    });
    window.on_mouse_event(|e: &MouseDownEvent, phase, window, cx| {
        if phase == DispatchPhase::Capture {
            Ui::get_mut(cx).last_press = Some(std::time::Instant::now());
            tooltip::on_press(cx);
            arm::on_press(e, window, cx);
            popover::on_press(e, window, cx);
            menu::on_press(e, window, cx);
        }
    });
    window.on_mouse_event(|e: &MouseUpEvent, phase, window, cx| {
        if phase == DispatchPhase::Capture {
            menu::on_release(e, window, cx);
        }
    });
    window.on_mouse_event(|e: &ScrollEvent, phase, window, cx| {
        if phase == DispatchPhase::Capture {
            tooltip::hide(cx);
            menu::on_wheel(e, window, cx);
        }
    });
    window.on_mouse_event(|_: &MouseExitEvent, phase, _, cx| {
        if phase == DispatchPhase::Capture {
            tooltip::hide(cx);
        }
    });
}

/// A wrapper that registers its child's bounds under a name, for `Ui::anchor`: what a menu drops
/// from, an armed control covers, a tooltip sits against. No layout of its own.
pub fn anchor(key: impl Into<SharedString>, child: impl IntoElement) -> tooltip::Anchor {
    tooltip::Anchor::new(child).key(key)
}

/// The width of `text` in the UI font at `size` px.
pub fn text_width(text: &str, size: f32, weight: FontWeight, cx: &App) -> f32 {
    Ui::get(cx).measure(text, size, weight)
}

/// The wrapped rows `text` makes in `width`, as a tooltip's box needs them.
pub fn text_size(text: &str, font_size: f32, line_h: f32, max_w: f32, window: &Window, cx: &App) -> (f32, f32) {
    let font = Ui::get(cx).font.clone().unwrap_or_else(|| window.text_style().font());
    let run = TextRun { len: text.len(), font, color: gpui::black(), background_color: None, underline: None, strikethrough: None };
    let lines = window.text_system().shape_text(SharedString::from(text.to_string()), px(font_size), &[run], Some(px(max_w)), None).unwrap_or_default();
    let (mut w, mut h) = (0f32, 0f32);
    for line in &lines {
        let size = line.size(px(line_h));
        w = w.max(size.width.as_f32());
        h += size.height.as_f32();
    }
    (w.ceil().min(max_w), h.max(line_h))
}
