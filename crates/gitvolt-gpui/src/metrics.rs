//! The sizes settings change: the density's (`theme/density.ts`) and the editors' (`diff/options.ts`). Read as often as the
//! constants they were: an atomic load, from any thread; what a change invalidates is `settings/effects.rs`'s to say.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

macro_rules! metric {
    ($($store:ident, $get:ident, $set:ident, $default:expr;)*) => {
        $(
            static $store: AtomicU32 = AtomicU32::new(f32::to_bits($default));
            #[inline]
            pub fn $get() -> f32 {
                f32::from_bits($store.load(Relaxed))
            }
            pub fn $set(v: f32) {
                $store.store(v.to_bits(), Relaxed);
            }
        )*
    };
}

metric! {
    ROW_H, row_h, set_row_h, 28.;
    LANE_W, lane_w, set_lane_w, 22.;
    BAND_INSET, band_inset, set_band_inset, 3.;
    CHIP_H, chip_h, set_chip_h, 22.;
    CELL_PAD, cell_pad, set_cell_pad, 8.;
    FILE_ROW_H, file_row_h, set_file_row_h, 26.;
    PANEL_BAR_H, panel_bar_h, set_panel_bar_h, 37.;
    FONT_SIZE, editor_font_size, set_editor_font_size, 13.;
    LINE_H, line_h, set_line_h, 18.;
    ADVANCE, advance, set_advance, 8.;
    TAB_SIZE, tab_size_f, set_tab_size_f, 4.;
}

pub fn pad_x() -> f32 {
    lane_w() / 2.
}

pub fn node_r() -> f32 {
    row_h() * 0.36
}

pub fn merge_r() -> f32 {
    row_h() * 0.18
}

pub fn tab_size() -> usize {
    tab_size_f() as usize
}

/// `DENSITY_METRICS`.
pub fn set_density(name: &str) -> bool {
    let (row, lane, band, chip, pad, file, bar) = match name {
        "compact" => (25., 16., 2., 17., 6., 24., 31.),
        "comfortable" => (32., 24., 4., 24., 10., 30., 41.),
        _ => (28., 22., 3., 22., 8., 26., 37.),
    };
    let changed = row_h() != row;
    set_row_h(row);
    set_lane_w(lane);
    set_band_inset(band);
    set_chip_h(chip);
    set_cell_pad(pad);
    set_file_row_h(file);
    set_panel_bar_h(bar);
    changed
}

/// How the editors draw: the font, its ligatures, what shows of whitespace.
#[derive(Clone, PartialEq)]
pub struct EditorStyle {
    pub family: String,
    pub ligatures: bool,
    pub whitespace: String,
    pub minimap: bool,
    pub sticky: bool,
}

thread_local! {
    static STYLE: RefCell<EditorStyle> = RefCell::new(EditorStyle { family: String::new(), ligatures: false, whitespace: "none".into(), minimap: true, sticky: false });
}

pub fn editor_style() -> EditorStyle {
    STYLE.with_borrow(Clone::clone)
}

pub fn set_editor_style(s: EditorStyle) {
    STYLE.set(s);
}

pub fn whitespace_mode() -> u8 {
    STYLE.with_borrow(|s| match s.whitespace.as_str() {
        "all" => 2,
        "boundary" => 1,
        _ => 0,
    })
}

/// `formatDate`'s preset and relative dates: the graph's rows are built off the UI thread.
static DATE_FORMAT: AtomicU32 = AtomicU32::new(0);
static RELATIVE: AtomicU32 = AtomicU32::new(0);

pub fn set_date_options(format: &str, relative: bool) -> bool {
    let f = match format {
        "ymd24h" => 1,
        "dmy24h" => 2,
        "dmydot24h" => 3,
        "mdy12h" => 4,
        _ => 0,
    };
    let changed = DATE_FORMAT.swap(f, Relaxed) != f;
    RELATIVE.swap(u32::from(relative), Relaxed) != u32::from(relative) || changed
}

pub fn date_format() -> u32 {
    DATE_FORMAT.load(Relaxed)
}

pub fn relative_dates() -> bool {
    RELATIVE.load(Relaxed) == 1
}
