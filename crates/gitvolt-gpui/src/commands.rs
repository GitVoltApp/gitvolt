//! The one table of the app's actions (`ui/src/app/actions.ts`, `coreActions.ts` and each feature's
//! registrations): id, label, group, icon, tooltip, shortcuts, when it's usable and what it runs. The
//! hamburger menu, the shortcuts, the toolbar and the palette's actions all read it; the Keyboard
//! Shortcuts panel will too. A command with no `run` isn't ported yet: its menu rows stay, disabled.
//!
//! To add one: a row in `all()`. Shortcuts are named as the web UI names them, `Mod` for Ctrl (Cmd on
//! macOS). The chords here go through `dispatch`; bare keys (arrows, Esc) stay GPUI bindings.

use crate::app::GitVolt;
use crate::ui::menu::{self, MenuRow};
use crate::ui::toast;
use gitvolt_core::api::Request;
use gpui::*;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    File,
    Edit,
    View,
    Repository,
    Help,
}

pub type Run = fn(&mut GitVolt, &mut Window, &mut Context<GitVolt>);
pub type When = fn(&GitVolt, &App) -> bool;

#[derive(Clone)]
pub struct Command {
    pub id: &'static str,
    pub label: SharedString,
    pub group: Group,
    pub icon: &'static str,
    pub tooltip: SharedString,
    /// `Mod+Shift+T`; the first is what menus show.
    pub shortcuts: Vec<String>,
    pub when: Option<When>,
    /// Left out of the hamburger, still in the palette and on its keys.
    pub menu: bool,
    /// Its keys are the field's while one has the keyboard (Ctrl+V).
    pub yields_to_typing: bool,
    pub run: Option<Run>,
}

impl Command {
    fn new(id: &'static str, label: impl Into<SharedString>, group: Group, icon: &'static str, tooltip: impl Into<SharedString>, shortcuts: &[&str]) -> Self {
        Self { id, label: label.into(), group, icon, tooltip: tooltip.into(), shortcuts: shortcuts.iter().map(|s| s.to_string()).collect(), when: None, menu: true, yields_to_typing: false, run: None }
    }

    fn when(mut self, when: When) -> Self {
        self.when = Some(when);
        self
    }

    fn run(mut self, run: Run) -> Self {
        self.run = Some(run);
        self
    }

    fn hidden(mut self) -> Self {
        self.menu = false;
        self
    }

    fn yields(mut self) -> Self {
        self.yields_to_typing = true;
        self
    }

    pub fn usable(&self, v: &GitVolt, cx: &App) -> bool {
        self.run.is_some() && self.when.is_none_or(|w| w(v, cx))
    }

    /// The first chord, as menus show it.
    pub fn chord(&self) -> Option<SharedString> {
        self.shortcuts.first().map(|s| display_chord(s).into())
    }

    /// `tooltip (chord)`, as a toolbar button's.
    pub fn full_tooltip(&self) -> SharedString {
        match self.chord() {
            Some(c) => format!("{} ({c})", self.tooltip).into(),
            None => self.tooltip.clone(),
        }
    }
}

/// `Mod` for the platform's primary modifier.
pub fn resolve_chord(chord: &str) -> String {
    match chord.strip_prefix("Mod+") {
        Some(rest) => format!("{}+{rest}", if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" }),
        None => chord.to_string(),
    }
}

/// A chord as menus and tooltips show it.
pub fn display_chord(chord: &str) -> String {
    resolve_chord(chord)
}

/// `comboOf`: the name a key press goes by; `None` for a bare key.
pub fn combo_of(k: &Keystroke) -> Option<String> {
    let m = &k.modifiers;
    if m.platform && (m.control || !cfg!(target_os = "macos")) {
        return None;
    }
    let primary = if m.platform { Some("Cmd") } else if m.control { Some("Ctrl") } else { None };
    let key = match k.key.as_str() {
        "tab" => "Tab".to_string(),
        "pageup" => "PageUp".to_string(),
        "pagedown" => "PageDown".to_string(),
        "enter" => "Enter".to_string(),
        "space" => "Space".to_string(),
        "up" => "Up".to_string(),
        "down" => "Down".to_string(),
        "left" => "Left".to_string(),
        "right" => "Right".to_string(),
        // A Shift+digit by the character it types, as some layouts report it.
        "!" if m.shift => "1".to_string(),
        "@" if m.shift => "2".to_string(),
        "#" if m.shift => "3".to_string(),
        key if key.len() == 1 => key.to_uppercase(),
        key if key.starts_with('f') && key[1..].parse::<u8>().is_ok() => key.to_uppercase(),
        _ => return None,
    };
    if primary.is_none() {
        let digit = key.len() == 1 && key.as_bytes()[0].is_ascii_digit();
        let function = key.starts_with('F') && key.len() > 1;
        if !function && !(m.alt && digit) && !(m.alt && key == "Space") {
            return None;
        }
    }
    let mut parts: Vec<&str> = Vec::new();
    parts.extend(primary);
    if m.alt {
        parts.push("Alt");
    }
    if m.shift {
        parts.push("Shift");
    }
    parts.push(&key);
    Some(parts.join("+"))
}

fn has_tab(v: &GitVolt, _: &App) -> bool {
    !v.tabs.is_empty()
}

fn many_tabs(v: &GitVolt, _: &App) -> bool {
    v.tabs.len() > 1
}

fn file_open(v: &GitVolt, _: &App) -> bool {
    v.tabs.get(v.active).is_some_and(|t| t.diff.is_some())
}

fn has_graph(v: &GitVolt, _: &App) -> bool {
    v.tabs.get(v.active).and_then(|t| t.graph.as_ref()).is_some_and(|g| !g.rows.is_empty())
}

fn has_wip(v: &GitVolt, _: &App) -> bool {
    v.tabs.get(v.active).and_then(|t| t.graph.as_ref()).is_some_and(|g| g.rows.first().is_some_and(|r| r.kind == gitvolt_graph::NodeKind::Wip))
}

/// Every action, in the order the web UI registers them.
pub fn all() -> Vec<Command> {
    use Group::*;
    let mut out = vec![
        Command::new("file.openRepo", "Open repository…", File, "folder-plus", "Open the Open Repository screen in a new tab", &["Mod+O"]).run(|v, _, cx| v.pick_repository(cx)),
        Command::new("file.clone", "Clone repository…", File, "copy-plus", "Clone a repository into a new tab", &[]),
        Command::new("file.closeFile", "Close file", File, "file-x", "Close the open file and go back to the graph", &["Mod+W"]).when(file_open).hidden().run(|v, _, cx| v.close_file_or_history(cx)),
        Command::new("file.closeTab", "Close tab", File, "x", "Close the current tab", &["Mod+W"]).when(has_tab).run(|v, _, cx| v.close_tab(v.active, cx)),
        Command::new("file.reopenTab", "Reopen closed tab", File, "rotate-ccw", "Reopen the most recently closed tab", &["Mod+Shift+T"]).when(|v, _| !v.closed_tabs.is_empty()).run(|v, _, cx| v.reopen_tab(cx)),
        Command::new("file.settings", "Settings", File, "settings", "App, profile and repository settings", &["Mod+,"]),
        Command::new("file.quit", "Quit", File, "log-out", "Quit GitVolt", &[]).run(|_, _, cx| cx.quit()),
        Command::new("view.nextTab", "Next tab", View, "arrow-right-to-line", "Switch to the next tab", &["Ctrl+Tab", "Ctrl+PageDown"]).when(many_tabs).hidden().run(|v, _, cx| v.step_tab(1, cx)),
        Command::new("view.prevTab", "Previous tab", View, "arrow-left-to-line", "Switch to the previous tab", &["Ctrl+Shift+Tab", "Ctrl+PageUp"]).when(many_tabs).hidden().run(|v, _, cx| v.step_tab(-1, cx)),
    ];
    macro_rules! tab_command {
        ($id:literal, $n:literal) => {
            Command::new($id, concat!("Go to tab ", $n), View, "arrow-right-to-line", concat!("Switch to tab ", $n), &[concat!("Mod+", $n)]).when(|v, _| v.tabs.len() >= $n).hidden().run(|v, _, cx| v.go_to_tab($n - 1, cx))
        };
    }
    out.extend([tab_command!("view.tab1", 1), tab_command!("view.tab2", 2), tab_command!("view.tab3", 3), tab_command!("view.tab4", 4), tab_command!("view.tab5", 5), tab_command!("view.tab6", 6), tab_command!("view.tab7", 7), tab_command!("view.tab8", 8)]);
    out.extend([
        Command::new("view.lastTab", "Go to the last tab", View, "arrow-right-to-line", "Switch to the last tab", &["Mod+9"]).when(many_tabs).hidden().run(|v, _, cx| v.go_to_tab(v.tabs.len() - 1, cx)),
        Command::new("edit.undo", "Undo", Edit, "undo-2", "Undo the last action", &["Mod+Z"]).yields(),
        Command::new("edit.redo", "Redo", Edit, "redo-2", "Redo what was undone", &["Mod+Shift+Z"]).yields(),
        Command::new("edit.filterSidebar", "Filter sidebar", Edit, "list-filter", "Focus the sidebar filter", &["Mod+Alt+F"]).when(has_tab).run(|v, window, cx| v.focus_sidebar_filter(window, cx)),
        Command::new("edit.find", "Find in graph", Edit, "search", "Find commits by message, SHA or path", &["Mod+F"]).when(has_graph).run(|v, window, cx| v.open_find(window, cx)),
        Command::new("edit.palette", "Command palette", Edit, "command", "Search actions, branches, files, settings and tabs", &["Mod+P"]).run(|v, window, cx| v.open_palette("", window, cx)),
        Command::new("edit.gotoClipboard", "Go to what's on the clipboard", Edit, "clipboard-paste", "Go to the commit, merge request, file or branch the clipboard names", &["Mod+V"]).when(has_tab).yields().run(|v, _, cx| v.go_to_clipboard(cx)),
        Command::new("view.goToWip", "Go to working directory changes", View, "file-pen", "Show the working directory changes", &["Mod+Shift+G"]).when(has_wip).run(|v, _, cx| v.go_to_wip(cx)),
        Command::new("view.toggleSidebar", "Toggle sidebar", View, "panel-left", "Switch the sidebar between full and icon strip", &["Mod+B"]).when(has_tab).run(|v, _, cx| v.toggle_sidebar(cx)),
        Command::new("view.theme", "Theme…", View, "palette", "Choose the colour theme", &[]),
        Command::new("nav.back", "Go back", View, "arrow-left", "Back to the previous MR/PR, file or commit you visited", &[]),
        Command::new("nav.forward", "Go forward", View, "arrow-right", "Forward to the next MR/PR, file or commit you visited", &[]),
        Command::new("presets.next", "Next branch preset", View, "git-fork", "Switch the graph to the next branch preset in the cycle", &["Mod+Alt+G"]),
        Command::new("presets.choose", "Choose branch preset…", View, "git-fork", "List every branch preset, at the button beside the sidebar filter", &["Mod+Alt+Shift+G"]),
        Command::new("view.focusSidebar", "Focus the sidebar", View, "panel-left", "Put the keyboard in the sidebar", &["Alt+1"]).when(has_tab).hidden().run(|v, window, cx| v.focus_sidebar(window, cx)),
        Command::new("view.focusGraph", "Focus the graph", View, "git-graph", "Put the keyboard in the graph (closing the open file, as Esc does)", &["Alt+2"]).when(has_tab).hidden().run(|v, window, cx| v.focus_graph(window, cx)),
        Command::new("view.focusFiles", "Focus the file list", View, "list", "Put the keyboard in the file list", &["Alt+3"]).when(|v, _| v.tabs.get(v.active).is_some_and(|t| t.files.is_some())).hidden().run(|v, window, cx| v.focus_files(window, cx)),
        Command::new("diff.nextChange", "Next change", View, "arrow-down", "Go to the next change in the diff", &["F7"]).when(file_open).hidden().run(|v, _, cx| v.step_change(true, cx)),
        Command::new("diff.prevChange", "Previous change", View, "arrow-up", "Go to the previous change in the diff", &["Shift+F7"]).when(file_open).hidden().run(|v, _, cx| v.step_change(false, cx)),
        Command::new("diff.nextFile", "Next file", View, "file-down", "Open the next file in the file list", &["F8"]).when(file_open).hidden().run(|v, _, cx| v.step_files(true, cx)),
        Command::new("diff.prevFile", "Previous file", View, "file-up", "Open the previous file in the file list", &["Shift+F8"]).when(file_open).hidden().run(|v, _, cx| v.step_files(false, cx)),
        Command::new("diff.mode.hunk", "Hunk diff view", View, "type", "Show the diff as Hunk", &["Mod+Shift+1"]).when(file_open).hidden().run(|v, _, cx| v.with_diff(cx, |d| d.set_mode(crate::diff::model::Mode::Hunk))),
        Command::new("diff.mode.inline", "Inline diff view", View, "type", "Show the diff as Inline", &["Mod+Shift+2"]).when(file_open).hidden().run(|v, _, cx| v.with_diff(cx, |d| d.set_mode(crate::diff::model::Mode::Inline))),
        Command::new("diff.mode.split", "Split diff view", View, "type", "Show the diff as Split", &["Mod+Shift+3"]).when(file_open).hidden().run(|v, _, cx| v.with_diff(cx, |d| d.set_mode(crate::diff::model::Mode::Split))),
        Command::new("history.file", "File history of the open file", View, "history", "Show the commits that changed the open file", &["Mod+Shift+H"]).when(file_open).hidden().run(|v, _, cx| v.open_history(false, cx)),
        Command::new("history.blame", "Blame the open file", View, "user", "Show who last changed each line of the open file", &["Mod+Shift+B"]).when(file_open).hidden().run(|v, _, cx| v.open_history(true, cx)),
        Command::new("repo.copyPath", "Copy repository path", Repository, "copy", "Copy the current repository's directory path", &[]).when(has_tab).run(|v, _, cx| v.copy_repo_path(cx)),
        Command::new("repo.openFolder", "Open in file manager", Repository, "folder-open", "Open the tab's worktree directory in the file manager", &[]).when(has_tab).run(|v, _, cx| v.open_repo_folder(cx)),
        Command::new("repo.fetch", "Fetch all", Repository, "download", "Fetch every remote of the current repository", &["Mod+L"]),
        Command::new("repo.syncDefault", "Fetch", Repository, "download", "Fetch every remote", &[]),
        Command::new("worktree.switch", "Switch worktree…", Repository, "arrow-right-left", "Change the tab's active worktree", &[]),
        Command::new("branch.create", "Branch", Repository, "git-branch-plus", "Create a branch at HEAD", &["Mod+Shift+N"]),
        Command::new("stash.push", "Stash", Repository, "archive", "Stash every change, named from the WIP message", &["Mod+Alt+S"]),
        Command::new("stash.pop", "Pop", Repository, "archive-restore", "Apply the newest stash and delete it", &["Mod+Alt+P"]),
        Command::new("sync.pull", "Pull", Repository, "arrow-down-to-line", "Pull the current branch's upstream", &["Mod+Shift+L"]),
        Command::new("sync.push", "Push", Repository, "upload", "Push the current branch", &["Mod+Shift+K"]),
        Command::new("gitflow.init", "Set up git flow…", Repository, "workflow", "Choose git flow's branches and prefixes for this repository", &[]),
        Command::new("repo.interactiveRebase", "Interactive rebase…", Repository, "list-ordered", "Reorder, reword, squash or drop the current branch's commits on its upstream", &[]),
        Command::new("file.create", "Create file…", Repository, "file-plus", "Create a new, empty file in the working tree (you can undo this)", &[]),
        Command::new("remote.add", "Add remote…", Repository, "plus", "Add a remote by URL, or one of the project's forks", &[]),
        Command::new("submodule.add", "Submodules: Add…", Repository, "boxes", "Clone a repository into this one as a submodule (git submodule add)", &[]),
        Command::new("stage.file", "Stage file", Repository, "file-plus", "Stage the open file", &["Mod+Shift+S"]).when(|v, _| v.open_wip_file().is_some_and(|f| f.1 == Some(false))).hidden().run(|v, _, cx| v.stage_open_file(cx)),
        Command::new("stage.unstageFile", "Unstage file", Repository, "file-minus", "Move the open file back to Unstaged", &["Mod+Shift+S"]).when(|v, _| v.open_wip_file().is_some_and(|f| f.1 == Some(true))).hidden().run(|v, _, cx| v.stage_open_file(cx)),
        Command::new("stage.all", "Stage all changes", Repository, "list-plus", "Stage every unstaged change", &[]).when(|v, _| v.wip_counts().is_some_and(|c| c.1 > 0)).hidden().run(|v, _, cx| {
            if let Some(ctx) = v.panel_ctx() {
                v.stage_all_guarded(&ctx, cx);
            }
        }),
        Command::new("stage.unstageAll", "Unstage all changes", Repository, "list-minus", "Move every staged change back to Unstaged", &[]).when(|v, _| v.wip_counts().is_some_and(|c| c.0 > 0)).hidden().run(|v, _, cx| {
            if let Some(ctx) = v.panel_ctx() {
                v.unstage_all(&ctx, cx);
            }
        }),
        Command::new("commit.commit", "Commit", Repository, "git-commit-horizontal", "Commit with the message in the commit box", &["Mod+Enter"]).when(|v, _| v.panel_ctx().is_some()).hidden().yields().run(|v, _, cx| v.commit_key(cx)),
        Command::new("help.about", "About GitVolt", Help, "info", "Version information", &[]).run(|v, window, cx| v.open_about(window, cx)),
        Command::new("help.activityLog", "Activity log", Help, "history", "Every finished fetch and clone, with git's message", &[]),
        Command::new("help.debug", "Debug…", Help, "bug", "The git commands GitVolt ran, the actions you ran, diagnostics and the perf overlay", &[]),
        Command::new("help.shortcuts", "Keyboard shortcuts", Help, "keyboard", "Every keyboard shortcut, filterable", &["Mod+/"]),
        Command::new("help.checkUpdates", "Check for updates", Help, "refresh-cw", "Ask GitHub for a newer GitVolt release (GitVolt's own; nothing about your repositories)", &[]),
    ]);
    out
}

thread_local! {
    static TABLE: Rc<Vec<Command>> = Rc::new(all());
}

pub fn table() -> Rc<Vec<Command>> {
    TABLE.with(Rc::clone)
}

pub fn get(id: &str) -> Option<Command> {
    table().iter().find(|c| c.id == id).cloned()
}

/// Runs `id` if it's usable.
pub fn invoke(id: &str, v: &mut GitVolt, window: &mut Window, cx: &mut Context<GitVolt>) -> bool {
    let Some(c) = get(id).filter(|c| c.usable(v, cx)) else { return false };
    if let Some(run) = c.run {
        run(v, window, cx);
    }
    true
}

/// A row for a menu: its action through the view. Disabled while it isn't ported, or isn't usable.
pub fn row(c: &Command, this: &WeakEntity<GitVolt>, usable: bool) -> MenuRow {
    let (run, weak) = (c.run, this.clone());
    let row = MenuRow::action(c.id, c.label.clone(), c.icon, c.tooltip.clone(), move |window, cx| {
        if let Some(run) = run {
            weak.update(cx, |v, cx| run(v, window, cx)).ok();
        }
    });
    let row = match c.chord() {
        Some(chord) => row.shortcut(chord),
        None => row,
    };
    if c.run.is_none() { row.disabled(menu::NOT_YET) } else if usable { row } else { row.disabled(c.tooltip.clone()) }
}

/// The app menu: File, Edit, View, Repository, Help; Quit last, after a separator.
pub fn hamburger_rows(this: &WeakEntity<GitVolt>, v: &GitVolt, cx: &App) -> Vec<MenuRow> {
    let list = table();
    [
        (Group::File, "File", "file-text", "Repositories, tabs and settings"),
        (Group::Edit, "Edit", "pencil", "Find, filter and the command palette"),
        (Group::View, "View", "layout-panel-left", "Tabs and panels"),
        (Group::Repository, "Repository", "folder-git-2", "Actions on the current repository"),
        (Group::Help, "Help", "circle-help", "About GitVolt"),
    ]
    .into_iter()
    .map(|(group, label, icon, tooltip)| {
        let mut rows = Vec::new();
        for c in list.iter().filter(|c| c.group == group && c.menu && c.id != "file.quit") {
            // What isn't usable now isn't listed, as in the web UI; what isn't ported stays, disabled.
            if c.run.is_none() || c.usable(v, cx) {
                rows.push(row(c, this, true));
            }
        }
        if group == Group::File
            && let Some(quit) = list.iter().find(|c| c.id == "file.quit")
        {
            rows.push(MenuRow::Separator);
            rows.push(row(quit, this, true));
        }
        MenuRow::submenu(format!("menu.{label}"), label, icon, tooltip, rows)
    })
    .collect()
}

impl GitVolt {
    /// The app layer of the key router: a chord that names a usable command runs it.
    pub fn app_key(&mut self, k: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(combo) = combo_of(k) else { return false };
        let typing = crate::ui::input::is_typing(window);
        let Some(c) = table().iter().find(|c| c.usable(self, cx) && c.shortcuts.iter().any(|s| resolve_chord(s) == combo)).cloned() else { return false };
        if c.yields_to_typing && typing {
            return false;
        }
        if let Some(run) = c.run {
            run(self, window, cx);
        }
        true
    }

    /// Copies, and says so.
    pub fn copy_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.into()));
        toast::show("Copied to the clipboard", cx);
    }

    pub fn copy_repo_path(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.tabs.get(self.active).map(|t| t.worktree.clone()) {
            self.copy_text(path, cx);
        }
    }

    /// Opens the tab's worktree in the file manager.
    pub fn open_repo_folder(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.tabs.get(self.active).map(|t| t.worktree.clone()) else { return };
        self.open_folder(path, cx);
    }

    pub fn open_folder(&mut self, path: String, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let Some(repo) = self.tabs.get(self.active).map(|t| t.id) else { return };
        let api = self.api.clone();
        // No request of its own: `OpenIn` finds the containing folder of `<path>/root`, `path` itself.
        let done = self.rt.spawn(async move { api.dispatch(Request::OpenIn { repo, worktree: path, path: "root".into(), line: None, opener: "file-manager".into(), source: None, fallback: None }).await });
        cx.spawn(async move |this, cx| {
            let failure = match done.await {
                Ok(Ok(_)) => None,
                Ok(Err(e)) => Some(e.message),
                Err(e) => Some(e.to_string()),
            };
            if let Some(failure) = failure {
                this.update(cx, |this, cx| this.show_toast(failure, cx)).ok();
            }
        })
        .detach();
    }

    pub fn step_tab(&mut self, by: i64, cx: &mut Context<Self>) {
        let n = self.tabs.len() as i64;
        if n > 0 {
            self.active = (self.active as i64 + by).rem_euclid(n) as usize;
            cx.notify();
        }
    }

    pub fn go_to_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.tabs.len() {
            self.active = ix;
            cx.notify();
        }
    }

    pub fn close_file_or_history(&mut self, cx: &mut Context<Self>) {
        if self.tabs.get(self.active).is_some_and(|t| t.history.is_some()) {
            self.close_history(cx);
        } else {
            self.close_file(cx);
        }
    }

    pub fn go_to_wip(&mut self, cx: &mut Context<Self>) {
        self.select(0, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn key(s: &'static str) -> Keystroke {
        Keystroke::parse(s).unwrap()
    }

    #[test]
    fn a_key_goes_by_the_web_uis_name() {
        assert_eq!(combo_of(&key("ctrl-shift-t")).as_deref(), Some("Ctrl+Shift+T"));
        assert_eq!(combo_of(&key("ctrl-alt-f")).as_deref(), Some("Ctrl+Alt+F"));
        assert_eq!(combo_of(&key("f7")).as_deref(), Some("F7"));
        assert_eq!(combo_of(&key("shift-f8")).as_deref(), Some("Shift+F8"));
        assert_eq!(combo_of(&key("alt-1")).as_deref(), Some("Alt+1"));
        assert_eq!(combo_of(&key("ctrl-tab")).as_deref(), Some("Ctrl+Tab"));
        assert_eq!(combo_of(&key("ctrl-,")).as_deref(), Some("Ctrl+,"));
        assert_eq!(combo_of(&key("a")), None);
        assert_eq!(combo_of(&key("down")), None);
        assert_eq!(combo_of(&key("alt-a")), None);
    }

    #[test]
    fn every_id_is_unique_and_every_chord_resolves() {
        let list = all();
        let mut ids: Vec<&str> = list.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), list.len());
        assert_eq!(resolve_chord("Mod+Shift+T"), "Ctrl+Shift+T");
        assert_eq!(resolve_chord("Alt+1"), "Alt+1");
    }
}
