//! The image diff (`ui/src/image`): Side-by-side, Swipe, Onion skin and Difference, zoom and pan shared
//! by every mode, Match sizes, the four backgrounds, and the Hex or Source toggle. Each side decodes off
//! the UI thread into a `RenderImage`.

use super::view::ViewKind;
use crate::app::{GitVolt, icon};
use crate::details::{segment, segmented};
use crate::format::format_bytes;
use crate::theme;
use crate::ui::input::TextInput;
use base64::Engine;
use gpui::prelude::*;
use gpui::*;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

pub const IMAGE_TOOLBAR_H: f32 = 33.;
/// Stepped zoom: the slider and Ctrl+wheel move one step.
pub const ZOOM_STEPS: [f32; 20] = [0.1, 0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1., 1.1, 1.25, 1.5, 1.75, 2., 2.5, 3., 4., 5., 6., 8., 10.];
pub const DEFAULT_STEP: usize = 7;
/// Wheel travel per zoom step: a mouse notch is one step, a trackpad pinch accumulates.
const WHEEL_STEP_PX: f32 = 50.;
const SWIPE_EDGE: f32 = 4.;
const MODE_START: f32 = 50.;
const AMPLIFY_DEFAULT: u32 = 4;
const AMPLIFY_MAX: u32 = 16;
const ASPECT_TOLERANCE: f32 = 0.01;
/// A side larger than this holds a smaller copy: the atlas takes textures only so big.
const MAX_TEXTURE: u32 = 4096;
const SLIDER_W: f32 = 129.;
const SLIDER_H: f32 = 16.;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub scale: f32,
    pub x: f32,
    pub y: f32,
}

pub fn fit_scale(w: f32, h: f32, box_w: f32, box_h: f32) -> f32 {
    if w <= 0. || h <= 0. || box_w <= 0. || box_h <= 0. {
        return 1.;
    }
    (box_w / w).min(box_h / h).min(10.)
}

pub fn centered(scale: f32, w: f32, h: f32, box_w: f32, box_h: f32) -> View {
    View { scale, x: (box_w - w * scale) / 2., y: (box_h - h * scale) / 2. }
}

/// One axis of `clamp_view`: centred when the image fits, else kept covering the box.
fn clamp_axis(pos: f32, size: f32, bx: f32) -> f32 {
    if size <= bx { (bx - size) / 2. } else { pos.max(bx - size).min(0.) }
}

/// Panned only where the image is larger than the box: where it fits it's centred, and where it's
/// larger its edges never come inside the box.
pub fn clamp_view(v: View, w: f32, h: f32, box_w: f32, box_h: f32) -> View {
    if w <= 0. || h <= 0. {
        return v;
    }
    View { scale: v.scale, x: clamp_axis(v.x, w * v.scale, box_w), y: clamp_axis(v.y, h * v.scale, box_h) }
}

/// Where an image opens: its top left where it overflows, centred where it fits.
pub fn start_view(scale: f32, w: f32, h: f32, box_w: f32, box_h: f32) -> View {
    clamp_view(View { scale, x: 0., y: 0. }, w, h, box_w, box_h)
}

/// A new scale, keeping the image point under `(px, py)` where it is.
pub fn zoom_around(v: View, scale: f32, px: f32, py: f32) -> View {
    let (ix, iy) = ((px - v.x) / v.scale, (py - v.y) / v.scale);
    View { scale, x: px - ix * scale, y: py - iy * scale }
}

/// The step above or below `scale`.
pub fn next_step_index(scale: f32, up: bool) -> usize {
    if up {
        return ZOOM_STEPS.iter().position(|s| *s > scale + 1e-6).unwrap_or(ZOOM_STEPS.len() - 1);
    }
    ZOOM_STEPS.iter().rposition(|s| *s < scale - 1e-6).unwrap_or(0)
}

/// The step closest to any scale: for the slider's thumb only; the exact scale is kept.
pub fn nearest_step_index(scale: f32) -> usize {
    let mut best = 0;
    for (i, s) in ZOOM_STEPS.iter().enumerate() {
        if (s - scale).abs() < (ZOOM_STEPS[best] - scale).abs() {
            best = i;
        }
    }
    best
}

/// The swipe handle's place: anywhere over the image, but on screen.
pub fn clamp_swipe(px: f32, v: View, w: f32, box_w: f32) -> f32 {
    let left = SWIPE_EDGE.max(v.x);
    let right = left.max((box_w - SWIPE_EDGE).min(v.x + w * v.scale));
    px.max(left).min(right)
}

pub fn sizes_differ(a: Option<(f32, f32)>, b: Option<(f32, f32)>) -> bool {
    matches!((a, b), (Some(a), Some(b)) if a != b)
}

pub fn aspects_match(a: (f32, f32), b: (f32, f32)) -> bool {
    if a.0 <= 0. || a.1 <= 0. || b.0 <= 0. || b.1 <= 0. {
        return false;
    }
    ((a.0 / a.1) / (b.0 / b.1) - 1.).abs() <= ASPECT_TOLERANCE
}

/// Match sizes' default: on for a pure resize, off for a crop or another image.
pub fn default_match(a: Option<(f32, f32)>, b: Option<(f32, f32)>) -> bool {
    sizes_differ(a, b) && matches!((a, b), (Some(a), Some(b)) if aspects_match(a, b))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The old image's place in the new one's box: the whole box for a pure resize, otherwise fitted
/// inside it and centred.
pub fn matched_rect(old: (f32, f32), new: (f32, f32)) -> Rect {
    if aspects_match(old, new) {
        return Rect { x: 0., y: 0., w: new.0, h: new.1 };
    }
    let s = (new.0 / old.0).min(new.1 / old.1);
    let (w, h) = (old.0 * s, old.1 * s);
    Rect { x: (new.0 - w) / 2., y: (new.1 - h) / 2., w, h }
}

/// A decoded side.
pub struct Decoded {
    pub image: Arc<RenderImage>,
    /// Its own size, the frame's and the zoom's, whatever the size of the copy held.
    pub w: f32,
    pub h: f32,
}

/// A side of the file.
pub enum Side {
    None,
    Failed { size: u64 },
    Image { decoded: Arc<Decoded>, size: u64 },
}

impl Side {
    fn dim(&self) -> Option<(f32, f32)> {
        match self {
            Side::Image { decoded, .. } => Some((decoded.w, decoded.h)),
            _ => None,
        }
    }

    fn decoded(&self) -> Option<&Arc<Decoded>> {
        match self {
            Side::Image { decoded, .. } => Some(decoded),
            _ => None,
        }
    }

    fn size(&self) -> Option<u64> {
        match self {
            Side::None => None,
            Side::Failed { size } | Side::Image { size, .. } => Some(*size),
        }
    }

    fn exists(&self) -> bool {
        !matches!(self, Side::None)
    }

    fn dims(&self) -> String {
        self.dim().map_or("?".into(), |(w, h)| format!("{}×{}", w.round() as u32, h.round() as u32))
    }

    fn failed(&self) -> bool {
        matches!(self, Side::Failed { .. })
    }
}

pub fn render_image(mut rgba: image::RgbaImage) -> RenderImage {
    // RenderImage is BGRA.
    for px in rgba.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    RenderImage::new(vec![image::Frame::new(rgba)])
}

/// A raster side's base64 bytes, decoded.
pub fn decode_raster(base64_text: &str) -> Option<Decoded> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(base64_text).ok()?;
    let decoded = image::load_from_memory(&bytes).ok()?;
    let (w, h) = (decoded.width(), decoded.height());
    let rgba = if w.max(h) > MAX_TEXTURE { decoded.thumbnail(MAX_TEXTURE, MAX_TEXTURE).into_rgba8() } else { decoded.into_rgba8() };
    Some(Decoded { image: Arc::new(render_image(rgba)), w: w as f32, h: h as f32 })
}

/// One side of a read: its bytes, base64 for a raster image and text for an SVG.
pub fn decode_side(v: &Value, svg: &SvgRenderer) -> Side {
    if !v.is_object() {
        return Side::None;
    }
    let size = v["size"].as_u64().unwrap_or(0);
    let decoded = if let Some(b) = v["base64"].as_str() {
        decode_raster(b)
    } else if let Some(text) = v["text"].as_str() {
        svg.render_single_frame(text.as_bytes(), 1.).ok().map(|image| {
            let s = image.size(0);
            Decoded { w: s.width.0 as f32, h: s.height.0 as f32, image }
        })
    } else {
        None
    };
    match decoded {
        Some(d) => Side::Image { decoded: Arc::new(d), size },
        None => Side::Failed { size },
    }
}

fn rgba_of(d: &Decoded) -> Option<image::RgbaImage> {
    let s = d.image.size(0);
    image::RgbaImage::from_raw(s.width.0 as u32, s.height.0 as u32, d.image.as_bytes(0)?.to_vec())
}

/// |a − b| on black, brightened by `amplify`, opaque: the canvas `difference` blend's result. The old
/// side at `rect` (Match sizes) or its own size, smoothed so a scale doesn't alias into differences
/// that aren't there. Channels stay in the images' own order.
pub fn difference(old: &Decoded, new: &Decoded, w: f32, h: f32, rect: Option<Rect>, amplify: u32) -> Option<RenderImage> {
    let (cw, ch) = ((w.max(1.) as u32).min(MAX_TEXTURE), (h.max(1.) as u32).min(MAX_TEXTURE));
    let scale = (cw as f32 / w.max(1.)).min(ch as f32 / h.max(1.));
    let place = |img: image::RgbaImage, tw: f32, th: f32| -> image::RgbaImage {
        let (tw, th) = (((tw * scale).round() as u32).max(1), ((th * scale).round() as u32).max(1));
        if img.dimensions() == (tw, th) { img } else { image::imageops::resize(&img, tw, th, image::imageops::FilterType::CatmullRom) }
    };
    let rect = rect.unwrap_or(Rect { x: 0., y: 0., w: old.w, h: old.h });
    let a = place(rgba_of(old)?, rect.w, rect.h);
    let b = place(rgba_of(new)?, new.w, new.h);
    let (ax, ay) = ((rect.x * scale).round() as u32, (rect.y * scale).round() as u32);
    let mut out = vec![0u8; (cw * ch * 4) as usize];
    for y in 0..ch {
        for x in 0..cw {
            let mut c = [0f32; 3];
            if let Some(p) = x.checked_sub(ax).zip(y.checked_sub(ay)).and_then(|(ix, iy)| a.get_pixel_checked(ix, iy)) {
                let alpha = f32::from(p[3]) / 255.;
                for (i, v) in c.iter_mut().enumerate() {
                    *v = f32::from(p[i]) * alpha;
                }
            }
            if let Some(p) = b.get_pixel_checked(x, y) {
                let alpha = f32::from(p[3]) / 255.;
                for (i, v) in c.iter_mut().enumerate() {
                    *v = (1. - alpha) * *v + alpha * (*v - f32::from(p[i])).abs();
                }
            }
            let at = ((y * cw + x) * 4) as usize;
            for i in 0..3 {
                out[at + i] = (c[i] * amplify as f32).min(255.) as u8;
            }
            out[at + 3] = 255;
        }
    }
    let buffer = image::RgbaImage::from_raw(cw, ch, out)?;
    Some(RenderImage::new(vec![image::Frame::new(buffer)]))
}

/// The side's pixels `factor` times as big, each one square: the shader only samples linearly, and a
/// zoomed image's pixels stay crisp as in the web UI (`image-rendering: pixelated`).
pub fn crisp(image: &RenderImage, factor: u32) -> Option<RenderImage> {
    let s = image.size(0);
    let (w, h) = (s.width.0 as u32, s.height.0 as u32);
    let from = image.as_bytes(0)?;
    let mut out = vec![0u8; (w * factor * h * factor * 4) as usize];
    let stride = (w * factor * 4) as usize;
    for y in 0..h {
        let row = &from[(y * w * 4) as usize..((y + 1) * w * 4) as usize];
        let first = (y * factor) as usize * stride;
        for (x, p) in row.as_chunks::<4>().0.iter().enumerate() {
            for k in 0..factor as usize {
                out[first + (x * factor as usize + k) * 4..][..4].copy_from_slice(p);
            }
        }
        for k in 1..factor as usize {
            out.copy_within(first..first + stride, first + k * stride);
        }
    }
    let buffer = image::RgbaImage::from_raw(w * factor, h * factor, out)?;
    Some(RenderImage::new(vec![image::Frame::new(buffer)]))
}

/// The copy `crisp` makes for a scale, if there's to be one: a big image's would be too large.
pub fn crisp_factor(scale: f32, w: u32, h: u32) -> Option<u32> {
    let f = scale.ceil() as u32;
    (scale > 1. && w.max(h) * f <= MAX_TEXTURE).then_some(f)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Side,
    Swipe,
    Onion,
    Difference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Background {
    Checker,
    Black,
    White,
    Grey,
}

/// What's being dragged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImageDrag {
    None,
    Pan(f32, f32),
    Swipe,
    Zoom,
    Opacity,
    Amplify,
}

/// A shown image diff.
pub struct ImageView {
    pub old: Side,
    pub new: Side,
    /// An SVG: its toggle's Source shows text instead of hex.
    pub svg: bool,
    pub mode: Mode,
    pub background: Background,
    pub view: View,
    /// The target rung, and the zoom % shown, set by every zoom action.
    pub step: usize,
    pub zoom_pct: u32,
    pub fit: bool,
    pub amplify: u32,
    pub swipe: f32,
    pub opacity: f32,
    pub match_pick: Option<bool>,
    /// The zoom % being typed.
    /// A number's being typed for the zoom (`GitVolt::zoom_input`).
    pub zoom_edit: bool,
    /// The Hex or Source toggle.
    pub source_on: bool,
    pub drag: ImageDrag,
    /// The stage, as last laid out.
    pub stage: (f32, f32),
    last_box: Option<(f32, f32)>,
    placed: bool,
    wheel: f32,
    /// The Difference canvas; a newer computation replaces an older one.
    pub diff_image: Option<Arc<RenderImage>>,
    pub diff_generation: u64,
    /// What the canvas was computed for: amplify, matched.
    pub diff_key: Option<(u32, bool)>,
    /// The stage's left edge and the sliders' tracks, as last painted, for the mouse.
    pub stage_left: Rc<Cell<Option<f32>>>,
    pub zoom_track: Rc<Cell<Option<(f32, f32)>>>,
    pub opacity_track: Rc<Cell<Option<(f32, f32)>>>,
    pub amplify_track: Rc<Cell<Option<(f32, f32)>>>,
    /// Crisp copies by the image they're of and their factor, and what's being made.
    pub crisp: HashMap<(usize, u32), Arc<RenderImage>>,
    pub crisp_pending: HashMap<(usize, u32), ()>,
    /// Images no longer shown, for the window to drop.
    retired: Rc<RefCell<Vec<Arc<RenderImage>>>>,
}

impl Drop for ImageView {
    fn drop(&mut self) {
        let mut retired = self.retired.borrow_mut();
        for side in [&self.old, &self.new] {
            if let Some(d) = side.decoded() {
                retired.push(d.image.clone());
            }
        }
        retired.extend(self.diff_image.take());
        retired.extend(self.crisp.drain().map(|(_, v)| v));
    }
}

impl ImageView {
    pub fn new(old: Side, new: Side, svg: bool, background: Background, match_pick: Option<bool>, retired: Rc<RefCell<Vec<Arc<RenderImage>>>>) -> Self {
        Self {
            old,
            new,
            svg,
            mode: Mode::Side,
            background,
            view: View { scale: 1., x: 0., y: 0. },
            step: DEFAULT_STEP,
            zoom_pct: 100,
            fit: false,
            amplify: AMPLIFY_DEFAULT,
            swipe: MODE_START,
            opacity: MODE_START,
            match_pick,
            zoom_edit: false,
            source_on: false,
            drag: ImageDrag::None,
            stage: (0., 0.),
            last_box: None,
            placed: false,
            wheel: 0.,
            diff_image: None,
            diff_generation: 0,
            diff_key: None,
            stage_left: Rc::default(),
            zoom_track: Rc::default(),
            opacity_track: Rc::default(),
            amplify_track: Rc::default(),
            crisp: HashMap::new(),
            crisp_pending: HashMap::new(),
            retired,
        }
    }

    /// The sides a view shows: File View has one revision.
    pub fn sides(&self, file: bool) -> (&Side, &Side) {
        let none = &Side::None;
        (if file && self.new.exists() { none } else { &self.old }, if file && !self.new.exists() { none } else { &self.new })
    }

    fn both(&self, file: bool) -> bool {
        let (o, n) = self.sides(file);
        o.exists() && n.exists()
    }

    pub fn active_mode(&self, file: bool) -> Mode {
        if self.both(file) { self.mode } else { Mode::Side }
    }

    pub fn can_match(&self, file: bool) -> bool {
        let (o, n) = self.sides(file);
        self.both(file) && sizes_differ(o.dim(), n.dim())
    }

    pub fn matched(&self, file: bool) -> bool {
        let (o, n) = self.sides(file);
        self.can_match(file) && self.match_pick.unwrap_or_else(|| default_match(o.dim(), n.dim()))
    }

    /// Where the old side is drawn while matched, in the new image's pixels.
    pub fn old_rect(&self, file: bool) -> Option<Rect> {
        let (o, n) = self.sides(file);
        if self.matched(file) { Some(matched_rect(o.dim()?, n.dim()?)) } else { None }
    }

    /// The common size every mode, zoom and pan work in: the new image's while matched.
    pub fn content(&self, file: bool) -> (f32, f32) {
        let (o, n) = self.sides(file);
        if self.matched(file)
            && let Some(d) = n.dim()
        {
            return d;
        }
        let (a, b) = (o.dim().unwrap_or((0., 0.)), n.dim().unwrap_or((0., 0.)));
        (a.0.max(b.0), a.1.max(b.1))
    }

    /// A viewport's size: side-by-side has two, with a gap.
    pub fn box_size(&self, file: bool) -> (f32, f32) {
        let (w, h) = self.stage;
        if self.active_mode(file) == Mode::Side && self.both(file) { ((w - 2.) / 2., h) } else { (w, h) }
    }

    fn clamped(&self, v: View, file: bool) -> View {
        let (w, h) = self.content(file);
        let (bw, bh) = self.box_size(file);
        clamp_view(v, w, h, bw, bh)
    }

    /// A rung of the ladder. `around`: the point to keep steady; `start`: a new image.
    pub fn apply_step(&mut self, step: usize, around: Option<(f32, f32)>, start: bool, file: bool) {
        self.step = step;
        self.zoom_pct = (ZOOM_STEPS[step] * 100.).round() as u32;
        self.fit = false;
        let (w, h) = self.content(file);
        if w == 0. {
            return;
        }
        let (bw, bh) = self.box_size(file);
        self.last_box = Some((bw, bh));
        let s = ZOOM_STEPS[step];
        let (px, py) = around.unwrap_or((bw / 2., bh / 2.));
        self.view = if start { start_view(s, w, h, bw, bh) } else { self.clamped(zoom_around(self.view, s, px, py), file) };
    }

    /// The Fit button: the exact % that fits, kept across a resize until another zoom turns it off.
    pub fn apply_fit(&mut self, file: bool) {
        let (w, h) = self.content(file);
        if w == 0. {
            return;
        }
        let (bw, bh) = self.box_size(file);
        self.last_box = Some((bw, bh));
        let s = fit_scale(w, h, bw, bh);
        self.view = centered(s, w, h, bw, bh);
        self.zoom_pct = (s * 100.).round() as u32;
        self.step = nearest_step_index(s);
        self.fit = true;
    }

    /// A typed exact %, zoomed around the box's centre.
    pub fn apply_scale(&mut self, scale: f32, file: bool) {
        let scale = scale.clamp(ZOOM_STEPS[0], ZOOM_STEPS[ZOOM_STEPS.len() - 1]);
        let (bw, bh) = self.box_size(file);
        self.last_box = Some((bw, bh));
        self.view = self.clamped(zoom_around(self.view, scale, bw / 2., bh / 2.), file);
        self.zoom_pct = (scale * 100.).round() as u32;
        self.step = nearest_step_index(scale);
        self.fit = false;
    }

    pub fn pan_by(&mut self, dx: f32, dy: f32, file: bool) {
        self.view = self.clamped(View { x: self.view.x + dx, y: self.view.y + dy, ..self.view }, file);
    }

    /// After a layout, a mode switch or a match toggle: places a new image at 100%, refits while on
    /// Fit, otherwise keeps the view and its centre.
    pub fn sync(&mut self, stage: (f32, f32), file: bool) {
        self.stage = stage;
        if !self.placed {
            self.placed = true;
            self.apply_step(DEFAULT_STEP, None, true, file);
            return;
        }
        if self.fit {
            self.apply_fit(file);
            return;
        }
        let (w, h) = self.box_size(file);
        let prev = self.last_box.replace((w, h));
        if let Some(prev) = prev
            && prev != (w, h)
        {
            self.view = self.clamped(View { x: self.view.x + (w - prev.0) / 2., y: self.view.y + (h - prev.1) / 2., ..self.view }, file);
        } else {
            self.view = self.clamped(self.view, file);
        }
    }

    /// File View and Diff View show different sides: it opens anew.
    pub fn view_changed(&mut self) {
        self.placed = false;
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if mode != self.mode && mode == Mode::Swipe {
            self.swipe = MODE_START;
        }
        if mode != self.mode && mode == Mode::Onion {
            self.opacity = MODE_START;
        }
        self.mode = mode;
        self.source_on = false;
    }

    /// The handle's place over the whole image, in % of the box.
    pub fn swipe_pct(&self, file: bool) -> f32 {
        let (bw, _) = self.box_size(file);
        let (w, _) = self.content(file);
        if bw > 0. && w > 0. { clamp_swipe(self.swipe / 100. * bw, self.view, w, bw) / bw * 100. } else { self.swipe }
    }

    /// A wheel's travel: 1 (zoom in), -1 (out) or 0 (not a step yet).
    pub fn wheel_step(&mut self, dy: f32, lines: bool) -> i32 {
        if dy == 0. {
            return 0;
        }
        if lines {
            self.wheel = 0.;
            return if dy > 0. { 1 } else { -1 };
        }
        if dy.signum() != self.wheel.signum() {
            self.wheel = 0.;
        }
        self.wheel += dy;
        if self.wheel.abs() < WHEEL_STEP_PX {
            return 0;
        }
        self.wheel = 0.;
        if dy > 0. { 1 } else { -1 }
    }

    /// The toolbar's "320×200 → 400×240 · 106.7 KB → 128.7 KB".
    pub fn meta(&self, file: bool) -> (Vec<String>, Vec<String>, Option<&'static str>) {
        let (o, n) = self.sides(file);
        if o.exists() && n.exists() {
            return (vec![o.dims(), n.dims()], vec![format_bytes(o.size()), format_bytes(n.size())], None);
        }
        let side = if o.exists() { o } else { n };
        let single = if file { None } else if !self.old.exists() { Some("added") } else { Some("deleted") };
        (vec![side.dims()], vec![format_bytes(side.size())], single)
    }

    /// The images a frame at this scale would show, and the factor of each one's crisp copy.
    fn wanted_crisp(&self, file: bool) -> Vec<(Arc<RenderImage>, u32)> {
        let (o, n) = self.sides(file);
        let mut out = Vec::new();
        let images = [o.decoded().map(|d| d.image.clone()), n.decoded().map(|d| d.image.clone()), if self.active_mode(file) == Mode::Difference { self.diff_image.clone() } else { None }];
        for image in images.into_iter().flatten() {
            let s = image.size(0);
            if let Some(f) = crisp_factor(self.view.scale, s.width.0 as u32, s.height.0 as u32) {
                out.push((image, f));
            }
        }
        out
    }

    fn crisp_or<'a>(&'a self, image: &'a Arc<RenderImage>, scale: f32) -> &'a Arc<RenderImage> {
        let s = image.size(0);
        crisp_factor(scale, s.width.0 as u32, s.height.0 as u32).and_then(|f| self.crisp.get(&(image_key(image), f))).unwrap_or(image)
    }

    fn retire(&self, image: Arc<RenderImage>) {
        self.retired.borrow_mut().push(image);
    }
}

fn image_key(image: &RenderImage) -> usize {
    image.id.0
}

impl GitVolt {
    pub fn with_image(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut ImageView, bool)) {
        self.with_diff(cx, |d| {
            let file = d.view == ViewKind::File;
            if let Some(img) = d.image.as_mut() {
                f(img, file);
            }
        });
        self.after_image_change(cx);
    }

    /// What a change may need from off the UI thread: the Difference canvas, and crisp copies.
    pub fn after_image_change(&mut self, cx: &mut Context<Self>) {
        self.request_difference(cx);
        self.request_crisp(cx);
    }

    fn request_difference(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let repo = tab.id;
        let Some(diff) = tab.diff.as_mut() else { return };
        let generation = diff.generation;
        let file = diff.view == ViewKind::File;
        let Some(img) = diff.image.as_mut() else { return };
        if img.active_mode(file) != Mode::Difference || cfg!(test) {
            return;
        }
        let want = (img.amplify, img.matched(file));
        if img.diff_key == Some(want) {
            return;
        }
        img.diff_key = Some(want);
        let (o, n) = img.sides(file);
        let (Some(o), Some(n)) = (o.decoded().cloned(), n.decoded().cloned()) else { return };
        let (w, h) = img.content(file);
        let (rect, amplify) = (img.old_rect(file), img.amplify);
        img.diff_generation += 1;
        let job = img.diff_generation;
        let made = self.rt.spawn_blocking(move || difference(&o, &n, w, h, rect, amplify));
        cx.spawn(async move |this, cx| {
            let Ok(Some(made)) = made.await else { return };
            this.update(cx, |this, cx| {
                let Some(img) = this.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.diff.as_mut()).filter(|d| d.generation == generation).and_then(|d| d.image.as_mut()) else { return };
                if img.diff_generation == job {
                    if let Some(old) = img.diff_image.replace(Arc::new(made)) {
                        img.retire(old);
                    }
                    this.request_crisp(cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn request_crisp(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let repo = tab.id;
        let Some(diff) = tab.diff.as_mut() else { return };
        let generation = diff.generation;
        let file = diff.view == ViewKind::File;
        let Some(img) = diff.image.as_mut() else { return };
        if cfg!(test) {
            return;
        }
        let wanted = img.wanted_crisp(file);
        // What this zoom doesn't show any more goes: the copies are large.
        let keep: Vec<(usize, u32)> = wanted.iter().map(|(i, f)| (image_key(i), *f)).collect();
        let stale: Vec<(usize, u32)> = img.crisp.keys().filter(|k| !keep.contains(k)).copied().collect();
        for k in stale {
            if let Some(v) = img.crisp.remove(&k) {
                img.retire(v);
            }
        }
        for (image, factor) in wanted {
            let key = (image_key(&image), factor);
            if img.crisp.contains_key(&key) || img.crisp_pending.contains_key(&key) {
                continue;
            }
            img.crisp_pending.insert(key, ());
            let made = self.rt.spawn_blocking(move || crisp(&image, factor));
            cx.spawn(async move |this, cx| {
                let made = made.await.ok().flatten();
                this.update(cx, |this, cx| {
                    let Some(img) = this.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.diff.as_mut()).filter(|d| d.generation == generation).and_then(|d| d.image.as_mut()) else { return };
                    img.crisp_pending.remove(&key);
                    if let Some(made) = made {
                        img.crisp.insert(key, Arc::new(made));
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    /// The Hex or Source toggle. A raster image's dumps load when they're first asked for.
    fn toggle_source(&mut self, cx: &mut Context<Self>) {
        let mut load = false;
        self.with_diff(cx, |d| {
            if let Some(img) = d.image.as_mut() {
                img.source_on = !img.source_on;
                load = img.source_on && !img.svg && d.hex.is_none();
            }
            d.layout_again();
        });
        if load {
            self.read_hex(cx);
        }
    }

    fn set_match(&mut self, on: bool, cx: &mut Context<Self>) {
        self.with_image(cx, |img, file| {
            img.match_pick = Some(on);
            img.sync(img.stage, file);
        });
        self.with_diff(cx, |d| {
            let path = d.file.path.to_string();
            d.match_picks.borrow_mut().insert(path, on);
        });
    }
}

/// The range input, as Chromium draws it in dark: an 8 px track, filled to a 16 px thumb.
fn paint_slider(b: Bounds<Pixels>, frac: f32, window: &mut Window) {
    let y = b.origin.y + (b.size.height - px(8.)) / 2.;
    let track = Bounds::new(point(b.origin.x, y), size(b.size.width, px(8.)));
    window.paint_quad(quad(track, px(4.), theme::range_track(), px(1.), theme::range_border(), BorderStyle::default()));
    let x = b.origin.x + px(8.) + (b.size.width - px(16.)) * frac.clamp(0., 1.);
    window.paint_quad(quad(Bounds::from_corners(track.origin, point(x, track.origin.y + px(8.))), px(4.), theme::range_fill(), px(0.), transparent_black(), BorderStyle::default()));
    window.paint_quad(quad(Bounds::new(point(x - px(8.), b.origin.y + (b.size.height - px(16.)) / 2.), size(px(16.), px(16.))), px(8.), theme::range_fill(), px(0.), transparent_black(), BorderStyle::default()));
}

/// Where a press or a drag falls along a slider: the thumb's centre travels inside its half-width.
pub fn slider_frac(x: f32, left: f32, width: f32) -> f32 {
    ((x - left - 8.) / (width - 16.).max(1.)).clamp(0., 1.)
}

/// A slider, from its value's fraction. `on`: what a press and a drag set.
fn slider(id: &'static str, width: Option<f32>, frac: f32, which: ImageDrag, track: Rc<Cell<Option<(f32, f32)>>>, cx: &mut Context<GitVolt>) -> Stateful<Div> {
    let (paint, press) = (track.clone(), track);
    let el = div().id(id).h(px(SLIDER_H)).when_some(width, |d, w| d.flex_none().w(px(w))).when(width.is_none(), |d| d.flex_1().min_w_0());
    el.on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            if let Some((left, width)) = press.get() {
                this.set_slider(which, slider_frac(e.position.x.as_f32(), left, width), cx);
            }
            this.with_image(cx, |img, _| img.drag = which);
        }),
    )
    .child(
        canvas(
            |_, _, _| {},
            move |b, _, window, _| {
                paint.set(Some((b.origin.x.as_f32(), b.size.width.as_f32())));
                paint_slider(b, frac, window);
            },
        )
        .size_full(),
    )
}

impl GitVolt {
    fn set_slider(&mut self, which: ImageDrag, frac: f32, cx: &mut Context<Self>) {
        self.with_image(cx, |img, file| match which {
            ImageDrag::Zoom => img.apply_step((frac * (ZOOM_STEPS.len() - 1) as f32).round() as usize, None, false, file),
            ImageDrag::Opacity => img.opacity = (frac * 100.).round(),
            ImageDrag::Amplify => img.amplify = 1 + (frac * (AMPLIFY_MAX - 1) as f32).round() as u32,
            _ => {}
        });
    }
}

fn toolbar_button(id: &'static str, label: impl Into<SharedString>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(8.))
        .py(px(2.))
        .rounded(px(4.))
        .text_color(if on { theme::text_selected() } else { theme::text_secondary() })
        .when(on, |d| d.bg(theme::selected_row()))
        .when(!on, |d| d.hover(|d| d.bg(theme::hover_row()).text_color(theme::text_selected())))
        .child(label.into())
}

fn arrowed(parts: Vec<String>) -> Div {
    let mut d = div().flex().flex_none().items_center().gap(px(4.));
    for (i, p) in parts.into_iter().enumerate() {
        if i > 0 {
            d = d.child(icon("arrow-right", 12., theme::text_disabled()));
        }
        d = d.child(p);
    }
    d
}

/// The checkerboard toggle's icon: a 3×3 board of big squares.
fn checker_icon() -> Div {
    let cell = |x: f32, y: f32| div().absolute().left(px(x * 4.)).top(px(y * 4.)).size(px(4.)).bg(theme::c(0xcfcfcf));
    div().relative().size(px(12.)).rounded(px(2.)).overflow_hidden().bg(theme::c(0x4a4a4a)).border_1().border_color(theme::a(0xffffff59)).child(cell(0., 0.)).child(cell(2., 0.)).child(cell(1., 1.)).child(cell(0., 2.)).child(cell(2., 2.))
}

fn background_button(id: &'static str, which: Background, on: bool, swatch: Div, cx: &mut Context<GitVolt>) -> AnyElement {
    div()
        .id(id)
        .flex()
        .flex_none()
        .size(px(22.))
        .items_center()
        .justify_center()
        .rounded(px(3.))
        .when(on, |d| d.bg(theme::selected_row()))
        .when(!on, |d| d.hover(|d| d.bg(theme::hover_row())))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.with_diff(cx, |d| {
                d.background = which;
                if let Some(img) = d.image.as_mut() {
                    img.background = which;
                }
            });
        }))
        .child(swatch)
        .into_any_element()
}

fn image_toolbar(img: &ImageView, file: bool, zoom_input: &Entity<TextInput>, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let both = img.both(file);
    let active = img.active_mode(file);
    let mode_button = |id: &'static str, label: &'static str, mode: Mode, cx: &mut Context<GitVolt>| {
        segment(id, label, active == mode && !img.source_on, true)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.with_diff(cx, |d| {
                    let file = d.view == ViewKind::File;
                    if let Some(img) = d.image.as_mut() {
                        img.set_mode(mode);
                        img.sync(img.stage, file);
                    }
                    d.layout_again();
                });
                this.after_image_change(cx);
            }))
            .into_any_element()
    };
    let (dims, sizes, single) = img.meta(file);
    let matched = img.matched(file);
    let swatch = |bg: Hsla| div().size(px(12.)).rounded(px(2.)).bg(bg).border_1().border_color(theme::a(0xffffff59));
    div()
        .flex()
        .flex_none()
        .h(px(IMAGE_TOOLBAR_H))
        .items_center()
        .gap(px(10.))
        .px(px(8.))
        .border_b_1()
        .border_color(theme::section_border())
        .when(both, |d| d.child(segmented(vec![mode_button("img-side", "Side-by-side", Mode::Side, cx), mode_button("img-swipe", "Swipe", Mode::Swipe, cx), mode_button("img-onion", "Onion skin", Mode::Onion, cx), mode_button("img-diff", "Difference", Mode::Difference, cx)])))
        .when(img.can_match(file), |d| {
            d.child(
                div()
                    .id("img-match")
                    .flex()
                    .flex_none()
                    .size(px(22.))
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .when(matched, |d| d.bg(theme::selected_row()))
                    .when(!matched, |d| d.hover(|d| d.bg(theme::hover_row())))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_match(!matched, cx)))
                    .child(icon("scaling", 14., if matched { theme::text_selected() } else { theme::text_secondary() })),
            )
        })
        .child(
            div()
                .id("img-source")
                .flex_none()
                .px(px(8.))
                .py(px(2.))
                .border_1()
                .border_color(theme::section_border())
                .rounded(px(4.))
                .text_color(if img.source_on { theme::text_selected() } else { theme::text_secondary() })
                .when(img.source_on, |d| d.bg(theme::selected_row()))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_source(cx)))
                .child(if img.svg { "Source" } else { "Hex" }),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .child(toolbar_button("img-fit", "Fit", img.fit).on_click(cx.listener(|this, _, _, cx| this.with_image(cx, |img, file| img.apply_fit(file)))))
                .child(slider("img-zoom", Some(SLIDER_W), img.step as f32 / (ZOOM_STEPS.len() - 1) as f32, ImageDrag::Zoom, img.zoom_track.clone(), cx).on_click(cx.listener(|this, e: &ClickEvent, _, cx| {
                    // A double click resets to 100%.
                    if e.click_count() == 2 {
                        this.with_image(cx, |img, file| img.apply_step(DEFAULT_STEP, None, false, file));
                    }
                })))
                .child(match img.zoom_edit {
                    true => div()
                        .id("zoom-input")
                        .flex()
                        .flex_none()
                        .w(px(51.))
                        .items_center()
                        .px(px(4.))
                        .py(px(1.))
                        .bg(theme::app_bg())
                        .border_1()
                        .border_color(theme::section_border())
                        .rounded(px(4.))
                        .text_color(theme::text_selected())
                        .child(zoom_input.clone())
                        .into_any_element(),
                    false => toolbar_button("img-zoom-pct", format!("{}%", img.zoom_pct), false)
                        .min_w(px(51.))
                        .flex()
                        .justify_center()
                        .on_click(cx.listener(|this, _, window, cx| this.start_zoom_edit(window, cx)))
                        .into_any_element(),
                }),
        )
        .child(
            div()
                .flex()
                .flex_shrink(1.)
                .min_w_0()
                .overflow_hidden()
                .ml(px(6.))
                .items_center()
                .gap(px(6.))
                .text_color(theme::text_disabled())
                .child(arrowed(dims))
                .child("·")
                .child(arrowed(sizes))
                .when_some(single, |d, s| d.child(format!("({s})"))),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .gap(px(2.))
                .ml_auto()
                .child(background_button("bg-checker", Background::Checker, img.background == Background::Checker, checker_icon(), cx))
                .child(background_button("bg-black", Background::Black, img.background == Background::Black, swatch(theme::c(0x000000)), cx))
                .child(background_button("bg-white", Background::White, img.background == Background::White, swatch(theme::c(0xffffff)), cx))
                .child(background_button("bg-grey", Background::Grey, img.background == Background::Grey, swatch(theme::c(0x808080)), cx)),
        )
}

/// What's behind the pixels, in the image's bounds on screen: the pick, and a 1 px border outside it.
fn paint_frame(r: Bounds<Pixels>, background: Background, window: &mut Window) {
    window.paint_quad(quad(Bounds::new(r.origin - point(px(1.), px(1.)), size(r.size.width + px(2.), r.size.height + px(2.))), px(0.), transparent_black(), px(1.), theme::image_frame_border(), BorderStyle::default()));
    match background {
        Background::Black => window.paint_quad(fill(r, theme::c(0x000000))),
        Background::White => window.paint_quad(fill(r, theme::c(0xffffff))),
        Background::Grey => window.paint_quad(fill(r, theme::c(0x808080))),
        Background::Checker => {
            window.paint_quad(fill(r, theme::c(0x2a2a2a)));
            // Only what's on screen: a zoomed-in image's frame is far larger than the window.
            let clip = window.content_mask().bounds.intersect(&r);
            let tile = px(8.);
            let (x0, y0) = (((clip.origin.x - r.origin.x) / tile).floor() as i64, ((clip.origin.y - r.origin.y) / tile).floor() as i64);
            let (x1, y1) = (((clip.origin.x + clip.size.width - r.origin.x) / tile).ceil() as i64, ((clip.origin.y + clip.size.height - r.origin.y) / tile).ceil() as i64);
            for y in y0..y1 {
                for x in x0..x1 {
                    if (x + y) % 2 == 0 {
                        continue;
                    }
                    let at = r.origin + point(tile * x as f32, tile * y as f32);
                    let cell = Bounds::new(at, size(tile, tile)).intersect(&r);
                    window.paint_quad(fill(cell, theme::c(0x3a3a3a)));
                }
            }
        }
    }
}

fn rect_of(b: Bounds<Pixels>, v: View, at: Rect) -> Bounds<Pixels> {
    Bounds::new(point(b.origin.x + px(v.x + at.x * v.scale), b.origin.y + px(v.y + at.y * v.scale)), size(px(at.w * v.scale), px(at.h * v.scale)))
}

fn paint_layer(clip: Bounds<Pixels>, image: Arc<RenderImage>, at: Bounds<Pixels>, window: &mut Window) {
    window.paint_image(clip, at, Corners::default(), image, 0, false).ok();
}

fn broken() -> impl IntoElement {
    div().absolute().top_0().left_0().size_full().flex().items_center().justify_center().text_color(theme::text_secondary()).child("Image couldn't be decoded")
}

/// A chip over the image, never in the way of a drag.
fn chip(text: Vec<AnyElement>, large: bool) -> Div {
    div()
        .flex()
        .items_center()
        .rounded(px(3.))
        .bg(theme::a(0x0000008c))
        .text_color(theme::a(0xffffffd9))
        .text_size(px(if large { 13. } else { 10. }))
        .line_height(px(if large { 19.5 } else { 15. }))
        .px(px(if large { 8. } else { 6. }))
        .py(px(if large { 2. } else { 1. }))
        .children(text)
}

fn old_label(matched: bool) -> Vec<AnyElement> {
    let mut out = vec![div().child("OLD").into_any_element()];
    if matched {
        out.push(div().ml(px(6.)).opacity(0.75).child("scaled to match").into_any_element());
    }
    out
}

/// One viewport's canvas: the frame and what `layers` paint, in its bounds.
fn viewport_canvas(frame: Option<(f32, f32)>, background: Background, v: View, layers: Vec<(Arc<RenderImage>, Rect, Option<f32>)>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            window.with_content_mask(Some(ContentMask { bounds: b }), |window| {
                if let Some((w, h)) = frame {
                    paint_frame(rect_of(b, v, Rect { x: 0., y: 0., w, h }), background, window);
                }
                for (image, at, clip_from) in layers {
                    let mut clip = b;
                    if let Some(from) = clip_from {
                        clip = Bounds::from_corners(point(b.origin.x + px(from), b.origin.y), point(b.origin.x + b.size.width, b.origin.y + b.size.height));
                    }
                    paint_layer(clip, image, rect_of(b, v, at), window);
                }
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

fn own_rect(d: &Decoded) -> Rect {
    Rect { x: 0., y: 0., w: d.w, h: d.h }
}

/// The viewports, in the stage below the toolbar.
fn stage(img: &ImageView, file: bool, cx: &mut Context<GitVolt>) -> AnyElement {
    let (o, n) = img.sides(file);
    let both = img.both(file);
    let mode = img.active_mode(file);
    let (v, bg) = (img.view, img.background);
    let matched = img.matched(file);
    let content = img.content(file);
    let overlay = (content.0 > 0.).then_some(content);
    let old_at = img.old_rect(file);
    let layer = |d: &Arc<Decoded>, at: Rect, clip: Option<f32>| (img.crisp_or(&d.image, v.scale).clone(), at, clip);
    let grab = if matches!(img.drag, ImageDrag::Pan(..)) { CursorStyle::ClosedHand } else { CursorStyle::OpenHand };
    let viewport = || div().relative().flex_1().min_w_0().h_full().overflow_hidden().bg(theme::app_bg()).cursor(grab);
    let mut stage = div()
        .id("image-stage")
        .relative()
        .flex()
        .flex_1()
        .min_h_0()
        .gap(px(2.))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                let pos = (e.position.x.as_f32(), e.position.y.as_f32());
                this.with_image(cx, |img, file| {
                    if img.active_mode(file) == Mode::Swipe {
                        img.drag = ImageDrag::Swipe;
                        if let Some(left) = img.stage_left.get() {
                            img.swipe = ((pos.0 - left) / img.box_size(file).0.max(1.) * 100.).clamp(0., 100.);
                        }
                    } else {
                        img.drag = ImageDrag::Pan(pos.0, pos.1);
                    }
                });
            }),
        );
    match mode {
        Mode::Side => {
            if let Side::Image { .. } | Side::Failed { .. } = o {
                let frame = if matched { n.dim() } else { o.dim() };
                let layers: Vec<_> = o.decoded().map(|d| layer(d, old_at.unwrap_or(own_rect(d)), None)).into_iter().collect();
                stage = stage.child(viewport().child(viewport_canvas(frame, bg, v, layers)).when(o.failed(), |d| d.child(broken())).when(both, |d| d.child(div().absolute().top(px(8.)).left_0().right_0().flex().justify_center().child(chip(old_label(matched), true)))));
            }
            if both {
                stage = stage.child(div().absolute().top_0().bottom_0().left(relative(0.5)).w(px(1.)).bg(theme::section_border()));
            }
            if n.exists() {
                let layers: Vec<_> = n.decoded().map(|d| layer(d, own_rect(d), None)).into_iter().collect();
                stage = stage.child(viewport().child(viewport_canvas(n.dim(), bg, v, layers)).when(n.failed(), |d| d.child(broken())).when(both, |d| d.child(div().absolute().top(px(8.)).left_0().right_0().flex().justify_center().child(chip(vec![div().child("NEW").into_any_element()], true)))));
            }
        }
        Mode::Swipe => {
            let pct = img.swipe_pct(file);
            let from = pct / 100. * img.box_size(file).0;
            let mut layers: Vec<_> = o.decoded().map(|d| layer(d, old_at.unwrap_or(own_rect(d)), None)).into_iter().collect();
            layers.extend(n.decoded().map(|d| layer(d, own_rect(d), Some(from))));
            stage = stage.child(
                viewport()
                    .child(viewport_canvas(overlay, bg, v, layers))
                    .when(o.failed() || n.failed(), |d| d.child(broken()))
                    .child(div().absolute().bottom(px(6.)).left(px(6.)).child(chip(old_label(matched), false)))
                    .child(div().absolute().bottom(px(6.)).right(px(6.)).child(chip(vec![div().child("NEW").into_any_element()], false)))
                    .child(div().absolute().top_0().bottom_0().left(relative(pct / 100.)).ml(px(-1.)).w(px(3.)).bg(theme::blue()).cursor(CursorStyle::ResizeLeftRight)),
            );
        }
        Mode::Onion => {
            let old_layers: Vec<_> = o.decoded().map(|d| layer(d, old_at.unwrap_or(own_rect(d)), None)).into_iter().collect();
            let new_layers: Vec<_> = n.decoded().map(|d| layer(d, own_rect(d), None)).into_iter().collect();
            let label = |text: Vec<AnyElement>| div().flex().flex_none().items_center().text_size(px(10.)).line_height(px(15.)).text_color(theme::text_secondary()).children(text);
            stage = stage.child(
                viewport()
                    .child(viewport_canvas(overlay, bg, v, old_layers))
                    .child(div().absolute().top_0().left_0().size_full().opacity(img.opacity / 100.).child(viewport_canvas(None, bg, v, new_layers)))
                    .when(o.failed() || n.failed(), |d| d.child(broken()))
                    .child(
                        div().absolute().left_0().right_0().bottom(px(10.)).flex().justify_center().child(
                            div()
                                .id("onion-control")
                                .flex()
                                .w(relative(0.4))
                                .items_center()
                                .gap(px(6.))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(label(old_label(matched)))
                                .child(slider("onion-opacity", None, img.opacity / 100., ImageDrag::Opacity, img.opacity_track.clone(), cx))
                                .child(label(vec![div().child("NEW").into_any_element()])),
                        ),
                    ),
            );
        }
        Mode::Difference => {
            let layers: Vec<_> = img.diff_image.iter().map(|i| (img.crisp_or(i, v.scale).clone(), Rect { x: 0., y: 0., w: content.0, h: content.1 }, None)).collect();
            stage = stage.child(
                viewport()
                    .child(viewport_canvas(overlay, Background::Black, v, layers))
                    .when(o.failed() || n.failed(), |d| d.child(broken()))
                    .child(
                        div().absolute().left_0().right_0().bottom(px(10.)).flex().justify_center().child(
                            div()
                                .id("difference-amplify")
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(div().flex_none().text_size(px(10.)).line_height(px(15.)).text_color(theme::text_secondary()).child("AMPLIFY"))
                                .child(slider("amplify", Some(SLIDER_W), (img.amplify - 1) as f32 / (AMPLIFY_MAX - 1) as f32, ImageDrag::Amplify, img.amplify_track.clone(), cx))
                                .child(div().flex_none().min_w(px(21.)).text_color(theme::text_secondary()).child(format!("{}×", img.amplify))),
                        ),
                    ),
            );
        }
    }
    let left = img.stage_left.clone();
    let tracking = img.drag != ImageDrag::None;
    let entity = cx.entity().downgrade();
    stage
        // The stage's place, for a press; and while something's dragged, the pointer wherever it goes.
        .child(
            canvas(
                |_, _, _| {},
                move |b, _, window, _| {
                    left.set(Some(b.origin.x.as_f32()));
                    let scroll_entity = entity.clone();
                    window.on_mouse_event(move |e: &ScrollEvent, phase, window, cx| {
                        if phase != DispatchPhase::Capture || !b.contains(&e.position) || !e.modifiers.control {
                            return;
                        }
                        cx.stop_propagation();
                        let lines = matches!(e.delta, ScrollDelta::Lines(_));
                        let dy = e.delta.pixel_delta(window.line_height()).y.as_f32();
                        let at = ((e.position.x - b.origin.x).as_f32(), (e.position.y - b.origin.y).as_f32());
                        scroll_entity.update(cx, |this, cx| this.with_image(cx, |img, file| {
                            let step = img.wheel_step(dy, lines);
                            if step != 0 {
                                // In a side-by-side's second box, the point within it.
                                let (bw, _) = img.box_size(file);
                                let x = if img.active_mode(file) == Mode::Side && img.both(file) && at.0 > bw { at.0 - bw - 2. } else { at.0 };
                                img.apply_step(next_step_index(img.view.scale, step > 0), Some((x, at.1)), false, file);
                            }
                        })).ok();
                    });
                    if !tracking {
                        return;
                    }
                    let move_entity = entity.clone();
                    window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                        if phase != DispatchPhase::Capture {
                            return;
                        }
                        let pos = (e.position.x.as_f32(), e.position.y.as_f32());
                        let pressed = e.pressed_button == Some(MouseButton::Left);
                        move_entity.update(cx, |this, cx| this.image_drag_to(pos, pressed, cx)).ok();
                    });
                    let up_entity = entity.clone();
                    window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            up_entity.update(cx, |this, cx| this.with_image(cx, |img, _| img.drag = ImageDrag::None)).ok();
                        }
                    });
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .into_any_element()
}

impl GitVolt {
    fn image_drag_to(&mut self, pos: (f32, f32), pressed: bool, cx: &mut Context<Self>) {
        let Some(img) = self.tabs.get(self.active).and_then(|t| t.diff.as_ref()).and_then(|d| d.image.as_ref()) else { return };
        let drag = img.drag;
        if !pressed {
            self.with_image(cx, |img, _| img.drag = ImageDrag::None);
            return;
        }
        match drag {
            ImageDrag::Pan(x, y) => self.with_image(cx, |img, file| {
                img.pan_by(pos.0 - x, pos.1 - y, file);
                img.drag = ImageDrag::Pan(pos.0, pos.1);
            }),
            ImageDrag::Swipe => self.with_image(cx, |img, file| {
                if let Some(left) = img.stage_left.get() {
                    img.swipe = ((pos.0 - left) / img.box_size(file).0.max(1.) * 100.).clamp(0., 100.);
                }
            }),
            which @ (ImageDrag::Zoom | ImageDrag::Opacity | ImageDrag::Amplify) => {
                if let Some(b) = self.slider_track(which) {
                    self.set_slider(which, slider_frac(pos.0, b.0, b.1), cx);
                }
            }
            ImageDrag::None => {}
        }
    }

    /// A click on the zoom %: type an exact one.
    fn start_zoom_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut pct = None;
        self.with_image(cx, |img, _| {
            img.zoom_edit = true;
            pct = Some(img.zoom_pct);
        });
        let Some(pct) = pct else { return };
        let Some(zoom_input) = self.tabs.get(self.active).map(|t| t.zoom_input.clone()) else { return };
        zoom_input.update(cx, |input, cx| {
            input.set_text(&pct.to_string(), cx);
            input.select_all_text(cx);
        });
        window.focus(&zoom_input.read(cx).focus.clone(), cx);
    }

    /// Enter or leaving it applies what's typed; Esc doesn't.
    pub fn end_zoom_edit(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some(typed) = self.tabs.get(self.active).map(|t| t.zoom_input.read(cx).text().to_string()) else { return };
        let mut was = false;
        self.with_image(cx, |img, file| {
            was = std::mem::take(&mut img.zoom_edit);
            if was
                && commit
                && let Ok(n) = typed.trim().parse::<f32>()
                && n.is_finite()
                && n > 0.
            {
                img.apply_scale(n / 100., file);
            }
        });
        if was {
            self.focus_next_frame = Some(self.focus.clone());
        }
    }

    fn slider_track(&self, which: ImageDrag) -> Option<(f32, f32)> {
        let img = self.tabs.get(self.active)?.diff.as_ref()?.image.as_ref()?;
        match which {
            ImageDrag::Zoom => img.zoom_track.get(),
            ImageDrag::Opacity => img.opacity_track.get(),
            ImageDrag::Amplify => img.amplify_track.get(),
            _ => None,
        }
    }
}

pub fn image_body(img: &ImageView, file: bool, source: Option<AnyElement>, zoom_input: &Entity<TextInput>, cx: &mut Context<GitVolt>) -> AnyElement {
    let body = match source {
        Some(source) if img.source_on => source,
        _ => stage(img, file, cx),
    };
    div().flex().flex_col().flex_1().min_w_0().min_h_0().child(image_toolbar(img, file, zoom_input, cx)).child(body).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn png(w: u32, h: u32) -> String {
        let image = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).unwrap();
        base64::engine::general_purpose::STANDARD.encode(out.into_inner())
    }

    fn side(w: u32, h: u32) -> Side {
        Side::Image { decoded: Arc::new(decode_raster(&png(w, h)).unwrap()), size: 100 }
    }

    fn view(old: Side, new: Side) -> ImageView {
        ImageView::new(old, new, false, Background::Checker, None, Rc::default())
    }

    /// A tab with an image open; the stage is under the header, the toolbar and the image's toolbar.
    fn opened(v: &mut GitVolt, cx: &mut Context<GitVolt>, img: ImageView) {
        let tab = &mut v.tabs[0];
        let mut d = crate::diff::view::DiffState::new(crate::details::FileChange { path: "a.png".into(), old_path: None, status: 'M', additions: None, deletions: None, old: Value::Null, new: Value::Null, conflict: None, origin: crate::details::Origin::Commit { id: "c".into() } });
        d.notice = None;
        d.image = Some(img);
        tab.diff = Some(d);
        cx.notify();
    }

    fn image_of<R>(window: &WindowHandle<GitVolt>, cx: &mut VisualTestContext, f: impl FnOnce(&ImageView) -> R) -> R {
        window.read_with(cx, |v, _| f(v.tabs[0].diff.as_ref().unwrap().image.as_ref().unwrap())).unwrap()
    }

    #[gpui::test]
    fn a_drag_pans_where_the_image_is_larger_and_ends_on_release(cx: &mut TestAppContext) {
        let (window, mut cx, _runtime) = crate::app::tests::with_rows(cx, 1);
        window.update(&mut cx, |v, _, cx| opened(v, cx, view(side(4000, 3000), side(4000, 3000)))).unwrap();
        cx.run_until_parked();
        let before = image_of(&window, &mut cx, |i| i.view);
        assert_eq!((before.x, before.y), (0., 0.), "it opens at its top left");
        let (from, to) = (point(px(600.), px(500.)), point(px(570.), px(480.)));
        cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::none());
        cx.run_until_parked();
        let moved = image_of(&window, &mut cx, |i| i.view);
        assert_eq!((moved.x, moved.y), (-30., -20.));
        cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        cx.simulate_mouse_move(from, None, Modifiers::none());
        cx.run_until_parked();
        assert_eq!(image_of(&window, &mut cx, |i| i.view), moved, "released");
    }

    #[gpui::test]
    fn a_press_in_swipe_jumps_the_handle_there(cx: &mut TestAppContext) {
        let (window, mut cx, _runtime) = crate::app::tests::with_rows(cx, 1);
        let mut img = view(side(400, 400), side(400, 400));
        img.set_mode(Mode::Swipe);
        window.update(&mut cx, |v, _, cx| opened(v, cx, img)).unwrap();
        cx.run_until_parked();
        let (left, width) = image_of(&window, &mut cx, |i| (i.stage_left.get().unwrap(), i.box_size(false).0));
        cx.simulate_mouse_down(point(px(left + width * 0.5 + 100.), px(500.)), MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        let swipe = image_of(&window, &mut cx, |i| i.swipe);
        assert!((swipe - (50. + 100. / width * 100.)).abs() < 0.01, "{swipe}");
        cx.simulate_mouse_up(point(px(left + 100.), px(500.)), MouseButton::Left, Modifiers::none());
    }

    #[gpui::test]
    fn ctrl_and_the_wheel_zoom_around_the_pointer_and_a_plain_wheel_doesnt(cx: &mut TestAppContext) {
        let (window, mut cx, _runtime) = crate::app::tests::with_rows(cx, 1);
        window.update(&mut cx, |v, _, cx| opened(v, cx, view(Side::None, side(4000, 4000)))).unwrap();
        cx.run_until_parked();
        let at = point(px(300.), px(400.));
        cx.simulate_event(ScrollEvent { position: at, delta: ScrollDelta::Pixels(point(px(0.), px(100.))), ..Default::default() });
        cx.run_until_parked();
        assert_eq!(image_of(&window, &mut cx, |i| i.zoom_pct), 100);
        cx.simulate_event(ScrollEvent { position: at, delta: ScrollDelta::Pixels(point(px(0.), px(100.))), modifiers: Modifiers::control(), ..Default::default() });
        cx.run_until_parked();
        assert_eq!(image_of(&window, &mut cx, |i| i.zoom_pct), 110);
    }

    #[test]
    fn fit_is_the_smaller_ratio_up_to_ten_times() {
        assert_eq!(fit_scale(100., 100., 500., 200.), 2.);
        assert_eq!(fit_scale(10., 10., 1000., 1000.), 10.);
        assert_eq!(fit_scale(0., 10., 100., 100.), 1.);
    }

    #[test]
    fn a_view_centres_where_the_image_fits_and_covers_where_its_larger() {
        let v = clamp_view(View { scale: 1., x: 50., y: 50. }, 100., 1000., 300., 300.);
        assert_eq!((v.x, v.y), (100., 0.));
        let v = clamp_view(View { scale: 1., x: -5000., y: -5000. }, 100., 1000., 300., 300.);
        assert_eq!((v.x, v.y), (100., -700.));
        assert_eq!(start_view(1., 1000., 1000., 300., 300.), View { scale: 1., x: 0., y: 0. });
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let v = zoom_around(View { scale: 1., x: 10., y: 20. }, 2., 110., 120.);
        assert_eq!((v.x, v.y), (-90., -80.));
    }

    #[test]
    fn the_ladder_steps_from_any_scale() {
        assert_eq!(ZOOM_STEPS[DEFAULT_STEP], 1.);
        assert_eq!(next_step_index(1., true), DEFAULT_STEP + 1);
        assert_eq!(next_step_index(1., false), DEFAULT_STEP - 1);
        assert_eq!(next_step_index(1.05, true), DEFAULT_STEP + 1);
        assert_eq!(next_step_index(1.05, false), DEFAULT_STEP);
        assert_eq!(next_step_index(10., true), ZOOM_STEPS.len() - 1);
        assert_eq!(next_step_index(0.1, false), 0);
        assert_eq!(nearest_step_index(1.04), DEFAULT_STEP);
    }

    #[test]
    fn the_swipe_handle_stays_over_the_image_and_on_screen() {
        let v = View { scale: 1., x: 100., y: 0. };
        assert_eq!(clamp_swipe(0., v, 200., 400.), 100.);
        assert_eq!(clamp_swipe(390., v, 200., 400.), 300.);
        let zoomed = View { scale: 4., x: -100., y: 0. };
        assert_eq!(clamp_swipe(0., zoomed, 200., 400.), SWIPE_EDGE);
        assert_eq!(clamp_swipe(400., zoomed, 200., 400.), 400. - SWIPE_EDGE);
    }

    #[test]
    fn match_sizes_is_on_for_a_pure_resize_and_fits_anything_else() {
        assert!(default_match(Some((100., 50.)), Some((200., 100.))));
        assert!(!default_match(Some((100., 50.)), Some((100., 50.))), "the same size");
        assert!(!default_match(Some((100., 50.)), Some((100., 100.))), "a crop");
        assert_eq!(matched_rect((100., 50.), (200., 100.)), Rect { x: 0., y: 0., w: 200., h: 100. });
        assert_eq!(matched_rect((100., 100.), (200., 100.)), Rect { x: 50., y: 0., w: 100., h: 100. });
    }

    #[test]
    fn a_new_image_opens_at_a_hundred_percent_and_fit_follows_a_resize() {
        let mut img = view(side(320, 200), side(400, 240));
        img.sync((1158., 666.), false);
        assert_eq!(img.box_size(false), (578., 666.));
        assert_eq!((img.zoom_pct, img.fit), (100, false));
        assert_eq!(img.view, View { scale: 1., x: 89., y: 213. }, "centred on the common size");
        img.apply_fit(false);
        assert!(img.fit);
        assert_eq!(img.zoom_pct, 145);
        img.sync((800., 666.), false);
        assert_eq!(img.box_size(false), (399., 666.));
        assert!((img.view.scale - 399. / 400.).abs() < 1e-3, "{}", img.view.scale);
        img.apply_step(DEFAULT_STEP, None, false, false);
        assert!(!img.fit);
    }

    #[test]
    fn modes_start_their_controls_over_and_one_side_has_no_modes() {
        let mut img = view(side(4, 4), side(4, 4));
        img.swipe = 10.;
        img.set_mode(Mode::Swipe);
        assert_eq!(img.swipe, MODE_START);
        img.opacity = 10.;
        img.set_mode(Mode::Onion);
        assert_eq!(img.opacity, MODE_START);
        assert_eq!(img.active_mode(false), Mode::Onion);
        assert_eq!(img.active_mode(true), Mode::Side, "File View shows one revision");
        assert!(img.sides(true).0.dim().is_none());
        let added = view(Side::None, side(4, 4));
        assert_eq!(added.meta(false).2, Some("added"));
        assert_eq!(added.meta(true).2, None);
    }

    #[test]
    fn the_meta_reads_as_the_toolbars() {
        let img = view(side(320, 200), side(400, 240));
        assert_eq!(img.meta(false).0, ["320×200", "400×240"]);
        assert_eq!(img.meta(false).1, ["100 B", "100 B"]);
    }

    #[test]
    fn a_wheel_adds_up_until_its_a_step() {
        let mut img = view(side(4, 4), side(4, 4));
        assert_eq!(img.wheel_step(20., false), 0);
        assert_eq!(img.wheel_step(40., false), 1);
        assert_eq!(img.wheel_step(-20., false), 0);
        assert_eq!(img.wheel_step(25., false), 0, "turning the other way starts over");
        assert_eq!(img.wheel_step(-1., true), -1, "a mouse notch is a step");
    }

    #[test]
    fn an_undecodable_side_says_so_and_a_decoded_one_is_bgra() {
        assert!(decode_raster("not base64!").is_none());
        let d = decode_raster(&png(2, 3)).unwrap();
        assert_eq!((d.w, d.h), (2., 3.));
        assert_eq!(&d.image.as_bytes(0).unwrap()[..4], [30, 20, 10, 255]);
    }

    #[test]
    fn the_difference_is_absolute_amplified_and_opaque() {
        let solid = |c: [u8; 4], w, h| {
            let rgba = image::RgbaImage::from_pixel(w, h, image::Rgba(c));
            Decoded { image: Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])), w: w as f32, h: h as f32 }
        };
        let d = difference(&solid([10, 20, 30, 255], 2, 2), &solid([12, 10, 30, 255], 2, 2), 2., 2., None, 4).unwrap();
        assert_eq!(&d.as_bytes(0).unwrap()[..4], [8, 40, 0, 255]);
        // Where the new side has no pixels the old one shows as it is, and is brightened too.
        let d = difference(&solid([10, 0, 0, 255], 2, 1), &solid([0, 0, 0, 255], 1, 1), 2., 1., None, 2).unwrap();
        assert_eq!(&d.as_bytes(0).unwrap()[4..8], [20, 0, 0, 255]);
    }

    #[test]
    fn a_crisp_copy_repeats_each_pixel() {
        let rgba = image::RgbaImage::from_raw(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        let copy = crisp(&RenderImage::new(vec![image::Frame::new(rgba)]), 2).unwrap();
        assert_eq!(copy.size(0).width.0, 4);
        assert_eq!(copy.size(0).height.0, 2);
        assert_eq!(copy.as_bytes(0).unwrap(), [1, 2, 3, 4, 1, 2, 3, 4, 5, 6, 7, 8, 5, 6, 7, 8, 1, 2, 3, 4, 1, 2, 3, 4, 5, 6, 7, 8, 5, 6, 7, 8]);
        assert_eq!(crisp_factor(1., 100, 100), None);
        assert_eq!(crisp_factor(2.5, 100, 100), Some(3));
        assert_eq!(crisp_factor(4., 2000, 100), None, "too large");
    }
}
