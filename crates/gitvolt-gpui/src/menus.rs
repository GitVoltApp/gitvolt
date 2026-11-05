//! What each right-click shows (`ui/src/menu/builders.ts`, `menuEnv.ts`, each feature's `menus.ts`):
//! labels, groups and order as in the web UI. Rows whose action isn't ported yet (interactive rebase,
//! worktrees, LFS, file colours, presets) stay in their place, disabled (`MenuRow::pending`). No forge
//! rows: there's no forge layer here yet.

use crate::app::GitVolt;
use crate::details::{FileChange, Origin};
use crate::model::{Chip, RowView, SHORT_SHA};
use crate::open_in::{Opener, menu_rows, opener_icon, opener_label, row_disabled};
use crate::ui::menu::{MenuRow, Variant, grouped};
use gitvolt_graph::NodeKind;
use gpui::prelude::*;
use gpui::*;
use std::rc::Rc;

fn short(sha: &str) -> String {
    sha.chars().take(SHORT_SHA).collect()
}

fn short7(sha: &str) -> String {
    sha.chars().take(7).collect()
}

fn variant(id: &str, label: &str, tooltip: impl Into<SharedString>, run: impl Fn(&mut Window, &mut App) + 'static) -> Variant {
    Variant { id: id.to_string().into(), label: Some(label.to_string().into()), icon: None, tooltip: tooltip.into(), run: Rc::new(run), disabled: None }
}

fn copier(this: &WeakEntity<GitVolt>, text: String) -> impl Fn(&mut Window, &mut App) + 'static {
    let this = this.clone();
    move |_, cx| {
        let text = text.clone();
        this.update(cx, |v, cx| v.copy_text(text, cx)).ok();
    }
}

fn create_file_row(this: &WeakEntity<GitVolt>) -> MenuRow {
    let weak = this.clone();
    MenuRow::action("files.createFile", "Create file…", "file-plus", "Create an empty file in the working tree", move |window, cx| {
        weak.update(cx, |v, cx| v.open_create_file(window, cx)).ok();
    })
}

fn copy_sha_row(id: &str, sha: &str, this: &WeakEntity<GitVolt>) -> MenuRow {
    MenuRow::action(id.to_string(), "Copy SHA", "hash", "Copy the full commit id", copier(this, sha.to_string())).variants(
        vec![
            variant("short", "Short", format!("Copy the short id ({})", short(sha)), copier(this, short(sha))),
            variant("full", "Full", "Copy the full 40-character id", copier(this, sha.to_string())),
        ],
        None,
    )
}

fn show_in_graph(sha: Option<&str>, loaded: bool, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
    let Some(sha) = sha else { return Vec::new() };
    let (sha, weak) = (sha.to_string(), this.clone());
    vec![
        MenuRow::action("sidebar.showInGraph", "Show in graph", "git-graph", "Select this commit in the graph", move |_, cx| {
            let sha = sha.clone();
            weak.update(cx, |v, cx| v.go_to_sha(&sha, cx)).ok();
        })
        .disabled_if((!loaded).then_some("Not in the loaded history")),
    ]
}

/// The branch a commit's menu is about: the clicked chip, else its primary one.
pub fn primary_chip(row: &RowView) -> Option<&Chip> {
    row.chips.iter().find(|c| c.head).or_else(|| row.chips.iter().find(|c| !c.tag))
}

impl GitVolt {
    /// A commit's menu (a graph row, a branch chip, a sidebar branch).
    pub fn commit_rows(&self, row_ix: usize, chip: Option<&Chip>, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let Some(graph) = tab.graph.as_ref() else { return Vec::new() };
        let Some(row) = graph.rows.get(row_ix) else { return Vec::new() };
        if row.kind == NodeKind::Wip {
            return self.wip_rows(this, row);
        }
        let sha = row.id.to_string();
        let head_sha = graph.head_row.and_then(|i| graph.rows.get(i)).map(|r| r.id.to_string());
        let is_head_commit = head_sha.as_deref() == Some(sha.as_str());
        let stash = row.kind == NodeKind::Stash;
        let branch = chip.or_else(|| primary_chip(row)).filter(|c| !c.tag && !stash);
        let local = branch.filter(|c| c.local);
        let (s6, s7) = (short(&sha), short7(&sha));
        let summary = row.summary.to_string();
        let target = branch.map_or(s6.clone(), |b| b.name.to_string());

        let ctx = tab.write_target();
        let local_b = local.and_then(|c| tab.local_branch(&c.name));
        let head_for = tab.head_branch();
        let head = head_for.clone().unwrap_or_else(|| "HEAD".to_string());
        let head_sha_now = tab.head_sha();
        let is_head_commit = head_sha_now.as_deref() == Some(sha.as_str()) || is_head_commit;
        let busy = tab.busy_reason();

        let mut sync = Vec::new();
        if let Some(b) = &local_b {
            sync.extend(crate::sync::sync_rows(self, b, this));
            let (weak, c, name) = (this.clone(), ctx.clone(), b.name.clone());
            sync.push(MenuRow::action("branch.setUpstream", "Set upstream", "arrow-up-right", format!("Choose the remote branch {name} tracks"), move |_, cx| {
                let (c, name) = (c.clone(), name.clone());
                weak.update(cx, |v, cx| v.pick_upstream(c, name, cx)).ok();
            }));
        }
        if !stash && !is_head_commit {
            let reset = |mode: &'static str| -> crate::ui::Action {
                let (weak, to) = (this.clone(), sha.clone());
                Rc::new(move |_: &mut Window, cx: &mut App| {
                    let to = to.clone();
                    let origin = crate::ui::arm::Origin::None;
                    weak.update(cx, |v, cx| v.reset_to(to, mode, origin, cx)).ok();
                })
            };
            let v = |id: &str, label: &str, tip: &str, run: crate::ui::Action| Variant { id: id.to_string().into(), label: Some(label.to_string().into()), icon: None, tooltip: tip.to_string().into(), run, disabled: busy.clone().map(Into::into) };
            let mixed = reset("mixed");
            sync.push(
                MenuRow::action("commit.reset", format!("Reset {head} to this commit"), "rotate-ccw", format!("Move {head} to {s7}, keeping the working copy but resetting the index"), move |w, cx| mixed(w, cx))
                    .variants(vec![v("soft", "Soft", "keep all changes", reset("soft")), v("mixed", "Mixed", "keep working copy but reset index", reset("mixed")), v("hard", "Hard", "discard all changes", reset("hard"))], Some("mixed"))
                    .disabled_if(busy.clone()),
            );
        }

        let mut integrate = Vec::new();
        if let Some(b) = branch {
            let why = busy.clone().or_else(|| head_for.is_none().then(|| "Check out a branch first".to_string()));
            let env = crate::refs::IntegrateEnv { head_branch: head_for.clone(), head_sha: head_sha_now.clone(), busy: why };
            let checked_out = local_b.as_ref().is_some_and(|l| l.checked_out.is_some());
            integrate = crate::refs::integrate_rows(&sha, b, checked_out, graph, &env, this, &ctx);
            integrate.extend(self.rebase_onto_rows(&sha, b, None, this));
        }

        let mut branch_rows = Vec::new();
        let mut commit_rows = Vec::new();
        if !stash {
            let mut checkout = Vec::new();
            let mut worktree = Vec::new();
            for c in row.chips.iter().filter(|c| !c.tag) {
                if c.local && !c.head {
                    let (weak, chip, id) = (this.clone(), c.clone(), sha.clone());
                    checkout.push(MenuRow::action(format!("checkout:{}", c.name), c.name.clone(), "git-branch", format!("Check out {}", c.name), move |_, cx| {
                        let (chip, id) = (chip.clone(), id.clone());
                        weak.update(cx, |v, cx| v.checkout_chip(&id, &chip, cx)).ok();
                    }).disabled_if(busy.clone()));
                    let (weak, name) = (this.clone(), c.name.to_string());
                    worktree.push(MenuRow::action(format!("wt:{}", c.name), c.name.clone(), "git-branch", format!("Check out {} in a new directory", c.name), move |window, cx| {
                        let name = name.clone();
                        weak.update(cx, |v, cx| v.open_create_worktree(serde_json::json!({ "kind": "existing", "name": name }), window, cx)).ok();
                    }));
                } else if !c.local && !c.remotes.is_empty() {
                    let r = c.remotes.iter().find(|x| x.0 == "origin").or(c.remotes.first()).cloned().unwrap_or_default();
                    let branch_name = r.1.trim_start_matches(&format!("refs/remotes/{}/", r.0)).to_string();
                    let (weak, chip, id) = (this.clone(), c.clone(), sha.clone());
                    checkout.push(MenuRow::action(format!("checkout:{}", r.1), format!("{}/{branch_name}", r.0), "git-branch", format!("Check out {branch_name}, tracking {}/{branch_name}", r.0), move |_, cx| {
                        let (chip, id) = (chip.clone(), id.clone());
                        weak.update(cx, |v, cx| v.checkout_chip(&id, &chip, cx)).ok();
                    }).disabled_if(busy.clone()));
                }
            }
            if !checkout.is_empty() {
                checkout.push(MenuRow::Separator);
            }
            if !worktree.is_empty() {
                worktree.push(MenuRow::Separator);
            }
            let (weak, id, c, h) = (this.clone(), sha.clone(), ctx.clone(), head_sha_now.clone());
            checkout.push(MenuRow::action("checkout:detached", format!("Detached HEAD at {s7}"), "git-commit-vertical", "Check out this commit without a branch", move |_, cx| {
                let (id, c, h) = (id.clone(), c.clone(), h.clone());
                weak.update(cx, |v, cx| v.checkout(c, serde_json::json!({ "kind": "detached", "oid": id }), serde_json::json!({ "head": h, "refs": {} }), None, crate::ui::arm::Origin::None, None, cx)).ok();
            }).disabled_if(busy.clone()));
            let (weak, at) = (this.clone(), sha.clone());
            worktree.push(MenuRow::action("wt:new", "New branch here…", "folder-plus", "Create a branch here and check it out in a new directory", move |window, cx| {
                let at = at.clone();
                weak.update(cx, |v, cx| v.open_create_worktree(serde_json::json!({ "kind": "new", "name": "", "at": at }), window, cx)).ok();
            }));
            branch_rows.push(MenuRow::submenu("branch.checkout", "Checkout", "git-branch", "Check out a branch at this commit, or the commit itself", checkout));
            branch_rows.push(MenuRow::submenu("branch.createWorktree", "Create worktree from", "folder-plus", "Check out a branch in a new directory", worktree));
            let (weak, id, start) = (this.clone(), sha.clone(), branch.map(|b| if b.local { format!("refs/heads/{}", b.name) } else { b.remotes.first().map_or(String::new(), |r| r.1.clone()) }).filter(|s| !s.is_empty()));
            branch_rows.push(MenuRow::action("branch.createHere", "Create branch here", "git-branch-plus", format!("Create a branch at {}", branch.map_or(s7.clone(), |b| b.name.to_string())), move |window, cx| {
                let (id, start) = (id.clone(), start.clone());
                weak.update(cx, |v, cx| v.create_branch_at(id, start, window, cx)).ok();
            }));

            let one = crate::selection::CommitRef { oid: sha.clone(), summary: summary.clone(), merge: row.parents.len() > 1 };
            commit_rows.extend(self.pick_rows(&[one], this));
            let tag = |annotated: bool| -> crate::ui::Action {
                let (weak, id) = (this.clone(), sha.clone());
                Rc::new(move |window: &mut Window, cx: &mut App| {
                    let id = id.clone();
                    weak.update(cx, |v, cx| v.create_tag_at(id, annotated, window, cx)).ok();
                })
            };
            let plain = tag(false);
            commit_rows.push(MenuRow::action("tag.createHere", "Create tag here", "tag", format!("Create a lightweight tag at {s6}"), move |w, cx| plain(w, cx)).variants(vec![Variant { id: "lightweight".into(), label: Some("Lightweight".into()), icon: None, tooltip: format!("A tag that only names {s6}").into(), run: tag(false), disabled: None }, Variant { id: "annotated".into(), label: Some("Annotated".into()), icon: None, tooltip: "A tag with its own message, author and date".into(), run: tag(true), disabled: None }], None));
            commit_rows.extend(self.rebase_commit_rows(&sha, &row.parents, this));
        }
        let _ = &target;

        let mut stash_rows = Vec::new();
        if stash {
            stash_rows = self.stash_action_rows(&sha, Some(&summary), this);
        }

        let mut manage = Vec::new();
        if let Some(b) = branch {
            if let Some(l) = &local_b {
                let (weak, name, oid) = (this.clone(), l.name.clone(), l.target.clone());
                manage.push(MenuRow::action("branch.rename", format!("Rename {}", l.name), "pencil-line", format!("Rename the local branch {}", l.name), move |window, cx| {
                    let (name, oid) = (name.clone(), oid.clone());
                    weak.update(cx, |v, cx| v.rename_branch(name, oid, window, cx)).ok();
                }));
            }
            manage.extend(crate::refs::delete_row(tab, b, this));
        }

        let mut copy = Vec::new();
        if let Some(b) = branch {
            let name = b.name.to_string();
            let mut variants = Vec::new();
            if b.local {
                variants.push(variant("local", "Local", format!("Copy the local branch name ({name})"), copier(this, name.clone())));
            }
            if let Some((remote, full)) = b.remotes.first() {
                let label = format!("{remote}/{}", full.trim_start_matches(&format!("refs/remotes/{remote}/")));
                variants.push(variant("remote", "Remote", format!("Copy \"{label}\""), copier(this, label.clone())));
            }
            copy.push(MenuRow::action("commit.copyBranch", "Copy branch name", "git-branch", format!("Copy \"{name}\""), copier(this, name.clone())).variants(variants, None));
        }
        copy.push(copy_sha_row("commit.copySha", &sha, this));
        {
            let (weak, id) = (this.clone(), sha.clone());
            copy.push(MenuRow::action("commit.copyMessage", "Copy message", "message-square", "Copy the full commit message", move |_, cx| {
                let id = id.clone();
                weak.update(cx, |v, cx| v.copy_message(id, cx)).ok();
            }));
        }

        let mut view = Vec::new();
        let (weak, from) = (this.clone(), sha.clone());
        if let (Some(b), Some(h)) = (branch, head_sha.filter(|h| *h != sha)) {
            view.push(MenuRow::action("commit.compareHead", "Compare with HEAD", "git-compare", format!("Compare {} with HEAD ({})", b.name, graph.head_branch), move |_, cx| {
                let (from, to) = (from.clone(), h.clone());
                weak.update(cx, |v, cx| v.compare_commits(from, to, cx)).ok();
            }));
        } else {
            view.push(MenuRow::action("commit.compareWorktree", "Compare with working tree", "git-compare", "Compare this commit with the files on disk", move |_, cx| {
                let from = from.clone();
                weak.update(cx, |v, cx| v.compare_with_worktree(from, cx)).ok();
            }));
        }
        if let Some(b) = branch {
            let hide = MenuRow::pending("presets.hideBranch", "Hide branch", "eye-off");
            view.push(if b.head { hide.disabled("The trunk is always shown") } else { hide });
        }
        grouped(vec![sync, integrate, branch_rows, commit_rows, stash_rows, manage, copy, view])
    }

    fn wip_rows(&self, this: &WeakEntity<GitVolt>, row: &RowView) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let worktree = row.worktree.clone().unwrap_or_else(|| tab.worktree.clone());
        let name = row.wip.as_ref().and_then(|w| w.name.clone()).map_or("this worktree".to_string(), |n| n.to_string());
        let (weak, ctx) = (this.clone(), crate::write::WriteCtx { repo: tab.id, worktree });
        let stash = MenuRow::action("wip.stash", "Stash changes", "archive", format!("Stash every change in {name}, named from its WIP message"), move |_, cx| {
            let ctx = ctx.clone();
            weak.update(cx, |v, cx| v.stash_push(ctx, cx)).ok();
        });
        vec![stash, self.discard_all_row(this)]
    }

    /// A stash's Apply, Pop and Delete.
    pub fn stash_action_rows(&self, sha: &str, message: Option<&str>, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let ctx = tab.write_target();
        let apply = |pop: bool| {
            let (weak, c, oid) = (this.clone(), ctx.clone(), sha.to_string());
            move |_: &mut Window, cx: &mut App| {
                let (c, oid) = (c.clone(), oid.clone());
                weak.update(cx, |v, cx| v.apply_stash(c, oid, pop, cx)).ok();
            }
        };
        let (weak, oid, name) = (this.clone(), sha.to_string(), message.map(str::to_string));
        vec![
            MenuRow::action("stash.apply", "Apply", "package-open", "Apply the stash and keep it (staged changes come back staged)", apply(false)),
            MenuRow::action("stash.pop", "Pop", "archive-restore", "Apply the stash, then delete it (you can undo this)", apply(true)),
            MenuRow::action("stash.drop", "Delete", "trash-2", "Delete the stash (you can undo this)", move |window, cx| {
                let (c, oid, name) = (ctx.clone(), oid.clone(), name.clone());
                let origin = crate::ui::arm::Origin::current(cx);
                weak.update(cx, |v, cx| v.drop_stash(c, oid, name, origin, window, cx)).ok();
            }),
        ]
    }

    /// A tag's menu.
    pub fn tag_rows(&self, name: &str, sha: Option<&str>, in_sidebar: bool, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let loaded = sha.is_some_and(|s| self.in_graph(s));
        let remotes: Vec<String> = self.tabs.get(self.active).map_or(Vec::new(), |t| t.remote_groups().into_iter().map(|g| g.0).collect());
        let remote = remotes.iter().find(|r| *r == "origin").or(remotes.first()).cloned();
        let del = |local: bool, r: Option<String>| -> crate::ui::Action {
            let (weak, tag) = (this.clone(), name.to_string());
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let (tag, r) = (tag.clone(), r.clone());
                let origin = crate::ui::arm::Origin::current(cx);
                weak.update(cx, |v, cx| v.delete_tag(tag, local, r, origin, window, cx)).ok();
            })
        };
        let vr = |id: &str, label: &str, tip: String, run: crate::ui::Action| Variant { id: id.to_string().into(), label: Some(label.to_string().into()), icon: None, tooltip: tip.into(), run, disabled: None };
        let mut variants = vec![vr("local", "Local", format!("Delete the tag {name} here (you can undo this)"), del(true, None))];
        if let Some(r) = &remote {
            variants.push(vr("remote", "Remote", format!("Delete {name} from {r} (a push: can't be undone)"), del(false, Some(r.clone()))));
            variants.push(vr("both", "Both", format!("Delete {name} from {r}, then here"), del(true, Some(r.clone()))));
        }
        let plain = del(true, None);
        let push: Vec<MenuRow> = match remotes.len() {
            0 => Vec::new(),
            1 => {
                let (weak, r, tag) = (this.clone(), remotes[0].clone(), name.to_string());
                vec![MenuRow::action("tag.push", format!("Push {name} to {r}"), "upload", format!("Send the tag {name} to {r}"), move |_, cx| {
                    let (r, tag) = (r.clone(), tag.clone());
                    weak.update(cx, |v, cx| v.push_tags(r, Some(tag), cx)).ok();
                })]
            }
            _ => vec![MenuRow::submenu(
                "tag.push",
                format!("Push {name} to"),
                "upload",
                format!("Send the tag {name} to a remote"),
                remotes
                    .iter()
                    .map(|r| {
                        let (weak, r2, tag) = (this.clone(), r.clone(), name.to_string());
                        MenuRow::action(format!("tag.push.{r}"), r.clone(), "upload", format!("Send the tag {name} to {r}"), move |_, cx| {
                            let (r2, tag) = (r2.clone(), tag.clone());
                            weak.update(cx, |v, cx| v.push_tags(r2, Some(tag), cx)).ok();
                        })
                    })
                    .collect(),
            )],
        };
        grouped(vec![
            push,
            vec![MenuRow::action("tag.delete", "Delete", "trash-2", format!("Delete the tag {name} here"), move |w, cx| plain(w, cx)).variants(variants, Some("local"))],
            vec![MenuRow::action("tag.copyName", "Copy tag name", "copy", format!("Copy \"{name}\""), copier(this, name.to_string()))],
            vec![MenuRow::pending("presets.hideTag", "Hide tag", "eye-off")],
            if in_sidebar { show_in_graph(sha, loaded, this) } else { Vec::new() },
        ])
    }

    pub fn in_graph(&self, sha: &str) -> bool {
        self.tabs.get(self.active).and_then(|t| t.graph.as_ref()).is_some_and(|g| g.by_sha.contains_key(sha))
    }

    /// A worktree in the sidebar.
    pub fn worktree_rows(&self, path: &str, branch: Option<&str>, head: Option<&str>, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let mut copy = vec![MenuRow::action("sidebar.copyPath", "Copy path", "copy", format!("Copy \"{path}\""), copier(this, path.to_string()))];
        if let Some(b) = branch {
            copy.push(MenuRow::action("sidebar.copyBranch", "Copy branch name", "git-branch", format!("Copy \"{b}\""), copier(this, b.to_string())));
        }
        if let Some(h) = head {
            copy.push(copy_sha_row("sidebar.copySha", h, this));
        }
        let (weak, folder) = (this.clone(), path.to_string());
        let open = vec![MenuRow::action("sidebar.openFolder", "Open in file manager", "folder-open", "Show the worktree's directory in the file manager", move |_, cx| {
            let folder = folder.clone();
            weak.update(cx, |v, cx| v.open_folder(folder, cx)).ok();
        })];
        let tab = self.tabs.get(self.active);
        let is_main = tab.and_then(|t| t.graph.as_ref()).and_then(|g| g.worktrees.iter().find(|w| w.path == path)).is_some_and(|w| w.is_main);
        let manage = {
            let mut rows = Vec::new();
            if tab.is_some_and(|t| t.worktree != path) {
                let (weak, p) = (this.clone(), path.to_string());
                let shown = path.to_string();
                rows.push(MenuRow::action("sidebar.worktree.switch", "Switch to", "arrow-right-left", format!("Make {shown} this tab's worktree (its WIP, commit box and Undo)"), move |_, cx| {
                    let p = p.clone();
                    weak.update(cx, |v, cx| v.switch_worktree(p, cx)).ok();
                }));
            }
            if !is_main {
                let (weak, p, b) = (this.clone(), path.to_string(), branch.map(str::to_string));
                let shown = path.to_string();
                rows.push(MenuRow::action("sidebar.worktree.remove", "Remove worktree", "x", format!("Remove {shown}: its directory is deleted"), move |window, cx| {
                    let (p, b, origin) = (p.clone(), b.clone(), crate::ui::arm::Origin::current(cx));
                    weak.update(cx, |v, cx| v.remove_worktree(p, b, origin, window, cx)).ok();
                }));
            }
            rows
        };
        grouped(vec![manage, copy, open, show_in_graph(head, head.is_some_and(|h| self.in_graph(h)), this)])
    }

    /// A stash in the sidebar.
    pub fn stash_rows(&self, sha: &str, message: &str, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        grouped(vec![
            self.stash_action_rows(sha, Some(message), this),
            vec![copy_sha_row("sidebar.copySha", sha, this), MenuRow::action("sidebar.copyStash", "Copy message", "message-square", "Copy the stash's message", copier(this, message.to_string()))],
            show_in_graph(Some(sha), self.in_graph(sha), this),
        ])
    }

    /// A remote's folder in the sidebar.
    pub fn remote_rows(&self, name: &str, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        grouped(vec![
            vec![{
                let (weak, r) = (this.clone(), name.to_string());
                MenuRow::action("remote.pushTags", format!("Push all tags to {name}"), "upload", format!("Send every local tag {name} doesn't have"), move |_, cx| {
                    let r = r.clone();
                    weak.update(cx, |v, cx| v.push_tags(r, None, cx)).ok();
                })
            }],
            vec![MenuRow::action("sidebar.copyRemote", "Copy remote name", "copy", format!("Copy \"{name}\""), copier(this, name.to_string())), {
                let (weak, r) = (this.clone(), name.to_string());
                MenuRow::action("sidebar.copyUrl", "Copy URL", "square-arrow-out-up-right", format!("Copy the URL of {name}"), move |_, cx| {
                    let r = r.clone();
                    weak.update(cx, |v, cx| v.copy_remote_url(r, cx)).ok();
                })
            }],
            vec![{
                let (weak, r) = (this.clone(), name.to_string());
                MenuRow::action("sidebar.removeRemote", "Remove remote…", "trash-2", format!("Remove {name} from this repository"), move |window, cx| {
                    let (r, origin) = (r.clone(), crate::ui::arm::Origin::current(cx));
                    weak.update(cx, |v, cx| v.remove_remote(r, origin, window, cx)).ok();
                })
            }],
        ])
    }

    /// A branch folder in the sidebar.
    pub fn folder_rows(&self) -> Vec<MenuRow> {
        vec![MenuRow::pending("presets.hideFolder", "Hide folder", "eye-off")]
    }

    /// A tab.
    pub fn tab_rows(&self, ix: usize, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(ix) else { return Vec::new() };
        let only = self.tabs.len() == 1;
        let last = ix + 1 == self.tabs.len();
        let path = tab.worktree.clone();
        let close = |id: &'static str, label: &'static str, ico: &'static str, tooltip: &'static str, run: fn(&mut GitVolt, usize, &mut Context<GitVolt>)| {
            let weak = this.clone();
            MenuRow::action(id, label, ico, tooltip, move |_, cx| {
                weak.update(cx, |v, cx| run(v, ix, cx)).ok();
            })
        };
        let (weak, rename_ix) = (this.clone(), ix);
        let groups = self.tab_group_rows(ix, this);
        let (weak2, open_path) = (this.clone(), path.clone());
        let (weak3, reopen_weak) = (this.clone(), this.clone());
        let _ = weak3;
        grouped(vec![
            vec![MenuRow::action("tab.rename", "Rename…", "pencil", "Give this tab a display name (the repository is unchanged)", move |window, cx| {
                weak.update(cx, |v, cx| v.prompt_tab_name(rename_ix, window, cx)).ok();
            })],
            groups,
            vec![
                close("tab.close", "Close", "x", "Close this tab", |v, i, cx| v.close_tab(i, cx)).shortcut("Ctrl+W"),
                close("tab.closeOthers", "Close others", "square-x", "Close every other tab", |v, i, cx| v.close_other_tabs(i, cx)).disabled_if(only.then_some("This is the only tab")),
                close("tab.closeRight", "Close to the right", "arrow-right-to-line", "Close the tabs to the right", |v, i, cx| v.close_tabs_right(i, cx)).disabled_if(last.then_some("No tabs to the right")),
            ],
            vec![
                MenuRow::action("tab.reopen", "Reopen closed tab", "rotate-ccw", "Reopen the most recently closed tab", move |_, cx| {
                    reopen_weak.update(cx, |v, cx| v.reopen_tab(cx)).ok();
                })
                .shortcut("Ctrl+Shift+T")
                .disabled_if(self.closed_tabs.is_empty().then_some("No recently closed tabs")),
            ],
            vec![
                MenuRow::action("tab.copyPath", "Copy repo path", "copy", format!("Copy \"{path}\""), copier(this, path.clone())),
                MenuRow::action("tab.openFolder", "Open in file manager", "folder-open", "Open the tab's worktree directory in the file manager", move |_, cx| {
                    let p = open_path.clone();
                    weak2.update(cx, |v, cx| v.open_folder(p, cx)).ok();
                }),
            ],
        ])
    }

    /// The tab bar's empty space.
    pub fn tabbar_rows(&self, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let (a, b, c) = (this.clone(), this.clone(), this.clone());
        grouped(vec![
            vec![
                MenuRow::action("tabbar.reopen", "Reopen closed tab", "rotate-ccw", "Reopen the most recently closed tab", move |_, cx| {
                    a.update(cx, |v, cx| v.reopen_tab(cx)).ok();
                })
                .shortcut("Ctrl+Shift+T")
                .disabled_if(self.closed_tabs.is_empty().then_some("No recently closed tabs")),
            ],
            vec![
                MenuRow::action("tabbar.openRepo", "Open repository…", "folder-plus", "Open the Open Repository screen in a new tab", move |_, cx| {
                    b.update(cx, |v, cx| {
                        v.open_blank_tab(cx);
                    })
                    .ok();
                })
                .shortcut("Ctrl+O"),
                MenuRow::action("tabbar.clone", "Clone repository…", "copy-plus", "Clone a repository into a new tab", move |_, cx| {
                    c.update(cx, |v, cx| v.open_clone_tab(cx)).ok();
                }),
            ],
        ])
    }

    /// A file of the file list (or a diff's).
    pub fn file_rows(&self, file: &FileChange, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let path = file.path.to_string();
        let abs = format!("{}/{path}", tab.worktree.trim_end_matches('/'));
        let stage = self.file_stage_rows(file, this);
        let restore = match &file.origin {
            Origin::Commit { id } if file.status != crate::details::UNCHANGED && !id.is_empty() => {
                let (weak, sha, p) = (this.clone(), id.clone(), path.clone());
                vec![MenuRow::action("file.restore", "Restore from this commit", "rotate-ccw", format!("Put {path} back as this commit has it (you can undo this)"), move |_, cx| {
                    let (sha, p) = (sha.clone(), p.clone());
                    let origin = crate::ui::arm::Origin::None;
                    weak.update(cx, |v, cx| v.restore_file(sha, p, origin, cx)).ok();
                })]
            }
            Origin::Commit { .. } => Vec::new(),
            Origin::Wip { .. } => Vec::new(),
        };
        let ext = path.rsplit_once('.').map(|(_, e)| e.to_string());
        let colour = ext.as_ref().map_or(Vec::new(), |e| vec![MenuRow::pending("file.colour.type", format!("Colour *.{e} files…"), "palette")]);
        let copy = vec![
            MenuRow::action("file.copyPath", "Copy path", "copy", format!("Copy \"{path}\""), copier(this, path.clone())).variants(
                vec![
                    variant("rel", "Rel", format!("Copy the repository-relative path ({path})"), copier(this, path.clone())),
                    variant("abs", "Abs", format!("Copy the absolute path on disk ({abs})"), copier(this, abs.clone())),
                ],
                None,
            ),
        ];
        let open = self.open_in_rows(file, this, "file.openIn", "Open the file in another application");
        let (a, b, c, d) = (this.clone(), this.clone(), this.clone(), this.clone());
        let (fa, fb, fc, fd) = (file.clone(), file.clone(), file.clone(), file.clone());
        let view = vec![
            MenuRow::action("file.view", "View", "file-diff", "Show this file's changes in the center panel", move |_, cx| {
                let f = fa.clone();
                a.update(cx, |v, cx| v.open_change(f, cx)).ok();
            })
            .variants(
                vec![
                    Variant { id: "diff".into(), label: None, icon: Some("file-diff"), tooltip: "Show this file's changes in the center panel".into(), run: Rc::new(move |_, cx| {
                        let f = fb.clone();
                        b.update(cx, |v, cx| v.open_change(f, cx)).ok();
                    }), disabled: None },
                    Variant { id: "file".into(), label: None, icon: Some("file-text"), tooltip: "Show the whole file as it was in this commit".into(), run: Rc::new(move |_, cx| {
                        let f = fc.clone();
                        c.update(cx, |v, cx| v.open_change_as_file(f, cx)).ok();
                    }), disabled: None },
                ],
                None,
            ),
        ];
        let (e, fe) = (this.clone(), file.clone());
        let history = vec![
            MenuRow::action("file.fileHistory", "File history", "history", format!("Show the commits that changed {path}"), move |_, cx| {
                let f = fd.clone();
                d.update(cx, |v, cx| v.open_change_history(f, false, cx)).ok();
            }),
            MenuRow::action("file.blame", "Blame", "user", format!("Show who last changed each line of {path}"), move |_, cx| {
                let f = fe.clone();
                e.update(cx, |v, cx| v.open_change_history(f, true, cx)).ok();
            }),
        ];
        let lfs = ext.map_or(Vec::new(), |e| vec![MenuRow::pending("file.lfsTrack", format!("Track *.{e} with LFS"), "file-plus")]);
        grouped(vec![stage, restore, colour, copy, vec![open], view, history, lfs, vec![create_file_row(this)]])
    }

    /// A folder of the file list.
    pub fn file_folder_rows(&self, path: &str, section: Option<crate::wip::Which>, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let abs = format!("{}/{path}", tab.worktree.trim_end_matches('/'));
        let (weak, folder) = (this.clone(), abs.clone());
        grouped(vec![
            section.map_or(Vec::new(), |which| self.folder_stage_rows(path, which, this)),
            vec![MenuRow::pending("folder.colour", "Colour this folder…", "palette")],
            vec![
                MenuRow::action("folder.copyPath", "Copy path", "copy", format!("Copy \"{path}\""), copier(this, path.to_string())).variants(
                    vec![
                        variant("rel", "Rel", format!("Copy the repository-relative path ({path})"), copier(this, path.to_string())),
                        variant("abs", "Abs", format!("Copy the absolute path on disk ({abs})"), copier(this, abs.clone())),
                    ],
                    None,
                ),
            ],
            vec![MenuRow::action("folder.openIn", "Open in file manager", "folder-open", "Show the directory in the file manager", move |_, cx| {
                let f = folder.clone();
                weak.update(cx, |v, cx| v.open_folder(f, cx)).ok();
            })],
        ])
    }

    /// The diff's text.
    pub fn diff_text_rows(&self, file: &FileChange, selection: Option<String>, lines: (u32, u32), this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let path = file.path.to_string();
        let location = if lines.1 != lines.0 { format!("{path}:{}-{}", lines.0, lines.1) } else { format!("{path}:{}", lines.0) };
        let abs = format!("{}/{location}", tab.worktree.trim_end_matches('/'));
        let has = selection.is_some();
        let copy = MenuRow::action("monaco.copySelection", "Copy", "copy", "Copy the selected text", copier(this, selection.unwrap_or_default())).disabled_if((!has).then_some("No text selected"));
        grouped(vec![
            vec![
                copy,
                MenuRow::action("monaco.copyLocation", "Copy location", "copy", format!("Copy \"{location}\""), copier(this, location.clone())).variants(
                    vec![variant("rel", &location, "Copy the repository-relative location", copier(this, location.clone())), variant("abs", "Abs", "Copy the absolute path with the line", copier(this, abs))],
                    None,
                ),
            ],
            vec![self.open_in_rows(file, this, "monaco.openIn", &format!("Open the file in another application at line {}", lines.0))],
        ])
    }

    fn open_in_rows(&self, file: &FileChange, this: &WeakEntity<GitVolt>, id: &'static str, tooltip: &str) -> MenuRow {
        let list = self.tabs.get(self.active).and_then(|t| t.open_in.openers.clone());
        let path = file.path.to_string();
        let rows: Vec<MenuRow> = match list {
            None => vec![MenuRow::action("open.loading", "Looking for editors…", "loader-circle", "Looking for editors…", |_, _| {}).disabled("Looking for editors…")],
            Some(list) => menu_rows(&list, file)
                .into_iter()
                .map(|row| match row {
                    None => MenuRow::Separator,
                    Some(o) => opener_row(o, &path, file, this),
                })
                .collect(),
        };
        MenuRow::submenu(id, "Open in", "code", tooltip.to_string(), rows)
    }
}

fn opener_tip(o: &Opener, label: &str) -> String {
    match o.kind.as_str() {
        "fileManager" => format!("Show the file's directory in {}", label.trim_start_matches("Show in ")),
        "defaultApp" => "Open this version in its default application: the file itself, or a temporary read-only copy if it has changed since".into(),
        "other" => "Choose another application to open this version (the file, or a read-only copy if it has changed since) with".into(),
        _ => format!("Open this version (the file, or a read-only copy if it has changed since) in {}", label.trim_start_matches("Open in ")),
    }
}

fn opener_row(o: Opener, path: &str, file: &FileChange, this: &WeakEntity<GitVolt>) -> MenuRow {
    let label = opener_label(&o, path);
    let glyph = opener_icon(&o, path);
    let disabled = row_disabled(&o, path);
    let tip = opener_tip(&o, &label);
    let id = format!("opener.{}", o.id);
    let (weak, file) = (this.clone(), file.clone());
    MenuRow::action(id, label, glyph, tip, move |_, cx| {
        let (o, file) = (o.clone(), file.clone());
        weak.update(cx, |v, cx| v.open_file_with(o, file, cx)).ok();
    })
    .disabled_if(disabled.then_some("Not available for this file"))
}
