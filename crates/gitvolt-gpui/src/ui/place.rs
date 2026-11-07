//! Where things float: the pure placements of `menu/position.ts`, `HoverTooltip.tsx`,
//! `arm/anchor.ts` and `arm/grow.ts`, and `Placed`, the element that puts a child of measured size
//! where one of them says.

use gpui::*;

const MENU_M: f32 = 4.;
/// The gap to the window's edge a card or a tooltip keeps.
pub const EDGE: f32 = 8.;
pub const GAP: f32 = 4.;
/// Between the pointer's hotspot and a tooltip placed at it.
pub const POINTER_GAP: f32 = 12.;
const POPOVER_M: f32 = 8.;
const POPOVER_GAP: f32 = 6.;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl Rect {
    pub fn of(b: Bounds<Pixels>) -> Self {
        Self { left: b.left().as_f32(), right: b.right().as_f32(), top: b.top().as_f32(), bottom: b.bottom().as_f32() }
    }

    pub fn at(x: f32, y: f32) -> Self {
        Self { left: x, right: x, top: y, bottom: y }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// The top-left of a menu opened at the pointer: flipped left at the right edge; too tall, it still
/// opens downward, shifted up just enough. Dropped from a button (`anchor`), it flips by the button.
pub fn place_menu(x: f32, y: f32, w: f32, h: f32, vw: f32, vh: f32, anchor: Option<Rect>) -> (f32, f32) {
    if let Some(a) = anchor {
        let left = if a.left + w + MENU_M > vw { MENU_M.max(a.right - w) } else { a.left };
        let top = if a.bottom + h + MENU_M > vh { MENU_M.max(a.top - h) } else { a.bottom };
        return (left, top);
    }
    let left = if x + w + MENU_M > vw { MENU_M.max(x - w) } else { x };
    let top = MENU_M.max(y.min(vh - h - MENU_M));
    (left, top)
}

/// A submenu beside its row: to the right, else the left; clamped vertically.
pub fn place_submenu(parent: Rect, w: f32, h: f32, vw: f32, vh: f32) -> (f32, f32) {
    let left = if parent.right + w + MENU_M > vw { MENU_M.max(parent.left - w) } else { parent.right };
    let top = MENU_M.max(parent.top.min(vh - h - MENU_M));
    (left, top)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardPlace {
    pub left: f32,
    pub top: f32,
    pub max_height: Option<f32>,
}

/// The one placement for every card and tooltip, kept inside the window. Below the anchor when it
/// fits, else above it, else at the top margin with a max height.
pub fn place_card(anchor: Rect, w: f32, h: f32, gap: f32, vw: f32, vh: f32) -> CardPlace {
    let left = EDGE.max(anchor.left.min(vw - EDGE - w));
    let room = vh - 2. * EDGE;
    if h > room {
        return CardPlace { left, top: EDGE, max_height: Some(room) };
    }
    let mut top = anchor.bottom + gap;
    if top + h > vh - EDGE {
        top = EDGE.max(anchor.top - gap - h);
    }
    CardPlace { left, top, max_height: None }
}

/// Beside the anchor with its top on the anchor's, moved up just enough to fit (a hover card).
pub fn place_card_top(anchor: Rect, w: f32, h: f32, vw: f32, vh: f32) -> CardPlace {
    let left = EDGE.max(anchor.left.min(vw - EDGE - w));
    let room = vh - 2. * EDGE;
    if h > room {
        return CardPlace { left, top: EDGE, max_height: Some(room) };
    }
    CardPlace { left, top: EDGE.max(anchor.top.min(vh - EDGE - h)), max_height: None }
}

/// Beside `anchor` on `right` or left (flipped when it doesn't fit), at its top: a menu row's tooltip.
pub fn place_beside(anchor: Rect, w: f32, h: f32, right_side: bool, vw: f32, vh: f32) -> (f32, f32) {
    let right = anchor.right + GAP;
    let left = anchor.left - GAP - w;
    let fits_right = right + w <= vw - EDGE;
    let fits_left = left >= EDGE;
    let x = if right_side { if fits_right || !fits_left { right } else { left } } else if fits_left || !fits_right { left } else { right };
    (EDGE.max(x.min(vw - EDGE - w)), EDGE.max(anchor.top.min(vh - EDGE - h)))
}

/// POINTER_GAP right of the cursor, or left of it at the edge; at its height, inside the window.
pub fn place_at_pointer(x: f32, y: f32, w: f32, h: f32, vw: f32, vh: f32) -> (f32, f32) {
    let mut left = x + POINTER_GAP;
    if left + w > vw - EDGE {
        left = x - POINTER_GAP - w;
    }
    (EDGE.max(left), EDGE.max(y.min(vh - EDGE - h)))
}

/// A popover: under its anchor, left-aligned (above with no room below), on screen; with no anchor,
/// centred near the top.
pub fn place_popover(anchor: Option<Rect>, w: f32, h: f32, vw: f32, vh: f32) -> (f32, f32) {
    let Some(a) = anchor else { return (POPOVER_M.max(((vw - w) / 2.).round()), 72.) };
    let left = POPOVER_M.max(a.left.min(vw - w - POPOVER_M));
    let mut top = a.bottom + POPOVER_GAP;
    if top + h > vh - POPOVER_M && a.top - POPOVER_GAP - h >= POPOVER_M {
        top = a.top - POPOVER_GAP - h;
    }
    (left, POPOVER_M.max(top.min(vh - h - POPOVER_M)))
}

/// Which way an armed control's label grows, and how wide it may get: toward the side of `bounds`
/// with room for it, `preferred` first; with room on neither, the roomier one, clipped.
pub fn arm_placement(control: (f32, f32), bounds: (f32, f32), needed: f32, preferred: Option<bool>) -> (bool, f32) {
    let (room_left, room_right) = (control.1 - bounds.0, bounds.1 - control.0);
    let roomier_right = room_right >= room_left;
    let first_right = preferred.unwrap_or(roomier_right);
    let (first, other) = if first_right { (room_right, room_left) } else { (room_left, room_right) };
    let grow_right = if needed <= first { first_right } else if needed <= other { !first_right } else { roomier_right };
    (grow_right, (control.1 - control.0).max(if grow_right { room_right } else { room_left }))
}

/// A child put where `place` says from its measured size and the window's. Use it as a root child,
/// so its coordinates are the window's.
pub struct Placed {
    child: AnyElement,
    place: Option<PlaceFn>,
    on_placed: Option<OnPlaced>,
}

type PlaceFn = Box<dyn FnOnce(Size<Pixels>, Size<Pixels>, &App) -> Point<Pixels>>;
type OnPlaced = Box<dyn FnOnce(Bounds<Pixels>, &mut App)>;

pub fn placed(child: impl IntoElement, place: impl FnOnce(Size<Pixels>, Size<Pixels>, &App) -> Point<Pixels> + 'static) -> Placed {
    Placed { child: child.into_any_element(), place: Some(Box::new(place)), on_placed: None }
}

impl Placed {
    /// Told the child's window bounds, each frame.
    pub fn on_placed(mut self, f: impl FnOnce(Bounds<Pixels>, &mut App) + 'static) -> Self {
        self.on_placed = Some(Box::new(f));
        self
    }
}

impl IntoElement for Placed {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Placed {
    type RequestLayoutState = LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, LayoutId) {
        let child = self.child.request_layout(window, cx);
        let style = Style { position: Position::Absolute, display: Display::Flex, ..Style::default() };
        (window.request_layout(style, [child], cx), child)
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, child: &mut LayoutId, window: &mut Window, cx: &mut App) {
        let measured = window.layout_bounds(*child);
        let at = self.place.take().map_or(measured.origin, |place| place(measured.size, window.viewport_size(), cx));
        let offset = at - measured.origin;
        let offset = point(offset.x.round(), offset.y.round());
        if let Some(f) = self.on_placed.take() {
            f(Bounds::new(measured.origin + offset, measured.size), cx);
        }
        window.with_element_offset(offset, |window| self.child.prepaint(window, cx));
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut LayoutId, _: &mut (), window: &mut Window, cx: &mut App) {
        self.child.paint(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_menu_flips_left_at_the_edge_and_shifts_up_when_too_tall() {
        assert_eq!(place_menu(100., 100., 200., 100., 1000., 800., None), (100., 100.));
        assert_eq!(place_menu(900., 100., 200., 100., 1000., 800., None), (700., 100.));
        assert_eq!(place_menu(100., 750., 200., 100., 1000., 800., None), (100., 696.));
        assert_eq!(place_menu(100., 10., 200., 900., 1000., 800., None), (100., 4.));
    }

    #[test]
    fn a_dropdown_flips_by_its_button() {
        let a = Rect { left: 900., right: 950., top: 700., bottom: 720. };
        assert_eq!(place_menu(0., 0., 200., 100., 1000., 800., Some(a)), (750., 600.));
        let b = Rect { left: 10., right: 50., top: 10., bottom: 30. };
        assert_eq!(place_menu(0., 0., 200., 100., 1000., 800., Some(b)), (10., 30.));
    }

    #[test]
    fn a_submenu_goes_right_then_left() {
        let p = Rect { left: 100., right: 300., top: 50., bottom: 76. };
        assert_eq!(place_submenu(p, 200., 100., 1000., 800.), (300., 50.));
        assert_eq!(place_submenu(p, 800., 100., 1000., 800.), (4., 50.));
    }

    // Web: ui/placeCard.test.ts.
    #[test]
    fn a_card_goes_below_flips_above_clamps_and_caps() {
        let a = Rect { left: 100., right: 100., top: 300., bottom: 320. };
        assert_eq!(place_card(a, 300., 200., 4., 1000., 800.), CardPlace { left: 100., top: 324., max_height: None });
        let low = Rect { left: 100., right: 100., top: 700., bottom: 720. };
        assert_eq!(place_card(low, 300., 300., 4., 1000., 800.).top, 396.);
        assert_eq!(place_card(Rect { left: 900., ..a }, 300., 100., 4., 1000., 800.).left, 692.);
        assert_eq!(place_card(a, 300., 900., 4., 1000., 800.), CardPlace { left: 100., top: 8., max_height: Some(784.) });
        assert_eq!(place_card_top(Rect::at(50., 700.), 300., 200., 1000., 800.).top, 592.);
    }

    #[test]
    fn an_armed_label_grows_where_it_fits() {
        assert_eq!(arm_placement((100., 150.), (0., 1000.), 200., None), (true, 900.));
        assert_eq!(arm_placement((900., 950.), (0., 1000.), 200., None), (false, 950.));
        assert_eq!(arm_placement((100., 150.), (0., 1000.), 120., Some(false)), (false, 150.));
        assert_eq!(arm_placement((100., 150.), (0., 200.), 500., Some(true)), (false, 150.), "room on neither side: the roomier");
    }

    #[test]
    fn a_popover_sits_under_its_anchor_or_centred_without_one() {
        assert_eq!(place_popover(None, 300., 100., 1000., 800.), (350., 72.));
        let a = Rect { left: 100., right: 200., top: 700., bottom: 720. };
        assert_eq!(place_popover(Some(a), 300., 100., 1000., 800.), (100., 594.));
    }
}
