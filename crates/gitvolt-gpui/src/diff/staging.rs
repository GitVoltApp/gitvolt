//! The staging controls of a WIP text diff (`ui/src/diff/hunkActions.tsx`, `wipHunks.ts`, `LineActionBar.tsx`):
//! Hunk mode's header row above each hunk, the gutter's + or − on the hovered changed line, the right-click's
//! rows and the bar under a selection that covers changed lines, over the hunks the core read for this show.

use super::model::{Row, RowKind};
use super::view::{DiffState};
use crate::app::{GitVolt, icon};
use crate::details::Origin;
use crate::theme;
use crate::ui::arm::Origin as ArmOrigin;
use crate::ui::menu::MenuRow;
use crate::ui::tooltip::tip;
use crate::wip::COMMIT_QUEUED;
use crate::write::{WriteCtx, call};
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};

/// Hunk and line buttons while the working copy has unsaved edits.
pub const SAVE_FIRST: &str = "Save first";
pub const HUNK_ROW_H: f32 = 24.;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub del: Vec<u32>,
    pub add: Vec<u32>,
}

/// What the core says of the shown file's hunks.
#[derive(Debug, Clone, PartialEq)]
pub struct WipHunks {
    pub base: Value,
    pub hunks: Vec<Hunk>,
    pub refused: Option<String>,
}

impl WipHunks {
    pub fn of(v: &Value) -> Self {
        let nums = |a: &Value| a.as_array().map_or(Vec::new(), |l| l.iter().filter_map(|n| n.as_u64().map(|n| n as u32)).collect());
        let hunks: Vec<Hunk> = v["hunks"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .map(|h| Hunk {
                old_start: h["oldStart"].as_u64().unwrap_or(0) as u32,
                old_lines: h["oldLines"].as_u64().unwrap_or(0) as u32,
                new_start: h["newStart"].as_u64().unwrap_or(0) as u32,
                new_lines: h["newLines"].as_u64().unwrap_or(0) as u32,
                del: nums(&h["del"]),
                add: nums(&h["add"]),
            })
            .collect();
        let binary = v["binary"].as_bool() == Some(true);
        let refused = v["refused"].as_str().map(str::to_string);
        // None for a binary file or one git refuses to stage by hunk.
        let usable = if binary || refused.is_some() { Vec::new() } else { hunks };
        Self { base: v["base"].clone(), hunks: usable, refused }
    }
}

/// 1-based, inclusive, as the core takes them.
pub fn to_ranges(lines: &[u32]) -> Vec<(u32, u32)> {
    let mut sorted = lines.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut out: Vec<(u32, u32)> = Vec::new();
    for n in sorted {
        match out.last_mut() {
            Some(last) if last.1 + 1 == n => last.1 = n,
            _ => out.push((n, n)),
        }
    }
    out
}

fn ranges_json(lines: &[u32]) -> Value {
    Value::Array(to_ranges(lines).into_iter().map(|(start, end)| json!({ "start": start, "end": end })).collect())
}

pub fn lines_selection(old: &[u32], new: &[u32]) -> Value {
    json!({ "kind": "lines", "old": ranges_json(old), "new": ranges_json(new) })
}

pub fn hunk_selection(i: usize) -> Value {
    json!({ "kind": "hunks", "hunks": [i] })
}

/// The hunk whose range on a side holds `line` (its context lines included).
pub fn hunk_at(hunks: &[Hunk], old_side: bool, line: u32) -> Option<usize> {
    hunks.iter().position(|h| {
        let (start, n) = if old_side { (h.old_start, h.old_lines) } else { (h.new_start, h.new_lines) };
        if n == 0 { line == start } else { line >= start && line < start + n }
    })
}

/// Ctrl+Shift+D's selection: the changed lines selected, else the hunk the cursor is in. A pure deletion's
/// hunk is numbered by the line before it, and Next change puts the cursor on the line after it.
pub fn at_cursor(hunks: &[Hunk], selected: (&[u32], &[u32]), cursor: Option<(bool, u32)>) -> Option<Value> {
    if !selected.0.is_empty() || !selected.1.is_empty() {
        return Some(lines_selection(selected.0, selected.1));
    }
    let (old_side, line) = cursor?;
    let i = hunk_at(hunks, old_side, line).or_else(|| if old_side { None } else { hunks.iter().position(|h| h.new_lines == 0 && h.new_start + 1 == line) });
    i.map(hunk_selection)
}

/// git's header for a hunk.
pub fn hunk_header(h: &Hunk) -> String {
    format!("@@ -{},{} +{},{} @@", h.old_start, h.old_lines, h.new_start, h.new_lines)
}

/// A header row's place: after this 1-based new-side line (0: the top). An empty range is numbered by the
/// line before it.
pub fn zone_after(h: &Hunk) -> u32 {
    if h.new_lines == 0 { h.new_start } else { h.new_start.saturating_sub(1) }
}

/// Hunk mode's rows: a header above each hunk.
pub fn with_headers(rows: Vec<Row>, hunks: &[Hunk]) -> Vec<Row> {
    let mut places: Vec<(usize, usize)> = hunks
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let after = zone_after(h);
            let at = if after == 0 {
                0
            } else {
                let line = after - 1;
                match rows.iter().position(|r| r.new == Some(line) && !matches!(r.kind, RowKind::Hidden { .. })) {
                    Some(p) => p + 1,
                    // Folded away: below the fold.
                    None => rows.iter().rposition(|r| r.new.is_some_and(|n| n <= line)).map_or(0, |p| p + 1),
                }
            };
            (at, i)
        })
        .collect();
    places.sort_unstable();
    let mut out = Vec::with_capacity(rows.len() + places.len());
    let mut next = places.into_iter().peekable();
    for (i, r) in rows.into_iter().enumerate() {
        while let Some((_, hunk)) = next.next_if(|p| p.0 <= i) {
            out.push(Row { old: None, new: None, kind: RowKind::HunkHeader { hunk: hunk as u32 } });
        }
        out.push(r);
    }
    for (_, hunk) in next {
        out.push(Row { old: None, new: None, kind: RowKind::HunkHeader { hunk: hunk as u32 } });
    }
    out
}

/// The changed lines rows `from..=to` show, by side, 1-based. `side`: only that side (a split's selection).
pub fn changed_in_rows(rows: &[Row], from: usize, to: usize, hunks: &[Hunk], side: Option<bool>) -> (Vec<u32>, Vec<u32>) {
    let del: std::collections::HashSet<u32> = hunks.iter().flat_map(|h| h.del.iter().copied()).collect();
    let add: std::collections::HashSet<u32> = hunks.iter().flat_map(|h| h.add.iter().copied()).collect();
    let (mut old, mut new) = (Vec::new(), Vec::new());
    for r in rows.iter().take(to + 1).skip(from) {
        if let Some(n) = r.old.map(|n| n + 1).filter(|n| side != Some(false) && del.contains(n)) {
            old.push(n);
        }
        if let Some(n) = r.new.map(|n| n + 1).filter(|n| side != Some(true) && add.contains(n)) {
            new.push(n);
        }
    }
    (old, new)
}

fn lines_word(n: usize) -> String {
    format!("{n} {}", if n == 1 { "line" } else { "lines" })
}

/// The file's side in the working copy: whether it can be discarded here.
fn can_discard(diff: &DiffState) -> bool {
    diff.file.new["kind"] == "worktree"
}

/// Why the buttons can't act now.
fn reason_of(diff: &DiffState) -> Option<&'static str> {
    if diff.editing_dirty() {
        Some(SAVE_FIRST)
    } else if diff.commit_held {
        Some(COMMIT_QUEUED)
    } else {
        None
    }
}

impl GitVolt {
    /// Stages, unstages or discards a selection of the open file, behind the base it was shown with.
    pub fn run_patch(&mut self, selection: Value, discard: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(diff) = tab.diff.as_ref() else { return };
        let (Some(h), Origin::Wip { worktree, staged }) = (diff.hunks.as_ref(), &diff.file.origin) else { return };
        let (repo, worktree, staged, path, base) = (tab.id, worktree.clone(), *staged, diff.file.path.to_string(), h.base.clone());
        let ctx = WriteCtx { repo, worktree: worktree.clone() };
        self.staging_write(
            &ctx,
            ArmOrigin::None,
            false,
            move || {
                if discard {
                    call("discard", json!({ "repo": repo, "worktree": worktree, "scope": { "kind": "patch", "path": path, "selection": selection, "base": base } }))
                } else {
                    call("stagePatch", json!({ "repo": repo, "worktree": worktree, "path": path, "staged": staged, "selection": selection, "base": base }))
                }
            },
            cx,
        );
    }

    /// Ctrl+Shift+D: the selected lines, else the hunk at the cursor.
    pub fn stage_at_cursor(&mut self, cx: &mut Context<Self>) {
        let Some(diff) = self.tabs.get(self.active).and_then(|t| t.diff.as_ref()) else { return };
        if let Some(why) = reason_of(diff) {
            return self.show_toast(why, cx);
        }
        let Some(h) = diff.hunks.as_ref() else { return };
        let (old, new) = diff.selected_changes().unwrap_or_default();
        let cursor = diff.rows.get(diff.cursor).and_then(|r| match (r.new, r.old) {
            (Some(n), _) => Some((false, n + 1)),
            (None, Some(n)) => Some((true, n + 1)),
            _ => None,
        });
        if let Some(selection) = at_cursor(&h.hunks, (&old, &new), cursor) {
            self.run_patch(selection, false, cx);
        }
    }

    /// The right-click's rows: for the changed lines the selection covers (else the clicked line), Stage,
    /// Discard or Unstage; then the clicked line's hunk.
    pub fn staging_menu_rows(&self, row: usize, old_side: bool, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(diff) = self.tabs.get(self.active).and_then(|t| t.diff.as_ref()) else { return Vec::new() };
        let (Some(h), Origin::Wip { staged, .. }) = (diff.hunks.as_ref(), &diff.file.origin) else { return Vec::new() };
        if h.hunks.is_empty() || diff.view != super::view::ViewKind::Diff {
            return Vec::new();
        }
        let Some(r) = diff.rows.get(row) else { return Vec::new() };
        let split = diff.mode == super::model::Mode::Split;
        let clicked_old = if split { old_side } else { r.new.is_none() };
        let selected = diff.selected_changes().filter(|(o, n)| !o.is_empty() || !n.is_empty()).filter(|_| diff.selection_holds(row));
        let (old, new) = selected.unwrap_or_else(|| changed_in_rows(&diff.rows, row, row, &h.hunks, split.then_some(old_side)));
        let reason = reason_of(diff);
        let staged = *staged;
        let discardable = can_discard(diff);
        let action = |id: &'static str, label: String, glyph: &'static str, tooltip: String, selection: Value, discard: bool| {
            let weak = this.clone();
            MenuRow::action(id, label, glyph, tooltip, move |_, cx| {
                let selection = selection.clone();
                weak.update(cx, |v, cx| v.run_patch(selection, discard, cx)).ok();
            })
            .disabled_if(reason)
        };
        let mut out = Vec::new();
        let count = old.len() + new.len();
        if count > 0 {
            let which = if count == 1 { "this line" } else { "these lines" };
            let sel = lines_selection(&old, &new);
            if staged {
                out.push(action("diff.unstageLines", format!("Unstage {which}"), "minus", format!("Move {which} back to Unstaged"), sel, false));
            } else {
                out.push(action("diff.stageLines", format!("Stage {which}"), "plus", format!("Stage {which} only"), sel.clone(), false));
                if discardable {
                    out.push(action("diff.discardLines", format!("Discard {which}"), "trash-2", format!("Discard {which} from the file (you can undo this)"), sel, true));
                }
            }
        }
        let line = if clicked_old { r.old } else { r.new }.map(|n| n + 1);
        if let Some(i) = line.and_then(|l| hunk_at(&h.hunks, clicked_old, l)) {
            if !out.is_empty() {
                out.push(MenuRow::Separator);
            }
            let sel = hunk_selection(i);
            if staged {
                out.push(action("diff.unstageHunk", "Unstage hunk".into(), "minus", "Move this hunk back to Unstaged".into(), sel, false));
            } else {
                out.push(action("diff.stageHunk", "Stage hunk".into(), "plus", "Stage this hunk".into(), sel.clone(), false));
                if discardable {
                    out.push(action("diff.discardHunk", "Discard hunk".into(), "trash-2", "Discard this hunk from the file (you can undo this)".into(), sel, true));
                }
            }
        }
        out
    }

    /// The selection settled: what its write will take (the no-newline tie included).
    pub fn read_selection_counts(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let repo = tab.id;
        let Some(diff) = tab.diff.as_mut() else { return };
        let picked = diff.selected_changes().filter(|(o, n)| !o.is_empty() || !n.is_empty());
        let Origin::Wip { worktree, staged } = diff.file.origin.clone() else { return };
        let Some((old, new)) = picked else {
            diff.bar_counts = None;
            return;
        };
        let (path, generation) = (diff.file.path.to_string(), diff.generation);
        let local = old.len() + new.len();
        diff.bar_counts = Some((local, local));
        if cfg!(test) {
            return;
        }
        let selection = lines_selection(&old, &new);
        let api = self.api.clone();
        let body = call("selectionLines", json!({ "repo": repo, "worktree": worktree, "path": path, "staged": staged, "selection": selection }));
        let read = self.rt.spawn(async move { api.dispatch(crate::write::request_of(body)?).await });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(c)) = read.await {
                this.update(cx, |this, cx| {
                    if let Some(diff) = this.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.diff.as_mut()).filter(|d| d.generation == generation && d.bar_counts.is_some()) {
                        diff.bar_counts = Some((c["apply"].as_u64().unwrap_or(0) as usize, c["discard"].as_u64().unwrap_or(0) as usize));
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }
}

/// `.hunk-button`, `.line-action`.
fn small_button(id: impl Into<ElementId>, label: impl Into<SharedString>, glyph: Option<&'static str>, positive: bool, reason: Option<&'static str>, on_click: impl Fn(&mut Window, &mut App) + 'static) -> impl IntoElement {
    let label: SharedString = label.into();
    let color = if positive { theme::green() } else { theme::red() };
    let group: SharedString = format!("hb-{label}").into();
    let hover_text = if positive { theme::text_on_accent() } else { color };
    tip(
        reason.map_or(label.clone(), SharedString::from),
        div()
            .id(id)
            .group(group.clone())
            .flex()
            .flex_none()
            .items_center()
            .gap(px(3.))
            .h(px(18.))
            .px(px(6.))
            .border_1()
            .border_color(color)
            .rounded(px(3.))
            .text_size(px(11.))
            .text_color(color)
            .when(reason.is_some(), |d| d.opacity(0.5))
            .when(reason.is_none(), |d| {
                d.cursor_pointer().hover(move |d| if positive { d.bg(color).text_color(hover_text) } else { d.bg(theme::mix(color, 0.18, transparent_black())) })
            })
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                if reason.is_none() {
                    on_click(window, cx);
                }
            })
            .children(glyph.map(|g| icon(g, 12., color).group_hover(group, move |s| s.text_color(hover_text))))
            .child(label),
    )
}

/// What floats over the text: the visible header rows, the hovered line's button, the selection's bar.
pub fn overlays(diff: &DiffState, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    let Some(h) = diff.hunks.as_ref() else { return Vec::new() };
    let Origin::Wip { staged, .. } = &diff.file.origin else { return Vec::new() };
    if h.hunks.is_empty() || diff.view != super::view::ViewKind::Diff || diff.layout.offsets.len() != diff.rows.len() + 1 {
        return Vec::new();
    }
    let staged = *staged;
    let reason = reason_of(diff);
    let discardable = can_discard(diff);
    let mut out = Vec::new();
    let weak = cx.weak_entity();
    let run = move |selection: Value, discard: bool| {
        let weak = weak.clone();
        move |_: &mut Window, cx: &mut App| {
            let selection = selection.clone();
            weak.update(cx, |v, cx| v.run_patch(selection, discard, cx)).ok();
        }
    };
    let (scroll, height) = (diff.scroll, diff.layout.height);
    let first = diff.layout.row_at(scroll);
    for ix in first..diff.rows.len() {
        let top = diff.layout.offsets[ix] - scroll;
        if top >= height {
            break;
        }
        let RowKind::HunkHeader { hunk } = diff.rows[ix].kind else { continue };
        let Some(header) = h.hunks.get(hunk as usize) else { continue };
        let sel = hunk_selection(hunk as usize);
        let mut buttons = div().flex().flex_none().gap(px(6.));
        if staged {
            buttons = buttons.child(small_button(("hunk-unstage", hunk as usize), "Unstage hunk", Some("minus"), false, reason, run(sel, false)));
        } else {
            if discardable {
                buttons = buttons.child(small_button(("hunk-discard", hunk as usize), "Discard hunk", Some("trash-2"), false, reason, run(sel.clone(), true)));
            }
            buttons = buttons.child(small_button(("hunk-stage", hunk as usize), "Stage hunk", Some("plus"), true, reason, run(sel, false)));
        }
        out.push(
            div()
                .absolute()
                .left(px(diff.gutter_w()))
                .right_0()
                .top(px(top))
                .h(px(HUNK_ROW_H))
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .bg(theme::panel_bg1())
                .border_t_1()
                .border_b_1()
                .border_color(theme::section_border())
                .cursor_default()
                .child(div().flex_1().min_w_0().truncate().font_family(super::view::FONT).text_size(px(12.)).text_color(theme::text_secondary()).child(hunk_header(header)))
                .child(buttons)
                .into_any_element(),
        );
    }
    // The gutter's button, on the hovered changed line.
    if let Some((row, _)) = diff.hover
        && let Some(r) = diff.rows.get(row)
    {
        let (x_old, x_new) = diff.glyph_columns();
        let old_on = r.old.map(|n| n + 1).filter(|n| h.hunks.iter().any(|h| h.del.contains(n)));
        let new_on = r.new.map(|n| n + 1).filter(|n| h.hunks.iter().any(|h| h.add.contains(n)));
        let top = diff.layout.offsets[row] - scroll;
        let glyph = |id: &'static str, x: f32, selection: Value, cx: &mut Context<GitVolt>| {
            let color = if staged { theme::red() } else { theme::green() };
            let label = match (staged, reason) {
                (_, Some(why)) => why.to_string(),
                (true, None) => "Unstage this line".to_string(),
                (false, None) => "Stage this line".to_string(),
            };
            let weak = cx.weak_entity();
            tip(
                label,
                div()
                    .id(id)
                    .absolute()
                    .left(px(x))
                    .top(px(top + (crate::metrics::line_h() - 14.) / 2.))
                    .flex()
                    .size(px(14.))
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .bg(color)
                    .text_color(theme::text_on_accent())
                    .text_size(px(13.))
                    .font_weight(FontWeight::BOLD)
                    .when(reason.is_some(), |d| d.opacity(0.45))
                    .when(reason.is_none(), |d| d.cursor_pointer())
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        if reason.is_none() {
                            let selection = selection.clone();
                            weak.update(cx, |v, cx| v.run_patch(selection, false, cx)).ok();
                        }
                    })
                    .child(if staged { "−" } else { "+" }),
            )
            .into_any_element()
        };
        if let Some(n) = old_on {
            out.push(glyph("line-glyph-old", x_old, lines_selection(&[n], &[]), cx));
        } else if let Some(n) = new_on {
            out.push(glyph("line-glyph-new", x_new, lines_selection(&[], &[n]), cx));
        }
    }
    // The bar, under a selection that covers changed lines.
    if let (Some((apply, discard)), Some(sel), true) = (diff.bar_counts, diff.selection, diff.drag == super::view::Drag::None)
        && apply > 0
        && let Some((old, new)) = diff.selected_changes()
    {
        let (_, to) = sel.range();
        let bottom = diff.layout.offsets.get(to.row + 1).copied().unwrap_or(0.) - scroll;
        let selection = lines_selection(&old, &new);
        let mut bar = div()
            .absolute()
            .left(px(diff.gutter_w() + 24.))
            .top(px(bottom + 4.))
            .flex()
            .gap(px(4.))
            .p(px(3.))
            .rounded(px(4.))
            .bg(theme::panel_bg1())
            .shadow(crate::ui::shadow(theme::shadow_popup(), 1., 4.));
        if staged {
            bar = bar.child(small_button("line-unstage", format!("Unstage {}", lines_word(apply)), None, false, reason, run(selection, false)));
        } else {
            bar = bar.child(small_button("line-stage", format!("Stage {}", lines_word(apply)), None, true, reason, run(selection.clone(), false)));
            if discardable {
                bar = bar.child(small_button("line-discard", format!("Discard {}", lines_word(discard)), None, false, reason, run(selection, true)));
            }
        }
        out.push(bar.into_any_element());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::testing::Fixture;
    use std::prelude::v1::test;

    fn h(old_start: u32, old_lines: u32, new_start: u32, new_lines: u32, del: &[u32], add: &[u32]) -> Hunk {
        Hunk { old_start, old_lines, new_start, new_lines, del: del.to_vec(), add: add.to_vec() }
    }

    #[test]
    fn lines_become_ranges() {
        assert_eq!(to_ranges(&[5, 3, 4, 9]), [(3, 5), (9, 9)]);
        assert_eq!(lines_selection(&[1], &[]), json!({ "kind": "lines", "old": [{ "start": 1, "end": 1 }], "new": [] }));
    }

    #[test]
    fn a_line_finds_its_hunk_context_included() {
        let hunks = [h(1, 4, 1, 5, &[2], &[2, 3]), h(10, 1, 11, 0, &[10], &[])];
        assert_eq!(hunk_at(&hunks, false, 5), Some(0));
        assert_eq!(hunk_at(&hunks, false, 6), None);
        assert_eq!(hunk_at(&hunks, true, 10), Some(1));
        assert_eq!(hunk_at(&hunks, false, 11), Some(1), "an empty range: the line it's numbered by");
    }

    #[test]
    fn the_key_takes_the_selected_lines_else_the_hunk_at_the_cursor() {
        let hunks = [h(1, 2, 1, 3, &[1], &[1, 2]), h(9, 1, 10, 0, &[9], &[])];
        assert_eq!(at_cursor(&hunks, (&[1], &[2]), Some((false, 1))).unwrap()["kind"], "lines");
        assert_eq!(at_cursor(&hunks, (&[], &[]), Some((false, 2))), Some(hunk_selection(0)));
        // A pure deletion: the cursor on the line after it.
        assert_eq!(at_cursor(&hunks, (&[], &[]), Some((false, 11))), Some(hunk_selection(1)));
        assert_eq!(at_cursor(&hunks, (&[], &[]), Some((false, 50))), None);
    }

    fn row(new: Option<u32>, old: Option<u32>, kind: RowKind) -> Row {
        Row { old, new, kind }
    }

    #[test]
    fn a_header_goes_above_its_hunk_and_a_pure_deletion_after_its_line() {
        let same = |n: u32| row(Some(n), Some(n), RowKind::Same);
        let rows = vec![same(0), same(1), row(None, Some(2), RowKind::Removed), same(3)];
        // Hunk from new line 2 (1-based): after line 1, the first row.
        let out = with_headers(rows.clone(), &[h(2, 2, 2, 1, &[3], &[])]);
        assert!(matches!(out[1].kind, RowKind::HunkHeader { hunk: 0 }));
        // A pure deletion numbered by new line 2.
        let out = with_headers(rows.clone(), &[h(3, 1, 2, 0, &[3], &[])]);
        assert!(matches!(out[2].kind, RowKind::HunkHeader { hunk: 0 }));
        // At the very top.
        let out = with_headers(rows, &[h(1, 1, 1, 1, &[1], &[1])]);
        assert!(matches!(out[0].kind, RowKind::HunkHeader { hunk: 0 }));
    }

    #[test]
    fn only_changed_lines_count_on_the_selections_side() {
        let hunks = [h(1, 3, 1, 3, &[2], &[2])];
        let rows = vec![row(Some(0), Some(0), RowKind::Same), row(None, Some(1), RowKind::Removed), row(Some(1), None, RowKind::Added), row(Some(2), Some(2), RowKind::Same)];
        assert_eq!(changed_in_rows(&rows, 0, 3, &hunks, None), (vec![2], vec![2]));
        assert_eq!(changed_in_rows(&rows, 0, 3, &hunks, Some(false)), (vec![], vec![2]));
        assert_eq!(changed_in_rows(&rows, 0, 0, &hunks, None), (vec![], vec![]));
    }

    async fn hunks_of(f: &Fixture, path: &str, staged: bool) -> Value {
        f.ok(call("wipHunks", json!({ "repo": f.repo, "worktree": f.worktree, "path": path, "staged": staged }))).await
    }

    #[tokio::test]
    async fn a_hunk_stages_alone_and_unstages_and_discards() {
        let f = Fixture::new().await;
        let text = f.read("a.txt").replace("line 1\n", "line one\n").replace("line 10\n", "line ten\n");
        f.write("a.txt", &text);
        let v = hunks_of(&f, "a.txt", false).await;
        let h = WipHunks::of(&v);
        assert_eq!(h.hunks.len(), 2, "two hunks: {v}");
        let (repo, wt) = (f.repo, f.worktree.clone());
        let stage = |sel: Value, base: Value, staged: bool| call("stagePatch", json!({ "repo": repo, "worktree": wt, "path": "a.txt", "staged": staged, "selection": sel, "base": base }));
        f.ok(stage(hunk_selection(0), h.base.clone(), false)).await;
        assert!(f.git(&["diff", "--cached"]).contains("+line one"));
        assert!(!f.git(&["diff", "--cached"]).contains("+line ten"));
        assert!(f.git(&["diff"]).contains("+line ten"));
        // Behind a stale base it refuses.
        let stale = f.send(stage(hunk_selection(0), h.base.clone(), false)).await.unwrap_err();
        assert_eq!(stale.kind, gitvolt_core::error::GbErrorKind::Stale);
        // Unstaged again from the staged side.
        let v = hunks_of(&f, "a.txt", true).await;
        let s = WipHunks::of(&v);
        f.ok(stage(hunk_selection(0), s.base.clone(), true)).await;
        assert_eq!(f.git(&["diff", "--cached"]), "");
        // Discarded from the file.
        let v = hunks_of(&f, "a.txt", false).await;
        let u = WipHunks::of(&v);
        f.ok(call("discard", json!({ "repo": repo, "worktree": f.worktree, "scope": { "kind": "patch", "path": "a.txt", "selection": hunk_selection(1), "base": u.base } }))).await;
        assert!(!f.read("a.txt").contains("line ten"));
        assert!(f.read("a.txt").contains("line one"));
    }

    #[tokio::test]
    async fn selected_lines_stage_and_the_core_counts_them() {
        let f = Fixture::new().await;
        let text = f.read("a.txt").replace("line 2\n", "two\nmore\n");
        f.write("a.txt", &text);
        let v = hunks_of(&f, "a.txt", false).await;
        let h = WipHunks::of(&v);
        let add = h.hunks[0].add.clone();
        assert_eq!(add.len(), 2);
        let sel = lines_selection(&[], &add[..1]);
        let c = f.ok(call("selectionLines", json!({ "repo": f.repo, "worktree": f.worktree, "path": "a.txt", "staged": false, "selection": sel }))).await;
        assert_eq!(c["apply"].as_u64(), Some(1));
        f.ok(call("stagePatch", json!({ "repo": f.repo, "worktree": f.worktree, "path": "a.txt", "staged": false, "selection": sel, "base": h.base }))).await;
        let cached = f.git(&["diff", "--cached"]);
        assert!(cached.contains("+two") && !cached.contains("+more"), "{cached}");
        // And the staging log takes it back.
        f.ok(call("stagingUndo", json!({ "repo": f.repo, "worktree": f.worktree }))).await;
        assert_eq!(f.git(&["diff", "--cached"]), "");
    }

    #[tokio::test]
    async fn a_binary_file_is_refused_with_its_reason() {
        let f = Fixture::new().await;
        std::fs::write(f.path.join("b.bin"), [0u8, 1, 2, 0, 3]).unwrap();
        f.git(&["add", "b.bin"]);
        f.git(&["commit", "-qm", "bin"]);
        std::fs::write(f.path.join("b.bin"), [0u8, 9, 2, 0, 3]).unwrap();
        let v = hunks_of(&f, "b.bin", false).await;
        let h = WipHunks::of(&v);
        assert!(h.hunks.is_empty());
        assert!(h.refused.is_some(), "{v}");
    }
}
