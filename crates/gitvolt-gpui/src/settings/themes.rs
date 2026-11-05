//! Settings › Theme (`theme/editor/`): the cards, the editing strip, the lanes, the colours beside a live preview, and the
//! floating editor; `edit.ts`, `themeJson.ts`, `share.ts`.

use super::frame::group_title;
use super::pages::Def;
use crate::app::{GitVolt, icon};
use crate::theme::{self, Col, ThemeDef, ThemePatch};
use crate::ui::controls::{self, OnSelect};
use crate::ui::{arm, colour, toast};
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::rc::Rc;

/// `CORE_COLOURS`.
const CORE_GROUPS: [&str; 4] = ["Surfaces", "Text", "Diff", "Status"];
const CORE_COLOURS: [(&str, &str, &str, &str); 12] = [
    ("Surfaces", "app-bg0", "Window background", ""),
    ("Surfaces", "panel-bg0", "Panels", "sidebar, details"),
    ("Surfaces", "panel-bg1", "Menus and popovers", ""),
    ("Text", "text-normal", "Text", ""),
    ("Text", "text-secondary", "Secondary text", ""),
    ("Text", "blue", "Accent", "selection, links, focus"),
    ("Diff", "diff-inserted-line", "Added lines", ""),
    ("Diff", "diff-removed-line", "Removed lines", ""),
    ("Status", "green", "Added, success", ""),
    ("Status", "red", "Deleted, danger", ""),
    ("Status", "orange", "Modified, warning", ""),
    ("Status", "yellow", "Attention", ""),
];

/// `TOKEN_LABELS`.
pub fn token_label(t: &str) -> &'static str {
    match t {
        "app-bg0" => "Window background", "panel-bg0" => "Panels", "panel-bg1" => "Menus and popovers", "toolbar-bg0" => "Status bar",
        "action-bar-bg" => "Action bar", "tab-bar-bg" => "Tab bar", "file-bar-bg" => "File bar",
        "text-selected" => "Brightest text", "text-normal" => "Text", "text-secondary" => "Secondary text", "text-disabled" => "Disabled text",
        "text-on-accent" => "Text on accent",
        "stats-added" => "Added count", "stats-deleted" => "Deleted count",
        "red" => "Red", "orange" => "Orange", "yellow" => "Yellow", "green" => "Green", "blue" => "Accent", "link" => "Links",
        "count-blue" => "Sidebar counts", "checked-out-row" => "Checked-out row", "checked-out" => "Checked-out mark", "selection-bg" => "Text selection",
        "conflict-ours" => "Conflict: ours", "conflict-theirs" => "Conflict: theirs", "conflict-base" => "Conflict: base",
        "wip-input-bg" => "Commit message box", "wip-input-border" => "Commit message box border", "wip-input-placeholder" => "Commit message placeholder",
        "hover-row" => "Row under the pointer", "selected-row" => "Selected row", "section-border" => "Borders", "focus-outline" => "Focus outline",
        "text-row-dimmed" => "Dimmed row", "text-row-dimmed-branch" => "Dimmed branch", "sidebar-icon" => "Sidebar icons", "avatar-backdrop" => "Behind avatars",
        "stats-added-bright" => "Added total", "stats-deleted-bright" => "Deleted total",
        "status-added" => "Added file", "status-modified" => "Modified file", "status-deleted" => "Deleted file", "status-renamed" => "Renamed file", "status-conflicted" => "Conflicted file",
        "scroll-thumb-bg" => "Scrollbar", "scroll-thumb-hover-bg" => "Scrollbar under the pointer",
        "control-bg" => "Quiet buttons", "control-hover-bg" => "Quiet buttons under the pointer", "image-frame-border" => "Image frame",
        "shadow-modal" => "Dialog shadow", "shadow-popup" => "Popup shadow", "shadow-tooltip" => "Tooltip shadow", "backdrop" => "Dialog backdrop",
        "node-fill" => "Commit dot fill", "node-text" => "Commit dot text", "collapse-strip" => "Collapsed commits shade",
        "conflict-ours-tick" => "Conflict tick: ours", "conflict-theirs-tick" => "Conflict tick: theirs",
        "diff-inserted-text" => "Added text", "diff-removed-text" => "Removed text", "diff-inserted-line" => "Added lines", "diff-removed-line" => "Removed lines",
        "diff-diagonal-fill" => "Past the end of a side",
        "md-diff-ins-text" => "Markdown: added words", "md-diff-del-text" => "Markdown: removed words", "md-diff-ins-block" => "Markdown: added blocks", "md-diff-del-block" => "Markdown: removed blocks",
        "forge-gitlab" => "GitLab mark", "forge-github" => "GitHub mark", "forge-forgejo" => "Forgejo mark", "purple" => "Merged",
        "switch-on" => "Switch on", "switch-off" => "Switch off",
        "tg-ink" => "Tab group text", "tg-blue" => "Tab group: blue", "tg-purple" => "Tab group: purple", "tg-cyan" => "Tab group: cyan", "tg-orange" => "Tab group: orange",
        "tg-yellow" => "Tab group: yellow", "tg-pink" => "Tab group: pink", "tg-green" => "Tab group: green", "tg-gray" => "Tab group: grey", "tg-red" => "Tab group: red",
        "button-primary" => "Main button", "button-warn" => "Warning button", "button-warn-ink" => "Warning button text",
        "review-pending" => "Pending review", "review-pending-bg" => "Pending review background",
        _ => "",
    }
}

const TOKEN_GROUPS: [(&str, &[&str]); 7] = [
    ("Surfaces", &["app-bg0", "panel-bg0", "panel-bg1", "toolbar-bg0", "action-bar-bg", "tab-bar-bg", "file-bar-bg", "hover-row", "selected-row", "section-border", "control-bg", "control-hover-bg", "scroll-thumb-bg", "scroll-thumb-hover-bg", "shadow-modal", "shadow-popup", "shadow-tooltip", "backdrop"]),
    ("Text", &["text-selected", "text-normal", "text-secondary", "text-disabled", "text-on-accent", "blue", "link", "focus-outline", "selection-bg", "sidebar-icon", "count-blue", "wip-input-bg", "wip-input-border", "wip-input-placeholder"]),
    ("Diff", &["diff-inserted-text", "diff-removed-text", "diff-inserted-line", "diff-removed-line", "diff-diagonal-fill", "md-diff-ins-text", "md-diff-del-text", "md-diff-ins-block", "md-diff-del-block", "conflict-ours", "conflict-theirs", "conflict-base", "conflict-ours-tick", "conflict-theirs-tick"]),
    ("Status", &["green", "red", "orange", "yellow", "stats-added", "stats-deleted", "stats-added-bright", "stats-deleted-bright", "status-added", "status-modified", "status-deleted", "status-renamed", "status-conflicted"]),
    ("Graph and canvas", &["node-fill", "node-text", "collapse-strip", "avatar-backdrop", "text-row-dimmed", "text-row-dimmed-branch", "checked-out-row", "checked-out"]),
    ("Forge and tabs", &["forge-gitlab", "forge-github", "forge-forgejo", "purple", "review-pending", "review-pending-bg", "tg-ink", "tg-blue", "tg-purple", "tg-cyan", "tg-orange", "tg-yellow", "tg-pink", "tg-green", "tg-gray", "tg-red"]),
    ("Other", &["image-frame-border", "switch-on", "switch-off", "button-primary", "button-warn", "button-warn-ink"]),
];

/// `share.ts` `uniqueName`: `name`, else `"name (2)"`, `"name (3)"`…; compared without case.
pub fn unique_name(name: &str, taken: &[String]) -> String {
    let used: Vec<String> = taken.iter().map(|t| t.to_lowercase()).collect();
    if !used.contains(&name.to_lowercase()) {
        return name.to_string();
    }
    let base = match name.rfind(" (") {
        Some(i) if name.ends_with(')') && name[i + 2..name.len() - 1].chars().all(|c| c.is_ascii_digit()) && i + 3 < name.len() => &name[..i],
        _ => name,
    };
    (2..).map(|n| format!("{base} ({n})")).find(|c| !used.contains(&c.to_lowercase())).expect("one is free")
}

/// `exportFileName`.
pub fn export_file_name(name: &str, kind: &str) -> String {
    let mut slug = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    format!("{}.gitvolt-{kind}.json", if slug.is_empty() { kind } else { slug })
}

/// `parseShareJson`: the envelope's body, or `None` for anything that isn't one.
pub fn parse_share(text: &str, kind: &str, max_version: u64) -> Option<(serde_json::Map<String, Value>, u64)> {
    if text.len() > 65_536 {
        return None;
    }
    let mut raw = text.trim();
    if let Some(r) = raw.strip_prefix("```") {
        raw = r.strip_prefix("json").unwrap_or(r).trim_start_matches([' ', '\t']).trim_start_matches(['\r', '\n']);
    }
    let raw = raw.strip_suffix("```").unwrap_or(raw).trim();
    let v: Value = serde_json::from_str(raw).ok()?;
    let mut body = v.as_object()?.clone();
    if body.remove("gitvolt")?.as_str()? != kind {
        return None;
    }
    let version = body.remove("version")?.as_u64().filter(|n| *n >= 1 && *n <= max_version)?;
    Some((body, version))
}

/// `toShareJson`: two-space JSON with the envelope first (`preserve_order` isn't on: the keys go by hand).
pub fn to_share(kind: &str, version: u64, body: &[(&str, Value)]) -> String {
    let mut out = format!("{{\n  \"gitvolt\": {},\n  \"version\": {version}", json!(kind));
    for (k, v) in body {
        let text = serde_json::to_string_pretty(v).unwrap_or_default().replace('\n', "\n  ");
        out.push_str(&format!(",\n  {}: {text}", json!(k)));
    }
    out.push_str("\n}");
    out
}

fn is_lane(v: &Value) -> bool {
    v.as_str().is_some_and(|s| s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit()))
}

fn lanes_of(v: &Value) -> Value {
    match v.as_array() {
        Some(a) if a.len() == 10 && a.iter().all(is_lane) => json!(a.iter().map(|c| c.as_str().unwrap_or_default().to_lowercase()).collect::<Vec<_>>()),
        _ => json!([]),
    }
}

fn colours_of(v: &Value) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(m) = v.as_object() {
        for (t, c) in m {
            if theme::TOKEN_NAMES.contains(&t.as_str()) && c.as_str().is_some_and(|s| s == s.trim() && Col::parse(s).is_some()) {
                out.insert(t.clone(), c.clone());
            }
        }
    }
    Value::Object(out)
}

/// `themeToJson`.
pub fn theme_to_json(def: &ThemeDef, custom: Option<&Value>) -> String {
    let own = custom.filter(|c| def.base.is_some() && c["id"] == def.id.as_str());
    to_share("theme", 1, &[("name", json!(def.label)), ("base", json!(def.base.as_deref().unwrap_or(&def.id))), ("colors", own.map_or(json!({}), |o| colours_of(&o["colors"]))), ("graph", own.map_or(json!([]), |o| lanes_of(&o["graph"])))])
}

pub fn new_custom_id() -> String {
    let mut buf = [0u8; 4];
    if getrandom_bytes(&mut buf).is_err() {
        let t = crate::format::now() as u64;
        buf = (t as u32).to_le_bytes();
    }
    format!("custom-{:02x}{:02x}{:02x}{:02x}", buf[0], buf[1], buf[2], buf[3])
}

fn getrandom_bytes(buf: &mut [u8]) -> std::io::Result<()> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")?.read_exact(buf)
}

/// `themeFromJson`: a pasted or imported theme as a new custom theme, or `None`.
pub fn theme_from_json(text: &str, existing: &[Value], id: String) -> Option<Value> {
    let (body, _) = parse_share(text, "theme", 1)?;
    let name = body.get("name")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }
    let cut: String = name.chars().take(80).collect();
    let taken: Vec<String> = existing.iter().filter_map(|t| t["name"].as_str().map(str::to_string)).collect();
    let base = body.get("base").and_then(Value::as_str).filter(|b| theme::is_theme_id(b)).unwrap_or(theme::DEFAULT_THEME_ID);
    Some(json!({ "id": id, "name": unique_name(cut.trim(), &taken), "base": base, "colors": colours_of(body.get("colors").unwrap_or(&Value::Null)), "graph": lanes_of(body.get("graph").unwrap_or(&Value::Null)) }))
}

/// `typedColour`: what's typed as a theme colour (`#` optional on hex), in the form themes keep.
pub fn typed_colour(text: &str) -> Option<String> {
    let t = text.trim();
    let hexy = (3..=8).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_hexdigit());
    let c = if hexy { Col::parse(&format!("#{t}")) } else { Col::parse(t) };
    c.map(Col::css)
}

/// `asHex`: `#rrggbb`, or `#rrggbbaa` when translucent.
pub fn as_hex(c: Col) -> String {
    if c.a >= 1. { c.css() } else { format!("{}{:02x}", c.at(1.).css(), (c.a * 255.).round() as u8) }
}

fn customs(v: &GitVolt) -> Vec<Value> {
    v.settings["customThemes"].as_array().cloned().unwrap_or_default()
}

impl GitVolt {
    /// The theme Settings › Theme edits, and its custom theme when it is one.
    pub fn edited_theme(&self) -> (ThemeDef, Option<Value>) {
        let id = self.settings["theme"].as_str().unwrap_or(theme::DEFAULT_THEME_ID);
        let def = theme::theme_def(id, &self.settings["customThemes"]);
        let custom = customs(self).into_iter().find(|c| c["id"] == def.id.as_str());
        (def, custom)
    }

    /// `ensureEditable`: the current theme's id when it's a custom theme; else a new custom copy of it, selected.
    pub fn ensure_editable(&mut self, cx: &mut Context<Self>) -> String {
        let (def, custom) = self.edited_theme();
        if custom.is_some() {
            return def.id;
        }
        let list = customs(self);
        let taken: Vec<String> = list.iter().filter_map(|t| t["name"].as_str().map(str::to_string)).collect();
        let id = new_custom_id();
        let base = if theme::is_theme_id(&def.id) { def.id.as_str() } else { theme::DEFAULT_THEME_ID };
        let ct = json!({ "id": id, "name": unique_name(&format!("{} (Custom)", def.label), &taken), "base": base, "colors": {}, "graph": [] });
        let id2 = id.clone();
        self.set_settings(
            move |s| {
                if !s["customThemes"].is_array() {
                    s["customThemes"] = json!([]);
                }
                s["customThemes"].as_array_mut().expect("just made").push(ct);
                s["theme"] = json!(id2);
            },
            cx,
        );
        id
    }

    fn update_custom(&mut self, id: &str, change: impl FnOnce(&mut Value), cx: &mut Context<Self>) {
        let id = id.to_string();
        self.set_settings(
            move |s| {
                if let Some(t) = s["customThemes"].as_array_mut().and_then(|l| l.iter_mut().find(|t| t["id"] == id.as_str())) {
                    change(t);
                }
            },
            cx,
        );
    }

    pub fn set_theme_colour(&mut self, token: &str, value: &str, cx: &mut Context<Self>) {
        let id = self.ensure_editable(cx);
        let (token, value) = (token.to_string(), value.to_string());
        self.update_custom(&id, move |t| {
            if !t["colors"].is_object() {
                t["colors"] = json!({});
            }
            t["colors"][token] = json!(value);
        }, cx);
    }

    fn lanes_now(&self, id: &str) -> Vec<String> {
        theme::theme_def(id, &self.settings["customThemes"]).graph.iter().map(|c| c.css()).collect()
    }

    pub fn set_lane_colour(&mut self, index: usize, hex: &str, cx: &mut Context<Self>) {
        let id = self.ensure_editable(cx);
        let mut graph = self.lanes_now(&id);
        if index >= graph.len() {
            return;
        }
        graph[index] = hex.to_lowercase();
        self.update_custom(&id, move |t| t["graph"] = json!(graph), cx);
    }

    pub fn move_lane(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let id = self.ensure_editable(cx);
        let mut graph = self.lanes_now(&id);
        if from == to || from >= graph.len() || to >= graph.len() {
            return;
        }
        let lane = graph.remove(from);
        graph.insert(to, lane);
        self.update_custom(&id, move |t| t["graph"] = json!(graph), cx);
    }

    pub fn rename_theme(&mut self, id: &str, name: &str, cx: &mut Context<Self>) {
        let next = name.trim().to_string();
        let list = customs(self);
        let Some(t) = list.iter().find(|c| c["id"] == id) else { return };
        if next.is_empty() || t["name"] == next.as_str() {
            return;
        }
        let taken: Vec<String> = list.iter().filter(|c| c["id"] != id).filter_map(|c| c["name"].as_str().map(str::to_string)).collect();
        let named = unique_name(&next, &taken);
        self.update_custom(id, move |c| c["name"] = json!(named), cx);
    }

    pub fn delete_theme(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = id.to_string();
        let Some(t) = customs(self).into_iter().find(|c| c["id"] == id.as_str()) else { return };
        let current = self.settings["theme"] == id.as_str();
        self.set_settings(
            move |s| {
                if let Some(l) = s["customThemes"].as_array_mut() {
                    l.retain(|c| c["id"] != id.as_str());
                }
                if current {
                    s["theme"] = json!(t["base"].as_str().filter(|b| theme::is_theme_id(b)).unwrap_or(theme::DEFAULT_THEME_ID));
                }
            },
            cx,
        );
    }

    /// `pasteTheme`: added and selected, or a toast and no change.
    pub fn paste_theme(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(theme) = theme_from_json(text, &customs(self), new_custom_id()) else {
            toast::show("The clipboard doesn't hold a GitVolt theme", cx);
            return;
        };
        let name = theme["name"].as_str().unwrap_or_default().to_string();
        let id = theme["id"].clone();
        self.set_settings(
            move |s| {
                if !s["customThemes"].is_array() {
                    s["customThemes"] = json!([]);
                }
                s["customThemes"].as_array_mut().expect("just made").push(theme);
                s["theme"] = id;
            },
            cx,
        );
        toast::show(format!("Added {name}"), cx);
    }

    /// Ctrl+V on a Settings page that takes one (`useSettingsPaste`).
    pub fn settings_paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else { return };
        match self.settings_ui.page.as_str() {
            "theme" => self.paste_theme(&text, cx),
            "branch-presets" => self.paste_preset(&text, cx),
            "file-colours" => self.paste_file_colours(&text, cx),
            _ => {}
        }
    }

    pub fn save_export(&mut self, name: String, contents: String, cx: &mut Context<Self>) {
        let api = self.api.clone();
        let done = self.rt.spawn(async move { api.dispatch(gitvolt_core::api::Request::SaveExport { suggested_name: name, contents }).await });
        cx.spawn(async move |this, cx| {
            let answer = done.await;
            this.update(cx, |_, cx| match answer {
                Ok(Ok(v)) => {
                    if let Some(path) = v.as_str() {
                        let file = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
                        toast::show_with(format!("Saved {file}"), toast::Options { detail: Some(path.to_string().into()), ..Default::default() }, cx);
                    }
                }
                Ok(Err(e)) => toast::show(e.message, cx),
                Err(e) => toast::show(e.to_string(), cx),
            })
            .ok();
        })
        .detach();
    }

    /// A lane dragged over another takes its place on release.
    pub fn on_lane_drag(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some((from, moved, start)) = self.settings_ui.lane_drag else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            self.settings_ui.lane_drag = None;
            return;
        }
        let far = moved || (e.position.x - start.x).abs() > px(4.);
        if !far {
            return;
        }
        self.settings_ui.lane_drag = Some((from, true, start));
        let ui = crate::ui::Ui::get(cx);
        let x = e.position.x;
        if let Some(to) = (0..10).find(|i| i != &from && ui.anchor(&format!("lane:{i}")).is_some_and(|b| x >= b.left() && x < b.right())) {
            self.move_lane(from, to, cx);
            self.settings_ui.lane_drag = Some((to, true, start));
        }
    }

    pub fn end_lane_drag(&mut self, _: &mut Context<Self>) {
        self.settings_ui.lane_drag = None;
    }
}

fn preview_colour(token: Option<&str>, v: Option<&str>, cx: &mut App) {
    match (token, v.and_then(Col::parse)) {
        (Some(t), Some(c)) => theme::preview(Some(ThemePatch { colors: vec![(t.to_string(), c)], graph: None })),
        _ => theme::preview(None),
    }
    cx.refresh_windows();
}

fn translucent(def: &ThemeDef, token: &str) -> bool {
    let base = def.base.as_deref().or(theme::is_theme_id(&def.id).then_some(def.id.as_str()));
    base.and_then(theme::builtin).map_or(def.colors.get(token), |b| b.colors.get(token)).is_some_and(|c| c.a < 1.)
}

/// One colour: its swatch (the picker, previewing on the app), its name, and a field that applies on Enter or blur.
#[allow(clippy::too_many_arguments)]
fn colour_row(this: &mut GitVolt, token: &'static str, label: &'static str, note: &str, compact: bool, def: &ThemeDef, window: &Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let value = def.colors.get(token).copied().unwrap_or(Col::opaque(0, 0, 0));
    let shown = as_hex(value);
    let weak = cx.weak_entity();
    let w2 = weak.clone();
    let swatch = colour::swatch(
        format!("colour:{token}"),
        &value.css(),
        format!("{label} colour"),
        translucent(def, token),
        22.,
        Rc::new(move |v, _, cx| preview_colour(Some(token), Some(v), cx)),
        Rc::new(move |v, _, cx| {
            preview_colour(None, None, cx);
            let v = v.to_string();
            w2.update(cx, |this, cx| {
                if Col::parse(&v) != Some(value) {
                    this.set_theme_colour(token, &v, cx);
                }
            })
            .ok();
        }),
        Rc::new(|_, cx| preview_colour(None, None, cx)),
        cx,
    );
    let key = format!("{}colour-value:{token}", if compact { "float-" } else { "" });
    let input = this.settings_field(
        &key,
        &shown,
        "",
        Rc::new(move |v, text, cx| {
            let Some(typed) = typed_colour(text) else {
                // Not a colour: back to what it was.
                if let Some(f) = v.settings_ui.fields.values_mut().find(|f| f.shown == text) {
                    f.shown = "\0".into();
                }
                cx.notify();
                return;
            };
            let (def, _) = v.edited_theme();
            if Col::parse(&typed) != def.colors.get(token).copied() {
                v.set_theme_colour(token, &typed, cx);
            }
        }),
        Some(Rc::new(|_, _, cx| cx.notify())),
        window,
        cx,
    );
    let invalid = typed_colour(input.read(cx).text()).is_none();
    let focused = input.read(cx).is_focused(window);
    let (w, mono) = (if compact { 86. } else { 110. }, crate::diff::view::FONT);
    div()
        .flex()
        .items_center()
        .gap(px(9.))
        .min_h(px(if compact { 28. } else { 30. }))
        .child(swatch)
        .child(div().flex().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_size(px(12.)).text_color(theme::text_normal()).child(label).when(!note.is_empty(), |d| d.child(div().ml(px(4.)).text_color(theme::text_secondary()).child(note.to_string()))))
        .child(div().flex().flex_none().items_center().w(px(w)).h(px(24.)).px(px(6.)).rounded(px(4.)).bg(theme::app_bg()).border_1().border_color(if invalid { theme::status_deleted() } else if focused { theme::blue() } else { theme::section_border() }).font_family(mono).text_size(px(12.)).text_color(theme::text_selected()).overflow_hidden().child(input))
        .into_any_element()
}

fn colour_group_title(t: &str) -> Div {
    div().mb(px(4.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_secondary()).child(t.to_uppercase())
}

/// `ThemeColourList`: the core colours in four groups, then All colours behind a disclosure.
fn colour_list(this: &mut GitVolt, filter: &str, compact: bool, extra: Option<AnyElement>, window: &Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let (def, _) = this.edited_theme();
    let q = filter.trim().to_lowercase();
    let keep = |token: &str, label: &str, note: &str| q.is_empty() || [label, note, token].iter().any(|s| s.to_lowercase().contains(&q));
    let core: Vec<_> = CORE_COLOURS.iter().filter(|c| keep(c.1, c.2, c.3)).collect();
    let all: Vec<(&str, Vec<&'static str>)> = TOKEN_GROUPS.iter().map(|(g, t)| (*g, t.iter().copied().filter(|t| keep(t, token_label(t), "")).collect::<Vec<_>>())).filter(|g| !g.1.is_empty()).collect();
    let open = if compact { this.settings_ui.float_all } else { this.settings_ui.all_colours };
    let expanded = open.unwrap_or(!q.is_empty() && core.is_empty() && !all.is_empty());
    let mut list = div().flex().flex_col().min_w_0();
    // `@container (max-width: 600px)`: beside the preview the list is narrower than that.
    let stacked = compact || (this.window_width > 900. && this.settings_body_width() - 360. <= 600.) || this.settings_body_width() <= 600.;
    let columns = |items: Vec<AnyElement>, compact: bool| -> AnyElement {
        if compact || stacked {
            return div().flex().flex_col().gap(px(8.)).children(items).into_any_element();
        }
        // Two columns, filled across as a grid is.
        let mut rows = div().flex().flex_col().gap(px(12.));
        let mut it = items.into_iter().peekable();
        while it.peek().is_some() {
            let mut row = div().flex().gap(px(24.)).child(div().flex_1().min_w_0().child(it.next().expect("peeked")));
            row = row.child(div().flex_1().min_w_0().children(it.next()));
            rows = rows.child(row);
        }
        rows.into_any_element()
    };
    if !core.is_empty() {
        let mut groups = Vec::new();
        for g in CORE_GROUPS {
            let rows: Vec<_> = core.iter().filter(|c| c.0 == g).collect();
            if rows.is_empty() {
                continue;
            }
            let rows: Vec<AnyElement> = rows.iter().map(|c| colour_row(this, c.1, c.2, c.3, compact, &def, window, cx)).collect();
            groups.push(div().child(colour_group_title(g)).children(rows).into_any_element());
        }
        list = list.child(columns(groups, compact));
    }
    let has_extra = extra.is_some();
    list = list.children(extra.map(|e| div().mt(px(8.)).child(e)));
    if !all.is_empty() {
        list = list.child(
            crate::ui::anchor(
                if compact { "float-all-colours" } else { "all-colours" },
                div()
                    .id(if compact { "float-all-toggle" } else { "all-toggle" })
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .mt(px(10.))
                    .py(px(2.))
                    .text_size(px(12.))
                    .text_color(theme::link())
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if compact {
                            this.settings_ui.float_all = Some(!expanded);
                        } else {
                            this.settings_ui.all_colours = Some(!expanded);
                        }
                        cx.notify();
                    }))
                    .child(icon(if expanded { "chevron-down" } else { "chevron-right" }, 14., theme::link()))
                    .child("All colours")
                    .child(div().ml(px(6.)).text_size(px(11.)).text_color(theme::text_secondary()).child("every token; the ones above set most of them")),
            ),
        );
    }
    if expanded {
        let mut groups = Vec::new();
        for (g, tokens) in &all {
            let rows: Vec<AnyElement> = tokens.iter().map(|t| colour_row(this, t, token_label(t), t, compact, &def, window, cx)).collect();
            groups.push(div().child(colour_group_title(g)).children(rows).into_any_element());
        }
        list = list.child(div().mt(px(8.)).child(columns(groups, compact)));
    }
    if core.is_empty() && all.is_empty() && !has_extra {
        list = list.child(div().my(px(4.)).text_size(px(12.)).text_color(theme::text_secondary()).child(format!("No colour matches \"{}\"", filter.trim())));
    }
    list.into_any_element()
}

/// `LaneColours`: drag one to reorder, click to change it in the picker.
fn lane_colours(this: &mut GitVolt, compact: bool, cx: &mut Context<GitVolt>) -> AnyElement {
    let (def, _) = this.edited_theme();
    let lanes: Vec<Col> = def.graph.clone();
    let mut row = div().relative().flex().gap(px(if compact { 4. } else { 6. }));
    for (i, c) in lanes.iter().enumerate() {
        let c = *c;
        let graph: Vec<Col> = lanes.clone();
        let weak = cx.weak_entity();
        let swatch = colour::swatch(
            format!("lane-colour:{i}"),
            &c.css(),
            format!("Lane {} colour", i + 1),
            false,
            if compact { 18. } else { 24. },
            Rc::new(move |v, _, cx| {
                if let Some(n) = Col::parse(v) {
                    let mut g = graph.clone();
                    g[i] = n;
                    theme::preview(Some(ThemePatch { colors: Vec::new(), graph: Some(g) }));
                    cx.refresh_windows();
                }
            }),
            Rc::new(move |v, _, cx| {
                preview_colour(None, None, cx);
                let v = v.to_string();
                weak.update(cx, |this, cx| {
                    if Col::parse(&v) != Some(c) {
                        this.set_lane_colour(i, &v, cx);
                    }
                })
                .ok();
            }),
            Rc::new(|_, cx| preview_colour(None, None, cx)),
            cx,
        );
        let dragging = this.settings_ui.lane_drag.is_some_and(|d| d.0 == i && d.1);
        row = row.child(crate::ui::anchor(
            format!("lane:{i}"),
            div()
                .id(("lane", i))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.))
                .when(!compact, |d| d.flex_1().min_w_0().px(px(2.)).py(px(6.)).rounded(px(6.)).border_1().border_color(theme::section_border()).bg(if dragging { theme::panel_bg1() } else { theme::panel_bg() }))
                .when(compact, |d| d.p(px(3.)))
                .cursor_grab()
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.settings_ui.lane_drag = Some((i, false, e.position));
                }))
                .child(swatch)
                .when(!compact, |d| d.child(div().max_w_full().overflow_hidden().font_family(crate::diff::view::FONT).text_size(px(11.)).text_color(theme::text_secondary()).child(c.css()))),
        ));
    }
    row.on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| this.on_lane_drag(e, cx))).on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.end_lane_drag(cx))).into_any_element()
}

/// A card's picture of a theme, in that theme's own colours.
fn card_preview(def: &ThemeDef) -> Div {
    let get = |t: &str| def.colors.get(t).copied().unwrap_or(Col::opaque(0, 0, 0)).hsla();
    let lanes = def.graph.iter().take(4).map(|g| div().size(px(10.)).rounded_full().border_2().border_color(g.hsla()));
    div()
        .flex()
        .gap(px(6.))
        .h(px(66.))
        .flex_none()
        .p(px(8.))
        .bg(get("app-bg0"))
        .child(div().flex_none().w(px(22.)).h_full().rounded(px(3.)).bg(get("panel-bg0")))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(5.))
                .pt(px(2.))
                .child(div().h(px(6.)).w(relative(0.7)).rounded(px(3.)).bg(get("blue")))
                .child(div().h(px(6.)).w(relative(0.9)).rounded(px(3.)).opacity(0.35).bg(get("text-normal")))
                .child(div().flex().gap(px(3.)).mt(px(2.)).children(lanes)),
        )
}

fn theme_cards(this: &mut GitVolt, cx: &mut Context<GitVolt>) -> AnyElement {
    let (current, _) = this.edited_theme();
    let list = customs(this);
    let mut cards: Vec<(ThemeDef, bool)> = theme::builtins().iter().map(|t| (t.clone(), false)).collect();
    cards.extend(list.iter().map(|c| (theme::custom_theme_def(c), true)));
    let width = this.settings_body_width() - 2.;
    // `repeat(auto-fill, minmax(150px, 1fr))`, gap 10.
    let cols = (((width + 10.) / 160.).floor() as usize).max(1);
    let card_w = (width - (cols as f32 - 1.) * 10.) / cols as f32;
    let mut grid = div().flex().flex_wrap().gap(px(10.));
    for (i, (def, custom)) in cards.into_iter().enumerate() {
        let checked = def.id == current.id;
        let id = def.id.clone();
        let label = def.label.clone();
        grid = grid.child(crate::ui::anchor(
            format!("theme-card:{}", def.id),
            div()
                .id(("theme-card", i))
                .flex()
                .flex_col()
                .flex_none()
                .w(px(card_w))
                .h(px(98.))
                .overflow_hidden()
                .rounded(px(8.))
                .border_1()
                .border_color(theme::section_border())
                .bg(theme::panel_bg())
                .text_color(theme::text_selected())
                .cursor_pointer()
                .hover(|d| d.border_color(theme::text_secondary()))
                .relative()
                .when(checked, |d| d.child(div().absolute().top_0().left_0().size_full().rounded(px(8.)).border_2().border_color(theme::blue())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let id = id.clone();
                    this.set_settings(move |s| s["theme"] = json!(id), cx);
                }))
                .child(card_preview(&def))
                .child(div().flex().flex_1().items_center().justify_between().gap(px(6.)).px(px(9.)).text_size(px(12.)).border_t_1().border_color(theme::section_border()).child(div().min_w_0().truncate().child(label)).when(custom, |d| d.child(div().flex_none().text_color(theme::text_secondary()).child("✎")))),
        ));
    }
    grid.child(crate::ui::anchor(
        "theme-paste",
        div()
            .id("theme-paste")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(card_w))
            .h(px(98.))
            .items_center()
            .justify_center()
            .gap(px(4.))
            .rounded(px(8.))
            .border_1()
            .border_dashed()
            .border_color(theme::section_border())
            .text_color(theme::text_secondary())
            .cursor_pointer()
            .hover(|d| d.text_color(theme::text_selected()).border_color(theme::text_secondary()))
            .on_click(cx.listener(|this, _, _, cx| {
                let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
                this.paste_theme(&text, cx);
            }))
            .child(icon("clipboard-paste", 16., theme::text_secondary()))
            .child("Paste a theme")
            .child(div().text_size(px(11.)).text_color(theme::text_disabled()).child(format!("or {} on this page", crate::commands::display_chord("Mod+V")))),
    ))
    .into_any_element()
}

fn strip_button(id: &'static str, label: &'static str) -> Stateful<Div> {
    div().id(id).flex().flex_none().items_center().px(px(12.)).py(px(4.)).rounded(px(4.)).border_1().border_color(theme::section_border()).bg(theme::panel_bg1()).text_color(theme::text_selected()).whitespace_nowrap().cursor_pointer().hover(|d| d.bg(theme::hover_row())).child(label)
}

/// Under the cards: for a custom theme, its name and what to do with it; for a built-in, a line and Edit mode.
fn editing_strip(this: &mut GitVolt, window: &Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let (def, custom) = this.edited_theme();
    let edit_mode = crate::ui::anchor(
        "theme-edit-mode",
        div().id("theme-edit-mode").flex().flex_none().items_center().px(px(12.)).py(px(4.)).rounded(px(4.)).border_1().border_color(theme::button_primary()).bg(theme::button_primary()).text_color(theme::text_on_accent()).whitespace_nowrap().cursor_pointer().on_click(cx.listener(|this, _, window, cx| this.open_floating_editor(window, cx))).child("Edit mode"),
    );
    let Some(custom) = custom else {
        return div().flex().items_center().gap(px(12.)).min_h(px(66.)).mt(px(10.)).px(px(12.)).py(px(6.)).text_size(px(12.)).text_color(theme::text_secondary()).child(div().flex_1().min_w_0().child("Change any colour below to make your own copy.")).child(edit_mode).into_any_element();
    };
    let base = custom["base"].as_str().and_then(theme::builtin).map_or("Default Dark".to_string(), |b| b.label.clone());
    let name = custom["name"].as_str().unwrap_or_default().to_string();
    let id = def.id.clone();
    let id2 = id.clone();
    let input = this.settings_field("theme-name", &name, "", Rc::new(move |v, text, cx| v.rename_theme(&id2, text, cx)), None, window, cx);
    let focused = input.read(cx).is_focused(window);
    let (json_def, json_custom) = (def.clone(), custom.clone());
    let (export_def, export_custom) = (def.clone(), custom.clone());
    let (remove_id, remove_name, remove_base) = (id.clone(), name.clone(), base.clone());
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .min_h(px(66.))
        .mt(px(10.))
        .px(px(12.))
        .py(px(6.))
        .rounded(px(8.))
        .border_1()
        .border_color(Hsla { a: 0.35, ..theme::blue() })
        .bg(Hsla { a: 0.08, ..theme::blue() })
        .text_size(px(12.))
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .gap(px(2.))
                .child("Editing ")
                .child(div().flex().flex_none().items_center().w(px(170.)).h(px(24.)).mx(px(2.)).px(px(8.)).rounded(px(4.)).bg(theme::app_bg()).border_1().border_color(if focused { theme::blue() } else { theme::section_border() }).text_color(theme::text_selected()).overflow_hidden().child(input))
                .child(" made from ")
                .child(div().font_weight(FontWeight::BOLD).text_color(theme::text_selected()).child(base))
                .child("; your other themes are untouched."),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .flex_none()
                .justify_end()
                .gap(px(8.))
                .child(edit_mode)
                .child(crate::ui::anchor("theme-copy", strip_button("theme-copy", "Copy as JSON").on_click(cx.listener(move |_, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(theme_to_json(&json_def, Some(&json_custom))));
                    toast::show(format!("Copied {} as JSON", json_def.label), cx);
                }))))
                .child(crate::ui::anchor("theme-export", strip_button("theme-export", "Export file…").on_click(cx.listener(move |this, _, _, cx| {
                    this.save_export(export_file_name(&export_def.label, "theme"), theme_to_json(&export_def, Some(&export_custom)), cx);
                }))))
                .child(crate::ui::anchor("theme-delete", strip_button("theme-delete", "Delete").text_color(theme::status_deleted()).on_click(cx.listener(move |_, _, window, cx| {
                    let weak = cx.weak_entity();
                    let id = remove_id.clone();
                    let req = arm::ArmRequest { body: format!("{remove_base} is selected instead. A theme you copied or exported can be pasted back.").into(), ..arm::ArmRequest::danger(format!("Click again to delete {remove_name}"), format!("Delete {remove_name}?"), "Delete") };
                    arm::request(
                        req,
                        arm::Origin::control("theme-delete"),
                        move |answer, _, cx| {
                            if answer.ok {
                                weak.update(cx, |v, cx| v.delete_theme(&id, cx)).ok();
                            }
                        },
                        window,
                        cx,
                    );
                })))),
        )
        .into_any_element()
}

/// A static slice of the app beside the colours: drawn with the theme's tokens, previews included.
fn live_preview() -> AnyElement {
    let chip = |text: &'static str, lane: usize, head: bool| div().flex().flex_none().items_center().h(px(22.)).px(px(5.)).rounded(px(3.)).bg(theme::lane_alpha(lane, if head { 0.45 } else { 0.25 })).text_color(if head { theme::text_selected() } else { theme::text_normal() }).child(text);
    let dot = |x: f32, y: f32, lane: usize| div().absolute().left(px(x)).top(px(y)).size(px(16.)).rounded_full().bg(theme::lane(lane)).border_1().border_color(theme::node_fill());
    let line = |x: f32, y: f32, h: f32, lane: usize| div().absolute().left(px(x)).top(px(y)).w(px(2.)).h(px(h)).bg(theme::lane(lane));
    let row = |i: usize, chip_el: Option<Div>, summary: &'static str, selected: bool| {
        div().absolute().top(px(i as f32 * 28.)).left_0().right_0().h(px(28.)).flex().items_center().when(selected, |d| d.bg(theme::selected_row())).child(div().flex_none().w(px(120.)).pl(px(8.)).children(chip_el)).child(div().flex_none().w(px(66.))).child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(if selected { theme::text_selected() } else { theme::text_normal() }).child(summary))
    };
    let graph = div()
        .relative()
        .mx(px(-8.))
        .h(px(140.))
        .overflow_hidden()
        .child(row(0, Some(chip("main", 0, true)), "Merge branch 'feature/trade-ui'", false))
        .child(row(1, Some(chip("feature/trade-ui", 1, false)), "Trade screen lists the cargo hold", false))
        .child(row(2, None, "Fuel gauge reads the tank", true))
        .child(row(3, None, "Trade screen: buy and sell prices", false))
        .child(row(4, None, "Cargo hold has a weight limit", false))
        .child(line(132., 14., 112., 0))
        .child(line(154., 42., 84., 1))
        .child(dot(125., 6., 0))
        .child(dot(147., 34., 1))
        .child(dot(125., 62., 0))
        .child(dot(147., 90., 1))
        .child(dot(125., 118., 0));
    let code = |text: &'static str, bg: Option<Hsla>| div().px(px(4.)).py(px(1.)).font_family(crate::diff::view::FONT).text_size(px(10.)).whitespace_nowrap().overflow_hidden().text_color(theme::text_selected()).when_some(bg, |d, b| d.bg(b)).child(text);
    div()
        .flex()
        .flex_col()
        .flex_none()
        .w(px(340.))
        .p(px(8.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme::section_border())
        .bg(theme::app_bg())
        .text_size(px(11.))
        .text_color(theme::text_normal())
        .child(div().mb(px(4.)).text_size(px(10.)).text_color(theme::text_secondary()).child("LIVE PREVIEW"))
        .child(graph)
        .child(div().flex().items_center().gap(px(6.)).mt(px(6.)).px(px(6.)).py(px(4.)).rounded_t(px(4.)).bg(theme::panel_bg()).child(icon("status-modified", 12., theme::status_modified())).child("economy.rs"))
        .child(div().px(px(6.)).pb(px(6.)).rounded_b(px(4.)).bg(theme::panel_bg()).child(code("  let base = cargo.value();", None)).child(code("- let fee = base * 0.9;", Some(theme::diff_removed_line()))).child(code("+ let fee = base * SELL_SHARE;", Some(theme::diff_inserted_line()))))
        .into_any_element()
}

/// Settings › Theme's body.
pub fn theme_body(this: &mut GitVolt, window: &mut Window, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    let cards = theme_cards(this, cx);
    let strip = editing_strip(this, window, cx);
    let lanes = lane_colours(this, false, cx);
    let colours = colour_list(this, "", false, None, window, cx);
    let wide = window.viewport_size().width.as_f32() > 900.;
    vec![
        div().mt(px(20.)).child(group_title("Theme", None)).child(cards).child(strip).into_any_element(),
        div().mt(px(20.)).child(group_title("Graph lanes", Some("drag to reorder, click to change"))).child(lanes).into_any_element(),
        div().mt(px(20.)).child(group_title("Colours", None)).child(if wide { div().flex().items_start().gap(px(20.)).child(div().flex_1().min_w_0().child(colours)).child(live_preview()) } else { div().flex().flex_col().gap(px(20.)).child(colours).child(live_preview()) }).into_any_element(),
    ]
}

/// `ThemeSelect`: the theme's control in the search results.
pub fn theme_select(this: &mut GitVolt, _: &'static Def, _: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let (def, _) = this.edited_theme();
    let mut options: Vec<(String, SharedString, Option<SharedString>)> = theme::builtins().iter().map(|t| (t.id.clone(), t.label.clone().into(), None)).collect();
    options.extend(customs(this).iter().map(|c| (c["id"].as_str().unwrap_or_default().to_string(), c["name"].as_str().unwrap_or_default().to_string().into(), None)));
    let weak = cx.weak_entity();
    let on_change: OnSelect<String> = Rc::new(move |v, _, cx| {
        let v = v.clone();
        weak.update(cx, |this, cx| this.set_settings(move |s| s["theme"] = json!(v), cx)).ok();
    });
    div().flex().flex_none().w(px(240.)).child(controls::select("setting:theme", def.id, Rc::new(options), None, on_change, cx)).into_any_element()
}

const FLOAT_W: f32 = 320.;

impl GitVolt {
    fn open_floating_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_ui.floating = true;
        let filter = self.settings_ui.float_filter.get_or_insert_with(|| {
            cx.new(|cx| crate::ui::input::TextInput::new(cx).placeholder("Find a colour").claim_escape())
        }).clone();
        cx.subscribe(&filter, |this, input, e: &crate::ui::input::InputEvent, cx| match e {
            crate::ui::input::InputEvent::Changed => cx.notify(),
            crate::ui::input::InputEvent::Escape => {
                if input.read(cx).text().is_empty() {
                    this.floating_done(cx);
                } else {
                    input.update(cx, |i, cx| i.set_text("", cx));
                    cx.notify();
                }
            }
            _ => {}
        })
        .detach();
        let f = filter.read(cx).focus.clone();
        window.focus(&f, cx);
        cx.notify();
    }

    fn floating_done(&mut self, cx: &mut Context<Self>) {
        self.settings_ui.floating = false;
        self.settings_ui.page = "theme".into();
        let handle = self.settings_ui.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        self.focus_next_frame = Some(handle);
        cx.notify();
    }

    pub fn on_float_drag(&mut self, e: &MouseMoveEvent, viewport: Size<Pixels>, cx: &mut Context<Self>) {
        let Some((from, at)) = self.settings_ui.float_drag else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            self.settings_ui.float_drag = None;
            return;
        }
        let (dx, dy) = ((e.position.x - from.x).as_f32(), (e.position.y - from.y).as_f32());
        self.settings_ui.float_at = in_window(at.0 + dx, at.1 + dy, viewport);
        cx.notify();
    }
}

/// The panel's offset from its place at the lower right, moved back inside the window.
fn in_window(x: f32, y: f32, viewport: Size<Pixels>) -> (f32, f32) {
    let (vw, vh) = (viewport.width.as_f32(), viewport.height.as_f32());
    let h = 560f32.min(vh * 0.8);
    (x.clamp(-(vw - FLOAT_W - 24.).max(0.), 24.), y.clamp(-(vh - h - 24.).max(0.), 24.))
}

/// `FloatingThemeEditor`: the editor as a small panel over the live app, which stays usable behind it.
pub fn floating_editor(this: &mut GitVolt, window: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let (def, _) = this.edited_theme();
    let viewport = window.viewport_size();
    let (fx, fy) = in_window(this.settings_ui.float_at.0, this.settings_ui.float_at.1, viewport);
    let filter = this.settings_ui.float_filter.clone().expect("opened with it");
    let q = filter.read(cx).text().to_string();
    let lanes = q.trim().is_empty() || "graph lanes".contains(&q.trim().to_lowercase());
    let extra = lanes.then(|| {
        let l = lane_colours(this, true, cx);
        div().child(colour_group_title("Graph lanes")).child(l).into_any_element()
    });
    let list = colour_list(this, &q, true, extra, window, cx);
    let h = 560f32.min(viewport.height.as_f32() * 0.8);
    let bar = div()
        .id("theme-float-bar")
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .h(px(34.))
        .pl(px(10.))
        .pr(px(8.))
        .border_b_1()
        .border_color(theme::section_border())
        .text_color(theme::text_selected())
        .cursor_move()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            this.settings_ui.float_drag = Some((e.position, this.settings_ui.float_at));
        }))
        .child(icon("grip-vertical", 14., theme::text_disabled()))
        .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(def.label.clone()))
        .child(crate::ui::anchor(
            "theme-float-done",
            div().id("theme-float-done").flex().flex_none().px(px(10.)).py(px(3.)).rounded(px(4.)).border_1().border_color(theme::button_primary()).bg(theme::button_primary()).text_color(theme::text_on_accent()).cursor_pointer().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(cx.listener(|this, _, _, cx| this.floating_done(cx))).child("Done"),
        ));
    let find = {
        let weak = cx.weak_entity();
        let f = filter.clone();
        div().flex_none().px(px(10.)).pt(px(8.)).pb(px(4.)).child(controls::clearable("theme-float-find", &filter, move |_, cx| {
            f.update(cx, |i, cx| i.set_text("", cx));
            weak.update(cx, |_, cx| cx.notify()).ok();
        }, window, cx))
    };
    div()
        .id("theme-float")
        .absolute()
        .right(px(24. - fx))
        .bottom(px(24. - fy))
        .occlude()
        .flex()
        .flex_col()
        .w(px(FLOAT_W))
        .h(px(h))
        .overflow_hidden()
        .bg(theme::panel_bg())
        .border_1()
        .border_color(theme::section_border())
        .rounded(px(8.))
        .shadow(crate::ui::shadow(theme::shadow_modal(), 14., 40.))
        .text_size(px(12.))
        .text_color(theme::text_normal())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, window, cx| this.on_float_drag(e, window.viewport_size(), cx)))
        .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.settings_release(cx)))
        .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
            let k = &e.keystroke;
            if k.key == "escape" && !k.modifiers.modified() {
                cx.stop_propagation();
                this.floating_done(cx);
            }
        }))
        .child(bar)
        .child(find)
        .child(div().id("theme-float-body").flex_1().min_h_0().overflow_y_scroll().px(px(10.)).pt(px(6.)).pb(px(10.)).child(list))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_name_takes_the_next_free_number() {
        let taken = vec!["Nord (Custom)".to_string(), "Nord (Custom) (2)".to_string()];
        assert_eq!(unique_name("Fresh", &taken), "Fresh");
        assert_eq!(unique_name("nord (custom)", &taken), "nord (custom) (3)");
        assert_eq!(unique_name("Nord (Custom) (2)", &taken), "Nord (Custom) (3)");
    }

    #[test]
    fn an_export_file_is_named_from_its_slug() {
        assert_eq!(export_file_name("Nord (Custom)", "theme"), "nord-custom.gitvolt-theme.json");
        assert_eq!(export_file_name("%%", "theme"), "theme.gitvolt-theme.json");
    }

    #[test]
    fn a_shared_theme_goes_out_and_comes_back() {
        let def = theme::builtin("nord").unwrap();
        let text = theme_to_json(def, None);
        assert!(text.starts_with("{\n  \"gitvolt\": \"theme\",\n  \"version\": 1,\n  \"name\": \"Nord\""), "{text}");
        let back = theme_from_json(&text, &[], "custom-00000001".into()).unwrap();
        assert_eq!(back["base"], "nord");
        assert_eq!(back["name"], "Nord");
    }

    #[test]
    fn a_pasted_theme_is_checked_and_a_fence_is_tolerated() {
        let t = "```json\n{\"gitvolt\":\"theme\",\"version\":1,\"name\":\"X\",\"base\":\"bogus\",\"colors\":{\"blue\":\"#123\",\"nope\":\"#123\",\"red\":\"junk\"},\"graph\":[\"#000000\"]}\n```";
        let v = theme_from_json(t, &[json!({ "name": "X" })], "custom-00000002".into()).unwrap();
        assert_eq!(v["name"], "X (2)");
        assert_eq!(v["base"], "default-dark");
        assert_eq!(v["colors"], json!({ "blue": "#123" }));
        assert_eq!(v["graph"], json!([]));
        assert!(theme_from_json("{\"gitvolt\":\"branch-preset\",\"version\":1}", &[], "x".into()).is_none());
        assert!(theme_from_json("not json", &[], "x".into()).is_none());
    }

    #[test]
    fn a_typed_colour_takes_its_hash_optional() {
        assert_eq!(typed_colour("abc").as_deref(), Some("#aabbcc"));
        assert_eq!(typed_colour(" #112233 ").as_deref(), Some("#112233"));
        assert_eq!(typed_colour("rgba(1, 2, 3, 0.5)").as_deref(), Some("rgba(1, 2, 3, 0.5)"));
        assert_eq!(typed_colour("nope"), None);
        assert_eq!(as_hex(Col { r: 1, g: 2, b: 3, a: 0.5 }), "#01020380");
    }
}
