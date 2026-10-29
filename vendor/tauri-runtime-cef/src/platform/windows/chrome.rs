// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! GitVolt patch: a window that draws its own title bar keeps Windows' frame everywhere else.
//!
//! winit builds an undecorated window (`decorations(false)`) with the usual styles, caption and
//! sizing frame included, and makes its client area the whole window in `WM_NCCALCSIZE`: no
//! resize border, so nothing to resize it from, and no shadow. A subclass here, called before
//! winit's window procedure, gives such a window back the system's resize border at its left,
//! right and bottom, the geometry in `window_frame.rs`:
//! - `WM_NCCALCSIZE`: the client area is the window less the border, so the border stays outside
//!   the visible window, where Windows draws the shadow and its 1 px edge. The top has none; the
//!   page resizes from there with `start_resize_dragging`. A maximized window is left to winit
//!   (its client area is the monitor's work area, so it never covers the taskbar), as are a
//!   minimized one and one without a sizing frame (not resizable, or fullscreen);
//! - `WM_NCHITTEST`: the border's edges and corners, so Windows resizes the window natively;
//! - `WM_GETMINMAXINFO` and `WM_DPICHANGED`: winit sizes the window as if it had no border
//!   (`adjust_rect` drops the frame of an undecorated window), so its minimum and maximum sizes
//!   and the size it picks at a DPI change get the border added back, the client area then being
//!   the size asked for. `AppWindow::request_surface_size` does the same for `set_size`.
//!
//! The browsers cover the client area, so the window only ever hit-tests its border; moving the
//! window and resizing it from the top go through winit's `drag_window` and
//! `drag_resize_window`, which hand the press to Windows' own move and size loops (Aero Snap
//! included).
//!
//! The window menu at the pointer is shown from a posted message, by the subclass: from the
//! window's own message dispatch, as Windows shows it for a right click on a title bar, rather
//! than inside the runtime's event handling, whose caller would wait for the menu to close.

use std::sync::OnceLock;

use windows::{
  Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    UI::{
      HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
      Shell::{DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass},
      WindowsAndMessaging::{
        EnableMenuItem, GWL_STYLE, GetCursorPos, GetSystemMenu, GetWindowLongW,
        GetWindowPlacement, GetWindowRect, HWND_BOTTOM, IsIconic, IsZoomed, MF_BYCOMMAND,
        MF_DISABLED, MF_ENABLED, MF_GRAYED, MINMAXINFO, NCCALCSIZE_PARAMS, PostMessageW,
        RegisterWindowMessageW, SC_KEYMENU, SM_CXPADDEDBORDER, SM_CXSIZEFRAME, SM_CYSIZEFRAME,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
        SWP_NOZORDER, SetMenuDefaultItem, SetWindowPlacement, SetWindowPos, TPM_RETURNCMD,
        TPM_RIGHTBUTTON, TrackPopupMenu, WINDOWPLACEMENT, WM_DPICHANGED, WM_GETMINMAXINFO,
        WM_NCCALCSIZE, WM_NCHITTEST, WM_SYSCOMMAND, WS_THICKFRAME,
      },
    },
  },
  core::w,
};

use crate::window_frame::{
  Border, Rect, SC_CLOSE, client_area, frame_applies, frame_extra, hit_test, pack_point,
  unpack_point, window_menu_states,
};

/// The subclass's id on the window ("GVfr").
const SUBCLASS_ID: usize = 0x4756_6672;

/// The message that has the subclass show the window menu at the point in its `LPARAM`.
fn window_menu_message() -> u32 {
  static MESSAGE: OnceLock<u32> = OnceLock::new();
  *MESSAGE.get_or_init(|| unsafe { RegisterWindowMessageW(w!("GitVolt.ShowWindowMenu")) })
}

fn rect(r: RECT) -> Rect {
  Rect {
    left: r.left,
    top: r.top,
    right: r.right,
    bottom: r.bottom,
  }
}

fn win_rect(r: Rect) -> RECT {
  RECT {
    left: r.left,
    top: r.top,
    right: r.right,
    bottom: r.bottom,
  }
}

/// The resize border's thickness at the window's DPI.
fn border(hwnd: HWND) -> Border {
  unsafe {
    let dpi = GetDpiForWindow(hwnd);
    let padded = GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
    Border {
      x: GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) + padded,
      y: GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) + padded,
    }
  }
}

/// Whether the border applies to the window now (`frame_applies`).
fn applies(hwnd: HWND) -> bool {
  unsafe {
    let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
    frame_applies(
      style & WS_THICKFRAME.0 != 0,
      IsZoomed(hwnd).as_bool(),
      IsIconic(hwnd).as_bool(),
    )
  }
}

/// Whether the window has the subclass, so GitVolt's frame.
pub(super) fn installed(hwnd: HWND) -> bool {
  unsafe { GetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, None).as_bool() }
}

/// How much bigger than its client area the window is now: the border, if it applies.
pub(super) fn extra(hwnd: HWND) -> (i32, i32) {
  if installed(hwnd) && applies(hwnd) {
    frame_extra(border(hwnd))
  } else {
    (0, 0)
  }
}

/// Gives an undecorated window the frame (`custom`), or takes it away, and has Windows lay the
/// window out again. When the frame comes, the window grows by the border, so its client area
/// keeps the size it was given (a maximized window's restored size does).
pub(super) fn set_custom_frame(hwnd: HWND, custom: bool) {
  if custom == installed(hwnd) {
    return;
  }
  unsafe {
    if custom {
      let _ = SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0);
    } else {
      let _ = RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID);
    }
    let _ = SetWindowPos(
      hwnd,
      None,
      0,
      0,
      0,
      0,
      SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
    );
    if custom {
      let (dx, dy) = frame_extra(border(hwnd));
      if IsZoomed(hwnd).as_bool() {
        let mut placement = WINDOWPLACEMENT {
          length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
          ..Default::default()
        };
        if GetWindowPlacement(hwnd, &mut placement).is_ok() {
          placement.rcNormalPosition.right += dx;
          placement.rcNormalPosition.bottom += dy;
          let _ = SetWindowPlacement(hwnd, &placement);
        }
      } else if applies(hwnd) {
        grow(hwnd, dx, dy);
      }
    }
  }
}

/// Makes the window `dx` wider and `dy` taller, where it is.
fn grow(hwnd: HWND, dx: i32, dy: i32) {
  unsafe {
    let mut r = RECT::default();
    if GetWindowRect(hwnd, &mut r).is_ok() {
      let _ = SetWindowPos(
        hwnd,
        None,
        0,
        0,
        r.right - r.left + dx,
        r.bottom - r.top + dy,
        SWP_NOMOVE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
      );
    }
  }
}

/// Shows the window menu at the pointer, from the window's own message dispatch. False if the
/// window doesn't have GitVolt's frame.
pub(super) fn show_window_menu(hwnd: HWND) -> bool {
  if !installed(hwnd) {
    return false;
  }
  let mut pointer = POINT::default();
  if unsafe { GetCursorPos(&mut pointer) }.is_err() {
    return false;
  }
  unsafe {
    PostMessageW(
      Some(hwnd),
      window_menu_message(),
      WPARAM(0),
      LPARAM(pack_point(pointer.x, pointer.y)),
    )
  }
  .is_ok()
}

/// Shows the window menu as Alt+Space does: at the window's top left corner, for the keyboard.
pub(super) fn show_keyboard_window_menu(hwnd: HWND) -> bool {
  installed(hwnd)
    && unsafe {
      PostMessageW(
        Some(hwnd),
        WM_SYSCOMMAND,
        WPARAM(SC_KEYMENU as usize),
        LPARAM(' ' as isize),
      )
    }
    .is_ok()
}

/// Puts the window below the others.
pub(super) fn lower(hwnd: HWND) -> bool {
  unsafe {
    SetWindowPos(
      hwnd,
      Some(HWND_BOTTOM),
      0,
      0,
      0,
      0,
      SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
    )
  }
  .is_ok()
}

/// The window's own menu at `(x, y)`, its commands enabled as for a captioned window; the chosen
/// one is posted as `WM_SYSCOMMAND`, as Windows does.
unsafe fn track_window_menu(hwnd: HWND, x: i32, y: i32) {
  unsafe {
    let menu = GetSystemMenu(hwnd, false);
    if menu.is_invalid() {
      return;
    }
    for (command, enabled) in window_menu_states(IsZoomed(hwnd).as_bool()) {
      let state = if enabled {
        MF_ENABLED
      } else {
        MF_DISABLED | MF_GRAYED
      };
      let _ = EnableMenuItem(menu, command, MF_BYCOMMAND | state);
    }
    let _ = SetMenuDefaultItem(menu, SC_CLOSE, 0);
    let command = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, x, y, None, hwnd, None);
    if command.0 != 0 {
      let _ = PostMessageW(
        Some(hwnd),
        WM_SYSCOMMAND,
        WPARAM(command.0 as usize),
        LPARAM(0),
      );
    }
  }
}

unsafe extern "system" fn subclass_proc(
  hwnd: HWND,
  msg: u32,
  wparam: WPARAM,
  lparam: LPARAM,
  _subclass_id: usize,
  _ref_data: usize,
) -> LRESULT {
  unsafe {
    match msg {
      WM_NCCALCSIZE if wparam.0 != 0 && applies(hwnd) => {
        let params = &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS);
        params.rgrc[0] = win_rect(client_area(rect(params.rgrc[0]), border(hwnd)));
        LRESULT(0)
      }
      WM_NCHITTEST if applies(hwnd) => {
        let mut window = RECT::default();
        if GetWindowRect(hwnd, &mut window).is_err() {
          return DefSubclassProc(hwnd, msg, wparam, lparam);
        }
        let point = unpack_point(lparam.0);
        LRESULT(hit_test(point, rect(window), border(hwnd)) as isize)
      }
      WM_GETMINMAXINFO => {
        let result = DefSubclassProc(hwnd, msg, wparam, lparam);
        if applies(hwnd) {
          let (dx, dy) = frame_extra(border(hwnd));
          let info = &mut *(lparam.0 as *mut MINMAXINFO);
          info.ptMinTrackSize.x += dx;
          info.ptMinTrackSize.y += dy;
          info.ptMaxTrackSize.x += dx;
          info.ptMaxTrackSize.y += dy;
        }
        result
      }
      WM_DPICHANGED => {
        let result = DefSubclassProc(hwnd, msg, wparam, lparam);
        if applies(hwnd) {
          let (dx, dy) = frame_extra(border(hwnd));
          grow(hwnd, dx, dy);
        }
        result
      }
      _ if msg == window_menu_message() => {
        let (x, y) = unpack_point(lparam.0);
        track_window_menu(hwnd, x, y);
        LRESULT(0)
      }
      _ => DefSubclassProc(hwnd, msg, wparam, lparam),
    }
  }
}
