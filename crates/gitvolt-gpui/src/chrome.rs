//! The window's own chrome: `ui/src/chrome/` (`TitleBar.tsx`, `dragRegion.ts`, `windowChrome.ts`) and the app's `chrome.rs`. Settings ›
//! Appearance › Title bar is read when the window is built, so what's drawn is [`WindowChrome`], and a change waits for a restart
//! ([`GitVolt::restart_app`]). On top of `GitVolt::settings`' two switches: System wins over Combined, and an X11 session has System.

use crate::app::{GitVolt, TITLE_H, icon};
use crate::theme;
use crate::ui::popover::{self, Choice, ChoiceRequest};
use gitvolt_core::api::Request;
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};

/// `dragRegion.ts`'s `DRAG_THRESHOLD`: a click stays a click, and a double click's second press doesn't start a move.
pub const DRAG_THRESHOLD: f32 = 4.;

/// What the window was built with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowChrome {
    /// GitVolt draws the title bar, not the system.
    pub custom: bool,
    /// Combined: no title bar row, the tab bar is the title bar. Only with `custom`.
    pub compact: bool,
    /// The system's frame whatever the setting says: an X11 session.
    pub system_only: bool,
}

impl Default for WindowChrome {
    fn default() -> Self {
        Self { custom: true, compact: false, system_only: false }
    }
}

impl WindowChrome {
    /// `custom_frame_on`, `with_compact` and `system_only` in the app's `chrome.rs`. On Linux, a Wayland session is `WAYLAND_DISPLAY`, or
    /// what the session says it is: this build may run through Xwayland inside one, where `WAYLAND_DISPLAY` is off so that GPUI takes X11.
    pub fn at_start(system_title_bar: bool, compact_title_bar: bool, wayland_display: Option<&str>, session_type: Option<&str>) -> Self {
        let own = !cfg!(target_os = "linux") || wayland_display.is_some_and(|d| !d.is_empty()) || session_type == Some("wayland");
        let custom = own && !system_title_bar;
        Self { custom, compact: custom && compact_title_bar, system_only: !own }
    }

    /// `runningChoice`: the choice the window was built with.
    pub fn choice(self) -> &'static str {
        if !self.custom {
            "system"
        } else if self.compact {
            "combined"
        } else {
            "separate"
        }
    }

    /// The title bar row's height.
    pub fn title_h(self) -> f32 {
        if self.custom && !self.compact { TITLE_H } else { 0. }
    }

    /// `.tab-bar-btn`'s width: a little narrower where the tab bar is the title bar.
    pub fn bar_button_w(self) -> f32 {
        if self.compact { 28. } else { 34. }
    }

    /// `.tab`'s minimum width, its padding and border on top of the CSS's.
    pub fn tab_min_w(self) -> f32 {
        if self.compact { 91. } else { 109. }
    }

    /// The space before ☰: gone in full screen and maximized, since nothing's on the left here and the screen's top edge moves the window then.
    pub fn lead_space(self, maximized: bool, fullscreen: bool) -> bool {
        self.compact && !fullscreen && !maximized
    }
}

/// `titleBarChoice`: the choice the two stored switches make, System winning.
pub fn stored_choice(settings: &Value) -> &'static str {
    if settings["systemTitleBar"] == true {
        "system"
    } else if settings["compactTitleBar"] == true {
        "combined"
    } else {
        "separate"
    }
}

/// `TitleBarControl`'s `restartShown`: the choice isn't what the window was built with.
pub fn restart_shown(chrome: WindowChrome, settings: &Value) -> bool {
    !chrome.system_only && stored_choice(settings) != chrome.choice()
}

/// A press that may become a window move, on the title bar's empty space or a button that also moves the window (`beginWindowDrag`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowPress {
    pub at: (f32, f32),
    /// Still down, and not moved far enough yet.
    pub waiting: bool,
    /// It moved the window: its click doesn't run. Cleared by the next press: the window manager may keep this one's release.
    pub dragged: bool,
}

impl WindowPress {
    pub fn new(at: Point<Pixels>) -> Self {
        Self { at: (at.x.as_f32(), at.y.as_f32()), waiting: true, dragged: false }
    }

    /// The pointer moved with the button down: whether the window moves now.
    pub fn moved(&mut self, to: Point<Pixels>) -> bool {
        if !self.waiting || (to.x.as_f32() - self.at.0).hypot(to.y.as_f32() - self.at.1) < DRAG_THRESHOLD {
            return false;
        }
        self.waiting = false;
        self.dragged = true;
        true
    }
}

impl GitVolt {
    /// A left press on what can move the window, under GitVolt's own title bar only.
    pub fn window_press(&mut self, e: &MouseDownEvent) {
        self.window_press = self.chrome.custom.then(|| WindowPress::new(e.position));
    }

    /// `guardClick`: whether this click is the end of a window move, or a click to run.
    pub fn press_moved_window(&mut self) -> bool {
        self.window_press.take().is_some_and(|p| p.dragged)
    }

    /// Any move of the pointer, wherever it is: the press's button may be up by now, and it may have left what it pressed.
    pub fn window_press_moved(&mut self, e: &MouseMoveEvent, window: &mut Window) {
        let Some(press) = self.window_press.as_mut() else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            press.waiting = false;
        } else if press.moved(e.position) {
            window.start_window_move();
        }
    }

    /// Unsaved edits or an unsaved merge, in any tab (`anyDirty`).
    pub fn any_unsaved(&self) -> bool {
        self.tabs.iter().any(|t| t.diff.as_ref().is_some_and(|d| d.editing_dirty()) || t.merge_dirty())
    }

    /// `confirmDiscardUnsaved`: before GitVolt goes, unsaved edits ask, under `anchor` if it was asked from a control; `go` runs if nothing's
    /// unsaved or they're to be discarded. Nothing's taken back here: what's left unsaved goes with the process.
    pub fn confirm_discard_unsaved(&mut self, anchor: Option<&'static str>, go: impl FnOnce(&mut Self, &mut Context<Self>) + 'static, cx: &mut Context<Self>) {
        if !self.any_unsaved() {
            return go(self, cx);
        }
        let this = cx.weak_entity();
        let req = ChoiceRequest { title: "Discard unsaved changes?".into(), body: None, note: None, choices: vec![Choice { id: "discard".into(), label: "Discard edits".into(), danger: true, ..Default::default() }] };
        self.with_window(cx, move |window, cx| {
            let bounds = anchor.and_then(|k| crate::ui::Ui::get(cx).anchor(k));
            popover::ask_choice(
                req,
                bounds,
                move |pick, _, cx| {
                    if pick.as_deref() == Some("discard") {
                        this.update(cx, |v, cx| go(v, cx)).ok();
                    }
                },
                window,
                cx,
            );
        });
    }

    /// Restart GitVolt (Settings › Appearance › Title bar, the update dialog): unsaved edits ask first, the settings, tabs and drafts are saved
    /// before this process goes. The core starts the new process once this one has gone; `req` says which of its two ways.
    pub fn restart_app(&mut self, req: fn() -> Request, anchor: &'static str, cx: &mut Context<Self>) {
        self.confirm_discard_unsaved(Some(anchor), move |v, cx| v.restart_now(req, cx), cx);
    }

    fn restart_now(&mut self, req: fn() -> Request, cx: &mut Context<Self>) {
        self.flush_drafts();
        self.save_layout();
        self.save_tabs(cx);
        let (settings, profile) = (crate::write::call("saveSettings", json!({ "settings": self.settings })), crate::write::call("saveProfile", json!({ "profile": self.profile })));
        if cfg!(test) {
            let restart = json!({ "method": if matches!(req(), Request::UpdateRestart) { "updateRestart" } else { "relaunchApp" } });
            self.sent.borrow_mut().extend([settings, profile, restart]);
            return;
        }
        let api = self.api.clone();
        let job = self.rt.spawn(async move {
            // In order, so what the next process reads is here before this one goes.
            for body in [settings, profile] {
                api.dispatch(crate::write::request_of(body)?).await?;
            }
            api.dispatch(req()).await
        });
        cx.spawn(async move |_, cx| {
            let answer = job.await;
            cx.update(|cx| match answer {
                // The core's started what waits for this process to end.
                Ok(Ok(_)) => cx.quit(),
                Ok(Err(e)) => crate::ui::toast::show(e.message, cx),
                Err(e) => crate::ui::toast::show(e.to_string(), cx),
            })
        })
        .detach();
    }
}

/// `titleBarMouse`, on an empty space: dragged, it moves the window; a double click does what a title bar's does, a right click shows the window menu.
pub fn drag_space(el: Stateful<Div>, cx: &mut Context<GitVolt>) -> Stateful<Div> {
    el.on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, window, _| {
        if e.click_count == 2 {
            this.window_press = None;
            window.zoom_window();
        } else {
            this.window_press(e);
        }
    }))
    .on_mouse_down(MouseButton::Right, |e, window, _| window.show_window_menu(e.position))
}

/// `useDragOrClick`: a tab bar's or a window's button that also moves the window: a press that moves first moves it, a press and release in place runs `run`.
pub fn drag_or_click(el: Stateful<Div>, run: impl Fn(&mut GitVolt, &mut Window, &mut Context<GitVolt>) + 'static, cx: &mut Context<GitVolt>) -> Stateful<Div> {
    el.on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| {
        cx.stop_propagation();
        this.window_press(e);
    }))
    .on_click(cx.listener(move |this, _, window, cx| {
        if !this.press_moved_window() {
            run(this, window, cx);
        }
    }))
}

fn wm_button(id: &'static str, name: &'static str) -> Stateful<Div> {
    div().id(id).role(Role::Button).aria_label(id).group(id).flex().items_center().justify_center().size(px(24.)).rounded_full().bg(theme::control_bg()).hover(|d| d.bg(theme::control_hover_bg())).child(icon(name, 10., theme::text_normal()).group_hover(id, |d| d.text_color(theme::text_selected())))
}

/// `WindowButtons`: minimise, maximise and close, in the title bar or the tab bar.
pub fn window_buttons(window: &Window, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .px(px(6.))
        .child(crate::ui::anchor("win-minimize", drag_or_click(wm_button("minimize", "wm-minimize"), |_, window, _| window.minimize_window(), cx)))
        .child(crate::ui::anchor("win-maximize", drag_or_click(wm_button("maximize", if window.is_maximized() { "wm-restore" } else { "wm-maximize" }), |_, window, _| window.zoom_window(), cx)))
        .child(crate::ui::anchor("win-close", drag_or_click(wm_button("close", "wm-close"), |this, _, cx| this.close_window(cx), cx)))
}

/// `TitleBar`'s row, under Separate.
pub fn title_bar(this: &GitVolt, window: &Window, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let name = this.tabs.get(this.active).map(|t| format!("GitVolt — {}", t.name)).unwrap_or_else(|| "GitVolt".into());
    drag_space(div().id("title-bar"), cx)
        .flex()
        .flex_none()
        .h(px(TITLE_H))
        .bg(theme::tab_bar_bg())
        .text_color(theme::text_secondary())
        .child(crate::app::hamburger(34., cx))
        .child(div().flex().flex_1().min_w_0().items_center().justify_center().text_size(px(12.)).child(name))
        .child(window_buttons(window, cx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn on_wayland(system: bool, compact: bool) -> WindowChrome {
        WindowChrome::at_start(system, compact, Some("wayland-1"), None)
    }

    #[test]
    fn the_two_switches_are_one_choice_system_winning() {
        assert_eq!(stored_choice(&json!({ "systemTitleBar": false, "compactTitleBar": false })), "separate");
        assert_eq!(stored_choice(&json!({ "systemTitleBar": false, "compactTitleBar": true })), "combined");
        assert_eq!(stored_choice(&json!({ "systemTitleBar": true, "compactTitleBar": false })), "system");
        assert_eq!(stored_choice(&json!({ "systemTitleBar": true, "compactTitleBar": true })), "system");
        assert_eq!(on_wayland(false, false).choice(), "separate");
        assert_eq!(on_wayland(false, true).choice(), "combined");
        assert_eq!(on_wayland(true, true).choice(), "system");
        assert!(!on_wayland(true, true).compact, "there's no row of the system's to fold away");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn an_x11_session_has_the_system_title_bar_whatever_the_setting() {
        for session in [None, Some("x11"), Some("tty")] {
            let c = WindowChrome::at_start(false, true, None, session);
            assert_eq!((c.custom, c.compact, c.system_only), (false, false, true), "{session:?}");
            assert_eq!(WindowChrome::at_start(false, true, Some(""), session), c, "an empty WAYLAND_DISPLAY isn't one");
        }
        // This build through Xwayland in a Wayland session: GPUI wants WAYLAND_DISPLAY off to take X11.
        let c = WindowChrome::at_start(false, true, None, Some("wayland"));
        assert_eq!((c.custom, c.compact, c.system_only), (true, true, false));
        assert!(!WindowChrome::at_start(true, false, None, Some("wayland")).custom);
    }

    #[test]
    fn restart_is_offered_once_the_choice_isnt_what_the_window_was_built_with() {
        let separate = on_wayland(false, false);
        assert!(!restart_shown(separate, &json!({})));
        assert!(restart_shown(separate, &json!({ "compactTitleBar": true })));
        assert!(restart_shown(separate, &json!({ "systemTitleBar": true })));
        let combined = on_wayland(false, true);
        assert!(!restart_shown(combined, &json!({ "compactTitleBar": true })));
        assert!(restart_shown(combined, &json!({})), "back to Separate needs one too");
        let x11 = WindowChrome { custom: false, compact: false, system_only: true };
        assert!(!restart_shown(x11, &json!({ "compactTitleBar": true })), "nothing can change there");
    }

    #[test]
    fn combined_has_no_title_bar_row_and_a_tighter_tab_bar() {
        let (separate, combined, system) = (on_wayland(false, false), on_wayland(false, true), on_wayland(true, false));
        assert_eq!((separate.title_h(), combined.title_h(), system.title_h()), (30., 0., 0.));
        assert_eq!((separate.bar_button_w(), combined.bar_button_w(), system.bar_button_w()), (34., 28., 34.));
        assert_eq!((separate.tab_min_w(), combined.tab_min_w()), (109., 91.));
        assert!(combined.lead_space(false, false));
        assert!(!combined.lead_space(true, false), "maximized: the screen's top edge moves it");
        assert!(!combined.lead_space(false, true));
        assert!(!separate.lead_space(false, false));
    }

    #[gpui::test]
    fn restart_saves_the_settings_and_tabs_before_it_goes(cx: &mut TestAppContext) {
        let (handle, mut vcx, _rt) = crate::app::tests::with_rows(cx, 3);
        handle
            .update(&mut vcx, |v, _, cx| {
                v.settings = json!({ "compactTitleBar": true });
                v.profile = json!({ "id": "default", "tabs": [] });
                v.restart_app(|| Request::RelaunchApp, "setting:titleBarRestart", cx);
                let sent = v.sent.borrow();
                let methods: Vec<&str> = sent.iter().filter_map(|b| b["method"].as_str()).collect();
                assert_eq!(methods.last(), Some(&"relaunchApp"), "{methods:?}");
                let settings = methods.iter().rposition(|m| *m == "saveSettings").unwrap();
                let profile = methods.iter().rposition(|m| *m == "saveProfile").unwrap();
                assert!(settings < methods.len() - 1 && profile < methods.len() - 1);
                assert!(!crate::ui::Ui::get(cx).popover.is_some(), "nothing unsaved: no question");
            })
            .unwrap();
    }

    #[test]
    fn a_press_moves_the_window_only_past_the_threshold_and_only_once() {
        let mut p = WindowPress::new(point(px(100.), px(10.)));
        assert!(!p.moved(point(px(102.), px(11.))));
        assert!(!p.dragged);
        assert!(p.moved(point(px(104.), px(10.))));
        assert!(p.dragged);
        assert!(!p.moved(point(px(200.), px(10.))), "the window manager has it now");
        assert!(p.dragged);
    }
}
