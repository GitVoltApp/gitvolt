//! Settings (`ui/src/settings/`): a large overlay with the registry's pages in a nav, a search over every setting, and the
//! shown page. `GitVolt::settings` holds what the core says (`AppSettings`); `set_settings` changes and saves it, and
//! `settings_changed` makes each change take effect.

pub mod effects;
pub mod fonts;
pub mod frame;
pub mod pages;
pub mod search;
pub mod themes;

use crate::app::GitVolt;
use crate::theme;
use crate::ui::input::{InputEvent, TextInput};
use gpui::*;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub type Commit = Rc<dyn Fn(&mut GitVolt, &str, &mut Context<GitVolt>)>;

/// A text box that applies on Enter or blur (`fields.tsx` `TextField`, `NumberField`).
pub struct Field {
    pub input: Entity<TextInput>,
    pub commit: Commit,
    /// Each keystroke's text: a live preview.
    pub draft: Option<Commit>,
    pub shown: String,
}

/// `useSettingsUi`, with what each page keeps while it's on screen.
pub struct SettingsUi {
    pub open: bool,
    pub page: String,
    /// A setting to flash (deep link).
    pub flash: Option<String>,
    /// The floating theme editor instead of the overlay.
    pub floating: bool,
    pub float_at: (f32, f32),
    pub float_drag: Option<(Point<Pixels>, (f32, f32))>,
    pub float_filter: Option<Entity<TextInput>>,
    pub search: Option<Entity<TextInput>>,
    pub fields: HashMap<String, Field>,
    pub focus: Option<FocusHandle>,
    pub return_to: Option<FocusHandle>,
    pub scroll: ScrollHandle,
    pub nav_cursor: Option<usize>,
    /// All colours, by hand; `None` follows the filter.
    pub all_colours: Option<bool>,
    pub float_all: Option<bool>,
    /// A −/+ button held: the press its repeats belong to.
    pub hold: u64,
    pub scroll_drag: Option<(f32, f32)>,
    /// A lane being dragged: its index, and where the pointer is over.
    pub lane_drag: Option<(usize, bool, Point<Pixels>)>,
    /// `custom-<hex>` counter for a test; a new id's randomness otherwise.
    pub fonts: Option<Rc<Vec<Value>>>,
    pub fonts_loading: bool,
    pub font_picker: Option<fonts::FontPicker>,
    pub font_preview: Option<String>,
    pub font_draft: HashMap<String, String>,
    pub font_asked: HashSet<String>,
    /// Editor pickers' custom command: what's being typed, its error.
    pub editor_custom: HashMap<String, (bool, Option<String>)>,
    pub editor_seq: u64,
    /// Git identity's last read: the shown tab's.
    pub identity: Option<Value>,
    pub identity_for: Option<(u32, String)>,
    /// A preset page's and the file colours page's own state.
    #[allow(dead_code)]
    pub presets: crate::presets::settings_page::PageState,
    pub file_colours: crate::filecolours::settings_page::PageState,
    /// Hosts' "Add a host" kind.
    pub host_kind: String,
    pub keyboard_filter: Option<Entity<TextInput>>,
}

impl Default for SettingsUi {
    fn default() -> Self {
        Self { open: false, page: "general".into(), flash: None, floating: false, float_at: (0., 0.), float_drag: None, float_filter: None, search: None, fields: HashMap::new(), focus: None, return_to: None, scroll: ScrollHandle::new(), nav_cursor: None, all_colours: None, float_all: None, hold: 0, scroll_drag: None, lane_drag: None, fonts: None, fonts_loading: false, font_picker: None, font_preview: None, font_draft: HashMap::new(), font_asked: HashSet::new(), editor_custom: HashMap::new(), editor_seq: 0, identity: None, identity_for: None, presets: Default::default(), file_colours: Default::default(), host_kind: "gitlab".into(), keyboard_filter: None }
    }
}

impl GitVolt {
    /// `show`: opens Settings, on the page of setting `focus` if it's given, its row flashing.
    pub fn show_settings(&mut self, focus: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let ui = &mut self.settings_ui;
        let handle = ui.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        ui.return_to = if ui.open { ui.return_to.take() } else { window.focused(cx) };
        ui.open = true;
        ui.floating = false;
        ui.flash = focus.map(str::to_string);
        ui.nav_cursor = None;
        if let Some(page) = focus.and_then(pages::def).map(|d| d.page) {
            ui.page = page.to_string();
        }
        let search = self.settings_search(cx);
        search.update(cx, |i, cx| i.set_text("", cx));
        if focus.is_some() {
            self.scroll_to_flash(cx);
            window.focus(&handle, cx);
        } else {
            let f = search.read(cx).focus.clone();
            window.focus(&f, cx);
        }
        cx.notify();
    }

    fn scroll_to_flash(&mut self, cx: &mut Context<Self>) {
        // The flash ends by itself: nothing runs after it.
        let timer = cx.background_executor().timer(std::time::Duration::from_millis(1500));
        cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |v, cx| {
                v.settings_ui.flash = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ui = &mut self.settings_ui;
        ui.open = false;
        ui.floating = false;
        ui.flash = None;
        ui.font_picker = None;
        ui.fields.clear();
        if let Some(back) = ui.return_to.take() {
            window.focus(&back, cx);
        }
        self.settings_commit_all(cx);
        cx.notify();
    }

    /// The search box.
    pub fn settings_search(&mut self, cx: &mut Context<Self>) -> Entity<TextInput> {
        if let Some(s) = &self.settings_ui.search {
            return s.clone();
        }
        let input = cx.new(|cx| TextInput::new(cx).placeholder("Search settings").claim_escape_with_text());
        cx.subscribe(&input, |this, input, e: &InputEvent, cx| match e {
            InputEvent::Changed => {
                this.settings_ui.scroll = ScrollHandle::new();
                cx.notify();
            }
            InputEvent::Escape => {
                input.update(cx, |i, cx| i.set_text("", cx));
                cx.notify();
            }
            _ => {}
        })
        .detach();
        self.settings_ui.search = Some(input.clone());
        input
    }

    pub fn settings_query(&self, cx: &App) -> String {
        self.settings_ui.search.as_ref().map_or(String::new(), |s| s.read(cx).text().to_string())
    }

    /// A field by id, made once; its `commit` and `value` follow the frame that draws it.
    #[allow(clippy::too_many_arguments)]
    pub fn settings_field(&mut self, id: &str, value: &str, placeholder: &str, commit: Commit, draft: Option<Commit>, window: &Window, cx: &mut Context<Self>) -> Entity<TextInput> {
        if !self.settings_ui.fields.contains_key(id) {
            let placeholder = SharedString::from(placeholder.to_string());
            let input = cx.new(|cx| {
                let mut i = TextInput::new(cx).placeholder(placeholder).enter_submits();
                i.set_text(value, cx);
                i
            });
            let key = id.to_string();
            cx.subscribe(&input, move |this, input, e: &InputEvent, cx| match e {
                InputEvent::Submit { .. } | InputEvent::Blurred => {
                    let text = input.read(cx).text().to_string();
                    if let Some(c) = this.settings_ui.fields.get(&key).map(|f| f.commit.clone()) {
                        c(this, &text, cx);
                    }
                }
                InputEvent::Changed => {
                    let text = input.read(cx).text().to_string();
                    if let Some(d) = this.settings_ui.fields.get(&key).and_then(|f| f.draft.clone()) {
                        d(this, &text, cx);
                    }
                }
                _ => {}
            })
            .detach();
            self.settings_ui.fields.insert(id.to_string(), Field { input, commit: commit.clone(), draft: draft.clone(), shown: value.to_string() });
        }
        let f = self.settings_ui.fields.get_mut(id).expect("just made");
        f.commit = commit;
        f.draft = draft;
        let input = f.input.clone();
        if f.shown != value && !input.read(cx).is_focused(window) {
            f.shown = value.to_string();
            input.update(cx, |i, cx| i.set_text(value, cx));
        }
        input
    }

    /// `setSettings`: changes the settings, saves them through the core, and applies what changed.
    pub fn set_settings(&mut self, change: impl FnOnce(&mut Value), cx: &mut Context<Self>) {
        if self.settings.is_null() {
            return;
        }
        change(&mut self.settings);
        let body = crate::write::call("saveSettings", json!({ "settings": self.settings }));
        if !cfg!(test) {
            let api = self.api.clone();
            self.rt.spawn(async move {
                if let Ok(req) = crate::write::request_of(body) {
                    api.dispatch(req).await.ok();
                }
            });
        }
        self.settings_changed(cx);
        cx.notify();
    }

    /// Fields that a close takes along: nothing to do beyond what their blur did.
    fn settings_commit_all(&mut self, _cx: &mut Context<Self>) {}

    /// What the settings say now, applied.
    pub fn settings_changed(&mut self, cx: &mut Context<Self>) {
        self.sync_theme(cx);
        self.sync_effects(cx);
    }

    /// `bindThemeToSettings`: the theme the settings name, resolved against the custom ones.
    pub fn sync_theme(&mut self, cx: &mut Context<Self>) {
        let id = self.settings["theme"].as_str().unwrap_or(theme::DEFAULT_THEME_ID);
        let def = theme::theme_def(id, &self.settings["customThemes"]);
        let editor_before = theme::applied().editor_theme.clone();
        if theme::apply(def) {
            self.theme_shown(editor_before != theme::applied().editor_theme, cx);
        }
    }

    /// The colours on screen changed: everything redraws, and what's built from colours is built again.
    pub fn theme_shown(&mut self, recolour_files: bool, cx: &mut Context<Self>) {
        self.colours_cache.borrow_mut().clear();
        if recolour_files {
            self.recolour_open_files(cx);
        }
        cx.refresh_windows();
    }

    /// The token colours of what's open come from the theme: each open file's read again, keeping what's unsaved.
    pub fn recolour_open_files(&mut self, cx: &mut Context<Self>) {
        let repos: Vec<u32> = self.tabs.iter().filter(|t| !t.is_open_tab()).map(|t| t.id).collect();
        for repo in repos {
            self.recolour_file(repo, cx);
        }
    }
}
