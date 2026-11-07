//! The view's header (`MrHeader.tsx`) and where it goes (`BranchFlow.tsx`, `rangeStats.ts`): the status line, the branch card with what it brings
//! and its commits, the facts, and the people.

use super::{Mode, MrUi, RangeState};
use crate::app::{GitVolt, Tab, core_repo, icon};
use crate::forge::card::{forge_avatar, state_chip};
use crate::forge::section::pipeline_icon;
use crate::forge::text::{owner_of, pipeline_word, review_text};
use crate::theme;
use crate::ui::tooltip::tip;
use gitvolt_core::api::Request;
use gitvolt_core::forge::types::{ForgeMr, ForgeMrDetail, MrState, PipelineStatus, ReviewDecision};
use gitvolt_core::forge::ForgeKind;
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};

/// A remote branch's tip, as the sidebar has it.
pub fn remote_tip(t: &Tab, remote: &str, branch: &str) -> Option<String> {
    t.sidebar["remotes"].as_array()?.iter().find(|g| g["name"].as_str() == Some(remote))?["branches"].as_array()?.iter().find(|b| {
        let n = b["name"].as_str().unwrap_or_default();
        n == branch || n.strip_prefix(remote).and_then(|x| x.strip_prefix('/')) == Some(branch)
    })?["target"].as_str().map(str::to_string)
}

/// `.mr-button`.
pub fn mr_button(id: impl Into<ElementId>, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .px(px(10.))
        .py(px(3.))
        .rounded(px(4.))
        .border_1()
        .border_color(theme::section_border())
        .bg(theme::control_bg())
        .text_size(px(12.))
        .text_color(if enabled { theme::text_normal() } else { theme::text_disabled() })
        .whitespace_nowrap()
        .when(enabled, |d| d.cursor_pointer().hover(|d| d.bg(theme::control_hover_bg())))
}

/// `.mr-button.primary`.
pub fn mr_primary(id: impl Into<ElementId>, enabled: bool) -> Stateful<Div> {
    // The web's disabled primary stays blue: `.mr-button.primary`'s colours outrank `:disabled`'s.
    mr_button(id, enabled).bg(theme::button_primary()).border_color(theme::button_primary()).text_color(theme::text_on_accent()).when(enabled, |d| d.hover(|d| d.bg(theme::button_primary())))
}

/// `.card-btn`.
pub fn card_btn(id: impl Into<ElementId>) -> Stateful<Div> {
    div().id(id).flex().flex_none().items_center().justify_center().h(px(22.)).min_w(px(22.)).rounded(px(4.)).border_1().border_color(theme::section_border()).bg(theme::control_bg()).cursor_pointer().hover(|d| d.bg(theme::control_hover_bg()))
}

/// `.mr-fact`: a card's box.
pub fn fact(id: &'static str) -> Stateful<Div> {
    div().id(id).flex().flex_col().flex_1().min_w_0().min_h(px(52.)).pt(px(6.)).pb(px(8.)).px(px(10.)).rounded(px(6.)).border_1().border_color(theme::section_border()).bg(theme::panel_bg1())
}

/// A card's uppercase name.
pub fn fact_k(text: &str) -> Div {
    div().flex_1().min_w_0().truncate().min_h(px(22.)).line_height(px(22.)).font_features(crate::glyphs::spacing(0.04)).text_size(px(11.)).text_color(theme::text_secondary()).child(text.to_uppercase())
}

fn fact_v(color: Hsla) -> Div {
    div().flex().flex_1().items_center().gap(px(6.)).pt(px(4.)).text_size(px(12.)).text_color(color)
}

impl GitVolt {
    /// What the branch brings: local reads only (`mergeBase`, then the compare's file list; the commits come from the loaded graph). Read
    /// again when the tip, the base or the tab's commits change.
    pub fn mr_recount(&mut self, tab: u32, number: u64, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some(mr) = t.forge.details.get(&number).map(|d| &d.mr).or_else(|| t.forge.known_mr(number)) else { return };
        let open = matches!(mr.state, MrState::Open | MrState::Draft);
        let tip = if open { mr.head_sha.clone() } else { None };
        let base_ref = t.forge.remote.as_ref().filter(|_| open).map(|r| format!("refs/remotes/{r}/{}", mr.target_branch));
        let print = t.graph.as_ref().map_or(0, |g| g.rows.len() as u64);
        let (Some(tip), Some(base_ref)) = (tip, base_ref) else {
            if let Some(ui) = self.mr_ui(tab, number) {
                ui.stats = RangeState::None;
                ui.stats_for = None;
            }
            return;
        };
        let key = (tip.clone(), base_ref.clone(), print);
        let Some(ui) = self.mr_ui(tab, number) else { return };
        if ui.stats_for.as_ref() == Some(&key) {
            return;
        }
        ui.stats_for = Some(key);
        if ui.stats == RangeState::None {
            ui.stats = RangeState::Loading;
        }
        let (api, rt, repo) = (self.api.clone(), self.rt.clone(), core_repo(tab));
        let t2 = tip.clone();
        let job = rt.spawn(async move {
            let base = api.dispatch(Request::MergeBase { repo, a: base_ref, b: t2.clone() }).await.ok()?;
            let base = base.as_str()?.to_string();
            let spec: gitvolt_core::diff::DiffSpec = serde_json::from_value(json!({ "kind": "compare", "from": base, "to": t2 })).ok()?;
            let list = api.dispatch(Request::FileList { repo, spec }).await.ok()?;
            Some((base, list))
        });
        cx.spawn(async move |this, cx| {
            let answer = job.await.ok().flatten();
            this.update(cx, |v, cx| {
                let Some(t) = v.tabs.iter().find(|t| t.id == tab) else { return };
                let state = match answer {
                    None => RangeState::None,
                    Some((base, list)) => {
                        let commits = t.graph.as_ref().and_then(|g| commits_between(g.rows.iter().filter(|r| r.wip.is_none()).map(|r| (r.id.as_ref(), r.summary.as_ref(), r.parents.first().map(String::as_str))), &tip, &base));
                        RangeState::Ready { commits, files: list["files"].as_array().map_or(0, |f| f.len() as u64), added: list["added"].as_u64().unwrap_or(0), deleted: list["deleted"].as_u64().unwrap_or(0) }
                    }
                };
                if let Some(ui) = v.mr_ui(tab, number) {
                    ui.rechecking = false;
                    ui.missed = state == RangeState::None;
                    ui.stats = state;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

const MAX_WALK: usize = 1000;

/// The commits on `tip`'s first-parent line down to `base` (excluded), oldest first; `None` when the walk leaves what's loaded or never meets it.
pub fn commits_between<'a>(rows: impl Iterator<Item = (&'a str, &'a str, Option<&'a str>)>, tip: &str, base: &str) -> Option<Vec<(String, String)>> {
    if tip == base {
        return Some(Vec::new());
    }
    let by: std::collections::HashMap<&str, (&str, Option<&str>)> = rows.map(|(id, s, p)| (id, (s, p))).collect();
    let mut out = Vec::new();
    let mut at = Some(tip);
    while let Some(id) = at.filter(|i| *i != base) {
        let (summary, parent) = by.get(id)?;
        if out.len() >= MAX_WALK {
            return None;
        }
        out.push((id.to_string(), summary.to_string()));
        at = *parent;
    }
    if at == Some(base) {
        out.reverse();
        Some(out)
    } else {
        None
    }
}

/// `BranchFlow`, without Create's parts (`create.rs` draws its own ends).
pub fn flow_ends(from: (String, String), into: (String, String)) -> Div {
    let end = |caption: &'static str, name: String, sub: String| {
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .flex_1()
            .min_w_0()
            .px(px(12.))
            .py(px(10.))
            .child(div().font_features(crate::glyphs::spacing(0.06)).text_size(px(10.5)).text_color(theme::text_disabled()).child(caption.to_uppercase()))
            .child(tip(name.clone(), div().min_w_0().truncate().font_family(crate::app::MONO_FONT).text_size(px(12.5)).text_color(theme::text_selected()).child(name)))
            .child(div().min_w_0().truncate().text_size(px(12.)).text_color(theme::text_secondary()).child(sub))
    };
    div().flex().items_stretch().child(end("From", from.0, from.1)).child(div().flex().flex_none().w(px(28.)).items_center().justify_center().child(icon("arrow-right", 16., theme::text_disabled()))).child(end("Into", into.0, into.1))
}

fn branch_card(this: &mut GitVolt, tab: u32, kind: ForgeKind, mr: &ForgeMr, detail: Option<&ForgeMrDetail>, ui: &MrUi, cx: &mut Context<GitVolt>) -> Stateful<Div> {
    let open = matches!(mr.state, MrState::Open | MrState::Draft);
    let number = mr.number;
    let from_fork = !mr.source_project.is_empty() && mr.source_project != mr.target_project;
    let none = match mr.state {
        MrState::Merged => format!("Merged into {}", mr.target_branch),
        MrState::Closed => "Closed".to_string(),
        _ => "Its commits aren't fetched into this repository".to_string(),
    };
    let in_graph = this.tabs.iter().find(|t| t.id == tab).and_then(|t| t.graph.clone());
    let counts = |d: Div| match &ui.stats {
        RangeState::Loading => d.child(div().text_color(theme::text_disabled()).child("Counting…")),
        RangeState::None => d.child(div().text_color(theme::text_disabled()).child(none.clone())),
        RangeState::Ready { commits, files, added, deleted } => {
            let bold = |n: String, word: &str| div().flex().gap(px(3.)).child(div().font_weight(FontWeight::SEMIBOLD).text_color(theme::text_normal()).child(n)).child(word.to_string());
            d.when_some(commits.as_ref(), |d, c| d.child(bold(c.len().to_string(), if c.len() == 1 { "commit" } else { "commits" })))
                .child(bold(files.to_string(), if *files == 1 { "file" } else { "files" }))
                .child(div().text_color(theme::green()).child(format!("+{added}")))
                .child(div().text_color(theme::red()).child(format!("−{deleted}")))
        }
    };
    let has_commits = matches!(&ui.stats, RangeState::Ready { commits: Some(c), .. } if !c.is_empty());
    let meta = div().flex().flex_1().min_w_0().items_center().gap(px(12.)).min_h(px(31.)).px(px(12.)).py(px(6.)).text_size(px(12.)).text_color(theme::text_secondary());
    let meta_el: AnyElement = if has_commits {
        counts(meta)
            .child(div().flex().ml_auto().items_center().gap(px(4.)).text_color(theme::text_secondary()).child(if ui.commits_open { "Hide commits" } else { "Show commits" }).child(icon(if ui.commits_open { "chevron-up" } else { "chevron-down" }, 12., theme::text_secondary())))
            .id("mr-flow-toggle")
            .cursor_pointer()
            .hover(|d| d.bg(theme::control_hover_bg()))
            .on_click(cx.listener(move |v, _, _, cx| {
                if let Some(ui) = v.mr_ui(tab, number) {
                    ui.commits_open = !ui.commits_open;
                }
                cx.notify();
            }))
            .into_any_element()
    } else if matches!(ui.stats, RangeState::None) && open {
        let text = if ui.missed { "Still not in this repository. Fetching the source branch brings them in." } else { "Check again" };
        tip(text, counts(meta).id("mr-flow-recheck").cursor_pointer().when(ui.rechecking, |d| d.child(crate::commit_box::spinner("mr-flow-spin", 12., theme::text_disabled()))).on_click(cx.listener(move |v, _, _, cx| {
            if let Some(ui) = v.mr_ui(tab, number) {
                ui.rechecking = true;
                ui.stats_for = None;
            }
            v.mr_recount(tab, number, cx);
        })))
        .into_any_element()
    } else {
        counts(meta).into_any_element()
    };
    let has_head = detail.and_then(|d| d.mr.head_sha.as_ref()).or(mr.head_sha.as_ref()).is_some();
    let comparing = ui.comparing;
    let (m2, d2) = (mr.clone(), detail.cloned());
    let compare = has_head.then(|| {
        let glyph: AnyElement = if comparing { crate::commit_box::spinner("mr-compare-spin", 14., theme::text_secondary()).into_any_element() } else { icon("git-compare-arrows", 14., theme::text_secondary()).into_any_element() };
        tip(
            "Compare the MR's changes in the diff view",
            crate::ui::anchor(
                "mr-compare",
                div()
                    .id("mr-compare")
                    .flex()
                    .flex_none()
                    .w(px(34.))
                    .items_center()
                    .justify_center()
                    .border_l_1()
                    .border_color(theme::section_border())
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::control_hover_bg()))
                    .on_click(cx.listener(move |v, _, _, cx| {
                        if v.mr_ui(tab, number).is_some_and(|u| u.comparing) {
                            return;
                        }
                        v.mr_comparing(tab, number, true);
                        v.mr_compare(tab, kind, m2.clone(), d2.clone(), cx);
                        cx.notify();
                    }))
                    .child(glyph),
            ),
        )
    });
    let commit_list = (ui.commits_open).then(|| {
        let commits = if let RangeState::Ready { commits: Some(c), .. } = &ui.stats { c.clone() } else { Vec::new() };
        let mut list = div().id("mr-commits").flex().flex_col().py(px(4.)).max_h(px(200.)).overflow_y_scroll().border_t_1().border_color(theme::section_border());
        for (i, (sha, summary)) in commits.into_iter().enumerate() {
            let shown = in_graph.as_ref().is_some_and(|g| g.by_sha.contains_key(&sha));
            let short = sha.chars().take(7).collect::<String>();
            let body = div().flex().items_baseline().gap(px(8.)).w_full().px(px(12.)).py(px(3.)).text_size(px(12.)).text_color(if shown { theme::text_normal() } else { theme::text_secondary() }).child(div().flex_none().font_family(crate::app::MONO_FONT).text_size(px(11.)).text_color(theme::text_disabled()).child(short.clone())).child(div().min_w_0().truncate().child(summary));
            list = list.child(if shown {
                tip(format!("Show {short} in the graph"), body.id(("mr-commit", i)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).on_click(cx.listener(move |v, _, _, cx| v.go_to_sha(&sha, cx)))).into_any_element()
            } else {
                tip("Not in the graph yet: fetch first", body).into_any_element()
            });
        }
        list
    });
    div()
        .id("mr-flow")
        .flex_none()
        .rounded(px(8.))
        .border_1()
        .border_color(theme::section_border())
        .bg(theme::panel_bg1())
        .overflow_hidden()
        .child(flow_ends((mr.source_branch.clone(), if from_fork { owner_of(&mr.source_project).to_string() } else { mr.target_project.clone() }), (mr.target_branch.clone(), mr.target_project.clone())))
        .child(div().flex().items_stretch().border_t_1().border_color(theme::section_border()).bg(theme::panel_bg()).child(meta_el).children(compare))
        .children(commit_list)
}

/// The header's cards share a grid of `minmax(150px, 1fr)`: as many in a row as fit, each the same width.
fn card_grid(width: f32, cards: Vec<AnyElement>) -> Div {
    let per_row = (((width + 8.) / 158.).floor() as usize).clamp(1, 6);
    let mut col = div().flex().flex_col().gap(px(8.));
    let mut it = cards.into_iter().peekable();
    while it.peek().is_some() {
        let mut row = div().flex().items_stretch().gap(px(8.));
        let mut n = 0;
        for _ in 0..per_row {
            if let Some(c) = it.next() {
                row = row.child(div().flex().flex_1().flex_basis(px(0.)).min_w_0().child(c));
                n += 1;
            }
        }
        // `auto-fit` collapses empty tracks, so a short row stretches too.
        let _ = n;
        col = col.child(row);
    }
    col
}

pub fn mr_header(this: &mut GitVolt, tab: u32, kind: ForgeKind, mr: &ForgeMr, detail: Option<&ForgeMrDetail>, window: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let number = mr.number;
    this.mr_recount(tab, number, cx);
    let Some(ui) = this.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.mr_views.remove(&number)) else { return div().into_any_element() };
    let mode = ui.mode;
    let flow = branch_card(this, tab, kind, mr, detail, &ui, cx);
    if let Some(t) = this.tabs.iter_mut().find(|t| t.id == tab) {
        t.mr_views.insert(number, ui);
    }
    let live = matches!(mr.state, MrState::Open | MrState::Draft);
    let editing = live && mode == Mode::Edit;
    let actions = super::actions::status_actions(this, tab, kind, mr, detail, cx);
    let when = crate::format::relative_time(mr.updated_at, crate::format::now());
    let line = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .text_color(theme::text_secondary())
        .child(state_chip(mr.state, 12., 9., 2.))
        .child(forge_avatar(&mr.author, 20.))
        .child(div().text_color(theme::text_normal()).child(mr.author.name.clone()))
        .child(super::thread::rel_time(mr.updated_at, format!("updated {when}"), theme::text_secondary(), 11., this))
        .child(div().flex().flex_none().ml_auto().items_center().gap(px(6.)).children(actions));
    let mut out = div().flex().flex_col().gap(px(8.)).child(line).child(flow);
    if !editing {
        let pipe_color = match mr.pipeline.as_ref().map(|p| p.status) {
            Some(PipelineStatus::Success) => theme::green(),
            Some(PipelineStatus::Failed) => theme::red(),
            Some(PipelineStatus::Pending | PipelineStatus::Running) => theme::orange(),
            _ => theme::text_normal(),
        };
        let word = pipeline_word(mr.pipeline.as_ref());
        let pipe_url = mr.pipeline.as_ref().and_then(|p| p.web_url.clone());
        let pipeline = fact("mr-fact-pipeline")
            .child(fact_k(if kind == ForgeKind::GitLab { "Pipeline" } else { "Checks" }))
            .child(fact_v(pipe_color).children(mr.pipeline.as_ref().map(|p| pipeline_icon(p, 12.))).child(match pipe_url {
                Some(url) => div().id("mr-pipeline-link").cursor_pointer().text_color(theme::link()).hover(|d| d.underline()).child(word).on_click(cx.listener(move |v, _, _, _| v.forge_open_url(url.clone()))).into_any_element(),
                None => div().child(word).into_any_element(),
            }));
        let tone = match detail.map(|d| d.mr.review.decision) {
            Some(ReviewDecision::Approved) => theme::green(),
            Some(ReviewDecision::ChangesRequested) => theme::red(),
            _ => theme::text_normal(),
        };
        let reviews = crate::ui::anchor(
            "mr-fact-reviews",
            fact("mr-fact-reviews")
                .child(div().flex().items_center().gap(px(6.)).child(fact_k(if kind == ForgeKind::GitLab { "Approvals" } else { "Reviews" })).children(super::actions::review_buttons(this, tab, kind, mr, detail, cx)))
                .child(fact_v(tone).child(crate::ui::hanging(div().min_w_0()).child(detail.map_or("Loading…".to_string(), |d| review_text(&d.mr.review))))),
        );
        let (conflict_text, conflict_color) = match mr.conflicts {
            None => ("Checking…", theme::text_secondary()),
            Some(true) => ("Yes", theme::red()),
            Some(false) => ("None", theme::green()),
        };
        let conflicts = fact("mr-fact-conflicts").child(fact_k("Conflicts")).child(fact_v(conflict_color).child(div().min_w_0().child(conflict_text)));
        let mut cards = vec![pipeline.into_any_element(), reviews.into_any_element(), conflicts.into_any_element()];
        cards.extend(super::people::people_cards(this, tab, kind, mr, detail, cx));
        let width = this.mr_body_width(window);
        out = out.child(card_grid(width, cards));
    }
    out.into_any_element()
}

impl GitVolt {
    /// The view's content width, what the header's grid goes by.
    pub fn mr_body_width(&self, window: &Window) -> f32 {
        let room = (self.window_width.max(window.viewport_size().width.as_f32()) - self.sidebar_w).max(0.);
        let w = match self.mr_dock_width(room) {
            Some(w) => w,
            None => super::flyout_width(self.profile["flyoutWidth"].as_f64().map(|w| w as f32), room),
        };
        (w - 25.).max(100.)
    }
}

/// `Value` of what the core's compare lists.
pub type _Unused = Value;
