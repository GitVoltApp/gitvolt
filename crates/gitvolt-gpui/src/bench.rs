//! `GITVOLT_BENCH=1`: the numbers `docs/decisions/gpui-rewrite.md` compares against the CEF build,
//! measured in the app, then one JSON line on stdout and quit. `GITVOLT_T0`: the launcher's clock
//! (ms since the epoch), for the time from exec to the first frame with graph rows.

use crate::app::GitVolt;
use gpui::*;
use std::rc::Rc;
use std::cell::RefCell;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
                let handle = &tab.list.0.borrow().base_handle;
                s.reached_end |= px(s.y) >= handle.max_offset().y;
                handle.set_offset(point(px(0.), px(-s.y)));
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
                "{{\"t0_to_graph_rows_ms\":{},\"settled_rss_anon_pss_mb\":{:?},\"idle_cpu_pct\":{idle:.3},\"idle_frames\":{idle_frames},\"scroll\":{{\"step\":{STEP},\"reached_end\":{},{},\"cpu_ms\":{cpu},\"cpu_ms_per_frame\":{:.2}}},\"after_scroll_rss_anon_pss_mb\":{:?}}}",
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
