//! Lays boxes out at a width, as the browser does: margins that collapse, line boxes with half-leading, the table's
//! auto layout, advances rounded to whole pixels as Chromium's are here. Each top-level block is laid out on its own,
//! when it's near the view, so a long document costs what's on screen.

use super::ast::Align;
use super::boxes::*;
use gpui::{FontId, GlyphId};

/// Chromium's `LayoutUnit`: sizes floor to a 64th of a pixel.
pub fn lu(x: f32) -> f32 {
    (x * 64.).floor() / 64.
}

#[derive(Debug, Clone, Copy)]
pub struct Glyph {
    pub id: GlyphId,
    pub font: FontId,
    /// Its byte in the shaped text.
    pub index: u32,
    pub x: f32,
    pub emoji: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Shaped {
    pub glyphs: Vec<Glyph>,
    pub width: f32,
}

impl Shaped {
    pub fn x_at_byte(&self, byte: usize) -> f32 {
        let i = self.glyphs.partition_point(|g| (g.index as usize) < byte);
        self.glyphs.get(i).map_or(self.width, |g| g.x)
    }

    pub fn glyph_at_byte(&self, byte: usize) -> usize {
        self.glyphs.partition_point(|g| (g.index as usize) < byte)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontM {
    pub ascent: f32,
    pub descent: f32,
    pub gap: f32,
    pub x_height: f32,
}

/// What layout needs of the text system: `view.rs` has the window's, a test a fake.
pub trait Shaper {
    fn shape(&mut self, text: &str, style: &TextStyle) -> Shaped;
    fn metrics(&mut self, style: &TextStyle) -> FontM;
    /// An image's size, once it's decoded; what its box shows otherwise.
    fn image(&mut self, slot: u32) -> ImageState;
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImageState {
    /// What it reserves while it loads, fails or waits: the box's room, by what it says.
    Boxed { text_w: f32 },
    Shown { w: f32, h: f32 },
}

#[derive(Debug, Clone, Default)]
pub struct Frag {
    pub item: u32,
    /// The glyphs it shows, of its item.
    pub from: u32,
    pub to: u32,
    pub x: f32,
    pub w: f32,
    /// A box item's height and how far its top is above the baseline.
    pub above: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Default)]
pub struct Line {
    pub y: f32,
    pub h: f32,
    pub baseline: f32,
    pub x: f32,
    pub w: f32,
    pub frags: Vec<Frag>,
}

#[derive(Debug, Clone, Default)]
pub struct LaidInline {
    pub shaped: Vec<Shaped>,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone, Default)]
pub struct LaidCode {
    pub lines: Vec<Shaped>,
    pub line_h: f32,
    pub baseline: f32,
    pub width: f32,
    /// Room for its scrollbar, when it scrolls.
    pub overflow: bool,
    pub view_w: f32,
}

#[derive(Debug, Clone, Default)]
pub struct LaidRow {
    pub y: f32,
    pub h: f32,
    pub cells: Vec<Placed>,
    /// Each cell's x and width.
    pub boxes: Vec<(f32, f32)>,
}

#[derive(Debug, Clone, Default)]
pub struct LaidTable {
    pub caption: Option<Box<Placed>>,
    pub rows: Vec<LaidRow>,
    pub width: f32,
    pub top: f32,
    pub overflow: bool,
    pub view_w: f32,
}

#[derive(Debug, Clone, Default)]
pub enum LaidKind {
    #[default]
    Empty,
    Flow(Vec<Placed>),
    Inline(LaidInline),
    Code(LaidCode),
    Table(LaidTable),
}

/// A box at a width: its size, and the margins that collapse out of it.
#[derive(Debug, Clone, Default)]
pub struct Laid {
    pub w: f32,
    pub h: f32,
    pub mt: f32,
    pub mb: f32,
    pub kind: LaidKind,
}

#[derive(Debug, Clone, Default)]
pub struct Placed {
    pub x: f32,
    pub y: f32,
    pub laid: Laid,
}

/// Collapsing margins: the largest positive and the most negative.
fn collapse(a: f32, b: f32) -> f32 {
    if a >= 0. && b >= 0. {
        a.max(b)
    } else if a < 0. && b < 0. {
        a.min(b)
    } else {
        a + b
    }
}

/// Above and below the baseline, with half-leading: `FontHeight::AddLeading`'s rounding.
pub fn text_box(style: &TextStyle, m: FontM) -> (f32, f32) {
    let lh = match style.line {
        LineH::Num(k) => lu(style.size * k),
        LineH::Normal => m.ascent + m.descent + m.gap,
        LineH::Zero => 0.,
    };
    let half = ((lh - (m.ascent + m.descent)) / 2.).floor();
    let above = m.ascent + half;
    (above, lh - above)
}

/// Blink's `vertical-align: super` and `sub`, from the parent's size.
fn raise(style: &TextStyle) -> f32 {
    match style.valign {
        VAlign::Base => 0.,
        VAlign::Super => (style.size * 1.2 / 3. + 1.).floor(),
        VAlign::Sub => -((style.size * 1.2 / 5. + 1.).floor()),
    }
}

struct Seg {
    /// Item, from byte, to byte in that item's own text. Pads and boxes: 0..0.
    pieces: Vec<(usize, usize, usize)>,
    width: f32,
    /// Without its trailing spaces.
    fit: f32,
    forced: bool,
}

fn item_text<'a>(item: &Item, stream: &'a str) -> &'a str {
    match item {
        Item::Text { range, .. } => &stream[range.clone()],
        _ => "",
    }
}

/// `true`: a box item's width is what the line takes, not its glyphs'.
fn box_size(item: &Item, shaped: &Shaped, s: &mut dyn Shaper, stream: &str, max_w: f32) -> Option<(f32, f32, f32)> {
    match item {
        Item::Image { slot, style } => {
            let (w, h) = match s.image(*slot) {
                ImageState::Shown { w, h } => {
                    let scale = if w > max_w && max_w > 0. { max_w / w } else { 1. };
                    (w * scale, h * scale)
                }
                ImageState::Boxed { text_w } => (text_w.min(max_w), 24_f32.max(0.)),
            };
            let m = s.metrics(style);
            // `vertical-align: middle`: its middle on the parent's baseline plus half an x-height.
            let above = h / 2. + m.x_height / 2.;
            Some((w, above, h))
        }
        Item::Kbd { range, style } => {
            let m = s.metrics(style);
            let _ = &stream[range.clone()];
            // Padding 1px 5px, a border of 1px with 2 along the bottom.
            let lh = m.ascent + m.descent + m.gap;
            Some((shaped.width + 12., 2. + m.ascent, lh + 5.))
        }
        Item::Task { style, .. } => {
            let m = s.metrics(style);
            Some((13., 13. / 2. + m.x_height / 2., 13.))
        }
        _ => None,
    }
}

/// `ib`'s lines at `width`. `nowrap`: one line, however long.
pub fn layout_inline(ib: &InlineBlock, width: f32, stream: &str, s: &mut dyn Shaper) -> LaidInline {
    let n = ib.items.len();
    let mut shaped: Vec<Shaped> = Vec::with_capacity(n);
    let mut para = String::new();
    let mut at: Vec<usize> = Vec::with_capacity(n);
    for item in &ib.items {
        at.push(para.len());
        match item {
            Item::Text { range, style } => {
                let text = &stream[range.clone()];
                shaped.push(s.shape(text, style));
                para.push_str(text);
            }
            Item::Kbd { range, style } => {
                shaped.push(s.shape(&stream[range.clone()], style));
                para.push('\u{fffc}');
            }
            Item::Image { .. } | Item::Task { .. } => {
                shaped.push(Shaped::default());
                para.push('\u{fffc}');
            }
            Item::Pad { .. } => shaped.push(Shaped::default()),
            Item::Break => {
                shaped.push(Shaped::default());
                para.push('\n');
            }
        }
    }
    // The line boxes' pieces: what fits between one place a line may break and the next.
    let mut breaks: Vec<(usize, bool)> = unicode_linebreak::linebreaks(&para).map(|(i, o)| (i, o == unicode_linebreak::BreakOpportunity::Mandatory)).collect();
    if breaks.last().map(|b| b.0) != Some(para.len()) {
        breaks.push((para.len(), true));
    }
    let bytes = para.as_bytes();
    let mut segs: Vec<Seg> = Vec::with_capacity(breaks.len());
    let mut from = 0;
    let mut i = 0;
    for &(to, forced) in &breaks {
        let mut seg = Seg { pieces: Vec::new(), width: 0., fit: 0., forced: forced && to > 0 && bytes.get(to - 1) == Some(&b'\n') };
        while i < n {
            let start = at[i];
            let end = at.get(i + 1).copied().unwrap_or(para.len());
            match &ib.items[i] {
                Item::Pad { px, end: closes, .. } => {
                    let inside = if *closes { start <= to } else { start < to || to == para.len() };
                    if !inside {
                        break;
                    }
                    seg.pieces.push((i, 0, 0));
                    seg.width += *px;
                    seg.fit = seg.width;
                    i += 1;
                    continue;
                }
                Item::Text { .. } => {
                    if start >= to {
                        break;
                    }
                    let a = from.max(start) - start;
                    let b = to.min(end) - start;
                    let text = item_text(&ib.items[i], stream);
                    let sh = &shaped[i];
                    let w = sh.x_at_byte(b) - sh.x_at_byte(a);
                    let trimmed = text[a..b].trim_end_matches(' ');
                    let fit_w = sh.x_at_byte(a + trimmed.len()) - sh.x_at_byte(a);
                    seg.fit = if trimmed.is_empty() { seg.fit.max(seg.width) } else { seg.width + fit_w };
                    seg.width += w;
                    seg.pieces.push((i, a, b));
                    if end > to {
                        break;
                    }
                    i += 1;
                }
                Item::Break => {
                    if start >= to {
                        break;
                    }
                    i += 1;
                }
                other => {
                    if start >= to {
                        break;
                    }
                    let (w, ..) = box_size(other, &shaped[i], s, stream, width).unwrap_or((0., 0., 0.));
                    seg.pieces.push((i, 0, 0));
                    seg.width += w;
                    seg.fit = seg.width;
                    i += 1;
                }
            }
        }
        // A pad that opens right before this break belongs with what comes after it.
        segs.push(seg);
        from = to;
    }
    // Item, its bytes, x and width.
    type Placing = (usize, usize, usize, f32, f32);
    let mut lines: Vec<Vec<Placing>> = vec![Vec::new()];
    let mut x = 0.;
    let wrap = !ib.nowrap;
    for seg in &segs {
        if wrap && x > 0. && x + seg.fit > width + 0.001 {
            lines.push(Vec::new());
            x = 0.;
        }
        let too_long = wrap && seg.fit > width && width > 0.;
        for &(item, a, b) in &seg.pieces {
            let it = &ib.items[item];
            match it {
                Item::Text { .. } if too_long => {
                    // `overflow-wrap: anywhere`: it breaks at a character.
                    let sh = &shaped[item];
                    let text = item_text(it, stream);
                    let mut start = a;
                    for (k, c) in text[a..b].char_indices() {
                        let pos = a + k;
                        let w = sh.x_at_byte(pos + c.len_utf8()) - sh.x_at_byte(start);
                        if x + w > width && x + (sh.x_at_byte(pos) - sh.x_at_byte(start)) > 0. {
                            let done = sh.x_at_byte(pos) - sh.x_at_byte(start);
                            if pos > start {
                                lines.last_mut().expect("a line").push((item, start, pos, x, done));
                            }
                            lines.push(Vec::new());
                            x = 0.;
                            start = pos;
                        }
                    }
                    if b > start {
                        let w = sh.x_at_byte(b) - sh.x_at_byte(start);
                        lines.last_mut().expect("a line").push((item, start, b, x, w));
                        x += w;
                    }
                }
                Item::Text { .. } => {
                    let sh = &shaped[item];
                    let w = sh.x_at_byte(b) - sh.x_at_byte(a);
                    lines.last_mut().expect("a line").push((item, a, b, x, w));
                    x += w;
                }
                Item::Pad { px, .. } => {
                    lines.last_mut().expect("a line").push((item, 0, 0, x, *px));
                    x += *px;
                }
                other => {
                    let (w, ..) = box_size(other, &shaped[item], s, stream, width).unwrap_or((0., 0., 0.));
                    lines.last_mut().expect("a line").push((item, 0, 0, x, w));
                    x += w;
                }
            }
        }
        if seg.forced {
            lines.push(Vec::new());
            x = 0.;
        }
    }
    // A forced break at the very end makes no line of its own.
    if lines.len() > 1 && lines.last().is_some_and(Vec::is_empty) {
        lines.pop();
    }
    let strut = s.metrics(&ib.strut);
    let (strut_above, strut_below) = text_box(&ib.strut, strut);
    let mut out = Vec::with_capacity(lines.len());
    let mut y = 0.;
    for list in lines {
        let (mut above, mut below) = (strut_above, strut_below);
        let mut frags = Vec::with_capacity(list.len());
        let mut right: f32 = 0.;
        for (item, a, b, fx, w) in list {
            let it = &ib.items[item];
            let mut frag = Frag { item: item as u32, x: fx, w, ..Default::default() };
            match it {
                Item::Text { style, .. } => {
                    let sh = &shaped[item];
                    frag.from = sh.glyph_at_byte(a) as u32;
                    frag.to = sh.glyph_at_byte(b) as u32;
                    let (ta, tb) = text_box(style, s.metrics(style));
                    let up = raise(style);
                    above = above.max(ta + up);
                    below = below.max(tb - up);
                    // Trailing spaces hang: they don't move a centred line.
                    let text = item_text(it, stream);
                    let kept = text[a..b].trim_end_matches(' ').len();
                    if kept > 0 {
                        right = right.max(fx + sh.x_at_byte(a + kept) - sh.x_at_byte(a));
                    }
                }
                Item::Pad { .. } => right = right.max(fx + w),
                other => {
                    let (_, ab, h) = box_size(other, &shaped[item], s, stream, width).unwrap_or((0., 0., 0.));
                    frag.above = ab;
                    frag.h = h;
                    above = above.max(ab);
                    below = below.max(h - ab);
                    right = right.max(fx + w);
                }
            }
            frags.push(frag);
        }
        let h = above + below;
        let offset = match ib.align {
            Align::Center => ((width - right) / 2.).max(0.),
            Align::Right => (width - right).max(0.),
            _ => 0.,
        };
        out.push(Line { y, h, baseline: above, x: offset, w: right, frags });
        y += h;
    }
    LaidInline { shaped, lines: out }
}

/// The least and the most an inline block can take.
pub fn inline_widths(ib: &InlineBlock, stream: &str, s: &mut dyn Shaper) -> (f32, f32) {
    let laid = layout_inline(ib, f32::MAX / 4., stream, s);
    let max = laid.lines.iter().map(|l| l.w).fold(0., f32::max);
    if ib.nowrap {
        return (max, max);
    }
    // The widest thing a line can't break.
    let mut para = String::new();
    let mut min: f32 = 0.;
    for (i, item) in ib.items.iter().enumerate() {
        match item {
            Item::Text { .. } => {
                let text = item_text(item, stream);
                para.clear();
                para.push_str(text);
                let sh = &laid.shaped[i];
                let mut from = 0;
                for (to, _) in unicode_linebreak::linebreaks(&para) {
                    let kept = para[from..to].trim_end_matches(' ').len();
                    min = min.max(sh.x_at_byte(from + kept) - sh.x_at_byte(from));
                    from = to;
                }
            }
            other => {
                if let Some((w, ..)) = box_size(other, &laid.shaped[i], s, stream, f32::MAX / 4.) {
                    min = min.max(w);
                }
            }
        }
    }
    (min, max)
}

/// A box's least and most widths, its padding and margins with it.
fn intrinsic(node: &BoxNode, stream: &str, s: &mut dyn Shaper) -> (f32, f32) {
    let extra = node.margin.left + node.margin.right + node.padding.left + node.padding.right + node.border.left + node.border.right;
    let (min, max) = match &node.kind {
        BoxKind::Inline(ib) => inline_widths(ib, stream, s),
        BoxKind::Flow(kids) => kids.iter().map(|k| intrinsic(k, stream, s)).fold((0_f32, 0_f32), |a, b| (a.0.max(b.0), a.1.max(b.1))),
        BoxKind::Details { summary, body, .. } => {
            let mut w = intrinsic(summary, stream, s);
            for k in body {
                let x = intrinsic(k, stream, s);
                w = (w.0.max(x.0), w.1.max(x.1));
            }
            w
        }
        BoxKind::Code(c) => {
            let w = c.lines.iter().map(|r| s.shape(&stream[r.clone()], &c.style).width).fold(0., f32::max);
            (w.min(100.), w)
        }
        BoxKind::Table(t) => {
            let (_, min, max) = table_columns(t, stream, s);
            (min.iter().sum::<f32>() + 1., max.iter().sum::<f32>() + 1.)
        }
        BoxKind::Columns { cols, gap } => {
            let list: Vec<(f32, f32)> = cols.iter().map(|k| intrinsic(k, stream, s)).collect();
            (list.iter().map(|x| x.0).sum::<f32>() + gap, list.iter().map(|x| x.1).sum::<f32>() + gap)
        }
        _ => (0., 0.),
    };
    (min + extra, max + extra)
}

/// The most columns a table has: past it, a row's cells are left out. `colspan` multiplies a cell into up to a thousand.
pub const MAX_COLUMNS: usize = 1024;

/// Each column's least and most, a cell's half borders in them.
fn table_columns(t: &TableBox, stream: &str, s: &mut dyn Shaper) -> (usize, Vec<f32>, Vec<f32>) {
    let cols = t.rows.iter().map(|r| r.cells.iter().map(|c| c.span as usize).sum::<usize>()).max().unwrap_or(0).min(MAX_COLUMNS);
    let (mut min, mut max) = (vec![0_f32; cols], vec![0_f32; cols]);
    // Each cell's asked once: asked again per pass, tables in tables with spans cost two to their depth.
    let mut spanning = Vec::new();
    for r in &t.rows {
        let mut at = 0;
        for c in &r.cells {
            if at >= cols {
                break;
            }
            let span = (c.span as usize).min(cols - at);
            let (a, b) = intrinsic(&c.content, stream, s);
            if span == 1 {
                let (a, b) = (if c.nowrap { b } else { a }, b);
                min[at] = min[at].max(a + 1.);
                max[at] = max[at].max(b + 1.);
            } else {
                spanning.push((at, span, a, b));
            }
            at += span;
        }
    }
    // A spanning cell's room goes to its columns evenly.
    {
        for (at, span, a, b) in spanning {
            {
                let (have_min, have_max): (f32, f32) = (min[at..at + span].iter().sum(), max[at..at + span].iter().sum());
                let (add_min, add_max) = ((a + 1. - have_min).max(0.) / span as f32, (b + 1. - have_max).max(0.) / span as f32);
                for k in at..at + span {
                    min[k] += add_min;
                    max[k] += add_max;
                }
            }
        }
    }
    (cols, min, max)
}

fn layout_table(t: &TableBox, avail: f32, stream: &str, s: &mut dyn Shaper) -> (LaidTable, f32) {
    let (cols, min, max) = table_columns(t, stream, s);
    let (sum_min, sum_max): (f32, f32) = (min.iter().sum(), max.iter().sum());
    let room = (avail - 1.).max(0.);
    let mut widths: Vec<f32>;
    if t.full {
        // The front matter's: keys at their width, values in the rest.
        widths = max.clone();
        if cols == 2 {
            widths[1] = (room - widths[0]).max(min[1]);
        }
    } else if sum_max <= room {
        widths = max.clone();
    } else if sum_min >= room {
        widths = min.clone();
    } else {
        let k = (room - sum_min) / (sum_max - sum_min);
        widths = min.iter().zip(&max).map(|(a, b)| a + (b - a) * k).collect();
    }
    // Whole pixels, as a table's columns are; what rounding leaves goes to the last one.
    let sum: f32 = widths.iter().sum();
    let mut rounded: Vec<f32> = widths.iter().map(|w| w.floor()).collect();
    let left = (sum - rounded.iter().sum::<f32>()).round().max(0.);
    if let Some(last) = rounded.last_mut() {
        *last += left;
    }
    widths = rounded;
    let total: f32 = widths.iter().sum::<f32>() + 1.;
    let mut y = 0.;
    let caption = t.caption.as_ref().map(|c| {
        let laid = layout_box(c, total.max(if t.full { avail } else { 0. }), stream, s);
        let p = Placed { x: 0., y: 0., laid };
        y = p.laid.h;
        Box::new(p)
    });
    let top = y;
    let mut rows = Vec::with_capacity(t.rows.len());
    // Half of the outer border.
    let mut ry = y + 0.5;
    for r in &t.rows {
        let mut at = 0;
        let mut x = 0.5;
        let mut cells = Vec::with_capacity(r.cells.len());
        let mut boxes = Vec::with_capacity(r.cells.len());
        let mut h: f32 = 0.;
        for c in &r.cells {
            if at >= cols {
                break;
            }
            let span = (c.span as usize).min(cols - at);
            let w: f32 = widths[at..at + span].iter().sum();
            let laid = layout_box(&c.content, (w - 1.).max(0.), stream, s);
            h = h.max(laid.h + laid.mt + laid.mb);
            cells.push(Placed { x: x + 0.5, y: 0.5 + laid.mt, laid });
            boxes.push((x, w));
            x += w;
            at += span;
        }
        let h = lu(h + 1.);
        for (p, c) in cells.iter_mut().zip(&r.cells) {
            if !c.top {
                p.y += ((h - 1. - p.laid.h - p.laid.mt - p.laid.mb) / 2.).max(0.);
            }
        }
        rows.push(LaidRow { y: ry, h, cells, boxes });
        ry += h;
    }
    let width = if t.full { total.max(avail) } else { total };
    let overflow = width > avail + 0.5;
    let h = ry + 0.5 + if overflow { 10. } else { 0. };
    (LaidTable { caption, rows, width, top, overflow, view_w: avail }, h)
}

fn layout_code(c: &CodeBox, view_w: f32, stream: &str, s: &mut dyn Shaper) -> LaidCode {
    let m = s.metrics(&c.style);
    let (above, _) = text_box(&c.style, m);
    let line_h = match c.style.line {
        LineH::Num(k) => lu(c.style.size * k),
        _ => m.ascent + m.descent,
    };
    let lines: Vec<Shaped> = c.lines.iter().map(|r| s.shape(&stream[r.clone()], &c.style)).collect();
    let sign_w = if c.signs { (c.style.size * 0.6).round() * 2. } else { 0. };
    let width = lines.iter().map(|l| l.width).fold(0., f32::max) + sign_w;
    LaidCode { lines, line_h, baseline: above, width, overflow: width > view_w + 0.5, view_w }
}

/// `node` at `avail`: its margins and collapsing with its first and last children.
pub fn layout_box(node: &BoxNode, avail: f32, stream: &str, s: &mut dyn Shaper) -> Laid {
    let w = (avail - node.margin.left - node.margin.right).max(0.);
    let (pad_x, pad_top, pad_bottom) = (node.padding.left + node.padding.right + node.border.left + node.border.right, node.padding.top + node.border.top, node.padding.bottom + node.border.bottom);
    let content_w = (w - pad_x).max(0.);
    let inner_x = node.padding.left + node.border.left;
    let mut laid = Laid { w, h: 0., mt: node.margin.top, mb: node.margin.bottom, kind: LaidKind::Empty };
    match &node.kind {
        BoxKind::Inline(ib) => {
            let inline = layout_inline(ib, content_w, stream, s);
            let h: f32 = inline.lines.iter().map(|l| l.h).sum();
            laid.h = h + pad_top + pad_bottom;
            laid.kind = LaidKind::Inline(inline);
        }
        BoxKind::Code(c) => {
            let code = layout_code(c, content_w, stream, s);
            let body = code.line_h * code.lines.len() as f32;
            laid.h = body + pad_top + pad_bottom + if code.overflow { 10. } else { 0. };
            laid.kind = LaidKind::Code(code);
        }
        BoxKind::Table(t) => {
            let (table, h) = layout_table(t, content_w, stream, s);
            laid.h = h + pad_top + pad_bottom;
            laid.kind = LaidKind::Table(table);
        }
        BoxKind::Rule => laid.h = 2.,
        BoxKind::Hatch => {
            laid.h = 0.;
            laid.kind = LaidKind::Empty;
        }
        BoxKind::Empty => {}
        BoxKind::Flow(kids) => flow(kids.iter(), &mut laid, node, content_w, inner_x, pad_top, pad_bottom, stream, s),
        BoxKind::Details { summary, body, open } => {
            let list: Vec<&BoxNode> = if *open { std::iter::once(&**summary).chain(body.iter()).collect() } else { vec![&**summary] };
            flow(list.into_iter(), &mut laid, node, content_w, inner_x, pad_top, pad_bottom, stream, s);
        }
        BoxKind::Columns { cols, gap } => {
            let n = cols.len().max(1) as f32;
            let col_w = ((content_w - gap * (n - 1.)) / n).max(0.);
            let mut placed = Vec::with_capacity(cols.len());
            let mut h: f32 = 0.;
            for (i, c) in cols.iter().enumerate() {
                let l = layout_box(c, col_w, stream, s);
                h = h.max(l.h + l.mt + l.mb);
                placed.push(Placed { x: inner_x + i as f32 * (col_w + gap), y: pad_top + l.mt, laid: l });
            }
            laid.h = h + pad_top + pad_bottom;
            laid.kind = LaidKind::Flow(placed);
        }
    }
    laid
}

#[allow(clippy::too_many_arguments)]
fn flow<'a>(kids: impl Iterator<Item = &'a BoxNode>, laid: &mut Laid, node: &BoxNode, content_w: f32, inner_x: f32, pad_top: f32, pad_bottom: f32, stream: &str, s: &mut dyn Shaper) {
    let mut placed = Vec::new();
    let mut y = pad_top;
    let mut pending: Option<f32> = None;
    let mut first = true;
    for k in kids {
        let c = layout_box(k, content_w, stream, s);
        let through = c.h == 0. && !matches!(k.kind, BoxKind::Rule);
        let x = inner_x + k.margin.left;
        if through {
            // An empty box: its margins collapse through it.
            let m = collapse(c.mt, c.mb);
            pending = Some(pending.map_or(m, |p| collapse(p, m)));
            placed.push(Placed { x, y, laid: c });
            continue;
        }
        let m = pending.map_or(c.mt, |p| collapse(p, c.mt));
        if first && pad_top == 0. {
            // Its top margin goes out through ours.
            laid.mt = collapse(laid.mt, m);
        } else {
            y += m;
        }
        first = false;
        let (h, mb) = (c.h, c.mb);
        placed.push(Placed { x, y, laid: c });
        y += h;
        pending = Some(mb);
    }
    if pad_bottom == 0. && !placed.is_empty() {
        laid.mb = pending.map_or(node.margin.bottom, |p| collapse(node.margin.bottom, p));
        laid.h = y;
    } else {
        laid.h = y + pending.unwrap_or(0.) + pad_bottom;
    }
    laid.kind = LaidKind::Flow(placed);
}

#[cfg(test)]
pub mod tests {
    use super::super::ast::{Flavor, parse};
    use super::super::boxes::{Params, build};
    use super::super::safe::to_dom;
    use super::*;
    use std::prelude::v1::test;

    /// Every character `0.5 em` wide, on a font with an ascent of 14 and a descent of 4 at 13 px.
    pub struct Fake;

    impl Shaper for Fake {
        fn shape(&mut self, text: &str, style: &TextStyle) -> Shaped {
            let adv = (style.size * 0.5).round();
            let glyphs: Vec<Glyph> = text.char_indices().enumerate().map(|(i, (b, _))| Glyph { id: GlyphId(0), font: FontId(0), index: b as u32, x: i as f32 * adv, emoji: false }).collect();
            let width = glyphs.len() as f32 * adv;
            Shaped { glyphs, width }
        }
        fn metrics(&mut self, style: &TextStyle) -> FontM {
            FontM { ascent: (style.size * 14. / 13.).round(), descent: (style.size * 4. / 13.).round(), gap: 0., x_height: (style.size / 2.).round() }
        }
        fn image(&mut self, _: u32) -> ImageState {
            ImageState::Shown { w: 100., h: 50. }
        }
    }

    pub fn built(md: &str) -> Built {
        build(&to_dom(&parse(md, Flavor::Github, false).root), Params { font_px: 13., line_height: 1.55, editor_code: true }, None)
    }

    fn tops(md: &str, w: f32) -> (Built, Vec<Laid>) {
        let b = built(md);
        let laid = b.tops.iter().map(|t| layout_box(t, w, &b.stream, &mut Fake)).collect();
        (b, laid)
    }

    fn lines(l: &Laid) -> &LaidInline {
        match &l.kind {
            LaidKind::Inline(i) => i,
            other => panic!("not inline: {other:?}"),
        }
    }

    #[test]
    fn a_paragraph_breaks_where_it_may_and_keeps_its_line_height() {
        let (_, l) = tops("aaaa bbbb cccc dddd", 7. * 10.);
        let i = lines(&l[0]);
        assert_eq!(i.lines.len(), 2, "ten characters a line");
        assert_eq!(l[0].h, 2. * lu(13. * 1.55));
        assert_eq!(i.lines[0].baseline, 14. + 1., "ascent and half the leading, floored");
    }

    #[test]
    fn a_word_too_long_for_the_line_breaks_at_a_character() {
        let (_, l) = tops(&"x".repeat(25), 70.);
        assert_eq!(lines(&l[0]).lines.len(), 3);
    }

    #[test]
    fn margins_collapse_between_siblings_and_through_parents() {
        let (b, l) = tops("para\n\n## head\n\n> quote\n\nend", 700.);
        // A heading's top of 16 beats a paragraph's bottom of 10.
        assert_eq!(b.tops[1].margin.top, 16.);
        assert_eq!(l[1].mt, 16.);
        // A quote takes its last paragraph's 10, not 20.
        assert_eq!(l[2].mb, 10.);
        assert_eq!(l[0].mb, 10.);
    }

    #[test]
    fn the_first_and_the_last_blocks_lose_their_outer_margins() {
        let (b, _) = tops("## a\n\nb", 700.);
        assert_eq!(b.tops[0].margin.top, 0.);
        assert_eq!(b.tops[1].margin.bottom, 0.);
    }

    #[test]
    fn a_tight_item_has_no_paragraph_and_a_nested_list_a_margin() {
        let (b, l) = tops("- one\n- two\n  - three\n", 700.);
        let BoxKind::Flow(items) = &b.tops[0].kind else { panic!() };
        assert_eq!(items[1].margin.top, 2.);
        assert!(matches!(items[0].marker, Some(Marker::Bullet(MarkerKind::Disc))));
        let LaidKind::Flow(p) = &l[0].kind else { panic!() };
        assert_eq!(p[0].x, 26., "2em of 13 px");
        assert_eq!(p[1].y - p[0].y, lu(13. * 1.55) + 2.);
    }

    #[test]
    fn inline_code_has_its_padding_and_sub_and_sup_leave_the_line_alone() {
        let (_, l) = tops("a `b` c<sup>2</sup>", 700.);
        let i = lines(&l[0]);
        assert_eq!(l[0].h, lu(13. * 1.55), "the line's as tall as the strut");
        assert!(i.lines[0].frags.iter().any(|f| f.w > 4. && f.w < 5.), "0.35em");
    }

    #[test]
    fn a_table_takes_its_contents_width_up_to_the_measure() {
        let (b, l) = tops("| a | b |\n|---|---|\n| long cell | x |\n", 700.);
        let LaidKind::Table(t) = &l[0].kind else { panic!() };
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[0].boxes[0].1, 9. * 7. + 20. + 1., "max content, padding, half borders");
        assert_eq!(t.width, 84. + 28. + 1.);
        let BoxKind::Table(tb) = &b.tops[0].kind else { panic!() };
        assert_eq!(tb.rows[1].bg, None, "the first row of tbody isn't shaded");
        assert!(!t.overflow);
        // Narrower than its contents: the columns give up what they can.
        let (_, l) = tops("| a | b |\n|---|---|\n| one two three four five six | x |\n", 150.);
        let LaidKind::Table(t) = &l[0].kind else { panic!() };
        assert!(t.width <= 150.);
        assert!(t.rows[1].h > 30., "the long cell wraps");
        let (_, l) = tops(&format!("| a |\n|---|\n| {} |\n", "x".repeat(50)), 100.);
        let LaidKind::Table(t) = &l[0].kind else { panic!() };
        assert!(t.overflow, "a word it can't break scrolls");
    }

    #[test]
    fn colspans_cannot_multiply_a_table_into_billions_of_columns() {
        let row = "<td colspan=1000>x</td>".repeat(50_000);
        let (_, l) = tops(&format!("<table><tr>{row}</tr><tr><td>a</td></tr></table>"), 700.);
        let LaidKind::Table(t) = &l[0].kind else { panic!() };
        assert_eq!(t.rows.len(), 2);
        assert!(t.rows[0].boxes.len() <= MAX_COLUMNS);
    }

    #[test]
    fn tables_in_tables_with_spans_cost_their_size_not_two_to_their_depth() {
        let s = "<table><tr><td colspan=2>".repeat(40) + "x";
        let started = std::time::Instant::now();
        let (_, l) = tops(&s, 700.);
        assert!(!l.is_empty());
        assert!(started.elapsed().as_secs() < 30);
    }

    #[test]
    fn the_deepest_tree_lets_boxes_and_layout_finish_on_a_small_stack() {
        std::thread::Builder::new()
            .stack_size(1 << 21)
            .spawn(|| {
                for s in ["<div>".repeat(5_000) + "x", "<details open>".repeat(5_000) + "x", "<blockquote>".repeat(5_000) + "x", "> ".repeat(5_000) + "x", "- ".repeat(5_000) + "x", "<ul><li>".repeat(5_000) + "x", "<table><tr><td>".repeat(5_000) + "x"] {
                    let (_, l) = tops(&s, 700.);
                    assert!(!l.is_empty(), "{}", &s[..8]);
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn a_long_code_line_scrolls_and_takes_a_scrollbars_room() {
        let (_, l) = tops(&format!("```\n{}\n```\n", "x".repeat(200)), 300.);
        let LaidKind::Code(c) = &l[0].kind else { panic!() };
        assert!(c.overflow);
        assert_eq!(l[0].h, c.line_h + 16. + 10.);
    }

    #[test]
    fn an_empty_html_block_takes_no_room_and_an_anchor_stays() {
        let (b, l) = tops("before\n\n<a name=\"here\"></a>\n\nafter", 700.);
        // Its paragraph holds nothing, so its margins collapse through it, as in the browser.
        assert_eq!(l[1].h, 0.);
        assert!(b.anchors.contains_key("user-content-here"));
    }

    #[test]
    fn a_closed_details_shows_only_its_summary() {
        let (_, closed) = tops("<details><summary>S</summary>\n\nbody\n\n</details>", 700.);
        let (_, open) = tops("<details open><summary>S</summary>\n\nbody\n\n</details>", 700.);
        assert!(open[0].h > closed[0].h);
    }

    #[test]
    fn the_stream_holds_the_text_in_order_for_a_selection() {
        let b = built("# T\n\na **b** c\n\n- x\n- y\n\n| h |\n|---|\n| d |\n");
        assert_eq!(b.stream, "T\n\na b c\n\nx\ny\nh\nd\n\n");
    }
}
