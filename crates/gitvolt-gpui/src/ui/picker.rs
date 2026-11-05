//! `ui/RefPicker.tsx`: a small searchable list anchored under a button: the toolbar's branch picker, Set upstream, the
//! graph header's pin. Typing filters by substring; ↑, ↓ move, Enter picks, Esc closes, a press outside closes it. Opened
//! by a press: a release on an item picks it. While it's open it owns the keyboard, as a menu does.

use super::Ui;
use super::controls::clearable;
use super::input::{InputEvent, TextInput};
use super::tooltip::tip;
use crate::app::icon;
use crate::theme;
use gpui::prelude::*;
use gpui::*;
use std::rc::Rc;

const WIDTH: f32 = 300.;
const MAX_H: f32 = 360.;
const ROW_H: f32 = 24.;
const HEAD_H: f32 = 30.2 + 12.;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PickItem {
    pub id: SharedString,
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub icon: Option<&'static str>,
    pub current: bool,
    pub tooltip: Option<SharedString>,
}

type OnPick = Rc<dyn Fn(&PickItem, &mut Window, &mut App)>;

/// The toggle right of the search box (the branch picker's alphabetical or recent).
#[derive(Clone)]
pub struct PickToggle {
    pub icon: &'static str,
    pub label: SharedString,
    pub run: super::Action,
}

pub struct PickRequest {
    /// What opened it: a press on it closes it, and it doesn't reopen.
    pub key: Option<SharedString>,
    /// The box it opens under.
    pub anchor: Bounds<Pixels>,
    pub items: Vec<PickItem>,
    pub placeholder: SharedString,
    pub toggle: Option<PickToggle>,
    pub on_pick: OnPick,
}

pub struct Picker {
    key: Option<SharedString>,
    anchor: Bounds<Pixels>,
    items: Rc<Vec<PickItem>>,
    toggle: Option<PickToggle>,
    on_pick: OnPick,
    input: Entity<TextInput>,
    shown: Rc<Vec<usize>>,
    cursor: usize,
    list: UniformListScrollHandle,
    return_to: Option<FocusHandle>,
    _keys: Subscription,
    bounds: Bounds<Pixels>,
}

/// `shown`: the items whose label holds the query.
pub fn filtered(items: &[PickItem], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    (0..items.len()).filter(|i| q.is_empty() || items[*i].label.to_lowercase().contains(&q)).collect()
}

fn current_of(items: &[PickItem], shown: &[usize]) -> usize {
    shown.iter().position(|i| items[*i].current).unwrap_or(0)
}

/// Opens it. A press on `key`'s control that closed it swallows this.
pub fn open(req: PickRequest, window: &mut Window, cx: &mut App) {
    if let (Some(k), Some(sw)) = (&req.key, &Ui::get(cx).swallow_anchor)
        && k == sw
    {
        return;
    }
    super::menu::close(window, cx);
    super::tooltip::hide(cx);
    let placeholder = req.placeholder;
    let input = cx.new(|cx| TextInput::new(cx).placeholder(placeholder).claim_arrows().claim_escape());
    let keys = cx.subscribe(&input, |input, e: &InputEvent, cx| match e {
        InputEvent::Changed => {
            let q = input.read(cx).text().to_string();
            if let Some(p) = Ui::get_mut(cx).picker.as_mut() {
                let shown = filtered(&p.items, &q);
                p.cursor = current_of(&p.items, &shown);
                p.shown = Rc::new(shown);
                p.list.scroll_to_item(p.cursor, ScrollStrategy::Nearest);
            }
            cx.refresh_windows();
        }
        InputEvent::Arrow { down, .. } => {
            if let Some(p) = Ui::get_mut(cx).picker.as_mut() {
                let last = p.shown.len().saturating_sub(1);
                p.cursor = if *down { (p.cursor + 1).min(last) } else { p.cursor.saturating_sub(1) };
                p.list.scroll_to_item(p.cursor, ScrollStrategy::Nearest);
            }
            cx.refresh_windows();
        }
        InputEvent::Submit { .. } => {
            cx.defer(|cx| {
                let Some(w) = cx.active_window() else { return };
                w.update(cx, |_, window, cx| pick_cursor(window, cx)).ok();
            });
        }
        InputEvent::Escape => {
            cx.defer(|cx| {
                let Some(w) = cx.active_window() else { return };
                w.update(cx, |_, window, cx| close(window, cx)).ok();
            });
        }
        _ => {}
    });
    let shown = filtered(&req.items, "");
    let cursor = current_of(&req.items, &shown);
    let list = UniformListScrollHandle::new();
    list.scroll_to_item(cursor, ScrollStrategy::Center);
    let return_to = window.focused(cx);
    window.focus(&input.read(cx).focus.clone(), cx);
    Ui::get_mut(cx).picker = Some(Picker { key: req.key, anchor: req.anchor, items: Rc::new(req.items), toggle: req.toggle, on_pick: req.on_pick, input, shown: Rc::new(shown), cursor, list, return_to, _keys: keys, bounds: Bounds::default() });
    cx.refresh_windows();
}

/// Opens it under a control marked with `ui::anchor(key, ..)`, or at (`fallback`).
pub fn open_below(key: impl Into<SharedString>, items: Vec<PickItem>, placeholder: impl Into<SharedString>, toggle: Option<PickToggle>, on_pick: impl Fn(&PickItem, &mut Window, &mut App) + 'static, window: &mut Window, cx: &mut App) {
    let key: SharedString = key.into();
    let anchor = Ui::get(cx).anchor(&key).unwrap_or_else(|| Bounds::new(point(px(200.), px(100.)), size(px(0.), px(0.))));
    open(PickRequest { key: Some(key), anchor, items, placeholder: placeholder.into(), toggle, on_pick: Rc::new(on_pick) }, window, cx);
}

pub fn is_open(cx: &App) -> bool {
    Ui::get(cx).picker.is_some()
}

pub fn is_open_on(key: &str, cx: &App) -> bool {
    Ui::get(cx).picker.as_ref().is_some_and(|p| p.key.as_deref() == Some(key))
}

/// For the toggle: the items anew, the cursor back on the current one.
pub fn set_items(items: Vec<PickItem>, toggle: Option<PickToggle>, cx: &mut App) {
    let Some(q) = Ui::get(cx).picker.as_ref().map(|p| p.input.read(cx).text().to_string()) else { return };
    if let Some(p) = Ui::get_mut(cx).picker.as_mut() {
        let shown = filtered(&items, &q);
        p.cursor = current_of(&items, &shown);
        p.shown = Rc::new(shown);
        p.items = Rc::new(items);
        p.toggle = toggle;
        p.list.scroll_to_item(p.cursor, ScrollStrategy::Center);
    }
    cx.refresh_windows();
}

pub fn close(window: &mut Window, cx: &mut App) {
    let Some(p) = Ui::get_mut(cx).picker.take() else { return };
    if let Some(back) = p.return_to {
        window.focus(&back, cx);
    }
    cx.refresh_windows();
}

fn pick_at(index: usize, window: &mut Window, cx: &mut App) {
    let Some(p) = Ui::get(cx).picker.as_ref() else { return };
    let Some(item) = p.items.get(index).cloned() else { return };
    let run = p.on_pick.clone();
    close(window, cx);
    run(&item, window, cx);
}

fn pick_cursor(window: &mut Window, cx: &mut App) {
    let Some(p) = Ui::get(cx).picker.as_ref() else { return };
    let Some(index) = p.shown.get(p.cursor).copied() else { return };
    pick_at(index, window, cx);
}

/// For a script: where an item's row is, and the picker's items as text.
pub fn row_point(text: &str, cx: &App) -> Option<Point<Pixels>> {
    let p = Ui::get(cx).picker.as_ref()?;
    let row = p.shown.iter().position(|i| p.items[*i].label.contains(text))?;
    let top = p.bounds.origin.y.as_f32() + HEAD_H + row as f32 * ROW_H + ROW_H / 2.;
    Some(point(p.bounds.origin.x + px(40.), px(top)))
}

pub fn shown(cx: &App) -> Vec<String> {
    Ui::get(cx).picker.as_ref().map_or(Vec::new(), |p| p.shown.iter().map(|i| p.items[*i].label.to_string()).collect())
}

/// A press outside closes it; on what opened it, the opener's own handler doesn't open it again.
pub(super) fn on_press(e: &MouseDownEvent, window: &mut Window, cx: &mut App) {
    let Some(p) = Ui::get(cx).picker.as_ref() else {
        return;
    };
    if p.bounds.contains(&e.position) {
        return;
    }
    let on_opener = p.key.clone().filter(|_| p.anchor.contains(&e.position));
    if let Some(k) = on_opener {
        Ui::get_mut(cx).swallow_anchor = Some(k);
    }
    close(window, cx);
}

pub(super) fn element(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let p = Ui::get(cx).picker.as_ref()?;
    let vp = window.viewport_size();
    let (items, shown, cursor) = (p.items.clone(), p.shown.clone(), p.cursor);
    let count = shown.len();
    let list_h = if count == 0 { 30.2 } else { (count as f32 * ROW_H + 4.).min(MAX_H - 2. - HEAD_H) };
    let h = HEAD_H + list_h + 2.;
    let left = (vp.width.as_f32() - WIDTH - 4.).min(p.anchor.left().as_f32()).max(4.);
    let top = (vp.height.as_f32() - h - 4.).min(p.anchor.bottom().as_f32() + 4.).max(4.);
    let input = p.input.clone();
    let toggle = p.toggle.clone();
    let list_handle = p.list.clone();
    let on_clear = input.clone();
    let head = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .m(px(6.))
        .child(clearable(
            "picker-input",
            &input,
            move |_, cx| {
                on_clear.update(cx, |i, cx| i.set_text("", cx));
                let items = Ui::get(cx).picker.as_ref().map(|p| p.items.clone());
                if let (Some(items), Some(p)) = (items, Ui::get_mut(cx).picker.as_mut()) {
                    let all = filtered(&items, "");
                    p.cursor = current_of(&items, &all);
                    p.shown = Rc::new(all);
                }
                cx.refresh_windows();
            },
            window,
            cx,
        )
        .h(px(30.2)))
        .children(toggle.map(|t| {
            let run = t.run.clone();
            tip(t.label.clone(), div().id("picker-toggle").flex().flex_none().size(px(22.)).items_center().justify_center().rounded(px(3.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            }).on_click(move |_, window, cx| run(window, cx)).child(icon(t.icon, 14., theme::text_secondary())))
        }));
    let body: AnyElement = if count == 0 {
        div().px(px(10.)).py(px(6.)).text_color(theme::text_disabled()).child("No matches").into_any_element()
    } else {
        uniform_list("picker-list", count, move |range, _, _| {
            range
                .map(|row| {
                    let i = shown[row];
                    let item = &items[i];
                    let on = row == cursor;
                    let color = if on { theme::text_selected() } else { theme::text_normal() };
                    let el = div()
                        .id(("picker-item", i))
                        .flex()
                        .w_full()
                        .h(px(ROW_H))
                        .items_center()
                        .gap(px(6.))
                        .px(px(10.))
                        .whitespace_nowrap()
                        .cursor_pointer()
                        .text_color(color)
                        .when(on, |d| d.bg(theme::selected_row()))
                        .on_mouse_move(move |_, _, cx| {
                            if let Some(p) = Ui::get_mut(cx).picker.as_mut()
                                && p.cursor != row
                            {
                                p.cursor = row;
                                cx.refresh_windows();
                            }
                        })
                        .on_mouse_up(MouseButton::Left, move |_, window, cx| pick_at(i, window, cx))
                        .children(item.icon.map(|g| icon(g, 13., color)))
                        .child(div().flex_1().min_w_0().truncate().when(item.current, |d| d.font_weight(FontWeight::SEMIBOLD)).child(item.label.clone()))
                        .children(item.detail.clone().map(|d| div().flex_none().text_size(px(11.)).text_color(theme::text_secondary()).child(d)));
                    match &item.tooltip {
                        Some(t) => tip(t.clone(), el).into_any_element(),
                        None => el.into_any_element(),
                    }
                })
                .collect()
        })
        .track_scroll(&list_handle)
        .h(px(list_h - 4.))
        .into_any_element()
    };
    let bounds = Bounds::new(point(px(left), px(top)), size(px(WIDTH), px(h)));
    if let Some(p) = Ui::get_mut(cx).picker.as_mut() {
        p.bounds = bounds;
    }
    Some(
        div()
            .id("picker")
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(WIDTH))
            .h(px(h))
            .occlude()
            .flex()
            .flex_col()
            .bg(theme::panel_bg1())
            .border_1()
            .border_color(theme::section_border())
            .rounded(px(6.))
            .shadow(super::shadow(theme::shadow_popup(), 6., 20.))
            .text_color(theme::text_normal())
            .child(head)
            .child(body)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn item(label: &str, current: bool) -> PickItem {
        PickItem { id: label.to_string().into(), label: label.to_string().into(), current, ..Default::default() }
    }

    #[test]
    fn typing_filters_and_the_cursor_starts_on_what_is_current() {
        let items = vec![item("main", false), item("feature/one", true), item("Feature/two", false)];
        assert_eq!(filtered(&items, ""), [0, 1, 2]);
        assert_eq!(filtered(&items, " FEAT "), [1, 2]);
        assert_eq!(current_of(&items, &filtered(&items, "")), 1);
        assert_eq!(current_of(&items, &filtered(&items, "main")), 0);
    }
}
