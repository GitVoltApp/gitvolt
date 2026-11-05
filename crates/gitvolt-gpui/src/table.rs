//! The graph table (`ui/src/graph/GraphView.tsx`, `columns.ts`, `RefLabels.tsx`, `rowDim.ts`): the
//! header with its resizers and its right-click menu, the rows with their chips, the WIP row and the
//! Load older commits row.

use crate::app::{GitVolt, Tab, icon};
use crate::graph::{RowGeometry, paint_row};
use crate::model::*;
use crate::theme;
use crate::ui::menu::{self, MenuRow};
use crate::ui::tooltip::{Anchor, Placement, tip};
use gitvolt_core::api::Request;
use gitvolt_graph::NodeKind;
use gpui::prelude::*;
use gpui::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

pub const HEADER_H: f32 = 25.;
pub const SCROLLBAR_W: f32 = 10.;
const SHA_PAD: f32 = 6.;
const PIN_FULL_MIN: f32 = 130.;
/// How long the pointer rests on a chip before its branch is focused.
const BRANCH_FOCUS_DELAY: Duration = Duration::from_millis(500);
const MESSAGE_TIP_DELAY: u64 = 500;
const CHIP_PAD: f32 = 10.;
const CHIP_GAP: f32 = 3.;
const CHIP_SPACING: f32 = 4.;
const CONNECTOR_MIN: f32 = 8.;
const ICON: f32 = 14.;
const HEAD_CHECK: f32 = 16.;
const WIP_DRAFT_H: f32 = 22.;

/// A column's edge being dragged.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnDrag {
    pub col: Col,
    pub x: f32,
    pub start: Columns,
    pub prefs: ColumnPrefs,
    pub available: f32,
    pub graph_max: f32,
}

/// Commit messages for the row's tooltip, by id; `None`: on its way.
pub type MessageCache = Rc<RefCell<HashMap<String, Option<SharedString>>>>;

fn chip_width(c: &Chip, cx: &App) -> f32 {
    let sources = usize::from(c.local) + c.remotes.len();
    let icons = usize::from(c.head) + usize::from(c.tag) + sources;
    let icon_w = if c.head { HEAD_CHECK } else { 0. } + if c.tag { 12. } else { 0. } + sources as f32 * ICON;
    CHIP_PAD + crate::ui::text_width(&c.name, 13., FontWeight::NORMAL, cx) + icon_w + icons as f32 * CHIP_GAP
}

/// `fitCount`: how many chips fit, a `+N` badge for the rest.
pub fn fit_count(widths: &[f32], avail: f32, more: impl Fn(usize) -> f32) -> usize {
    let n = widths.len();
    if n <= 1 {
        return n.min(1);
    }
    let room = avail - CONNECTOR_MIN;
    let mut sum = 0.;
    let mut cum = Vec::with_capacity(n);
    for (i, w) in widths.iter().enumerate() {
        sum += w + if i > 0 { CHIP_SPACING } else { 0. };
        cum.push(sum);
    }
    if sum <= room {
        return n;
    }
    for k in (2..n).rev() {
        if cum[k - 1] + more(n - k) <= room {
            return k;
        }
    }
    1
}

/// Where a row's chips end, for the connector to start: what fits, and the `+N` for the rest.
fn chips_end(row: &RowView, width: f32, cx: &App) -> f32 {
    let widths: Vec<f32> = row.chips.iter().map(|c| chip_width(c, cx)).collect();
    let more_w = |n: usize| crate::ui::text_width(&format!("+{n}"), 11., FontWeight::NORMAL, cx) + 8. + CHIP_SPACING;
    let shown = fit_count(&widths, width - CELL_PAD, more_w);
    let hidden = widths.len().saturating_sub(shown);
    let sum: f32 = widths.iter().take(shown).sum::<f32>() + shown.saturating_sub(1) as f32 * CHIP_SPACING;
    (CELL_PAD + sum + if hidden > 0 { more_w(hidden) } else { 0. }).min(width)
}

/// A chip. `row`: its row's group, whose hover lights it as selecting the row does.
fn chip_inner(c: &Chip, lane: usize, full: bool, lit: bool, row: Option<SharedString>) -> Div {
    let tint = theme::lane_alpha(lane, if c.head || lit { 0.45 } else { 0.25 });
    let text = if c.head || lit { theme::text_selected() } else { theme::text_normal() };
    let local_name = c.name.clone();
    let mut chip = div()
        .flex()
        .when(!full, |d| d.min_w_0())
        .items_center()
        .gap(px(CHIP_GAP))
        .h(px(CHIP_H))
        .px(px(5.))
        .rounded(px(3.))
        .bg(tint)
        .text_color(text)
        .when_some(row, |d, group| d.group_hover(group, move |d| d.bg(theme::lane_alpha(lane, 0.45)).text_color(theme::text_selected())))
        .when(c.head, |d| d.child(icon("check", HEAD_CHECK, text)))
        .when(c.tag, |d| d.child(icon("tag", 12., text)))
        .child(div().when(!full, |d| d.min_w_0().truncate()).child(c.name.clone()));
    if c.local {
        chip = chip.child(tip(format!("{local_name} (Local)"), icon("laptop", ICON, text)));
    }
    for (remote, full_ref) in &c.remotes {
        let branch = full_ref.trim_start_matches("refs/remotes/").trim_start_matches(&format!("{remote}/")).to_string();
        chip = chip.child(tip(format!("{remote} → {branch} (Remote)"), icon("cloud", ICON, text)));
    }
    chip
}

/// The labels cell: what fits, `+N` for the rest with their list on hover; each chip its own menu and
/// its branch's focus.
#[allow(clippy::too_many_arguments)]
/// The branch a rebase is replaying, at HEAD's row while its worktree rebases: a ring and the name, dashed.
fn rebasing_chip(name: &str, lane: usize) -> impl IntoElement + use<> {
    let color = theme::lane(lane);
    tip(
        format!("{name} is being rebased"),
        div().flex().flex_none().items_center().gap(px(CHIP_GAP)).h(px(CHIP_H)).px(px(5.)).rounded(px(3.)).bg(theme::lane_alpha(lane, 0.45)).border_1().border_dashed().border_color(color).text_color(theme::text_selected()).child(icon("loader-circle", 12., theme::text_selected())).child(name.to_string()),
    )
}

#[allow(clippy::too_many_arguments)]
fn chips_cell(row: &RowView, ix: usize, width: f32, selected: bool, group: &SharedString, row_pending: &[&str], rebasing: Option<&str>, member: Option<std::sync::Arc<Membership>>, this: &WeakEntity<GitVolt>, cx: &mut Context<GitVolt>) -> Stateful<Div> {
    let lane = row.color as usize;
    // A detached HEAD sharing the cell with the rebasing chip is just its check.
    let chips: Vec<Chip> = row.chips.iter().map(|c| if rebasing.is_some() && c.head && c.name.as_ref() == "HEAD" { Chip { name: SharedString::default(), ..c.clone() } } else { c.clone() }).collect();
    let widths: Vec<f32> = chips.iter().map(|c| chip_width(c, cx)).collect();
    let more_w = |n: usize| crate::ui::text_width(&format!("+{n}"), 11., FontWeight::NORMAL, cx) + 8. + CHIP_SPACING;
    let room = rebasing.map_or(0., |n| crate::ui::text_width(n, 13., FontWeight::NORMAL, cx) + CHIP_PAD + 12. + CHIP_GAP + CHIP_SPACING + 2.);
    let shown = fit_count(&widths, width - CELL_PAD - room, more_w);
    let hidden = chips.len().saturating_sub(shown);
    let sha = row.id.to_string();
    // Only the chips select the row: a press on the space around them stops here.
    let mut cell = div().id(("labels", ix)).flex().flex_none().w(px(width)).h(px(ROW_H)).items_center().gap(px(CHIP_SPACING)).pl(px(CELL_PAD)).overflow_hidden().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).children(rebasing.map(|n| rebasing_chip(n, lane)));
    for (i, c) in chips.iter().take(shown).enumerate() {
        let (chip, menu_sha, weak, hover) = (c.clone(), sha.clone(), this.clone(), this.clone());
        let (press_chip, press_sha) = (c.clone(), sha.clone());
        let refs = chip_refs(c);
        let tag_tip = c.tag.then(|| format!("Tag at {}", &sha[..sha.len().min(SHORT_SHA)]));
        let pending = row_pending.iter().any(|p| c_refs(c).iter().any(|r| r == *p));
        let body = chip_inner(c, lane, false, selected, Some(group.clone()))
            .when(pending, |d| d.child(crate::commit_box::spinner(("chip-spin", ix * 64 + i), 11., theme::text_secondary())))
            .id(("chip", ix * 64 + i))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                // The press selects the row; a double click checks it out; a drag takes a local branch onto another.
                cx.stop_propagation();
                if e.click_count == 2 {
                    this.checkout_chip(&press_sha, &press_chip, cx);
                } else {
                    this.start_branch_drag(ix, press_chip.clone(), e.position);
                    this.select_row(ix, e.modifiers.into(), cx);
                }
            }))
            .on_hover(move |entered, _, cx| {
                hover
                    .update(cx, |v, cx| {
                        if !refs.is_empty() {
                            v.hover_branch(entered.then(|| refs.clone()), cx);
                        }
                    })
                    .ok();
            })
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                let rows = if chip.tag { this.tag_rows(&chip.name, Some(&menu_sha), false, &weak) } else { this.commit_rows(ix, Some(&chip), &weak) };
                menu::open_context_for(format!("row:{ix}"), rows, e.position, window, cx);
            }));
        let key = format!("chip:{ix}:{}", c.name);
        cell = cell.child(match tag_tip {
            Some(t) => crate::ui::anchor(key, body).tip(t).into_any_element(),
            None => crate::ui::anchor(key, body).into_any_element(),
        });
    }
    if hidden > 0 {
        let list: Vec<Chip> = chips.clone();
        let tint = theme::lane_alpha(lane, 0.25);
        let (weak, menu_list, menu_sha) = (this.clone(), list.clone(), sha);
        let badge = div()
            .id(("chip-more", ix))
            .flex_none()
            .px(px(4.))
            .rounded(px(3.))
            .bg(tint)
            .cursor_pointer()
            .text_size(px(11.))
            .text_color(theme::text_normal())
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.select_row(ix, e.modifiers.into(), cx);
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                let rows = menu_list
                    .iter()
                    .enumerate()
                    .map(|(k, c)| {
                        let sub = if c.tag { this.tag_rows(&c.name, Some(&menu_sha), false, &weak) } else { this.commit_rows(ix, Some(c), &weak) };
                        MenuRow::submenu(format!("chip:{k}"), c.name.clone(), if c.tag { "tag" } else { "git-branch" }, "", sub)
                    })
                    .collect();
                menu::open_context_for(format!("row:{ix}"), rows, e.position, window, cx);
            }))
            .child(format!("+{hidden}"));
        cell = cell.child(Anchor::new(badge).card(format!("chips:{ix}"), move |_, _| {
            let mut stack = div().flex().flex_col().gap(px(3.)).p(px(3.)).bg(theme::panel_bg1()).border_1().border_color(theme::section_border()).rounded(px(4.)).shadow(crate::ui::shadow(theme::shadow_popup(), 6., 20.)).text_size(px(13.));
            for c in &list {
                stack = stack.child(chip_inner(c, lane, true, false, None));
            }
            stack.into_any_element()
        }));
    }
    if let Some(m) = member {
        let used = chips_end(row, width, cx);
        let w = dim_chip_width(&m, cx);
        if row.chips.is_empty() || used + CHIP_SPACING + w <= width {
            cell = cell.child(dim_chip(&m, ix, row.chips.is_empty(), this, cx));
        }
    }
    cell
}

fn dim_chip_width(m: &Membership, cx: &App) -> f32 {
    CHIP_PAD + crate::ui::text_width(&m.name, 13., FontWeight::NORMAL, cx)
}

/// The dimmed branch-membership chip: the lit chip look at half opacity, just the branch's name, no tooltip. Hovered, it's
/// at full strength; a press selects the row.
fn dim_chip(m: &Membership, ix: usize, alone: bool, this: &WeakEntity<GitVolt>, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let lane = m.color as usize;
    let hover = this.clone();
    let refs = vec![m.full_ref.clone()];
    div()
        .id(("chip-dim", ix))
        .flex()
        .items_center()
        .when(alone, |d| d.min_w_0())
        .when(!alone, |d| d.flex_none())
        .h(px(CHIP_H))
        .px(px(5.))
        .rounded(px(3.))
        .bg(theme::lane_alpha(lane, 0.45))
        .text_color(theme::text_selected())
        .opacity(0.5)
        .hover(|d| d.opacity(1.))
        .cursor_pointer()
        .on_hover(move |entered, _, cx| {
            hover.update(cx, |v, cx| v.hover_branch(entered.then(|| refs.clone()), cx)).ok();
        })
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            this.select_row(ix, e.modifiers.into(), cx);
        }))
        .child(div().min_w_0().truncate().child(m.name.clone()))
}

fn status_count(glyph: &'static str, color: Hsla, n: u32) -> Option<Div> {
    (n > 0).then(|| div().flex().flex_none().items_center().gap(px(2.)).text_size(px(11.)).text_color(theme::text_secondary()).child(icon(glyph, 12., color)).child(n.to_string()))
}

/// The WIP row's message: its draft box, the worktree's name, and the counts.
fn wip_cell(draft: Option<&Entity<crate::ui::input::TextInput>>, w: &WipCounts, width: f32, focused: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .w(px(width))
        .h(px(ROW_H))
        .items_center()
        .gap(px(8.))
        .px(px(CELL_PAD))
        .overflow_hidden()
        .child(
            div()
                .flex()
                .flex_none()
                .w(px(116.))
                .h(px(WIP_DRAFT_H))
                .items_center()
                .px(px(8.))
                .bg(theme::wip_input_bg())
                .border_1()
                .border_color(if focused { theme::blue() } else { theme::wip_input_border() })
                .rounded(px(4.))
                .text_color(theme::text_normal())
                .children(draft.cloned()),
        )
        .when_some(w.name.clone(), |d, n| d.child(div().min_w_0().truncate().text_color(theme::text_secondary()).child(n)))
        .children(status_count("status-modified", theme::status_modified(), w.modified))
        .children(status_count("status-added", theme::status_added(), w.added))
        .children(status_count("status-deleted", theme::status_deleted(), w.deleted))
        .children(status_count("status-renamed", theme::blue(), w.renamed))
        .children(status_count("status-conflicted", theme::status_conflicted(), w.conflicted))
}

/// A chip's full ref names.
fn c_refs(c: &Chip) -> Vec<String> {
    let mut out: Vec<String> = c.remotes.iter().map(|r| r.1.clone()).collect();
    if c.local {
        out.push(format!("refs/heads/{}", c.name));
    }
    out
}

/// A number with thousands separators.
pub fn with_commas(n: u32) -> String {
    let text = n.to_string();
    let mut out = String::new();
    for (i, c) in text.chars().enumerate() {
        if i > 0 && (text.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl GitVolt {
    /// A chip hovered: after a rest, every row outside its branch dims; leaving it undims at once.
    pub fn hover_branch(&mut self, refs: Option<Vec<String>>, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        tab.branch_hover = tab.branch_hover.wrapping_add(1);
        let (generation, repo) = (tab.branch_hover, tab.id);
        let Some(refs) = refs else {
            if tab.branch_keep.take().is_some() {
                cx.notify();
            }
            return;
        };
        let timer = cx.background_executor().timer(BRANCH_FOCUS_DELAY);
        cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |this, cx| {
                let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo).filter(|t| t.branch_hover == generation) else { return };
                let Some(graph) = tab.graph.clone() else { return };
                tab.branch_keep = Some(Rc::new(focus_rows(&graph, &refs)));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Asks for a row's whole message, for its tooltip.
    fn read_message(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(repo) = self.tabs.get(self.active).map(|t| t.id) else { return };
        if cfg!(test) {
            return;
        }
        let api = self.api.clone();
        let at = id.clone();
        let done = self.rt.spawn(async move { api.dispatch(Request::CommitMessage { repo, id: at }).await });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(v)) = done.await else { return };
            let (summary, body) = (v["summary"].as_str().unwrap_or_default(), v["body"].as_str().unwrap_or_default().trim());
            let text: SharedString = if body.is_empty() { summary.to_string().into() } else { format!("{summary}\n\n{body}").into() };
            this.update(cx, |this, cx| {
                this.messages.borrow_mut().insert(id, Some(text));
                cx.refresh_windows();
            })
            .ok();
        })
        .detach();
    }

    pub fn on_column_drag(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag) = self.column_drag.clone() else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            return self.end_column_drag(cx);
        }
        let prefs = resize_column(drag.col, &drag.start, &drag.prefs, e.position.x.as_f32() - drag.x, drag.available, drag.graph_max);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.column_prefs = prefs;
            cx.notify();
        }
    }

    pub fn end_column_drag(&mut self, cx: &mut Context<Self>) {
        if self.column_drag.take().is_some() {
            self.save_layout();
            cx.notify();
        }
    }

    fn reset_column(&mut self, col: Col, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let d = ColumnPrefs::default();
        match col {
            Col::Labels => tab.column_prefs.labels = d.labels,
            Col::Graph => tab.column_prefs.graph = None,
            Col::Message => tab.column_prefs.message = None,
            Col::Author => tab.column_prefs.author = d.author,
            Col::Date => tab.column_prefs.date = d.date,
            Col::Sha => tab.column_prefs.sha = d.sha,
        }
        self.save_layout();
        cx.notify();
    }

    fn toggle_column(&mut self, col: Col, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        if !tab.hidden_columns.remove(&col) {
            tab.hidden_columns.insert(col);
        }
        self.save_layout();
        cx.notify();
    }

    /// `column` menu: every column but Graph and Message can be hidden, per repository.
    fn column_rows(&self, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let hidden = self.tabs.get(self.active).map(|t| t.hidden_columns.clone()).unwrap_or_default();
        HIDEABLE
            .iter()
            .map(|(col, name)| {
                let off = hidden.contains(col);
                let (col, weak) = (*col, this.clone());
                MenuRow::action(format!("column.{name}"), format!("{} {name}", if off { "Show" } else { "Hide" }), if off { "eye" } else { "eye-off" }, format!("{} the {name} column for this repository", if off { "Show" } else { "Hide" }), move |_, cx| {
                    weak.update(cx, |v, cx| v.toggle_column(col, cx)).ok();
                })
            })
            .collect()
    }
}

/// What a header cell needs of the table.
#[derive(Clone)]
struct HeadContext {
    cols: Columns,
    available: f32,
    graph_max: f32,
}

fn head_cell(col: Col, width: f32, title: &'static str, name: &'static str, glyph: &'static str, head: &HeadContext, cx: &mut Context<GitVolt>) -> Stateful<Div> {
    let start = head.clone();
    let resizer = div()
        .id(("col-resizer", col as usize))
        .group("col-resizer")
        .absolute()
        .top_0()
        .bottom_0()
        .right(px(-4.))
        .w(px(8.))
        .cursor(CursorStyle::ResizeLeftRight)
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            if e.click_count == 2 {
                return this.reset_column(col, cx);
            }
            let Some(tab) = this.tabs.get(this.active) else { return };
            this.column_drag = Some(ColumnDrag { col, x: e.position.x.as_f32(), start: start.cols, prefs: tab.column_prefs.clone(), available: start.available, graph_max: start.graph_max });
        }))
        .child(div().absolute().top(px(4.)).bottom(px(4.)).left(px(3.)).w(px(2.)).hover(|d| d.bg(theme::blue())));
    let label: AnyElement = if is_collapsed(col, width) { tip(name, icon(glyph, 13., theme::text_secondary())).into_any_element() } else { div().min_w_0().truncate().child(title).into_any_element() };
    div().id(("col-head", col as usize)).relative().flex().flex_none().w(px(width)).h_full().items_center().px(px(if col == Col::Sha { SHA_PAD } else { CELL_PAD })).child(label).child(resizer)
}

pub fn graph_table(this: &GitVolt, width: f32, window: &Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let Some(tab) = this.tabs.get(this.active) else {
        return div().flex().flex_1().items_center().justify_center().text_color(theme::text_disabled()).child("Open a repository (Ctrl+O)").into_any_element();
    };
    let Some(graph) = tab.graph.clone() else {
        let text: SharedString = tab.error.clone().unwrap_or_else(|| "Loading…".into());
        return div().flex().flex_1().items_center().justify_center().text_color(theme::text_disabled()).child(text).into_any_element();
    };
    // The scrollbar takes its room only when there's something to scroll.
    let list_h = window.viewport_size().height.as_f32() - 30. - 34. - 44. - 22. - HEADER_H;
    let scrolls = (graph.rows.len() + usize::from(graph.truncated)) as f32 * ROW_H > list_h;
    let gutter = if scrolls { SCROLLBAR_W } else { 0. };
    let available = width - gutter;
    let cols = allocate_columns(&tab.column_prefs, available, &tab.hidden_columns, graph.max_lanes);
    let hidden = tab.hidden_columns.clone();
    let head = HeadContext { cols, available, graph_max: lanes_width(graph.max_lanes) };
    let shown = |c: Col| !hidden.contains(&c);
    let weak = cx.weak_entity();
    let pinned = graph.pinned.clone();
    let compact = cols.graph < PIN_FULL_MIN;
    let pin = (!pinned.is_empty()).then(|| {
        tip(
            format!("Pinned trunk: {pinned} (default)"),
            div().flex().flex_none().items_center().gap(px(3.)).child(icon("pin", 11., theme::text_secondary())).when(!compact, |d| d.child(div().truncate().child(pinned.clone()))),
        )
    });
    let mut header = div()
        .id("graph-header")
        .flex()
        .flex_1()
        .h(px(HEADER_H))
        .items_center()
        .border_b_1()
        .border_color(theme::section_border())
        .text_size(px(11.))
        .text_color(theme::text_secondary())
        .on_mouse_down(MouseButton::Right, {
            let weak = weak.clone();
            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                let rows = this.column_rows(&weak);
                menu::open_context(rows, e.position, window, cx);
            })
        });
    if shown(Col::Labels) {
        header = header.child(head_cell(Col::Labels, cols.labels, "BRANCH / TAG", "Branch / Tag", "tag", &head, cx));
    }
    header = header
        .child(head_cell(Col::Graph, cols.graph, "GRAPH", "Graph", "git-graph", &head, cx).gap(px(10.)).justify_between().children(pin))
        .child(head_cell(Col::Message, cols.message, "COMMIT MESSAGE", "Commit message", "message-square", &head, cx));
    if shown(Col::Author) {
        header = header.child(head_cell(Col::Author, cols.author, "AUTHOR", "Author", "user", &head, cx));
    }
    if shown(Col::Date) {
        header = header.child(head_cell(Col::Date, cols.date, "COMMIT DATE / TIME", "Commit date / time", "clock", &head, cx));
    }
    if shown(Col::Sha) {
        header = header.child(head_cell(Col::Sha, cols.sha, "SHA", "SHA", "hash", &head, cx));
    }
    let selected = tab.selected;
    let picked = tab.picked();
    let hover_chip = tab.hover_chip.clone();
    let rows = graph.clone();
    let keep = tab.find.keep.clone();
    let branch_keep = tab.branch_keep.clone();
    let row_cols = cols;
    let drafts = tab.row_drafts.clone();
    let pending = tab.pending.clone();
    let head_sha = tab.graph.as_ref().and_then(|g| g.head_sha.clone());
    let rebasing_branch = tab.graph.as_ref().and_then(|g| match g.in_progress.get(&tab.worktree) {
        Some(gitvolt_graph::InProgress::Rebase { head_name, .. }) => Some(head_name.trim_start_matches("refs/heads/").to_string()),
        _ => None,
    });
    let (repo, loaded, loading) = (tab.id, graph.rows.len() as u32, tab.loading);
    let older = graph.truncated;
    let messages = this.messages.clone();
    let list = uniform_list(
        "graph",
        graph.rows.len() + usize::from(older),
        cx.processor(move |_, range: Range<usize>, window, cx| {
            let graph = rows.clone();
            let cols = &row_cols;
            let weak = cx.weak_entity();
            range
                .map(|ix| {
                    if ix == graph.rows.len() {
                        return load_older_row(repo, loaded, loading, cx);
                    }
                    let row = &graph.rows[ix];
                    let is_selected = selected == Some(ix) || picked.contains(&ix);
                    let member = graph.membership.get(ix).cloned().flatten().filter(|_| is_selected || hover_chip.as_ref() == Some(&row.id));
                    let filter = keep.as_ref().is_some_and(|k| !k.contains(&ix));
                    let branch = branch_keep.as_ref().is_some_and(|k| !k.contains(&ix));
                    let chips_stop = chips_end(row, cols.labels, cx);
                    let chips_stop = match &member {
                        Some(m) if !row.chips.is_empty() && chips_stop + CHIP_SPACING + dim_chip_width(m, cx) <= cols.labels => chips_stop + CHIP_SPACING + dim_chip_width(m, cx),
                        _ => chips_stop,
                    };
                    let geometry = RowGeometry { chips_end: chips_stop, graph_x: cols.labels, graph_w: cols.graph, clipped: cols.clipped, selected: is_selected, head: graph.head_row == Some(ix) };
                    let painter = graph.clone();
                    let picture = if row.kind == NodeKind::Commit { crate::avatars::image_for(&row.email, &row.author) } else { None };
                    let group: SharedString = format!("row-{ix}").into();
                    let cell = |w: f32, color: Hsla, text: SharedString| {
                        // A dim row keeps its dim; the stronger of Find's and a branch's wins.
                        let color = if filter { theme::text_row_dimmed() } else if branch { theme::text_row_dimmed_branch() } else { color };
                        div()
                            .h(px(ROW_H))
                            .flex()
                            .flex_none()
                            .w(px(w))
                            .items_center()
                            .px(px(CELL_PAD))
                            .min_w_0()
                            .overflow_hidden()
                            .text_color(color)
                            .when(is_selected && !filter && !branch, |d| d.text_color(theme::text_selected()))
                            .when(is_selected, |d| d.bg(theme::selected_row()))
                            .when(!is_selected, |d| d.group_hover(group.clone(), |d| d.bg(theme::hover_row())))
                            .child(div().truncate().child(text))
                    };
                    let message: AnyElement = match row.wip.as_ref() {
                        Some(w) => {
                            let draft = row.worktree.as_ref().and_then(|p| drafts.get(p));
                            let focused = draft.is_some_and(|d| d.read(cx).is_focused(window));
                            wip_cell(draft, w, cols.message, focused)
                        }
                        .when(is_selected, |d| d.bg(theme::selected_row())).when(!is_selected, |d| d.group_hover(group.clone(), |d| d.bg(theme::hover_row()))).into_any_element(),
                        None => {
                            let dim_color = if filter { theme::text_row_dimmed() } else if branch { theme::text_row_dimmed_branch() } else { theme::text_disabled() };
                            let tail = row.stash_branch.clone().into_iter().chain((!row.body.is_empty()).then(|| row.body.clone()));
                            let body = message_cell(cols.message, is_selected, filter, branch, row.summary.clone(), tail, dim_color, group.clone());
                            if row.kind == NodeKind::Collapsed {
                                body.into_any_element()
                            } else {
                                let (id, cache, weak_msg, summary) = (row.id.to_string(), messages.clone(), weak.clone(), row.summary.clone());
                                Anchor::new(body).card(format!("msg:{id}"), move |_, cx| message_card(&id, &summary, &cache, &weak_msg, cx)).delay(MESSAGE_TIP_DELAY).placement(Placement::Pointer).into_any_element()
                            }
                        }
                    };
                    let node_tip = (row.kind == NodeKind::Commit && !row.email.is_empty()).then(|| {
                        let x = cols.labels + node_x(row.lane, cols.graph, cols.clipped).unwrap_or_else(|| packed_x(cols.graph)) - NODE_R;
                        tip(format!("{} <{}>", row.author, row.email), div().id(("node", ix)).absolute().left(px(x)).top(px((ROW_H - 2. * NODE_R) / 2.)).size(px(2. * NODE_R)))
                    });
                    let outlined = menu::is_target(&format!("row:{ix}"), cx);
                    let off = |c: Col| hidden.contains(&c);
                    let menu_weak = weak.clone();
                    let pending_refs: Vec<&str> = pending.keys().map(String::as_str).collect();
                    let rebasing = rebasing_branch.as_deref().filter(|_| head_sha.as_deref() == Some(row.id.as_ref()));
                    let labels = (!off(Col::Labels)).then(|| chips_cell(row, ix, cols.labels, is_selected, &group, &pending_refs, rebasing, member, &weak, cx));
                    div()
                        .id(ix)
                        .group(group.clone())
                        .relative()
                        .flex()
                        .h(px(ROW_H))
                        .on_hover(cx.listener(move |this, entered: &bool, _, cx| this.hover_row(ix, *entered, cx)))
                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                            if e.click_count == 2 {
                                this.double_click_row(ix, cx);
                            } else {
                                this.select_row(ix, e.modifiers.into(), cx);
                            }
                        }))
                        .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            let rows = this.row_menu(ix, &menu_weak);
                            if !rows.is_empty() {
                                menu::open_context_for(format!("row:{ix}"), rows, e.position, window, cx);
                            }
                        }))
                        .child(canvas(|_, _, _| {}, move |bounds, _, window, cx| paint_row(bounds, &painter.rows[ix], ix, geometry, picture.clone(), window, cx)).absolute().top_0().left_0().size_full())
                        .children(labels)
                        .child(div().flex_none().w(px(cols.graph)))
                        .child(message)
                        .when(!off(Col::Author), |d| {
                            if is_collapsed(Col::Author, cols.author) && matches!(row.kind, NodeKind::Commit | NodeKind::Merge) {
                                d.child(cell(cols.author, theme::text_disabled(), SharedString::default()).justify_center().child(tip(row.author.clone(), crate::details::avatar(&row.author, &row.email, 16.))))
                            } else {
                                d.child(cell(cols.author, theme::text_disabled(), row.author.clone()))
                            }
                        })
                        .when(!off(Col::Date), |d| d.child(cell(cols.date, theme::text_disabled(), row.date.clone())))
                        .when(!off(Col::Sha), |d| {
                            let color = if filter { theme::text_row_dimmed() } else if branch { theme::text_row_dimmed_branch() } else if is_selected { theme::text_selected() } else { theme::text_disabled() };
                            d.child(sha_cell(cols.sha, row, color, is_selected, group.clone(), cx))
                        })
                        // The space past the last column takes the row's hover and selection too.
                        .child(div().flex_1().h(px(ROW_H)).when(is_selected, |d| d.bg(theme::selected_row())).when(!is_selected, |d| d.group_hover(group.clone(), |d| d.bg(theme::hover_row()))))
                        .children(node_tip)
                        .when(outlined, |d| d.child(div().absolute().top_0().left_0().size_full().border_1().border_color(theme::blue())))
                        .into_any_element()
                })
                .collect()
        }),
    )
    .track_scroll(&tab.list)
    .flex_1()
    .min_w_0();
    let find = tab.find.open.then(|| crate::find::find_box(&tab.find, window, cx));
    let thumb = scrollbar(tab, cx);
    div().relative().flex().flex_col().flex_1().min_w_0().min_h_0().bg(theme::app_bg()).children(find).child(div().flex().flex_none().pr(px(gutter)).overflow_hidden().child(header)).child(div().flex().flex_1().min_h_0().child(list).when(scrolls, |d| d.child(thumb))).into_any_element()
}

/// The Message cell: the summary, and after it what's dimmed (a stash's branch, a body's first line), cut off first.
#[allow(clippy::too_many_arguments)]
fn message_cell(width: f32, selected: bool, filter: bool, branch: bool, summary: SharedString, tail: impl Iterator<Item = SharedString>, dim: Hsla, group: SharedString) -> Div {
    let color = if filter { theme::text_row_dimmed() } else if branch { theme::text_row_dimmed_branch() } else if selected { theme::text_selected() } else { theme::text_normal() };
    div()
        .h(px(ROW_H))
        .flex()
        .flex_none()
        .w(px(width))
        .items_center()
        .px(px(CELL_PAD))
        .min_w_0()
        .overflow_hidden()
        .text_color(color)
        .when(selected, |d| d.bg(theme::selected_row()))
        .when(!selected, |d| d.group_hover(group, |d| d.bg(theme::hover_row())))
        .child(div().flex_none().max_w_full().truncate().child(summary))
        .children(tail.map(|t| div().flex_1().min_w_0().ml(px(10.)).truncate().text_color(dim).child(t)))
}

/// The SHA cell: the hash, which copies itself; a press on it doesn't select the row.
fn sha_cell(width: f32, row: &RowView, color: Hsla, selected: bool, group: SharedString, cx: &mut Context<GitVolt>) -> Div {
    let id = row.id.to_string();
    let base = div().font_family("JetBrains Mono").h(px(ROW_H)).flex().flex_none().w(px(width)).items_center().px(px(SHA_PAD)).min_w_0().overflow_hidden().when(selected, |d| d.bg(theme::selected_row())).when(!selected, |d| d.group_hover(group, |d| d.bg(theme::hover_row())));
    if row.sha.is_empty() {
        return base;
    }
    let button = div().id(SharedString::from(format!("sha-{id}"))).cursor_pointer().text_color(color).hover(|d| d.text_color(theme::text_selected()).underline()).on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(cx.listener(move |this, _, _, cx| this.copy_text(id.clone(), cx))).child(row.sha.clone());
    base.child(tip("Copy full SHA", button))
}

/// The row after the last of a truncated graph: loads the next step.
fn load_older_row(repo: u32, loaded: u32, loading: bool, cx: &mut Context<GitVolt>) -> AnyElement {
    let hint = if loading { "Loading…".to_string() } else { format!("{} more", with_commas(crate::app::GRAPH_LIMIT)) };
    div()
        .id("load-older")
        .flex()
        .h(px(ROW_H))
        .items_center()
        .gap(px(12.))
        .px(px(12.))
        .whitespace_nowrap()
        .text_color(theme::text_secondary())
        .child(
            div()
                .id("load-older-btn")
                .flex()
                .gap(px(8.))
                .px(px(8.))
                .py(px(1.))
                .rounded(px(4.))
                .text_color(if loading { theme::text_disabled() } else { theme::text_selected() })
                .when(!loading, |d| {
                    d.cursor_pointer().hover(|d| d.bg(theme::hover_row())).on_click(cx.listener(move |this, _, _, cx| {
                        let limit = this.tabs.iter().find(|t| t.id == repo).map_or(crate::app::GRAPH_LIMIT, |t| t.limit) + crate::app::GRAPH_LIMIT;
                        this.load_deep(repo, limit, None, cx);
                    }))
                })
                .child("Load older commits")
                .child(div().text_color(theme::text_secondary()).child(hint)),
        )
        .child(format!("{} commits loaded", with_commas(loaded)))
        .into_any_element()
}

/// The whole message, once it's read; the summary meanwhile.
fn message_card(id: &str, summary: &SharedString, cache: &MessageCache, this: &WeakEntity<GitVolt>, cx: &mut App) -> AnyElement {
    let known = cache.borrow().get(id).cloned();
    let text = match known {
        Some(Some(t)) => t,
        Some(None) => summary.clone(),
        None => {
            cache.borrow_mut().insert(id.to_string(), None);
            let (this, id) = (this.clone(), id.to_string());
            cx.defer(move |cx| {
                this.update(cx, |v, cx| v.read_message(id, cx)).ok();
            });
            summary.clone()
        }
    };
    div().flex_none().w(px(320.)).px(px(8.)).py(px(5.)).rounded(px(4.)).border_1().border_color(theme::section_border()).bg(theme::panel_bg1()).text_color(theme::text_selected()).text_size(px(12.)).line_height(px(16.8)).shadow(crate::ui::shadow(theme::shadow_tooltip(), 2., 8.)).child(text).into_any_element()
}

/// The graph's scrollbar: its thumb follows the list; a press or a drag jumps the list there.
fn scrollbar(tab: &Tab, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let handle = tab.list.0.borrow().base_handle.clone();
    let track_bounds = Rc::new(std::cell::Cell::new(Bounds::<Pixels>::default()));
    let (for_paint, on_down, on_move) = (track_bounds.clone(), track_bounds.clone(), track_bounds);
    div()
        .id("graph-scrollbar")
        .relative()
        .flex_none()
        .w(px(SCROLLBAR_W))
        .h_full()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                this.dragging_scrollbar = true;
                let b = on_down.get();
                this.scroll_to_pointer(e.position.y, b.origin.y, b.size.height, cx);
            }),
        )
        .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, _| this.dragging_scrollbar = false))
        .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.dragging_scrollbar = false))
        .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
            if this.dragging_scrollbar && e.pressed_button == Some(MouseButton::Left) {
                let b = on_move.get();
                this.scroll_to_pointer(e.position.y, b.origin.y, b.size.height, cx);
            }
        }))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    for_paint.set(bounds);
                    let max = handle.max_offset().y;
                    if max <= px(0.) {
                        return;
                    }
                    let height = bounds.size.height;
                    let thumb = (height * (height / (height + max))).max(px(24.));
                    let top = (height - thumb) * (-handle.offset().y / max);
                    let b = Bounds::new(point(bounds.origin.x + px(2.), bounds.origin.y + top), size(px(6.), thumb));
                    window.paint_quad(quad(b, px(3.), theme::control_hover_bg(), px(0.), transparent_black(), BorderStyle::default()));
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::with_rows;
    use std::prelude::v1::test;

    const HEADER_Y: f32 = 30. + 34. + 44. + 8.;

    #[test]
    fn as_many_chips_fit_as_there_is_room_for_with_the_badge() {
        let more = |_: usize| 30.;
        assert_eq!(fit_count(&[50.], 10., more), 1);
        assert_eq!(fit_count(&[50., 50., 50.], 200., more), 3);
        assert_eq!(fit_count(&[50., 50., 50.], 145., more), 2);
        assert_eq!(fit_count(&[50., 50., 50.], 100., more), 1);
    }

    #[test]
    fn thousands_get_their_commas() {
        assert_eq!(with_commas(2000), "2,000");
        assert_eq!(with_commas(999), "999");
        assert_eq!(with_commas(1234567), "1,234,567");
    }

    #[gpui::test]
    fn a_right_click_on_the_header_hides_a_column(cx: &mut TestAppContext) {
        let (window, mut cx, _runtime) = with_rows(cx, 3);
        cx.run_until_parked();
        let at = point(px(1100.), px(HEADER_Y));
        cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
        cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
        let rows = cx.update(|_, cx| menu::shown(cx));
        assert_eq!(rows[0].iter().map(|r| r.trim_start_matches('*')).collect::<Vec<_>>(), ["Hide Branch / Tag", "Hide Author", "Hide Commit date / time", "Hide SHA"]);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        window.read_with(&cx, |v, _| assert!(v.tabs[0].hidden_columns.contains(&Col::Labels))).unwrap();
    }

    #[gpui::test]
    fn dragging_a_columns_edge_resizes_it(cx: &mut TestAppContext) {
        let (window, mut cx, _runtime) = with_rows(cx, 3);
        cx.run_until_parked();
        // The labels column's right edge: the sidebar and 200 px in.
        let x0 = crate::sidebar::DEFAULT_SIDEBAR_W + 200.;
        cx.simulate_mouse_move(point(px(x0), px(HEADER_Y)), None, Modifiers::none());
        cx.simulate_mouse_down(point(px(x0), px(HEADER_Y)), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(point(px(x0 + 40.), px(HEADER_Y)), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(point(px(x0 + 40.), px(HEADER_Y)), MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        window.read_with(&cx, |v, _| assert_eq!(v.tabs[0].column_prefs.labels, 240.)).unwrap();
    }

    #[gpui::test]
    fn a_right_click_on_a_row_opens_its_menu_without_selecting_it(cx: &mut TestAppContext) {
        let (window, mut cx, _runtime) = with_rows(cx, 5);
        window
            .update(&mut cx, |v, _, _| {
                let g = Rc::get_mut(v.tabs[0].graph.as_mut().unwrap()).unwrap();
                g.rows.iter_mut().for_each(|r| r.kind = NodeKind::Commit);
            })
            .unwrap();
        cx.run_until_parked();
        let at = point(px(1100.), px(30. + 34. + 44. + 25. + ROW_H * 1.5));
        cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
        cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
        assert!(cx.update(|_, cx| menu::is_open(cx) && menu::is_target("row:1", cx)));
        window.read_with(&cx, |v, _| assert_eq!(v.tabs[0].selected, None)).unwrap();
    }
}
