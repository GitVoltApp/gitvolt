//! What turns. GPUI's own animation asks for the next frame at the display's rate, and a frame lays out and paints the whole
//! window, since `GitVolt` is one view: a single spinner on a 144 Hz screen took a whole core's third, and more on a big
//! graph. Here a turning icon only keeps its place in the view. It's drawn by `SpinLayer`, a view of its own over `GitVolt`,
//! which `main.rs` caches as Zed caches a pane: a frame then reuses the app's painted scene as it is, draws what turns over
//! it, and comes at `FPS`, whatever the display's rate. The web's turn is the compositor's alone, as cheap as this gets.
//!
//! A floating thing's box is a `cover`: what turns under one isn't drawn. Without a layer (a test), the icon stands still.

use gpui::prelude::*;
use gpui::*;
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

/// A turn a second, as `.spin`, in steps the eye reads as turning at a 14 px icon's size.
const FRAME: Duration = Duration::from_millis(1000 / 30);
const TURN_MS: u128 = 1000;

struct Spin {
    order: u32,
    bounds: Bounds<Pixels>,
    mask: ContentMask<Pixels>,
    path: SharedString,
    color: Hsla,
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
                p.spins.push(Spin { order, bounds, mask, path: path.clone(), color });
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

/// Draws what turns, over the app's cached scene.
pub struct SpinLayer;

impl SpinLayer {
    pub fn new(cx: &mut App) -> Entity<Self> {
        let layer = cx.new(|_| SpinLayer);
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
                let angle = radians(turn());
                let scale = window.scale_factor();
                let mut any = false;
                PAINTED.with_borrow(|p| {
                    for s in p.spins.iter().filter(|s| !covered(p, s)) {
                        any = true;
                        let centre = s.bounds.center();
                        let m = TransformationMatrix::unit().translate(centre.scale(scale)).rotate(angle).translate(centre.scale(-scale));
                        window.with_content_mask(Some(s.mask), |window| {
                            window.paint_svg(s.bounds, s.path.clone(), None, m, s.color, cx).ok();
                        });
                    }
                    // Under something that floats it still comes back later, so it keeps time without drawing.
                    any = any || !p.spins.is_empty();
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
