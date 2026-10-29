// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! GitVolt patch: the geometry of a Windows window that draws its own title bar but keeps the
//! system's frame everywhere else (`platform/windows/chrome.rs`), as pure functions so they're
//! tested on every platform.
//!
//! The window keeps Windows' resize border at its left, right and bottom: outside the visible
//! window, where the system draws the shadow and resizes it natively (Aero Snap included). The
//! top has none, so the page's title bar goes up to the window's edge; the page resizes from
//! there itself (`start_resize_dragging`).

#![cfg_attr(not(windows), allow(dead_code))]

/// `WM_NCHITTEST` results (Win32's `HT*` values).
pub(crate) const HTCLIENT: u32 = 1;
pub(crate) const HTLEFT: u32 = 10;
pub(crate) const HTRIGHT: u32 = 11;
pub(crate) const HTTOPLEFT: u32 = 13;
pub(crate) const HTTOPRIGHT: u32 = 14;
pub(crate) const HTBOTTOM: u32 = 15;
pub(crate) const HTBOTTOMLEFT: u32 = 16;
pub(crate) const HTBOTTOMRIGHT: u32 = 17;

/// The window menu's commands (Win32's `SC_*` values).
pub(crate) const SC_SIZE: u32 = 0xF000;
pub(crate) const SC_MOVE: u32 = 0xF010;
pub(crate) const SC_MINIMIZE: u32 = 0xF020;
pub(crate) const SC_MAXIMIZE: u32 = 0xF030;
pub(crate) const SC_CLOSE: u32 = 0xF060;
pub(crate) const SC_RESTORE: u32 = 0xF120;

/// A rectangle in screen pixels, right and bottom exclusive (Win32's `RECT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rect {
  pub left: i32,
  pub top: i32,
  pub right: i32,
  pub bottom: i32,
}

/// The resize border's thickness, in pixels: `SM_CXSIZEFRAME` + `SM_CXPADDEDBORDER` across,
/// `SM_CYSIZEFRAME` + `SM_CXPADDEDBORDER` down, at the window's DPI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Border {
  pub x: i32,
  pub y: i32,
}

/// Whether the frame applies: only to a resizable window (`WS_THICKFRAME`; a fullscreen one has
/// none), and neither maximized (the client area is then the monitor's work area, winit's
/// `WM_NCCALCSIZE`) nor minimized.
pub(crate) fn frame_applies(thick_frame: bool, maximized: bool, minimized: bool) -> bool {
  thick_frame && !maximized && !minimized
}

/// The client area of a window at `window`: all of it but the resize border at the left, the
/// right and the bottom.
pub(crate) fn client_area(window: Rect, border: Border) -> Rect {
  Rect {
    left: window.left + border.x,
    top: window.top,
    right: window.right - border.x,
    bottom: window.bottom - border.y,
  }
}

/// How much wider and taller the window is than its client area.
pub(crate) fn frame_extra(border: Border) -> (i32, i32) {
  (2 * border.x, border.y)
}

/// What's at `point` in a window at `window`: an edge or corner of the resize border, or the
/// client area. Near a corner, the border resizes from the corner: within twice the border's
/// thickness of it, which is also how far a top corner reaches down the sides (the top has no
/// border of its own).
pub(crate) fn hit_test(point: (i32, i32), window: Rect, border: Border) -> u32 {
  let (x, y) = point;
  let corner = 2 * border.x.max(border.y);
  let left = x < window.left + border.x;
  let right = x >= window.right - border.x;
  let bottom = y >= window.bottom - border.y;
  let near_top = y < window.top + corner;
  let near_bottom = y >= window.bottom - corner;
  let near_left = x < window.left + corner;
  let near_right = x >= window.right - corner;
  match () {
    _ if left && near_top => HTTOPLEFT,
    _ if right && near_top => HTTOPRIGHT,
    _ if (left && near_bottom) || (bottom && near_left) => HTBOTTOMLEFT,
    _ if (right && near_bottom) || (bottom && near_right) => HTBOTTOMRIGHT,
    _ if left => HTLEFT,
    _ if right => HTRIGHT,
    _ if bottom => HTBOTTOM,
    _ => HTCLIENT,
  }
}

/// Which window menu commands are available, as Windows has them for a captioned window: Restore
/// only when maximized, Move, Size and Maximize only when not.
pub(crate) fn window_menu_states(maximized: bool) -> [(u32, bool); 6] {
  [
    (SC_RESTORE, maximized),
    (SC_MOVE, !maximized),
    (SC_SIZE, !maximized),
    (SC_MINIMIZE, true),
    (SC_MAXIMIZE, !maximized),
    (SC_CLOSE, true),
  ]
}

/// A screen point in a message's `LPARAM`, as `MAKELPARAM` packs it: x in the low word, y in
/// the high one, each a signed 16-bit value (a monitor left of or above the primary one has
/// negative coordinates).
pub(crate) fn pack_point(x: i32, y: i32) -> isize {
  ((x as u16 as u32) | ((y as u16 as u32) << 16)) as i32 as isize
}

/// The point [`pack_point`] packed (`GET_X_LPARAM`, `GET_Y_LPARAM`).
pub(crate) fn unpack_point(lparam: isize) -> (i32, i32) {
  (
    (lparam & 0xFFFF) as u16 as i16 as i32,
    ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32,
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  /// At 100%: Windows 11's 4 px sizing frame and 4 px padded border.
  const BORDER: Border = Border { x: 8, y: 8 };
  const WINDOW: Rect = Rect {
    left: 100,
    top: 50,
    right: 1100,
    bottom: 850,
  };

  #[test]
  fn the_client_area_reaches_the_top_and_leaves_the_border_elsewhere() {
    assert_eq!(
      client_area(WINDOW, BORDER),
      Rect {
        left: 108,
        top: 50,
        right: 1092,
        bottom: 842
      }
    );
    assert_eq!(frame_extra(BORDER), (16, 8));
    // At 150%.
    assert_eq!(frame_extra(Border { x: 11, y: 11 }), (22, 11));
  }

  #[test]
  fn only_a_resizable_window_in_its_normal_state_has_the_frame() {
    assert!(frame_applies(true, false, false));
    assert!(!frame_applies(false, false, false));
    assert!(!frame_applies(true, true, false));
    assert!(!frame_applies(true, false, true));
  }

  #[test]
  fn the_border_resizes_from_its_edges_and_corners() {
    let at = |x, y| hit_test((x, y), WINDOW, BORDER);
    // The sides and the bottom, away from the corners.
    assert_eq!(at(100, 400), HTLEFT);
    assert_eq!(at(107, 400), HTLEFT);
    assert_eq!(at(1092, 400), HTRIGHT);
    assert_eq!(at(1099, 400), HTRIGHT);
    assert_eq!(at(600, 842), HTBOTTOM);
    assert_eq!(at(600, 849), HTBOTTOM);
    // Inside: the client area (the page's own title bar at the top).
    assert_eq!(at(108, 400), HTCLIENT);
    assert_eq!(at(600, 50), HTCLIENT);
    assert_eq!(at(600, 841), HTCLIENT);
    // The corners, from the bottom border or the sides.
    assert_eq!(at(100, 849), HTBOTTOMLEFT);
    assert_eq!(at(115, 845), HTBOTTOMLEFT);
    assert_eq!(at(104, 835), HTBOTTOMLEFT);
    assert_eq!(at(1099, 849), HTBOTTOMRIGHT);
    assert_eq!(at(1085, 845), HTBOTTOMRIGHT);
    assert_eq!(at(1095, 835), HTBOTTOMRIGHT);
    // The top corners, from the sides' upper end.
    assert_eq!(at(100, 50), HTTOPLEFT);
    assert_eq!(at(104, 65), HTTOPLEFT);
    assert_eq!(at(104, 66), HTLEFT);
    assert_eq!(at(1099, 50), HTTOPRIGHT);
    assert_eq!(at(1095, 65), HTTOPRIGHT);
  }

  #[test]
  fn the_window_menu_follows_the_maximized_state() {
    let enabled = |maximized| {
      window_menu_states(maximized)
        .into_iter()
        .filter(|(_, on)| *on)
        .map(|(command, _)| command)
        .collect::<Vec<_>>()
    };
    assert_eq!(
      enabled(false),
      [SC_MOVE, SC_SIZE, SC_MINIMIZE, SC_MAXIMIZE, SC_CLOSE]
    );
    assert_eq!(enabled(true), [SC_RESTORE, SC_MINIMIZE, SC_CLOSE]);
  }

  #[test]
  fn a_point_survives_the_lparam_including_negative_coordinates() {
    for point in [(0, 0), (640, 12), (-1920, 1070), (3839, -1), (-32768, 32767)] {
      assert_eq!(unpack_point(pack_point(point.0, point.1)), point, "{point:?}");
    }
    assert_eq!(pack_point(1, 2), 0x0002_0001);
  }
}
