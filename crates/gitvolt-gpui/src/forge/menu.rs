//! A sidebar MR/PR row's menu (`mrMenu.ts`), and the commit menu's "Open MR !12" (`entryPoints.ts`). Built from memory, like every menu;
//! requests start only on a click.

use super::text::{forge_name, mr_long_noun, mr_name, mr_noun, mr_ref, owner_of};
use crate::app::GitVolt;
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
}
