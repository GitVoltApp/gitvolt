//! The themes: `ui/src/theme/themes.ts`'s ten, as `assets/themes.json` (generated) has them, and custom ones
//! (`custom.ts`), resolved from a base and what was set. Every colour reads the shown theme: a cheap read of the
//! UI thread's copy, which a switch replaces. Then a refresh redraws everything, and nothing else differs.

// Every token the CSS has, whether a view uses it yet or not.
#![allow(dead_code)]

use gpui::{Hsla, Rgba, rgb, rgba};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

pub fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

pub fn a(hex: u32) -> Hsla {
    rgba(hex).into()
}

/// A CSS colour as the theme files write it: bytes and an alpha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Col {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f32,
}

fn round2(n: f32) -> f32 {
    (n * 100.).round() / 100.
}

impl Col {
    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 1. }
    }

    /// `colour.ts` `parseColour`: `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb()` and `rgba()`, numbers in range.
    pub fn parse(css: &str) -> Option<Self> {
        let s = css.trim();
        if let Some(h) = s.strip_prefix('#') {
            if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let wide: String = if h.len() == 3 { h.chars().flat_map(|c| [c, c]).collect() } else { h.to_string() };
            let n = |i: usize| u8::from_str_radix(&wide[i..i + 2], 16).ok();
            return match wide.len() {
                6 => Some(Self::opaque(n(0)?, n(2)?, n(4)?)),
                8 => Some(Self { r: n(0)?, g: n(2)?, b: n(4)?, a: round2(n(6)? as f32 / 255.) }),
                _ => None,
            };
        }
        let lower = s.to_ascii_lowercase();
        let (is_a, rest) = match lower.strip_prefix("rgba") {
            Some(r) => (true, r),
            None => (false, lower.strip_prefix("rgb")?),
        };
        let inner = rest.strip_prefix('(')?.strip_suffix(')')?;
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() != if is_a { 4 } else { 3 } {
            return None;
        }
        let byte = |p: &str| -> Option<u8> {
            if p.is_empty() || p.len() > 3 || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            p.parse::<u16>().ok().filter(|v| *v <= 255).map(|v| v as u8)
        };
        let alpha = if is_a {
            let p = parts[3];
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit() || b == b'.') || p.ends_with('.') {
                return None;
            }
            let v: f32 = p.parse().ok()?;
            if !(0. ..=1.).contains(&v) {
                return None;
            }
            round2(v)
        } else {
            1.
        };
        Some(Self { r: byte(parts[0])?, g: byte(parts[1])?, b: byte(parts[2])?, a: alpha })
    }

    /// `formatColour`: `#rrggbb` when opaque, else `rgba(r, g, b, a)`.
    pub fn css(self) -> String {
        if self.a >= 1. {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("rgba({}, {}, {}, {})", self.r, self.g, self.b, round2(self.a))
        }
    }

    pub fn hsla(self) -> Hsla {
        Rgba { r: self.r as f32 / 255., g: self.g as f32 / 255., b: self.b as f32 / 255., a: self.a }.into()
    }

    pub fn at(self, a: f32) -> Self {
        Self { a, ..self }
    }

    pub fn hex(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }

    pub fn of_hex(hex: u32) -> Self {
        Self::opaque((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
    }

    fn opaque_self(self) -> Self {
        self.at(1.)
    }

    fn over(self, bg: Col) -> [f32; 3] {
        let m = |v: u8, b: u8| v as f32 * self.a + b as f32 * (1. - self.a);
        [m(self.r, bg.r), m(self.g, bg.g), m(self.b, bg.b)]
    }

    fn luminance(rgb: [f32; 3]) -> f32 {
        let ch = |v: f32| {
            let s = v / 255.;
            if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * ch(rgb[0]) + 0.7152 * ch(rgb[1]) + 0.0722 * ch(rgb[2])
    }

    /// `contrast.ts` `contrastRatio`: this colour, composited if translucent, over the opaque `bg`.
    pub fn contrast(self, bg: Col) -> f32 {
        let (x, y) = (Self::luminance(self.over(bg)), Self::luminance([bg.r as f32, bg.g as f32, bg.b as f32]));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }
}

macro_rules! tokens {
    ($($name:ident = $css:literal),* $(,)?) => {
        /// Every colour token, as the views draw with them.
        #[derive(Clone, Copy)]
        pub struct Tokens { $(pub $name: Hsla),* }

        /// The CSS names, in `themes.ts`'s order.
        pub const TOKEN_NAMES: &[&str] = &[$($css),*];

        impl Tokens {
            fn of(colors: &HashMap<String, Col>) -> Self {
                let get = |t: &str| colors.get(t).copied().unwrap_or(Col::opaque(0xff, 0, 0xff)).hsla();
                Self { $($name: get($css)),* }
            }
        }

        $(#[inline] pub fn $name() -> Hsla { CURRENT.with_borrow(|p| p.tokens.$name) })*
    };
}

tokens! {
    app_bg = "app-bg0", panel_bg = "panel-bg0", panel_bg1 = "panel-bg1", toolbar_bg = "toolbar-bg0", action_bar_bg = "action-bar-bg", tab_bar_bg = "tab-bar-bg", file_bar_bg = "file-bar-bg",
    text_selected = "text-selected", text_normal = "text-normal", text_secondary = "text-secondary", text_disabled = "text-disabled", text_on_accent = "text-on-accent",
    stats_added = "stats-added", stats_deleted = "stats-deleted",
    red = "red", orange = "orange", yellow = "yellow", green = "green", blue = "blue", link = "link",
    count_blue = "count-blue", checked_out_row = "checked-out-row", checked_out = "checked-out", selection_bg = "selection-bg",
    conflict_ours = "conflict-ours", conflict_theirs = "conflict-theirs", conflict_base = "conflict-base",
    wip_input_bg = "wip-input-bg", wip_input_border = "wip-input-border", wip_input_placeholder = "wip-input-placeholder",
    hover_row = "hover-row", selected_row = "selected-row", section_border = "section-border", focus_outline = "focus-outline",
    text_row_dimmed = "text-row-dimmed", text_row_dimmed_branch = "text-row-dimmed-branch", sidebar_icon = "sidebar-icon", avatar_backdrop = "avatar-backdrop",
    stats_added_bright = "stats-added-bright", stats_deleted_bright = "stats-deleted-bright",
    status_added = "status-added", status_modified = "status-modified", status_deleted = "status-deleted", status_renamed = "status-renamed", status_conflicted = "status-conflicted",
    scroll_thumb = "scroll-thumb-bg", scroll_thumb_hover = "scroll-thumb-hover-bg",
    control_bg = "control-bg", control_hover_bg = "control-hover-bg", image_frame_border = "image-frame-border",
    shadow_modal = "shadow-modal", shadow_popup = "shadow-popup", shadow_tooltip = "shadow-tooltip", backdrop = "backdrop",
    node_fill = "node-fill", node_text = "node-text", collapse_strip = "collapse-strip",
    conflict_ours_tick = "conflict-ours-tick", conflict_theirs_tick = "conflict-theirs-tick",
    diff_inserted_text = "diff-inserted-text", diff_removed_text = "diff-removed-text", diff_inserted_line = "diff-inserted-line", diff_removed_line = "diff-removed-line", diff_diagonal = "diff-diagonal-fill",
    md_diff_ins_text = "md-diff-ins-text", md_diff_del_text = "md-diff-del-text", md_diff_ins_block = "md-diff-ins-block", md_diff_del_block = "md-diff-del-block",
    forge_gitlab = "forge-gitlab", forge_github = "forge-github", forge_forgejo = "forge-forgejo", purple = "purple",
    switch_on = "switch-on", switch_off = "switch-off",
    tg_ink = "tg-ink", tg_blue = "tg-blue", tg_purple = "tg-purple", tg_cyan = "tg-cyan", tg_orange = "tg-orange", tg_yellow = "tg-yellow", tg_pink = "tg-pink", tg_green = "tg-green", tg_gray = "tg-gray", tg_red = "tg-red",
    button_primary = "button-primary", button_warn = "button-warn", button_warn_ink = "button-warn-ink",
    review_pending = "review-pending", review_pending_bg = "review-pending-bg",
}

const CORE_TOKENS: &[&str] = &["app-bg0", "panel-bg0", "panel-bg1", "toolbar-bg0", "action-bar-bg", "tab-bar-bg", "file-bar-bg", "text-selected", "text-normal", "text-secondary", "text-disabled", "text-on-accent", "stats-added", "stats-deleted", "red", "orange", "yellow", "green", "blue", "link", "count-blue", "checked-out-row", "checked-out", "selection-bg", "conflict-ours", "conflict-theirs", "conflict-base", "wip-input-bg", "wip-input-border", "wip-input-placeholder"];

pub const THEME_IDS: [&str; 10] = ["default-dark", "light", "monokai", "darcula", "dracula", "one-dark", "solarized-dark", "solarized-light", "github-dark", "nord"];
pub const DEFAULT_THEME_ID: &str = "default-dark";

pub fn is_theme_id(id: &str) -> bool {
    THEME_IDS.contains(&id)
}

/// `custom.ts` `isCustomThemeId`.
pub fn is_custom_theme_id(id: &str) -> bool {
    id.strip_prefix("custom-").is_some_and(|h| h.len() == 8 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// `ThemeDef`: a built-in, or a custom theme resolved.
#[derive(Clone, Debug)]
pub struct ThemeDef {
    pub id: String,
    pub base: Option<String>,
    pub label: String,
    pub light: bool,
    pub editor_theme: String,
    pub colors: HashMap<String, Col>,
    pub graph: Vec<Col>,
    pub lane_text: Option<Col>,
}

fn parse_builtin(v: &Value) -> ThemeDef {
    let colors = v["colors"].as_object().map_or(HashMap::new(), |m| m.iter().filter_map(|(k, x)| Some((k.clone(), Col::parse(x.as_str()?)?))).collect());
    ThemeDef {
        id: v["id"].as_str().unwrap_or_default().to_string(),
        base: None,
        label: v["label"].as_str().unwrap_or_default().to_string(),
        light: v["kind"] == "light",
        editor_theme: v["editorTheme"].as_str().unwrap_or_default().to_string(),
        colors,
        graph: v["graph"].as_array().map_or(Vec::new(), |g| g.iter().filter_map(|x| Col::parse(x.as_str()?)).collect()),
        lane_text: v["laneText"].as_str().and_then(Col::parse),
    }
}

pub fn builtins() -> &'static [ThemeDef] {
    static THEMES: OnceLock<Vec<ThemeDef>> = OnceLock::new();
    THEMES.get_or_init(|| {
        let v: Value = serde_json::from_str(include_str!("../assets/themes.json")).expect("the embedded themes parse");
        v.as_array().map_or(Vec::new(), |list| list.iter().map(parse_builtin).collect())
    })
}

pub fn builtin(id: &str) -> Option<&'static ThemeDef> {
    builtins().iter().find(|t| t.id == id)
}

fn default_def() -> &'static ThemeDef {
    builtin(DEFAULT_THEME_ID).expect("Default Dark is built in")
}

const fn hexc(hex: u32) -> Col {
    Col { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8, a: 1. }
}

const WHITE: Col = hexc(0xffffff);
const BLACK: Col = hexc(0x000000);

fn is_light(c: Col) -> bool {
    c.contrast(BLACK) > c.contrast(WHITE)
}

/// `stepUntil`: `hex` taken toward black (`dark`) or white, in 2% steps, until `reads` says it reads.
fn step_until(from: Col, dark: bool, reads: impl Fn(Col) -> bool) -> Col {
    let to = if dark { 0. } else { 255. };
    // `for (t = 0; t <= 1; t += 0.02)`, in doubles as JavaScript adds them.
    let mut t = 0.0_f64;
    while t <= 1. {
        let ch = |v: u8| (v as f64 + (to - v as f64) * t).round() as u8;
        let c = Col::opaque(ch(from.r), ch(from.g), ch(from.b));
        if reads(c) {
            return c;
        }
        t += 0.02;
    }
    if dark { BLACK } else { WHITE }
}

fn button_fill(fill: Col, ink: Col) -> Col {
    step_until(fill, is_light(ink), |c| ink.contrast(c) >= 4.5)
}

fn readable_on(fg: Col, bg: Col) -> Col {
    step_until(fg, is_light(bg), |c| c.contrast(bg) >= 4.5)
}

fn tick_on(fill: Col, ground: Col) -> Col {
    if WHITE.contrast(fill) >= ground.contrast(fill) { WHITE } else { ground }
}

fn over_opaque(top: Col, a: f32, bg: Col) -> Col {
    let m = |v: u8, b: u8| ((v as f32 * a + b as f32 * (1. - a)).round()) as u8;
    Col::opaque(m(top.r, bg.r), m(top.g, bg.g), m(top.b, bg.b))
}

const GROUP_DARK: [(&str, u32); 10] = [("tg-ink", 0x14161a), ("tg-blue", 0x5b9cff), ("tg-purple", 0xb48cff), ("tg-cyan", 0x3fc8d8), ("tg-orange", 0xff9e4a), ("tg-yellow", 0xf2c94c), ("tg-pink", 0xff7ab8), ("tg-green", 0x5fd17f), ("tg-gray", 0xa6aab1), ("tg-red", 0xff6b6b)];
const GROUP_LIGHT: [(&str, u32); 10] = [("tg-ink", 0xffffff), ("tg-blue", 0x1a62d6), ("tg-purple", 0x7c4ddb), ("tg-cyan", 0x0b7f8f), ("tg-orange", 0xb85500), ("tg-yellow", 0x8a6500), ("tg-pink", 0xbf2c76), ("tg-green", 0x23803a), ("tg-gray", 0x5f646c), ("tg-red", 0xc42e2a)];

/// `themes.ts` `derive`: the derived tokens, from the core ones.
pub fn derive(core: &HashMap<String, Col>, light: bool) -> HashMap<String, Col> {
    let get = |t: &str| core.get(t).copied().unwrap_or(BLACK);
    let (fg, blue, green, red, orange, yellow) = (get("text-selected"), get("blue"), get("green"), get("red"), get("orange"), get("yellow"));
    let (app, ink) = (get("app-bg0"), get("text-on-accent"));
    let pick = |l: f32, d: f32| if light { l } else { d };
    let ink_on_orange = if ink.contrast(orange) >= 3. {
        ink
    } else if !is_light(ink) {
        WHITE
    } else if app.contrast(WHITE) > get("text-selected").contrast(WHITE) {
        app
    } else {
        get("text-selected")
    };
    let mut out: Vec<(&str, Col)> = vec![
        ("hover-row", blue.at(pick(0.08, 0.1))),
        ("selected-row", blue.at(pick(0.16, 0.2))),
        ("section-border", fg.at(pick(0.12, 0.08))),
        ("focus-outline", blue.at(pick(0.5, 0.6))),
        ("text-row-dimmed", fg.at(pick(0.28, 0.2))),
        ("text-row-dimmed-branch", fg.at(0.5)),
        ("sidebar-icon", fg.at(pick(0.5, 0.45))),
        ("avatar-backdrop", hexc(0xe6e7ea)),
        ("stats-added-bright", green),
        ("stats-deleted-bright", red),
        ("status-added", green),
        ("status-modified", orange),
        ("status-deleted", red),
        ("status-renamed", blue),
        ("status-conflicted", get("conflict-base")),
        ("scroll-thumb-bg", fg.at(pick(0.2, 0.15))),
        ("scroll-thumb-hover-bg", fg.at(pick(0.32, 0.25))),
        ("control-bg", fg.at(0.06)),
        ("control-hover-bg", fg.at(0.12)),
        ("image-frame-border", fg.at(pick(0.3, 0.22))),
        ("shadow-modal", BLACK.at(pick(0.2, 0.5))),
        ("shadow-popup", BLACK.at(pick(0.18, 0.45))),
        ("shadow-tooltip", BLACK.at(pick(0.16, 0.4))),
        ("backdrop", BLACK.at(pick(0.25, 0.45))),
        ("node-fill", app),
        ("node-text", fg),
        ("collapse-strip", BLACK.at(pick(0.15, 0.4))),
        ("conflict-ours-tick", tick_on(get("conflict-ours"), app)),
        ("conflict-theirs-tick", tick_on(get("conflict-theirs"), app)),
        ("diff-inserted-text", green.at(0.12)),
        ("diff-removed-text", red.at(0.2)),
        ("diff-inserted-line", green.at(0.1)),
        ("diff-removed-line", red.at(0.15)),
        ("diff-diagonal-fill", if light { Col { r: 34, g: 34, b: 34, a: 0.2 } } else { Col { r: 204, g: 204, b: 204, a: 0.2 } }),
        ("md-diff-ins-text", green.at(0.3)),
        ("md-diff-del-text", red.at(0.3)),
        ("md-diff-ins-block", green.at(0.16)),
        ("md-diff-del-block", red.at(0.16)),
        ("forge-gitlab", hexc(0xe2432a)),
        ("forge-github", hexc(0x6e5494)),
        ("forge-forgejo", hexc(0xde6a29)),
        ("purple", if light { hexc(0x8250df) } else { hexc(0xa371f7) }),
        ("switch-on", blue),
        ("switch-off", fg.at(pick(0.24, 0.16))),
        ("button-primary", button_fill(blue, ink)),
        ("button-warn", button_fill(orange, ink_on_orange)),
        ("button-warn-ink", ink_on_orange),
        ("review-pending", readable_on(yellow, over_opaque(yellow, 0.14, get("action-bar-bg")))),
        ("review-pending-bg", yellow.at(0.14)),
    ];
    for (t, hex) in if light { GROUP_LIGHT } else { GROUP_DARK } {
        out.push((t, hexc(hex)));
    }
    out.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

fn valid_colour(v: &Value) -> Option<Col> {
    let s = v.as_str()?;
    if s != s.trim() {
        return None;
    }
    Col::parse(s)
}

/// `custom.ts` `customThemeDef`, from a `CustomTheme` as the settings hold it.
pub fn custom_theme_def(ct: &Value) -> ThemeDef {
    let base = ct["base"].as_str().and_then(builtin).unwrap_or_else(default_def);
    let mut set: HashMap<String, Col> = HashMap::new();
    if let Some(colors) = ct["colors"].as_object() {
        for (t, v) in colors {
            if TOKEN_NAMES.contains(&t.as_str())
                && let Some(c) = valid_colour(v)
            {
                set.insert(t.clone(), c);
            }
        }
    }
    let mut core = HashMap::new();
    let mut for_derive = HashMap::new();
    let mut base_core = HashMap::new();
    for t in CORE_TOKENS {
        let b = base.colors.get(*t).copied().unwrap_or(BLACK);
        core.insert((*t).to_string(), set.get(*t).copied().unwrap_or(b));
        for_derive.insert((*t).to_string(), set.get(*t).map_or(b, |c| c.opaque_self()));
        base_core.insert((*t).to_string(), b);
    }
    let base_derived = derive(&base_core, base.light);
    let now_derived = derive(&for_derive, base.light);
    // A derived token the base set by hand keeps the base's value; the rest follow the edit.
    let mut colors = core;
    for (t, now) in &now_derived {
        let own = base.colors.get(t).copied().unwrap_or(*now);
        colors.insert(t.clone(), if base_derived.get(t) == Some(&own) { *now } else { own });
    }
    colors.extend(set);
    let lanes: Option<Vec<Col>> = ct["graph"].as_array().filter(|g| g.len() == 10).and_then(|g| g.iter().map(|x| x.as_str().filter(|s| s.len() == 7 && s.starts_with('#')).and_then(Col::parse)).collect());
    ThemeDef {
        id: ct["id"].as_str().unwrap_or_default().to_string(),
        base: Some(base.id.clone()),
        label: ct["name"].as_str().unwrap_or_default().to_string(),
        light: base.light,
        editor_theme: base.editor_theme.clone(),
        colors,
        graph: lanes.unwrap_or_else(|| base.graph.clone()),
        lane_text: base.lane_text,
    }
}

/// `custom.ts` `themeDef`: a built-in, else that custom theme, else Default Dark.
pub fn theme_def(id: &str, customs: &Value) -> ThemeDef {
    if let Some(t) = builtin(id) {
        return t.clone();
    }
    match customs.as_array().and_then(|l| l.iter().find(|c| c["id"] == id)) {
        Some(ct) => custom_theme_def(ct),
        None => default_def().clone(),
    }
}

/// `store.ts`'s `ThemePatch`: unsaved colours shown over the theme while a picker drags.
#[derive(Clone, Default, PartialEq)]
pub struct ThemePatch {
    pub colors: Vec<(String, Col)>,
    pub graph: Option<Vec<Col>>,
}

/// The colours Monaco draws an editor with: its theme's own, or Monaco's defaults for its kind.
#[derive(Clone, Copy)]
pub struct EditorColors {
    pub foreground: Hsla,
    pub line_number: Hsla,
    pub line_number_active: Hsla,
    pub indent_guide: Hsla,
    pub whitespace: Hsla,
    pub cursor: Hsla,
    pub find_match: Hsla,
    pub find_highlight: Hsla,
    pub fold_icon: Hsla,
    pub line_highlight: Option<Hsla>,
    pub line_highlight_border: Option<Hsla>,
}

/// What's drawn with now.
#[derive(Clone)]
pub struct Palette {
    pub tokens: Tokens,
    pub lanes: [Hsla; 10],
    pub lane_text: [Hsla; 10],
    pub editor: EditorColors,
    pub light: bool,
    /// Bumped on each change: what's built from colours (a file row's tint) goes by it.
    pub version: u64,
}

impl Palette {
    fn of(def: &ThemeDef, version: u64) -> Self {
        let tokens = Tokens::of(&def.colors);
        let lane = |i: usize| def.graph.get(i).copied().unwrap_or(BLACK);
        // White or the theme's dark ink, whichever reads better on the lane (`apply.ts` `resolveColors`).
        let ink = def.colors.get(if def.light { "text-selected" } else { "app-bg0" }).copied().unwrap_or(BLACK);
        let on = |i: usize| def.lane_text.unwrap_or_else(|| if WHITE.contrast(lane(i)) >= ink.contrast(lane(i)) { WHITE } else { ink });
        let t = crate::diff::textmate::theme_named(&def.editor_theme);
        let h = |c: Col| c.hsla();
        let e = &t.editor;
        let editor = EditorColors { foreground: h(e.foreground), line_number: h(e.line_number), line_number_active: h(e.line_number_active), indent_guide: h(e.indent_guide), whitespace: h(e.whitespace), cursor: h(e.cursor), find_match: h(e.find_match), find_highlight: h(e.find_highlight), fold_icon: h(e.fold_icon), line_highlight: e.line_highlight.map(h), line_highlight_border: e.line_highlight_border.map(h) };
        Self { tokens, lanes: std::array::from_fn(|i| lane(i).hsla()), lane_text: std::array::from_fn(|i| on(i).hsla()), editor, light: def.light, version }
    }
}

thread_local! {
    static CURRENT: RefCell<Palette> = RefCell::new(Palette::of(default_def(), 0));
    /// The theme shown and any preview over it.
    static APPLIED: RefCell<(Rc<ThemeDef>, Option<ThemePatch>)> = RefCell::new((Rc::new(default_def().clone()), None));
}

fn show(def: &ThemeDef) {
    let version = CURRENT.with_borrow(|p| p.version) + 1;
    crate::diff::textmate::set_token_theme(&def.editor_theme);
    let palette = Palette::of(def, version);
    CURRENT.set(palette);
}

fn with_patch(def: &ThemeDef, patch: &ThemePatch) -> ThemeDef {
    let mut d = def.clone();
    for (t, c) in &patch.colors {
        d.colors.insert(t.clone(), *c);
    }
    if let Some(g) = &patch.graph {
        d.graph = g.clone();
    }
    d
}

/// Applies `def`; the caller refreshes the windows. The same theme again does nothing and says so.
pub fn apply(def: ThemeDef) -> bool {
    let same = APPLIED.with_borrow(|(d, p)| p.is_none() && d.id == def.id && d.colors == def.colors && d.graph == def.graph && d.label == def.label);
    if same {
        return false;
    }
    show(&def);
    APPLIED.set((Rc::new(def), None));
    true
}

/// `store.ts` `preview`: shows `patch` over the applied theme without saving it; `None` goes back.
pub fn preview(patch: Option<ThemePatch>) {
    let def = APPLIED.with_borrow(|(d, _)| d.clone());
    match &patch {
        Some(p) => show(&with_patch(&def, p)),
        None => show(&def),
    }
    APPLIED.with_borrow_mut(|a| a.1 = patch);
}

pub fn applied() -> Rc<ThemeDef> {
    APPLIED.with_borrow(|(d, _)| d.clone())
}

pub fn version() -> u64 {
    CURRENT.with_borrow(|p| p.version)
}

pub fn is_light_theme() -> bool {
    CURRENT.with_borrow(|p| p.light)
}

pub fn editor() -> EditorColors {
    CURRENT.with_borrow(|p| p.editor)
}

/// `at(c.green, '99')`: the overview ruler's marks.
pub fn ruler_added() -> Hsla {
    Hsla { a: 0x99 as f32 / 255., ..green() }
}

pub fn ruler_removed() -> Hsla {
    Hsla { a: 0x99 as f32 / 255., ..red() }
}

/// `--yellow` at 18%: a banner.
pub fn yellow_banner() -> Hsla {
    Hsla { a: 0.18, ..yellow() }
}

/// `--blue` at 35%: the filter's matched substring.
pub fn filter_match() -> Hsla {
    Hsla { a: 0.35, ..blue() }
}

/// Chromium's range input, by `color-scheme` (the web UI leaves it unstyled).
pub fn range_fill() -> Hsla {
    if is_light_theme() { c(0x0075ff) } else { c(0x99c8ff) }
}

pub fn range_track() -> Hsla {
    if is_light_theme() { c(0xefefef) } else { c(0x3b3b3b) }
}

pub fn range_border() -> Hsla {
    if is_light_theme() { c(0xb2b2b2) } else { c(0x858585) }
}

/// `--tg-<name>`: a tab group's and a preset's colours; a name outside the palette is gray.
pub fn tag_group(name: &str) -> Hsla {
    match name {
        "blue" => tg_blue(),
        "purple" => tg_purple(),
        "cyan" => tg_cyan(),
        "orange" => tg_orange(),
        "yellow" => tg_yellow(),
        "pink" => tg_pink(),
        "green" => tg_green(),
        "red" => tg_red(),
        _ => tg_gray(),
    }
}

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
#[inline]
pub fn lane(i: usize) -> Hsla {
    CURRENT.with_borrow(|p| p.lanes[i % 10])
}

pub fn lane_alpha(i: usize, alpha: f32) -> Hsla {
    Hsla { a: alpha, ..lane(i) }
}

/// The text on a lane colour (`apply.ts` `resolveColors`): the theme's, or per lane whichever contrasts more.
#[inline]
pub fn lane_text(i: usize) -> Hsla {
    CURRENT.with_borrow(|p| p.lane_text[i % 10])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(id: &str, name: &str, got: Option<&Col>, want: &str) {
        let want = Col::parse(want).unwrap_or_else(|| panic!("{want} parses"));
        let got = *got.unwrap_or_else(|| panic!("{id} has no {name}"));
        assert!(got.r == want.r && got.g == want.g && got.b == want.b && (got.a - want.a).abs() < 0.006, "{id} {name}: {got:?} wants {want:?}");
    }

    #[test]
    fn every_built_in_has_every_token_and_ten_lanes() {
        assert_eq!(builtins().len(), 10);
        for t in builtins() {
            for name in TOKEN_NAMES {
                assert!(t.colors.contains_key(*name), "{} lacks {name}", t.id);
            }
            assert_eq!(t.graph.len(), 10, "{}", t.id);
        }
        assert!(builtin("light").unwrap().light);
        assert_eq!(THEME_IDS.map(|i| builtin(i).is_some()), [true; 10]);
    }

    #[test]
    fn derive_gives_what_each_built_in_states_or_it_overrode_it() {
        // Each theme's by-hand derived tokens, as themes.ts overrides them; Default Dark spells out its own values.
        let overrides: &[(&str, &[&str])] = &[
            ("monokai", &["stats-deleted-bright"]),
            ("darcula", &["stats-added-bright", "stats-deleted-bright"]),
            ("solarized-dark", &["stats-deleted-bright"]),
            ("solarized-light", &["stats-added-bright", "stats-deleted-bright", "status-added"]),
            ("github-dark", &["section-border"]),
            ("nord", &["stats-deleted-bright", "status-deleted"]),
        ];
        for t in builtins().iter().filter(|t| t.id != "default-dark") {
            let core: HashMap<String, Col> = CORE_TOKENS.iter().map(|k| ((*k).to_string(), t.colors[*k])).collect();
            let d = derive(&core, t.light);
            let by_hand = overrides.iter().find(|o| o.0 == t.id).map_or(&[][..], |o| o.1);
            for (k, v) in &d {
                if !by_hand.contains(&k.as_str()) {
                    check(&t.id, k, Some(v), &t.colors[k].css());
                }
            }
        }
    }

    #[test]
    fn a_custom_theme_resolves_as_custom_ts_does() {
        let golden: Value = serde_json::from_str(include_str!("testdata/themes-golden.json")).unwrap();
        for g in golden.as_array().unwrap() {
            let def = custom_theme_def(&g["custom"]);
            let id = g["custom"]["id"].as_str().unwrap();
            for (name, want) in g["colors"].as_object().unwrap() {
                check(id, name, def.colors.get(name), want.as_str().unwrap());
            }
            let lanes: Vec<Col> = g["graph"].as_array().unwrap().iter().map(|c| Col::parse(c.as_str().unwrap()).unwrap()).collect();
            assert_eq!(def.graph, lanes, "{id}");
            assert_eq!(def.light, g["kind"] == "light");
            assert_eq!(def.editor_theme, g["editorTheme"].as_str().unwrap());
        }
    }

    #[test]
    fn colours_parse_as_colour_ts_does() {
        assert_eq!(Col::parse("#abc"), Some(Col::opaque(0xaa, 0xbb, 0xcc)));
        assert_eq!(Col::parse("#11223344"), Some(Col { r: 0x11, g: 0x22, b: 0x33, a: 0.27 }));
        assert_eq!(Col::parse("rgb(1, 2, 3)"), Some(Col::opaque(1, 2, 3)));
        assert_eq!(Col::parse("rgba(1,2,3,0.5)"), Some(Col { r: 1, g: 2, b: 3, a: 0.5 }));
        for bad in ["red", "#12", "#12345", "rgb(1,2,3,0.5)", "rgba(1,2,3)", "rgb(300,2,3)", "rgba(1,2,3,2)", "rgb(a,b,c)", "#ggg"] {
            assert_eq!(Col::parse(bad), None, "{bad}");
        }
        assert_eq!(Col::parse("#FF0000").unwrap().css(), "#ff0000");
        assert_eq!(Col { r: 1, g: 2, b: 3, a: 0.5 }.css(), "rgba(1, 2, 3, 0.5)");
    }

    #[test]
    fn a_switch_changes_what_every_token_reads_and_a_preview_goes_back() {
        let before = app_bg();
        assert_eq!(before, Col::of_hex(0x1c1e23).hsla());
        assert!(apply(builtin("light").unwrap().clone()));
        assert!(!apply(builtin("light").unwrap().clone()), "the same theme again");
        assert_eq!(app_bg(), c(0xffffff));
        assert!(is_light_theme());
        preview(Some(ThemePatch { colors: vec![("app-bg0".into(), Col::of_hex(0x112233))], graph: None }));
        assert_eq!(app_bg(), Col::of_hex(0x112233).hsla());
        preview(None);
        assert_eq!(app_bg(), c(0xffffff));
        apply(default_def().clone());
        assert_eq!(app_bg(), before);
    }

    #[test]
    fn a_lanes_text_is_the_themes_or_whichever_contrasts_more() {
        assert_eq!(lane_text(7), c(0xffffff), "Default Dark's white, on its yellow too");
        let nord = builtin("nord").unwrap();
        let p = Palette::of(nord, 1);
        assert_eq!(p.lane_text[5], Col::of_hex(0x2e3440).hsla(), "dark ink on a pale lane");
    }
}
