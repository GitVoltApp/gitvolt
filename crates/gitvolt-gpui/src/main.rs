//! GitVolt on GPUI: the same core, drawn natively instead of in Chromium. A work in progress beside
//! `gitvolt-app`: it reads (graph, sidebar, commit details, diffs) and doesn't write yet.
//!
//! `gitvolt-gpui [path]`, or `GITVOLT_OPEN=<path>`, opens a repository.

mod app;
mod bench;
mod details;
mod diff;
mod field;
mod format;
mod graph;
mod history;
mod icons;
mod model;
mod theme;

use app::GitVolt;
use gitvolt_core::api::Api;
use gitvolt_core::git::GitCli;
use gitvolt_core::log::CommandLog;
use gpui::*;
use gpui_platform::application;
use std::borrow::Cow;
use std::sync::Arc;

struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(icons::load(path).map(Cow::Borrowed))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

/// What `system-ui` is here (the web UI's `--font-ui`): fontconfig says, as Chromium asks it.
fn ui_font() -> SharedString {
    let asked = std::process::Command::new("fc-match").args(["-f", "%{family[0]}", "system-ui"]).output();
    match asked {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => String::from_utf8_lossy(&o.stdout).into_owned().into(),
        _ => ".SystemUIFont".into(),
    }
}

fn main() {
    bench::mark("main");
    let path = std::env::args().nth(1).or_else(|| std::env::var("GITVOLT_OPEN").ok());
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("a tokio runtime");
    let handle = runtime.handle().clone();
    // No `with_data_dir` yet: the core keeps its journal in a temp dir, apart from the app's, until
    // this build writes.
    let api = Arc::new(Api::new(GitCli::new(Arc::new(CommandLog::new(1000))), None).with_app_version(env!("CARGO_PKG_VERSION")));
    let font = ui_font();
    bench::mark("before_platform");
    application().with_assets(Assets).run(move |cx: &mut App| {
        bench::mark("platform_up");
        app::bind_keys(cx);
        // The bundled editor font's Latin face (the web UI's, assets/generate.mjs); what it lacks
        // falls back to the system's.
        if let Err(e) = cx.text_system().add_fonts(vec![Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Latin-400.ttf"))]) {
            eprintln!("Couldn't load the editor font: {e}");
        }
        let bounds = Bounds::centered(None, size(px(1600.), px(900.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            app_id: Some("gitvolt-gpui".into()),
            // Measuring: a window behind another isn't throttled to GPUI's 24 Hz.
            inactive_frame_interval: std::env::var_os("GITVOLT_BENCH").map(|_| std::time::Duration::ZERO),
            ..Default::default()
        };
        let result = cx.open_window(options, |window, cx| {
            bench::mark("window_open");
            let view = cx.new(|cx| GitVolt::new(api.clone(), handle.clone(), font.clone(), cx));
            view.update(cx, |v, cx| {
                window.focus(&v.focus, cx);
                v.listen(cx);
                if let Some(path) = path.clone() {
                    v.open(path, cx);
                }
            });
            if std::env::var_os("GITVOLT_BENCH").is_some() || std::env::var_os("GITVOLT_SHOW").is_some() {
                bench::start(view.clone(), window, cx);
            }
            view
        });
        if let Err(e) = result {
            eprintln!("Couldn't open a window: {e}");
            cx.quit();
        }
        cx.activate(true);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
    drop(runtime);
}
