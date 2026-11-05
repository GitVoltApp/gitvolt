//! App zoom (`ui/src/ui/zoom.ts`): Chromium scales the whole page; here the platform's window says it's that much
//! denser than it is: a larger scale factor over a smaller logical size, so everything lays out and rasterizes as at a
//! browser's zoom, sharp. `Zoomed` wraps the platform only to wrap its windows; the delegation below is mechanical, one
//! method per trait method of `gpui`'s `Platform` and `PlatformWindow`.

use gpui::*;
use smallvec::SmallVec;
use raw_window_handle::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::ffi::OsString;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use anyhow::Result;
use futures::channel::oneshot;

/// `ZOOM_STEPS`.
pub const ZOOM_STEPS: [u32; 12] = [80, 90, 100, 110, 120, 130, 140, 150, 175, 200, 250, 300];

static ZOOM: AtomicU32 = AtomicU32::new(100);

thread_local! {
    /// The cursor last asked of the platform, for the real-input flows: the window server's isn't readable from a client.
    static LAST_CURSOR: RefCell<String> = const { RefCell::new(String::new()) };
}

pub fn last_cursor() -> String {
    LAST_CURSOR.with_borrow(Clone::clone)
}

pub fn percent() -> u32 {
    ZOOM.load(Relaxed)
}

fn factor() -> f32 {
    percent() as f32 / 100.
}

/// `nextZoom`: the step after `current` that way, clamped; `0` resets.
pub fn next_zoom(current: u32, dir: i32) -> u32 {
    match dir {
        0 => 100,
        d if d > 0 => ZOOM_STEPS.iter().copied().find(|s| *s > current).unwrap_or(300),
        _ => ZOOM_STEPS.iter().rev().copied().find(|s| *s < current).unwrap_or(80),
    }
}

/// `parseZoom`: anything that isn't one of the steps isn't a zoom.
pub fn parse_zoom(raw: Option<u64>) -> Option<u32> {
    raw.map(|n| n as u32).filter(|n| ZOOM_STEPS.contains(n))
}

type Resized = Rc<RefCell<Option<Box<dyn FnMut(Size<Pixels>, f32)>>>>;
type ResizedWeak = std::rc::Weak<RefCell<Option<Box<dyn FnMut(Size<Pixels>, f32)>>>>;
type ReadSize = Box<dyn Fn() -> Option<(Size<Pixels>, f32)>>;

thread_local! {
    /// The window's resize callback and how to read its true size: what a change of zoom goes through.
    static WINDOW: RefCell<Option<(ResizedWeak, ReadSize)>> = const { RefCell::new(None) };
}

/// Sets the zoom before the first window, or for a shown one: it relays out as on a resize. Not from inside a window's
/// update: `GitVolt::set_zoom` defers it.
pub fn set_percent(p: u32) {
    ZOOM.store(p, Relaxed);
    WINDOW.with_borrow(|w| {
        let Some((resized, read)) = w else { return };
        let (Some((size_now, scale)), Some(resized)) = (read(), resized.upgrade()) else { return };
        if let Some(cb) = resized.borrow_mut().as_mut() {
            cb(scaled_down(size_now), scale * factor());
        }
    });
}

fn scaled_down(s: Size<Pixels>) -> Size<Pixels> {
    size(s.width / factor(), s.height / factor())
}

fn scaled_up(s: Size<Pixels>) -> Size<Pixels> {
    size(s.width * factor(), s.height * factor())
}

fn down(p: Point<Pixels>) -> Point<Pixels> {
    point(p.x / factor(), p.y / factor())
}

fn up(p: Point<Pixels>) -> Point<Pixels> {
    point(p.x * factor(), p.y * factor())
}

fn input_down(e: PlatformInput) -> PlatformInput {
    match e {
        PlatformInput::MouseDown(mut x) => {
            x.position = down(x.position);
            PlatformInput::MouseDown(x)
        }
        PlatformInput::MouseUp(mut x) => {
            x.position = down(x.position);
            PlatformInput::MouseUp(x)
        }
        PlatformInput::MousePressure(mut x) => {
            x.position = down(x.position);
            PlatformInput::MousePressure(x)
        }
        PlatformInput::MouseMove(mut x) => {
            x.position = down(x.position);
            PlatformInput::MouseMove(x)
        }
        PlatformInput::MouseExited(mut x) => {
            x.position = down(x.position);
            PlatformInput::MouseExited(x)
        }
        PlatformInput::Scroll(mut x) => {
            x.position = down(x.position);
            if let ScrollDelta::Pixels(d) = x.delta {
                x.delta = ScrollDelta::Pixels(down(d));
            }
            PlatformInput::Scroll(x)
        }
        other => other,
    }
}

pub struct Zoomed(pub Rc<dyn Platform>);

impl Platform for Zoomed {
    fn open_window(&self, handle: AnyWindowHandle, mut options: WindowParams) -> anyhow::Result<Box<dyn PlatformWindow>> {
        options.bounds.size = scaled_up(options.bounds.size);
        let inner = self.0.open_window(handle, options)?;
        let inner = Rc::new(RefCell::new(inner));
        let resized: Resized = Rc::default();
        // Weak: the window goes when GPUI drops it, not with this thread's locals after the display connection's gone.
        let i2 = Rc::downgrade(&inner);
        WINDOW.set(Some((Rc::downgrade(&resized), Box::new(move || {
            let w = i2.upgrade()?;
            let w = w.borrow();
            Some((w.content_size(), w.scale_factor()))
        }))));
        Ok(Box::new(ZoomWindow { inner: SharedWindow(inner), resized }))
    }

    fn platform_info(&self) -> PlatformInfo {
        self.0.platform_info()
    }
    fn background_executor(&self) -> BackgroundExecutor {
        self.0.background_executor()
    }
    fn foreground_executor(&self) -> ForegroundExecutor {
        self.0.foreground_executor()
    }
    fn text_system(&self) -> Arc<dyn PlatformTextSystem> {
        self.0.text_system()
    }
    fn run(&self, on_finish_launching: Box<dyn 'static + FnOnce()>) {
        self.0.run(on_finish_launching)
    }
    fn quit(&self) {
        self.0.quit()
    }
    fn set_initial_windowing(&self, request: WindowingRequest) {
        self.0.set_initial_windowing(request)
    }
    fn request_windowing(&self, request: WindowingRequest) -> Task<anyhow::Result<()>> {
        self.0.request_windowing(request)
    }
    fn restart(&self, binary_path: Option<PathBuf>, arguments: Vec<OsString>) {
        self.0.restart(binary_path, arguments)
    }
    fn activate(&self, ignoring_other_apps: bool) {
        self.0.activate(ignoring_other_apps)
    }
    fn set_activation_policy(&self, policy: ActivationPolicy) {
        self.0.set_activation_policy(policy)
    }
    fn hide(&self) {
        self.0.hide()
    }
    fn hide_other_apps(&self) {
        self.0.hide_other_apps()
    }
    fn unhide_other_apps(&self) {
        self.0.unhide_other_apps()
    }
    fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
        self.0.displays()
    }
    fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        self.0.primary_display()
    }
    fn on_displays_changed(&self, callback: Box<dyn FnMut()>) {
        self.0.on_displays_changed(callback)
    }
    fn active_window(&self) -> Option<AnyWindowHandle> {
        self.0.active_window()
    }
    fn window_stack(&self) -> Option<Vec<AnyWindowHandle>> {
        self.0.window_stack()
    }
    fn is_screen_capture_supported(&self) -> bool {
        self.0.is_screen_capture_supported()
    }
    fn screen_capture_sources(
        &self,
    ) -> oneshot::Receiver<anyhow::Result<Vec<Rc<dyn ScreenCaptureSource>>>> {
        self.0.screen_capture_sources()
    }
    fn window_appearance(&self) -> WindowAppearance {
        self.0.window_appearance()
    }
    fn set_window_appearance(&self, appearance: Option<WindowAppearance>) {
        self.0.set_window_appearance(appearance)
    }
    fn button_layout(&self) -> Option<WindowButtonLayout> {
        self.0.button_layout()
    }
    fn open_url(&self, url: &str) {
        self.0.open_url(url)
    }
    fn on_open_urls(&self, callback: Box<dyn FnMut(Vec<String>)>) {
        self.0.on_open_urls(callback)
    }
    fn register_url_scheme(&self, url: &str) -> Task<Result<()>> {
        self.0.register_url_scheme(url)
    }
    fn prompt_for_paths(
        &self,
        options: PathPromptOptions,
    ) -> oneshot::Receiver<Result<Option<Vec<PathBuf>>>> {
        self.0.prompt_for_paths(options)
    }
    fn prompt_for_new_path(
        &self,
        directory: &Path,
        suggested_name: Option<&str>,
    ) -> oneshot::Receiver<Result<Option<PathBuf>>> {
        self.0.prompt_for_new_path(directory, suggested_name)
    }
    fn can_select_mixed_files_and_dirs(&self) -> bool {
        self.0.can_select_mixed_files_and_dirs()
    }
    fn reveal_path(&self, path: &Path) {
        self.0.reveal_path(path)
    }
    fn open_with_system(&self, path: &Path) {
        self.0.open_with_system(path)
    }
    fn on_quit(&self, callback: Box<dyn FnMut() -> bool>) {
        self.0.on_quit(callback)
    }
    fn on_reopen(&self, callback: Box<dyn FnMut()>) {
        self.0.on_reopen(callback)
    }
    fn on_system_sleep(&self, callback: Box<dyn FnMut()>) {
        self.0.on_system_sleep(callback)
    }
    fn on_system_wake(&self, callback: Box<dyn FnMut()>) {
        self.0.on_system_wake(callback)
    }
    fn on_app_lifecycle(&self, callback: Box<dyn FnMut(AppLifecyclePhase)>) {
        self.0.on_app_lifecycle(callback)
    }
    fn on_memory_warning(&self, callback: Box<dyn FnMut()>) {
        self.0.on_memory_warning(callback)
    }
    fn gestures(&self) -> Option<Rc<dyn PlatformGestures>> {
        self.0.gestures()
    }
    fn set_menus(&self, menus: Vec<Menu>, keymap: &Keymap) {
        self.0.set_menus(menus, keymap)
    }
    fn get_menus(&self) -> Option<Vec<OwnedMenu>> {
        self.0.get_menus()
    }
    fn set_dock_menu(&self, menu: Vec<MenuItem>, keymap: &Keymap) {
        self.0.set_dock_menu(menu, keymap)
    }
    fn perform_dock_menu_action(&self, action: usize) {
        self.0.perform_dock_menu_action(action)
    }
    fn add_recent_document(&self, path: &Path) {
        self.0.add_recent_document(path)
    }
    fn update_jump_list(
        &self,
        menus: Vec<MenuItem>,
        entries: Vec<SmallVec<[PathBuf; 2]>>,
    ) -> Task<Vec<SmallVec<[PathBuf; 2]>>> {
        self.0.update_jump_list(menus, entries)
    }
    fn on_app_menu_action(&self, callback: Box<dyn FnMut(&dyn Action)>) {
        self.0.on_app_menu_action(callback)
    }
    fn on_will_open_app_menu(&self, callback: Box<dyn FnMut()>) {
        self.0.on_will_open_app_menu(callback)
    }
    fn on_validate_app_menu_command(&self, callback: Box<dyn FnMut(&dyn Action) -> bool>) {
        self.0.on_validate_app_menu_command(callback)
    }
    fn thermal_state(&self) -> ThermalState {
        self.0.thermal_state()
    }
    fn on_thermal_state_change(&self, callback: Box<dyn FnMut()>) {
        self.0.on_thermal_state_change(callback)
    }
    fn prevent_idle_sleep(&self, reason: &str) -> Task<Result<ActivityGuard>> {
        self.0.prevent_idle_sleep(reason)
    }
    fn power_mode(&self) -> PowerMode {
        self.0.power_mode()
    }
    fn on_power_mode_change(&self, callback: Box<dyn FnMut()>) {
        self.0.on_power_mode_change(callback)
    }
    fn set_app_identity(&self, identifier: &str, name: &str) {
        self.0.set_app_identity(identifier, name)
    }
    fn show_system_notification(&self, notification: SystemNotification) {
        self.0.show_system_notification(notification)
    }
    fn dismiss_system_notification(&self, tag: &str) {
        self.0.dismiss_system_notification(tag)
    }
    fn on_system_notification_response(
        &self,
        callback: Box<dyn FnMut(SystemNotificationResponse)>,
    ) {
        self.0.on_system_notification_response(callback)
    }
    fn compositor_name(&self) -> &'static str {
        self.0.compositor_name()
    }
    fn graphical_environment(&self) -> Option<GraphicalEnvironment> {
        self.0.graphical_environment()
    }
    fn app_path(&self) -> Result<PathBuf> {
        self.0.app_path()
    }
    fn path_for_auxiliary_executable(&self, name: &str) -> Result<PathBuf> {
        self.0.path_for_auxiliary_executable(name)
    }
    fn set_cursor_style(&self, style: CursorStyle) {
        LAST_CURSOR.with_borrow_mut(|c| c.clone_from(&format!("{style:?}")));
        self.0.set_cursor_style(style)
    }
    fn hide_cursor_until_mouse_moves(&self) {
        self.0.hide_cursor_until_mouse_moves()
    }
    fn is_cursor_visible(&self) -> bool {
        self.0.is_cursor_visible()
    }
    fn should_auto_hide_scrollbars(&self) -> bool {
        self.0.should_auto_hide_scrollbars()
    }
    fn read_from_clipboard(&self) -> Option<ClipboardItem> {
        self.0.read_from_clipboard()
    }
    fn write_to_clipboard(&self, item: ClipboardItem) {
        self.0.write_to_clipboard(item)
    }
    fn read_from_clipboard_async(&self) -> Task<Result<Option<ClipboardItem>, ClipboardReadError>> {
        self.0.read_from_clipboard_async()
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn read_from_primary(&self) -> Option<ClipboardItem> {
        self.0.read_from_primary()
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn write_to_primary(&self, item: ClipboardItem) {
        self.0.write_to_primary(item)
    }
    #[cfg(target_os = "macos")]
    fn read_from_find_pasteboard(&self) -> Option<ClipboardItem> {
        self.0.read_from_find_pasteboard()
    }
    #[cfg(target_os = "macos")]
    fn write_to_find_pasteboard(&self, item: ClipboardItem) {
        self.0.write_to_find_pasteboard(item)
    }
    fn write_credentials(&self, url: &str, username: &str, password: &[u8]) -> Task<Result<()>> {
        self.0.write_credentials(url, username, password)
    }
    #[cfg(target_os = "macos")]
    fn write_credentials_if_not_exists(
        &self,
        url: &str,
        username: &str,
        password: &[u8],
    ) -> Task<Result<bool>> {
        self.0.write_credentials_if_not_exists(url, username, password)
    }
    fn read_credentials(&self, url: &str) -> Task<Result<Option<(String, Vec<u8>)>>> {
        self.0.read_credentials(url)
    }
    fn delete_credentials(&self, url: &str) -> Task<Result<()>> {
        self.0.delete_credentials(url)
    }
    fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout> {
        self.0.keyboard_layout()
    }
    fn keyboard_mapper(&self) -> Rc<dyn PlatformKeyboardMapper> {
        self.0.keyboard_mapper()
    }
    fn on_keyboard_layout_change(&self, callback: Box<dyn FnMut()>) {
        self.0.on_keyboard_layout_change(callback)
    }

}

/// The window, shared with what a change of zoom reads.
struct SharedWindow(Rc<RefCell<Box<dyn PlatformWindow>>>);

struct ZoomWindow {
    inner: SharedWindow,
    resized: Resized,
}

impl HasWindowHandle for ZoomWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let w = self.inner.0.borrow();
        // SAFETY: the handle names the OS window, which lives as long as `self.inner`'s box.
        let raw = w.window_handle()?.as_raw();
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for ZoomWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let w = self.inner.0.borrow();
        let raw = w.display_handle()?.as_raw();
        // SAFETY: as above, the display outlives the window.
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}

impl PlatformWindow for ZoomWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        let b = self.inner.0.borrow().bounds();
        Bounds::new(b.origin, scaled_down(b.size))
    }

    fn window_bounds(&self) -> WindowBounds {
        self.inner.0.borrow().window_bounds()
    }

    fn content_size(&self) -> Size<Pixels> {
        scaled_down(self.inner.0.borrow().content_size())
    }

    fn resize(&mut self, s: Size<Pixels>) {
        self.inner.0.borrow_mut().resize(scaled_up(s));
    }

    fn scale_factor(&self) -> f32 {
        self.inner.0.borrow().scale_factor() * factor()
    }

    fn mouse_position(&self) -> Point<Pixels> {
        down(self.inner.0.borrow().mouse_position())
    }

    fn on_input(&self, mut callback: Box<dyn FnMut(PlatformInput) -> DispatchEventResult>) {
        self.inner.0.borrow().on_input(Box::new(move |e| callback(input_down(e))));
    }

    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        *self.resized.borrow_mut() = Some(callback);
        let resized = self.resized.clone();
        self.inner.0.borrow().on_resize(Box::new(move |s, scale| {
            if let Some(cb) = resized.borrow_mut().as_mut() {
                cb(scaled_down(s), scale * factor());
            }
        }));
    }

    fn show_window_menu(&self, position: Point<Pixels>) {
        self.inner.0.borrow().show_window_menu(up(position));
    }

    fn update_ime_position(&self, bounds: Bounds<Pixels>) {
        self.inner.0.borrow().update_ime_position(Bounds::new(up(bounds.origin), scaled_up(bounds.size)));
    }

    fn set_client_inset(&self, inset: Pixels) {
        self.inner.0.borrow().set_client_inset(inset * factor());
    }

    fn visual_viewport_bounds(&self) -> Bounds<Pixels> {
        let b = self.inner.0.borrow().visual_viewport_bounds();
        Bounds::new(down(b.origin), scaled_down(b.size))
    }

    fn draw(&self, scene: &Scene) {
        self.inner.0.borrow().draw(scene);
    }

    fn is_maximized(&self) -> bool {
        self.inner.0.borrow_mut().is_maximized()
    }
    fn on_visual_viewport_changed(&self, callback: Box<dyn FnMut()>) {
        self.inner.0.borrow_mut().on_visual_viewport_changed(callback)
    }
    fn prepare_frame(&self) -> bool {
        self.inner.0.borrow_mut().prepare_frame()
    }
    fn appearance(&self) -> WindowAppearance {
        self.inner.0.borrow_mut().appearance()
    }
    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        self.inner.0.borrow_mut().display()
    }
    fn modifiers(&self) -> Modifiers {
        self.inner.0.borrow_mut().modifiers()
    }
    fn capslock(&self) -> Capslock {
        self.inner.0.borrow_mut().capslock()
    }
    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        self.inner.0.borrow_mut().set_input_handler(input_handler)
    }
    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.inner.0.borrow_mut().take_input_handler()
    }
    fn set_text_input_configuration(&mut self, configuration: TextInputConfiguration) {
        self.inner.0.borrow_mut().set_text_input_configuration(configuration)
    }
    fn prompt(
        &self,
        level: PromptLevel,
        msg: &str,
        detail: Option<&str>,
        answers: &[PromptButton],
    ) -> Option<oneshot::Receiver<usize>> {
        self.inner.0.borrow_mut().prompt(level, msg, detail, answers)
    }
    fn activate(&self) {
        self.inner.0.borrow_mut().activate()
    }
    fn request_attention(&self) {
        self.inner.0.borrow_mut().request_attention()
    }
    fn is_active(&self) -> bool {
        self.inner.0.borrow_mut().is_active()
    }
    fn visibility(&self) -> WindowVisibility {
        self.inner.0.borrow_mut().visibility()
    }
    fn is_hovered(&self) -> bool {
        self.inner.0.borrow_mut().is_hovered()
    }
    fn background_appearance(&self) -> WindowBackgroundAppearance {
        self.inner.0.borrow_mut().background_appearance()
    }
    fn set_title(&mut self, title: &str) {
        self.inner.0.borrow_mut().set_title(title)
    }
    fn set_background_appearance(&self, background_appearance: WindowBackgroundAppearance) {
        self.inner.0.borrow_mut().set_background_appearance(background_appearance)
    }
    fn minimize(&self) {
        self.inner.0.borrow_mut().minimize()
    }
    fn zoom(&self) {
        self.inner.0.borrow_mut().zoom()
    }
    fn toggle_fullscreen(&self) {
        self.inner.0.borrow_mut().toggle_fullscreen()
    }
    fn is_fullscreen(&self) -> bool {
        self.inner.0.borrow_mut().is_fullscreen()
    }
    fn frame_waker(&self) -> Option<Rc<dyn Fn()>> {
        self.inner.0.borrow_mut().frame_waker()
    }
    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        self.inner.0.borrow_mut().on_request_frame(callback)
    }
    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.inner.0.borrow_mut().on_active_status_change(callback)
    }
    fn on_visibility_change(&self, callback: Box<dyn FnMut(WindowVisibility)>) {
        self.inner.0.borrow_mut().on_visibility_change(callback)
    }
    fn on_hover_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.inner.0.borrow_mut().on_hover_status_change(callback)
    }
    fn on_moved(&self, callback: Box<dyn FnMut()>) {
        self.inner.0.borrow_mut().on_moved(callback)
    }
    fn on_display_changed(&self, callback: Box<dyn FnMut()>) {
        self.inner.0.borrow_mut().on_display_changed(callback)
    }
    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>) {
        self.inner.0.borrow_mut().on_should_close(callback)
    }
    fn on_hit_test_window_control(&self, callback: Box<dyn FnMut() -> Option<WindowControlArea>>) {
        self.inner.0.borrow_mut().on_hit_test_window_control(callback)
    }
    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        self.inner.0.borrow_mut().on_close(callback)
    }
    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        self.inner.0.borrow_mut().on_appearance_changed(callback)
    }
    fn on_button_layout_changed(&self, callback: Box<dyn FnMut()>) {
        self.inner.0.borrow_mut().on_button_layout_changed(callback)
    }
    fn schedule_frame(&self) {
        self.inner.0.borrow_mut().schedule_frame()
    }
    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.inner.0.borrow_mut().sprite_atlas()
    }
    fn is_subpixel_rendering_supported(&self) -> bool {
        self.inner.0.borrow_mut().is_subpixel_rendering_supported()
    }
    fn get_title(&self) -> String {
        self.inner.0.borrow_mut().get_title()
    }
    fn tabbed_windows(&self) -> Option<Vec<SystemWindowTab>> {
        self.inner.0.borrow_mut().tabbed_windows()
    }
    fn tab_bar_visible(&self) -> bool {
        self.inner.0.borrow_mut().tab_bar_visible()
    }
    fn set_edited(&mut self, edited: bool) {
        self.inner.0.borrow_mut().set_edited(edited)
    }
    fn set_document_path(&self, path: Option<&std::path::Path>) {
        self.inner.0.borrow_mut().set_document_path(path)
    }
    fn toggle_simple_fullscreen(&self) {
        self.inner.0.borrow_mut().toggle_simple_fullscreen()
    }
    fn is_simple_fullscreen(&self) -> bool {
        self.inner.0.borrow_mut().is_simple_fullscreen()
    }
    #[cfg(target_os = "macos")]
    fn set_traffic_light_position(&self, position: Point<Pixels>) {
        self.inner.0.borrow_mut().set_traffic_light_position(position)
    }
    fn show_character_palette(&self) {
        self.inner.0.borrow_mut().show_character_palette()
    }
    fn titlebar_double_click(&self, is_resizable: bool, is_minimizable: bool) {
        self.inner.0.borrow_mut().titlebar_double_click(is_resizable, is_minimizable)
    }
    fn on_move_tab_to_new_window(&self, callback: Box<dyn FnMut()>) {
        self.inner.0.borrow_mut().on_move_tab_to_new_window(callback)
    }
    fn on_merge_all_windows(&self, callback: Box<dyn FnMut()>) {
        self.inner.0.borrow_mut().on_merge_all_windows(callback)
    }
    fn on_select_previous_tab(&self, callback: Box<dyn FnMut()>) {
        self.inner.0.borrow_mut().on_select_previous_tab(callback)
    }
    fn on_select_next_tab(&self, callback: Box<dyn FnMut()>) {
        self.inner.0.borrow_mut().on_select_next_tab(callback)
    }
    fn on_toggle_tab_bar(&self, callback: Box<dyn FnMut()>) {
        self.inner.0.borrow_mut().on_toggle_tab_bar(callback)
    }
    fn merge_all_windows(&self) {
        self.inner.0.borrow_mut().merge_all_windows()
    }
    fn move_tab_to_new_window(&self) {
        self.inner.0.borrow_mut().move_tab_to_new_window()
    }
    fn toggle_window_tab_overview(&self) {
        self.inner.0.borrow_mut().toggle_window_tab_overview()
    }
    fn set_tabbing_identifier(&self, identifier: Option<String>) {
        self.inner.0.borrow_mut().set_tabbing_identifier(identifier)
    }
    fn native_window_state(&self) -> Option<Vec<u8>> {
        self.inner.0.borrow_mut().native_window_state()
    }
    fn restore_native_window_state(&self, state: &[u8]) {
        self.inner.0.borrow_mut().restore_native_window_state(state)
    }
    #[cfg(target_os = "windows")]
    fn get_raw_handle(&self) -> windows::Win32::Foundation::HWND {
        self.inner.0.borrow_mut().get_raw_handle()
    }
    fn inner_window_bounds(&self) -> WindowBounds {
        self.inner.0.borrow_mut().inner_window_bounds()
    }
    fn request_decorations(&self, decorations: WindowDecorations) {
        self.inner.0.borrow_mut().request_decorations(decorations)
    }
    fn start_window_move(&self) {
        self.inner.0.borrow_mut().start_window_move()
    }
    fn can_start_external_drag(&self) -> bool {
        self.inner.0.borrow_mut().can_start_external_drag()
    }
    fn start_external_drag(&self, payload: &ExternalDragPayload) -> bool {
        self.inner.0.borrow_mut().start_external_drag(payload)
    }
    fn start_window_resize(&self, edge: ResizeEdge) {
        self.inner.0.borrow_mut().start_window_resize(edge)
    }
    fn set_exclusive_zone(&self, zone: Pixels) {
        self.inner.0.borrow_mut().set_exclusive_zone(zone)
    }
    fn set_input_region(&self, region: Option<&[Bounds<Pixels>]>) {
        self.inner.0.borrow_mut().set_input_region(region)
    }
    fn window_decorations(&self) -> Decorations {
        self.inner.0.borrow_mut().window_decorations()
    }
    fn set_app_id(&mut self, app_id: &str) {
        self.inner.0.borrow_mut().set_app_id(app_id)
    }
    fn map_window(&mut self) -> anyhow::Result<()> {
        self.inner.0.borrow_mut().map_window()
    }
    fn window_controls(&self) -> WindowControls {
        self.inner.0.borrow_mut().window_controls()
    }
    fn gpu_specs(&self) -> Option<GpuSpecs> {
        self.inner.0.borrow_mut().gpu_specs()
    }
    fn insets(&self) -> WindowInsets {
        self.inner.0.borrow_mut().insets()
    }
    fn on_insets_changed(&self, callback: Box<dyn FnMut(WindowInsets)>) {
        self.inner.0.borrow_mut().on_insets_changed(callback)
    }
    fn set_back_handler(&self, callback: Box<dyn FnMut()>) {
        self.inner.0.borrow_mut().set_back_handler(callback)
    }
    fn set_back_enabled(&self, enabled: bool) {
        self.inner.0.borrow_mut().set_back_enabled(enabled)
    }
    fn show_soft_keyboard(&self) {
        self.inner.0.borrow_mut().show_soft_keyboard()
    }
    fn hide_soft_keyboard(&self) {
        self.inner.0.borrow_mut().hide_soft_keyboard()
    }
    fn text_input_state_changed(&self, change: TextInputStateChange) {
        self.inner.0.borrow_mut().text_input_state_changed(change)
    }
    fn play_system_bell(&self) {
        self.inner.0.borrow_mut().play_system_bell()
    }
    fn a11y_init(&self, callbacks: A11yCallbacks) {
        self.inner.0.borrow_mut().a11y_init(callbacks)
    }
    fn a11y_tree_update(&self, tree_update: accesskit::TreeUpdate) {
        self.inner.0.borrow_mut().a11y_tree_update(tree_update)
    }
    fn a11y_update_window_bounds(&self) {
        self.inner.0.borrow_mut().a11y_update_window_bounds()
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_step_goes_to_the_next_and_stops_at_the_ends() {
        assert_eq!(next_zoom(100, 1), 110);
        assert_eq!(next_zoom(100, -1), 90);
        assert_eq!(next_zoom(105, 1), 110, "between steps, the nearest that way");
        assert_eq!(next_zoom(300, 1), 300);
        assert_eq!(next_zoom(80, -1), 80);
        assert_eq!(next_zoom(175, 0), 100);
        assert_eq!(parse_zoom(Some(150)), Some(150));
        assert_eq!(parse_zoom(Some(151)), None);
    }
}
