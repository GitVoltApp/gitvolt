//! Create an MR/PR (`forge/create`): the flyout, its prefill, its resumable drafts, and what comes back. Only Ctrl+Enter and the
//! Create button create: it's an irreversible write on a forge.

use super::mrview::people::{Role, label_pill, limit_tip, max_of, person_chip, rm_button};
use super::search::{self, SearchOption, Source};
use super::text::{forge_name, mr_long_noun, mr_noun, mr_ref};
use crate::app::{GitVolt, core_repo, icon};
use crate::md::field::{FieldEvent, FieldOptions, MarkdownField};
use crate::theme;
use crate::ui::controls;
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::tooltip::tip;
use gitvolt_core::api::Request;
use gitvolt_core::forge::create::CreateContext;
use gitvolt_core::forge::types::{CreateMr, CreateOutcome, CreatePart, ForgeProject, ForgeProjectSettings, ForgeUser, MrTemplate, PartFailure, SourceRef, SquashOption};
use gitvolt_core::forge::ForgeKind;
use gpui::prelude::*;
use gpui::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::rc::Rc;

const NO_FORGE: &str = "No forge account for this repository's remotes: add one in Settings › Accounts";
pub const MAX_MR_DRAFTS: usize = 50;
pub const MR_DRAFT_MAX_AGE_SECS: f64 = 90. * 24. * 60. * 60.;
/// Past this, a browser or the forge may refuse the link.
pub const MAX_URL_LENGTH: usize = 8000;

/// One per repository and source branch, once the user changed something; an untouched prefill is made again on each open.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrDraft {
    pub source_remote: String,
    pub target_remote: String,
    pub target_branch: String,
    pub title: String,
    pub description: String,
    /// The description as the prefill or a template last wrote it.
    pub prefilled: String,
    pub template: Option<String>,
    pub reviewers: Vec<ForgeUser>,
    pub assignees: Vec<ForgeUser>,
    pub labels: Vec<String>,
    pub draft: bool,
    /// GitLab only.
    pub squash: Option<bool>,
    pub delete_source_branch: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub source_remote: String,
    pub target_remote: String,
    pub target_branch: String,
}

impl MrDraft {
    fn route(&self) -> Route {
        Route { source_remote: self.source_remote.clone(), target_remote: self.target_remote.clone(), target_branch: self.target_branch.clone() }
    }

    fn on(mut self, r: &Route) -> Self {
        self.source_remote = r.source_remote.clone();
        self.target_remote = r.target_remote.clone();
        self.target_branch = r.target_branch.clone();
        self
    }
}

pub fn draft_key(repo_path: &str, branch: &str) -> String {
    format!("{repo_path}\0{branch}")
}

/// Only what the flyout needs: no email address is kept.
fn slim(mut d: MrDraft) -> MrDraft {
    for u in d.reviewers.iter_mut().chain(d.assignees.iter_mut()) {
        u.email = None;
    }
    d
}

/// Drops drafts past their age, then the oldest beyond the cap.
pub fn prune_drafts(all: &mut serde_json::Map<String, serde_json::Value>, now: f64) {
    let at = |v: &serde_json::Value| v["savedAt"].as_f64().unwrap_or(now);
    all.retain(|_, v| now - at(v) <= MR_DRAFT_MAX_AGE_SECS);
    while all.len() > MAX_MR_DRAFTS {
        let Some(oldest) = all.iter().min_by(|a, b| at(a.1).total_cmp(&at(b.1))).map(|p| p.0.clone()) else { break };
        all.remove(&oldest);
    }
}

pub fn default_template(templates: &[MrTemplate]) -> Option<&MrTemplate> {
    if templates.len() == 1 {
        return templates.first();
    }
    templates.iter().find(|t| t.name.eq_ignore_ascii_case("default"))
}

/// The first commit's body, a blank line, then the template.
pub fn join_description(body: &str, template: Option<&MrTemplate>) -> String {
    [body.trim(), template.map_or("", |t| t.body.trim())].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("\n\n")
}

pub struct Toggle {
    pub value: bool,
    pub locked: bool,
    pub caption: Option<&'static str>,
}

/// GitLab's squash, from the project.
pub fn squash_toggle(s: &ForgeProjectSettings) -> Toggle {
    match s.squash {
        SquashOption::Always => Toggle { value: true, locked: true, caption: Some("This project always squashes") },
        SquashOption::Never => Toggle { value: false, locked: true, caption: Some("This project doesn't allow squashing") },
        SquashOption::DefaultOn => Toggle { value: true, locked: false, caption: None },
        _ => Toggle { value: false, locked: false, caption: None },
    }
}

/// `pushedAs`: the remote whose name is the longest matching prefix of the push target, and the branch's name there.
pub fn pushed_as(push_target: Option<&str>, remotes: &[String]) -> Option<(String, String)> {
    let t = push_target?;
    let mut names: Vec<&String> = remotes.iter().collect();
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    let r = names.into_iter().find(|n| t.starts_with(&format!("{n}/")))?;
    Some((r.clone(), t[r.len() + 1..].to_string()))
}

/// The branch's name on the source remote: its push target's there, unless that's the target branch itself in the same project.
pub fn source_branch_of(branch: &str, pushed: Option<&(String, String)>, source_remote: &str, target_branch: &str, same_project: bool) -> String {
    match pushed {
        Some((r, b)) if r == source_remote && !(same_project && b == target_branch) => b.clone(),
        _ => branch.to_string(),
    }
}

/// The local commits the remote's branch doesn't have: `None` when they differ by a count not known.
pub fn unpushed_count(local: Option<(&str, Option<&str>, u32)>, remote: Option<(&str, &str)>) -> Option<u32> {
    let (Some((target, upstream, ahead)), Some((r_target, r_full))) = (local, remote) else { return Some(0) };
    if target == r_target {
        return Some(0);
    }
    (upstream == Some(r_full)).then_some(ahead)
}

pub fn default_target(branch: &str, stack: Option<&[String]>, pushed_of: impl Fn(&str) -> Option<String>, project_default: Option<&str>) -> String {
    if let Some(s) = stack
        && let Some(i) = s.iter().position(|b| b == branch)
        && i > 0
    {
        return pushed_of(&s[i - 1]).unwrap_or_else(|| s[i - 1].clone());
    }
    project_default.unwrap_or("main").to_string()
}

pub fn fresh_draft(ctx: &CreateContext, route: &Route) -> MrDraft {
    let tpl = default_template(&ctx.templates);
    let body = ctx.first_commit.as_ref().map_or("", |c| c.body.as_str());
    let description = join_description(body, tpl);
    let gitlab = ctx.project.kind == ForgeKind::GitLab;
    MrDraft {
        source_remote: route.source_remote.clone(),
        target_remote: route.target_remote.clone(),
        target_branch: route.target_branch.clone(),
        title: ctx.first_commit.as_ref().map_or(String::new(), |c| c.summary.clone()),
        prefilled: description.clone(),
        description,
        template: tpl.map(|t| t.path.clone()),
        reviewers: Vec::new(),
        assignees: Vec::new(),
        labels: Vec::new(),
        draft: false,
        squash: gitlab.then(|| squash_toggle(&ctx.settings).value),
        delete_source_branch: gitlab.then_some(ctx.settings.delete_source_branch),
    }
}

/// `tpl` in place of the current template; `edited`: the description isn't what the prefill or a template wrote.
pub fn with_template(d: &MrDraft, ctx: &CreateContext, tpl: Option<&MrTemplate>) -> (MrDraft, bool) {
    let description = join_description(ctx.first_commit.as_ref().map_or("", |c| c.body.as_str()), tpl);
    let edited = d.description != d.prefilled;
    (MrDraft { prefilled: description.clone(), description, template: tpl.map(|t| t.path.clone()), ..d.clone() }, edited)
}

pub fn create_request(d: &MrDraft, ctx: &CreateContext, source_branch: &str) -> CreateMr {
    let gitlab = ctx.project.kind == ForgeKind::GitLab;
    CreateMr {
        source: SourceRef { project: ctx.source_project.clone(), branch: source_branch.to_string() },
        target_branch: d.target_branch.clone(),
        title: d.title.trim().to_string(),
        description: d.description.clone(),
        draft: d.draft,
        reviewers: d.reviewers.iter().map(|u| u.id).collect(),
        assignees: d.assignees.iter().map(|u| u.id).collect(),
        labels: d.labels.clone(),
        squash: if gitlab && !squash_toggle(&ctx.settings).locked { d.squash } else { None },
        delete_source_branch: if gitlab { d.delete_source_branch } else { None },
    }
}

/// Why Create is disabled.
pub fn create_blocked(d: &MrDraft, source_branch: &str, on_remote: bool, source_project: &str, target_project: &str) -> Option<String> {
    if !on_remote {
        return Some(format!("Push {source_branch} to {} first", d.source_remote));
    }
    if source_project == target_project && source_branch == d.target_branch {
        return Some(format!("{source_branch} can't target itself"));
    }
    d.title.trim().is_empty().then(|| "Enter a title".to_string())
}

pub fn part_word(p: &CreatePart) -> &'static str {
    match p {
        CreatePart::Reviewers => "reviewers",
        CreatePart::Assignees => "assignees",
        CreatePart::Labels => "labels",
    }
}

pub fn parts_text(parts: &[CreatePart]) -> String {
    let words: Vec<&str> = parts.iter().map(part_word).collect();
    match words.split_last() {
        None => String::new(),
        Some((last, [])) => last.to_string(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

pub fn failure_text(failed: &[PartFailure]) -> String {
    failed.iter().map(|f| format!("couldn't add {}: {}", part_word(&f.part), f.message.trim_end_matches('.'))).collect::<Vec<_>>().join("; ")
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn segments(s: &str) -> String {
    s.split('/').map(encode).collect::<Vec<_>>().join("/")
}

fn query(pairs: &[(&str, String)]) -> String {
    pairs.iter().filter(|p| !p.1.is_empty()).map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect::<Vec<_>>().join("&")
}

fn marked(title: &str, marks: &[&str]) -> bool {
    let t = title.trim_start().to_lowercase();
    marks.iter().any(|m| t.starts_with(m))
}

/// The forge's own new-MR/PR page, prefilled where its URL takes it: the URL, and whether the description, and more, were left out.
pub fn new_mr_url(target: &ForgeProject, source_project: &str, source_branch: &str, d: &MrDraft) -> (String, bool, bool) {
    let fork = source_project != target.path;
    let build = |description: &str, labels: &[String], title_text: &str| -> String {
        let trimmed = title_text.trim();
        match target.kind {
            ForgeKind::GitLab => {
                let base = if fork && target.web_url.ends_with(&target.path) { format!("{}{}", &target.web_url[..target.web_url.len() - target.path.len()], segments(source_project)) } else { target.web_url.clone() };
                let title = if d.draft && !title_text.is_empty() && !marked(title_text, &["draft:", "[draft]", "(draft)"]) { format!("Draft: {trimmed}") } else { trimmed.to_string() };
                let mut q = vec![("merge_request[source_branch]", source_branch.to_string())];
                if fork {
                    q.push(("merge_request[target_project_id]", target.id.to_string()));
                }
                q.extend([("merge_request[target_branch]", d.target_branch.clone()), ("merge_request[title]", title), ("merge_request[description]", description.to_string())]);
                format!("{base}/-/merge_requests/new?{}", query(&q))
            }
            _ => {
                let owner = source_project.split('/').next().unwrap_or_default();
                let head = if fork { format!("{}:{}", encode(owner), segments(source_branch)) } else { segments(source_branch) };
                let root = format!("{}/compare/{}...{head}", target.web_url, segments(&d.target_branch));
                if target.kind == ForgeKind::Forgejo {
                    let title = if d.draft && !title_text.is_empty() && !marked(title_text, &["wip:", "[wip]"]) { format!("WIP: {trimmed}") } else { trimmed.to_string() };
                    return format!("{root}?{}", query(&[("expand", "1".into()), ("title", title), ("body", description.to_string())]));
                }
                let people = d.assignees.iter().map(|u| u.username.as_str()).collect::<Vec<_>>().join(",");
                format!("{root}?{}", query(&[("expand", "1".into()), ("title", trimmed.to_string()), ("body", description.to_string()), ("labels", labels.join(",")), ("assignees", people)]))
            }
        }
    };
    let full = build(&d.description, &d.labels, &d.title);
    if full.len() <= MAX_URL_LENGTH || d.description.is_empty() {
        return (full, false, false);
    }
    let no_desc = build("", &d.labels, &d.title);
    if no_desc.len() <= MAX_URL_LENGTH {
        return (no_desc, true, false);
    }
    let no_labels = build("", &[], &d.title);
    if no_labels.len() <= MAX_URL_LENGTH {
        return (no_labels, true, true);
    }
    (build("", &[], ""), true, true)
}

/// A flyout's state, while it's open in its tab.
pub struct CreateUi {
    pub branch: String,
    pub heading: String,
    pub problem: Option<String>,
    pub choices: Vec<String>,
    pub route: Option<Route>,
    pub loaded: Option<Route>,
    pub good_route: Option<Route>,
    pub ctx: Option<CreateContext>,
    pub draft: Option<MrDraft>,
    pub saved: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub route_error: Option<String>,
    pub me: Option<ForgeUser>,
    pub colors: HashMap<String, Option<String>>,
    pub title: Option<Entity<TextInput>>,
    pub description: Option<Entity<MarkdownField>>,
    pub stats: Option<(u64, u64, u64)>,
    pub commits: Vec<(String, String)>,
    pub commits_open: bool,
    pub stats_for: Option<(String, String)>,
    pub limits: Option<gitvolt_core::forge::types::PeopleLimits>,
    pub template_arm: Option<String>,
    pub discard_arm: bool,
    pub scroll: ScrollHandle,
    /// Which open it is: a create still going closes only the flyout it was started from.
    pub seq: u64,
    pub _keep: Vec<Subscription>,
    pub save_task: Option<Task<()>>,
}

thread_local! {
    static SEQ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

impl GitVolt {
    fn read_mr_draft(&self, repo_path: &str, branch: &str) -> Option<MrDraft> {
        serde_json::from_value(self.prefs["mrDrafts"][draft_key(repo_path, branch).as_str()].clone()).ok()
    }

    fn write_mr_draft(&mut self, repo_path: &str, branch: &str, d: Option<&MrDraft>) {
        if !self.prefs["mrDrafts"].is_object() {
            self.prefs["mrDrafts"] = serde_json::json!({});
        }
        let key = draft_key(repo_path, branch);
        let now = crate::format::now();
        let Some(all) = self.prefs["mrDrafts"].as_object_mut() else { return };
        match d {
            Some(d) => {
                let mut v = serde_json::to_value(slim(d.clone())).unwrap_or_default();
                v["savedAt"] = serde_json::json!(now);
                all.insert(key, v);
                prune_drafts(all, now);
            }
            None => {
                all.remove(&key);
            }
        }
    }

    /// `createTarget`: the repo's target, when `branch` may get an MR/PR there: not its default branch, and no open one already.
    pub fn create_target(&self, branch: Option<&str>, chip: Option<&crate::model::Chip>) -> Option<ForgeKind> {
        let t = self.tabs.get(self.active)?;
        let f = &t.forge;
        let (kind, project, branch) = (f.kind?, f.project.as_ref()?, branch?);
        f.remote.as_ref()?;
        if project.default_branch.as_deref() == Some(branch) {
            return None;
        }
        let own = match chip {
            Some(c) => f.mr_for_chip(c).map(|m| m.0),
            None => {
                let upstream = t.local_branch(branch).and_then(|l| l.upstream);
                upstream.and_then(|u| f.by_ref.get(u.as_str()))
            }
        };
        if own.is_some_and(|m| matches!(m.state, gitvolt_core::forge::types::MrState::Open | gitvolt_core::forge::types::MrState::Draft)) {
            return None;
        }
        Some(kind)
    }

    /// Opens the branch's Create flyout: one flyout per tab, it replaces the open one.
    pub fn forge_open_create(&mut self, branch: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        let tab = t.id;
        let seq = SEQ.with(|s| {
            s.set(s.get() + 1);
            s.get()
        });
        let (remote, project, error) = (t.forge.remote.clone(), t.forge.project.clone(), t.forge.error.clone());
        t.forge.open_mr = None;
        let problem = match (&remote, &project) {
            (None, _) => Some(NO_FORGE.to_string()),
            (_, None) => Some(error.unwrap_or_else(|| NO_FORGE.to_string())),
            _ => None,
        };
        let heading = project.as_ref().map_or("Create MR/PR".to_string(), |p| format!("Create {}", mr_long_noun(p.kind)));
        let saved = self.read_mr_draft(&self.tabs[self.active].repo_path.clone(), &branch).is_some();
        let t = &mut self.tabs[self.active];
        t.create = Some(CreateUi { branch: branch.clone(), heading, problem: problem.clone(), choices: Vec::new(), route: None, loaded: None, good_route: None, ctx: None, draft: None, saved, busy: false, error: None, route_error: None, me: None, colors: HashMap::new(), title: None, description: None, stats: None, commits: Vec::new(), commits_open: false, stats_for: None, limits: None, template_arm: None, discard_arm: false, scroll: ScrollHandle::new(), seq, _keep: Vec::new(), save_task: None });
        cx.notify();
        if problem.is_some() || cfg!(test) {
            return;
        }
        let (Some(remote), Some(project)) = (remote, project) else { return };
        let host = project.host.clone();
        let (api, rt) = (self.api.clone(), self.rt.clone());
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let mapped = super::ask::<super::store::RepoProjects>(api.clone(), rt.clone(), Request::ForgeRepoProjects { repo: core_repo(tab), refresh: false }).await;
            let mapped: Vec<String> = match mapped {
                Ok(p) => p.remotes.iter().filter(|r| r.project.as_ref().is_some_and(|x| x.host.eq_ignore_ascii_case(&host))).map(|r| r.remote.clone()).collect(),
                Err(_) => vec![remote.clone()],
            };
            let accounts = super::ask::<Vec<super::accounts::AccountView>>(api, rt, Request::ForgeAccounts).await.unwrap_or_default();
            let me = accounts.into_iter().find(|a| a.account.host == host).map(|a| a.account.user);
            handle
                .update(cx, |_, window, cx| {
                    this.update(cx, |v, cx| v.create_begin(tab, seq, remote, project, mapped, me, window, cx)).ok();
                })
                .ok();
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn create_begin(&mut self, tab: u32, seq: u64, remote: String, project: ForgeProject, mapped: Vec<String>, me: Option<ForgeUser>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some(ui) = t.create.as_ref().filter(|u| u.seq == seq) else { return };
        let branch = ui.branch.clone();
        let names: Vec<String> = t.remote_groups().into_iter().map(|g| g.0).collect();
        let locals = t.locals();
        let pushed = pushed_as(locals.iter().find(|l| l.name == branch).and_then(|l| l.push_target.as_deref()), &names);
        let route = match self.read_mr_draft(&t.repo_path, &branch) {
            Some(d) => d.route(),
            None => {
                let stack = t.graph.as_ref().and_then(|g| {
                    let remotes: Vec<(String, Option<String>)> = names.iter().map(|n| (n.clone(), None)).collect();
                    let all = crate::stacks::detect_stacks(g, &crate::stacks::stack_base(g, &remotes)?);
                    crate::stacks::stack_for(&all, &branch, Some(g.head_branch.as_ref())).map(|s| s.branches.clone())
                });
                let pushed_of = |b: &str| pushed_as(locals.iter().find(|l| l.name == b).and_then(|l| l.push_target.as_deref()), &names).map(|p| p.1);
                let target_branch = default_target(&branch, stack.as_deref(), pushed_of, project.default_branch.as_deref());
                Route { source_remote: pushed.as_ref().filter(|p| mapped.contains(&p.0)).map_or(remote.clone(), |p| p.0.clone()), target_remote: remote.clone(), target_branch }
            }
        };
        let Some(ui) = self.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.create.as_mut()) else { return };
        ui.choices = mapped;
        ui.me = me;
        ui.route = Some(route);
        self.create_load_route(tab, window, cx);
    }

    /// The context for this route: an untouched draft is prefilled again; a saved one keeps its text.
    fn create_load_route(&mut self, tab: u32, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some(ui) = t.create.as_ref() else { return };
        let Some(route) = ui.route.clone() else { return };
        let req = Request::ForgeCreateContext { repo: core_repo(tab), remote: route.target_remote.clone(), source_remote: route.source_remote.clone(), branch: ui.branch.clone(), target: route.target_branch.clone() };
        let seq = ui.seq;
        self.forge_ask::<CreateContext>(req, cx, move |v, answer, cx| {
            let Some(t) = v.tabs.iter().find(|t| t.id == tab) else { return };
            let Some(ui) = t.create.as_ref().filter(|u| u.seq == seq && u.route.as_ref() == Some(&route)) else { return };
            let (path, branch) = (t.repo_path.clone(), ui.branch.clone());
            let keep = v.read_mr_draft(&path, &branch);
            match answer {
                Ok(c) => {
                    let mut next = keep.map_or_else(|| fresh_draft(&c, &route), |d| d.on(&route));
                    if next.template.as_ref().is_some_and(|p| !c.templates.iter().any(|t| &t.path == p)) {
                        next.template = None;
                    }
                    let Some(ui) = v.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.create.as_mut()) else { return };
                    ui.ctx = Some(c);
                    ui.draft = Some(next);
                    ui.good_route = Some(route.clone());
                    ui.loaded = Some(route.clone());
                    v.create_ensure_fields(tab, cx);
                    v.create_recount(tab, cx);
                    v.create_load_limits(tab, cx);
                }
                Err(e) => {
                    let good = ui.good_route.clone();
                    let Some(good) = good else {
                        if let Some(ui) = v.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.create.as_mut()) {
                            ui.problem = Some(e);
                        }
                        cx.notify();
                        return;
                    };
                    // A failed route change goes back to the route that worked, in state and in the saved draft.
                    if let Some(k) = keep {
                        v.write_mr_draft(&path, &branch, Some(&k.on(&good)));
                    }
                    let Some(ui) = v.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.create.as_mut()) else { return };
                    ui.draft = ui.draft.take().map(|d| d.on(&good));
                    ui.route_error = Some(format!("Couldn't load {}/{}: {e}", route.target_remote, route.target_branch));
                    ui.route = Some(good.clone());
                    ui.loaded = Some(good);
                }
            }
            cx.notify();
        });
    }

    fn create_ensure_fields(&mut self, tab: u32, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some(kind) = t.forge.kind else { return };
        let Some(ui) = t.create.as_ref() else { return };
        if ui.title.is_some() {
            return;
        }
        let Some(d) = ui.draft.clone() else { return };
        let md = crate::md::view::MdOptions::for_comment(core_repo(tab), super::mrview::link_env(t), super::mrview::flavor_of(kind), Default::default());
        let people = self.create_people_source(tab);
        let opts = FieldOptions { md, label: "Description".into(), placeholder: "".into(), rows: 10, grow_preview: false, people: Some(people) };
        let (api, rt, retired) = (self.api.clone(), self.rt.clone(), self.retired_images.clone());
        let field = cx.new(|cx| MarkdownField::new(opts, api, rt, retired, cx));
        field.update(cx, |f, cx| f.set_text(&d.description, cx));
        let title = cx.new(TextInput::new);
        title.update(cx, |i, cx| i.set_text(&d.title, cx));
        let keep = vec![
            cx.subscribe(&field, move |v, f, e: &FieldEvent, cx| match e {
                FieldEvent::Submit => v.create_submit(tab, None, cx),
                FieldEvent::Changed => {
                    let text = f.read(cx).text(cx);
                    v.create_update(tab, cx, move |d| d.description = text);
                }
                FieldEvent::Blurred => v.create_flush(),
            }),
            cx.subscribe(&title, move |v, i, e: &InputEvent, cx| match e {
                InputEvent::Submit { ctrl: true, .. } => v.create_submit(tab, None, cx),
                InputEvent::Changed => {
                    let text = i.read(cx).text().to_string();
                    v.create_update(tab, cx, move |d| d.title = text);
                }
                _ => {}
            }),
        ];
        let focus = title.read(cx).focus.clone();
        let Some(ui) = self.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.create.as_mut()) else { return };
        ui.title = Some(title);
        ui.description = Some(field);
        ui._keep = keep;
        self.with_window(cx, move |window, cx| focus.focus(window, cx));
    }

    fn create_people_source(&self, tab: u32) -> crate::md::field::PeopleSource {
        let remote = self.tabs.iter().find(|t| t.id == tab).and_then(|t| t.forge.remote.clone());
        let (api, rt) = (self.api.clone(), self.rt.clone());
        let search: Option<crate::md::field::PeopleSearch> = remote.map(|remote| {
            Rc::new(move |q: String, cx: &mut App| {
                let (api, rt, remote) = (api.clone(), rt.clone(), remote.clone());
                cx.spawn(async move |_| {
                    let users = super::ask::<Vec<ForgeUser>>(api, rt, Request::ForgeSearchUsers { repo: core_repo(tab), remote, query: q }).await.unwrap_or_default();
                    users.into_iter().map(|u| crate::md::complete::Person { id: u.id, username: u.username, name: u.name }).collect()
                })
            }) as crate::md::field::PeopleSearch
        });
        crate::md::field::PeopleSource { participants: Vec::new(), search, avatar: None }
    }

    /// What the branch brings over its target: counted locally, its merge base, then the compare list.
    fn create_recount(&mut self, tab: u32, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        let target = t.locals().into_iter().find(|l| Some(&l.name) == t.create.as_ref().map(|u| &u.branch)).map(|l| l.target);
        let Some(ui) = t.create.as_mut() else { return };
        let (Some(tip), Some(route)) = (target, ui.route.clone()) else { return };
        let base_ref = format!("refs/remotes/{}/{}", route.target_remote, route.target_branch);
        let key = (tip.clone(), base_ref.clone());
        if ui.stats_for.as_ref() == Some(&key) {
            return;
        }
        ui.stats_for = Some(key);
        ui.stats = None;
        let (api, repo) = (self.api.clone(), core_repo(tab));
        let job = self.rt.spawn(async move {
            let base = api.dispatch(Request::MergeBase { repo, a: base_ref, b: tip.clone() }).await.ok()?;
            let spec: gitvolt_core::diff::DiffSpec = serde_json::from_value(serde_json::json!({ "kind": "compare", "from": base.as_str()?, "to": tip.clone() })).ok()?;
            let list = api.dispatch(Request::FileList { repo, spec }).await.ok()?;
            Some((base.as_str()?.to_string(), tip, list))
        });
        cx.spawn(async move |this, cx| {
            let answer = job.await.ok().flatten();
            this.update(cx, |v, cx| {
                let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) else { return };
                let commits = answer.as_ref().and_then(|(base, tip, _)| {
                    let g = t.graph.as_ref()?;
                    super::mrview::header::commits_between(g.rows.iter().filter(|r| r.wip.is_none()).map(|r| (r.id.as_ref(), r.summary.as_ref(), r.parents.first().map(String::as_str))), tip, base)
                });
                if let Some(ui) = t.create.as_mut() {
                    ui.commits = commits.unwrap_or_default();
                    ui.stats = answer.map(|(_, _, l)| (l["files"].as_array().map_or(0, |f| f.len() as u64), l["added"].as_u64().unwrap_or(0), l["deleted"].as_u64().unwrap_or(0)));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn create_load_limits(&mut self, tab: u32, cx: &mut Context<Self>) {
        let Some(remote) = self.tabs.iter().find(|t| t.id == tab).and_then(|t| t.create.as_ref()).and_then(|u| u.draft.as_ref()).map(|d| d.target_remote.clone()) else { return };
        self.forge_ask::<gitvolt_core::forge::types::PeopleLimits>(Request::ForgePeopleLimits { repo: core_repo(tab), remote }, cx, move |v, answer, cx| {
            if let (Ok(l), Some(ui)) = (answer, v.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.create.as_mut())) {
                ui.limits = Some(l);
                cx.notify();
            }
        });
    }

    /// A change by the user: from now on it's a draft of its own, saved a moment after the last one.
    fn create_update(&mut self, tab: u32, cx: &mut Context<Self>, change: impl FnOnce(&mut MrDraft)) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        let path = t.repo_path.clone();
        let Some(ui) = t.create.as_mut() else { return };
        let Some(d) = ui.draft.as_mut() else { return };
        let before = d.clone();
        change(d);
        if *d == before {
            return;
        }
        let (branch, next) = (ui.branch.clone(), d.clone());
        ui.error = None;
        ui.saved = true;
        ui.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(300)).await;
            this.update(cx, |v, _| v.save_layout()).ok();
        }));
        self.write_mr_draft(&path, &branch, Some(&next));
        cx.notify();
    }

    fn create_flush(&mut self) {
        self.save_layout();
    }

    fn create_move(&mut self, tab: u32, change: impl FnOnce(&mut Route), window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        let path = t.repo_path.clone();
        let Some(ui) = t.create.as_mut() else { return };
        let Some(mut next) = ui.route.clone() else { return };
        change(&mut next);
        ui.route_error = None;
        ui.route = Some(next.clone());
        ui.draft = ui.draft.take().map(|d| d.on(&next));
        let (saved, branch, d) = (ui.saved, ui.branch.clone(), ui.draft.clone());
        if saved {
            self.write_mr_draft(&path, &branch, d.as_ref());
        }
        self.create_load_route(tab, window, cx);
        cx.notify();
    }

    pub fn forge_close_create(&mut self, tab: u32, cx: &mut Context<Self>) {
        self.create_flush();
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
            t.create = None;
        }
        cx.notify();
    }

    fn create_discard(&mut self, tab: u32, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        let path = t.repo_path.clone();
        let Some(ui) = t.create.take() else { return };
        self.write_mr_draft(&path, &ui.branch, None);
        self.save_layout();
        cx.notify();
    }

    /// `asDraft`: the split button's choice, kept in the draft, so a failed create offers the same again.
    fn create_submit(&mut self, tab: u32, as_draft: Option<bool>, cx: &mut Context<Self>) {
        if let Some(on) = as_draft {
            self.create_update(tab, cx, move |d| d.draft = on);
        }
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some(ui) = t.create.as_ref() else { return };
        let (Some(d), Some(c)) = (ui.draft.clone(), ui.ctx.clone()) else { return };
        let state = create_state(t, ui, &d, &c);
        if ui.busy || state.blocked.is_some() {
            return;
        }
        let kind = c.project.kind;
        let req = create_request(&d, &c, &state.source_branch);
        let (seq, branch, path) = (ui.seq, ui.branch.clone(), t.repo_path.clone());
        if let Some(ui) = self.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.create.as_mut()) {
            ui.busy = true;
            ui.error = None;
            if let Some(i) = &ui.title {
                i.update(cx, |i, cx| i.set_disabled(true, cx));
            }
            if let Some(f) = &ui.description {
                f.update(cx, |f, cx| f.set_disabled(true, cx));
            }
        }
        cx.notify();
        let remote = d.target_remote.clone();
        let sent = d.clone();
        let (r2, req2) = (remote.clone(), req.clone());
        self.forge_ask::<CreateOutcome>(Request::ForgeCreateMr { repo: core_repo(tab), remote, req }, cx, move |v, answer, cx| {
            let here = v.tabs.iter().find(|t| t.id == tab).and_then(|t| t.create.as_ref()).is_some_and(|u| u.seq == seq);
            match answer {
                Ok(out) => {
                    v.write_mr_draft(&path, &branch, None);
                    v.save_layout();
                    if here && let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) {
                        t.create = None;
                    }
                    v.create_show_outcome(tab, kind, r2.clone(), req2.clone(), out.mr.number, out.mr.web_url.clone(), out.failed, cx);
                    v.forge_written(tab, cx);
                }
                Err(e) => {
                    if here {
                        if let Some(ui) = v.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.create.as_mut()) {
                            ui.busy = false;
                            ui.error = Some(e);
                            if let Some(i) = &ui.title {
                                i.update(cx, |i, cx| i.set_disabled(false, cx));
                            }
                            if let Some(f) = &ui.description {
                                f.update(cx, |f, cx| f.set_disabled(false, cx));
                            }
                        }
                    } else {
                        v.write_mr_draft(&path, &branch, Some(&sent));
                        v.save_layout();
                        let noun = mr_long_noun(kind);
                        crate::ui::toast::show_with(format!("Couldn't create {noun} from {branch}: {e}; the draft is kept"), crate::ui::toast::Options { warning: true, ..Default::default() }, cx);
                    }
                }
            }
            cx.notify();
        });
    }

    /// `showCreated`: "Created !12", or a sticky warning with a Retry for what failed.
    #[allow(clippy::too_many_arguments)]
    fn create_show_outcome(&mut self, tab: u32, kind: ForgeKind, remote: String, req: CreateMr, number: u64, url: String, failed: Vec<PartFailure>, cx: &mut Context<Self>) {
        let r = mr_ref(kind, number);
        let weak = cx.weak_entity();
        let open = {
            let (w, u) = (weak.clone(), url.clone());
            crate::ui::toast::ToastAction {
                label: format!("Open {r}").into(),
                run: Rc::new(move |_: &mut Window, cx: &mut App| {
                    let u = u.clone();
                    w.update(cx, |v, _| v.forge_open_url(u)).ok();
                }),
            }
        };
        if failed.is_empty() {
            crate::ui::toast::show_with(format!("Created {r}"), crate::ui::toast::Options { actions: vec![open], ..Default::default() }, cx);
            return;
        }
        let parts: Vec<CreatePart> = failed.iter().map(|f| f.part).collect();
        let w = weak.clone();
        let retry = crate::ui::toast::ToastAction {
            label: "Retry".into(),
            run: Rc::new(move |_: &mut Window, cx: &mut App| {
                let (remote, req, url, parts) = (remote.clone(), req.clone(), url.clone(), parts.clone());
                w.update(cx, |v, cx| v.create_retry(tab, kind, remote, req, number, url, parts, cx)).ok();
            }),
        };
        crate::ui::toast::show_with(format!("{} {r} created; {}", mr_noun(kind), failure_text(&failed)), crate::ui::toast::Options { actions: vec![retry, open], sticky: true, warning: true, ..Default::default() }, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn create_retry(&mut self, tab: u32, kind: ForgeKind, remote: String, req: CreateMr, number: u64, url: String, parts: Vec<CreatePart>, cx: &mut Context<Self>) {
        crate::ui::toast::show_with("Retrying…", crate::ui::toast::Options { sticky: true, ..Default::default() }, cx);
        let (r2, req2, p2) = (remote.clone(), req.clone(), parts.clone());
        self.forge_ask::<Vec<PartFailure>>(Request::ForgeCompleteCreate { repo: core_repo(tab), remote, number, req, parts }, cx, move |v, answer, cx| {
            let still = answer.unwrap_or_else(|e| p2.iter().map(|p| PartFailure { part: *p, message: e.clone() }).collect());
            if still.is_empty() {
                crate::ui::toast::show(format!("Added {} to {}", parts_text(&p2), mr_ref(kind, number)), cx);
            } else {
                v.create_show_outcome(tab, kind, r2.clone(), req2.clone(), number, url.clone(), still, cx);
            }
            v.forge_written(tab, cx);
        });
    }

    /// The push toast's link, for a branch this push created on a remote of the target's forge.
    pub fn push_create_link(&self, b: &crate::sync::LocalBranch, created: bool, remote: &str, up_to_date: bool, cx: &mut Context<Self>) -> Vec<crate::ui::toast::ToastAction> {
        if !created || up_to_date || !self.tabs.get(self.active).is_some_and(|t| t.forge.mapped.iter().any(|m| m == remote)) {
            return Vec::new();
        }
        let Some(kind) = self.create_target(Some(&b.name), None) else { return Vec::new() };
        let (w, name) = (cx.weak_entity(), b.name.clone());
        vec![crate::ui::toast::ToastAction {
            label: format!("Create {}", mr_noun(kind)).into(),
            run: Rc::new(move |window: &mut Window, cx: &mut App| {
                let name = name.clone();
                w.update(cx, |v, cx| v.forge_open_create(name, window, cx)).ok();
            }),
        }]
    }

    fn create_on_forge(&mut self, tab: u32, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some(ui) = t.create.as_ref() else { return };
        let (Some(d), Some(c)) = (ui.draft.as_ref(), ui.ctx.as_ref()) else { return };
        let state = create_state(t, ui, d, c);
        let (url, without, more) = new_mr_url(&c.project, &c.source_project, &state.source_branch, d);
        if without {
            cx.write_to_clipboard(ClipboardItem::new_string(d.description.clone()));
            crate::ui::toast::show(format!("The description is too long for the link: it's on the clipboard to paste on {}{}", forge_name(c.project.kind), if more { "; the labels or title were trimmed too" } else { "" }), cx);
        }
        self.forge_open_url(url);
    }

    fn create_choose_template(&mut self, tab: u32, path: String, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        let Some(ui) = t.create.as_mut() else { return };
        let (Some(d), Some(c)) = (ui.draft.clone(), ui.ctx.clone()) else { return };
        let (next, edited) = with_template(&d, &c, c.templates.iter().find(|t| t.path == path));
        // The description would go: said in place, and it's the next choice that replaces it.
        if edited && ui.template_arm.as_deref() != Some(path.as_str()) {
            ui.template_arm = Some(path);
            crate::ui::toast::show("Your edits to the description are replaced by the template: choose it again to replace them", cx);
            cx.notify();
            return;
        }
        ui.template_arm = None;
        if let Some(f) = ui.description.clone() {
            let text = next.description.clone();
            f.update(cx, |f, cx| f.set_text(&text, cx));
        }
        self.create_update(tab, cx, move |d| *d = next);
    }

    pub fn create_escape(&mut self, tab: u32, cx: &mut Context<Self>) {
        self.forge_close_create(tab, cx);
    }
}

/// What the draft and the sidebar come to.
pub struct CreateState {
    pub source_branch: String,
    pub on_remote: bool,
    pub unpushed: Option<u32>,
    pub push_elsewhere: bool,
    pub blocked: Option<String>,
}

fn create_state(t: &crate::app::Tab, ui: &CreateUi, d: &MrDraft, c: &CreateContext) -> CreateState {
    let names: Vec<String> = t.remote_groups().into_iter().map(|g| g.0).collect();
    let local = t.local_branch(&ui.branch);
    let pushed = pushed_as(local.as_ref().and_then(|l| l.push_target.as_deref()), &names);
    let source_branch = source_branch_of(&ui.branch, pushed.as_ref(), &d.source_remote, &d.target_branch, c.source_project == c.project.path);
    let on_source = t.side.sections.iter().flat_map(|s| &s.items).find_map(|i| match &i.kind {
        crate::sidebar::ItemKind::Remote { remote, full, .. } if *remote == d.source_remote && i.name == source_branch => Some((i.target.clone().unwrap_or_default(), full.clone())),
        _ => None,
    });
    let on_remote = on_source.is_some();
    let unpushed = unpushed_count(local.as_ref().map(|l| (l.target.as_str(), l.upstream.as_deref(), l.ahead)), on_source.as_ref().map(|r| (r.0.as_str(), r.1.as_str())));
    let push_elsewhere = pushed.as_ref().is_some_and(|p| p.0 != d.source_remote || p.1 != source_branch);
    let push_note = create_blocked(d, &source_branch, on_remote, &c.source_project, &c.project.path);
    let loading = ui.loaded != ui.route;
    let blocked = if loading { Some("Loading…".to_string()) } else { push_note.clone() };
    CreateState { source_branch, on_remote, unpushed, push_elsewhere, blocked }
}

type OnPicked = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// A quiet dropdown: its text and a caret, the options in the app's menu.
fn quiet_select(key: &'static str, value: &str, shown: AnyElement, options: Vec<(String, String)>, on_change: OnPicked) -> AnyElement {
    let current = value.to_string();
    crate::ui::anchor(
        key,
        div()
            .id(key)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .px(px(4.))
            .ml(px(-4.))
            .rounded(px(4.))
            .text_size(px(12.))
            .text_color(theme::text_secondary())
            .cursor_pointer()
            .hover(|s| s.bg(theme::control_hover_bg()).text_color(theme::text_normal()))
            .child(shown)
            .child(icon("chevron-down", 12., theme::text_secondary()))
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                let rows: Vec<crate::ui::menu::MenuRow> = options
                    .iter()
                    .map(|(v, label)| {
                        let (on, v2) = (on_change.clone(), v.clone());
                        crate::ui::menu::MenuRow::action(format!("opt.{v}"), label.clone(), if *v == current { "check" } else { "" }, "", move |window, cx| on(v2.clone(), window, cx))
                    })
                    .collect();
                let o = crate::ui::menu::OpenOptions { initial: Some(format!("opt.{current}").into()), narrow: true, ..Default::default() };
                crate::ui::menu::open_below(key, rows, o, window, cx);
            }),
    )
    .into_any_element()
}

/// `+ Add`: a dashed pill.
fn add_pill(key: String, label: &'static str, off: bool) -> Stateful<Div> {
    div()
        .id(ElementId::Name(key.into()))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .h(px(24.))
        .px(px(8.))
        .rounded(px(12.))
        .border_1()
        .border_dashed()
        .border_color(theme::mix(theme::text_selected(), 0.18, theme::panel_bg1()))
        .text_size(px(12.))
        .text_color(theme::text_secondary())
        .when(off, |d| d.opacity(0.6))
        .when(!off, |d| d.cursor_pointer().hover(|s| s.border_color(theme::blue()).text_color(theme::text_normal())))
        .child(icon("plus", 12., theme::text_secondary()))
        .child(label)
}

fn ghost(id: impl Into<ElementId>, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .h(px(30.))
        .px(px(12.))
        .rounded(px(6.))
        .border_1()
        .border_color(theme::section_border())
        .text_color(if enabled { theme::text_normal() } else { theme::text_disabled() })
        .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(theme::control_hover_bg())))
}

fn head_btn(id: &'static str) -> Stateful<Div> {
    div().id(id).flex().flex_none().size(px(24.)).items_center().justify_center().rounded(px(4.)).cursor_pointer().hover(|d| d.bg(theme::control_hover_bg()))
}

fn field_label(text: &'static str, hint: Option<String>) -> Div {
    div().flex().items_center().gap(px(8.)).text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_secondary()).child(text).when_some(hint, |d, h| d.child(div().ml_auto().font_weight(FontWeight::NORMAL).text_color(theme::text_disabled()).child(h)))
}

#[allow(clippy::too_many_arguments)]
fn people_row(this: &GitVolt, tab: u32, kind: ForgeKind, role: Role, list: &[ForgeUser], limits: Option<&gitvolt_core::forge::types::PeopleLimits>, repo_remote: &str, me: Option<&ForgeUser>, disabled: bool, cx: &mut Context<GitVolt>) -> AnyElement {
    let max = max_of(limits, role);
    let count = list.len() as u32;
    let swap = max == Some(1) && count >= 1;
    let full = !swap && max.is_some_and(|m| count >= m);
    let name = format!("{} {}", if swap { "Replace" } else { "Add" }, role.noun());
    let text = if swap || full { max.map_or(name.clone(), |m| limit_tip(kind, role, m)) } else { name };
    let key = format!("create-add-{}", role.name());
    let (api, rt, weak, open_key, remote) = (this.api.clone(), this.rt.clone(), cx.weak_entity(), key.clone(), repo_remote.to_string());
    let current = list.to_vec();
    let add = tip(
        text.clone(),
        crate::ui::anchor(
            key.clone(),
            add_pill(key, if swap { "Replace" } else { "Add" }, full || disabled).when(!full && !disabled, |d| {
                d.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    let Some(at) = crate::ui::Ui::get(cx).anchor(&open_key) else { return };
                    let w = weak.clone();
                    let taken = current.iter().map(|u| u.id.to_string()).collect();
                    let source = Source::People { repo: core_repo(tab), remote: remote.clone() };
                    let on_pick = Rc::new(move |o: &SearchOption, cx: &mut App| {
                        let Some(user) = o.user.clone() else { return };
                        w.update(cx, |v, cx| {
                            v.create_update(tab, cx, move |d| {
                                let l = if role == Role::Reviewers { &mut d.reviewers } else { &mut d.assignees };
                                if max == Some(1) {
                                    l.clear();
                                }
                                l.push(user);
                            })
                        })
                        .ok();
                    });
                    search::open(format!("create-search-{}", role.name()).into(), role.name_plural().into(), at, true, source, taken, api.clone(), rt.clone(), on_pick, window, cx);
                })
            }),
        ),
    );
    let mut chips = div().flex().flex_1().flex_wrap().items_center().gap(px(6.)).min_w_0();
    for u in list {
        let id = u.id;
        let rm = (!disabled).then(|| {
            rm_button(format!("create-rm-{}-{}", role.name(), u.username), &format!("Remove {}", u.name), move |v, cx| {
                v.create_update(tab, cx, move |d| {
                    let l = if role == Role::Reviewers { &mut d.reviewers } else { &mut d.assignees };
                    l.retain(|x| x.id != id);
                })
            }, cx)
        });
        chips = chips.child(person_chip(u, &u.name, rm));
    }
    // Kept in the row once assigned (hidden), so the row never reflows.
    let me_btn = (role == Role::Assignees).then_some(me).flatten().map(|m| {
        let assigned = list.iter().any(|u| u.id == m.id);
        let who = m.clone();
        div()
            .id("create-assign-me")
            .flex_none()
            .ml(px(4.))
            .text_size(px(11.5))
            .text_color(theme::link())
            .when(assigned, |d| d.invisible())
            .when(!assigned && !disabled, |d| {
                d.cursor_pointer().on_click(cx.listener(move |v, _, _, cx| {
                    let who = who.clone();
                    v.create_update(tab, cx, move |d| d.assignees.push(who));
                }))
            })
            .child("Assign to me")
    });
    let chips = chips.child(add).children(me_btn);
    div().flex().items_center().gap(px(8.)).min_h(px(40.)).px(px(10.)).py(px(6.)).child(div().flex_none().w(px(92.)).text_size(px(12.)).text_color(theme::text_secondary()).child(role.name_plural())).child(chips).into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn labels_row(this: &GitVolt, tab: u32, kind: ForgeKind, labels: &[String], colors: &HashMap<String, Option<String>>, remote: &str, disabled: bool, cx: &mut Context<GitVolt>) -> AnyElement {
    let (api, rt, weak, remote) = (this.api.clone(), this.rt.clone(), cx.weak_entity(), remote.to_string());
    let list: Vec<String> = labels.to_vec();
    let add = tip(
        "Add label",
        crate::ui::anchor(
            "create-add-label",
            add_pill("create-add-label".to_string(), "Add", disabled).when(!disabled, |d| {
                d.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    let Some(at) = crate::ui::Ui::get(cx).anchor("create-add-label") else { return };
                    let w = weak.clone();
                    let source = Source::Labels { repo: core_repo(tab), remote: remote.clone(), limit: if kind == ForgeKind::GitHub { 300 } else { 50 } };
                    let on_pick = Rc::new(move |o: &SearchOption, cx: &mut App| {
                        let (name, color) = (o.key.clone(), o.color.clone());
                        w.update(cx, |v, cx| {
                            if let Some(ui) = v.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.create.as_mut()) {
                                ui.colors.insert(name.clone(), color);
                            }
                            v.create_update(tab, cx, move |d| {
                                if !d.labels.contains(&name) {
                                    d.labels.push(name);
                                }
                            });
                        })
                        .ok();
                    });
                    search::open("create-search-labels".into(), "Labels".into(), at, true, source, list.clone(), api.clone(), rt.clone(), on_pick, window, cx);
                })
            }),
        ),
    );
    let mut chips = div().flex().flex_1().flex_wrap().items_center().gap(px(6.)).min_w_0();
    for l in labels {
        let name = l.clone();
        let rm = (!disabled).then(|| {
            rm_button(format!("create-rm-label-{l}"), &format!("Remove {l}"), move |v, cx| {
                let name = name.clone();
                v.create_update(tab, cx, move |d| d.labels.retain(|x| *x != name))
            }, cx)
        });
        chips = chips.child(label_pill(l, colors.get(l).and_then(|c| c.as_deref()), rm));
    }
    let chips = chips.child(add);
    div().flex().items_center().gap(px(8.)).min_h(px(40.)).px(px(10.)).py(px(6.)).child(div().flex_none().w(px(92.)).text_size(px(12.)).text_color(theme::text_secondary()).child("Labels")).child(chips).into_any_element()
}

/// The flyout, when one's open in the shown tab.
pub fn create_flyout(this: &mut GitVolt, room: f32, window: &mut Window, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let t = this.tabs.get(this.active)?;
    let tab = t.id;
    let ui = t.create.as_ref()?;
    if t.diff.is_some() || t.history.is_some() {
        return None;
    }
    let width = super::mrview::flyout_width(this.profile["flyoutWidth"].as_f64().map(|w| w as f32), room);

    let close = tip("Close (Esc)", crate::ui::anchor("create-close", head_btn("create-close").on_click(cx.listener(move |v, _, _, cx| v.forge_close_create(tab, cx))).child(icon("x", 14., theme::text_secondary()))));

    let c = ui.ctx.clone();
    let d = ui.draft.clone();
    let heading = match &c {
        Some(c) => {
            let noun = mr_long_noun(c.project.kind);
            let color = match c.project.kind {
                ForgeKind::GitLab => theme::forge_gitlab(),
                ForgeKind::GitHub => theme::text_selected(),
                ForgeKind::Forgejo => theme::forge_forgejo(),
            };
            div().flex().flex_1().min_w_0().items_center().gap(px(6.)).child(icon(super::text::mark_icon(c.project.kind), 16., color)).child(div().flex_none().child(format!("New {noun}"))).child(div().min_w_0().ml(px(2.)).truncate().text_size(px(12.)).font_weight(FontWeight::NORMAL).text_color(theme::text_secondary()).child(c.project.path.clone()))
        }
        None => div().flex_1().min_w_0().child(ui.heading.clone()),
    };
    let head = div().flex().flex_none().items_center().gap(px(6.)).pt(px(8.)).pb(px(8.)).pl(px(12.)).pr(px(8.)).border_b_1().border_color(theme::section_border()).text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).child(heading).child(close);

    let mut body = div().id("create-body").flex().flex_col().flex_1().min_h_0().overflow_y_scroll().gap(px(14.)).pt(px(14.)).pb(px(14.)).px(px(14.)).track_scroll(&ui.scroll);
    let mut foot: Option<AnyElement> = None;
    match (&ui.problem, &c, &d) {
        (Some(p), _, _) => {
            body = body.child(div().text_color(theme::red()).child(p.clone()));
            if ui.saved {
                foot = Some(div().flex().flex_1().justify_end().child(crate::ui::anchor("create-discard", ghost("create-discard", true).child("Discard draft").on_click(cx.listener(move |v, _, _, cx| v.create_discard(tab, cx))))).into_any_element());
            }
        }
        (None, Some(c), Some(d)) => {
            let (b, f) = create_form(this, tab, c, d, window, cx);
            body = body.children(b);
            foot = Some(f);
        }
        _ => body = body.child(div().text_color(theme::text_secondary()).child("Loading…")),
    }
    let frame = crate::ui::anchor("flyout-frame", div()
        .id("create-flyout")
        .key_context("MrView")
        .occlude()
            .child(crate::ui::spin::cover())
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        .w(px(width))
        .h_full()
        .min_h_0()
        .bg(theme::panel_bg1())
        .text_color(theme::text_normal())
        .border_r_1()
        .border_color(theme::section_border())
        .shadow(vec![BoxShadow { color: theme::shadow_popup(), offset: point(px(4.), px(0.)), blur_radius: px(16.), spread_radius: px(0.), inset: false }])
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_key_down(cx.listener(move |v, e: &KeyDownEvent, _, cx| {
            if e.keystroke.key == "escape" && !e.keystroke.modifiers.modified() {
                v.create_escape(tab, cx);
                cx.stop_propagation();
            }
        }))
        .child(head)
        .child(body)
        .when_some(foot, |d, f| d.child(div().flex().flex_none().px(px(14.)).py(px(12.)).border_t_1().border_color(theme::section_border()).child(f)))
        .child(
            div()
                .id("create-resize")
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(-3.))
                .w(px(6.))
                .cursor(CursorStyle::ResizeLeftRight)
                .on_mouse_down(MouseButton::Left, cx.listener(move |v, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if e.click_count == 2 {
                        v.mr_set_width(None, false, cx);
                        return;
                    }
                    v.mr_drag = Some((e.position.x.as_f32(), width, false));
                })),
        ))
        .into_any_element();
    Some(frame)
}

fn create_form(this: &mut GitVolt, tab: u32, c: &CreateContext, d: &MrDraft, window: &mut Window, cx: &mut Context<GitVolt>) -> (Vec<AnyElement>, AnyElement) {
    let t = this.tabs.iter().find(|t| t.id == tab).expect("the flyout's tab");
    let ui = t.create.as_ref().expect("the flyout");
    let state = create_state(t, ui, d, c);
    let kind = c.project.kind;
    let noun = mr_long_noun(kind);
    let busy = ui.busy;
    let branch = ui.branch.clone();
    let names = ui.choices.clone();
    let remote_opts: Vec<(String, String)> = names.iter().map(|r| (r.clone(), r.clone())).collect();
    let weak = cx.weak_entity();
    let on_remote = |source: bool| -> OnPicked {
        let w = weak.clone();
        Rc::new(move |v: String, window: &mut Window, cx: &mut App| {
            w.update(cx, |this, cx| this.create_move(tab, move |r| if source { r.source_remote = v } else { r.target_remote = v }, window, cx)).ok();
        })
    };
    let end_head = |caption: &'static str| div().font_features(crate::glyphs::spacing(0.06)).text_size(px(10.5)).text_color(theme::text_disabled()).child(caption.to_uppercase());
    let name_el = |name: String| tip(name.clone(), div().min_w_0().truncate().font_family(crate::app::MONO_FONT).text_size(px(12.5)).text_color(theme::text_selected()).child(name));
    let targets: Vec<String> = t.side.sections.iter().flat_map(|s| &s.items).filter_map(|i| match &i.kind {
        crate::sidebar::ItemKind::Remote { remote, .. } if *remote == d.target_remote && i.name != "HEAD" => Some(i.name.clone()),
        _ => None,
    }).collect();
    let current_target = d.target_branch.clone();
    let w = weak.clone();
    let pick_target = crate::ui::anchor(
        "create-target",
        div().id("create-target").flex().items_center().gap(px(4.)).min_w_0().mt(px(-1.)).px(px(4.)).ml(px(-4.)).rounded(px(4.)).cursor_pointer().hover(|s| s.bg(theme::control_hover_bg())).child(name_el(d.target_branch.clone())).child(icon("chevron-down", 12., theme::text_secondary())).on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            let items = targets.iter().map(|n| crate::ui::picker::PickItem { id: n.clone().into(), label: n.clone().into(), detail: None, icon: None, current: *n == current_target, tooltip: None }).collect();
            let w = w.clone();
            crate::ui::picker::open_below("create-target", items, "Target branch", None, move |item, window, cx| {
                let name = item.id.to_string();
                w.update(cx, |this, cx| this.create_move(tab, move |r| r.target_branch = name, window, cx)).ok();
            }, window, cx);
        }),
    );
    let from_end = div().flex().flex_col().gap(px(4.)).flex_1().min_w_0().px(px(12.)).py(px(10.)).child(end_head("From")).child(name_el(state.source_branch.clone())).child(div().flex().child(quiet_select("create-source-remote", &d.source_remote, div().child(d.source_remote.clone()).into_any_element(), remote_opts.clone(), on_remote(true))));
    let into_end = div().flex().flex_col().gap(px(4.)).flex_1().min_w_0().px(px(12.)).py(px(10.)).child(end_head("Into")).child(pick_target).child(div().flex().child(quiet_select("create-target-remote", &d.target_remote, div().child(d.target_remote.clone()).into_any_element(), remote_opts, on_remote(false))));
    let counts: AnyElement = match ui.stats {
        None => div().text_color(theme::text_disabled()).child(format!("{}/{} isn't fetched: no counts yet", d.target_remote, d.target_branch)).into_any_element(),
        Some((files, added, deleted)) => {
            let bold = |n: String, word: &'static str| div().flex().gap(px(3.)).child(div().font_weight(FontWeight::SEMIBOLD).text_color(theme::text_normal()).child(n)).child(word);
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .when_some(c.first_commit.as_ref(), |e, f| e.child(bold(f.count.to_string(), if f.count == 1 { "commit" } else { "commits" })))
                .child(bold(files.to_string(), if files == 1 { "file" } else { "files" }))
                .child(div().text_color(theme::green()).child(format!("+{added}")))
                .child(div().text_color(theme::red()).child(format!("−{deleted}")))
                .into_any_element()
        }
    };
    let commits_open = ui.commits_open;
    let toggle = (!ui.commits.is_empty()).then(|| {
        div().id("create-commits-toggle").flex().ml_auto().items_center().gap(px(4.)).text_color(theme::text_secondary()).cursor_pointer().child(if commits_open { "Hide commits" } else { "Show commits" }).child(icon(if commits_open { "chevron-up" } else { "chevron-down" }, 12., theme::text_secondary())).on_click(cx.listener(move |v, _, _, cx| {
            if let Some(u) = v.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.create.as_mut()) {
                u.commits_open = !u.commits_open;
            }
            cx.notify();
        }))
    });
    let commit_list = commits_open.then(|| {
        let mut list = div().id("create-commits").flex().flex_col().py(px(4.)).max_h(px(200.)).overflow_y_scroll().border_t_1().border_color(theme::section_border());
        for (i, (sha, summary)) in ui.commits.iter().enumerate() {
            let s2 = sha.clone();
            list = list.child(div().id(("create-commit", i)).flex().items_baseline().gap(px(8.)).px(px(12.)).py(px(3.)).text_size(px(12.)).text_color(theme::text_normal()).cursor_pointer().hover(|d| d.bg(theme::hover_row())).child(div().flex_none().font_family(crate::app::MONO_FONT).text_color(theme::text_secondary()).child(sha.chars().take(7).collect::<String>())).child(div().min_w_0().truncate().child(summary.clone())).on_click(cx.listener(move |v, _, _, cx| v.go_to_sha(&s2, cx))));
        }
        list
    });
    // Not there, or not all of it: what's in the MR/PR is what's on the remote.
    let strip: Option<AnyElement> = if !state.on_remote {
        let local = t.local_branch(&branch);
        let elsewhere = state.push_elsewhere;
        let label = format!("Push {branch}{}", if elsewhere { "…" } else { "" });
        let ctx = t.write_target();
        let btn = local.map(|b| {
            crate::ui::anchor(
                "create-push",
                super::mrview::header::mr_primary("create-push", true).child(label).on_click(cx.listener(move |v, _, _, cx| {
                    if elsewhere {
                        v.open_push_upstream(ctx.clone(), b.clone(), crate::ui::arm::Origin::None, cx);
                    } else {
                        v.push_branch(ctx.clone(), b.clone(), crate::ui::arm::Origin::None, cx);
                    }
                })),
            )
        });
        Some(flow_strip(div().flex().gap(px(4.)).flex_wrap().child(div().font_family(crate::app::MONO_FONT).child(state.source_branch.clone())).child(format!("isn't on {} yet.", d.source_remote)).into_any_element(), btn.map(IntoElement::into_any_element)))
    } else if state.unpushed != Some(0) {
        let text = match state.unpushed {
            None => format!("{branch} differs from {}/{}", d.source_remote, state.source_branch),
            Some(1) => "1 commit isn't pushed yet".to_string(),
            Some(n) => format!("{n} commits aren't pushed yet"),
        };
        Some(flow_strip(div().child(format!("{text}: the {noun} has what's on {}.", d.source_remote)).into_any_element(), None))
    } else {
        None
    };
    let flow = div().id("create-flow").flex_none().rounded(px(8.)).border_1().border_color(theme::section_border()).bg(theme::panel_bg1()).overflow_hidden().child(div().flex().items_stretch().child(from_end).child(div().flex().flex_none().w(px(28.)).items_center().justify_center().child(icon("arrow-right", 16., theme::text_secondary()))).child(into_end)).child(div().min_h(px(31.)).flex().items_center().px(px(12.)).py(px(6.)).text_size(px(12.)).border_t_1().border_color(theme::section_border()).bg(theme::panel_bg()).child(counts).children(toggle)).children(commit_list).children(strip);

    let from_first = c.first_commit.as_ref().is_some_and(|f| !f.summary.is_empty() && f.summary == d.title);
    let title_el = ui.title.clone().map(|i| crate::ui::anchor("create-title", div().id("create-title").px(px(10.)).py(px(7.)).rounded(px(6.)).border_1().border_color(if i.read(cx).is_focused(window) { theme::blue() } else { theme::wip_input_border() }).bg(theme::wip_input_bg()).text_size(px(14.)).font_weight(FontWeight::MEDIUM).text_color(theme::text_selected()).child(i)));
    let template_opts: Vec<(String, String)> = std::iter::once((String::new(), "No template".to_string())).chain(c.templates.iter().map(|t| (t.path.clone(), t.name.clone()))).collect();
    let template_name = c.templates.iter().find(|t| Some(&t.path) == d.template.as_ref()).map_or("none".to_string(), |t| t.name.clone());
    let w = weak.clone();
    let on_template: OnPicked = Rc::new(move |p: String, _: &mut Window, cx: &mut App| {
        w.update(cx, |this, cx| this.create_choose_template(tab, p, cx)).ok();
    });
    let chosen = d.template.clone().unwrap_or_default();
    if let Some(f) = ui.description.clone() {
        f.update(cx, |f, _| {
            f.set_toolbar(Rc::new(move |_| {
                let shown = div().flex().items_center().gap(px(4.)).child(icon("file-text", 13., theme::text_secondary())).child(format!("Template: {template_name}")).into_any_element();
                quiet_select("create-template", &chosen, shown, template_opts.clone(), on_template.clone())
            }));
        });
    }
    let desc = ui.description.clone();
    let limits = ui.limits;
    let (me, colors, reviewers, assignees, labels, target_remote) = (ui.me.clone(), ui.colors.clone(), d.reviewers.clone(), d.assignees.clone(), d.labels.clone(), d.target_remote.clone());
    let mut out: Vec<AnyElement> = vec![flow.into_any_element()];
    out.push(div().flex().flex_col().gap(px(5.)).min_w_0().child(field_label("Title", from_first.then(|| "from the first commit".to_string()))).children(title_el).into_any_element());
    let hint = c.templates_local.then(|| format!("Templates from the local copy of {}", d.target_branch));
    out.push(div().flex().flex_col().gap(px(5.)).min_w_0().child(field_label("Description", hint)).children(desc).into_any_element());
    let t_people = (people_row(this, tab, kind, Role::Reviewers, &reviewers, limits.as_ref(), &target_remote, None, busy, cx), people_row(this, tab, kind, Role::Assignees, &assignees, limits.as_ref(), &target_remote, me.as_ref(), busy, cx), labels_row(this, tab, kind, &labels, &colors, &target_remote, busy, cx));
    out.push(div().flex().flex_col().rounded(px(8.)).border_1().border_color(theme::section_border()).child(t_people.0).child(div().h(px(1.)).bg(theme::section_border())).child(t_people.1).child(div().h(px(1.)).bg(theme::section_border())).child(t_people.2).into_any_element());
    if kind == ForgeKind::GitLab {
        let squash = squash_toggle(&c.settings);
        let on_squash = if squash.locked { squash.value } else { d.squash == Some(true) };
        let desc_squash = squash.caption.map_or(format!("One commit on {}", d.target_branch), str::to_string);
        let w1 = weak.clone();
        let w2 = weak.clone();
        out.push(
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(field_label("When merged", None))
                .child(
                    div()
                        .flex()
                        .gap(px(16.))
                        .child(controls::switch("create-squash", on_squash, "Squash commits", Some(desc_squash.into()), squash.locked || busy, move |v, _, cx| {
                            w1.update(cx, |this, cx| this.create_update(tab, cx, move |d| d.squash = Some(v))).ok();
                        }))
                        .child(controls::switch("create-delete", d.delete_source_branch == Some(true), "Delete the source branch", Some(format!("On {}, after the merge", d.source_remote).into()), busy, move |v, _, cx| {
                            w2.update(cx, |this, cx| this.create_update(tab, cx, move |d| d.delete_source_branch = Some(v))).ok();
                        })),
                )
                .into_any_element(),
        );
    }

    // The footer's above the buttons: always in view, and the buttons never move.
    let t = this.tabs.iter().find(|t| t.id == tab).expect("the flyout's tab");
    let ui = t.create.as_ref().expect("the flyout");
    let blocked_shown = state.blocked.clone().filter(|_| state.on_remote);
    let messages = div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .text_size(px(12.))
        .children(ui.route_error.clone().map(|e| div().text_color(theme::red()).child(e)))
        .children(ui.error.clone().map(|e| div().text_color(theme::red()).child(e)))
        .children(blocked_shown.map(|b| div().text_color(theme::text_secondary()).child(b)));
    let can = state.blocked.is_none() && !busy;
    let label = if busy { "Creating…".to_string() } else if d.draft { format!("Create draft {noun}") } else { format!("Create {noun}") };
    let primary = crate::ui::anchor(
        "create-submit",
        div().id("create-submit").flex().flex_none().items_center().h(px(30.)).px(px(12.)).when(can, |e| e.bg(theme::blue()).text_color(theme::text_on_accent()).cursor_pointer()).when(!can, |e| e.bg(theme::control_bg()).text_color(theme::text_disabled())).font_weight(FontWeight::SEMIBOLD).child(label).when(can, |e| e.on_click(cx.listener(move |v, _, _, cx| v.create_submit(tab, None, cx)))),
    );
    let as_draft = d.draft;
    let wk = weak.clone();
    let caret = tip(
        "More create options",
        crate::ui::anchor(
            "create-caret",
            div().id("create-caret").flex().flex_none().items_center().h(px(30.)).px(px(8.)).border_l_1().border_color(theme::shadow_popup()).when(can, |e| e.bg(theme::blue()).cursor_pointer()).when(!can, |e| e.bg(theme::control_bg())).child(icon("chevron-down", 13., if can { theme::text_on_accent() } else { theme::text_disabled() })).when(can, |e| {
                e.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    let (w1, w2) = (wk.clone(), wk.clone());
                    let rows = vec![
                        crate::ui::menu::MenuRow::action("create.ready", format!("Create {noun}"), "git-pull-request", format!("Create the {noun}, ready for review"), move |_, cx| {
                            w1.update(cx, |v, cx| v.create_submit(tab, Some(false), cx)).ok();
                        }),
                        crate::ui::menu::MenuRow::action("create.draft", "Create as draft", "git-pull-request-draft", format!("Create the {noun} as a draft"), move |_, cx| {
                            w2.update(cx, |v, cx| v.create_submit(tab, Some(true), cx)).ok();
                        }),
                    ];
                    let o = crate::ui::menu::OpenOptions { initial: Some(if as_draft { "create.draft".into() } else { "create.ready".into() }), ..Default::default() };
                    crate::ui::menu::open_below("create-caret", rows, o, window, cx);
                })
            }),
        ),
    );
    let actions = div()
        .flex()
        .flex_1()
        .flex_wrap()
        .items_center()
        .gap(px(8.))
        .child(crate::ui::anchor("create-on-forge", ghost("create-on-forge", !busy).child(format!("Continue on {}", forge_name(kind))).child(icon("external-link", 12., theme::text_secondary())).when(!busy, |e| e.on_click(cx.listener(move |v, _, _, cx| v.create_on_forge(tab, cx))))))
        .when(ui.saved, |e| e.child(crate::ui::anchor("create-discard", ghost("create-discard", !busy).child(if ui.discard_arm { "Click again to discard" } else { "Discard draft" }).when(ui.discard_arm, |d| d.text_color(theme::red())).when(!busy, |d| d.on_click(cx.listener(move |v, _, _, cx| {
            let armed = v.tabs.iter().find(|t| t.id == tab).and_then(|t| t.create.as_ref()).is_some_and(|u| u.discard_arm);
            if armed {
                v.create_discard(tab, cx);
            } else if let Some(u) = v.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.create.as_mut()) {
                u.discard_arm = true;
                cx.notify();
            }
        }))))))
        .child(div().flex_1())
        .child(div().flex().flex_none().rounded(px(6.)).overflow_hidden().child(primary).child(caret));
    (out, div().flex().flex_1().min_w_0().flex_col().gap(px(8.)).child(messages).child(actions).into_any_element())
}

fn flow_strip(msg: AnyElement, action: Option<AnyElement>) -> AnyElement {
    div().flex().items_center().gap(px(10.)).min_h(px(42.)).pl(px(12.)).pr(px(10.)).py(px(6.)).border_t_1().border_color(theme::mix(theme::orange(), 0.35, theme::section_border())).bg(theme::mix(theme::orange(), 0.1, theme::panel_bg1())).text_size(px(12.)).text_color(theme::text_normal()).child(icon("triangle-alert", 14., theme::orange())).child(div().flex_1().min_w_0().child(msg)).children(action).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn project(kind: &str, path: &str) -> ForgeProject {
        serde_json::from_value(serde_json::json!({"kind": kind, "id": 7, "host": "forge.example.com", "path": path, "name": "p", "owner": "o", "webUrl": format!("https://forge.example.com/{path}"), "defaultBranch": "main", "cloneHttps": "", "cloneSsh": "", "forkOf": null, "updatedAt": null, "archived": false})).unwrap()
    }

    fn ctx(kind: &str, templates: Vec<MrTemplate>) -> CreateContext {
        serde_json::from_value(serde_json::json!({"project": project(kind, "g/p"), "sourceProject": "g/p", "settings": {"mergeMethods": ["merge"], "squash": "defaultOn", "deleteSourceBranch": true}, "templates": templates, "templatesLocal": false, "firstCommit": {"summary": "Add the widget", "body": "It spins.", "count": 2}})).unwrap()
    }

    fn route() -> Route {
        Route { source_remote: "origin".into(), target_remote: "origin".into(), target_branch: "main".into() }
    }

    fn template(name: &str, body: &str) -> MrTemplate {
        MrTemplate { name: name.into(), path: format!("{name}.md"), body: body.into() }
    }

    #[test]
    fn a_draft_is_prefilled_from_the_first_commit_and_the_default_template() {
        let d = fresh_draft(&ctx("gitlab", vec![template("Default", "## Why\n"), template("Bug", "b")]), &route());
        assert_eq!(d.title, "Add the widget");
        assert_eq!(d.description, "It spins.\n\n## Why");
        assert_eq!(d.template.as_deref(), Some("Default.md"));
        assert_eq!((d.squash, d.delete_source_branch), (Some(true), Some(true)));
        let gh = fresh_draft(&ctx("github", vec![template("Bug", "b"), template("Other", "o")]), &route());
        assert_eq!(gh.template, None);
        assert_eq!((gh.squash, gh.delete_source_branch), (None, None));
    }

    #[test]
    fn a_template_says_when_it_replaces_edits() {
        let c = ctx("github", vec![template("Bug", "b")]);
        let mut d = fresh_draft(&c, &route());
        assert!(!with_template(&d, &c, None).1);
        d.description = "edited".into();
        let (next, edited) = with_template(&d, &c, None);
        assert!(edited);
        assert_eq!(next.description, "It spins.");
    }

    #[test]
    fn what_blocks_create() {
        let mut d = fresh_draft(&ctx("github", Vec::new()), &route());
        assert_eq!(create_blocked(&d, "dev", false, "g/p", "g/p").as_deref(), Some("Push dev to origin first"));
        assert_eq!(create_blocked(&d, "main", true, "g/p", "g/p").as_deref(), Some("main can't target itself"));
        assert_eq!(create_blocked(&d, "main", true, "fork/p", "g/p"), None);
        d.title = "  ".into();
        assert_eq!(create_blocked(&d, "dev", true, "g/p", "g/p").as_deref(), Some("Enter a title"));
    }

    #[test]
    fn where_it_is_pushed() {
        let remotes = vec!["origin".to_string(), "up/stream".to_string()];
        assert_eq!(pushed_as(Some("up/stream/x/y"), &remotes), Some(("up/stream".into(), "x/y".into())));
        assert_eq!(pushed_as(Some("gone/x"), &remotes), None);
        let p = ("origin".to_string(), "main".to_string());
        assert_eq!(source_branch_of("feature", Some(&p), "origin", "main", true), "feature");
        assert_eq!(source_branch_of("feature", Some(&p), "origin", "dev", true), "main");
        assert_eq!(source_branch_of("feature", Some(&p), "fork", "dev", true), "feature");
    }

    #[test]
    fn what_isnt_pushed_yet() {
        assert_eq!(unpushed_count(Some(("a", None, 0)), Some(("a", "r"))), Some(0));
        assert_eq!(unpushed_count(Some(("a", Some("r"), 3)), Some(("b", "r"))), Some(3));
        assert_eq!(unpushed_count(Some(("a", Some("x"), 3)), Some(("b", "r"))), None);
        assert_eq!(unpushed_count(None, None), Some(0));
    }

    #[test]
    fn the_stack_decides_the_target() {
        let s = vec!["a".to_string(), "b".to_string()];
        assert_eq!(default_target("b", Some(&s), |_| Some("a-pushed".into()), Some("main")), "a-pushed");
        assert_eq!(default_target("a", Some(&s), |_| None, Some("main")), "main");
        assert_eq!(default_target("x", None, |_| None, None), "main");
    }

    #[test]
    fn the_request_sends_squash_only_while_the_project_leaves_it_open() {
        let mut c = ctx("gitlab", Vec::new());
        let d = fresh_draft(&c, &route());
        assert_eq!(create_request(&d, &c, "dev").squash, Some(true));
        c.settings.squash = SquashOption::Always;
        assert_eq!(create_request(&d, &c, "dev").squash, None);
    }

    #[test]
    fn the_forges_new_page_is_prefilled_where_it_can_be() {
        let mut d = fresh_draft(&ctx("gitlab", Vec::new()), &route());
        d.draft = true;
        let (u, ..) = new_mr_url(&project("gitlab", "g/p"), "g/p", "feat/a", &d);
        assert!(u.starts_with("https://forge.example.com/g/p/-/merge_requests/new?merge_request%5Bsource_branch%5D=feat%2Fa"), "{u}");
        assert!(u.contains("Draft%3A%20Add%20the%20widget"));
        let (u, ..) = new_mr_url(&project("github", "g/p"), "fork/p", "feat/a", &d);
        assert!(u.starts_with("https://forge.example.com/g/p/compare/main...fork:feat/a?expand=1"), "{u}");
        let (u, ..) = new_mr_url(&project("forgejo", "g/p"), "g/p", "x", &d);
        assert!(u.contains("title=WIP%3A%20Add"), "{u}");
        d.description = "x".repeat(MAX_URL_LENGTH * 2);
        let (u, without, more) = new_mr_url(&project("github", "g/p"), "g/p", "x", &d);
        assert!(without && !more && u.len() <= MAX_URL_LENGTH);
    }

    #[test]
    fn parts_read_as_a_list() {
        assert_eq!(parts_text(&[CreatePart::Reviewers, CreatePart::Assignees, CreatePart::Labels]), "reviewers, assignees and labels");
        assert_eq!(parts_text(&[CreatePart::Labels]), "labels");
        assert_eq!(failure_text(&[PartFailure { part: CreatePart::Reviewers, message: "Nope.".into() }]), "couldn't add reviewers: Nope");
    }

    #[test]
    fn drafts_are_capped_and_age_out() {
        let now = 1_000_000_000.;
        let mut all = serde_json::Map::new();
        for i in 0..(MAX_MR_DRAFTS + 3) {
            all.insert(format!("k{i}"), serde_json::json!({ "savedAt": now - i as f64 }));
        }
        all.insert("old".into(), serde_json::json!({ "savedAt": now - MR_DRAFT_MAX_AGE_SECS - 1. }));
        prune_drafts(&mut all, now);
        assert_eq!(all.len(), MAX_MR_DRAFTS);
        assert!(all.contains_key("k0") && !all.contains_key("old") && !all.contains_key(&format!("k{}", MAX_MR_DRAFTS + 2)));
    }
}
