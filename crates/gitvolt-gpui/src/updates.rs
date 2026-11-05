//! GitVolt's own updates (`ui/src/updates/`): the status bar's pill and the dialog, showing the core's `UpdateState`. This
//! build's `Api` has no updater wired (it isn't what gets installed yet), so it never asks any host: the state stays
//! idle, Help › Check for updates says so, and the views are there for when it is.

use crate::app::{GitVolt, icon};
use crate::theme;
use crate::ui::controls::{self, ButtonKind};
use crate::ui::modal::{self, Modal};
use gitvolt_core::api::Request;
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};

const DIALOG: &str = "update-dialog";

/// `percentOf`.
pub fn percent_of(s: &Value) -> u32 {
    let (got, total) = (s["received"].as_f64().unwrap_or(0.), s["total"].as_f64().unwrap_or(0.));
    if total > 0. { ((got / total * 100.).round() as u32).min(100) } else { 0 }
}

impl GitVolt {
    /// `updateStatus`, once at the start; `updateChanged` keeps it from there.
    pub fn read_update_state(&mut self, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        if let Some(v) = std::env::var_os("GITVOLT_FAKE_UPDATE") {
            // A script's way to the pill and the dialog: nothing's asked of any host.
            self.update_state = json!({ "state": v.to_string_lossy(), "received": 40, "total": 100, "message": "The package didn't match its checksum", "release": { "version": "9.9.9", "name": "GitVolt 9.9.9", "prerelease": false, "asset": { "name": "gitvolt_9.9.9_amd64.deb", "size": 73400320 }, "notes": [{ "version": "9.9.9", "notes": "A fictional release for this check." }] } });
            return;
        }
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(Request::UpdateStatus).await });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(s)) = read.await {
                this.update(cx, |v, cx| {
                    v.update_state = s;
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn update_call(&mut self, req: Request, cx: &mut Context<Self>) {
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(req).await });
        cx.spawn(async move |this, cx| {
            let answer = read.await;
            this.update(cx, |v, cx| match answer {
                Ok(Ok(s)) => {
                    if s["state"].is_string() {
                        v.update_state = s;
                    }
                    cx.notify();
                }
                Ok(Err(e)) => crate::ui::toast::show(e.message, cx),
                Err(e) => crate::ui::toast::show(e.to_string(), cx),
            })
            .ok();
        })
        .detach();
    }

    /// Help › Check for updates.
    pub fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_about(window, cx);
        self.update_call(Request::UpdateCheck, cx);
    }

    fn open_update_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.update_state.clone();
        let release = state["release"].clone();
        if !release.is_object() {
            return;
        }
        let weak = cx.weak_entity();
        let m = Modal::new(DIALOG, move |_, _| {
            let v = release["version"].as_str().unwrap_or_default();
            let s = state["state"].as_str().unwrap_or_default();
            let call = |id: &'static str, label: String, req: fn() -> Request, weak: WeakEntity<GitVolt>| {
                crate::ui::anchor(id, controls::button(id, label, ButtonKind::Primary, true).on_click(move |_, window, cx| {
                    modal::close(DIALOG, window, cx);
                    weak.update(cx, |t, cx| t.update_call(req(), cx)).ok();
                }))
            };
            let mut notes = div().id("update-notes").flex().flex_col().gap(px(8.)).max_h(px(320.)).overflow_y_scroll();
            for n in release["notes"].as_array().cloned().unwrap_or_default() {
                notes = notes.child(div().child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_secondary()).child(n["version"].as_str().unwrap_or_default().to_string())).child(n["notes"].as_str().filter(|t| !t.trim().is_empty()).unwrap_or("No release notes.").to_string()));
            }
            let message = match s {
                "failed" => Some(state["message"].as_str().unwrap_or_default().to_string()),
                "installed" => Some(format!("GitVolt {v} is installed. Restart GitVolt to use it.")),
                "ready" => Some("The package is here, checked against the release's SHA256SUMS.".to_string()),
                "available" if !release["asset"].is_object() => Some("This release has no package for this system: get it from its release page.".to_string()),
                _ => None,
            };
            let mut actions = modal::actions_row();
            actions = actions.child(crate::ui::anchor("update-later", controls::button("update-later", "Later", ButtonKind::Plain, true).on_click(|_, window, cx| modal::close(DIALOG, window, cx))));
            match s {
                "available" if release["asset"].is_object() => actions = actions.child(call("update-download", "Download".into(), || Request::UpdateDownload, weak.clone())),
                "failed" => actions = actions.child(call("update-download", "Download again".into(), || Request::UpdateDownload, weak.clone())),
                "downloading" => actions = actions.child(call("update-cancel", "Cancel".into(), || Request::UpdateCancel, weak.clone())),
                "ready" => actions = actions.child(call("update-install", "Install".into(), || Request::UpdateInstall, weak.clone())),
                "installed" => actions = actions.child(call("update-restart", "Restart GitVolt".into(), || Request::UpdateRestart, weak.clone())),
                _ => {}
            }
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(modal::title(release["name"].as_str().unwrap_or_default().to_string()).when(release["prerelease"] == true, |d| d.child(div().px(px(6.)).rounded(px(3.)).text_size(px(11.)).bg(theme::control_bg()).child("Pre-release"))))
                .child(div().text_color(theme::text_secondary()).child(format!("You have GitVolt {}.", env!("CARGO_PKG_VERSION"))))
                .child(notes)
                .when_some(release["asset"].as_object().cloned(), |d, a| d.child(div().text_size(px(12.)).text_color(theme::text_secondary()).child(format!("{} · {}", a["name"].as_str().unwrap_or_default(), crate::format::format_bytes(a["size"].as_u64())))))
                .when_some(message, |d, t| d.child(div().text_color(if s == "failed" { theme::status_deleted() } else { theme::text_normal() }).child(t)))
                .child(actions)
                .into_any_element()
        })
        .width(520.);
        modal::open(m, window, cx);
    }
}

/// The status bar's pill: there only while there's an update, the same width in every state.
pub fn update_pill(this: &GitVolt, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let s = &this.update_state;
    let release = s["release"].as_object()?;
    let v = release.get("version")?.as_str()?.to_string();
    let state = s["state"].as_str()?;
    let via_dialog = !release.get("asset").is_some_and(Value::is_object);
    let (glyph, label, tooltip): (Option<&'static str>, String, Option<String>) = match state {
        "available" => (Some("arrow-down-to-line"), format!("Update to {v}"), Some(if via_dialog { format!("GitVolt {v} is available") } else { format!("Download GitVolt {v}") })),
        "downloading" => (None, String::new(), Some(format!("Downloading GitVolt {v}: {}%", percent_of(s)))),
        "ready" => (Some("arrow-down-to-line"), format!("Install {v}"), None),
        "installing" => (Some("loader-circle"), "Installing".to_string(), None),
        "installed" => (Some("rotate-cw"), "Restart to update".to_string(), None),
        "failed" => (Some("triangle-alert"), "Update failed".to_string(), s["message"].as_str().map(str::to_string)),
        _ => return None,
    };
    let failed = state == "failed";
    let pct = percent_of(s);
    let downloading = state == "downloading";
    let start_download = state == "available" && !via_dialog;
    let pill = crate::ui::anchor(
        "status-update",
        div()
            .id("status-update")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(5.))
            .min_w(px(118.))
            .h(px(16.))
            .mx(px(4.))
            .px(px(10.))
            .rounded(px(8.))
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme::text_on_accent())
            .when(!failed, |d| d.bg(linear_gradient(120., linear_color_stop(theme::blue(), 0.), linear_color_stop(theme::mix(theme::blue(), 0.65, theme::purple()), 1.))))
            .when(failed, |d| d.bg(theme::mix(theme::orange(), 0.85, theme::panel_bg())))
            .cursor_pointer()
            .on_click(cx.listener(move |v, _, window, cx| {
                if start_download {
                    v.update_call(Request::UpdateDownload, cx);
                } else {
                    v.open_update_dialog(window, cx);
                }
            }))
            .when_some(glyph, |d, g| d.child(icon(g, 11., theme::text_on_accent())))
            .when(downloading, |d| d.child(div().w(px(84.)).h(px(4.)).rounded(px(2.)).overflow_hidden().bg(Hsla { a: 0.28, ..theme::text_on_accent() }).child(div().h_full().rounded(px(2.)).w(px(84. * pct as f32 / 100.)).bg(theme::text_on_accent()))))
            .when(!label.is_empty(), |d| d.child(label)),
    );
    Some(match tooltip {
        Some(t) => crate::ui::tooltip::tip(t, pill).into_any_element(),
        None => pill.into_any_element(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_downloads_share_is_whole_and_bounded() {
        assert_eq!(percent_of(&json!({ "received": 1, "total": 3 })), 33);
        assert_eq!(percent_of(&json!({ "received": 9, "total": 3 })), 100);
        assert_eq!(percent_of(&json!({})), 0);
    }
}
