//! GitVolt's own title bar, on Linux: the window is built without the window manager's
//! decorations, and the UI draws a title bar (`ui/src/chrome/`) whose drags, resize edges,
//! window menu and buttons ask the window manager through the CEF runtime
//! (vendor/tauri-runtime-cef/GITVOLT-PATCH.md, "A window that draws its own title bar").
//! Settings › Appearance › "Use the system title bar" keeps the decorations; it's read when the
//! window is built, so a change applies after a restart. macOS and Windows keep the system's
//! frame for now.
//!
//! Only in a Wayland session (GitVolt itself runs under Xwayland there): the compositor moves
//! and resizes the window with a grab of its own. In an X11 session the window manager needs
//! the X pointer grab, which Chromium's own connection holds while the button is down, and
//! only Chromium could release it; a move or resize would never start, so the window keeps the
//! system's frame there.
//!
//! The UI learns which frame it has, and the window buttons and title bar clicks to follow, from
//! `window.__GITVOLT_WINDOW__`, set by an initialization script before its first render (no
//! title bar popping in), and from [`CHROME_EVENT`] when GNOME's settings change.

use serde::Serialize;

/// The event that carries a new [`WindowChrome`] when the desktop's settings change.
pub const CHROME_EVENT: &str = "gb:window-chrome";

/// What the UI needs to draw the title bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowChrome {
    /// `custom`: no decorations, the UI draws the title bar; `system`: the window manager's.
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

/// Whether GitVolt draws its own title bar: on Linux in a Wayland session (`wayland_display`,
/// the session's `WAYLAND_DISPLAY`), unless the user asked for the system's.
pub fn custom_frame(system_title_bar: bool, wayland_display: Option<&str>) -> bool {
    cfg!(target_os = "linux") && !system_title_bar && wayland_display.is_some_and(|d| !d.is_empty())
}

/// Whether `XDG_CURRENT_DESKTOP` (a `:`-separated list) names GNOME, whose window manager
/// settings the title bar follows. Elsewhere `org.gnome.desktop.wm.preferences` may be
/// installed but isn't what the window manager reads.
pub fn is_gnome(current_desktop: Option<&str>) -> bool {
    current_desktop.is_some_and(|d| d.split(':').any(|name| name.trim().eq_ignore_ascii_case("gnome")))
}

/// The keys of `org.gnome.desktop.wm.preferences` the title bar follows.
pub const GNOME_KEYS: [&str; 4] = ["button-layout", "action-double-click-titlebar", "action-middle-click-titlebar", "action-right-click-titlebar"];

/// Reads one of [`GNOME_KEYS`] (`None`: unset or unreadable).
pub type SettingReader<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The title bar for this frame, from GNOME's settings (`gnome`, by key; `None` off GNOME) or
/// the defaults: the window buttons on the right, a double click toggling maximise, a middle
/// click lowering the window and a right click showing the window menu.
pub fn chrome(custom: bool, gnome: Option<SettingReader<'_>>) -> WindowChrome {
    let read = |key: &str, default: &str| gnome.and_then(|get| get(key)).unwrap_or_else(|| default.to_owned());
    WindowChrome {
        frame: if custom { "custom" } else { "system" },
        platform: PLATFORM,
        layout: read("button-layout", DEFAULT_LAYOUT),
        double_click: read("action-double-click-titlebar", "toggle-maximize"),
        middle_click: read("action-middle-click-titlebar", "lower"),
        right_click: read("action-right-click-titlebar", "menu"),
    }
}

/// The initialization script that hands `chrome` to the page before its own scripts run.
pub fn init_script(chrome: &WindowChrome) -> String {
    // JSON is a JavaScript expression; `<` is escaped so no value can close a script element.
    let json = serde_json::to_string(chrome).unwrap_or_else(|_| "null".into()).replace('<', "\\u003c");
    format!("window.__GITVOLT_WINDOW__ = {json};")
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
    fn only_a_linux_wayland_session_draws_its_own_title_bar_unless_asked_not_to() {
        assert_eq!(custom_frame(false, Some("wayland-0")), cfg!(target_os = "linux"));
        assert!(!custom_frame(true, Some("wayland-0")));
        // An X11 session: the window manager couldn't take the pointer from Chromium.
        assert!(!custom_frame(false, None));
        assert!(!custom_frame(false, Some("")));
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
        let plain = chrome(true, None);
        assert_eq!((plain.frame, plain.layout.as_str()), ("custom", DEFAULT_LAYOUT));
        assert_eq!((plain.double_click.as_str(), plain.middle_click.as_str(), plain.right_click.as_str()), ("toggle-maximize", "lower", "menu"));
        let gnome = |key: &str| match key {
            "button-layout" => Some("appmenu:close".to_owned()),
            "action-middle-click-titlebar" => Some("none".to_owned()),
            _ => None,
        };
        let c = chrome(false, Some(&gnome));
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
