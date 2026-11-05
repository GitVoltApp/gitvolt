//! The sidebar's MR/PR section (`mrSection.ts`, `MrFilterButton.tsx`, `RefreshButton.tsx`, `ForgeStale.tsx`) and what its rows and
//! the remotes' rows carry: state and pipeline icons (`MrIcons.tsx`), the forge's marks, the "main" chip.

use super::poll::now_ms;
use super::store::{Refreshing, stale_text};
use super::text::{detect_host_kind, effective_kind, kind_of, mark_icon, mr_section_label, pipeline_text};
use crate::app::{GitVolt, Tab, icon};
use crate::sidebar::ItemKind;
use crate::theme;
use crate::ui::menu::{self, MenuRow};
use crate::ui::tooltip::tip;
use gitvolt_core::forge::types::{ForgePipeline, MrFilter, MrState, PipelineStatus};
use gitvolt_core::forge::ForgeKind;
use gpui::prelude::*;
use gpui::*;
use std::collections::HashMap;
use std::time::Duration;

const FILTERS: [(MrFilter, &str, &str, &str); 3] = [
    (MrFilter::Mine, "mine", "Mine", "Show the open ones you opened"),
    (MrFilter::ReviewRequested, "reviewRequested", "Review requested", "Show the open ones waiting for your review"),
    (MrFilter::All, "all", "All", "Show every open one"),
];

pub fn state_colour(s: MrState) -> Hsla {
    match s {
        MrState::Open => theme::green(),
        MrState::Draft => theme::text_secondary(),
        MrState::Merged => theme::blue(),
        MrState::Merging => theme::purple(),
        MrState::Closed => theme::red(),
    }
}

pub fn state_glyph(s: MrState) -> &'static str {
    match s {
        MrState::Open => "git-pull-request",
        MrState::Draft => "git-pull-request-draft",
        MrState::Merged => "git-merge",
        MrState::Merging => "loader-circle",
        MrState::Closed => "git-pull-request-closed",
    }
}

fn turning(name: &'static str, size: f32, color: Hsla, key: impl Into<ElementId>) -> AnyElement {
    icon(name, size, color).with_animation(key, Animation::new(Duration::from_secs(1)).repeat(), |s, d| s.with_transformation(Transformation::rotate(percentage(d)))).into_any_element()
}

/// The badge's icon: one per state, coloured by it. A merge under way turns, for the moments it lasts.
pub fn state_icon(s: MrState, size: f32) -> AnyElement {
    if s == MrState::Merging {
        return turning("loader-circle", size, state_colour(s), SharedString::from(format!("mr-merging-{size}")));
    }
    icon(state_glyph(s), size, state_colour(s)).into_any_element()
}

pub fn state_icon_coloured(s: MrState, size: f32, color: Hsla) -> AnyElement {
    if s == MrState::Merging {
        return turning("loader-circle", size, color, SharedString::from(format!("mr-merging-c-{size}")));
    }
    icon(state_glyph(s), size, color).into_any_element()
}

/// A running pipeline's icon turns: it's something working, not an idle animation.
pub fn pipeline_icon(p: &ForgePipeline, size: f32) -> AnyElement {
    let (glyph, color) = match p.status {
        PipelineStatus::Pending => ("circle-dashed", theme::yellow()),
        PipelineStatus::Running => ("loader-circle", theme::blue()),
        PipelineStatus::Success => ("circle-check", theme::green()),
        PipelineStatus::Failed => ("circle-x", theme::red()),
        PipelineStatus::Canceled | PipelineStatus::Skipped => ("circle-slash", theme::text_secondary()),
        PipelineStatus::Manual => ("hand", theme::text_secondary()),
    };
    // A pipeline can run for an hour, and a turning icon redraws the window all that while: it stands still.
    icon(glyph, size, color).into_any_element()
}

/// A remote's forge by its host, with the profile's override.
pub fn remote_kinds(tab: &Tab, overrides: &serde_json::Value) -> HashMap<String, ForgeKind> {
    let mut out = HashMap::new();
    for r in tab.info["remotes"].as_array().into_iter().flatten() {
        let Some(name) = r["name"].as_str() else { continue };
        let Some(host) = r["host"].as_str() else { continue };
        let detected = r["hostKind"].as_str().unwrap_or_else(|| detect_host_kind(host));
        if let Some(k) = kind_of(effective_kind(host, detected, overrides)) {
            out.insert(name.to_string(), k);
        }
    }
    out
}

/// `RemoteIcon`: the project owner's picture once it's loaded, else the forge's mark, else a cloud.
pub fn remote_icon(_remote: &str, kind: Option<ForgeKind>, owner: Option<&str>, size: f32) -> AnyElement {
    if let (Some(url), Some(_)) = (owner, kind)
        && let Some(picture) = crate::avatars::image_for_url(url)
    {
        return div().flex_none().size(px(size)).rounded_full().overflow_hidden().child(img(picture).size(px(size)).rounded_full()).into_any_element();
    }
    match kind {
        Some(k) => icon(mark_icon(k), size, theme::sidebar_icon()).into_any_element(),
        None => icon("cloud", size + 2., theme::sidebar_icon()).into_any_element(),
    }
}

/// The dim "main" chip on the remote whose forge project the repo's MRs/PRs target.
pub fn main_chip(remote: &str, kind: Option<ForgeKind>, chosen: bool) -> impl IntoElement + use<> {
    let noun = if matches!(kind, Some(ForgeKind::GitHub | ForgeKind::Forgejo)) { "pull requests" } else { "merge requests" };
    let text = if chosen { format!("The main remote: its forge project is used for {noun}") } else { "Picked automatically: right-click a remote to choose".to_string() };
    let _ = remote;
    tip(text, div().flex_none().text_size(px(10.)).line_height(px(10.)).px(px(5.)).py(px(2.)).rounded(px(8.)).text_color(theme::text_secondary()).bg(theme::panel_bg1()).when(!chosen, |d| d.opacity(0.65)).child(if chosen { "main" } else { "main (auto)" }))
}

/// A dim warning after a remote's name: why its forge project couldn't be loaded.
pub fn lookup_warning(why: &str) -> impl IntoElement + use<> {
    tip(format!("Not on the forge: {why}"), div().flex().flex_none().opacity(0.7).child(icon("triangle-alert", 13., theme::orange())))
}

/// What a row carries at its end: a Local row's badge; an MR/PR row's pipeline, only when it needs a look (the hover card says the
/// rest), and its mark when it's set to merge by itself.
pub fn row_trailing(kind: &ItemKind, tab: u32, forge: Option<ForgeKind>, by_ref: &std::collections::BTreeMap<String, gitvolt_core::forge::types::ForgeMr>, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    let mut out = Vec::new();
    match kind {
        ItemKind::Local { upstream, .. } => {
            let found = forge.zip(upstream.as_ref().and_then(|u| by_ref.get(u)));
            if let Some((forge, mr)) = found {
                out.push(super::card::mr_badge(tab, forge, mr, 13., false, cx));
            }
        }
        ItemKind::Mr { mr, forge } => {
            if let Some(p) = mr.pipeline.as_ref().filter(|p| matches!(p.status, PipelineStatus::Failed | PipelineStatus::Running | PipelineStatus::Pending)) {
                out.push(tip(pipeline_text(*forge, Some(p)), div().flex().flex_none().child(pipeline_icon(p, 12.))).into_any_element());
            }
            if mr.auto_merge.is_some() {
                out.push(tip("Set to auto-merge", div().flex().flex_none().opacity(0.8).child(icon("git-merge", 12., theme::text_secondary()))).into_any_element());
            }
        }
        _ => {}
    }
    out
}

fn head_button(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    div().id(id).flex().flex_none().size(px(18.)).items_center().justify_center().rounded(px(3.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).when(on, |d| d.bg(theme::selected_row()))
}

/// The section header's stale note, refresh and filter.
pub fn head_actions(tab: &Tab, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    let Some(kind) = tab.forge.kind else { return Vec::new() };
    let mut out = Vec::new();
    if let Some(text) = stale_text(&tab.forge, now_ms()) {
        // Not a failure (a limit's wait, the last session's data): a clock, not a warning.
        let glyph = if tab.forge.error.is_some() { "triangle-alert" } else { "clock" };
        out.push(tip(text, div().flex().flex_none().child(icon(glyph, 12., theme::orange()))).into_any_element());
    }
    let id = tab.id;
    let label = format!("Refresh {}", mr_section_label(kind).to_lowercase());
    let busy = tab.forge.refreshing.contains(&Refreshing::List);
    let glyph = if busy { turning("refresh-cw", 12., theme::text_secondary(), "mr-list-refresh") } else { icon("refresh-cw", 12., theme::text_secondary()).into_any_element() };
    out.push(
        tip(
            label,
            crate::ui::anchor(
                "mr-refresh-list",
                head_button("mr-refresh-list", false)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.forge_refresh_list_now(id, cx);
                    }))
                    .child(glyph),
            ),
        )
        .into_any_element(),
    );
    let filter = tab.forge.filter;
    let current = FILTERS.iter().find(|x| x.0 == filter).map_or("All", |x| x.2);
    let label = format!("Filter {}: {current}", mr_section_label(kind).to_lowercase());
    let key = "mr-filter";
    out.push(
        tip(
            label,
            crate::ui::anchor(
                key,
                head_button(key, filter != MrFilter::All)
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        let rows = this.mr_filter_rows(id, &cx.weak_entity());
                        menu::open_below(key, rows, Default::default(), window, cx);
                    }))
                    .child(icon("list-filter", 13., if filter == MrFilter::All { theme::text_secondary() } else { theme::text_selected() })),
            ),
        )
        .into_any_element(),
    );
    out
}

impl GitVolt {
    fn mr_filter_rows(&self, tab: u32, weak: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let filter = self.tabs.iter().find(|t| t.id == tab).map_or(MrFilter::All, |t| t.forge.filter);
        FILTERS
            .iter()
            .map(|(v, name, label, tooltip)| {
                let (w, v, name) = (weak.clone(), *v, *name);
                MenuRow::action(format!("mrFilter.{name}"), *label, if v == filter { "check" } else { "" }, *tooltip, move |_, cx| {
                    w.update(cx, |this, cx| this.mr_set_filter(tab, v, name, cx)).ok();
                })
            })
            .collect()
    }

    fn mr_set_filter(&mut self, tab: u32, v: MrFilter, name: &'static str, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        t.forge.filter = v;
        let path = t.repo_path.clone();
        self.update_repo(&path, move |r| r["mrFilter"] = serde_json::json!(name), cx);
        self.forge_refresh_list(tab, cx);
        self.forge_list_changed(tab, cx);
    }
}
