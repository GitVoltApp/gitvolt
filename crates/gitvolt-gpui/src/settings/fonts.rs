//! Font settings: `FontSetting.tsx` and `FontPicker.tsx`: the picker, the box for a hand-written CSS list, the preview under
//! the label. `fontStack.ts`: what a setting's text means.

use super::frame::input_box;
use super::pages::Def;
use crate::app::{GitVolt, icon};
use crate::theme;
use crate::ui::input::{InputEvent, TextInput};
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::rc::Rc;

const MIXED_SCRIPTS: &str = "Café, naïve, Straße, Tiếng Việt, Ελληνικά, Кириллица, 日本語";
const CODE_SAMPLE: [&str; 6] = ["// Café, naïve, Straße, Tiếng Việt, Ελληνικά, Кириллица, 日本語", "function scale(values: number[], factor = 1.5): number[] {", "  const label = \"The quick brown fox 0O 1lI\"; // -> label", "  if (values.length >= 10 && factor !== 0) return values;", "  return values.map((v) => (v === 42 ? v : v * factor));", "}"];

/// `singleFamily`: the one family a setting names, unquoted; `None` for a list.
pub fn single_family(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if (t.starts_with('"') && t.ends_with('"') && t.len() > 1 && !t[1..t.len() - 1].contains('"')) || (t.starts_with('\'') && t.ends_with('\'') && t.len() > 1 && !t[1..t.len() - 1].contains('\'')) {
        return Some(t[1..t.len() - 1].to_string());
    }
    (!t.contains(',')).then(|| t.to_string())
}

/// The first family of a setting's text, for a font that takes one family and its fallbacks.
pub fn families_of(list: &str) -> Vec<String> {
    list.split(',').map(|f| f.trim().trim_matches(['"', '\'']).to_string()).filter(|f| !f.is_empty()).collect()
}

/// What `system-ui` and other generic names are here: fontconfig says.
fn resolve_generic(name: &str) -> Option<String> {
    let o = std::process::Command::new("fc-match").args(["-f", "%{family[0]}", name]).output().ok()?;
    (o.status.success() && !o.stdout.is_empty()).then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// The UI font a setting gives: its first family, generic names resolved; empty is the platform's.
pub fn ui_family(setting: &str, default: &str) -> SharedString {
    let Some(first) = families_of(setting).into_iter().next() else { return default.to_string().into() };
    let generic = ["system-ui", "sans-serif", "serif", "monospace", "ui-monospace", "ui-sans-serif"];
    if generic.contains(&first.as_str()) {
        return resolve_generic(&first).unwrap_or_else(|| default.to_string()).into();
    }
    first.into()
}

fn role_of(id: &str) -> (&'static str, &'static str, bool, &'static str) {
    match id {
        "uiFontFamily" => ("UI", "system UI font", false, "\"Inter\", sans-serif"),
        "readingFontFamily" => ("reading", "the UI font", false, "\"Literata\", Georgia, serif"),
        _ => ("editor", "JetBrains Mono", true, "\"Fira Code\", monospace"),
    }
}

/// `installedName`: `value`'s family as the list spells it, if it's there.
fn installed_name(value: &str, list: &[Value]) -> Option<String> {
    let one = single_family(value)?;
    list.iter().filter_map(|f| f["name"].as_str()).find(|n| n.eq_ignore_ascii_case(&one)).map(str::to_string)
}

#[derive(Clone, PartialEq)]
pub enum Item {
    Default,
    /// A bundled or set font not in the list, or an installed one.
    Family(String),
    Custom,
}

pub struct FontPicker {
    pub role: &'static str,
    pub all: bool,
    pub query: Entity<TextInput>,
    pub cursor: usize,
    pub scroll: UniformListScrollHandle,
    pub return_to: Option<FocusHandle>,
    _sub: Subscription,
}

fn items_for(this: &GitVolt, role: &str, all: bool, q: &str) -> Vec<Item> {
    let value = this.settings[role].as_str().unwrap_or_default();
    let list = this.settings_ui.fonts.clone().unwrap_or_default();
    let monospace = role == "editorFontFamily";
    let mut names: Vec<String> = Vec::new();
    if monospace {
        names.push("JetBrains Mono".into());
    }
    if let Some(one) = single_family(value)
        && installed_name(value, &list).is_none()
        && !names.iter().any(|n| n.eq_ignore_ascii_case(&one))
    {
        names.push(one);
    }
    for f in list.iter() {
        let n = f["name"].as_str().unwrap_or_default();
        if (all || !monospace || f["monospace"] == true) && !names.iter().any(|x| x == n) {
            names.push(n.to_string());
        }
    }
    let q = q.trim().to_lowercase();
    let mut out = Vec::new();
    if q.is_empty() || "default".contains(&q) {
        out.push(Item::Default);
    }
    out.extend(names.into_iter().filter(|n| q.is_empty() || n.to_lowercase().contains(&q)).map(Item::Family));
    out.push(Item::Custom);
    out
}

impl GitVolt {
    /// `loadFontFamilies`: the list, once a picker is on screen; reading every font's header isn't for the start.
    pub fn load_fonts(&mut self, refresh: bool, cx: &mut Context<Self>) {
        if cfg!(test) || self.settings_ui.fonts_loading || (self.settings_ui.fonts.is_some() && !refresh) {
            return;
        }
        self.settings_ui.fonts_loading = true;
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(gitvolt_core::api::Request::FontFamilies { refresh }).await });
        cx.spawn(async move |this, cx| {
            let answer = read.await;
            this.update(cx, |v, cx| {
                v.settings_ui.fonts_loading = false;
                if let Ok(Ok(list)) = answer {
                    v.settings_ui.fonts = Some(Rc::new(list.as_array().cloned().unwrap_or_default()));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_font_picker(&mut self, role: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.load_fonts(false, cx);
        let query = cx.new(|cx| TextInput::new(cx).placeholder("Search fonts").claim_arrows().claim_escape().enter_submits());
        let sub = cx.subscribe(&query, move |this, input, e: &InputEvent, cx| match e {
            InputEvent::Changed => {
                if let Some(p) = this.settings_ui.font_picker.as_mut() {
                    p.cursor = 0;
                }
                let _ = input;
                cx.notify();
            }
            InputEvent::Arrow { down, .. } => this.font_step(*down, cx),
            InputEvent::Submit { .. } => this.font_pick_cursor(cx),
            InputEvent::Escape => this.font_close(cx),
            _ => {}
        });
        let list = self.settings_ui.fonts.clone().unwrap_or_default();
        let items = items_for(self, role, false, "");
        let value = self.settings[role].as_str().unwrap_or_default().to_string();
        let cursor = items.iter().position(|i| is_current(i, &value, &list)).unwrap_or(0);
        let return_to = window.focused(cx);
        let f = query.read(cx).focus.clone();
        window.focus(&f, cx);
        self.settings_ui.font_preview = None;
        self.settings_ui.font_picker = Some(FontPicker { role, all: false, query, cursor, scroll: UniformListScrollHandle::new(), return_to, _sub: sub });
        cx.notify();
    }

    fn font_close(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.settings_ui.font_picker.take()
            && let Some(back) = p.return_to
        {
            self.focus_next_frame = Some(back);
        }
        self.settings_ui.font_preview = None;
        cx.notify();
    }

    fn font_step(&mut self, down: bool, cx: &mut Context<Self>) {
        let Some(p) = self.settings_ui.font_picker.as_ref() else { return };
        let (role, all, q) = (p.role, p.all, p.query.read(cx).text().to_string());
        let items = items_for(self, role, all, &q);
        let Some(p) = self.settings_ui.font_picker.as_mut() else { return };
        if items.is_empty() {
            return;
        }
        p.cursor = if down { (p.cursor + 1).min(items.len() - 1) } else { p.cursor.saturating_sub(1) };
        p.scroll.scroll_to_item(p.cursor, ScrollStrategy::Nearest);
        self.settings_ui.font_preview = match &items[self.settings_ui.font_picker.as_ref().map_or(0, |p| p.cursor)] {
            Item::Default => Some(String::new()),
            Item::Family(n) => Some(n.clone()),
            Item::Custom => None,
        };
        cx.notify();
    }

    fn font_pick_cursor(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.settings_ui.font_picker.as_ref() else { return };
        let (role, all, q, cursor) = (p.role, p.all, p.query.read(cx).text().to_string(), p.cursor);
        let items = items_for(self, role, all, &q);
        if let Some(item) = items.get(cursor).cloned() {
            self.font_pick(role, item, cx);
        }
    }

    fn font_pick(&mut self, role: &'static str, item: Item, cx: &mut Context<Self>) {
        match item {
            Item::Custom => {
                self.settings_ui.font_asked.insert(role.to_string());
            }
            Item::Default => {
                self.settings_ui.font_asked.remove(role);
                self.set_settings(|s| s[role] = json!(""), cx);
            }
            Item::Family(n) => {
                self.settings_ui.font_asked.remove(role);
                let n = if n == "JetBrains Mono" && role == "editorFontFamily" { String::new() } else { n };
                self.set_settings(move |s| s[role] = json!(n), cx);
            }
        }
        self.font_close(cx);
    }
}

fn is_current(item: &Item, value: &str, list: &[Value]) -> bool {
    match item {
        Item::Default => value.trim().is_empty(),
        Item::Family(n) => installed_name(value, list).as_deref() == Some(n.as_str()) || single_family(value).as_deref() == Some(n.as_str()),
        Item::Custom => false,
    }
}

/// The control: the picker's button, and the box for a hand-written list.
pub fn font_control(this: &mut GitVolt, def: &'static Def, window: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let role = def.id;
    let value = this.settings[role].as_str().unwrap_or_default().to_string();
    let list = this.settings_ui.fonts.clone().unwrap_or_default();
    if value.is_empty() {
        this.settings_ui.font_asked.remove(role);
    }
    let custom = this.settings_ui.font_asked.contains(role) || (!value.is_empty() && single_family(&value).is_none());
    let (_, default_name, _, placeholder) = role_of(role);
    let open = this.settings_ui.font_picker.as_ref().is_some_and(|p| p.role == role);
    let shown: (String, Option<String>) = if value.is_empty() { ("Default".into(), Some(format!("({default_name})"))) } else if custom { ("Custom font list…".into(), None) } else { (installed_name(&value, &list).or_else(|| single_family(&value)).unwrap_or_default(), None) };
    let button = crate::ui::anchor(
        format!("setting:{role}"),
        div()
            .id(ElementId::Name(format!("input-{role}").into()))
            .flex()
            .flex_none()
            .w_full()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .pl(px(8.))
            .pr(px(10.))
            .py(px(5.))
            .text_color(theme::text_selected())
            .bg(theme::app_bg())
            .border_1()
            .border_color(if open { theme::blue() } else { theme::section_border() })
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|d| d.bg(theme::hover_row()))
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.open_font_picker(role, window, cx);
            }))
            .child(div().flex().flex_1().min_w_0().items_baseline().gap(px(6.)).child(div().min_w_0().truncate().when(!custom, |d| d.font_family(if value.is_empty() { if role == "editorFontFamily" { SharedString::from(crate::diff::view::FONT) } else { this.base_font.clone() } } else { SharedString::from(shown.0.clone()) })).child(shown.0)).children(shown.1.map(|d| div().flex_none().text_size(px(11.)).text_color(theme::text_secondary()).child(d))))
            .child(icon("chevron-down", 14., theme::text_secondary())),
    );
    let mut col = div().flex().flex_col().items_end().gap(px(6.)).w_full().min_w_0().child(button);
    if custom {
        let key = format!("{role}-custom");
        let input = this.settings_field(
            &key,
            &value,
            placeholder,
            Rc::new(move |v, text, cx| {
                let t = text.trim().to_string();
                v.settings_ui.font_draft.remove(role);
                if v.settings[role].as_str() != Some(t.as_str()) {
                    v.set_settings(move |s| s[role] = json!(t), cx);
                }
            }),
            Some(Rc::new(move |v, text, cx| {
                v.settings_ui.font_draft.insert(role.to_string(), text.to_string());
                cx.notify();
            })),
            window,
            cx,
        );
        col = col.child(input_box(ElementId::Name(key.into()), input, false, window, cx).w_full());
    }
    col.into_any_element()
}

fn preview_panel(this: &GitVolt, role: &str, window: &Window, cx: &App) -> AnyElement {
    let open = this.settings_ui.font_picker.as_ref().is_some_and(|p| p.role == role);
    let hover = open.then(|| this.settings_ui.font_preview.clone()).flatten();
    let family = hover.or_else(|| this.settings_ui.font_draft.get(role).cloned()).unwrap_or_else(|| this.settings[role].as_str().unwrap_or_default().to_string());
    let frame = div().overflow_hidden().rounded(px(4.)).border_1().border_color(theme::section_border()).bg(theme::app_bg()).text_color(theme::text_selected());
    let _ = (window, cx);
    let body = match role {
        "editorFontFamily" => {
            let size = this.settings["editorFontSize"].as_f64().unwrap_or(13.).clamp(8., 32.) as f32;
            let set = this.settings["editorLineHeight"].as_f64().unwrap_or(0.) as f32;
            let line = if set > 0. { set.max(size) } else { (size * 1.35).round() };
            let first = families_of(&family).into_iter().next().unwrap_or_else(|| crate::diff::view::FONT.to_string());
            let spans = code_sample_spans();
            let font = Font { family: first.into(), features: if this.settings["editorFontLigatures"] == true { FontFeatures::default() } else { FontFeatures::disable_ligatures() }, fallbacks: Some(FontFallbacks::from_fonts(vec![crate::diff::view::FONT.to_string(), "Noto Sans Mono".into(), "monospace".into()])), weight: FontWeight::NORMAL, style: FontStyle::Normal };
            let lines = CODE_SAMPLE.iter().zip(spans.iter()).map(|(l, s)| {
                let runs = crate::diff::view::runs(l, s, &font);
                div().h(px(line)).whitespace_nowrap().overflow_hidden().child(StyledText::new(l.to_string()).with_runs(runs))
            });
            frame.px(px(12.)).py(px(8.)).h(px(CODE_SAMPLE.len() as f32 * line + 18.)).text_size(px(size)).line_height(px(line)).children(lines).into_any_element()
        }
        "readingFontFamily" => {
            let size = this.settings["readingFontSize"].as_f64().unwrap_or(13.).clamp(8., 32.) as f32;
            let lh = this.settings["readingLineHeight"].as_f64().unwrap_or(1.55).clamp(1.2, 2.4) as f32;
            let first = families_of(&family).into_iter().next().unwrap_or_else(|| this.font.to_string());
            frame.p(px(12.)).h(px((16. + size * (1.3 * 1.25 + 0.3) + size * lh * 5. + size * 0.4 * 3.).round())).font_family(SharedString::from(first)).text_size(px(size)).line_height(relative(lh)).child(div().text_size(px(size * 1.25)).font_weight(FontWeight::SEMIBOLD).child("Release notes")).child(div().mt(px(size * 0.4)).child("The quick brown fox jumps over the lazy dog. READMEs, descriptions and comments show in this font, wherever Markdown is rendered.")).child(div().mt(px(size * 0.4)).child("Call scale(values, 1.5) to resize every value.")).child(div().mt(px(size * 0.4)).child(MIXED_SCRIPTS)).into_any_element()
        }
        _ => {
            let first = families_of(&family).into_iter().next().unwrap_or_else(|| this.base_font.to_string());
            frame.flex().flex_col().gap(px(3.)).px(px(12.)).py(px(8.)).font_family(SharedString::from(first)).text_size(px(13.)).text_color(theme::text_normal()).child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).child("Sphinx of black quartz, judge my vow")).child("The quick brown fox jumps over the lazy dog. 0123456789").child(div().text_size(px(11.)).text_color(theme::text_secondary()).child("How vexingly quick daft zebras jump")).child(MIXED_SCRIPTS).into_any_element()
        }
    };
    div().min_w_0().child(div().mb(px(4.)).text_size(px(10.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_secondary()).child("PREVIEW")).child(body).into_any_element()
}

/// A font row's parts: the label's column takes the preview, the control's its picker.
/// `placeList`: the list at its button's width, under it, else above it, wherever all of it fits without covering the preview; with room
/// for neither, the larger side that keeps clear, scrolling more rows. `(left, top, rows' height)`.
pub fn place_list(anchor: (f32, f32, f32, f32), chrome: f32, rows: f32, row: f32, avoid: Option<(f32, f32, f32, f32)>, bounds: (f32, f32, f32, f32)) -> (f32, f32, f32) {
    const EDGE: f32 = 8.;
    const GAP: f32 = 4.;
    let width = anchor.2 - anchor.0;
    let left = (bounds.0 + EDGE).max(anchor.0.min(bounds.2 - EDGE - width));
    let clear = |top: f32, height: f32| avoid.is_none_or(|a| left >= a.2 || left + width <= a.0 || top >= a.3 || top + height <= a.1);
    let height = chrome + rows;
    let (room_below, room_above) = (bounds.3 - EDGE - anchor.3 - GAP, anchor.1 - GAP - EDGE - bounds.1);
    let below = |h: f32| (left, anchor.3 + GAP, h - chrome);
    let above = |h: f32| (left, anchor.1 - GAP - h, h - chrome);
    if height <= room_below && clear(anchor.3 + GAP, height) {
        return below(height);
    }
    if height <= room_above && clear(anchor.1 - GAP - height, height) {
        return above(height);
    }
    let (ok_below, ok_above) = (clear(anchor.3 + GAP, room_below), clear(anchor.1 - GAP - room_above, room_above));
    let fit = |room: f32| (chrome + row).max(height.min(room));
    if ok_above && (!ok_below || room_above > room_below) { above(fit(room_above)) } else { below(fit(room_below)) }
}

pub fn font_row_parts(this: &mut GitVolt, def: &'static Def, text: Div, control: AnyElement, window: &mut Window, cx: &mut Context<GitVolt>) -> (Div, AnyElement, AnyElement) {
    (text, crate::ui::anchor(format!("font-preview:{}", def.id), div().child(preview_panel(this, def.id, window, cx))).into_any_element(), control)
}

fn code_sample_spans() -> Vec<Vec<crate::diff::textmate::Span>> {
    // Coloured when the theme's tokens are known: the engine's fast on six lines, and the frame's the settings'.
    crate::diff::textmate::Tokenizer::new("typescript").and_then(|mut t| t.lines(&CODE_SAMPLE)).unwrap_or_else(|| vec![Vec::new(); CODE_SAMPLE.len()])
}

/// The open list, beside the preview.
pub fn font_picker_overlay(this: &mut GitVolt, window: &mut Window, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let p = this.settings_ui.font_picker.as_ref()?;
    let (role, all, query, cursor, scroll) = (p.role, p.all, p.query.clone(), p.cursor, p.scroll.clone());
    let q = query.read(cx).text().to_string();
    let items = Rc::new(items_for(this, role, all, &q));
    let (noun, _, monospace, _) = role_of(role);
    let value = this.settings[role].as_str().unwrap_or_default().to_string();
    let list = this.settings_ui.fonts.clone().unwrap_or_default();
    let anchor = crate::ui::Ui::get(cx).anchor(&format!("setting:{role}"));
    let vp = window.viewport_size();
    const ROW: f32 = 26.;
    let chrome = 40. + if monospace { 26. } else { 0. } + 4.;
    let rows = (items.len() as f32 * ROW).min(312.);
    let width = anchor.map_or(320., |a| a.size.width.as_f32());
    let edges = |b: Bounds<Pixels>| (b.left().as_f32(), b.top().as_f32(), b.right().as_f32(), b.bottom().as_f32());
    let avoid = crate::ui::Ui::get(cx).anchor(&format!("font-preview:{role}")).map(edges);
    let (left, top, view) = match anchor {
        Some(a) => place_list(edges(a), chrome, rows, ROW, avoid, (0., 0., vp.width.as_f32(), vp.height.as_f32())),
        None => (100., 100., rows),
    };
    let weak = cx.weak_entity();
    let default_font = this.base_font.clone();
    let body = uniform_list("font-list", items.len(), {
        let (items, weak, value, list, default_font) = (items.clone(), weak.clone(), value.clone(), list.clone(), default_font.clone());
        move |range, _, _| {
            range
                .map(|i| {
                    let item = items[i].clone();
                    let selected = i == cursor;
                    let current = is_current(&item, &value, &list);
                    let (label, family, detail): (String, Option<SharedString>, Option<&'static str>) = match &item {
                        Item::Default => ("Default".into(), Some(default_font.clone()), None),
                        Item::Family(n) => (n.clone(), Some(n.clone().into()), None),
                        Item::Custom => ("Custom font list…".into(), None, None),
                    };
                    let is_custom = item == Item::Custom;
                    let (w, w2, it) = (weak.clone(), weak.clone(), item.clone());
                    let preview: Option<String> = match &item {
                        Item::Default => Some(String::new()),
                        Item::Family(n) => Some(n.clone()),
                        Item::Custom => None,
                    };
                    div()
                        .id(("font-row", i))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(ROW))
                        .px(px(10.))
                        .cursor_pointer()
                        .whitespace_nowrap()
                        .when(!is_custom, |d| d.text_size(px(14.)))
                        .when(selected, |d| d.bg(theme::selected_row()).text_color(theme::text_selected()))
                        .when(is_custom, |d| d.text_size(px(13.)).text_color(theme::link()).border_t_1().border_color(theme::section_border()))
                        .on_mouse_move(move |_, _, cx| {
                            let preview = preview.clone();
                            w.update(cx, |v, cx| {
                                if let Some(p) = v.settings_ui.font_picker.as_mut()
                                    && p.cursor != i
                                {
                                    p.cursor = i;
                                    v.settings_ui.font_preview = preview;
                                    cx.notify();
                                }
                            })
                            .ok();
                        })
                        .on_click(move |_, _, cx| {
                            let it = it.clone();
                            w2.update(cx, |v, cx| v.font_pick(role, it, cx)).ok();
                        })
                        .child(div().flex_1().min_w_0().truncate().when_some(family, |d, f| d.font_family(f)).when(current, |d| d.font_weight(FontWeight::SEMIBOLD)).child(label))
                        .children(detail)
                })
                .collect()
        }
    })
    .track_scroll(&scroll)
    .h(px(view));
    let w = weak.clone();
    let refresh = weak.clone();
    let panel = div()
        .id("font-picker")
        .occlude()
        .absolute()
        .left(px(left))
        .top(px(top))
        .flex()
        .flex_col()
        .w(px(width))
        .bg(theme::panel_bg1())
        .border_1()
        .border_color(theme::section_border())
        .rounded(px(6.))
        .shadow(crate::ui::shadow(theme::shadow_popup(), 8., 24.))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .m(px(6.))
                .child(crate::ui::controls::clearable("font-search", &query, |_, _| {}, window, cx))
                .child(crate::ui::tooltip::tip("List the fonts again", div().id("font-refresh").flex().flex_none().size(px(24.)).items_center().justify_center().rounded(px(3.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default()).on_click(move |_, _, cx| {
                    refresh.update(cx, |v, cx| v.load_fonts(true, cx)).ok();
                }).child(icon("refresh-cw", 13., theme::text_secondary())))),
        )
        .when(monospace, |d| {
            d.child(div().id("font-all").flex().items_center().gap(px(6.)).mx(px(10.)).mb(px(6.)).text_size(px(12.)).text_color(theme::text_secondary()).cursor_pointer().on_click(move |_, _, cx| {
                w.update(cx, |v, cx| {
                    if let Some(p) = v.settings_ui.font_picker.as_mut() {
                        p.all = !p.all;
                        p.cursor = 0;
                    }
                    cx.notify();
                })
                .ok();
            }).child(crate::ui::controls::check_box(all, 13.).mt_0()).child(format!("Show all fonts, not only monospace ones for the {noun} font")))
        })
        .child(body);
    Some(
        div()
            .id("font-layer")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.font_close(cx)))
            .child(panel)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn one_family_or_a_list() {
        assert_eq!(single_family("Inter").as_deref(), Some("Inter"));
        assert_eq!(single_family("\"Fira Code\"").as_deref(), Some("Fira Code"));
        assert_eq!(single_family("Inter, sans-serif"), None);
        assert_eq!(families_of("\"Inter\", 'Noto Sans', sans-serif"), ["Inter", "Noto Sans", "sans-serif"]);
    }
}

#[cfg(test)]
mod place_tests {
    use super::place_list;
    use std::prelude::v1::test;

    #[test]
    fn the_list_goes_under_else_above_and_keeps_off_the_preview() {
        let bounds = (0., 0., 1000., 800.);
        let button = (600., 300., 900., 330.);
        assert_eq!(place_list(button, 40., 200., 26., None, bounds), (600., 334., 200.));
        // Under it would cover the preview; above it fits.
        assert_eq!(place_list(button, 40., 200., 26., Some((500., 340., 950., 500.)), bounds), (600., 56., 200.));
        // No room for all of it anywhere: the larger clear side, scrolling.
        let low = (600., 700., 900., 730.);
        let (_, top, view) = place_list(low, 40., 900., 26., None, bounds);
        assert_eq!((top, view), (8., 648.));
    }
}
