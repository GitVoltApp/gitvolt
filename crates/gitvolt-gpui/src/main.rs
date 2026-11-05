//! GitVolt on GPUI: the same core, drawn natively instead of in Chromium. A work in progress beside
//! `gitvolt-app`, with its data kept apart from it (`gitvolt-gpui` folders) until the switch.
//!
//! `gitvolt-gpui [path]`, or `GITVOLT_OPEN=<path>`, opens a repository.

mod app;
mod avatars;
mod bench;
mod commands;
mod commit_box;
mod details;
mod feedback;
mod diff;
mod format;
mod find;
mod goto;
mod graph;
mod history;
mod icons;
mod irebase;
mod layout;
mod manage;
mod menus;
mod merge;
mod model;
mod open_in;
mod ops;
mod palette;
mod refs;
mod selection;
mod sidebar;
mod sync;
mod table;
mod theme;
mod ui;
mod wip;
mod write;

use app::GitVolt;
use gitvolt_core::api::Api;
use gitvolt_core::avatar::AvatarProvider;
use gitvolt_core::git::GitCli;
use gitvolt_core::log::CommandLog;
use gitvolt_core::openers::chooser::system_chooser;
use gitvolt_core::openers::default_app::system_default_app;
use gitvolt_core::openers::{ChildEnvHook, detect_system, launcher};
use gitvolt_core::shellenv::ShellEnv;
use gitvolt_forge::gravatar::{DEFAULT_BASE_URL, Gravatar};
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

/// This build's folder in the user's data and cache folders, apart from the CEF app's until the switch.
const APP_DIR: &str = "gitvolt-gpui";

/// "Open in…", as `gitvolt-app`'s `build_api` wires it. Stored versions' copies have a cache folder of
/// their own, apart from the app's.
fn with_openers(api: Api, shell_env: Option<Arc<ShellEnv>>) -> Api {
    let child_env: ChildEnvHook = Arc::new(|_| {});
    let api = api.with_openers(Arc::new(detect_system), launcher(child_env.clone(), shell_env));
    let api = match system_default_app(child_env.clone()) {
        Some(default_app) => api.with_default_app(default_app),
        None => api,
    };
    let api = match system_chooser(child_env) {
        Some(chooser) => api.with_chooser(chooser),
        None => api,
    };
    match gitvolt_core::paths::cache_base() {
        // `GITVOLT_NO_AVATARS`: checks on fixtures ask no one for their fictional authors' pictures.
        Some(cache) if std::env::var_os("GITVOLT_NO_AVATARS").is_none() => api.with_open_cache(cache.join(APP_DIR).join("open")).with_avatars(Arc::new(Gravatar::new(cache.join(APP_DIR).join("avatars"), DEFAULT_BASE_URL)) as Arc<dyn AvatarProvider>),
        Some(cache) => api.with_open_cache(cache.join(APP_DIR).join("open")),
        None => api,
    }
}

fn main() {
    // Git and ssh run this binary as their askpass: it asks the running app over its socket, prints
    // the answer and exits, before any of the app starts.
    if let Some(code) = gitvolt_core::askpass::run_client_from_env() {
        std::process::exit(code);
    }
    bench::mark("main");
    let path = std::env::args().nth(1).or_else(|| std::env::var("GITVOLT_OPEN").ok());
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("a tokio runtime");
    let handle = runtime.handle().clone();
    // The login shell's environment, captured once: git started meanwhile waits for it, bounded.
    let shell_env = ShellEnv::from_login_shell();
    let cli = GitCli::new(Arc::new(CommandLog::new(1000))).with_shell_env(shell_env.clone());
    let opener_env = cli.shell_env().cloned();
    let api = Api::new(cli, None).with_app_version(env!("CARGO_PKG_VERSION")).with_data_dir(gitvolt_core::paths::data_base().join(APP_DIR));
    let api = Arc::new(with_openers(api, opener_env));
    runtime.spawn(shell_env.warm());
    // The per-session askpass socket, before anything can run a network command. Without it the app
    // still runs: a credential prompt then just fails (`GIT_TERMINAL_PROMPT=0`).
    let askpass = std::env::current_exe().and_then(|exe| Ok((gitvolt_core::paths::runtime_dir()?, exe)));
    match askpass {
        Ok((dir, exe)) => {
            if let Err(e) = runtime.block_on(api.start_askpass(&dir, exe)) {
                eprintln!("The askpass socket is unavailable in {}: {e}; credential prompts will fail", dir.display());
            }
        }
        Err(e) => eprintln!("Askpass is unavailable: {e}; credential prompts will fail"),
    }
    let exit_api = api.clone();
    let font = ui_font();
    bench::mark("before_platform");
    application().with_assets(Assets).run(move |cx: &mut App| {
        bench::mark("platform_up");
        app::bind_keys(cx);
        ui::init(cx);
        ui::set_font(font.clone(), cx);
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
                v.load_layout();
                v.load_drafts();
                v.listen(cx);
                if let Some(path) = path.clone() {
                    v.open(path, cx);
                }
            });
            let closing = view.clone();
            window.on_window_should_close(cx, move |_, cx| closing.update(cx, |v, cx| v.may_close(cx)));
            if std::env::var_os("GITVOLT_BENCH").is_some() || std::env::var_os("GITVOLT_SHOW").is_some() || std::env::var_os("GITVOLT_SCRIPT").is_some() {
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
    // The socket file would otherwise stay behind in the runtime folder.
    if let Some(askpass) = exit_api.askpass() {
        askpass.close();
    }
    drop(runtime);
}
