//! Whole-pixel advances, as Chromium's on Linux at a device scale of 1: no subpixel positioning there, so Skia hands HarfBuzz
//! each glyph's advance rounded, and a line's width is whole pixels. GPUI's text system lays out unhinted, so a line came out
//! up to 3% off and wrapped elsewhere (459 px against 472 for one line, an editor's advance 8 against 7.8). `Whole` wraps the
//! platform's text system and rounds what it lays out, so everything drawn or measured, the wrapper's widths too, agrees.
//! Page zoom keeps the grid in device pixels, as Chromium lays out at the zoomed size; a display's own scale turns it off, as
//! Chromium turns subpixel positioning on there.
//!
//! GPUI has no `letter-spacing`, so a font asked for with the feature `ltsp`, its value in thousandths of an em, gets it here:
//! `spacing(0.04)` on any element. No shaper knows that tag, and it gives the font an id of its own. Text then ends on fractions
//! of a pixel as Chromium's does, and each glyph is drawn on a whole one, as Skia draws it with no subpixel positioning.

use anyhow::Result;
use gpui::*;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

/// Device pixels a logical pixel is on the grid: 0 for no grid. `f32` bits.
static GRID: AtomicU32 = AtomicU32::new(1f32.to_bits());

const LETTER_SPACING: &str = "ltsp";

/// CSS's `letter-spacing` in ems, for `.font_features(..)`; `spacing(0.)` turns an inherited one off.
pub fn spacing(em: f32) -> FontFeatures {
    FontFeatures(Arc::new(vec![(LETTER_SPACING.to_string(), (em * 1000.).round() as u32)]))
}

/// A line's glyphs on the grid, as Blink's: Skia gives HarfBuzz each glyph's own advance rounded, GPOS adds its kerning to
/// that, and `RoundHarfBuzzBufferPositions` rounds the sum. `own` is a glyph's own advance at this size.
pub fn round_advances(layout: &mut LineLayout, grid: f32, own: impl Fn(FontId, GlyphId) -> f32, spacing: impl Fn(FontId) -> f32) {
    let snap = |gap: f32| if grid > 0. { (gap * grid).round() / grid } else { gap };
    let mut at = 0f32;
    // A glyph's place in the shaper's terms, its own advance and its letter spacing.
    let mut last: Option<(f32, f32, f32)> = None;
    let width = layout.width.as_f32();
    let step = |last: Option<(f32, f32, f32)>, next: f32| last.map_or(next, |(x, own, extra): (f32, f32, f32)| {
        if grid <= 0. {
            return next - x + extra;
        }
        let kerning = next - x - own;
        // FreeType scales an advance to 26.6 first: 6.498 is 6.5 there, and rounds up.
        snap(snap((own * 64.).round() / 64.) + kerning) + extra
    });
    for run in &mut layout.runs {
        let extra = spacing(run.font_id);
        for glyph in &mut run.glyphs {
            let x = glyph.position.x.as_f32();
            at += step(last, x);
            last = Some((x, own(run.font_id, glyph.id), extra));
            glyph.position.x = px(snap(at));
        }
    }
    if last.is_some() {
        layout.width = px(at + step(last, width));
    }
}

/// Keeps the grid with the window's scale and zoom; what's been laid out on another is laid out again.
pub fn follow(window: &Window, cx: &App) {
    let real = window.scale_factor() / crate::zoom::percent() as f32 * 100.;
    let grid = if (real - 1.).abs() < 0.001 { crate::zoom::percent() as f32 / 100. } else { 0. };
    if GRID.swap(grid.to_bits(), Relaxed) != grid.to_bits() {
        // Adding no font clears GPUI's line caches, which is all that's wanted of it.
        cx.text_system().add_fonts(Vec::new()).ok();
    }
}

pub struct Whole(pub Arc<dyn PlatformTextSystem>, pub RwLock<HashMap<FontId, f32>>);

impl Whole {
    pub fn new(inner: Arc<dyn PlatformTextSystem>) -> Self {
        Self(inner, RwLock::default())
    }
}

impl PlatformTextSystem for Whole {
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        self.0.add_fonts(fonts)
    }
    fn set_missing_glyph_sink(&self, sink: Option<Arc<dyn MissingGlyphSink>>) {
        self.0.set_missing_glyph_sink(sink)
    }
    fn all_font_names(&self) -> Vec<String> {
        self.0.all_font_names()
    }
    fn font_id(&self, descriptor: &Font) -> Result<FontId> {
        let id = self.0.font_id(descriptor)?;
        if let Some((_, em)) = descriptor.features.tag_value_list().iter().find(|(tag, _)| tag == LETTER_SPACING)
            && let Ok(mut spaced) = self.1.write()
        {
            spaced.insert(id, *em as f32 / 1000.);
        }
        Ok(id)
    }
    fn prewarm_fonts(&self, font_ids: &[FontId]) {
        self.0.prewarm_fonts(font_ids)
    }
    fn font_metrics(&self, font_id: FontId) -> FontMetrics {
        self.0.font_metrics(font_id)
    }
    fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>> {
        self.0.typographic_bounds(font_id, glyph_id)
    }
    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
        self.0.advance(font_id, glyph_id)
    }
    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        self.0.glyph_for_char(font_id, ch)
    }
    fn glyph_raster_bounds(&self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        self.0.glyph_raster_bounds(params)
    }
    fn rasterize_glyph(&self, params: &RenderGlyphParams, raster_bounds: Bounds<DevicePixels>) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        self.0.rasterize_glyph(params, raster_bounds)
    }
    fn layout_line(&self, text: &str, font_size: Pixels, runs: &[FontRun]) -> LineLayout {
        let mut layout = self.0.layout_line(text, font_size, runs);
        let spaced = self.1.read().ok();
        let spacing = |font: FontId| spaced.as_ref().and_then(|s| s.get(&font).copied()).unwrap_or(0.) * font_size.as_f32();
        let own = |font: FontId, glyph: GlyphId| {
            let per_em = self.0.font_metrics(font).units_per_em as f32;
            self.0.advance(font, glyph).map_or(0., |a| a.width / per_em * font_size.as_f32())
        };
        round_advances(&mut layout, f32::from_bits(GRID.load(Relaxed)), own, spacing);
        layout
    }
    fn recommended_rendering_mode(&self, font_id: FontId, font_size: Pixels) -> TextRenderingMode {
        self.0.recommended_rendering_mode(font_id, font_size)
    }
    fn glyph_dilation_for_color(&self, color: Hsla) -> u8 {
        self.0.glyph_dilation_for_color(color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn line(xs: &[f32], width: f32) -> LineLayout {
        let glyphs = xs.iter().enumerate().map(|(i, x)| ShapedGlyph { id: GlyphId(i as u32), position: point(px(*x), px(0.)), index: i, is_emoji: false }).collect();
        LineLayout { font_size: px(13.), width: px(width), ascent: px(0.), descent: px(0.), runs: vec![ShapedRun { font_id: FontId(0), glyphs }], len: xs.len() }
    }

    fn xs(l: &LineLayout) -> Vec<f32> {
        l.runs.iter().flat_map(|r| r.glyphs.iter().map(|g| g.position.x.as_f32())).collect()
    }

    #[test]
    fn each_advance_rounds_on_its_own() {
        let mut l = line(&[0., 7.29, 14.58, 21.87], 29.16);
        round_advances(&mut l, 1., |_, _| 7.29, |_| 0.);
        assert_eq!(xs(&l), [0., 7., 14., 21.]);
        assert_eq!(l.width.as_f32(), 28.);
    }

    #[test]
    fn kerning_adds_to_the_rounded_advance() {
        // Noto Sans's `r.` at 13 px: 5.37 with a kerning of -1.7, then 3.48: 3, then 3; rounding the sum would say 4.
        let mut l = line(&[0., 3.67], 7.15);
        round_advances(&mut l, 1., |_, g| if g.0 == 0 { 5.37 } else { 3.48 }, |_| 0.);
        assert_eq!(xs(&l), [0., 3.]);
        assert_eq!(l.width.as_f32(), 6.);
    }

    #[test]
    fn a_zoom_rounds_in_device_pixels() {
        let mut l = line(&[0., 7.33], 14.66);
        round_advances(&mut l, 1.5, |_, _| 7.33, |_| 0.);
        // 7.33 × 1.5 = 10.995: 11 device pixels.
        assert!((xs(&l)[1] - 11. / 1.5).abs() < 0.001);
    }

    #[test]
    fn no_grid_leaves_the_line_alone() {
        let mut l = line(&[0., 7.29], 14.5);
        round_advances(&mut l, 0., |_, _| 7.29, |_| 0.);
        assert_eq!(xs(&l), [0., 7.29]);
        assert_eq!(l.width.as_f32(), 14.5);
    }

    #[test]
    fn letter_spacing_adds_to_each_and_a_glyph_stays_on_a_whole_pixel() {
        let mut l = line(&[0., 7.29, 14.58], 21.87);
        round_advances(&mut l, 1., |_, _| 7.29, |_| 0.4);
        assert_eq!(xs(&l), [0., 7., 15.]);
        assert!((l.width.as_f32() - 22.2).abs() < 0.001);
    }

    #[test]
    fn an_empty_line_stays_empty() {
        let mut l = line(&[], 0.);
        round_advances(&mut l, 1., |_, _| 0., |_| 0.);
        assert_eq!(l.width.as_f32(), 0.);
    }
}
