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
            arm::request(arm::ArmRequest::danger("Click again to discard 5 files", "Discard?", "Discard"), arm::Origin::control("tb-search"), |_, _, _| {}, window, cx);
        } else if part == "narrow" {
            self.sidebar_narrow = true;
        }
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
            if s.first_rows_ms == 0 && tab.graph.is_some() {
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
                Ok(spec) => Some(show(&v, &spec, cx).await),
                Err(_) => None,
            };
            // Another file, then the first again: what a grammar already compiled costs.
            let mut later = Vec::new();
            if std::env::var_os("GITVOLT_BENCH").is_some() && shown.is_some() {
                for file in [1, 0] {
                    timer(cx, 2).await;
                    later.push(reopen(&v, file, cx).await);
                }
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
