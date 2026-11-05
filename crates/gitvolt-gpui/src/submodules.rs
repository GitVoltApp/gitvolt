//! Submodules (`ui/src/submodules/`), only in a worktree that uses them: palette actions, a submodule's menu in the file
//! lists and the sidebar's panel (double click opens it, or initialises one not checked out), and Add, in any repository.

use crate::app::GitVolt;
use crate::details::FileChange;
use crate::gitflow::labelled;
use crate::palette::{Choices, Entry, PaletteGroup};
use crate::theme;
use crate::ui::arm::{self, ArmRequest, Origin};
use crate::ui::controls::{self, ButtonKind};
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::menu::{MenuRow, grouped};
use crate::ui::modal;
use crate::ui::toast::{self, Options};
use crate::write::{Write, call};
use gitvolt_core::api::Request;
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::cell::Cell;
use std::rc::Rc;

/// `defaultSubmodulePath`: where git puts a submodule of `url`: the URL's last part, without `.git`.
pub fn default_submodule_path(url: &str) -> String {
    let s = url.trim().trim_end_matches(['/', '\\']);
    let last = s.rsplit(['/', '\\', ':']).next().unwrap_or_default();
    if last.len() >= 4 && last[last.len() - 4..].eq_ignore_ascii_case(".git") { last[..last.len() - 4].to_string() } else { last.to_string() }
}

/// `submodulePathError`.
pub fn submodule_path_error(path: &str) -> Option<&'static str> {
    if path.is_empty() {
        return Some("Enter a path for the submodule");
    }
    let drive = path.as_bytes().get(1) == Some(&b':') && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic);
    if path.starts_with('/') || drive {
        return Some("A path inside the repository, not an absolute one");
    }
    path.split(['/', '\\']).any(|p| p == ".." || p == ".git").then_some("A path inside the repository")
}

/// `unmappedReason`: why git's submodule commands can't run in a worktree.
pub fn unmapped_reason(unmapped: &[String]) -> Option<String> {
    let (first, rest) = unmapped.split_first()?;
    let which = if rest.is_empty() { format!("{first} has") } else { format!("{first} and {} more have", rest.len()) };
    Some(format!("{which} no entry in .gitmodules: git's submodule commands can't run in this worktree"))
}

/// `SUBMODULE_STATUS_TEXT`.
pub fn status_text(status: &str) -> &'static str {
    match status {
        "current" => "checked out",
        "newCommits" => "new commits",
        "uninitialized" => "not initialised",
        _ => "conflicted",
    }
}

/// `submoduleCardState`: a card's last line.
pub fn card_state(status: &str) -> &'static str {
    match status {
        "current" => "At the recorded commit · double-click to open",
        "newCommits" => "Checked out at another commit than the one recorded · double-click to open",
        "uninitialized" => "Nothing checked out · double-click to initialise",
        _ => "Conflicted in the index · double-click to open",
    }
}

impl GitVolt {
    /// What the core says of the tab's worktree; read when a tab opens or its repo changes.
    pub fn read_repo_info(&mut self, repo: u32, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(Request::RepoInfo { repo: crate::app::core_repo(repo) }).await });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(info)) = read.await else { return };
            this.update(cx, |this, cx| {
                if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) {
                    tab.info = info;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// `SubmoduleSupport` for the tab's worktree: `None` when it uses no submodules.
    fn support(&self) -> Option<Value> {
        let tab = self.tabs.get(self.active)?;
        let v = &tab.info["submodules"][tab.write_target().worktree.as_str()];
        (!v.is_null()).then(|| v.clone())
    }

    pub fn uses_submodules(&self) -> bool {
        self.support().is_some()
    }

    fn unmapped(&self) -> Option<String> {
        let s = self.support()?;
        let list: Vec<String> = s["unmapped"].as_array().map_or(Vec::new(), |a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect());
        unmapped_reason(&list)
    }

    /// Why a write from a submodule's menu can't run now.
    fn submodule_blocked(&self) -> Option<String> {
        let tab = self.tabs.get(self.active)?;
        self.unmapped().or_else(|| tab.wip.committing.then(|| "A commit is queued".to_string())).or_else(|| tab.busy_reason())
    }

    /// `git submodule update --init --recursive`, of `path` only when given: a network command, with its op's feedback.
    pub fn update_submodule(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        let Some(ctx) = self.tabs.get(self.active).map(|t| t.write_target()) else { return };
        let (repo, worktree, p) = (ctx.repo, ctx.worktree.clone(), path.clone());
        let w = Write::new(&ctx, move |asked| {
            let mut params = json!({ "repo": crate::app::core_repo(repo), "worktree": worktree });
            if let Some(p) = &p {
                params["path"] = json!(p);
            }
            if asked.discard {
                params["confirm"] = json!(true);
            }
            call("submoduleUpdate", params)
        })
        .on_success(move |_, out, cx| {
            let done = path.as_ref().map_or("Submodules updated".to_string(), |p| format!("Updated {p}"));
            ran(done, out, cx);
        });
        self.write(w, cx);
    }

    fn run_in_worktree(&mut self, f: impl FnOnce(&mut Self, &mut Context<Self>), cx: &mut Context<Self>) {
        if !self.uses_submodules() {
            return;
        }
        match self.unmapped() {
            Some(why) => toast::show_with(why, Options { warning: true, ..Default::default() }, cx),
            None => f(self, cx),
        }
    }

    pub fn palette_submodule_update(&mut self, cx: &mut Context<Self>) {
        self.run_in_worktree(|v, cx| v.update_submodule(None, cx), cx);
    }

    /// `git submodule sync --recursive`: `.gitmodules`' URLs into the config.
    pub fn palette_submodule_sync(&mut self, cx: &mut Context<Self>) {
        self.run_in_worktree(
            |v, cx| {
                let Some(ctx) = v.tabs.get(v.active).map(|t| t.write_target()) else { return };
                let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
                let w = Write::new(&ctx, move |_| call("submoduleSync", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree }))).on_success(|_, out, cx| ran("Submodule URLs synced".to_string(), out, cx));
                v.write(w, cx);
            },
            cx,
        );
    }

    /// "Submodules: Open in a tab…": the palette lists them (`git submodule status`).
    pub fn palette_submodule_open(&mut self, cx: &mut Context<Self>) {
        self.run_in_worktree(
            |v, cx| {
                let Some(ctx) = v.tabs.get(v.active).map(|t| t.write_target()) else { return };
                let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
                let api = v.api.clone();
                let tree = worktree.clone();
                let read = v.rt.spawn(async move { api.dispatch(Request::Submodules { repo: crate::app::core_repo(repo), worktree: tree }).await });
                cx.spawn(async move |this, cx| {
                    let answer = read.await;
                    this.update(cx, |this, cx| match answer {
                        Ok(Ok(list)) => this.choose_submodule(worktree, &list, cx),
                        Ok(Err(e)) => toast::show_with(format!("Couldn't list the submodules: {}", e.message), Options { error: true, ..Default::default() }, cx),
                        Err(e) => toast::show_with(format!("Couldn't list the submodules: {e}"), Options { error: true, ..Default::default() }, cx),
                    })
                    .ok();
                })
                .detach();
            },
            cx,
        );
    }

    fn choose_submodule(&mut self, worktree: String, list: &Value, cx: &mut Context<Self>) {
        let items = list.as_array().cloned().unwrap_or_default();
        if items.is_empty() {
            return toast::show("This worktree has no submodules", cx);
        }
        let weak = cx.weak_entity();
        let entries: Vec<Entry> = items
            .iter()
            .map(|s| {
                let path = s["path"].as_str().unwrap_or_default().to_string();
                let status = s["status"].as_str().unwrap_or_default().to_string();
                let (weak, tree, p) = (weak.clone(), worktree.clone(), path.clone());
                Entry {
                    group: PaletteGroup::Choice,
                    label: path.into(),
                    detail: Some(status_text(&status).into()),
                    run: Rc::new(move |_, cx| {
                        let (tree, p, uninit) = (tree.clone(), p.clone(), status == "uninitialized");
                        weak.update(cx, |v, cx| if uninit { toast::show(format!("{p} isn't initialised: Submodules: Update checks it out"), cx) } else { v.open_submodule(tree, p, cx) }).ok();
                    }),
                    alt: None,
                }
            })
            .collect();
        self.with_window(cx, move |window, cx| {
            weak.update(cx, |v, cx| v.choose(Choices { title: "Submodules".into(), placeholder: "Open a submodule in a tab".into(), entries }, window, cx)).ok();
        });
    }

    /// Opens the submodule as a repository tab; one with nothing checked out says so instead.
    pub fn open_submodule(&mut self, worktree: String, path: String, cx: &mut Context<Self>) {
        let dir = format!("{}/{path}", worktree.trim_end_matches('/'));
        let api = self.api.clone();
        let d = dir.clone();
        let peek = self.rt.spawn(async move { api.dispatch(Request::PeekRepo { path: d }).await });
        cx.spawn(async move |this, cx| {
            let ok = matches!(peek.await, Ok(Ok(_)));
            this.update(cx, |this, cx| {
                if ok {
                    this.open_in(dir, None, cx);
                } else {
                    toast::show(format!("{path} isn't initialised: Submodules: Update checks it out"), cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// A double click on a Submodules row, and Enter: open it, or initialise one that has nothing checked out yet.
    pub fn activate_submodule(&mut self, path: String, status: &str, cx: &mut Context<Self>) {
        let Some(ctx) = self.tabs.get(self.active).map(|t| t.write_target()) else { return };
        if status != "uninitialized" {
            return self.open_submodule(ctx.worktree, path, cx);
        }
        match self.submodule_blocked() {
            Some(why) => toast::show_with(why, Options { warning: true, ..Default::default() }, cx),
            None => self.update_submodule(Some(path), cx),
        }
    }

    /// Remove submodule: asked first, in place; `git rm` takes out its checkout, gitlink and `.gitmodules` entry, staged.
    pub fn remove_submodule(&mut self, path: String, origin: Origin, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ctx) = self.tabs.get(self.active).map(|t| t.write_target()) else { return };
        let mut req = ArmRequest::danger(format!("Click again to remove {path}"), format!("Remove the submodule {path}?"), "Remove submodule");
        req.body = "Its checkout, its entry in .gitmodules and the commit recorded here are removed and the removal is staged; commit to record it. Its repository stays in .git/modules. Not undoable from here: unstaging and Submodules: Update bring it back.".into();
        req.caption = Some("Removes the checkout and stages the removal (git rm)".into());
        let this = cx.weak_entity();
        let again = origin.clone();
        arm::request(
            req,
            origin,
            move |a, _, cx| {
                if !a.ok {
                    return;
                }
                let (ctx, path, origin) = (ctx.clone(), path.clone(), again.clone());
                this.update(cx, |v, cx| {
                    let (repo, worktree, p) = (ctx.repo, ctx.worktree.clone(), path.clone());
                    let w = Write::new(&ctx, move |_| call("submoduleRemove", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "path": p }))).on_success(move |_, out, cx| ran(format!("Removed {path}: commit to record it"), out, cx)).origin(origin);
                    v.write(w, cx);
                })
                .ok();
            },
            window,
            cx,
        );
    }

    /// A Submodules panel row's menu.
    pub fn sidebar_submodule_rows(&self, path: &str, status: &str, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let worktree = tab.write_target().worktree;
        let blocked = self.submodule_blocked();
        let uninit = status == "uninitialized";
        let (w, tree, p) = (this.clone(), worktree.clone(), path.to_string());
        let open = MenuRow::action("sidebar.submodule.open", "Open in a tab", "square-plus", format!("Open the submodule {path} as a repository tab"), move |_, cx| {
            let (tree, p) = (tree.clone(), p.clone());
            w.update(cx, |v, cx| v.open_submodule(tree, p, cx)).ok();
        })
        .disabled_if(uninit.then_some("Not initialised: Initialise checks it out"));
        let (w, p) = (this.clone(), path.to_string());
        let run = move |_: &mut Window, cx: &mut App| {
            let p = p.clone();
            w.update(cx, |v, cx| v.update_submodule(Some(p), cx)).ok();
        };
        let update = if uninit {
            MenuRow::action("sidebar.submodule.init", "Initialise", "folder-sync", format!("Clone {path} and check out its recorded commit (git submodule update --init --recursive -- {path})"), run)
        } else {
            MenuRow::action("sidebar.submodule.update", "Update", "folder-sync", format!("Check out {path} at its recorded commit (git submodule update --init --recursive -- {path})"), run)
        }
        .disabled_if(blocked);
        let first = if uninit { vec![update, open] } else { vec![open, update] };
        let reveal = (!uninit).then(|| {
            let (w, dir) = (this.clone(), format!("{}/{path}", worktree.trim_end_matches('/')));
            MenuRow::action("sidebar.submodule.reveal", "Open in file manager", "folder-open", "Show the submodule's directory in the file manager", move |_, cx| {
                let dir = dir.clone();
                w.update(cx, |v, cx| v.open_folder(dir, cx)).ok();
            })
        });
        let (w, p) = (this.clone(), path.to_string());
        let remove = MenuRow::action("sidebar.submodule.remove", "Remove submodule…", "trash-2", format!("Remove {path}'s checkout and its .gitmodules entry, staged (git rm -- {path})"), move |window, cx| {
            let (p, origin) = (p.clone(), Origin::current(cx));
            w.update(cx, |v, cx| v.remove_submodule(p, origin, window, cx)).ok();
        })
        .disabled_if(self.submodule_blocked());
        let copy = vec![MenuRow::action("sidebar.copyPath", "Copy path", "copy", format!("Copy \"{path}\""), {
            let (w, p) = (this.clone(), path.to_string());
            move |_, cx| {
                let p = p.clone();
                w.update(cx, |v, cx| v.copy_text(p, cx)).ok();
            }
        })];
        grouped(vec![first, vec![remove], copy, reveal.into_iter().collect()])
    }

    /// A file row's menu for a gitlink: Open in a tab; in a working-tree list, Update this submodule.
    pub fn file_submodule_rows(&self, file: &FileChange, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        if !file.is_submodule() || file.status == 'D' {
            return Vec::new();
        }
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let path = file.path.to_string();
        let tree = tab.write_target().worktree;
        let (w, t, p) = (this.clone(), tree, path.clone());
        let mut rows = vec![MenuRow::action("file.submodule.open", "Open in a tab", "square-plus", format!("Open the submodule {path} as a repository tab"), move |_, cx| {
            let (t, p) = (t.clone(), p.clone());
            w.update(cx, |v, cx| v.open_submodule(t, p, cx)).ok();
        })];
        if matches!(file.origin, crate::details::Origin::Wip { .. }) {
            let (w, p) = (this.clone(), path.clone());
            rows.push(
                MenuRow::action("file.submodule.update", "Update this submodule", "folder-sync", format!("Check out {path} at its recorded commit (git submodule update --init --recursive -- {path})"), move |_, cx| {
                    let p = p.clone();
                    w.update(cx, |v, cx| v.update_submodule(Some(p), cx)).ok();
                })
                .disabled_if(self.submodule_blocked()),
            );
        }
        rows
    }

    /// Add submodule: the URL, a path that follows it until edited, a branch.
    pub fn open_add_submodule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        const ID: &str = "add-submodule";
        let Some(ctx) = self.tabs.get(self.active).map(|t| t.write_target()) else { return };
        let url = cx.new(|cx| TextInput::new(cx).placeholder("https://git.example.com/group/library.git"));
        let path = cx.new(|cx| TextInput::new(cx).placeholder("vendor/library"));
        let branch = cx.new(|cx| TextInput::new(cx).placeholder("The remote's default branch"));
        let edited = Rc::new(Cell::new(false));
        let this = cx.weak_entity();
        let check = {
            let (url, path, branch, edited) = (url.clone(), path.clone(), branch.clone(), edited.clone());
            move |cx: &App| -> (String, String, String, Option<String>) {
                let u = url.read(cx).text().trim().to_string();
                let p = if edited.get() { path.read(cx).text().trim().to_string() } else { default_submodule_path(&u) };
                let b = branch.read(cx).text().trim().to_string();
                let error = if u.is_empty() { Some("Enter the URL of the repository to add".to_string()) } else { submodule_path_error(&p).map(str::to_string).or_else(|| b.starts_with('-').then(|| format!("\"{b}\" isn't a branch name"))) };
                (u, p, b, error)
            }
        };
        let send: crate::ui::Action = {
            let (check, ctx) = (check.clone(), ctx);
            Rc::new(move |window, cx| {
                let (u, p, b, error) = check(cx);
                if error.is_some() {
                    return;
                }
                modal::close(ID, window, cx);
                let ctx = ctx.clone();
                this.update(cx, |v, cx| {
                    let (repo, worktree, u2, p2) = (ctx.repo, ctx.worktree.clone(), u.clone(), p.clone());
                    let w = Write::new(&ctx, move |_| {
                        let mut params = json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "url": u2, "path": p2 });
                        if !b.is_empty() {
                            params["branch"] = json!(b);
                        }
                        call("submoduleAdd", params)
                    })
                    .on_success(move |this, out, cx| {
                        ran(format!("Added the submodule {p}: commit to record it"), out, cx);
                        this.read_repo_info(repo, cx);
                    });
                    v.write(w, cx);
                })
                .ok();
            })
        };
        let handle = window.window_handle();
        let keep: Vec<Subscription> = [(&url, false), (&path, true), (&branch, false)]
            .into_iter()
            .map(|(input, is_path)| {
                let (send, edited) = (send.clone(), edited.clone());
                App::subscribe(cx, input, move |_, e: &InputEvent, cx| match e {
                    InputEvent::Submit { .. } => {
                        let send = send.clone();
                        handle.update(cx, move |_, window, cx| window.defer(cx, move |window, cx| send(window, cx))).ok();
                    }
                    InputEvent::Changed => {
                        if is_path {
                            edited.set(true);
                        }
                        cx.refresh_windows();
                    }
                    _ => {}
                })
            })
            .collect();
        let focus = url.read(cx).focus.clone();
        let stops = [cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true)];
        let mut m = modal::Modal::new(ID, move |window, cx| {
            let (u, p, _, error) = check(cx);
            if !edited.get() && path.read(cx).text() != p {
                path.update(cx, |i, cx| i.set_text(&p, cx));
            }
            let (s1, s2) = (send.clone(), send.clone());
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(modal::title("Add submodule"))
                .child(labelled("URL", &url, window, cx))
                .child(labelled("Path", &path, window, cx))
                .child(labelled("Branch", &branch, window, cx))
                .when_some(error.clone().filter(|_| !u.is_empty()), |d, e| d.child(div().text_color(theme::red()).child(e)))
                .child(modal::actions_row().child(controls::tabbable(controls::button("form-cancel", "Cancel", ButtonKind::Plain, true).on_click(|_, window, cx| modal::close(ID, window, cx)), &stops[0], |window, cx| modal::close(ID, window, cx))).child(crate::ui::anchor("form-ok", controls::tabbable(controls::button("form-ok", "Add submodule", ButtonKind::Primary, error.is_none()).when(error.is_none(), |d| d.on_click(move |_, window, cx| s1(window, cx))), &stops[1], move |window, cx| s2(window, cx)))))
                .into_any_element()
        })
        .focus(focus);
        m.keep.extend(keep);
        modal::open(m, window, cx);
    }
}

/// Says what ran, as the terminal command.
fn ran(done: String, out: &Value, cx: &mut Context<GitVolt>) {
    toast::show_with(done, Options { detail: out["command"].as_str().map(|c| c.to_string().into()), ..Default::default() }, cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_path_follows_the_url_and_stays_inside_the_repository() {
        assert_eq!(default_submodule_path("https://git.example.com/group/library.git"), "library");
        assert_eq!(default_submodule_path("git@example.com:group/lib/"), "lib");
        assert_eq!(submodule_path_error(""), Some("Enter a path for the submodule"));
        assert_eq!(submodule_path_error("/abs"), Some("A path inside the repository, not an absolute one"));
        assert_eq!(submodule_path_error("a/../b"), Some("A path inside the repository"));
        assert_eq!(submodule_path_error("vendor/lib"), None);
    }

    #[test]
    fn unmapped_gitlinks_say_why_nothing_runs() {
        assert_eq!(unmapped_reason(&[]), None);
        assert!(unmapped_reason(&["a".to_string()]).unwrap().starts_with("a has no entry"));
        assert!(unmapped_reason(&["a".to_string(), "b".to_string()]).unwrap().starts_with("a and 1 more have"));
    }
}
