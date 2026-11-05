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

/// `remoteNameError`: what a branch's name may be, no more.
pub fn remote_name_error(name: &str, taken: &[String]) -> Option<String> {
    if let Some(e) = branch_name_error(name) {
        return Some(e.replace("branch", "remote"));
    }
    taken.iter().any(|t| t == name).then(|| format!("A remote named {name} already exists"))
}

/// `remoteUrlError`: empty, option-like, or with whitespace or control characters.
pub fn remote_url_error(url: &str) -> Option<String> {
    if url.is_empty() {
        return Some("Enter the remote's URL".to_string());
    }
    if url.starts_with('-') {
        return Some("A remote URL can't start with -".to_string());
    }
    url.chars().any(|c| c.is_whitespace() || c.is_control()).then(|| "A remote URL can't contain spaces or control characters".to_string())
}

/// `parseRemoteUrl`: host and project path, of an https, ssh or scp-style URL; a local one has none.
pub fn parse_remote_url(url: &str) -> Option<(String, String)> {
    let u = url.trim();
    let (host, path) = if let Some(at) = u.find("://") {
        if u[..at].eq_ignore_ascii_case("file") {
            return None;
        }
        let rest = &u[at + 3..];
        let (authority, path) = rest.split_once('/')?;
        (authority.rsplit('@').next().unwrap_or_default().split(':').next().unwrap_or_default().to_string(), path.to_string())
    } else {
        if u.starts_with('/') || u.starts_with('.') {
            return None;
        }
        let (left, path) = u.split_once(':')?;
        if left.contains('/') {
            return None;
        }
        (left.rsplit('@').next().unwrap_or_default().to_string(), path.to_string())
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path).to_string();
    (!host.is_empty() && !path.is_empty()).then_some((host, path))
}

/// `freeRemoteName`: `base` as a remote name (lowercase, unsafe characters as `-`), or `base-2`, `base-3`… when taken.
pub fn free_remote_name(base: &str, taken: &[String]) -> String {
    let mut clean = String::new();
    for c in base.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            clean.push(c);
        } else if !clean.ends_with('-') {
            clean.push('-');
        }
    }
    let clean = clean.trim_matches(['-', '.']).to_string();
    let clean = if clean.is_empty() { "remote".to_string() } else { clean };
    if !taken.contains(&clean) {
        return clean;
    }
    (2..).map(|n| format!("{clean}-{n}")).find(|c| !taken.contains(c)).unwrap_or(clean)
}

type Suggest = Rc<dyn Fn(&str, &mut App)>;

/// The inline "new file path" input on top of a file list.
pub struct CreateFile {
    pub section: Option<crate::wip::Which>,
    pub input: Entity<TextInput>,
    pub busy: bool,
}

/// `CreateFileInput`: validated live; Enter creates, Esc cancels, and leaving it empty cancels too.
pub fn create_file_input(c: &CreateFile, cx: &App) -> Div {
    let text = c.input.read(cx).text();
    let error = if text.is_empty() { None } else { file_path_error(text) };
    let border = if error.is_some() { theme::red() } else { theme::blue() };
    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap(px(3.))
        .px(px(8.))
        .py(px(4.))
        .bg(theme::panel_bg1())
        .child(div().flex().items_center().h(px(24.)).px(px(6.)).bg(theme::app_bg()).border_1().border_color(border).rounded(px(3.)).text_color(theme::text_selected()).child(c.input.clone()))
        .child(div().text_size(px(11.)).text_color(if error.is_some() { theme::red() } else { theme::text_normal() }).child(error.unwrap_or("Enter: create · Esc: cancel · folders are made as needed")))
}

/// A worktree's path as the row shows it: relative to the main one's parent when it's beside it.
pub fn worktree_shown(main: &str, path: &str) -> String {
    let parent = main.rsplit_once('/').map_or("", |p| p.0);
    match path.strip_prefix(parent).and_then(|p| p.strip_prefix('/')) {
        Some(rest) if !parent.is_empty() => format!("../{rest}"),
        _ => path.to_string(),
    }
}

impl GitVolt {
    /// The Remote panel's +: the name and URL, then a fetch of it.
    /// Add remote: the URL, what it's on, a name that follows its owner until it's edited. No forks list: that's the
    /// forge layer's, not here yet.
    pub fn open_add_remote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        const ID: &str = "add-remote";
        let Some(tab) = self.tabs.get(self.active) else { return };
        let ctx = tab.write_target();
        let taken: Vec<String> = tab.remote_groups().into_iter().map(|g| g.0).collect();
        let url = cx.new(|cx| TextInput::new(cx).placeholder("https://gitlab.example.com/group/project.git"));
        let name = cx.new(TextInput::new);
        let edited = Rc::new(Cell::new(false));
        let this = cx.weak_entity();
        let effective = {
            let (name, edited, taken) = (name.clone(), edited.clone(), taken.clone());
            move |url_text: &str, cx: &App| -> String {
                if edited.get() {
                    return name.read(cx).text().to_string();
                }
                parse_remote_url(url_text).map_or(String::new(), |(_, path)| free_remote_name(path.split('/').next().unwrap_or_default(), &taken))
            }
        };
        let send: crate::ui::Action = {
            let (url, effective, taken, ctx) = (url.clone(), effective.clone(), taken.clone(), ctx.clone());
            Rc::new(move |window, cx| {
                let u = url.read(cx).text().trim().to_string();
                let n = effective(&u, cx);
                if remote_url_error(&u).is_some() || remote_name_error(&n, &taken).is_some() {
                    return;
                }
                modal::close(ID, window, cx);
                let ctx = ctx.clone();
                this.update(cx, |v, cx| v.add_remote(ctx, n, u, cx)).ok();
            })
        };
        let handle = window.window_handle();
        let watch = |input: &Entity<TextInput>, typed: Option<Rc<Cell<bool>>>, cx: &mut App| {
            let send = send.clone();
            cx.subscribe(input, move |_, e: &InputEvent, cx| match e {
                InputEvent::Submit { .. } => {
                    let send = send.clone();
                    handle.update(cx, move |_, window, cx| window.defer(cx, move |window, cx| send(window, cx))).ok();
                }
                InputEvent::Changed => {
                    if let Some(t) = &typed {
                        t.set(true);
                    }
                    cx.refresh_windows();
                }
                _ => {}
            })
        };
        let keep = [watch(&url, None, cx), watch(&name, Some(edited.clone()), cx)];
        let focus = url.read(cx).focus.clone();
        let stops = [cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true)];
        let mut m = modal::Modal::new(ID, move |window, cx| {
            let u = url.read(cx).text().trim().to_string();
            let n = effective(&u, cx);
            if !edited.get() && name.read(cx).text() != n {
                name.update(cx, |i, cx| i.set_text(&n, cx));
            }
            let parsed = parse_remote_url(&u);
            let error = remote_url_error(&u).or_else(|| remote_name_error(&n, &taken));
            let (send1, send2) = (send.clone(), send.clone());
            let focused_field = |input: &Entity<TextInput>, cx: &App| input.read(cx).is_focused(window);
            let boxed = |label: &'static str, input: &Entity<TextInput>, on: bool| div().flex().flex_col().gap(px(4.)).child(label).child(div().h(px(30.2)).px(px(8.)).flex().items_center().bg(theme::app_bg()).border_1().border_color(if on { theme::blue() } else { theme::section_border() }).rounded(px(4.)).text_color(theme::text_selected()).child(input.clone()));
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(modal::title("Add remote"))
                .child(boxed("URL", &url, focused_field(&url, cx)))
                .children(parsed.map(|(host, path)| div().flex().items_center().gap(px(6.)).mt(px(-4.)).text_size(px(12.)).text_color(theme::text_secondary()).child(crate::app::icon("cloud", 14., theme::text_secondary())).child(format!("{host}/{path}"))))
                .child(boxed("Name", &name, focused_field(&name, cx)))
                .when_some(error.clone().filter(|_| !u.is_empty()), |d, e| d.child(div().text_color(theme::red()).child(e)))
                .child(
                    modal::actions_row()
                        .child(controls::tabbable(controls::button("form-cancel", "Cancel", ButtonKind::Plain, true).on_click(move |_, window, cx| modal::close(ID, window, cx)), &stops[0], move |window, cx| modal::close(ID, window, cx)))
                        .child(controls::tabbable(controls::button("form-ok", "Add remote", ButtonKind::Primary, error.is_none()).when(error.is_none(), |d| d.on_click(move |_, window, cx| send1(window, cx))), &stops[1], move |window, cx| send2(window, cx))),
                )
                .into_any_element()
        })
        .width(520.)
        .focus(focus);
        m.keep.extend(keep);
        modal::open(m, window, cx);
    }

    pub fn add_remote(&mut self, ctx: WriteCtx, name: String, url: String, cx: &mut Context<Self>) {
        let (repo, worktree, n) = (ctx.repo, ctx.worktree.clone(), name.clone());
        let w = Write::new(&ctx, move |_| call("addRemote", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "name": n, "url": url }))).on_success(move |this, _, cx| {
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
                        v.write(Write::new(&ctx, move |_| call("removeRemote", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "name": name }))), cx);
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
        let read = self.rt.spawn(async move { api.dispatch(Request::RepoInfo { repo: crate::app::core_repo(repo) }).await });
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
    /// The palette's Create file…: a dialog, a keyboard-started action.
    pub fn open_create_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let ctx = tab.write_target();
        let validate: modal::Validate = Rc::new(|v| file_path_error(v).map(str::to_string));
        let req = modal::PromptRequest { title: "Create a file".into(), label: "File path".into(), initial: String::new(), confirm_label: "Create file".into(), validate: Some(validate), ..Default::default() };
        let this = cx.weak_entity();
        modal::prompt(
            req,
            move |answer, _, cx| {
                let Some(answer) = answer else { return };
                let (ctx, path) = (ctx.clone(), answer.value);
                this.update(cx, |v, cx| v.create_file(ctx, path, None, cx)).ok();
            },
            window,
            cx,
        );
    }

    /// Create file… from a file list: the inline input on top of that list. `prefill`: a folder's path.
    pub fn open_create_file_inline(&mut self, section: Option<crate::wip::Which>, prefill: Option<String>, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let repo = tab.id;
        let input = cx.new(|cx| {
            let mut i = TextInput::new(cx).placeholder("path/to/new-file").claim_escape();
            if let Some(p) = &prefill {
                i.set_text(p, cx);
            }
            i
        });
        cx.subscribe(&input, move |this, input, e: &InputEvent, cx| {
            match e {
                InputEvent::Escape => this.close_create_file(repo, cx),
                InputEvent::Blurred if input.read(cx).text().is_empty() => this.close_create_file(repo, cx),
                InputEvent::Submit { .. } => this.submit_create_file(repo, cx),
                _ => {}
            }
            cx.notify();
        })
        .detach();
        self.focus_next_frame = Some(input.read(cx).focus.clone());
        tab.create_file = Some(CreateFile { section, input, busy: false });
        cx.notify();
    }

    pub fn close_create_file(&mut self, repo: u32, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo)
            && tab.create_file.take().is_some()
        {
            self.focus_next_frame = Some(self.focus.clone());
            cx.notify();
        }
    }

    fn submit_create_file(&mut self, repo: u32, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) else { return };
        let Some(c) = tab.create_file.as_mut().filter(|c| !c.busy) else { return };
        let path = c.input.read(cx).text().to_string();
        if path.is_empty() || file_path_error(&path).is_some() {
            return;
        }
        c.busy = true;
        let ctx = tab.write_target();
        self.create_file(ctx, path, Some(repo), cx);
    }

    /// Makes `path` in the worktree (journaled: Undo removes it), then opens it in File View, to be typed into.
    /// `inline`: the input that asked stays, with the name, if the core refuses it.
    pub fn create_file(&mut self, ctx: WriteCtx, path: String, inline: Option<u32>, cx: &mut Context<Self>) {
        let (repo, worktree, p) = (ctx.repo, ctx.worktree.clone(), path.clone());
        let w = Write::new(&ctx, move |_| call("createWorktreeFile", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "path": p })))
            .on_success(move |this, _, cx| {
                if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) {
                    tab.create_file = None;
                    tab.open_after_wip = Some(path.clone());
                }
                this.reveal_wip(repo, cx);
            })
            .on_end(move |this, _, cx| {
                if let Some(tab) = inline.and_then(|r| this.tabs.iter_mut().find(|t| t.id == r))
                    && let Some(c) = tab.create_file.as_mut()
                {
                    c.busy = false;
                    cx.notify();
                }
            });
        self.write(w, cx);
    }

    /// Create worktree: the branch, a directory that follows it until it's edited, Browse…, and Open in a new tab.
    pub fn open_create_worktree(&mut self, branch: Value, window: &mut Window, cx: &mut Context<Self>) {
        const ID: &str = "create-worktree";
        let Some(tab) = self.tabs.get(self.active) else { return };
        let ctx = tab.write_target();
        let existing = branch["kind"] == "existing";
        let taken: Vec<String> = tab.locals().into_iter().map(|b| b.name).collect();
        let first = branch["name"].as_str().unwrap_or_default().to_string();
        let name = cx.new(|cx| {
            let mut i = TextInput::new(cx);
            i.set_text(&first, cx);
            i.set_disabled(existing, cx);
            i
        });
        let dir = cx.new(TextInput::new);
        let edited = Rc::new(Cell::new(false));
        let suggested = Rc::new(RefCell::new(String::new()));
        let suggest_error = Rc::new(RefCell::new(None::<String>));
        let open_tab = Rc::new(Cell::new(true));
        let (api, rt, repo) = (self.api.clone(), self.rt.clone(), tab.id);
        let suggest: Suggest = {
            let (dir, edited, suggested, suggest_error) = (dir.clone(), edited.clone(), suggested.clone(), suggest_error.clone());
            Rc::new(move |name: &str, cx: &mut App| {
                if edited.get() || name.is_empty() || branch_name_error(name).is_some() || cfg!(test) {
                    return;
                }
                let (api, branch, dir, suggested, suggest_error) = (api.clone(), name.to_string(), dir.clone(), suggested.clone(), suggest_error.clone());
                let read = rt.spawn(async move { api.dispatch(Request::SuggestWorktreePath { repo: crate::app::core_repo(repo), branch }).await });
                cx.spawn(async move |cx| {
                    let answer = read.await;
                    cx.update(|cx| match answer {
                        Ok(Ok(path)) => {
                            let Some(path) = path.as_str().map(str::to_string) else { return };
                            *suggest_error.borrow_mut() = None;
                            dir.update(cx, |input, cx| {
                                if input.text() == suggested.borrow().as_str() {
                                    input.set_text(&path, cx);
                                    *suggested.borrow_mut() = path;
                                }
                            });
                            cx.refresh_windows();
                        }
                        Ok(Err(e)) => {
                            *suggest_error.borrow_mut() = Some(format!("Couldn't suggest a directory ({}): type one", e.message));
                            cx.refresh_windows();
                        }
                        Err(_) => {}
                    });
                })
                .detach();
            })
        };
        let check = {
            let taken = taken.clone();
            move |n: &str| -> Option<String> {
                if existing {
                    return None;
                }
                branch_name_error(n).or_else(|| taken.iter().any(|t| t == n).then(|| format!("A branch named {n} already exists")))
            }
        };
        let this = cx.weak_entity();
        let send: crate::ui::Action = {
            let (name, dir, check, ctx, branch, open_tab, this) = (name.clone(), dir.clone(), check.clone(), ctx.clone(), branch.clone(), open_tab.clone(), this.clone());
            Rc::new(move |window, cx| {
                let (n, path) = (name.read(cx).text().to_string(), dir.read(cx).text().to_string());
                if check(&n).is_some() || path.is_empty() {
                    return;
                }
                modal::close(ID, window, cx);
                let mut branch = branch.clone();
                branch["name"] = json!(n);
                let (ctx, open) = (ctx.clone(), open_tab.get());
                this.update(cx, |v, cx| v.add_worktree(ctx, path, branch, open, cx)).ok();
            })
        };
        let handle = window.window_handle();
        let keep = [
            App::subscribe(cx, &name, {
                let (send, suggest) = (send.clone(), suggest.clone());
                move |input, e: &InputEvent, cx| match e {
                    InputEvent::Submit { .. } => {
                        let send = send.clone();
                        handle.update(cx, move |_, window, cx| window.defer(cx, move |window, cx| send(window, cx))).ok();
                    }
                    InputEvent::Changed => {
                        let t = input.read(cx).text().to_string();
                        suggest(&t, cx);
                        cx.refresh_windows();
                    }
                    _ => {}
                }
            }),
            App::subscribe(cx, &dir, {
                let (send, edited) = (send.clone(), edited.clone());
                move |_, e: &InputEvent, cx| match e {
                    InputEvent::Submit { .. } => {
                        let send = send.clone();
                        handle.update(cx, move |_, window, cx| window.defer(cx, move |window, cx| send(window, cx))).ok();
                    }
                    InputEvent::Changed => {
                        edited.set(true);
                        cx.refresh_windows();
                    }
                    _ => {}
                }
            }),
        ];
        if !first.is_empty() {
            suggest(&first, cx);
        }
        let focus = if existing { dir.read(cx).focus.clone() } else { name.read(cx).focus.clone() };
        let stops = [cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true), cx.focus_handle().tab_stop(true)];
        let weak = cx.weak_entity();
        let mut m = modal::Modal::new(ID, move |window, cx| {
            let (n, path) = (name.read(cx).text().to_string(), dir.read(cx).text().to_string());
            let error = check(&n).filter(|_| !n.is_empty());
            let can = check(&n).is_none() && !path.is_empty();
            let (send1, send2) = (send.clone(), send.clone());
            let (t, t2) = (open_tab.clone(), open_tab.clone());
            let browse: crate::ui::Action = {
                let (dir, edited, weak, path) = (dir.clone(), edited.clone(), weak.clone(), path.clone());
                Rc::new(move |_, cx| {
                    let (dir, edited, start) = (dir.clone(), edited.clone(), path.rsplit_once('/').map(|p| p.0.to_string()));
                    weak.update(cx, move |v, cx| {
                        v.pick_folder(start, move |_, picked, cx| {
                            if let Some(p) = picked {
                                edited.set(true);
                                dir.update(cx, |i, cx| i.set_text(&p, cx));
                                cx.notify();
                            }
                        }, cx);
                    })
                    .ok();
                })
            };
            let b2 = browse.clone();
            let box_of = |input: &Entity<TextInput>, on: bool| div().flex().flex_1().min_w_0().h(px(30.2)).px(px(8.)).items_center().bg(theme::app_bg()).border_1().border_color(if on { theme::blue() } else { theme::section_border() }).rounded(px(4.)).text_color(theme::text_selected()).child(input.clone());
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(modal::title("Create worktree"))
                .child(div().flex().flex_col().gap(px(4.)).child(if existing { "Branch" } else { "New branch" }).child(div().flex().child(box_of(&name, name.read(cx).is_focused(window)))))
                .when_some(error, |d, e| d.child(div().text_color(theme::red()).child(e)))
                .child(div().flex().flex_col().gap(px(4.)).child("Directory").child(div().flex().items_center().gap(px(6.)).child(box_of(&dir, dir.read(cx).is_focused(window))).child(crate::ui::anchor("wt-browse", controls::tabbable(controls::button("wt-browse", "Browse…", ButtonKind::Plain, true).on_click(move |_, window, cx| browse(window, cx)), &stops[0], move |window, cx| b2(window, cx))))))
                .when_some(suggest_error.borrow().clone().filter(|_| path.is_empty()), |d, e| d.child(div().text_color(theme::red()).child(e)))
                .child(controls::tabbable(
                    controls::check_row("wt-open", open_tab.get(), "Open in a new tab", move |on, _, cx| {
                        t.set(on);
                        cx.refresh_windows();
                    })
                    .border_1()
                    .border_color(transparent_black())
                    .rounded(px(3.))
                    .ml(px(-1.)),
                    &stops[1],
                    move |_, cx| {
                        t2.set(!t2.get());
                        cx.refresh_windows();
                    },
                ))
                .child(
                    modal::actions_row()
                        .child(controls::tabbable(controls::button("form-cancel", "Cancel", ButtonKind::Plain, true).on_click(move |_, window, cx| modal::close(ID, window, cx)), &stops[2], move |window, cx| modal::close(ID, window, cx)))
                        .child(crate::ui::anchor("form-ok", controls::tabbable(controls::button("form-ok", "Create worktree", ButtonKind::Primary, can).when(can, |d| d.on_click(move |_, window, cx| send1(window, cx))), &stops[3], move |window, cx| send2(window, cx)))),
                )
                .into_any_element()
        })
        .focus(focus);
        m.keep.extend(keep);
        modal::open(m, window, cx);
    }

    /// `open`: the new worktree shows at once. One tab per repository here: it's this tab that goes to it.
    pub fn add_worktree(&mut self, ctx: WriteCtx, path: String, branch: Value, open: bool, cx: &mut Context<Self>) {
        let (repo, worktree, p) = (ctx.repo, ctx.worktree.clone(), path.clone());
        let w = Write::new(&ctx, move |_| call("worktreeAdd", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "path": p, "branch": branch }))).on_success(move |this, out, cx| {
            let made = out["path"].as_str().map_or_else(|| path.clone(), str::to_string);
            toast::show(format!("Created worktree {made}"), cx);
            this.reload(repo, cx);
            if open {
                this.switch_worktree(made, cx);
            }
        });
        self.write(w, cx);
    }

    /// Always asked: it isn't undoable. A dirty one asks once more before `force`.
    pub fn remove_worktree(&mut self, path: String, branch: Option<String>, with_branch: bool, origin: Origin, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(main) = tab.graph.as_ref().and_then(|g| g.worktrees.iter().find(|w| w.is_main).map(|w| w.path.clone())) else { return };
        let shown = worktree_shown(&main, &path);
        let name = branch.map(|b| b.trim_start_matches("refs/heads/").to_string());
        let stays = name.as_ref().map_or("its commits stay".to_string(), |b| format!("branch {b} stays"));
        let delete = name.clone().filter(|_| with_branch).and_then(|b| tab.local_branch(&b).map(|l| (l.name, l.target)));
        let mut req = match &delete {
            Some((b, _)) => {
                let mut r = ArmRequest::danger(format!("Click again to remove {shown} and delete {b}"), format!("Remove worktree {shown} and delete {b}?"), "Remove and delete");
                r.body = "Its directory is deleted (that can't be undone); you can undo deleting the branch.".into();
                r
            }
            None => ArmRequest::danger(format!("Click again to remove {shown}: its directory is deleted"), format!("Remove worktree {shown}?"), "Remove"),
        };
        if delete.is_none() {
            req.body = format!("Its directory is deleted; {stays}.").into();
        }
        let ctx = WriteCtx { repo: tab.id, worktree: main };
        let this = cx.weak_entity();
        let again = origin.clone();
        arm::request(
            req,
            origin,
            move |a, _, cx| {
                if a.ok {
                    let (ctx, path, shown, origin) = (ctx.clone(), path.clone(), shown.clone(), again.clone());
                    let delete = delete.clone();
                    this.update(cx, |v, cx| v.send_worktree_remove(ctx, path, shown, false, delete, origin, cx)).ok();
                }
            },
            window,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn send_worktree_remove(&mut self, ctx: WriteCtx, path: String, shown: String, force: bool, delete: Option<(String, String)>, origin: Origin, cx: &mut Context<Self>) {
        let (repo, worktree, p) = (ctx.repo, ctx.worktree.clone(), path.clone());
        // A tab on the removed worktree moves to the main one first.
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo && t.worktree == path) {
            tab.worktree = worktree.clone();
        }
        let (c2, o2) = (ctx.clone(), origin.clone());
        let d2 = delete.clone();
        let w = Write::new(&ctx, move |_| call("worktreeRemove", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "path": p, "force": force }))).on_success(move |this, out, cx| {
            this.reload(repo, cx);
            if out["status"] != "needsForce" {
                // With its branch: then the branch's delete, from the main worktree, with its unmerged question.
                if let Some((name, oid)) = d2.clone() {
                    let plan = crate::refs::DeletePlan { branch: name, local: Some(oid), remote: None, unpushed: None };
                    this.send_delete(c2.clone(), plan, false, o2.clone(), cx);
                }
                return;
            }
            let delete = d2.clone();
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
                            let delete = delete.clone();
                            weak.update(cx, |v, cx| v.send_worktree_remove(ctx, path, shown, true, delete, origin, cx)).ok();
                        }
                    },
                    window,
                    cx,
                );
            });
        });
        self.write(w.origin(origin), cx);
    }

    /// Switch worktree…: the palette's picker of the others.
    pub fn choose_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(g) = tab.graph.clone() else { return };
        let main = g.worktrees.iter().find(|w| w.is_main).map_or(String::new(), |w| w.path.clone());
        let this = cx.weak_entity();
        let entries = g
            .worktrees
            .iter()
            .filter(|w| w.path != tab.worktree)
            .map(|w| {
                let (weak, path) = (this.clone(), w.path.clone());
                crate::palette::Entry {
                    group: crate::palette::PaletteGroup::Choice,
                    label: worktree_shown(&main, &w.path).into(),
                    detail: Some(w.branch.as_deref().map_or("detached".to_string(), |b| b.trim_start_matches("refs/heads/").to_string()).into()),
                    run: Rc::new(move |_, cx| {
                        let path = path.clone();
                        weak.update(cx, |v, cx| v.switch_worktree(path, cx)).ok();
                    }),
                    alt: None,
                }
            })
            .collect();
        self.choose(crate::palette::Choices { title: "Switch worktree".into(), placeholder: "Switch this tab to a worktree".into(), entries }, window, cx);
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
        f.ok(call("addRemote", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "name": "backup", "url": "/nowhere/fictional.git" }))).await;
        assert_eq!(f.git(&["remote"]), "backup");
        f.ok(call("removeRemote", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "name": "backup" }))).await;
        assert_eq!(f.git(&["remote"]), "");
        let dir = format!("{wt}-tree");
        let head = f.git(&["rev-parse", "HEAD"]);
        f.ok(call("worktreeAdd", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "path": dir, "branch": { "kind": "new", "name": "side", "at": head } }))).await;
        assert!(f.git(&["worktree", "list"]).contains("side"));
        std::fs::write(format!("{dir}/dirty.txt"), "x").expect("a file");
        let out = f.ok(call("worktreeRemove", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "path": dir, "force": false }))).await;
        assert_eq!(out["outcome"]["status"], "needsForce", "{out}");
        let out = f.ok(call("worktreeRemove", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "path": dir, "force": true }))).await;
        assert_eq!(out["outcome"]["status"], "removed");
        assert!(!f.git(&["worktree", "list"]).contains("side"));
        f.ok(call("createWorktreeFile", json!({ "repo": crate::app::core_repo(repo), "worktree": wt, "path": "deep/new.txt" }))).await;
        assert_eq!(f.read("deep/new.txt"), "");
    }
}
