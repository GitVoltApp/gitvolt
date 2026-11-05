//! The writes that manage more than a branch: remotes (add, remove, Copy URL), worktrees (create, remove,
//! switch), Create file, and a submodule's update. All through `Write`, as the web UI's `remotes/`,
//! `worktrees/`, `files/createFile.ts` and `submodules/commands.ts`.

use crate::app::GitVolt;
use crate::refs::branch_name_error;
use crate::theme;
use crate::ui::arm::{self, ArmRequest, Origin};
use crate::ui::controls::{self, ButtonKind};
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::{modal, toast};
use crate::write::{Write, WriteCtx, call};
use gitvolt_core::api::Request;
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// `createFilePathError`: `None` when it can be used.
pub fn file_path_error(v: &str) -> Option<&'static str> {
    if v.trim().is_empty() {
        return Some("Enter a file name");
    }
    let drive = v.as_bytes().get(1) == Some(&b':') && v.as_bytes().first().is_some_and(u8::is_ascii_alphabetic) && matches!(v.as_bytes().get(2), Some(b'/' | b'\\'));
    if v.starts_with('/') || drive {
        return Some("Enter a path relative to the repository");
    }
    if v.contains('\0') || v.contains('\\') {
        return Some("A file name can't contain \\ or NUL");
    }
    let parts: Vec<&str> = v.split('/').collect();
    if parts.contains(&"..") {
        return Some("The path can't leave the repository (..)");
    }
    if parts.iter().any(|p| p.trim().trim_end_matches('.').eq_ignore_ascii_case(".git")) {
        return Some("Files can't be created in .git");
    }
    if v.ends_with('/') {
        return Some("Enter a file name after the folder");
    }
    if parts.iter().any(|p| p.is_empty() || *p == ".") {
        return Some("The path has an empty or \".\" folder");
    }
    None
}

/// `removeRemoteText`: what removing `name` costs, from the sidebar's payload.
pub fn remove_remote_text(name: &str, sidebar: &Value) -> String {
    let plural = |n: usize, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
    let groups = sidebar["remotes"].as_array().map_or(&[][..], Vec::as_slice);
    let branches = groups.iter().find(|g| g["name"] == name).and_then(|g| g["branches"].as_array()).map_or(0, Vec::len);
    let mut remotes: Vec<&str> = groups.iter().filter_map(|g| g["name"].as_str()).collect();
    remotes.sort_by_key(|r| std::cmp::Reverse(r.len()));
    let remote_of = |upstream: &str| remotes.iter().find(|r| upstream.starts_with(&format!("refs/remotes/{r}/"))).copied();
    let tracking: Vec<&str> = sidebar["locals"].as_array().map_or(&[][..], Vec::as_slice).iter().filter(|l| l["upstream"].as_str().and_then(remote_of) == Some(name)).filter_map(|l| l["name"].as_str()).collect();
    let mut parts = vec![format!("Its {} this repository.", plural(branches, "remote branch leaves", "remote branches leave"))];
    if !tracking.is_empty() {
        let lose = if tracking.len() == 1 { "tracking it loses its upstream" } else { "tracking it lose their upstream" };
        let names = if tracking.len() <= 3 { tracking.join(", ") } else { format!("{} and {} more", tracking[..3].join(", "), tracking.len() - 3) };
        parts.push(format!("{} {lose}: {names}.", plural(tracking.len(), "local branch", "local branches")));
    }
    parts.push("Undo puts it all back.".to_string());
    parts.join(" ")
}

/// A remote's name is one path of a ref: what a branch's may be, no more.
pub fn remote_name_error(name: &str, taken: &[String]) -> Option<String> {
    if name.is_empty() {
        return Some("Enter a remote name".to_string());
    }
    if taken.iter().any(|t| t == name) {
        return Some(format!("A remote named {name} exists"));
    }
    branch_name_error(name).map(|_| format!("{name} can't be a remote's name"))
}

/// A worktree's path as the row shows it: relative to the main one's parent when it's beside it.
pub fn worktree_shown(main: &str, path: &str) -> String {
    let parent = main.rsplit_once('/').map_or("", |p| p.0);
    match path.strip_prefix(parent).and_then(|p| p.strip_prefix('/')) {
        Some(rest) if !parent.is_empty() => format!("../{rest}"),
        _ => path.to_string(),
    }
}

fn field(label: &'static str, input: &Entity<TextInput>) -> Div {
    div().flex().flex_col().gap(px(4.)).child(label).child(div().h(px(30.)).px(px(8.)).flex().items_center().bg(theme::app_bg()).border_1().border_color(theme::section_border()).rounded(px(4.)).text_color(theme::text_selected()).child(input.clone()))
}

/// Two fields and a confirm: Enter in either submits, Tab goes on through them and the buttons.
struct TwoFields {
    id: &'static str,
    title: &'static str,
    labels: (&'static str, &'static str),
    values: (String, String),
    confirm: &'static str,
}

type Check = Rc<dyn Fn(&str, &str) -> Option<String>>;
type Submit = Rc<dyn Fn(String, String, &mut Window, &mut App)>;
type Changed = Rc<dyn Fn(&str, &Entity<TextInput>, &mut App)>;

fn two_fields(f: TwoFields, check: Check, submit: Submit, on_first: Option<Changed>, window: &mut Window, cx: &mut App) {
    let id = f.id;
    let make = |text: &str, cx: &mut App| {
        cx.new(|cx| {
            let mut i = TextInput::new(cx);
            i.set_text(text, cx);
            i
        })
    };
    let (first, second) = (make(&f.values.0, cx), make(&f.values.1, cx));
    let tried = Rc::new(Cell::new(false));
    let send: crate::ui::Action = {
        let (first, second, check, tried) = (first.clone(), second.clone(), check.clone(), tried.clone());
        Rc::new(move |window, cx| {
            tried.set(true);
            let (a, b) = (first.read(cx).text().trim().to_string(), second.read(cx).text().trim().to_string());
            cx.refresh_windows();
            if check(&a, &b).is_some() {
                return;
            }
            modal::close(id, window, cx);
            submit(a, b, window, cx);
        })
    };
    let handle = window.window_handle();
    let watch = |input: &Entity<TextInput>, changed: Option<Changed>, second: Entity<TextInput>, cx: &mut App| {
        let send = send.clone();
        cx.subscribe(input, move |input, e: &InputEvent, cx| match e {
            InputEvent::Submit { .. } => {
                let send = send.clone();
                handle.update(cx, move |_, window, cx| window.defer(cx, move |window, cx| send(window, cx))).ok();
            }
            InputEvent::Changed => {
                if let Some(c) = &changed {
                    let text = input.read(cx).text().trim().to_string();
                    c(&text, &second, cx);
                }
                cx.refresh_windows();
            }
            _ => {}
        })
    };
    if let Some(c) = on_first.as_ref().filter(|_| !f.values.0.is_empty()) {
        c(&f.values.0, &second, cx);
    }
    let keep = [watch(&first, on_first, second.clone(), cx), watch(&second, None, second.clone(), cx)];
    let focus = first.read(cx).focus.clone();
    let stops = [cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true)];
    let (title, labels, confirm) = (f.title, f.labels, f.confirm);
    let mut m = modal::Modal::new(id, move |_, cx| {
        let (a, b) = (first.read(cx).text().trim().to_string(), second.read(cx).text().trim().to_string());
        let error = check(&a, &b);
        let shown = error.clone().filter(|_| tried.get() || !a.is_empty());
        let (send1, send2) = (send.clone(), send.clone());
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(modal::title(title))
            .child(field(labels.0, &first))
            .child(field(labels.1, &second))
            .when_some(shown, |d, e| d.child(div().text_color(theme::red()).child(e)))
            .child(
                modal::actions_row()
                    .child(controls::tabbable(controls::button("form-cancel", "Cancel", ButtonKind::Plain, true).on_click(move |_, window, cx| modal::close(id, window, cx)), &stops[0], move |window, cx| modal::close(id, window, cx)))
                    .child(controls::tabbable(controls::button("form-ok", confirm, ButtonKind::Primary, error.is_none()).when(error.is_none(), |d| d.on_click(move |_, window, cx| send1(window, cx))), &stops[1], move |window, cx| send2(window, cx))),
            )
            .into_any_element()
    })
    .focus(focus);
    m.keep.extend(keep);
    modal::open(m, window, cx);
}

impl GitVolt {
    /// The Remote panel's +: the name and URL, then a fetch of it.
    pub fn open_add_remote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let ctx = tab.write_target();
        let taken: Vec<String> = tab.remote_groups().into_iter().map(|g| g.0).collect();
        let this = cx.weak_entity();
        let check: Check = Rc::new(move |name, url| remote_name_error(name, &taken).or_else(|| url.is_empty().then(|| "Enter the remote's URL".to_string())));
        let submit: Submit = Rc::new(move |name, url, _, cx| {
            let ctx = ctx.clone();
            this.update(cx, |v, cx| v.add_remote(ctx, name, url, cx)).ok();
        });
        two_fields(TwoFields { id: "add-remote", title: "Add remote", labels: ("Name", "URL"), values: (String::new(), String::new()), confirm: "Add remote" }, check, submit, None, window, cx);
    }

    pub fn add_remote(&mut self, ctx: WriteCtx, name: String, url: String, cx: &mut Context<Self>) {
        let (repo, worktree, n) = (ctx.repo, ctx.worktree.clone(), name.clone());
        let w = Write::new(&ctx, move |_| call("addRemote", json!({ "repo": repo, "worktree": worktree, "name": n, "url": url }))).on_success(move |this, _, cx| {
            toast::show(format!("Added remote {name}"), cx);
            this.run_fetch(repo, false, Some(name.clone()), cx);
        });
        self.write(w, cx);
    }

    /// Asked first, in place; journaled, so Undo puts it back.
    pub fn remove_remote(&mut self, name: String, origin: Origin, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let ctx = tab.write_target();
        let body = remove_remote_text(&name, &tab.sidebar);
        let mut req = ArmRequest::danger(format!("Click again to remove {name}"), format!("Remove the remote {name}?"), "Remove remote");
        req.body = body.clone().into();
        req.caption = Some(body.into());
        let this = cx.weak_entity();
        arm::request(
            req,
            origin,
            move |a, _, cx| {
                if a.ok {
                    let (ctx, name) = (ctx.clone(), name.clone());
                    this.update(cx, |v, cx| {
                        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
                        v.write(Write::new(&ctx, move |_| call("removeRemote", json!({ "repo": repo, "worktree": worktree, "name": name }))), cx);
                    })
                    .ok();
                }
            },
            window,
            cx,
        );
    }

    /// Copy URL: the redacted one the core reports.
    pub fn copy_remote_url(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(repo) = self.tabs.get(self.active).map(|t| t.id) else { return };
        if cfg!(test) {
            return;
        }
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(Request::RepoInfo { repo }).await });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(info)) = read.await else { return };
            let url = info["remotes"].as_array().and_then(|l| l.iter().find(|r| r["name"] == name.as_str())).and_then(|r| r["url"].as_str()).map(str::to_string);
            this.update(cx, |v, cx| {
                if let Some(url) = url {
                    v.copy_text(url, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Create file…: a dialog. The core makes it empty, its folders too, and Undo removes it.
    pub fn open_create_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let ctx = tab.write_target();
        let validate: modal::Validate = Rc::new(|v| file_path_error(v).map(str::to_string));
        let req = modal::PromptRequest { title: "Create file".into(), label: "Path in the repository".into(), initial: String::new(), confirm_label: "Create".into(), validate: Some(validate), require_change: false, checkbox: None };
        let this = cx.weak_entity();
        modal::prompt(
            req,
            move |answer, _, cx| {
                let Some(answer) = answer else { return };
                let (ctx, path) = (ctx.clone(), answer.value);
                this.update(cx, |v, cx| v.create_file(ctx, path, cx)).ok();
            },
            window,
            cx,
        );
    }

    pub fn create_file(&mut self, ctx: WriteCtx, path: String, cx: &mut Context<Self>) {
        let (repo, worktree, p) = (ctx.repo, ctx.worktree.clone(), path.clone());
        let w = Write::new(&ctx, move |_| call("createWorktreeFile", json!({ "repo": repo, "worktree": worktree, "path": p }))).on_success(move |_, _, cx| toast::show(format!("Created {path}"), cx));
        self.write(w, cx);
    }

    /// `git submodule update --init` of one, or all; a network command, its feedback the op's.
    pub fn update_submodule(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        let Some(ctx) = self.tabs.get(self.active).map(|t| t.write_target()) else { return };
        let (repo, worktree, p) = (ctx.repo, ctx.worktree.clone(), path.clone());
        let w = Write::new(&ctx, move |asked| call("submoduleUpdate", json!({ "repo": repo, "worktree": worktree, "path": p, "confirm": asked.discard }))).on_success(move |_, _, cx| {
            toast::show(path.as_ref().map_or("Updated the submodules".to_string(), |p| format!("Updated {p}")), cx);
        });
        self.write(w, cx);
    }

    /// `branch`: what the worktree checks out, as the core's `WorktreeBranch`; `{ "kind": "new", "at" }` asks a name.
    pub fn open_create_worktree(&mut self, branch: Value, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let ctx = tab.write_target();
        let existing = branch["kind"] == "existing";
        let name = branch["name"].as_str().unwrap_or_default().to_string();
        let taken: Vec<String> = tab.locals().into_iter().map(|b| b.name).collect();
        let check: Check = Rc::new(move |name, path| {
            if !existing && taken.iter().any(|t| t == name) {
                return Some(format!("A branch named {name} exists"));
            }
            branch_name_error(name).or_else(|| path.is_empty().then(|| "Enter a directory".to_string()))
        });
        let this = cx.weak_entity();
        let b = branch.clone();
        let submit: Submit = Rc::new(move |name, path, _, cx| {
            let mut branch = b.clone();
            branch["name"] = json!(name);
            let ctx = ctx.clone();
            this.update(cx, |v, cx| v.add_worktree(ctx, path, branch, cx)).ok();
        });
        // The directory follows the name, until it's edited: while it still holds the last suggestion.
        let suggested = Rc::new(RefCell::new(String::new()));
        let (api, rt, repo) = (self.api.clone(), self.rt.clone(), tab.id);
        let suggest: Changed = Rc::new(move |name, dir, cx| {
            if name.is_empty() || branch_name_error(name).is_some() || cfg!(test) {
                return;
            }
            let (api, branch, dir, suggested) = (api.clone(), name.to_string(), dir.clone(), suggested.clone());
            let read = rt.spawn(async move { api.dispatch(Request::SuggestWorktreePath { repo, branch }).await });
            cx.spawn(async move |cx| {
                let Ok(Ok(path)) = read.await else { return };
                let Some(path) = path.as_str().map(str::to_string) else { return };
                dir.update(cx, |input, cx| {
                    if input.text() == suggested.borrow().as_str() {
                        input.set_text(&path, cx);
                        *suggested.borrow_mut() = path;
                    }
                });
            })
            .detach();
        });
        two_fields(TwoFields { id: "create-worktree", title: "Create worktree", labels: (if existing { "Branch" } else { "New branch" }, "Directory"), values: (name, String::new()), confirm: "Create worktree" }, check, submit, Some(suggest), window, cx);
    }

    pub fn add_worktree(&mut self, ctx: WriteCtx, path: String, branch: Value, cx: &mut Context<Self>) {
        let (repo, worktree, p) = (ctx.repo, ctx.worktree.clone(), path.clone());
        let w = Write::new(&ctx, move |_| call("worktreeAdd", json!({ "repo": repo, "worktree": worktree, "path": p, "branch": branch }))).on_success(move |this, _, cx| {
            toast::show(format!("Created worktree {path}"), cx);
            this.reload(repo, cx);
        });
        self.write(w, cx);
    }

    /// Always asked: it isn't undoable. A dirty one asks once more before `force`.
    pub fn remove_worktree(&mut self, path: String, branch: Option<String>, origin: Origin, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(main) = tab.graph.as_ref().and_then(|g| g.worktrees.iter().find(|w| w.is_main).map(|w| w.path.clone())) else { return };
        let shown = worktree_shown(&main, &path);
        let stays = branch.map_or("its commits stay".to_string(), |b| format!("branch {} stays", b.trim_start_matches("refs/heads/")));
        let mut req = ArmRequest::danger(format!("Click again to remove {shown}: its directory is deleted"), format!("Remove worktree {shown}?"), "Remove");
        req.body = format!("Its directory is deleted; {stays}.").into();
        let ctx = WriteCtx { repo: tab.id, worktree: main };
        let this = cx.weak_entity();
        let again = origin.clone();
        arm::request(
            req,
            origin,
            move |a, _, cx| {
                if a.ok {
                    let (ctx, path, shown, origin) = (ctx.clone(), path.clone(), shown.clone(), again.clone());
                    this.update(cx, |v, cx| v.send_worktree_remove(ctx, path, shown, false, origin, cx)).ok();
                }
            },
            window,
            cx,
        );
    }

    fn send_worktree_remove(&mut self, ctx: WriteCtx, path: String, shown: String, force: bool, origin: Origin, cx: &mut Context<Self>) {
        let (repo, worktree, p) = (ctx.repo, ctx.worktree.clone(), path.clone());
        // A tab on the removed worktree moves to the main one first.
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo && t.worktree == path) {
            tab.worktree = worktree.clone();
        }
        let (c2, o2) = (ctx.clone(), origin.clone());
        let w = Write::new(&ctx, move |_| call("worktreeRemove", json!({ "repo": repo, "worktree": worktree, "path": p, "force": force }))).on_success(move |this, out, cx| {
            this.reload(repo, cx);
            if out["status"] != "needsForce" {
                return;
            }
            let body = format!("{shown} has changes that aren't committed. They're lost: this can't be undone.");
            let mut req = ArmRequest::danger(format!("Click again to remove {shown}: its uncommitted changes are lost"), "Remove it anyway?", "Remove");
            req.body = body.clone().into();
            req.caption = Some(body.into());
            let weak = cx.weak_entity();
            let (ctx, path, shown, origin, again) = (c2.clone(), path.clone(), shown.clone(), o2.clone(), o2.clone());
            this.with_window(cx, move |window, cx| {
                arm::request(
                    req,
                    origin,
                    move |a, _, cx| {
                        if a.ok {
                            let (ctx, path, shown, origin) = (ctx.clone(), path.clone(), shown.clone(), again.clone());
                            weak.update(cx, |v, cx| v.send_worktree_remove(ctx, path, shown, true, origin, cx)).ok();
                        }
                    },
                    window,
                    cx,
                );
            });
        });
        self.write(w.origin(origin), cx);
    }

    /// Switch to: this tab's worktree becomes `path` (its WIP, commit box and Undo).
    pub fn switch_worktree(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        tab.worktree = path;
        let repo = tab.id;
        self.reload(repo, cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::testing::Fixture;
    use std::prelude::v1::test;

    #[test]
    fn a_new_files_path_is_checked_as_the_web_does() {
        assert_eq!(file_path_error("src/a.rs"), None);
        assert_eq!(file_path_error(" "), Some("Enter a file name"));
        assert_eq!(file_path_error("/etc/x"), Some("Enter a path relative to the repository"));
        assert_eq!(file_path_error("C:/x"), Some("Enter a path relative to the repository"));
        assert_eq!(file_path_error("../x"), Some("The path can't leave the repository (..)"));
        assert_eq!(file_path_error(".git/x"), Some("Files can't be created in .git"));
        assert_eq!(file_path_error("a/"), Some("Enter a file name after the folder"));
        assert_eq!(file_path_error("a//b"), Some("The path has an empty or \".\" folder"));
    }

    #[test]
    fn removing_a_remote_says_what_it_costs() {
        let sidebar = json!({ "remotes": [{ "name": "up", "branches": [{}, {}] }], "locals": [{ "name": "main", "upstream": "refs/remotes/up/main" }, { "name": "x" }] });
        assert_eq!(remove_remote_text("up", &sidebar), "Its 2 remote branches leave this repository. 1 local branch tracking it loses its upstream: main. Undo puts it all back.");
    }

    #[test]
    fn a_worktree_beside_the_main_one_is_shown_relative() {
        assert_eq!(worktree_shown("/w/shop", "/w/shop-x"), "../shop-x");
        assert_eq!(worktree_shown("/w/shop", "/other/x"), "/other/x");
    }

    #[tokio::test]
    async fn remotes_worktrees_and_a_new_file_go_through_the_core() {
        let f = Fixture::new().await;
        let (repo, wt) = (f.repo, f.worktree.clone());
        f.ok(call("addRemote", json!({ "repo": repo, "worktree": wt, "name": "backup", "url": "/nowhere/fictional.git" }))).await;
        assert_eq!(f.git(&["remote"]), "backup");
        f.ok(call("removeRemote", json!({ "repo": repo, "worktree": wt, "name": "backup" }))).await;
        assert_eq!(f.git(&["remote"]), "");
        let dir = format!("{wt}-tree");
        let head = f.git(&["rev-parse", "HEAD"]);
        f.ok(call("worktreeAdd", json!({ "repo": repo, "worktree": wt, "path": dir, "branch": { "kind": "new", "name": "side", "at": head } }))).await;
        assert!(f.git(&["worktree", "list"]).contains("side"));
        std::fs::write(format!("{dir}/dirty.txt"), "x").expect("a file");
        let out = f.ok(call("worktreeRemove", json!({ "repo": repo, "worktree": wt, "path": dir, "force": false }))).await;
        assert_eq!(out["outcome"]["status"], "needsForce", "{out}");
        let out = f.ok(call("worktreeRemove", json!({ "repo": repo, "worktree": wt, "path": dir, "force": true }))).await;
        assert_eq!(out["outcome"]["status"], "removed");
        assert!(!f.git(&["worktree", "list"]).contains("side"));
        f.ok(call("createWorktreeFile", json!({ "repo": repo, "worktree": wt, "path": "deep/new.txt" }))).await;
        assert_eq!(f.read("deep/new.txt"), "");
    }
}
