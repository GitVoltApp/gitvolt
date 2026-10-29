// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! GitVolt patch: window-manager requests for a window that draws its own title bar.
//!
//! winit-gtk4 starts a move or resize with GDK's `begin_move` / `begin_resize`, from the last
//! button press GTK saw. GTK never sees one here: the presses go to the CEF browsers, foreign X
//! children of [`super::CefX11Host`], so `drag_window` and `drag_resize_window` always fail
//! ("requires a pointer button press") and `startDragging` did nothing. Those requests are made
//! here instead, the way GDK makes them on X11 (`gdk/x11/gdksurface-x11.c`, `wmspec_moveresize`
//! and `gdk_x11_toplevel_show_window_menu`): the pointer's root position comes from
//! `XQueryPointer`, the pointer grab is released, and a client message goes to the root window
//! for the window manager:
//! - a move or a resize from an edge or corner: EWMH `_NET_WM_MOVERESIZE`, with button 1 and a
//!   "normal application" source. Only while button 1 is still down: a click whose release
//!   came before the request would leave the window manager moving the window with no button
//!   held (mutter cancels that itself; other window managers may not);
//! - the window menu: `_GTK_SHOW_WINDOW_MENU` at the pointer, which mutter, KWin and xfwm4
//!   support; it is only sent when the window manager lists it in `_NET_SUPPORTED`;
//! - lowering the window below the others: `XLowerWindow`, as `gdk_x11_surface_lower` does.
//!
//! The grab the button press made belongs to Chromium's X connection, and only the client that
//! holds a grab can release it. The ungrab is made on the display CEF hands out
//! (`cef_get_xdisplay`), which is enough where the window manager is also the compositor
//! (Xwayland: mutter, KWin), whose own grab doesn't depend on the X server's. It isn't on a
//! plain X server: that display is not the connection Chromium reads its events on, the window
//! manager's own pointer grab fails, and nothing moves (checked with openbox on Xvfb).

use std::os::raw::{c_int, c_long, c_uint, c_ulong};

use tauri_runtime::ResizeDirection;
use x11_dl::xlib;

use super::utils::{atom, with_cef_display, with_x11};

/// EWMH `_NET_WM_MOVERESIZE` directions.
const MOVERESIZE_SIZE_TOPLEFT: c_long = 0;
const MOVERESIZE_SIZE_TOP: c_long = 1;
const MOVERESIZE_SIZE_TOPRIGHT: c_long = 2;
const MOVERESIZE_SIZE_RIGHT: c_long = 3;
const MOVERESIZE_SIZE_BOTTOMRIGHT: c_long = 4;
const MOVERESIZE_SIZE_BOTTOM: c_long = 5;
const MOVERESIZE_SIZE_BOTTOMLEFT: c_long = 6;
const MOVERESIZE_SIZE_LEFT: c_long = 7;
const MOVERESIZE_MOVE: c_long = 8;

/// The button the page starts a move or resize with.
const BUTTON: c_uint = 1;
/// EWMH source indication: a normal application.
const SOURCE_APPLICATION: c_long = 1;
/// XInput 2's virtual core pointer, the device id GDK names in `_GTK_SHOW_WINDOW_MENU`; mutter
/// and KWin don't read it.
const CORE_POINTER: c_long = 2;

/// The `_NET_WM_MOVERESIZE` direction for a move (`None`) or a resize from `edge`.
pub(super) fn moveresize_direction(edge: Option<ResizeDirection>) -> c_long {
  match edge {
    None => MOVERESIZE_MOVE,
    Some(ResizeDirection::NorthWest) => MOVERESIZE_SIZE_TOPLEFT,
    Some(ResizeDirection::North) => MOVERESIZE_SIZE_TOP,
    Some(ResizeDirection::NorthEast) => MOVERESIZE_SIZE_TOPRIGHT,
    Some(ResizeDirection::East) => MOVERESIZE_SIZE_RIGHT,
    Some(ResizeDirection::SouthEast) => MOVERESIZE_SIZE_BOTTOMRIGHT,
    Some(ResizeDirection::South) => MOVERESIZE_SIZE_BOTTOM,
    Some(ResizeDirection::SouthWest) => MOVERESIZE_SIZE_BOTTOMLEFT,
    Some(ResizeDirection::West) => MOVERESIZE_SIZE_LEFT,
  }
}

/// `_NET_WM_MOVERESIZE`'s data: the pointer's root position, the direction, the button and the
/// source indication.
pub(super) fn moveresize_data(root: (c_int, c_int), direction: c_long) -> [c_long; 5] {
  [
    root.0.into(),
    root.1.into(),
    direction,
    BUTTON.into(),
    SOURCE_APPLICATION,
  ]
}

/// `_GTK_SHOW_WINDOW_MENU`'s data: the pointer device and the root position to show it at.
pub(super) fn window_menu_data(root: (c_int, c_int)) -> [c_long; 5] {
  [CORE_POINTER, root.0.into(), root.1.into(), 0, 0]
}

/// Whether button 1 is down in an `XQueryPointer` mask.
pub(super) fn button_held(mask: c_uint) -> bool {
  mask & xlib::Button1Mask != 0
}

/// The pointer's root position and button mask.
///
/// # Safety
/// `display` must be a live Xlib display.
unsafe fn query_pointer(xlib: &xlib::Xlib, display: *mut xlib::Display) -> Option<((c_int, c_int), c_uint)> {
  unsafe {
    let root = (xlib.XDefaultRootWindow)(display);
    let (mut root_return, mut child) = (0, 0);
    let (mut root_x, mut root_y, mut win_x, mut win_y) = (0, 0, 0, 0);
    let mut mask = 0;
    let on_screen = (xlib.XQueryPointer)(
      display,
      root,
      &mut root_return,
      &mut child,
      &mut root_x,
      &mut root_y,
      &mut win_x,
      &mut win_y,
      &mut mask,
    );
    (on_screen != 0).then_some(((root_x, root_y), mask))
  }
}

/// Releases the pointer grab the button press made, so the window manager can take its own.
fn release_pointer_grab() {
  with_cef_display((), |xlib, display| unsafe {
    (xlib.XUngrabPointer)(display, xlib::CurrentTime);
  });
}

/// Sends a client message about `window` to the root window, for the window manager.
///
/// # Safety
/// `display` must be a live Xlib display.
unsafe fn send_to_window_manager(
  xlib: &xlib::Xlib,
  display: *mut xlib::Display,
  window: c_ulong,
  message_type: c_ulong,
  data: [c_long; 5],
) {
  unsafe {
    let root = (xlib.XDefaultRootWindow)(display);
    let mut event: xlib::XEvent = std::mem::zeroed();
    event.client_message = xlib::XClientMessageEvent {
      type_: xlib::ClientMessage,
      serial: 0,
      send_event: 1,
      display,
      window,
      message_type,
      format: 32,
      data: xlib::ClientMessageData::from(data),
    };
    (xlib.XSendEvent)(
      display,
      root,
      0,
      xlib::SubstructureRedirectMask | xlib::SubstructureNotifyMask,
      &mut event,
    );
  }
}

/// Whether the window manager lists `name` in `_NET_SUPPORTED`.
///
/// # Safety
/// `display` must be a live Xlib display.
unsafe fn wm_supports(xlib: &xlib::Xlib, display: *mut xlib::Display, name: &str) -> bool {
  unsafe {
    let root = (xlib.XDefaultRootWindow)(display);
    let supported = atom(xlib, display, "_NET_SUPPORTED");
    let wanted = atom(xlib, display, name);
    let (mut actual_type, mut actual_format) = (0, 0);
    let (mut count, mut after) = (0, 0);
    let mut data: *mut u8 = std::ptr::null_mut();
    let status = (xlib.XGetWindowProperty)(
      display,
      root,
      supported,
      0,
      4096,
      0,
      xlib::XA_ATOM,
      &mut actual_type,
      &mut actual_format,
      &mut count,
      &mut after,
      &mut data,
    );
    if status != 0 || data.is_null() {
      return false;
    }
    // Format 32 properties come back as an array of C longs.
    let atoms = std::slice::from_raw_parts(data.cast::<c_ulong>(), count as usize);
    let found = actual_format == 32 && atoms.contains(&wanted);
    (xlib.XFree)(data.cast());
    found
  }
}

/// Asks the window manager to move `window` (`edge` is `None`) or resize it from `edge`,
/// following the pointer until button 1 is released. False if button 1 isn't down any more, or
/// there's no X display.
pub(super) fn begin_move_resize(window: c_ulong, edge: Option<ResizeDirection>) -> bool {
  let Some(root) = with_x11(None, |xlib, display| unsafe {
    query_pointer(xlib, display)
      .filter(|(_, mask)| button_held(*mask))
      .map(|(root, _)| root)
  }) else {
    return false;
  };
  release_pointer_grab();
  with_x11((), |xlib, display| unsafe {
    let message = atom(xlib, display, "_NET_WM_MOVERESIZE");
    let data = moveresize_data(root, moveresize_direction(edge));
    send_to_window_manager(xlib, display, window, message, data);
  });
  true
}

/// Asks the window manager to show its window menu for `window` at the pointer. False if it
/// doesn't support `_GTK_SHOW_WINDOW_MENU`, or there's no X display.
pub(super) fn show_window_menu(window: c_ulong) -> bool {
  let Some(root) = with_x11(None, |xlib, display| unsafe {
    if !wm_supports(xlib, display, "_GTK_SHOW_WINDOW_MENU") {
      return None;
    }
    query_pointer(xlib, display).map(|(root, _)| root)
  }) else {
    return false;
  };
  release_pointer_grab();
  with_x11((), |xlib, display| unsafe {
    let message = atom(xlib, display, "_GTK_SHOW_WINDOW_MENU");
    send_to_window_manager(xlib, display, window, message, window_menu_data(root));
  });
  true
}

/// Puts `window` below the other windows.
pub(super) fn lower(window: c_ulong) -> bool {
  with_x11(false, |xlib, display| unsafe {
    (xlib.XLowerWindow)(display, window);
    true
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Every edge and corner maps to EWMH's direction for it, clockwise from the top left, and a
  /// move to `_NET_WM_MOVERESIZE_MOVE`.
  #[test]
  fn directions_follow_ewmh() {
    let edges = [
      (ResizeDirection::NorthWest, 0),
      (ResizeDirection::North, 1),
      (ResizeDirection::NorthEast, 2),
      (ResizeDirection::East, 3),
      (ResizeDirection::SouthEast, 4),
      (ResizeDirection::South, 5),
      (ResizeDirection::SouthWest, 6),
      (ResizeDirection::West, 7),
    ];
    for (edge, direction) in edges {
      assert_eq!(moveresize_direction(Some(edge)), direction, "{edge:?}");
    }
    assert_eq!(moveresize_direction(None), 8);
  }

  /// The pointer's root position, the direction, button 1 and the "application" source.
  #[test]
  fn a_move_names_the_pointer_and_button_1() {
    assert_eq!(moveresize_data((640, 12), 8), [640, 12, 8, 1, 1]);
    assert_eq!(
      moveresize_data((-20, 1070), moveresize_direction(Some(ResizeDirection::SouthEast))),
      [-20, 1070, 4, 1, 1]
    );
  }

  /// GDK's layout: the device, then the root position.
  #[test]
  fn the_window_menu_is_asked_for_at_the_pointer() {
    assert_eq!(window_menu_data((300, 15)), [2, 300, 15, 0, 0]);
  }

  #[test]
  fn only_a_held_left_button_starts_a_move() {
    assert!(button_held(xlib::Button1Mask));
    assert!(button_held(xlib::Button1Mask | xlib::ShiftMask));
    assert!(!button_held(0));
    assert!(!button_held(xlib::Button3Mask));
  }
}
