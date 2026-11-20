//! Check out an MR/PR's source branch (`checkout.ts`): from a fork without a remote, the fork's added and fetched first; a local branch
//! already tracking the source is the answer; else the remote branch, under its own name, or `<remote>-<branch>` when a local branch of that
//! name tracks something else (that one is never moved).

use crate::app::{GitVolt, Tab, core_repo};
use crate::forge::menu::unreadable_source;
use crate::forge::text::owner_of;
use crate::write::Write;
use gitvolt_core::api::Request;
use gitvolt_core::forge::types::{ForgeMr, ForgeProject};
use gpui::*;
use serde_json::Value;

fn host_name(h: &str) -> String {
    h.split(':').next().unwrap_or_default().to_lowercase()
}

fn bare_path(p: &str) -> String {
    let t = p.trim().trim_end_matches('/');
    let t = if t.len() >= 4 && t[t.len() - 4..].eq_ignore_ascii_case(".git") { &t[..t.len() - 4] } else { t };
    t.to_lowercase()
}

/// The repository's remote on the MR's source project, if it has one.
pub fn source_remote(t: &Tab, mr: &ForgeMr) -> Option<String> {
    if unreadable_source(mr) {
        return None;
    }
    let host = host_name(&t.forge.project.as_ref()?.host);
    let path = bare_path(&mr.source_project);
    t.info["remotes"].as_array()?.iter().find(|r| r["host"].as_str().map(host_name).as_deref() == Some(host.as_str()) && r["path"].as_str().map(bare_path).as_deref() == Some(path.as_str()))?["name"].as_str().map(str::to_string)
}

fn track_ref(remote: &str, mr: &ForgeMr) -> String {
    format!("refs/remotes/{remote}/{}", mr.source_branch)
}

fn locals(t: &Tab) -> &[Value] {
    t.sidebar["locals"].as_array().map_or(&[], Vec::as_slice)
}

fn tracking_local<'a>(t: &'a Tab, remote: &str, mr: &ForgeMr) -> Option<&'a Value> {
    let want = track_ref(remote, mr);
    locals(t).iter().find(|l| l["upstream"].as_str() == Some(want.as_str()))
}

fn remote_tip(t: &Tab, remote: &str, branch: &str) -> Option<String> {
    t.sidebar["remotes"].as_array()?.iter().find(|g| g["name"].as_str() == Some(remote))?["branches"].as_array()?.iter().find(|b| {
        let n = b["name"].as_str().unwrap_or_default();
        n == branch || n.strip_prefix(remote).and_then(|x| x.strip_prefix('/')) == Some(branch)
    })?["target"].as_str().map(str::to_string)
}

/// The button's label, and why it's disabled.
pub fn checkout_state(t: &Tab, mr: &ForgeMr) -> (String, Option<String>) {
    if unreadable_source(mr) {
        return ("Check out".into(), Some("The source project isn't readable".into()));
    }
    let Some(remote) = source_remote(t, mr) else {
        let owner = owner_of(&mr.source_project);
        return (format!("Add {}'s fork and check out", owner.rsplit('/').next().unwrap_or(owner)), None);
    };
    let head = locals(t).iter().find(|l| l["isHead"].as_bool() == Some(true));
    if let Some(h) = head
        && h["upstream"].as_str() == Some(track_ref(&remote, mr).as_str())
    {
        return ("Checked out".into(), Some(format!("{} is checked out", h["name"].as_str().unwrap_or_default())));
    }
    ("Check out".into(), None)
}

/// Why the MR's branch can't go into a new worktree, from what's loaded.
pub fn worktree_blocked(t: &Tab, mr: &ForgeMr) -> Option<String> {
    if unreadable_source(mr) {
        return Some("The source project isn't readable".into());
    }
    let remote = source_remote(t, mr)?;
    let local = tracking_local(t, &remote, mr)?;
    let at = local["checkedOut"].as_str()?;
    Some(format!("{} is checked out in {}", local["name"].as_str().unwrap_or_default(), crate::sidebar::tilde(at)))
}

/// Where the source branch lands locally.
#[derive(Debug, Clone, PartialEq)]
pub enum MrBranch {
    Local { name: String },
    Remote { remote: String, name: String, tip: String },
}

/// The second half of `resolveMrBranch`: from what the sidebar says now.
pub fn branch_from_sidebar(t: &Tab, remote: &str, mr: &ForgeMr) -> Option<MrBranch> {
    if let Some(l) = tracking_local(t, remote, mr) {
        return Some(MrBranch::Local { name: l["name"].as_str().unwrap_or_default().to_string() });
    }
    let tip = remote_tip(t, remote, &mr.source_branch)?;
    if !locals(t).iter().any(|l| l["name"].as_str() == Some(mr.source_branch.as_str())) {
        return Some(MrBranch::Remote { remote: remote.to_string(), name: mr.source_branch.clone(), tip });
    }
    let alt = format!("{remote}-{}", mr.source_branch);
    if let Some(l) = locals(t).iter().find(|l| l["name"].as_str() == Some(alt.as_str()))
        && l["upstream"].as_str() == Some(track_ref(remote, mr).as_str())
    {
        return Some(MrBranch::Local { name: alt });
    }
    Some(MrBranch::Remote { remote: remote.to_string(), name: alt, tip })
}

/// `base` as a remote name (lowercase, unsafe characters as `-`), or `base-2`, `base-3`… when taken.
pub fn free_remote_name(base: &str, taken: &[String]) -> String {
    let mut clean = String::new();
    for c in base.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            clean.push(c);
        } else if !clean.ends_with('-') {
            clean.push('-');
        }
    }
    let clean = clean.trim_matches(['-', '.']);
    let clean = if clean.is_empty() { "remote" } else { clean };
    if !taken.iter().any(|t| t == clean) {
        return clean.to_string();
    }
    (2..1000).map(|n| format!("{clean}-{n}")).find(|c| !taken.contains(c)).unwrap_or_else(|| format!("{clean}-x"))
}

/// How to reach a fork: over SSH when origin is reached over SSH, else HTTPS.
pub fn fork_clone_url(origin: Option<&str>, fork: &ForgeProject) -> String {
    let o = origin.unwrap_or_default().trim();
    let lower = o.to_lowercase();
    let has_scheme = o.split_once("://").is_some_and(|(s, _)| !s.is_empty() && s.starts_with(|c: char| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)));
    let scp = !has_scheme && !o.starts_with('/') && !o.starts_with('.') && o.split_once(':').is_some_and(|(l, r)| !l.contains('/') && !l.is_empty() && !r.is_empty());
    if (lower.starts_with("ssh://") || scp) && !fork.clone_ssh.is_empty() { fork.clone_ssh.clone() } else { fork.clone_https.clone() }
}

impl GitVolt {
    /// `addForkRemote`: `fork` as a remote named after its owner, reached the way origin is, then fetched. A remote already on that
    /// project is reused. `then` gets its name.
    pub fn add_fork_remote(&mut self, tab: u32, fork: ForgeProject, cx: &mut Context<Self>, then: impl FnOnce(&mut GitVolt, String, &mut Context<GitVolt>) + 'static) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let remotes = t.info["remotes"].as_array().map_or(&[][..], Vec::as_slice);
        let is_project = |r: &Value, host: &str, path: &str| r["host"].as_str().map(host_name).as_deref() == Some(host_name(host).as_str()) && r["path"].as_str().unwrap_or_default().eq_ignore_ascii_case(path);
        if let Some(name) = remotes.iter().find(|r| is_project(r, &fork.host, &fork.path)).and_then(|r| r["name"].as_str()) {
            return then(self, name.to_string(), cx);
        }
        let owner = fork.owner.rsplit('/').next().filter(|o| !o.is_empty()).unwrap_or(&fork.owner).to_string();
        let taken: Vec<String> = remotes.iter().filter_map(|r| r["name"].as_str().map(str::to_string)).collect();
        let name = free_remote_name(&owner, &taken);
        let origin = remotes.iter().find(|r| r["name"].as_str() == Some("origin")).or_else(|| fork.fork_of.as_deref().and_then(|p| remotes.iter().find(|r| is_project(r, &fork.host, p)))).or(remotes.first()).and_then(|r| r["url"].as_str());
        let url = fork_clone_url(origin, &fork);
        let ctx = t.write_target();
        let (repo, worktree, n) = (core_repo(tab), ctx.worktree.clone(), name.clone());
        let (fetch_ctx, added, fetched) = (ctx.clone(), name.clone(), name.clone());
        let then = std::cell::RefCell::new(Some(then));
        let w = Write::new(&ctx, move |_| crate::write::call("addRemote", serde_json::json!({ "repo": repo, "worktree": worktree, "name": n, "url": url }))).on_success(move |v, _, cx| {
            crate::ui::toast::show(format!("Added {owner}'s fork as {added}"), cx);
            let (r, name, then2) = (added.clone(), fetched.clone(), std::cell::RefCell::new(then.borrow_mut().take()));
            v.write(
                Write::new(&fetch_ctx, move |_| crate::write::call("fetch", serde_json::json!({ "repo": repo, "background": false, "remote": r }))).on_end(move |v, _, cx| {
                    let (name, then) = (name.clone(), then2.borrow_mut().take());
                    v.reload_then(tab, cx, move |v, cx| {
                        if let Some(then) = then {
                            then(v, name, cx);
                        }
                    });
                }),
                cx,
            );
        });
        self.write(w, cx);
    }

    /// `resolveMrBranch`: `then` gets the branch, or a toast said why it can't.
    fn mr_resolve_branch(&mut self, tab: u32, mr: ForgeMr, cx: &mut Context<Self>, then: impl FnOnce(&mut GitVolt, MrBranch, &mut Context<GitVolt>) + 'static) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        if unreadable_source(&mr) {
            return self.mr_end_checking(tab, mr.number);
        }
        match source_remote(t, &mr) {
            Some(remote) => self.mr_branch_on_remote(tab, remote, mr, cx, then),
            None => {
                let path = mr.source_project.clone();
                self.forge_ask::<ForgeProject>(Request::ForgeProjectByPath { repo: core_repo(tab), path: path.clone() }, cx, move |v, answer, cx| match answer {
                    Err(e) => {
                        crate::ui::toast::show_error(format!("Couldn't find {path}: {e}"), cx);
                        v.mr_end_checking(tab, mr.number);
                    }
                    Ok(fork) => v.add_fork_remote(tab, fork, cx, move |v, remote, cx| v.mr_branch_on_remote(tab, remote, mr, cx, then)),
                });
            }
        }
    }

    fn mr_branch_on_remote(&mut self, tab: u32, remote: String, mr: ForgeMr, cx: &mut Context<Self>, then: impl FnOnce(&mut GitVolt, MrBranch, &mut Context<GitVolt>) + 'static) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        if let Some(b) = branch_from_sidebar(t, &remote, &mr) {
            return then(self, b, cx);
        }
        // Not fetched yet: that remote first.
        let ctx = t.write_target();
        let (r2, fetched) = (remote.clone(), remote.clone());
        let repo = core_repo(tab);
        let then = std::cell::RefCell::new(Some(then));
        self.write(
            Write::new(&ctx, move |_| crate::write::call("fetch", serde_json::json!({ "repo": repo, "background": false, "remote": r2 }))).on_end(move |v, _, cx| {
                let mr = mr.clone();
                let fetched = fetched.clone();
                let then = then.borrow_mut().take();
                v.reload_then(tab, cx, move |v, cx| {
                    let Some(t) = v.tabs.iter().find(|t| t.id == tab) else { return };
                    match (branch_from_sidebar(t, &fetched, &mr), then) {
                        (Some(b), Some(then)) => then(v, b, cx),
                        _ => {
                            crate::ui::toast::show(format!("{fetched}/{} isn't on the remote any more", mr.source_branch), cx);
                            v.mr_end_checking(tab, mr.number);
                        }
                    }
                });
            }),
            cx,
        );
    }

    pub fn mr_end_checking(&mut self, tab: u32, number: u64) {
        if let Some(ui) = self.mr_ui(tab, number)
            && ui.busy == Some(super::actions::Busy::CheckingOut)
        {
            ui.busy = None;
        }
    }

    /// Check out the MR's source branch; the branch's tip is then selected in the graph.
    pub fn mr_checkout(&mut self, tab: u32, mr: ForgeMr, cx: &mut Context<Self>) {
        let source = mr.source_branch.clone();
        let number = mr.number;
        self.mr_resolve_branch(tab, mr, cx, move |v, b, cx| {
            let name = match &b {
                MrBranch::Local { name } | MrBranch::Remote { name, .. } => name.clone(),
            };
            // The write refreshes the tab: the branch is HEAD now, so its tip is selected. A failure leaves the selection alone.
            if let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) {
                t.after_reload.push(Box::new(move |v, cx| {
                    v.mr_end_checking(tab, number);
                    let tip = v.tabs.iter().find(|t| t.id == tab).and_then(|t| locals(t).iter().find(|l| l["name"].as_str() == Some(name.as_str()) && l["isHead"].as_bool() == Some(true)).and_then(|l| l["target"].as_str()).map(str::to_string));
                    if let Some(tip) = tip
                        && v.in_graph(&tip)
                    {
                        v.go_to_sha(&tip, cx);
                    }
                }));
            }
            match b {
                MrBranch::Local { name } => v.checkout_local(&name, cx),
                MrBranch::Remote { remote, name, tip } => v.checkout_remote_as(&remote, &source, &tip, (name != source).then_some(name.as_str()), cx),
            }
        });
    }

    /// Check out in a new worktree: the Create worktree dialog, pre-filled with that branch.
    pub fn mr_checkout_in_worktree(&mut self, tab: u32, mr: ForgeMr, _window: &mut Window, cx: &mut Context<Self>) {
        let source = mr.source_branch.clone();
        let head = mr.head_sha.clone().unwrap_or_default();
        self.mr_resolve_branch(tab, mr, cx, move |v, b, cx| {
            let kind = match b {
                MrBranch::Remote { remote, name, tip } => serde_json::json!({ "kind": "remote", "remote": remote, "branch": source, "name": name, "at": tip }),
                MrBranch::Local { name } => {
                    let Some(t) = v.tabs.iter().find(|t| t.id == tab) else { return };
                    let local = locals(t).iter().find(|l| l["name"].as_str() == Some(name.as_str()));
                    if let Some(at) = local.and_then(|l| l["checkedOut"].as_str()) {
                        crate::ui::toast::show(format!("{name} is checked out in {at}"), cx);
                        return;
                    }
                    let at = local.and_then(|l| l["target"].as_str()).map_or(head.clone(), str::to_string);
                    serde_json::json!({ "kind": "existing", "name": name, "at": at })
                }
            };
            let weak = cx.weak_entity();
            v.with_window(cx, move |window, cx| {
                weak.update(cx, |v, cx| v.open_create_worktree(kind, window, cx)).ok();
            });
        });
    }
}
