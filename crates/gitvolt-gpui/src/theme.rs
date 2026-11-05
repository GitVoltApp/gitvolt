//! The default dark theme: `ui/src/theme/tokens.css`'s first-paint values, as colours.

// Every token the CSS has, whether a view uses it yet or not.
#![allow(dead_code)]

use gpui::{Hsla, Rgba, rgb, rgba};

pub fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

pub fn a(hex: u32) -> Hsla {
    rgba(hex).into()
}

pub fn app_bg() -> Hsla { c(0x1c1e23) }
pub fn panel_bg() -> Hsla { c(0x262930) }
pub fn toolbar_bg() -> Hsla { c(0x2a2e36) }
pub fn action_bar_bg() -> Hsla { c(0x33373f) }
pub fn tab_bar_bg() -> Hsla { c(0x1f2228) }
pub fn text_selected() -> Hsla { c(0xffffff) }
pub fn text_normal() -> Hsla { a(0xffffffbf) }
pub fn text_secondary() -> Hsla { a(0xffffff99) }
pub fn text_disabled() -> Hsla { a(0xffffff66) }
pub fn hover_row() -> Hsla { a(0x4d88ff1a) }
pub fn selected_row() -> Hsla { a(0x4d88ff33) }
pub fn section_border() -> Hsla { a(0xffffff14) }
pub fn blue() -> Hsla { c(0x4d88ff) }
pub fn count_blue() -> Hsla { c(0x6d9deb) }
pub fn sidebar_icon() -> Hsla { a(0xffffff73) }
pub fn checked_out_row() -> Hsla { c(0x37563e) }
pub fn checked_out() -> Hsla { c(0x457c4a) }
pub fn avatar_backdrop() -> Hsla { c(0xe6e7ea) }
pub fn control_bg() -> Hsla { a(0xffffff0f) }
pub fn control_hover_bg() -> Hsla { a(0xffffff1f) }
pub fn node_fill() -> Hsla { c(0x1c1e23) }
pub fn file_bar_bg() -> Hsla { c(0x272a31) }
pub fn panel_bg1() -> Hsla { c(0x30343c) }
pub fn status_added() -> Hsla { c(0x5cb85c) }
pub fn status_modified() -> Hsla { c(0xde9b43) }
pub fn status_deleted() -> Hsla { c(0xd9413d) }
pub fn status_conflicted() -> Hsla { c(0xb56ee6) }
pub fn stats_added_bright() -> Hsla { c(0x5cb85c) }
pub fn stats_deleted_bright() -> Hsla { c(0xf0625d) }
pub fn selection_bg() -> Hsla { c(0x264f78) }
pub fn scroll_thumb() -> Hsla { a(0xffffff26) }
pub fn diff_inserted_text() -> Hsla { a(0x5cb85c1f) }
pub fn diff_removed_text() -> Hsla { a(0xd9413d33) }
pub fn diff_inserted_line() -> Hsla { a(0x5cb85c1a) }
pub fn diff_removed_line() -> Hsla { a(0xd9413d26) }
pub fn diff_diagonal() -> Hsla { a(0xcccccc33) }
pub fn ruler_added() -> Hsla { a(0x5cb85c99) }
pub fn ruler_removed() -> Hsla { a(0xd9413d99) }
pub fn red() -> Hsla { c(0xd9413d) }
pub fn green() -> Hsla { c(0x5cb85c) }
pub fn orange() -> Hsla { c(0xde9b43) }
pub fn yellow() -> Hsla { c(0xecb91c) }
pub fn link() -> Hsla { c(0x40c5ec) }
pub fn purple() -> Hsla { c(0xa371f7) }
pub fn stats_added() -> Hsla { c(0x347d39) }
pub fn stats_deleted() -> Hsla { c(0xc93c37) }
pub fn switch_on() -> Hsla { c(0x4d88ff) }
pub fn switch_off() -> Hsla { a(0xffffff29) }
pub fn focus_outline() -> Hsla { a(0x4d88ff99) }
pub fn text_on_accent() -> Hsla { c(0xffffff) }
pub fn text_row_dimmed() -> Hsla { a(0xffffff33) }
pub fn text_row_dimmed_branch() -> Hsla { a(0xffffff80) }
pub fn shadow_modal() -> Hsla { a(0x00000080) }
pub fn shadow_popup() -> Hsla { a(0x00000073) }
pub fn shadow_tooltip() -> Hsla { a(0x00000066) }
pub fn backdrop() -> Hsla { a(0x00000073) }
pub fn collapse_strip() -> Hsla { a(0x00000066) }
pub fn wip_input_bg() -> Hsla { c(0x16181c) }
pub fn wip_input_border() -> Hsla { c(0x282a2d) }
pub fn wip_input_placeholder() -> Hsla { c(0x6e6f71) }
pub fn yellow_banner() -> Hsla { a(0xecb91c2e) }
/// `--blue` at 35%: the filter's matched substring.
pub fn filter_match() -> Hsla { a(0x4d88ff59) }
pub fn image_frame_border() -> Hsla { a(0xffffff38) }
/// Chromium's dark range input (the web UI leaves it unstyled).
pub fn range_fill() -> Hsla { c(0x99c8ff) }
pub fn range_track() -> Hsla { c(0x3b3b3b) }
pub fn range_border() -> Hsla { c(0x858585) }

/// `color-mix(in srgb, a share, b)`: `share` of `a`, the rest of `b`, alpha mixed too.
pub fn mix(a: Hsla, share: f32, b: Hsla) -> Hsla {
    let (x, y) = (a.to_rgb(), b.to_rgb());
    let (ax, ay) = (x.a * share, y.a * (1. - share));
    let alpha = ax + ay;
    if alpha <= 0. {
        return Hsla::transparent_black();
    }
    let ch = |p: f32, q: f32| (p * ax + q * ay) / alpha;
    Rgba { r: ch(x.r, y.r), g: ch(x.g, y.g), b: ch(x.b, y.b), a: alpha }.into()
}

/// `top`, with its alpha, painted over `below`: an opaque stand-in for a translucent background.
pub fn over(top: Hsla, below: Hsla) -> Hsla {
    mix(Hsla { a: 1., ..top }, top.a, below)
}

/// Lane colours 0-9 (`--graph-0`..`--graph-9`).
pub const LANES: [u32; 10] = [0x15a0bf, 0x0669f7, 0x8e00c2, 0xc517b6, 0xd90171, 0xcd0101, 0xf25d2e, 0xf2ca33, 0x7bd938, 0x2ece9d];

pub fn lane(i: usize) -> Hsla {
    c(LANES[i % LANES.len()])
}

pub fn lane_alpha(i: usize, alpha: f32) -> Hsla {
    Hsla { a: alpha, ..lane(i) }
}

fn luminance(hex: u32) -> f32 {
    let c: Rgba = rgb(hex);
    let ch = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * ch(c.r) + 0.7152 * ch(c.g) + 0.0722 * ch(c.b)
}

/// The text on a lane colour: white or the darkest background, whichever contrasts more
/// (`themes.ts` `tickOn`).
pub fn lane_text(i: usize) -> Hsla {
    let hex = LANES[i % LANES.len()];
    let l = luminance(hex);
    let (on_white, on_dark) = (1.05 / (l + 0.05), (l + 0.05) / (luminance(0x1c1e23) + 0.05));
    if on_white >= on_dark { c(0xffffff) } else { c(0x1c1e23) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lanes_text_is_whichever_contrasts_more() {
        assert_eq!(lane_text(1), c(0xffffff), "the blue");
        assert_eq!(lane_text(7), c(0x1c1e23), "the yellow");
    }
}
