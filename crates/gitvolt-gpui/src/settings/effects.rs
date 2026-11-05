//! What each setting does once it's set: the rows only store a value, this makes it take effect.

use crate::app::GitVolt;
use gitvolt_core::api::Request;
use gpui::*;
use serde_json::{Value, json};

impl GitVolt {
    /// The shown tab's graph again.
    pub fn reload_active(&mut self, cx: &mut Context<Self>) {
        if let Some(repo) = self.tabs.get(self.active).filter(|t| !t.is_open_tab()).map(|t| t.id) {
            self.reload(repo, cx);
        }
    }

    pub fn send_debug_logging(&mut self, _: &mut Context<Self>) {
        let debug = self.settings["debugLogging"] == true;
        let api = self.api.clone();
        self.rt.spawn(async move { api.dispatch(Request::SetDebugLogging { debug }).await });
    }

    pub fn open_logs_folder(&mut self, cx: &mut Context<Self>) {
        let api = self.api.clone();
        let done = self.rt.spawn(async move { api.dispatch(Request::OpenLogsFolder).await });
        cx.spawn(async move |this, cx| {
            if let Ok(Err(e)) = done.await {
                this.update(cx, |_, cx| crate::ui::toast::show(e.message, cx)).ok();
            }
        })
        .detach();
    }

    /// `gitIdentity`: on showing the page, when the tab or the extra config changes.
    pub fn read_git_identity(&mut self, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let tab = self.tabs.get(self.active).filter(|t| !t.is_open_tab());
        let (repo, worktree) = (tab.map(|t| t.id), tab.map(|t| t.worktree.clone()));
        let key = (repo.unwrap_or(0), format!("{}|{}", worktree.as_deref().unwrap_or_default(), self.profile["extraGitconfig"].as_str().unwrap_or_default()));
        if self.settings_ui.identity_for.as_ref() == Some(&key) {
            return;
        }
        self.settings_ui.identity_for = Some(key);
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(Request::GitIdentity { repo, worktree }).await });
        cx.spawn(async move |this, cx| {
            let answer = read.await;
            this.update(cx, |v, cx| {
                v.settings_ui.identity = answer.ok().and_then(Result::ok);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The openers when no tab has them (Settings over the Open screen).
    pub fn read_settings_openers(&mut self, cx: &mut Context<Self>) {
        if cfg!(test) || self.settings_openers.is_some() {
            return;
        }
        let api = self.api.clone();
        self.settings_openers = Some(Default::default());
        let read = self.rt.spawn(async move { api.dispatch(Request::ListOpeners).await });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(v)) = read.await {
                this.update(cx, |t, cx| {
                    t.settings_openers = Some(std::rc::Rc::new(crate::open_in::openers_of(&v)));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// The list again: a Custom entry joins or leaves it.
    pub fn refresh_openers(&mut self, cx: &mut Context<Self>) {
        let repos: Vec<u32> = self.tabs.iter().filter(|t| !t.is_open_tab()).map(|t| t.id).collect();
        for repo in repos {
            self.list_openers(repo, cx);
        }
    }

    /// `EditorPicker`'s check: only a template that passes is saved; the draft stays in the field meanwhile.
    pub fn validate_editor_template(&mut self, def: &'static crate::settings::pages::Def, text: &str, cx: &mut Context<Self>) {
        self.settings_ui.editor_seq += 1;
        let seq = self.settings_ui.editor_seq;
        let template = text.to_string();
        if template.trim().is_empty() {
            self.settings_ui.editor_custom.insert(def.id.to_string(), (true, None));
            cx.notify();
            return;
        }
        let api = self.api.clone();
        let timer = cx.background_executor().timer(std::time::Duration::from_millis(300));
        cx.spawn(async move |this, cx| {
            timer.await;
            let go = this.update(cx, |v, _| v.settings_ui.editor_seq == seq).unwrap_or(false);
            if !go {
                return;
            }
            let t = template.clone();
            let read = this.update(cx, |v, _| v.rt.spawn(async move { api.dispatch(Request::ValidateEditorTemplate { template: t }).await }));
            let Ok(read) = read else { return };
            let answer = read.await;
            this.update(cx, |v, cx| {
                if v.settings_ui.editor_seq != seq {
                    return;
                }
                match answer {
                    Ok(Ok(_)) => {
                        v.settings_ui.editor_custom.insert(def.id.to_string(), (true, None));
                        crate::settings::frame::set_bound(v, &def.binding, json!({ "kind": "custom", "template": template }), cx);
                        v.refresh_openers(cx);
                    }
                    Ok(Err(e)) => {
                        v.settings_ui.editor_custom.insert(def.id.to_string(), (true, Some(e.message)));
                    }
                    Err(e) => {
                        v.settings_ui.editor_custom.insert(def.id.to_string(), (true, Some(e.to_string())));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn set_host_kind(&mut self, host: &str, kind: &str, cx: &mut Context<Self>) {
        let (host, kind) = (host.to_string(), kind.to_string());
        self.update_profile(
            move |p| {
                if !p["hostOverrides"].is_object() {
                    p["hostOverrides"] = json!({});
                }
                let o = p["hostOverrides"].as_object_mut().expect("just made");
                if kind.is_empty() {
                    o.remove(&host);
                } else {
                    o.insert(host, Value::String(kind));
                }
            },
            cx,
        );
    }

    /// How wide the page's content is: for what lays itself out by columns.
    pub fn settings_body_width(&self) -> f32 {
        let vw = self.window_width;
        let dialog = crate::settings::frame::SETTINGS_W.min(vw - 48.);
        dialog - if vw <= 760. { 180. } else { 220. } - 56.
    }

    /// What depends on the settings, compared with what's applied: nothing runs for an unrelated setting.
    pub fn sync_effects(&mut self, cx: &mut Context<Self>) {
        let s = &self.settings;
        let ui = crate::settings::fonts::ui_family(s["uiFontFamily"].as_str().unwrap_or_default(), &self.base_font);
        if ui != self.font {
            self.font = ui.clone();
            crate::ui::set_font(ui, cx);
            cx.refresh_windows();
        }
        let density = self.prefs["density"].as_str().unwrap_or("standard").to_string();
        if crate::metrics::set_density(&density) {
            self.layout_changed(cx);
            cx.refresh_windows();
        }
        let s = &self.settings;
        if crate::metrics::set_date_options(s["dateFormat"].as_str().unwrap_or("ymd12h"), s["relativeDates"] == true) {
            self.dates_changed(cx);
        }
        self.sync_relative_clock(cx);
        let s = &self.settings;
        let size = s["editorFontSize"].as_f64().unwrap_or(13.).round().clamp(8., 32.) as f32;
        let set = s["editorLineHeight"].as_f64().unwrap_or(0.);
        let set = if set <= 0. { 0. } else { set.round().clamp(8., 64.) as f32 };
        let line = if set > 0. { set.max(size) } else { (size * 1.35).round() };
        let tabs = s["editorTabSize"].as_f64().unwrap_or(4.).round().clamp(1., 16.) as f32;
        let style = crate::metrics::EditorStyle { family: s["editorFontFamily"].as_str().unwrap_or_default().to_string(), ligatures: s["editorFontLigatures"] == true, whitespace: s["editorWhitespace"].as_str().unwrap_or("none").to_string(), minimap: s["editorMinimap"] != false, sticky: s["editorStickyScroll"] == true };
        let tab_changed = crate::metrics::tab_size_f() != tabs;
        let changed = crate::metrics::editor_font_size() != size || crate::metrics::line_h() != line || tab_changed || crate::metrics::editor_style() != style;
        if changed {
            crate::metrics::set_editor_font_size(size);
            crate::metrics::set_line_h(line);
            crate::metrics::set_tab_size_f(tabs);
            crate::metrics::set_editor_style(style);
            self.editor_font_dirty = true;
            if tab_changed {
                self.recolour_open_files(cx);
            }
            self.layout_changed(cx);
            cx.refresh_windows();
        }
        self.sync_fetch_schedule(cx);
    }

    /// The advance a line wraps by, of the font that's set: measured with a window to hand.
    pub fn measure_editor_font(&mut self, window: &Window) {
        if !self.editor_font_dirty {
            return;
        }
        self.editor_font_dirty = false;
        let ts = window.text_system();
        let id = ts.resolve_font(&crate::diff::view::mono());
        if let Ok(a) = ts.advance(id, px(crate::metrics::editor_font_size()), 'm') {
            crate::metrics::set_advance(a.width.as_f32());
        }
    }

    /// What's laid out by these sizes is laid out again.
    fn layout_changed(&mut self, cx: &mut Context<Self>) {
        for tab in &mut self.tabs {
            if let Some(d) = tab.diff.as_mut() {
                d.invalidate_layout();
            }
        }
        cx.notify();
    }

    /// The shown commit's own dates are read with it.
    fn dates_changed(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        if tab.diff.is_none() && tab.commit.is_some() && let Some(ix) = tab.selected {
            self.select(ix, cx);
        }
        cx.notify();
    }

    /// Relative dates tick once a minute while they're on, and nothing runs while they're off.
    fn sync_relative_clock(&mut self, cx: &mut Context<Self>) {
        let on = crate::metrics::relative_dates();
        if on == self.relative_clock.is_some() {
            return;
        }
        self.relative_clock = on.then(|| {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(std::time::Duration::from_secs(60)).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        return;
                    }
                }
            })
        });
    }
}
