//! What turns. GPUI's own animation asks for the next frame at the display's rate, and a frame lays out and paints the whole
//! window, since `GitVolt` is one view: a single spinner on a 144 Hz screen took a whole core's third, and more on a big
//! graph. Here a turning icon only keeps its place in the view. It's drawn by `SpinLayer`, a view of its own over `GitVolt`,
//! which `main.rs` caches as Zed caches a pane: a frame then reuses the app's painted scene as it is, draws what turns over
//! it, and comes at `FPS`, whatever the display's rate. The web's turn is the compositor's alone, as cheap as this gets.
//!
//! A floating thing's box is a `cover`: what turns under one isn't drawn. Without a layer (a test), the icon stands still.
//!
//! What turns for the forge's work (`forge_spinner`: a pipeline, a merge) stands still while the window is in the
//! background, where the forge isn't polled, and turns again when it comes back: the web's `data-window-blurred`.

use gpui::prelude::*;
use gpui::*;
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

/// A turn a second, as `.spin`, in steps the eye reads as turning at a 14 px icon's size.
const FRAME: Duration = Duration::from_millis(1000 / 30);
const TURN_MS: u128 = 1000;
/// How long the window stays in the background before the forge's spinners stop: longer than a WM's focus bounce, so a
/// click doesn't stop and restart them (the web's `BLURRED_AFTER_MS`).
const BLURRED_AFTER: Duration = Duration::from_secs(1);

struct Spin {
    order: u32,
    bounds: Bounds<Pixels>,
    mask: ContentMask<Pixels>,
    path: SharedString,
    color: Hsla,
    /// Stops while the window is in the background.
    forge: bool,
}

#[derive(Default)]
struct Painted {
    spins: Vec<Spin>,
    covers: Vec<(u32, Bounds<Pixels>)>,
    order: u32,
}

thread_local! {
    static PAINTED: RefCell<Painted> = RefCell::default();
    static LAYER: RefCell<Option<WeakEntity<SpinLayer>>> = const { RefCell::new(None) };
    static TICKING: Cell<bool> = const { Cell::new(false) };
    static LAST_TICK: Cell<Option<Instant>> = const { Cell::new(None) };
    static START: Instant = Instant::now();
    /// Since when the window has been in the background.
    static BLURRED_SINCE: Cell<Option<Instant>> = const { Cell::new(None) };
    /// Where the forge's spinners stand while it is.
    static FROZEN: Cell<Option<f32>> = const { Cell::new(None) };
}

/// The app's view is painting anew.
pub fn begin_frame() {
    PAINTED.with_borrow_mut(|p| {
        p.spins.clear();
        p.covers.clear();
        p.order = 0;
    });
}

/// A turning icon of `size_px`, in the place it would stand.
pub fn spinner(name: &'static str, size_px: f32, color: Hsla) -> impl IntoElement + use<> {
    turning(name, size_px, color, false)
}

/// `spinner` for work on the forge, not in GitVolt (a pipeline, a merge): it stands still while the window is in the
/// background.
pub fn forge_spinner(name: &'static str, size_px: f32, color: Hsla) -> impl IntoElement + use<> {
    turning(name, size_px, color, true)
}

fn turning(name: &'static str, size_px: f32, color: Hsla, forge: bool) -> impl IntoElement + use<> {
    let path = SharedString::from(format!("icons/{name}.svg"));
    canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let layered = LAYER.with_borrow(|l| l.as_ref().is_some_and(|w| w.upgrade().is_some()));
            if !layered {
                // A test's window, or before the layer exists: it stands.
                window.paint_svg(bounds, path.clone(), None, TransformationMatrix::unit(), color, cx).ok();
                return;
            }
            let mask = window.content_mask();
            PAINTED.with_borrow_mut(|p| {
                p.order += 1;
                let order = p.order;
                p.spins.push(Spin { order, bounds, mask, path: path.clone(), color, forge });
            });
        },
    )
    .flex_none()
    .size(px(size_px))
}

/// As the first child of something that floats: what turns under its box isn't drawn. Absolute, so it takes no room.
pub fn cover() -> impl IntoElement + use<> {
    canvas(
        |_, _, _| {},
        |bounds, _, _, _| {
            PAINTED.with_borrow_mut(|p| {
                p.order += 1;
                let order = p.order;
                p.covers.push((order, bounds));
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Whether what's in this order is under something painted later.
fn covered(p: &Painted, s: &Spin) -> bool {
    p.covers.iter().any(|(order, b)| *order > s.order && b.intersects(&s.bounds))
}

fn turn() -> f32 {
    let ms = START.with(|s| s.elapsed().as_millis());
    (ms % TURN_MS) as f32 / TURN_MS as f32 * std::f32::consts::TAU
}

/// Whether the window has been in the background long enough for the forge's spinners to stand still.
fn background(active: bool, now: Instant) -> bool {
    let since = if active { None } else { Some(BLURRED_SINCE.get().unwrap_or(now)) };
    BLURRED_SINCE.set(since);
    since.is_some_and(|t| now.duration_since(t) >= BLURRED_AFTER)
}

/// Draws what turns, over the app's cached scene.
pub struct SpinLayer;

impl SpinLayer {
    pub fn new(window: &mut Window, cx: &mut App) -> Entity<Self> {
        // Back in front: what stood still turns again.
        let layer = cx.new(|cx| {
            cx.observe_window_activation(window, |_, _, cx| cx.notify()).detach();
            SpinLayer
        });
        LAYER.with_borrow_mut(|l| *l = Some(layer.downgrade()));
        layer
    }
}

impl Render for SpinLayer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        canvas(
            |_, _, _| {},
            move |_, _, window, cx| {
                let live = turn();
                let paused = background(window.is_window_active(), Instant::now());
                let frozen = if paused { FROZEN.get().unwrap_or(live) } else { live };
                FROZEN.set(paused.then_some(frozen));
                let scale = window.scale_factor();
                let any = PAINTED.with_borrow(|p| {
                    for s in p.spins.iter().filter(|s| !covered(p, s)) {
                        let angle = radians(if s.forge && paused { frozen } else { live });
                        let centre = s.bounds.center();
                        let m = TransformationMatrix::unit().translate(centre.scale(scale)).rotate(angle).translate(centre.scale(-scale));
                        window.with_content_mask(Some(s.mask), |window| {
                            window.paint_svg(s.bounds, s.path.clone(), None, m, s.color, cx).ok();
                        });
                    }
                    // Under something that floats it still comes back later, so it keeps time without drawing; one that
                    // stands still needs no next frame.
                    p.spins.iter().any(|s| !(s.forge && paused))
                });
                if any && !TICKING.replace(true) {
                    let this = this.clone();
                    // A frame waits for the display after the tick: the wait counts from the last tick, so the rate is `FRAME`'s.
                    let wait = LAST_TICK.get().map_or(FRAME, |t| FRAME.saturating_sub(t.elapsed()));
                    window
                        .spawn(cx, async move |cx| {
                            cx.background_executor().timer(wait).await;
                            TICKING.set(false);
                            LAST_TICK.set(Some(Instant::now()));
                            this.update(cx, |_, cx| cx.notify()).ok();
                        })
                        .detach();
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn the_forge_spinners_stand_still_a_second_into_the_background_and_turn_again_in_front() {
        let t = Instant::now();
        assert!(!background(true, t));
        // A WM's focus bounce: not long enough.
        assert!(!background(false, t));
        assert!(!background(false, t + Duration::from_millis(500)));
        assert!(!background(true, t + Duration::from_millis(510)));
        // Counted from when it went: a second later, they stand.
        assert!(!background(false, t + Duration::from_millis(600)));
        assert!(background(false, t + Duration::from_millis(1600)));
        assert!(background(false, t + Duration::from_secs(5)));
        assert!(!background(true, t + Duration::from_secs(6)));
    }
}
