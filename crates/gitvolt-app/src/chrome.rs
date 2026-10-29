//! GitVolt's own title bar, on Linux, Windows and macOS. On Linux and Windows the window is built
//! without the window manager's decorations, and the UI draws a title bar (`ui/src/chrome/`)
//! whose drags, resize edges, window menu and buttons ask the window manager through the CEF
//! runtime (vendor/tauri-runtime-cef/GITVOLT-PATCH.md, "A window that draws its own title bar").
//! On macOS the window keeps its frame, with an overlay title bar (transparent, the title hidden,
//! the page under it), and Apple's traffic lights sit at the start of the UI's title bar, inset to
//! its middle ([`TRAFFIC_LIGHTS`]). Settings › Appearance › "Use the system title bar" keeps the
//! system's; it's read when the window is built, so a change applies after a restart.
//!
//! On Windows the undecorated window keeps the system's resize border, shadow and edge at its
//! left, right and bottom (the runtime's `platform/windows/chrome.rs`), so only the top edge is
//! the page's.
//!
//! On Linux, only in a Wayland session (GitVolt itself runs under Xwayland there): the compositor moves
//! and resizes the window with a grab of its own. In an X11 session the window manager needs
//! the X pointer grab, which Chromium's own connection holds while the button is down, and
//! only Chromium could release it; a move or resize would never start, so the window keeps the
//! system's frame there.
//!
//! The UI learns which frame it has, and the window buttons and title bar clicks to follow, from
//! `window.__GITVOLT_WINDOW__`, set by an initialization script before its first render (no
//! title bar popping in), and from [`CHROME_EVENT`] when GNOME's settings, or macOS's double
//! click, change.

use serde::Serialize;

/// The event that carries a new [`WindowChrome`] when the desktop's settings change.
pub const CHROME_EVENT: &str = "gb:window-chrome";

/// What the UI needs to draw the title bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowChrome {
    /// `custom`: the UI draws the title bar (no decorations on Linux, an overlay title bar on
    /// macOS); `system`: the window manager's.
    pub frame: &'static str,
    /// `linux`, `macos` or `windows`: which button set the title bar draws.
    pub platform: &'static str,
    /// The window buttons in GNOME's `button-layout` form (`appmenu:close`).
    pub layout: String,
    /// What a double, middle and right click on the title bar do, in GNOME's
    /// `action-*-click-titlebar` terms (`toggle-maximize`, `lower`, `menu`, `none`, …).
    pub double_click: String,
    pub middle_click: String,
    pub right_click: String,
}

/// The window buttons where the desktop doesn't say: minimise, maximise and close on the right.
pub const DEFAULT_LAYOUT: &str = ":minimize,maximize,close";

const PLATFORM: &str = if cfg!(target_os = "macos") {
    "macos"
} else if cfg!(windows) {
    "windows"
} else {
    "linux"
};

/// Whether GitVolt draws its own title bar here: see [`custom_frame_on`].
pub fn custom_frame(system_title_bar: bool, wayland_display: Option<&str>) -> bool {
    custom_frame_on(PLATFORM, system_title_bar, wayland_display)
}

/// Whether GitVolt draws its own title bar on `platform`: on Windows and macOS, and on Linux in a
/// Wayland session (`wayland_display`, the session's `WAYLAND_DISPLAY`), unless the user asked
/// for the system's.
pub fn custom_frame_on(platform: &str, system_title_bar: bool, wayland_display: Option<&str>) -> bool {
    !system_title_bar
        && match platform {
            "macos" | "windows" => true,
            "linux" => wayland_display.is_some_and(|d| !d.is_empty()),
            _ => false,
        }
}

/// Where macOS's traffic lights go, in logical px from the window's top-left corner (Tauri's
/// `trafficLightPosition`: the close button's left edge, and the height AppKit's title bar view
/// is stretched to under the buttons, which keep their distance from its bottom): vertically
/// centred in the 30 px title bar. `ui/src/chrome/chrome.css`'s `.title-bar-lights` keeps their
/// width free.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const TRAFFIC_LIGHTS: (f64, f64) = (12.0, 13.0);

/// What a double click on a macOS title bar does, from System Settings › Desktop & Dock ›
/// "Double-click a window's title bar to": `AppleActionOnDoubleClick` (`Maximize` is Zoom,
/// `Fill`, `Minimize`, `None`), or older systems' `AppleMiniaturizeOnDoubleClick`; Zoom when
/// neither is set. Fill zooms too: either way the window grows to the screen's free space.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn mac_double_click(action: Option<&str>, miniaturize: Option<bool>) -> &'static str {
    match action {
        Some("Minimize") => "minimize",
        Some("None") => "none",
        Some(_) => "toggle-maximize",
        None if miniaturize == Some(true) => "minimize",
        None => "toggle-maximize",
    }
}

/// The title bar on macOS: Apple's traffic lights at its start (no layout of its own), a double
/// click doing what System Settings says, and nothing on a middle or right click, as on the
/// system's own title bars.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn macos_chrome(custom: bool, double_click: &str) -> WindowChrome {
    WindowChrome {
        frame: if custom { "custom" } else { "system" },
        platform: "macos",
        layout: String::new(),
        double_click: double_click.to_owned(),
        middle_click: "none".to_owned(),
        right_click: "none".to_owned(),
    }
}

/// Whether `XDG_CURRENT_DESKTOP` (a `:`-separated list) names GNOME, whose window manager
/// settings the title bar follows. Elsewhere `org.gnome.desktop.wm.preferences` may be
/// installed but isn't what the window manager reads.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn is_gnome(current_desktop: Option<&str>) -> bool {
    current_desktop.is_some_and(|d| d.split(':').any(|name| name.trim().eq_ignore_ascii_case("gnome")))
}

/// The keys of `org.gnome.desktop.wm.preferences` the title bar follows.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const GNOME_KEYS: [&str; 4] = ["button-layout", "action-double-click-titlebar", "action-middle-click-titlebar", "action-right-click-titlebar"];

/// Reads one of [`GNOME_KEYS`] (`None`: unset or unreadable).
pub type SettingReader<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The title bar for this frame, from GNOME's settings (`gnome`, by key; `None` off GNOME) or
/// the defaults: the window buttons on the right, a double click toggling maximise, a middle
/// click lowering the window (nothing on Windows, as there) and a right click showing the window
/// menu.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn chrome(custom: bool, gnome: Option<SettingReader<'_>>) -> WindowChrome {
    chrome_on(PLATFORM, custom, gnome)
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn chrome_on(platform: &'static str, custom: bool, gnome: Option<SettingReader<'_>>) -> WindowChrome {
    let read = |key: &str, default: &str| gnome.and_then(|get| get(key)).unwrap_or_else(|| default.to_owned());
    WindowChrome {
        frame: if custom { "custom" } else { "system" },
        platform,
        layout: read("button-layout", DEFAULT_LAYOUT),
        double_click: read("action-double-click-titlebar", "toggle-maximize"),
        middle_click: read("action-middle-click-titlebar", if platform == "windows" { "none" } else { "lower" }),
        right_click: read("action-right-click-titlebar", "menu"),
    }
}

/// The initialization script that hands `chrome` to the page before its own scripts run.
pub fn init_script(chrome: &WindowChrome) -> String {
    // JSON is a JavaScript expression; `<` is escaped so no value can close a script element.
    let json = serde_json::to_string(chrome).unwrap_or_else(|_| "null".into()).replace('<', "\\u003c");
    format!("window.__GITVOLT_WINDOW__ = {json};")
}

/// macOS's title bar settings.
#[cfg(target_os = "macos")]
pub mod macos {
    use objc2_foundation::{NSUserDefaults, ns_string};

    /// What a title bar double click does ([`super::mac_double_click`]), as System Settings has
    /// it now: a read of the user defaults, cheap enough to repeat whenever the window comes to
    /// the front, so a change there applies without a restart.
    pub fn double_click() -> &'static str {
        let defaults = NSUserDefaults::standardUserDefaults();
        let action = defaults.stringForKey(ns_string!("AppleActionOnDoubleClick")).map(|s| s.to_string());
        let legacy = ns_string!("AppleMiniaturizeOnDoubleClick");
        let miniaturize = defaults.objectForKey(legacy).map(|_| defaults.boolForKey(legacy));
        super::mac_double_click(action.as_deref(), miniaturize)
    }
}

/// GNOME's window manager settings, watched for changes. On the GTK main thread.
#[cfg(target_os = "linux")]
pub mod gnome {
    use gio::prelude::*;

    const SCHEMA: &str = "org.gnome.desktop.wm.preferences";

    /// GNOME's `org.gnome.desktop.wm.preferences`, if this is GNOME and the schema has every
    /// key the title bar reads. Looked up first: `Settings::new` aborts on a missing schema.
    pub fn settings() -> Option<gio::Settings> {
        if !super::is_gnome(std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref()) {
            return None;
        }
        let schema = gio::SettingsSchemaSource::default()?.lookup(SCHEMA, true)?;
        if !super::GNOME_KEYS.iter().all(|key| schema.has_key(key)) {
            return None;
        }
        Some(gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None))
    }

    /// One key's value as a string.
    pub fn read(settings: &gio::Settings, key: &str) -> Option<String> {
        Some(settings.string(key).to_string())
    }

    /// Calls `changed` whenever one of the keys the title bar reads changes.
    pub fn watch(settings: &gio::Settings, changed: impl Fn(&gio::Settings) + 'static) -> glib::SignalHandlerId {
        settings.connect_changed(None, move |settings, key| {
            if super::GNOME_KEYS.contains(&key) {
                changed(settings);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_linux_only_a_wayland_session_draws_its_own_title_bar_unless_asked_not_to() {
        assert!(custom_frame_on("linux", false, Some("wayland-0")));
        assert!(!custom_frame_on("linux", true, Some("wayland-0")));
        // An X11 session: the window manager couldn't take the pointer from Chromium.
        assert!(!custom_frame_on("linux", false, None));
        assert!(!custom_frame_on("linux", false, Some("")));
        assert!(custom_frame(false, Some("wayland-0")));
    }

    /// Windows and macOS always have their own unless asked not to: no session to depend on.
    #[test]
    fn windows_and_macos_draw_their_own_title_bar_unless_asked_not_to() {
        for platform in ["windows", "macos"] {
            assert!(custom_frame_on(platform, false, None), "{platform}");
            assert!(!custom_frame_on(platform, true, None), "{platform}");
        }
    }

    /// Windows' title bar: its buttons on the right, and a middle click does nothing there.
    #[test]
    fn the_windows_title_bar_has_its_own_defaults() {
        let c = chrome_on("windows", true, None);
        assert_eq!((c.platform, c.layout.as_str()), ("windows", DEFAULT_LAYOUT));
        assert_eq!((c.double_click.as_str(), c.middle_click.as_str(), c.right_click.as_str()), ("toggle-maximize", "none", "menu"));
    }

    /// System Settings' choice, the older switch when it's the only one set, and Zoom otherwise.
    #[test]
    fn a_mac_double_click_follows_system_settings() {
        assert_eq!(mac_double_click(Some("Maximize"), None), "toggle-maximize");
        assert_eq!(mac_double_click(Some("Fill"), None), "toggle-maximize");
        assert_eq!(mac_double_click(Some("Minimize"), Some(false)), "minimize");
        assert_eq!(mac_double_click(Some("None"), Some(true)), "none");
        assert_eq!(mac_double_click(None, Some(true)), "minimize");
        assert_eq!(mac_double_click(None, Some(false)), "toggle-maximize");
        assert_eq!(mac_double_click(None, None), "toggle-maximize");
    }

    /// The traffic lights instead of a button layout, and no middle or right click action.
    #[test]
    fn the_mac_title_bar_has_no_buttons_of_its_own() {
        let c = macos_chrome(true, "minimize");
        assert_eq!((c.frame, c.platform, c.layout.as_str()), ("custom", "macos", ""));
        assert_eq!((c.double_click.as_str(), c.middle_click.as_str(), c.right_click.as_str()), ("minimize", "none", "none"));
        assert_eq!(macos_chrome(false, "toggle-maximize").frame, "system");
    }

    #[test]
    fn gnome_is_any_entry_of_the_desktop_list() {
        for desktop in ["GNOME", "ubuntu:GNOME", "GNOME-Classic:GNOME", "gnome"] {
            assert!(is_gnome(Some(desktop)), "{desktop}");
        }
        for desktop in ["KDE", "XFCE", "", "GNOME-Flashback"] {
            assert!(!is_gnome(Some(desktop)), "{desktop}");
        }
        assert!(!is_gnome(None));
    }

    /// Off GNOME: the buttons on the right and the usual clicks. On GNOME: its settings, key by
    /// key, with the default for one it can't read.
    #[test]
    fn the_title_bar_follows_gnome_or_the_defaults() {
        let plain = chrome_on("linux", true, None);
        assert_eq!((plain.frame, plain.layout.as_str()), ("custom", DEFAULT_LAYOUT));
        assert_eq!((plain.double_click.as_str(), plain.middle_click.as_str(), plain.right_click.as_str()), ("toggle-maximize", "lower", "menu"));
        let gnome = |key: &str| match key {
            "button-layout" => Some("appmenu:close".to_owned()),
            "action-middle-click-titlebar" => Some("none".to_owned()),
            _ => None,
        };
        let c = chrome_on("linux", false, Some(&gnome));
        assert_eq!((c.frame, c.layout.as_str(), c.middle_click.as_str(), c.double_click.as_str()), ("system", "appmenu:close", "none", "toggle-maximize"));
    }

    /// The page reads it as one object, in camelCase, and a value can't end the script.
    #[test]
    fn the_init_script_sets_the_object() {
        let mut c = chrome(true, None);
        c.layout = "</script>:close".into();
        let script = init_script(&c);
        assert!(script.starts_with("window.__GITVOLT_WINDOW__ = {\"frame\":\"custom\",\"platform\":"), "{script}");
        assert!(script.contains("\"doubleClick\":\"toggle-maximize\""), "{script}");
        assert!(!script.contains("</script>"), "{script}");
        assert!(script.ends_with("};"));
    }
}
