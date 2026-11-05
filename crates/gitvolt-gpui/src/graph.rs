//! One graph row's bands, lines and node: `ui/src/graph/draw.ts`, by row. Straight lines are quads
//! and only curves are tessellated, since a row can cross dozens of lanes.

use crate::model::*;
use crate::theme;
use gitvolt_graph::NodeKind;
use gpui::*;
use std::sync::Arc;

const DASH: f32 = 3.;
/// Left of the collapse zone, how wide its shade fades.
const SHADE_W: f32 = 12.;
const DOT_EVERY: f32 = 2. * LINE_W;

#[derive(Clone, Copy)]
pub struct RowGeometry {
    /// Where the row's chips end, from the row's left.
    pub chips_end: f32,
    /// Where the graph column starts, from the row's left.
    pub graph_x: f32,
    pub graph_w: f32,
    pub clipped: bool,
    pub selected: bool,
    pub head: bool,
}

fn vline(window: &mut Window, x: Pixels, top: Pixels, bottom: Pixels, color: Hsla) {
    window.paint_quad(fill(Bounds::from_corners(point(x - px(LINE_W / 2.), top), point(x + px(LINE_W / 2.), bottom)), color));
}

/// A line's pattern is phased by the row's content y, so it runs on unbroken from row to row.
fn patterned(window: &mut Window, x: Pixels, top: f32, bottom: f32, origin_y: Pixels, seg: Seg, color: Hsla) {
    if seg.dashed {
        let mut y = (top / (2. * DASH)).floor() * 2. * DASH;
        while y < bottom {
            let (a, b) = (y.max(top), (y + DASH).min(bottom));
            if b > a {
                vline(window, x, origin_y + px(a - top), origin_y + px(b - top), color);
            }
            y += 2. * DASH;
        }
    } else {
        let mut y = (top / DOT_EVERY).ceil() * DOT_EVERY;
        while y < bottom {
            let c = origin_y + px(y - top);
            window.paint_quad(quad(Bounds::new(point(x - px(LINE_W / 2.), c - px(LINE_W / 2.)), size(px(LINE_W), px(LINE_W))), px(LINE_W / 2.), color, px(0.), transparent_black(), BorderStyle::default()));
            y += DOT_EVERY;
        }
    }
}

fn paint_segment(window: &mut Window, seg: Seg, origin: Point<Pixels>, content_y: f32, color: Hsla) {
    let (xa, xb) = (origin.x + px(lane_x(seg.from)), origin.x + px(lane_x(seg.to)));
    let (top, mid, bottom) = (origin.y, origin.y + px(ROW_H / 2.), origin.y + px(ROW_H));
    let straight = seg.from == seg.to || seg.half == HALF_FULL;
    if straight {
        let (a, b, from) = match seg.half {
            HALF_FULL => (top, bottom, 0.),
            HALF_TOP => (top, mid, 0.),
            _ => (mid, bottom, ROW_H / 2.),
        };
        if seg.dashed || seg.dotted {
            let span = (b - a).as_f32();
            patterned(window, xa, content_y + from, content_y + from + span, a, seg, color);
        } else {
            vline(window, xa, a, b, color);
        }
        return;
    }
    let r = px((LANE_W / 2.).min(ROW_H / 2.));
    let dir = if xb > xa { 1. } else { -1. };
    let mut p = PathBuilder::stroke(px(LINE_W));
    if seg.half == HALF_TOP {
        p.move_to(point(xa, top));
        p.line_to(point(xa, mid - r));
        p.curve_to(point(xa + r * dir, mid), point(xa, mid));
        p.line_to(point(xb, mid));
    } else {
        p.move_to(point(xa, mid));
        p.line_to(point(xb - r * dir, mid));
        p.curve_to(point(xb, mid + r), point(xb, mid));
        p.line_to(point(xb, bottom));
    }
    if let Ok(path) = p.build() {
        window.paint_path(path, color);
    }
}

fn paint_node(window: &mut Window, cx: &mut App, row: &RowView, c: Point<Pixels>, packed: bool, picture: Option<Arc<RenderImage>>) {
    let color = theme::lane(row.color as usize);
    let dim = |h: Hsla| if packed { Hsla { a: h.a * 0.5, ..h } } else { h };
    let kind = if packed && row.kind == NodeKind::Merge { NodeKind::Commit } else { row.kind };
    let circle = |r: f32| Bounds::new(c - point(px(r), px(r)), size(px(2. * r), px(2. * r)));
    match kind {
        NodeKind::Collapsed => {}
        NodeKind::Merge => window.paint_quad(quad(circle(MERGE_R), px(MERGE_R), dim(color), px(0.), transparent_black(), BorderStyle::default())),
        NodeKind::Stash => {
            let s = NODE_R * 0.95 + LINE_W / 2.;
            window.paint_quad(quad(circle(s), px(0.), theme::node_fill(), px(LINE_W), dim(color), BorderStyle::Dashed));
        }
        NodeKind::Wip => window.paint_quad(quad(circle(NODE_R + LINE_W / 2.), px(NODE_R + LINE_W / 2.), theme::node_fill(), px(LINE_W), dim(color), BorderStyle::Dashed)),
        NodeKind::Commit => {
            let r = NODE_R + LINE_W / 2.;
            window.paint_quad(quad(circle(r), px(r), dim(theme::lane(row.avatar)), px(LINE_W), dim(color), BorderStyle::default()));
            if let Some(picture) = picture.filter(|_| !packed) {
                // A transparent picture shows on the backdrop, not on the lane's colour.
                let inner = NODE_R - LINE_W / 2.;
                window.paint_quad(quad(circle(inner), px(inner), theme::avatar_backdrop(), px(0.), transparent_black(), BorderStyle::default()));
                window.paint_image(circle(inner), circle(inner), Corners::all(px(inner)), picture, 0, false).ok();
                return;
            }
            let size_px = px((ROW_H * 0.32).round());
            let font = Font { weight: FontWeight::SEMIBOLD, ..window.text_style().font() };
            let run = TextRun { len: row.initials.len(), font, color: dim(theme::lane_text(row.avatar)), background_color: None, underline: None, strikethrough: None };
            let line = window.text_system().shape_line(row.initials.clone(), size_px, &[run], None);
            let origin = point(c.x - line.width / 2., c.y - px(ROW_H / 2.));
            line.paint(origin, px(ROW_H + 1.), TextAlign::Left, None, window, cx).ok();
        }
    }
}

/// Paints row `row` (index `ix`) in `bounds`, the whole row: the labels column's connector, the
/// graph column's band, lines, node and rail. The cells' text goes on top, as elements.
pub fn paint_row(bounds: Bounds<Pixels>, row: &RowView, ix: usize, g: RowGeometry, picture: Option<Arc<RenderImage>>, window: &mut Window, cx: &mut App) {
    let color = theme::lane(row.color as usize);
    let origin = point(bounds.origin.x + px(g.graph_x), bounds.origin.y);
    let column_end = origin.x + px(g.graph_w);
    let node = node_x(row.lane, g.graph_w, g.clipped);
    let area = if g.clipped { g.graph_w - zone_width() } else { g.graph_w };
    let node_left = origin.x + px(node.unwrap_or(area));
    let (band_top, band_bottom) = (origin.y + px(BAND_INSET), origin.y + px(ROW_H - BAND_INSET));

    window.paint_quad(fill(Bounds::from_corners(point(node_left, band_top), point(column_end, band_bottom)), theme::lane_alpha(row.color as usize, if g.selected { SELECTED_BAND_ALPHA } else { BAND_ALPHA })));
    if !row.chips.is_empty() {
        let thick = if g.head { LINE_W } else { 1. };
        let y = origin.y + px(((ROW_H - thick) / 2.).floor());
        let to = origin.x + px(node.unwrap_or_else(|| packed_x(g.graph_w) - NODE_R));
        let tint = if g.head { color } else { theme::lane_alpha(row.color as usize, CONNECTOR_ALPHA) };
        // From where the chips end: it reaches the node, it doesn't run under them.
        window.paint_quad(fill(Bounds::from_corners(point(bounds.origin.x + px(g.chips_end), y), point(to, y + px(thick))), tint));
    }
    let content_y = ix as f32 * ROW_H;
    let mask = ContentMask { bounds: Bounds::from_corners(point(origin.x, origin.y), point(origin.x + px(area), origin.y + px(ROW_H))) };
    window.with_content_mask(Some(mask), |window| {
        for seg in &row.segments {
            paint_segment(window, *seg, origin, content_y, theme::lane(seg.color as usize));
        }
    });
    let centre_y = origin.y + px(ROW_H / 2.);
    match node {
        Some(x) => paint_node(window, cx, row, point(origin.x + px(x), centre_y), false, picture),
        None => paint_node(window, cx, row, point(origin.x + px(packed_x(g.graph_w)), centre_y), true, picture),
    }
    // The collapse zone's shade: black fading in over the last lanes, up to the zone.
    if g.clipped {
        let zone = column_end - px(zone_width());
        let shade = Bounds::from_corners(point(zone - px(SHADE_W), origin.y), point(zone, origin.y + px(ROW_H)));
        window.paint_quad(fill(shade, linear_gradient(90., linear_color_stop(Hsla::transparent_black(), 0.), linear_color_stop(theme::collapse_strip(), 1.))));
    }
    window.paint_quad(fill(Bounds::from_corners(point(column_end - px(RAIL_W), band_top), point(column_end, band_bottom)), color));
}
