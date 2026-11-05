//! What each right-click shows (`ui/src/menu/builders.ts`, `menuEnv.ts`, each feature's `menus.ts`):
//! labels, groups and order as in the web UI. Rows whose action needs writes or a dialog not ported yet
//! stay in their place, disabled (`MenuRow::pending`); read-only ones work. Which integrate rows show
//! is approximated until they're wired. No forge rows: there's no forge layer here yet.

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

/// A variant of a row that's waiting for its action.
fn pending_variant(id: &str, label: &str, tooltip: impl Into<SharedString>) -> Variant {
    Variant { id: id.to_string().into(), label: Some(label.to_string().into()), icon: None, tooltip: tooltip.into(), run: Rc::new(|_, _| {}), disabled: Some(crate::ui::menu::NOT_YET.into()) }
}

fn copier(this: &WeakEntity<GitVolt>, text: String) -> impl Fn(&mut Window, &mut App) + 'static {
    let this = this.clone();
    move |_, cx| {
        let text = text.clone();
        this.update(cx, |v, cx| v.copy_text(text, cx)).ok();
    }
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
            return self.wip_rows(row);
        }
        let sha = row.id.to_string();
        let head = graph.head_branch.to_string();
        let head_sha = graph.head_row.and_then(|i| graph.rows.get(i)).map(|r| r.id.to_string());
        let is_head_commit = head_sha.as_deref() == Some(sha.as_str());
        let stash = row.kind == NodeKind::Stash;
        let branch = chip.or_else(|| primary_chip(row)).filter(|c| !c.tag && !stash);
        let local = branch.filter(|c| c.local);
        let (s6, s7) = (short(&sha), short7(&sha));
        let summary = row.summary.to_string();
        let target = branch.map_or(s6.clone(), |b| b.name.to_string());

        let mut sync = Vec::new();
        if let Some(b) = local {
            sync.push(MenuRow::pending("sync.push", "Push", "upload"));
            sync.push(MenuRow::pending("branch.setUpstream", "Set upstream", "arrow-up-from-dot"));
            let _ = b;
        }
        if !stash && !is_head_commit {
            sync.push(
                MenuRow::pending("commit.reset", format!("Reset {head} to this commit"), "rotate-ccw").variants(
                    vec![pending_variant("soft", "Soft", "keep all changes"), pending_variant("mixed", "Mixed", "keep working copy but reset index"), pending_variant("hard", "Hard", "discard all changes")],
                    Some("mixed"),
                ),
            );
        }

        let mut integrate = Vec::new();
        if let Some(b) = branch.filter(|b| !b.head) {
            let below = graph.head_row.is_some_and(|h| row_ix > h);
            if below {
                integrate.push(MenuRow::pending("integrate.ff", format!("Fast-forward {} to {head}", b.name), "git-merge"));
            } else {
                integrate.push(MenuRow::pending("integrate.merge", format!("Merge {} into {head}", b.name), "git-merge"));
                integrate.push(MenuRow::pending("integrate.rebase", format!("Rebase {head} onto {}", b.name), "git-branch"));
            }
            integrate.push(MenuRow::pending("irebase.onto", format!("Interactive rebase {head} onto {}", b.name), "list-ordered"));
        }

        let mut branch_rows = Vec::new();
        let mut commit_rows = Vec::new();
        if !stash {
            let mut checkout = Vec::new();
            let mut worktree = Vec::new();
            for c in row.chips.iter().filter(|c| c.local && !c.head) {
                checkout.push(MenuRow::pending(format!("checkout:{}", c.name), c.name.clone(), "git-branch"));
                worktree.push(MenuRow::pending(format!("wt:{}", c.name), c.name.clone(), "git-branch"));
            }
            if !checkout.is_empty() {
                checkout.push(MenuRow::Separator);
                worktree.push(MenuRow::Separator);
            }
            checkout.push(MenuRow::pending("checkout:detached", format!("Detached HEAD at {s7}"), "git-commit-horizontal"));
            worktree.push(MenuRow::pending("wt:new", "New branch here…", "git-branch-plus"));
            branch_rows.push(MenuRow::submenu("branch.checkout", "Checkout", "git-branch", "Check out a branch at this commit, or the commit itself", checkout));
            branch_rows.push(MenuRow::submenu("branch.createWorktree", "Create worktree from", "tree-pine", "Check out a branch in a new directory", worktree));
            branch_rows.push(MenuRow::pending("branch.createHere", "Create branch here", "git-branch-plus"));

            if !is_head_commit {
                commit_rows.push(MenuRow::pending("commit.cherryPick", format!("Cherry-pick onto {head}"), "git-commit-horizontal").variants(vec![pending_variant("noCommit", "No commit", format!("Apply the changes to {head}'s files and stage them, without committing"))], None));
                commit_rows.push(MenuRow::pending("commit.revert", "Revert this commit", "rotate-ccw").variants(vec![pending_variant("noCommit", "No commit", format!("Undo the changes in {head}'s files and stage that, without committing"))], None));
            }
            commit_rows.push(MenuRow::pending("tag.createHere", "Create tag here", "tag").variants(vec![pending_variant("lightweight", "Lightweight", format!("A tag that only names {s6}")), pending_variant("annotated", "Annotated", "A tag with its own message, author and date")], None));
            if !is_head_commit {
                commit_rows.push(MenuRow::pending("irebase.fromHere", "Interactive rebase after this commit", "list-ordered"));
                commit_rows.push(MenuRow::pending("irebase.drop", "Drop commit", "trash-2"));
            }
        }
        let _ = (&summary, &target);

        let mut stash_rows = Vec::new();
        if stash {
            stash_rows.push(MenuRow::pending("stash.apply", "Apply", "archive-restore"));
            stash_rows.push(MenuRow::pending("stash.pop", "Pop", "archive-restore"));
            stash_rows.push(MenuRow::pending("stash.drop", "Delete", "trash-2"));
        }

        let mut manage = Vec::new();
        if let Some(b) = local {
            manage.push(MenuRow::pending("branch.rename", format!("Rename {}", b.name), "pencil"));
            let delete = MenuRow::pending("branch.delete", "Delete", "trash-2").variants(vec![pending_variant("local", "Local", format!("Delete the local branch {} (you can undo this)", b.name))], None);
            manage.push(if b.head { delete.disabled(format!("Can't delete {}: it's checked out", b.name)) } else { delete });
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
            view.push(MenuRow::action("commit.compareHead", "Compare with HEAD", "git-compare", format!("Compare {} with HEAD ({head})", b.name), move |_, cx| {
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

    fn wip_rows(&self, _: &RowView) -> Vec<MenuRow> {
        vec![MenuRow::pending("wip.stash", "Stash changes", "archive"), MenuRow::pending("wip.discardAll", "Discard all changes…", "trash-2")]
    }

    /// A tag's menu.
    pub fn tag_rows(&self, name: &str, sha: Option<&str>, in_sidebar: bool, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let loaded = sha.is_some_and(|s| self.in_graph(s));
        grouped(vec![
            vec![MenuRow::pending("tag.delete", "Delete", "trash-2").variants(vec![pending_variant("local", "Local", format!("Delete the tag {name} here (you can undo this)"))], Some("local"))],
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
        grouped(vec![copy, open, show_in_graph(head, head.is_some_and(|h| self.in_graph(h)), this)])
    }

    /// A stash in the sidebar.
    pub fn stash_rows(&self, sha: &str, message: &str, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        grouped(vec![
            vec![MenuRow::pending("stash.apply", "Apply", "archive-restore"), MenuRow::pending("stash.pop", "Pop", "archive-restore"), MenuRow::pending("stash.drop", "Delete", "trash-2")],
            vec![copy_sha_row("sidebar.copySha", sha, this), MenuRow::action("sidebar.copyStash", "Copy message", "message-square", "Copy the stash's message", copier(this, message.to_string()))],
            show_in_graph(Some(sha), self.in_graph(sha), this),
        ])
    }

    /// A remote's folder in the sidebar.
    pub fn remote_rows(&self, name: &str, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        grouped(vec![
            vec![MenuRow::pending("remote.pushTags", format!("Push all tags to {name}"), "upload")],
            vec![MenuRow::action("sidebar.copyRemote", "Copy remote name", "copy", format!("Copy \"{name}\""), copier(this, name.to_string())), MenuRow::pending("sidebar.copyUrl", "Copy URL", "square-arrow-out-up-right")],
            vec![MenuRow::pending("sidebar.removeRemote", "Remove remote…", "trash-2")],
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
        let (a, b) = (this.clone(), this.clone());
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
                    b.update(cx, |v, cx| v.pick_repository(cx)).ok();
                })
                .shortcut("Ctrl+O"),
                MenuRow::pending("tabbar.clone", "Clone repository…", "copy-plus"),
            ],
        ])
    }

    /// A file of the file list (or a diff's).
    pub fn file_rows(&self, file: &FileChange, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let path = file.path.to_string();
        let abs = format!("{}/{path}", tab.worktree.trim_end_matches('/'));
        let mut stage = Vec::new();
        match &file.origin {
            Origin::Wip { staged: true, .. } => stage.push(MenuRow::pending("file.unstage", "Unstage", "minus")),
            Origin::Wip { staged: false, .. } => {
                stage.push(MenuRow::pending("file.stageFile", "Stage", "plus"));
                stage.push(MenuRow::pending("file.discard", "Discard changes…", "trash-2"));
            }
            Origin::Commit { .. } => {}
        }
        let restore = match &file.origin {
            Origin::Commit { .. } => vec![MenuRow::pending("file.restore", "Restore from this commit", "rotate-ccw")],
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
        let lfs = ext.map_or(Vec::new(), |e| vec![MenuRow::pending("file.lfsTrack", format!("Track *.{e} with LFS"), "cloud-download")]);
        grouped(vec![stage, restore, colour, copy, vec![open], view, history, lfs, vec![MenuRow::pending("files.createFile", "Create file…", "file-plus")]])
    }

    /// A folder of the file list.
    pub fn file_folder_rows(&self, path: &str, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let abs = format!("{}/{path}", tab.worktree.trim_end_matches('/'));
        let (weak, folder) = (this.clone(), abs.clone());
        grouped(vec![
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
