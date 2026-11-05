//! "Open in…" (`ui/src/openIn`): the diff toolbar's split button, and its menu of every opener.

use crate::app::{GitVolt, Tab, icon};
use crate::details::{FileChange, Origin};
use crate::theme;
use gitvolt_core::api::Request;
use gpui::prelude::*;
use gpui::*;
use serde_json::Value;
use std::cell::Cell;
use std::rc::Rc;

const ROW_H: f32 = 26.;
const MENU_W: f32 = 220.;

/// One entry, as the core's `ListOpenersFor` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opener {
    pub id: String,
    pub name: String,
    /// `editor`, `fileManager`, `chooser` or `defaultApp`.
    pub kind: String,
}

pub fn openers_of(v: &Value) -> Vec<Opener> {
    v.as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|o| {
            let text = |k: &str| o[k].as_str().unwrap_or_default().to_string();
            Opener { id: text("id"), name: text("name"), kind: text("kind") }
        })
        .collect()
}

/// The last used, else VS Code, else the first editor, else the file manager, else "Other…". Never
/// the default app: it stays a menu row, since what it does depends on the file.
pub fn default_opener<'a>(list: &'a [Opener], last: Option<&str>) -> Option<&'a Opener> {
    let usable = || list.iter().filter(|o| o.kind != "defaultApp");
    usable().find(|o| Some(o.id.as_str()) == last).or_else(|| usable().find(|o| o.id == "vscode")).or_else(|| usable().find(|o| o.kind == "editor")).or_else(|| usable().find(|o| o.kind == "fileManager")).or_else(|| usable().next())
}

fn is_html(path: &str) -> bool {
    let lower = path.to_lowercase();
    [".html", ".htm", ".xhtml"].iter().any(|e| lower.ends_with(e))
}

/// The extensions the core never opens in their default app, since the OS would run the file
/// (`openers::default_app::PROGRAM_EXTENSIONS`; keep the two lists the same).
const PROGRAM_EXTENSIONS: [&str; 41] = [
    "exe", "bat", "cmd", "com", "scr", "vbs", "vbe", "js", "jse", "wsf", "wsh", "hta", "msi", "msp", "cpl", "lnk", "url", "reg", "jar", "ps1", "psm1", "scf", "pif", "application", "appref-ms", "desktop", "sh", "bash", "zsh", "command", "tool", "terminal", "app", "appimage", "run", "fileloc", "inetloc", "webloc", "scpt",
    "applescript", "workflow",
];

/// Whether the OS would run `path` as a program, by its name (trailing dots and spaces dropped).
pub fn runs_as_program(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or_default().trim_end_matches(['.', ' ']);
    name.rsplit_once('.').is_some_and(|(_, ext)| PROGRAM_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
}

pub fn opener_label(o: &Opener, path: &str) -> String {
    match o.kind.as_str() {
        "defaultApp" if is_html(path) => "Open in browser".into(),
        "defaultApp" => "Open in default app".into(),
        "chooser" => o.name.clone(),
        "fileManager" => format!("Show in {}", o.name),
        _ => format!("Open in {}", o.name),
    }
}

pub fn opener_icon(o: &Opener, path: &str) -> &'static str {
    match o.kind.as_str() {
        "defaultApp" if is_html(path) => "globe",
        "defaultApp" => "mouse-pointer-click",
        "chooser" => "app-window",
        "fileManager" => "folder-open",
        _ => "file-code",
    }
}

/// A submodule is a folder too: no default app row.
fn is_submodule(f: &FileChange) -> bool {
    f.old["kind"].as_str() == Some("submodule") || f.new["kind"].as_str() == Some("submodule")
}

/// The menu's rows, in the core's order, with a separator before what isn't an editor.
pub fn menu_rows(list: &[Opener], file: &FileChange) -> Vec<Option<Opener>> {
    let shown: Vec<&Opener> = list.iter().filter(|o| !(is_submodule(file) && o.kind == "defaultApp")).collect();
    let first_other = shown.iter().position(|o| o.kind != "editor").unwrap_or(0);
    let mut rows = Vec::with_capacity(shown.len() + 1);
    for (i, o) in shown.iter().enumerate() {
        if i == first_other && i > 0 {
            rows.push(None);
        }
        rows.push(Some((*o).clone()));
    }
    rows
}

fn row_disabled(o: &Opener, path: &str) -> bool {
    o.kind == "defaultApp" && runs_as_program(path)
}

fn stored(side: &Value) -> bool {
    matches!(side["kind"].as_str(), Some("object" | "atCommit"))
}

/// The version it opens: in a WIP list the working-tree file, which is what gets edited, or if it's
/// gone the list's stored one; elsewhere the version shown, the new side or a deleted file's old one.
pub fn open_request(repo: u32, tab_worktree: &str, file: &FileChange, opener: &str) -> Result<Request, serde_json::Error> {
    let (worktree, source, fallback) = match &file.origin {
        Origin::Wip { worktree, .. } => (worktree.clone(), serde_json::json!({ "kind": "worktree", "worktree": worktree }), [&file.new, &file.old].into_iter().find(|s| stored(s)).cloned()),
        Origin::Commit { .. } => {
            let side = if file.new["kind"].as_str() == Some("absent") { &file.old } else { &file.new };
            (tab_worktree.to_string(), side.clone(), None)
        }
    };
    Ok(Request::OpenIn {
        repo,
        worktree,
        path: file.path.to_string(),
        line: None,
        opener: opener.to_string(),
        source: Some(serde_json::from_value(source)?),
        fallback: fallback.map(serde_json::from_value).transpose()?,
    })
}

/// A tab's button: its openers, once listed, and its menu.
#[derive(Default)]
pub struct OpenIn {
    pub openers: Option<Rc<Vec<Opener>>>,
    /// The menu is open, on this row.
    pub menu: Option<usize>,
    pub toggle: Rc<Cell<Bounds<Pixels>>>,
}

impl GitVolt {
    /// Asks the core for the openers: when a tab opens, and again as its menu opens.
    pub fn list_openers(&mut self, repo: u32, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let api = self.api.clone();
        let read = self.rt.spawn(async move { api.dispatch(Request::ListOpenersFor { repo }).await });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(v)) = read.await else { return };
            this.update(cx, |this, cx| {
                if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) {
                    tab.open_in.openers = Some(Rc::new(openers_of(&v)));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Opens the open file with `opener`, remembered as the default (not the default app).
    pub fn open_with(&mut self, opener: Opener, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        tab.open_in.menu = None;
        let Some(file) = tab.diff.as_ref().map(|d| d.file.clone()) else { return };
        if opener.kind != "defaultApp" {
            self.last_opener = Some(opener.id.clone());
        }
        cx.notify();
        let request = match open_request(tab.id, &tab.worktree, &file, &opener.id) {
            Ok(r) => r,
            Err(e) => return self.show_toast(format!("Couldn't open {}: {e}", file.path), cx),
        };
        if cfg!(test) {
            return;
        }
        let api = self.api.clone();
        let done = self.rt.spawn(async move { api.dispatch(request).await });
        cx.spawn(async move |this, cx| {
            let failure = match done.await {
                Ok(Ok(_)) => None,
                Ok(Err(e)) => Some(e.message),
                Err(e) => Some(e.to_string()),
            };
            if let Some(failure) = failure {
                this.update(cx, |this, cx| this.show_toast(failure, cx)).ok();
            }
        })
        .detach();
    }

    fn open_in_menu_rows(&self) -> Option<(Vec<Option<Opener>>, String)> {
        let tab = self.tabs.get(self.active)?;
        let file = &tab.diff.as_ref()?.file;
        Some((menu_rows(tab.open_in.openers.as_ref()?, file), file.path.to_string()))
    }

    /// The toggle, or ↓ on it: the menu opens on the default opener. Pressed again it closes.
    pub fn toggle_open_in(&mut self, cx: &mut Context<Self>) {
        let Some((rows, _)) = self.open_in_menu_rows() else { return };
        let last = self.last_opener.clone();
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        if tab.open_in.menu.take().is_none() {
            let list = tab.open_in.openers.clone().unwrap_or_default();
            let current = default_opener(&list, last.as_deref()).cloned();
            tab.open_in.menu = Some(rows.iter().position(|r| *r == current).or(rows.iter().position(Option::is_some)).unwrap_or(0));
            let repo = tab.id;
            self.list_openers(repo, cx);
        }
        cx.notify();
    }

    /// ↑ / ↓ while the menu's open, past the separator.
    pub fn step_open_in(&mut self, down: bool, cx: &mut Context<Self>) {
        let Some((rows, path)) = self.open_in_menu_rows() else { return };
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let Some(at) = tab.open_in.menu else { return };
        let count = rows.len();
        let mut next = at;
        for _ in 0..count {
            next = if down { (next + 1) % count } else { (next + count - 1) % count };
            if rows[next].as_ref().is_some_and(|o| !row_disabled(o, &path)) {
                break;
            }
        }
        tab.open_in.menu = Some(next);
        cx.notify();
    }

    /// Enter on the menu's row. Whether a menu was open.
    pub fn activate_open_in(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(at) = self.tabs.get(self.active).and_then(|t| t.open_in.menu) else { return false };
        if let Some((rows, path)) = self.open_in_menu_rows()
            && let Some(Some(o)) = rows.get(at)
            && !row_disabled(o, &path)
        {
            self.open_with(o.clone(), cx);
        }
        true
    }

    pub fn close_open_in(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active) else { return false };
        let open = tab.open_in.menu.take().is_some();
        if open {
            cx.notify();
        }
        open
    }
}

/// The split button: "Open in <default>" and the toggle of its menu. Nothing until the openers are
/// listed, or when none were found.
pub fn open_in_button(tab: &Tab, last: Option<&str>, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let list = tab.open_in.openers.clone()?;
    let current = default_opener(&list, last)?.clone();
    let file = &tab.diff.as_ref()?.file;
    let path = file.path.to_string();
    let expanded = tab.open_in.menu.is_some();
    let anchor = tab.open_in.toggle.clone();
    let part = |id: &'static str| div().id(id).flex().items_center().gap(px(5.)).h_full().text_color(theme::text_secondary()).hover(|d| d.bg(theme::hover_row()).text_color(theme::text_selected()));
    let label = opener_label(&current, &path);
    let glyph = opener_icon(&current, &path);
    Some(
        div()
            .flex()
            .flex_none()
            .h(px(22.))
            .border_1()
            .border_color(theme::section_border())
            .rounded(px(4.))
            .overflow_hidden()
            .child(part("open-in-main").pl(px(6.)).pr(px(8.)).on_click(cx.listener(move |this, _, _, cx| this.open_with(current.clone(), cx))).child(icon(glyph, 14., theme::text_secondary())).child(label))
            .child(
                part("open-in-toggle")
                    .relative()
                    .px(px(4.))
                    .border_l_1()
                    .border_color(theme::section_border())
                    .when(expanded, |d| d.bg(theme::hover_row()))
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_open_in(cx);
                    }))
                    .child(icon("chevron-down", 12., theme::text_secondary()))
                    .child(canvas(|_, _, _| {}, move |b, _, _, _| anchor.set(b)).absolute().top_0().left_0().size_full()),
            )
            .into_any_element(),
    )
}

/// The menu, on top of everything, under its toggle.
pub fn open_in_menu(tab: &Tab, cx: &mut Context<GitVolt>) -> Option<impl IntoElement + use<>> {
    let at = tab.open_in.menu?;
    let list = tab.open_in.openers.clone()?;
    let file = &tab.diff.as_ref()?.file;
    let path = file.path.to_string();
    let anchor = tab.open_in.toggle.get();
    let rows = menu_rows(&list, file);
    let toggle = tab.open_in.toggle.clone();
    let mut menu = div()
        .id("open-in-menu")
        .flex()
        .flex_col()
        .min_w(px(MENU_W))
        .py(px(4.))
        .bg(theme::panel_bg1())
        .border_1()
        .border_color(theme::section_border())
        .rounded(px(6.))
        .shadow(vec![BoxShadow { color: theme::a(0x00000073), offset: point(px(0.), px(6.)), blur_radius: px(20.), spread_radius: px(0.), inset: false }])
        .on_mouse_down_out(cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            // A press on the toggle is the toggle's: it closes the menu itself.
            if !toggle.get().contains(&e.position) {
                this.close_open_in(cx);
            }
        }));
    for (i, row) in rows.into_iter().enumerate() {
        let Some(o) = row else {
            menu = menu.child(div().h(px(1.)).my(px(4.)).bg(theme::section_border()));
            continue;
        };
        let disabled = row_disabled(&o, &path);
        let active = at == i && !disabled;
        let glyph = opener_icon(&o, &path);
        let label = opener_label(&o, &path);
        menu = menu.child(
            div()
                .id(("open-in-row", i))
                .flex()
                .flex_none()
                .h(px(ROW_H))
                .items_center()
                .gap(px(8.))
                .pl(px(10.))
                .pr(px(8.))
                .when(active, |d| d.bg(theme::selected_row()).text_color(theme::text_selected()))
                .when(disabled, |d| d.text_color(theme::text_disabled()))
                .when(!disabled, |d| {
                    d.on_mouse_move(cx.listener(move |this, _, _, cx| {
                        if let Some(tab) = this.tabs.get_mut(this.active)
                            && tab.open_in.menu != Some(i)
                        {
                            tab.open_in.menu = Some(i);
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_with(o.clone(), cx)))
                })
                .child(div().flex_none().opacity(0.85).child(icon(glyph, 14., if active { theme::text_selected() } else { theme::text_normal() })))
                .child(label),
        );
    }
    let position = point(anchor.origin.x, anchor.origin.y + anchor.size.height);
    Some(deferred(anchored().position(position).snap_to_window_with_margin(px(4.)).child(menu)).with_priority(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn opener(id: &str, kind: &str) -> Opener {
        Opener { id: id.into(), name: id.to_uppercase(), kind: kind.into() }
    }

    fn file(path: &str, origin: Origin, old: Value, new: Value) -> FileChange {
        FileChange { path: path.to_string().into(), old_path: None, status: 'M', additions: None, deletions: None, old, new, origin }
    }

    fn all() -> Vec<Opener> {
        vec![opener("zed", "editor"), opener("vscode", "editor"), opener("default-app", "defaultApp"), opener("files", "fileManager"), opener("other", "chooser")]
    }

    #[test]
    fn the_default_is_the_last_used_then_vs_code_then_the_first_editor_and_never_the_default_app() {
        let list = all();
        assert_eq!(default_opener(&list, Some("zed")).unwrap().id, "zed");
        assert_eq!(default_opener(&list, None).unwrap().id, "vscode");
        assert_eq!(default_opener(&list, Some("gone")).unwrap().id, "vscode");
        assert_eq!(default_opener(&list, Some("default-app")).unwrap().id, "vscode");
        assert_eq!(default_opener(&list[2..], None).unwrap().id, "files");
        assert_eq!(default_opener(&list[..0], None), None);
    }

    #[test]
    fn labels_and_icons_say_what_each_does() {
        assert_eq!(opener_label(&opener("zed", "editor"), "a.rs"), "Open in ZED");
        assert_eq!(opener_label(&opener("files", "fileManager"), "a.rs"), "Show in FILES");
        assert_eq!(opener_label(&opener("other", "chooser"), "a.rs"), "OTHER");
        assert_eq!(opener_label(&opener("d", "defaultApp"), "a.rs"), "Open in default app");
        assert_eq!(opener_label(&opener("d", "defaultApp"), "a.HTML"), "Open in browser");
        assert_eq!(opener_icon(&opener("d", "defaultApp"), "a.html"), "globe");
        assert_eq!(opener_icon(&opener("zed", "editor"), "a.rs"), "file-code");
    }

    #[test]
    fn the_menu_has_a_separator_before_what_isnt_an_editor() {
        let plain = file("a.rs", Origin::Commit { id: "c".into() }, Value::Null, Value::Null);
        let rows = menu_rows(&all(), &plain);
        assert_eq!(rows.iter().map(|r| r.as_ref().map(|o| o.id.as_str())).collect::<Vec<_>>(), [Some("zed"), Some("vscode"), None, Some("default-app"), Some("files"), Some("other")]);
        let module = file("m", Origin::Commit { id: "c".into() }, serde_json::json!({ "kind": "submodule" }), Value::Null);
        assert!(!menu_rows(&all(), &module).iter().flatten().any(|o| o.kind == "defaultApp"), "a submodule has no default app");
        assert_eq!(menu_rows(&all()[2..], &plain)[0].as_ref().unwrap().id, "default-app", "no editors: no separator first");
    }

    #[test]
    fn a_file_the_os_would_run_has_no_default_app() {
        assert!(runs_as_program("dir/tool.SH"));
        assert!(runs_as_program("setup.exe. "));
        assert!(!runs_as_program("a.rs"));
        assert!(!runs_as_program("Makefile"));
        assert!(row_disabled(&opener("d", "defaultApp"), "x.exe"));
        assert!(!row_disabled(&opener("zed", "editor"), "x.exe"));
    }

    #[test]
    fn a_wip_file_opens_from_its_worktree_and_a_commits_as_the_version_shown() {
        let at = |c: &str| serde_json::json!({ "kind": "atCommit", "commit": c });
        let absent = serde_json::json!({ "kind": "absent" });
        let shown = |r: Request| match r {
            Request::OpenIn { worktree, opener, source, fallback, .. } => serde_json::json!({ "worktree": worktree, "opener": opener, "source": source, "fallback": fallback }),
            _ => unreachable!(),
        };
        let commit = shown(open_request(1, "/tab", &file("a", Origin::Commit { id: "c".into() }, at("p"), at("c")), "zed").unwrap());
        assert_eq!((commit["worktree"].as_str(), commit["source"]["commit"].as_str(), commit["opener"].as_str()), (Some("/tab"), Some("c"), Some("zed")));
        assert!(commit["fallback"].is_null());
        let deleted = shown(open_request(1, "/tab", &file("a", Origin::Commit { id: "c".into() }, at("p"), absent.clone()), "zed").unwrap());
        assert_eq!(deleted["source"]["commit"].as_str(), Some("p"), "a deleted file's old side");
        let wip = shown(open_request(1, "/tab", &file("a", Origin::Wip { worktree: "/w".into(), staged: true }, at("p"), at("q")), "zed").unwrap());
        assert_eq!((wip["worktree"].as_str(), wip["source"]["kind"].as_str(), wip["fallback"]["commit"].as_str()), (Some("/w"), Some("worktree"), Some("q")));
    }

    #[test]
    fn the_program_list_is_the_cores() {
        let text = include_str!("../../gitvolt-core/src/openers/default_app.rs");
        for ext in PROGRAM_EXTENSIONS {
            assert!(text.contains(&format!("\"{ext}\"")), "{ext}");
        }
    }

    #[gpui::test]
    fn the_keyboard_walks_the_menu_past_its_separator_and_enter_picks(cx: &mut TestAppContext) {
        let (window, mut cx, _runtime) = crate::app::tests::with_rows(cx, 1);
        window
            .update(&mut cx, |v, _, cx| {
                let tab = &mut v.tabs[0];
                tab.open_in.openers = Some(Rc::new(all()));
                tab.diff = Some(crate::diff::view::DiffState::new(file("a.rs", Origin::Commit { id: "c".into() }, Value::Null, Value::Null)));
                v.toggle_open_in(cx);
                assert_eq!(v.tabs[0].open_in.menu, Some(1), "on the default, VS Code");
                v.step_open_in(true, cx);
                assert_eq!(v.tabs[0].open_in.menu, Some(3), "past the separator");
                v.step_open_in(false, cx);
                v.step_open_in(false, cx);
                assert_eq!(v.tabs[0].open_in.menu, Some(0));
                assert!(v.activate_open_in(cx));
                assert_eq!(v.tabs[0].open_in.menu, None);
                assert_eq!(v.last_opener.as_deref(), Some("zed"), "remembered as the default");
                assert!(!v.activate_open_in(cx), "no menu");
            })
            .unwrap();
    }
}
