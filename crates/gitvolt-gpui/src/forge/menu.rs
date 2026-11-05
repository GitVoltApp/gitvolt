//! A sidebar MR/PR row's menu (`mrMenu.ts`), and the commit menu's "Open MR !12" (`entryPoints.ts`). Built from memory, like every menu;
//! requests start only on a click.

use super::urls::{self, ProjectRemote};
use super::text::{forge_name, mr_long_noun, mr_name, mr_noun, mr_ref, owner_of};
use crate::app::{GitVolt, Tab};
use crate::model::Chip;
use crate::ui::menu::{MenuRow, Variant};
use gitvolt_core::forge::types::{ForgeMr, MrState};
use gitvolt_core::forge::ForgeKind;
use gpui::*;

pub const NOT_LOCAL: &str = "Not in this repository yet: fetch or check out first";

fn bare(p: &str) -> String {
    p.trim().trim_end_matches('/').trim_end_matches(".git").to_lowercase()
}

/// The MR's source project couldn't be read: GitHub's "" (a deleted fork), GitLab's `project <id>`.
pub fn unreadable_source(mr: &ForgeMr) -> bool {
    mr.source_project.is_empty() || mr.source_project.strip_prefix("project ").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

fn copy_run(text: String) -> impl Fn(&mut Window, &mut App) + 'static {
    move |_, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        crate::ui::toast::show("Copied", cx);
    }
}

fn open_run(this: &WeakEntity<GitVolt>, url: String) -> impl Fn(&mut Window, &mut App) + 'static {
    let w = this.clone();
    move |_, cx| {
        let url = url.clone();
        w.update(cx, |v, _| v.forge_open_url(url)).ok();
    }
}

impl GitVolt {
    /// A link through the core, which opens only `http`, `https` and `mailto`.
    pub fn forge_open_url(&self, url: String) {
        let api = self.api.clone();
        self.rt.spawn(async move { api.dispatch(gitvolt_core::api::Request::OpenUrl { url }).await });
    }

    pub fn mr_rows(&self, mr: &ForgeMr, kind: ForgeKind, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(t) = self.tabs.get(self.active) else { return Vec::new() };
        let tab = t.id;
        let number = mr.number;
        let r = mr_ref(kind, number);
        let w = this.clone();
        let mut rows = vec![MenuRow::action("mr.open", format!("Open {}", mr_long_noun(kind)), "git-pull-request", format!("Show {r} in its view"), move |_, cx| {
            w.update(cx, |v, cx| v.forge_open_mr(tab, number, cx)).ok();
        })];
        rows.push(MenuRow::Separator);
        let (label, disabled) = crate::forge::mrview::checkout::checkout_state(t, mr);
        let blocked = crate::forge::mrview::checkout::worktree_blocked(t, mr);
        let fork = !unreadable_source(mr) && crate::forge::mrview::checkout::source_remote(t, mr).is_none();
        let suffix = if fork { ", adding the fork as a remote first" } else { "" };
        let (w, m) = (this.clone(), mr.clone());
        rows.push(
            MenuRow::action("mr.checkout", label, "git-branch", disabled.clone().unwrap_or_else(|| format!("Check out {}{suffix}", mr.source_branch)), move |_, cx| {
                let m = m.clone();
                w.update(cx, |v, cx| v.mr_checkout(tab, m, cx)).ok();
            })
            .disabled_if(disabled),
        );
        let (w, m) = (this.clone(), mr.clone());
        rows.push(
            MenuRow::action("mr.checkoutWorktree", "Check out in a new worktree…", "folder-plus", blocked.clone().unwrap_or_else(|| format!("Check out {} in a new directory{suffix}", mr.source_branch)), move |window, cx| {
                let m = m.clone();
                w.update(cx, |v, cx| v.mr_checkout_in_worktree(tab, m, window, cx)).ok();
            })
            .disabled_if(blocked),
        );
        rows.push(MenuRow::Separator);
        let in_graph = mr.head_sha.as_ref().is_some_and(|s| t.graph.as_ref().is_some_and(|g| g.by_sha.contains_key(s)));
        let (w, sha) = (this.clone(), mr.head_sha.clone().unwrap_or_default());
        rows.push(
            MenuRow::action("mr.showInGraph", "Show in graph", "git-graph", if in_graph { format!("Select {r}'s head commit in the graph") } else { NOT_LOCAL.to_string() }, move |_, cx| {
                let sha = sha.clone();
                w.update(cx, |v, cx| v.go_to_sha(&sha, cx)).ok();
            })
            .disabled_if((!in_graph).then_some(NOT_LOCAL)),
        );
        rows.push(MenuRow::Separator);
        let forked = !unreadable_source(mr) && bare(&mr.source_project) != bare(&mr.target_project);
        let owned = format!("{}:{}", owner_of(&mr.source_project), mr.source_branch);
        rows.push(MenuRow::action("mr.copyLink", "Copy link", "link", format!("Copy {r}'s web address"), copy_run(mr.web_url.clone())).variants(vec![Variant { id: "open".into(), label: None, icon: Some("external-link"), tooltip: format!("Open {r} on {} in the browser", forge_name(kind)).into(), run: std::rc::Rc::new(open_run(this, mr.web_url.clone())), disabled: None }], None));
        let branch = MenuRow::action("mr.copyBranch", "Copy branch name", "git-branch", format!("Copy \"{}\"", mr.source_branch), copy_run(mr.source_branch.clone()));
        rows.push(if forked { branch.variants(vec![Variant { id: "owner".into(), label: Some("owner:branch".into()), icon: None, tooltip: format!("Copy \"{owned}\"").into(), run: std::rc::Rc::new(copy_run(owned)), disabled: None }], None) } else { branch });
        rows.push(MenuRow::action("mr.copyNumber", "Copy number", "hash", format!("Copy \"{r}\""), copy_run(r.clone())));
        // Draft ⇄ ready: the author's own open MR/PR only (merge and approve stay in its view).
        if t.forge.me.as_deref() == Some(mr.author.username.as_str()) && matches!(mr.state, MrState::Open | MrState::Draft) {
            let draft = mr.state == MrState::Draft;
            let (w, m) = (this.clone(), mr.clone());
            rows.push(MenuRow::Separator);
            rows.push(MenuRow::action("mr.draft", if draft { "Mark as ready" } else { "Convert to draft" }, if draft { "git-pull-request" } else { "git-pull-request-draft" }, if draft { format!("Mark {r} as ready for review") } else { format!("Mark {r} as a draft") }, move |_, cx| {
                let m = m.clone();
                w.update(cx, |v, cx| v.mr_toggle_draft(tab, kind, m, cx)).ok();
            }));
        }
        rows
    }

    /// "Open MR !12" on a branch's menu and its last commit's, before the message's own rows, which open the browser.
    pub fn commit_mr_rows(&self, chips: &[Chip], this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(t) = self.tabs.get(self.active) else { return Vec::new() };
        let Some(kind) = t.forge.kind else { return Vec::new() };
        let tab = t.id;
        let mut seen: Vec<u64> = Vec::new();
        let mut rows = Vec::new();
        for c in chips.iter().filter(|c| !c.tag) {
            let Some((mr, _)) = t.forge.mr_for_chip(c) else { continue };
            if seen.contains(&mr.number) {
                continue;
            }
            seen.push(mr.number);
            let (w, number) = (this.clone(), mr.number);
            rows.push(MenuRow::action(format!("commit.openMr.{number}"), format!("Open {} {}", mr_noun(kind), mr_ref(kind, number)), "git-pull-request", format!("Show {} {} ({}) in GitVolt", mr_name(kind).to_lowercase(), mr_ref(kind, number), mr.title), move |_, cx| {
                w.update(cx, |v, cx| v.forge_open_mr(tab, number, cx)).ok();
            }));
        }
        rows
    }

    fn forge_of(&self, remote: Option<&str>) -> Option<(&Tab, ProjectRemote)> {
        let t = self.tabs.get(self.active)?;
        Some((t, urls::project_remote(t, &self.profile["hostOverrides"], remote)?))
    }

    /// `Open !12`: an MR/PR named in the commit's message, on the forge, in the browser.
    pub fn commit_ref_rows(&self, refs: &[String], this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some((_, f)) = self.forge_of(None) else { return Vec::new() };
        let name = forge_name(f.kind);
        let mut seen: Vec<String> = Vec::new();
        let mut rows = Vec::new();
        for label in refs.iter().take(MAX_MESSAGE_REFS) {
            let Some(url) = urls::mr_ref_url(label, &f) else { continue };
            if seen.contains(&url) {
                continue;
            }
            seen.push(url.clone());
            rows.push(MenuRow::action(format!("commit.openRef.{label}"), format!("Open {label}"), "git-pull-request", format!("Open {} {label} on {name}", mr_long_noun(f.kind)), open_run(this, url)));
        }
        rows
    }

    /// `Create merge request…`: a local branch with none open yet.
    pub fn create_mr_rows(&self, branch: Option<&Chip>, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(c) = branch.filter(|c| c.local) else { return Vec::new() };
        let name = c.name.to_string();
        let Some(kind) = self.create_target(Some(&name), Some(c)) else { return Vec::new() };
        let noun = mr_long_noun(kind);
        let w = this.clone();
        vec![MenuRow::action("forge.createMr", format!("Create {noun}…"), "git-pull-request-create", format!("Create a {noun} from {name} on {}", forge_name(kind)), move |window, cx| {
            let name = name.clone();
            w.update(cx, |v, cx| v.forge_open_create(name, window, cx)).ok();
        })]
    }

    /// The commit's `Forge link`, last of the copy group.
    pub fn commit_forge_row(&self, sha: &str, branch: Option<&Chip>, this: &WeakEntity<GitVolt>) -> Option<MenuRow> {
        let remote_branch = branch.and_then(|b| {
            let mut all: Vec<_> = b.remotes.iter().collect();
            all.sort_by_key(|(r, _)| r != "origin");
            let (r, full) = all.first()?;
            Some((r.clone(), full.strip_prefix(&format!("refs/remotes/{r}/"))?.to_string()))
        });
        let (_, f) = self.forge_of(remote_branch.as_ref().map(|b| b.0.as_str()))?;
        let name = forge_name(f.kind);
        let branch_link = remote_branch.map(|b| urls::branch_url(&f, &b.1));
        let commit_link = urls::commit_url(&f, sha);
        let url = branch_link.clone().unwrap_or_else(|| commit_link.clone());
        let mut variants = Vec::new();
        if let Some(b) = &branch_link {
            variants.push(icon_variant("branch", "git-branch", format!("Copy the branch's link on {name}"), copy_link(this, b.clone())));
        }
        variants.push(icon_variant("commit", "git-commit-horizontal", format!("Copy a {name} permalink pinned to this commit"), copy_link(this, commit_link)));
        variants.push(icon_variant("open", "square-arrow-out-up-right", format!("Open on {name} in the browser"), open_run(this, url.clone())));
        let what = if branch_link.is_some() { "the branch" } else { "this commit" };
        Some(MenuRow::action("commit.forgeLink", "Forge link", "external-link", format!("Copy the link to {what} on {name}"), copy_link(this, url)).variants(variants, None))
    }

    /// A tag's page.
    pub fn tag_forge_rows(&self, tag: &str, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some((_, f)) = self.forge_of(None) else { return Vec::new() };
        vec![simple_row("tag.forgeLink", urls::branch_url(&f, tag), &format!("the tag's page on {}", forge_name(f.kind)), this)]
    }

    /// A remote's project page, and "Use as the main remote".
    pub fn remote_forge_rows(&self, remote: &str, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some((t, f)) = self.forge_of(Some(remote)) else { return Vec::new() };
        let mut rows = vec![simple_row("sidebar.forgeLink", urls::repo_url(&f), &format!("the project on {}", forge_name(f.kind)), this)];
        let on = t.forge.target_chosen && t.forge.target.as_deref() == Some(remote);
        let tip = if on { "The main remote: click to go back to Automatic".to_string() } else { format!("Make this the remote whose project is used for {}", mr_long_noun(f.kind).to_owned() + "s") };
        let (w, name) = (this.clone(), remote.to_string());
        rows.push(MenuRow::action("sidebar.mainRemote", "Use as the main remote", if on { "check" } else { "flag" }, tip, move |_, cx| {
            let name = name.clone();
            w.update(cx, |v, cx| v.forge_toggle_main_remote(name, cx)).ok();
        }));
        rows
    }

    fn forge_toggle_main_remote(&mut self, remote: String, cx: &mut Context<Self>) {
        let Some(path) = self.tabs.get(self.active).map(|t| t.repo_path.clone()) else { return };
        self.update_repo(
            &path,
            |r| {
                let same = r["forgeTargetRemote"].as_str() == Some(remote.as_str());
                r["forgeTargetRemote"] = if same { serde_json::Value::Null } else { serde_json::json!(remote) };
            },
            cx,
        );
        self.forge_accounts_changed(cx);
    }

    /// The file's `Forge link` (the branch's page, a permalink, or Open). `at`: the commit whose branch the link goes by.
    pub fn file_forge_rows(&self, path: &str, sha: Option<&str>, at: Option<&str>, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let up = at.and_then(|a| urls::upstream_of(tab, a));
        let Some((_, f)) = self.forge_of(up.as_ref().map(|u| u.0.as_str())) else { return Vec::new() };
        let name = forge_name(f.kind);
        let branch = up.as_ref().map(|u| urls::file_url(&f, &u.1, path, None));
        let commit = sha.map(|s| urls::file_url(&f, s, path, None));
        let url = branch.clone().or_else(|| commit.clone());
        let branch_reason = if sha.is_some() { "The commit's branch has no known upstream" } else { "The working tree's branch has no known upstream" };
        let commit_reason = "Not committed yet";
        let reason = format!("{branch_reason}, and the file isn't committed yet");
        let which = match (&branch, &up) {
            (Some(_), Some(u)) => format!("the file on {}/{}", u.0, u.1),
            _ => "the permalink".to_string(),
        };
        let mut b = icon_variant(
            "branch",
            "git-branch",
            up.as_ref().map_or(format!("Copy the link to the file on its branch ({name})"), |u| format!("Copy the link to the file on {}/{} ({name})", u.0, u.1)),
            copy_link(this, branch.clone().unwrap_or_default()),
        );
        b.disabled = branch.is_none().then(|| branch_reason.into());
        let mut c = icon_variant(
            "commit",
            "git-commit-horizontal",
            sha.map_or(format!("Copy a {name} permalink pinned to the commit"), |s| format!("Copy a {name} permalink pinned to commit {}", &s[..s.len().min(7)])),
            copy_link(this, commit.clone().unwrap_or_default()),
        );
        c.disabled = commit.is_none().then(|| commit_reason.into());
        let mut o = icon_variant("open", "square-arrow-out-up-right", format!("Open {which} on {name} in the browser"), open_run(this, url.clone().unwrap_or_default()));
        o.disabled = url.is_none().then(|| reason.clone().into());
        let tip = if url.is_some() { format!("Copy the link to {which} on {name}") } else { reason.clone() };
        let row = MenuRow::action("file.forge", "Forge link", "external-link", tip, copy_link(this, url.clone().unwrap_or_default())).variants(vec![b, c, o], None);
        vec![row.disabled_if(url.is_none().then_some(reason))]
    }

    /// The diff's `Forge link`: the file at the lines. Nothing where no link can be made.
    pub fn monaco_forge_rows(&self, path: &str, sha: Option<&str>, at: Option<&str>, lines: (u32, u32), this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let up = at.and_then(|a| urls::upstream_of(tab, a));
        let Some((_, f)) = self.forge_of(up.as_ref().map(|u| u.0.as_str())) else { return Vec::new() };
        let name = forge_name(f.kind);
        let branch = up.as_ref().map(|u| urls::file_url(&f, &u.1, path, Some(lines)));
        let commit = sha.map(|s| urls::file_url(&f, s, path, Some(lines)));
        let Some(url) = branch.clone().or_else(|| commit.clone()) else { return Vec::new() };
        let mut b = icon_variant(
            "branch",
            "git-branch",
            up.as_ref().map_or(format!("Copy the branch's link on {name}"), |u| format!("Copy the link to this file on {}/{} ({name})", u.0, u.1)),
            copy_link(this, branch.clone().unwrap_or_default()),
        );
        b.disabled = branch.is_none().then(|| "The commit's branch has no known upstream".into());
        let mut c = icon_variant("commit", "git-commit-horizontal", format!("Copy a {name} permalink pinned to this commit"), copy_link(this, commit.clone().unwrap_or_default()));
        c.disabled = commit.is_none().then(|| "Not committed yet".into());
        let o = icon_variant("open", "square-arrow-out-up-right", format!("Open on {name} in the browser"), open_run(this, url.clone()));
        vec![MenuRow::action("monaco.forgeLink", "Forge link", "external-link", format!("Copy the link to this file on {name}"), copy_link(this, url)).variants(vec![b, c, o], None)]
    }
}

/// Far more than a message's references would ever open.
const MAX_MESSAGE_REFS: usize = 50;

fn copy_link(this: &WeakEntity<GitVolt>, text: String) -> impl Fn(&mut Window, &mut App) + 'static {
    let w = this.clone();
    move |_, cx| {
        let text = text.clone();
        w.update(cx, |v, cx| v.copy_text(text, cx)).ok();
    }
}

fn icon_variant(id: &str, icon: &'static str, tooltip: String, run: impl Fn(&mut Window, &mut App) + 'static) -> Variant {
    Variant { id: id.to_string().into(), label: None, icon: Some(icon), tooltip: tooltip.into(), run: std::rc::Rc::new(run), disabled: None }
}

fn simple_row(id: &'static str, url: String, what: &str, this: &WeakEntity<GitVolt>) -> MenuRow {
    let open = icon_variant("open", "square-arrow-out-up-right", format!("Open {what} in the browser"), open_run(this, url.clone()));
    MenuRow::action(id, "Forge link", "external-link", format!("Copy {what}"), copy_link(this, url)).variants(vec![open], None)
}
