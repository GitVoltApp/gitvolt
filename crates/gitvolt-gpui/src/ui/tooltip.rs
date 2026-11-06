//! Tooltips and hover cards (`ui/HoverTooltip.tsx`, `TooltipHost`, `tooltip.css`): shown at once by
//! default, under the target (flipped above), beside it, or at the pointer; hidden on leave, a press,
//! a scroll, Esc, the pointer leaving the window. One at a time.
//!
//! `tip("Fetch", child)` wraps any element, with no layout of its own. A delayed one: `.delay(500)`.
//! Rich content (a hover card): `Anchor::new(child).card(key, build)`.

use super::Ui;
use super::place::{self, Rect};
use crate::theme;
use gpui::prelude::*;
use gpui::*;
use std::rc::Rc;
use std::time::Duration;

const FONT: f32 = 12.;
const LINE_H: f32 = 16.8;
const PAD_X: f32 = 8.;
const PAD_Y: f32 = 5.;
const MAX_W: f32 = 320.;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Placement {
    #[default]
    Below,
    Right,
    Left,
    Pointer,
    /// Left of an x (a panel's edge), centred on the target's height; without room, below.
    LeftOf(f32),
    /// With its top on the anchor's, beside the given x (a sidebar hover card).
    BesideAt(f32),
}

pub type Build = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

#[derive(Clone)]
pub enum Content {
    Text(SharedString),
    Card { key: SharedString, build: Build },
}

impl Content {
    fn same(&self, other: &Content) -> bool {
        match (self, other) {
            (Content::Text(a), Content::Text(b)) => a == b,
            (Content::Card { key: a, .. }, Content::Card { key: b, .. }) => a == b,
            _ => false,
        }
    }
}

#[derive(Clone)]
pub struct Spec {
    pub content: Content,
    pub placement: Placement,
    pub delay_ms: u64,
}

/// `--motion-base`.
const FADE_SECS: f32 = 0.15;

struct Shown {
    spec: Spec,
    bounds: Bounds<Pixels>,
    pointer: Point<Pixels>,
    /// When it first drew: it fades in from there.
    drawn: Option<std::time::Instant>,
}

#[derive(Default)]
pub struct Tips {
    shown: Option<Shown>,
    /// Waiting out its delay.
    pending: Option<Shown>,
    generation: u64,
    /// What the pointer is over, set by the targets' listeners during this move.
    candidate: Option<(Spec, Bounds<Pixels>)>,
    /// A press hid this one: it stays hidden until the pointer leaves it.
    pressed: Option<(Content, Bounds<Pixels>)>,
    /// Set by the menu: a row's tooltip, not a target's.
    imperative: bool,
}

impl Tips {
    pub fn shown(&self) -> bool {
        self.shown.is_some()
    }

    pub fn text(&self) -> Option<SharedString> {
        match &self.shown.as_ref()?.spec.content {
            Content::Text(t) => Some(t.clone()),
            Content::Card { key, .. } => Some(key.clone()),
        }
    }
}

/// Hides it now.
pub fn hide(cx: &mut App) {
    let tips = &mut Ui::get_mut(cx).tips;
    let had = tips.shown.take().is_some();
    tips.pending = None;
    tips.imperative = false;
    tips.generation += 1;
    if had {
        cx.refresh_windows();
    }
}

/// Shows `text` beside `bounds` from code: a menu row's tooltip, a canvas-drawn target.
pub fn show_at(text: impl Into<SharedString>, bounds: Bounds<Pixels>, placement: Placement, cx: &mut App) {
    let tips = &mut Ui::get_mut(cx).tips;
    tips.pending = None;
    tips.generation += 1;
    tips.imperative = true;
    tips.shown = Some(Shown { spec: Spec { content: Content::Text(text.into()), placement, delay_ms: 0 }, bounds, drawn: None, pointer: bounds.origin });
    cx.refresh_windows();
}

/// `show_at` for a card: `key` tells one from another, and `build` makes it each frame.
pub fn show_card_at(key: impl Into<SharedString>, build: Build, bounds: Bounds<Pixels>, placement: Placement, cx: &mut App) {
    let tips = &mut Ui::get_mut(cx).tips;
    tips.pending = None;
    tips.generation += 1;
    tips.imperative = true;
    tips.shown = Some(Shown { spec: Spec { content: Content::Card { key: key.into(), build }, placement, delay_ms: 0 }, bounds, drawn: None, pointer: bounds.origin });
    cx.refresh_windows();
}

/// `show_at` after `delay_ms`, unless something else happens first; a canvas-drawn target's.
pub fn show_at_after(spec: Spec, bounds: Bounds<Pixels>, pointer: Point<Pixels>, cx: &mut App) {
    let tips = &mut Ui::get_mut(cx).tips;
    tips.imperative = true;
    start(spec, bounds, pointer, cx);
}

fn start(spec: Spec, bounds: Bounds<Pixels>, pointer: Point<Pixels>, cx: &mut App) {
    let delay = spec.delay_ms;
    let tips = &mut Ui::get_mut(cx).tips;
    tips.generation += 1;
    if delay == 0 {
        tips.pending = None;
        tips.shown = Some(Shown { spec, bounds, pointer, drawn: None });
        cx.refresh_windows();
        return;
    }
    let had = tips.shown.take().is_some();
    tips.pending = Some(Shown { spec, bounds, pointer, drawn: None });
    let generation = tips.generation;
    if had {
        cx.refresh_windows();
    }
    let timer = cx.background_executor().timer(Duration::from_millis(delay));
    cx.spawn(async move |cx| {
        timer.await;
        cx.update(|cx| {
            let tips = &mut Ui::get_mut(cx).tips;
            if tips.generation == generation
                && let Some(p) = tips.pending.take()
            {
                tips.shown = Some(p);
                cx.refresh_windows();
            }
        });
    })
    .detach();
}

/// Runs after every target's listener of this mouse move: the topmost target under the pointer wins.
pub(super) fn after_move(pointer: Point<Pixels>, _: &mut Window, cx: &mut App) {
    let tips = &mut Ui::get_mut(cx).tips;
    let candidate = tips.candidate.take();
    if tips.imperative {
        return;
    }
    match candidate {
        None => {
            tips.pressed = None;
            if tips.shown.is_some() || tips.pending.is_some() {
                hide(cx);
            }
        }
        Some((spec, bounds)) => {
            if tips.pressed.as_ref().is_some_and(|(c, b)| c.same(&spec.content) && *b == bounds) {
                return;
            }
            tips.pressed = None;
            let same = |s: &Shown| s.spec.content.same(&spec.content) && s.bounds == bounds;
            if tips.shown.as_ref().is_some_and(same) {
                // A tooltip at the pointer follows it.
                if let Some(s) = tips.shown.as_mut().filter(|s| s.spec.placement == Placement::Pointer) {
                    s.pointer = pointer;
                    cx.refresh_windows();
                }
                return;
            }
            if tips.pending.as_ref().is_some_and(same) {
                return;
            }
            start(spec, bounds, pointer, cx);
        }
    }
}

/// A press hides it, as a native tooltip goes: the user is acting on the target now.
pub(super) fn on_press(cx: &mut App) {
    let tips = &mut Ui::get_mut(cx).tips;
    if tips.imperative {
        return;
    }
    let target = tips.shown.as_ref().or(tips.pending.as_ref()).map(|s| (s.spec.content.clone(), s.bounds));
    if target.is_some() {
        tips.pressed = target;
        hide(cx);
    }
}

/// Its target went away under a resting pointer: it goes too.
pub(super) fn check_anchor(cx: &mut App) {
    let ui = Ui::get(cx);
    if ui.tips.imperative {
        return;
    }
    let Some(s) = ui.tips.shown.as_ref() else { return };
    let key = anchor_key(&s.spec.content);
    if !ui.last_anchors.contains_key(&key) {
        hide(cx);
    }
}

fn anchor_key(c: &Content) -> SharedString {
    match c {
        Content::Text(t) => SharedString::from(format!("tip:{t}")),
        Content::Card { key, .. } => SharedString::from(format!("tip:{key}")),
    }
}

pub(super) fn element(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let opacity = {
        let shown = Ui::get_mut(cx).tips.shown.as_mut()?;
        let t = shown.drawn.get_or_insert_with(std::time::Instant::now).elapsed().as_secs_f32() / FADE_SECS;
        if t < 1. && !cfg!(test) {
            window.request_animation_frame();
        }
        // `--motion-base`: ease-in.
        (t * t).min(1.)
    };
    let ui = Ui::get(cx);
    let s = ui.tips.shown.as_ref()?;
    let spec = s.spec.clone();
    let (bounds, pointer) = (s.bounds, s.pointer);
    let viewport = window.viewport_size();
    let (vw, vh) = (viewport.width.as_f32(), viewport.height.as_f32());
    let anchor = Rect::of(bounds);
    match spec.content {
        Content::Text(text) => {
            // A line's trailing space hangs, as a browser's: the text gets a space's room past the box it's measured for.
            let room = MAX_W.min(vw - 16.) - 2. * PAD_X - 2.;
            let (tw, th) = super::text_size(&text, FONT, LINE_H, room + 3., window, cx);
            let (wrap, tw) = (tw + 3., tw.min(room));
            let (w, h) = (tw + 2. * PAD_X + 2., th + 2. * PAD_Y + 2.);
            let (left, top) = position(spec.placement, anchor, pointer, w, h, vw, vh);
            let el = div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(w))
                .h(px(h))
                .px(px(PAD_X))
                .py(px(PAD_Y))
                .rounded(px(4.))
                .border_1()
                .border_color(theme::section_border())
                .bg(theme::panel_bg1())
                .text_color(theme::text_selected())
                .text_size(px(FONT))
                .line_height(px(LINE_H))
                .shadow(super::shadow(theme::shadow_tooltip(), 2., 8.))
                .overflow_hidden()
                .opacity(opacity)
                .child(div().w(px(wrap)).child(text));
            Some(el.into_any_element())
        }
        Content::Card { build, .. } => {
            let card = build(window, cx);
            let placement = spec.placement;
            let placed = place::placed(card, move |size, viewport, _| {
                let (w, h, vw, vh) = (size.width.as_f32(), size.height.as_f32(), viewport.width.as_f32(), viewport.height.as_f32());
                let (left, top) = position(placement, anchor, pointer, w, h, vw, vh);
                point(px(left), px(top))
            });
            Some(div().absolute().top_0().left_0().size_full().opacity(opacity).child(placed).into_any_element())
        }
    }
}

fn position(placement: Placement, a: Rect, pointer: Point<Pixels>, w: f32, h: f32, vw: f32, vh: f32) -> (f32, f32) {
    match placement {
        Placement::Below => {
            let p = place::place_card(a, w, h, place::GAP, vw, vh);
            (p.left, p.top)
        }
        Placement::Right => place::place_beside(a, w, h, true, vw, vh),
        Placement::Left => place::place_beside(a, w, h, false, vw, vh),
        Placement::Pointer => place::place_at_pointer(pointer.x.as_f32(), pointer.y.as_f32(), w, h, vw, vh),
        Placement::LeftOf(edge) => {
            let beside = edge - 6. - w;
            if beside >= place::EDGE {
                (beside, place::EDGE.max(((a.top + a.bottom - h) / 2.).min(vh - place::EDGE - h)))
            } else {
                let p = place::place_card(a, w, h, place::GAP, vw, vh);
                (p.left, p.top)
            }
        }
        Placement::BesideAt(x) => {
            let p = place::place_card_top(Rect { left: x, ..a }, w, h, vw, vh);
            (p.left, p.top)
        }
    }
}

/// Wraps an element: records where it is, under a name (`Ui::anchor`) and, with a tooltip, offers it
/// while the pointer's on it. Takes the child's layout as its own.
pub struct Anchor {
    child: AnyElement,
    key: Option<SharedString>,
    spec: Option<Spec>,
}

/// `text` at once under `child`, flipped above when it doesn't fit.
pub fn tip(text: impl Into<SharedString>, child: impl IntoElement) -> Anchor {
    Anchor::new(child).tip(text)
}

impl Anchor {
    pub fn new(child: impl IntoElement) -> Self {
        Self { child: child.into_any_element(), key: None, spec: None }
    }

    /// Named for `Ui::anchor`.
    pub fn key(mut self, key: impl Into<SharedString>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn tip(mut self, text: impl Into<SharedString>) -> Self {
        let text = text.into();
        self.spec = (!text.is_empty()).then_some(Spec { content: Content::Text(text), placement: Placement::Below, delay_ms: 0 });
        self
    }

    pub fn card(mut self, key: impl Into<SharedString>, build: impl Fn(&mut Window, &mut App) -> AnyElement + 'static) -> Self {
        self.spec = Some(Spec { content: Content::Card { key: key.into(), build: Rc::new(build) }, placement: Placement::Below, delay_ms: 0 });
        self
    }

    pub fn delay(mut self, ms: u64) -> Self {
        if let Some(s) = self.spec.as_mut() {
            s.delay_ms = ms;
        }
        self
    }

    pub fn placement(mut self, placement: Placement) -> Self {
        if let Some(s) = self.spec.as_mut() {
            s.placement = placement;
        }
        self
    }
}

impl IntoElement for Anchor {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Anchor {
    type RequestLayoutState = ();
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) -> Option<Hitbox> {
        if let Some(key) = self.key.clone() {
            super::set_anchor(key, bounds, cx);
        }
        // Under what the child paints: a target's own hitbox doesn't occlude this one.
        let hitbox = self.spec.as_ref().map(|_| window.insert_hitbox(bounds, HitboxBehavior::Normal));
        self.child.prepaint(window, cx);
        if let Some(s) = self.spec.as_ref() {
            super::set_anchor(anchor_key(&s.content), bounds, cx);
        }
        hitbox
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), hitbox: &mut Option<Hitbox>, window: &mut Window, cx: &mut App) {
        self.child.paint(window, cx);
        let (Some(hitbox), Some(spec)) = (hitbox.take(), self.spec.take()) else { return };
        window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
            if phase == DispatchPhase::Capture && hitbox.is_hovered(window) {
                Ui::get_mut(cx).tips.candidate = Some((spec.clone(), bounds));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    struct Host;
    impl Render for Host {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            super::super::begin_frame(window, cx);
            div()
                .size_full()
                .relative()
                .child(tip("First", div().id("a").w(px(50.)).h(px(20.))))
                .child(tip("Second", div().id("b").w(px(50.)).h(px(20.))).delay(500))
                .children(super::super::overlays(window, cx))
        }
    }

    fn open(cx: &mut TestAppContext) -> VisualTestContext {
        cx.update(super::super::init);
        let window = cx.update(|cx| cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| Host))).unwrap();
        let v = VisualTestContext::from_window(window.into(), cx);
        v.run_until_parked();
        v
    }

    fn text(cx: &mut VisualTestContext) -> Option<SharedString> {
        cx.update(|_, cx| Ui::get(cx).tips.text())
    }

    #[gpui::test]
    fn it_shows_at_once_goes_on_leave_and_a_press_keeps_it_away(cx: &mut TestAppContext) {
        let mut cx = open(cx);
        cx.simulate_mouse_move(point(px(10.), px(10.)), None, Modifiers::none());
        assert_eq!(text(&mut cx).as_deref(), Some("First"));
        cx.simulate_mouse_move(point(px(300.), px(300.)), None, Modifiers::none());
        assert_eq!(text(&mut cx), None);
        cx.simulate_mouse_move(point(px(10.), px(10.)), None, Modifiers::none());
        cx.simulate_mouse_down(point(px(10.), px(10.)), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(point(px(10.), px(10.)), MouseButton::Left, Modifiers::none());
        assert_eq!(text(&mut cx), None);
        cx.simulate_mouse_move(point(px(12.), px(10.)), None, Modifiers::none());
        assert_eq!(text(&mut cx), None, "moving within it doesn't bring it back");
    }

    #[gpui::test]
    fn a_delayed_one_waits_and_esc_dismisses(cx: &mut TestAppContext) {
        let mut cx = open(cx);
        cx.simulate_mouse_move(point(px(10.), px(30.)), None, Modifiers::none());
        assert_eq!(text(&mut cx), None);
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        assert_eq!(text(&mut cx).as_deref(), Some("Second"));
        cx.simulate_keystrokes("escape");
        assert_eq!(text(&mut cx), None);
    }
}
