//! K46: the main window reopens where it was — same monitor, position, size and maximized state.
//!
//! The geometry lives in `AppSettings::window` (logical pixels, see `WindowGeometry`). At
//! startup [`placement`] checks it against the monitors present now and the window is built
//! with that position and size (winit-gtk4 applies a builder position at realize, before the
//! window is first shown). It is maximized right after, if it was.
//! Afterwards every move or resize saves it again, debounced, and the close saves it once more.
//! A maximized or minimized window keeps the last normal rect, plus `maximized`.
//!
//! A window manager may still put the new window elsewhere: GTK 4 rewrites the size hints as it
//! maps the window, without the user-position flag winit set at realize, so mutter (GNOME, here
//! through XWayland) can treat it as unplaced and place it itself, on the monitor it picks,
//! fitted to that monitor. So once the window is on screen and the window manager has settled
//! it, [`settle`] compares where it is with where it was asked to go and, if it's elsewhere, puts
//! it back ([`fixes`]): moved first, then resized, so the size isn't fitted to the wrong monitor.
//! mutter also maximizes a new window that covers more than 80 % of the work area, and
//! unmaximizing it restores a rect mutter shrank to 80 % of the work area, late enough to undo
//! the resize that followed (the window reopened smaller). So such a window is built smaller
//! ([`build_size`]) and given its size by that put-back; one maximized anyway is unmaximized on
//! its own first, and moved and resized once that has settled. Each step is checked again, a few
//! times at most ([`MAX_CHECKS`]). Until then the window is see-through ([`veil`]: built hidden,
//! given an X11 opacity of 0, then shown), so where the window manager put it first is never
//! seen; it's made visible once it's settled, or after [`VEIL_FALLBACK`] at the latest.
//!
//! The vendored CEF runtime supports all of it through winit-gtk4 on X11 (`outer_position`,
//! `set_position`, `inner_size`, `is_maximized`/`is_minimized`, the monitors with their work
//! areas, and the `Moved`/`Resized`/`CloseRequested` events), so it needed no patch. Under Wayland
//! (only when `GDK_BACKEND` isn't x11) a window can't place itself: only the size and the
//! maximized state come back.

use gitvolt_core::settings::{MonitorIdentity, SettingsStore, WindowGeometry, SAVE_DEBOUNCE};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{LogicalPosition, LogicalSize, Monitor, Runtime, WebviewWindow, WindowEvent};

/// Less of the window than this on screen (of its area, over all work areas) and it's re-centred.
pub const MIN_VISIBLE: f64 = 0.5;

/// Quiet time after the new window's last move, resize or focus before it's checked: the window
/// manager has placed it by then.
const SETTLE: Duration = Duration::from_millis(150);

/// A check later than this after the window was built could undo the user's own move: skipped.
const SETTLE_DEADLINE: Duration = Duration::from_secs(3);

/// Checks of the new window, at most: unmaximized, then moved and resized, then checked once
/// more, with two more tries if the window manager undid some of it (mutter can restore its
/// unmaximized rect after the window already reports it's unmaximized).
const MAX_CHECKS: u32 = 5;

/// The longest a new window stays see-through ([`Veil`]) after it's built: visible by then,
/// settled or not.
const VEIL_FALLBACK: Duration = Duration::from_millis(800);

/// Logical pixels a shown window may be off by (rounding) and still be where it was asked.
const SLACK: f64 = 2.0;

/// A rect in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }

    fn area(&self) -> f64 {
        self.width.max(0.0) * self.height.max(0.0)
    }

    fn intersection_area(&self, other: &Rect) -> f64 {
        let w = (self.x + self.width).min(other.x + other.width) - self.x.max(other.x);
        let h = (self.y + self.height).min(other.y + other.height) - self.y.max(other.y);
        w.max(0.0) * h.max(0.0)
    }

    fn same_as(&self, other: &Rect) -> bool {
        const EPS: f64 = 0.5;
        (self.x - other.x).abs() < EPS && (self.y - other.y).abs() < EPS && (self.width - other.width).abs() < EPS && (self.height - other.height).abs() < EPS
    }
}

/// A monitor present now, in logical pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    pub name: Option<String>,
    pub bounds: Rect,
    /// The bounds minus panels and docks.
    pub work_area: Rect,
}

impl Screen {
    pub fn identity(&self) -> MonitorIdentity {
        MonitorIdentity { name: self.name.clone(), x: self.bounds.x, y: self.bounds.y, width: self.bounds.width, height: self.bounds.height }
    }
}

/// How to build the window: its outer position (none: the window manager picks), its inner size,
/// and whether to maximize it once shown.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub position: Option<(f64, f64)>,
    pub size: (f64, f64),
    pub maximized: bool,
}

fn identity_rect(id: &MonitorIdentity) -> Rect {
    Rect::new(id.x, id.y, id.width, id.height)
}

/// The screen that is the saved monitor: the one with its name (the one at the same place, if
/// several share it), or, for a monitor saved without a name, the one with the same rect.
fn find_screen<'a>(id: &MonitorIdentity, screens: &'a [Screen]) -> Option<&'a Screen> {
    let rect = identity_rect(id);
    match &id.name {
        Some(name) => {
            let mut named = screens.iter().filter(|s| s.name.as_deref() == Some(name.as_str()));
            let first = named.clone().next();
            named.find(|s| s.bounds.same_as(&rect)).or(first)
        }
        None => screens.iter().find(|s| s.bounds.same_as(&rect)),
    }
}

/// `(w, h)` no larger than `area`, and whether each axis was shrunk.
fn clamp_size((w, h): (f64, f64), area: &Rect) -> ((f64, f64), bool, bool) {
    let cw = w.min(area.width);
    let ch = h.min(area.height);
    ((cw, ch), cw < w, ch < h)
}

fn centred_on(screen: &Screen, size: (f64, f64)) -> Placement {
    let (size, _, _) = clamp_size(size, &screen.work_area);
    let wa = screen.work_area;
    let x = wa.x + (wa.width - size.0) / 2.0;
    let y = wa.y + (wa.height - size.1) / 2.0;
    Placement { position: Some((x.round(), y.round())), size, maximized: false }
}

/// Where the saved window goes on the monitors present now; `None` if the saved geometry is
/// unusable (the window then opens at its configured default).
///
/// On its saved monitor (found by name, even if it moved in the layout) it keeps its place
/// relative to that monitor, shrunk to the monitor's work area if it no longer fits. If that
/// monitor is gone, or the window would be mostly off every work area, it's centred on the
/// primary monitor with its saved size, clamped to the work area. Without `can_position`
/// (Wayland), only its size comes back, clamped to the primary monitor.
pub fn placement(saved: &WindowGeometry, screens: &[Screen], primary: Option<usize>, can_position: bool) -> Option<Placement> {
    let size = (saved.width, saved.height);
    if !(size.0.is_finite() && size.1.is_finite() && size.0 >= 1.0 && size.1 >= 1.0) {
        return None;
    }
    let maximized = saved.maximized;
    let primary = primary.and_then(|i| screens.get(i)).or(screens.first());
    let Some(primary) = primary else {
        return Some(Placement { position: None, size, maximized });
    };
    let with_maximized = |p: Placement| Some(Placement { maximized, ..p });
    let position = match (saved.x, saved.y) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Some((x, y)),
        _ => None,
    };
    let Some((x, y)) = position.filter(|_| can_position) else {
        let (size, _, _) = clamp_size(size, &primary.work_area);
        return Some(Placement { position: None, size, maximized });
    };

    let target = match &saved.monitor {
        Some(id) => find_screen(id, screens).map(|s| (s, s.bounds.x - id.x, s.bounds.y - id.y)),
        // No monitor saved: whichever holds most of the window.
        None => screen_holding(&Rect::new(x, y, size.0, size.1), screens).map(|s| (s, 0.0, 0.0)),
    };
    let Some((screen, dx, dy)) = target else {
        return with_maximized(centred_on(primary, size));
    };
    let wa = screen.work_area;
    let (size, shrunk_w, shrunk_h) = clamp_size(size, &wa);
    let mut x = x + dx;
    let mut y = y + dy;
    // A window shrunk to fit is moved onto the work area along that axis.
    if shrunk_w {
        x = wa.x;
    }
    if shrunk_h {
        y = wa.y;
    }
    let rect = Rect::new(x, y, size.0, size.1);
    let visible: f64 = screens.iter().map(|s| rect.intersection_area(&s.work_area)).sum();
    if visible < MIN_VISIBLE * rect.area() {
        return with_maximized(centred_on(primary, size));
    }
    Some(Placement { position: Some((x, y)), size, maximized })
}

/// The screen whose work area holds most of `rect`; none if it's off all of them.
pub fn screen_holding<'a>(rect: &Rect, screens: &'a [Screen]) -> Option<&'a Screen> {
    screens
        .iter()
        .map(|s| (s, rect.intersection_area(&s.work_area)))
        .filter(|(_, a)| *a > 0.0)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(s, _)| s)
}

/// What the window reports now, in logical pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Observed {
    pub position: Option<(f64, f64)>,
    pub size: (f64, f64),
    pub maximized: bool,
    pub minimized: bool,
    pub monitor: Option<MonitorIdentity>,
}

fn same_monitor(a: &MonitorIdentity, b: &MonitorIdentity) -> bool {
    match (&a.name, &b.name) {
        (Some(x), Some(y)) => x == y,
        _ => identity_rect(a).same_as(&identity_rect(b)),
    }
}

/// The geometry to save after observing the window; `None` keeps what was saved (a minimized
/// window). A maximized one keeps the last normal rect (or the default size, if there's none
/// yet) with `maximized` set; maximized onto another monitor, that rect moves with it.
pub fn next_saved(prev: Option<&WindowGeometry>, now: Observed, default_size: (f64, f64)) -> Option<WindowGeometry> {
    if now.minimized {
        return None;
    }
    if !now.maximized {
        let (x, y) = now.position.unzip();
        return Some(WindowGeometry { x, y, width: now.size.0, height: now.size.1, maximized: false, monitor: now.monitor });
    }
    let mut g = prev.cloned().unwrap_or(WindowGeometry { width: default_size.0, height: default_size.1, ..Default::default() });
    g.maximized = true;
    match (&g.monitor, &now.monitor) {
        (Some(old), Some(new)) if !same_monitor(old, new) => {
            if let (Some(x), Some(y)) = (g.x, g.y) {
                g.x = Some(x - old.x + new.x);
                g.y = Some(y - old.y + new.y);
            }
            g.monitor = Some(new.clone());
        }
        (None, Some(new)) => g.monitor = Some(new.clone()),
        _ => {}
    }
    Some(g)
}

/// A step of putting the new window back where it was asked to go.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fix {
    Unmaximize,
    Move(f64, f64),
    Resize(f64, f64),
    Maximize,
}

/// What to do, in order, once the window manager has shown the window built at `place` (meant
/// for the `target` monitor) and it reports `now`. Nothing if it's where it was asked, or if it
/// was built without a position (the window manager's choice stands). A maximized window only
/// has to be on the right monitor.
///
/// Otherwise, if the window manager maximized it (mutter does to a window that maps over 80 % of
/// the work area), it's only unmaximized: the window manager then restores a rect of its own
/// (mutter's shrunk to 80 % of the work area), later, and a move or resize sent before that
/// lands is undone. The next check, once that has settled, moves it and only then resizes it, so
/// its size is checked against the right monitor; then maximizes it again if it was.
pub fn fixes(place: &Placement, target: Option<&MonitorIdentity>, now: &Observed) -> Vec<Fix> {
    let Some((x, y)) = place.position else {
        return Vec::new();
    };
    let on_target = match (target, &now.monitor) {
        (Some(t), Some(m)) => same_monitor(t, m),
        // Can't tell: the rect decides.
        _ => true,
    };
    let near = |a: f64, b: f64| (a - b).abs() <= SLACK;
    let in_place = now.position.is_some_and(|(nx, ny)| near(nx, x) && near(ny, y)) && near(now.size.0, place.size.0) && near(now.size.1, place.size.1);
    if on_target && (place.maximized || in_place) {
        return Vec::new();
    }
    if now.maximized {
        return vec![Fix::Unmaximize];
    }
    let mut fixes = vec![Fix::Move(x, y), Fix::Resize(place.size.0, place.size.1)];
    if place.maximized {
        fixes.push(Fix::Maximize);
    }
    fixes
}

/// `x,y wxh on <monitor>`, for the placement log line.
fn describe(position: Option<(f64, f64)>, size: (f64, f64), monitor: Option<&MonitorIdentity>) -> String {
    let at = position.map_or_else(|| "anywhere".to_owned(), |(x, y)| format!("{x},{y}"));
    let on = monitor.and_then(|m| m.name.as_deref()).unwrap_or("an unknown monitor");
    format!("{at} {}x{} on {on}", size.0, size.1)
}

/// Whether this process's windows can place themselves: X11, not Wayland. `GDK_BACKEND` is a
/// list in order of preference; without it, GDK picks Wayland when `WAYLAND_DISPLAY` is set.
pub fn can_self_position(gdk_backend: Option<&str>, wayland_display: Option<&str>) -> bool {
    match gdk_backend.map(str::trim).filter(|b| !b.is_empty()) {
        Some(list) => list.split(',').next().is_some_and(|first| first.trim() == "x11"),
        None => wayland_display.is_none_or(str::is_empty),
    }
}

/// This process, now: the CEF runtime forces `GDK_BACKEND=x11` before GTK starts.
pub fn can_self_position_now() -> bool {
    let backend = std::env::var("GDK_BACKEND").ok();
    let wayland = std::env::var("WAYLAND_DISPLAY").ok();
    can_self_position(backend.as_deref(), wayland.as_deref())
}

fn logical_rect(x: f64, y: f64, w: f64, h: f64, scale: f64) -> Rect {
    let s = if scale > 0.0 { scale } else { 1.0 };
    Rect::new(x / s, y / s, w / s, h / s)
}

pub fn screen_of(m: &Monitor) -> Screen {
    let s = m.scale_factor();
    let (p, z, wa) = (m.position(), m.size(), m.work_area());
    Screen {
        name: m.name().cloned(),
        bounds: logical_rect(p.x.into(), p.y.into(), z.width.into(), z.height.into(), s),
        work_area: logical_rect(wa.position.x.into(), wa.position.y.into(), wa.size.width.into(), wa.size.height.into(), s),
    }
}

/// The monitors present now and which of them is the primary one.
pub fn screens(available: &[Monitor], primary: Option<&Monitor>) -> (Vec<Screen>, Option<usize>) {
    let screens: Vec<Screen> = available.iter().map(screen_of).collect();
    let primary = primary.map(screen_of).and_then(|p| screens.iter().position(|s| s.name == p.name && s.bounds.same_as(&p.bounds)));
    (screens, primary)
}

struct Tracker {
    store: Arc<SettingsStore>,
    /// Bumped by every move/resize; a debounced save runs only if it's still the latest.
    generation: AtomicU64,
    can_position: bool,
    default_size: (f64, f64),
}

fn observe<R: Runtime>(w: &WebviewWindow<R>, can_position: bool) -> tauri::Result<Observed> {
    let scale = w.scale_factor()?;
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let position = if can_position { w.outer_position().ok().map(|p| (f64::from(p.x) / scale, f64::from(p.y) / scale)) } else { None };
    let size = w.inner_size()?;
    Ok(Observed {
        position,
        size: (f64::from(size.width) / scale, f64::from(size.height) / scale),
        maximized: w.is_maximized()?,
        minimized: w.is_minimized()?,
        monitor: w.current_monitor().ok().flatten().map(|m| screen_of(&m).identity()),
    })
}

impl Tracker {
    fn save<R: Runtime>(&self, w: &WebviewWindow<R>) {
        match observe(w, self.can_position) {
            Ok(now) => {
                if let Some(g) = next_saved(self.store.window().as_ref(), now, self.default_size) {
                    self.store.save_window(g);
                }
            }
            Err(e) => tracing::debug!("window geometry unavailable: {e}"),
        }
    }
}

/// Saves the window's geometry on every move and resize (debounced like the settings file,
/// `SAVE_DEBOUNCE`) and once more when it's asked to close.
pub fn track<R: Runtime>(window: &WebviewWindow<R>, store: Arc<SettingsStore>, can_position: bool, default_size: (f64, f64)) {
    let tracker = Arc::new(Tracker { store, generation: AtomicU64::new(0), can_position, default_size });
    let w = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Moved(_) | WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
            let generation = tracker.generation.fetch_add(1, Ordering::SeqCst) + 1;
            let (tracker, w) = (tracker.clone(), w.clone());
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(SAVE_DEBOUNCE).await;
                if tracker.generation.load(Ordering::SeqCst) == generation {
                    // The getters block on the event loop: off the async workers.
                    let _ = tauri::async_runtime::spawn_blocking(move || tracker.save(&w)).await;
                }
            });
        }
        // On the main thread, inside the runtime's event dispatch, where its getters are
        // answered inline (`RuntimeContext::send_message`). The exit flush then writes it.
        WindowEvent::CloseRequested { .. } => {
            tracker.generation.fetch_add(1, Ordering::SeqCst);
            tracker.save(&w);
        }
        _ => {}
    });
}

/// Whether to build the window see-through and only make it visible once it's where it was asked
/// ([`Veil`]): when it's built with a position, which a window manager may ignore. Without one
/// there's nothing to put back, so nothing to hide. X11 only, where the position is applied.
pub fn veiled(place: Option<&Placement>) -> bool {
    cfg!(target_os = "linux") && place.is_some_and(|p| p.position.is_some())
}

/// The most of the smallest work area a new window may cover and not be maximized by mutter
/// (which does that to one whose frame maps over 80 % of the work area), title bar included.
const BUILD_AREA: f64 = 0.7;

/// The size to build the window at `place` with. A see-through one (`veiled`) that mutter would
/// maximize on mapping is built smaller, keeping its proportions, under [`BUILD_AREA`] of the
/// smallest work area (mutter may map it on any monitor); [`settle`] then gives it its size.
/// Built at that size, mutter maximizes it, and unmaximizing restores a rect of mutter's, late:
/// a resize sent before that lands is undone. A maximized one is built at its size: maximizing
/// is what it wants.
pub fn build_size(place: &Placement, screens: &[Screen]) -> (f64, f64) {
    let (w, h) = place.size;
    let smallest = screens.iter().map(|s| s.work_area.area()).filter(|a| *a > 0.0).min_by(f64::total_cmp);
    let Some(smallest) = smallest.filter(|_| veiled(Some(place)) && !place.maximized) else {
        return place.size;
    };
    let scale = (BUILD_AREA * smallest / (w * h)).sqrt();
    if scale >= 1.0 {
        return place.size;
    }
    ((w * scale).floor(), (h * scale).floor())
}

/// Why the see-through window was made visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reveal {
    /// The window manager put it where it was asked.
    InPlace,
    /// It was put back, and its moves have gone quiet since.
    PutBack,
    /// It wasn't settled within [`VEIL_FALLBACK`] (or was shown too late to check at all).
    Fallback,
    /// Its geometry couldn't be read once shown, so it couldn't be checked.
    Failed,
}

impl std::fmt::Display for Reveal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Reveal::InPlace => "settled where it was asked",
            Reveal::PutBack => "settled after it was put back",
            Reveal::Fallback => "fallback: not settled in time",
            Reveal::Failed => "its geometry was unavailable",
        })
    }
}

/// What to do once the window has been quiet for `SETTLE`: check it (and put it back), or, if it
/// was checked [`MAX_CHECKS`] times already or is too late to check, make it visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    Check,
    Reveal(Reveal),
}

/// The step after a quiet `SETTLE`, `checks` done so far, `elapsed` since the window was built.
/// Only the first check has a deadline: the later ones follow the moves it asked for.
pub fn next_step(checks: u32, elapsed: Duration) -> Next {
    if checks >= MAX_CHECKS {
        Next::Reveal(Reveal::PutBack)
    } else if checks == 0 && elapsed > SETTLE_DEADLINE {
        Next::Reveal(Reveal::Fallback)
    } else {
        Next::Check
    }
}

/// After check number `check` (from 1): visible now if there was nothing (left) to put back;
/// otherwise checked again once the moves it asked for have gone quiet (`None`).
pub fn reveal_after(fixes: &[Fix], check: u32) -> Option<Reveal> {
    fixes.is_empty().then_some(if check > 1 { Reveal::PutBack } else { Reveal::InPlace })
}

/// Keeps the new window see-through until it's lifted, once: when it's settled where it was
/// asked, or [`VEIL_FALLBACK`] after it was built, whichever comes first.
pub struct Veil {
    lift: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl Veil {
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn new(lift: impl FnOnce() + Send + 'static) -> Self {
        Self { lift: std::sync::Mutex::new(Some(Box::new(lift))) }
    }

    /// Makes the window visible, the first time only; whether this call did.
    pub fn lift(&self, why: Reveal) -> bool {
        let Some(lift) = self.lift.lock().unwrap_or_else(|e| e.into_inner()).take() else {
            return false;
        };
        tracing::debug!("window placement: shown ({why})");
        lift();
        true
    }

    pub fn lifted(&self) -> bool {
        self.lift.lock().unwrap_or_else(|e| e.into_inner()).is_none()
    }
}

/// Shows `window`, built hidden (`veiled`), see-through: its `_NET_WM_WINDOW_OPACITY` at 0,
/// which mutter's compositor applies to the whole window, frame and browser included. It's set
/// before the window is mapped, so the window manager's own placement is never seen. Lifting the
/// veil makes it opaque and focuses it, as the runtime does for a window built visible. If it
/// can't be made see-through it's just shown (`None`). On the main thread (`setup`).
#[cfg(target_os = "linux")]
pub fn veil<R: Runtime>(window: &WebviewWindow<R>) -> Option<Arc<Veil>> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use tauri::Manager as _;
    let xid = match window.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Xlib(h)) => Some(h.window),
        Ok(RawWindowHandle::Xcb(h)) => Some(h.window.get().into()),
        _ => None,
    };
    let hidden = xid.filter(|&xid| x11_opacity(xid, Some(0)));
    if let Err(e) = window.show() {
        tracing::warn!("couldn't show the window: {e}");
    }
    let Some(xid) = hidden else {
        tracing::debug!("window placement: couldn't make the window see-through, shown as is");
        return None;
    };
    let w = window.clone();
    Some(Arc::new(Veil::new(move || {
        if let Err(e) = w.run_on_main_thread(move || _ = x11_opacity(xid, None)) {
            tracing::warn!("couldn't make the window opaque: {e}");
        }
        if let Err(e) = w.set_focus() {
            tracing::debug!("couldn't focus the window once shown: {e}");
        }
    })))
}

#[cfg(not(target_os = "linux"))]
pub fn veil<R: Runtime>(window: &WebviewWindow<R>) -> Option<Arc<Veil>> {
    if let Err(e) = window.show() {
        tracing::warn!("couldn't show the window: {e}");
    }
    None
}

/// Sets the X11 window's `_NET_WM_WINDOW_OPACITY` (0 to `u32::MAX`), or removes it (`None`:
/// opaque), on GDK's own display, so it's ordered before GDK's later map; whether the X server
/// took it. Main thread only.
#[cfg(target_os = "linux")]
fn x11_opacity(xid: std::os::raw::c_ulong, opacity: Option<u32>) -> bool {
    use gdk4_x11::gdk::prelude::*;
    use x11_dl::xlib;
    static XLIB: std::sync::LazyLock<Option<xlib::Xlib>> = std::sync::LazyLock::new(|| xlib::Xlib::open().ok());
    let Some(x) = XLIB.as_ref() else {
        return false;
    };
    let Some(display) = gdk4_x11::gdk::Display::default().and_then(|d| d.downcast::<gdk4_x11::X11Display>().ok()) else {
        return false;
    };
    let atom = gdk4_x11::x11_get_xatom_by_name_for_display(&display, "_NET_WM_WINDOW_OPACITY");
    // SAFETY: Xlib calls on GDK's live display, on the GTK main thread, with X errors trapped
    // (a window already gone is only a `BadWindow`).
    unsafe {
        let xdisplay = display.xdisplay().cast::<xlib::Display>();
        display.error_trap_push();
        match opacity {
            Some(value) => {
                let data = std::os::raw::c_ulong::from(value);
                (x.XChangeProperty)(xdisplay, xid, atom, xlib::XA_CARDINAL, 32, xlib::PropModeReplace, (&raw const data).cast(), 1);
            }
            None => {
                (x.XDeleteProperty)(xdisplay, xid, atom);
            }
        }
        (x.XFlush)(xdisplay);
        display.error_trap_pop() == 0
    }
}

/// The window built at `place`, settling: checked once the window manager has shown it, put
/// back if need be and checked again, and made visible once that's done (its `veil`).
struct Settler<R: Runtime> {
    window: WebviewWindow<R>,
    place: Placement,
    target: Option<MonitorIdentity>,
    built: Instant,
    /// Bumped by every move, resize and focus; a step runs only after the latest.
    generation: AtomicU64,
    /// Checks started so far.
    checks: AtomicU32,
    /// It's where it was asked, or won't be checked again.
    done: AtomicBool,
    veil: Option<Arc<Veil>>,
}

impl<R: Runtime> Settler<R> {
    fn finished(&self) -> bool {
        self.done.load(Ordering::SeqCst) && self.veil.as_ref().is_none_or(|v| v.lifted())
    }

    fn reveal(&self, why: Reveal) {
        self.done.store(true, Ordering::SeqCst);
        if let Some(veil) = &self.veil {
            veil.lift(why);
        }
    }

    /// The window moved, was resized or focused: the next step once it's quiet for `SETTLE`.
    fn stirred(self: &Arc<Self>) {
        let g = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let s = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(SETTLE).await;
            if s.generation.load(Ordering::SeqCst) != g || s.done.load(Ordering::SeqCst) {
                return;
            }
            match next_step(s.checks.load(Ordering::SeqCst), s.built.elapsed()) {
                Next::Check => {
                    let check = s.checks.fetch_add(1, Ordering::SeqCst) + 1;
                    // The getters block on the event loop: off the async workers.
                    _ = tauri::async_runtime::spawn_blocking(move || s.check(check)).await;
                }
                Next::Reveal(Reveal::Fallback) => {
                    tracing::debug!("window placement: shown too late to check, left where it is");
                    s.reveal(Reveal::Fallback);
                }
                Next::Reveal(why) => s.reveal(why),
            }
        });
    }

    fn check(self: &Arc<Self>, check: u32) {
        match put_back(&self.window, &self.place, self.target.as_ref()) {
            Ok(fixes) => match reveal_after(&fixes, check) {
                Some(why) => self.reveal(why),
                // Checked again once its moves have gone quiet, even if they send no event.
                None => self.stirred(),
            },
            Err(e) => {
                tracing::debug!("window placement: geometry unavailable once shown: {e}");
                self.reveal(Reveal::Failed);
            }
        }
    }
}

/// Once the window manager has shown the window built at `place` (its first moves, resizes and
/// focus have gone quiet for `SETTLE`), puts it back where it was asked to go if it's elsewhere
/// ([`fixes`]), and checks it again once each step has gone quiet, until it's in place or
/// [`MAX_CHECKS`] checks are done. Only for a window built with a position. A `veil` (`veiled`)
/// is lifted once that's settled, or after [`VEIL_FALLBACK`] whatever happened.
pub fn settle<R: Runtime>(window: &WebviewWindow<R>, place: Placement, screens: &[Screen], veil: Option<Arc<Veil>>) {
    let Some((x, y)) = place.position else {
        if let Some(veil) = veil {
            veil.lift(Reveal::InPlace);
        }
        return;
    };
    if let Some(veil) = veil.clone() {
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(VEIL_FALLBACK).await;
            veil.lift(Reveal::Fallback);
        });
    }
    let target = screen_holding(&Rect::new(x, y, place.size.0, place.size.1), screens).map(Screen::identity);
    let settler = Arc::new(Settler {
        window: window.clone(),
        place,
        target,
        built: Instant::now(),
        generation: AtomicU64::new(0),
        checks: AtomicU32::new(0),
        done: AtomicBool::new(false),
        veil,
    });
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_) | WindowEvent::Focused(true)) && !settler.finished() {
            settler.stirred();
        }
    });
}

/// Puts the window back where it was asked to go, if it isn't; the steps taken.
fn put_back<R: Runtime>(w: &WebviewWindow<R>, place: &Placement, target: Option<&MonitorIdentity>) -> tauri::Result<Vec<Fix>> {
    let now = observe(w, true)?;
    let fixes = fixes(place, target, &now);
    tracing::debug!(
        "window placement: asked {} (maximized: {}), shown at {}; {}",
        describe(place.position, place.size, target),
        place.maximized,
        describe(now.position, now.size, now.monitor.as_ref()),
        if fixes.is_empty() { "left as is".to_owned() } else { format!("putting it back: {fixes:?}") },
    );
    for &fix in &fixes {
        let done = match fix {
            Fix::Unmaximize => w.unmaximize(),
            Fix::Move(x, y) => w.set_position(LogicalPosition::new(x, y)),
            Fix::Resize(width, height) => w.set_size(LogicalSize::new(width, height)),
            Fix::Maximize => w.maximize(),
        };
        if let Err(e) = done {
            tracing::warn!("couldn't put the window back where it was ({fix:?}): {e}");
        }
    }
    Ok(fixes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(name: &str, x: f64, y: f64, w: f64, h: f64) -> Screen {
        // A 32 px top bar, like GNOME's.
        Screen { name: Some(name.into()), bounds: Rect::new(x, y, w, h), work_area: Rect::new(x, y + 32.0, w, h - 32.0) }
    }

    fn laptop() -> Screen {
        screen("eDP-1", 0.0, 0.0, 1920.0, 1200.0)
    }

    fn external() -> Screen {
        screen("DP-1", 1920.0, 0.0, 2560.0, 1440.0)
    }

    fn saved_on(s: &Screen, x: f64, y: f64, w: f64, h: f64) -> WindowGeometry {
        WindowGeometry { x: Some(x), y: Some(y), width: w, height: h, maximized: false, monitor: Some(s.identity()) }
    }

    #[test]
    fn a_window_on_a_monitor_still_there_comes_back_in_place() {
        let g = saved_on(&external(), 2100.0, 100.0, 1600.0, 900.0);
        let p = placement(&g, &[laptop(), external()], Some(0), true).unwrap();
        assert_eq!(p, Placement { position: Some((2100.0, 100.0)), size: (1600.0, 900.0), maximized: false });
    }

    #[test]
    fn a_monitor_that_moved_in_the_layout_takes_the_window_with_it() {
        let g = saved_on(&external(), 2100.0, 100.0, 1600.0, 900.0);
        // The external monitor is now left of the laptop.
        let moved = screen("DP-1", -2560.0, 0.0, 2560.0, 1440.0);
        let p = placement(&g, &[laptop(), moved], Some(0), true).unwrap();
        assert_eq!(p.position, Some((-2560.0 + 180.0, 100.0)));
    }

    #[test]
    fn a_monitor_that_is_gone_centres_the_window_on_the_primary_one() {
        let g = saved_on(&external(), 2100.0, 100.0, 1600.0, 900.0);
        let p = placement(&g, &[laptop()], Some(0), true).unwrap();
        // Work area 0,32 1920x1168.
        assert_eq!(p, Placement { position: Some((160.0, 166.0)), size: (1600.0, 900.0), maximized: false });
    }

    #[test]
    fn the_fallback_clamps_the_saved_size_to_the_work_area() {
        let g = saved_on(&external(), 1920.0, 32.0, 2560.0, 1400.0);
        let p = placement(&g, &[laptop()], Some(0), true).unwrap();
        assert_eq!(p, Placement { position: Some((0.0, 32.0)), size: (1920.0, 1168.0), maximized: false });
    }

    #[test]
    fn a_window_mostly_offscreen_is_recentred() {
        // On its monitor by name, but 90 % off the right edge of everything.
        let g = saved_on(&laptop(), 1800.0, 100.0, 1200.0, 800.0);
        let p = placement(&g, &[laptop()], Some(0), true).unwrap();
        assert_eq!(p.position, Some((360.0, 216.0)));
        // Half on screen is enough to stay.
        let g = saved_on(&laptop(), 1320.0, 100.0, 1200.0, 800.0);
        assert_eq!(placement(&g, &[laptop()], Some(0), true).unwrap().position, Some((1320.0, 100.0)));
    }

    #[test]
    fn a_window_too_big_for_its_smaller_monitor_shrinks_onto_it() {
        // Same connector, lower resolution now.
        let g = saved_on(&external(), 1940.0, 40.0, 2500.0, 1380.0);
        let smaller = screen("DP-1", 1920.0, 0.0, 1920.0, 1080.0);
        let p = placement(&g, &[laptop(), smaller], Some(0), true).unwrap();
        assert_eq!(p, Placement { position: Some((1920.0, 32.0)), size: (1920.0, 1048.0), maximized: false });
    }

    #[test]
    fn the_primary_monitor_is_the_fallback_not_the_first() {
        let g = saved_on(&screen("HDMI-1", 5000.0, 0.0, 1920.0, 1080.0), 5100.0, 100.0, 1000.0, 600.0);
        let p = placement(&g, &[laptop(), external()], Some(1), true).unwrap();
        // DP-1's work area: 1920,32 2560x1408.
        assert_eq!(p.position, Some((1920.0 + 780.0, 32.0 + 404.0)));
    }

    #[test]
    fn maximized_comes_back_on_top_of_the_normal_rect_even_on_fallback() {
        let mut g = saved_on(&external(), 2100.0, 100.0, 1600.0, 900.0);
        g.maximized = true;
        assert!(placement(&g, &[laptop(), external()], Some(0), true).unwrap().maximized);
        let p = placement(&g, &[laptop()], Some(0), true).unwrap();
        assert!(p.maximized);
        assert_eq!(p.position, Some((160.0, 166.0)));
    }

    #[test]
    fn without_self_positioning_only_the_size_comes_back() {
        let mut g = saved_on(&external(), 2100.0, 100.0, 2400.0, 900.0);
        g.maximized = true;
        let p = placement(&g, &[laptop(), external()], Some(0), false).unwrap();
        assert_eq!(p, Placement { position: None, size: (1920.0, 900.0), maximized: true });
        // Saved on Wayland: no position at all.
        let g = WindowGeometry { width: 1200.0, height: 700.0, ..Default::default() };
        assert_eq!(placement(&g, &[laptop()], Some(0), true).unwrap().position, None);
    }

    #[test]
    fn a_monitor_saved_without_a_name_is_matched_by_its_rect() {
        let mut g = saved_on(&external(), 2100.0, 100.0, 1600.0, 900.0);
        g.monitor.as_mut().unwrap().name = None;
        let unnamed = Screen { name: None, ..external() };
        assert_eq!(placement(&g, &[laptop(), unnamed], Some(0), true).unwrap().position, Some((2100.0, 100.0)));
        // Different rect: gone.
        let other = Screen { name: None, ..screen("", 1920.0, 0.0, 1920.0, 1080.0) };
        assert_eq!(placement(&g, &[laptop(), other], Some(0), true).unwrap().position, Some((160.0, 166.0)));
    }

    #[test]
    fn unusable_or_screenless_geometry() {
        let g = WindowGeometry { width: 0.0, height: 700.0, ..Default::default() };
        assert_eq!(placement(&g, &[laptop()], Some(0), true), None);
        let g = WindowGeometry { width: f64::NAN, height: 700.0, ..Default::default() };
        assert_eq!(placement(&g, &[laptop()], Some(0), true), None);
        let g = saved_on(&laptop(), 10.0, 40.0, 1000.0, 700.0);
        assert_eq!(placement(&g, &[], None, true), Some(Placement { position: None, size: (1000.0, 700.0), maximized: false }));
    }

    fn observed(x: f64, y: f64, w: f64, h: f64, monitor: &Screen) -> Observed {
        Observed { position: Some((x, y)), size: (w, h), maximized: false, minimized: false, monitor: Some(monitor.identity()) }
    }

    #[test]
    fn a_normal_window_saves_what_it_sees() {
        let g = next_saved(None, observed(100.0, 50.0, 1400.0, 800.0, &laptop()), (1600.0, 900.0)).unwrap();
        assert_eq!(g, saved_on(&laptop(), 100.0, 50.0, 1400.0, 800.0));
    }

    #[test]
    fn maximized_keeps_the_last_normal_rect() {
        let prev = saved_on(&laptop(), 100.0, 50.0, 1400.0, 800.0);
        let now = Observed { maximized: true, ..observed(0.0, 32.0, 1920.0, 1168.0, &laptop()) };
        let g = next_saved(Some(&prev), now, (1600.0, 900.0)).unwrap();
        assert_eq!(g, WindowGeometry { maximized: true, ..prev });
    }

    #[test]
    fn maximized_with_nothing_saved_yet_keeps_the_default_size() {
        let now = Observed { maximized: true, ..observed(0.0, 32.0, 1920.0, 1168.0, &laptop()) };
        let g = next_saved(None, now, (1600.0, 900.0)).unwrap();
        assert_eq!(g, WindowGeometry { x: None, y: None, width: 1600.0, height: 900.0, maximized: true, monitor: Some(laptop().identity()) });
    }

    #[test]
    fn maximized_onto_another_monitor_moves_the_normal_rect_along() {
        let prev = saved_on(&laptop(), 100.0, 50.0, 1400.0, 800.0);
        let now = Observed { maximized: true, ..observed(1920.0, 32.0, 2560.0, 1408.0, &external()) };
        let g = next_saved(Some(&prev), now, (1600.0, 900.0)).unwrap();
        assert_eq!(g, WindowGeometry { maximized: true, ..saved_on(&external(), 2020.0, 50.0, 1400.0, 800.0) });
        // And it restores there, maximized.
        let p = placement(&g, &[laptop(), external()], Some(0), true).unwrap();
        assert_eq!(p, Placement { position: Some((2020.0, 50.0)), size: (1400.0, 800.0), maximized: true });
    }

    #[test]
    fn minimized_saves_nothing() {
        let prev = saved_on(&laptop(), 100.0, 50.0, 1400.0, 800.0);
        let now = Observed { minimized: true, ..observed(-32000.0, -32000.0, 160.0, 28.0, &laptop()) };
        assert_eq!(next_saved(Some(&prev), now, (1600.0, 900.0)), None);
    }

    #[test]
    fn self_positioning_only_on_x11() {
        assert!(can_self_position(Some("x11"), Some("wayland-0")));
        assert!(can_self_position(Some("x11,wayland"), None));
        assert!(!can_self_position(Some("wayland"), None));
        assert!(!can_self_position(Some("wayland,x11"), Some("wayland-0")));
        assert!(can_self_position(None, None));
        assert!(can_self_position(Some(""), Some("")));
        assert!(!can_self_position(None, Some("wayland-0")));
    }

    /// Wider than the external monitor, right of it.
    fn wide() -> Screen {
        screen("DP-2", 4480.0, 0.0, 3440.0, 1440.0)
    }

    fn asked(x: f64, y: f64, w: f64, h: f64) -> Placement {
        Placement { position: Some((x, y)), size: (w, h), maximized: false }
    }

    #[test]
    fn the_screen_holding_a_rect_is_the_one_with_most_of_it() {
        let screens = [laptop(), external(), wide()];
        let on = |x, w| screen_holding(&Rect::new(x, 100.0, w, 800.0), &screens).and_then(|s| s.name.clone());
        assert_eq!(on(4620.0, 3182.0), Some("DP-2".into()));
        // Straddling: the larger share wins.
        assert_eq!(on(1700.0, 1000.0), Some("DP-1".into()));
        assert_eq!(on(-5000.0, 1000.0), None);
    }

    #[test]
    fn a_window_shown_where_it_was_asked_is_left_alone() {
        let place = asked(4620.0, 66.0, 3182.0, 1296.0);
        let now = observed(4620.0, 66.0, 3182.0, 1296.0, &wide());
        assert_eq!(fixes(&place, Some(&wide().identity()), &now), vec![]);
        // Off by a pixel of rounding is still there.
        let now = observed(4621.0, 65.0, 3181.0, 1296.0, &wide());
        assert_eq!(fixes(&place, Some(&wide().identity()), &now), vec![]);
    }

    #[test]
    fn a_window_shown_on_another_monitor_is_moved_back_then_resized() {
        // The window manager placed it on the monitor it chose, fitted to it.
        let place = asked(4620.0, 66.0, 3182.0, 1296.0);
        let now = observed(1920.0, 32.0, 2560.0, 1296.0, &external());
        assert_eq!(fixes(&place, Some(&wide().identity()), &now), vec![Fix::Move(4620.0, 66.0), Fix::Resize(3182.0, 1296.0)]);
    }

    #[test]
    fn a_window_the_window_manager_maximized_is_unmaximized_before_it_is_put_back() {
        // The user's 3189x1299 on a 3440x1440 monitor: mutter maximizes a window that maps over
        // 80 % of the work area.
        let place = asked(4624.0, 76.0, 3189.0, 1299.0);
        let target = wide().identity();
        let now = Observed { maximized: true, ..observed(4480.0, 0.0, 3440.0, 1401.0, &wide()) };
        assert_eq!(fixes(&place, Some(&target), &now), vec![Fix::Unmaximize]);
        // Unmaximized, it's at mutter's own restore rect, 80 % of the work area: now moved and
        // resized, not before, or mutter's late restore undoes it.
        let now = observed(4662.0, 75.0, 3076.0, 1251.0, &wide());
        assert_eq!(fixes(&place, Some(&target), &now), vec![Fix::Move(4624.0, 76.0), Fix::Resize(3189.0, 1299.0)]);
        // Then it's where it was asked: at the size it was closed at.
        let now = observed(4624.0, 76.0, 3189.0, 1299.0, &wide());
        assert_eq!(fixes(&place, Some(&target), &now), vec![]);
        // Each step is checked once it has gone quiet, a few times at most, then shown.
        assert_eq!(reveal_after(&[Fix::Unmaximize], 1), None);
        assert_eq!(next_step(1, SETTLE_DEADLINE + Duration::from_millis(1)), Next::Check);
        assert_eq!(reveal_after(&[], 3), Some(Reveal::PutBack));
        assert_eq!(next_step(MAX_CHECKS, Duration::from_millis(900)), Next::Reveal(Reveal::PutBack));
    }

    #[test]
    fn a_window_shown_on_its_monitor_but_moved_or_shrunk_is_put_back() {
        let place = asked(4620.0, 66.0, 3182.0, 1296.0);
        let want = vec![Fix::Move(4620.0, 66.0), Fix::Resize(3182.0, 1296.0)];
        let target = wide().identity();
        assert_eq!(fixes(&place, Some(&target), &observed(4800.0, 66.0, 3182.0, 1296.0, &wide())), want);
        assert_eq!(fixes(&place, Some(&target), &observed(4620.0, 66.0, 2560.0, 1296.0, &wide())), want);
        // No position reported: it can't be shown to be in place.
        let now = Observed { position: None, ..observed(0.0, 0.0, 3182.0, 1296.0, &wide()) };
        assert_eq!(fixes(&place, Some(&target), &now), want);
    }

    #[test]
    fn a_maximized_window_only_has_to_be_on_its_monitor() {
        let place = Placement { maximized: true, ..asked(4620.0, 66.0, 3182.0, 1296.0) };
        let target = wide().identity();
        let now = Observed { maximized: true, ..observed(4480.0, 32.0, 3440.0, 1408.0, &wide()) };
        assert_eq!(fixes(&place, Some(&target), &now), vec![]);
        // Maximized on the wrong one: unmaximized, then back to the normal rect on the right one,
        // then maximized there.
        let now = Observed { maximized: true, ..observed(1920.0, 32.0, 2560.0, 1408.0, &external()) };
        assert_eq!(fixes(&place, Some(&target), &now), vec![Fix::Unmaximize]);
        let now = observed(2000.0, 100.0, 2048.0, 1126.0, &external());
        assert_eq!(fixes(&place, Some(&target), &now), vec![Fix::Move(4620.0, 66.0), Fix::Resize(3182.0, 1296.0), Fix::Maximize]);
    }

    #[test]
    fn a_window_built_without_a_position_is_where_the_window_manager_put_it() {
        let place = Placement { position: None, size: (1200.0, 700.0), maximized: false };
        assert_eq!(fixes(&place, None, &observed(10.0, 40.0, 900.0, 700.0, &laptop())), vec![]);
    }

    #[test]
    fn without_a_known_monitor_the_rect_decides() {
        let place = asked(2100.0, 100.0, 1600.0, 900.0);
        let now = Observed { monitor: None, ..observed(2100.0, 100.0, 1600.0, 900.0, &external()) };
        assert_eq!(fixes(&place, Some(&external().identity()), &now), vec![]);
        let now = observed(160.0, 166.0, 1600.0, 900.0, &laptop());
        assert_eq!(fixes(&place, None, &now).len(), 2);
    }

    #[test]
    fn only_a_window_built_with_a_position_is_hidden_until_placed() {
        assert_eq!(veiled(Some(&asked(4620.0, 66.0, 3182.0, 1296.0))), cfg!(target_os = "linux"));
        // Wayland, or no saved position: the window manager's place stands, nothing to hide.
        assert!(!veiled(Some(&Placement { position: None, size: (1200.0, 700.0), maximized: false })));
        // First launch: nothing saved.
        assert!(!veiled(None));
    }

    #[test]
    fn a_window_mutter_would_maximize_is_built_smaller_then_given_its_size() {
        let screens = [external(), wide()];
        let place = asked(4624.0, 76.0, 3189.0, 1299.0);
        let (w, h) = build_size(&place, &screens);
        if cfg!(target_os = "linux") {
            // Under 70 % of the smaller monitor's work area (2560x1408), in proportion.
            assert_eq!((w, h), (2488.0, 1013.0));
            // With a 39 px title bar it maps well under mutter's 80 % on either monitor.
            assert!(w * (h + 39.0) < 0.8 * 2560.0 * 1408.0);
            // Settling then gives it its size.
            let now = observed(4580.0, 199.0, w, h, &wide());
            assert_eq!(fixes(&place, Some(&wide().identity()), &now), vec![Fix::Move(4624.0, 76.0), Fix::Resize(3189.0, 1299.0)]);
        } else {
            assert_eq!((w, h), place.size);
        }
        // A window small enough, a maximized one, or one that won't be settled (no position):
        // built at its size.
        assert_eq!(build_size(&asked(4624.0, 76.0, 1600.0, 900.0), &screens), (1600.0, 900.0));
        let maximized = Placement { maximized: true, ..place.clone() };
        assert_eq!(build_size(&maximized, &screens), (3189.0, 1299.0));
        let unplaced = Placement { position: None, ..place };
        assert_eq!(build_size(&unplaced, &screens), (3189.0, 1299.0));
        assert_eq!(build_size(&asked(0.0, 0.0, 3189.0, 1299.0), &[]), (3189.0, 1299.0));
    }

    #[test]
    fn a_quiet_window_is_checked_then_shown() {
        assert_eq!(next_step(0, Duration::from_millis(300)), Next::Check);
        // Put back: checked again once its moves have gone quiet, up to MAX_CHECKS times.
        assert_eq!(next_step(1, Duration::from_millis(600)), Next::Check);
        assert_eq!(next_step(MAX_CHECKS, Duration::from_millis(1200)), Next::Reveal(Reveal::PutBack));
        // Shown too late to check: left where it is, and shown.
        assert_eq!(next_step(0, SETTLE_DEADLINE + Duration::from_millis(1)), Next::Reveal(Reveal::Fallback));
    }

    #[test]
    fn a_window_in_place_is_shown_at_once_one_put_back_once_checked_again() {
        assert_eq!(reveal_after(&[], 1), Some(Reveal::InPlace));
        assert_eq!(reveal_after(&[Fix::Move(4620.0, 66.0), Fix::Resize(3182.0, 1296.0)], 1), None);
        assert_eq!(reveal_after(&[], 2), Some(Reveal::PutBack));
    }

    #[test]
    fn the_veil_lifts_once_whichever_reason_comes_first() {
        let lifts = Arc::new(AtomicU64::new(0));
        let counted = lifts.clone();
        let veil = Veil::new(move || _ = counted.fetch_add(1, Ordering::SeqCst));
        assert!(!veil.lifted());
        assert!(veil.lift(Reveal::PutBack));
        assert!(veil.lifted());
        // The fallback timer, firing after it settled, does nothing.
        assert!(!veil.lift(Reveal::Fallback));
        assert_eq!(lifts.load(Ordering::SeqCst), 1);
    }
}
