//! The default dark theme: `ui/src/theme/tokens.css`'s first-paint values, as colours.

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
pub fn control_bg() -> Hsla { a(0xffffff0f) }
pub fn control_hover_bg() -> Hsla { a(0xffffff1f) }
pub fn node_fill() -> Hsla { c(0x1c1e23) }

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
