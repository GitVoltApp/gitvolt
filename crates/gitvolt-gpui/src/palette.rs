//! The command palette (`ui/src/palette`): Ctrl+P or the toolbar's Actions. Fuzzy search over actions,
//! branches and tags, files at HEAD, and tabs, ranked by group, then score as `fuzzysort` ranks; `>`,
//! `@`, `/` and `#` narrow it to one group. `choose` opens it as a picker of one list.

use crate::app::{GitVolt, icon};
use crate::commands;
use crate::sidebar::{ItemKind, SectionKind};
use crate::theme;
use crate::ui::Action;
use crate::ui::fuzzy::{Target, go};
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::modal::{self, Modal};
use gitvolt_core::api::Request;
use gpui::prelude::*;
use gpui::*;
use std::cell::RefCell;
use std::rc::Rc;

const ID: &str = "palette";
const PICKER_SHOWN: usize = 50;
const THRESHOLD: f64 = 0.3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteGroup {
    Action,
    Ref,
    File,
    Setting,
    Tab,
    Choice,
}

const ORDER: [PaletteGroup; 6] = [PaletteGroup::Action, PaletteGroup::Ref, PaletteGroup::File, PaletteGroup::Setting, PaletteGroup::Tab, PaletteGroup::Choice];

impl PaletteGroup {
    fn label(self) -> &'static str {
        match self {
            PaletteGroup::Action => "ACTIONS",
            PaletteGroup::Ref => "BRANCHES & TAGS",
            PaletteGroup::File => "FILES AT HEAD",
            PaletteGroup::Setting => "SETTINGS",
            PaletteGroup::Tab => "TABS & RECENT REPOS",
            PaletteGroup::Choice => "CHOOSE",
        }
    }
}

/// `parseQuery`: a prefix narrows it to a group.
pub fn parse_query(q: &str) -> (Option<PaletteGroup>, &str) {
    let group = match q.chars().next() {
        Some('>') => Some(PaletteGroup::Action),
        Some('@') => Some(PaletteGroup::Ref),
        Some('/') => Some(PaletteGroup::File),
        Some('#') => Some(PaletteGroup::Setting),
        _ => None,
    };
    (group, if group.is_some() { q[1..].trim() } else { q.trim() })
}

#[derive(Clone)]
pub struct Entry {
    pub group: PaletteGroup,
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub run: Action,
    /// Shift+Enter.
    pub alt: Option<Action>,
}

struct Prepared {
    entries: Vec<Entry>,
    targets: Vec<Target>,
}

impl Prepared {
    fn new(entries: Vec<Entry>) -> Self {
        let targets = entries.iter().map(|e| Target::new(&e.label)).collect();
        Self { entries, targets }
    }

    fn extend(&mut self, more: Vec<Entry>) {
        self.targets.extend(more.iter().map(|e| Target::new(&e.label)));
        self.entries.extend(more);
    }

    /// `searchPalette`: by group, then by score; `perGroup` each, or 50 of the one a prefix names.
    fn search(&self, query: &str, per_group: usize) -> Vec<usize> {
        let (group, text) = parse_query(query);
        let limit = if group.is_some() { 50 } else { per_group };
        let mut out = Vec::new();
        for g in ORDER.iter().filter(|g| group.is_none_or(|one| one == **g)) {
            let members: Vec<usize> = (0..self.entries.len()).filter(|i| self.entries[*i].group == *g).collect();
            if text.is_empty() {
                out.extend(members.iter().take(limit));
            } else {
                let found = go(text, members.iter().map(|i| &self.targets[*i]), limit, THRESHOLD);
                out.extend(found.into_iter().map(|i| members[i]));
            }
        }
        out
    }

    /// `searchChoices`: a picker's matches, every one, best first; no prefixes.
    fn search_choices(&self, query: &str) -> Vec<usize> {
        let text = query.trim();
        if text.is_empty() {
            return (0..self.entries.len()).collect();
        }
        go(text, self.targets.iter(), usize::MAX, THRESHOLD)
    }
}

struct PaletteState {
    prepared: Prepared,
    query: String,
    results: Vec<usize>,
    /// Everything a picker matched.
    matched: usize,
    cursor: usize,
    files_loading: bool,
    picker: Option<SharedString>,
    scroll: ScrollHandle,
}

impl PaletteState {
    fn refilter(&mut self) {
        if self.picker.is_some() {
            let all = self.prepared.search_choices(&self.query);
            self.matched = all.len();
            self.results = all.into_iter().take(PICKER_SHOWN).collect();
        } else {
            self.results = self.prepared.search(&self.query, 8);
        }
        self.cursor = 0;
    }
}

/// A picker for an action to open: only `entries`, under `title`. The writes' pickers will use it.
#[allow(dead_code)]
pub struct Choices {
    pub title: SharedString,
    pub placeholder: SharedString,
    pub entries: Vec<Entry>,
}

fn run_entry(state: &Rc<RefCell<PaletteState>>, index: usize, alt: bool, window: &mut Window, cx: &mut App) {
    let picked = {
        let s = state.borrow();
        s.results.get(index).map(|i| s.prepared.entries[*i].clone())
    };
    let Some(entry) = picked else { return };
    let run = if alt { entry.alt } else { Some(entry.run) };
    let Some(run) = run else { return };
    modal::close(ID, window, cx);
    run(window, cx);
}

impl GitVolt {
    fn action_entries(&self, this: &WeakEntity<GitVolt>, cx: &App) -> Vec<Entry> {
        commands::table()
            .iter()
            // What isn't ported is listed all the same: it says so when it's picked.
            .filter(|c| c.id != "edit.palette" && (c.usable(self, cx) || (c.run.is_none() && !self.tabs.is_empty())))
            .map(|c| {
                let (run, weak) = (c.run, this.clone());
                Entry {
                    group: PaletteGroup::Action,
                    label: c.label.clone(),
                    detail: c.chord(),
                    run: Rc::new(move |window, cx| match run {
                        Some(run) => {
                            weak.update(cx, |v, cx| run(v, window, cx)).ok();
                        }
                        None => crate::ui::toast::show(crate::ui::menu::NOT_YET, cx),
                    }),
                    alt: None,
                }
            })
            .collect()
    }

    fn ref_entries(&self, this: &WeakEntity<GitVolt>) -> Vec<Entry> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let jump = |target: Option<String>, this: &WeakEntity<GitVolt>| -> Action {
            let weak = this.clone();
            Rc::new(move |_, cx| {
                let target = target.clone();
                weak.update(cx, |v, cx| v.jump_to(target, cx)).ok();
            })
        };
        let mut out = Vec::new();
        for s in &tab.side.sections {
            for item in &s.items {
                let (label, detail): (String, &str) = match (&item.kind, s.kind) {
                    (ItemKind::Local { .. }, SectionKind::Local) => (item.name.clone(), "local · Shift+Enter checks out"),
                    (ItemKind::Remote { remote, .. }, SectionKind::Remote) => (format!("{remote}/{}", item.name), "remote · Shift+Enter checks out"),
                    (ItemKind::Tag { .. }, SectionKind::Tags) => (item.name.clone(), "tag"),
                    _ => continue,
                };
                out.push(Entry { group: PaletteGroup::Ref, label: label.into(), detail: Some(detail.into()), run: jump(item.target.clone(), this), alt: None });
            }
        }
        out
    }

    fn tab_entries(&self, this: &WeakEntity<GitVolt>) -> Vec<Entry> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let weak = this.clone();
                Entry {
                    group: PaletteGroup::Tab,
                    label: t.name.clone(),
                    detail: Some("tab".into()),
                    run: Rc::new(move |_, cx| {
                        weak.update(cx, |v, cx| v.go_to_tab(i, cx)).ok();
                    }),
                    alt: None,
                }
            })
            .chain(self.profile["recent"].as_array().map_or(&[][..], Vec::as_slice).iter().filter(|r| !self.tabs.iter().any(|t| Some(t.worktree.as_str()) == r["path"].as_str())).map(|r| {
                let (weak, path) = (this.clone(), r["path"].as_str().unwrap_or_default().to_string());
                Entry {
                    group: PaletteGroup::Tab,
                    label: r["name"].as_str().unwrap_or_default().to_string().into(),
                    detail: Some(path.clone().into()),
                    run: Rc::new(move |_, cx| {
                        let path = path.clone();
                        weak.update(cx, |v, cx| v.open(path, cx)).ok();
                    }),
                    alt: None,
                }
            }))
            .collect()
    }

    fn head_id(&self) -> Option<String> {
        let g = self.tabs.get(self.active)?.graph.as_ref()?;
        Some(g.rows.get(g.head_row?)?.id.to_string())
    }

    /// Every file at HEAD, for the palette's `/`.
    fn read_palette_files(&self, state: Rc<RefCell<PaletteState>>, this: WeakEntity<GitVolt>, cx: &mut Context<Self>) {
        let (Some(tab), Some(id)) = (self.tabs.get(self.active), self.head_id()) else {
            state.borrow_mut().files_loading = false;
            return;
        };
        if cfg!(test) {
            state.borrow_mut().files_loading = false;
            return;
        }
        let (repo, api) = (tab.id, self.api.clone());
        let at = id.clone();
        let done = self.rt.spawn(async move { api.dispatch(Request::TreeFiles { repo, id: at }).await });
        cx.spawn(async move |_, cx| {
            let paths: Vec<String> = match done.await {
                Ok(Ok(v)) => v.as_array().map_or(Vec::new(), |a| a.iter().filter_map(|p| p.as_str().map(str::to_string)).collect()),
                _ => Vec::new(),
            };
            cx.update(|cx| {
                let entries = paths
                    .into_iter()
                    .map(|p| {
                        let (a, b) = (this.clone(), this.clone());
                        let (open, history) = (p.clone(), p.clone());
                        let sha = id.clone();
                        let sha2 = id.clone();
                        Entry {
                            group: PaletteGroup::File,
                            label: p.into(),
                            detail: Some("Shift+Enter: file history".into()),
                            run: Rc::new(move |_, cx| {
                                let (p, s) = (open.clone(), sha.clone());
                                a.update(cx, |v, cx| v.open_path_at(&p, &s, false, cx)).ok();
                            }),
                            alt: Some(Rc::new(move |_, cx| {
                                let (p, s) = (history.clone(), sha2.clone());
                                b.update(cx, |v, cx| v.open_path_at(&p, &s, true, cx)).ok();
                            })),
                        }
                    })
                    .collect();
                let mut s = state.borrow_mut();
                s.prepared.extend(entries);
                s.files_loading = false;
                let (query, cursor) = (s.query.clone(), s.cursor);
                let _ = query;
                s.refilter();
                s.cursor = cursor.min(s.results.len().saturating_sub(1));
                cx.refresh_windows();
            });
        })
        .detach();
    }

    /// Opens a file as it is at a commit, in its File View.
    pub fn open_path_at(&mut self, path: &str, sha: &str, history: bool, cx: &mut Context<Self>) {
        let file = crate::details::file_at_commit(path, sha);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.history = None;
            tab.diff = Some(crate::diff::view::DiffState::carrying(file, tab.diff.as_ref()));
            if let Some(d) = tab.diff.as_mut() {
                d.view = crate::diff::view::ViewKind::File;
            }
            tab.sidebar_open = false;
        }
        self.read_contents(false, cx);
        if history {
            self.open_history(false, cx);
        }
        cx.notify();
    }

    /// Ctrl+P, and the toolbar's Actions: pressed again, it closes.
    pub fn open_palette(&mut self, prefix: &str, window: &mut Window, cx: &mut Context<Self>) {
        if modal::is_open(ID, cx) {
            return modal::close(ID, window, cx);
        }
        let this = cx.weak_entity();
        let mut entries = self.action_entries(&this, cx);
        entries.extend(self.ref_entries(&this));
        entries.extend(self.tab_entries(&this));
        self.open_palette_with(Prepared::new(entries), prefix, None, "Type to search · > actions · @ branches · / files · # settings".into(), window, cx);
    }

    /// A picker an action opens: only its entries, searched as the palette searches.
    #[allow(dead_code)]
    pub fn choose(&mut self, choices: Choices, window: &mut Window, cx: &mut Context<Self>) {
        modal::close(ID, window, cx);
        self.open_palette_with(Prepared::new(choices.entries), "", Some(choices.title), choices.placeholder, window, cx);
    }

    fn open_palette_with(&mut self, prepared: Prepared, prefix: &str, picker: Option<SharedString>, placeholder: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| {
            let mut i = TextInput::new(cx).placeholder(placeholder).claim_arrows();
            i.set_text(prefix, cx);
            i
        });
        let wants_files = picker.is_none();
        let mut s = PaletteState { prepared, query: prefix.to_string(), results: Vec::new(), matched: 0, cursor: 0, files_loading: wants_files, picker, scroll: ScrollHandle::new() };
        s.refilter();
        let state = Rc::new(RefCell::new(s));
        if wants_files {
            self.read_palette_files(state.clone(), cx.weak_entity(), cx);
        }
        let focus = input.read(cx).focus.clone();
        let handle = window.window_handle();
        let events = cx.subscribe(&input, {
            let state = state.clone();
            move |_, input, e: &InputEvent, cx| match e {
                InputEvent::Changed => {
                    let text = input.read(cx).text().to_string();
                    let mut s = state.borrow_mut();
                    s.query = text;
                    s.refilter();
                    drop(s);
                    cx.notify();
                }
                InputEvent::Arrow { down, .. } => {
                    let mut s = state.borrow_mut();
                    let last = s.results.len().saturating_sub(1);
                    s.cursor = if *down { (s.cursor + 1).min(last) } else { s.cursor.saturating_sub(1) };
                    let at = s.cursor;
                    s.scroll.scroll_to_item(at);
                    drop(s);
                    cx.notify();
                }
                InputEvent::Submit { shift, .. } => {
                    let (shift, state) = (*shift, state.clone());
                    handle
                        .update(cx, move |_, window, cx| {
                            window.defer(cx, move |window, cx| {
                                let at = state.borrow().cursor;
                                run_entry(&state, at, shift, window, cx);
                            });
                        })
                        .ok();
                }
                _ => {}
            }
        });
        let view = state.clone();
        let input_el = input.clone();
        let mut m = Modal::new(ID, move |window, cx| palette_view(&view, &input_el, window, cx)).unframed().width(640f32.min(window.viewport_size().width.as_f32() * 0.9)).top((window.viewport_size().height.as_f32() * 0.12).round()).toggle(commands::resolve_chord("Mod+P")).focus(focus);
        m.keep.push(events);
        modal::open(m, window, cx);
    }
}

fn palette_view(state: &Rc<RefCell<PaletteState>>, input: &Entity<TextInput>, window: &mut Window, _: &mut App) -> AnyElement {
    let s = state.borrow();
    let max_h = window.viewport_size().height.as_f32() * 0.5;
    let mut list = div().id("palette-list").flex().flex_col().py(px(4.)).max_h(px(max_h)).overflow_y_scroll().track_scroll(&s.scroll);
    let mut last_group = None;
    for (i, at) in s.results.iter().enumerate() {
        let e = &s.prepared.entries[*at];
        let mut row = div().id(("palette-row", i)).flex().flex_col();
        if last_group != Some(e.group) {
            let title: SharedString = match (&s.picker, e.group) {
                (Some(t), PaletteGroup::Choice) => t.to_uppercase().into(),
                _ => e.group.label().into(),
            };
            row = row.child(div().px(px(14.)).pt(px(6.)).pb(px(2.)).text_size(px(10.)).text_color(theme::text_disabled()).child(title));
            last_group = Some(e.group);
        }
        let on = i == s.cursor;
        let (hover, click) = (state.clone(), state.clone());
        row = row.child(
            div()
                .id(("palette-item", i))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(10.))
                .h(px(26.))
                .px(px(14.))
                .when(on, |d| d.bg(theme::selected_row()).text_color(theme::text_selected()))
                .on_hover(move |entered, _, cx| {
                    if *entered {
                        let mut s = hover.borrow_mut();
                        if s.cursor != i {
                            s.cursor = i;
                            cx.refresh_windows();
                        }
                    }
                })
                .on_click(move |_, window, cx| run_entry(&click, i, false, window, cx))
                .child(div().flex_1().min_w_0().truncate().child(e.label.clone()))
                .when_some(e.detail.clone(), |d, t| d.child(div().flex_none().max_w(relative(0.4)).truncate().text_size(px(11.)).text_color(theme::text_secondary()).child(t))),
        );
        list = list.child(row);
    }
    let (group, _) = parse_query(&s.query);
    let wants_files = s.picker.is_none() && (group == Some(PaletteGroup::File) || group.is_none());
    if s.files_loading && wants_files {
        list = list.child(div().flex().items_center().gap(px(6.)).px(px(14.)).py(px(6.)).text_color(theme::text_disabled()).child(icon("loader-circle", 12., theme::text_disabled())).child("Loading files…"));
    }
    if s.results.is_empty() && !(s.files_loading && wants_files) {
        list = list.child(div().px(px(14.)).py(px(6.)).text_color(theme::text_disabled()).child("No matches"));
    }
    if s.picker.is_some() && s.matched > PICKER_SHOWN {
        list = list.child(div().px(px(14.)).py(px(6.)).text_color(theme::text_disabled()).child(format!("{} more: type to narrow them down", s.matched - PICKER_SHOWN)));
    }
    div()
        .flex()
        .flex_col()
        .w_full()
        .bg(theme::panel_bg())
        .border_1()
        .border_color(theme::section_border())
        .rounded(px(8.))
        .overflow_hidden()
        .shadow(crate::ui::shadow(theme::shadow_modal(), 10., 30.))
        .text_color(theme::text_normal())
        .child(div().flex().flex_none().h(px(40.)).items_center().px(px(14.)).text_size(px(14.)).line_height(px(19.6)).bg(theme::app_bg()).border_b_1().border_color(theme::section_border()).text_color(theme::text_selected()).child(input.clone()))
        .child(list)
        .into_any_element()
}

impl GitVolt {
    pub fn tab_group_rows(&self, _ix: usize, _this: &WeakEntity<GitVolt>) -> Vec<crate::ui::menu::MenuRow> {
        vec![crate::ui::menu::MenuRow::pending("tab.newGroup", "Add to new group", "layers")]
    }

    /// About GitVolt: its version, the build it is.
    pub fn open_about(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let git = self.git_version.clone();
        let m = Modal::new("about", move |_, _| {
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(modal::title(concat!("GitVolt ", env!("CARGO_PKG_VERSION"))))
                .child(div().text_color(theme::text_secondary()).child(format!("git {git}")))
                .child(modal::actions_row().child(crate::ui::controls::button("about-close", "Close", crate::ui::controls::ButtonKind::Plain, true).on_click(|_, window, cx| modal::close("about", window, cx))))
                .into_any_element()
        });
        modal::open(m, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn entry(group: PaletteGroup, label: &str) -> Entry {
        Entry { group, label: label.to_string().into(), detail: None, run: Rc::new(|_, _| {}), alt: None }
    }

    fn labels(p: &Prepared, found: &[usize]) -> Vec<String> {
        found.iter().map(|i| p.entries[*i].label.to_string()).collect()
    }

    #[test]
    fn a_prefix_narrows_to_a_group() {
        assert_eq!(parse_query(">fetch"), (Some(PaletteGroup::Action), "fetch"));
        assert_eq!(parse_query("@ main"), (Some(PaletteGroup::Ref), "main"));
        assert_eq!(parse_query("/x"), (Some(PaletteGroup::File), "x"));
        assert_eq!(parse_query("#y"), (Some(PaletteGroup::Setting), "y"));
        assert_eq!(parse_query(" plain "), (None, "plain"));
    }

    #[test]
    fn results_go_by_group_then_score_and_cap_each_group() {
        let p = Prepared::new(vec![entry(PaletteGroup::Ref, "fetch-branch"), entry(PaletteGroup::Action, "Find in graph"), entry(PaletteGroup::Action, "Fetch all")]);
        assert_eq!(labels(&p, &p.search("fet", 8)), ["Fetch all", "fetch-branch"]);
        assert_eq!(labels(&p, &p.search(">fet", 8)), ["Fetch all"]);
        assert_eq!(labels(&p, &p.search("", 8)), ["Find in graph", "Fetch all", "fetch-branch"]);
        assert_eq!(labels(&p, &p.search("", 1)), ["Find in graph", "fetch-branch"]);
    }

    #[test]
    fn a_picker_has_no_prefixes() {
        let p = Prepared::new(vec![entry(PaletteGroup::Choice, ">one"), entry(PaletteGroup::Choice, "two")]);
        assert_eq!(labels(&p, &p.search_choices(">o")), [">one"]);
        assert_eq!(p.search_choices("").len(), 2);
    }

    #[gpui::test]
    fn it_opens_finds_and_runs_with_enter(cx: &mut TestAppContext) {
        let (window, mut cx, _runtime) = crate::app::tests::with_rows(cx, 3);
        cx.run_until_parked();
        window.update(&mut cx, |v, window, cx| v.open_palette("", window, cx)).unwrap();
        cx.run_until_parked();
        assert!(cx.update(|_, cx| modal::is_open(ID, cx)));
        cx.simulate_input("toggle sidebar");
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(cx.update(|_, cx| !modal::is_open(ID, cx)));
        window.read_with(&cx, |v, _| assert!(v.sidebar_narrow)).unwrap();
    }

    #[gpui::test]
    fn the_shortcut_toggles_it_and_esc_closes_it(cx: &mut TestAppContext) {
        let (_window, mut cx, _runtime) = crate::app::tests::with_rows(cx, 3);
        cx.run_until_parked();
        cx.simulate_keystrokes("ctrl-p");
        cx.run_until_parked();
        assert!(cx.update(|_, cx| modal::is_open(ID, cx)));
        cx.simulate_keystrokes("ctrl-p");
        cx.run_until_parked();
        assert!(cx.update(|_, cx| !modal::is_open(ID, cx)));
        cx.simulate_keystrokes("ctrl-p");
        cx.run_until_parked();
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(cx.update(|_, cx| !modal::is_open(ID, cx)));
    }
}
