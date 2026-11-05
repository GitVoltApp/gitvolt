//! The window: title bar, tabs, toolbar, sidebar, graph, details, status bar. Laid out with the web
//! UI's measured sizes (`ui/src/{chrome,tabs,toolbar,sidebar,graph,statusbar}/*.css`).

use crate::details::{Commit, Files, details_panel};
use crate::diff::view::{DiffState, body_size, diff_panel};
use crate::graph::{RowGeometry, paint_row};
use crate::history::{FileHistory, HISTORY_H, history_panel};
use crate::open_in::{OpenIn, open_in_menu};
use crate::model::*;
use crate::sidebar::{DEFAULT_SIDEBAR_W, NARROW_W, PushCache, Section, SectionKind, SideDrag, SideState, narrow_strip, sections_of, sidebar};
use crate::theme;
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::menu::{self, MenuRow, OpenOptions};
use crate::ui::tooltip::tip;
use gitvolt_core::api::{Api, Request};
use gitvolt_core::events::AppEvent;
use gpui::prelude::*;
use gpui::*;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

actions!(gitvolt, [SelectNext, SelectPrevious, ClearSelection, Copy, SelectAll, PageDown, PageUp]);

const TITLE_H: f32 = 30.;
const TAB_H: f32 = 34.;
const TOOLBAR_H: f32 = 44.;
const STATUS_H: f32 = 22.;
const SCROLLBAR_W: f32 = 10.;
const HEADER_H: f32 = 25.;
const RESIZE: f32 = 6.;
const SHA_PAD: f32 = 6.;
pub const MONO_FONT: &str = "Noto Sans Mono";
pub const GRAPH_LIMIT: u32 = 2000;

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, Some("GitVolt")),
        KeyBinding::new("up", SelectPrevious, Some("GitVolt")),
        KeyBinding::new("escape", ClearSelection, Some("GitVolt")),
        KeyBinding::new("ctrl-c", Copy, Some("GitVolt")),
        KeyBinding::new("ctrl-a", SelectAll, Some("GitVolt")),
        KeyBinding::new("pagedown", PageDown, Some("GitVolt")),
        KeyBinding::new("pageup", PageUp, Some("GitVolt")),
    ]);
}

/// What the arrow keys move: the area last clicked in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyArea {
    Graph,
    Files,
    History,
    Diff,
}

pub struct Tab {
    pub id: u32,
    pub name: SharedString,
    /// The worktree this tab shows, as the core's answer to opening it names it.
    pub worktree: String,
    pub graph: Option<Rc<GraphView>>,
    pub side: SideState,
    pub selected: Option<usize>,
    pub commit: Option<Commit>,
    pub files: Option<Files>,
    pub diff: Option<DiffState>,
    /// File History in the diff's and the graph's place; `diff` then is the file it shows.
    pub history: Option<FileHistory>,
    pub keys: KeyArea,
    /// The file list's View all files and its sort by status hold from commit to commit.
    pub all_files: bool,
    pub sort_by_status: bool,
    pub open_in: OpenIn,
    /// The sidebar stays open beside a file (it folds to its strip by default).
    pub sidebar_open: bool,
    /// "View all files"' filter.
    pub file_filter: Entity<TextInput>,
    /// An image's zoom, while a number's being typed.
    pub zoom_input: Entity<TextInput>,
    pub list: UniformListScrollHandle,
    /// A compare in the details panel: from and to, as its bar names them.
    pub compare: Option<(SharedString, SharedString)>,
    pub error: Option<SharedString>,
    /// A reload is on its way; a change meanwhile asks for one more after it.
    pub loading: bool,
    pub again: bool,
}

impl Tab {
    pub fn new(id: u32, name: SharedString, cx: &mut Context<GitVolt>) -> Self {
        let file_filter = cx.new(|cx| TextInput::new(cx).placeholder("Filter files").claim_escape_with_text());
        cx.subscribe(&file_filter, move |this, input, e: &InputEvent, cx| match e {
            InputEvent::Changed => this.file_filter_changed(id, cx),
            InputEvent::Escape => {
                input.update(cx, |input, cx| input.set_text("", cx));
                this.file_filter_changed(id, cx);
            }
            InputEvent::Focused => {
                if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == id) {
                    tab.keys = KeyArea::Files;
                }
            }
            _ => {}
        })
        .detach();
        let zoom_input = cx.new(|cx| TextInput::new(cx).claim_escape());
        cx.subscribe(&zoom_input, |this, _, e: &InputEvent, cx| match e {
            InputEvent::Submit { .. } | InputEvent::Blurred => this.end_zoom_edit(true, cx),
            InputEvent::Escape => this.end_zoom_edit(false, cx),
            _ => {}
        })
        .detach();
        Self {
            id,
            name,
            worktree: String::new(),
            graph: None,
            side: SideState::new(id, cx),
            selected: None,
            commit: None,
            files: None,
            diff: None,
            history: None,
            keys: KeyArea::Graph,
            all_files: false,
            sort_by_status: false,
            open_in: OpenIn::default(),
            sidebar_open: false,
            file_filter,
            zoom_input,
            list: UniformListScrollHandle::new(),
            compare: None,
            error: None,
            loading: false,
            again: false,
        }
    }
}

pub type FrameHook = Box<dyn FnMut(&mut Tab, &mut Window)>;

pub struct GitVolt {
    pub api: Arc<Api>,
    pub rt: tokio::runtime::Handle,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub focus: FocusHandle,
    pub font: SharedString,
    pub git_version: SharedString,
    pub dragging_scrollbar: bool,
    /// The closed tabs' paths, newest last, for Reopen closed tab.
    pub closed_tabs: Vec<String>,
    pub sidebar_w: f32,
    /// The icon strip, by hand (Ctrl+B); a file on top folds it too, by itself.
    pub sidebar_narrow: bool,
    /// The panels' dragged heights.
    pub side_weights: HashMap<SectionKind, f32>,
    pub side_drag: SideDrag,
    pub last_pushes: PushCache,
    /// Where the keyboard goes next frame: a subscription has no window to move it with.
    pub focus_next_frame: Option<FocusHandle>,
    /// The opener used last, "Open in…"'s default for this session.
    pub last_opener: Option<String>,
    /// Images no longer shown: the frame after they go, the window drops them from its atlas.
    pub retired_images: Rc<std::cell::RefCell<Vec<Arc<RenderImage>>>>,
    /// Measuring (`GITVOLT_BENCH`, bench.rs): called once per frame.
    pub frame_hook: Option<FrameHook>,
}

impl GitVolt {
    pub fn new(api: Arc<Api>, rt: tokio::runtime::Handle, font: SharedString, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let weak = cx.weak_entity();
        crate::ui::Ui::get_mut(cx).app_keys = Some(Rc::new(move |k, window, cx| weak.update(cx, |v, cx| v.app_key(k, window, cx)).unwrap_or(false)));
        Self { api, rt, tabs: Vec::new(), active: 0, focus, font, git_version: SharedString::default(), dragging_scrollbar: false, closed_tabs: Vec::new(), sidebar_w: DEFAULT_SIDEBAR_W, sidebar_narrow: false, side_weights: HashMap::new(), side_drag: SideDrag::None, last_pushes: Rc::default(), focus_next_frame: None, last_opener: None, retired_images: Rc::default(), frame_hook: None }
    }

    /// Follows the core's events and reads the versions the status bar shows.
    pub fn listen(&mut self, cx: &mut Context<Self>) {
        let (api, rt) = (self.api.clone(), self.rt.clone());
        let mut events = api.subscribe();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        rt.spawn(async move {
            loop {
                match events.recv().await {
                    Ok(e) => {
                        if tx.send(e).is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        });
        cx.spawn(async move |this, cx| {
            while let Some(e) = rx.recv().await {
                let repo = match e {
                    AppEvent::RepoChanged { repo, .. } | AppEvent::RefsUpdated { repo } => repo,
                    _ => continue,
                };
                if this.update(cx, |this, cx| this.reload(repo, cx)).is_err() {
                    return;
                }
            }
        })
        .detach();
        let info = rt.spawn({
            let api = api.clone();
            async move { api.dispatch(Request::AppInfo).await }
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(v)) = info.await {
                let version = v["gitVersion"].as_str().unwrap_or_default().to_string();
                this.update(cx, |this, cx| {
                    this.git_version = version.into();
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub fn open(&mut self, path: String, cx: &mut Context<Self>) {
        let api = self.api.clone();
        let opened = self.rt.spawn(async move {
            let v = api.dispatch(Request::OpenRepo { path }).await?;
            let id = v["id"].as_u64().unwrap_or(0) as u32;
            api.dispatch(Request::WatchRepo { repo: id }).await?;
            Ok::<_, gitvolt_core::error::GbError>((id, v["name"].as_str().unwrap_or_default().to_string(), v["worktree"].as_str().unwrap_or_default().to_string()))
        });
        cx.spawn(async move |this, cx| {
            let result = opened.await;
            this.update(cx, |this, cx| match result {
                Ok(Ok((id, name, worktree))) => {
                    crate::bench::mark("repo_open");
                    if let Some(i) = this.tabs.iter().position(|t| t.id == id) {
                        this.active = i;
                    } else {
                        let mut tab = Tab::new(id, name.into(), cx);
                        tab.worktree = worktree;
                        this.tabs.push(tab);
                        this.list_openers(id, cx);
                        this.active = this.tabs.len() - 1;
                        this.reload(id, cx);
                    }
                    cx.notify();
                }
                Ok(Err(e)) => eprintln!("Couldn't open the repository: {}", e.message),
                Err(e) => eprintln!("Couldn't open the repository: {e}"),
            })
            .ok();
        })
        .detach();
    }

    /// Reads the tab's graph and sidebar, parsed and formatted off the UI thread.
    pub fn reload(&mut self, repo: u32, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) else { return };
        if tab.loading {
            tab.again = true;
            return;
        }
        tab.loading = true;
        let api = self.api.clone();
        let worktree = tab.worktree.clone();
        let read = self.rt.spawn(async move {
            let graph = api.graph(repo, Some(GRAPH_LIMIT), None, None, None, None).await?;
            let sidebar = api.dispatch(Request::Sidebar { repo }).await?;
            Ok::<_, gitvolt_core::error::GbError>((SendGraph(graph_view(&graph)), SendSections(sections_of(&sidebar, &worktree))))
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) else { return };
                tab.loading = false;
                match result {
                    Ok(Ok((graph, sections))) => {
                        // The selection follows its commit across a reload.
                        let selected = tab.selected.and_then(|i| tab.graph.as_ref()?.rows.get(i)).map(|r| r.id.to_string());
                        tab.selected = selected.and_then(|id| graph.0.by_sha.get(&id).copied());
                        tab.graph = Some(Rc::new(graph.0));
                        crate::bench::mark("graph_read");
                        tab.side.sections = sections.0;
                        tab.side.relist(cx);
                        tab.error = None;
                    }
                    Ok(Err(e)) => tab.error = Some(e.message.into()),
                    Err(e) => tab.error = Some(e.to_string().into()),
                }
                if std::mem::take(&mut tab.again) {
                    this.reload(repo, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let Some(row) = tab.graph.as_ref().and_then(|g| g.rows.get(ix)) else { return };
        let (kind, id, worktree) = (row.kind, row.id.to_string(), row.worktree.clone());
        tab.selected = Some(ix);
        tab.keys = KeyArea::Graph;
        tab.commit = None;
        tab.files = None;
        tab.list.scroll_to_item(ix, ScrollStrategy::Nearest);
        let repo = tab.id;
        cx.notify();
        match (kind, worktree) {
            (gitvolt_graph::NodeKind::Commit | gitvolt_graph::NodeKind::Merge | gitvolt_graph::NodeKind::Stash, _) => self.read_commit(repo, id, ix, cx),
            (gitvolt_graph::NodeKind::Wip, Some(w)) => self.read_wip(repo, w, ix, cx),
            _ => {}
        }
    }

    fn move_selection(&mut self, by: i64, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(count) = tab.graph.as_ref().map(|g| g.rows.len()) else { return };
        if count == 0 {
            return;
        }
        let at = tab.selected.map_or(0, |s| (s as i64 + by).clamp(0, count as i64 - 1) as usize);
        self.select(at, cx);
    }

    /// Shows `message` for a few seconds.
    pub fn show_toast(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        crate::ui::toast::show(message, cx);
    }

    /// Page Up and Down, in what was last clicked in.
    fn page(&mut self, down: bool, window: &Window, cx: &mut Context<Self>) {
        let height = window.viewport_size().height.as_f32();
        let sign = if down { 1. } else { -1. };
        if self.tabs.get(self.active).is_some_and(|t| t.keys == KeyArea::Files && t.files.is_some()) {
            // What's over the list: the window's bars, the commit, the list's own header.
            self.page_files(down, ((height - 400.) / 26.).max(1.) as usize, cx);
        } else {
            self.scroll_diff(sign * (height / crate::diff::view::LINE_H - 8.).max(1.), cx);
        }
    }

    /// ↑ / ↓ move what was last clicked in: the graph, the file list, the history's list or the diff.
    fn arrow(&mut self, down: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        if tab.open_in.menu.is_some() {
            return self.step_open_in(down, cx);
        }
        let by = if down { 1 } else { -1 };
        match tab.keys {
            KeyArea::History if tab.history.is_some() => self.step_history(down, cx),
            KeyArea::Files if tab.files.is_some() => self.step_files(down, cx),
            _ if self.diff_open() => self.scroll_diff(by as f32, cx),
            _ => self.move_selection(by, cx),
        }
    }

    pub fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(ix);
        self.closed_tabs.push(tab.worktree.clone());
        let api = self.api.clone();
        self.rt.spawn(async move { api.dispatch(Request::UnwatchRepo { repo: tab.id }).await });
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
        cx.notify();
    }

    pub fn close_other_tabs(&mut self, ix: usize, cx: &mut Context<Self>) {
        for i in (0..self.tabs.len()).rev() {
            if i != ix {
                self.close_tab(i, cx);
            }
        }
        self.active = 0;
    }

    pub fn close_tabs_right(&mut self, ix: usize, cx: &mut Context<Self>) {
        for i in (ix + 1..self.tabs.len()).rev() {
            self.close_tab(i, cx);
        }
    }

    pub fn reopen_tab(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.closed_tabs.pop() {
            self.open(path, cx);
        }
    }

    /// Rename…: a display name for the tab; the repository is unchanged.
    pub fn prompt_tab_name(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        let id = tab.id;
        let this = cx.weak_entity();
        let req = crate::ui::modal::PromptRequest { title: "Rename tab".into(), label: "Name".into(), initial: tab.name.to_string(), confirm_label: "Rename".into(), require_change: true, ..Default::default() };
        crate::ui::modal::prompt(
            req,
            move |answer, _, cx| {
                let Some(answer) = answer else { return };
                this.update(cx, |v, cx| {
                    if let Some(tab) = v.tabs.iter_mut().find(|t| t.id == id) {
                        tab.name = answer.value.trim().to_string().into();
                        cx.notify();
                    }
                })
                .ok();
            },
            window,
            cx,
        );
    }

    pub fn pick_repository(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: None });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.first()
            {
                let path = path.to_string_lossy().into_owned();
                this.update(cx, |this, cx| this.open(path, cx)).ok();
            }
        })
        .detach();
    }

    fn diff_open(&self) -> bool {
        self.tabs.get(self.active).is_some_and(|t| t.diff.is_some())
    }

    fn scroll_to_pointer(&mut self, y: Pixels, top: Pixels, height: Pixels, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let handle = &tab.list.0.borrow().base_handle;
        let max = handle.max_offset().y;
        let share = ((y - top) / height).clamp(0., 1.);
        handle.set_offset(point(handle.offset().x, -max * share));
        cx.notify();
    }
}

/// Built on a worker, moved to the UI thread once.
struct SendGraph(GraphView);
struct SendSections(Vec<Section>);

pub fn icon(name: &'static str, size_px: f32, color: Hsla) -> Svg {
    svg().path(SharedString::from(format!("icons/{name}.svg"))).flex_none().size(px(size_px)).text_color(color)
}

fn title_bar(this: &GitVolt, window: &Window, cx: &mut Context<GitVolt>) -> impl IntoElement {
    let name = this.tabs.get(this.active).map(|t| format!("GitVolt — {}", t.name)).unwrap_or_else(|| "GitVolt".into());
    let button = |id: &'static str, name: &'static str| {
        div().id(id).flex().items_center().justify_center().size(px(22.)).rounded_full().bg(theme::control_bg()).hover(|d| d.bg(theme::control_hover_bg())).child(icon(name, 12., theme::text_normal()))
    };
    let _ = window;
    div()
        .id("title-bar")
        .flex()
        .flex_none()
        .h(px(TITLE_H))
        .bg(theme::tab_bar_bg())
        .text_color(theme::text_secondary())
        .on_mouse_down(MouseButton::Left, |e, window, _| {
            if e.click_count == 2 {
                window.zoom_window();
            } else {
                window.start_window_move();
            }
        })
        .on_mouse_down(MouseButton::Right, |e, window, _| window.show_window_menu(e.position))
        .child(tip(
            "Menu",
            crate::ui::anchor(
                "hamburger",
                div()
                    .id("hamburger")
                    .flex()
                    .flex_none()
                    .w(px(34.))
                    .h_full()
                    .items_center()
                    .justify_center()
                    .hover(|d| d.bg(theme::hover_row()))
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        let weak = cx.weak_entity();
                        let rows = crate::commands::hamburger_rows(&weak, this, cx);
                        menu::open_below("hamburger", rows, OpenOptions { label: Some("Menu".into()), ..Default::default() }, window, cx);
                    }))
                    .child(icon("menu", 16., theme::text_secondary())),
            ),
        ))
        .child(div().flex().flex_1().min_w_0().items_center().justify_center().text_size(px(12.)).child(name))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .px(px(6.))
                .child(button("minimize", "minus").on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(|_, window, _| window.minimize_window()))
                .child(button("maximize", "square").on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(|_, window, _| window.zoom_window()))
                .child(button("close", "x").on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(cx.listener(|_, _, window, _| window.remove_window()))),
        )
}

fn tab_bar(this: &GitVolt, cx: &mut Context<GitVolt>) -> impl IntoElement {
    let mut tabs = div().id("tabs").flex().flex_1().min_w_0().overflow_hidden().h_full();
    for (i, tab) in this.tabs.iter().enumerate() {
        let active = i == this.active;
        let outlined = menu::is_target(&format!("tab:{}", tab.id), cx);
        let key: SharedString = format!("tab:{}", tab.id).into();
        tabs = tabs.child(
            div()
                .id(("tab", i))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .min_w(px(90.))
                .max_w(px(220.))
                .pl(px(12.))
                .pr(px(6.))
                .border_r_1()
                .border_color(theme::section_border())
                .when(active, |d| d.bg(theme::action_bar_bg()).text_color(theme::text_selected()))
                .when(!active, |d| d.text_color(theme::text_secondary()).hover(|d| d.bg(theme::hover_row())))
                .when(outlined, |d| d.border_1().border_color(theme::blue()))
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                    this.active = i;
                    cx.notify();
                }))
                .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, _, cx| this.close_tab(i, cx)))
                .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    let weak = cx.weak_entity();
                    let rows = this.tab_rows(i, &weak);
                    menu::open_context_for(key.clone(), rows, e.position, window, cx);
                }))
                .child(div().flex_1().truncate().child(tab.name.clone()))
                .child(tip(
                    format!("Close ({})", crate::commands::display_chord("Mod+W")),
                    div()
                        .id(("tab-close", i))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(18.))
                        .rounded(px(3.))
                        .hover(|d| d.bg(theme::control_hover_bg()))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.close_tab(i, cx);
                        }))
                        .child(icon("x", 12., theme::text_secondary())),
                )),
        );
    }
    let tabs = tabs.on_mouse_down(MouseButton::Right, cx.listener(|this, e: &MouseDownEvent, window, cx| {
        let weak = cx.weak_entity();
        let rows = this.tabbar_rows(&weak);
        menu::open_context(rows, e.position, window, cx);
    }));
    let bar_button = |id: &'static str, name: &'static str| div().id(id).flex().flex_none().w(px(34.)).h_full().items_center().justify_center().hover(|d| d.bg(theme::hover_row())).child(icon(name, 16., theme::text_secondary()));
    div()
        .flex()
        .flex_none()
        .h(px(TAB_H))
        .bg(theme::tab_bar_bg())
        .border_b_1()
        .border_color(theme::section_border())
        .child(tip(format!("Open repository ({})", crate::commands::display_chord("Mod+O")), bar_button("open", "folder-plus").on_click(cx.listener(|this, _, _, cx| this.pick_repository(cx)))))
        .child(tabs)
        .child(tip("Notifications: background errors", bar_button("bell", "bell")))
        .child(tip(format!("Settings ({})", crate::commands::display_chord("Mod+,")), bar_button("settings", "settings")))
        .child(tip("Switch profile", div().flex().flex_none().items_center().gap(px(6.)).px(px(12.)).h_full().child(div().size(px(7.)).rounded_full().bg(theme::blue())).child("Default")))
}

/// A toolbar's button (`.tb-btn`): its icon over its caption. Writes aren't in this build yet: those say so by being disabled.
fn tb_button(id: &'static str, name: &'static str, label: &'static str, enabled: bool) -> Stateful<Div> {
    let color = if enabled { theme::text_normal() } else { theme::text_disabled() };
    div()
        .id(id)
        .flex()
        .flex_col()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(2.))
        .min_w(px(44.))
        .h(px(38.))
        .px(px(6.))
        .rounded(px(4.))
        .text_size(px(11.))
        .text_color(color)
        .when(enabled, |d| d.cursor_pointer().hover(|d| d.bg(theme::hover_row()).text_color(theme::text_selected())))
        .child(icon(name, 16., color))
        .child(label)
}

/// A split button's ▾, its own hover and tooltip; a press opens its menu.
fn tb_caret(id: &'static str, label: &'static str, rows: fn(&GitVolt, &WeakEntity<GitVolt>) -> Vec<MenuRow>, cx: &mut Context<GitVolt>) -> impl IntoElement {
    let open = menu::is_open_on(id, cx);
    tip(
        format!("{label} options"),
        crate::ui::anchor(
            id,
            div()
                .id(id)
                .flex()
                .flex_none()
                .w(px(14.))
                .h(px(38.))
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .cursor_pointer()
                .when(open, |d| d.bg(theme::hover_row()))
                .hover(|d| d.bg(theme::hover_row()))
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    let weak = cx.weak_entity();
                    let list = rows(this, &weak);
                    menu::open_below(id, list, OpenOptions { label: Some(format!("{label} options").into()), ..Default::default() }, window, cx);
                }))
                .child(icon("chevron-down", 11., theme::text_secondary())),
        ),
    )
}

fn undo_menu(_: &GitVolt, _: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
    vec![MenuRow::action("undo.history.none", "Nothing to undo", "undo-2", "Nothing to undo", |_, _| {}).disabled("Nothing to undo")]
}

fn fetch_menu(_: &GitVolt, _: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
    vec![
        MenuRow::header("fetch.title", "Default action"),
        MenuRow::pending("fetch.fetchAll", "Fetch All", "check"),
        MenuRow::pending("fetch.pullFfOrMerge", "Pull (fast-forward if possible)", ""),
        MenuRow::pending("fetch.pullFfOnly", "Pull (fast-forward only)", ""),
        MenuRow::pending("fetch.pullRebase", "Pull (rebase)", ""),
    ]
}

fn push_menu(_: &GitVolt, _: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
    vec![MenuRow::pending("push.otherBranch", "Other branch…", "git-branch"), MenuRow::Separator, MenuRow::pending("push.force", "Force push (with lease)…", "upload")]
}

fn repo_menu(this: &GitVolt, weak: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
    let Some(tab) = this.tabs.get(this.active) else { return Vec::new() };
    let mut rows = Vec::new();
    let worktrees = tab.side.sections.iter().find(|s| s.kind == SectionKind::Worktrees).map_or(&[][..], |s| &s.items[..]);
    for w in worktrees {
        if let crate::sidebar::ItemKind::Worktree { path, branch, is_current, .. } = &w.kind {
            let label = format!("{path} · {}", branch.as_deref().unwrap_or("detached"));
            let row = MenuRow::action(format!("repo.worktree:{path}"), label, if *is_current { "check" } else { "" }, if *is_current { "This tab's worktree" } else { "Switch this tab to this worktree" }, |_, _| {});
            rows.push(if *is_current { row } else { row.disabled(menu::NOT_YET) });
        }
    }
    let _ = weak;
    rows
}

fn branch_menu(this: &GitVolt, weak: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
    let Some(tab) = this.tabs.get(this.active) else { return Vec::new() };
    let locals = tab.side.sections.iter().find(|s| s.kind == SectionKind::Local).map_or(&[][..], |s| &s.items[..]);
    let _ = weak;
    locals
        .iter()
        .map(|b| {
            let is_head = matches!(b.kind, crate::sidebar::ItemKind::Local { is_head: true, .. });
            let row = MenuRow::action(format!("branch:{}", b.name), b.name.clone(), if is_head { "check" } else { "" }, if is_head { format!("{} is checked out", b.name) } else { format!("Check out {}", b.name) }, |_, _| {});
            if is_head { row } else { row.disabled(menu::NOT_YET) }
        })
        .collect()
}

fn toolbar(this: &GitVolt, cx: &mut Context<GitVolt>) -> impl IntoElement {
    let tab = this.tabs.get(this.active);
    let field = |id: &'static str, caption: &'static str, value: SharedString, branch: bool, rows: fn(&GitVolt, &WeakEntity<GitVolt>) -> Vec<MenuRow>, cx: &mut Context<GitVolt>| {
        let label = format!("{}: {value}", if branch { "Branch" } else { "Repository" });
        let open = menu::is_open_on(id, cx);
        crate::ui::anchor(
            id,
            div()
                .id(id)
                .flex()
                .flex_col()
                .justify_center()
                .px(px(10.))
                .min_w_0()
                .rounded(px(4.))
                .my(px(3.))
                .cursor_pointer()
                .when(open, |d| d.bg(theme::hover_row()))
                .hover(|d| d.bg(theme::hover_row()))
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    let weak = cx.weak_entity();
                    let list = rows(this, &weak);
                    menu::open_below(id, list, OpenOptions { label: Some(label.clone().into()), ..Default::default() }, window, cx);
                }))
                .child(div().text_size(px(10.)).text_color(theme::text_secondary()).child(caption))
                .child(div().flex().items_center().gap(px(4.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).when(branch, |d| d.child(icon("git-branch", 13., theme::text_selected()))).child(value).when(branch, |d| d.child(icon("chevron-down", 13., theme::text_selected())))),
        )
    };
    let tip_of = |id: &str| crate::commands::get(id).map_or(SharedString::default(), |c| c.full_tooltip());
    let write = |id: &'static str, name: &'static str, label: &'static str, command: &str| tip(tip_of(command), tb_button(id, name, label, false));
    let branch = tab.and_then(|t| t.graph.as_ref()).map(|g| g.head_branch.clone()).unwrap_or_default();
    let repo_name = tab.map(|t| t.name.clone()).unwrap_or_default();
    div()
        .flex()
        .flex_none()
        .h(px(TOOLBAR_H))
        .items_stretch()
        .px(px(8.))
        .bg(theme::action_bar_bg())
        .border_b_1()
        .border_color(theme::section_border())
        .child(div().flex().flex_1().min_w_0().gap(px(4.)).child(field("toolbar.repo", "REPOSITORY", repo_name, false, repo_menu, cx)).child(field("toolbar.branch", "BRANCH", branch, true, branch_menu, cx)))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(12.))
                .child(div().flex().flex_none().items_center().child(write("tb-undo", "undo-2", "Undo", "edit.undo")).child(tb_caret("tb-undo-caret", "Undo", undo_menu, cx)))
                .child(write("tb-redo", "redo-2", "Redo", "edit.redo"))
                .child(div().flex().flex_none().items_center().child(write("tb-fetch", "download", "Fetch", "repo.fetch")).child(tb_caret("tb-fetch-caret", "Fetch", fetch_menu, cx)))
                .child(div().flex().flex_none().items_center().child(write("tb-push", "upload", "Push", "sync.push")).child(tb_caret("tb-push-caret", "Push", push_menu, cx)))
                .child(write("tb-branch", "git-branch-plus", "Branch", "branch.create"))
                .child(write("tb-stash", "archive", "Stash", "stash.push"))
                .child(write("tb-pop", "archive", "Pop", "stash.pop")),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .justify_end()
                .items_center()
                .gap(px(12.))
                .child(tip(tip_of("edit.palette"), tb_button("tb-actions", "command", "Actions", true).on_click(cx.listener(|this, _, window, cx| {
                    crate::commands::invoke("edit.palette", this, window, cx);
                }))))
                .child(tip(tip_of("edit.find"), tb_button("tb-search", "search", "Search", true).on_click(cx.listener(|this, _, window, cx| {
                    crate::commands::invoke("edit.find", this, window, cx);
                })))),
        )
}

fn chip_view(c: &Chip, lane: usize, more: usize) -> Div {
    let tint = theme::lane_alpha(lane, if c.head { 0.45 } else { 0.25 });
    let text = if c.head { theme::text_selected() } else { theme::text_normal() };
    let source = if c.tag { "tag" } else if c.local { "laptop" } else { "cloud" };
    div()
        .flex()
        .min_w_0()
        .items_center()
        .gap(px(3.))
        .child(
            div()
                .flex()
                .min_w_0()
                .items_center()
                .gap(px(3.))
                .h(px(CHIP_H))
                .px(px(5.))
                .rounded(px(3.))
                .bg(tint)
                .text_color(text)
                .when(c.head, |d| d.child(icon("check", 16., text)))
                .child(div().min_w_0().truncate().child(c.name.clone()))
                .child(icon(source, 13., text)),
        )
        .when(more > 0, |d| d.child(div().flex_none().px(px(4.)).rounded(px(3.)).bg(tint).text_size(px(11.)).text_color(theme::text_normal()).child(format!("+{more}"))))
}

fn graph_table(this: &GitVolt, width: f32, cx: &mut Context<GitVolt>) -> AnyElement {
    let Some(tab) = this.tabs.get(this.active) else {
        return div().flex().flex_1().items_center().justify_center().text_color(theme::text_disabled()).child("Open a repository (Ctrl+O)").into_any_element();
    };
    let Some(graph) = tab.graph.clone() else {
        let text: SharedString = tab.error.clone().unwrap_or_else(|| "Loading…".into());
        return div().flex().flex_1().items_center().justify_center().text_color(theme::text_disabled()).child(text).into_any_element();
    };
    let cols = columns(width - SCROLLBAR_W, graph.max_lanes);
    let head_cell = |w: f32, text: &'static str| div().flex_none().w(px(w)).pl(px(CELL_PAD)).child(text);
    let header = div()
        .flex()
        .flex_none()
        .h(px(HEADER_H))
        .items_center()
        .border_b_1()
        .border_color(theme::section_border())
        .text_size(px(11.))
        .text_color(theme::text_secondary())
        .child(head_cell(LABELS_W, "BRANCH / TAG"))
        .child(head_cell(cols.graph, "GRAPH").flex().items_center().gap(px(10.)).when(!graph.pinned.is_empty(), |d| d.child(div().flex().items_center().gap(px(3.)).child(icon("pin", 11., theme::text_secondary())).child(graph.pinned.clone()))))
        .child(head_cell(cols.message, "COMMIT MESSAGE"))
        .child(head_cell(AUTHOR_W, "AUTHOR"))
        .child(head_cell(DATE_W, "COMMIT DATE / TIME"))
        .child(head_cell(SHA_W, "SHA"));
    let selected = tab.selected;
    let rows = graph.clone();
    let list = uniform_list(
        "graph",
        graph.rows.len(),
        cx.processor(move |_, range: Range<usize>, _, cx| {
            let graph = rows.clone();
            range
                .map(|ix| {
                    let row = &graph.rows[ix];
                    let is_selected = selected == Some(ix);
                    let geometry = RowGeometry { graph_x: LABELS_W, graph_w: cols.graph, clipped: cols.clipped, selected: is_selected, head: graph.head_row == Some(ix) };
                    let painter = graph.clone();
                    let group: SharedString = format!("row-{ix}").into();
                    let cell = |w: Option<f32>, color: Hsla, text: SharedString| {
                        let base = div().h(px(ROW_H)).flex().items_center().px(px(CELL_PAD)).min_w_0().overflow_hidden().text_color(color).when(is_selected, |d| d.bg(theme::selected_row()).text_color(theme::text_selected())).group_hover(group.clone(), |d| d.bg(theme::hover_row()));
                        match w {
                            Some(w) => base.flex_none().w(px(w)),
                            None => base.flex_none().w(px(cols.message)),
                        }
                        .child(div().truncate().child(text))
                    };
                    div()
                        .id(ix)
                        .group(group.clone())
                        .relative()
                        .flex()
                        .h(px(ROW_H))
                        .on_click(cx.listener(move |this, _, _, cx| this.select(ix, cx)))
                        .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            let weak = cx.weak_entity();
                            let rows = this.commit_rows(ix, None, &weak);
                            menu::open_context_for(format!("row:{ix}"), rows, e.position, window, cx);
                        }))
                        .child(canvas(|_, _, _| {}, move |bounds, _, window, cx| paint_row(bounds, &painter.rows[ix], ix, geometry, window, cx)).absolute().top_0().left_0().size_full())
                        .child(div().flex().flex_none().w(px(LABELS_W)).h(px(ROW_H)).items_center().px(px(CELL_PAD)).when_some(row.chips.first(), |d, c| d.child(chip_view(c, row.color as usize, row.chips.len() - 1))))
                        .child(div().flex_none().w(px(cols.graph)))
                        .child(cell(None, theme::text_normal(), row.summary.clone()))
                        .child(cell(Some(AUTHOR_W), theme::text_disabled(), row.author.clone()))
                        .child(cell(Some(DATE_W), theme::text_disabled(), row.date.clone()))
                        .child(cell(Some(SHA_W), theme::text_disabled(), row.sha.clone()).px(px(SHA_PAD)).font_family(MONO_FONT).text_size(px(12.)))
                })
                .collect()
        }),
    )
    .track_scroll(&tab.list)
    .flex_1()
    .min_w_0();
    let thumb = scrollbar(tab, cx);
    div().flex().flex_col().flex_1().min_w_0().min_h_0().bg(theme::app_bg()).child(div().flex().flex_none().pr(px(SCROLLBAR_W)).child(header.flex_1())).child(div().flex().flex_1().min_h_0().child(list).child(thumb)).into_any_element()
}

/// The graph's scrollbar: its thumb follows the list; a press or a drag jumps the list there.
fn scrollbar(tab: &Tab, cx: &mut Context<GitVolt>) -> impl IntoElement {
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

fn status_bar(this: &GitVolt) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .h(px(STATUS_H))
        .items_center()
        .px(px(12.))
        .gap(px(12.))
        .bg(theme::toolbar_bg())
        .border_t_1()
        .border_color(theme::section_border())
        .text_size(px(11.))
        .text_color(theme::text_secondary())
        .child("100%")
        .child(div().flex_1())
        .when(!this.git_version.is_empty(), |d| d.child(format!("git {}", this.git_version)))
        .child(concat!("GitVolt ", env!("CARGO_PKG_VERSION")))
}

fn resize_handles(window: &Window) -> Option<impl IntoElement + use<>> {
    let Decorations::Client { .. } = window.window_decorations() else { return None };
    let edge = |id: &'static str, edge: ResizeEdge, cursor: CursorStyle| div().id(id).absolute().occlude().cursor(cursor).on_mouse_down(MouseButton::Left, move |_, window, _| window.start_window_resize(edge));
    let w = px(RESIZE);
    Some(
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(edge("r-top", ResizeEdge::Top, CursorStyle::ResizeUpDown).top_0().left_0().right_0().h(w))
            .child(edge("r-bottom", ResizeEdge::Bottom, CursorStyle::ResizeUpDown).bottom_0().left_0().right_0().h(w))
            .child(edge("r-left", ResizeEdge::Left, CursorStyle::ResizeLeftRight).top_0().bottom_0().left_0().w(w))
            .child(edge("r-right", ResizeEdge::Right, CursorStyle::ResizeLeftRight).top_0().bottom_0().right_0().w(w))
            .child(edge("r-tl", ResizeEdge::TopLeft, CursorStyle::ResizeUpLeftDownRight).top_0().left_0().size(w))
            .child(edge("r-tr", ResizeEdge::TopRight, CursorStyle::ResizeUpRightDownLeft).top_0().right_0().size(w))
            .child(edge("r-bl", ResizeEdge::BottomLeft, CursorStyle::ResizeUpRightDownLeft).bottom_0().left_0().size(w))
            .child(edge("r-br", ResizeEdge::BottomRight, CursorStyle::ResizeUpLeftDownRight).bottom_0().right_0().size(w)),
    )
}

impl Render for GitVolt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(handle) = self.focus_next_frame.take() {
            window.focus(&handle, cx);
        }
        crate::ui::begin_frame(window, cx);
        if let (Some(hook), Some(tab)) = (self.frame_hook.as_mut(), self.tabs.get_mut(self.active)) {
            hook(tab, window);
        }
        for image in self.retired_images.borrow_mut().drain(..) {
            window.drop_image(image).ok();
        }
        let viewport = window.viewport_size();
        let details = self.tabs.get(self.active).and_then(|t| details_panel(t, window, cx));
        let tab = self.tabs.get(self.active);
        let open_file = tab.is_some_and(|t| t.diff.is_some() || t.history.is_some());
        let folded = open_file && !tab.is_some_and(|t| t.sidebar_open);
        let folded = folded || self.sidebar_narrow;
        let left = if folded { NARROW_W } else { self.sidebar_w };
        let center = viewport.width.as_f32() - left - if details.is_some() { crate::details::DETAILS_W } else { 0. };
        let body_h = viewport.height.as_f32() - TITLE_H - TAB_H - TOOLBAR_H - STATUS_H;
        if let Some(tab) = self.tabs.get_mut(self.active) {
            let (width, height) = match &tab.history {
                Some(h) => {
                    let editor = center - h.list_w;
                    (editor - h.margin(tab.diff.as_ref(), editor), body_h - HISTORY_H)
                }
                None => (center, body_h - crate::diff::view::HEADER_H),
            };
            if let Some(diff) = tab.diff.as_mut() {
                let (w, h) = body_size(diff, width, height);
                diff.ensure_layout(w, h);
            }
        }
        let tab = self.tabs.get(self.active);
        let strip = tab.filter(|_| folded).map(|t| narrow_strip(t, cx));
        let menu = tab.and_then(|t| open_in_menu(t, cx));
        let main = match tab.and_then(|t| if t.history.is_some() { history_panel(t, center, cx) } else { diff_panel(t, center, self.last_opener.as_deref(), cx) }) {
            Some(panel) => panel,
            None => graph_table(self, center, cx),
        };
        let overlays = crate::ui::overlays(window, cx);
        let app = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .key_context("GitVolt")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.arrow(true, cx)))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.arrow(false, cx)))
            .on_action(cx.listener(|this, _: &Copy, _, cx| this.copy_selection(cx)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &PageDown, window, cx| this.page(true, window, cx)))
            .on_action(cx.listener(|this, _: &PageUp, window, cx| this.page(false, window, cx)))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let k = &e.keystroke;
                // A field's characters are its own.
                if crate::ui::input::is_typing(window) {
                    return;
                }
                if (k.key == "enter" && this.activate_open_in(cx)) || this.sidebar_key(k, cx) || this.file_list_key(k, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &ClearSelection, _, cx| {
                if this.close_open_in(cx) || this.exit_compare(cx) {
                } else if this.tabs.get(this.active).is_some_and(|t| t.history.is_some()) {
                    this.close_history(cx);
                } else if this.diff_open() {
                    this.close_file(cx);
                } else if let Some(tab) = this.tabs.get_mut(this.active) {
                    tab.selected = None;
                    tab.commit = None;
                    tab.files = None;
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| this.on_side_drag(e, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.end_side_drag(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| this.end_side_drag(cx)))
            .child(title_bar(self, window, cx))
            .child(tab_bar(self, cx))
            .child(toolbar(self, cx))
            .child(div().flex().flex_1().min_h_0().when_some(strip, |d, s| d.child(s)).when(!folded, |d| d.child(sidebar(self, body_h, window, cx))).child(main).children(details))
            .child(status_bar(self))
            .children(menu);
        // Dialogs and menus float outside the app's key context: no shortcut acts behind them.
        div()
            .relative()
            .size_full()
            .font_family(self.font.clone())
            .text_size(px(13.))
            .line_height(px(18.2))
            .bg(theme::app_bg())
            .text_color(theme::text_normal())
            .child(app)
            .children(overlays)
            .children(resize_handles(window))
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use gitvolt_core::git::GitCli;
    use gitvolt_core::log::CommandLog;
    use gitvolt_graph::NodeKind;
    use std::collections::HashMap;
    // `gpui::*` above brings its own `test`, which `#[gpui::test]`'s `#[test]` would then expand to.
    use std::prelude::v1::test;

    /// The graph's first row's top: title bar, tab bar, toolbar, table header.
    const ROWS_TOP: f32 = TITLE_H + TAB_H + TOOLBAR_H + HEADER_H;

    /// Neither a commit nor WIP: selecting one would read on the runtime, whose wake-up from
    /// another thread GPUI's deterministic test scheduler refuses.
    fn row(i: usize) -> RowView {
        RowView { id: format!("{i:040}").into(), kind: NodeKind::Collapsed, lane: 0, color: 0, segments: Vec::new(), summary: format!("Change {i}").into(), author: "Ada".into(), date: SharedString::default(), sha: format!("{i:06}").into(), initials: "A".into(), avatar: 0, chips: Vec::new(), worktree: None }
    }

    pub fn with_rows(cx: &mut TestAppContext, count: usize) -> (WindowHandle<GitVolt>, VisualTestContext, tokio::runtime::Runtime) {
        cx.update(bind_keys);
        cx.update(crate::ui::init);
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let handle = runtime.handle().clone();
        let api = Arc::new(Api::new(GitCli::new(Arc::new(CommandLog::new(10))), None));
        let window = cx
            .update(|cx| {
                let bounds = Bounds::new(point(px(0.), px(0.)), size(px(1600.), px(900.)));
                cx.open_window(WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() }, |window, cx| {
                    cx.new(|cx| {
                        let mut v = GitVolt::new(api, handle, "Noto Sans".into(), cx);
                        let rows: Vec<RowView> = (0..count).map(row).collect();
                        let by_sha: HashMap<String, usize> = rows.iter().enumerate().map(|(i, r)| (r.id.to_string(), i)).collect();
                        let mut tab = Tab::new(1, "fixture".into(), cx);
                        tab.graph = Some(Rc::new(GraphView { rows, max_lanes: 1, head_row: None, head_branch: "main".into(), pinned: SharedString::default(), by_sha }));
                        v.tabs.push(tab);
                        window.focus(&v.focus, cx);
                        v
                    })
                })
            })
            .unwrap();
        let visual = VisualTestContext::from_window(window.into(), cx);
        (window, visual, runtime)
    }

    fn selected(window: &WindowHandle<GitVolt>, cx: &mut VisualTestContext) -> Option<usize> {
        window.read_with(cx, |v, _| v.tabs[0].selected).unwrap()
    }

    #[gpui::test]
    fn a_click_selects_its_row_and_the_arrows_and_esc_move_it(cx: &mut TestAppContext) {
        let (window, mut cx, _runtime) = with_rows(cx, 100);
        cx.run_until_parked();
        cx.simulate_click(point(px(1100.), px(ROWS_TOP + ROW_H * 6.5)), Modifiers::none());
        assert_eq!(selected(&window, &mut cx), Some(6));
        cx.simulate_keystrokes("down down");
        assert_eq!(selected(&window, &mut cx), Some(8));
        cx.simulate_keystrokes("up");
        assert_eq!(selected(&window, &mut cx), Some(7));
        cx.simulate_keystrokes("escape");
        assert_eq!(selected(&window, &mut cx), None);
    }

    #[gpui::test]
    fn the_wheel_and_the_scrollbar_scroll_the_graph(cx: &mut TestAppContext) {
        let (window, mut cx, _runtime) = with_rows(cx, 1000);
        cx.run_until_parked();
        let offset = |window: &WindowHandle<GitVolt>, cx: &mut VisualTestContext| window.read_with(cx, |v, _| v.tabs[0].list.0.borrow().base_handle.offset().y).unwrap();
        assert_eq!(offset(&window, &mut cx), px(0.));
        cx.simulate_event(ScrollEvent { position: point(px(800.), px(500.)), delta: ScrollDelta::Pixels(point(px(0.), px(-280.))), ..Default::default() });
        cx.run_until_parked();
        assert_eq!(offset(&window, &mut cx), px(-280.));
        // Halfway down the track, about halfway down the list.
        let track = ROWS_TOP + (900. - ROWS_TOP - STATUS_H) / 2.;
        cx.simulate_mouse_down(point(px(1595.), px(track)), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(point(px(1595.), px(track)), MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        let halfway = offset(&window, &mut cx);
        assert!(halfway < px(-10_000.) && halfway > px(-20_000.), "{halfway:?}");
    }

    #[gpui::test]
    fn ctrl_w_closes_the_tab(cx: &mut TestAppContext) {
        let (window, mut cx, _runtime) = with_rows(cx, 1);
        cx.run_until_parked();
        cx.simulate_keystrokes("ctrl-w");
        assert_eq!(window.read_with(&cx, |v, _| v.tabs.len()).unwrap(), 0);
    }
}
