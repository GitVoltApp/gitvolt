//! `GITVOLT_BENCH=1`: the numbers `docs/decisions/gpui-rewrite.md` compares against the CEF build,
//! measured in the app, then one JSON line on stdout and quit. `GITVOLT_T0`: the launcher's clock
//! (ms since the epoch), for the time from exec to the first frame with graph rows.

use crate::app::GitVolt;
use crate::diff::image::Mode as ImageMode;
use crate::diff::model::Mode;
use crate::diff::view::{Notice, ViewKind};
use gpui::*;
use std::rc::Rc;
use std::cell::RefCell;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// When the startup's phases were reached (`GITVOLT_T0`'s clock), for the report.
pub static PHASES: std::sync::Mutex<Vec<(&str, u128)>> = std::sync::Mutex::new(Vec::new());

pub fn mark(phase: &'static str) {
    if let Ok(mut p) = PHASES.lock() {
        p.push((phase, now_ms()));
    }
}

pub fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()
}

fn cpu_ms() -> u64 {
    let Ok(s) = std::fs::read_to_string("/proc/self/stat") else { return 0 };
    let Some(end) = s.rfind(')') else { return 0 };
    let f: Vec<&str> = s[end + 2..].split(' ').collect();
    (f[11].parse::<u64>().unwrap_or(0) + f[12].parse::<u64>().unwrap_or(0)) * 10
}

fn memory_mb() -> (u64, u64, u64) {
    let get = |file: &str, key: &str| std::fs::read_to_string(file).ok().and_then(|s| s.lines().find(|l| l.starts_with(key)).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())).unwrap_or(0) / 1024;
    (get("/proc/self/status", "VmRSS:"), get("/proc/self/status", "RssAnon:"), get("/proc/self/smaps_rollup", "Pss:"))
}

fn stats(xs: &[f32]) -> String {
    let mut s = xs.to_vec();
    s.sort_by(f32::total_cmp);
    let q = |p: f32| s.get(((s.len() as f32 * p) as usize).min(s.len().saturating_sub(1))).copied().unwrap_or(0.);
    let janky = xs.iter().filter(|x| **x > 20.).count() as f32 / xs.len().max(1) as f32 * 100.;
    format!("\"frames\":{},\"median\":{:.2},\"p99\":{:.2},\"max\":{:.2},\"janky_pct\":{:.2}", xs.len(), q(0.5), q(0.99), q(1.), janky)
}

#[derive(Default)]
struct State {
    first_rows_ms: u128,
    scrolling: bool,
    y: f32,
    last: Option<Instant>,
    deltas: Vec<f32>,
    reached_end: bool,
    frames: u64,
}

/// `GITVOLT_SHOW=<row>,<file>[,hunk|split|file|wrap|history|blame|swipe|onion|difference|fit|zoom4|menu|allfiles]...`: selects a graph row and opens a file of its list,
/// with no input device: for screenshots and `GITVOLT_BENCH=diff`. Returns when the file has its
/// text and when it has its colours, in ms from opening it.
async fn show(v: &Entity<GitVolt>, spec: &str, cx: &mut AsyncWindowContext) -> (u128, u128) {
    let parts: Vec<&str> = spec.split(',').collect();
    let (row, file): (usize, usize) = (parts[0].parse().unwrap_or(0), parts.get(1).and_then(|f| f.parse().ok()).unwrap_or(0));
    // `<row>,-`: no file, and with `-,-` no selection.
    let no_file = parts.get(1) == Some(&"-");
    let wait = |cx: &AsyncWindowContext| cx.background_executor().timer(Duration::from_millis(2));
    while !v.read_with(cx, |g, _| g.tabs.first().is_some_and(|t| t.graph.is_some())) {
        wait(cx).await;
    }
    if parts[0] != "-" {
        v.update(cx, |g, cx| g.select(row, cx));
        while !v.read_with(cx, |g, _| g.tabs.first().is_some_and(|t| t.files.is_some())) {
            wait(cx).await;
        }
    }
    let opened = Instant::now();
    v.update(cx, |g, cx| {
        if !no_file {
            g.open_file(file, cx);
        }
        for part in parts.iter().skip(2) {
            g.with_diff(cx, |d| match *part {
                "hunk" => d.mode = Mode::Hunk,
                "split" => d.mode = Mode::Split,
                "file" => d.view = ViewKind::File,
                "wrap" => d.wrap = true,
                _ => {}
            });
            match *part {
                "history" => g.open_history(false, cx),
                "blame" => g.open_history(true, cx),
                _ => {}
            }
        }
    });
    while !no_file && !v.read_with(cx, |g, _| g.tabs[0].history.is_some() || g.tabs[0].diff.as_ref().is_some_and(|d| d.data.is_some() || d.notice != Some(Notice::Loading))) {
        wait(cx).await;
    }
    let text = opened.elapsed().as_millis();
    while !v.read_with(cx, |g, _| g.tabs[0].history.is_some() || g.tabs[0].diff.as_ref().is_none_or(|d| d.coloured || d.data.is_none())) {
        wait(cx).await;
    }
    let shown = (text, opened.elapsed().as_millis());
    // Their first layout places an image at 100%.
    cx.background_executor().timer(Duration::from_millis(50)).await;
    for part in parts.iter().skip(2) {
        let part = part.to_string();
        cx.update(|window, cx| v.update(cx, |g, cx| g.show_ui(&part, window, cx))).ok();
        cx.background_executor().timer(Duration::from_millis(30)).await;
    }
    if parts.contains(&"allfiles") {
        v.update(cx, |g, cx| g.toggle_all_files(cx));
    }
    if parts.contains(&"menu") {
        while !v.read_with(cx, |g, _| g.tabs[0].open_in.openers.is_some()) {
            wait(cx).await;
        }
        v.update(cx, |g, cx| g.toggle_open_in(cx));
    }
    v.update(cx, |g, cx| {
        for part in parts.iter().skip(2) {
            g.with_image(cx, |img, file| match *part {
                "swipe" => img.set_mode(ImageMode::Swipe),
                "onion" => img.set_mode(ImageMode::Onion),
                "difference" => img.set_mode(ImageMode::Difference),
                "fit" => img.apply_fit(file),
                "zoom4" => img.apply_step(15, None, false, file),
                _ => {}
            });
        }
    });
    shown
}

async fn reopen(v: &Entity<GitVolt>, file: usize, cx: &mut AsyncWindowContext) -> (u128, u128) {
    let wait = |cx: &AsyncWindowContext| cx.background_executor().timer(Duration::from_millis(2));
    let opened = Instant::now();
    v.update(cx, |g, cx| g.open_file(file, cx));
    while !v.read_with(cx, |g, _| g.tabs[0].diff.as_ref().is_some_and(|d| d.data.is_some() || d.notice != Some(Notice::Loading))) {
        wait(cx).await;
    }
    let text = opened.elapsed().as_millis();
    while !v.read_with(cx, |g, _| g.tabs[0].diff.as_ref().is_none_or(|d| d.coloured || d.data.is_none())) {
        wait(cx).await;
    }
    (text, opened.elapsed().as_millis())
}

impl GitVolt {
    /// `GITVOLT_SHOW`'s parts for what floats, for screenshots: `menu<row>` a row's context menu,
    /// `sub<i>` the open menu's i-th row's submenu, `hamburger`, `filter=<text>`, `tip=<text>`,
    /// `toast=<text>`, `palette=<query>`, `find=<query>`, `prompt`, `confirm`, `armed`, `narrow`.
    pub fn show_ui(&mut self, part: &str, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::{arm, menu, modal, popover, toast, tooltip};
        let weak = cx.weak_entity();
        if let Some(row) = part.strip_prefix("menu").and_then(|n| n.parse::<usize>().ok()) {
            let rows = self.commit_rows(row, None, &weak);
            menu::open_context(rows, point(px(700.), px(160. + row as f32 * 28.)), window, cx);
        } else if let Some(i) = part.strip_prefix("sub").and_then(|n| n.parse::<usize>().ok()) {
            menu::open_sub_for_screenshot(i, window, cx);
        } else if part == "hamburger" {
            let rows = crate::commands::hamburger_rows(&weak, self, cx);
            menu::open_below("hamburger", rows, Default::default(), window, cx);
        } else if let Some(text) = part.strip_prefix("filter=") {
            if let Some(tab) = self.tabs.get(self.active) {
                let (input, id) = (tab.side.filter.clone(), tab.id);
                input.update(cx, |i, cx| i.set_text(text, cx));
                self.relist_sidebar(id, cx);
                window.focus(&input.read(cx).focus.clone(), cx);
            }
        } else if let Some(text) = part.strip_prefix("tip=") {
            tooltip::show_at(text.to_string(), Bounds::new(point(px(600.), px(85.)), size(px(60.), px(38.))), tooltip::Placement::Below, cx);
        } else if let Some(text) = part.strip_prefix("toast=") {
            toast::show(text.to_string(), cx);
        } else if let Some(q) = part.strip_prefix("palette=") {
            self.open_palette(q, window, cx);
        } else if let Some(q) = part.strip_prefix("find=") {
            self.open_find(window, cx);
            self.set_find_text(q, cx);
        } else if part == "prompt" {
            modal::prompt(modal::PromptRequest { title: "Rename tab".into(), label: "Name".into(), initial: "fixture".into(), confirm_label: "Rename".into(), require_change: true, ..Default::default() }, |_, _, _| {}, window, cx);
        } else if part == "confirm" {
            let mut req = arm::ArmRequest::danger("Click again to delete", "Delete the branch?", "Delete");
            req.body = "It has 3 commits no other branch has.".into();
            popover::confirm(req, true, None, |_, _, _| {}, window, cx);
        } else if part == "armed" {
            arm::request(arm::ArmRequest::danger("Click again to discard 5 files", "Discard?", "Discard"), arm::Origin::control("tb-undo-caret"), |_, _, _| {}, window, cx);
        } else if part == "narrow" {
            self.sidebar_narrow = true;
            cx.notify();
        }
    }
}

/// `GITVOLT_SCRIPT=<file>`: real flows on a fixture with no input device, a line at a time, each input
/// going through the window as an event from the platform would: `click x y`, `rclick x y`, `dclick x y`,
/// `move x y`, `press x y`, `release x y`, `key ctrl-s`, `type text`, `wait ms`, `select row`, `open index`,
/// `cmd id`, `shot out.png`, `dump` (a JSON line of what's on screen), `quit`.
async fn script(v: &Entity<GitVolt>, file: &str, cx: &mut AsyncWindowContext) {
    let Ok(text) = std::fs::read_to_string(file) else {
        eprintln!("No script at {file}");
        return;
    };
    let lines: Vec<String> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_string).collect();
    for line in lines {
        let (verb, rest) = line.split_once(' ').unwrap_or((&line, ""));
        let (verb, rest) = (verb.to_string(), rest.to_string());
        let verb = verb.as_str();
        // `pick <label>`: a row of the open menu. `clickon`, `rclickon`, `dclickon`, `presson`: the anchor whose key
        // ends with the text (a chip's `chip:<row>:<name>`). `clickrow`, `rclickrow`, `dclickrow`: a graph row by its summary.
        let target = cx
            .update(|_, cx| match verb {
                "pick" | "hover" => crate::ui::menu::row_point(&rest, cx),
                // `pickv Delete|Both`.
                "pickv" => rest.split_once('|').and_then(|(row, variant)| crate::ui::menu::variant_point(row, variant, cx)),
                "clickon" | "shiftclickon" | "ctrlclickon" | "rclickon" | "dclickon" | "presson" | "dragto" | "releaseon" => crate::ui::Ui::get(cx).anchor_ending(&rest).map(|b| b.center()),
                "clickrow" | "shiftclickrow" | "ctrlclickrow" | "rclickrow" | "dclickrow" => v.read(cx).row_point(&rest),
                _ => None,
            })
            .ok()
            .flatten();
        if target.is_none() && matches!(verb, "pick" | "pickv" | "hover" | "clickon" | "shiftclickon" | "ctrlclickon" | "rclickon" | "dclickon" | "presson" | "dragto" | "releaseon" | "clickrow" | "shiftclickrow" | "ctrlclickrow" | "rclickrow" | "dclickrow") {
            println!("{{\"nothing\":{:?},\"text\":{:?}}}", verb, rest);
        }
        let xy = || {
            if let Some(t) = target {
                return t;
            }
            let mut n = rest.split_whitespace().filter_map(|t| t.parse::<f32>().ok());
            point(px(n.next().unwrap_or(0.)), px(n.next().unwrap_or(0.)))
        };
        let modifiers = match verb {
            "shiftclickon" | "shiftclickrow" => Modifiers::shift(),
            "ctrlclickon" | "ctrlclickrow" => Modifiers::control(),
            _ => Modifiers::default(),
        };
        let down = move |position, button, click_count| PlatformInput::MouseDown(MouseDownEvent { button, position, modifiers, click_count, first_mouse: false });
        let up = move |position, button, click_count| PlatformInput::MouseUp(MouseUpEvent { button, position, modifiers, click_count });
        let moved = |position| PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() });
        let events: Vec<PlatformInput> = match verb {
            "click" | "pick" | "pickv" | "clickon" | "shiftclickon" | "ctrlclickon" | "clickrow" | "shiftclickrow" | "ctrlclickrow" if verb != "click" && target.is_none() => Vec::new(),
            "rclickon" | "dclickon" | "rclickrow" | "dclickrow" | "presson" | "dragto" | "releaseon" | "hover" if target.is_none() => Vec::new(),
            "click" | "pick" | "pickv" | "clickon" | "shiftclickon" | "ctrlclickon" | "clickrow" | "shiftclickrow" | "ctrlclickrow" => vec![moved(xy()), down(xy(), MouseButton::Left, 1), up(xy(), MouseButton::Left, 1)],
            "rclick" | "rclickon" | "rclickrow" => vec![moved(xy()), down(xy(), MouseButton::Right, 1), up(xy(), MouseButton::Right, 1)],
            "dclick" | "dclickon" | "dclickrow" => vec![moved(xy()), down(xy(), MouseButton::Left, 1), up(xy(), MouseButton::Left, 1), down(xy(), MouseButton::Left, 2), up(xy(), MouseButton::Left, 2)],
            "move" | "hover" => vec![moved(xy())],
            "press" | "presson" => vec![moved(xy()), down(xy(), MouseButton::Left, 1)],
            "drag" | "dragto" => vec![PlatformInput::MouseMove(MouseMoveEvent { position: xy(), pressed_button: Some(MouseButton::Left), modifiers: Modifiers::default() })],
            "release" | "releaseon" => vec![up(xy(), MouseButton::Left, 1)],
            _ => Vec::new(),
        };
        for e in events {
            cx.update(|window, cx| {
                window.dispatch_event(e, cx);
            })
            .ok();
            cx.background_executor().timer(Duration::from_millis(16)).await;
        }
        match verb {
            "key" => {
                cx.update(|window, cx| {
                    for k in rest.split_whitespace() {
                        if let Ok(stroke) = Keystroke::parse(k.to_string().leak()) {
                            window.dispatch_keystroke(stroke, cx);
                        }
                    }
                })
                .ok();
            }
            "type" => {
                cx.update(|window, cx| {
                    for c in rest.chars() {
                        let key = if c == ' ' { "space".to_string() } else { c.to_string() };
                        window.dispatch_keystroke(Keystroke { modifiers: Modifiers::default(), key, key_char: Some(c.to_string()) }, cx);
                    }
                })
                .ok();
            }
            "wait" => cx.background_executor().timer(Duration::from_millis(rest.parse().unwrap_or(100))).await,
            "select" => v.update(cx, |g, cx| g.select(rest.parse().unwrap_or(0), cx)),
            "open" => v.update(cx, |g, cx| g.open_file(rest.parse().unwrap_or(0), cx)),
            "cmd" => {
                cx.update(|window, cx| {
                    v.update(cx, |g, cx| {
                        if !crate::commands::invoke(&rest, g, window, cx) {
                            println!("{{\"unusable\":{:?}}}", rest);
                        }
                    });
                })
                .ok();
            }
            // The window, as a file: another process looks at it, so the frame's already on screen.
            "shot" => {
                cx.background_executor().timer(Duration::from_millis(120)).await;
                let status = std::process::Command::new("crates/gitvolt-gpui/tools/winshot.sh").arg(std::process::id().to_string()).arg(&rest).status();
                if let Err(e) = status {
                    eprintln!("No screenshot: {e}");
                }
            }
            "dump" => {
                cx.update(|_, cx| println!("{}", v.read(cx).dump(cx))).ok();
            }
            // The next directory picker answers this, and no dialog opens.
            "pickfolder" => v.update(cx, |g, _| g.script_folder = Some(rest.clone())),
            "quit" => {
                cx.update(|_, cx| cx.quit()).ok();
                return;
            }
            _ => {}
        }
        cx.background_executor().timer(Duration::from_millis(40)).await;
    }
}

impl GitVolt {
    /// What a script checks: the toast, the selection, what's open, what's going on.
    /// For a script: where the graph's row with this summary is, while it's scrolled to the top.
    pub fn row_point(&self, text: &str) -> Option<Point<Pixels>> {
        let g = self.tabs.get(self.active)?.graph.as_ref()?;
        let i = g.rows.iter().position(|r| r.summary.contains(text))?;
        Some(point(px(650.), px(133. + 28. * i as f32 + 14.)))
    }

    pub fn dump(&self, cx: &App) -> String {
        let tab = self.tabs.get(self.active);
        let sections = tab.and_then(|t| t.files.as_ref()).filter(|f| f.wip.is_some()).map(|f| {
            use crate::wip::Which;
            let list = |w| f.section_files(w).iter().map(|x| x.path.to_string()).collect::<Vec<_>>();
            serde_json::json!({ "conflicted": list(Which::Conflicted), "unstaged": list(Which::Unstaged), "staged": list(Which::Staged) })
        });
        let head = tab.and_then(|t| t.graph.as_ref()).map(|g| (g.head_branch.to_string(), g.head_sha.clone(), g.rows.len()));
        let journal = tab.and_then(|t| self.ops.journal(t.id, &t.worktree));
        let merge = tab.and_then(|t| t.merge_shown()).map(|m| {
            let view = |i: usize| m.view(i).scroll;
            serde_json::json!({
                "path": m.path,
                "dirty": m.dirty(),
                "count": m.count_text(),
                "regions": m.output.as_ref().map(|o| o.spans.iter().map(|s| o.edit.buffer.text[s.from..s.to].to_string()).collect::<Vec<_>>()),
                "lines": m.output.as_ref().map(|o| o.edit.buffer.text.lines().count()),
                "edited": m.output.as_ref().map(|o| o.edited.iter().copied().collect::<Vec<_>>()),
                "scroll": (0..m.editor_count()).map(view).collect::<Vec<_>>(),
                "picked": m.picks.values().filter(|p| p.any()).count(),
            })
        });
        let rebase = tab.and_then(|t| t.rebase.as_ref()).map(|s| {
            serde_json::json!({
                "open": s.view_open,
                "running": s.running,
                "rows": s.state.rows.iter().map(|r| format!("{} {}{}", r.action.wire(), r.summary, if r.edited.is_some() { " *" } else { "" })).collect::<Vec<_>>(),
                "selected": s.state.selected.len(),
                "chips": s.state.chips.iter().map(|c| format!("{}@{}{}", c.branch, &c.at[..c.at.len().min(7)], if c.deleted { " deleted" } else { "" })).collect::<Vec<_>>(),
                "editing": s.editing.is_some(),
                "predicted": s.prediction.by_row.len(),
                "moved": s.moved,
            })
        });
        serde_json::json!({
            "rebase": rebase,
            "merge": merge,
            "toast": crate::ui::toast::message(cx).map(|m| m.to_string()),
            "armed": crate::ui::arm::is_armed(cx),
            "menu": crate::ui::menu::shown(cx),
            "selected": tab.and_then(|t| t.selected),
            "tabs": self.tabs.iter().map(|t| t.name.to_string()).collect::<Vec<_>>(),
            "openScreen": tab.and_then(|t| t.open_screen.as_ref()).map(|s| serde_json::json!({ "error": s.error, "cloneError": s.clone_error, "busy": s.busy, "url": s.url.read(cx).text(), "dest": s.dest.read(cx).text(), "repos": s.repos.as_ref().map(|r| r.iter().map(|x| x.name.clone()).collect::<Vec<_>>()) })),
            "recent": self.profile["recent"].as_array().map(|l| l.iter().map(|r| r["name"].clone()).collect::<Vec<_>>()),
            "picks": tab.map(|t| t.picks.rows.clone()),
            "compare": tab.and_then(|t| t.compare.as_ref()).map(|c| format!("{c:?}")),
            "busy": tab.and_then(|t| t.busy_reason()),
            "operation": tab.and_then(|t| t.operation()).map(|o| (o.title, o.hint, o.detail)),
            "commit": tab.and_then(|t| t.commit.as_ref()).map(|c| c.summary.to_string()),
            "open": tab.and_then(|t| t.diff.as_ref()).map(|d| d.file.path.to_string()),
            "wip": sections,
            "summary": tab.map(|t| t.wip.summary.read(cx).text().to_string()),
            "head": head.as_ref().map(|h| h.0.clone()),
            "headSha": head.as_ref().and_then(|h| h.1.clone()),
            "rows": head.as_ref().map(|h| h.2),
            "undo": journal.and_then(|j| j.undo.as_ref()).map(|u| u.label.clone()),
            "redo": journal.and_then(|j| j.redo.as_ref()).map(|u| u.label.clone()),
            "running": self.ops.ops.values().map(|o| o.label.clone()).collect::<Vec<_>>(),
            "prompts": self.ops.prompts.iter().map(|p| p.text.clone()).collect::<Vec<_>>(),
            "activity": self.ops.activity.iter().take(5).map(|e| format!("{}: {:?}", e.label, e.outcome)).collect::<Vec<_>>(),
        })
        .to_string()
    }
}

/// The same run as the CEF build's: settle 5 s, idle 20 s, scroll 16 px per frame for 8 s.
pub fn start(view: Entity<GitVolt>, window: &mut Window, cx: &mut App) {
    const STEP: f32 = 16.;
    let t0: u128 = std::env::var("GITVOLT_T0").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let state = Rc::new(RefCell::new(State::default()));
    let hooked = state.clone();
    view.update(cx, |v, _| {
        v.frame_hook = Some(Box::new(move |tab, window| {
            let mut s = hooked.borrow_mut();
            s.frames += 1;
            if s.first_rows_ms == 0 && (tab.graph.is_some() || tab.is_open_tab()) {
                s.first_rows_ms = now_ms();
            }
            if s.scrolling {
                let now = Instant::now();
                if let Some(l) = s.last {
                    s.deltas.push((now - l).as_secs_f32() * 1000.);
                }
                s.last = Some(now);
                s.y += STEP;
                if let Some(d) = tab.diff.as_mut() {
                    s.reached_end |= s.y >= d.layout.max_scroll();
                    d.scroll = s.y.min(d.layout.max_scroll());
                } else {
                    let handle = tab.list.0.borrow().base_handle.clone();
                    s.reached_end |= px(s.y) >= handle.max_offset().y;
                    handle.set_offset(point(px(0.), px(-s.y)));
                }
                window.request_animation_frame();
            }
        }));
    });
    let v = view.clone();
    window
        .spawn(cx, async move |cx| {
            let timer = |cx: &AsyncWindowContext, secs: u64| cx.background_executor().timer(Duration::from_secs(secs));
            while state.borrow().first_rows_ms == 0 {
                cx.background_executor().timer(Duration::from_millis(5)).await;
            }
            let shown = match std::env::var("GITVOLT_SHOW") {
                Ok(spec) if !spec.is_empty() => Some(show(&v, &spec, cx).await),
                _ => None,
            };
            // Another file, then the first again: what a grammar already compiled costs.
            let mut later = Vec::new();
            if std::env::var_os("GITVOLT_BENCH").is_some() && shown.is_some() {
                for file in [1, 0] {
                    timer(cx, 2).await;
                    later.push(reopen(&v, file, cx).await);
                }
            }
            if let Ok(file) = std::env::var("GITVOLT_SCRIPT") {
                script(&v, &file, cx).await;
            }
            if std::env::var_os("GITVOLT_BENCH").is_none() {
                return;
            }
            timer(cx, 5).await;
            let settled = memory_mb();
            let (c0, f0) = (cpu_ms(), state.borrow().frames);
            timer(cx, 20).await;
            let idle = (cpu_ms() - c0) as f64 / 20_000. * 100.;
            let idle_frames = state.borrow().frames - f0;
            if std::env::var_os("GITVOLT_BENCH_HOLD").is_some() {
                // Time to read the GPU's memory from outside.
                timer(cx, 10).await;
            }
            let c1 = cpu_ms();
            state.borrow_mut().scrolling = true;
            v.update(cx, |_, cx| cx.notify());
            timer(cx, 8).await;
            state.borrow_mut().scrolling = false;
            let cpu = cpu_ms() - c1;
            let after = memory_mb();
            let s = state.borrow();
            println!(
                "{{\"phases\":{:?},\"file_text_colours_ms\":{shown:?},\"later\":{later:?},\"t0_to_graph_rows_ms\":{},\"settled_rss_anon_pss_mb\":{:?},\"idle_cpu_pct\":{idle:.3},\"idle_frames\":{idle_frames},\"scroll\":{{\"step\":{STEP},\"reached_end\":{},{},\"cpu_ms\":{cpu},\"cpu_ms_per_frame\":{:.2}}},\"after_scroll_rss_anon_pss_mb\":{:?}}}",
                PHASES.lock().map(|p| p.iter().map(|(n, at)| (*n, at.saturating_sub(t0))).collect::<Vec<_>>()).unwrap_or_default(),
                s.first_rows_ms.saturating_sub(t0),
                settled,
                s.reached_end,
                stats(&s.deltas),
                cpu as f32 / s.deltas.len().max(1) as f32,
                after,
            );
            drop(s);
            cx.update(|_, cx| cx.quit()).ok();
        })
        .detach();
}
