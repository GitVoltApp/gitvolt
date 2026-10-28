// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! GitVolt patch: the pointer leaving the window takes the page's hover with it.
//!
//! With GPU compositing, Chromium's GPU process renders into a child window of the browser's X
//! window (`NativeViewGLSurfaceEGLX11GLES2`, or ANGLE's GLX backend), and that child covers the
//! whole page. So the pointer over the page is in that child, and when it leaves the app, the
//! browser's own window gets its `LeaveNotify` with detail `NotifyVirtual` (the pointer passed
//! through it from an inferior). Chromium reads that detail as crossing an intermediate window
//! and turns the leave into a mouse *move* at the leave point (`kPropertyMouseCrossedIntermediateWindow`,
//! set in `ui/events/x/x11_event_translation.cc` and read in `ui/events/blink/web_input_event.cc`).
//! The page never gets a mouse leave: `:hover` and hover tooltips stay up until the pointer comes
//! back, or move to whatever is under the leave point when that is still inside the window (an
//! overlapping window's edge). Software compositing has no child window, the leave arrives as
//! `NotifyAncestor`, and it works.
//!
//! So [`super::CefX11Host`] watches its own crossings, and on a real leave the runtime sends its
//! browsers a mouse leave through CEF (`CefBrowserHost::SendMouseMoveEvent` with `mouseLeave`),
//! once GDK has dispatched the batch of X events, as it does the host's relayout.

use std::{cell::Cell, os::raw::c_ulong, rc::Rc};

use gtk::{glib, prelude::*};
use tauri_runtime::dpi::PhysicalPosition;
use x11_dl::xlib;

use super::utils::xlib_fns;

/// The host's pointer crossings, selected on GDK's X connection; disconnected on drop.
pub(super) struct PointerLeaveWatch {
  display: gdk4_x11::X11Display,
  handler: Option<glib::SignalHandlerId>,
  /// Where the pointer last left the host, until the runtime sends the leave or it comes back.
  left_at: Rc<Cell<Option<PhysicalPosition<i32>>>>,
}

impl PointerLeaveWatch {
  pub(super) fn take_left_at(&self) -> Option<PhysicalPosition<i32>> {
    self.left_at.take()
  }
}

impl Drop for PointerLeaveWatch {
  fn drop(&mut self) {
    if let Some(handler) = self.handler.take() {
      self.display.disconnect(handler);
    }
  }
}

/// What a crossing event on the X11 host means for the browsers in it.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum HostCrossing {
  /// The pointer left the host, at this point in the host's (physical) pixels.
  Left(PhysicalPosition<i32>),
  /// The pointer came back in.
  Entered,
}

/// Whether `event` is the pointer leaving or entering the X11 `host` window itself.
///
/// Moving between the host and a browser in it (`NotifyInferior`) isn't one. Neither is a grab
/// starting (`NotifyGrab`), which moves no pointer: a window manager grabs on a click, and a drag
/// grabs too. Chromium ignores those leaves as well.
pub(super) fn host_crossing(event: &xlib::XEvent, host: c_ulong) -> Option<HostCrossing> {
  let kind = event.get_type();
  if kind != xlib::LeaveNotify && kind != xlib::EnterNotify {
    return None;
  }
  // SAFETY: the event type says the crossing variant is the one in use.
  let crossing = unsafe { event.crossing };
  if crossing.window != host
    || crossing.detail == xlib::NotifyInferior
    || crossing.mode == xlib::NotifyGrab
  {
    return None;
  }
  Some(if kind == xlib::LeaveNotify {
    HostCrossing::Left(PhysicalPosition::new(crossing.x, crossing.y))
  } else {
    HostCrossing::Entered
  })
}

/// A point in the host's physical pixels, in the view coordinates (DIP) of a browser at `origin`.
pub(super) fn point_in_view(
  point: PhysicalPosition<i32>,
  origin: PhysicalPosition<i32>,
  scale: f64,
) -> (i32, i32) {
  let to_view = |value: i32, origin: i32| (f64::from(value - origin) / scale).round() as i32;
  (to_view(point.x, origin.x), to_view(point.y, origin.y))
}

/// Watches the pointer leaving `host` (see the module docs).
pub(super) fn watch_pointer_leave(
  display: &gtk::gdk::Display,
  host: c_ulong,
) -> Option<PointerLeaveWatch> {
  let display = display.clone().downcast::<gdk4_x11::X11Display>().ok()?;
  let xlib = xlib_fns()?;

  // Each X client has its own event mask on a window, so selecting on GDK's connection leaves
  // the runtime's (the host's creator) as it was, and the events come through GDK's `xevent`.
  // SAFETY: Xlib calls on GDK's own display, on the GTK main thread, inside a GDK error trap.
  unsafe {
    display.error_trap_push();
    (xlib.XSelectInput)(
      display.xdisplay(),
      host,
      xlib::EnterWindowMask | xlib::LeaveWindowMask,
    );
    (xlib.XFlush)(display.xdisplay());
    display.error_trap_pop_ignored();
  }

  let left_at = Rc::new(Cell::new(None));
  let pending = left_at.clone();
  // SAFETY: the handler only reads the event GDK passes it.
  let handler = unsafe {
    display.connect_xevent(move |_, event| match host_crossing(&*event, host) {
      Some(HostCrossing::Left(point)) => {
        pending.set(Some(point));
        glib::Propagation::Stop
      }
      // Back before the leave was sent: Chromium has the pointer again.
      Some(HostCrossing::Entered) => {
        pending.set(None);
        glib::Propagation::Stop
      }
      None => glib::Propagation::Proceed,
    })
  };

  Some(PointerLeaveWatch {
    display,
    handler: Some(handler),
    left_at,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  const HOST: c_ulong = 0x140_0002;
  const BROWSER: c_ulong = 0xa0_0003;

  fn crossing(kind: i32, window: c_ulong, mode: i32, detail: i32) -> xlib::XEvent {
    let mut event: xlib::XEvent = unsafe { std::mem::zeroed() };
    event.crossing = xlib::XCrossingEvent {
      type_: kind,
      serial: 0,
      send_event: 0,
      display: std::ptr::null_mut(),
      window,
      root: 0x3c,
      subwindow: 0,
      time: 0,
      x: 1499,
      y: 772,
      x_root: 1499,
      y_root: 811,
      mode,
      detail,
      same_screen: 1,
      focus: 1,
      state: 0,
    };
    event
  }

  #[test]
  fn leaving_the_host_from_a_browser_inside_it_is_a_leave() {
    // From the GPU process's child window to the root, as Xwayland moves the pointer when it
    // goes onto a Wayland window: the case Chromium turns into a move.
    let event = crossing(
      xlib::LeaveNotify,
      HOST,
      xlib::NotifyNormal,
      xlib::NotifyVirtual,
    );
    assert_eq!(
      host_crossing(&event, HOST),
      Some(HostCrossing::Left(PhysicalPosition::new(1499, 772)))
    );
    // Onto an unrelated window, from the host itself, or as a grab ends (a drag released
    // outside): leaves too.
    for (mode, detail) in [
      (xlib::NotifyNormal, xlib::NotifyNonlinearVirtual),
      (xlib::NotifyNormal, xlib::NotifyAncestor),
      (xlib::NotifyUngrab, xlib::NotifyVirtual),
    ] {
      let event = crossing(xlib::LeaveNotify, HOST, mode, detail);
      assert!(matches!(
        host_crossing(&event, HOST),
        Some(HostCrossing::Left(_))
      ));
    }
  }

  #[test]
  fn moving_within_the_window_or_a_grab_is_not_a_leave() {
    // Between the host and a browser in it.
    let into_browser = crossing(
      xlib::LeaveNotify,
      HOST,
      xlib::NotifyNormal,
      xlib::NotifyInferior,
    );
    assert_eq!(host_crossing(&into_browser, HOST), None);
    // A window manager's grab on a click.
    let grab = crossing(
      xlib::LeaveNotify,
      HOST,
      xlib::NotifyGrab,
      xlib::NotifyVirtual,
    );
    assert_eq!(host_crossing(&grab, HOST), None);
    // Another window's crossing, or not a crossing at all.
    let other = crossing(
      xlib::LeaveNotify,
      BROWSER,
      xlib::NotifyNormal,
      xlib::NotifyVirtual,
    );
    assert_eq!(host_crossing(&other, HOST), None);
    let mut motion = crossing(
      xlib::LeaveNotify,
      HOST,
      xlib::NotifyNormal,
      xlib::NotifyVirtual,
    );
    motion.type_ = xlib::MotionNotify;
    assert_eq!(host_crossing(&motion, HOST), None);
  }

  #[test]
  fn coming_back_in_cancels_a_pending_leave() {
    let enter = crossing(
      xlib::EnterNotify,
      HOST,
      xlib::NotifyNormal,
      xlib::NotifyVirtual,
    );
    assert_eq!(host_crossing(&enter, HOST), Some(HostCrossing::Entered));
    let from_browser = crossing(
      xlib::EnterNotify,
      HOST,
      xlib::NotifyNormal,
      xlib::NotifyInferior,
    );
    assert_eq!(host_crossing(&from_browser, HOST), None);
  }

  #[test]
  fn the_leave_point_is_in_the_browsers_view_coordinates() {
    let at = PhysicalPosition::new(1499, 772);
    assert_eq!(
      point_in_view(at, PhysicalPosition::new(0, 0), 1.0),
      (1499, 772)
    );
    assert_eq!(
      point_in_view(at, PhysicalPosition::new(0, 0), 2.0),
      (750, 386)
    );
    assert_eq!(
      point_in_view(at, PhysicalPosition::new(99, 72), 1.0),
      (1400, 700)
    );
    // Left through the top or left edge: just outside the view.
    assert_eq!(
      point_in_view(
        PhysicalPosition::new(-1, 10),
        PhysicalPosition::new(0, 0),
        1.0
      ),
      (-1, 10)
    );
  }
}
