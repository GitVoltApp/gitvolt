//! The interactive rebase editor's panel (`RebaseEditor.tsx`, `ChipColumn.tsx`, `editor.css`): the header, the rows and
//! the base, the footer's keys and Start. Every cell keeps its width, so a dot, a ⚠ or a chip appearing never shifts a row.

use super::model::*;
use super::*;
use crate::app::{GitVolt, Tab, icon};
use crate::commit_box::{neutral, positive};
use crate::theme;
use crate::ui::menu::{self, OpenOptions};
use crate::ui::tooltip::tip;

const CHIPS_W: f32 = 220.;
const ACTION_W: f32 = 84.;
const ROW_H: f32 = 30.;
const CHIP_H: f32 = 22.;
/// The footer's key hints: chords, label, dropped first on a narrow panel.
const HINTS: [(&[&str], &str, bool); 9] = [(&["P"], "Pick", false), (&["R"], "Reword", false), (&["S"], "Squash", false), (&["F"], "Fixup", true), (&["D"], "Drop", false), (&["E"], "Edit", true), (&["Mod+↑", "↓"], "Move", false), (&["Enter"], "Message", true), (&["Esc"], "Cancel", false)];

fn action_color(a: RowAction) -> Hsla {
    match a {
        RowAction::Pick => theme::green(),
        RowAction::Reword | RowAction::Squash | RowAction::Fixup => theme::orange(),
        RowAction::Drop => theme::red(),
        RowAction::Edit => theme::blue(),
    }
}

#[derive(Clone)]
enum ChipKind {
    Base,
    Rebased,
    Chip(EditorChip),
}

#[derive(Clone)]
struct ChipItem {
    branch: String,
    kind: ChipKind,
    color: Option<Hsla>,
}

impl ChipItem {
    fn free(&self) -> bool {
        matches!(&self.kind, ChipKind::Chip(c) if c.locked.is_none() && !c.deleted)
    }

    fn has_x(&self) -> bool {
        matches!(self.kind, ChipKind::Chip(_))
    }

    fn note(&self) -> String {
        match &self.kind {
            ChipKind::Base => "The base: read-only".to_string(),
            ChipKind::Rebased => "Ends here: git moves it".to_string(),
            ChipKind::Chip(c) => match &c.locked {
                Some(why) => format!("Can't move: {why}"),
                None if c.deleted => "Will be deleted when the rebase completes".to_string(),
                None => "Drag it to another commit to move it".to_string(),
            },
        }
    }
}

fn chip_items(s: &RebaseSession, state: &EditorState, g: &Grouping, row: &str) -> Vec<ChipItem> {
    let base = row == state.base.oid;
    let mut items: Vec<ChipItem> = Vec::new();
    if base {
        items.extend(state.base.chips.iter().map(|b| ChipItem { branch: b.clone(), kind: ChipKind::Base, color: s.chip_color(b) }));
    }
    if rebased_row(state, g).as_deref() == Some(row) {
        items.push(ChipItem { branch: state.branch.clone(), kind: ChipKind::Rebased, color: s.chip_color(&state.branch) });
    }
    items.extend(state.chips.iter().filter(|c| chip_row(state, &c.at, g) == row).map(|c| ChipItem { branch: c.branch.clone(), kind: ChipKind::Chip(c.clone()), color: if c.origin.is_none() { None } else { s.chip_color(&c.branch) } }));
    items
}

fn lane_or(color: Option<Hsla>, fallback: usize) -> Hsla {
    color.unwrap_or_else(|| theme::lane(fallback))
}

/// The chip itself: its colour by what it is.
fn chip_body(item: &ChipItem, dragged: bool) -> Div {
    let lane = lane_or(item.color, if matches!(item.kind, ChipKind::Base) { 1 } else { 0 });
    let mut d = div().flex_none().h(px(CHIP_H)).line_height(px(CHIP_H)).px(px(5.)).rounded(px(3.)).text_size(px(12.)).whitespace_nowrap().overflow_hidden().text_ellipsis().min_w_0().text_color(theme::text_selected()).bg(theme::mix(lane, 0.3, transparent_black()));
    match &item.kind {
        ChipKind::Rebased => {
            d = d.relative().font_weight(FontWeight::BOLD).bg(theme::mix(lane, 0.45, transparent_black())).border_1().border_color(lane);
        }
        ChipKind::Chip(c) => {
            if c.deleted {
                d = d.line_through().opacity(0.6);
            }
            if c.locked.is_some() {
                d = d.bg(theme::mix(lane, 0.18, transparent_black())).text_color(theme::text_normal());
            }
            if c.locked.is_none() && !c.deleted {
                d = d.cursor(CursorStyle::OpenHand);
            }
        }
        ChipKind::Base => {}
    }
    if dragged {
        d = d.opacity(0.5);
    }
    d
}

fn x_button(id: SharedString, c: &EditorChip, group: Option<SharedString>, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let branch = c.branch.clone();
    let deleted = c.deleted;
    let color = if deleted { theme::red() } else { theme::text_secondary() };
    let tooltip = if deleted { format!("Keep {branch}") } else { format!("Delete {branch} when the rebase completes") };
    tip(
        tooltip,
        div()
            .id(ElementId::Name(id))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(16.))
            .rounded(px(3.))
            .cursor_pointer()
            .hover(|d| d.bg(theme::control_hover_bg()))
            .when_some(group.filter(|_| !deleted), |d, g| d.invisible().group_hover(g, |d| d.visible()))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                let b = branch.clone();
                this.with_rebase(cx, |s, _| s.edit(|st| remove_chip(st, &b)));
            }))
            .child(icon("x", 11., color)),
    )
}

fn press_chip(item: &ChipItem) -> Option<String> {
    item.free().then(|| item.branch.clone())
}

/// A chip at its full name, every control of the chip in it: the hover expansion of a truncated chip, and the "+N" list.
fn full_chip(item: &ChipItem, key: String, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let lane = lane_or(item.color, if matches!(item.kind, ChipKind::Base) { 1 } else { 0 });
    let free = press_chip(item);
    let menu_branch = item.branch.clone();
    let id: SharedString = format!("{key}:full").into();
    let x_id: SharedString = format!("{key}:full-x").into();
    let name = div().flex_none().h_full().line_height(px(CHIP_H)).px(px(5.)).rounded(px(3.)).text_color(theme::text_selected()).bg(theme::mix(lane, 0.3, theme::panel_bg())).when(matches!(&item.kind, ChipKind::Chip(c) if c.deleted), |d| d.line_through().text_color(theme::text_secondary())).when(matches!(item.kind, ChipKind::Rebased), |d| d.font_weight(FontWeight::BOLD)).child(item.branch.clone());
    let controls = match &item.kind {
        ChipKind::Chip(c) if c.locked.is_some() => Some(icon("lock", 11., theme::text_secondary()).mx(px(2.)).into_any_element()),
        ChipKind::Chip(c) => Some(x_button(x_id, c, None, cx).into_any_element()),
        _ => None,
    };
    div()
        .id(ElementId::Name(id))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(1.))
        .h(px(CHIP_H))
        .pr(px(1.))
        .rounded(px(3.))
        .text_size(px(12.))
        .whitespace_nowrap()
        .bg(theme::panel_bg())
        .shadow(crate::ui::shadow(theme::shadow_popup(), 1., 4.))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            if let Some(b) = free.clone() {
                this.start_chip_drag(b, e.position, cx);
            }
        }))
        .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            let weak = cx.weak_entity();
            let rows = this.rebase_chip_rows(&menu_branch, &weak);
            menu::open_context(rows, e.position, window, cx);
        }))
        .child(name)
        .children(controls)
}

fn chip_column(s: &RebaseSession, state: &EditorState, g: &Grouping, row: &str, group: SharedString, cx: &mut Context<GitVolt>) -> Div {
    let base = row == state.base.oid;
    let items = chip_items(s, state, g, row);
    let pairs: Vec<(String, bool)> = items.iter().map(|i| (i.branch.clone(), i.has_x())).collect();
    let avail = if base { BASE_LIST_W } else { LIST_W };
    let shown = chips_shown(&pairs, avail);
    let reserve = if shown < items.len() { 36. } else { 0. };
    let wants: Vec<(f32, f32)> = items[..shown]
        .iter()
        .map(|i| {
            let weight = if matches!(i.kind, ChipKind::Rebased) { FontWeight::BOLD } else { FontWeight::NORMAL };
            let x = if i.has_x() { 17. } else { 0. };
            let text = crate::ui::text_width(&i.branch, 12., weight, cx).ceil() + 10. + if matches!(i.kind, ChipKind::Rebased) { 2. } else { 0. };
            (text + x, if i.branch.chars().count() <= READABLE { text + x } else { 6. * 7.5 + 10. + x })
        })
        .collect();
    let widths = share_widths(&wants, avail - reserve);
    let dragged = s.chip_drag.as_ref().filter(|d| d.active).map(|d| d.branch.clone());
    let mut list = div().flex().flex_none().items_center().gap(px(6.));
    for (i, item) in items[..shown].iter().enumerate() {
        let key = format!("ichip:{}:{}", row, item.branch);
        let is_dragged = dragged.as_deref() == Some(item.branch.as_str());
        let width = widths[i];
        let body_w = width - if item.has_x() { 17. } else { 0. };
        let full_w = wants[i].0;
        let truncated = full_w > width + 0.5;
        let free = press_chip(item);
        let menu_branch = item.branch.clone();
        let branch = item.branch.clone();
        let controls = match &item.kind {
            ChipKind::Chip(c) if c.locked.is_some() => Some(icon("lock", 11., theme::text_secondary()).mx(px(2.)).into_any_element()),
            ChipKind::Chip(c) => Some(x_button(format!("{key}:x").into(), c, Some(group.clone()), cx).into_any_element()),
            _ => None,
        };
        let expanded = s.expanded.as_deref() == Some(item.branch.as_str()) && s.chip_drag.is_none() && s.row_drag.is_none();
        let overlay = expanded.then(|| deferred(div().absolute().top_0().left_0().child(full_chip(item, key.clone(), cx))).with_priority(2).into_any_element());
        let name_id: SharedString = format!("{key}:name").into();
        let hover_branch = branch.clone();
        let name = div()
            .id(ElementId::Name(name_id))
            .flex_none()
            .w(px(body_w))
            .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                if !truncated {
                    return;
                }
                let (over, b) = (*over, hover_branch.clone());
                this.with_rebase(cx, |s, _| {
                    if over {
                        s.expanded = Some(b);
                    }
                });
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                if let Some(b) = free.clone() {
                    cx.stop_propagation();
                    this.start_chip_drag(b, e.position, cx);
                }
            }))
            .child(chip_body(item, is_dragged).w(px(body_w)).child(item.branch.clone()));
        let leave_branch = branch.clone();
        let wrap = div()
            .id(ElementId::Name(format!("{key}:wrap").into()))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(1.))
            .w(px(width))
            .text_size(px(12.))
            .text_color(theme::text_secondary())
            .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                if !*over {
                    let b = leave_branch.clone();
                    this.with_rebase(cx, |s, _| {
                        if s.expanded.as_deref() == Some(b.as_str()) {
                            s.expanded = None;
                        }
                    });
                }
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                let weak = cx.weak_entity();
                let rows = this.rebase_chip_rows(&menu_branch, &weak);
                menu::open_context(rows, e.position, window, cx);
            }))
            .child(name)
            .children(controls)
            .children(overlay);
        list = list.child(crate::ui::anchor(key.clone(), tip(item.note(), wrap)));
    }
    if shown < items.len() {
        let hidden: Vec<String> = items[shown..].iter().map(|i| i.branch.clone()).collect();
        let row_key = row.to_string();
        let (hover_row, menu_hidden, menu_hidden2) = (row_key.clone(), hidden.clone(), hidden.clone());
        let pill_key: SharedString = format!("ichip-more:{row}").into();
        let open_list = s.more_open.as_deref() == Some(row) && s.chip_drag.is_none() && s.row_drag.is_none();
        let pill = div()
            .id(ElementId::Name(pill_key.clone()))
            .flex_none()
            .min_w(px(24.))
            .h(px(18.))
            .px(px(5.))
            .rounded(px(9.))
            .text_size(px(11.))
            .line_height(px(18.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_center()
            .text_color(theme::text_selected())
            .bg(theme::panel_bg1())
            .cursor_pointer()
            .hover(|d| d.bg(theme::control_hover_bg()))
            .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                if *over {
                    let r = hover_row.clone();
                    this.with_rebase(cx, |s, _| s.more_open = Some(r));
                }
            }))
            .on_mouse_down(MouseButton::Left, {
                let key = pill_key.clone();
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    let rows = this.rebase_hidden_rows(&menu_hidden, cx);
                    menu::open_below(key.clone(), rows, OpenOptions { label: Some(format!("More branches: {}", menu_hidden.join(", ")).into()), ..Default::default() }, window, cx);
                })
            })
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                let rows = this.rebase_hidden_rows(&menu_hidden2, cx);
                menu::open_context(rows, e.position, window, cx);
            }))
            .child(format!("+{}", hidden.len()));
        let all = open_list.then(|| {
            let mut list = div().id(ElementId::Name(format!("ichip-all:{row}").into())).absolute().top(px(24.)).left_0().flex().flex_wrap().gap(px(4.)).max_w(px(480.)).p(px(4.)).rounded(px(4.)).bg(theme::panel_bg()).shadow(crate::ui::shadow(theme::shadow_popup(), 2., 8.)).on_hover(cx.listener(|this, over: &bool, _, cx| {
                if !*over {
                    this.with_rebase(cx, |s, _| s.more_open = None);
                }
            }));
            for it in &items {
                list = list.child(full_chip(it, format!("ichip-all:{row}:{}", it.branch), cx));
            }
            deferred(list).with_priority(3).into_any_element()
        });
        list = list.child(crate::ui::anchor(pill_key.clone(), div().relative().child(pill).children(all)));
    }
    let mut column = div().flex().flex_none().w(px(CHIPS_W)).h(px(ROW_H)).items_center().gap(px(3.)).child(list);
    if !base {
        let r = row.to_string();
        column = column.child(tip(
            "Add a branch at this commit",
            div()
                .id(ElementId::Name(format!("ichip-add:{row}").into()))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(16.))
                .rounded(px(3.))
                .cursor_pointer()
                .invisible()
                .group_hover(group, |d| d.visible())
                .hover(|d| d.bg(theme::control_hover_bg()))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| this.open_add_branch(r.clone(), cx)))
                .child(icon("plus", 12., theme::text_secondary())),
        ));
    }
    column
}

fn action_button(s: &RebaseSession, row: &EditorRow, selected: bool, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let color = action_color(row.action);
    let key: SharedString = format!("iaction:{}", row.oid).into();
    let open = menu::is_open_on(&key, cx);
    let targets: Vec<String> = if selected { s.state.selected.clone() } else { vec![row.oid.clone()] };
    let bottom = bottom_row(&s.state);
    let refused = bottom.as_deref() == Some(row.oid.as_str()) && targets.iter().all(|o| *o == row.oid);
    let (oid, chosen) = (row.oid.clone(), row.action);
    let open_key = key.clone();
    tip(
        chosen.tip(),
        crate::ui::anchor(
            key.clone(),
            div()
                .id(ElementId::Name(key))
                .flex()
                .flex_none()
                .items_center()
                .justify_between()
                .gap(px(4.))
                .w(px(ACTION_W))
                .h(px(22.))
                .pl(px(7.))
                .pr(px(4.))
                .rounded(px(4.))
                .border_1()
                .border_color(if open { theme::blue() } else { theme::mix(color, 0.45, transparent_black()) })
                .bg(theme::mix(color, 0.14, theme::app_bg()))
                .hover(move |d| d.bg(theme::mix(color, 0.24, theme::app_bg())))
                .text_size(px(12.))
                .line_height(px(16.8))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    // On a selected row the dropdown sets every selected row: the press keeps the selection.
                    if !selected {
                        this.rebase_select(&oid, false, false, cx);
                    }
                    let weak = cx.weak_entity();
                    let rows = ACTIONS
                        .iter()
                        .map(|a| {
                            let (a, weak, targets, oid) = (*a, weak.clone(), targets.clone(), oid.clone());
                            let row = MenuRow::action(format!("select.{}", a.wire()), a.label(), "", a.tip(), move |_, cx| {
                                let (targets, oid) = (targets.clone(), oid.clone());
                                weak.update(cx, |v, cx| v.rebase_action(targets, a, Some(oid), cx)).ok();
                            })
                            .tint(action_color(a));
                            if refused && matches!(a, RowAction::Squash | RowAction::Fixup) { row.disabled(NOTHING_BELOW) } else { row }
                        })
                        .collect();
                    menu::open_below(open_key.clone(), rows, OpenOptions { initial: Some(format!("select.{}", chosen.wire()).into()), label: Some(format!("Action for {}", short(&oid)).into()), narrow: true, ..Default::default() }, window, cx);
                }))
                .child(row.action.label())
                .child(icon("chevron-down", 14., theme::text_secondary())),
        ),
    )
}

/// The message editor sits in the summary column, at most the details panel's usual width, with Cancel and Save under it.
fn message_editor(s: &RebaseSession, window: &Window, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let focused = s.summary.read(cx).is_focused(window) || s.description.read(cx).is_focused(window);
    let n = s.summary.read(cx).text().chars().count();
    let fields = div()
        .flex()
        .flex_col()
        .flex_none()
        .border_1()
        .border_color(if focused { theme::focus_outline() } else { theme::wip_input_border() })
        .rounded(px(6.))
        .bg(theme::wip_input_bg())
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .pr(px(8.))
                .child(div().flex_1().min_w_0().h(px(32.)).pt(px(8.)).pb(px(4.)).pl(px(8.)).text_size(px(15.6)).line_height(px(20.3)).text_color(theme::text_selected()).child(s.summary.clone()))
                .when(n > crate::wip::WIP_DRAFT_COUNTER_FROM, |d| d.child(div().flex_none().text_size(px(11.)).text_color(if n > crate::wip::WIP_DRAFT_WARN_FROM { theme::orange() } else { theme::text_secondary() }).child(n.to_string()))),
        )
        .child(div().flex_none().h(px(82.8)).pt(px(2.)).pb(px(8.)).px(px(8.)).line_height(px(18.2)).text_color(theme::text_normal()).child(s.description.clone()));
    div()
        .id("irebase-message")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .max_w(px(440.))
        .gap(px(6.))
        .pt(px(4.))
        .pb(px(6.))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(fields)
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(6.))
                .child(crate::ui::anchor("irebase-message-cancel", neutral("irebase-message-cancel-b", "Cancel", false).on_click(cx.listener(|this, _, _, cx| this.finish_rebase_message(false, cx)))))
                .child(crate::ui::anchor("irebase-message-save", positive("irebase-message-save-b", "Save", false).on_click(cx.listener(|this, _, _, cx| this.finish_rebase_message(true, cx))))),
        )
}

#[allow(clippy::too_many_arguments)]
fn row_view(s: &RebaseSession, index: usize, row: &EditorRow, g: &Grouping, chips: (&EditorState, &Grouping), window: &Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let state = &s.state;
    let selected = state.selected.contains(&row.oid);
    let into = g.into.get(&row.oid).cloned();
    let group: SharedString = format!("irow-{}", row.oid).into();
    let dim = if into.is_some() { 0.6 } else { 1. };
    let dropped = row.action == RowAction::Drop;
    let chip_over = s.chip_drag.as_ref().is_some_and(|d| d.active && d.over.as_deref() == Some(row.oid.as_str()));
    let conflict = s.prediction.by_row.get(&row.oid).cloned();
    let drag = s.row_drag.as_ref().filter(|d| d.active);
    let (offset, in_group, count) = match drag {
        Some(d) if d.group.contains(&index) => (group_offset(&d.heights, &d.group, d.from, index, d.dy), true, (d.from == index && d.group.len() > 1).then_some(d.group.len())),
        Some(d) => (slot_shifts(&d.heights, &d.group, d.to).get(index).copied().unwrap_or(0.), false, None),
        None => (0., false, None),
    };
    let text = target_message(state, &row.oid, g);
    let first_line = text.split('\n').next().unwrap_or_default().to_string();
    let editing = s.editing.as_deref() == Some(row.oid.as_str());
    let key: SharedString = format!("irow:{}", row.oid).into();
    let selected_bg = theme::mix(theme::blue(), 0.24, theme::app_bg());
    let hover_bg = theme::mix(theme::blue(), if selected { 0.3 } else { 0.1 }, theme::app_bg());
    let oid_press = row.oid.clone();
    let leave_row = row.oid.clone();
    let chips_el = div().opacity(dim).child(chip_column(s, chips.0, chips.1, &row.oid, group.clone(), cx));
    let conflict_el = div().flex().flex_none().w(px(16.)).h(px(ROW_H)).items_center().justify_center().opacity(if s.prediction.pending { 0.4 } else { dim }).children(conflict.map(|paths| tip(format!("Predicted conflict in {}", paths.join(", ")), icon("triangle-alert", 13., theme::yellow()))));
    let fold_el = div().flex().flex_none().w(px(16.)).h(px(ROW_H)).items_center().justify_center().children(into.as_ref().map(|t| tip(format!("Folds into {}", short(t)), icon("arrow-down", 13., theme::orange()))));
    let avatar = crate::details::avatar(&SharedString::from(row.author_name.clone()), &row.author_email, 20.).mt(px(5.)).opacity(if dropped { 0.5 } else { dim });
    let summary = if editing {
        message_editor(s, window, cx).into_any_element()
    } else {
        div().flex_1().min_w_0().h(px(ROW_H)).line_height(px(ROW_H)).truncate().opacity(dim).when(dropped, |d| d.line_through().text_color(theme::text_disabled())).child(first_line).into_any_element()
    };
    let dot = div().flex().flex_none().w(px(12.)).h(px(ROW_H)).items_center().justify_center().opacity(dim).font_weight(FontWeight::BOLD).text_color(theme::blue()).when(row.edited.is_some(), |d| d.child("•"));
    let handle = div().flex().flex_none().w(px(18.)).h(px(ROW_H)).items_center().justify_center().cursor(CursorStyle::OpenHand).child(icon("grip-vertical", 14., theme::text_disabled()).group_hover(group.clone(), |s| s.text_color(theme::text_secondary())));
    let body = div()
        .id(ElementId::Name(key.clone()))
        .group(group.clone())
        .relative()
        .flex()
        .flex_none()
        .items_start()
        .gap(px(6.))
        .min_h(px(ROW_H))
        .pl(px(4.))
        .pr(px(10.))
        .text_color(if selected { theme::text_selected() } else { theme::text_normal() })
        .bg(if selected { selected_bg } else { theme::app_bg() })
        .hover(move |d| d.bg(hover_bg))
        .when(offset != 0., |d| d.top(px(offset)))
        .on_hover(cx.listener(move |this, over: &bool, window, cx| {
            if !*over {
                let keep = crate::ui::Ui::get(cx).anchor(&format!("ichip-more:{leave_row}")).is_some_and(|b| b.contains(&window.mouse_position()));
                if !keep {
                    this.with_rebase(cx, |s, _| {
                        if s.more_open.as_deref() == Some(leave_row.as_str()) {
                            s.more_open = None;
                        }
                    });
                }
            }
        }))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, window, cx| this.rebase_press(&oid_press, index, e, window, cx)))
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .child(handle)
        .child(chips_el)
        .child(conflict_el)
        .child(div().flex().flex_none().w(px(ACTION_W)).h(px(ROW_H)).items_center().child(action_button(s, row, selected, cx)))
        .child(fold_el)
        .child(avatar)
        .child(summary)
        .child(dot)
        .when(chip_over, |d| d.child(div().absolute().top_0().left_0().size_full().border_2().border_color(theme::blue())))
        .children(count.map(|n| div().absolute().right(px(40.)).top(px(5.)).h(px(20.)).px(px(8.)).rounded(px(10.)).text_size(px(11.)).line_height(px(20.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).bg(theme::mix(theme::blue(), 0.45, theme::panel_bg())).shadow(crate::ui::shadow(theme::shadow_popup(), 1., 4.)).child(format!("{n} commits"))));
    let mut row_el = div().relative().flex_none().child(crate::ui::anchor(key, body));
    if let Some(add) = s.add_branch.as_ref().filter(|a| a.row == row.oid) {
        row_el = row_el.child(add_input(add));
    }
    if in_group {
        deferred(row_el).with_priority(1).into_any_element()
    } else {
        row_el.into_any_element()
    }
}

fn add_input(add: &AddBranch) -> impl IntoElement + use<> {
    div()
        .absolute()
        .left(px(28.))
        .top(px(3.))
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(div().w(px(120.)).h(px(24.)).px(px(6.)).flex().items_center().text_size(px(12.)).text_color(theme::text_selected()).bg(theme::wip_input_bg()).border_1().border_color(theme::focus_outline()).rounded(px(3.)).child(add.input.clone()))
        .children(add.error.clone().map(|e| div().px(px(6.)).py(px(2.)).text_size(px(11.)).whitespace_nowrap().text_color(theme::text_selected()).bg(theme::mix(theme::red(), 0.35, theme::panel_bg1())).rounded(px(3.)).shadow(crate::ui::shadow(theme::shadow_popup(), 1., 4.)).child(e)))
}

fn base_row(s: &RebaseSession, chips: (&EditorState, &Grouping), shown: bool, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let base = &s.state.base;
    let group: SharedString = "irow-base".into();
    let hover_bg = theme::mix(theme::blue(), if shown { 0.3 } else { 0.1 }, theme::app_bg());
    let row = div()
        .id("irebase-base")
        .group(group.clone())
        .flex()
        .flex_none()
        .items_start()
        .gap(px(6.))
        .h(px(ROW_H + 1.))
        .pl(px(4.))
        .pr(px(10.))
        .border_t_1()
        .border_color(theme::section_border())
        .text_color(theme::text_normal())
        .bg(if shown { theme::mix(theme::blue(), 0.24, theme::app_bg()) } else { theme::app_bg() })
        .hover(move |d| d.bg(hover_bg))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.rebase_inspect_base(cx)))
        .child(div().flex_none().w(px(18.)))
        .child(div().opacity(0.6).child(chip_column(s, chips.0, chips.1, &base.oid, group, cx)))
        .child(div().flex_none().w(px(16.)))
        .child(div().flex().flex_none().w(px(ACTION_W)).h(px(ROW_H)).items_center().pl(px(8.)).opacity(0.6).font_family(crate::app::MONO_FONT).text_size(px(11.)).text_color(theme::text_secondary()).child(short(&base.oid)))
        .child(div().flex_none().w(px(16.)))
        .child(div().flex_none().w(px(22.)))
        .child(div().flex_1().min_w_0().h(px(ROW_H)).line_height(px(ROW_H)).truncate().opacity(0.6).child(base.summary.clone()));
    crate::ui::anchor("irebase-base-row", row)
}

fn history_button(id: &'static str, glyph: &'static str, tooltip: String, off: bool, back: bool, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    tip(
        tooltip,
        crate::ui::anchor(
            id,
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(22.))
                .rounded(px(3.))
                .when(off, |d| d.opacity(0.4))
                .when(!off, |d| d.cursor_pointer().hover(|d| d.bg(theme::control_hover_bg())))
                .on_click(cx.listener(move |this, _, _, cx| this.rebase_undo(back, cx)))
                .child(icon(glyph, 15., theme::text_secondary())),
        ),
    )
}

fn title_chip(name: &str, lane: Hsla) -> Div {
    div().flex_none().mx(px(2.)).px(px(5.)).py(px(1.)).rounded(px(3.)).text_size(px(12.)).line_height(px(17.)).text_color(theme::text_selected()).bg(theme::mix(lane, 0.3, transparent_black())).child(name.to_string())
}

/// The editor, in the graph's place.
pub fn rebase_panel(tab: &Tab, width: f32, window: &Window, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let s = tab.rebase_shown()?;
    let state = &s.state;
    let g = grouping(&state.rows);
    let why = problems(state, &g);
    let edited = dirty(state);
    let drag = s.row_drag.as_ref().filter(|d| d.active);
    let chip_view;
    let chip_g;
    let chips: (&EditorState, &Grouping) = match drag {
        // The chips show where they'd end.
        Some(d) => {
            let mut view = state.clone();
            move_rows(&mut view, &d.oids, d.to);
            chip_g = grouping(&view.rows);
            chip_view = view;
            (&chip_view, &chip_g)
        }
        None => (state, &g),
    };
    let selected_sha = tab.selected.and_then(|i| tab.graph.as_ref()?.rows.get(i)).map(|r| r.id.to_string());
    let base_shown = state.selected.is_empty() && selected_sha.as_deref() == Some(state.base.oid.as_str());
    let (undo_tip, redo_tip) = if s.editing.is_some() { (FINISH_MESSAGE.to_string(), FINISH_MESSAGE.to_string()) } else { (format!("Undo the last plan change ({})", crate::commands::display_chord("Mod+Z")), format!("Redo the plan change ({})", crate::commands::display_chord("Mod+Shift+Z"))) };
    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(12.))
        .h(px(45.))
        .px(px(10.))
        .bg(theme::file_bar_bg())
        .border_b_1()
        .border_color(theme::section_border())
        .child(div().flex_none().font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).child("Interactive Rebase"))
        .child(div().flex().flex_shrink(1.).min_w_0().items_center().overflow_hidden().whitespace_nowrap().text_color(theme::text_secondary()).child("Rebasing ").child(title_chip(&state.branch, lane_or(s.chip_color(&state.branch), 0))).child(" onto ").child(title_chip(&state.base.name, lane_or(s.chip_color(&state.base.name), 1))))
        .when(state.merges > 0, |d| d.child(div().flex().min_w_0().items_center().gap(px(4.)).whitespace_nowrap().overflow_hidden().text_color(theme::orange()).child(icon("triangle-alert", 13., theme::orange())).child(flatten_warning(state.merges))))
        .when_some(s.prediction.note.clone().filter(|_| !s.prediction.pending), |d, n| d.child(div().min_w_0().truncate().text_color(theme::text_secondary()).child(n)))
        .child(div().flex().flex_none().ml_auto().gap(px(2.)).child(history_button("irebase-undo", "undo-2", undo_tip, s.editing.is_some() || s.past.is_empty(), true, cx)).child(history_button("irebase-redo", "redo-2", redo_tip, s.editing.is_some() || s.future.is_empty(), false, cx)))
        .child(crate::ui::anchor("rebase-cancel-top", neutral("rebase-cancel-top-b", "Cancel", false).on_click(cx.listener(|this, _, window, cx| this.cancel_rebase(Origin::control("rebase-cancel-top"), window, cx)))));
    let moved = s.moved.clone().map(|m| {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .py(px(6.))
            .text_color(theme::text_selected())
            .bg(theme::mix(theme::orange(), 0.18, theme::panel_bg()))
            .border_b_1()
            .border_color(theme::section_border())
            .child(format!("The plan is out of date: {m}."))
            .child(crate::ui::anchor("rebase-reload", neutral("rebase-reload-b", "Reload", false).on_click(cx.listener(|this, _, _, cx| this.reload_rebase(cx)))))
    });
    let mut rows = div().id("irebase-rows").flex().flex_col().flex_1().min_h_0().overflow_y_scroll().track_scroll(&s.scroll).py(px(4.));
    for (i, row) in state.rows.iter().enumerate() {
        rows = rows.child(row_view(s, i, row, &g, chips, window, cx));
    }
    rows = rows.child(base_row(s, chips, base_shown, cx));
    let keys_w = width - 270.;
    let narrow = keys_w <= 330.;
    let mut hints = div().flex().flex_1().min_w_0().flex_wrap().gap_x(px(12.)).gap_y(px(4.)).items_center().text_size(px(11.)).text_color(theme::text_secondary());
    let mut hint_tip = Vec::new();
    for (chords, label, minor) in HINTS {
        hint_tip.push(format!("{} {label}", chords.iter().map(|c| crate::commands::display_chord(c)).collect::<Vec<_>>().join("/")));
        if minor && narrow {
            continue;
        }
        let mut hint = div().flex().flex_none().items_center().gap(px(5.)).whitespace_nowrap();
        for (i, c) in chords.iter().enumerate() {
            if i > 0 {
                hint = hint.child("/");
            }
            hint = hint.child(crate::ui::controls::keycaps(&crate::commands::display_chord(c)));
        }
        hints = hints.child(hint.child(label));
    }
    let start_tip = why.first().cloned().unwrap_or_else(|| format!("Rebase {} onto {}", state.branch, state.base.name));
    let blocked = !why.is_empty();
    let footer = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .h(px(49.))
        .px(px(10.))
        .bg(theme::file_bar_bg())
        .border_t_1()
        .border_color(theme::section_border())
        .child(tip(hint_tip.join(", "), hints))
        .child(crate::ui::anchor("rebase-reset", neutral("rebase-reset-b", "Reset", !edited).when(edited, |d| d.on_click(cx.listener(|this, _, _, cx| this.with_rebase(cx, |s, _| s.edit(reset)))))))
        .child(crate::ui::anchor("rebase-cancel", neutral("rebase-cancel-b", "Cancel", false).on_click(cx.listener(|this, _, window, cx| this.cancel_rebase(Origin::control("rebase-cancel"), window, cx)))))
        .child(tip(start_tip, crate::ui::anchor("rebase-start", start_button(blocked, cx))));
    let panel = div()
        .id("irebase")
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .bg(theme::app_bg())
        .track_focus(&s.focus)
        .key_context("Rebase")
        .on_action(cx.listener(|this, _: &crate::app::SelectNext, window, cx| {
            this.rebase_key(&Keystroke::parse("down").expect("a key"), window, cx);
        }))
        .on_action(cx.listener(|this, _: &crate::app::SelectPrevious, window, cx| {
            this.rebase_key(&Keystroke::parse("up").expect("a key"), window, cx);
        }))
        .on_action(cx.listener(|this, _: &crate::app::ClearSelection, window, cx| this.cancel_rebase(Origin::control("rebase-cancel"), window, cx)))
        .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
            if this.rebase_key(&e.keystroke, window, cx) {
                cx.stop_propagation();
            }
        }))
        .child(header)
        .children(moved)
        .child(rows)
        .child(footer);
    Some(panel.into_any_element())
}

fn start_button(blocked: bool, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    positive("rebase-start-b", "Start Rebase", blocked).when(!blocked, |d| d.on_click(cx.listener(|this, _, _, cx| this.start_rebase(Origin::control("rebase-start"), cx))))
}

/// The chip being dragged, under the pointer.
pub fn chip_ghost(tab: &Tab) -> Option<AnyElement> {
    let s = tab.rebase_shown()?;
    let d = s.chip_drag.as_ref().filter(|d| d.active)?;
    let lane = lane_or(s.chip_color(&d.branch), 0);
    Some(div().absolute().left(d.at.x + px(10.)).top(d.at.y + px(6.)).h(px(CHIP_H)).max_w(px(240.)).truncate().px(px(5.)).rounded(px(3.)).line_height(px(CHIP_H)).text_size(px(12.)).text_color(theme::text_selected()).bg(theme::mix(lane, 0.45, theme::panel_bg())).shadow(crate::ui::shadow(theme::shadow_popup(), 1., 4.)).child(d.branch.clone()).into_any_element())
}

impl GitVolt {
    fn rebase_press(&mut self, oid: &str, index: usize, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.tabs.get_mut(self.active).and_then(Tab::rebase_shown_mut) else { return };
        window.focus(&s.focus.clone(), cx);
        if e.click_count == 2 {
            s.row_drag = None;
            return self.start_rebase_message(oid.to_string(), cx);
        }
        let ui = crate::ui::Ui::get(cx);
        let all: Vec<String> = s.state.rows.iter().map(|r| r.oid.clone()).collect();
        let heights: Vec<f32> = all.iter().map(|o| ui.anchor(&format!("irow:{o}")).map_or(ROW_H, |b| b.size.height.as_f32())).collect();
        let picked = &s.state.selected;
        let group: Vec<usize> = if picked.len() > 1 && picked.iter().any(|o| o == oid) { all.iter().enumerate().filter(|(_, o)| picked.contains(o)).map(|(i, _)| i).collect() } else { vec![index] };
        let oids: Vec<String> = group.iter().map(|i| all[*i].clone()).collect();
        let home = group_target(&heights, &group, index, 0.);
        s.row_drag = Some(RowDrag { from: index, group, oids, home, to: home, dy: 0., heights, start: e.position, active: false, oid: oid.to_string(), shift: e.modifiers.shift, ctrl: e.modifiers.control || e.modifiers.platform });
        cx.notify();
    }

    fn start_chip_drag(&mut self, branch: String, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(s) = self.tabs.get_mut(self.active).and_then(Tab::rebase_shown_mut) else { return };
        s.chip_drag = Some(ChipDrag { branch, start: at, at, over: None, active: false });
        s.row_drag = None;
        cx.notify();
    }

    /// The hidden chips' menus, one submenu each: the "+N" pill's click and right-click.
    fn rebase_hidden_rows(&self, hidden: &[String], cx: &mut Context<Self>) -> Vec<MenuRow> {
        let weak = cx.weak_entity();
        hidden.iter().map(|b| MenuRow::submenu(format!("irebase.chip.more.{b}"), b.clone(), "git-branch", "A branch on this commit", self.rebase_chip_rows(b, &weak))).collect()
    }

    fn open_add_branch(&mut self, row: String, cx: &mut Context<Self>) {
        let input = cx.new(|cx| TextInput::new(cx).claim_escape());
        let repo = self.tabs.get(self.active).map_or(0, |t| t.id);
        cx.subscribe(&input, move |this, _, e: &InputEvent, cx| this.add_branch_event(repo, e, cx)).detach();
        let focus = input.read(cx).focus.clone();
        let Some(s) = self.tabs.get_mut(self.active).and_then(Tab::rebase_shown_mut) else { return };
        s.add_branch = Some(AddBranch { row, input, error: None });
        self.focus_next_frame = Some(focus);
        cx.notify();
    }

    fn add_branch_event(&mut self, repo: u32, e: &InputEvent, cx: &mut Context<Self>) {
        let Some(s) = self.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.rebase.as_mut()) else { return };
        let Some(add) = s.add_branch.as_mut() else { return };
        match e {
            InputEvent::Changed => add.error = None,
            InputEvent::Submit { .. } => {
                let name = add.input.read(cx).text().to_string();
                let row = add.row.clone();
                let mut next = s.state.clone();
                match add_chip(&mut next, &name, &row) {
                    Err(why) => {
                        if let Some(add) = s.add_branch.as_mut() {
                            add.error = Some(why);
                        }
                    }
                    Ok(()) => {
                        s.edit(|st| *st = next);
                        s.add_branch = None;
                        let focus = s.focus.clone();
                        self.focus_next_frame = Some(focus);
                    }
                }
            }
            InputEvent::Escape => {
                s.add_branch = None;
                let focus = s.focus.clone();
                self.focus_next_frame = Some(focus);
            }
            _ => {}
        }
        cx.notify();
    }
}
