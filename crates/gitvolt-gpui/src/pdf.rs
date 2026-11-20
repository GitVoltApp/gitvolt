//! A PDF's pages (`ui/src/pdf/`): File View's one version, or Diff View's two side by side, each side scrolled in step
//! with the other: page n beside page n. Both share one zoom: Fit width, kept on a resize, or a step of the image zoom's
//! ladder. Only pages near the view are drawn, by a `worker` that reads the file in its own process. Hex is its toggle.

use crate::app::GitVolt;
use crate::diff::image::{ZOOM_STEPS, arrowed, nearest_step_index, next_step_index, paint_slider, render_image, slider_frac, toolbar_button};
use crate::theme;
use crate::ui::tooltip::tip;
use crate::worker::Worker;
use gpui::prelude::*;
use gpui::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// A page's size in PDF points, at zoom 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub w: f32,
    pub h: f32,
}

/// The gap between pages, and the column's padding around them.
pub const PAGE_GAP: f32 = 12.;
/// How far past the pane's edges pages are drawn ahead of the scroll.
pub const DRAW_AHEAD: f32 = 800.;
/// The pane's right padding: with its scrollbar's gutter, the overview ruler's room.
pub const PANE_PAD: f32 = 20.;
const GUTTER: f32 = 10.;
pub const TOOLBAR_H: f32 = 33.;

pub fn page_tops(sizes: &[PageSize], scale: f32) -> Vec<f32> {
    let mut tops = Vec::with_capacity(sizes.len());
    let mut y = PAGE_GAP;
    for s in sizes {
        tops.push(y);
        y += (s.h * scale).round() + PAGE_GAP;
    }
    tops
}

pub fn column_height(sizes: &[PageSize], scale: f32) -> f32 {
    sizes.iter().fold(PAGE_GAP, |y, s| y + (s.h * scale).round() + PAGE_GAP)
}

/// The page at `y` down the column: the last one starting at or above it.
pub fn page_at(tops: &[f32], y: f32) -> usize {
    tops.partition_point(|t| *t <= y).saturating_sub(1)
}

/// The page the reader's on: the one at the pane's top third.
pub fn current_page(tops: &[f32], scroll: f32, pane_h: f32) -> usize {
    if tops.is_empty() { 0 } else { page_at(tops, scroll + pane_h / 3.) }
}

/// Where the other side scrolls so its page n lines up with this side's, at the same point down the page.
pub fn synced_scroll_top(from: &[PageSize], from_scale: f32, to: &[PageSize], to_scale: f32, scroll: f32) -> f32 {
    if from.is_empty() || to.is_empty() {
        return 0.;
    }
    let (ft, tt) = (page_tops(from, from_scale), page_tops(to, to_scale));
    let i = page_at(&ft, scroll + PAGE_GAP);
    if i >= to.len() {
        return column_height(to, to_scale);
    }
    let (fh, th) = ((from[i].h * from_scale).round() + PAGE_GAP, (to[i].h * to_scale).round() + PAGE_GAP);
    let fraction = ((scroll - (ft[i] - PAGE_GAP)) / fh).clamp(0., 1.);
    tt[i] - PAGE_GAP + fraction * th
}

/// The pages within `DRAW_AHEAD` of the pane's view.
pub fn visible_pages(sizes: &[PageSize], scale: f32, scroll: f32, pane_h: f32) -> Vec<usize> {
    let tops = page_tops(sizes, scale);
    let (from, to) = (scroll - DRAW_AHEAD, scroll + pane_h + DRAW_AHEAD);
    let mut out = Vec::new();
    let mut i = page_at(&tops, from.max(0.));
    while i < tops.len() && tops[i] <= to {
        if tops[i] + sizes[i].h * scale >= from {
            out.push(i);
        }
        i += 1;
    }
    out
}

/// The zoom that fits the widest page, within the ladder's ends.
pub fn fit_width_scale(sizes: &[PageSize], pane_w: f32) -> f32 {
    let widest = sizes.iter().fold(0_f32, |a, s| a.max(s.w));
    if widest <= 0. || pane_w <= 0. {
        return 1.;
    }
    ((pane_w - 2. * PAGE_GAP) / widest).clamp(ZOOM_STEPS[0], ZOOM_STEPS[ZOOM_STEPS.len() - 1])
}

enum Doc {
    Loading,
    Error(SharedString),
    Ready { sizes: Vec<PageSize>, worker: Arc<Mutex<Worker>> },
}

pub struct Side {
    doc: Doc,
    size: u64,
    /// Drawn pages: the page, the scale it was drawn at, and the picture.
    pages: HashMap<usize, (f32, Arc<RenderImage>)>,
    busy: bool,
    failed: HashSet<(usize, u32)>,
    scroll: f32,
    hscroll: f32,
}

impl Side {
    fn sizes(&self) -> &[PageSize] {
        if let Doc::Ready { sizes, .. } = &self.doc { sizes } else { &[] }
    }

    fn count(&self) -> Option<usize> {
        if let Doc::Ready { sizes, .. } = &self.doc { Some(sizes.len()) } else { None }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum PdfDrag {
    None,
    Slider,
    Ruler { old: bool, grab: f32 },
    Bar { old: bool, x: f32, start: f32, ratio: f32 },
}

pub struct PdfView {
    pub old: Option<Side>,
    pub new: Option<Side>,
    /// File View: the one version, the new side's if there is one.
    pub file: bool,
    /// What the core calls the file: a PDF of plain text can be blamed, and says its encoding.
    pub binary: bool,
    pub encoding: SharedString,
    /// `None`: Fit width.
    pub fixed: Option<f32>,
    pub source_on: bool,
    pane_w: f32,
    pane_h: f32,
    page: usize,
    wheel: f32,
    drag: PdfDrag,
    bounds: Rc<RefCell<(Bounds<Pixels>, Bounds<Pixels>)>>,
    slider_at: Rc<std::cell::Cell<Option<(f32, f32)>>>,
    retired: Rc<RefCell<Vec<Arc<RenderImage>>>>,
}

impl Drop for PdfView {
    fn drop(&mut self) {
        let mut retired = self.retired.borrow_mut();
        for side in [self.old.as_mut(), self.new.as_mut()].into_iter().flatten() {
            retired.extend(side.pages.drain().map(|(_, (_, image))| image));
        }
    }
}

fn format_bytes(n: u64) -> String {
    crate::format::format_bytes(Some(n))
}

fn pages_text(n: Option<usize>) -> String {
    match n {
        None => "?".to_string(),
        Some(1) => "1 page".to_string(),
        Some(n) => format!("{n} pages"),
    }
}

impl PdfView {
    pub fn new(old: Option<u64>, new: Option<u64>, file: bool, retired: Rc<RefCell<Vec<Arc<RenderImage>>>>) -> Self {
        let side = |s: u64| Side { doc: Doc::Loading, size: s, pages: HashMap::new(), busy: false, failed: HashSet::new(), scroll: 0., hscroll: 0. };
        Self { old: old.map(side), new: new.map(side), file, binary: true, encoding: SharedString::default(), fixed: None, source_on: false, pane_w: 0., pane_h: 0., page: 0, wheel: 0., drag: PdfDrag::None, bounds: Rc::default(), slider_at: Rc::default(), retired }
    }

    pub fn both(&self) -> bool {
        self.side(true).is_some() && self.side(false).is_some()
    }

    /// `(added)` or `(deleted)`, in Diff View.
    fn single(&self) -> Option<&'static str> {
        if self.file {
            None
        } else if self.old.is_none() {
            Some("added")
        } else if self.new.is_none() {
            Some("deleted")
        } else {
            None
        }
    }

    fn shown(&self) -> impl Iterator<Item = &Side> {
        [self.side(true), self.side(false)].into_iter().flatten()
    }

    fn all_sizes(&self) -> Vec<PageSize> {
        self.shown().flat_map(|s| s.sizes().iter().copied()).collect()
    }

    pub fn scale(&self) -> f32 {
        self.fixed.unwrap_or_else(|| fit_width_scale(&self.all_sizes(), self.pane_w - PANE_PAD - GUTTER))
    }

    fn side(&self, old: bool) -> Option<&Side> {
        if !old {
            self.new.as_ref()
        } else if self.file && self.new.is_some() {
            None
        } else {
            self.old.as_ref()
        }
    }

    fn side_mut(&mut self, old: bool) -> Option<&mut Side> {
        if !old {
            self.new.as_mut()
        } else if self.file && self.new.is_some() {
            None
        } else {
            self.old.as_mut()
        }
    }

    /// Between File View and Diff View: the fit changes with the sides shown.
    pub fn set_file(&mut self, file: bool) {
        if self.file != file {
            let was = self.scale();
            self.file = file;
            let now = self.scale();
            self.rescale(was, now);
        }
    }

    fn max_scroll(&self, old: bool, scale: f32) -> f32 {
        self.side(old).map_or(0., |s| (column_height(s.sizes(), scale) - self.pane_h).max(0.))
    }

    fn max_hscroll(&self, old: bool, scale: f32) -> f32 {
        self.side(old).map_or(0., |s| (s.sizes().iter().fold(0_f32, |a, p| a.max(p.w)) * scale + 2. * PAGE_GAP - (self.pane_w - PANE_PAD - GUTTER)).max(0.))
    }

    /// The pane the user scrolls leads; the other follows, at the same page.
    pub fn scroll_lead(&mut self, old: bool, y: f32) {
        let scale = self.scale();
        let y = y.clamp(0., self.max_scroll(old, scale));
        let Some(from) = self.side(old).map(|s| s.sizes().to_vec()) else { return };
        if let Some(s) = self.side_mut(old) {
            s.scroll = y;
        }
        self.page = current_page(&page_tops(&from, scale), y, self.pane_h);
        let to_old = !old;
        if let Some(to) = self.side(to_old).map(|s| s.sizes().to_vec()) {
            let top = synced_scroll_top(&from, scale, &to, scale, y);
            let max = self.max_scroll(to_old, scale);
            if let Some(s) = self.side_mut(to_old) {
                s.scroll = top.clamp(0., max);
            }
        }
    }

    pub fn scroll_by(&mut self, dy: f32) {
        let lead = self.new.is_none();
        let now = self.side(lead).map_or(0., |s| s.scroll);
        self.scroll_lead(lead, now + dy);
    }

    pub fn view_height(&self) -> f32 {
        self.pane_h
    }

    /// What a script checks.
    pub fn dump(&self) -> serde_json::Value {
        let side = |s: Option<&Side>| {
            s.map(|s| {
                let mut drawn: Vec<usize> = s.pages.keys().copied().collect();
                drawn.sort_unstable();
                let error = if let Doc::Error(e) = &s.doc { Some(e.to_string()) } else { None };
                serde_json::json!({ "pages": s.count(), "drawn": drawn, "scroll": s.scroll, "error": error })
            })
        };
        serde_json::json!({ "scale": self.scale(), "fit": self.fixed.is_none(), "page": self.page, "meta": self.meta().2, "hex": self.source_on, "old": side(self.old.as_ref()), "new": side(self.new.as_ref()) })
    }

    pub fn pane_size(&self) -> (f32, f32) {
        (self.pane_w, self.pane_h)
    }

    /// A zoom keeps the page being read where it is.
    pub fn set_fixed(&mut self, fixed: Option<f32>) {
        let was = self.scale();
        self.fixed = fixed;
        let now = self.scale();
        self.rescale(was, now);
    }

    fn rescale(&mut self, was: f32, now: f32) {
        if was == now {
            return;
        }
        for old in [true, false] {
            let max = self.max_scroll(old, now);
            if let Some(s) = self.side_mut(old) {
                let sizes = s.sizes().to_vec();
                s.scroll = synced_scroll_top(&sizes, was, &sizes, now, s.scroll).clamp(0., max);
            }
        }
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        if self.pane_w == w && self.pane_h == h {
            return;
        }
        let was = self.scale();
        self.pane_w = w;
        self.pane_h = h;
        let now = self.scale();
        self.rescale(was, now);
    }

    fn meta(&self) -> (Vec<String>, Vec<String>, String) {
        let counts: Vec<String> = self.shown().map(|s| pages_text(s.count())).collect();
        let sizes: Vec<String> = self.shown().map(|s| format_bytes(s.size)).collect();
        let shown = self.side(false).or(self.side(true)).and_then(Side::count);
        let text = format!("{}{} · {}{}", if shown.is_some() { format!("Page {} · ", self.page + 1) } else { String::new() }, counts.join(" → "), sizes.join(" → "), self.single().map_or(String::new(), |s| format!(" ({s})")));
        (counts, sizes, text)
    }
}

impl GitVolt {
    fn with_pdf(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut PdfView)) {
        self.with_diff(cx, |d| {
            if let Some(p) = d.pdf.as_mut() {
                f(p);
            }
        });
        self.draw_pdf_pages(cx);
    }

    /// Reads the sides' bytes into workers of their own, and their pages' sizes.
    pub fn open_pdf_sides(&mut self, old: Option<Arc<Vec<u8>>>, new: Option<Arc<Vec<u8>>>, generation: u64, cx: &mut Context<Self>) {
        let repo = self.tabs.get(self.active).map_or(0, |t| t.id);
        for (is_old, bytes) in [(true, old), (false, new)] {
            let Some(bytes) = bytes else { continue };
            let job = self.rt.spawn_blocking(move || {
                let mut worker = Worker::spawn().map_err(|_| "This PDF couldn't be opened".to_string())?;
                let sizes = worker.open_pdf(&bytes)?;
                if sizes.is_empty() {
                    return Err("This PDF has no pages".to_string());
                }
                Ok::<_, String>((worker, sizes))
            });
            cx.spawn(async move |this, cx| {
                let result = job.await;
                this.update(cx, |this, cx| {
                    let Some(diff) = this.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.diff.as_mut()).filter(|d| d.generation == generation) else { return };
                    let Some(pdf) = diff.pdf.as_mut() else { return };
                    let was = pdf.scale();
                    let Some(side) = pdf.side_mut(is_old) else { return };
                    side.doc = match result {
                        Ok(Ok((worker, sizes))) => Doc::Ready { sizes: sizes.into_iter().map(|(w, h)| PageSize { w, h }).collect(), worker: Arc::new(Mutex::new(worker)) },
                        Ok(Err(why)) => Doc::Error(why.into()),
                        Err(_) => Doc::Error("This PDF couldn't be opened".into()),
                    };
                    let now = pdf.scale();
                    pdf.rescale(was, now);
                    this.draw_pdf_pages(cx);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    /// The next page near each side's view that isn't drawn at this zoom, one at a time a side. Pages far off go.
    pub fn draw_pdf_pages(&mut self, cx: &mut Context<Self>) {
        let dpr = 1.;
        let repo = self.tabs.get(self.active).map_or(0, |t| t.id);
        let rt = self.rt.clone();
        let Some(diff) = self.tabs.get_mut(self.active).and_then(|t| t.diff.as_mut()) else { return };
        let generation = diff.generation;
        let Some(pdf) = diff.pdf.as_mut() else { return };
        if pdf.source_on {
            return;
        }
        let (scale, pane_h, retired) = (pdf.scale(), pdf.pane_h, pdf.retired.clone());
        for is_old in [true, false] {
            let Some(side) = pdf.side_mut(is_old) else { continue };
            let Doc::Ready { sizes, worker } = &side.doc else { continue };
            let near = visible_pages(sizes, scale, side.scroll, pane_h);
            let worker = worker.clone();
            let keep: HashSet<usize> = near.iter().copied().collect();
            let gone: Vec<usize> = side.pages.keys().filter(|p| !keep.contains(p)).copied().collect();
            for p in gone {
                if let Some((_, image)) = side.pages.remove(&p) {
                    retired.borrow_mut().push(image);
                }
            }
            if side.busy {
                continue;
            }
            let bits = scale.to_bits();
            let next = near.iter().copied().find(|p| side.pages.get(p).is_none_or(|(s, _)| *s != scale) && !side.failed.contains(&(*p, bits)));
            let Some(page) = next else { continue };
            side.busy = true;
            let job = rt.spawn_blocking(move || {
                let mut worker = worker.lock().map_err(|_| None)?;
                // Past its time or its memory the worker's gone, and with it the document.
                let drawn = worker.render_page(page as u32, scale * dpr).map_err(|why| worker.is_dead().then_some(why))?;
                let rgba = image::RgbaImage::from_raw(drawn.w, drawn.h, drawn.rgba).ok_or(None)?;
                Ok::<_, Option<String>>(render_image(rgba))
            });
            cx.spawn(async move |this, cx| {
                let result = job.await;
                this.update(cx, |this, cx| {
                    let Some(diff) = this.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.diff.as_mut()).filter(|d| d.generation == generation) else { return };
                    let Some(pdf) = diff.pdf.as_mut() else { return };
                    let retired = pdf.retired.clone();
                    let Some(side) = pdf.side_mut(is_old) else { return };
                    side.busy = false;
                    match result {
                        Ok(Ok(image)) => {
                            if let Some((_, old)) = side.pages.insert(page, (scale, Arc::new(image))) {
                                retired.borrow_mut().push(old);
                            }
                        }
                        Ok(Err(Some(why))) => side.doc = Doc::Error(format!("This PDF couldn't be drawn: {why}").into()),
                        // A page that can't be drawn stays blank paper, as the web's.
                        _ => {
                            side.failed.insert((page, bits));
                        }
                    }
                    this.draw_pdf_pages(cx);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    /// Larger and Smaller text, over a PDF: its zoom, a step of the ladder. 0: Fit width. `false`: no PDF's shown.
    pub fn pdf_zoom(&mut self, dir: i32, cx: &mut Context<Self>) -> bool {
        let shown = self.tabs.get(self.active).and_then(|t| t.diff.as_ref()).and_then(|d| d.pdf.as_ref()).is_some_and(|p| !p.source_on);
        if shown {
            self.with_pdf(cx, |p| {
                let next = (dir != 0).then(|| ZOOM_STEPS[next_step_index(p.scale(), dir > 0)]);
                p.set_fixed(next);
            });
        }
        shown
    }

    /// Hex, on a PDF or on audio, video or a font.
    pub fn toggle_rich_hex(&mut self, cx: &mut Context<Self>) {
        let mut load = false;
        self.with_diff(cx, |d| {
            let on = if let Some(p) = d.pdf.as_mut() {
                p.source_on = !p.source_on;
                p.source_on
            } else if let Some(m) = d.media.as_mut() {
                m.source_on = !m.source_on;
                m.source_on
            } else {
                return;
            };
            load = on && d.hex.is_none();
            d.layout_again();
        });
        if load {
            self.read_hex(cx);
        }
        self.draw_pdf_pages(cx);
    }

    pub fn pdf_scroll_to(&mut self, y: f32, cx: &mut Context<Self>) {
        self.with_pdf(cx, |p| {
            let lead = p.new.is_none();
            p.scroll_lead(lead, y);
        });
    }

    fn pdf_slider(&mut self, x: f32, cx: &mut Context<Self>) {
        let Some(track) = self.tabs.get(self.active).and_then(|t| t.diff.as_ref()).and_then(|d| d.pdf.as_ref()).and_then(|p| p.slider_at.get()) else { return };
        let i = (slider_frac(x, track.0, track.1) * (ZOOM_STEPS.len() - 1) as f32).round() as usize;
        self.with_pdf(cx, |p| p.set_fixed(Some(ZOOM_STEPS[i])));
    }
}

fn pane(is_old: bool, label: Option<&'static str>, pdf: &PdfView, cx: &mut Context<GitVolt>) -> AnyElement {
    let Some(side) = pdf.side(is_old) else { return div().into_any_element() };
    let message: Option<SharedString> = match &side.doc {
        Doc::Loading => Some("Loading…".into()),
        Doc::Error(e) => Some(e.clone()),
        Doc::Ready { .. } => None,
    };
    let scale = pdf.scale();
    let sizes: Vec<PageSize> = side.sizes().to_vec();
    let (scroll, hscroll) = (side.scroll, side.hscroll);
    let pages: Vec<(usize, Arc<RenderImage>)> = side.pages.iter().map(|(k, v)| (*k, v.1.clone())).collect();
    let bounds = pdf.bounds.clone();
    let max = pdf.max_scroll(is_old, scale);
    let max_h = pdf.max_hscroll(is_old, scale);
    let body = canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            {
                let mut cell = bounds.borrow_mut();
                if is_old {
                    cell.0 = b;
                } else {
                    cell.1 = b;
                }
            }
            window.with_content_mask(Some(ContentMask { bounds: b }), |window| {
                let tops = page_tops(&sizes, scale);
                let col_w = (b.size.width.as_f32() - PANE_PAD - GUTTER).max(sizes.iter().fold(0_f32, |a, s| a.max(s.w)) * scale + 2. * PAGE_GAP);
                let centre = b.origin.x.as_f32() + col_w / 2. - hscroll;
                for (i, s) in sizes.iter().enumerate() {
                    let y = b.origin.y.as_f32() + tops[i] - scroll;
                    let (w, h) = ((s.w * scale).round(), (s.h * scale).round());
                    if y > (b.origin.y + b.size.height).as_f32() || y + h < b.origin.y.as_f32() {
                        continue;
                    }
                    let at = Bounds::new(point(px(centre - w / 2.), px(y)), size(px(w), px(h)));
                    // The paper's white in every theme.
                    window.paint_quad(fill(at.dilate(px(1.)), theme::image_frame_border()));
                    window.paint_quad(fill(at, gpui::white()));
                    if let Some((_, image)) = pages.iter().find(|(p, _)| *p == i) {
                        window.paint_image(b, at, Corners::default(), image.clone(), 0, false).ok();
                    }
                }
                // The overview ruler's slider stands in for the scrollbar.
                let total = column_height(&sizes, scale);
                let view = b.size.height.as_f32();
                if total > view {
                    let thumb = (view * view / total).max(20.).round();
                    let top = (scroll / max.max(1.)) * (view - thumb);
                    let x = (b.origin.x + b.size.width).as_f32() - PANE_PAD - GUTTER;
                    window.paint_quad(fill(Bounds::new(point(px(x), b.origin.y + px(top)), size(px(PANE_PAD + GUTTER), px(thumb))), theme::scroll_thumb()));
                }
                if max_h > 0. {
                    let track = b.size.width.as_f32() - PANE_PAD - GUTTER;
                    let thumb = (track * track / (track + max_h)).max(24.);
                    let x = b.origin.x.as_f32() + (track - thumb) * (hscroll / max_h);
                    let y = (b.origin.y + b.size.height).as_f32() - 8.;
                    window.paint_quad(fill(Bounds::new(point(px(x), px(y)), size(px(thumb), px(6.))), theme::scroll_thumb()).corner_radii(px(3.)));
                }
            });
        },
    )
    .absolute()
    .size_full();
    div()
        .id(if is_old { "pdf-pane-old" } else { "pdf-pane-new" })
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_hidden()
        .bg(theme::app_bg())
        .on_scroll(cx.listener(move |this, e: &ScrollEvent, window, cx| {
            let delta = e.delta.pixel_delta(window.line_height());
            if e.modifiers.control {
                let up = delta.y.as_f32() > 0.;
                this.with_pdf(cx, |p| {
                    p.wheel += delta.y.as_f32();
                    if p.wheel.abs() >= 40. || matches!(e.delta, ScrollDelta::Lines(_)) {
                        p.wheel = 0.;
                        p.set_fixed(Some(ZOOM_STEPS[next_step_index(p.scale(), up)]));
                    }
                });
                cx.stop_propagation();
                return;
            }
            let (dx, dy) = (delta.x, delta.y);
            this.with_pdf(cx, |p| {
                let scale = p.scale();
                let now = p.side(is_old).map_or(0., |s| s.scroll);
                p.scroll_lead(is_old, now - dy.as_f32());
                let max_h = p.max_hscroll(is_old, scale);
                for old in [true, false] {
                    if let Some(s) = p.side_mut(old) {
                        s.hscroll = (s.hscroll - dx.as_f32()).clamp(0., max_h);
                    }
                }
            });
            cx.stop_propagation();
        }))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            let Some(pdf) = this.tabs.get_mut(this.active).and_then(|t| t.diff.as_mut()).and_then(|d| d.pdf.as_mut()) else { return };
            let b = if is_old { pdf.bounds.borrow().0 } else { pdf.bounds.borrow().1 };
            let (x, y) = ((e.position.x - b.origin.x).as_f32(), (e.position.y - b.origin.y).as_f32());
            let (w, view) = (b.size.width.as_f32(), b.size.height.as_f32());
            let scale = pdf.scale();
            if x >= w - PANE_PAD - GUTTER {
                let total = pdf.side(is_old).map_or(0., |s| column_height(s.sizes(), scale));
                if total > view {
                    let thumb = (view * view / total).max(20.).round();
                    let top = (pdf.side(is_old).map_or(0., |s| s.scroll) / pdf.max_scroll(is_old, scale).max(1.)) * (view - thumb);
                    let grab = if y >= top && y < top + thumb { y - top } else { thumb / 2. };
                    pdf.drag = PdfDrag::Ruler { old: is_old, grab };
                    let share = ((y - grab) / (view - thumb).max(1.)).clamp(0., 1.);
                    let to = share * pdf.max_scroll(is_old, scale);
                    pdf.scroll_lead(is_old, to);
                }
            } else if y >= view - 10. {
                let max_h = pdf.max_hscroll(is_old, scale);
                if max_h > 0. {
                    let track = w - PANE_PAD - GUTTER;
                    let thumb = (track * track / (track + max_h)).max(24.);
                    pdf.drag = PdfDrag::Bar { old: is_old, x: e.position.x.as_f32(), start: pdf.side(is_old).map_or(0., |s| s.hscroll), ratio: max_h / (track - thumb).max(1.) };
                }
            }
            this.draw_pdf_pages(cx);
            cx.notify();
        }))
        .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
            let Some(pdf) = this.tabs.get_mut(this.active).and_then(|t| t.diff.as_mut()).and_then(|d| d.pdf.as_mut()) else { return };
            if e.pressed_button != Some(MouseButton::Left) {
                return;
            }
            match pdf.drag {
                PdfDrag::Ruler { old, grab } => {
                    let b = if old { pdf.bounds.borrow().0 } else { pdf.bounds.borrow().1 };
                    let (y, view, scale) = ((e.position.y - b.origin.y).as_f32(), b.size.height.as_f32(), pdf.scale());
                    let total = pdf.side(old).map_or(0., |s| column_height(s.sizes(), scale));
                    let thumb = (view * view / total.max(1.)).max(20.).round();
                    let to = ((y - grab) / (view - thumb).max(1.)).clamp(0., 1.) * pdf.max_scroll(old, scale);
                    pdf.scroll_lead(old, to);
                    this.draw_pdf_pages(cx);
                    cx.notify();
                }
                PdfDrag::Bar { old, x, start, ratio } => {
                    let scale = pdf.scale();
                    let max_h = pdf.max_hscroll(old, scale);
                    let to = (start + (e.position.x.as_f32() - x) * ratio).clamp(0., max_h);
                    for side in [true, false] {
                        if let Some(s) = pdf.side_mut(side) {
                            s.hscroll = to;
                        }
                    }
                    cx.notify();
                }
                _ => {}
            }
        }))
        .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| {
            if let Some(pdf) = this.tabs.get_mut(this.active).and_then(|t| t.diff.as_mut()).and_then(|d| d.pdf.as_mut()) {
                pdf.drag = PdfDrag::None;
            }
        }))
        .child(body)
        .when_some(message, |d, m| d.child(div().absolute().size_full().flex().items_center().justify_center().text_color(theme::text_secondary()).child(m)))
        .when_some(label, |d, l| d.child(div().absolute().top(px(8.)).left_0().right_0().flex().justify_center().child(div().px(px(8.)).py(px(2.)).rounded(px(3.)).bg(rgba(0x0000008c)).text_color(rgba(0xffffffd9)).child(l.to_uppercase()))))
        .into_any_element()
}

fn toolbar(pdf: &PdfView, cx: &mut Context<GitVolt>) -> AnyElement {
    let scale = pdf.scale();
    let percent = (scale * 100.).round() as u32;
    let frac = nearest_step_index(scale) as f32 / (ZOOM_STEPS.len() - 1) as f32;
    let (counts, sizes, text) = pdf.meta();
    let shown = pdf.side(false).or(pdf.side(true)).and_then(Side::count);
    let track = pdf.slider_at.clone();
    let source_on = pdf.source_on;
    let fit = pdf.fixed.is_none();
    div()
        .flex()
        .flex_none()
        .h(px(TOOLBAR_H))
        .items_center()
        .gap(px(10.))
        .px(px(8.))
        .border_b_1()
        .border_color(theme::section_border())
        .child(
            div()
                .id("pdf-hex")
                .px(px(8.))
                .py(px(2.))
                .border_1()
                .border_color(theme::section_border())
                .rounded(px(4.))
                .text_color(if source_on { theme::text_selected() } else { theme::text_secondary() })
                .when(source_on, |d| d.bg(theme::selected_row()))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_rich_hex(cx)))
                .child("Hex"),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .child(toolbar_button("pdf-fit", "Fit width", fit).on_click(cx.listener(|this, _, _, cx| this.with_pdf(cx, |p| p.set_fixed(None)))))
                .child(
                    div()
                        .id("pdf-zoom")
                        .w(px(129.))
                        .h(px(16.))
                        .flex_none()
                        .when(source_on, |d| d.opacity(0.4))
                        .when(!source_on, |d| {
                            d.on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| {
                                if e.click_count == 2 {
                                    this.with_pdf(cx, |p| p.set_fixed(Some(1.)));
                                    return;
                                }
                                if let Some(p) = this.tabs.get_mut(this.active).and_then(|t| t.diff.as_mut()).and_then(|d| d.pdf.as_mut()) {
                                    p.drag = PdfDrag::Slider;
                                }
                                this.pdf_slider(e.position.x.as_f32(), cx);
                            }))
                            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                                let dragging = this.tabs.get(this.active).and_then(|t| t.diff.as_ref()).and_then(|d| d.pdf.as_ref()).is_some_and(|p| p.drag == PdfDrag::Slider);
                                if dragging && e.pressed_button == Some(MouseButton::Left) {
                                    this.pdf_slider(e.position.x.as_f32(), cx);
                                }
                            }))
                            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| {
                                if let Some(p) = this.tabs.get_mut(this.active).and_then(|t| t.diff.as_mut()).and_then(|d| d.pdf.as_mut()) {
                                    p.drag = PdfDrag::None;
                                }
                            }))
                        })
                        .child(canvas(|_, _, _| {}, move |b, _, window, _| {
                            track.set(Some((b.origin.x.as_f32(), b.size.width.as_f32())));
                            paint_slider(b, frac, window);
                        }).size_full()),
                )
                .child(toolbar_button("pdf-pct", format!("{percent}%"), false).min_w(px(51.)).flex().justify_center()),
        )
        .child(tip(
            text,
            div()
                .id("pdf-meta")
                .flex()
                .ml(px(6.))
                .min_w_0()
                .overflow_hidden()
                .items_center()
                .gap(px(6.))
                .text_color(theme::text_disabled())
                .when(shown.is_some(), |d| d.child(div().flex().flex_none().gap(px(4.)).mr(px(-3.)).child(format!("Page {}", pdf.page + 1)).child("·")))
                .child(arrowed(counts))
                .child("·")
                .child(arrowed(sizes))
                .when_some(pdf.single(), |d, s| d.child(div().ml(px(-2.5)).child(format!("({s})")))),
        ))
        .into_any_element()
}

/// A PDF, in the diff panel's body.
pub fn pdf_body(pdf: &PdfView, source: Option<AnyElement>, cx: &mut Context<GitVolt>) -> AnyElement {
    let both = pdf.both();
    let head = toolbar(pdf, cx);
    let stage: AnyElement = match source {
        Some(s) if pdf.source_on => div().flex().flex_col().flex_1().min_h_0().child(s).into_any_element(),
        _ => div()
            .id("pdf-stage")
            .relative()
            .flex()
            .gap(px(2.))
            .flex_1()
            .min_h_0()
            .when(pdf.side(true).is_some(), |d| d.child(pane(true, both.then_some("Old"), pdf, cx)))
            .when(pdf.side(false).is_some(), |d| d.child(pane(false, both.then_some("New"), pdf, cx)))
            .when(both, |d| d.child(div().absolute().top_0().bottom_0().left(relative(0.5)).w(px(1.)).bg(theme::section_border())))
            .into_any_element(),
    };
    div().flex().flex_col().flex_1().min_w_0().min_h_0().child(head).child(stage).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn sizes() -> Vec<PageSize> {
        vec![PageSize { w: 100., h: 200. }, PageSize { w: 120., h: 100. }, PageSize { w: 100., h: 300. }]
    }

    #[test]
    fn pages_lie_in_a_column_with_gaps() {
        assert_eq!(page_tops(&sizes(), 1.), [12., 224., 336.]);
        assert_eq!(column_height(&sizes(), 1.), 648.);
        assert_eq!(page_at(&[12., 224., 336.], 0.), 0);
        assert_eq!(page_at(&[12., 224., 336.], 224.), 1);
        assert_eq!(current_page(&[], 10., 100.), 0);
        assert_eq!(current_page(&[12., 224., 336.], 200., 90.), 1);
    }

    #[test]
    fn the_other_side_follows_at_the_same_page_and_stops_at_its_end() {
        let (a, b) = (sizes(), vec![PageSize { w: 100., h: 400. }]);
        assert_eq!(synced_scroll_top(&a, 1., &a, 1., 100.), 100.);
        assert_eq!(synced_scroll_top(&a, 1., &b, 1., 0.), 0.);
        // Halfway down page 1: halfway down the other's.
        let half = synced_scroll_top(&a, 1., &b, 1., 106.);
        assert_eq!(half, 206.);
        assert_eq!(synced_scroll_top(&a, 1., &b, 1., 400.), column_height(&b, 1.), "a side with fewer pages stops at its end");
        assert_eq!(synced_scroll_top(&[], 1., &a, 1., 5.), 0.);
    }

    #[test]
    fn only_pages_near_the_view_are_drawn() {
        let many = vec![PageSize { w: 100., h: 1000. }; 50];
        assert_eq!(visible_pages(&many, 1., 0., 500.), [0, 1]);
        assert_eq!(visible_pages(&many, 1., 10_000., 500.), [9, 10, 11]);
    }

    #[test]
    fn fit_width_stays_on_the_ladder() {
        assert_eq!(fit_width_scale(&sizes(), 144.), 1.);
        assert_eq!(fit_width_scale(&[], 100.), 1.);
        assert_eq!(fit_width_scale(&sizes(), 1.), ZOOM_STEPS[0]);
        assert_eq!(fit_width_scale(&sizes(), 1e6), ZOOM_STEPS[ZOOM_STEPS.len() - 1]);
    }

    #[test]
    fn a_zoom_keeps_the_page_being_read_and_scrolling_leads() {
        let mut p = PdfView::new(Some(1), Some(1), false, Rc::default());
        p.pane_w = 200.;
        p.pane_h = 100.;
        p.old.as_mut().unwrap().doc = Doc::Error("x".into());
        p.fixed = Some(1.);
        // With no pages, nothing moves and nothing panics.
        p.scroll_lead(true, 50.);
        assert_eq!(p.old.as_ref().unwrap().scroll, 0.);
        assert_eq!(pages_text(Some(1)), "1 page");
        assert_eq!(pages_text(None), "?");
        assert!(p.both() && p.single().is_none());
        p.set_file(true);
        assert!(!p.both() && p.side(true).is_none(), "File View shows the new side");
        let deleted = PdfView::new(Some(1), None, true, Rc::default());
        assert!(deleted.side(true).is_some(), "or the old one, if that's all there is");
        assert_eq!(PdfView::new(None, Some(1), false, Rc::default()).single(), Some("added"));
    }
}
