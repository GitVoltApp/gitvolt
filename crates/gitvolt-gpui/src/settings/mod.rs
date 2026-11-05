//! Settings: `ui/src/settings/`, its state, its pages (`pages.rs`) and its frame (`frame.rs`). `GitVolt::settings` holds
//! what the core says (`AppSettings`); `set_settings` changes and saves it, and each change takes effect here.

use crate::app::GitVolt;
use crate::theme;
use gpui::*;
use serde_json::{Value, json};

impl GitVolt {
    /// `setSettings`: changes the settings, saves them through the core, and applies what changed.
    pub fn set_settings(&mut self, change: impl FnOnce(&mut Value), cx: &mut Context<Self>) {
        if self.settings.is_null() {
            return;
        }
        change(&mut self.settings);
        let body = crate::write::call("saveSettings", json!({ "settings": self.settings }));
        if !cfg!(test) {
            let api = self.api.clone();
            self.rt.spawn(async move {
                if let Ok(req) = crate::write::request_of(body) {
                    api.dispatch(req).await.ok();
                }
            });
        }
        self.settings_changed(cx);
        cx.notify();
    }

    /// What the settings say now, applied: the theme first.
    pub fn settings_changed(&mut self, cx: &mut Context<Self>) {
        self.sync_theme(cx);
    }

    /// `bindThemeToSettings`: the theme the settings name, resolved against the custom ones.
    pub fn sync_theme(&mut self, cx: &mut Context<Self>) {
        let id = self.settings["theme"].as_str().unwrap_or(theme::DEFAULT_THEME_ID);
        let def = theme::theme_def(id, &self.settings["customThemes"]);
        let editor_before = theme::applied().editor_theme.clone();
        if theme::apply(def) {
            self.theme_shown(editor_before != theme::applied().editor_theme, cx);
        }
    }

    /// The colours on screen changed: everything redraws, and what's built from colours is built again.
    pub fn theme_shown(&mut self, recolour_files: bool, cx: &mut Context<Self>) {
        self.colours_cache.borrow_mut().clear();
        if recolour_files {
            self.recolour_open_files(cx);
        }
        cx.refresh_windows();
    }

    /// The token colours of what's open come from the theme: each open file's read again, keeping what's unsaved.
    fn recolour_open_files(&mut self, cx: &mut Context<Self>) {
        let repos: Vec<u32> = self.tabs.iter().filter(|t| !t.is_open_tab()).map(|t| t.id).collect();
        for repo in repos {
            self.recolour_file(repo, cx);
        }
    }
}
