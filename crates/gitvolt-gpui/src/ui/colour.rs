//! The colour picker (`ui/src/ui/colour/`): a swatch that opens a popover with a Picker tab (an OKLCH chroma × lightness
//! square, hue, and opacity where it's asked for) and an OKLCH tab of sliders, a hex field, and Old / New swatches. Every
//! change previews; Enter or a press outside commits; Esc cancels. It edits in OKLCH, so lightness and chroma move without
//! shifting the hue, and gives back hex or `rgba()`.

use super::input::{InputEvent, TextInput};
use super::{Action, Ui};
use crate::theme::{self, Col};
use gpui::prelude::*;
use gpui::*;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklch {
    pub l: f64,
    pub c: f64,
    pub h: f64,
}

fn to_linear(n: u8) -> f64 {
    let v = n as f64 / 255.;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn to_byte(v: f64) -> u8 {
    let x = v.clamp(0., 1.);
    (255. * if x <= 0.0031308 { 12.92 * x } else { 1.055 * x.powf(1. / 2.4) - 0.055 }).round().clamp(0., 255.) as u8
}

fn oklab_to_linear(l_: f64, a: f64, b: f64) -> [f64; 3] {
    let l = (l_ + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m = (l_ - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s = (l_ - 0.0894841775 * a - 1.291485548 * b).powi(3);
    [4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s, -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s, -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s]
}

fn linear_of(l: f64, c: f64, h: f64) -> [f64; 3] {
    let rad = h.to_radians();
    oklab_to_linear(l / 100., c * rad.cos(), c * rad.sin())
}

fn in_gamut(rgb: &[f64; 3]) -> bool {
    rgb.iter().all(|v| *v >= -1e-6 && *v <= 1. + 1e-6)
}

/// Unrounded, so a round trip is exact. A grey reads chroma 0, hue 0.
pub fn to_oklch(c: Col) -> Oklch {
    let (r, g, b) = (to_linear(c.r), to_linear(c.g), to_linear(c.b));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let big_l = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
    let a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
    let bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
    let chroma = a.hypot(bb);
    if chroma < 1e-4 { Oklch { l: big_l * 100., c: 0., h: 0. } } else { Oklch { l: big_l * 100., c: chroma, h: (bb.atan2(a).to_degrees() + 360.) % 360. } }
}

/// The most chroma, up to `limit`, sRGB holds at lightness `l` and hue `h`.
pub fn max_chroma(l: f64, h: f64, limit: f64) -> f64 {
    if in_gamut(&linear_of(l, limit, h)) {
        return limit;
    }
    let (mut lo, mut hi) = (0., limit);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.;
        if in_gamut(&linear_of(l, mid, h)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// An OKLCH as sRGB bytes: outside the gamut it gives up chroma at the same lightness and hue.
pub fn from_oklch(p: Oklch, a: f32) -> Col {
    let l = p.l.clamp(0., 100.);
    let [r, g, b] = linear_of(l, max_chroma(l, p.h, p.c.max(0.)), p.h);
    Col { r: to_byte(r), g: to_byte(g), b: to_byte(b), a }
}

/// An OKLCH's bytes, or `None` outside the gamut: the square, pixel by pixel.
pub fn oklch_pixel(l: f64, c: f64, h: f64) -> Option<[u8; 3]> {
    let rgb = linear_of(l, c, h);
    in_gamut(&rgb).then(|| [to_byte(rgb[0]), to_byte(rgb[1]), to_byte(rgb[2])])
}

/// The chroma at the square's right edge and the Chroma slider's top.
const CMAX: f64 = 0.4;
const C_STEPS: f64 = 40.;
const WIDTH: f32 = 236.;
const SQUARE_W: f32 = WIDTH - 22.;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Square,
    Hue,
    Alpha,
    Lightness,
    Chroma,
}

pub type OnValue = Rc<dyn Fn(&str, &mut Window, &mut App)>;

pub struct PickerState {
    key: SharedString,
    label: SharedString,
    alpha: bool,
    original: Col,
    pub cur: Col,
    oklch_tab: bool,
    held: Option<Oklch>,
    hex: Entity<TextInput>,
    invalid: bool,
    /// What the keyboard's arrows step, and the press being dragged.
    slot: Slot,
    dragging: Option<(Slot, Bounds<Pixels>)>,
    /// The square at a hue, drawn when the hue changes.
    image: Option<(i64, Arc<RenderImage>)>,
    retired: Vec<Arc<RenderImage>>,
    focus: FocusHandle,
    return_to: Option<FocusHandle>,
    on_preview: OnValue,
    on_commit: OnValue,
    on_cancel: Action,
    _subscription: Subscription,
}

impl PickerState {
    fn out(&self, c: Col) -> String {
        if self.alpha { c.css() } else { c.at(1.).css() }
    }

    fn wanted(&self) -> Oklch {
        match self.held {
            Some(h) if same_rgb(from_oklch(h, self.cur.a), self.cur) => h,
            _ => to_oklch(self.cur),
        }
    }

    /// What the square's pointer and the fields show: never past the gamut's edge.
    fn lch(&self) -> Oklch {
        let w = self.wanted();
        Oklch { c: max_chroma(w.l, w.h, w.c), ..w }
    }
}

fn same_rgb(x: Col, y: Col) -> bool {
    x.r == y.r && x.g == y.g && x.b == y.b
}

/// A swatch's fill over the checkerboard (`.colour-swatch`), that opens the picker on a press.
#[allow(clippy::too_many_arguments)]
pub fn swatch(key: impl Into<SharedString>, value: &str, label: impl Into<SharedString>, alpha: bool, size_px: f32, on_preview: OnValue, on_commit: OnValue, on_cancel: Action, cx: &App) -> AnyElement {
    let key: SharedString = key.into();
    let label: SharedString = label.into();
    let open = Ui::get(cx).colour.as_ref().is_some_and(|p| p.key == key);
    let fill = Col::parse(value);
    let value = value.to_string();
    let k = key.clone();
    super::tooltip::tip(
        label.clone(),
        super::anchor(
            key.clone(),
            div()
                .id(ElementId::Name(key))
                .flex_none()
                .size(px(size_px))
                .overflow_hidden()
                .rounded(px(4.))
                .border_1()
                .border_color(if open { theme::blue() } else { theme::section_border() })
                .hover(|d| d.border_color(theme::text_secondary()))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, cx| open_picker(k.clone(), &value, alpha, label.clone(), on_preview.clone(), on_commit.clone(), on_cancel.clone(), window, cx))
                .child(checker(size_px, size_px, 4.))
                .child(div().absolute().top_0().left_0().size_full().when_some(fill, |d, c| d.bg(c.hsla()))),
        ),
    )
    .into_any_element()
}

/// `repeating-conic-gradient`'s board, 8 px a tile, in `--text-secondary` at 27%.
fn checker(w: f32, h: f32, cell: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let color = Hsla { a: theme::text_secondary().a * 0.27, ..theme::text_secondary() };
            let (cols, rows) = ((w / cell).ceil() as i32, (h / cell).ceil() as i32);
            for y in 0..rows {
                for x in 0..cols {
                    if (x + y) % 2 == 0 {
                        window.paint_quad(fill(Bounds::new(b.origin + point(px(x as f32 * cell), px(y as f32 * cell)), size(px(cell), px(cell))).intersect(&b), color));
                    }
                }
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

#[allow(clippy::too_many_arguments)]
pub fn open_picker(key: SharedString, value: &str, alpha: bool, label: SharedString, on_preview: OnValue, on_commit: OnValue, on_cancel: Action, window: &mut Window, cx: &mut App) {
    super::tooltip::hide(cx);
    let original = Col::parse(value).unwrap_or(Col::opaque(0, 0, 0));
    let hex = cx.new(|cx| {
        let mut i = TextInput::new(cx).claim_escape();
        i.set_text(&original.css(), cx);
        i
    });
    let handle = window.window_handle();
    let sub = cx.subscribe(&hex, move |input, e: &InputEvent, cx| {
        let e = *e;
        handle
            .update(cx, |_, window, cx| match e {
                InputEvent::Changed => {
                    let text = input.read(cx).text().to_string();
                    type_hex(&text, window, cx);
                }
                InputEvent::Escape => cancel(window, cx),
                InputEvent::Submit { .. } => commit(window, cx),
                _ => {}
            })
            .ok();
    });
    let focus = cx.focus_handle();
    let return_to = window.focused(cx);
    window.focus(&focus, cx);
    let state = PickerState { key, label, alpha, original, cur: original, oklch_tab: false, held: None, hex, invalid: false, slot: Slot::Square, dragging: None, image: None, retired: Vec::new(), focus, return_to, on_preview, on_commit, on_cancel, _subscription: sub };
    Ui::get_mut(cx).colour = Some(Box::new(state));
    cx.refresh_windows();
}

pub fn is_open(cx: &App) -> bool {
    Ui::get(cx).colour.is_some()
}

fn with_state(cx: &mut App, f: impl FnOnce(&mut PickerState)) {
    if let Some(p) = Ui::get_mut(cx).colour.as_deref_mut() {
        f(p);
    }
    cx.refresh_windows();
}

fn apply(c: Col, window: &mut Window, cx: &mut App) {
    let Some(p) = Ui::get(cx).colour.as_deref() else { return };
    let next = if p.alpha { c } else { c.at(1.) };
    let (input, on_preview) = (p.hex.clone(), p.on_preview.clone());
    input.update(cx, |i, cx| i.set_text(&next.css(), cx));
    with_state(cx, |p| {
        p.cur = next;
        p.invalid = false;
    });
    on_preview(&next.css(), window, cx);
}

fn set_lch(change: impl FnOnce(&mut Oklch), c_set: bool, window: &mut Window, cx: &mut App) {
    let Some(p) = Ui::get(cx).colour.as_deref() else { return };
    let mut n = p.wanted();
    let a = p.cur.a;
    change(&mut n);
    if c_set {
        n.c = max_chroma(n.l, n.h, n.c);
    }
    with_state(cx, |p| p.held = Some(n));
    apply(from_oklch(n, a), window, cx);
}

/// One step (an arrow key, a wheel notch; Shift's arrows 10) from the shown, rounded value.
fn nudge(slot: Slot, by: f64, window: &mut Window, cx: &mut App) {
    let Some(p) = Ui::get(cx).colour.as_deref() else { return };
    let lch = p.lch();
    let a = p.cur;
    match slot {
        Slot::Square | Slot::Lightness => set_lch(|n| n.l = (lch.l.round() + by).clamp(0., 100.), false, window, cx),
        Slot::Chroma => set_lch(|n| n.c = ((lch.c * 100.).round() + by).clamp(0., C_STEPS) / 100., true, window, cx),
        Slot::Hue => set_lch(|n| n.h = (lch.h.round() + by).clamp(0., 360.), false, window, cx),
        Slot::Alpha => apply(a.at((((a.a * 100.).round() as f64 + by).clamp(0., 100.) / 100.) as f32), window, cx),
    }
}

fn type_hex(text: &str, window: &mut Window, cx: &mut App) {
    let parsed = Col::parse(text);
    let Some(c) = parsed else {
        with_state(cx, |p| p.invalid = true);
        return;
    };
    let Some(p) = Ui::get(cx).colour.as_deref() else { return };
    let next = if p.alpha { c } else { c.at(1.) };
    let on_preview = p.on_preview.clone();
    with_state(cx, |p| {
        p.invalid = false;
        p.cur = next;
    });
    on_preview(&next.css(), window, cx);
}

fn take(window: &mut Window, cx: &mut App) -> Option<Box<PickerState>> {
    let mut p = Ui::get_mut(cx).colour.take()?;
    for image in p.retired.drain(..) {
        window.drop_image(image).ok();
    }
    if let Some((_, image)) = p.image.take() {
        window.drop_image(image).ok();
    }
    if let Some(back) = &p.return_to {
        window.focus(back, cx);
    }
    cx.refresh_windows();
    Some(p)
}

pub fn commit(window: &mut Window, cx: &mut App) {
    let Some(p) = take(window, cx) else { return };
    let value = p.out(p.cur);
    (p.on_commit)(&value, window, cx);
}

pub fn cancel(window: &mut Window, cx: &mut App) {
    let Some(p) = take(window, cx) else { return };
    (p.on_cancel)(window, cx);
}

fn pick_at(slot: Slot, bounds: Bounds<Pixels>, at: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let x = ((at.x - bounds.origin.x).as_f32() / bounds.size.width.as_f32()).clamp(0., 1.) as f64;
    let y = ((at.y - bounds.origin.y).as_f32() / bounds.size.height.as_f32()).clamp(0., 1.) as f64;
    match slot {
        Slot::Square => set_lch(|n| { n.l = 100. * (1. - y); n.c = CMAX * x; }, true, window, cx),
        Slot::Hue => set_lch(|n| n.h = x * 360., false, window, cx),
        Slot::Alpha => {
            let c = Ui::get(cx).colour.as_ref().map_or(Col::opaque(0, 0, 0), |p| p.cur);
            apply(c.at(((x * 100.).round() / 100.) as f32), window, cx);
        }
        Slot::Lightness => set_lch(|n| n.l = (x * 100.).round(), false, window, cx),
        Slot::Chroma => set_lch(|n| n.c = (x * C_STEPS).round() / 100., true, window, cx),
    }
}

fn square_image(h: f64, w: u32, ht: u32) -> Arc<RenderImage> {
    let mut buffer = image::RgbaImage::new(w, ht);
    for y in 0..ht {
        let l = 100. * (1. - (y as f64 + 0.5) / ht as f64);
        for x in 0..w {
            let Some(p) = oklch_pixel(l, CMAX * (x as f64 + 0.5) / w as f64, h) else { break };
            // GPUI's images are BGRA.
            buffer.put_pixel(x, y, image::Rgba([p[2], p[1], p[0], 255]));
        }
    }
    Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
}

fn thumb(left: f32, top: Option<f32>, fill: Option<Hsla>) -> Div {
    div().absolute().left(px(left - 7.)).top(px(top.unwrap_or(-1.) - if top.is_some() { 7. } else { 0. })).size(px(14.)).rounded_full().border_2().border_color(theme::text_selected()).shadow(vec![BoxShadow { color: theme::shadow_popup(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.), inset: false }]).when_some(fill, |d, c| d.bg(c))
}

/// A square or strip: a press picks the spot under it, a drag follows, the wheel steps.
fn slider(id: &'static str, slot: Slot, focused: bool, content: impl IntoElement) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex_none()
        .rounded(px(4.))
        .when(focused, |d| d.border_1().border_color(theme::blue()))
        .on_scroll(move |e: &ScrollEvent, window, cx| {
            cx.stop_propagation();
            let dy = e.delta.pixel_delta(px(16.)).y.as_f32();
            if dy != 0. {
                nudge(slot, if dy > 0. { 1. } else { -1. }, window, cx);
            }
        })
        .child(content)
}

fn press_slider(slot: Slot) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
    move |e: &MouseDownEvent, window, cx| {
        cx.stop_propagation();
        // The element's own bounds aren't known here: the frame's anchor says.
        let Some(bounds) = Ui::get(cx).anchor(slot_anchor(slot)) else { return };
        with_state(cx, |p| {
            p.slot = slot;
            p.dragging = Some((slot, bounds));
        });
        pick_at(slot, bounds, e.position, window, cx);
    }
}

fn slot_anchor(slot: Slot) -> &'static str {
    match slot {
        Slot::Square => "colour-square",
        Slot::Hue => "colour-hue",
        Slot::Alpha => "colour-alpha",
        Slot::Lightness => "colour-lightness",
        Slot::Chroma => "colour-chroma",
    }
}

fn strip(id: &'static str, slot: Slot, value: f32, max: f32, bg: Vec<(Hsla, Hsla)>, checkered: bool, focused: bool) -> AnyElement {
    let w = SQUARE_W;
    let paint = canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let n = bg.len().max(1) as f32;
            for (i, (from, to)) in bg.iter().enumerate() {
                let at = Bounds::new(b.origin + point(px(i as f32 * b.size.width.as_f32() / n), px(0.)), size(px(b.size.width.as_f32() / n + 0.5), b.size.height));
                window.paint_quad(fill(at, linear_gradient(90., linear_color_stop(*from, 0.), linear_color_stop(*to, 1.))));
            }
        },
    )
    .size_full();
    let content = div().relative().h(px(12.)).w(px(w)).when(checkered, |d| d.child(checker(w, 12., 4.))).child(div().absolute().top_0().left_0().size_full().child(paint)).child(thumb(value / max * w, None, None));
    super::anchor(slot_anchor(slot), slider(id, slot, focused, content).cursor_pointer().on_mouse_down(MouseButton::Left, press_slider(slot))).into_any_element()
}

fn number_range(id: &'static str, label: &'static str, slot: Slot, value: f64, max: f64) -> AnyElement {
    let w = 94.;
    let track = div().relative().h(px(12.)).w(px(w)).child(div().absolute().top(px(4.)).left_0().right_0().h(px(4.)).rounded(px(2.)).bg(theme::range_track())).child(div().absolute().top(px(4.)).left_0().h(px(4.)).rounded(px(2.)).w(px((value / max) as f32 * w)).bg(theme::blue())).child(thumb((value / max) as f32 * w, None, Some(theme::blue())));
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .child(div().flex_none().w(px(70.)).child(label))
        .child(super::anchor(slot_anchor(slot), slider(id, slot, false, track).cursor_pointer().on_mouse_down(MouseButton::Left, press_slider(slot))))
        .child(div().flex_none().w(px(48.)).px(px(6.)).py(px(3.)).rounded(px(4.)).border_1().border_color(theme::section_border()).bg(theme::app_bg()).text_color(theme::text_selected()).child(format!("{}", value.round() as i64)))
        .into_any_element()
}

pub(super) fn element(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let (cur, alpha, oklch_tab, original, invalid, hex, slot, focus, key, label) = {
        let p = Ui::get(cx).colour.as_deref()?;
        (p.cur, p.alpha, p.oklch_tab, p.original, p.invalid, p.hex.clone(), p.slot, p.focus.clone(), p.key.clone(), p.label.clone())
    };
    let lch = Ui::get(cx).colour.as_deref()?.lch();
    let anchor = Ui::get(cx).anchor(&key);
    // The square's picture, again when its hue changed.
    let hue_key = (lch.h * 10.).round() as i64;
    let square_h = if alpha { 170. - 20. - 8. - 8. } else { 150. - 20. - 8. };
    let scale = window.scale_factor();
    let need = Ui::get(cx).colour.as_deref()?.image.as_ref().is_none_or(|i| i.0 != hue_key);
    if need {
        let image = square_image(lch.h, (SQUARE_W * scale) as u32, (square_h * scale) as u32);
        if let Some(p) = Ui::get_mut(cx).colour.as_deref_mut() {
            if let Some((_, old)) = p.image.take() {
                p.retired.push(old);
            }
            p.image = Some((hue_key, image));
        }
    }
    let (image, retired) = {
        let p = Ui::get_mut(cx).colour.as_deref_mut()?;
        (p.image.as_ref()?.1.clone(), std::mem::take(&mut p.retired))
    };
    for old in retired {
        window.drop_image(old).ok();
    }
    let opaque = cur.at(1.);
    let chip = |c: Col| div().flex_none().size(px(18.)).rounded(px(3.)).border_1().border_color(theme::section_border()).bg(c.hsla());
    let compare = div()
        .flex()
        .gap(px(6.))
        .text_size(px(12.))
        .child(
            super::tooltip::tip(
                "Restore the old colour",
                div().id("colour-old").flex().flex_1().items_center().gap(px(6.)).px(px(6.)).py(px(3.)).border_1().border_color(theme::section_border()).rounded(px(4.)).cursor_pointer().hover(|d| d.border_color(theme::blue())).on_click(move |_, window, cx| {
                    with_state(cx, |p| p.held = None);
                    apply(original, window, cx);
                }).child(chip(if alpha { original } else { original.at(1.) })).child("Old"),
            ),
        )
        .child(div().flex().flex_1().items_center().gap(px(6.)).px(px(6.)).py(px(3.)).border_1().border_color(theme::section_border()).rounded(px(4.)).child(chip(cur)).child("New"));
    let tab = |id: &'static str, text: &'static str, on: bool, to: bool| {
        div().id(id).px(px(10.)).py(px(3.)).text_size(px(12.)).border_b_2().cursor_pointer().border_color(if on { theme::blue() } else { transparent_black() }).text_color(if on { theme::text_selected() } else { theme::text_secondary() }).on_click(move |_, _, cx| with_state(cx, |p| p.oklch_tab = to)).child(text)
    };
    let tabs = div().flex().gap(px(2.)).border_b_1().border_color(theme::section_border()).child(tab("colour-tab-picker", "Picker", !oklch_tab, false)).child(tab("colour-tab-oklch", "OKLCH", oklch_tab, true));
    let body = if oklch_tab {
        let mut b = div().flex().flex_col().gap(px(8.)).h(px(if alpha { 170. } else { 150. }));
        b = b.child(number_range("colour-r-l", "Lightness", Slot::Lightness, lch.l, 100.)).child(number_range("colour-r-c", "Chroma", Slot::Chroma, lch.c * 100., C_STEPS)).child(number_range("colour-r-h", "Hue", Slot::Hue, lch.h, 360.));
        if alpha {
            b = b.child(number_range("colour-r-a", "Opacity", Slot::Alpha, cur.a as f64 * 100., 100.));
        }
        b
    } else {
        let (w, h) = (SQUARE_W, square_h);
        let (tx, ty) = (((lch.c / CMAX).min(1.)) as f32 * w, (1. - lch.l / 100.) as f32 * h);
        let picture = canvas(
            |_, _, _| {},
            move |b, _, window, _| {
                window.paint_image(b, b, Corners::all(px(4.)), image.clone(), 0, false).ok();
            },
        )
        .size_full();
        let content = div().relative().w(px(w)).h(px(h)).rounded(px(4.)).bg(theme::app_bg()).child(picture).child(thumb(tx, Some(ty), Some(opaque.hsla())));
        let mut b = div().flex().flex_col().gap(px(8.)).h(px(if alpha { 170. } else { 150. })).child(super::anchor("colour-square", slider("colour-square", Slot::Square, slot == Slot::Square, content).cursor_crosshair().on_mouse_down(MouseButton::Left, press_slider(Slot::Square))));
        let hues: Vec<(Hsla, Hsla)> = (0..36).map(|i| (from_oklch(Oklch { l: 72., c: 0.12, h: i as f64 * 10. }, 1.).hsla(), from_oklch(Oklch { l: 72., c: 0.12, h: (i + 1) as f64 * 10. }, 1.).hsla())).collect();
        b = b.child(strip("colour-hue", Slot::Hue, lch.h as f32, 360., hues, false, slot == Slot::Hue));
        if alpha {
            b = b.child(strip("colour-alpha", Slot::Alpha, cur.a * 100., 100., vec![(opaque.at(0.).hsla(), opaque.hsla())], true, slot == Slot::Alpha));
        }
        b
    };
    let ink = if Col::opaque(0, 0, 0).contrast(opaque) >= Col::opaque(255, 255, 255).contrast(opaque) { Col::opaque(0, 0, 0) } else { Col::opaque(255, 255, 255) };
    hex.update(cx, |i, _| i.set_text_color(Some(ink.hsla())));
    let field = div().flex().items_center().h(px(26.)).px(px(6.)).rounded(px(4.)).border_1().border_color(if invalid { theme::red() } else { theme::section_border() }).bg(opaque.hsla()).text_size(px(12.)).child(hex);
    let panel = div()
        .id("colour-picker")
        .occlude()
        .flex()
        .flex_col()
        .gap(px(8.))
        .w(px(WIDTH))
        .p(px(10.))
        .bg(theme::panel_bg1())
        .border_1()
        .border_color(theme::section_border())
        .rounded(px(6.))
        .shadow(super::shadow(theme::shadow_popup(), 8., 24.))
        .text_color(theme::text_normal())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(compare)
        .child(tabs)
        .child(body)
        .child(field);
    let placed = super::place::placed(panel, move |sz, vp, _| {
        let a = anchor.map_or(super::place::Rect::at(0., 0.), super::place::Rect::of);
        let (x, y) = super::place::place_popover(Some(a), sz.width.as_f32(), sz.height.as_f32(), vp.width.as_f32(), vp.height.as_f32());
        point(px(x), px(y))
    });
    Some(
        div()
            .id("colour-layer")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .key_context("Colour")
            .track_focus(&focus)
            .debug_selector(|| label.to_string())
            .on_mouse_down(MouseButton::Left, |_, window, cx| commit(window, cx))
            .on_mouse_move(|e: &MouseMoveEvent, window, cx| {
                let Some((slot, bounds)) = Ui::get(cx).colour.as_deref().and_then(|p| p.dragging) else { return };
                if e.pressed_button == Some(MouseButton::Left) {
                    pick_at(slot, bounds, e.position, window, cx);
                }
            })
            .on_mouse_up(MouseButton::Left, |_, _, cx| with_state(cx, |p| p.dragging = None))
            .on_key_down(|e: &KeyDownEvent, window, cx| {
                let k = &e.keystroke;
                let m = &k.modifiers;
                cx.stop_propagation();
                let step = if m.shift { 10. } else { 1. };
                let slot = Ui::get(cx).colour.as_deref().map_or(Slot::Square, |p| p.slot);
                match k.key.as_str() {
                    "enter" => commit(window, cx),
                    "escape" => cancel(window, cx),
                    "tab" => with_state(cx, |p| {
                        let order: &[Slot] = if p.oklch_tab { &[Slot::Lightness, Slot::Chroma, Slot::Hue] } else if p.alpha { &[Slot::Square, Slot::Hue, Slot::Alpha] } else { &[Slot::Square, Slot::Hue] };
                        let at = order.iter().position(|s| *s == p.slot).unwrap_or(0);
                        p.slot = order[(at + if m.shift { order.len() - 1 } else { 1 }) % order.len()];
                    }),
                    "up" | "down" | "left" | "right" if !m.control && !m.alt => {
                        let across = matches!(k.key.as_str(), "left" | "right");
                        let dir = if matches!(k.key.as_str(), "right" | "up") { step } else { -step };
                        let target = if slot == Slot::Square && across { Slot::Chroma } else { slot };
                        nudge(target, dir, window, cx);
                    }
                    _ => {}
                }
            })
            .child(placed)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_colour_round_trips_through_oklch() {
        for hex in [0x4d88ffu32, 0xd9413d, 0x1c1e23, 0xffffff, 0x000000, 0x5cb85c] {
            let c = Col::of_hex(hex);
            assert_eq!(from_oklch(to_oklch(c), 1.), c, "{hex:06x}");
        }
    }

    #[test]
    fn a_grey_has_no_chroma_or_hue() {
        let g = to_oklch(Col::opaque(128, 128, 128));
        assert_eq!((g.c, g.h), (0., 0.));
    }

    #[test]
    fn outside_the_gamut_it_gives_up_chroma_at_the_same_hue() {
        assert!(oklch_pixel(50., 0.4, 200.).is_none());
        let limit = max_chroma(50., 200., 0.4);
        assert!(limit < 0.4 && oklch_pixel(50., limit, 200.).is_some());
        let c = from_oklch(Oklch { l: 50., c: 0.4, h: 200. }, 1.);
        assert!((to_oklch(c).h - 200.).abs() < 2.);
    }
}
