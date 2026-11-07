//! Settings › Accounts (`AccountsSection.tsx`, `HostCombobox.tsx`, `accountChoice.ts`): the profile's forge accounts, any number per host. A token goes
//! from its field to the core's request and nowhere else: not in the state here, a log, or anything a script's `dump` says.

use super::text::{bare_host, detect_host_kind, effective_kind, forge_name, kind_of, mark_icon};
use crate::app::{GitVolt, core_repo, icon};
use crate::theme;
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::{self, arm, tooltip::tip};
use gitvolt_core::api::Request;
use gitvolt_core::forge::accounts::{AccountStatus, ForgeAccount};
use gitvolt_core::settings::RepoSettings;
use gitvolt_core::forge::types::TokenStorage;
use gitvolt_core::forge::ForgeKind;
use gitvolt_core::redact::Secret;
use gpui::prelude::*;
use gpui::*;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

pub const FILE_WARNING: &str = "Stored in a file, not the system keyring: install or start a Secret Service (GNOME Keyring, KWallet), then restart GitVolt to secure it.";
const KINDS: [(ForgeKind, &str); 3] = [(ForgeKind::GitLab, "GitLab"), (ForgeKind::GitHub, "GitHub"), (ForgeKind::Forgejo, "Forgejo")];
const CLASSIC_HELP: &str = "Opens GitHub's classic token page with the repo and read:org scopes selected. Classic tokens don't expire unless you set a date.";
/// GitHub's answer when an organization caps fine-grained token lifetimes.
const FINE_GRAINED_REFUSED: &str = "forbids access via a fine-grained";

fn token_help(k: ForgeKind) -> &'static str {
    match k {
        ForgeKind::GitLab => "Opens your profile's token page with the api scope selected. Copy the token, then paste it here.",
        ForgeKind::GitHub => "Opens GitHub's fine-grained token page with the permissions GitVolt needs selected. Some organizations cap these tokens' lifetime (e.g. 90 days); use a classic token for those.",
        ForgeKind::Forgejo => "Opens the new-token form in your Applications settings. Generate a token with the read:user, write:repository and write:issue scopes, then paste it here.",
    }
}

/// Untyped, with no suggestion of its kind, the host a forge picked starts with (GitLab: none).
fn default_host(k: ForgeKind) -> Option<&'static str> {
    match k {
        ForgeKind::GitHub => Some("github.com"),
        ForgeKind::Forgejo => Some("codeberg.org"),
        ForgeKind::GitLab => None,
    }
}

/// A host worth asking which forge it runs: a name with a dot, and maybe a port.
pub fn askable(host: &str) -> bool {
    let (name, port) = host.split_once(':').map_or((host, None), |(a, b)| (a, Some(b)));
    if port.is_some_and(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return false;
    }
    let mut parts = name.split('.');
    let labelled = |l: &str| !l.is_empty() && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    parts.clone().count() >= 2 && parts.all(labelled)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub account: ForgeAccount,
    pub status: AccountStatus,
}

fn hhmm(unix: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(unix, 0).single().map_or(String::new(), |t| t.format("%H:%M").to_string())
}

/// What an account's last requests said, in plain words; `None` when all is well.
pub fn status_text(s: &AccountStatus) -> Option<String> {
    match s {
        AccountStatus::Ok => None,
        AccountStatus::RateLimited { until } => Some(format!("Rate limited until {}", hhmm(*until))),
        AccountStatus::AuthFailed { message } | AccountStatus::Unreachable { message } => Some(message.clone()),
        AccountStatus::TokenMissing => Some("Token missing: replace it to reconnect".to_string()),
    }
}

/// The status chip: `(colour's name, words)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
    Err,
}

pub fn status_chip(s: &AccountStatus) -> (Tone, String) {
    match s {
        AccountStatus::Ok => (Tone::Ok, "Connected".into()),
        AccountStatus::RateLimited { until } => (Tone::Warn, format!("Rate limited until {}", hhmm(*until))),
        AccountStatus::AuthFailed { .. } => (Tone::Err, "Token rejected".into()),
        AccountStatus::Unreachable { .. } => (Tone::Err, "Unreachable".into()),
        AccountStatus::TokenMissing => (Tone::Err, "Token missing".into()),
    }
}

/// Forgejo's version carries the Gitea it's compatible with (`11.0.1+gitea-1.22.0`).
pub fn shown_version(v: &str) -> &str {
    v.split_once("+gitea-").map_or(v, |x| x.0)
}

/// `hostDefault`: a host's default account, its first: what Automatic uses.
pub fn host_default<'a>(accounts: &'a [AccountView], host: &str) -> Option<&'a AccountView> {
    accounts.iter().find(|a| a.account.host == host)
}

/// `byHost`: Settings › Accounts' order: each host's accounts together, as added, hosts in the order their first account was added.
pub fn by_host(accounts: &[AccountView]) -> Vec<&AccountView> {
    let mut hosts: Vec<&str> = Vec::new();
    for a in accounts {
        if !hosts.contains(&a.account.host.as_str()) {
            hosts.push(&a.account.host);
        }
    }
    hosts.into_iter().flat_map(|h| accounts.iter().filter(move |a| a.account.host == h)).collect()
}

/// `RepoAccounts`: the accounts a repository can use: those on the hosts its remotes map to.
#[derive(Debug, Clone, Default)]
pub struct RepoAccounts {
    pub hosts: Vec<String>,
    pub accounts: Vec<AccountView>,
}

/// `repoAccountsOf`: each remote's `accountId` names an account on the host the core matched, port or not.
pub fn repo_accounts_of(all: &[AccountView], projects: &super::store::RepoProjects) -> RepoAccounts {
    let mut hosts: Vec<String> = Vec::new();
    for r in &projects.remotes {
        if let Some(a) = all.iter().find(|a| Some(&a.account.id) == r.account_id.as_ref())
            && !hosts.contains(&a.account.host)
        {
            hosts.push(a.account.host.clone());
        }
    }
    RepoAccounts { accounts: all.iter().filter(|a| hosts.contains(&a.account.host)).cloned().collect(), hosts }
}

/// `removalEffects`: what removing `a` changes beyond itself, for its armed Remove: the host's default moving to the next account, and the
/// repositories that chose it going back to Automatic. Empty when nothing else changes.
pub fn removal_effects(all: &[AccountView], a: &ForgeAccount, repos: &BTreeMap<String, RepoSettings>) -> String {
    let others: Vec<&AccountView> = all.iter().filter(|v| v.account.host == a.host && v.account.id != a.id).collect();
    let Some(next) = others.first() else { return String::new() };
    let mut parts: Vec<String> = Vec::new();
    if host_default(all, &a.host).is_some_and(|d| d.account.id == a.id) {
        parts.push(format!("Automatic then uses @{}", next.account.user.username));
    }
    match repos.values().filter(|r| r.forge_account.as_deref() == Some(a.id.as_str())).count() {
        0 => {}
        1 => parts.push("the repository that chose it goes back to Automatic".to_string()),
        n => parts.push(format!("{n} repositories that chose it go back to Automatic")),
    }
    let text = parts.join("; ");
    let mut chars = text.chars();
    chars.next().map_or(String::new(), |c| format!("{}{}.", c.to_uppercase(), chars.as_str()))
}

/// What Settings › Repository › Forge account has to choose from.
#[derive(Debug, Clone, Default)]
pub struct RepoSeen {
    pub all: Vec<AccountView>,
    pub mine: RepoAccounts,
}

/// `useForgeAccountShown`: it's a choice where the repository's hosts have two accounts or more, or one was chosen, to go back to Automatic.
pub fn forge_account_shown(seen: Option<&RepoSeen>, chosen: Option<&str>) -> bool {
    seen.map_or(0, |s| s.mine.accounts.len()) > 1 || chosen.is_some()
}

/// `ForgeAccountControl`'s options: `("", Automatic, naming the default it uses)`, then each account, then a chosen one that's gone.
pub fn forge_account_options(seen: Option<&RepoSeen>, chosen: Option<&str>) -> Vec<(String, String)> {
    let empty = RepoSeen::default();
    let seen = seen.unwrap_or(&empty);
    let several = seen.mine.hosts.len() > 1;
    let name = |a: &AccountView| if several { format!("@{} · {}", a.account.user.username, a.account.host) } else { format!("@{}", a.account.user.username) };
    let defaults: Vec<String> = seen.mine.hosts.iter().filter_map(|h| host_default(&seen.mine.accounts, h)).map(name).collect();
    let mut out = vec![(String::new(), if defaults.is_empty() { "Automatic".to_string() } else { format!("Automatic ({})", defaults.join(", ")) })];
    out.extend(seen.mine.accounts.iter().map(|a| (a.account.id.clone(), name(a))));
    if let Some(c) = chosen.filter(|c| !out.iter().any(|o| o.0 == *c)) {
        // Removed since, or on a host none of the repository's remotes is on now.
        let label = seen.all.iter().find(|a| a.account.id == c).map_or("Removed account".to_string(), |a| format!("@{} · {} (no remote there)", a.account.user.username, a.account.host));
        out.push((c.to_string(), label));
    }
    out
}

pub struct Form {
    /// Replacing this account's token, else adding.
    pub replacing: Option<ForgeAccount>,
    pub host: Entity<TextInput>,
    pub token: Entity<TextInput>,
    /// What's typed; untyped, the host follows the forge picked.
    pub typed: Option<String>,
    pub picked: Option<ForgeKind>,
    pub asked: Option<(String, String)>,
    pub asking: Option<String>,
    pub opening: bool,
    pub classic: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub list_open: bool,
    pub cursor: usize,
    /// A click on Create token or Add, waiting for the asked host's answer.
    pub after_ask: Option<AfterAsk>,
    pub focused_once: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterAsk {
    OpenPage,
    Submit,
}

#[derive(Default)]
pub struct AccountsUi {
    pub list: Option<Vec<AccountView>>,
    pub error: Option<String>,
    pub form: Option<Form>,
    pub asked: bool,
    /// Tab stops of the buttons, by what they're for.
    pub stops: HashMap<String, FocusHandle>,
    /// `+ Add account` takes the keyboard back when the form closes.
    pub refocus_add: bool,
    /// What the last add did when it wasn't a new row ("Updated @ada's token"), until the next action.
    pub notice: Option<String>,
    /// By the core's repository: the last answer answers at once, so the row doesn't come and go each time the page opens.
    pub repo_seen: HashMap<u32, RepoSeen>,
    /// The repository read since the accounts last changed.
    pub repo_asked: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub host: String,
    pub source: String,
}

impl GitVolt {
    fn accounts_reload(&mut self, keep_error: bool, cx: &mut Context<Self>) {
        self.forge_ask::<Vec<AccountView>>(Request::ForgeAccounts, cx, move |v, answer, _| {
            let ui = &mut v.settings_ui.accounts;
            match answer {
                Ok(list) => {
                    ui.list = Some(list);
                    if !keep_error {
                        ui.error = None;
                    }
                }
                Err(e) => ui.error = Some(e),
            }
        });
    }

    /// The page shows: its list, read again.
    pub fn accounts_page_shown(&mut self, cx: &mut Context<Self>) {
        if !self.settings_ui.accounts.asked {
            self.settings_ui.accounts.asked = true;
            self.accounts_reload(false, cx);
        }
    }

    /// An account was added or removed: avatars and pollers ask again (what the core answers depends on the accounts).
    pub fn accounts_changed(&mut self, cx: &mut Context<Self>) {
        self.accounts_reload(false, cx);
        self.reset_avatars(cx);
        self.forge_accounts_changed(cx);
        self.settings_ui.accounts.repo_asked = None;
    }

    /// `useRepoAccounts`: the profile's accounts and those the active repository can use, read once while its page shows.
    pub fn repo_accounts_wanted(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.tabs.get(self.active).filter(|t| !t.is_open_tab()).map(|t| core_repo(t.id)) else { return };
        if self.settings_ui.accounts.repo_asked == Some(repo) || cfg!(test) {
            return;
        }
        self.settings_ui.accounts.repo_asked = Some(repo);
        let (api, rt) = (self.api.clone(), self.rt.clone());
        cx.spawn(async move |this, cx| {
            let all = super::ask::<Vec<AccountView>>(api.clone(), rt.clone(), Request::ForgeAccounts).await;
            let projects = super::ask::<super::store::RepoProjects>(api, rt, Request::ForgeRepoProjects { repo, refresh: false }).await;
            let (Ok(all), Ok(projects)) = (all, projects) else { return };
            let seen = RepoSeen { mine: repo_accounts_of(&all, &projects), all };
            this.update(cx, |v, cx| {
                v.settings_ui.accounts.repo_seen.insert(repo, seen);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// What the active repository has to choose from, as last read.
    pub fn repo_accounts_seen(&self) -> Option<&RepoSeen> {
        let t = self.tabs.get(self.active).filter(|t| !t.is_open_tab())?;
        self.settings_ui.accounts.repo_seen.get(&core_repo(t.id))
    }

    /// The repository's chosen account, once it's saved: which account asks the forge changed, avatars too (`repoField`'s `after`).
    pub fn forge_account_chosen(&mut self, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let body = crate::write::call("saveProfile", serde_json::json!({ "profile": self.profile }));
        let api = self.api.clone();
        let saved = self.rt.spawn(async move {
            if let Ok(req) = crate::write::request_of(body) {
                api.dispatch(req).await.ok();
            }
        });
        cx.spawn(async move |this, cx| {
            saved.await.ok();
            this.update(cx, |v, cx| {
                v.reset_avatars(cx);
                v.forge_accounts_changed(cx);
                v.settings_ui.accounts.repo_asked = None;
                v.repo_accounts_wanted(cx);
            })
            .ok();
        })
        .detach();
    }

    pub fn accounts_open_form(&mut self, replacing: Option<ForgeAccount>, cx: &mut Context<Self>) {
        self.settings_ui.accounts.notice = None;
        let host_text = replacing.as_ref().map(|r| r.host.clone());
        let host = cx.new(|cx| TextInput::new(cx).placeholder("gitlab.example.com").claim_escape().claim_arrows().enter_submits());
        let token = cx.new(|cx| TextInput::new(cx).placeholder("Paste the personal access token").secret(true).claim_escape().enter_submits());
        if let Some(h) = &host_text {
            host.update(cx, |i, cx| {
                i.set_text(h, cx);
                i.set_disabled(true, cx);
            });
        }
        cx.subscribe(&host, |this, input, e: &InputEvent, cx| {
            let Some(form) = this.settings_ui.accounts.form.as_mut() else { return };
            match e {
                InputEvent::Changed => {
                    form.typed = Some(input.read(cx).text().to_string());
                    form.picked = None;
                    form.list_open = true;
                    form.cursor = 0;
                }
                InputEvent::Focused => form.list_open = true,
                InputEvent::Blurred => {
                    form.list_open = false;
                    this.accounts_ask(None, cx);
                }
                InputEvent::Arrow { down, .. } => {
                    let shown = this.accounts_shown_suggestions(cx).len();
                    let Some(form) = this.settings_ui.accounts.form.as_mut() else { return };
                    if *down {
                        form.list_open = true;
                        form.cursor = (form.cursor + 1).min(shown.saturating_sub(1));
                    } else {
                        form.cursor = form.cursor.saturating_sub(1);
                    }
                }
                InputEvent::Submit { .. } => {
                    let shown = this.accounts_shown_suggestions(cx);
                    let Some(form) = this.settings_ui.accounts.form.as_mut() else { return };
                    if form.list_open && shown.get(form.cursor).is_some() {
                        let h = shown[form.cursor].host.clone();
                        form.typed = Some(h);
                        form.picked = None;
                        form.list_open = false;
                    } else {
                        this.accounts_submit(cx);
                    }
                }
                InputEvent::Escape => this.accounts_escape(cx),
            }
            cx.notify();
        })
        .detach();
        cx.subscribe(&token, |this, _, e: &InputEvent, cx| {
            match e {
                InputEvent::Submit { .. } => this.accounts_submit(cx),
                InputEvent::Escape => this.accounts_escape(cx),
                _ => {}
            }
            cx.notify();
        })
        .detach();
        let picked = replacing.as_ref().map(|r| r.kind);
        self.settings_ui.accounts.form = Some(Form { replacing, host, token, typed: host_text, picked, asked: None, asking: None, opening: false, classic: false, busy: false, error: None, list_open: false, cursor: 0, after_ask: None, focused_once: false });
        cx.notify();
    }

    fn accounts_close_form(&mut self, cx: &mut Context<Self>) {
        self.settings_ui.accounts.notice = None;
        if let Some(form) = self.settings_ui.accounts.form.take() {
            form.token.update(cx, |i, cx| i.set_text("", cx));
        }
        self.settings_ui.accounts.refocus_add = true;
        cx.notify();
    }

    /// Esc: the host list first, then the panel, before the Settings dialog sees it.
    fn accounts_escape(&mut self, cx: &mut Context<Self>) {
        let shown = !self.accounts_shown_suggestions(cx).is_empty();
        if let Some(form) = self.settings_ui.accounts.form.as_mut()
            && form.list_open
            && shown
        {
            form.list_open = false;
            return;
        }
        self.accounts_close_form(cx);
    }

    /// The open repository's forge hosts, each with where it came from; a host with an account stays: another user's token adds one.
    fn accounts_suggestions(&self) -> Vec<Suggestion> {
        let Some(tab) = self.tabs.get(self.active).filter(|t| !t.is_open_tab()) else { return Vec::new() };
        let overrides = &self.profile["hostOverrides"];
        let mut out: Vec<Suggestion> = Vec::new();
        for r in tab.info["remotes"].as_array().into_iter().flatten() {
            let Some(host) = r["host"].as_str().filter(|h| !h.is_empty()) else { continue };
            if out.iter().any(|s| s.host == host) || effective_kind(host, r["hostKind"].as_str().unwrap_or("generic"), overrides) == "generic" {
                continue;
            }
            let name = r["name"].as_str().unwrap_or_default();
            out.push(Suggestion { host: host.to_string(), source: format!("{name} · {}", tab.repo_name) });
        }
        out
    }

    /// The host the form holds: what's typed, else the forge picked's first suggestion or its default.
    fn accounts_host(&self) -> String {
        let Some(form) = self.settings_ui.accounts.form.as_ref() else { return String::new() };
        if let Some(t) = &form.typed {
            return t.clone();
        }
        let suggestions = self.accounts_suggestions();
        let overrides = &self.profile["hostOverrides"];
        match form.picked {
            None => suggestions.first().map_or(String::new(), |s| s.host.clone()),
            Some(picked) => suggestions
                .iter()
                .find(|s| kind_of(effective_kind(&s.host, detect_host_kind(&s.host), overrides)) == Some(picked))
                .map(|s| s.host.clone())
                .or_else(|| default_host(picked).map(str::to_string))
                .unwrap_or_default(),
        }
    }

    /// What the list shows (`HostCombobox`): a value that is itself a suggestion shows them all, to switch to another.
    fn accounts_shown_suggestions(&self, _cx: &App) -> Vec<Suggestion> {
        let all = self.accounts_suggestions();
        let value = self.accounts_host();
        if all.iter().any(|s| s.host == value) {
            return if all.len() == 1 { Vec::new() } else { all };
        }
        let q = value.trim().to_lowercase();
        all.into_iter().filter(|s| q.is_empty() || s.host.to_lowercase().contains(&q)).collect()
    }

    /// `existing`: the accounts the host has: another user's token adds one, the same user's replaces theirs (the core knows once it's read the token).
    fn accounts_existing(&self, bare: &str) -> Vec<ForgeAccount> {
        let Some(form) = self.settings_ui.accounts.form.as_ref() else { return Vec::new() };
        if form.replacing.is_some() || bare.is_empty() {
            return Vec::new();
        }
        self.settings_ui.accounts.list.iter().flatten().filter(|a| a.account.host == bare).map(|a| a.account.clone()).collect()
    }

    /// The forge the form means now: picked, else what the host's name or its answer says, else GitLab.
    fn accounts_kind(&self) -> ForgeKind {
        let Some(form) = self.settings_ui.accounts.form.as_ref() else { return ForgeKind::GitLab };
        if let Some(k) = form.picked {
            return k;
        }
        let bare = bare_host(&self.accounts_host());
        let overrides = &self.profile["hostOverrides"];
        let by_name = if bare.is_empty() { "generic" } else { effective_kind(&bare, detect_host_kind(&bare), overrides) };
        if let Some(k) = kind_of(by_name) {
            return k;
        }
        // The forge an account on this host already names.
        if let Some(a) = self.accounts_existing(&bare).first() {
            return a.kind;
        }
        form.asked.as_ref().filter(|a| a.0 == bare).and_then(|a| kind_of(&a.1)).unwrap_or(ForgeKind::GitLab)
    }

    fn accounts_needs_ask(&self) -> bool {
        let Some(form) = self.settings_ui.accounts.form.as_ref() else { return false };
        let bare = bare_host(&self.accounts_host());
        let overrides = &self.profile["hostOverrides"];
        form.replacing.is_none()
            && form.picked.is_none()
            && self.accounts_existing(&bare).is_empty()
            && effective_kind(&bare, detect_host_kind(&bare), overrides) == "generic"
            && askable(&bare)
            && form.asked.as_ref().is_none_or(|a| a.0 != bare)
    }

    /// A host its name doesn't place is asked which forge it runs, no token, once it's settled: when the field is left, or
    /// when Create token or Add is clicked. Never while typing, so a half-typed host is never asked. `then` goes on after.
    fn accounts_ask(&mut self, then: Option<AfterAsk>, cx: &mut Context<Self>) {
        if !self.accounts_needs_ask() {
            if let Some(after) = then {
                self.accounts_continue(after, cx);
            }
            return;
        }
        let bare = bare_host(&self.accounts_host());
        let Some(form) = self.settings_ui.accounts.form.as_mut() else { return };
        if then.is_some() {
            form.after_ask = then;
        }
        if form.asking.as_deref() == Some(bare.as_str()) {
            return;
        }
        form.asking = Some(bare.clone());
        let host = bare.clone();
        self.forge_ask::<String>(Request::DetectForge { host: bare }, cx, move |v, answer, cx| {
            let Some(form) = v.settings_ui.accounts.form.as_mut() else { return };
            // Another host was asked since: this answer is stale.
            if form.asking.as_deref() != Some(host.as_str()) {
                return;
            }
            form.asking = None;
            form.asked = Some((host.clone(), answer.unwrap_or_else(|_| "generic".to_string())));
            let after = form.after_ask.take();
            // The host changed while it was asked: the click was for another host.
            if let Some(after) = after
                && bare_host(&v.accounts_host()) == host
            {
                v.accounts_continue(after, cx);
            }
        });
    }

    fn accounts_continue(&mut self, after: AfterAsk, cx: &mut Context<Self>) {
        match after {
            AfterAsk::OpenPage => {
                let (host, kind) = (self.accounts_host(), self.accounts_kind());
                let classic = self.settings_ui.accounts.form.as_ref().is_some_and(|f| f.classic) && kind == ForgeKind::GitHub;
                self.forge_ask::<String>(Request::ForgeTokenPage { host, kind, classic }, cx, |v, answer, cx| {
                    let ok = answer.map(|url| {
                        let job_api = v.api.clone();
                        v.rt.spawn(async move { job_api.dispatch(Request::OpenUrl { url }).await });
                    });
                    if let Some(form) = v.settings_ui.accounts.form.as_mut() {
                        form.opening = false;
                        if let Err(e) = ok {
                            form.error = Some(e);
                        }
                    }
                    let _ = cx;
                });
            }
            AfterAsk::Submit => {
                let (host, kind) = (self.accounts_host(), self.accounts_kind());
                let Some(form) = self.settings_ui.accounts.form.as_ref() else { return };
                let text = form.token.read(cx).text().to_string();
                let account = form.replacing.as_ref().map(|a| a.id.clone());
                self.forge_ask::<AccountView>(Request::AddForgeAccount { host, kind, token: Secret::new(text), account }, cx, |v, answer, cx| {
                    match answer {
                        Ok(view) => {
                            if let Some(form) = v.settings_ui.accounts.form.as_mut() {
                                form.busy = false;
                            }
                            // A token for a user who has an account replaced theirs: say so, as no row is added.
                            let updated = v.settings_ui.accounts.list.iter().flatten().any(|a| a.account.id == view.account.id);
                            v.accounts_close_form(cx);
                            v.accounts_changed(cx);
                            if updated {
                                v.settings_ui.accounts.notice = Some(format!("Updated @{}'s token", view.account.user.username));
                            }
                        }
                        Err(e) => {
                            if let Some(form) = v.settings_ui.accounts.form.as_mut() {
                                form.busy = false;
                                form.error = Some(e);
                            }
                        }
                    }
                });
            }
        }
    }

    fn accounts_submit(&mut self, cx: &mut Context<Self>) {
        let host = self.accounts_host();
        let Some(form) = self.settings_ui.accounts.form.as_mut() else { return };
        if host.is_empty() || form.busy || form.token.read(cx).text().trim().is_empty() {
            return;
        }
        form.busy = true;
        form.error = None;
        self.accounts_ask(Some(AfterAsk::Submit), cx);
    }

    fn accounts_open_page(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.settings_ui.accounts.form.as_mut() else { return };
        if form.opening {
            return;
        }
        form.opening = true;
        self.accounts_ask(Some(AfterAsk::OpenPage), cx);
    }

    fn accounts_remove(&mut self, account: ForgeAccount, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_ui.accounts.notice = None;
        let this = cx.weak_entity();
        let (host, id) = (account.host.clone(), account.id.clone());
        let who = format!("@{} on {host}", account.user.username);
        let repos: BTreeMap<String, RepoSettings> = serde_json::from_value(self.profile["repos"].clone()).unwrap_or_default();
        let effects = removal_effects(self.settings_ui.accounts.list.as_deref().unwrap_or_default(), &account, &repos);
        let mut req = arm::ArmRequest::danger(format!("Click again to remove {who}"), format!("Remove {who}?"), "Remove account");
        req.body = format!("{}Its token is deleted from this computer. Revoke it on {host} too if you no longer need it.", if effects.is_empty() { String::new() } else { format!("{effects} ") }).into();
        req.caption = (!effects.is_empty()).then(|| effects.into());
        let key = format!("account-remove:{id}");
        arm::request(
            req,
            arm::Origin::control(key),
            move |answer, _, cx| {
                if !answer.ok {
                    return;
                }
                this.update(cx, |v, cx| {
                    v.forge_ask::<serde_json::Value>(Request::RemoveForgeAccount { host, account: Some(id) }, cx, |v, answer, cx| match answer {
                        Ok(_) => {
                            // The button that had the keyboard is gone: the dialog's Esc still has to reach it.
                            v.focus_next_frame = v.settings_ui.focus.clone();
                            v.accounts_changed(cx);
                        }
                        // The list is read again, but the error stays to be read.
                        Err(e) => {
                            v.settings_ui.accounts.error = Some(e);
                            v.accounts_reload(true, cx);
                        }
                    });
                })
                .ok();
            },
            window,
            cx,
        );
    }

    fn accounts_stop(&mut self, key: &str, cx: &mut Context<Self>) -> FocusHandle {
        self.settings_ui.accounts.stops.entry(key.to_string()).or_insert_with(|| cx.focus_handle().tab_stop(true)).clone()
    }
}

/// `.accounts-box button`.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    Plain,
    Primary,
    Ghost,
    Danger,
}

fn account_button(id: impl Into<ElementId>, label: impl Into<SharedString>, look: Look, enabled: bool) -> Stateful<Div> {
    let base = div().id(id).flex().flex_none().items_center().justify_center().h(px(30.)).px(px(12.)).rounded(px(5.)).border_1().whitespace_nowrap().child(label.into());
    let base = match look {
        Look::Plain => base.border_color(theme::section_border()).bg(theme::panel_bg()).text_color(theme::text_selected()),
        Look::Primary => base.border_color(theme::blue()).bg(theme::blue()).text_color(theme::text_on_accent()),
        Look::Ghost => base.border_color(transparent_black()).text_color(theme::text_secondary()),
        Look::Danger => base.border_color(transparent_black()).text_color(theme::red()),
    };
    if enabled {
        base.cursor_pointer().when(look != Look::Primary, |d| d.hover(|d| d.bg(theme::control_hover_bg()))).when(look == Look::Primary, |d| d.hover(|d| d.bg(theme::control_hover_bg())))
    } else {
        base.opacity(0.45)
    }
}

fn chip(text: String, color: Option<Hsla>) -> Div {
    let line = color.unwrap_or(theme::section_border());
    let fg = color.unwrap_or(theme::text_secondary());
    div().flex().flex_none().items_center().h(px(19.)).px(px(7.)).rounded(px(10.)).border_1().border_color(line).text_size(px(11.)).text_color(fg).child(text)
}

fn field_label(text: &'static str) -> Div {
    div().w(px(90.)).flex_none().flex().justify_end().text_color(theme::text_secondary()).child(text)
}

fn field_row(label: &'static str, control: impl IntoElement) -> Div {
    div().flex().items_center().gap(px(12.)).child(field_label(label)).child(div().flex_1().min_w_0().child(control))
}

fn help_row(child: impl IntoElement) -> Div {
    div().flex().mt(px(6.)).child(div().w(px(102.)).flex_none()).child(div().flex_1().min_w_0().child(child))
}

fn field_box(id: &'static str, input: Entity<TextInput>, disabled: bool, window: &Window, cx: &App) -> Stateful<Div> {
    let focused = !disabled && input.read(cx).is_focused(window);
    div()
        .id(id)
        .flex()
        .items_center()
        .h(px(32.))
        .px(px(10.))
        .rounded(px(5.))
        .border_1()
        .border_color(if focused { theme::blue() } else { theme::section_border() })
        .bg(theme::app_bg())
        .text_color(theme::text_selected())
        .when(focused, |d| d.shadow(vec![BoxShadow { color: theme::blue(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.), inset: false }]))
        .child(input)
}

fn seg_button(id: impl Into<ElementId>, label: &'static str, on: bool, first: bool, run: impl Fn(&mut GitVolt, &mut Context<GitVolt>) + 'static, stop: FocusHandle, cx: &mut Context<GitVolt>) -> Stateful<Div> {
    let weak = cx.weak_entity();
    let on_key = weak.clone();
    let run = std::rc::Rc::new(run);
    let run2 = run.clone();
    let el = div()
        .id(id)
        .flex()
        .items_center()
        .h(px(28.))
        .px(px(14.))
        .when(!first, |d| d.border_l_1().border_color(theme::section_border()))
        .text_color(if on { theme::text_selected() } else { theme::text_secondary() })
        .when(on, |d| d.bg(theme::panel_bg1()))
        .cursor_pointer()
        .hover(|d| d.bg(theme::control_hover_bg()))
        .child(label)
        .on_click(cx.listener(move |this, _, _, cx| {
            run(this, cx);
            cx.notify();
        }));
    keyboard_stop(el, &stop, move |_, cx| {
        let r = run2.clone();
        on_key
            .update(cx, |v, cx| {
                r(v, cx);
                cx.notify();
            })
            .ok();
    })
}

/// `tabbable`, its outline only when the keyboard brought it there (`:focus-visible`).
fn keyboard_stop(el: Stateful<Div>, focus: &FocusHandle, run: impl Fn(&mut Window, &mut App) + 'static) -> Stateful<Div> {
    el.track_focus(focus).focus_visible(|s| s.border_color(theme::blue())).on_key_down(move |e: &KeyDownEvent, window, cx| {
        let m = &e.keystroke.modifiers;
        if matches!(e.keystroke.key.as_str(), "enter" | "space") && !m.control && !m.alt && !m.shift && !m.platform {
            cx.stop_propagation();
            run(window, cx);
        }
    })
}

/// A button that Tab reaches; Enter and Space run it.
fn stop_button(key: impl Into<SharedString>, el: Stateful<Div>, stop: FocusHandle, enabled: bool, run: impl Fn(&mut GitVolt, &mut Window, &mut Context<GitVolt>) + 'static, cx: &mut Context<GitVolt>) -> AnyElement {
    let weak = cx.weak_entity();
    let run = std::rc::Rc::new(run);
    let run2 = run.clone();
    let el = el.when(enabled, |d| {
        d.on_click(cx.listener(move |this, _, window, cx| {
            run(this, window, cx);
            cx.notify();
        }))
    });
    let el = keyboard_stop(el, &stop, move |window, cx| {
        if !enabled {
            return;
        }
        let r = run2.clone();
        weak.update(cx, |v, cx| {
            r(v, window, cx);
            cx.notify();
        })
        .ok();
    });
    ui::anchor(key, el).into_any_element()
}

fn account_row(v: &AccountView, is_default: bool, this: &mut GitVolt, cx: &mut Context<GitVolt>) -> Stateful<Div> {
    let a = &v.account;
    let note = status_text(&v.status);
    let (tone, label) = status_chip(&v.status);
    let file = a.storage == TokenStorage::File;
    let version = a.version.as_deref().map_or(String::new(), |x| format!(" {}", shown_version(x)));
    let colour = match tone {
        Tone::Ok => theme::green(),
        Tone::Warn => theme::orange(),
        Tone::Err => theme::red(),
    };
    let mark = match a.kind {
        ForgeKind::GitLab => theme::forge_gitlab(),
        ForgeKind::GitHub => theme::forge_github(),
        ForgeKind::Forgejo => theme::forge_forgejo(),
    };
    let id = a.id.clone();
    let replace_stop = this.accounts_stop(&format!("replace:{id}"), cx);
    let remove_stop = this.accounts_stop(&format!("remove:{id}"), cx);
    let replace = {
        let account = a.clone();
        stop_button(format!("account-replace:{id}"), account_button(ElementId::Name(format!("account-replace:{id}").into()), "Replace token", if tone == Tone::Err { Look::Plain } else { Look::Ghost }, true), replace_stop, true, move |v, _, cx| v.accounts_open_form(Some(account.clone()), cx), cx)
    };
    let remove_key = format!("account-remove:{id}");
    let remove = {
        let account = a.clone();
        stop_button(remove_key.clone(), account_button(ElementId::Name(remove_key.clone().into()), "Remove", Look::Danger, true), remove_stop, true, move |v, window, cx| v.accounts_remove(account.clone(), window, cx), cx)
    };
    let default_for = format!("Default for {}", a.host);
    div()
        .id(ElementId::Name(format!("account:{id}").into()))
        .flex()
        .items_start()
        .gap(px(12.))
        .px(px(12.))
        .py(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme::section_border())
        .bg(theme::panel_bg())
        .child(div().flex().flex_none().items_center().justify_center().size(px(32.)).rounded(px(7.)).bg(mark).child(icon(mark_icon(a.kind), 18., theme::text_on_accent())))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(div().flex().flex_wrap().gap(px(4.)).child(div().font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).child(a.user.name.clone())).child(div().text_color(theme::text_secondary()).child(format!("@{}", a.user.username))))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x(px(10.))
                        .gap_y(px(4.))
                        .text_size(px(12.))
                        .text_color(theme::text_secondary())
                        .child(format!("{} · {}{version}", a.host, forge_name(a.kind)))
                        .child(chip(format!("● {label}"), Some(colour)))
                        .child(chip(if file { "File — not secure".to_string() } else { "System keyring".to_string() }, file.then(theme::orange)))
                        // Only where there's a choice: the host has another account.
                        .when(is_default, |d| d.child(tip("Automatic uses it, in Settings › Repository › Forge account", ui::anchor(format!("account-default:{id}"), chip(default_for, None))))),
                )
                .when(file, |d| d.child(div().mt(px(4.)).text_size(px(12.)).text_color(theme::orange()).child(FILE_WARNING)))
                .when_some(note.filter(|_| tone == Tone::Err), |d, n| d.child(div().mt(px(4.)).text_size(px(12.)).text_color(theme::red()).child(n))),
        )
        .child(div().flex().flex_none().gap(px(4.)).child(replace).child(remove))
}

fn add_form(this: &mut GitVolt, window: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let host = this.accounts_host();
    let bare = bare_host(&host);
    let existing = this.accounts_existing(&bare);
    let kind = this.accounts_kind();
    let shown = this.accounts_shown_suggestions(cx);
    let asking_now = this.accounts_needs_ask() && this.settings_ui.accounts.form.as_ref().is_some_and(|f| f.asking.as_deref() == Some(bare.as_str()));
    let Some(form) = this.settings_ui.accounts.form.as_mut() else { return div().into_any_element() };
    let (host_input, token_input) = (form.host.clone(), form.token.clone());
    if form.typed.is_none() && host_input.read(cx).text() != host {
        let h = host.clone();
        host_input.update(cx, |i, cx| i.set_text(&h, cx));
    }
    if !std::mem::replace(&mut form.focused_once, true) {
        let first = if form.replacing.is_some() || !host.is_empty() { &token_input } else { &host_input };
        this.focus_next_frame = Some(first.read(cx).focus.clone());
        cx.notify();
    }
    let Some(form) = this.settings_ui.accounts.form.as_ref() else { return div().into_any_element() };
    let replacing = form.replacing.clone();
    let (picked, classic, busy, error, cursor) = (form.picked, form.classic, form.busy, form.error.clone(), form.cursor);
    let list_open = form.list_open && !shown.is_empty();
    host_input.update(cx, |i, _| i.set_popup_keys(false));
    let token_empty = token_input.read(cx).text().trim().is_empty();
    let title = match &replacing {
        Some(a) => format!("Replace the token for @{} on {}", a.user.username, a.host),
        None => "Add a forge account".to_string(),
    };
    // The host box, its list floating so opening it shifts nothing.
    let mut host_box = div().relative().child(crate::ui::anchor("account-host", field_box("account-host", host_input, replacing.is_some(), window, cx)));
    if list_open {
        let mut list = div().id("host-combo-list").occlude().absolute().left_0().right_0().top(px(36.)).p(px(4.)).bg(theme::panel_bg1()).border_1().border_color(theme::section_border()).rounded(px(6.)).shadow(vec![BoxShadow { color: theme::shadow_modal(), offset: point(px(0.), px(6.)), blur_radius: px(18.), spread_radius: px(0.), inset: false }]);
        for (i, s) in shown.iter().enumerate() {
            let h = s.host.clone();
            list = list.child(
                div()
                    .id(ElementId::Name(format!("host-opt:{}", s.host).into()))
                    .flex()
                    .justify_between()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .when(i == cursor, |d| d.bg(theme::panel_bg()))
                    .child(s.host.clone())
                    .child(div().text_size(px(11.)).text_color(theme::text_secondary()).child(s.source.clone()))
                    .on_mouse_down(MouseButton::Left, {
                        let h = h.clone();
                        cx.listener(move |v, _, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            if let Some(f) = v.settings_ui.accounts.form.as_mut() {
                                f.typed = Some(h.clone());
                                f.picked = None;
                                f.list_open = false;
                                let input = f.host.clone();
                                let h = h.clone();
                                input.update(cx, |i, cx| i.set_text(&h, cx));
                            }
                            cx.notify();
                        })
                    }),
            );
        }
        host_box = host_box.child(deferred(list).with_priority(1));
    }
    let mut host_field = div().flex().flex_col().child(field_row("Host", host_box));
    // Its line is kept while empty: the form doesn't move as a host is typed.
    if replacing.is_none() {
        let replace_one = (existing.len() == 1).then(|| existing[0].clone());
        let line = (!existing.is_empty()).then(|| format!("Has {}; another user's token adds an account.", existing.iter().map(|t| format!("@{}", t.user.username)).collect::<Vec<_>>().join(", ")));
        host_field = host_field.child(help_row(
            div()
                .flex()
                .gap(px(4.))
                .h(px(17.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(12.))
                .text_color(theme::text_secondary())
                .children(line)
                .when_some(replace_one, |d, a| {
                    d.child(ui::anchor("account-taken-replace", div().id("account-taken-replace").cursor_pointer().text_color(theme::link()).underline().child(format!("Replace @{}'s token", a.user.username)).on_click(cx.listener(move |v, _, _, cx| v.accounts_open_form(Some(a.clone()), cx)))))
                }),
        ));
    }
    // Forge.
    let mut seg = div().flex().flex_none().border_1().border_color(theme::section_border()).rounded(px(6.)).overflow_hidden();
    for (i, (k, name)) in KINDS.iter().copied().enumerate() {
        let stop = this.accounts_stop(&format!("kind:{name}"), cx);
        seg = seg.child(crate::ui::anchor(format!("account-kind:{name}"), seg_button(ElementId::Name(format!("account-kind:{name}").into()), name, kind == k, i == 0, move |v, _| {
            if let Some(f) = v.settings_ui.accounts.form.as_mut() {
                f.picked = Some(k);
                if f.typed.is_some() && f.replacing.is_none() {
                    // The host stays what was typed.
                }
            }
        }, stop, cx)));
    }
    let note = if picked.is_none() && !host.is_empty() {
        if asking_now { format!("Asking {bare} which forge it runs…") } else { "Detected from the host".to_string() }
    } else {
        String::new()
    };
    let forge_field = field_row("Forge", div().flex().items_center().gap(px(8.)).child(seg).child(div().text_color(theme::text_secondary()).child(note)));
    // Token type: GitHub's.
    let token_type = (kind == ForgeKind::GitHub).then(|| {
        let mut seg = div().flex().flex_none().border_1().border_color(theme::section_border()).rounded(px(6.)).overflow_hidden();
        for (i, (c, name)) in [(false, "Fine-grained"), (true, "Classic")].into_iter().enumerate() {
            let stop = this.accounts_stop(&format!("token-type:{name}"), cx);
            seg = seg.child(seg_button(ElementId::Name(format!("account-token-type:{name}").into()), name, classic == c, i == 0, move |v, _| {
                if let Some(f) = v.settings_ui.accounts.form.as_mut() {
                    f.classic = c;
                }
            }, stop, cx));
        }
        field_row("Token type", seg)
    });
    // The token.
    let token_stop = this.accounts_stop("create-token", cx);
    let create_enabled = !host.is_empty();
    let create_label = format!("Create token on {} ↗", if host.is_empty() { "the host" } else { &host });
    let create = stop_button("account-create-token", account_button("account-create-token", create_label, Look::Plain, create_enabled), token_stop, create_enabled, |v, _, cx| v.accounts_open_page(cx), cx);
    let create = if host.is_empty() { tip("Enter the host first", create).into_any_element() } else { create };
    let token_field = div().flex().flex_col().child(field_row("Token", div().flex().items_center().gap(px(8.)).child(div().flex_1().min_w_0().child(crate::ui::anchor("account-token", field_box("account-token", token_input, false, window, cx)))).child(create))).child(help_row(div().text_size(px(12.)).text_color(theme::text_secondary()).child(format!(
        // The token page opens in the browser, as whoever is signed in there.
        "{} {}",
        match &replacing {
            Some(a) => format!("Sign in as @{} in your browser first.", a.user.username),
            None => "Sign in as the account's user in your browser first.".to_string(),
        },
        if kind == ForgeKind::GitHub && classic { CLASSIC_HELP } else { token_help(kind) }
    ))));
    // The foot.
    let refused = error.as_deref().is_some_and(|e| kind == ForgeKind::GitHub && !classic && e.contains(FINE_GRAINED_REFUSED));
    let error_line = div()
        .flex()
        .flex_1()
        .min_w_0()
        .flex_wrap()
        .gap(px(4.))
        .min_h(px(18.))
        .text_color(theme::status_deleted())
        .when_some(error, |d, e| d.child(e))
        .when(refused, |d| d.child(div().id("account-use-classic").cursor_pointer().text_color(theme::link()).underline().child("Use a classic token instead").on_click(cx.listener(|v, _, _, cx| {
            if let Some(f) = v.settings_ui.accounts.form.as_mut() {
                f.classic = true;
                f.error = None;
            }
            cx.notify();
        }))));
    let submit_enabled = !host.is_empty() && !token_empty && !busy;
    let submit_label = if busy { if asking_now { "Asking…" } else { "Checking…" } } else if replacing.is_some() { "Replace token" } else { "Add account" };
    let cancel_stop = this.accounts_stop("cancel", cx);
    let submit_stop = this.accounts_stop("submit", cx);
    let foot = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .pt(px(12.))
        .border_t_1()
        .border_color(theme::section_border())
        .child(error_line)
        .child(stop_button("account-cancel", account_button("account-cancel", "Cancel", Look::Ghost, true), cancel_stop, true, |v, _, cx| v.accounts_close_form(cx), cx))
        .child(stop_button("account-submit", account_button("account-submit", submit_label, Look::Primary, submit_enabled).min_w(px(112.)), submit_stop, submit_enabled, |v, _, cx| v.accounts_submit(cx), cx));
    div()
        .id("account-add")
        .flex()
        .flex_col()
        .gap(px(12.))
        .px(px(16.))
        .py(px(14.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme::blue())
        .child(div().font_weight(FontWeight::BOLD).text_color(theme::text_selected()).child(title))
        .child(host_field)
        .child(forge_field)
        .when_some(token_type, |d, t| d.child(t))
        .child(token_field)
        .child(foot)
        .into_any_element()
}

pub fn body(this: &mut GitVolt, window: &mut Window, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    this.accounts_page_shown(cx);
    let list_empty = this.settings_ui.accounts.list.as_ref().is_some_and(Vec::is_empty);
    let has_list = this.settings_ui.accounts.list.as_ref().is_some_and(|l| !l.is_empty());
    let open = this.settings_ui.accounts.form.is_some();
    let add_stop = this.accounts_stop("add", cx);
    if std::mem::take(&mut this.settings_ui.accounts.refocus_add) {
        this.focus_next_frame = Some(add_stop.clone());
    }
    let mut col = div().id("accounts-box").flex().flex_col().gap(px(10.)).min_w_0();
    col = col.child(div().text_color(theme::text_secondary()).child(format!("Connect GitLab, GitHub or Forgejo to see merge requests, pull requests and forks. A host can have several accounts: repositories use its default unless Settings › Repository picks another.{}", if list_empty { " Tokens are kept in the system keyring." } else { "" })));
    if let Some(e) = this.settings_ui.accounts.error.clone() {
        col = col.child(div().text_color(theme::status_deleted()).child(e));
    }
    if let Some(n) = this.settings_ui.accounts.notice.clone() {
        col = col.child(div().text_color(theme::green()).child(n));
    }
    if list_empty && !open {
        let add = stop_button("account-add-first", account_button("account-add-first", "+ Add account", Look::Primary, true), add_stop.clone(), true, |v, _, cx| v.accounts_open_form(None, cx), cx);
        col = col.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(12.))
                .p(px(22.))
                .rounded(px(8.))
                .border_1()
                .border_dashed()
                .border_color(theme::section_border())
                .text_color(theme::text_secondary())
                .child("No accounts yet.")
                .child(add),
        );
    }
    if has_list {
        let list = this.settings_ui.accounts.list.clone().unwrap_or_default();
        let mut rows = div().flex().flex_col().gap(px(8.));
        for v in by_host(&list) {
            let several = list.iter().any(|o| o.account.host == v.account.host && o.account.id != v.account.id);
            let is_default = several && host_default(&list, &v.account.host).is_some_and(|d| d.account.id == v.account.id);
            rows = rows.child(account_row(v, is_default, this, cx));
        }
        col = col.child(rows);
    }
    if open {
        col = col.child(add_form(this, window, cx));
    }
    if has_list && !open {
        col = col.child(div().flex().child(stop_button("account-add-more", account_button("account-add-more", "+ Add account", Look::Plain, true), add_stop, true, |v, _, cx| v.accounts_open_form(None, cx), cx)));
    }
    vec![col.mt(px(0.)).into_any_element()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_host_is_asked_only_when_it_looks_settled() {
        assert!(askable("git.example.dev"));
        assert!(askable("git.example.dev:3000"));
        assert!(!askable("git"));
        assert!(!askable("git."));
        assert!(!askable(""));
        assert!(!askable("git.example.dev:"));
        assert!(!askable("Git.example.dev"));
    }

    fn view(id: &str, host: &str, username: &str) -> AccountView {
        serde_json::from_value(serde_json::json!({
            "account": { "id": id, "host": host, "kind": "gitlab", "user": { "id": 1, "username": username, "name": username }, "storage": "keyring" },
            "status": { "kind": "ok" },
        }))
        .unwrap()
    }

    const H: &str = "gitlab.example.com";

    fn all() -> Vec<AccountView> {
        vec![view(H, H, "ada"), view("github.com", "github.com", "octocat"), view("gitlab.example.com#8", H, "grace"), view("gitlab.example.com:8443", "gitlab.example.com:8443", "linus")]
    }

    fn ids(list: &[&AccountView]) -> Vec<String> {
        list.iter().map(|a| a.account.id.clone()).collect()
    }

    #[test]
    fn a_hosts_default_is_its_first_and_rows_group_by_host() {
        let all = all();
        assert_eq!(host_default(&all, H).unwrap().account.user.username, "ada");
        assert!(host_default(&all, "code.example.org").is_none());
        assert_eq!(ids(&by_host(&all)), [H, "gitlab.example.com#8", "github.com", "gitlab.example.com:8443"]);
    }

    fn projects(account_ids: &[Option<&str>]) -> crate::forge::store::RepoProjects {
        let remotes: Vec<serde_json::Value> = account_ids.iter().enumerate().map(|(i, id)| serde_json::json!({ "remote": format!("r{i}"), "host": H, "path": "g/p", "account": id.map(|_| "gitlab"), "accountId": id, "project": null, "error": null })).collect();
        serde_json::from_value(serde_json::json!({ "remotes": remotes, "target": null, "targetChosen": false })).unwrap()
    }

    #[test]
    fn a_repositorys_accounts_are_those_on_the_hosts_its_remotes_map_to_port_or_not() {
        let all = all();
        let mine = repo_accounts_of(&all, &projects(&[Some("gitlab.example.com#8"), None]));
        assert_eq!(mine.hosts, [H]);
        assert_eq!(mine.accounts.iter().map(|a| a.account.id.as_str()).collect::<Vec<_>>(), [H, "gitlab.example.com#8"]);
        // An SSH remote (no port) the core mapped to the ported host's account.
        let mine = repo_accounts_of(&all, &projects(&[Some("gitlab.example.com:8443")]));
        assert_eq!(mine.hosts, ["gitlab.example.com:8443"]);
        assert!(repo_accounts_of(&all, &projects(&[None])).accounts.is_empty());
    }

    #[test]
    fn removing_an_account_says_when_automatic_moves_on_and_how_many_repositories_chose_it() {
        let all = all();
        let repos: BTreeMap<String, RepoSettings> = serde_json::from_value(serde_json::json!({
            "/a": { "forgeAccount": H }, "/b": { "forgeAccount": H }, "/c": { "forgeAccount": "gitlab.example.com#8" },
        }))
        .unwrap();
        assert_eq!(removal_effects(&all, &all[0].account, &repos), "Automatic then uses @grace; 2 repositories that chose it go back to Automatic.");
        assert_eq!(removal_effects(&all, &all[2].account, &repos), "The repository that chose it goes back to Automatic.");
        assert_eq!(removal_effects(&all, &all[2].account, &BTreeMap::new()), "");
        assert_eq!(removal_effects(&all, &all[1].account, &repos), "", "the host has no other account: nothing else to say");
    }

    #[test]
    fn a_repositorys_choice_shows_where_theres_one_automatic_naming_the_default() {
        let everyone = all();
        let seen = RepoSeen { mine: repo_accounts_of(&everyone, &projects(&[Some(H)])), all: everyone.clone() };
        assert!(forge_account_shown(Some(&seen), None));
        let labels = |chosen| forge_account_options(Some(&seen), chosen).into_iter().map(|o| o.1).collect::<Vec<_>>();
        assert_eq!(labels(None), ["Automatic (@ada)", "@ada", "@grace"]);
        assert_eq!(labels(Some("gitlab.example.com#9")).last().unwrap(), "Removed account");
        assert_eq!(labels(Some("github.com")).last().unwrap(), "@octocat · github.com (no remote there)");
        // One account on the host: no choice, unless one was chosen, to go back to Automatic.
        let one = RepoSeen { mine: RepoAccounts { hosts: vec!["github.com".into()], accounts: vec![everyone[1].clone()] }, all: everyone.clone() };
        assert!(!forge_account_shown(Some(&one), None));
        assert!(forge_account_shown(Some(&one), Some("github.com")));
        assert!(!forge_account_shown(None, None));
        // Two hosts: each name says its host.
        let two = RepoSeen { mine: RepoAccounts { hosts: vec![H.into(), "github.com".into()], accounts: vec![everyone[0].clone(), everyone[1].clone()] }, all: everyone };
        assert_eq!(forge_account_options(Some(&two), None)[0].1, "Automatic (@ada · gitlab.example.com, @octocat · github.com)");
    }

    #[test]
    fn a_chip_says_how_the_account_stands() {
        assert_eq!(status_chip(&AccountStatus::Ok), (Tone::Ok, "Connected".to_string()));
        assert_eq!(status_chip(&AccountStatus::TokenMissing).0, Tone::Err);
        assert_eq!(status_text(&AccountStatus::Ok), None);
        assert_eq!(status_text(&AccountStatus::Unreachable { message: "down".into() }).as_deref(), Some("down"));
        assert_eq!(shown_version("11.0.1+gitea-1.22.0"), "11.0.1");
    }
}
