//! `MdMermaid`: a diagram drawn by `mermaid-rs-renderer`, in the `worker` process, and shown as a picture, so nothing in it
//! can run or load. The layout and style are that crate's, not mermaid.js's. Until it's drawn the source shows; on an
//! error it stays, with the error under it, as the web's.

use crate::worker::Worker;
use gpui::{RenderImage, SvgRenderer};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A document's diagrams past this show their source. The web draws what's near the view; this draws them all, in turn.
pub const MAX_DIAGRAMS: usize = 256;
/// A picture's pixels, as a page's canvas.
const MAX_PIXELS: f32 = 16_777_216.;
/// How far a small diagram is scaled up to its column, as the web's `<img>`.
const MAX_SCALE: f32 = 4.;
/// How much of an SVG's start its `viewBox` can be in.
const HEAD: usize = 4096;

#[derive(Clone)]
pub struct Drawn {
    pub image: Arc<RenderImage>,
    pub w: f32,
    pub h: f32,
}

/// One document's worker. One diagram that took too long or too much means the file's hostile or past drawing: it's
/// killed, and the rest stay as their source rather than each taking as long again.
#[derive(Default)]
pub struct Diagrams {
    worker: Mutex<Option<Worker>>,
    gave_up: AtomicBool,
}

impl Diagrams {
    /// Off the UI thread. `Err`: what the error line says.
    pub fn draw(&self, source: &str, dark: bool, width: f32, svg: &SvgRenderer) -> Result<Drawn, String> {
        if self.gave_up.load(Ordering::Relaxed) {
            return Err("an earlier diagram took too long".to_string());
        }
        let text = {
            let mut worker = self.worker.lock().map_err(|_| "the worker's gone".to_string())?;
            if worker.is_none() {
                *worker = Some(Worker::spawn().map_err(|e| e.to_string())?);
            }
            let w = worker.as_mut().ok_or("the worker's gone")?;
            let drawn = w.mermaid(source, dark);
            if w.is_dead() {
                *worker = None;
                self.gave_up.store(true, Ordering::Relaxed);
            }
            drawn?
        };
        let (nw, nh) = svg_size(&text).ok_or("it isn't an SVG")?;
        let scale = fit_scale(nw, nh, width).ok_or("it's too large to draw")?;
        let image = svg.render_single_frame(text.as_bytes(), scale).map_err(|e| e.to_string())?;
        Ok(Drawn { image, w: nw * scale, h: nh * scale })
    }
}

/// The scale for its column: never past a canvas. `None`: a diagram too large at any useful size.
pub fn fit_scale(w: f32, h: f32, column: f32) -> Option<f32> {
    if !(w > 0. && h > 0. && w.is_finite() && h.is_finite()) {
        return None;
    }
    // `render_single_frame` draws at twice its scale, for smoothness.
    let limit = (MAX_PIXELS / (w * h * 4.)).sqrt();
    let wanted = if column > 0. { (column / w).clamp(0.05, MAX_SCALE) } else { 1. };
    let scale = wanted.min(limit);
    (scale >= 0.05).then_some(scale)
}

/// The root's `viewBox`: its size, by what's written, before anything allocates for it.
pub fn svg_size(svg: &str) -> Option<(f32, f32)> {
    let mut end = svg.len().min(HEAD);
    while !svg.is_char_boundary(end) {
        end -= 1;
    }
    let head = &svg[..end];
    let root = &head[head.find("<svg")?..];
    let root = &root[..root.find('>').unwrap_or(root.len())];
    let at = root.find("viewBox=\"")? + 9;
    let box_ = &root[at..at + root[at..].find('"')?];
    let v: Vec<f32> = box_.split([' ', ',']).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
    (v.len() == 4).then(|| (v[2], v[3]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_size_comes_from_the_roots_view_box() {
        assert_eq!(svg_size(r#"<?xml?><svg xmlns="x" viewBox="0 0 120.5 40"><g/></svg>"#), Some((120.5, 40.)));
        assert_eq!(svg_size(r#"<svg viewBox="0,0,8,9">"#), Some((8., 9.)));
        assert_eq!(svg_size(r#"<svg><rect viewBox="0 0 1 1"/></svg>"#), None, "not the root's");
        assert_eq!(svg_size("no svg"), None);
    }

    #[test]
    fn a_diagram_scales_to_its_column_and_no_further_than_a_canvas() {
        assert_eq!(fit_scale(440., 200., 880.), Some(2.));
        assert_eq!(fit_scale(100., 100., 880.), Some(4.), "a small one stops");
        assert_eq!(fit_scale(1760., 200., 880.), Some(0.5));
        let huge = fit_scale(10_000., 10_000., 880.).unwrap();
        assert!(10_000. * huge * 2. * 10_000. * huge * 2. <= MAX_PIXELS * 1.01, "{huge}");
        assert_eq!(fit_scale(1_000_000., 1_000_000., 880.), None, "past a useful size");
        assert_eq!(fit_scale(0., 10., 880.), None);
        assert_eq!(fit_scale(f32::INFINITY, 10., 880.), None);
    }

    #[test]
    fn labels_cant_put_markup_into_the_picture() {
        let svg = mermaid_rs_renderer::render("graph TD\n  A[\"<script>evil()</script><image href='file:///x'/>\"] --> B\n").unwrap();
        assert!(!svg.contains("<script") && !svg.contains("<image"), "{svg}");
    }
}
