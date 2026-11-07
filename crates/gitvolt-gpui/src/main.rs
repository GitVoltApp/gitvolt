//! GitVolt on GPUI: the same core, drawn natively instead of in Chromium. A work in progress beside
//! `gitvolt-app`, with its data kept apart from it (`gitvolt-gpui` folders) until the switch.
//!
//! `gitvolt-gpui [path]`, or `GITVOLT_OPEN=<path>`, opens a repository.
#![recursion_limit = "256"]

mod app;
mod avatars;
mod bench;
mod chrome;
mod commands;
mod commit_box;
mod details;
mod feedback;
mod glyphs;
mod diff;
mod format;
mod filecolours;
mod find;
mod forge;
mod gitflow;
mod goto;
mod graph;
mod history;
mod inline;
mod icons;
mod irebase;
mod layout;
mod lightbox;
mod lfs;
mod manage;
mod md;
mod media;
mod menus;
mod metrics;
mod merge;
mod model;
mod open;
mod open_in;
mod ops;
mod palette;
mod pdf;
mod presets;
mod profiles;
mod refs;
mod selection;
mod settings;
mod shortcuts;
mod sidebar;
mod stacks;
mod submodules;
mod sync;
mod table;
mod tabstrip;
mod tabs;
mod theme;
mod ui;
mod updates;
mod wip;
mod worker;
mod zoom;
mod write;

use app::GitVolt;
use gitvolt_core::api::Api;
use gitvolt_core::avatar::AvatarProvider;
use gitvolt_core::error::{GbError, GbErrorKind};
use gitvolt_core::forge::ForgeFuture;
use gitvolt_core::git::GitCli;
use gitvolt_core::log::CommandLog;
use gitvolt_core::openers::chooser::system_chooser;
use gitvolt_core::openers::default_app::system_default_app;
use gitvolt_core::openers::{ChildEnvHook, detect_system, launcher};
use gitvolt_core::shellenv::ShellEnv;
use gitvolt_core::updates::install::InstallKind;
use gitvolt_core::updates::release::GhRelease;
use gitvolt_core::updates::{DownloadProgress, SystemRunner, UpdateConfig, UpdateSource};
use gitvolt_forge::gravatar::{DEFAULT_BASE_URL, Gravatar};
use gpui::*;
use std::borrow::Cow;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// The window's root: the app, cached, so a frame of what turns over it (`ui::spin`) doesn't lay it out and paint it again.
struct Root {
    app: Entity<GitVolt>,
    layer: Entity<ui::spin::SpinLayer>,
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        glyphs::follow(window, cx);
        let mut full = StyleRefinement::default();
        full.size.width = Some(relative(1.).into());
        full.size.height = Some(relative(1.).into());
        div().size_full().relative().child(AnyView::from(self.app.clone()).cached(full)).child(self.layer.clone())
    }
}

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

/// The forge layer. As `gitvolt-app`'s `build_api`, in this build's own folders and keyring service: a profile's id is
/// the same across builds (`default`), so sharing a service would let this build overwrite the app's tokens.
///
/// `GITVOLT_TEST_FORGE=<http://127.0.0.1:port>` (or any scripted run, or `off`) never touches the keyring: tokens go in a
/// file in the (isolated) data folder, only the fake forge's hosts can be reached, and a link only prints.
fn with_forge(api: Api, scripted: bool) -> Api {
    use gitvolt_forge::connector::{Forge, ForgeConfig};
    use gitvolt_forge::endpoints::HostEndpoints;
    use gitvolt_forge::tokens::{FileTokenStore, KeyringBackend, SystemTokenStore};
    let data = gitvolt_core::paths::data_base().join(APP_DIR);
    let test = std::env::var("GITVOLT_TEST_FORGE").ok().or_else(|| scripted.then(|| "off".to_string()));
    if let Some(base) = test {
        let mut overrides = std::collections::HashMap::new();
        // Only ever this machine's own address.
        if base.starts_with("http://127.0.0.1:") {
            overrides.insert("gitlab.example.com".to_string(), HostEndpoints { api: format!("{base}/gitlab/api/v4"), web: format!("{base}/gitlab"), avatars: None });
            overrides.insert("github.com".to_string(), HostEndpoints { api: format!("{base}/github"), web: format!("{base}/github-web"), avatars: Some(format!("{base}/github-avatars")) });
            overrides.insert("forgejo.example.com".to_string(), HostEndpoints { api: format!("{base}/forgejo/api/v1"), web: format!("{base}/forgejo"), avatars: None });
        }
        let forge = Forge::new(ForgeConfig { overrides, only_overrides: true, avatar_dir: None });
        let forge = if base.starts_with("http://127.0.0.1:") { forge.with_image_bases("github.com", vec![format!("{base}/github-web"), format!("{base}/github-images"), format!("{base}/github-avatars")]) } else { forge };
        return api.with_forge(Arc::new(forge), Arc::new(FileTokenStore::new(data.join("forge-tokens")))).with_url_opener(Arc::new(|url: &str| {
            eprintln!("openUrl {url}");
            Ok(())
        }));
    }
    let child_env: ChildEnvHook = Arc::new(|_| {});
    let avatar_dir = gitvolt_core::paths::cache_base().map(|c| c.join(APP_DIR).join("forge-avatars"));
    let tokens = SystemTokenStore::new(Some(Box::new(KeyringBackend::new("gitvolt-gpui"))), FileTokenStore::new(data.join("forge-tokens")));
    let api = api.with_forge(Arc::new(Forge::new(ForgeConfig { overrides: Default::default(), only_overrides: false, avatar_dir })), Arc::new(tokens));
    match gitvolt_core::openers::system_url_opener(child_env) {
        Some(opener) => api.with_url_opener(opener),
        None => api,
    }
}

/// The updater's source here: this build isn't what gets installed yet, so it never asks any host, and Check for updates says so.
struct NoReleases;

fn no_releases() -> GbError {
    GbError::new(GbErrorKind::InvalidInput, "Updates aren't available in this build")
}

impl UpdateSource for NoReleases {
    fn releases(&self) -> ForgeFuture<'_, Vec<GhRelease>> {
        Box::pin(async { Err(no_releases()) })
    }

    fn fetch<'a>(&'a self, _: &'a str) -> ForgeFuture<'a, Vec<u8>> {
        Box::pin(async { Err(no_releases()) })
    }

    fn download<'a>(&'a self, _: &'a str, _: &'a Path, _: DownloadProgress, _: Arc<AtomicBool>) -> ForgeFuture<'a, u64> {
        Box::pin(async { Err(no_releases()) })
    }
}

/// The updater, for what it can do with no releases: `Request::RelaunchApp`. `restart_app` quits on its answer, from the UI thread, so the runner's
/// `quit` has nothing to do. A script or a measurement has none: a Restart there says to start it again by hand, and starts nothing.
fn with_relaunch(api: Api, scripted: bool) -> Api {
    if scripted {
        return api;
    }
    let runner = SystemRunner { hook: Arc::new(|_| {}), quit: Arc::new(|| {}) };
    let dir = gitvolt_core::paths::cache_base().unwrap_or_default().join(APP_DIR).join("updates");
    api.with_updates(UpdateConfig { source: Arc::new(NoReleases), runner: Arc::new(runner), dir, kind: InstallKind::Unpackaged, exe: std::env::current_exe().ok() })
}

type Forwarded = std::sync::mpsc::Receiver<Option<String>>;

fn claim_instance(launch: Option<&str>) -> (Option<Arc<gitvolt_core::instance::Primary>>, Option<Forwarded>) {
    use gitvolt_core::instance::{Claim, claim};
    let Ok(runtime) = gitvolt_core::paths::runtime_dir() else { return (None, None) };
    match claim(&runtime, &gitvolt_core::paths::config_base().join(APP_DIR), launch) {
        Claim::Forwarded => std::process::exit(0),
        Claim::Primary(p) => {
            let (tx, rx) = std::sync::mpsc::channel();
            let p = Arc::new(p);
            if let Err(e) = p.serve(move |path| {
                tx.send(path).ok();
            }) {
                eprintln!("Couldn't listen for later launches: {e}");
            }
            (Some(p), Some(rx))
        }
        Claim::Unguarded(_) => (None, None),
    }
}

fn main() {
    if worker::run_if_worker() {
        return;
    }
    // Git and ssh run this binary as their askpass: it asks the running app over its socket, prints
    // the answer and exits, before any of the app starts.
    if let Some(code) = gitvolt_core::askpass::run_client_from_env() {
        std::process::exit(code);
    }
    bench::mark("main");
    let path = std::env::args().nth(1).or_else(|| std::env::var("GITVOLT_OPEN").ok()).filter(|p| !p.is_empty());
    // The single instance, by this build's own config folder: a later launch hands its path over and goes. A script or a
    // measurement is a process of its own.
    let scripted = ["GITVOLT_BENCH", "GITVOLT_SHOW", "GITVOLT_SCRIPT"].iter().any(|k| std::env::var_os(k).is_some());
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("a tokio runtime");
    let handle = runtime.handle().clone();
    // Serving later launches registers its socket with the runtime, so it has to be running by then.
    let (instance, mut forwarded) = if scripted {
        (None, None)
    } else {
        let _entered = runtime.enter();
        claim_instance(path.as_deref())
    };
    // The login shell's environment, captured once: git started meanwhile waits for it, bounded.
    let shell_env = ShellEnv::from_login_shell();
    let cli = GitCli::new(Arc::new(CommandLog::new(1000))).with_shell_env(shell_env.clone());
    let opener_env = cli.shell_env().cloned();
    let api = Api::new(cli, None).with_app_version(env!("CARGO_PKG_VERSION")).with_data_dir(gitvolt_core::paths::data_base().join(APP_DIR));
    // The profile and the settings (recent repositories, presets, file colours), in a folder of this build's own, never the CEF app's.
    let store = gitvolt_core::settings::SettingsStore::open(gitvolt_core::paths::config_base().join(APP_DIR));
    let exit_store = store.clone();
    let saved_store = store.clone();
    let saved = store.state();
    let api = with_relaunch(with_forge(api.with_store(store), scripted), scripted);
    // This build's log files, in its own cache folder: Settings' Debug logging and Logs folder work on them.
    let logging = gitvolt_core::paths::cache_base().and_then(|c| gitvolt_core::logging::init(&c.join(APP_DIR).join("logs"), false, false).ok());
    let (api, _log_guard) = match logging {
        Some(l) => (api.with_log_handle(l.handle), Some(l.guard)),
        None => (api, None),
    };
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
    zoom::set_percent(zoom::parse_zoom(layout::saved_zoom()).unwrap_or(100));
    Application::with_platform(Rc::new(zoom::Zoomed(gpui_platform::current_platform(false)))).with_assets(Assets).run(move |cx: &mut App| {
        bench::mark("platform_up");
        app::bind_keys(cx);
        ui::init(cx);
        ui::set_font(font.clone(), cx);
        // The saved theme before the first paint: a light theme's user never sees the dark one.
        if let Ok(v) = serde_json::to_value(&saved.settings) {
            theme::apply(theme::theme_def(v["theme"].as_str().unwrap_or(theme::DEFAULT_THEME_ID), &v["customThemes"]));
        }
        // The bundled editor font's Latin face (the web UI's, assets/generate.mjs); what it lacks
        // falls back to the system's.
        if let Err(e) = cx.text_system().add_fonts(vec![Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Latin-400.ttf"))]) {
            eprintln!("Couldn't load the editor font: {e}");
        }
        // The window as it was left; the first time, the web UI's first size.
        let start = saved_store.state().settings;
        let chrome = chrome::WindowChrome::at_start(start.system_title_bar, start.compact_title_bar, std::env::var("WAYLAND_DISPLAY").ok().as_deref(), std::env::var("XDG_SESSION_TYPE").ok().as_deref());
        let saved_window = saved_store.window();
        let start_size = saved_window.as_ref().map_or(size(px(1600.), px(900.)), |w| size(px(w.width as f32), px(w.height as f32)));
        let bounds = Bounds::centered(None, start_size, cx);
        let options = WindowOptions {
            window_bounds: Some(if saved_window.as_ref().is_some_and(|w| w.maximized) { WindowBounds::Maximized(bounds) } else { WindowBounds::Windowed(bounds) }),
            titlebar: None,
            // `tauri.conf.json`'s.
            window_min_size: Some(size(px(900.), px(500.))),
            window_decorations: Some(if chrome.custom { WindowDecorations::Client } else { WindowDecorations::Server }),
            app_id: Some("gitvolt-gpui".into()),
            // Measuring: a window behind another isn't throttled to GPUI's 24 Hz.
            inactive_frame_interval: std::env::var_os("GITVOLT_BENCH").map(|_| std::time::Duration::ZERO),
            ..Default::default()
        };
        let mut forwarded = forwarded.take();
        let result = cx.open_window(options, |window, cx| {
            bench::mark("window_open");
            let view = cx.new(|cx| GitVolt::new(api.clone(), handle.clone(), font.clone(), cx));
            view.update(cx, |v, cx| {
                window.focus(&v.focus, cx);
                v.load_layout();
                v.load_drafts();
                v.listen(cx);
                cx.observe_window_activation(window, |this, window, cx| {
                    this.forge_on_focus(window.is_window_active(), cx);
                    if window.is_window_active() {
                        this.lfs_on_focus(cx);
                        this.fetch_on_focus(cx);
                    }
                })
                .detach();
                cx.observe_window_visibility(window, |this, _, window, cx| {
                    this.window_hidden = !window.is_visible();
                    if !this.window_hidden {
                        this.fetch_on_focus(cx);
                    }
                })
                .detach();
                let geometry = saved_store.clone();
                cx.observe_window_bounds(window, move |_, window, _| {
                    let maximized = window.is_maximized();
                    let b = window.window_bounds().get_bounds();
                    let was = geometry.window();
                    let size_now = if maximized { was.as_ref().map_or((b.size.width.as_f32() as f64, b.size.height.as_f32() as f64), |w| (w.width, w.height)) } else { (b.size.width.as_f32() as f64, b.size.height.as_f32() as f64) };
                    geometry.save_window(gitvolt_core::settings::WindowGeometry { x: None, y: None, width: size_now.0, height: size_now.1, maximized, monitor: None });
                })
                .detach();
                // A later launch's path: a tab, and the window comes forward.
                if let Some(rx) = forwarded.take() {
                    v.listen_for_launches(rx, cx);
                }
                v.start_path = path.clone();
                v.chrome = chrome;
            });
            let closing = view.clone();
            window.on_window_should_close(cx, move |_, cx| closing.update(cx, |v, cx| v.may_close(cx)));
            if std::env::var_os("GITVOLT_BENCH").is_some() || std::env::var_os("GITVOLT_SHOW").is_some() || std::env::var_os("GITVOLT_SCRIPT").is_some() {
                bench::start(view.clone(), window, cx);
            }
            bench::start_debug_pipe(view.clone(), window, cx);
            let layer = ui::spin::SpinLayer::new(cx);
            cx.new(|_| Root { app: view, layer })
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
    if let Err(e) = exit_store.flush_now() {
        eprintln!("Couldn't save the settings: {}", e.message);
    }
    // The socket file would otherwise stay behind in the runtime folder.
    if let Some(askpass) = exit_api.askpass() {
        askpass.close();
    }
    if let Some(i) = instance {
        i.close();
    }
    drop(runtime);
}
