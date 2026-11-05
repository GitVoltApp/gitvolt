//! The window: title bar, tabs, toolbar, sidebar, graph, details, status bar. Laid out with the web
//! UI's measured sizes (`ui/src/{chrome,tabs,toolbar,sidebar,graph,statusbar}/*.css`).

use crate::details::{Commit, Files, details_panel};
use crate::diff::view::{DiffState, body_size, diff_panel};
use crate::history::{FileHistory, HISTORY_H, history_panel};
use crate::open_in::{OpenIn, open_in_menu};
use crate::model::*;
use crate::sidebar::{DEFAULT_SIDEBAR_W, NARROW_W, PushCache, Section, SectionKind, SideDrag, SideState, narrow_strip, sections_of, sidebar};
use crate::table::{ColumnDrag, MessageCache, graph_table};
use crate::theme;
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::menu::{self, MenuRow, OpenOptions};
use crate::ui::tooltip::tip;
use gitvolt_core::api::{Api, Request};
use gitvolt_core::events::AppEvent;
use gpui::prelude::*;
use gpui::*;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

actions!(gitvolt, [SelectNext, SelectPrevious, ExtendNext, ExtendPrevious, ClearSelection, Copy, SelectAll, PageDown, PageUp]);

const TITLE_H: f32 = 30.;
const TAB_H: f32 = 34.;
const TOOLBAR_H: f32 = 44.;
const STATUS_H: f32 = 22.;
const RESIZE: f32 = 6.;
pub const MONO_FONT: &str = "Noto Sans Mono";
pub const GRAPH_LIMIT: u32 = 2000;

/// A tab's id: the core's repository id, with which of that repository's tabs it is above it. The first tab on a
/// repository is plainly its id. What goes to the core goes through `core_repo`; an event from it goes to every tab on it.
const CORE_BITS: u32 = 20;

#[inline]
pub fn core_repo(tab_id: u32) -> u32 {
    tab_id & ((1 << CORE_BITS) - 1)
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, Some("GitVolt")),
        KeyBinding::new("up", SelectPrevious, Some("GitVolt")),
        KeyBinding::new("shift-down", ExtendNext, Some("GitVolt")),
        KeyBinding::new("shift-up", ExtendPrevious, Some("GitVolt")),
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

/// How `open_with` opens a path.
pub struct OpenAs {
    pub path: String,
    /// The Open tab the repository takes the place of.
    pub in_tab: Option<u32>,
    /// A tab of its own, even with one on that worktree already.
    pub new_tab: bool,
    pub alias: Option<String>,
    pub at: Option<usize>,
    pub activate: bool,
}

pub type AfterReload = Box<dyn FnOnce(&mut GitVolt, &mut Context<GitVolt>)>;

pub struct Tab {
    pub id: u32,
    pub name: SharedString,
    /// Rename's display name, kept in the profile; `name` shows it.
    pub alias: Option<String>,
    pub repo_name: SharedString,
    /// The profile's id for this tab.
    pub state_id: String,
    /// The worktree this tab shows, as the core's answer to opening it names it.
    pub worktree: String,
    /// The repository's main path: what the profile's settings for it go by.
    pub repo_path: String,
    /// What's unfolded this session under its preset, until it switches.
    pub expanded: Vec<String>,
    /// The reveal panel open, and the hover that may open one.
    pub reveal: Option<crate::presets::Reveal>,
    pub reveal_hover: u64,
    pub reveal_over: bool,
    /// A hidden commit selected in a reveal panel: the details panel shows it, no row does.
    pub revealed: Option<String>,
    pub graph: Option<Rc<GraphView>>,
    pub side: SideState,
    /// Each MR/PR view's own state, by number: it outlives closing the view.
    pub mr_views: HashMap<u64, crate::forge::mrview::MrUi>,
    /// What waits for the next reload to land.
    /// The Create MR/PR flyout, while it's open.
    pub create: Option<crate::forge::create::CreateUi>,
    pub after_reload: Vec<AfterReload>,
    /// The tab's forge state, in memory.
    pub forge: crate::forge::store::TabForge,
    pub selected: Option<usize>,
    pub commit: Option<Commit>,
    pub files: Option<Files>,
    pub diff: Option<DiffState>,
    /// The merge tool, while `diff` is a conflicted file.
    pub merge: Option<crate::merge::MergeTool>,
    /// The interactive rebase's session: its view stands in the graph's place.
    pub rebase: Option<crate::irebase::RebaseSession>,
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
    /// Create file…'s inline input, on top of the list it was asked from.
    pub create_file: Option<crate::manage::CreateFile>,
    /// A file just made: it opens in File View once the WIP lists hold it.
    pub open_after_wip: Option<String>,
    /// The inline editor open on a row of the graph.
    pub row_editor: Option<crate::inline::RowEditor>,
    /// An Open tab's screen: it has no repository, and `id` isn't one.
    pub open_screen: Option<Box<crate::open::OpenScreen>>,
    pub compare: Option<crate::selection::Compare>,
    /// A compare's or multi-selection's rows; `selected` is then the keyboard's row among them.
    pub picks: crate::selection::Picks,
    pub find: crate::find::FindState,
    /// The WIP panel's own state.
    pub wip: crate::wip::WipUi,
    /// The WIP rows' draft boxes, by worktree.
    pub row_drafts: HashMap<String, Entity<TextInput>>,
    /// The journal's paused operation, as of the last look: what the operation's status names.
    pub paused: Option<gitvolt_core::journal::PausedInfo>,
    /// Each stop selects the WIP row once; a paused entry that ended outside GitVolt settles once.
    pub stops_shown: HashSet<String>,
    pub settled: HashSet<u64>,
    /// `repoInfo`, as the core says it: the remotes' URLs, the worktrees' submodule support.
    pub info: serde_json::Value,
    /// The sidebar's read, as the core says it: what the writes go by.
    pub sidebar: Rc<serde_json::Value>,
    /// Writes queued or running that move HEAD: `(id, branch, commit)`, folded into what's shown.
    pub head_moves: Vec<(u64, Option<Option<String>>, Option<String>)>,
    /// Actions queued or running: the same one on the same target isn't started twice.
    pub in_flight: HashSet<String>,
    /// What a ref is being put through: its chip and row show a spinner meanwhile.
    pub pending: HashMap<String, &'static str>,
    /// The WIP row, once the graph that has it arrives.
    pub reveal_wip: bool,
    pub last_fetch: Option<std::time::Instant>,
    /// "Fetch skipped: authentication required", until a fetch succeeds.
    pub fetch_skipped: Option<String>,
    pub column_prefs: ColumnPrefs,
    pub hidden_columns: HashSet<Col>,
    /// The rows of the branch whose chip the pointer rests on; the others dim.
    pub branch_keep: Option<Rc<HashSet<usize>>>,
    pub branch_hover: u64,
    /// The commit whose dimmed branch chip the pointer's hover shows.
    pub hover_chip: Option<SharedString>,
    /// How deep the graph reads, past `GRAPH_LIMIT` once a jump or Load older asked for more.
    pub limit: u32,
    /// What to select once the graph that's loading holds it.
    pub select_after_load: Option<String>,
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
            alias: None,
            repo_name: SharedString::default(),
            state_id: crate::tabs::new_tab_state_id(),
            worktree: String::new(),
            repo_path: String::new(),
            expanded: Vec::new(),
            reveal: None,
            reveal_hover: 0,
            reveal_over: false,
            revealed: None,
            graph: None,
            side: SideState::new(id, cx),
            mr_views: HashMap::new(),
            create: None,
            after_reload: Vec::new(),
            forge: Default::default(),
            selected: None,
            commit: None,
            files: None,
            diff: None,
            merge: None,
            rebase: None,
            history: None,
            keys: KeyArea::Graph,
            all_files: false,
            sort_by_status: false,
            open_in: OpenIn::default(),
            sidebar_open: false,
            file_filter,
            zoom_input,
            list: UniformListScrollHandle::new(),
            create_file: None,
            open_after_wip: None,
            row_editor: None,
            open_screen: None,
            compare: None,
            picks: Default::default(),
            find: crate::find::FindState::new(id, cx),
            wip: crate::wip::WipUi::new(id, cx),
            row_drafts: HashMap::new(),
            paused: None,
            stops_shown: HashSet::new(),
            settled: HashSet::new(),
            info: serde_json::Value::Null,
            sidebar: Rc::default(),
            head_moves: Vec::new(),
            in_flight: HashSet::new(),
            pending: HashMap::new(),
            reveal_wip: false,
            last_fetch: None,
            fetch_skipped: None,
            column_prefs: ColumnPrefs::default(),
            hidden_columns: HashSet::new(),
            branch_keep: None,
            branch_hover: 0,
            hover_chip: None,
            limit: GRAPH_LIMIT,
            select_after_load: None,
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
    /// What `system-ui` is here: an unset UI font.
    pub base_font: SharedString,
    pub settings_ui: crate::settings::SettingsUi,
    /// What the web UI keeps in localStorage: density, the file list's preferences, the zoom.
    pub prefs: serde_json::Value,
    pub settings_openers: Option<Rc<Vec<crate::open_in::Opener>>>,
    /// `UpdateState`, as the core says it.
    pub update_state: serde_json::Value,
    /// Git can't be run, or is too old: nothing works, and this says why (`GitTooOldScreen`).
    pub git_problem: Option<(bool, SharedString)>,
    /// `ProfileMeta[]`: the switcher's list.
    pub profiles: serde_json::Value,
    /// The profile's tabs are coming up: nothing saves over them meanwhile.
    pub restoring_tabs: bool,
    /// What the start was asked to open, once the profile's read.
    pub start_path: Option<String>,
    pub state_loaded: bool,
    /// A saved group reopening: its tabs join it as they open.
    pub pending_group: Option<(serde_json::Value, usize)>,
    pub fetch_schedule: Option<(u32, Task<()>)>,
    pub relative_clock: Option<Task<()>>,
    pub editor_font_dirty: bool,
    /// A minimized window: the background fetch waits.
    pub window_hidden: bool,
    /// The desktop draws the title bar: the setting, as it was when the window was made.
    pub system_title_bar: bool,
    pub fetch_missed: bool,
    pub window_width: f32,
    pub window_height: f32,
    pub git_version: SharedString,
    pub dragging_scrollbar: bool,
    /// The closed tabs' paths, newest last, for Reopen closed tab.
    pub closed_tabs: Vec<crate::tabs::ClosedTab>,
    pub sidebar_w: f32,
    /// The icon strip, by hand (Ctrl+B); a file on top folds it too, by itself.
    pub sidebar_narrow: bool,
    /// The panels' dragged heights.
    pub side_weights: HashMap<SectionKind, f32>,
    pub side_drag: SideDrag,
    pub last_pushes: PushCache,
    /// The profile and the settings, as the core says them; `Null` until they're read.
    pub lfs: crate::lfs::Lfs,
    /// Each repository's file colours, compiled, until the profile changes.
    pub colours_cache: std::cell::RefCell<HashMap<String, Rc<crate::filecolours::Matcher>>>,
    pub profile: serde_json::Value,
    pub settings: serde_json::Value,
    /// What opened before the profile was read.
    pub pending_recent: Vec<(String, String)>,
    pub open_tabs_made: u32,
    /// A script's answer to the next directory picker.
    pub script_folder: Option<String>,
    pub repo_order: crate::open::RepoOrder,
    pub repo_grouped: bool,
    /// The branch picker's order: the newest tip first, else by name.
    pub branch_order_recent: bool,
    pub details_w: f32,
    /// The commit's share of the details panel, and its divider being dragged.
    pub details_split: f32,
    pub split_drag: Option<crate::details::SplitDrag>,
    pub column_drag: Option<ColumnDrag>,
    pub messages: MessageCache,
    /// A tab being dragged along the strip.
    /// A text selection's drag past its pane's edge: where the pointer is, while the text comes to it.
    pub text_drag_at: Option<Point<Pixels>>,
    pub tab_drag: Option<usize>,
    /// The strip's order when the press went down, for Esc during a drag.
    pub tab_drag_home: Vec<u32>,
    pub tab_press_x: f32,
    pub tab_drag_dx: Option<f32>,
    /// What the layout file says of repositories' columns, until their tabs open.
    pub saved_columns: serde_json::Value,
    /// Where the keyboard goes next frame: a subscription has no window to move it with.
    pub focus_next_frame: Option<FocusHandle>,
    pub activity_shown: Rc<std::cell::RefCell<crate::feedback::ActivityShown>>,
    /// By worktree: the drafts stopped merges set aside.
    pub merge_asides: std::collections::HashMap<String, crate::wip::MergeAside>,
    /// Worktrees whose merge Abort is running: Abort's handler restores their drafts.
    pub merge_aborting: HashSet<String>,
    /// The opener used last, "Open in…"'s default for this session.
    pub last_opener: Option<String>,
    /// Images no longer shown: the frame after they go, the window drops them from its atlas.
    pub retired_images: Rc<std::cell::RefCell<Vec<Arc<RenderImage>>>>,
    /// Measuring (`GITVOLT_BENCH`, bench.rs): called once per frame.
    pub frame_hook: Option<FrameHook>,
    /// The shown tab's forge poller.
    pub forge_poller: Option<crate::forge::poll::Poller>,
    pub after_stack_push: Option<crate::stacks::AfterPush>,
    /// The control Tab is on in a flyout.
    pub flyout_kb: Option<String>,
    /// The MR/PR view's edge being dragged: where it started, the width then, docked.
    pub mr_drag: Option<(f32, f32, bool)>,
    /// A Markdown field on its own, for a script: the forge views will hold theirs.
    pub field_demo: Option<Entity<crate::md::field::MarkdownField>>,
    /// The window, for what an answer from the core has to ask or open.
    pub window: Option<AnyWindowHandle>,
    /// What the core's events say is going on.
    pub ops: crate::ops::Ops,
    /// The WIP draft of each worktree, and how far they're saved.
    pub drafts: HashMap<String, crate::wip::Draft>,
    pub drafts_dirty: u64,
    pub drafts_saved: u64,
    /// The WIP lists' split, and which sections are collapsed.
    pub wip_ratio: f32,
    pub wip_closed: [bool; 3],
    /// The Fetch/Pull button's default.
    pub sync_mode: crate::sync::SyncMode,
    /// The activity log's own state.
    pub activity: crate::feedback::ActivityUi,
    /// The credential prompt on screen.
    pub auth_shown: Option<u64>,
    /// Repos whose last fetch failed: a background failure reaches the bell only when it starts.
    pub failing_fetch: HashSet<u32>,
    pub next_move: u64,
    pub branch_drag: Option<crate::refs::BranchDrag>,
    /// A split button's main part held down: which, and whether its menu opened.
    pub hold: Option<(&'static str, bool)>,
    /// For a test: what was sent, and what's not been answered yet.
    pub sent: Rc<std::cell::RefCell<Vec<serde_json::Value>>>,
    pub unanswered: Vec<(Rc<crate::write::Write>, crate::write::Asked, Vec<gitvolt_core::askpass::SshOffer>)>,
    /// A newer conflict read replaces what an older one was going to show.
    pub merge_generation: u64,
    /// The newest colouring: an older one stops at its next piece.
    pub merge_colour_stop: Arc<std::sync::atomic::AtomicU64>,
    /// What the unsaved question's Save goes on to, once that file's resolved.
    pub merge_after_save: Option<(String, crate::write::After)>,
}

impl GitVolt {
    pub fn new(api: Arc<Api>, rt: tokio::runtime::Handle, font: SharedString, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let weak = cx.weak_entity();
        crate::ui::Ui::get_mut(cx).app_keys = Some(Rc::new(move |k, window, cx| weak.update(cx, |v, cx| v.app_key(k, window, cx)).unwrap_or(false)));
        Self { flyout_kb: None, after_stack_push: None, forge_poller: None, mr_drag: None, base_font: font.clone(), settings_ui: Default::default(), prefs: serde_json::json!({ "density": "standard", "advanceAfterStage": true }), settings_openers: None, update_state: serde_json::Value::Null, git_problem: None, profiles: serde_json::json!([]), restoring_tabs: false, start_path: None, state_loaded: false, pending_group: None, fetch_schedule: None, relative_clock: None, editor_font_dirty: false, window_hidden: false, system_title_bar: false, fetch_missed: false, window_width: 1600., window_height: 900., activity_shown: Default::default(), merge_asides: HashMap::new(), merge_aborting: HashSet::new(), api, rt, tabs: Vec::new(), active: 0, focus, font, git_version: SharedString::default(), dragging_scrollbar: false, closed_tabs: Vec::new(), sidebar_w: DEFAULT_SIDEBAR_W, sidebar_narrow: false, side_weights: HashMap::new(), side_drag: SideDrag::None, last_pushes: Rc::default(), lfs: Default::default(), colours_cache: Default::default(), profile: serde_json::Value::Null, settings: serde_json::Value::Null, pending_recent: Vec::new(), open_tabs_made: 0, script_folder: None, repo_order: Default::default(), repo_grouped: false, branch_order_recent: false, details_w: crate::details::DETAILS_W, details_split: crate::details::SPLIT_DEFAULT, split_drag: None, column_drag: None, messages: Rc::default(), text_drag_at: None, tab_drag: None, tab_drag_home: Vec::new(), tab_press_x: 0., tab_drag_dx: None, saved_columns: serde_json::Value::Null, focus_next_frame: None, last_opener: None, retired_images: Rc::default(), frame_hook: None, field_demo: None, window: None, ops: Default::default(), drafts: HashMap::new(), drafts_dirty: 0, drafts_saved: 0, wip_ratio: crate::wip::SPLIT_DEFAULT, wip_closed: [false; 3], sync_mode: crate::sync::SyncMode::FetchAll, activity: Default::default(), auth_shown: None, failing_fetch: HashSet::new(), next_move: 0, branch_drag: None, hold: None, sent: Rc::default(), unanswered: Vec::new(), merge_generation: 0, merge_colour_stop: Arc::default(), merge_after_save: None }
    }

    /// `gitCheck.ts`'s retry.
    pub fn listen_app_info(&mut self, cx: &mut Context<Self>) {
        let api = self.api.clone();
        let info = self.rt.spawn(async move { api.dispatch(Request::AppInfo).await });
        cx.spawn(async move |this, cx| {
            if let Ok(Err(e)) = info.await
                && crate::feedback::blocks_git(&e)
            {
                let too_old = format!("{:?}", e.kind) == "GitTooOld";
                this.update(cx, |this, cx| {
                    this.git_problem = Some((too_old, e.message.clone().into()));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
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
                if this.update(cx, |this, cx| this.on_event(e, cx)).is_err() {
                    return;
                }
            }
        })
        .detach();
        self.load_state(cx);
        let info = rt.spawn({
            let api = api.clone();
            async move { api.dispatch(Request::AppInfo).await }
        });
        self.read_update_state(cx);
        cx.spawn(async move |this, cx| {
            match info.await {
                Ok(Ok(v)) => {
                    let version = v["gitVersion"].as_str().unwrap_or_default().to_string();
                    this.update(cx, |this, cx| {
                        this.git_version = version.into();
                        this.git_problem = None;
                        cx.notify();
                    })
                    .ok();
                }
                // No git at all, or one too old: `gitCheck.ts`.
                Ok(Err(e)) if crate::feedback::blocks_git(&e) => {
                    let too_old = format!("{:?}", e.kind) == "GitTooOld";
                    this.update(cx, |this, cx| {
                        this.git_problem = Some((too_old, e.message.clone().into()));
                        cx.notify();
                    })
                    .ok();
                }
                _ => {}
            }
        })
        .detach();
    }

    /// The tabs on one of the core's repositories.
    pub fn tabs_on(&self, repo: u32) -> Vec<u32> {
        self.tabs.iter().filter(|t| !t.is_open_tab() && core_repo(t.id) == repo).map(|t| t.id).collect()
    }

    /// A tab id free on `repo`: the first tab is plainly the repository's id.
    fn free_tab_id(&self, repo: u32) -> u32 {
        (0..).map(|n| repo | (n << CORE_BITS)).find(|id| !self.tabs.iter().any(|t| t.id == *id)).unwrap_or(repo)
    }

    /// One event from the core: what's going on, and what to read again.
    pub fn on_event(&mut self, e: AppEvent, cx: &mut Context<Self>) {
        self.ops.apply(&e);
        self.lfs_on_event(&e, cx);
        match e {
            AppEvent::RepoChanged { repo, .. } => {
                for tab in self.tabs_on(repo) {
                    self.reload(tab, cx);
                    self.reread_wip(tab, cx);
                    self.reread_open_file(tab, cx);
                }
            }
            AppEvent::RefsUpdated { repo } => {
                for tab in self.tabs_on(repo) {
                    self.reload(tab, cx);
                    self.read_repo_info(tab, cx);
                }
            }
            AppEvent::UpdateChanged { state } => self.update_state = serde_json::to_value(&state).unwrap_or_default(),
            AppEvent::OpStarted { .. } => self.after_op_started(cx),
            AppEvent::AuthWaiting { .. } | AppEvent::AuthResolved { .. } | AppEvent::OpFinished { .. } => self.sync_auth_modal(cx),
            _ => {}
        }
        cx.notify();
    }

    /// Opens a path; an active Open tab takes it (a fresh start opens the folder in its only tab).
    pub fn open(&mut self, path: String, cx: &mut Context<Self>) {
        let here = self.tabs.get(self.active).filter(|t| t.is_open_tab()).map(|t| t.id);
        self.open_in(path, here, cx);
    }

    /// `in_tab`: the Open tab the repository takes the place of.
    pub fn open_in(&mut self, path: String, in_tab: Option<u32>, cx: &mut Context<Self>) {
        self.open_as(OpenAs { path, in_tab, new_tab: false, alias: None, at: None, activate: true }, cx);
    }

    /// "Open in a new tab": a second tab on the same repository, whichever worktree this path is.
    pub fn open_new_tab(&mut self, path: String, cx: &mut Context<Self>) {
        self.open_as(OpenAs { path, in_tab: None, new_tab: true, alias: None, at: None, activate: true }, cx);
    }

    /// `openWorktreeTab`: the tab already showing it, else one of its own on the same repository.
    pub fn open_worktree_tab(&mut self, path: String, cx: &mut Context<Self>) {
        if let Some(i) = self.tabs.iter().position(|t| !t.is_open_tab() && t.worktree == path) {
            self.active = i;
            self.save_tabs(cx);
            cx.notify();
            return;
        }
        self.open_new_tab(path, cx);
    }

    pub fn open_as(&mut self, how: OpenAs, cx: &mut Context<Self>) {
        let OpenAs { path, in_tab, new_tab, alias, at: place, activate } = how;
        let asked = path.clone();
        let api = self.api.clone();
        let opened = self.rt.spawn(async move {
            let v = api.dispatch(Request::OpenRepo { path }).await?;
            let id = v["id"].as_u64().unwrap_or(0) as u32;
            api.dispatch(Request::WatchRepo { repo: crate::app::core_repo(id) }).await?;
            Ok::<_, gitvolt_core::error::GbError>((id, v["name"].as_str().unwrap_or_default().to_string(), v["worktree"].as_str().unwrap_or_default().to_string(), v["path"].as_str().unwrap_or_default().to_string()))
        });
        cx.spawn(async move |this, cx| {
            let result = opened.await;
            this.update(cx, |this, cx| match result {
                Ok(Ok((id, name, worktree, repo_path))) => {
                    crate::bench::mark("repo_open");
                    this.note_recent(&repo_path, &name);
                    let spot = in_tab.and_then(|t| this.tabs.iter().position(|x| x.id == t));
                    // Already showing that worktree: there it goes, unless a tab of its own was asked for.
                    let already = this.tabs.iter().position(|t| core_repo(t.id) == id && !t.is_open_tab() && t.worktree == worktree).filter(|_| !new_tab);
                    if let Some(i) = already {
                        if activate {
                            this.active = i;
                        }
                        if let Some(s) = spot {
                            let on = this.tabs[i].id;
                            this.tabs.remove(s);
                            this.active = this.tabs.iter().position(|t| t.id == on).unwrap_or(0);
                        }
                    } else {
                        let opened = crate::tabs::Opened { id, name: name.clone(), worktree: worktree.clone(), path: repo_path.clone() };
                        match spot {
                            Some(s) => {
                                let state = this.tabs[s].state_id.clone();
                                this.tabs.remove(s);
                                this.add_opened_tab(opened, alias.clone(), Some(state), s, cx);
                                if activate {
                                    this.active = s;
                                }
                            }
                            None => {
                                let at = place.unwrap_or(if this.tabs.is_empty() { 0 } else { (this.active + 1).min(this.tabs.len()) }).min(this.tabs.len());
                                let state = this.add_opened_tab(opened, alias.clone(), None, at, cx);
                                if activate {
                                    this.active = at;
                                }
                                this.take_into_pending_group(&state, cx);
                            }
                        }
                    }
                    this.save_tabs(cx);
                    cx.notify();
                }
                Ok(Err(e)) => this.open_failed(in_tab, &asked, e.message, cx),
                Err(e) => this.open_failed(in_tab, &asked, e.to_string(), cx),
            })
            .ok();
        })
        .detach();
    }

    /// A tab on what the core opened, at `at`; its state id, given or new.
    pub fn add_opened_tab(&mut self, o: crate::tabs::Opened, alias: Option<String>, state_id: Option<String>, at: usize, cx: &mut Context<Self>) -> String {
        let id = self.free_tab_id(o.id);
        let mut tab = Tab::new(id, alias.clone().unwrap_or_else(|| o.name.clone()).into(), cx);
        tab.alias = alias;
        tab.repo_name = o.name.into();
        tab.worktree = o.worktree.clone();
        tab.repo_path = o.path;
        if let Some(s) = state_id {
            tab.state_id = s;
        }
        let state = tab.state_id.clone();
        self.restore_columns(&mut tab);
        self.tabs.insert(at.min(self.tabs.len()), tab);
        self.load_lfs(id, false, false, cx);
        self.read_repo_info(id, cx);
        self.list_openers(id, cx);
        self.load_journal(id, o.worktree, cx);
        self.reload(id, cx);
        state
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
        let limit = tab.limit;
        let (view, pin) = self.graph_options(repo);
        let read = self.rt.spawn(async move {
            let graph = api.graph(core_repo(repo), Some(limit), pin, None, None, view).await?;
            let sidebar = api.dispatch(Request::Sidebar { repo: crate::app::core_repo(repo) }).await?;
            let view_json = graph.view.as_ref().and_then(|v| serde_json::to_value(v).ok());
            let sections = SendSections(sections_of(&sidebar, &worktree, view_json.as_ref()));
            Ok::<_, gitvolt_core::error::GbError>((SendGraph(graph_view(&graph)), sections, sidebar))
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) else { return };
                tab.loading = false;
                let was_wip = tab.is_wip_selected();
                match result {
                    Ok(Ok((graph, sections, raw))) => {
                        tab.sidebar = Rc::new(raw);
                        // The selection follows its commit across a reload.
                        // What's on screen stays where it was: the selected commit if it shows, else the top row.
                        if let Some(old) = tab.graph.as_ref() {
                            let handle = tab.list.0.borrow().base_handle.clone();
                            let (top, view) = (-handle.offset().y.as_f32(), handle.bounds().size.height.as_f32());
                            let on_screen = tab.selected.filter(|i| (*i as f32 + 1.) * crate::metrics::row_h() > top && (*i as f32) * crate::metrics::row_h() < top + view);
                            let anchor = on_screen.or(Some((top / crate::metrics::row_h()).floor() as usize));
                            if let Some(a) = anchor
                                && let Some(id) = old.rows.get(a).map(|r| r.id.clone())
                                && let Some(now) = graph.0.by_sha.get(id.as_ref())
                                && *now != a
                            {
                                let to = (top + (*now as f32 - a as f32) * crate::metrics::row_h()).max(0.);
                                handle.set_offset(point(handle.offset().x, px(-to)));
                            }
                        }
                        let selected = tab.selected.and_then(|i| tab.graph.as_ref()?.rows.get(i)).map(|r| r.id.to_string());
                        tab.selected = selected.and_then(|id| graph.0.by_sha.get(&id).copied());
                        let before: Vec<SharedString> = if tab.picks.rows.is_empty() { Vec::new() } else { tab.graph.as_ref().map_or(Vec::new(), |g| g.rows.iter().map(|r| r.id.clone()).collect()) };
                        tab.graph = Some(Rc::new(graph.0));
                        // Its rows are laid out on the old graph's lanes.
                        tab.reveal = None;
                        crate::bench::mark("graph_read");
                        tab.side.sections = sections.0;
                        tab.sync_mr_section(cx);
                        tab.side.relist(cx);
                        tab.error = None;
                        this.sync_row_drafts(repo, cx);
                        this.sync_paused(repo);
                        this.check_amend(repo, cx);
                        this.after_stop(repo, cx);
                        this.remap_picks(repo, &before, cx);
                    }
                    Ok(Err(e)) => tab.error = Some(e.message.into()),
                    Err(e) => tab.error = Some(e.to_string().into()),
                }
                let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) else { return };
                // A WIP row that's gone (the last change committed) leaves the panel with it; a commit
                // that's gone (undone) too.
                if (was_wip && !tab.is_wip_selected()) || (tab.selected.is_none() && tab.compare.is_none() && tab.picks.rows.is_empty() && tab.revealed.is_none()) {
                    tab.selected = None;
                    tab.commit = None;
                    tab.files = None;
                }
                let again = std::mem::take(&mut tab.again);
                let pending = tab.select_after_load.take_if(|sha| tab.graph.as_ref().is_some_and(|g| g.by_sha.contains_key(sha.as_str())));
                let wip_row = std::mem::take(&mut tab.reveal_wip).then(|| tab.graph.as_ref().and_then(|g| g.rows.iter().position(|r| r.worktree.as_deref() == Some(tab.worktree.as_str())))).flatten();
                if again {
                    this.reload(repo, cx);
                }
                if let Some(row) = wip_row {
                    this.select(row, cx);
                }
                if let Some(sha) = pending {
                    this.go_to_sha(&sha, cx);
                }
                let waiting = this.tabs.iter_mut().find(|t| t.id == repo).map_or(Vec::new(), |t| std::mem::take(&mut t.after_reload));
                for f in waiting {
                    f(this, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The journal's pause, where the operation's status reads it.
    pub fn sync_paused(&mut self, repo: u32) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == repo) else { return };
        let worktree = tab.wip_worktree().unwrap_or(tab.worktree.as_str()).to_string();
        let paused = self.ops.journal(repo, &worktree).and_then(|j| j.paused.clone());
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) {
            tab.paused = paused;
        }
    }

    /// `ConflictBanner`'s watcher, per reload of the tab's worktree: an operation that ended outside GitVolt with its
    /// journal entry paused settles once; a new stop selects the WIP row, which holds the operation. It waits while
    /// a merge, rebase or pull of the repo runs: those states are transient.
    pub fn after_stop(&mut self, repo: u32, cx: &mut Context<Self>) {
        if self.integrating(repo) {
            return;
        }
        let Some(tab) = self.tabs.iter().find(|t| t.id == repo) else { return };
        let worktree = tab.worktree.clone();
        let Some(g) = tab.graph.clone() else { return };
        let progress = g.in_progress.get(&worktree);
        if progress.is_none() {
            let Some(entry) = self.ops.journal(repo, &worktree).and_then(|j| j.paused.as_ref()).map(|p| p.entry) else { return };
            let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) else { return };
            if tab.settled.insert(entry) {
                let ctx = crate::write::WriteCtx { repo, worktree: worktree.clone() };
                let w = crate::write::Write::new(&ctx, move |_| crate::write::call("settlePaused", serde_json::json!({ "repo": crate::app::core_repo(repo), "worktree": worktree }))).on_end(move |this, ok, _| {
                    if !ok && let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) {
                        tab.settled.remove(&entry);
                    }
                });
                self.write(w, cx);
            }
            return;
        }
        let Some(p) = progress else { return };
        let stop = crate::commit_box::operation_view(p, None, None, &|_| None, &|_| None).stop;
        let is_active = self.tabs.get(self.active).is_some_and(|t| t.id == repo);
        let Some(wip_row) = g.rows.iter().position(|r| r.worktree.as_deref() == Some(worktree.as_str())) else { return };
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) else { return };
        if !is_active || tab.stops_shown.contains(&stop) {
            return;
        }
        tab.stops_shown.insert(stop);
        // Already there with a file open (a click beat it): leave the view alone.
        if tab.selected == Some(wip_row) && tab.diff.is_some() {
            return;
        }
        if tab.selected != Some(wip_row) {
            self.select(wip_row, cx);
        }
    }

    /// Reads a deeper window; `select` once it holds that commit.
    pub fn load_deep(&mut self, repo: u32, limit: u32, select: Option<String>, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) else { return };
        tab.limit = tab.limit.max(limit);
        tab.select_after_load = select;
        self.reload(repo, cx);
    }

    pub fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.tabs.get(self.active).is_some_and(|t| t.selected != Some(ix)) && self.ask_unsaved(move |v, cx| v.select(ix, cx), cx) {
            return;
        }
        self.flush_drafts();
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let Some(row) = tab.graph.as_ref().and_then(|g| g.rows.get(ix)) else { return };
        let (kind, id, worktree) = (row.kind, row.id.to_string(), row.worktree.clone());
        tab.selected = Some(ix);
        tab.revealed = None;
        tab.compare = None;
        tab.picks = Default::default();
        tab.keys = KeyArea::Graph;
        tab.commit = None;
        tab.files = None;
        tab.list.scroll_to_item(ix, ScrollStrategy::Nearest);
        let repo = tab.id;
        cx.notify();
        match (kind, worktree) {
            (gitvolt_graph::NodeKind::Commit | gitvolt_graph::NodeKind::Merge | gitvolt_graph::NodeKind::Stash, _) => self.read_commit(repo, id, ix, cx),
            (gitvolt_graph::NodeKind::Wip, Some(w)) => {
                self.show_draft(repo, cx);
                self.check_amend(repo, cx);
                self.read_wip(repo, w, cx);
            }
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
        // In the graph, a page of rows from the keyboard's row.
        if self.tabs.get(self.active).is_some_and(|t| t.keys == KeyArea::Graph && t.diff.is_none() && t.history.is_none()) {
            let rows = (((height - TITLE_H - TAB_H - TOOLBAR_H - STATUS_H - crate::table::HEADER_H) / crate::metrics::row_h()).floor() - 1.).max(1.);
            return self.move_selection((sign * rows) as i64, cx);
        }
        if self.tabs.get(self.active).is_some_and(|t| t.keys == KeyArea::Files && t.files.is_some()) {
            // What's over the list: the window's bars, the commit, the list's own header.
            self.page_files(down, ((height - 400.) / 26.).max(1.) as usize, cx);
        } else {
            self.scroll_diff(sign * (height / crate::metrics::line_h() - 8.).max(1.), cx);
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

    /// Shift+↑ / ↓: the range from the anchor grows, in the graph. False: they're not the graph's (a file or the rebase
    /// editor is on top, or the keyboard's elsewhere).
    fn extend(&mut self, down: bool, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.tabs.get(self.active) else { return false };
        if tab.keys != KeyArea::Graph || tab.diff.is_some() || tab.history.is_some() || tab.rebase_shown().is_some() {
            return false;
        }
        let Some(count) = tab.graph.as_ref().map(|g| g.rows.len()).filter(|n| *n > 0) else { return false };
        let at = tab.selected.map_or(0, |s| if down { (s + 1).min(count - 1) } else { s.saturating_sub(1) });
        if tab.selected != Some(at) {
            self.select_row(at, crate::selection::SelectMods { ctrl: false, shift: true }, cx);
        }
        true
    }

    pub fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        // A hidden tab's unsaved merge is shown, to be asked about.
        if ix != self.active && self.tabs[ix].merge_dirty() {
            self.active = ix;
            cx.notify();
        }
        if ix == self.active && self.ask_unsaved(move |v, cx| v.close_tab(ix, cx), cx) {
            return;
        }
        let tab = self.tabs.remove(ix);
        if !tab.is_open_tab() {
            self.closed_tabs.push(crate::tabs::ClosedTab { path: tab.worktree.clone(), alias: tab.alias.clone(), index: ix });
            if self.closed_tabs.len() > 20 {
                self.closed_tabs.remove(0);
            }
            // The last tab on it stops watching.
            if !self.tabs.iter().any(|t| core_repo(t.id) == core_repo(tab.id) && !t.is_open_tab()) {
                let api = self.api.clone();
                self.rt.spawn(async move { api.dispatch(Request::UnwatchRepo { repo: crate::app::core_repo(tab.id) }).await });
            }
        }
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
        // With no tab open, the Open Repository screen.
        if self.tabs.is_empty() {
            self.open_blank_tab(cx);
        }
        self.save_tabs(cx);
        cx.notify();
    }

    /// A tab dragged over another's far half takes its place.
    pub fn on_tab_drag(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(from) = self.tab_drag else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            self.tab_drag = None;
            self.tab_drag_dx = None;
            return;
        }
        // It follows the pointer: the rest of the strip moves when it crosses another's far half.
        let dx = e.position.x.as_f32() - self.tab_press_x;
        if dx.abs() > 4. || self.tab_drag_dx.is_some() {
            self.tab_drag_dx = Some(dx);
            cx.notify();
        }
        let ui = crate::ui::Ui::get(cx);
        let x = e.position.x.as_f32();
        let spot = (0..self.tabs.len()).find(|i| ui.anchor(&format!("tabx:{i}")).is_some_and(|b| x >= b.left().as_f32() && x < b.right().as_f32()));
        let Some(to) = spot.filter(|to| *to != from) else { return };
        let Some(b) = ui.anchor(&format!("tabx:{to}")) else { return };
        let middle = b.left().as_f32() + b.size.width.as_f32() / 2.;
        if (to > from && x < middle) || (to < from && x > middle) {
            return;
        }
        let moved = self.tabs.remove(from);
        let width = b.size.width.as_f32();
        self.tabs.insert(to, moved);
        self.active = to;
        self.tab_drag = Some(to);
        // The strip laid out anew: the pointer's offset is from the tab's new slot.
        self.tab_press_x += if to > from { width } else { -width };
        self.tab_drag_dx = Some(e.position.x.as_f32() - self.tab_press_x);
        cx.notify();
    }

    /// Esc during a drag that's begun: it ends, and what it moved goes back; the release does nothing.
    pub fn cancel_drag(&mut self, cx: &mut Context<Self>) -> bool {
        if self.tab_drag.is_some() && self.tab_drag_dx.is_some() {
            self.tab_drag = None;
            self.tab_drag_dx = None;
            let shown = self.tabs.get(self.active).map(|t| t.id);
            let home = std::mem::take(&mut self.tab_drag_home);
            self.tabs.sort_by_key(|t| home.iter().position(|id| *id == t.id));
            self.active = shown.and_then(|id| self.tabs.iter().position(|t| t.id == id)).unwrap_or(self.active);
            cx.notify();
            return true;
        }
        if self.branch_drag.as_ref().is_some_and(|d| d.active) {
            self.branch_drag = None;
            cx.notify();
            return true;
        }
        self.cancel_rebase_drag(cx)
    }

    fn end_tab_drag(&mut self, cx: &mut Context<Self>) {
        if self.tab_drag.take().is_some() && self.tab_drag_dx.take().is_some() {
            self.save_tabs(cx);
            cx.notify();
        }
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
        if let Some(c) = self.closed_tabs.pop() {
            let at = Some(c.index);
            self.open_as(OpenAs { path: c.path, in_tab: None, new_tab: true, alias: c.alias, at, activate: true }, cx);
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
                        let name = answer.value.trim().to_string();
                        tab.alias = (!name.is_empty()).then(|| name.clone());
                        tab.name = if name.is_empty() { tab.repo_name.clone() } else { name.into() };
                        cx.notify();
                    }
                    v.save_tabs(cx);
                })
                .ok();
            },
            window,
            cx,
        );
    }

    fn diff_open(&self) -> bool {
        self.tabs.get(self.active).is_some_and(|t| t.diff.is_some())
    }

    pub fn scroll_to_pointer(&mut self, y: Pixels, top: Pixels, height: Pixels, cx: &mut Context<Self>) {
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

/// ☰, on the title bar or, with the desktop's, in the tab bar.
fn hamburger(cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    tip(
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
    )
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
        .child(hamburger(cx))
        .child(div().flex().flex_1().min_w_0().items_center().justify_center().text_size(px(12.)).child(name))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .px(px(6.))
                .child(crate::ui::anchor("win-minimize", button("minimize", "minus").on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(|_, window, _| window.minimize_window())))
                .child(crate::ui::anchor("win-maximize", button("maximize", "square").on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(|_, window, _| window.zoom_window())))
                .child(crate::ui::anchor("win-close", button("close", "x").on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(cx.listener(|this, _, _, cx| this.close_window(cx))))),
        )
}

fn tab_bar(this: &GitVolt, cx: &mut Context<GitVolt>) -> impl IntoElement {
    let mut tabs = div().id("tabs").flex().flex_1().min_w_0().overflow_hidden().h_full();
    let groups = this.profile["tabGroups"].as_array().cloned().unwrap_or_default();
    for (i, tab) in this.tabs.iter().enumerate() {
        let active = i == this.active;
        let group = groups.iter().find(|g| g["tabs"].as_array().is_some_and(|t| t.iter().any(|x| x == tab.state_id.as_str())));
        if let Some(g) = group
            && g["tabs"][0] == tab.state_id.as_str()
        {
            tabs = tabs.child(this.group_chip(g, cx));
        }
        // In a folded group only the active tab shows.
        if group.is_some_and(|g| g["collapsed"] == true) && !active {
            continue;
        }
        let group_color = group.map(|g| theme::tag_group(g["color"].as_str().unwrap_or("gray")));
        let drag_dx = this.tab_drag_dx.filter(|_| this.tab_drag == Some(i)).unwrap_or(0.);
        let outlined = menu::is_target(&format!("tab:{}", tab.id), cx);
        let key: SharedString = format!("tab:{}", tab.id).into();
        tabs = tabs.child(crate::ui::anchor(
            format!("tabx:{i}"),
            div()
                .id(("tab", i))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .h(px(TAB_H))
                .min_w(px(90.))
                .max_w(px(220.))
                .pl(px(12.))
                .pr(px(6.))
                .border_r_1()
                .border_color(theme::section_border())
                .when(active, |d| d.bg(theme::action_bar_bg()).text_color(theme::text_selected()))
                .when(!active, |d| d.text_color(theme::text_secondary()).hover(|d| d.bg(theme::hover_row())))
                .relative()
                .when(outlined, |d| d.child(crate::ui::outline()))
                .when_some(group_color, |d, c| d.child(div().absolute().left_0().right_0().bottom_0().h(px(2.)).bg(c)))
                .when(drag_dx != 0., |d| d.left(px(drag_dx)).opacity(0.9).bg(theme::action_bar_bg()))
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.active = i;
                    this.tab_drag = Some(i);
                    this.tab_drag_home = this.tabs.iter().map(|t| t.id).collect();
                    this.tab_press_x = e.position.x.as_f32();
                    this.tab_drag_dx = None;
                    this.save_tabs(cx);
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
        ));
    }
    let tabs = tabs.on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| {
        // Empty space only, as a browser's tab strip: a new tab on the Open repository screen.
        if e.click_count == 2 {
            this.open_blank_tab(cx);
        }
    }))
    .on_mouse_down(MouseButton::Right, cx.listener(|this, e: &MouseDownEvent, window, cx| {
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
        .when(this.system_title_bar, |d| d.child(hamburger(cx)))
        .child(tip(format!("Open repository ({})", crate::commands::display_chord("Mod+O")), bar_button("open", "folder-plus").on_click(cx.listener(|this, _, _, cx| {
            this.open_blank_tab(cx);
        }))))
        .child(tabs)
        .child(crate::feedback::bell(this, cx))
        .child(tip(format!("Settings ({})", crate::commands::display_chord("Mod+,")), crate::ui::anchor("tb-settings", bar_button("settings", "settings").on_click(cx.listener(|this, _, window, cx| this.show_settings(None, window, cx))))))
        .child(tip("Switch profile", crate::ui::anchor("profile-switcher", div().id("profile-switcher").flex().flex_none().items_center().gap(px(6.)).px(px(12.)).h_full().cursor_pointer().when(menu::is_open_on("profile-switcher", cx), |d| d.bg(theme::hover_row())).hover(|d| d.bg(theme::hover_row())).on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
            cx.stop_propagation();
            let weak = cx.weak_entity();
            let rows = this.profile_rows(&weak);
            let initial = this.profile["id"].as_str().map(|id| SharedString::from(format!("profile.{id}")));
            menu::open_below("profile-switcher", rows, OpenOptions { label: Some("Switch profile".into()), initial, ..Default::default() }, window, cx);
        })).child(div().size(px(7.)).rounded_full().bg(crate::profiles::profile_color(this))).child(this.profile["name"].as_str().unwrap_or_default().trim().to_string()))))
}

/// How long holding a split button's main part takes to open its menu.
const LONG_PRESS: std::time::Duration = std::time::Duration::from_millis(400);

/// What a toolbar's action button says and does.
struct TbAction {
    id: &'static str,
    glyph: &'static str,
    label: SharedString,
    tooltip: SharedString,
    off: bool,
    busy: bool,
    queued: bool,
    /// Its ▾, when it's a split button: holding the button opens that menu.
    caret: Option<(&'static str, &'static str)>,
    run: fn(&mut GitVolt, &mut Window, &mut Context<GitVolt>),
}

fn tb_action(a: TbAction, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let TbAction { id, glyph, label, tooltip, off, busy, queued, caret, run } = a;
    let color = if off { theme::text_disabled() } else { theme::text_normal() };
    let (icon_el, spin): (Option<Svg>, Option<AnyElement>) = if busy { (None, Some(crate::commit_box::spinner(ElementId::Name(format!("{id}-spin").into()), 16., color).into_any_element())) } else { (Some(icon(glyph, 16., color)), None) };
    let button = div()
        .id(id)
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(2.))
        .min_w(px(44.))
        .h(px(38.))
        .px(px(6.))
        .when(caret.is_some(), |d| d.pr(px(4.)).rounded_l(px(4.)))
        .when(caret.is_none(), |d| d.rounded(px(4.)))
        .text_size(px(11.))
        .text_color(color)
        .when(!off && !busy, |d| d.cursor_pointer().hover(|d| d.bg(theme::hover_row()).text_color(theme::text_selected())))
        .children(icon_el)
        .children(spin)
        .child(label)
        .when(queued, |d| d.child(div().absolute().top(px(4.)).right(px(4.)).size(px(6.)).rounded_full().bg(theme::blue())))
        .when_some(caret, |d, (caret_id, caption)| {
            d.on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                this.hold = Some((id, false));
                let timer = cx.background_executor().timer(LONG_PRESS);
                cx.spawn(async move |this, cx| {
                    timer.await;
                    let Some(this) = this.upgrade() else { return };
                    let window = this.read_with(cx, |v, _| if v.hold == Some((id, false)) { v.window } else { None });
                    let Some(window) = window else { return };
                    window
                        .update(cx, |_, window, cx| {
                            this.update(cx, |v, cx| {
                                v.hold = Some((id, true));
                                open_caret_menu(caret_id, caption, window, cx, v);
                            });
                        })
                        .ok();
                })
                .detach();
            }))
        })
        .on_click(cx.listener(move |this, _, window, cx| {
            let held = this.hold.take().is_some_and(|h| h.1);
            if !held && !off && !busy {
                run(this, window, cx);
            }
        }));
    tip(tooltip, crate::ui::anchor(id, button))
}

/// A toolbar's button (`.tb-btn`): its icon over its caption.
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

/// Opens a split button's menu under its ▾.
fn open_caret_menu(id: &'static str, label: &str, window: &mut Window, cx: &mut Context<GitVolt>, this: &mut GitVolt) {
    let weak = cx.weak_entity();
    let rows = match id {
        "tb-undo-caret" => return this.open_undo_menu(window, cx),
        "tb-sync-caret" => crate::sync::sync_menu_rows(this.sync_mode, &weak),
        _ => crate::sync::push_menu_rows(this, &weak),
    };
    menu::open_below(id, rows, OpenOptions { label: Some(format!("{label} options").into()), ..Default::default() }, window, cx);
}

/// A split button's ▾, its own hover and tooltip; a press opens its menu.
fn tb_caret(id: &'static str, label: &'static str, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
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
                    open_caret_menu(id, label, window, cx, this);
                }))
                .child(icon("chevron-down", 11., theme::text_secondary())),
        ),
    )
}

fn repo_menu(this: &GitVolt, weak: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
    let Some(tab) = this.tabs.get(this.active) else { return Vec::new() };
    let mut rows = Vec::new();
    let worktrees = tab.side.sections.iter().find(|s| s.kind == SectionKind::Worktrees).map_or(&[][..], |s| &s.items[..]);
    for w in worktrees {
        if let crate::sidebar::ItemKind::Worktree { path, branch, is_current, .. } = &w.kind {
            let label = format!("{path} · {}", branch.as_deref().unwrap_or("detached"));
            let (w, p) = (weak.clone(), path.clone());
            rows.push(MenuRow::action(format!("repo.worktree:{path}"), label, if *is_current { "check" } else { "" }, if *is_current { "This tab's worktree" } else { "Switch this tab to this worktree" }, move |_, cx| {
                let p = p.clone();
                w.update(cx, |v, cx| v.switch_worktree(p, cx)).ok();
            }));
        }
    }
    rows
}

/// The toolbar's branch picker: the local branches, by name or by the newest tip.
pub fn branch_items(this: &GitVolt) -> Vec<crate::ui::picker::PickItem> {
    let Some(tab) = this.tabs.get(this.active) else { return Vec::new() };
    let current = tab.head_branch();
    let mut locals = tab.locals();
    let by_name = |a: &str, b: &str| a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b));
    if this.branch_order_recent {
        locals.sort_by(|a, b| b.tip_time.cmp(&a.tip_time).then_with(|| by_name(&a.name, &b.name)));
    } else {
        locals.sort_by(|a, b| by_name(&a.name, &b.name));
    }
    locals
        .into_iter()
        .map(|b| {
            let on = current.as_deref() == Some(b.name.as_str());
            let detail = (b.ahead > 0 || b.behind > 0).then(|| SharedString::from(format!("{}↑ {}↓", b.ahead, b.behind)));
            crate::ui::picker::PickItem { id: b.name.clone().into(), label: b.name.into(), detail, icon: Some(if on { "check" } else { "git-branch" }), current: on, tooltip: None }
        })
        .collect()
}

fn branch_toggle(this: &GitVolt, weak: &WeakEntity<GitVolt>) -> crate::ui::picker::PickToggle {
    let w = weak.clone();
    let recent = this.branch_order_recent;
    crate::ui::picker::PickToggle {
        icon: if recent { "clock" } else { "arrow-down-az" },
        label: if recent { "Most recent first: switch to A–Z".into() } else { "Sorted A–Z: switch to most recent first".into() },
        run: Rc::new(move |_, cx| {
            let w2 = w.clone();
            w.update(cx, |v, cx| {
                v.branch_order_recent = !v.branch_order_recent;
                v.save_layout();
                crate::ui::picker::set_items(branch_items(v), Some(branch_toggle(v, &w2)), cx);
            })
            .ok();
        }),
    }
}

fn open_branch_picker(this: &GitVolt, window: &mut Window, cx: &mut Context<GitVolt>) {
    let weak = cx.weak_entity();
    let items = branch_items(this);
    let toggle = branch_toggle(this, &weak);
    crate::ui::picker::open_below(
        "toolbar.branch",
        items,
        "Check out a local branch",
        Some(toggle),
        move |item, _, cx| {
            let name = item.id.clone();
            if item.current {
                return;
            }
            weak.update(cx, |v, cx| v.checkout_local(&name, cx)).ok();
        },
        window,
        cx,
    );
}

fn toolbar(this: &GitVolt, cx: &mut Context<GitVolt>) -> impl IntoElement {
    let tab = this.tabs.get(this.active);
    let field = |id: &'static str, caption: &'static str, value: SharedString, branch: bool, rows: fn(&GitVolt, &WeakEntity<GitVolt>) -> Vec<MenuRow>, cx: &mut Context<GitVolt>| {
        let label = format!("{}: {value}", if branch { "Branch" } else { "Repository" });
        let open = menu::is_open_on(id, cx) || crate::ui::picker::is_open_on(id, cx);
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
                    if branch {
                        return open_branch_picker(this, window, cx);
                    }
                    let weak = cx.weak_entity();
                    let list = rows(this, &weak);
                    menu::open_below(id, list, OpenOptions { label: Some(label.clone().into()), ..Default::default() }, window, cx);
                }))
                .child(div().text_size(px(10.)).text_color(theme::text_secondary()).child(caption))
                .child(div().flex().items_center().gap(px(4.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).when(branch, |d| d.child(icon("git-branch", 13., theme::text_selected()))).child(value).when(branch, |d| d.child(icon("chevron-down", 13., theme::text_selected())))),
        )
    };
    let tip_of = |id: &str| crate::commands::get(id).map_or(SharedString::default(), |c| c.full_tooltip());
    let head = tab.and_then(Tab::head_branch);
    let branch: SharedString = match (&head, tab.and_then(Tab::head_sha)) {
        (Some(b), _) => b.clone().into(),
        (None, Some(sha)) => format!("HEAD ({})", sha.chars().take(7).collect::<String>()).into(),
        _ => tab.and_then(|t| t.graph.as_ref()).map(|g| g.head_branch.clone()).unwrap_or_default(),
    };
    let repo_name = tab.map(|t| t.name.clone()).unwrap_or_default();
    let changes = tab.and_then(|t| t.graph.as_ref()).and_then(|g| g.rows.first()).and_then(|r| r.wip.as_ref()).map_or(0, WipCounts::files);
    use gitvolt_core::events::OpKind;
    let local = tab.and_then(|t| head.as_ref().and_then(|h| t.local_branch(h)));
    let journal = tab.and_then(|t| this.ops.journal(t.id, &t.worktree));
    let (undo_tip, undo_off) = crate::sync::undo_view(journal, true, &crate::commands::display_chord("Mod+Z"));
    let (redo_tip, redo_off) = crate::sync::undo_view(journal, false, &crate::commands::display_chord("Mod+Shift+Z"));
    let repo = tab.map(|t| t.id).unwrap_or_default();
    let queued = |kind: OpKind| this.ops.queued_kind(repo, kind);
    let sync = crate::sync::sync_view(this.sync_mode, local.as_ref(), head.as_deref());
    let in_flight = |key: String| tab.is_some_and(|t| t.in_flight.contains(&key));
    let sync_running = this.ops.ops.values().any(|o| o.repo == Some(core_repo(repo)) && ((o.kind == OpKind::Fetch && (o.interactive || o.shown)) || o.kind == OpKind::Pull));
    let sync_busy = in_flight("fetch|".into()) || in_flight(format!("pull|{}", head.as_deref().unwrap_or(""))) || sync_running;
    let sync_tip: SharedString = if in_flight("fetch|".into()) && this.sync_mode == crate::sync::SyncMode::FetchAll { "Fetching…".into() } else if sync_busy && this.sync_mode != crate::sync::SyncMode::FetchAll && head.is_some() { format!("Pulling {}…", head.as_deref().unwrap_or_default()).into() } else { sync.tooltip.clone().into() };
    let (push_tip, push_off) = crate::sync::push_view(local.as_ref(), head.as_deref(), &tab.map_or("origin".to_string(), Tab::default_remote));
    let push_busy = local.as_ref().is_some_and(|b| in_flight(format!("push|{}", b.full_name)));
    let push_tip: SharedString = if push_busy { format!("Pushing {}…", local.as_ref().map_or("", |b| b.name.as_str())).into() } else { push_tip.into() };
    let dirty = tab.is_some_and(|t| t.graph.as_ref().is_some_and(|g| g.rows.iter().any(|r| r.worktree.as_deref() == Some(t.worktree.as_str()))));
    let newest = tab.and_then(|t| t.sidebar["stashes"].as_array().and_then(|s| s.first().cloned()));
    let pop_tip: SharedString = newest.as_ref().map_or("No stashes".to_string(), |s| format!("Pop \"{}\"", s["message"].as_str().unwrap_or_default())).into();
    let has_head = tab.and_then(Tab::head_sha).is_some();
    let undo = tb_action(TbAction { id: "tb-undo", glyph: "undo-2", label: "Undo".into(), tooltip: undo_tip.into(), off: undo_off, busy: false, queued: queued(OpKind::Undo), caret: Some(("tb-undo-caret", "Undo")), run: |v, _, cx| {
        if let Some(ctx) = v.tabs.get(v.active).map(Tab::write_target) {
            v.undo(ctx, crate::ui::arm::Origin::control("tb-undo"), cx);
        }
    } }, cx);
    let redo = tb_action(TbAction { id: "tb-redo", glyph: "redo-2", label: "Redo".into(), tooltip: redo_tip.into(), off: redo_off, busy: false, queued: queued(OpKind::Redo), caret: None, run: |v, _, cx| {
        if let Some(ctx) = v.tabs.get(v.active).map(Tab::write_target) {
            v.redo(ctx, crate::ui::arm::Origin::control("tb-redo"), cx);
        }
    } }, cx);
    let sync_button = tb_action(TbAction { id: "tb-sync", glyph: if this.sync_mode == crate::sync::SyncMode::FetchAll { "download" } else { "arrow-down-to-line" }, label: sync.label.clone().into(), tooltip: sync_tip, off: sync.disabled, busy: sync_busy, queued: queued(OpKind::Fetch) || queued(OpKind::Pull), caret: Some(("tb-sync-caret", "Fetch")), run: |v, _, cx| v.sync_default(cx) }, cx);
    let push = tb_action(TbAction { id: "tb-push", glyph: "upload", label: "Push".into(), tooltip: push_tip, off: push_off, busy: push_busy, queued: queued(OpKind::Push), caret: Some(("tb-push-caret", "Push")), run: |v, _, cx| v.push_head(crate::ui::arm::Origin::control("tb-push"), cx) }, cx);
    let branch_button = tb_action(TbAction { id: "tb-branch", glyph: "git-branch-plus", label: "Branch".into(), tooltip: tip_of("branch.create"), off: !has_head, busy: false, queued: false, caret: None, run: |v, window, cx| {
        crate::commands::invoke("branch.create", v, window, cx);
    } }, cx);
    let stash = tb_action(TbAction { id: "tb-stash", glyph: "archive", label: "Stash".into(), tooltip: if dirty { "Stash every change, named from the WIP message".into() } else { "No changes to stash".into() }, off: !dirty, busy: false, queued: queued(OpKind::Stash), caret: None, run: |v, window, cx| {
        crate::commands::invoke("stash.push", v, window, cx);
    } }, cx);
    let pop = tb_action(TbAction { id: "tb-pop", glyph: "archive-restore", label: "Pop".into(), tooltip: pop_tip, off: newest.is_none(), busy: false, queued: false, caret: None, run: |v, window, cx| {
        crate::commands::invoke("stash.pop", v, window, cx);
    } }, cx);
    div()
        .flex()
        .flex_none()
        .h(px(TOOLBAR_H))
        .items_stretch()
        .px(px(8.))
        .bg(theme::action_bar_bg())
        .border_b_1()
        .border_color(theme::section_border())
        .child(div().flex().flex_1().min_w_0().gap(px(4.)).child(field("toolbar.repo", "REPOSITORY", repo_name, false, repo_menu, cx)).child(tip("Check out a local branch", field("toolbar.branch", "BRANCH", branch, true, repo_menu, cx))).children(crate::forge::review::zones::review_chip(this, cx)))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(12.))
                .child(div().flex().flex_none().items_center().child(undo).child(tb_caret("tb-undo-caret", "Undo", cx)))
                .child(redo)
                .child(div().flex().flex_none().items_center().child(sync_button).child(tb_caret("tb-sync-caret", "Fetch", cx)))
                .child(div().flex().flex_none().items_center().child(push).child(tb_caret("tb-push-caret", "Push", cx)))
                .child(branch_button)
                .child(stash)
                .child(pop),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .justify_end()
                .items_center()
                .gap(px(12.))
                .when(changes > 0, |d| {
                    let pressed = tab.is_some_and(Tab::is_wip_selected);
                    let color = if pressed { theme::text_selected() } else { theme::orange() };
                    d.child(tip(
                        tip_of("view.goToWip"),
                        div()
                            .id("tb-wip")
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
                            .bg(theme::mix(theme::orange(), if pressed { 0.3 } else { 0.14 }, transparent_black()))
                            .border_1()
                            .border_color(theme::mix(theme::orange(), if pressed { 0.6 } else { 0.4 }, transparent_black()))
                            .cursor_pointer()
                            .when(!pressed, |d| d.hover(|d| d.bg(theme::mix(theme::orange(), 0.24, transparent_black()))))
                            .on_click(cx.listener(|this, _, window, cx| {
                                crate::commands::invoke("view.goToWip", this, window, cx);
                            }))
                            .child(icon("file-pen", 16., color))
                            .child(format!("{changes} change{}", if changes == 1 { "" } else { "s" })),
                    ))
                })
                .child(tip(tip_of("edit.palette"), tb_button("tb-actions", "command", "Actions", true).on_click(cx.listener(|this, _, window, cx| {
                    crate::commands::invoke("edit.palette", this, window, cx);
                }))))
                .child(tip(tip_of("edit.find"), tb_button("tb-search", "search", "Search", true).on_click(cx.listener(|this, _, window, cx| {
                    crate::commands::invoke("edit.find", this, window, cx);
                })))),
        )
}

fn status_bar(this: &GitVolt, cx: &mut Context<GitVolt>) -> impl IntoElement + use<> {
    let items = crate::feedback::status_items(this, cx);
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
        .child(tip(format!("Zoom ({} / {})", crate::commands::display_chord("Mod+="), crate::commands::display_chord("Mod+-")), crate::ui::anchor("status-zoom", div().id("status-zoom").cursor_pointer().on_mouse_down(MouseButton::Left, cx.listener(|_, _, window, cx| {
            cx.stop_propagation();
            let weak = cx.weak_entity();
            let now = crate::zoom::percent();
            let rows: Vec<MenuRow> = crate::zoom::ZOOM_STEPS.iter().map(|p| {
                let (w, p) = (weak.clone(), *p);
                MenuRow::action(format!("zoom.{p}"), format!("{p}%"), if p == now { "check" } else { "" }, format!("Zoom to {p}%"), move |_, cx| {
                    w.update(cx, |v, cx| v.set_zoom(p, cx)).ok();
                })
            }).collect();
            menu::open_below("status-zoom", rows, OpenOptions { label: Some("Zoom".into()), initial: Some(format!("zoom.{now}").into()), ..Default::default() }, window, cx);
        })).child(format!("{}%", crate::zoom::percent())))))
        .children(items)
        .child(div().flex_1())
        .children(crate::updates::update_pill(this, cx))
        .child(tip(format!("Keyboard shortcuts ({})", crate::commands::display_chord("Mod+/")), crate::ui::anchor("status-shortcuts", div().id("status-shortcuts").flex().cursor_pointer().on_click(cx.listener(|this, _, window, cx| this.open_shortcuts(window, cx))).child(icon("keyboard", 13., theme::text_secondary())))))
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
        self.forge_sync_poller(cx);
        self.measure_editor_font(window);
        self.window = Some(window.window_handle());
        if let (Some(hook), Some(tab)) = (self.frame_hook.as_mut(), self.tabs.get_mut(self.active)) {
            hook(tab, window);
        }
        for image in self.retired_images.borrow_mut().drain(..) {
            window.drop_image(image).ok();
        }
        let viewport = window.viewport_size();
        self.fetch_avatars(cx);
        if let Some(repo) = self.tabs.get(self.active).map(|t| t.id) {
            self.sync_paused(repo);
        }
        let details_w = self.details_w;
        let panel_h = viewport.height.as_f32() - TITLE_H - TAB_H - TOOLBAR_H - STATUS_H;
        let details = self.tabs.get(self.active).and_then(|t| details_panel(self, t, details_w, panel_h, window, cx));
        let tab = self.tabs.get(self.active);
        let open_file = tab.is_some_and(|t| t.diff.is_some() || t.history.is_some());
        let folded = open_file && !tab.is_some_and(|t| t.sidebar_open);
        let folded = folded || self.sidebar_narrow;
        // The rebase editor takes the sidebar's place too.
        let open_id = tab.filter(|t| t.is_open_tab()).map(|t| t.id);
        let no_sidebar = tab.is_some_and(|t| t.rebase_shown().is_some()) || open_id.is_some();
        let left = if no_sidebar { 0. } else if folded { NARROW_W } else { self.sidebar_w };
        let center = viewport.width.as_f32() - left - if details.is_some() { self.details_w + 1. } else { 0. };
        // A docked MR/PR view is a pane beside the graph, which shrinks to fit.
        let center_room = center;
        let center = center - self.mr_dock_width(center_room).unwrap_or(0.);
        let body_h = viewport.height.as_f32() - TITLE_H - TAB_H - TOOLBAR_H - STATUS_H;
        if let Some(m) = self.tabs.get_mut(self.active).and_then(Tab::merge_shown_mut) {
            m.set_size(center, body_h - crate::metrics::panel_bar_h());
        }
        self.review_sync(window, cx);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            let (width, height) = match &tab.history {
                Some(h) => {
                    let editor = center - h.list_w;
                    (editor - h.margin(tab.diff.as_ref(), editor), body_h - HISTORY_H)
                }
                None => (center, body_h - crate::metrics::panel_bar_h()),
            };
            let committing = tab.wip.committing;
            if let Some(diff) = tab.diff.as_mut() {
                diff.commit_held = committing;
                let (w, h) = body_size(diff, width, height);
                diff.ensure_layout(w, h);
                let file = diff.view == crate::diff::view::ViewKind::File;
                let pdf_resized = diff.pdf.as_mut().is_some_and(|p| {
                    let was_file = p.file;
                    p.set_file(file);
                    if p.source_on {
                        return was_file != file;
                    }
                    let pane = if p.both() { (w - 2.) / 2. } else { w };
                    let changed = was_file != file || p.pane_size() != (pane, h);
                    p.resize(pane, h);
                    changed
                });
                let font_size = diff.media.as_mut().filter(|m| !m.source_on).map(|m| {
                    m.set_file(file);
                    (w, h)
                });
                // The rendered diff's Split gives way to one column when the pane's narrow.
                diff.md_width = Some(width);
                let split_changed = diff.md.as_ref().is_some_and(|m| m.split != crate::md::files::wants_split(diff));
                if split_changed {
                    self.sync_markdown(cx);
                }
                if pdf_resized {
                    self.draw_pdf_pages(cx);
                }
                if let Some((w, h)) = font_size {
                    self.draw_font_samples(w, h, cx);
                }
            }
        }
        let submit_popover = crate::forge::review::zones::submit_popover(self, window, cx);
        let tab = self.tabs.get(self.active);
        let strip = tab.filter(|_| folded && !no_sidebar).map(|t| narrow_strip(t, cx));
        let menu = tab.and_then(|t| open_in_menu(t, cx));
        let rebase_ghost = tab.and_then(crate::irebase::view::chip_ghost);
        let rebase = tab.and_then(|t| crate::irebase::view::rebase_panel(t, center, window, cx));
        let opener_id = self.default_opener_id();
        let open_page = open_id.map(|id| crate::open::open_screen(self, id, window, cx));
        let tab = self.tabs.get(self.active);
        let main = if let Some(page) = open_page { page } else if let Some(panel) = rebase { panel } else { match tab.and_then(|t| if t.history.is_some() { history_panel(t, center, cx) } else { diff_panel(t, center, opener_id.as_deref(), t.wip_ctx().and_then(|c| self.ops.staging(c.repo, &c.worktree)), window, cx) }) {
            Some(panel) => panel,
            None => graph_table(self, center, window, cx),
        } };
        let main = match crate::forge::mrview::mr_flyout(self, center_room, window, cx) {
            Some((flyout, true)) => div().flex().flex_1().min_w_0().min_h_0().child(flyout).child(main).into_any_element(),
            Some((flyout, false)) => div().relative().flex().flex_1().min_w_0().min_h_0().child(main).child(div().absolute().left_0().top_0().bottom_0().flex().child(flyout)).into_any_element(),
            None => main,
        };
        let banner_list = self.tabs.get(self.active).map_or(Vec::new(), |t| self.banner_list(t, cx));
        let banners = crate::ui::banner::banners(&banner_list);
        let drag_ghost = self.branch_drag.as_ref().filter(|d| d.active).map(|d| {
            div().absolute().left(d.at.x + px(8.)).top(d.at.y + px(8.)).flex().items_center().gap(px(4.)).h(px(20.)).px(px(5.)).rounded(px(3.)).bg(theme::panel_bg1()).border_1().border_color(theme::blue()).text_color(theme::text_selected()).child(icon("git-branch", 13., theme::text_selected())).child(d.chip.name.clone())
        });
        if self.activity_open(cx) {
            self.sync_activity_shown();
        }
        self.window_width = viewport.width.as_f32();
        self.window_height = viewport.height.as_f32();
        let settings = crate::settings::frame::settings_overlay(self, window, cx);
        let font_picker = crate::settings::fonts::font_picker_overlay(self, window, cx);
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
            .on_action(cx.listener(|this, _: &ExtendNext, window, cx| {
                if crate::ui::input::is_typing(window) || !this.extend(true, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &ExtendPrevious, window, cx| {
                if crate::ui::input::is_typing(window) || !this.extend(false, cx) {
                    cx.propagate();
                }
            }))
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
                if this.flyout_key(k, window, cx) {
                    cx.stop_propagation();
                    return;
                }
                if (k.key == "enter" && this.activate_open_in(cx)) || this.sidebar_key(k, cx) || this.file_list_key(k, cx) || this.graph_key(k, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &ClearSelection, window, cx| {
                if this.flyout_escape(window, cx) || this.close_open_in(cx) || this.close_reveal(cx) {
                } else if this.tabs.get(this.active).is_some_and(|t| t.history.is_some()) {
                    this.close_history(cx);
                } else if this.diff_open() {
                    this.close_file(cx);
                } else if this.exit_compare(cx) {
                } else if let Some(tab) = this.tabs.get_mut(this.active) {
                    tab.selected = None;
                    tab.commit = None;
                    tab.files = None;
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
                this.on_merge_drag(e, window, cx);
                this.on_split_drag(e, cx);
                this.on_rebase_drag(e, cx);
                this.on_side_drag(e, cx);
                this.on_column_drag(e, cx);
                this.on_tab_drag(e, cx);
                this.on_branch_drag(e, cx);
            }))
.on_mouse_up(MouseButton::Left, cx.listener(|this, _, window, cx| {
                this.end_merge_drag();
                this.end_split_drag();
                this.end_rebase_drag(cx);
                this.end_side_drag(cx);
                this.end_column_drag(cx);
                this.end_tab_drag(cx);
                this.end_branch_drag(window, cx);
                if this.hold.is_some_and(|h| !h.1) {
                    // A click follows and takes it.
                    let hold = this.hold;
                    cx.defer_in(window, move |this, _, _| {
                        if this.hold == hold {
                            this.hold = None;
                        }
                    });
                }
            }))
.on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, window, cx| {
                this.end_merge_drag();
                this.end_split_drag();
                this.end_rebase_drag(cx);
                this.end_side_drag(cx);
                this.end_column_drag(cx);
                this.end_tab_drag(cx);
                this.end_branch_drag(window, cx);
                this.hold = None;
            }))
            .when(!self.system_title_bar, |d| d.child(title_bar(self, window, cx)))
            .child(tab_bar(self, cx))
            .when(open_id.is_none(), |d| d.child(toolbar(self, cx)))
            .children(banners)
            .child(div().flex().flex_1().min_h_0().when_some(strip, |d, s| d.child(s)).when(!folded && !no_sidebar, |d| d.child(sidebar(self, body_h, window, cx))).child(main).children(details))
            .child(status_bar(self, cx))
            .children(menu)
            .children(drag_ghost)
            .children(rebase_ghost);
        if let Some((too_old, message)) = self.git_problem.clone() {
            return crate::feedback::git_too_old_screen(too_old, message, cx).into_any_element();
        }
        // Dialogs and menus float outside the app's key context: no shortcut acts behind them.
        div()
            .relative()
            .size_full()
            .font_family(self.font.clone())
            .text_size(px(13.))
            .line_height(relative(1.4))
            .bg(theme::app_bg())
            .text_color(theme::text_normal())
            .child(app)
            .children(settings)
            .children(font_picker)
            .children(submit_popover)
            .children(self.flyout_ring(cx))
            .children(overlays)
            .children(resize_handles(window))
            .into_any_element()
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
    const ROWS_TOP: f32 = TITLE_H + TAB_H + TOOLBAR_H + crate::table::HEADER_H;

    /// Neither a commit nor WIP: selecting one would read on the runtime, whose wake-up from
    /// another thread GPUI's deterministic test scheduler refuses.
    fn row(i: usize) -> RowView {
        RowView { id: format!("{i:040}").into(), kind: NodeKind::Collapsed, lane: 0, color: 0, segments: Vec::new(), summary: format!("Change {i}").into(), author: "Ada".into(), date: SharedString::default(), time: 0, sha: format!("{i:06}").into(), initials: "A".into(), avatar: 0, chips: Vec::new(), worktree: None, email: String::new(), parents: Vec::new(), stash_branch: None, mr_refs: Vec::new(), body: SharedString::default(), wip: None, collapsed: None, fold: None, ghosts: Vec::new() }
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
                        tab.graph = Some(Rc::new(GraphView { rows, max_lanes: 1, head_row: None, head_branch: "main".into(), truncated: false, by_sha, head_sha: None, worktrees: Vec::new(), in_progress: Default::default(), membership: Vec::new(), pinned_refs: Vec::new(), view: None, fork: None }));
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
        cx.simulate_click(point(px(1100.), px(ROWS_TOP + crate::metrics::row_h() * 6.5)), Modifiers::none());
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
        let shown = window.read_with(&cx, |v, _| v.tabs.iter().map(Tab::is_open_tab).collect::<Vec<_>>()).unwrap();
        assert_eq!(shown, [true], "with no tab open, the Open Repository screen");
    }
}
