//! GitVolt's own updates (`ui/src/updates/`): the status bar's pill and the dialog, showing the core's `UpdateState`, and Install
//! and restart, which downloads, installs and restarts in one go. This build's updater has no source of releases (it isn't what
//! gets installed yet; `main.rs`'s `NoReleases`), so it never asks any host: the state stays idle, Help › Check for updates says
//! so, and the views are there for when it is. The updater's there for the restart it can do (`Request::RelaunchApp`).

use crate::app::{GitVolt, icon};
use crate::md::view::{Content, MdOptions, MdView};
use crate::theme;
use crate::ui::controls::{self, ButtonKind};
use crate::ui::modal::{self, Modal};
use gitvolt_core::api::Request;
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

const DIALOG: &str = "update-dialog";

/// `percentOf`.
pub fn percent_of(s: &Value) -> u32 {
    let (got, total) = (s["received"].as_f64().unwrap_or(0.), s["total"].as_f64().unwrap_or(0.));
    if total > 0. { ((got / total * 100.).round() as u32).min(100) } else { 0 }
}

/// What GitVolt does with the verified package, per install kind (the core's `install.rs`).
fn installs(kind: &str) -> &'static str {
    match kind {
        "deb" => "installs it with apt (your system asks for your password) and restarts",
        "arch" => "installs it with pacman (your system asks for your password) and restarts",
        "rpm" => "installs it with dnf or zypper (your system asks for your password) and restarts",
        "dmg" => "copies its GitVolt.app over this one and restarts",
        // The installer's last page starts GitVolt (its Start GitVolt box, ticked).
        "nsis" => "opens the installer and closes so it can be replaced; the installer's last page starts the new version",
        // Windows Installer doesn't start it again.
        "msi" => "opens Windows Installer (which may ask for permission) and closes so it can be replaced; start GitVolt again once it's done",
        _ => "",
    }
}

/// The one button's label: on Windows the installer, not GitVolt, finishes the job.
fn install_label(kind: &str) -> &'static str {
    if kind == "msi" { "Install" } else { "Install and restart" }
}

/// An update a download can start from: offered, or offered again after a failure.
fn downloadable(s: &Value) -> bool {
    s["state"] == "available" || (s["state"] == "failed" && s["release"].is_object())
}

/// `Oct 19, 2026` for a release's publish time; empty when it has none. UTC, as the release's day.
fn release_day(iso: Option<&str>) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let Some(date) = iso.and_then(|s| s.get(..10)) else { return String::new() };
    let parts: Vec<&str> = date.split('-').collect();
    match parts[..] {
        [y, m, d] => match (y.parse::<u32>(), m.parse::<usize>(), d.parse::<u32>()) {
            (Ok(y), Ok(m @ 1..=12), Ok(d @ 1..=31)) => format!("{} {d}, {y}", MONTHS[m - 1]),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

impl GitVolt {
    /// `updateStatus`, once at the start; `updateChanged` keeps it from there.
    pub fn read_update_state(&mut self, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        if let Some(v) = std::env::var_os("GITVOLT_FAKE_UPDATE") {
            // A script's way to the pill and the dialog: nothing's asked of any host. A package install, so the whole flow shows.
            self.install_kind = "deb".into();
            self.update_state = json!({ "state": v.to_string_lossy(), "received": 40, "total": 100, "message": "The package didn't match its checksum", "release": { "version": "9.9.9", "name": "GitVolt 9.9.9", "prerelease": false, "asset": { "name": "gitvolt_9.9.9_amd64.deb", "size": 73400320 }, "notes": [{ "version": "9.9.9", "notes": "A **fictional** release for this check.", "publishedAt": "2026-11-12T10:00:00Z" }, { "version": "9.9.8", "notes": "", "publishedAt": null }] } });
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

    /// Help › Check for updates (`checkAndShow`): About shows the answer; an update it finds, or one already known, opens the update
    /// dialog over it, About closing under it.
    pub fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_about(window, cx);
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(Request::UpdateCheck).await });
        cx.spawn(async move |this, cx| {
            let answer = read.await;
            this.update(cx, |v, cx| match answer {
                Ok(Ok(s)) => {
                    if s["state"].is_string() {
                        v.update_state = s;
                    }
                    if v.update_state["release"].is_object() {
                        let weak = cx.weak_entity();
                        v.with_window(cx, move |window, cx| {
                            modal::close("about", window, cx);
                            weak.update(cx, |v, cx| v.open_update_dialog(window, cx)).ok();
                        });
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

    /// Every unsaved piece of work, by what identifies it (`unsavedKeys`): a tab's edited file, a tab's merge. To tell work that
    /// came later from work the user already agreed to discard.
    pub fn unsaved_keys(&self) -> HashSet<String> {
        let mut keys = HashSet::new();
        for t in &self.tabs {
            if let Some(d) = t.diff.as_ref().filter(|d| d.editing_dirty()) {
                keys.insert(format!("wc:{}:{}", t.id, d.file.path));
            }
            if t.merge_dirty() {
                keys.insert(format!("merge:{}", t.id));
            }
        }
        keys
    }

    /// The dialog's one action (`installAndRestart`): downloads the update (unless it already is), installs it and restarts
    /// GitVolt, each step only on the last one's success. Unsaved edits ask first, before anything starts, and again before the
    /// restart only for edits made since. It runs here, not in the dialog, so closing the dialog doesn't stop it.
    pub fn install_and_restart(&mut self, cx: &mut Context<Self>) {
        if self.update_running {
            return;
        }
        self.confirm_discard_unsaved(Some("update-install"), |v, cx| {
            let agreed = v.unsaved_keys();
            v.update_running = true;
            v.update_outcome = Value::Null;
            cx.notify();
            v.update_steps(agreed, cx);
        }, cx);
    }

    fn update_steps(&mut self, agreed: HashSet<String>, cx: &mut Context<Self>) {
        let installed = self.update_state["state"] == "installed";
        let (api, rt) = (self.api.clone(), self.rt.clone());
        cx.spawn(async move |this, cx| {
            if !installed {
                let start = this.update(cx, |v, _| downloadable(&v.update_state)).unwrap_or(false);
                if start {
                    let api = api.clone();
                    let answer = rt.spawn(async move { api.dispatch(Request::UpdateDownload).await }).await;
                    let ok = this
                        .update(cx, |v, cx| match answer {
                            // Its events may have moved it on already (a small, fast download).
                            Ok(Ok(s)) => {
                                if downloadable(&v.update_state) && s["state"].is_string() {
                                    v.update_state = s;
                                }
                                true
                            }
                            Ok(Err(e)) => {
                                crate::ui::toast::show(e.message, cx);
                                false
                            }
                            Err(e) => {
                                crate::ui::toast::show(e.to_string(), cx);
                                false
                            }
                        })
                        .unwrap_or(false);
                    if !ok {
                        this.update(cx, |v, cx| v.update_stopped(cx)).ok();
                        return;
                    }
                }
                // Until the download under way has ended: ready, failed, or cancelled (offered again).
                loop {
                    let Ok(state) = this.update(cx, |v, _| v.update_state["state"].as_str().unwrap_or_default().to_string()) else { return };
                    if state != "downloading" {
                        if state != "ready" {
                            this.update(cx, |v, cx| v.update_stopped(cx)).ok();
                            return;
                        }
                        break;
                    }
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                }
                let api = api.clone();
                let answer = rt.spawn(async move { api.dispatch(Request::UpdateInstall).await }).await;
                let go_on = this
                    .update(cx, |v, cx| match answer {
                        Ok(Ok(outcome)) => {
                            let installed = outcome["outcome"] == "installed";
                            // Windows: the installer took over and GitVolt is quitting. Manual: the dialog shows the command instead.
                            v.update_outcome = outcome;
                            cx.notify();
                            installed
                        }
                        Ok(Err(e)) => {
                            crate::ui::toast::show(e.message, cx);
                            false
                        }
                        Err(e) => {
                            crate::ui::toast::show(e.to_string(), cx);
                            false
                        }
                    })
                    .unwrap_or(false);
                if !go_on {
                    this.update(cx, |v, cx| v.update_stopped(cx)).ok();
                    return;
                }
            }
            this.update(cx, |v, cx| {
                // Edits made since the answer ask again; kept, the update waits on Restart GitVolt.
                let later = v.unsaved_keys().difference(&agreed).next().is_some();
                v.update_running = false;
                if later {
                    v.confirm_discard_unsaved(Some("update-install"), |v, cx| v.restart_now(|| Request::UpdateRestart, cx), cx);
                } else {
                    v.restart_now(|| Request::UpdateRestart, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn update_stopped(&mut self, cx: &mut Context<Self>) {
        self.update_running = false;
        cx.notify();
    }

    /// The update dialog (from the status bar's pill, or a check from Help): the notes of every version since this one, rendered;
    /// the package and its size; and Install and restart, which does what this install takes. A build from source, or a release
    /// without this install's package, only points to the release page.
    pub fn open_update_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let release = self.update_state["release"].clone();
        if !release.is_object() {
            return;
        }
        let notes: Vec<Value> = release["notes"].as_array().cloned().unwrap_or_default();
        let many = notes.len() > 1;
        let sections: Vec<(Option<String>, Option<Entity<MdView>>)> = notes
            .iter()
            .map(|n| {
                let label = many.then(|| [n["version"].as_str().unwrap_or_default().to_string(), release_day(n["publishedAt"].as_str())].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · "));
                let text = n["notes"].as_str().unwrap_or_default().trim().to_string();
                let view = (!text.is_empty()).then(|| {
                    let opts = MdOptions::for_comment(0, Default::default(), crate::md::ast::Flavor::Github, Default::default());
                    let (api, rt, retired) = (self.api.clone(), self.rt.clone(), self.retired_images.clone());
                    let text: Arc<str> = text.into();
                    cx.new(|cx| {
                        let mut v = MdView::new(opts, api, rt, retired, cx);
                        v.set_content(Content::Text(text), false, cx);
                        v
                    })
                });
                (label, view)
            })
            .collect();
        let weak = cx.weak_entity();
        let mirror = self.update_mirror.clone();
        let m = Modal::new(DIALOG, move |_, _| {
            let (state, running, outcome, kind) = mirror.borrow().clone();
            let release = if state["release"].is_object() { state["release"].clone() } else { release.clone() };
            let ver = release["version"].as_str().unwrap_or_default().to_string();
            let s = state["state"].as_str().unwrap_or_default();
            let message = |t: String| div().text_color(theme::text_normal()).child(t);
            let go = |label: &'static str, busy: bool| {
                let weak = weak.clone();
                crate::ui::anchor("update-install", controls::button("update-install", label, ButtonKind::Primary, !busy).on_click(move |_, _, cx| {
                    weak.update(cx, |v, cx| v.install_and_restart(cx)).ok();
                }))
            };
            let mut middle = div().flex().flex_col().gap(px(8.));
            let mut actions = modal::actions_row();
            actions = actions.child(crate::ui::anchor("update-later", controls::button("update-later", "Later", ButtonKind::Plain, true).on_click(|_, window, cx| modal::close(DIALOG, window, cx))));
            if !release["asset"].is_object() || kind == "none" {
                middle = middle.child(message(if kind == "none" { "This GitVolt was built from source: get the new version from its release page.".into() } else { "This release has no package for this install: get it from its release page.".into() }));
            } else {
                let whole = format!("Downloads GitVolt {ver}, checks it against the release's SHA256SUMS, {}.", installs(&kind));
                match s {
                    "available" => {
                        middle = middle.child(message(whole));
                        actions = actions.child(go(install_label(&kind), running));
                    }
                    "downloading" => {
                        let pct = percent_of(&state);
                        let weak = weak.clone();
                        middle = middle.child(message(whole)).child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().w(px(180.)).h(px(6.)).rounded(px(3.)).overflow_hidden().bg(theme::control_bg()).child(div().h_full().w(px(180. * pct as f32 / 100.)).bg(theme::blue())))
                                .child(format!("{pct}%"))
                                .child(crate::ui::anchor("update-cancel", controls::button("update-cancel", "Cancel", ButtonKind::Plain, true).on_click(move |_, _, cx| {
                                    weak.update(cx, |v, cx| v.update_call(Request::UpdateCancel, cx)).ok();
                                }))),
                        );
                    }
                    "ready" | "installing" => {
                        let busy = running || s == "installing";
                        middle = middle.child(message(format!("Downloaded and checked against the release's SHA256SUMS: GitVolt {}.", installs(&kind))));
                        if outcome["outcome"] == "manual" {
                            middle = middle
                                .child(div().text_color(theme::status_deleted()).child(outcome["reason"].as_str().unwrap_or_default().to_string()))
                                .child(div().text_color(theme::text_secondary()).child("Run this in a terminal:"))
                                .child(div().px(px(8.)).py(px(4.)).rounded(px(4.)).bg(theme::control_bg()).font_family(crate::app::MONO_FONT).text_size(px(12.)).child(outcome["command"].as_str().unwrap_or_default().to_string()));
                        }
                        actions = actions.child(go(if busy { "Installing…" } else { install_label(&kind) }, busy));
                    }
                    // A restart that didn't happen (it failed, or unsaved edits were kept).
                    "installed" => {
                        middle = middle.child(message(format!("GitVolt {ver} is installed. Restart GitVolt to use it.")));
                        actions = actions.child(go(if running { "Restarting…" } else { "Restart GitVolt" }, running));
                    }
                    "failed" => {
                        middle = middle.child(div().text_color(theme::status_deleted()).child(state["message"].as_str().unwrap_or_default().to_string()));
                        actions = actions.child(go("Try again", running));
                    }
                    _ => {}
                }
            }
            let mut notes = div().id("update-notes").flex().flex_col().gap(px(8.)).max_h(px(320.)).overflow_y_scroll();
            if !sections.iter().any(|(_, v)| v.is_some()) {
                notes = notes.child(div().italic().text_color(theme::text_secondary()).child("No release notes."));
            } else {
                for (label, view) in &sections {
                    notes = notes.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .when_some(label.clone(), |d, l| d.child(div().text_size(px(12.)).text_color(theme::text_secondary()).child(l)))
                            .map(|d| match view {
                                Some(v) => d.child(v.clone()),
                                None => d.child(div().italic().text_color(theme::text_secondary()).child("No release notes.")),
                            }),
                    );
                }
            }
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(modal::title(release["name"].as_str().unwrap_or_default().to_string()).when(release["prerelease"] == true, |d| d.child(div().px(px(6.)).rounded(px(3.)).text_size(px(11.)).bg(theme::control_bg()).child("Pre-release"))))
                .child(div().text_color(theme::text_secondary()).child(format!("You have GitVolt {}.", env!("CARGO_PKG_VERSION"))))
                .child(notes)
                .when_some(release["asset"].as_object().cloned(), |d, a| d.child(div().text_size(px(12.)).text_color(theme::text_secondary()).child(format!("{} · {}", a["name"].as_str().unwrap_or_default(), crate::format::format_bytes(a["size"].as_u64())))))
                .child(middle)
                .child(actions)
                .into_any_element()
        })
        .width(520.);
        modal::open(m, window, cx);
    }
}

/// The status bar's pill: there only while there's an update, the same width in every state. Every state opens the update
/// dialog: "Update to 0.3.0" (its notes, and Install and restart); while that runs, a small progress bar, then "Installing
/// 0.3.0". "Install 0.3.0" and "Restart to update" are left when it stopped short: an install that couldn't run by itself, a
/// restart that failed.
pub fn update_pill(this: &GitVolt, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let s = &this.update_state;
    let release = s["release"].as_object()?;
    let v = release.get("version")?.as_str()?.to_string();
    let state = match s["state"].as_str()? {
        // Install and restart goes straight on to the install.
        "ready" if this.update_running => "installing",
        other => other,
    };
    let (glyph, label, tooltip): (Option<&'static str>, String, Option<String>) = match state {
        "available" => (Some("arrow-down-to-line"), format!("Update to {v}"), Some(format!("GitVolt {v} is available"))),
        "downloading" => (None, String::new(), Some(format!("Downloading GitVolt {v}: {}%", percent_of(s)))),
        "ready" => (Some("arrow-down-to-line"), format!("Install {v}"), None),
        "installing" => (Some("loader-circle"), format!("Installing {v}"), None),
        "installed" => (Some("rotate-cw"), "Restart to update".to_string(), None),
        "failed" => (Some("triangle-alert"), "Update failed".to_string(), s["message"].as_str().map(str::to_string)),
        _ => return None,
    };
    let failed = state == "failed";
    let pct = percent_of(s);
    let downloading = state == "downloading";
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
            .on_click(cx.listener(|v, _, window, cx| v.open_update_dialog(window, cx)))
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

    #[test]
    fn a_download_starts_from_an_offer_or_after_a_failure_with_its_release() {
        assert!(downloadable(&json!({ "state": "available", "release": {} })));
        assert!(downloadable(&json!({ "state": "failed", "release": {} })));
        assert!(!downloadable(&json!({ "state": "failed", "release": null })));
        assert!(!downloadable(&json!({ "state": "ready", "release": {} })));
    }

    #[test]
    fn the_button_and_its_plan_follow_the_install() {
        assert_eq!(install_label("deb"), "Install and restart");
        assert_eq!(install_label("msi"), "Install");
        assert!(installs("deb").contains("apt"));
        assert_eq!(installs("none"), "");
        assert_eq!(release_day(Some("2026-10-19T08:00:00Z")), "Oct 19, 2026");
        assert_eq!(release_day(None), "");
    }
}
