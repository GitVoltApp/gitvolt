//! The Open Repository screen (`ui/src/open/`), the whole page of an Open tab: recent repositories, opening a directory,
//! cloning and "Your repos". What it keeps lasts in the profile, as the web UI's does: `recent`, `reposFolder`, `reposFolders`.

use crate::app::{GitVolt, Tab, icon};
use crate::theme;
use crate::ui::controls::clearable;
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::tooltip::tip;
use gitvolt_core::api::Request;
use gitvolt_core::events::OpKind;
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::collections::HashMap;

/// Pinned entries are all kept; the rest fill what's left of this many.
pub const MAX_RECENT: usize = 50;
/// An Open tab has no repository: its id comes from a range none has.
const OPEN_TAB_IDS: u32 = 0x7000_0000;

#[derive(Debug, Clone, PartialEq)]
pub struct ScannedRepo {
    pub path: String,
    pub name: String,
    pub branch: Option<String>,
    pub modified: i64,
    pub worktree_of: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepoOrder {
    #[default]
    Recent,
    Alpha,
}

/// One Open tab's own state.
pub struct OpenScreen {
    pub filter: Entity<TextInput>,
    pub repos_filter: Entity<TextInput>,
    pub url: Entity<TextInput>,
    pub dest: Entity<TextInput>,
    pub dest_edited: bool,
    pub busy: bool,
    /// The clone's op is labelled with its destination.
    pub cloning_to: Option<String>,
    pub error: Option<String>,
    pub clone_error: Option<String>,
    /// The recent entry whose open just failed: the error offers to drop it.
    pub failed_recent: Option<String>,
    pub suggestion: Option<String>,
    pub banner_error: Option<String>,
    pub repos: Option<Vec<ScannedRepo>>,
    pub scan_error: Option<String>,
    /// The folders `repos` was scanned for.
    pub scanned: Vec<String>,
    pub focus: HashMap<String, FocusHandle>,
}

impl OpenScreen {
    fn new(id: u32, cx: &mut Context<GitVolt>) -> Self {
        let filter = cx.new(|cx| TextInput::new(cx).placeholder("Filter recent repositories").claim_escape_with_text());
        let repos_filter = cx.new(|cx| TextInput::new(cx).placeholder("Filter your repositories").claim_escape_with_text());
        let url = cx.new(|cx| TextInput::new(cx).placeholder("git@gitlab.example.com:group/project.git"));
        let dest = cx.new(|cx| TextInput::new(cx).placeholder("/home/you/repos/project"));
        let clear_on_escape = |this: &mut GitVolt, input: Entity<TextInput>, e: &InputEvent, cx: &mut Context<GitVolt>| {
            if matches!(e, InputEvent::Escape) {
                input.update(cx, |i, cx| i.set_text("", cx));
            }
            if matches!(e, InputEvent::Changed | InputEvent::Escape) {
                this.notify_open(cx);
            }
        };
        cx.subscribe(&filter, clear_on_escape).detach();
        cx.subscribe(&repos_filter, clear_on_escape).detach();
        cx.subscribe(&url, move |this, _, e: &InputEvent, cx| match e {
            InputEvent::Changed => this.clone_url_changed(id, cx),
            InputEvent::Submit { .. } => this.start_clone(id, cx),
            _ => {}
        })
        .detach();
        cx.subscribe(&dest, move |this, _, e: &InputEvent, cx| match e {
            InputEvent::Changed => this.clone_dest_changed(id, cx),
            InputEvent::Submit { .. } => this.start_clone(id, cx),
            _ => {}
        })
        .detach();
        Self { filter, repos_filter, url, dest, dest_edited: false, busy: false, cloning_to: None, error: None, clone_error: None, failed_recent: None, suggestion: None, banner_error: None, repos: None, scan_error: None, scanned: Vec::new(), focus: HashMap::new() }
    }
}

/// `repoNameFromUrl`: the destination's folder name for a clone URL.
pub fn repo_name_from_url(url: &str) -> String {
    let s = url.trim().trim_end_matches(['/', '\\']);
    let s = if s.len() >= 4 && s[s.len() - 4..].eq_ignore_ascii_case(".git") { &s[..s.len() - 4] } else { s };
    let last = s.rsplit(['/', ':', '\\']).next().unwrap_or("");
    last.chars().filter(|c| c.is_alphanumeric() || *c == '_' || *c == '.' || *c == '-').collect()
}

fn scheme_url(u: &str) -> bool {
    let Some((scheme, rest)) = u.split_once("://") else { return false };
    matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https" | "ssh" | "git" | "file") && !rest.is_empty() && !rest.chars().any(char::is_whitespace)
}

fn scp_style(u: &str) -> bool {
    let Some((who, rest)) = u.split_once('@') else { return false };
    let word = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-');
    let Some((host, path)) = rest.split_once(':') else { return false };
    word(who) && word(host) && !path.is_empty() && !path.chars().any(char::is_whitespace)
}

/// `cloneUrlProblem`: why `url` can't be cloned: http(s), ssh, git and file URLs, scp-style `user@host:path` and absolute
/// paths, each with a name to give the folder.
pub fn clone_url_problem(url: &str) -> Option<&'static str> {
    let u = url.trim();
    if u.is_empty() {
        return None;
    }
    let ok = scheme_url(u) || scp_style(u) || u.starts_with('/');
    (!ok || repo_name_from_url(u).is_empty()).then_some("Not a repository URL: use https://, ssh://, git@host:path, file:// or an absolute path")
}

/// `cloneDestProblem`: it must be an absolute path.
pub fn clone_dest_problem(dest: &str) -> Option<&'static str> {
    let d = dest.trim();
    (!d.is_empty() && !d.starts_with('/')).then_some("The destination must be an absolute path")
}

fn segments(p: &str) -> Vec<&str> {
    p.split(['/', '\\']).filter(|s| !s.is_empty()).collect()
}

/// `folderLabels`: short names for the scanned directories, each one's shortest trailing path no other shares.
pub fn folder_labels(folders: &[String]) -> HashMap<String, String> {
    let segs: Vec<Vec<&str>> = folders.iter().map(|f| segments(f)).collect();
    let mut out = HashMap::new();
    for (i, f) in folders.iter().enumerate() {
        let suffix = |s: &[&str], k: usize| s[s.len().saturating_sub(k)..].join("/");
        let mut n = 1;
        while n < segs[i].len() && segs.iter().enumerate().any(|(j, s)| j != i && suffix(s, n) == suffix(&segs[i], n)) {
            n += 1;
        }
        let label = suffix(&segs[i], n);
        out.insert(f.clone(), if label.is_empty() { f.clone() } else { label });
    }
    out
}

/// `folderOf`: the scanned directory a repository was found in: the deepest one containing it.
pub fn folder_of<'a>(path: &str, folders: &'a [String]) -> Option<&'a String> {
    let mut best: Option<&String> = None;
    for f in folders {
        let root = f.trim_end_matches(['/', '\\']);
        if (path == root || path.starts_with(&format!("{root}/")) || path.starts_with(&format!("{root}\\"))) && best.is_none_or(|b| root.len() > b.trim_end_matches(['/', '\\']).len()) {
            best = Some(f);
        }
    }
    best
}

/// `addRecent`: to the top of the list, keeping its pin; pinned entries are all kept, the rest fill what's left.
pub fn add_recent(profile: &mut Value, path: &str, name: &str, now: i64) {
    let list = profile["recent"].as_array().cloned().unwrap_or_default();
    let pinned_before = list.iter().find(|r| r["path"] == path).is_some_and(|r| r["pinned"] == true);
    let mut all = vec![json!({ "path": path, "name": name, "pinned": pinned_before, "openedAt": now })];
    all.extend(list.into_iter().filter(|r| r["path"] != path));
    let pinned = all.iter().filter(|r| r["pinned"] == true).count();
    let mut unpinned = 0;
    all.retain(|r| {
        if r["pinned"] == true {
            return true;
        }
        unpinned += 1;
        unpinned <= MAX_RECENT.saturating_sub(pinned)
    });
    profile["recent"] = Value::Array(all);
}

pub fn toggle_pin_recent(profile: &mut Value, path: &str) {
    if let Some(list) = profile["recent"].as_array_mut() {
        for r in list.iter_mut().filter(|r| r["path"] == path) {
            r["pinned"] = json!(r["pinned"] != true);
        }
    }
}

pub fn remove_recent(profile: &mut Value, path: &str) {
    if let Some(list) = profile["recent"].as_array_mut() {
        list.retain(|r| r["path"] != path);
    }
}

/// The recent list as it's shown: pinned first, newest first, what the filter leaves.
pub fn recent_shown(profile: &Value, filter: &str) -> Vec<(String, String, bool)> {
    let q = filter.trim().to_lowercase();
    let mut list: Vec<(String, String, bool, i64)> = profile["recent"].as_array().map_or(Vec::new(), |a| a.iter().map(|r| (r["path"].as_str().unwrap_or_default().to_string(), r["name"].as_str().unwrap_or_default().to_string(), r["pinned"] == true, r["openedAt"].as_i64().unwrap_or(0))).collect());
    list.sort_by_key(|r| (!r.2, -r.3));
    list.into_iter().filter(|r| q.is_empty() || r.1.to_lowercase().contains(&q) || r.0.to_lowercase().contains(&q)).map(|r| (r.0, r.1, r.2)).collect()
}

/// The directories "Your repos" scans.
pub fn repos_folders(profile: &Value) -> Vec<String> {
    let strings = |v: &Value| v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect::<Vec<_>>());
    strings(&profile["reposFolders"]).unwrap_or_else(|| profile["reposFolder"].as_str().map(str::to_string).into_iter().collect())
}

fn same_folder(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

pub fn scanned_of(v: &Value) -> Vec<ScannedRepo> {
    v.as_array().map_or(Vec::new(), |a| {
        a.iter()
            .map(|r| ScannedRepo { path: r["path"].as_str().unwrap_or_default().to_string(), name: r["name"].as_str().unwrap_or_default().to_string(), branch: r["branch"].as_str().map(str::to_string), modified: r["modified"].as_i64().unwrap_or(0), worktree_of: r["worktreeOf"].as_str().map(str::to_string) })
            .collect()
    })
}

/// `shown`: the filter, then A–Z when asked; the scan's own order is most recently changed first.
pub fn repos_shown(repos: &[ScannedRepo], filter: &str, order: RepoOrder) -> Vec<ScannedRepo> {
    let q = filter.trim().to_lowercase();
    let mut list: Vec<ScannedRepo> = repos.iter().filter(|r| q.is_empty() || r.name.to_lowercase().contains(&q) || r.path.to_lowercase().contains(&q)).cloned().collect();
    if order == RepoOrder::Alpha {
        list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.path.cmp(&b.path)));
    }
    list
}

impl Tab {
    pub fn is_open_tab(&self) -> bool {
        self.open_screen.is_some()
    }
}

impl GitVolt {
    /// The profile, once at the start, and what its readers wait for.
    pub fn load_state(&mut self, cx: &mut Context<Self>) {
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(Request::LoadState).await });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(state)) = read.await else { return };
            this.update(cx, |this, cx| {
                this.profile = state["profile"].clone();
                this.settings = state["settings"].clone();
                this.profiles = state["profiles"].clone();
                this.colours_cache.borrow_mut().clear();
                this.apply_profile_layout();
                this.settings_changed(cx);
                let opened: Vec<(String, String)> = std::mem::take(&mut this.pending_recent);
                for (path, name) in opened {
                    this.note_recent(&path, &name);
                }
                this.refresh_open_tabs(cx);
                if !this.state_loaded {
                    this.state_loaded = true;
                    let path = this.start_path.take();
                    this.restore_tabs(path, cx);
                }
                // What was read before the profile knew its preset and pin.
                let repos: Vec<u32> = this.tabs.iter().filter(|t| !t.is_open_tab()).map(|t| t.id).collect();
                for repo in repos {
                    if this.graph_options(repo) != (None, None) {
                        this.reload(repo, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A change to the profile, saved at once. The web UI saves through `saveProfile` too.
    pub fn update_profile(&mut self, change: impl FnOnce(&mut Value), cx: &mut Context<Self>) {
        if self.profile.is_null() {
            return;
        }
        change(&mut self.profile);
        self.colours_cache.borrow_mut().clear();
        let body = crate::write::call("saveProfile", json!({ "profile": self.profile }));
        if !cfg!(test) {
            let api = self.api.clone();
            self.rt.spawn(async move {
                if let Ok(req) = crate::write::request_of(body) {
                    api.dispatch(req).await.ok();
                }
            });
        }
        cx.notify();
    }

    /// A repository opened: it heads the recent list.
    pub fn note_recent(&mut self, path: &str, name: &str) {
        if self.profile.is_null() {
            return self.pending_recent.push((path.to_string(), name.to_string()));
        }
        let now = chrono::Utc::now().timestamp();
        add_recent(&mut self.profile, path, name, now);
        let body = crate::write::call("saveProfile", json!({ "profile": self.profile }));
        if !cfg!(test) {
            let api = self.api.clone();
            self.rt.spawn(async move {
                if let Ok(req) = crate::write::request_of(body) {
                    api.dispatch(req).await.ok();
                }
            });
        }
    }

    fn next_open_id(&mut self) -> u32 {
        self.open_tabs_made += 1;
        OPEN_TAB_IDS + self.open_tabs_made
    }

    /// A new tab on the Open Repository screen, after the active one.
    pub fn open_blank_tab(&mut self, cx: &mut Context<Self>) -> u32 {
        let id = self.next_open_id();
        let mut tab = Tab::new(id, "Open repository".into(), cx);
        tab.open_screen = Some(Box::new(OpenScreen::new(id, cx)));
        let at = if self.tabs.is_empty() { 0 } else { (self.active + 1).min(self.tabs.len()) };
        self.tabs.insert(at, tab);
        self.active = at;
        self.refresh_open_tabs(cx);
        cx.notify();
        id
    }

    /// `file.clone`: a new Open tab with the keyboard in its URL.
    pub fn open_clone_tab(&mut self, cx: &mut Context<Self>) {
        let id = self.open_blank_tab(cx);
        if let Some(handle) = self.open_tab(id).map(|s| s.url.read(cx).focus.clone()) {
            self.focus_next_frame = Some(handle);
        }
    }

    pub fn open_tab(&self, id: u32) -> Option<&OpenScreen> {
        self.tabs.iter().find(|t| t.id == id)?.open_screen.as_deref()
    }

    pub fn open_tab_mut(&mut self, id: u32) -> Option<&mut OpenScreen> {
        self.tabs.iter_mut().find(|t| t.id == id)?.open_screen.as_deref_mut()
    }

    fn notify_open(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    /// The banner's suggestion and the scans of Open tabs that haven't their directories' repositories.
    pub fn refresh_open_tabs(&mut self, cx: &mut Context<Self>) {
        if self.profile.is_null() || cfg!(test) {
            return;
        }
        let folders = repos_folders(&self.profile);
        let want: Vec<u32> = self.tabs.iter().filter(|t| t.open_screen.as_ref().is_some_and(|s| s.repos.is_none() || s.scanned != folders)).map(|t| t.id).collect();
        for id in want {
            self.scan_repos(id, false, cx);
        }
        if self.profile["reposFolder"].is_null() && self.tabs.iter().any(|t| t.open_screen.as_ref().is_some_and(|s| s.suggestion.is_none())) {
            let api = self.api.clone();
            let read = self.rt.spawn(async move { api.dispatch(Request::SuggestReposFolder).await });
            cx.spawn(async move |this, cx| {
                let Ok(Ok(v)) = read.await else { return };
                let Some(found) = v.as_str().map(str::to_string) else { return };
                this.update(cx, |this, cx| {
                    for s in this.tabs.iter_mut().filter_map(|t| t.open_screen.as_mut()) {
                        s.suggestion = Some(found.clone());
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    pub fn scan_repos(&mut self, id: u32, refresh: bool, cx: &mut Context<Self>) {
        let folders = repos_folders(&self.profile);
        let Some(screen) = self.open_tab_mut(id) else { return };
        screen.scan_error = None;
        screen.scanned = folders.clone();
        if folders.is_empty() {
            screen.repos = Some(Vec::new());
            return cx.notify();
        }
        if !refresh {
            screen.repos = None;
        }
        if cfg!(test) {
            return;
        }
        let api = self.api.clone();
        let roots = folders.clone();
        let read = self.rt.spawn(async move { api.dispatch(Request::ScanFolders { roots, refresh }).await });
        cx.spawn(async move |this, cx| {
            let answer = read.await;
            this.update(cx, |this, cx| {
                let Some(screen) = this.open_tab_mut(id) else { return };
                if screen.scanned != folders {
                    return;
                }
                match answer {
                    Ok(Ok(v)) => screen.repos = Some(scanned_of(&v)),
                    Ok(Err(e)) => {
                        screen.repos = Some(Vec::new());
                        screen.scan_error = Some(e.message);
                    }
                    Err(e) => {
                        screen.repos = Some(Vec::new());
                        screen.scan_error = Some(e.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The directory picker. `start` is where it opens; a script answers it with `pickfolder <path>` (`bench.rs`), so no
    /// flow ever drives a real dialog.
    pub fn pick_folder(&mut self, start: Option<String>, then: impl FnOnce(&mut GitVolt, Option<String>, &mut Context<GitVolt>) + 'static, cx: &mut Context<Self>) {
        if let Some(answer) = self.script_folder.take() {
            return then(self, Some(answer), cx);
        }
        // A script never opens a real dialog on whoever's desktop it runs.
        if std::env::var_os("GITVOLT_SCRIPT").is_some() {
            return then(self, None, cx);
        }
        let _ = start;
        let picked = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: None });
        cx.spawn(async move |this, cx| {
            let answer = picked.await;
            let path = match &answer {
                Ok(Ok(Some(paths))) => paths.first().map(|p| p.to_string_lossy().into_owned()),
                _ => None,
            };
            this.update(cx, |this, cx| {
                // No desktop portal: nothing would open, and a click that does nothing says nothing.
                if let Ok(Err(e)) = &answer {
                    this.show_toast(format!("The folder dialog isn't available: {e}"), cx);
                }
                then(this, path, cx);
            })
            .ok();
        })
        .detach();
    }

    /// `open`: a path from this Open tab: the tab becomes the repository, or stays with the error.
    pub fn open_from(&mut self, id: u32, path: String, cx: &mut Context<Self>) {
        if let Some(s) = self.open_tab_mut(id) {
            s.error = None;
            s.failed_recent = None;
        }
        self.open_in(path, Some(id), cx);
    }

    /// Couldn't open it: the Open tab says why; with no Open tab asking, a toast.
    pub fn open_failed(&mut self, id: Option<u32>, path: &str, message: String, cx: &mut Context<Self>) {
        let recent = self.profile["recent"].as_array().is_some_and(|l| l.iter().any(|r| r["path"] == path));
        match id.and_then(|id| self.open_tab_mut(id)) {
            Some(s) => {
                s.error = Some(message);
                s.failed_recent = recent.then(|| path.to_string());
            }
            None => crate::ui::toast::show_with(message, crate::ui::toast::Options { error: true, ..Default::default() }, cx),
        }
        cx.notify();
    }

    fn clone_url_changed(&mut self, id: u32, cx: &mut Context<Self>) {
        let folder = self.profile["reposFolder"].as_str().map(str::to_string);
        let Some(s) = self.open_tab_mut(id) else { return };
        s.clone_error = None;
        if !s.dest_edited {
            let (url, dest) = (s.url.clone(), s.dest.clone());
            let name = repo_name_from_url(url.read(cx).text());
            let text = match (&folder, name.is_empty()) {
                (Some(f), false) => format!("{}/{name}", f.trim_end_matches('/')),
                _ => String::new(),
            };
            dest.update(cx, |d, cx| d.set_text(&text, cx));
        }
        cx.notify();
    }

    fn clone_dest_changed(&mut self, id: u32, cx: &mut Context<Self>) {
        if let Some(s) = self.open_tab_mut(id) {
            s.clone_error = None;
            s.dest_edited = true;
        }
        cx.notify();
    }

    /// `submit`: runs the clone; what's cloned opens in this tab, or in its own if this one's gone by then.
    pub fn start_clone(&mut self, id: u32, cx: &mut Context<Self>) {
        let Some(s) = self.open_tab_mut(id) else { return };
        let (url, dest) = (s.url.read(cx).text().trim().to_string(), s.dest.read(cx).text().trim().to_string());
        if s.busy || url.is_empty() || dest.is_empty() || clone_url_problem(&url).is_some() || clone_dest_problem(&dest).is_some() {
            return;
        }
        s.busy = true;
        s.clone_error = None;
        s.cloning_to = Some(dest.clone());
        let (a, b) = (s.url.clone(), s.dest.clone());
        a.update(cx, |i, cx| i.set_disabled(true, cx));
        b.update(cx, |i, cx| i.set_disabled(true, cx));
        cx.notify();
        if cfg!(test) {
            self.sent.borrow_mut().push(crate::write::call("clone", json!({ "url": url, "dest": dest })));
            return;
        }
        let api = self.api.clone();
        let done = self.rt.spawn(async move { api.dispatch(Request::Clone { url, dest }).await });
        cx.spawn(async move |this, cx| {
            let answer = done.await;
            this.update(cx, |this, cx| {
                if let Some(s) = this.open_tab_mut(id) {
                    s.busy = false;
                    s.cloning_to = None;
                    let (a, b) = (s.url.clone(), s.dest.clone());
                    a.update(cx, |i, cx| i.set_disabled(false, cx));
                    b.update(cx, |i, cx| i.set_disabled(false, cx));
                }
                match answer {
                    Ok(Ok(repo)) => {
                        let path = repo["path"].as_str().unwrap_or_default().to_string();
                        let here = this.open_tab(id).is_some().then_some(id);
                        this.open_in(path, here, cx);
                    }
                    Ok(Err(e)) => {
                        use gitvolt_core::error::GbErrorKind::*;
                        let text = match e.kind {
                            AuthFailed => "Authentication failed or was cancelled".to_string(),
                            Cancelled => "Clone cancelled".to_string(),
                            _ => e.message,
                        };
                        if let Some(s) = this.open_tab_mut(id) {
                            s.clone_error = Some(text);
                        }
                    }
                    Err(e) => {
                        if let Some(s) = this.open_tab_mut(id) {
                            s.clone_error = Some(e.to_string());
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn set_repos_folders(&mut self, id: u32, next: Vec<String>, cx: &mut Context<Self>) {
        self.update_profile(move |p| p["reposFolders"] = json!(next), cx);
        self.scan_repos(id, false, cx);
    }

    /// The banner's choice: the default directory, and the first scanned.
    fn set_default_folder(&mut self, path: String, cx: &mut Context<Self>) {
        self.update_profile(
            move |p| {
                let mut folders = repos_folders(p);
                if !folders.contains(&path) {
                    folders.push(path.clone());
                }
                p["reposFolder"] = json!(path);
                p["reposFolders"] = json!(folders);
            },
            cx,
        );
        self.refresh_open_tabs(cx);
    }
}

fn dim() -> Hsla {
    theme::text_disabled()
}

fn open_btn(id: impl Into<ElementId>, primary: bool, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .h(px(30.2))
        .px(px(12.))
        .rounded(px(4.))
        .border_1()
        .when(primary, |d| d.bg(theme::blue()).border_color(theme::blue()).text_color(theme::text_on_accent()))
        .when(!primary, |d| d.bg(theme::panel_bg1()).border_color(theme::section_border()).text_color(theme::text_selected()))
        .when(!enabled, |d| d.opacity(0.5))
        .when(enabled, |d| d.cursor_pointer().when(!primary, |d| d.hover(|d| d.bg(theme::hover_row()))).when(primary, |d| d.hover(|d| d.bg(theme::mix(theme::blue(), 0.9, white())))))
}

fn icon_btn(id: impl Into<ElementId>, glyph: &'static str, size: f32) -> Stateful<Div> {
    div().id(id).group("open-icon").flex().flex_none().p(px(4.)).rounded(px(3.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).child(icon(glyph, size, theme::text_secondary()).group_hover("open-icon", |s| s.text_color(theme::text_selected())))
}

fn heading(text: &'static str) -> Div {
    div().flex().flex_none().items_center().gap(px(8.)).mb(px(8.)).font_weight(FontWeight::BOLD).text_color(theme::text_selected()).child(text)
}

fn filter_box(id: &'static str, input: &Entity<TextInput>, window: &Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let input2 = input.clone();
    let weak = cx.weak_entity();
    clearable(
        id,
        input,
        move |_, cx| {
            input2.update(cx, |i, cx| i.set_text("", cx));
            weak.update(cx, |_, cx| cx.notify()).ok();
        },
        window,
        cx,
    )
    .flex_none()
    .h(px(30.2))
    .into_any_element()
}

fn field(label: &'static str, input: &Entity<TextInput>, window: &Window, cx: &App) -> Div {
    let focused = input.read(cx).is_focused(window);
    div().flex().flex_col().flex_none().gap(px(4.)).mb(px(10.)).text_color(theme::text_secondary()).child(label).child(div().flex().items_center().h(px(30.2)).px(px(8.)).bg(theme::app_bg()).border_1().border_color(if focused { theme::blue() } else { theme::section_border() }).rounded(px(4.)).text_color(theme::text_selected()).child(input.clone()))
}

fn error_text(text: impl Into<SharedString>) -> Div {
    div().my(px(6.)).text_color(theme::stats_deleted_bright()).child(text.into())
}

/// One keyboard stop: a handle the screen keeps, by what it's for.
fn stop(screen: &mut HashMap<String, FocusHandle>, key: String, cx: &mut App) -> FocusHandle {
    screen.entry(key).or_insert_with(|| cx.focus_handle().tab_stop(true)).clone()
}

fn stop_of(el: Stateful<Div>, focus: &FocusHandle, run: impl Fn(&mut Window, &mut App) + 'static) -> Stateful<Div> {
    el.track_focus(focus).focus(|s| s.bg(theme::hover_row())).on_key_down(move |e: &KeyDownEvent, window, cx| {
        let m = &e.keystroke.modifiers;
        if matches!(e.keystroke.key.as_str(), "enter" | "space") && !m.control && !m.alt && !m.shift && !m.platform {
            cx.stop_propagation();
            run(window, cx);
        }
    })
}

/// The screen, the whole of its tab's page.
pub fn open_screen(this: &mut GitVolt, id: u32, window: &Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let profile = this.profile.clone();
    let tilde_of = |p: &str| crate::sidebar::tilde(p);
    let weak = cx.weak_entity();
    let order = this.repo_order;
    let grouped_pref = this.repo_grouped;
    let Some(screen) = this.open_tab_mut(id) else { return div().into_any_element() };
    let folders = repos_folders(&profile);
    let default_folder = profile["reposFolder"].as_str().map(str::to_string);

    let banner = (default_folder.is_none() && !profile.is_null()).then(|| {
        let use_btn = screen.suggestion.clone().map(|s| {
            let shown = format!("Use {}", tilde_of(&s));
            open_btn("open-use", true, true).child(shown).on_click(cx.listener(move |this, _, _, cx| this.set_default_folder(s.clone(), cx)))
        });
        let start = Option::<String>::None;
        div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .mb(px(16.))
            .px(px(14.))
            .py(px(10.))
            .rounded(px(6.))
            .bg(theme::panel_bg1())
            .border_1()
            .border_color(theme::section_border())
            .child("Set a default repos directory: it's where clones go (and “Your repos” scans it).")
            .children(use_btn)
            .child(crate::ui::anchor("open-choose", open_btn("open-choose", false, true).child("Choose…").on_click(cx.listener(move |this, _, _, cx| {
                this.pick_folder(start.clone(), |this, picked, cx| {
                    if let Some(p) = picked {
                        this.set_default_folder(p, cx);
                    }
                }, cx);
            }))))
            .children(screen.banner_error.clone().map(error_text))
    });

    let error = screen.error.clone().map(|e| {
        let failed = screen.failed_recent.clone();
        error_text(e).flex().items_center().gap(px(10.)).children(failed.map(|path| {
            open_btn("open-forget", false, true).text_color(theme::text_selected()).child("Remove from recent").on_click(cx.listener(move |this, _, _, cx| {
                let p = path.clone();
                this.update_profile(move |prof| remove_recent(prof, &p), cx);
                if let Some(s) = this.open_tab_mut(id) {
                    s.error = None;
                    s.failed_recent = None;
                }
            }))
        }))
    });

    // Recent.
    let filter_text = screen.filter.read(cx).text().to_string();
    let recent_all = profile["recent"].as_array().map_or(0, Vec::len);
    let shown = recent_shown(&profile, &filter_text);
    let filter_el = screen.filter.clone();
    let mut recent_rows = Vec::new();
    for (i, (path, name, pinned)) in shown.iter().enumerate() {
        let (open_path, pin_path, drop_path) = (path.clone(), path.clone(), path.clone());
        let focus = stop(&mut screen.focus, format!("recent:{path}"), cx);
        let w = weak.clone();
        let p2 = path.clone();
        let row = stop_of(
            div().id(("open-recent", i)).flex().flex_1().min_w_0().items_baseline().gap(px(10.)).px(px(8.)).py(px(5.)).rounded(px(4.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).on_click(cx.listener(move |this, _, _, cx| this.open_from(id, open_path.clone(), cx))).child(div().flex_none().text_color(theme::text_selected()).child(name.clone())).child(div().min_w_0().truncate().text_size(px(11.)).text_color(dim()).child(tilde_of(path))),
            &focus,
            move |_, cx| {
                let p = p2.clone();
                w.update(cx, |v, cx| v.open_from(id, p, cx)).ok();
            },
        );
        let pin_tip = if *pinned { "Unpin" } else { "Pin to the top" };
        recent_rows.push(
            crate::ui::anchor(
                format!("open-recent:{name}"),
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .child(tip(path.clone(), row).into_any_element())
                    .child(tip(pin_tip, icon_btn(("open-pin", i), if *pinned { "pin-off" } else { "pin" }, 13.).on_click(cx.listener(move |this, _, _, cx| {
                        let p = pin_path.clone();
                        this.update_profile(move |prof| toggle_pin_recent(prof, &p), cx);
                    }))))
                    .child(tip("Remove from recent (the repository is untouched)", icon_btn(("open-drop", i), "x", 13.).on_click(cx.listener(move |this, _, _, cx| {
                        let p = drop_path.clone();
                        this.update_profile(move |prof| remove_recent(prof, &p), cx);
                    })))),
            )
            .into_any_element(),
        );
    }
    let screen = this.open_tab_mut(id).expect("the tab");
    let recent_empty = shown.is_empty().then(|| div().my(px(13.)).text_color(dim()).child(if recent_all == 0 { "Repositories you open appear here." } else { "No recent repository matches." }));
    let recent = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .child(heading("Recent"))
        .child(filter_box("open-recent-filter", &filter_el, window, cx))
        .child(div().id("open-recent-list").flex_1().min_h_0().overflow_y_scroll().children(recent_empty).when(!shown.is_empty(), |d| d.child(div().flex().flex_col().mt(px(8.)).children(recent_rows))));

    // Open and Clone.
    let dir_focus = stop(&mut screen.focus, "open-directory".into(), cx);
    let w = weak.clone();
    let start = default_folder.clone();
    let open_dir = crate::ui::anchor(
        "open-directory",
        stop_of(
            open_btn("open-directory", true, true).child(icon("folder-open", 14., theme::text_on_accent())).child("Open directory…").on_click(cx.listener({
                let start = start.clone();
                move |this, _, _, cx| {
                    this.pick_folder(start.clone(), move |this, picked, cx| {
                        if let Some(p) = picked {
                            this.open_from(id, p, cx);
                        }
                    }, cx);
                }
            })),
            &dir_focus,
            move |_, cx| {
                let start = start.clone();
                w.update(cx, |v, cx| {
                    v.pick_folder(start, move |this, picked, cx| {
                        if let Some(p) = picked {
                            this.open_from(id, p, cx);
                        }
                    }, cx);
                })
                .ok();
            },
        ),
    );
    let (url_text, dest_text) = (screen.url.read(cx).text().to_string(), screen.dest.read(cx).text().to_string());
    let busy = screen.busy;
    let problem = clone_url_problem(&url_text).or(clone_dest_problem(&dest_text));
    let can_clone = !busy && !url_text.trim().is_empty() && !dest_text.trim().is_empty() && problem.is_none();
    let (url_field, dest_field) = (screen.url.clone(), screen.dest.clone());
    let cloning_to = screen.cloning_to.clone();
    let clone_error = screen.clone_error.clone();
    let clone_focus = stop(&mut screen.focus, "open-clone".into(), cx);
    let op = cloning_to.as_ref().and_then(|d| this.ops.ops.values().find(|o| o.kind == OpKind::Clone && &o.label == d).cloned());
    let progress = busy.then(|| {
        let phase = op.as_ref().and_then(|o| o.phase.clone()).unwrap_or_else(|| "Starting…".to_string());
        let percent = op.as_ref().and_then(|o| o.percent);
        let text = match percent {
            Some(p) => format!("{phase} {p}%"),
            None => phase,
        };
        let op_id = op.as_ref().map(|o| o.op);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .my(px(6.))
            .child(crate::commit_box::spinner("clone-spin", 14., theme::text_secondary()))
            .child(div().flex_none().child(text))
            .child(div().relative().flex_1().h(px(8.)).rounded(px(4.)).bg(theme::app_bg()).overflow_hidden().child(div().absolute().left_0().top_0().bottom_0().w(relative(f32::from(percent.unwrap_or(0)) / 100.)).bg(theme::blue())))
            .children(op_id.map(|op| {
                open_btn("clone-cancel", false, true).child("Cancel").on_click(cx.listener(move |this, _, _, cx| {
                    let api = this.api.clone();
                    this.rt.spawn(async move { api.dispatch(Request::CancelOp { op }).await });
                    cx.notify();
                }))
            }))
    });
    let w = weak.clone();
    let clone_btn = crate::ui::anchor(
        "open-clone",
        stop_of(
            open_btn("open-clone", true, can_clone).child("Clone").when(can_clone, |d| d.on_click(cx.listener(move |this, _, _, cx| this.start_clone(id, cx)))),
            &clone_focus,
            move |_, cx| {
                w.update(cx, |v, cx| v.start_clone(id, cx)).ok();
            },
        ),
    );
    let middle = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .gap(px(20.))
        .child(div().flex().flex_col().flex_none().child(heading("Open")).child(div().flex().flex_col().child(open_dir)).child(div().my(px(13.)).text_color(dim()).child("Any directory inside a repository opens that repository; a linked worktree opens as its own tab.")))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .child(heading("Clone"))
                .child(crate::ui::anchor("open-url", field("Repository URL", &url_field, window, cx)))
                .child(crate::ui::anchor("open-dest", field("Destination", &dest_field, window, cx)))
                .children(progress)
                .children(problem.filter(|_| !busy).map(error_text))
                .children(clone_error.map(error_text))
                .child(div().flex().flex_none().justify_end().child(clone_btn)),
        );

    // Your repos.
    let screen = this.open_tab_mut(id).expect("the tab");
    let repos_filter = screen.repos_filter.clone();
    let filter_q = repos_filter.read(cx).text().to_string();
    let scanning = screen.repos.is_none();
    let list = screen.repos.as_ref().map(|r| repos_shown(r, &filter_q, order));
    let total = screen.repos.as_ref().map_or(0, Vec::len);
    let scan_error = screen.scan_error.clone();
    let several = folders.len() > 1;
    let grouped = grouped_pref && several;
    let labels = several.then(|| folder_labels(&folders));
    let now = chrono::Utc::now().timestamp() as f64;
    let row = |r: &ScannedRepo, tagged: bool, key: String, cx: &mut Context<GitVolt>| -> AnyElement {
        let path = r.path.clone();
        let mut card = r.path.clone();
        if let Some(of) = &r.worktree_of {
            card.push_str(&format!("\nWorktree of {of}"));
        }
        card.push_str(&match &r.branch {
            Some(b) => format!("\nOn {b}"),
            None => "\nDetached HEAD".to_string(),
        });
        let from = tagged.then(|| labels.as_ref().and_then(|l| folder_of(&r.path, &folders).and_then(|f| l.get(f)).cloned())).flatten();
        let name = r.name.clone();
        let body = div()
            .id(SharedString::from(key))
            .flex()
            .flex_none()
            .items_baseline()
            .gap(px(10.))
            .min_w_0()
            .px(px(8.))
            .py(px(5.))
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|d| d.bg(theme::hover_row()))
            .on_click(cx.listener(move |this, _, _, cx| this.open_from(id, path.clone(), cx)))
            .child(div().flex().flex_none().items_center().text_color(theme::text_selected()).when(r.worktree_of.is_some(), |d| d.child(div().mr(px(5.)).child(icon("tree-pine", 12., theme::text_secondary())))).child(name))
            .children(from.map(|f| div().flex_initial().min_w(px(20.)).truncate().px(px(5.)).rounded(px(3.)).bg(theme::panel_bg1()).text_size(px(11.)).text_color(dim()).child(f)))
            .child(div().flex().flex_initial().min_w_0().items_center().gap(px(3.)).text_size(px(11.)).text_color(dim()).child(icon("git-branch", 11., dim())).child(div().truncate().child(r.branch.clone().unwrap_or_else(|| "detached".to_string()))))
            .child(div().flex_none().ml_auto().text_size(px(11.)).text_color(dim()).child(crate::format::relative_time(r.modified, now)));
        tip(card, body).into_any_element()
    };
    let chip = |f: &String, cx: &mut Context<GitVolt>| -> AnyElement {
        let (next, full) = (folders.iter().filter(|x| *x != f).cloned().collect::<Vec<_>>(), f.clone());
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .min_w_0()
            .pl(px(8.))
            .pr(px(2.))
            .rounded(px(4.))
            .bg(theme::panel_bg1())
            .child(div().flex_1().min_w_0().truncate().text_size(px(11.)).text_color(dim()).child(tip(full, div().child(tilde_of(f)))))
            .child(tip("Stop scanning this directory", icon_btn(SharedString::from(format!("open-unscan:{f}")), "x", 12.).on_click(cx.listener(move |this, _, _, cx| this.set_repos_folders(id, next.clone(), cx)))))
            .into_any_element()
    };
    let mut head = heading("Your repos");
    head = head
        .child(tip("Scan the directories again", icon_btn("open-refresh", "refresh-cw", 13.).on_click(cx.listener(move |this, _, _, cx| this.scan_repos(id, true, cx)))))
        .child(tip("Add a directory to scan", crate::ui::anchor("open-add-folder", icon_btn("open-add-folder", "plus", 14.).on_click(cx.listener({
            let folders = folders.clone();
            move |this, _, _, cx| {
                let f = folders.clone();
                this.pick_folder(folders.last().cloned(), move |this, picked, cx| {
                    if let Some(p) = picked
                        && !f.iter().any(|x| same_folder(x, &p))
                    {
                        let mut next = f.clone();
                        next.push(p);
                        this.set_repos_folders(id, next, cx);
                    }
                }, cx);
            }
        })))));
    if several {
        let (t, g) = if grouped { ("Grouped by directory: switch to one list", "folder-tree") } else { ("One list: switch to grouped by directory", "list") };
        head = head.child(div().ml_auto().child(tip(t, icon_btn("open-layout", g, 14.).on_click(cx.listener(|this, _, _, cx| {
            this.repo_grouped = !this.repo_grouped;
            this.save_layout();
            cx.notify();
        })))));
    }
    let (t, g, s) = if order == RepoOrder::Alpha { ("Sorted A–Z: switch to most recently changed first", "arrow-down-az", 14.) } else { ("Most recently changed first: switch to A–Z", "clock", 13.) };
    head = head.child(div().when(!several, |d| d.ml_auto()).child(tip(t, icon_btn("open-order", g, s).on_click(cx.listener(|this, _, _, cx| {
        this.repo_order = if this.repo_order == RepoOrder::Alpha { RepoOrder::Recent } else { RepoOrder::Alpha };
        this.save_layout();
        cx.notify();
    })))));
    let body: AnyElement = if folders.is_empty() {
        div().my(px(13.)).text_color(dim()).child("Add a directory with + and its repositories are listed here.").into_any_element()
    } else if grouped {
        let mut groups = div().flex().flex_col().gap(px(10.));
        for (g, f) in folders.iter().enumerate() {
            let inner: Vec<ScannedRepo> = list.as_ref().map_or(Vec::new(), |l| l.iter().filter(|r| folder_of(&r.path, &folders) == Some(f)).cloned().collect());
            let rows: Vec<AnyElement> = inner.iter().enumerate().map(|(i, r)| row(r, false, format!("open-repo:{g}:{i}"), cx)).collect();
            let empty = inner.is_empty() && !scanning;
            groups = groups.child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .child(chip(f, cx))
                    .when(empty, |d| d.child(div().px(px(8.)).py(px(5.)).text_color(dim()).child(if filter_q.trim().is_empty() { "No repositories found two levels deep." } else { "No repository matches." })))
                    .when(!rows.is_empty(), |d| d.child(div().flex().flex_col().mt(px(2.)).children(rows))),
            );
        }
        groups.into_any_element()
    } else if scanning {
        div().my(px(13.)).text_color(dim()).child("Scanning…").into_any_element()
    } else if total == 0 {
        if scan_error.is_some() { div().into_any_element() } else { div().my(px(13.)).text_color(dim()).child("No repositories found two levels deep.").into_any_element() }
    } else if list.as_ref().is_some_and(Vec::is_empty) {
        div().my(px(13.)).text_color(dim()).child("No repository matches.").into_any_element()
    } else {
        let rows: Vec<AnyElement> = list.as_ref().map_or(Vec::new(), |l| l.iter().enumerate().map(|(i, r)| row(r, true, format!("open-repo:{i}"), cx)).collect());
        div().flex().flex_col().mt(px(8.)).children(rows).into_any_element()
    };
    let chips: Vec<AnyElement> = if grouped { Vec::new() } else { folders.iter().map(|f| chip(f, cx)).collect() };
    let yours = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .child(head)
        .when(!folders.is_empty(), |d| d.child(div().mb(px(6.)).child(filter_box("open-repos-filter", &repos_filter, window, cx))))
        .when(!chips.is_empty(), |d| d.child(div().flex().flex_col().flex_none().gap(px(2.)).mb(px(4.)).children(chips)))
        .children(scan_error.map(|e| error_text(format!("Couldn't scan: {e}"))))
        .child(div().id("open-yours-list").flex_1().min_h_0().overflow_y_scroll().child(body));

    div()
        .id("open-screen")
        .key_context("OpenScreen")
        .on_action(|_: &crate::ui::modal::FocusNext, window, cx| window.focus_next(cx))
        .on_action(|_: &crate::ui::modal::FocusPrev, window, cx| window.focus_prev(cx))
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .px(px(28.))
        .py(px(20.))
        .bg(theme::app_bg())
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .w_full()
                .max_w(px(1440.))
                .mx_auto()
                .children(banner)
                .children(error)
                .child(div().flex().flex_1().min_h_0().gap(px(20.)).child(recent).child(middle).child(yours)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_name_comes_from_the_url() {
        assert_eq!(repo_name_from_url("git@gitlab.example.com:group/project.git"), "project");
        assert_eq!(repo_name_from_url("https://example.com/a/b/"), "b");
        assert_eq!(repo_name_from_url("/srv/git/thing.GIT"), "thing");
        assert_eq!(repo_name_from_url("  "), "");
    }

    #[test]
    fn a_url_and_a_destination_are_checked_as_the_web_does() {
        for ok in ["https://example.com/a/b.git", "ssh://git@example.com/a.git", "git://example.com/a", "file:///srv/a.git", "git@example.com:a/b.git", "/srv/a.git", ""] {
            assert_eq!(clone_url_problem(ok), None, "{ok}");
        }
        for bad in ["not a url", "ftp://example.com/a", "example.com/a", "https://"] {
            assert!(clone_url_problem(bad).is_some(), "{bad}");
        }
        assert_eq!(clone_dest_problem("/home/x"), None);
        assert_eq!(clone_dest_problem(""), None);
        assert!(clone_dest_problem("x/y").is_some());
    }

    #[test]
    fn folders_get_their_shortest_distinct_names() {
        let f = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let two = f(&["/home/u/repos", "/home/u/work"]);
        let l = folder_labels(&two);
        assert_eq!((l["/home/u/repos"].as_str(), l["/home/u/work"].as_str()), ("repos", "work"));
        let same = f(&["/a/src", "/b/src"]);
        let l = folder_labels(&same);
        assert_eq!((l["/a/src"].as_str(), l["/b/src"].as_str()), ("a/src", "b/src"));
        let nested = f(&["/a", "/a/b/"]);
        assert_eq!(folder_of("/a/b/c", &nested).map(String::as_str), Some("/a/b/"));
        assert_eq!(folder_of("/a/x", &nested).map(String::as_str), Some("/a"));
        assert_eq!(folder_of("/z", &nested), None);
    }

    #[test]
    fn the_recent_list_keeps_its_pins_and_its_length() {
        let mut p = json!({ "recent": [{ "path": "/a", "name": "a", "pinned": true, "openedAt": 1 }, { "path": "/b", "name": "b", "pinned": false, "openedAt": 2 }] });
        add_recent(&mut p, "/a", "a", 3);
        assert_eq!(p["recent"][0]["pinned"], true);
        assert_eq!(p["recent"].as_array().unwrap().len(), 2);
        for i in 0..60 {
            add_recent(&mut p, &format!("/r{i}"), "r", 10 + i);
        }
        let list = p["recent"].as_array().unwrap();
        assert_eq!(list.len(), MAX_RECENT);
        assert!(list.iter().any(|r| r["path"] == "/a"), "pinned stays");
        toggle_pin_recent(&mut p, "/r59");
        assert_eq!(recent_shown(&p, "")[0].0, "/r59", "pinned on top, newest first");
        remove_recent(&mut p, "/r59");
        assert_eq!(recent_shown(&p, "r59").len(), 0);
    }

    #[test]
    fn the_scan_filters_and_sorts() {
        let repo = |name: &str, path: &str| ScannedRepo { path: path.into(), name: name.into(), branch: None, modified: 0, worktree_of: None };
        let list = vec![repo("zeta", "/x/zeta"), repo("Alpha", "/x/alpha")];
        assert_eq!(repos_shown(&list, "", RepoOrder::Recent)[0].name, "zeta");
        assert_eq!(repos_shown(&list, "", RepoOrder::Alpha)[0].name, "Alpha");
        assert_eq!(repos_shown(&list, "/x/z", RepoOrder::Recent).len(), 1);
    }
}
