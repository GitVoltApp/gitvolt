//! The registry (`registry.ts`) and the pages' settings (`pages/*.tsx`, and each feature's own).

use super::frame::{self, bound, input_box, plain_button, set_bound};
use crate::app::{GitVolt, icon};
use crate::theme;
use crate::ui::arm;
use crate::ui::controls::{self, OnSelect};
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::rc::Rc;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    App,
    Profile,
    Repo,
}

pub type BodyFn = fn(&mut GitVolt, &mut Window, &mut Context<GitVolt>) -> Vec<AnyElement>;
pub type ControlFn = fn(&mut GitVolt, &'static Def, &mut Window, &mut Context<GitVolt>) -> AnyElement;
pub type After = fn(&mut GitVolt, &mut Context<GitVolt>);

pub struct Page {
    pub id: &'static str,
    pub label: &'static str,
    pub scope: Scope,
    pub icon: &'static str,
    pub summary: fn(&GitVolt) -> String,
    pub body: Option<BodyFn>,
}

/// A setting's value where it's stored, its default, and what a change has to do.
pub enum Binding {
    None,
    App(&'static str, fn() -> Value, Option<After>),
    Profile(&'static str, fn() -> Value),
    Repo(&'static str, fn() -> Value, Option<After>),
    /// Kept beside the sizes, where the web UI uses localStorage.
    Local(&'static str, fn() -> Value),
}

#[derive(Clone, Copy)]
pub struct NumberSpec {
    pub get: fn(&GitVolt) -> f64,
    pub set: fn(&mut GitVolt, f64, &mut Context<GitVolt>),
    pub min: f64,
    pub max: f64,
    pub step: f64,
    /// The value an empty box stands for.
    pub blank: Option<f64>,
    /// Where + from the empty box starts.
    pub blank_start: Option<fn(&GitVolt) -> f64>,
    pub decimals: u8,
    pub placeholder: &'static str,
}

impl NumberSpec {
    pub fn shown(&self, v: f64) -> String {
        if Some(v) == self.blank {
            return String::new();
        }
        if self.decimals == 0 { format!("{v}") } else { format!("{}", (v * 10f64.powi(self.decimals as i32)).round() / 10f64.powi(self.decimals as i32)) }
    }

    fn round(&self, n: f64) -> f64 {
        if self.decimals == 0 { n.round() } else { (n * 10f64.powi(self.decimals as i32)).round() / 10f64.powi(self.decimals as i32) }
    }

    fn clamp(&self, n: f64) -> f64 {
        n.clamp(self.min, self.max)
    }

    /// What's typed, applied on Enter or blur; `None`: it isn't a number, so it snaps back.
    pub fn committed(&self, text: &str, _now: f64) -> Option<f64> {
        let t = text.trim();
        if t.is_empty() {
            return self.blank;
        }
        let n = self.round(t.parse::<f64>().ok().filter(|n| n.is_finite())?);
        Some(if Some(n) == self.blank { n } else { self.clamp(n) })
    }

    pub fn stepped(&self, typed: &str, now: f64, dir: f64, times: f64, start: f64) -> f64 {
        let t = typed.trim();
        let from: Option<f64> = if t.is_empty() { if self.blank.is_some() { None } else { Some(now) } } else { t.parse::<f64>().ok().filter(|n| n.is_finite()).or(if Some(now) == self.blank { None } else { Some(now) }) };
        let blank_below = self.blank.is_some_and(|b| b < self.min);
        let Some(from) = from else {
            return if dir > 0. { self.clamp(self.round(start)) } else { self.blank.unwrap_or(now) };
        };
        if dir < 0. && blank_below && from <= self.min {
            return self.blank.unwrap_or(now);
        }
        self.clamp(self.round(from + dir * self.step * times))
    }

    pub fn at_limit(&self, dir: f64, now: f64) -> bool {
        let is_blank = Some(now) == self.blank;
        if dir > 0. { !is_blank && now >= self.max } else if is_blank { true } else { !self.blank.is_some_and(|b| b < self.min) && now <= self.min }
    }
}

pub enum Ctl {
    Switch,
    Select(fn(&GitVolt) -> Vec<(Value, String)>),
    Number(NumberSpec),
    Custom(ControlFn),
    OpenPage,
}

pub struct Def {
    pub id: &'static str,
    pub label: &'static str,
    pub page: &'static str,
    pub group: &'static str,
    pub keywords: &'static str,
    pub description: String,
    pub control: Ctl,
    pub binding: Binding,
}

pub fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or(String::new(), |f| f.to_lowercase().chain(c).collect())
}

fn fixed(text: &'static str) -> fn(&GitVolt) -> String {
    // A page's line that names nothing goes through this table: fn pointers can't capture.
    match text {
        "general" => |_| "How GitVolt behaves day to day.".into(),
        "appearance" => |_| "Density and fonts. The theme and colours are under Theme, the editor font under Diffs and editors, the zoom in the status bar.".into(),
        "theme" => |_| "The colours of the app, the commit graph and the editors. Density and fonts are under Appearance.".into(),
        "graph" => |_| "How the commit graph loads and draws. Branch visibility is in Branch presets.".into(),
        "branch-presets" => |_| "Views of the graph that hide, collapse or fold branches. The button beside the branch filter switches between them.".into(),
        "diffs" => |_| "The diff and file views, the working copy and the merge editor.".into(),
        "fetch" => |_| "Background fetch and what a push sends.".into(),
        "keyboard" => |_| "Every keyboard shortcut.".into(),
        "advanced" => |_| "Logs and resetting GitVolt's settings.".into(),
        "git-identity" => |_| "Who your commits say they're from in this profile.".into(),
        "editor-folders" => |_| "Where repositories open from and which editor opens files.".into(),
        "accounts" => |_| "Forge accounts for merge requests, pull requests, forks and avatars.".into(),
        "hosts" => |_| "Which forge a self-hosted host runs.".into(),
        _ => |_| String::new(),
    }
}

/// In nav order within each scope.
pub fn pages() -> &'static [Page] {
    static PAGES: OnceLock<Vec<Page>> = OnceLock::new();
    PAGES.get_or_init(|| {
        vec![
            Page { id: "general", label: "General", scope: Scope::App, icon: "settings", summary: fixed("general"), body: None },
            Page { id: "appearance", label: "Appearance", scope: Scope::App, icon: "type", summary: fixed("appearance"), body: None },
            Page { id: "theme", label: "Theme", scope: Scope::App, icon: "palette", summary: fixed("theme"), body: Some(super::themes::theme_body) },
            Page { id: "graph", label: "Graph", scope: Scope::App, icon: "git-branch", summary: fixed("graph"), body: None },
            Page { id: "branch-presets", label: "Branch presets", scope: Scope::App, icon: "git-fork", summary: fixed("branch-presets"), body: Some(crate::presets::settings_page::body) },
            Page { id: "diffs", label: "Diffs and editors", scope: Scope::App, icon: "file-diff", summary: fixed("diffs"), body: None },
            Page { id: "fetch", label: "Fetch and push", scope: Scope::App, icon: "refresh-cw", summary: fixed("fetch"), body: None },
            Page { id: "keyboard", label: "Keyboard", scope: Scope::App, icon: "keyboard", summary: fixed("keyboard"), body: Some(crate::shortcuts::settings_body) },
            Page { id: "advanced", label: "Advanced", scope: Scope::App, icon: "terminal", summary: fixed("advanced"), body: None },
            Page { id: "git-identity", label: "Git identity", scope: Scope::Profile, icon: "user-round", summary: fixed("git-identity"), body: None },
            Page { id: "editor-folders", label: "Editor and folders", scope: Scope::Profile, icon: "folder-open", summary: fixed("editor-folders"), body: None },
            Page { id: "accounts", label: "Accounts", scope: Scope::Profile, icon: "key-round", summary: fixed("accounts"), body: Some(accounts_body) },
            Page { id: "hosts", label: "Hosts", scope: Scope::Profile, icon: "server", summary: fixed("hosts"), body: None },
            Page { id: "repository", label: "Repository", scope: Scope::Repo, icon: "folder-git-2", summary: |v| format!("Settings for {} only.", repo_name(v)), body: None },
            Page { id: "file-colours", label: "File colours", scope: Scope::Repo, icon: "palette", summary: |v| match v.tabs.get(v.active).filter(|t| !t.is_open_tab()) { Some(t) => format!("For {}. Top to bottom: a file takes the first scope that matches it.", t.name), None => "Top to bottom: a file takes the first scope that matches it.".into() }, body: Some(crate::filecolours::settings_page::body) },
        ]
    })
}

pub fn page(id: &str) -> Option<&'static Page> {
    pages().iter().find(|p| p.id == id)
}

fn repo_name(v: &GitVolt) -> String {
    v.tabs.get(v.active).filter(|t| !t.is_open_tab()).map_or("this repository".to_string(), |t| t.name.to_string())
}

macro_rules! num_field {
    ($key:literal, $default:literal) => {
        |v: &GitVolt| v.settings[$key].as_f64().unwrap_or($default)
    };
}

pub fn defs() -> &'static [Def] {
    static DEFS: OnceLock<Vec<Def>> = OnceLock::new();
    DEFS.get_or_init(build_defs)
}

pub fn def(id: &str) -> Option<&'static Def> {
    defs().iter().find(|d| d.id == id)
}

fn app(key: &'static str, default: fn() -> Value) -> Binding {
    Binding::App(key, default, None)
}

fn build_defs() -> Vec<Def> {
    let chord = |c: &str| crate::commands::display_chord(c);
    let d = |id, label, page, group, keywords, description: &str, control, binding| Def { id, label, page, group, keywords, description: description.to_string(), control, binding };
    let refresh_graph: After = |v, cx| v.reload_active(cx);
    let avatars: After = |v, cx| v.reset_avatars(cx);
    let presets: After = |v, cx| v.reload_active(cx);
    let mut out = vec![
        d("advanceAfterStage", "After staging a file, show the next one", "general", "Staging", "stage unstage discard next file advance move diff", "Staging, unstaging or discarding the open file opens the next one in its list. Off keeps showing it.", Ctl::Switch, Binding::Local("advanceAfterStage", || json!(true))),
        d("updateCheck", "Check for updates automatically", "general", "Updates", "update upgrade new version release github check daily", "Asks GitHub for new releases at startup and once a day; nothing about you is sent. Off: only Help › Check for updates asks.", Ctl::Switch, app("updateCheck", || json!(true))),
        d("updatePrereleases", "Include pre-releases", "general", "Updates", "update beta rc alpha pre-release preview", "Also offers release candidates and betas.", Ctl::Switch, app("updatePrereleases", || json!(false))),
        d("density", "Density", "appearance", "Density", "compact standard comfortable row height spacing padding", "Row height and padding of the graph and the file lists.", Ctl::Custom(density_control), Binding::Local("density", || json!("standard"))),
        d("uiFontFamily", "UI font", "appearance", "Fonts", "font family face typeface interface chrome menus lists graph dialogs installed picker custom list css", "The menus, lists, graph, settings and dialogs. Default: the system's UI font, which also draws what a chosen font lacks. \"Custom font list…\" takes a CSS font-family list. Its size follows the zoom and the density.", Ctl::Custom(super::fonts::font_control), app("uiFontFamily", || json!(""))),
        d("readingFontFamily", "Reading font", "appearance", "Fonts", "font family face typeface markdown prose rendered readme description comment serif installed picker custom list css", "Rendered Markdown: the Markdown view, the rendered diff, MR/PR descriptions and comments. Default: the UI font. Code in it keeps the editor font.", Ctl::Custom(super::fonts::font_control), app("readingFontFamily", || json!(""))),
        d("readingFontSize", "Reading text size", "appearance", "Fonts", "font size markdown prose rendered px zoom", &format!("Rendered Markdown in the Markdown view and the rendered diff, 8 to 32 px; descriptions and comments show it 1 px smaller. Over it, {} and {} change it, {} resets it.", chord("Mod+="), chord("Mod+-"), chord("Mod+0")), Ctl::Number(NumberSpec { get: num_field!("readingFontSize", 13.), set: |v, n, cx| v.set_settings(|s| s["readingFontSize"] = json!(n as u32), cx), min: 8., max: 32., step: 1., blank: None, blank_start: None, decimals: 0, placeholder: "" }), app("readingFontSize", || json!(13))),
        d("readingLineHeight", "Reading line height", "appearance", "Fonts", "markdown prose line spacing leading", "A multiple of the text size, 1.2 to 2.4.", Ctl::Number(NumberSpec { get: num_field!("readingLineHeight", 1.55), set: |v, n, cx| v.set_settings(|s| s["readingLineHeight"] = json!(n), cx), min: 1.2, max: 2.4, step: 0.05, blank: None, blank_start: None, decimals: 2, placeholder: "" }), app("readingLineHeight", || json!(1.55))),
        d("readingLineWidth", "Reading line width", "appearance", "Fonts", "markdown prose measure column max width characters ch", "The longest line in the Markdown view and the rendered diff, in characters, 20 to 240. Empty: none (their usual 880 px column); + from there starts at 80.", Ctl::Number(NumberSpec { get: num_field!("readingLineWidth", 0.), set: |v, n, cx| v.set_settings(|s| s["readingLineWidth"] = json!(n as u32), cx), min: 20., max: 240., step: 1., blank: Some(0.), blank_start: Some(|_| 80.), decimals: 0, placeholder: "None" }), app("readingLineWidth", || json!(0))),
        d("systemTitleBar", "Use the system title bar", "appearance", "Window", "title bar titlebar window frame decorations buttons minimize maximize close native gnome desktop windows macos traffic lights", "The desktop's own title bar and window frame instead of GitVolt's, with ☰ back in the tab bar. Takes effect when GitVolt restarts. An X11 session always has the desktop's.", Ctl::Switch, app("systemTitleBar", || json!(false))),
        d("theme", "Theme", "theme", "Theme", "colour color scheme dark light default monokai darcula dracula one dark solarized github nord", "The colours of the app and the commit graph.", Ctl::Custom(super::themes::theme_select), app("theme", || json!("default-dark"))),
        d("graphColors", "Graph lanes", "theme", "Graph lanes", "colour color branch lane graph palette override reorder", "The commit graph's ten lane colours. Changing one on a built-in theme makes a custom copy of it.", Ctl::OpenPage, Binding::None),
        d("themeColours", "Colours", "theme", "Colours", "colour color token background text accent diff custom", "The theme's colours, each with a picker. Changing one on a built-in theme makes a custom copy of it.", Ctl::OpenPage, Binding::None),
        d("commitLimit", "Commits loaded in the graph", "graph", "History", "window history limit", "Newest first. More is slower on a big history.", Ctl::Number(NumberSpec { get: num_field!("commitLimit", 2000.), set: |v, n, cx| { v.set_settings(|s| s["commitLimit"] = json!(n as u32), cx); v.reload_active(cx); }, min: 1., max: 50_000., step: 100., blank: None, blank_start: None, decimals: 0, placeholder: "" }), Binding::App("commitLimit", || json!(2000), Some(refresh_graph))),
        d("relativeDates", "Show relative dates", "graph", "Dates", "relative ago yesterday humanize", "\"5 minutes ago\", \"yesterday\", \"3 weeks ago\". The exact format below shows on hover.", Ctl::Switch, app("relativeDates", || json!(false))),
        d("dateFormat", "Date format", "graph", "Dates", "time 24h 12h graph column", "The exact date: in the graph, details, compare and find, or on hover under relative dates.", Ctl::Select(|_| vec![(json!("ymd12h"), "2026-09-26 @ 3:14 PM".into()), (json!("ymd24h"), "2026-09-26 15:14".into()), (json!("dmy24h"), "26/09/2026 15:14".into()), (json!("dmydot24h"), "26.09.2026 15:14".into()), (json!("mdy12h"), "09/26/2026 3:14 PM".into())]), app("dateFormat", || json!("ymd12h"))),
        d("gravatar", "From Gravatar", "graph", "Avatars", "avatar privacy images", "Looked up by a hash of the author's email, never the email itself. Off shows initials and makes no request.", Ctl::Switch, Binding::App("gravatar", || json!(true), Some(avatars))),
        d("forgeAvatars", "From your forge accounts", "graph", "Avatars", "avatar gitlab github forge privacy images", "Asks the forge hosting the repository, when you have an account there, so colleagues without Gravatar get their avatar. Sends authors' emails (or names) to that forge. With From Gravatar on, a GitHub noreply address also gets GitHub's public avatar by its user id alone, without an account.", Ctl::Switch, Binding::App("forgeAvatars", || json!(true), Some(avatars))),
        d("collapseHoverOpen", "Open collapsed commits on hover", "graph", "Hidden commits", "branch presets collapsed folded reveal panel hover hidden commits", "Hovering a \"⋮ N more commits\" row or a folded merge opens them over the graph: the dotted lane at once, the rest of the row after a moment. Off: click the row.", Ctl::Switch, app("collapseHoverOpen", || json!(true))),
        d("collapsePlacement", "Collapsed branches", "graph", "Hidden commits", "branch presets collapse placement date order pull together oldest commit", "Keep date order leaves a collapsed branch's oldest commit at its date, a dotted lane running down to it; Pull together moves it up under the ⋮ row.", Ctl::Select(|_| vec![(json!("keepDateOrder"), "Keep date order".into()), (json!("pullTogether"), "Pull together".into())]), Binding::App("collapsePlacement", || json!("keepDateOrder"), Some(presets))),
        d("editorFontSize", "Font size", "diffs", "Editor", "monaco diff editor text size px zoom", &format!("Text in the diffs, file views and editors, 8 to 32 px. Over one, {} and {} change it, {} resets it.", chord("Mod+="), chord("Mod+-"), chord("Mod+0")), Ctl::Number(NumberSpec { get: num_field!("editorFontSize", 13.), set: |v, n, cx| v.set_settings(|s| s["editorFontSize"] = json!(n as u32), cx), min: 8., max: 32., step: 1., blank: None, blank_start: None, decimals: 0, placeholder: "" }), app("editorFontSize", || json!(13))),
        d("editorFontFamily", "Font family", "diffs", "Editor", "monaco editor font face typeface monospace jetbrains mono css installed picker custom list", "Default: JetBrains Mono, which comes with GitVolt. What a chosen font lacks draws in it or another monospace font. \"Custom font list…\" takes a CSS font-family list. The reading and UI fonts are under Appearance.", Ctl::Custom(super::fonts::font_control), app("editorFontFamily", || json!(""))),
        d("editorFontLigatures", "Font ligatures", "diffs", "Editor", "monaco editor font ligature arrows => != calt", "Draws character sequences such as => and != as one symbol, in a font that has them.", Ctl::Switch, app("editorFontLigatures", || json!(false))),
        d("editorLineHeight", "Line height", "diffs", "Editor", "monaco editor line spacing height px", "In px, at least the font size. Empty: automatic, from the font size; + from there starts at the automatic height.", Ctl::Number(NumberSpec { get: num_field!("editorLineHeight", 0.), set: |v, n, cx| v.set_settings(|s| s["editorLineHeight"] = json!(n as u32), cx), min: 8., max: 64., step: 1., blank: Some(0.), blank_start: Some(|v| (v.settings["editorFontSize"].as_f64().unwrap_or(13.) * 1.35).round()), decimals: 0, placeholder: "Auto" }), app("editorLineHeight", || json!(0))),
        d("editorTabSize", "Tab width", "diffs", "Editor", "monaco editor tab size indent columns spaces", "How many columns a tab character takes, 1 to 16. It changes how files look, never what they hold.", Ctl::Number(NumberSpec { get: num_field!("editorTabSize", 4.), set: |v, n, cx| v.set_settings(|s| s["editorTabSize"] = json!(n as u32), cx), min: 1., max: 16., step: 1., blank: None, blank_start: None, decimals: 0, placeholder: "" }), app("editorTabSize", || json!(4))),
        d("editorWhitespace", "Show whitespace", "diffs", "Editor", "monaco editor whitespace spaces tabs dots render invisible", "Marks spaces and tabs: at boundaries leaves out single spaces between words.", Ctl::Select(|_| vec![(json!("none"), "None".into()), (json!("boundary"), "At boundaries".into()), (json!("all"), "Everywhere".into())]), app("editorWhitespace", || json!("none"))),
        d("editorMinimap", "Minimap", "diffs", "Editor", "monaco editor minimap overview scroll", "A small view of the whole file beside the scrollbar.", Ctl::Switch, app("editorMinimap", || json!(true))),
        d("stickyScroll", "Sticky scroll", "diffs", "Editor", "monaco pin scope header", "Pins the enclosing scope's first line at the top. With brace-on-next-line code (common in PHP) it may show only the brace.", Ctl::Switch, app("editorStickyScroll", || json!(false))),
        d("fetchInterval", "Background fetch interval", "fetch", "Fetch", "auto fetch timer minutes off", "Fetches the shown repository on this schedule.", Ctl::Select(|v| {
            let secs = crate::feedback::clamp_fetch_interval(v.settings["fetchIntervalSecs"].as_f64().unwrap_or(60.));
            let mut list: Vec<(Value, String)> = vec![(json!(0), "Off".into()), (json!(60), "Every minute".into()), (json!(300), "Every 5 minutes".into()), (json!(600), "Every 10 minutes".into()), (json!(1800), "Every 30 minutes".into())];
            if !list.iter().any(|o| o.0 == json!(secs)) {
                list.push((json!(secs), format!("Every {} min", (secs as f64 / 60.).round() as u32)));
            }
            list
        }), app("fetchIntervalSecs", || json!(60))),
        d("prune", "Prune deleted remote branches on fetch", "fetch", "Fetch", "prune --prune remote tracking", "Fetch with --prune: remote-tracking branches the remote deleted are removed.", Ctl::Switch, app("prune", || json!(true))),
        d("pushFollowTags", "Push tags with branches", "fetch", "Push", "push follow-tags annotated tags release", "Push with --follow-tags: a branch's annotated tags the remote lacks go with it.", Ctl::Switch, app("pushFollowTags", || json!(true))),
        d("shortcuts", "Keyboard shortcuts", "keyboard", "Shortcuts", "keys hotkeys bindings chord", "Every shortcut, and what it does.", Ctl::OpenPage, Binding::None),
        d("debugLogging", "Debug logging", "advanced", "Logs", "log file verbose diagnostics troubleshooting", "Writes debug-level detail to the log files, from now on. Replaces a RUST_LOG set at launch.", Ctl::Switch, Binding::App("debugLogging", || json!(false), Some(|v, cx| v.send_debug_logging(cx)))),
        d("openLogs", "Logs folder", "advanced", "Logs", "log file folder directory diagnostics troubleshooting", "Where GitVolt writes its log files.", Ctl::Custom(open_logs), Binding::None),
        d("resetAll", "Reset all settings", "advanced", "Reset", "reset defaults restore factory", "Puts every GitVolt setting back to its default. Profile and repository settings and what you made stay.", Ctl::Custom(reset_all), Binding::None),
        d("gitIdentity", "Commits are made as", "git-identity", "Commits", "user name email author committer signing gpg ssh key identity", "Read-only. Set in git config, not in GitVolt.", Ctl::Custom(git_identity), Binding::None),
        d("extraGitconfig", "Extra git config for this profile", "git-identity", "Config", "include.path identity account signing", "A gitconfig file added to every git command of this profile (as include.path): your identity, signing key.", Ctl::Custom(extra_gitconfig), Binding::Profile("extraGitconfig", || Value::Null)),
        d("editor", "Default editor", "editor-folders", "Editor", "vscode phpstorm jetbrains zed sublime open in custom command template", "What Open in uses. Custom runs your own command, with {file}, {line} and {repo} filled in.", Ctl::Custom(editor_picker), Binding::Profile("editor", || Value::Null)),
        d("reposFolder", "Default repos directory", "editor-folders", "Folders", "clone destination scan your repos folder", "Where Open Repository looks for your repositories, and where a clone goes by default.", Ctl::Custom(repos_folder), Binding::Profile("reposFolder", || Value::Null)),
        d("forgeAccounts", "Forge accounts", "accounts", "Accounts", "gitlab github forgejo gitea codeberg token personal access token pat account login keyring merge request pull request forks", "GitLab, GitHub and Forgejo accounts for merge requests, pull requests, forks and avatars.", Ctl::OpenPage, Binding::None),
        d("hostOverrides", "Forge type per host", "hosts", "Hosts", "gitlab github forgejo gitea codeberg self-hosted icon links", "For the remote icons and web links of a self-hosted GitLab, GitHub or Forgejo.", Ctl::Custom(host_rows), Binding::None),
        d("repoEditor", "Editor for this repository", "repository", "Opening", "override open in", "Overrides the profile's default editor here.", Ctl::Custom(editor_picker), Binding::Repo("editor", || Value::Null, None)),
        d("repoPin", "Pinned trunk", "repository", "Graph", "pin trunk main branch left lane", "The branch kept at the graph's left. The pin button in the graph's header sets it too.", Ctl::Custom(pin_control), Binding::Repo("pin", || Value::Null, Some(refresh_graph))),
        d("trunkPatterns", "Trunk branches", "repository", "Graph", "trunk branch presets release patterns glob never hidden collapsed main", "Branch presets never hide or collapse these, beside the detected trunk. Comma-separated globs, like release/*.", Ctl::Custom(trunk_patterns), Binding::Repo("trunkPatterns", || Value::Null, Some(refresh_graph))),
        d("forgeTargetRemote", "Merge requests target", "repository", "Merge requests", "merge request pull request target remote upstream fork project", "The remote whose project merge requests and pull requests go to.", Ctl::Custom(target_remote), Binding::Repo("forgeTargetRemote", || Value::Null, None)),
        d("branchUpdate", "Update an MR's branch by", "repository", "Merge requests", "rebase merge update branch behind pipeline skip ci pull request", "What the merge panel's update button does. It remembers your last choice.", Ctl::Select(|_| vec![(Value::Null, "The forge's default".into()), (json!("rebase"), "Rebase".into()), (json!("rebaseSkipCi"), "Rebase without pipeline".into()), (json!("merge"), "Merge".into())]), Binding::Repo("branchUpdate", || Value::Null, None)),
        d("gitflow", "Git flow", "repository", "Branches", "gitflow git-flow feature release hotfix develop production prefix version tag", "Its production and development branches, and the prefixes of its feature, release and hotfix branches.", Ctl::Custom(gitflow_control), Binding::None),
    ];
    out.extend(crate::presets::settings_page::defs());
    out.extend(crate::filecolours::settings_page::defs());
    out
}

fn density_control(this: &mut GitVolt, _: &'static Def, _: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let now = this.prefs["density"].as_str().unwrap_or("standard").to_string();
    let mut row = div().flex().flex_none().gap(px(16.));
    for (id, label) in [("compact", "Compact"), ("standard", "Standard"), ("comfortable", "Comfortable")] {
        let on = now == id;
        row = row.child(crate::ui::anchor(format!("setting:density:{id}"), radio(id, on, label, cx.listener(move |this, _, _, cx| {
            let def = def("density").expect("density");
            set_bound(this, &def.binding, json!(id), cx);
        }))));
    }
    row.into_any_element()
}

/// `.radio`.
pub fn radio(id: &'static str, on: bool, label: &'static str, click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(6.))
        .cursor_pointer()
        .on_click(click)
        .child(div().flex_none().flex().items_center().justify_center().size(px(13.)).rounded_full().border_1().border_color(if on { theme::blue() } else { theme::text_secondary() }).when(on, |d| d.child(div().size(px(7.)).rounded_full().bg(theme::blue()))))
        .child(label)
}

fn open_logs(_: &mut GitVolt, _: &'static Def, _: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    crate::ui::anchor("setting:openLogs", plain_button("input-openLogs", "Open logs folder", true).on_click(cx.listener(|this, _, _, cx| this.open_logs_folder(cx)))).into_any_element()
}

fn reset_all(_: &mut GitVolt, _: &'static Def, _: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    crate::ui::anchor(
        "setting:resetAll",
        plain_button("input-resetAll", "Reset all settings…", true).text_color(theme::status_deleted()).on_click(cx.listener(|_, _, window, cx| {
            let weak = cx.weak_entity();
            let req = arm::ArmRequest { body: "Every setting in the GitVolt group goes back to its default. Profile and repository settings, custom themes, branch presets and file colours stay.".into(), ..arm::ArmRequest::danger("Click again to reset every GitVolt setting", "Reset GitVolt settings?", "Reset") };
            arm::request(
                req,
                arm::Origin::control("setting:resetAll"),
                move |answer, _, cx| {
                    if answer.ok {
                        weak.update(cx, frame::reset_app_settings).ok();
                    }
                },
                window,
                cx,
            );
        })),
    )
    .into_any_element()
}

fn git_identity(this: &mut GitVolt, _: &'static Def, _: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    this.read_git_identity(cx);
    let tab = this.tabs.get(this.active).filter(|t| !t.is_open_tab());
    let from = match tab {
        Some(t) => format!("From {}'s git config", t.name),
        None => "From your global git config".to_string(),
    };
    let info = this.settings_ui.identity.clone();
    let desc = |t: String| div().text_size(px(12.)).text_color(theme::text_secondary()).child(t);
    let mut col = div().flex().flex_col().items_end().min_w_0();
    if let Some(i) = &info {
        if i["identity"].is_object() {
            let signed = match i["signing"]["format"].as_str() {
                _ if !i["signing"].is_object() => None,
                Some("ssh") => Some("signed (SSH key)"),
                Some("x509") => Some("signed (X.509)"),
                _ => Some("signed (GPG key)"),
            };
            let line = [i["identity"]["email"].as_str(), signed].iter().flatten().copied().collect::<Vec<_>>().join(" · ");
            col = col.child(div().font_weight(FontWeight::BOLD).child(i["identity"]["name"].as_str().unwrap_or_default().to_string())).child(desc(line));
        } else {
            col = col.child(desc("No name and email set: commits will fail until you set user.name and user.email.".into()));
        }
    }
    col.child(desc(format!("{from}, with this profile's extra config applied. Change it there, not here."))).into_any_element()
}

fn extra_gitconfig(this: &mut GitVolt, _: &'static Def, window: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let value = this.profile["extraGitconfig"].as_str().unwrap_or_default().to_string();
    let input = this.settings_field("extraGitconfig", &value, &crate::settings::pages::example_path(".gitconfig-work"), Rc::new(|v, text, cx| {
        let t = text.trim().to_string();
        if t != v.profile["extraGitconfig"].as_str().unwrap_or_default() {
            v.update_profile(move |p| p["extraGitconfig"] = if t.is_empty() { Value::Null } else { json!(t) }, cx);
        }
    }), None, window, cx);
    input_box("input-extraGitconfig", input, false, window, cx).into_any_element()
}

/// `examplePath`.
pub fn example_path(name: &str) -> String {
    format!("{}/{name}", std::env::var("HOME").unwrap_or_else(|_| "~".into()))
}

fn repos_folder(this: &mut GitVolt, _: &'static Def, window: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let value = this.profile["reposFolder"].as_str().unwrap_or_default().to_string();
    let input = this.settings_field("reposFolder", &value, &example_path("repos"), Rc::new(|v, text, cx| {
        let t = text.trim().to_string();
        if t != v.profile["reposFolder"].as_str().unwrap_or_default() {
            v.update_profile(move |p| p["reposFolder"] = if t.is_empty() { Value::Null } else { json!(t) }, cx);
        }
    }), None, window, cx);
    let start = (!value.is_empty()).then_some(value);
    div()
        .flex()
        .flex_1()
        .gap(px(8.))
        .child(input_box("input-reposFolder", input, false, window, cx))
        .child(crate::ui::anchor("setting:reposFolder:choose", plain_button("reposFolder-choose", "Choose…", true).on_click(cx.listener(move |this, _, _, cx| {
            this.pick_folder(start.clone(), |v, dir, cx| {
                if let Some(dir) = dir {
                    v.update_profile(move |p| p["reposFolder"] = json!(dir), cx);
                }
            }, cx);
        }))))
        .into_any_element()
}

/// `EditorPicker.tsx`: Last used (or Same as the profile), a detected editor, or a custom command, checked by the core.
fn editor_picker(this: &mut GitVolt, def: &'static Def, window: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let value = bound(this, &def.binding);
    let inherit = def.id == "repoEditor";
    let openers = this.tabs.get(this.active).and_then(|t| t.open_in.openers.clone());
    if openers.is_none() {
        this.read_settings_openers(cx);
    }
    let editors: Vec<(String, String)> = openers.or_else(|| this.settings_openers.clone()).map_or(Vec::new(), |l| l.iter().filter(|o| o.kind == "editor" && o.id != "custom").map(|o| (o.id.clone(), o.name.clone())).collect());
    let saved_custom = value["kind"] == "custom";
    let (asked, error) = this.settings_ui.editor_custom.get(def.id).cloned().unwrap_or((saved_custom, None));
    let selected = if asked { "custom".to_string() } else if value["kind"] == "opener" { value["id"].as_str().unwrap_or_default().to_string() } else { String::new() };
    let mut options: Vec<(String, SharedString, Option<SharedString>)> = vec![(String::new(), if inherit { "Same as the profile" } else { "Last used" }.into(), None)];
    for (id, name) in &editors {
        options.push((id.clone(), name.clone().into(), None));
    }
    if value["kind"] == "opener" && !editors.iter().any(|e| Some(e.0.as_str()) == value["id"].as_str()) {
        let id = value["id"].as_str().unwrap_or_default();
        options.push((id.to_string(), format!("{id} (not found)").into(), None));
    }
    options.push(("custom".to_string(), "Custom command…".into(), None));
    let weak = cx.weak_entity();
    let on_change: OnSelect<String> = Rc::new(move |v, _, cx| {
        let v = v.clone();
        weak.update(cx, |this, cx| {
            if v == "custom" {
                this.settings_ui.editor_custom.insert(def.id.to_string(), (true, None));
            } else {
                this.settings_ui.editor_custom.insert(def.id.to_string(), (false, None));
                set_bound(this, &def.binding, if v.is_empty() { Value::Null } else { json!({ "kind": "opener", "id": v }) }, cx);
                this.refresh_openers(cx);
            }
            cx.notify();
        })
        .ok();
    });
    let mut col = div().flex().flex_col().gap(px(6.)).flex_1().min_w_0().child(div().flex().child(controls::select(format!("setting:{}", def.id), selected, Rc::new(options), None, on_change, cx)));
    if asked {
        let saved = if saved_custom { value["template"].as_str().unwrap_or_default().to_string() } else { String::new() };
        let key = format!("{}-custom", def.id);
        let input = this.settings_field(&key, &saved, "myeditor --goto {file}:{line}", Rc::new(|_, _, _| {}), Some(Rc::new(move |v, text, cx| v.validate_editor_template(def, text, cx))), window, cx);
        col = col.child(input_box(ElementId::Name(key.into()), input, error.is_some(), window, cx)).child(match error {
            Some(e) => div().text_size(px(12.)).text_color(theme::status_deleted()).child(e),
            None => div().text_size(px(12.)).text_color(theme::text_secondary()).child("{file}, {line} and {repo} are filled in per open."),
        });
    }
    col.into_any_element()
}

fn host_rows(this: &mut GitVolt, _: &'static Def, window: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    const KINDS: [(&str, &str); 4] = [("gitlab", "GitLab"), ("github", "GitHub"), ("forgejo", "Forgejo"), ("generic", "Generic (no forge links)")];
    let name_of = |k: &str| match k {
        "gitlab" => "GitLab",
        "github" => "GitHub",
        "forgejo" => "Forgejo",
        _ => "Generic",
    };
    let mut hosts: Vec<(String, String)> = Vec::new();
    if let Some(r) = this.tabs.get(this.active).and_then(|t| t.info["remotes"].as_array()) {
        for x in r {
            if let Some(h) = x["host"].as_str()
                && !hosts.iter().any(|y| y.0 == h)
            {
                hosts.push((h.to_string(), x["hostKind"].as_str().unwrap_or("generic").to_string()));
            }
        }
    }
    let overrides = this.profile["hostOverrides"].clone();
    if let Some(o) = overrides.as_object() {
        for h in o.keys() {
            if !hosts.iter().any(|y| &y.0 == h) {
                hosts.push((h.clone(), "generic".into()));
            }
        }
    }
    let mut col = div().flex().flex_col().gap(px(8.)).flex_1().min_w_0();
    if hosts.is_empty() {
        col = col.child(div().text_color(theme::text_secondary()).child("No remotes in the open repository. Add a host to set its type for every repository."));
    }
    for (h, kind) in hosts {
        let weak = cx.weak_entity();
        let mut options: Vec<(String, SharedString, Option<SharedString>)> = vec![(String::new(), format!("Detected ({})", name_of(&kind)).into(), None)];
        options.extend(KINDS.iter().map(|(k, l)| ((*k).to_string(), (*l).into(), None)));
        let value = overrides[&h].as_str().unwrap_or_default().to_string();
        let host = h.clone();
        let on_change: OnSelect<String> = Rc::new(move |v, _, cx| {
            let (v, host) = (v.clone(), host.clone());
            weak.update(cx, |this, cx| this.set_host_kind(&host, &v, cx)).ok();
        });
        col = col.child(div().flex().items_center().gap(px(8.)).child(div().flex_1().min_w_0().text_color(theme::text_selected()).child(h.clone())).child(div().flex().flex_none().w(px(190.)).child(controls::select(format!("setting:host:{h}"), value, Rc::new(options), None, on_change, cx))));
    }
    let added = this.settings_field("host-add", "", "git.example.com", Rc::new(|_, _, _| {}), Some(Rc::new(|_, _, cx| cx.notify())), window, cx);
    let typed = added.read(cx).text().trim().to_lowercase();
    let host = {
        let t = typed.split_once("://").map_or(typed.as_str(), |x| x.1);
        t.split('/').next().unwrap_or_default().to_string()
    };
    let weak = cx.weak_entity();
    let on_kind: OnSelect<String> = Rc::new(move |v, _, cx| {
        let v = v.clone();
        weak.update(cx, |this, cx| {
            this.settings_ui.host_kind = v;
            cx.notify();
        })
        .ok();
    });
    let options: Vec<(String, SharedString, Option<SharedString>)> = KINDS.iter().map(|(k, l)| ((*k).to_string(), (*l).into(), None)).collect();
    let kind = this.settings_ui.host_kind.clone();
    let enabled = !host.is_empty();
    col.child(
        div()
            .flex()
            .gap(px(8.))
            .child(input_box("host-add-box", added, false, window, cx))
            .child(div().flex().flex_none().w(px(120.)).child(controls::select("setting:host-add-kind", kind, Rc::new(options), None, on_kind, cx)))
            .child(crate::ui::anchor("setting:host-add", plain_button("host-add-button", "Add", enabled).when(enabled, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    let kind = this.settings_ui.host_kind.clone();
                    this.set_host_kind(&host, &kind, cx);
                    if let Some(f) = this.settings_ui.fields.get_mut("host-add") {
                        f.shown.clear();
                        f.input.clone().update(cx, |i, cx| i.set_text("", cx));
                    }
                }))
            }))),
    )
    .into_any_element()
}

fn pin_key(pin: &Value) -> String {
    match pin["kind"].as_str() {
        Some("ref") => format!("ref:{}", pin["name"].as_str().unwrap_or_default()),
        Some(k) => k.to_string(),
        None => "auto".into(),
    }
}

fn pin_control(this: &mut GitVolt, def: &'static Def, _: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let pin = bound(this, &def.binding);
    let mut options: Vec<(String, SharedString, Option<SharedString>)> = vec![("auto".into(), "Automatic".into(), None), ("off".into(), "Off".into(), None)];
    if let Some(tab) = this.tabs.get(this.active) {
        for b in tab.locals() {
            options.push((format!("ref:{}", b.full_name), b.name.into(), None));
        }
    }
    let key = pin_key(&pin);
    if pin["kind"] == "ref" && !options.iter().any(|o| o.0 == key) {
        let short = pin["name"].as_str().unwrap_or_default().trim_start_matches("refs/remotes/").trim_start_matches("refs/heads/").to_string();
        options.push((key.clone(), short.into(), None));
    }
    let weak = cx.weak_entity();
    let on_change: OnSelect<String> = Rc::new(move |v, _, cx| {
        let value = match v.as_str() {
            "auto" => json!({ "kind": "auto" }),
            "off" => json!({ "kind": "off" }),
            r => json!({ "kind": "ref", "name": &r[4..] }),
        };
        weak.update(cx, |this, cx| set_bound(this, &def.binding, value, cx)).ok();
    });
    div().flex().flex_none().w(px(240.)).child(controls::select("setting:repoPin", key, Rc::new(options), None, on_change, cx)).into_any_element()
}

fn trunk_patterns(this: &mut GitVolt, def: &'static Def, window: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let patterns = bound(this, &def.binding);
    let joined = patterns.as_array().map_or(String::new(), |a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "));
    let input = this.settings_field("trunkPatterns", &joined, "release/*, hotfix/*", Rc::new(move |v, text, cx| {
        let next: Vec<String> = text.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
        let def = def_of("trunkPatterns");
        let now: Vec<String> = bound(v, &def.binding).as_array().map_or(Vec::new(), |a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect());
        if now != next {
            set_bound(v, &def.binding, if next.is_empty() { Value::Null } else { json!(next) }, cx);
        }
    }), None, window, cx);
    let trunk = this.tabs.get(this.active).and_then(|t| t.graph.as_ref()).map_or(Vec::new(), |g| {
        let mut names: Vec<String> = Vec::new();
        for r in &g.pinned_refs {
            let n = r.strip_prefix("refs/heads/").map(str::to_string).or_else(|| r.strip_prefix("refs/remotes/").and_then(|x| x.split_once('/').map(|y| y.1.to_string()))).unwrap_or_else(|| r.to_string());
            if !names.contains(&n) {
                names.push(n);
            }
        }
        names
    });
    let note = if trunk.is_empty() { "Detected: the pinned trunk, or the default branch with the pin off".to_string() } else { format!("Detected: {}", trunk.join(", ")) };
    div().flex().flex_col().items_stretch().gap(px(4.)).flex_1().min_w_0().child(input_box("input-trunkPatterns", input, false, window, cx)).child(div().text_size(px(11.)).text_color(theme::text_secondary()).flex().justify_end().child(note)).into_any_element()
}

fn def_of(id: &str) -> &'static Def {
    def(id).expect("a registered setting")
}

fn target_remote(this: &mut GitVolt, def: &'static Def, _: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let remote = bound(this, &def.binding).as_str().unwrap_or_default().to_string();
    let mut options: Vec<(String, SharedString, Option<SharedString>)> = vec![(String::new(), "Automatic".into(), None)];
    if let Some(r) = this.tabs.get(this.active).and_then(|t| t.info["remotes"].as_array()) {
        for x in r {
            if let Some(n) = x["name"].as_str() {
                options.push((n.to_string(), n.to_string().into(), None));
            }
        }
    }
    if !remote.is_empty() && !options.iter().any(|o| o.0 == remote) {
        options.push((remote.clone(), remote.clone().into(), None));
    }
    let weak = cx.weak_entity();
    let on_change: OnSelect<String> = Rc::new(move |v, _, cx| {
        let value = if v.is_empty() { Value::Null } else { json!(v) };
        weak.update(cx, |this, cx| set_bound(this, &def.binding, value, cx)).ok();
    });
    div().flex().flex_none().w(px(240.)).child(controls::select("setting:forgeTargetRemote", remote, Rc::new(options), None, on_change, cx)).into_any_element()
}

fn gitflow_control(this: &mut GitVolt, _: &'static Def, _: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let has = this.tabs.get(this.active).is_some_and(|t| t.flow().is_some());
    crate::ui::anchor("setting:gitflow", plain_button("input-gitflow", if has { "Git flow settings…" } else { "Set up git flow…" }, true).on_click(cx.listener(move |this, _, window, cx| {
        this.close_settings(window, cx);
        let id = if has { "gitflow.settings" } else { "gitflow.init" };
        crate::commands::invoke(id, this, window, cx);
    })))
    .into_any_element()
}

/// Accounts belong to the forge round: the page holds its place.
fn accounts_body(_: &mut GitVolt, _: &mut Window, _: &mut Context<GitVolt>) -> Vec<AnyElement> {
    vec![div().mt(px(16.)).text_color(theme::text_secondary()).child("Forge accounts come with the merge request views.").child(div().child(icon("key-round", 14., theme::text_disabled()))).into_any_element()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn spec(blank: Option<f64>) -> NumberSpec {
        NumberSpec { get: |_| 0., set: |_, _, _| {}, min: 8., max: 64., step: 1., blank, blank_start: None, decimals: 0, placeholder: "" }
    }

    #[test]
    fn a_number_box_applies_clamped_and_snaps_back() {
        let s = spec(None);
        assert_eq!(s.committed("100", 13.), Some(64.));
        assert_eq!(s.committed("2", 13.), Some(8.));
        assert_eq!(s.committed("abc", 13.), None);
        assert_eq!(s.committed("", 13.), None, "empty with no blank snaps back");
        assert_eq!(spec(Some(0.)).committed("", 13.), Some(0.));
        assert_eq!(spec(Some(0.)).committed("0", 13.), Some(0.), "the blank value itself");
    }

    #[test]
    fn a_step_goes_from_whats_typed_and_through_the_blank() {
        let s = spec(Some(0.));
        assert_eq!(s.stepped("20", 13., 1., 1., 8.), 21.);
        assert_eq!(s.stepped("20", 13., 1., 10., 8.), 30.);
        assert_eq!(s.stepped("8", 8., -1., 1., 8.), 0., "− from the minimum is the blank");
        assert_eq!(s.stepped("", 0., 1., 1., 18.), 18., "+ from the empty box starts at its start");
        assert_eq!(s.stepped("", 0., -1., 1., 8.), 0.);
        assert!(s.at_limit(-1., 0.));
        assert!(!s.at_limit(1., 0.));
        assert!(s.at_limit(1., 64.));
    }

    #[test]
    fn every_setting_names_a_registered_page_and_ids_are_unique() {
        let mut ids: Vec<&str> = defs().iter().map(|d| d.id).collect();
        for d in defs() {
            assert!(page(d.page).is_some(), "{} names {}", d.id, d.page);
        }
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), defs().len());
    }
}
