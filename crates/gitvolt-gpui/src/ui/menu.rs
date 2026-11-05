//! Context menus and dropdowns (`ui/src/menu/*`): rows with icons, shortcuts and variants, separators,
//! headers and submenus; the keyboard; press, drag and release to pick; a row takes the pointer at
//! once unless it's heading into the open submenu (menu-aim).
//!
//! ```ignore
//! on_mouse_down(MouseButton::Right, |e, window, cx| {
//!     menu::open_context(rows(), e.position, window, cx);
//! })
//! // A dropdown under a `ui::anchor("toolbar.fetch", ..)`, opened by its press:
//! menu::open_below("toolbar.fetch", rows(), Default::default(), window, cx);
//! ```
//!
//! A row's `run` gets the window; it arms itself with `arm::request(req, Origin::current(cx), ..)`, and
//! then the menu stays open with the row armed. Without arming, the menu closes after `run`.

use super::Ui;
use super::arm;
use super::place::{self, Rect};
use super::tooltip::{self, Placement};
use crate::app::icon;
use crate::theme;
use gpui::prelude::*;
use gpui::*;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub const ROW_H: f32 = 26.;
const SEP_H: f32 = 9.;
const PAD_Y: f32 = 4.;
const BORDER: f32 = 1.;
const ROOT_MIN_W: f32 = 220.;
const SUB_MIN_W: f32 = 200.;
const OPTION_H: f32 = 28.;
const NOTE_W: f32 = 264.;
const NOTE_LINE_H: f32 = 15.;
const NOTE_PAD_Y: f32 = 10.;
/// The pointer may cross sibling rows on a diagonal into an open submenu: a row takes over once it
/// stops heading there, at once for any other move, or after this long without one.
pub const SUBMENU_GRACE: Duration = Duration::from_millis(300);
const AIM_TRAIL: usize = 3;
/// A release this near the press point is a click on the opener, not a drag onto an item.
pub const PRESS_SLOP: f32 = 4.;
/// A right press's menu opens under the pointer: how long before a still release picks.
pub const CONTEXT_GUARD: Duration = Duration::from_millis(150);
/// A window blur closes the menu only once it's stayed unfocused this long (a window manager's focus
/// bounce on a press).
pub const BLUR_SETTLE: Duration = Duration::from_millis(150);

pub type Run = super::Action;
pub type Builder = Rc<dyn Fn(&mut App) -> Vec<MenuRow>>;

/// An icon-only or short-label button at a row's right end.
#[derive(Clone)]
pub struct Variant {
    pub id: SharedString,
    pub label: Option<SharedString>,
    pub icon: Option<&'static str>,
    pub tooltip: SharedString,
    pub run: Run,
    pub disabled: Option<SharedString>,
}

#[derive(Clone)]
pub struct ActionRow {
    pub id: SharedString,
    pub label: SharedString,
    pub icon: &'static str,
    pub tooltip: SharedString,
    pub run: Run,
    pub variants: Vec<Variant>,
    /// The variant the row itself runs: lit while the row's body has the pointer or the keyboard.
    pub default_variant: Option<SharedString>,
    /// Why it can't be used: greyed, with this as its tooltip.
    pub disabled: Option<SharedString>,
    pub shortcut: Option<SharedString>,
    pub swatch: Option<Hsla>,
    /// A choice among its siblings: a radio instead of the icon, green when it's the one.
    pub radio: Option<bool>,
}

#[derive(Clone)]
pub enum MenuRow {
    Action(ActionRow),
    Submenu { id: SharedString, label: SharedString, icon: &'static str, tooltip: SharedString, rows: Rc<Vec<MenuRow>>, initial: Option<SharedString> },
    /// Names what the menu is about; not focusable.
    Header { id: SharedString, label: SharedString, detail: Option<SharedString>, swatch: Option<Hsla> },
    /// A line or `lines` of small text on what the rows below choose.
    Note { label: SharedString, lines: u8 },
    Separator,
}

/// The reason a row not ported yet stays disabled.
pub const NOT_YET: &str = "Not in this build yet";

impl MenuRow {
    pub fn action(id: impl Into<SharedString>, label: impl Into<SharedString>, icon: &'static str, tooltip: impl Into<SharedString>, run: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        MenuRow::Action(ActionRow { id: id.into(), label: label.into(), icon, tooltip: tooltip.into(), run: Rc::new(run), variants: Vec::new(), default_variant: None, disabled: None, shortcut: None, swatch: None, radio: None })
    }

    /// In its place, greyed: its action needs what isn't ported yet.
    pub fn pending(id: impl Into<SharedString>, label: impl Into<SharedString>, icon: &'static str) -> Self {
        MenuRow::action(id, label, icon, SharedString::default(), |_, _| {}).disabled(NOT_YET)
    }

    pub fn submenu(id: impl Into<SharedString>, label: impl Into<SharedString>, icon: &'static str, tooltip: impl Into<SharedString>, rows: Vec<MenuRow>) -> Self {
        MenuRow::Submenu { id: id.into(), label: label.into(), icon, tooltip: tooltip.into(), rows: Rc::new(rows), initial: None }
    }

    pub fn header(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        MenuRow::Header { id: id.into(), label: label.into(), detail: None, swatch: None }
    }

    pub fn note(label: impl Into<SharedString>, lines: u8) -> Self {
        MenuRow::Note { label: label.into(), lines }
    }

    pub fn radio(mut self, on: bool) -> Self {
        if let MenuRow::Action(a) = &mut self {
            a.radio = Some(on);
        }
        self
    }

    pub fn disabled(mut self, why: impl Into<SharedString>) -> Self {
        if let MenuRow::Action(a) = &mut self {
            a.disabled = Some(why.into());
        }
        self
    }

    pub fn disabled_if(self, why: Option<impl Into<SharedString>>) -> Self {
        match why {
            Some(w) => self.disabled(w),
            None => self,
        }
    }

    pub fn shortcut(mut self, chord: impl Into<SharedString>) -> Self {
        if let MenuRow::Action(a) = &mut self {
            a.shortcut = Some(chord.into());
        }
        self
    }

    pub fn swatch(mut self, color: Hsla) -> Self {
        if let MenuRow::Action(a) = &mut self {
            a.swatch = Some(color);
        }
        self
    }

    pub fn variants(mut self, variants: Vec<Variant>, default_variant: Option<&str>) -> Self {
        if let MenuRow::Action(a) = &mut self {
            a.variants = variants;
            a.default_variant = default_variant.map(|d| SharedString::from(d.to_string()));
        }
        self
    }

    fn id(&self) -> Option<&SharedString> {
        match self {
            MenuRow::Action(a) => Some(&a.id),
            MenuRow::Submenu { id, .. } | MenuRow::Header { id, .. } => Some(id),
            MenuRow::Note { .. } | MenuRow::Separator => None,
        }
    }

    fn enabled(&self) -> bool {
        match self {
            MenuRow::Action(a) => a.disabled.is_none(),
            MenuRow::Submenu { .. } => true,
            MenuRow::Header { .. } | MenuRow::Note { .. } | MenuRow::Separator => false,
        }
    }

    fn interactive(&self) -> bool {
        matches!(self, MenuRow::Action(_) | MenuRow::Submenu { .. })
    }

    fn tip(&self) -> SharedString {
        match self {
            MenuRow::Action(a) => a.disabled.clone().unwrap_or_else(|| a.tooltip.clone()),
            MenuRow::Submenu { tooltip, .. } => tooltip.clone(),
            _ => SharedString::default(),
        }
    }

    fn height(&self) -> f32 {
        match self {
            MenuRow::Separator => SEP_H,
            MenuRow::Note { lines, .. } => NOTE_PAD_Y + *lines as f32 * NOTE_LINE_H,
            _ => ROW_H,
        }
    }
}

/// Whether `p` lies inside the triangle (either winding, edges included).
fn in_triangle(p: (f32, f32), t: [(f32, f32); 3]) -> bool {
    let cross = |o: (f32, f32), a: (f32, f32), b: (f32, f32)| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let (d1, d2, d3) = (cross(t[0], t[1], p), cross(t[1], t[2], p), cross(t[2], t[0], p));
    !((d1 < 0. || d2 < 0. || d3 < 0.) && (d1 > 0. || d2 > 0. || d3 > 0.))
}

/// Whether the move `from` → `to` heads into the submenu's box: `to` lies in the triangle from `from`
/// to the submenu's near edge (4 px past its corners). `None`: the pointer didn't move.
pub fn aims_at(from: (f32, f32), to: (f32, f32), sub: Rect) -> Option<bool> {
    if from == to {
        return None;
    }
    let edge = if sub.left >= from.0 { sub.left } else { sub.right };
    Some(in_triangle(to, [from, (edge, sub.top - 4.), (edge, sub.bottom + 4.)]))
}

fn first_enabled(rows: &[MenuRow]) -> usize {
    rows.iter().position(MenuRow::enabled).unwrap_or(0)
}

fn start_index(rows: &[MenuRow], initial: Option<&str>) -> usize {
    let at = initial.and_then(|i| rows.iter().position(|r| r.id().is_some_and(|id| id.as_ref() == i)));
    match at {
        Some(i) if rows[i].enabled() => i,
        _ => first_enabled(rows),
    }
}

fn step(rows: &[MenuRow], from: usize, down: bool) -> usize {
    let n = rows.len();
    for i in 1..=n {
        let j = if down { (from + i) % n } else { (from + n - i % n) % n };
        if rows[j].enabled() {
            return j;
        }
    }
    from
}

/// One open level: the root, then each open submenu.
pub struct Level {
    pub rows: Rc<Vec<MenuRow>>,
    pub active: usize,
    /// The variant the keyboard's on.
    pub variant: Option<usize>,
    pub left: f32,
    pub top: f32,
    pub w: f32,
    pub h: f32,
    /// The row it hangs from, and that row's index a level up.
    anchor: Option<Rect>,
    parent: Option<usize>,
}

impl Level {
    fn rect(&self) -> Rect {
        Rect { left: self.left, right: self.left + self.w, top: self.top, bottom: self.top + self.h }
    }

    /// The row at `y` (window coordinates) and its top.
    fn row_at(&self, y: f32, option_after: Option<usize>) -> Option<usize> {
        let mut at = self.top + BORDER + PAD_Y;
        for (i, r) in self.rows.iter().enumerate() {
            let h = r.height();
            if y >= at && y < at + h {
                return Some(i);
            }
            at += h;
            if option_after == Some(i) {
                at += OPTION_H;
            }
        }
        None
    }

    fn row_top(&self, index: usize, option_after: Option<usize>) -> f32 {
        let mut at = self.top + BORDER + PAD_Y;
        for (i, r) in self.rows.iter().enumerate() {
            if i == index {
                return at;
            }
            at += r.height();
            if option_after == Some(i) {
                at += OPTION_H;
            }
        }
        at
    }

    fn row_rect(&self, index: usize, option_after: Option<usize>) -> Rect {
        let top = self.row_top(index, option_after);
        Rect { left: self.left, right: self.left + self.w, top, bottom: top + self.rows.get(index).map_or(ROW_H, MenuRow::height) }
    }

    /// The variant under `x` on row `i`.
    fn variant_at(&self, i: usize, x: f32, cx: &App) -> Option<usize> {
        let MenuRow::Action(a) = self.rows.get(i)? else { return None };
        if a.variants.is_empty() {
            return None;
        }
        let mut right = self.left + self.w - BORDER - 8.;
        for (j, v) in a.variants.iter().enumerate().rev() {
            let w = variant_w(v, cx);
            if x >= right - w && x < right {
                return Some(j);
            }
            right -= w;
        }
        None
    }
}

fn variant_w(v: &Variant, cx: &App) -> f32 {
    match (&v.icon, &v.label) {
        (Some(_), _) => 23.,
        (None, Some(l)) => (Ui::get(cx).measure(l, 11., FontWeight::NORMAL) + 10.).max(22.) + 1.,
        _ => 23.,
    }
}

/// The press that opened the menu, still held.
#[derive(Clone, Copy)]
struct Press {
    at: (f32, f32),
    right: bool,
    opened: Instant,
}

/// What's waiting while the pointer heads into the open submenu below `depth`.
struct Grace {
    depth: usize,
    pending: (usize, usize),
    generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    Row(usize, usize),
    Variant(usize, usize, usize),
    Option,
    Inside,
}

pub struct MenuState {
    pub levels: Vec<Level>,
    build: Option<Builder>,
    anchor: Option<SharedString>,
    target: Option<SharedString>,
    pub seq: u64,
    label: Option<SharedString>,
    press: Option<Press>,
    down: Option<Hit>,
    trail: Vec<(f32, f32)>,
    grace: Option<Grace>,
    grace_generation: u64,
    over: Option<(usize, usize)>,
    over_variant: Option<(usize, usize)>,
    /// The keyboard's on the armed row's option.
    on_option: bool,
    /// The row that's armed: (depth, index).
    armed_row: Option<(usize, usize)>,
    viewport: Size<Pixels>,
    blur_pending: bool,
}

#[derive(Default, Clone)]
pub struct OpenOptions {
    /// The row to start on, else the first enabled one.
    pub initial: Option<SharedString>,
    /// Re-run by `refresh`.
    pub build: Option<Builder>,
    pub label: Option<SharedString>,
}

fn level_size(rows: &[MenuRow], min_w: f32, cx: &App) -> (f32, f32) {
    let ui = Ui::get(cx);
    let mut w = min_w;
    let mut h = 2. * (PAD_Y + BORDER);
    for r in rows {
        h += r.height();
        let row = match r {
            MenuRow::Separator => 0.,
            MenuRow::Note { label, .. } => 24. + ui.measure(label, 11., FontWeight::NORMAL).min(NOTE_W),
            MenuRow::Header { label, detail, swatch, .. } => 18. + ui.measure(label, 13., FontWeight::NORMAL) + swatch.map_or(0., |_| 18.) + detail.as_ref().map_or(0., |d| 24. + ui.measure(d, 11., FontWeight::NORMAL)),
            MenuRow::Submenu { label, .. } => 18. + 22. + ui.measure(label, 13., FontWeight::NORMAL) + 20.,
            MenuRow::Action(a) => {
                let variants: f32 = a.variants.iter().map(|v| variant_w(v, cx)).sum();
                18. + 22.
                    + ui.measure(&a.label, 13., FontWeight::NORMAL)
                    + a.swatch.map_or(0., |_| 16.)
                    + a.shortcut.as_ref().map_or(0., |s| 24. + ui.measure(s, 11., FontWeight::NORMAL))
                    + if a.variants.is_empty() { 0. } else { 20. + variants }
            }
        };
        w = w.max(row.ceil() + 2. * BORDER);
    }
    (w, h)
}

impl MenuState {
    fn option_after(&self, depth: usize, cx: &App) -> Option<usize> {
        let (d, i) = self.armed_row?;
        (d == depth && Ui::get(cx).arm.armed.as_ref().is_some_and(|a| a.req.option.is_some())).then_some(i)
    }

    fn hit(&self, p: (f32, f32), cx: &App) -> Option<Hit> {
        for (d, lv) in self.levels.iter().enumerate().rev() {
            if !lv.rect().contains(p.0, p.1) {
                continue;
            }
            let option = self.option_after(d, cx);
            if let Some(i) = option {
                let top = lv.row_top(i, option) + ROW_H;
                if p.1 >= top && p.1 < top + OPTION_H {
                    return Some(Hit::Option);
                }
            }
            let Some(i) = lv.row_at(p.1, option) else { return Some(Hit::Inside) };
            return Some(match lv.variant_at(i, p.0, cx) {
                Some(v) => Hit::Variant(d, i, v),
                None => Hit::Row(d, i),
            });
        }
        None
    }
}

fn with_menu<R>(cx: &mut App, f: impl FnOnce(&mut MenuState) -> R) -> Option<R> {
    Ui::get_mut(cx).menu.as_mut().map(f)
}

fn open(rows: Vec<MenuRow>, at: (f32, f32), anchor: Option<SharedString>, o: OpenOptions, target: Option<SharedString>, window: &mut Window, cx: &mut App) -> bool {
    if rows.is_empty() {
        return false;
    }
    arm::disarm(window, cx);
    tooltip::hide(cx);
    let viewport = window.viewport_size();
    let (vw, vh) = (viewport.width.as_f32(), viewport.height.as_f32());
    let (w, h) = level_size(&rows, ROOT_MIN_W, cx);
    let bounds = anchor.as_ref().and_then(|k| Ui::get(cx).anchor(k)).map(Rect::of);
    let (left, top) = place::place_menu(at.0, at.1, w, h, vw, vh, bounds);
    let active = start_index(&rows, o.initial.as_deref());
    let press = Ui::get(cx).held_press().map(|(p, right)| Press { at: p, right, opened: Instant::now() });
    let ui = Ui::get_mut(cx);
    ui.menu_seq += 1;
    let seq = ui.menu_seq;
    ui.menu = Some(MenuState {
        levels: vec![Level { rows: Rc::new(rows), active, variant: None, left, top, w, h, anchor: None, parent: None }],
        build: o.build,
        anchor,
        target,
        seq,
        label: o.label,
        press,
        down: None,
        trail: Vec::new(),
        grace: None,
        grace_generation: 0,
        over: None,
        over_variant: None,
        on_option: false,
        armed_row: None,
        viewport,
        blur_pending: false,
    });
    cx.refresh_windows();
    true
}

/// Opens at the pointer. `true` when it opened.
pub fn open_context(rows: Vec<MenuRow>, at: Point<Pixels>, window: &mut Window, cx: &mut App) -> bool {
    open(rows, (at.x.as_f32(), at.y.as_f32()), None, OpenOptions::default(), None, window, cx)
}

/// `open_context`, the row it's for marked (`is_target`) while it's open. A right-click never moves
/// a list's selection; the row gets an outline.
pub fn open_context_for(key: impl Into<SharedString>, rows: Vec<MenuRow>, at: Point<Pixels>, window: &mut Window, cx: &mut App) -> bool {
    open(rows, (at.x.as_f32(), at.y.as_f32()), None, OpenOptions::default(), Some(key.into()), window, cx)
}

/// Opens under an `anchor`, which toggles it: a press on it while it's open just closes it.
pub fn open_below(key: impl Into<SharedString>, rows: Vec<MenuRow>, o: OpenOptions, window: &mut Window, cx: &mut App) -> bool {
    let key = key.into();
    if Ui::get(cx).swallow_anchor.as_ref() == Some(&key) {
        return false;
    }
    open(rows, (0., 0.), Some(key), o, None, window, cx)
}

/// Whether the press now going down just closed `key`'s menu.
pub fn press_closed(key: &str, cx: &App) -> bool {
    Ui::get(cx).swallow_anchor.as_ref().is_some_and(|k| k.as_ref() == key)
}

pub fn is_open(cx: &App) -> bool {
    Ui::get(cx).menu.is_some()
}

/// Whether `key`'s menu is the open one, for an opener's look.
pub fn is_open_on(key: &str, cx: &App) -> bool {
    Ui::get(cx).menu.as_ref().is_some_and(|m| m.anchor.as_ref().is_some_and(|a| a.as_ref() == key))
}

/// Whether the menu open now is for the row `key` (a right-click's outline).
pub fn is_target(key: &str, cx: &App) -> bool {
    Ui::get(cx).menu.as_ref().is_some_and(|m| m.target.as_ref().is_some_and(|t| t.as_ref() == key))
}

/// What's open, for a test: the deepest level's rows, active marked.
pub fn shown(cx: &App) -> Vec<Vec<String>> {
    Ui::get(cx)
        .menu
        .as_ref()
        .map(|m| {
            m.levels
                .iter()
                .map(|l| {
                    l.rows
                        .iter()
                        .enumerate()
                        .map(|(i, r)| {
                            let label = match r {
                                MenuRow::Action(a) => a.label.to_string(),
                                MenuRow::Submenu { label, .. } | MenuRow::Header { label, .. } | MenuRow::Note { label, .. } => label.to_string(),
                                MenuRow::Separator => "-".to_string(),
                            };
                            if i == l.active { format!("*{label}") } else { label }
                        })
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Opens the root level's row `index`'s submenu: a scripted screenshot's stand-in for the pointer.
pub fn open_sub_for_screenshot(index: usize, window: &Window, cx: &mut App) {
    with_menu(cx, |m| m.levels[0].active = index);
    open_sub(0, index, window, cx);
}

/// Closes it; what's armed on a row goes too.
pub fn close(window: &mut Window, cx: &mut App) {
    if arm::armed_on_row(cx) {
        arm::disarm(window, cx);
    }
    tooltip::hide(cx);
    if Ui::get_mut(cx).menu.take().is_some() {
        cx.refresh_windows();
    }
}

/// Runs the menu's builder again: the rows update, the open submenus and the active rows stay.
pub fn refresh(window: &mut Window, cx: &mut App) {
    let Some(build) = Ui::get(cx).menu.as_ref().and_then(|m| m.build.clone()) else { return };
    let rows = build(cx);
    if rows.is_empty() {
        return;
    }
    if arm::armed_on_row(cx) {
        arm::disarm(window, cx);
    }
    let (root_w, root_h) = level_size(&rows, ROOT_MIN_W, cx);
    let viewport = window.viewport_size();
    let (vw, vh) = (viewport.width.as_f32(), viewport.height.as_f32());
    let Some(old) = Ui::get_mut(cx).menu.as_mut().map(|m| std::mem::take(&mut m.levels)) else { return };
    let mut out: Vec<Level> = Vec::with_capacity(old.len());
    let mut cur = Rc::new(rows);
    let mut initial: Option<SharedString> = None;
    for (d, o) in old.iter().enumerate() {
        let id = o.rows.get(o.active).and_then(MenuRow::id).cloned();
        let found = id.as_ref().and_then(|id| cur.iter().position(|r| r.id() == Some(id)));
        let active = match found {
            Some(i) if cur[i].enabled() || matches!(cur[i], MenuRow::Submenu { .. }) => i,
            _ => start_index(&cur, initial.as_deref()),
        };
        let (w, h) = if d == 0 { (root_w, root_h) } else { level_size(&cur, SUB_MIN_W, cx) };
        let (left, top) = match o.anchor {
            Some(a) if d > 0 => place::place_submenu(a, w, h, vw, vh),
            _ => (o.left, o.top),
        };
        let variant = o.variant.filter(|v| Some(active) == found && matches!(&cur[active], MenuRow::Action(a) if *v < a.variants.len()));
        let parent = (d > 0).then(|| out[d - 1].active);
        out.push(Level { rows: cur.clone(), active, variant, left, top, w, h, anchor: o.anchor, parent });
        let next = old.get(d + 1);
        let MenuRow::Submenu { rows, initial: i, id: sub_id, .. } = &cur[active] else { break };
        if next.is_none() || Some(sub_id) != id.as_ref() {
            break;
        }
        initial = i.clone();
        cur = rows.clone();
    }
    if let Some(m) = Ui::get_mut(cx).menu.as_mut() {
        m.levels = out;
    }
    cx.refresh_windows();
}

/// A row's tooltip sits beside the menu, on the side away from its parent level.
fn tip_row(depth: usize, index: usize, cx: &mut App) {
    if arm::armed_on_row(cx) {
        return;
    }
    let Some(m) = Ui::get(cx).menu.as_ref() else { return };
    let Some(lv) = m.levels.get(depth) else { return };
    let Some(row) = lv.rows.get(index) else { return };
    let tip = row.tip();
    if tip.is_empty() {
        return tooltip::hide(cx);
    }
    let leftward = lv.anchor.is_some_and(|a| lv.left < a.left);
    let r = lv.row_rect(index, m.option_after(depth, cx));
    let bounds = Bounds::from_corners(point(px(r.left), px(r.top)), point(px(r.right), px(r.bottom)));
    tooltip::show_at(tip, bounds, if leftward { Placement::Left } else { Placement::Right }, cx);
}

fn tip_variant(depth: usize, index: usize, variant: usize, cx: &mut App) {
    if arm::armed_on_row(cx) {
        return;
    }
    let Some(m) = Ui::get(cx).menu.as_ref() else { return };
    let Some(lv) = m.levels.get(depth) else { return };
    let Some(MenuRow::Action(a)) = lv.rows.get(index) else { return };
    let Some(v) = a.variants.get(variant) else { return };
    let tip = v.disabled.clone().unwrap_or_else(|| v.tooltip.clone());
    let r = lv.row_rect(index, m.option_after(depth, cx));
    let mut right = r.right - BORDER - 8.;
    for vv in a.variants[variant + 1..].iter().rev() {
        right -= variant_w(vv, cx);
    }
    let w = variant_w(v, cx);
    let bounds = Bounds::from_corners(point(px(right - w), px(r.top + 3.)), point(px(right), px(r.bottom - 3.)));
    tooltip::show_at(tip, bounds, Placement::Below, cx);
}

/// Opens the submenu of row `index` at `depth` now, kept as it is if it's open already.
fn open_sub(depth: usize, index: usize, window: &Window, cx: &mut App) {
    tooltip::hide(cx);
    let viewport = window.viewport_size();
    let (vw, vh) = (viewport.width.as_f32(), viewport.height.as_f32());
    let Some(m) = Ui::get(cx).menu.as_ref() else { return };
    let Some(lv) = m.levels.get(depth) else { return };
    let Some(MenuRow::Submenu { rows, initial, .. }) = lv.rows.get(index) else { return };
    let rows = rows.clone();
    let start = start_index(&rows, initial.as_deref());
    let a = lv.row_rect(index, m.option_after(depth, cx));
    let already = m.levels.get(depth + 1).is_some_and(|l| l.parent == Some(index) && Rc::ptr_eq(&l.rows, &rows));
    let (w, h) = level_size(&rows, SUB_MIN_W, cx);
    let (left, top) = place::place_submenu(a, w, h, vw, vh);
    with_menu(cx, |m| {
        m.levels[depth].active = index;
        m.levels[depth].variant = None;
        if already {
            m.levels.truncate(depth + 2);
        } else {
            m.levels.truncate(depth + 1);
            m.levels.push(Level { rows, active: start, variant: None, left, top, w, h, anchor: Some(a), parent: Some(index) });
        }
    });
    cx.refresh_windows();
}

fn row_enter_now(depth: usize, index: usize, window: &Window, cx: &mut App) {
    let Some(m) = Ui::get_mut(cx).menu.as_mut() else { return };
    let Some(lv) = m.levels.get_mut(depth) else { return };
    let Some(row) = lv.rows.get(index) else { return };
    let is_sub = matches!(row, MenuRow::Submenu { .. });
    lv.active = index;
    lv.variant = None;
    let sub_open = m.levels.get(depth + 1).is_some_and(|l| l.parent == Some(index));
    if is_sub {
        if sub_open {
            // Back on the open submenu's own row: keep it, and no tooltip over it.
            tooltip::hide(cx);
            cx.refresh_windows();
            return;
        }
        tip_row(depth, index, cx);
        open_sub(depth, index, window, cx);
        return;
    }
    m.levels.truncate(depth + 1);
    tip_row(depth, index, cx);
    cx.refresh_windows();
}

fn end_grace(apply: bool, window: &Window, cx: &mut App) {
    let Some(g) = with_menu(cx, |m| m.grace.take()).flatten() else { return };
    if apply {
        row_enter_now(g.pending.0, g.pending.1, window, cx);
    }
}

/// The rest timer: without a move toward the submenu for this long, the row under the pointer takes over.
fn arm_grace(window: &Window, cx: &mut App) {
    let Some(generation) = with_menu(cx, |m| {
        m.grace_generation += 1;
        if let Some(g) = m.grace.as_mut() {
            g.generation = m.grace_generation;
        }
        m.grace_generation
    }) else {
        return;
    };
    let handle = window.window_handle();
    let timer = cx.background_executor().timer(SUBMENU_GRACE);
    cx.spawn(async move |cx| {
        timer.await;
        handle
            .update(cx, |_, window, cx| {
                if Ui::get(cx).menu.as_ref().and_then(|m| m.grace.as_ref()).is_some_and(|g| g.generation == generation) {
                    end_grace(true, window, cx);
                }
            })
            .ok();
    })
    .detach();
}

fn aiming(depth: usize, p: (f32, f32), cx: &App) -> Option<bool> {
    let m = Ui::get(cx).menu.as_ref()?;
    let from = *m.trail.first()?;
    let sub = m.levels.get(depth + 1)?;
    aims_at(from, p, sub.rect())
}

pub(super) fn on_move(e: &MouseMoveEvent, window: &mut Window, cx: &mut App) {
    if Ui::get(cx).menu.is_none() {
        return;
    }
    let p = (e.position.x.as_f32(), e.position.y.as_f32());
    if Ui::get(cx).menu.as_ref().and_then(|m| m.trail.last().copied()) == Some(p) {
        return;
    }
    // Each move still toward the submenu restarts the rest timer; any other hands over at once.
    if let Some(depth) = Ui::get(cx).menu.as_ref().and_then(|m| m.grace.as_ref().map(|g| g.depth)) {
        match aiming(depth, p, cx) {
            Some(true) => arm_grace(window, cx),
            Some(false) => end_grace(true, window, cx),
            None => {}
        }
    }
    let hit = Ui::get(cx).menu.as_ref().and_then(|m| m.hit(p, cx));
    let (row, variant) = match hit {
        Some(Hit::Row(d, i)) => (Some((d, i)), None),
        Some(Hit::Variant(d, i, v)) => (Some((d, i)), Some((d, i, v))),
        _ => (None, None),
    };
    let was = Ui::get(cx).menu.as_ref().and_then(|m| m.over);
    let was_variant = Ui::get(cx).menu.as_ref().and_then(|m| m.over_variant);
    with_menu(cx, |m| {
        m.over = row;
        m.over_variant = variant.map(|(d, i, _)| (d, i));
        m.trail.push(p);
        if m.trail.len() > AIM_TRAIL {
            m.trail.remove(0);
        }
    });
    if was != row {
        if let Some((d, i)) = row {
            on_row_enter(d, i, p, window, cx);
        } else if !Ui::get(cx).menu.as_ref().is_some_and(|m| m.grace.is_some()) {
            // Off the rows: the tooltip goes unless it's an open submenu's own row.
            tooltip::hide(cx);
        }
        cx.refresh_windows();
    }
    if let Some((d, i, v)) = variant {
        if was_variant != Some((d, i)) || was != row {
            tip_variant(d, i, v, cx);
            cx.refresh_windows();
        } else {
            tip_variant(d, i, v, cx);
        }
    } else if was_variant.is_some() {
        if let Some((d, i)) = row {
            tip_row(d, i, cx);
        }
        cx.refresh_windows();
    }
}

fn on_row_enter(depth: usize, index: usize, p: (f32, f32), window: &Window, cx: &mut App) {
    let Some(m) = Ui::get(cx).menu.as_ref() else { return };
    let Some(lv) = m.levels.get(depth) else { return };
    if !lv.rows.get(index).is_some_and(MenuRow::interactive) {
        return;
    }
    // Reaching a submenu's level: it stays open, its row active again.
    if depth > 0 {
        let parent = lv.parent;
        with_menu(cx, |m| {
            m.grace = None;
            m.levels.truncate(depth + 1);
            if let Some(parent) = parent {
                m.levels[depth - 1].active = parent;
                m.levels[depth - 1].variant = None;
            }
        });
    }
    let Some(m) = Ui::get(cx).menu.as_ref() else { return };
    let has_sub = m.levels.len() > depth + 1;
    let sub_here = m.levels.get(depth + 1).is_some_and(|l| l.parent == Some(index));
    // A sibling of the open submenu's row, crossed on the way into the submenu: nothing changes yet.
    if has_sub && !sub_here && aiming(depth, p, cx) == Some(true) {
        with_menu(cx, |m| match m.grace.as_mut() {
            Some(g) if g.depth == depth => g.pending = (depth, index),
            _ => m.grace = Some(Grace { depth, pending: (depth, index), generation: 0 }),
        });
        arm_grace(window, cx);
        return;
    }
    end_grace(false, window, cx);
    row_enter_now(depth, index, window, cx);
}

pub(super) fn on_press(e: &MouseDownEvent, window: &mut Window, cx: &mut App) {
    // A press carries on to what it hit. Its release may be the toggle's.
    let p = (e.position.x.as_f32(), e.position.y.as_f32());
    let ui = Ui::get_mut(cx);
    ui.held = Some((p, e.button == MouseButton::Right));
    ui.swallow_anchor = None;
    let Some(m) = Ui::get(cx).menu.as_ref() else { return };
    let hit = m.hit(p, cx);
    let armed_row = m.armed_row;
    match hit {
        Some(h) => {
            // A press on another row while one is armed only disarms it.
            let on_armed = match h {
                Hit::Row(d, i) | Hit::Variant(d, i, _) => armed_row == Some((d, i)),
                Hit::Option => true,
                Hit::Inside => false,
            };
            if armed_row.is_some() && arm::armed_on_row(cx) && !on_armed {
                arm::disarm(window, cx);
                with_menu(cx, |m| {
                    m.down = None;
                    m.armed_row = None;
                    m.press = None;
                });
                return;
            }
            with_menu(cx, |m| {
                m.down = Some(h);
                m.press = None;
            });
        }
        None => {
            let anchor = m.anchor.clone();
            let on_anchor = anchor.as_ref().and_then(|k| Ui::get(cx).anchor(k)).is_some_and(|b| b.contains(&e.position));
            if on_anchor {
                Ui::get_mut(cx).swallow_anchor = anchor;
            }
            close(window, cx);
        }
    }
}

pub(super) fn on_release(e: &MouseUpEvent, window: &mut Window, cx: &mut App) {
    let p = (e.position.x.as_f32(), e.position.y.as_f32());
    let ui = Ui::get_mut(cx);
    ui.held = None;
    let swallow = ui.swallow_anchor.take();
    let _ = swallow;
    let Some(m) = ui.menu.as_mut() else { return };
    let press = m.press.take();
    let down = m.down.take();
    let hit = Ui::get(cx).menu.as_ref().and_then(|m| m.hit(p, cx));
    if let Some(press) = press {
        // The release of the press that opened it.
        let far = ((p.0 - press.at.0).powi(2) + (p.1 - press.at.1).powi(2)).sqrt() >= PRESS_SLOP;
        if !far && !(press.right && press.opened.elapsed() >= CONTEXT_GUARD) {
            return;
        }
        if let Some(hit) = hit {
            click(hit, window, cx);
        }
    } else if let (Some(a), Some(b)) = (down, hit)
        && a == b
    {
        click(a, window, cx);
    }
}

/// A row, a variant or the armed row's option clicked.
fn click(hit: Hit, window: &mut Window, cx: &mut App) {
    match hit {
        Hit::Row(d, i) => activate_row(d, i, false, window, cx),
        Hit::Variant(d, i, v) => activate_variant(d, i, v, false, window, cx),
        Hit::Option => {
            let checked = Ui::get(cx).arm.armed.as_ref().is_some_and(|a| !a.checked);
            arm::set_option(checked, cx);
        }
        Hit::Inside => {}
    }
}

fn activate_row(depth: usize, index: usize, by_key: bool, window: &mut Window, cx: &mut App) {
    let Some(m) = Ui::get(cx).menu.as_ref() else { return };
    let Some(row) = m.levels.get(depth).and_then(|l| l.rows.get(index)).cloned() else { return };
    match row {
        MenuRow::Submenu { .. } => open_sub(depth, index, window, cx),
        MenuRow::Action(a) if a.disabled.is_none() => pick(depth, index, a.run, by_key, window, cx),
        _ => {}
    }
}

fn activate_variant(depth: usize, index: usize, variant: usize, by_key: bool, window: &mut Window, cx: &mut App) {
    let Some(m) = Ui::get(cx).menu.as_ref() else { return };
    let Some(MenuRow::Action(a)) = m.levels.get(depth).and_then(|l| l.rows.get(index)).cloned() else { return };
    if let Some(v) = a.variants.get(variant).filter(|v| v.disabled.is_none()) {
        pick(depth, index, v.run.clone(), by_key, window, cx);
    }
}

/// Runs a pick with its row as the origin: an action that asks arms the row and the menu stays open,
/// and the armed row's own second pick confirms it. Otherwise the menu closes after it.
fn pick(depth: usize, index: usize, run: Run, by_key: bool, window: &mut Window, cx: &mut App) {
    let Some(m) = Ui::get(cx).menu.as_ref() else { return };
    let seq = m.seq;
    if m.armed_row == Some((depth, index)) && arm::armed_on_row(cx) {
        if if by_key { arm::confirm_key(window, cx) } else { arm::confirm_click(window, cx) } {
            close(window, cx);
        }
        return;
    }
    Ui::get_mut(cx).arm.running_row = true;
    run(window, cx);
    Ui::get_mut(cx).arm.running_row = false;
    let held = std::mem::take(&mut Ui::get_mut(cx).arm.held);
    if arm::armed_on_row(cx) || held {
        with_menu(cx, |m| m.armed_row = Some((depth, index)));
        tooltip::hide(cx);
        cx.refresh_windows();
        return;
    }
    if Ui::get(cx).menu.as_ref().is_some_and(|m| m.seq == seq) {
        close(window, cx);
    }
}

pub(super) fn on_wheel(e: &ScrollEvent, _: &mut Window, cx: &mut App) {
    // A scroll doesn't close it, and the wheel outside it moves nothing behind it.
    let Some(m) = Ui::get(cx).menu.as_ref() else { return };
    let p = (e.position.x.as_f32(), e.position.y.as_f32());
    if m.hit(p, cx).is_none() {
        cx.stop_propagation();
    }
}

pub(super) fn on_key(k: &Keystroke, window: &mut Window, cx: &mut App) {
    let Some(m) = Ui::get(cx).menu.as_ref() else { return };
    let depth = m.levels.len() - 1;
    let lv = &m.levels[depth];
    let rows = lv.rows.clone();
    let (active, variant) = (lv.active, lv.variant);
    let armed = arm::armed_on_row(cx);
    // The armed row's option: ↓ reaches it and ↑ goes back without disarming; Enter or Space toggles.
    let has_option = armed && m.armed_row == Some((depth, active)) && Ui::get(cx).arm.armed.as_ref().is_some_and(|a| a.req.option.is_some());
    let on_option = m.on_option;
    if has_option {
        match k.key.as_str() {
            "down" if !on_option => {
                with_menu(cx, |m| m.on_option = true);
                return cx.refresh_windows();
            }
            "up" if on_option => {
                with_menu(cx, |m| m.on_option = false);
                return cx.refresh_windows();
            }
            "enter" | "space" if on_option => {
                let checked = Ui::get(cx).arm.armed.as_ref().is_some_and(|a| !a.checked);
                return arm::set_option(checked, cx);
            }
            _ => {}
        }
    }
    let move_to = |to: usize, window: &mut Window, cx: &mut App| {
        if armed {
            arm::disarm(window, cx);
        }
        with_menu(cx, |m| {
            let lv = &mut m.levels[depth];
            lv.active = to;
            lv.variant = None;
            m.armed_row = None;
            m.on_option = false;
        });
        tip_row(depth, to, cx);
        cx.refresh_windows();
    };
    let to_variant = |v: Option<usize>, window: &mut Window, cx: &mut App| {
        if armed {
            arm::disarm(window, cx);
        }
        with_menu(cx, |m| m.levels[depth].variant = v);
        match v {
            Some(v) => tip_variant(depth, active, v, cx),
            None => tip_row(depth, active, cx),
        }
        cx.refresh_windows();
    };
    let variants: &[Variant] = match rows.get(active) {
        Some(MenuRow::Action(a)) => &a.variants,
        _ => &[],
    };
    match k.key.as_str() {
        "down" => move_to(step(&rows, active, true), window, cx),
        "up" => move_to(step(&rows, active, false), window, cx),
        "home" => move_to(step(&rows, rows.len() - 1, true), window, cx),
        "end" => move_to(step(&rows, 0, false), window, cx),
        "right" => {
            if matches!(rows.get(active), Some(MenuRow::Submenu { .. })) {
                open_sub(depth, active, window, cx);
            } else {
                let mut v = variant.map_or(0, |v| v + 1);
                while v < variants.len() && variants[v].disabled.is_some() {
                    v += 1;
                }
                if v < variants.len() {
                    to_variant(Some(v), window, cx);
                }
            }
        }
        "left" => {
            if let Some(cur) = variant {
                let prev = (0..cur).rev().find(|v| variants[*v].disabled.is_none());
                to_variant(prev, window, cx);
            } else if depth > 0 {
                tooltip::hide(cx);
                with_menu(cx, |m| {
                    m.levels.pop();
                });
                cx.refresh_windows();
            }
        }
        "enter" | "space" => match rows.get(active) {
            Some(MenuRow::Submenu { .. }) => open_sub(depth, active, window, cx),
            Some(MenuRow::Action(_)) => match variant {
                Some(v) => activate_variant(depth, active, v, true, window, cx),
                None => activate_row(depth, active, true, window, cx),
            },
            _ => {}
        },
        "escape" => {
            // Esc disarms an armed row first; the menu stays.
            if armed {
                arm::disarm(window, cx);
                with_menu(cx, |m| {
                    m.armed_row = None;
                    m.on_option = false;
                });
            } else if depth > 0 {
                tooltip::hide(cx);
                with_menu(cx, |m| {
                    m.levels.pop();
                });
                cx.refresh_windows();
            } else {
                close(window, cx);
            }
        }
        "tab" => close(window, cx),
        _ => {}
    }
}

fn watch_window(window: &mut Window, cx: &mut App) {
    let Some(m) = Ui::get(cx).menu.as_ref() else { return };
    // A resize closes it.
    if m.viewport != window.viewport_size() {
        return window.defer(cx, close);
    }
    // So does the window losing focus, once that lasts.
    if window.is_window_active() || m.blur_pending {
        return;
    }
    with_menu(cx, |m| m.blur_pending = true);
    let handle = window.window_handle();
    let seq = Ui::get(cx).menu_seq;
    let timer = cx.background_executor().timer(BLUR_SETTLE);
    cx.spawn(async move |cx| {
        timer.await;
        handle
            .update(cx, |_, window, cx| {
                if Ui::get(cx).menu_seq != seq {
                    return;
                }
                with_menu(cx, |m| m.blur_pending = false);
                if !window.is_window_active() {
                    close(window, cx);
                }
            })
            .ok();
    })
    .detach();
}

pub(super) fn element(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    Ui::get(cx).menu.as_ref()?;
    watch_window(window, cx);
    let m = Ui::get(cx).menu.as_ref()?;
    let armed = Ui::get(cx).arm.armed.as_ref().filter(|a| a.origin == arm::Origin::MenuRow);
    let mut root = div().absolute().top_0().left_0().size_0();
    for (depth, lv) in m.levels.iter().enumerate() {
        let mut level = div()
            .id(("menu-level", depth))
            .absolute()
            .left(px(lv.left))
            .top(px(lv.top))
            .w(px(lv.w))
            .min_h(px(lv.h))
            .occlude()
            .flex()
            .flex_col()
            .py(px(PAD_Y))
            .bg(theme::panel_bg1())
            .border_1()
            .border_color(theme::section_border())
            .rounded(px(6.))
            .shadow(super::shadow(theme::shadow_popup(), 6., 20.))
            .text_size(px(13.))
            .text_color(theme::text_normal());
        for (i, r) in lv.rows.iter().enumerate() {
            let active = i == lv.active;
            match r {
                MenuRow::Separator => level = level.child(div().flex_none().h(px(1.)).my(px(4.)).bg(theme::section_border())),
                MenuRow::Note { label, lines } => {
                    level = level.child(div().flex_none().h(px(r.height())).max_h(px(NOTE_PAD_Y + *lines as f32 * NOTE_LINE_H)).pt(px(4.)).pb(px(6.)).px(px(12.)).text_size(px(11.)).line_height(px(NOTE_LINE_H)).text_color(theme::text_secondary()).child(label.clone()));
                }
                MenuRow::Header { label, detail, swatch, .. } => {
                    level = level.child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(8.))
                            .h(px(ROW_H))
                            .pl(px(10.))
                            .pr(px(8.))
                            .whitespace_nowrap()
                            .text_color(theme::text_secondary())
                            .when_some(*swatch, |d, s| d.child(div().flex_none().size(px(10.)).rounded(px(3.)).bg(s).border_1().border_color(theme::section_border())))
                            .child(div().flex_1().child(label.clone()))
                            .when_some(detail.clone(), |d, t| d.child(div().ml_auto().font_family(crate::app::MONO_FONT).text_size(px(11.)).child(t))),
                    );
                }
                MenuRow::Submenu { label, icon: name, .. } => {
                    let color = if active { theme::text_selected() } else { theme::text_normal() };
                    level = level.child(
                        row_base(active, false)
                            .text_color(color)
                            .child(icon_slot(name, color))
                            .child(div().flex_1().child(label.clone()))
                            .child(icon("chevron-right", 12., theme::text_secondary())),
                    );
                }
                MenuRow::Action(a) => {
                    let disabled = a.disabled.is_some();
                    let color = if disabled { theme::text_disabled() } else if a.radio == Some(true) { theme::green() } else if active { theme::text_selected() } else { theme::text_normal() };
                    let armed_here = armed.is_some() && m.armed_row == Some((depth, i));
                    let over_variant = m.over_variant == Some((depth, i));
                    let mut row = row_base(active, disabled)
                        .text_color(color)
                        .child(match a.radio {
                            Some(on) => radio_mark(on),
                            None => icon_slot(a.icon, color),
                        })
                        .when_some(a.swatch, |d, s| d.child(div().flex_none().size(px(8.)).rounded_full().bg(s)))
                        .child(div().flex_1().child(a.label.clone()))
                        .when_some(a.shortcut.clone(), |d, s| d.child(div().ml(px(16.)).text_size(px(11.)).text_color(theme::text_secondary()).child(s)));
                    if !a.variants.is_empty() {
                        let mut group = div().flex().flex_none().ml(px(12.)).rounded(px(3.)).overflow_hidden();
                        for (j, v) in a.variants.iter().enumerate() {
                            let on = active && lv.variant == Some(j);
                            let lit = active && !disabled && lv.variant.is_none() && !over_variant && a.default_variant.as_ref() == Some(&v.id);
                            let vd = v.disabled.is_some();
                            let hovered = m.over_variant == Some((depth, i)) && m.over.is_some() && hovered_variant(lv, i, j, window, cx);
                            let bg = if vd { transparent_black() } else if on || hovered { theme::blue() } else if lit { theme::mix(theme::blue(), 0.7, theme::control_bg()) } else { theme::control_bg() };
                            let fg = if on || hovered || lit { theme::text_on_accent() } else { theme::text_secondary() };
                            group = group.child(
                                div()
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .justify_center()
                                    .min_w(px(22.))
                                    .h(px(20.))
                                    .px(px(5.))
                                    .when(j > 0, |d| d.border_l_1().border_color(theme::section_border()))
                                    .when(vd, |d| d.opacity(0.35))
                                    .bg(bg)
                                    .text_size(px(11.))
                                    .text_color(fg)
                                    .when_some(v.icon, |d, n| d.child(icon(n, 13., fg)))
                                    .when(v.icon.is_none(), |d| d.children(v.label.clone())),
                            );
                        }
                        row = row.child(group);
                    }
                    if armed_here && let Some(armed) = armed {
                        let tone = armed.req.tone.color();
                        let needed = Ui::get(cx).measure(&armed.req.arm, 13., FontWeight::SEMIBOLD) + 40.;
                        row = row.child(
                            div()
                                .absolute()
                                .left(px(2.))
                                .top(px(1.))
                                .bottom(px(1.))
                                .w(px(needed.max(lv.w - 6.)))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .px(px(8.))
                                .border_1()
                                .border_color(tone)
                                .rounded(px(4.))
                                .bg(theme::mix(tone, 0.22, theme::panel_bg1()))
                                .text_color(theme::mix(tone, 0.35, theme::text_selected()))
                                .font_weight(FontWeight::SEMIBOLD)
                                .whitespace_nowrap()
                                .shadow(super::shadow(theme::shadow_popup(), 2., 8.))
                                .child(icon_slot(a.icon, theme::mix(tone, 0.35, theme::text_selected())))
                                .child(armed.req.arm.clone()),
                        );
                    }
                    level = level.child(row);
                    if armed_here && let Some(option) = armed.and_then(|a| a.req.option.as_ref()) {
                        let checked = armed.is_some_and(|a| a.checked);
                        level = level.child(
                            div()
                                .flex()
                                .flex_none()
                                .items_start()
                                .gap(px(8.))
                                .pt(px(4.))
                                .pb(px(5.))
                                .pl(px(12.))
                                .pr(px(8.))
                                .h(px(OPTION_H))
                                .when(m.on_option, |d| d.bg(theme::hover_row()).text_color(theme::text_selected()))
                                .child(super::controls::check_box(checked, 12.))
                                .child(div().flex_1().child(option.label.clone())),
                        );
                    }
                }
            }
        }
        root = root.child(level);
    }
    Some(root.into_any_element())
}

fn hovered_variant(lv: &Level, i: usize, j: usize, window: &Window, cx: &App) -> bool {
    let p = window.mouse_position();
    lv.variant_at(i, p.x.as_f32(), cx) == Some(j)
}

fn row_base(active: bool, _disabled: bool) -> Div {
    div().relative().flex().flex_none().items_center().gap(px(8.)).h(px(ROW_H)).pl(px(10.)).pr(px(8.)).whitespace_nowrap().when(active, |d| d.bg(theme::selected_row()))
}

/// For a script: where the open menu's row whose label holds `text` is, the deepest level first.
pub fn row_point(text: &str, cx: &App) -> Option<Point<Pixels>> {
    let menu = Ui::get(cx).menu.as_ref()?;
    menu.levels.iter().rev().find_map(|lv| {
        let i = lv.rows.iter().position(|r| match r {
            MenuRow::Action(a) => a.label.contains(text),
            MenuRow::Submenu { label, .. } => label.contains(text),
            _ => false,
        })?;
        Some(point(px(lv.left + 40.), px(lv.row_top(i, None) + lv.rows[i].height() / 2.)))
    })
}

/// For a script: where the variant `label` of the open menu's row holding `text` is.
pub fn variant_point(text: &str, label: &str, cx: &App) -> Option<Point<Pixels>> {
    let menu = Ui::get(cx).menu.as_ref()?;
    menu.levels.iter().rev().find_map(|lv| {
        let i = lv.rows.iter().position(|r| matches!(r, MenuRow::Action(a) if a.label.contains(text) && !a.variants.is_empty()))?;
        let MenuRow::Action(a) = &lv.rows[i] else { return None };
        let mut right = lv.left + lv.w - BORDER - 8.;
        for v in a.variants.iter().rev() {
            let w = variant_w(v, cx);
            if v.label.as_ref().is_some_and(|l| l.as_ref() == label) {
                return Some(point(px(right - w / 2.), px(lv.row_top(i, None) + lv.rows[i].height() / 2.)));
            }
            right -= w;
        }
        None
    })
}

/// `.tb-radio`.
fn radio_mark(on: bool) -> AnyElement {
    let ring = if on { theme::green() } else { theme::text_secondary() };
    div().flex().flex_none().size(px(14.)).items_center().justify_center().rounded_full().border(px(1.5)).border_color(ring).when(on, |d| d.child(div().size(px(7.)).rounded_full().bg(theme::green()))).into_any_element()
}

/// The row's icon, 14 px at 85%; a blank keeps the column (a select's unchosen rows).
fn icon_slot(name: &'static str, color: Hsla) -> AnyElement {
    if name.is_empty() {
        div().flex_none().size(px(14.)).into_any_element()
    } else {
        icon(name, 14., Hsla { a: color.a * 0.85, ..color }).into_any_element()
    }
}

/// `{X}` the HEAD branch, `{Y}` the clicked branch (`registry.ts`'s `tmpl`).
pub fn tmpl(label: &str, x: Option<&str>, y: Option<&str>) -> String {
    label.replace("{X}", x.filter(|s| !s.is_empty()).unwrap_or("HEAD")).replace("{Y}", y.filter(|s| !s.is_empty()).unwrap_or("this branch"))
}

/// Joins groups with separators, leaving out the empty ones: `buildMenu`'s grouping.
pub fn grouped(groups: Vec<Vec<MenuRow>>) -> Vec<MenuRow> {
    let mut out = Vec::new();
    for g in groups {
        if g.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(MenuRow::Separator);
        }
        out.extend(g);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::prelude::v1::test;

    type Log = Rc<RefCell<Vec<&'static str>>>;

    fn rows(log: &Log) -> Vec<MenuRow> {
        let a = log.clone();
        let b = log.clone();
        let c = log.clone();
        vec![
            MenuRow::action("one", "One", "check", "first", move |_, _| a.borrow_mut().push("one")),
            MenuRow::action("off", "Off", "check", "", |_, _| {}).disabled("nope"),
            MenuRow::Separator,
            MenuRow::submenu("sub", "Sub", "check", "more", vec![MenuRow::action("deep", "Deep", "check", "", move |_, _| b.borrow_mut().push("deep"))]),
            MenuRow::action("two", "Two", "check", "", move |_, _| c.borrow_mut().push("two")),
        ]
    }

    struct Host {
        log: Log,
    }
    impl Render for Host {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            super::super::begin_frame(window, cx);
            let log = self.log.clone();
            let log2 = self.log.clone();
            div()
                .size_full()
                .relative()
                .child(super::super::anchor(
                    "opener",
                    div().id("opener").w(px(60.)).h(px(24.)).on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        open_below("opener", rows(&log2), OpenOptions::default(), window, cx);
                    }),
                ))
                .child(div().id("field").w_full().h(px(400.)).on_mouse_down(MouseButton::Right, move |e, window, cx| {
                    open_context(rows(&log), e.position, window, cx);
                }))
                .children(super::super::overlays(window, cx))
        }
    }

    fn open_window(cx: &mut TestAppContext) -> (VisualTestContext, Log) {
        cx.update(super::super::init);
        let log: Log = Rc::default();
        let host = log.clone();
        let window = cx
            .update(|cx| {
                let bounds = Bounds::new(point(px(0.), px(0.)), size(px(1000.), px(800.)));
                cx.open_window(WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() }, move |_, cx| cx.new(|_| Host { log: host }))
            })
            .unwrap();
        let v = VisualTestContext::from_window(window.into(), cx);
        v.run_until_parked();
        (v, log)
    }

    fn levels(cx: &mut VisualTestContext) -> Vec<Vec<String>> {
        cx.update(|_, cx| shown(cx))
    }

    const FIRST: f32 = 200. + BORDER + PAD_Y + ROW_H / 2.;

    fn row_y(i: usize) -> f32 {
        // One, Off, separator, Sub, Two.
        let heights = [ROW_H, ROW_H, SEP_H, ROW_H, ROW_H];
        200. + BORDER + PAD_Y + heights[..i].iter().sum::<f32>() + heights[i] / 2.
    }

    #[test]
    fn the_aim_counts_a_move_into_the_submenus_edge_only() {
        let sub = Rect { left: 300., right: 500., top: 100., bottom: 400. };
        assert_eq!(aims_at((280., 150.), (290., 155.), sub), Some(true));
        assert_eq!(aims_at((280., 150.), (280., 180.), sub), Some(false), "straight down");
        assert_eq!(aims_at((280., 150.), (270., 150.), sub), Some(false), "away");
        assert_eq!(aims_at((280., 150.), (280., 150.), sub), None);
    }

    #[test]
    fn stepping_skips_what_cant_be_used_and_wraps() {
        let log: Log = Rc::default();
        let r = rows(&log);
        assert_eq!(first_enabled(&r), 0);
        assert_eq!(step(&r, 0, true), 3);
        assert_eq!(step(&r, 4, true), 0);
        assert_eq!(step(&r, 0, false), 4);
        assert_eq!(start_index(&r, Some("two")), 4);
        assert_eq!(start_index(&r, Some("off")), 0);
    }

    #[gpui::test]
    fn a_right_click_opens_and_a_row_click_picks_and_closes(cx: &mut TestAppContext) {
        let (mut cx, log) = open_window(cx);
        cx.simulate_mouse_down(point(px(100.), px(200.)), MouseButton::Right, Modifiers::none());
        cx.simulate_mouse_up(point(px(100.), px(200.)), MouseButton::Right, Modifiers::none());
        assert_eq!(levels(&mut cx), [["*One", "Off", "-", "Sub", "Two"]], "a still release doesn't pick");
        let at = point(px(150.), px(row_y(4)));
        cx.simulate_mouse_move(at, None, Modifiers::none());
        assert_eq!(levels(&mut cx)[0][4], "*Two");
        cx.simulate_click(at, Modifiers::none());
        assert_eq!(*log.borrow(), ["two"]);
        assert!(levels(&mut cx).is_empty());
    }

    #[gpui::test]
    fn a_held_row_keeps_its_menu_until_it_is_released(cx: &mut TestAppContext) {
        let (mut cx, _) = open_window(cx);
        let hold: Rc<std::cell::Cell<Option<u64>>> = Rc::default();
        let h = hold.clone();
        cx.update(|window, cx| {
            open_context(vec![MenuRow::action("ask", "Ask", "check", "", move |_, cx| h.set(arm::hold_row(cx)))], point(px(100.), px(200.)), window, cx);
        });
        cx.simulate_click(point(px(150.), px(row_y(0))), Modifiers::none());
        assert!(hold.get().is_some());
        assert_eq!(levels(&mut cx).len(), 1, "the action asks later");
        cx.update(|window, cx| arm::release_row(hold.get(), window, cx));
        assert!(levels(&mut cx).is_empty());
    }

    #[gpui::test]
    fn press_drag_release_picks(cx: &mut TestAppContext) {
        let (mut cx, log) = open_window(cx);
        cx.simulate_mouse_down(point(px(100.), px(200.)), MouseButton::Right, Modifiers::none());
        let at = point(px(150.), px(row_y(0)));
        cx.simulate_mouse_move(at, MouseButton::Right, Modifiers::none());
        cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
        assert_eq!(*log.borrow(), ["one"]);
    }

    #[gpui::test]
    fn a_press_on_the_dropdowns_opener_toggles_it(cx: &mut TestAppContext) {
        let (mut cx, _) = open_window(cx);
        let at = point(px(10.), px(10.));
        cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
        assert_eq!(levels(&mut cx).len(), 1, "open, and stays open after its press's release");
        cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
        assert!(levels(&mut cx).is_empty(), "the second press closes it, and doesn't reopen");
        cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
        assert_eq!(levels(&mut cx).len(), 1);
    }

    #[gpui::test]
    fn the_keyboard_moves_opens_submenus_and_picks(cx: &mut TestAppContext) {
        let (mut cx, log) = open_window(cx);
        cx.simulate_mouse_down(point(px(100.), px(200.)), MouseButton::Right, Modifiers::none());
        cx.simulate_mouse_up(point(px(100.), px(200.)), MouseButton::Right, Modifiers::none());
        cx.simulate_keystrokes("down");
        assert_eq!(levels(&mut cx)[0][3], "*Sub", "past the disabled row and the separator");
        cx.simulate_keystrokes("right");
        assert_eq!(levels(&mut cx)[1], ["*Deep"]);
        cx.simulate_keystrokes("escape");
        assert_eq!(levels(&mut cx).len(), 1, "Esc closes the submenu first");
        cx.simulate_keystrokes("enter");
        cx.simulate_keystrokes("enter");
        assert_eq!(*log.borrow(), ["deep"]);
        assert!(levels(&mut cx).is_empty());
    }

    #[gpui::test]
    fn a_submenu_opens_at_once_on_hover_and_survives_a_diagonal(cx: &mut TestAppContext) {
        let (mut cx, _) = open_window(cx);
        cx.simulate_mouse_down(point(px(100.), px(200.)), MouseButton::Right, Modifiers::none());
        cx.simulate_mouse_up(point(px(100.), px(200.)), MouseButton::Right, Modifiers::none());
        let (right, sub_y) = cx.update(|_, cx| (Ui::get(cx).menu.as_ref().unwrap().levels[0].rect().right, row_y(3)));
        // Onto Sub: open at once.
        cx.simulate_mouse_move(point(px(right - 40.), px(sub_y + 11.)), None, Modifiers::none());
        cx.simulate_mouse_move(point(px(right - 38.), px(sub_y + 11.)), None, Modifiers::none());
        assert_eq!(levels(&mut cx).len(), 2);
        // Toward the submenu, across Two: nothing changes yet.
        cx.simulate_mouse_move(point(px(right - 24.), px(row_y(4) - 11.)), None, Modifiers::none());
        assert_eq!(levels(&mut cx).len(), 2);
        assert_eq!(levels(&mut cx)[0][3], "*Sub");
        // Straight back up and down the rows: the row takes it at once.
        cx.simulate_mouse_move(point(px(right - 24.), px(row_y(0))), None, Modifiers::none());
        assert_eq!(levels(&mut cx).len(), 1);
        assert_eq!(levels(&mut cx)[0][0], "*One");
    }

    #[gpui::test]
    fn a_row_that_asks_arms_in_place_and_its_second_pick_confirms(cx: &mut TestAppContext) {
        let (mut cx, _) = open_window(cx);
        let done: Rc<RefCell<Vec<bool>>> = Rc::default();
        let log = done.clone();
        cx.update(|window, cx| {
            let row = MenuRow::action("del", "Delete", "check", "", move |window, cx| {
                let log = log.clone();
                let origin = arm::Origin::current(cx);
                arm::request(arm::ArmRequest::danger("Click again to delete", "Delete?", "Delete"), origin, move |a, _, _| log.borrow_mut().push(a.ok), window, cx);
            });
            open_context(vec![row], point(px(100.), px(200.)), window, cx);
        });
        cx.run_until_parked();
        let at = point(px(150.), px(FIRST));
        cx.simulate_click(at, Modifiers::none());
        assert!(cx.update(|_, cx| is_open(cx) && arm::armed_on_row(cx)), "armed, the menu stays");
        cx.update(|_, cx| arm::age_for_test(cx));
        cx.simulate_click(at, Modifiers::none());
        assert_eq!(*done.borrow(), [true]);
        assert!(!cx.update(|_, cx| is_open(cx)));
    }

    #[gpui::test]
    fn the_wheel_outside_it_moves_nothing(cx: &mut TestAppContext) {
        let (mut cx, _) = open_window(cx);
        cx.simulate_mouse_down(point(px(100.), px(200.)), MouseButton::Right, Modifiers::none());
        cx.simulate_mouse_up(point(px(100.), px(200.)), MouseButton::Right, Modifiers::none());
        cx.simulate_event(ScrollEvent { position: point(px(900.), px(700.)), delta: ScrollDelta::Pixels(point(px(0.), px(-40.))), ..Default::default() });
        assert_eq!(levels(&mut cx).len(), 1, "a scroll doesn't close it");
    }
}
