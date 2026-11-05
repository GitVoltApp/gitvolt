//! The merge tool's panel: the file's header, Current and Incoming over the output, or the buttons of a conflict
//! that isn't text (`MergeTool.tsx`, `mergeTool.css`, `NonTextConflict.tsx`). Each editor is one canvas, with
//! Monaco's metrics: 18 px lines, the hunk column, the numbers, the line buttons' lane, a minimap and a 10 px ruler.

use super::model::*;
use super::*;
use crate::app::{GitVolt, Tab, icon};
use crate::diff::view::{DiffState, mono, runs};
use crate::theme;
use crate::ui::tooltip::tip;
use gpui::prelude::*;

const SCROLLBAR_H: f32 = 10.;
/// Monaco's overview ruler draws a mark at least this tall.
const MIN_RULER_MARK: f32 = 6.;
const MINIMAP_LINE: f32 = 2.;
/// What Monaco's minimap keeps clear on its left.
const MINIMAP_GUTTER: f32 = 8.;

fn side_color(side: Side) -> Hsla {
    match side {
        Side::Current => theme::conflict_ours(),
        Side::Incoming => theme::conflict_theirs(),
    }
}

fn tinted(color: Hsla, share: f32) -> Hsla {
    theme::mix(color, share, transparent_black())
}

/// What a line of an editor is drawn with.
#[derive(Clone, Copy, Default)]
struct LineDeco {
    tint: Option<Hsla>,
    hunk: Option<Hsla>,
    /// A rule where an empty region's lines would go: below this line, or above.
    rule: Option<(bool, Hsla)>,
    /// The minimap's and the ruler's mark.
    mark: Option<Hsla>,
}

struct Snapshot {
    index: usize,
    lines: Arc<Lines>,
    colours: Vec<Arc<Vec<Vec<TmSpan>>>>,
    decos: HashMap<usize, LineDeco>,
    scroll: f32,
    hscroll: f32,
    widest: usize,
    /// Where the text starts, and the widths from the right.
    gutter: f32,
    pane: bool,
    minimap: f32,
    hover_button: Option<(usize, bool)>,
    current_line: usize,
    caret: Option<(usize, usize)>,
    selection: Option<((usize, usize), (usize, usize))>,
    focus: FocusHandle,
    body: Rc<Cell<Bounds<Pixels>>>,
    input: Option<Entity<GitVolt>>,
    weak: WeakEntity<GitVolt>,
}

fn pane_decos(tool: &MergeTool, pane: &SidePane) -> HashMap<usize, LineDeco> {
    let Some(file) = tool.file() else { return HashMap::new() };
    let color = side_color(pane.side);
    let count = pane.view.line_count();
    let mut out: HashMap<usize, LineDeco> = HashMap::new();
    for r in &pane.data.regions {
        if r.lines == 0 {
            // Nothing on this side: a rule where its lines would go.
            let below = r.start > 1;
            let n = if below { r.start - 2 } else { 0 }.min(count - 1);
            let d = out.entry(n).or_default();
            d.rule = Some((below, color));
            d.mark = Some(color);
            continue;
        }
        let pick = tool.picks.get(&r.id);
        let seg = file.conflict(r.id);
        let state = match (seg, pick) {
            (Some(s), Some(p)) => hunk_state(s, p, pane.side),
            _ => CheckState::None,
        };
        let hunk = match state {
            CheckState::All => color,
            CheckState::Some => tinted(color, 0.6),
            CheckState::None => tinted(color, 0.35),
        };
        for i in 0..r.lines {
            let on = pick.is_some_and(|p| p.of(pane.side).get(i).copied().unwrap_or(false));
            out.insert(r.start - 1 + i, LineDeco { tint: Some(tinted(color, if on { 0.34 } else { 0.18 })), hunk: Some(hunk), rule: None, mark: Some(color) });
        }
    }
    out
}

fn output_decos(tool: &MergeTool, o: &Output) -> HashMap<usize, LineDeco> {
    let base = theme::conflict_base();
    let mut out: HashMap<usize, LineDeco> = HashMap::new();
    for r in &o.regions {
        let pick = tool.picks.get(&r.id);
        let edited = o.edited.contains(&r.id);
        let resolved = edited || pick.is_some_and(Pick::any);
        let mark = if resolved { Hsla { a: 0.5, ..base } } else { base };
        if r.lines == 0 {
            let below = r.start > 1;
            let n = if below { r.start - 2 } else { 0 };
            let d = out.entry(n).or_default();
            d.rule = Some((below, base));
            d.mark = Some(mark);
            continue;
        }
        for (i, tint) in tint_kinds(pick, r.lines, edited).into_iter().enumerate() {
            let color = match tint {
                OutputTint::From(s) => side_color(s),
                OutputTint::Edited => base,
            };
            let d = out.entry(r.start - 1 + i).or_default();
            d.tint = Some(tinted(color, 0.18));
            d.mark = Some(mark);
        }
    }
    out
}

fn editor(s: Snapshot, cx: &mut Context<GitVolt>) -> AnyElement {
    let (index, focus, pane) = (s.index, s.focus.clone(), s.pane);
    let id: SharedString = format!("merge-editor-{index}").into();
    let el = div().id(id).relative().flex_1().min_h_0().w_full().overflow_hidden().cursor(CursorStyle::IBeam).track_focus(&focus);
    let el = if pane {
        el.key_context("MergePane")
            .on_action(cx.listener(move |this, _: &crate::app::SelectNext, _, cx| this.merge_pane_key(index, "down", false, cx)))
            .on_action(cx.listener(move |this, _: &crate::app::SelectPrevious, _, cx| this.merge_pane_key(index, "up", false, cx)))
            .on_action(cx.listener(move |this, _: &crate::app::PageDown, _, cx| this.merge_pane_key(index, "pagedown", false, cx)))
            .on_action(cx.listener(move |this, _: &crate::app::PageUp, _, cx| this.merge_pane_key(index, "pageup", false, cx)))
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
                let k = &e.keystroke;
                if k.modifiers.control || k.modifiers.alt || k.modifiers.platform {
                    return;
                }
                if matches!(k.key.as_str(), "left" | "right" | "up" | "down" | "home" | "end" | "space") {
                    this.merge_pane_key(index, &k.key, k.modifiers.shift, cx);
                    cx.stop_propagation();
                }
            }))
    } else {
        crate::diff::view::edit_keys_of(el, cx)
    };
    el.on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, window, cx| this.merge_press(index, e, window, cx)))
        .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, window, cx| this.merge_text_menu(index, e, window, cx)))
        .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| this.merge_hover(index, e, cx)))
        .on_hover(cx.listener(move |this, over: &bool, _, cx| {
            let over = *over;
            this.with_merge(cx, |m, _| {
                if index < m.editor_count() {
                    let v = m.view_mut(index);
                    v.pointer_in = over;
                    if !over {
                        v.hover = None;
                    }
                }
            });
        }))
        .child(canvas(|_, _, _| {}, move |bounds, _, window, cx| paint_editor(&s, bounds, window, cx)).size_full())
        .into_any_element()
}

fn paint_editor(s: &Snapshot, b: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    s.body.set(b);
    if let Some(entity) = &s.input {
        window.handle_input(&s.focus, ElementInputHandler::new(b, entity.clone()), cx);
    }
    let focused = s.focus.is_focused(window);
    let (width, height) = (b.size.width.as_f32(), b.size.height.as_f32());
    let view_right = width - RULER_W - s.minimap;
    let font = mono();
    let lines = &s.lines.lines;
    let count = lines.len().max(1);
    let first = (s.scroll / crate::metrics::line_h()).max(0.) as usize;
    let last = (((s.scroll + height) / crate::metrics::line_h()).ceil() as usize).min(count);
    let text_x = b.origin.x + px(s.gutter);
    let hovered_pointer = b.contains(&window.mouse_position());
    window.with_content_mask(Some(ContentMask { bounds: b }), |window| {
        let view_w = px(view_right - s.gutter);
        for n in first..last {
            let y = b.origin.y + px(n as f32 * crate::metrics::line_h() - s.scroll);
            let deco = s.decos.get(&n).copied().unwrap_or_default();
            if let Some(t) = deco.tint {
                window.paint_quad(fill(Bounds::new(point(text_x, y), size(view_w, px(crate::metrics::line_h()))), t));
            }
            if let Some((below, color)) = deco.rule {
                let at = if below { y + px(crate::metrics::line_h() - 2.) } else { y };
                window.paint_quad(fill(Bounds::new(point(text_x, at), size(view_w, px(2.))), color));
            }
            if let Some(h) = deco.hunk {
                window.paint_quad(fill(Bounds::new(point(b.origin.x, y), size(px(HUNK_W), px(crate::metrics::line_h()))), h));
            }
            // Monaco's current line: a border the width of the text.
            if n == s.current_line {
                let o = Bounds::new(point(text_x, y), size(view_w, px(crate::metrics::line_h())));
                if let Some(c) = theme::editor().line_highlight {
                    window.paint_quad(fill(o, c));
                }
                if let Some(c) = theme::editor().line_highlight_border {
                    window.paint_quad(outline(o, c, BorderStyle::Solid));
                }
            }
            let number: SharedString = (n + 1).to_string().into();
            // A file's last, empty line: its number dimmed, as Monaco's `renderFinalNewline`.
            let last_empty = n + 1 == count && count > 1 && lines.get(n).is_some_and(|l| l.text.is_empty());
            let color = if n == s.current_line { theme::editor().line_number_active } else { theme::editor().line_number };
            let color = if last_empty && n != s.current_line { Hsla { a: 0.3, ..color } } else { color };
            let shaped = window.text_system().shape_line(number.clone(), px(crate::metrics::editor_font_size()), &[TextRun { len: number.len(), font: font.clone(), color, background_color: None, underline: None, strikethrough: None }], None);
            let num_right = b.origin.x + px(if s.pane { HUNK_W + crate::diff::view::num_w() } else { crate::diff::view::num_w() });
            shaped.paint(point(num_right - shaped.width, y), px(crate::metrics::line_h()), TextAlign::Left, None, window, cx).ok();
            let Some(line) = lines.get(n) else { continue };
            let clip = Bounds::from_corners(point(text_x, y), point(b.origin.x + px(view_right), y + px(crate::metrics::line_h())));
            window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                let start = text_x - px(s.hscroll);
                let spans = s.colours.get(n / COLOUR_CHUNK).and_then(|p| p.get(n % COLOUR_CHUNK)).map_or(&[][..], Vec::as_slice);
                let shaped = window.text_system().shape_line(line.text.clone(), px(crate::metrics::editor_font_size()), &runs(&line.text, spans, &font), None);
                for level in 1..=line.guides {
                    let gx = start + px(crate::metrics::advance() * ((usize::from(level) - 1) * crate::metrics::tab_size()) as f32);
                    window.paint_quad(fill(Bounds::new(point(gx, y), size(px(1.), px(crate::metrics::line_h()))), theme::editor().indent_guide));
                }
                if let Some((from, to)) = s.selection
                    && from.0 <= n
                    && n <= to.0
                {
                    let l = if n == from.0 { from.1.min(line.text.len()) } else { 0 };
                    let r = if n == to.0 { to.1.min(line.text.len()) } else { line.text.len() };
                    let (lx, mut rx) = (shaped.x_for_index(l), shaped.x_for_index(r));
                    if n != to.0 {
                        rx += px(crate::metrics::advance());
                    }
                    if rx > lx {
                        window.paint_quad(fill(Bounds::from_corners(point(start + lx, y), point(start + rx, y + px(crate::metrics::line_h()))), theme::selection_bg()));
                    }
                }
                if !line.text.is_empty() {
                    shaped.paint(point(start, y), px(crate::metrics::line_h()), TextAlign::Left, None, window, cx).ok();
                }
                if focused
                    && let Some((cn, cb)) = s.caret
                    && cn == n
                {
                    window.paint_quad(fill(Bounds::new(point(start + shaped.x_for_index(cb.min(line.text.len())), y), size(px(2.), px(crate::metrics::line_h()))), theme::editor().cursor));
                }
            });
            // A line's button, under the pointer: a green + takes it, a red − drops it once taken.
            if let Some((hn, on)) = s.hover_button
                && hn == n
            {
                let box_ = Bounds::new(point(b.origin.x + px(HUNK_W + crate::diff::view::num_w() + (LANE_W - 15.) / 2.), y + px((crate::metrics::line_h() - 15.) / 2.)), size(px(15.), px(15.)));
                window.paint_quad(quad(box_, px(3.), if on { theme::red() } else { theme::green() }, px(0.), transparent_black(), BorderStyle::default()));
                let glyph = Bounds::new(box_.origin + point(px(1.5), px(1.5)), size(px(12.), px(12.)));
                window.paint_svg(glyph, if on { "icons/minus.svg" } else { "icons/plus.svg" }.into(), None, TransformationMatrix::unit(), theme::text_on_accent(), cx).ok();
            }
        }
        paint_minimap(s, b, view_right, window);
        paint_ruler(s, b, hovered_pointer, window);
        // Sideways: only while the pointer is over the text, or it's scrolled.
        let content = s.widest as f32 * crate::metrics::advance();
        let track = view_right - s.gutter;
        let max_h = (content - track).max(0.);
        if max_h > 0. && (s.hscroll > 0. || hovered_pointer) {
            let thumb = (track * (track / content)).max(24.);
            let x = text_x + px((track - thumb) * (s.hscroll / max_h));
            window.paint_quad(fill(Bounds::new(point(x, b.origin.y + px(height - SCROLLBAR_H)), size(px(thumb), px(SCROLLBAR_H))), theme::scroll_thumb()));
        }
    });
    let (weak, index) = (s.weak.clone(), s.index);
    window.on_mouse_event(move |e: &ScrollEvent, phase, window, cx| {
        if phase != DispatchPhase::Capture || !b.contains(&e.position) {
            return;
        }
        cx.stop_propagation();
        let delta = e.delta.pixel_delta(window.line_height());
        let (dx, dy) = (delta.x * crate::diff::view::MONACO_WHEEL, delta.y * crate::diff::view::MONACO_WHEEL);
        weak.update(cx, |this, cx| {
            this.with_merge(cx, |m, _| {
                if index >= m.editor_count() {
                    return;
                }
                let v = m.view_mut(index);
                v.scroll = (v.scroll - dy.as_f32()).clamp(0., v.max_scroll());
                let content = v.widest as f32 * crate::metrics::advance();
                let gutter = if index < 2 { pane_gutter() } else { out_gutter() };
                let track = v.width - RULER_W - minimap_width(v.width, gutter) - gutter;
                v.hscroll = (v.hscroll - dx.as_f32()).clamp(0., (content - track).max(0.));
                m.sync_scroll(index);
            });
        })
        .ok();
    });
}

/// The file in small, each character a pixel wide, a region's lines marked across.
fn paint_minimap(s: &Snapshot, b: Bounds<Pixels>, view_right: f32, window: &mut Window) {
    if s.minimap <= 0. {
        return;
    }
    let (height, x) = (b.size.height.as_f32(), b.origin.x + px(view_right + MINIMAP_GUTTER));
    let count = s.lines.lines.len().max(1);
    let shown = (height / MINIMAP_LINE) as usize;
    let max = (count as f32 * crate::metrics::line_h() - height).max(0.);
    let share = if max > 0. { s.scroll / max } else { 0. };
    let first = (share * count.saturating_sub(shown) as f32) as usize;
    let width = s.minimap - MINIMAP_GUTTER;
    let area = Bounds::new(point(x, b.origin.y), size(px(width), b.size.height));
    window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
        for (i, n) in (first..count.min(first + shown)).enumerate() {
            let y = b.origin.y + px(i as f32 * MINIMAP_LINE);
            if let Some(mark) = s.decos.get(&n).and_then(|d| d.mark) {
                window.paint_quad(fill(Bounds::new(point(x, y), size(px(width), px(MINIMAP_LINE))), mark));
            }
            let Some(line) = s.lines.lines.get(n) else { continue };
            let text = line.text.as_bytes();
            let cols = width as usize;
            let mut paint = |from: usize, to: usize, color: Hsla| {
                let to = to.min(cols);
                let mut at = from;
                while at < to {
                    if text.get(at) == Some(&b' ') {
                        at += 1;
                        continue;
                    }
                    let end = (at..to).find(|k| text.get(*k) == Some(&b' ')).unwrap_or(to);
                    window.paint_quad(fill(Bounds::new(point(x + px(at as f32), y), size(px((end - at) as f32), px(MINIMAP_LINE - 0.5))), color));
                    at = end;
                }
            };
            let spans = s.colours.get(n / COLOUR_CHUNK).and_then(|p| p.get(n % COLOUR_CHUNK)).map_or(&[][..], Vec::as_slice);
            if spans.is_empty() {
                paint(0, text.len(), Hsla { a: 0.6, ..theme::editor().foreground });
            } else {
                for t in spans {
                    paint(t.start as usize, (t.end as usize).min(text.len()), Hsla { a: 0.75, ..theme::c(t.color) });
                }
            }
        }
    });
}

/// The 10 px ruler: each region's mark, and the thumb while the pointer's over the editor.
fn paint_ruler(s: &Snapshot, b: Bounds<Pixels>, hovered: bool, window: &mut Window) {
    let height = b.size.height.as_f32();
    let x = b.origin.x + b.size.width - px(RULER_W);
    let count = s.lines.lines.len().max(1) as f32;
    let scale = (height / (count * crate::metrics::line_h()).max(height)) * crate::metrics::line_h();
    let mut lines: Vec<(usize, Hsla)> = s.decos.iter().filter_map(|(n, d)| d.mark.map(|m| (*n, m))).collect();
    lines.sort_by_key(|l| l.0);
    let mut at = 0;
    while at < lines.len() {
        let (first, color) = lines[at];
        let mut end = at + 1;
        while end < lines.len() && lines[end].0 == lines[end - 1].0 + 1 {
            end += 1;
        }
        let rows = (end - at) as f32;
        let h = (rows * scale).max(MIN_RULER_MARK);
        let y = first as f32 * scale + (rows * scale - h) / 2.;
        window.paint_quad(fill(Bounds::new(point(x, b.origin.y + px(y.max(0.))), size(px(RULER_W), px(h))), color));
        at = end;
    }
    let total = count * crate::metrics::line_h();
    if hovered && total > height {
        let thumb = (height * height / total).max(20.);
        let max = total - height;
        let top = (height - thumb) * (s.scroll / max.max(1.));
        window.paint_quad(fill(Bounds::new(point(x, b.origin.y + px(top)), size(px(RULER_W), px(thumb))), theme::scroll_thumb()));
    }
}

/// The merge tool's own checkbox: a 14 px rounded square outlined in its side's colour, filled with a tick when
/// taken, a dash when partly.
fn check_box(side: Side, state: CheckState, disabled: bool) -> Div {
    let color = side_color(side);
    let tick = theme::app_bg();
    let on = state != CheckState::None;
    let mut d = div().flex().flex_none().items_center().justify_center().size(px(14.)).rounded(px(3.)).border(px(1.5)).border_color(color).bg(if on { color } else { theme::app_bg() });
    if on {
        d = d.shadow(vec![BoxShadow { color: theme::app_bg(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.), inset: false }]);
    }
    d = d.when(disabled, |d| d.opacity(0.5));
    match state {
        CheckState::All => d.child(icon("check", 12., tick)),
        CheckState::Some => d.child(icon("minus", 12., tick)),
        CheckState::None => d,
    }
}

fn pane_header(tool: &MergeTool, file: &ConflictFile, side: Side, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let (label, stripe) = match side {
        Side::Current => (format!("Current: {}", file.label_current), theme::conflict_ours()),
        Side::Incoming => (format!("Incoming: {}", file.label_incoming), theme::conflict_theirs()),
    };
    let has = side_has_lines(&file.segments, side);
    let state = if has { side_state(&file.segments, &tool.picks, side) } else { CheckState::None };
    let key: SharedString = format!("merge-take-all-{}", side.index()).into();
    div()
        .relative()
        .flex()
        .flex_none()
        .h(px(BAR_H))
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .bg(theme::file_bar_bg())
        .border_b_1()
        .border_color(theme::section_border())
        .text_size(px(12.))
        .text_color(theme::text_normal())
        .child(div().absolute().left_0().top_0().bottom_0().w(px(3.)).bg(stripe))
        .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::SEMIBOLD).child(label))
        .child(crate::ui::anchor(
            key.clone(),
            div()
                .id(ElementId::Name(key))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .whitespace_nowrap()
                .text_color(theme::text_secondary())
                .when(!has, |d| d.opacity(0.5))
                .when(has, |d| d.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.merge_take_all(side, cx))))
                .child(check_box(side, state, false))
                .child("Take all from this side"),
        ))
}

fn merge_nav_button(id: &'static str, glyph: &'static str, label: String, forward: bool, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    tip(
        label,
        crate::ui::anchor(
            id,
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(px(32.))
                .h(px(28.))
                .rounded(px(4.))
                .border_1()
                .border_color(theme::section_border())
                .bg(theme::control_bg())
                .cursor_pointer()
                .hover(|d| d.bg(theme::control_hover_bg()))
                .on_click(cx.listener(move |this, _, _, cx| this.merge_step(forward, cx)))
                .child(icon(glyph, 14., theme::text_normal())),
        ),
    )
}

fn plain_button(id: impl Into<ElementId>, label: impl Into<SharedString>, off: bool) -> Stateful<Div> {
    div().id(id).flex().flex_none().items_center().justify_center().px(px(10.)).py(px(3.)).rounded(px(4.)).border_1().border_color(theme::section_border()).bg(theme::control_bg()).text_color(theme::text_normal()).when(off, |d| d.opacity(0.5)).when(!off, |d| d.cursor_pointer().hover(|d| d.bg(theme::control_hover_bg()))).child(label.into())
}

fn output_bar(tool: &MergeTool, file: &ConflictFile, path: &str, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let deleted = tool.base.is_none();
    let saving = tool.saving;
    let mut start = div().flex().flex_1().min_w_0().items_center().gap(px(10.)).child(div().flex_none().font_weight(FontWeight::SEMIBOLD).child("Output"));
    if file.eol == "mixed" {
        start = start.child(div().min_w_0().truncate().text_color(theme::text_secondary()).child("Mixed line endings: lines you type get one ending"));
    }
    if deleted {
        start = start
            .child(div().min_w_0().truncate().text_color(theme::red()).child(format!("{path} was deleted on disk: copy the output, restore the file (Take current or Take incoming), then paste it in")))
            .child(plain_button("merge-copy-output", "Copy output", false).py(px(2.)).on_click(cx.listener(|this, _, _, cx| this.merge_copy_output(cx))));
    }
    let save_tip = if deleted { format!("{path} was deleted on disk") } else { format!("Save ({})", crate::commands::display_chord("Mod+S")) };
    let off = deleted || saving;
    let hover = Hsla { l: (theme::green().l * 1.12).min(1.), ..theme::green() };
    let save = div()
        .id("merge-save-button")
        .flex()
        .flex_none()
        .items_center()
        .h(px(22.))
        .px(px(10.))
        .rounded(px(4.))
        .border_1()
        .border_color(theme::green())
        .bg(theme::green())
        .text_color(theme::text_on_accent())
        .font_weight(FontWeight::SEMIBOLD)
        .when(off, |d| d.opacity(0.5))
        .when(!off, |d| d.cursor_pointer().hover(move |d| d.bg(hover)))
        .on_click(cx.listener(move |this, _, _, cx| {
            if !off {
                this.merge_save(false, cx);
            }
        }))
        .child("Save and mark resolved");
    div()
        .flex()
        .flex_none()
        .h(px(BAR_H))
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .bg(theme::file_bar_bg())
        .border_b_1()
        .border_color(theme::section_border())
        .text_size(px(12.))
        .text_color(theme::text_normal())
        .child(start)
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(2.))
                .child(div().w(px(126.)).flex().justify_center().overflow_hidden().whitespace_nowrap().text_color(theme::text_secondary()).child(tool.count_text()))
                .child(merge_nav_button("merge-prev", "chevron-up", format!("Previous conflict ({})", crate::commands::display_chord("Shift+F7")), false, cx))
                .child(merge_nav_button("merge-next", "chevron-down", format!("Next conflict ({})", crate::commands::display_chord("F7")), true, cx)),
        )
        .child(div().flex().flex_1().min_w_0().items_center().justify_end().child(tip(save_tip, crate::ui::anchor("merge-save", save))))
}

fn snapshot_pane(tool: &MergeTool, i: usize, cx: &mut Context<GitVolt>) -> Snapshot {
    let p = &tool.panes[i];
    let selection = p.anchor.filter(|a| *a != p.caret).map(|a| if a < p.caret { (a, p.caret) } else { (p.caret, a) });
    let hover_button = p.view.hover.and_then(|n| p.data.region_at(n + 1).map(|(id, at)| (n, tool.picks.get(&id).is_some_and(|x| x.of(p.side).get(at).copied().unwrap_or(false)))));
    Snapshot {
        index: i,
        lines: p.view.lines.clone(),
        colours: p.view.colours.clone(),
        decos: pane_decos(tool, p),
        scroll: p.view.scroll,
        hscroll: p.view.hscroll,
        widest: p.view.widest,
        gutter: pane_gutter(),
        pane: true,
        minimap: minimap_width(p.view.width, pane_gutter()),
        hover_button,
        current_line: p.caret.0,
        caret: Some(p.caret),
        selection,
        focus: p.view.focus.clone(),
        body: p.view.body.clone(),
        input: None,
        weak: cx.weak_entity(),
    }
}

fn snapshot_output(tool: &MergeTool, cx: &mut Context<GitVolt>) -> Option<Snapshot> {
    let o = tool.output.as_ref()?;
    let pos = |at: usize| {
        let (n, col) = o.edit.line_col(at);
        let byte = o.view.lines.lines.get(n).map_or(col, |l| l.display(col as u32) as usize);
        (n, byte)
    };
    let caret = pos(o.edit.buffer.caret());
    let sel = &o.edit.buffer.sel;
    let selection = (!sel.is_empty()).then(|| (pos(sel.start), pos(sel.end)));
    Some(Snapshot {
        index: 2,
        lines: o.view.lines.clone(),
        colours: o.view.colours.clone(),
        decos: output_decos(tool, o),
        scroll: o.view.scroll,
        hscroll: o.view.hscroll,
        widest: o.view.widest,
        gutter: out_gutter(),
        pane: false,
        minimap: minimap_width(o.view.width, out_gutter()),
        hover_button: None,
        current_line: caret.0,
        caret: Some(caret),
        selection,
        focus: o.edit.focus.clone(),
        body: o.view.body.clone(),
        input: Some(cx.entity()),
        weak: cx.weak_entity(),
    })
}

/// A hunk's checkbox, over the hunk column, for what's on screen: a control of its own, as in the web UI.
fn hunk_boxes(tool: &MergeTool, i: usize, file: &ConflictFile, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    let p = &tool.panes[i];
    let mut out = Vec::new();
    let view_h = p.view.height;
    for (n, r) in p.data.regions.iter().enumerate() {
        let Some(seg) = file.conflict(r.id) else { continue };
        if r.lines == 0 || seg.of(p.side).is_empty() {
            continue;
        }
        let y = (r.start - 1) as f32 * crate::metrics::line_h() - p.view.scroll;
        if y + crate::metrics::line_h() <= 0. || y >= view_h {
            continue;
        }
        let state = tool.picks.get(&r.id).map_or(CheckState::None, |x| hunk_state(seg, x, p.side));
        let (id, side) = (r.id, p.side);
        let key: SharedString = format!("merge-hunk:{}:{}", side.index(), n + 1).into();
        out.push(
            crate::ui::anchor(
                key.clone(),
                div()
                    .id(ElementId::Name(key))
                    .absolute()
                    .left_0()
                    .top(px(y))
                    .w(px(HUNK_W))
                    .h(px(crate::metrics::line_h()))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| this.merge_toggle(id, side, None, cx)))
                    .child(check_box(side, state, false)),
            )
            .into_any_element(),
        );
    }
    out
}

/// The hovered line's button, as a control a script can press too; what it shows is the canvas's.
fn line_button(tool: &MergeTool, i: usize, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let p = &tool.panes[i];
    let n = p.view.hover?;
    let (id, at) = p.data.region_at(n + 1)?;
    let side = p.side;
    let y = n as f32 * crate::metrics::line_h() - p.view.scroll;
    let key: SharedString = format!("merge-line:{}", side.index()).into();
    Some(
        crate::ui::anchor(
            key.clone(),
            div()
                .id(ElementId::Name(key))
                .absolute()
                .left(px(HUNK_W + crate::diff::view::num_w()))
                .top(px(y))
                .w(px(LANE_W))
                .h(px(crate::metrics::line_h()))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| this.merge_toggle(id, side, Some(at), cx))),
        )
        .into_any_element(),
    )
}

fn non_text(tool: &MergeTool, file: &ConflictFile, repo: u32, cx: &mut Context<GitVolt>) -> AnyElement {
    let named = |label: &str, generic: &str| (!label.is_empty() && label != generic).then(|| label.to_string());
    let (cur, inc) = (named(&file.label_current, "Current"), named(&file.label_incoming, "Incoming"));
    let mut sentence = conflict_sentence(&file.kind, cur.as_deref(), inc.as_deref());
    if file.kind == "bothModified" {
        sentence.push_str(" (not text)");
    }
    let busy = tool.busy;
    let (worktree, path, base) = (tool.worktree.clone(), tool.path.clone(), tool.base.clone());
    let button = |id: &'static str, label: &'static str, kind: &'static str, danger: bool, cx: &mut Context<GitVolt>| {
        let (worktree, path, base) = (worktree.clone(), path.clone(), base.clone());
        crate::ui::anchor(
            id,
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .h(px(26.))
                .px(px(10.))
                .rounded(px(4.))
                .border_1()
                .bg(theme::control_bg())
                .border_color(if danger { theme::red() } else { theme::section_border() })
                .text_color(if danger { theme::red() } else { theme::text_normal() })
                .when(busy, |d| d.opacity(0.5))
                .when(!busy, |d| d.cursor_pointer().hover(|d| d.bg(theme::control_hover_bg())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if busy {
                        return;
                    }
                    let ctx = WriteCtx { repo, worktree: worktree.clone() };
                    this.resolve_file(&ctx, path.clone(), kind, base.clone(), ArmOrigin::None, cx);
                }))
                .child(label),
        )
    };
    let current = button("merge-take-current", "Take current", "current", false, cx);
    let incoming = button("merge-take-incoming", "Take incoming", "incoming", false, cx);
    let delete = button("merge-delete-file", "Delete file", "delete", true, cx);
    div().flex().flex_col().flex_1().p(px(16.)).text_color(theme::text_normal()).child(div().mb(px(12.)).child(sentence)).child(div().flex().gap(px(8.)).child(current).child(incoming).child(delete)).into_any_element()
}

fn message(text: impl Into<SharedString>) -> AnyElement {
    div().flex_1().p(px(16.)).text_color(theme::text_secondary()).child(text.into()).into_any_element()
}

/// The center panel for a conflicted WIP file, in place of the diff.
pub fn merge_panel(tab: &Tab, diff: &DiffState, tool: Option<&MergeTool>, cx: &mut Context<GitVolt>) -> AnyElement {
    let repo = tab.id;
    let body = match tool {
        None => message("Loading the conflict…"),
        Some(tool) => match &tool.loaded {
            Loaded::Loading => message("Loading the conflict…"),
            Loaded::Failed(e) => {
                let (worktree, path) = (tool.worktree.clone(), tool.path.clone());
                div()
                    .flex_1()
                    .p(px(16.))
                    .text_color(theme::text_secondary())
                    .child(format!("Couldn't read the conflict in {path}: {e}"))
                    .child(div().mt(px(8.)).child(plain_button("merge-retry", "Retry", false).on_click(cx.listener(move |this, _, _, cx| this.read_conflict_again(repo, worktree.clone(), path.clone(), false, cx)))))
                    .into_any_element()
            }
            Loaded::Ready(None) => message(format!("{} isn't conflicted any more.", tool.path)),
            Loaded::Ready(Some(file)) if !file.textual() => non_text(tool, file, repo, cx),
            Loaded::Ready(Some(file)) => {
                let pane = |i: usize, cx: &mut Context<GitVolt>| {
                    let snapshot = snapshot_pane(tool, i, cx);
                    let side = tool.panes[i].side;
                    let boxes = hunk_boxes(tool, i, file, cx);
                    let line = line_button(tool, i, cx);
                    div().flex().flex_col().flex_1().min_w_0().min_h_0().when(i == 1, |d| d.border_l_1().border_color(theme::section_border())).child(pane_header(tool, file, side, cx)).child(div().relative().flex().flex_col().flex_1().min_h_0().overflow_hidden().child(editor(snapshot, cx)).children(boxes).children(line))
                };
                let panes_h = tool.panes.first().map_or(0., |p| p.view.height) + BAR_H + 1.;
                let (left, right) = (pane(0, cx), pane(1, cx));
                let output = snapshot_output(tool, cx).map(|s| editor(s, cx));
                let bar = output_bar(tool, file, &tool.path, cx);
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex().flex_none().h(px(panes_h)).border_b_1().border_color(theme::section_border()).child(left).child(right))
                    .child(div().flex().flex_col().flex_1().min_h_0().child(bar).children(output))
                    .into_any_element()
            }
        },
    };
    div().relative().flex().flex_col().flex_1().min_w_0().min_h_0().bg(theme::app_bg()).child(crate::diff::view::header(diff, cx)).child(body).into_any_element()
}

pub fn caret_bounds(m: &MergeTool, bounds: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
    let o = m.output.as_ref()?;
    let (n, col) = o.edit.line_col(o.edit.buffer.caret());
    let y = n as f32 * crate::metrics::line_h() - o.view.scroll;
    Some(Bounds::new(bounds.origin + point(px(out_gutter() + col as f32 * crate::metrics::advance() - o.view.hscroll), px(y)), size(px(1.), px(crate::metrics::line_h()))))
}

impl GitVolt {
    /// A press in an editor: the hunk column, a line's button, the strips, or the text.
    fn merge_press(&mut self, index: usize, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        tab.keys = crate::app::KeyArea::Diff;
        let Some(m) = tab.merge_shown_mut() else { return };
        if index >= m.editor_count() {
            return;
        }
        let v = m.view(index);
        let b = v.body.get();
        let (x, y) = ((e.position.x - b.origin.x).as_f32(), (e.position.y - b.origin.y).as_f32());
        let (width, height) = (b.size.width.as_f32(), b.size.height.as_f32());
        let gutter = if index < 2 { pane_gutter() } else { out_gutter() };
        let mini = minimap_width(v.width, gutter);
        // The minimap and the ruler: the view goes where the press is.
        if x >= width - RULER_W - mini {
            let v = m.view_mut(index);
            v.scroll = (y / height).clamp(0., 1.) * v.max_scroll();
            v.drag = PaneDrag::Strip;
            m.sync_scroll(index);
            cx.notify();
            return;
        }
        // The sideways scrollbar, along the bottom.
        let content = v.widest as f32 * crate::metrics::advance();
        let track = width - RULER_W - mini - gutter;
        if content > track && y >= height - SCROLLBAR_H && x >= gutter {
            let v = m.view_mut(index);
            v.hscroll = ((x - gutter) / track).clamp(0., 1.) * (content - track);
            v.drag = PaneDrag::Horizontal;
            m.sync_scroll(index);
            cx.notify();
            return;
        }
        let line = v.line_at(y);
        if index < 2 {
            let (side, region) = (m.panes[index].side, m.panes[index].data.region_at(line + 1));
            if x < HUNK_W {
                if let Some((id, _)) = region {
                    let takes = m.file().and_then(|f| f.conflict(id)).is_some_and(|s| !s.of(side).is_empty());
                    if takes {
                        self.merge_toggle(id, side, None, cx);
                    }
                }
                return;
            }
            if (HUNK_W + crate::diff::view::num_w()..pane_gutter()).contains(&x) {
                if let Some((id, at)) = region {
                    self.merge_toggle(id, side, Some(at), cx);
                }
                return;
            }
        }
        let shape = |text: &SharedString, window: &Window| window.text_system().shape_line(text.clone(), px(crate::metrics::editor_font_size()), &[TextRun { len: text.len(), font: mono(), color: theme::text_normal(), background_color: None, underline: None, strikethrough: None }], None);
        let text = m.view(index).lines.lines.get(line).map_or(SharedString::default(), |l| l.text.clone());
        let hscroll = m.view(index).hscroll;
        let byte = shape(&text, window).closest_index_for_x(px((x - gutter + hscroll).max(0.)));
        window.focus(&m.view(index).focus.clone(), cx);
        if index < 2 {
            let p = &mut m.panes[index];
            p.want_col = None;
            match e.click_count {
                2 => {
                    let (a, z) = crate::diff::view::word_at(&text, byte);
                    p.anchor = Some((line, a));
                    p.caret = (line, z);
                }
                n if n >= 3 => {
                    p.anchor = Some((line, 0));
                    p.caret = (line, text.len());
                }
                _ if e.modifiers.shift => {
                    p.anchor = p.anchor.or(Some(p.caret));
                    p.caret = (line, byte);
                }
                _ => {
                    p.anchor = None;
                    p.caret = (line, byte);
                }
            }
        } else if let Some(o) = m.output.as_mut() {
            let raw = o.view.lines.lines.get(line).map_or(byte as u32, |l| l.raw(byte as u32)) as usize;
            let at = o.edit.at_line(line, raw);
            match e.click_count {
                2 => {
                    o.edit.buffer.sel = o.edit.buffer.word_at(at);
                    o.edit.buffer.reversed = false;
                }
                n if n >= 3 => {
                    o.edit.buffer.sel = o.edit.line_range(line);
                    o.edit.buffer.reversed = false;
                }
                _ if e.modifiers.shift => o.edit.buffer.select_to(at),
                _ => o.edit.buffer.move_to(at),
            }
            o.edit.want_col = None;
        }
        m.view_mut(index).drag = PaneDrag::Text;
        cx.notify();
    }

    /// The press drags, wherever the pointer's gone: the root sends it here.
    pub fn on_merge_drag(&mut self, e: &MouseMoveEvent, window: &Window, cx: &mut Context<Self>) {
        let Some(m) = self.tabs.get_mut(self.active).and_then(Tab::merge_shown_mut) else { return };
        let Some(index) = (0..m.editor_count()).find(|i| m.view(*i).drag != PaneDrag::None) else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            m.view_mut(index).drag = PaneDrag::None;
            return;
        }
        let v = m.view(index);
        let b = v.body.get();
        let (x, y) = ((e.position.x - b.origin.x).as_f32(), (e.position.y - b.origin.y).as_f32());
        let gutter = if index < 2 { pane_gutter() } else { out_gutter() };
        let (width, height) = (b.size.width.as_f32(), b.size.height.as_f32());
        match v.drag {
            PaneDrag::Strip => {
                let v = m.view_mut(index);
                v.scroll = (y / height).clamp(0., 1.) * v.max_scroll();
                m.sync_scroll(index);
            }
            PaneDrag::Horizontal => {
                let track = width - RULER_W - minimap_width(v.width, gutter) - gutter;
                let content = v.widest as f32 * crate::metrics::advance();
                let v = m.view_mut(index);
                v.hscroll = ((x - gutter) / track).clamp(0., 1.) * (content - track).max(0.);
                m.sync_scroll(index);
            }
            PaneDrag::Text => {
                // Past the top or the bottom, the text comes to the pointer.
                let v = m.view_mut(index);
                if y < 0. {
                    v.scroll = (v.scroll + y.max(-crate::metrics::line_h() * 3.)).max(0.);
                } else if y > height {
                    v.scroll = (v.scroll + (y - height).min(crate::metrics::line_h() * 3.)).min(v.max_scroll());
                }
                let line = v.line_at(y.clamp(0., height));
                let text = v.lines.lines.get(line).map_or(SharedString::default(), |l| l.text.clone());
                let hscroll = v.hscroll;
                let shaped = window.text_system().shape_line(text.clone(), px(crate::metrics::editor_font_size()), &[TextRun { len: text.len(), font: mono(), color: theme::text_normal(), background_color: None, underline: None, strikethrough: None }], None);
                let byte = shaped.closest_index_for_x(px((x - gutter + hscroll).max(0.)));
                if index < 2 {
                    let p = &mut m.panes[index];
                    p.anchor = p.anchor.or(Some(p.caret));
                    p.caret = (line, byte);
                } else if let Some(o) = m.output.as_mut() {
                    let raw = o.view.lines.lines.get(line).map_or(byte as u32, |l| l.raw(byte as u32)) as usize;
                    let at = o.edit.at_line(line, raw);
                    o.edit.buffer.select_to(at);
                }
                m.sync_scroll(index);
            }
            PaneDrag::None => {}
        }
        cx.notify();
    }

    pub fn end_merge_drag(&mut self) {
        if let Some(m) = self.tabs.get_mut(self.active).and_then(Tab::merge_shown_mut) {
            for i in 0..m.editor_count() {
                m.view_mut(i).drag = PaneDrag::None;
            }
        }
    }

    fn merge_hover(&mut self, index: usize, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(m) = self.tabs.get_mut(self.active).and_then(Tab::merge_shown_mut) else { return };
        if index >= m.editor_count() {
            return;
        }
        let v = m.view_mut(index);
        let b = v.body.get();
        let (x, y) = ((e.position.x - b.origin.x).as_f32(), (e.position.y - b.origin.y).as_f32());
        let over = x < b.size.width.as_f32() - RULER_W - minimap_width(v.width, if index < 2 { pane_gutter() } else { out_gutter() });
        let hover = (over && y >= 0. && y < b.size.height.as_f32() && (y + v.scroll) < v.content()).then(|| v.line_at(y));
        let changed = hover != v.hover || !v.pointer_in;
        v.hover = hover;
        v.pointer_in = true;
        if changed {
            cx.notify();
        }
    }

    /// A pane's keys: the caret, its selection, and Space's tick. Shift+Space takes the whole hunk.
    fn merge_pane_key(&mut self, index: usize, key: &str, shift: bool, cx: &mut Context<Self>) {
        let Some(m) = self.tabs.get_mut(self.active).and_then(Tab::merge_shown_mut) else { return };
        let Some(p) = m.panes.get_mut(index) else { return };
        let (line, byte) = p.caret;
        let count = p.view.line_count();
        let page = (p.view.height / crate::metrics::line_h()).max(1.) as usize;
        let want = |p: &mut SidePane, n: usize| {
            let col = *p.want_col.get_or_insert(byte);
            (n, col.min(p.line_len(n)))
        };
        let at = match key {
            "space" => {
                let Some((id, i)) = p.data.region_at(line + 1) else { return };
                let side = p.side;
                self.merge_toggle(id, side, (!shift).then_some(i), cx);
                return;
            }
            "up" => want(p, line.saturating_sub(1)),
            "down" => want(p, (line + 1).min(count - 1)),
            "pageup" => want(p, line.saturating_sub(page)),
            "pagedown" => want(p, (line + page).min(count - 1)),
            "left" => {
                p.want_col = None;
                let text = p.view.lines.lines.get(line).map_or("", |l| l.text.as_ref());
                match text[..byte.min(text.len())].char_indices().next_back() {
                    Some((i, _)) => (line, i),
                    None if line > 0 => (line - 1, p.line_len(line - 1)),
                    None => (line, 0),
                }
            }
            "right" => {
                p.want_col = None;
                let text = p.view.lines.lines.get(line).map_or("", |l| l.text.as_ref());
                match text[byte.min(text.len())..].chars().next() {
                    Some(c) => (line, byte + c.len_utf8()),
                    None if line + 1 < count => (line + 1, 0),
                    None => (line, byte),
                }
            }
            "home" => {
                p.want_col = None;
                (line, 0)
            }
            "end" => {
                p.want_col = None;
                (line, p.line_len(line))
            }
            _ => return,
        };
        p.caret_to(at, shift);
        m.sync_scroll(index);
        cx.notify();
    }

    /// Right-click on the text: Copy; in the output, Cut and Paste too.
    fn merge_text_menu(&mut self, index: usize, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::menu::{self, MenuRow};
        let selected = self.merge_selected_text();
        let has = selected.is_some();
        let weak = cx.weak_entity();
        let (w1, text) = (weak.clone(), selected.clone().unwrap_or_default());
        let mut rows = vec![MenuRow::action("monaco.copySelection", "Copy", "copy", "Copy the selected text", move |_, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
            let _ = &w1;
        })
        .disabled_if((!has).then_some("No text selected"))];
        if index == 2 {
            let w2 = weak.clone();
            rows.push(MenuRow::action("monaco.paste", "Paste", "clipboard-paste", "Paste at the caret", move |_, cx| {
                if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    w2.update(cx, |v, cx| v.merge_paste(&text, cx)).ok();
                }
            }));
        }
        menu::open_context(rows, e.position, window, cx);
    }

    fn merge_paste(&mut self, text: &str, cx: &mut Context<Self>) {
        self.with_merge(cx, |m, _| {
            if let Some(o) = m.output.as_mut() {
                o.edit.buffer.replace(None, text, crate::ui::input::EditKind::Other);
            }
            m.after_output_edit();
            m.sync_scroll(2);
        });
        self.merge_colour_after_edit(cx);
    }

    fn merge_copy_output(&mut self, cx: &mut Context<Self>) {
        let text = self.tabs.get(self.active).and_then(Tab::merge_shown).and_then(|m| m.output.as_ref()).map(|o| o.edit.text_to_save());
        if let Some(text) = text {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            crate::ui::toast::show("Copied", cx);
        }
    }

    /// The guard's Save, and then what was asked.
    pub fn merge_save_then(&mut self, go: crate::write::After, cx: &mut Context<Self>) {
        let Some(path) = self.tabs.get(self.active).and_then(Tab::merge_shown).map(|m| m.path.clone()) else { return };
        self.merge_after_save = Some((path, go));
        self.merge_save(false, cx);
    }

    /// The guard's Discard edits: the tool goes back to nothing ticked.
    pub fn merge_discard(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let repo = tab.id;
        let Some(m) = tab.merge_shown_mut() else { return };
        let Some(file) = m.file().cloned() else { return };
        let (base, generation, worktree) = (m.base.clone(), m.generation, m.worktree.clone());
        let picks = empty_picks(&file.segments);
        let tool = MergeTool::build(file, picks, base, generation, worktree, cx);
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) {
            tab.merge = Some(tool);
        }
        self.colour_merge(repo, generation, cx);
        cx.notify();
    }
}
