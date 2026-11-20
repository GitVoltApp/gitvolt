//! Chromium's styled scrollbar (`tokens.css`): a lane of 10 px that takes its width from the content, a thumb inside it with 2 px
//! of transparent border, at least 24 px long. A press on the track pages towards the pointer and goes on while it's held; a press
//! on the thumb drags it. It reads a `ScrollHandle`, so any scrolled element or uniform list can have one.

use crate::theme;
use gpui::prelude::*;
use gpui::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// The lane's width, as `::-webkit-scrollbar`.
pub const LANE: f32 = 10.;
const BORDER: f32 = 2.;
const MIN_THUMB: f32 = 24.;
/// Chromium's first delay and its repeat, on a held press.
const FIRST_REPEAT: Duration = Duration::from_millis(250);
const REPEAT: Duration = Duration::from_millis(50);

#[derive(Clone)]
struct Drag {
    id: ElementId,
    /// Where in the thumb it was taken.
    grab: f32,
}

thread_local! {
    static DRAG: RefCell<Option<Drag>> = const { RefCell::new(None) };
    /// The press on a track, while it's held: it stops the repeating.
    static PAGING: Cell<bool> = const { Cell::new(false) };
}

/// A thumb's length and place on a track, in the same units as `view` and `max`: `None` when nothing scrolls.
pub fn thumb(track: f32, view: f32, max: f32, offset: f32) -> Option<(f32, f32)> {
    if max <= 0.5 || view <= 0. || track <= 0. {
        return None;
    }
    let len = (track * view / (view + max)).max(MIN_THUMB).min(track);
    Some((len, (offset / max).clamp(0., 1.) * (track - len)))
}

fn along(p: Point<Pixels>, axis: Axis) -> f32 {
    match axis {
        Axis::Vertical => p.y.as_f32(),
        Axis::Horizontal => p.x.as_f32(),
    }
}

fn set(handle: &ScrollHandle, axis: Axis, offset: f32) {
    let now = handle.offset();
    handle.set_offset(match axis {
        Axis::Vertical => point(now.x, px(-offset)),
        Axis::Horizontal => point(px(-offset), now.y),
    });
}

/// How far along the axis the handle is, with its room and the view's length.
fn read(handle: &ScrollHandle, axis: Axis) -> (f32, f32, f32) {
    let (offset, max, view) = (handle.offset(), handle.max_offset(), handle.bounds().size);
    match axis {
        Axis::Vertical => (-offset.y.as_f32(), max.y.as_f32(), view.height.as_f32()),
        Axis::Horizontal => (-offset.x.as_f32(), max.x.as_f32(), view.width.as_f32()),
    }
}

/// Whether there's anything to scroll, by the last frame's sizes: what a lane takes its room by.
pub fn shows(handle: &ScrollHandle, axis: Axis) -> bool {
    read(handle, axis).1 > 0.5
}

pub fn vertical(id: impl Into<ElementId>, handle: &ScrollHandle) -> AnyElement {
    bar(id.into(), handle.clone(), Axis::Vertical)
}

pub fn horizontal(id: impl Into<ElementId>, handle: &ScrollHandle) -> AnyElement {
    bar(id.into(), handle.clone(), Axis::Horizontal)
}

fn bar(id: ElementId, handle: ScrollHandle, axis: Axis) -> AnyElement {
    if !shows(&handle, axis) {
        return div().into_any_element();
    }
    let track = Rc::new(Cell::new(Bounds::<Pixels>::default()));
    let (track_paint, track_down) = (track.clone(), track.clone());
    let (down_handle, move_handle) = (handle.clone(), handle.clone());
    let (down_id, move_id) = (id.clone(), id.clone());
    let dragging = DRAG.with_borrow(|d| d.as_ref().is_some_and(|d| d.id == id));
    let lane = div().id(id).relative().flex_none();
    let lane = match axis {
        Axis::Vertical => lane.w(px(LANE)).h_full(),
        Axis::Horizontal => lane.h(px(LANE)).w_full(),
    };
    lane.occlude()
        .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            let b = track_down.get();
            let (start, length) = match axis {
                Axis::Vertical => (b.origin.y.as_f32(), b.size.height.as_f32()),
                Axis::Horizontal => (b.origin.x.as_f32(), b.size.width.as_f32()),
            };
            let (offset, max, view) = read(&down_handle, axis);
            let Some((len, at)) = thumb(length, view, max, offset) else { return };
            let pointer = along(e.position, axis) - start;
            if pointer >= at && pointer < at + len {
                DRAG.set(Some(Drag { id: down_id.clone(), grab: pointer - at }));
                window.refresh();
                return;
            }
            page(down_handle.clone(), axis, pointer, length);
            window.refresh();
            PAGING.set(true);
            let (repeat_handle, first) = (down_handle.clone(), Cell::new(true));
            window
                .spawn(cx, async move |cx| {
                    loop {
                        let wait = if first.replace(false) { FIRST_REPEAT } else { REPEAT };
                        cx.background_executor().timer(wait).await;
                        if !PAGING.get() || !page(repeat_handle.clone(), axis, pointer, length) {
                            break;
                        }
                        if cx.update(|window, _| window.refresh()).is_err() {
                            break;
                        }
                    }
                })
                .detach();
        })
        .child(
            canvas(
                move |bounds, _, _| track_paint.set(bounds),
                move |bounds, _, window, _| {
                    let handle = move_handle.clone();
                    let id = move_id.clone();
                    window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                        if phase != DispatchPhase::Capture {
                            return;
                        }
                        let Some(grab) = DRAG.with_borrow(|d| d.as_ref().filter(|d| d.id == id).map(|d| d.grab)) else { return };
                        if e.pressed_button != Some(MouseButton::Left) {
                            DRAG.set(None);
                            window.refresh();
                            return;
                        }
                        let (start, length) = match axis {
                            Axis::Vertical => (bounds.origin.y.as_f32(), bounds.size.height.as_f32()),
                            Axis::Horizontal => (bounds.origin.x.as_f32(), bounds.size.width.as_f32()),
                        };
                        let (offset, max, view) = read(&handle, axis);
                        let Some((len, _)) = thumb(length, view, max, offset) else { return };
                        let room = (length - len).max(1.);
                        set(&handle, axis, ((along(e.position, axis) - start - grab) / room * max).clamp(0., max));
                        cx.stop_propagation();
                        window.refresh();
                    });
                    window.on_mouse_event(move |e: &MouseUpEvent, phase, window, _| {
                        if phase == DispatchPhase::Capture && e.button == MouseButton::Left {
                            PAGING.set(false);
                            if DRAG.take().is_some() {
                                window.refresh();
                            }
                        }
                    });
                },
            )
            .absolute()
            .size_full(),
        )
        .child(painted_thumb(handle, axis, dragging))
        .into_any_element()
}

/// One page towards where the track was pressed; false once the thumb has reached it.
fn page(handle: ScrollHandle, axis: Axis, pointer: f32, length: f32) -> bool {
    let (offset, max, view) = read(&handle, axis);
    let Some((len, at)) = thumb(length, view, max, offset) else { return false };
    if pointer >= at && pointer < at + len {
        return false;
    }
    // Chromium's page is the view less 12.5%.
    let step = (view * 0.875).max(1.);
    let next = if pointer < at { offset - step } else { offset + step };
    set(&handle, axis, next.clamp(0., max));
    true
}

/// A thumb where the last frame's sizes put it, as Chromium's lane is drawn from the element's own bounds.
fn painted_thumb(handle: ScrollHandle, axis: Axis, dragging: bool) -> impl IntoElement {
    let (offset, max, view) = read(&handle, axis);
    let (len, at) = thumb(view, view, max, offset).unwrap_or((0., 0.));
    let t = div().id("thumb").absolute().rounded(px(5.)).bg(if dragging { theme::scroll_thumb_hover() } else { theme::scroll_thumb() }).hover(|d| d.bg(theme::scroll_thumb_hover()));
    match axis {
        Axis::Vertical => t.left(px(BORDER)).w(px(LANE - 2. * BORDER)).top(px(at + BORDER)).h(px((len - 2. * BORDER).max(0.))),
        Axis::Horizontal => t.top(px(BORDER)).h(px(LANE - 2. * BORDER)).left(px(at + BORDER)).w(px((len - 2. * BORDER).max(0.))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_thumb_has_a_minimum_and_ends_at_the_tracks_end() {
        assert_eq!(thumb(100., 100., 0., 0.), None);
        assert_eq!(thumb(100., 100., 900., 0.), Some((24., 0.)));
        assert_eq!(thumb(100., 100., 900., 900.), Some((24., 76.)));
        assert_eq!(thumb(100., 100., 100., 50.), Some((50., 25.)));
    }
}
