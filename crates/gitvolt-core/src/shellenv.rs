//! Login-shell environment capture. Apps started from the desktop menu miss what the
//! user's shell setup adds (`SSH_AUTH_SOCK`, `PATH` entries, ...). We run
//! `$SHELL -l -i -c '<script>'` once, in the background, with a 5 s timeout, and use the result
//! for every git command and every editor we launch. Output before a marker line is shell noise
//! (motd, rc echo) and is ignored.
//!
//! The app starts the shell first thing in `main` ([`ShellEnv::start`]), so a slow shell setup
//! (often half a second or more) runs while the webview starts, not after it, when the first git
//! command would wait on it; the first [`ShellEnv::get`] collects it.
//!
//! The capture shell is a child like any other (`openers::launch_command`): GitVolt's own
//! variables (`openers::PRIVATE_ENV`) are removed and the app's [`ChildEnvHook`] runs on it, so
//! the session's values of what the app changed for itself (`GDK_BACKEND`, ...) are what the
//! shell sees and what the capture returns. `PRIVATE_ENV` is also dropped from the result, in
//! case the user's shell setup exports one itself.
//!
//! macOS: an app started from Finder or the Dock gets launchd's bare `PATH` (no Homebrew), which
//! is exactly what the capture fixes; the shell is `$SHELL` (zsh by default), else the account's
//! own login shell.
//!
//! Windows has no login shell to ask: an app started from the Start menu already has the user's
//! environment, so nothing is captured and git inherits the process environment.

use crate::openers::{ChildEnvHook, PRIVATE_ENV};
use crate::platform::process::{own_session, Group};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::sync::OnceCell;

pub const ENV_MARKER: &str = "__GITVOLT_ENV_START__";
pub const CAPTURE_TIMEOUT: Duration = Duration::from_secs(5);
/// After the shell exits, how long its output may take to finish arriving.
const OUTPUT_GRACE: Duration = Duration::from_millis(300);
const SCRIPT: &str = "printf '%s\\n' __GITVOLT_ENV_START__; env -0";
const SKIP: [&[u8]; 4] = [b"PWD", b"OLDPWD", b"SHLVL", b"_"];

/// A captured environment, shared by every git command and launch that uses it.
pub type EnvVars = Arc<Vec<(OsString, OsString)>>;

pub struct ShellEnv {
    shell: Option<PathBuf>,
    timeout: Duration,
    hook: Option<ChildEnvHook>,
    /// The shell [`ShellEnv::start`] started, for the first [`ShellEnv::get`] to collect.
    started: Mutex<Option<Started>>,
    cell: OnceCell<Option<EnvVars>>,
}

/// A capture shell already started (or that failed to start), and when.
struct Started {
    child: std::io::Result<std::process::Child>,
    at: Instant,
}

impl ShellEnv {
    /// `$SHELL`, or no capture at all if it isn't set.
    pub fn from_login_shell() -> Arc<Self> {
        Self::login_shell(None)
    }

    /// [`Self::from_login_shell`], with the shell started through the app's child-env hook
    /// (`desktop::restore_child_env`), so it sees the session's environment, not CEF's.
    pub fn from_login_shell_with_hook(hook: ChildEnvHook) -> Arc<Self> {
        Self::login_shell(Some(hook))
    }

    fn login_shell(hook: Option<ChildEnvHook>) -> Arc<Self> {
        // Windows: no capture (see the module docs), even with a `SHELL` from Git Bash or MSYS.
        let shell = if cfg!(unix) { std::env::var_os("SHELL").map(PathBuf::from) } else { None };
        #[cfg(target_os = "macos")]
        let shell = shell.or_else(account_shell);
        Arc::new(Self { shell, timeout: CAPTURE_TIMEOUT, hook, started: Mutex::new(None), cell: OnceCell::new() })
    }

    pub fn with_shell(shell: PathBuf, timeout: Duration) -> Arc<Self> {
        Arc::new(Self { shell: Some(shell), timeout, hook: None, started: Mutex::new(None), cell: OnceCell::new() })
    }

    /// [`Self::with_shell`], started through `hook` (see [`Self::from_login_shell_with_hook`]).
    pub fn with_shell_and_hook(shell: PathBuf, timeout: Duration, hook: ChildEnvHook) -> Arc<Self> {
        Arc::new(Self { shell: Some(shell), timeout, hook: Some(hook), started: Mutex::new(None), cell: OnceCell::new() })
    }

    /// An already-known environment (tests).
    pub fn fixed(vars: Vec<(OsString, OsString)>) -> Arc<Self> {
        Arc::new(Self { shell: None, timeout: CAPTURE_TIMEOUT, hook: None, started: Mutex::new(None), cell: OnceCell::new_with(Some(Some(Arc::new(vars)))) })
    }

    /// Starts the capture shell now, with `env` as its whole environment (before the hook), for
    /// the first [`Self::get`] to collect; the timeout counts from now. No thread and no async
    /// runtime: for the top of `main`, before anything changes the process environment, while
    /// nothing else can read it. Nothing without a shell, or once started or captured.
    pub fn start(&self, env: Vec<(OsString, OsString)>) {
        let Some(shell) = self.shell.as_deref() else { return };
        let mut started = self.started.lock().unwrap_or_else(|e| e.into_inner());
        if started.is_some() || self.cell.initialized() {
            return;
        }
        let mut cmd = command(shell, Some(env), self.hook.as_ref());
        *started = Some(Started { child: spawn(&mut cmd), at: Instant::now() });
    }

    /// The captured environment, or `None` (use the app's own). The first caller collects the
    /// shell [`Self::start`] started, else runs the capture; callers that arrive meanwhile wait
    /// for it (bounded by the timeout).
    pub async fn get(&self) -> Option<Arc<Vec<(OsString, OsString)>>> {
        self.cell
            .get_or_init(|| async {
                let shell = self.shell.as_deref()?;
                let early = self.started.lock().unwrap_or_else(|e| e.into_inner()).take();
                let Started { child, at } = early.unwrap_or_else(|| {
                    let mut cmd = command(shell, None, self.hook.as_ref());
                    Started { child: spawn(&mut cmd), at: Instant::now() }
                });
                let vars = match child {
                    Ok(child) => collect(child, at, self.timeout).await,
                    Err(e) => {
                        tracing::warn!("cannot run {}: {e}", shell.display());
                        None
                    }
                };
                if vars.is_some() {
                    tracing::info!("login shell environment captured {} ms after the shell started", at.elapsed().as_millis());
                } else {
                    tracing::warn!("login shell environment not captured; git runs with GitVolt's own environment");
                }
                vars.map(Arc::new)
            })
            .await
            .clone()
    }

    /// The captured environment if the capture has finished, without waiting or starting it:
    /// for synchronous callers (the app's editor launcher) after something awaited [`Self::get`].
    pub fn captured(&self) -> Option<EnvVars> {
        self.cell.get().cloned().flatten()
    }

    /// Starts the capture in the background at startup.
    pub async fn warm(self: Arc<Self>) {
        let _ = self.get().await;
    }
}

/// The account's login shell (its directory record), for an app launched without `$SHELL`.
#[cfg(target_os = "macos")]
fn account_shell() -> Option<PathBuf> {
    nix::unistd::User::from_uid(nix::unistd::getuid()).ok().flatten().map(|u| u.shell).filter(|s| s.is_absolute())
}

/// The capture shell's command: `env` (else the app's own environment) without GitVolt's own
/// variables, then the hook, in a session of its own.
fn command(shell: &Path, env: Option<Vec<(OsString, OsString)>>, hook: Option<&ChildEnvHook>) -> std::process::Command {
    let mut cmd = std::process::Command::new(shell);
    cmd.args(["-l", "-i", "-c", SCRIPT]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    if let Some(env) = env {
        cmd.env_clear().envs(env);
    }
    for var in PRIVATE_ENV {
        cmd.env_remove(var);
    }
    if let Some(hook) = hook {
        hook(&mut cmd);
    }
    // A new session, not just a process group: the shell has no controlling terminal. In a
    // background group of a terminal's session (a dev run), `-i` would make it touch the tty and
    // stop on SIGTTIN/SIGTTOU until the timeout. As a session leader its pgid is its pid, so
    // killing the group reaches everything it started.
    own_session(&mut cmd);
    cmd
}

/// Kills the shell's group when dropped armed: on a timeout, or a capture abandoned midway (its
/// future dropped), nothing it started keeps running.
struct KillOnDrop(Option<Group>);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(group) = &self.0 {
            group.kill();
        }
    }
}

/// Reads the shell started at `at` until it exits (killed once `timeout` has passed since `at`),
/// then parses what it printed.
async fn collect(mut child: std::process::Child, at: Instant, timeout: Duration) -> Option<Vec<(OsString, OsString)>> {
    let mut guard = KillOnDrop(Some(Group::of_std(&child)));
    let stdout = child.stdout.take().expect("stdout is piped");
    let buf = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut reader = match tokio::process::ChildStdout::from_std(stdout) {
        Ok(mut stdout) => {
            let buf = buf.clone();
            tokio::spawn(async move {
                let mut chunk = [0u8; 8192];
                while let Ok(n @ 1..) = stdout.read(&mut chunk).await {
                    buf.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(&chunk[..n]);
                }
            })
        }
        Err(e) => {
            tracing::warn!("cannot read the login shell's output: {e}");
            tokio::spawn(async {})
        }
    };
    // Wait for the shell itself (off the async workers: a std child), then give its output a
    // short grace: a background job the rc files started may keep the pipe open long after the
    // shell (and its `env -0`) are done. After a timeout the kill ends the wait, which reaps it.
    let mut wait = tokio::task::spawn_blocking(move || child.wait());
    let waited = tokio::time::timeout(timeout.saturating_sub(at.elapsed()), &mut wait).await;
    if matches!(waited, Ok(Ok(Ok(_)))) {
        guard.0 = None;
        let _ = tokio::time::timeout(OUTPUT_GRACE, &mut reader).await;
    }
    drop(guard);
    // Dropping a JoinHandle would leave the reader running; stop it (a no-op once it's done).
    reader.abort();
    let out = std::mem::take(&mut *buf.lock().unwrap_or_else(|e| e.into_inner()));
    match waited {
        Ok(Ok(Ok(_))) => parse_env_output(&out).map(|vars| vars.into_iter().filter(|(k, _)| !PRIVATE_ENV.iter().any(|p| k == p)).collect()),
        Ok(Ok(Err(e))) => {
            tracing::warn!("login shell failed: {e}");
            None
        }
        Ok(Err(e)) => {
            tracing::warn!("login shell wait failed: {e}");
            None
        }
        Err(_) => {
            tracing::warn!("login shell environment capture timed out after {timeout:?}");
            None
        }
    }
}

/// `cmd.spawn()`, retried briefly on `ETXTBSY`: a shell binary that was just written (an update
/// in progress, or a test's fake shell) stays "busy" while any other thread's fork still holds
/// the writer's descriptor, until that child execs.
fn spawn(cmd: &mut std::process::Command) -> std::io::Result<std::process::Child> {
    let mut attempts = 0;
    loop {
        match cmd.spawn() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempts < 5 => {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            r => return r,
        }
    }
}

/// `env -0` output after the marker line: NUL-separated `KEY=VALUE` entries.
pub fn parse_env_output(out: &[u8]) -> Option<Vec<(OsString, OsString)>> {
    let marker = format!("{ENV_MARKER}\n");
    let start = out.windows(marker.len()).position(|w| w == marker.as_bytes())? + marker.len();
    let vars: Vec<(OsString, OsString)> = out[start..]
        .split(|b| *b == 0)
        .filter_map(|entry| {
            let eq = entry.iter().position(|b| *b == b'=')?;
            if eq == 0 || SKIP.contains(&&entry[..eq]) {
                return None;
            }
            Some((crate::platform::osstr::from_vec(entry[..eq].to_vec()), crate::platform::osstr::from_vec(entry[eq + 1..].to_vec())))
        })
        .collect();
    (!vars.is_empty()).then_some(vars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)] // a #!/bin/sh fake shell
    fn fake_shell(dir: &Path, body: &str) -> PathBuf {
        let p = dir.join("fake-shell");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        crate::platform::fs::set_mode(&p, 0o755).unwrap();
        p
    }

    /// Windows: the login-shell environment is the process's own; nothing is captured, even
    /// with a `SHELL` set (Git Bash, MSYS).
    #[cfg(windows)]
    #[tokio::test]
    async fn windows_captures_no_login_shell() {
        let env = ShellEnv::from_login_shell();
        assert!(env.get().await.is_none());
        assert!(env.captured().is_none());
    }

    fn get<'a>(vars: &'a [(OsString, OsString)], k: &str) -> Option<&'a OsString> {
        vars.iter().find(|(key, _)| key == k).map(|(_, v)| v)
    }

    #[test]
    fn parses_after_the_marker_and_drops_shell_noise() {
        let out = b"motd: welcome\nsome rc output\n__GITVOLT_ENV_START__\nA=1\0B=two\nlines\0PWD=/x\0SHLVL=3\0=bad\0";
        let vars = parse_env_output(out).unwrap();
        assert_eq!(get(&vars, "A").unwrap(), "1");
        assert_eq!(get(&vars, "B").unwrap(), "two\nlines");
        assert!(get(&vars, "PWD").is_none() && get(&vars, "SHLVL").is_none());
        assert_eq!(vars.len(), 2);
        assert!(parse_env_output(b"no marker at all").is_none());
    }

    #[cfg(unix)] // a #!/bin/sh fake shell
    #[tokio::test]
    async fn captures_from_a_shell() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), "echo noise\nprintf '%s\\n' __GITVOLT_ENV_START__\nprintf 'SSH_AUTH_SOCK=/run/agent.sock\\0GB_X=y\\0'");
        let env = ShellEnv::with_shell(shell, Duration::from_secs(2));
        assert!(env.captured().is_none(), "nothing yet: the capture hasn't run");
        let vars = env.get().await.expect("captured");
        assert_eq!(get(&vars, "SSH_AUTH_SOCK").unwrap(), "/run/agent.sock");
        // Captured once: a second get returns the same Arc, and so does the non-waiting read.
        assert!(Arc::ptr_eq(&vars, &env.get().await.unwrap()));
        assert!(Arc::ptr_eq(&vars, &env.captured().unwrap()));
        assert!(ShellEnv::with_shell("/nonexistent/shell".into(), Duration::from_secs(1)).captured().is_none());
    }

    #[cfg(unix)] // a #!/bin/sh fake shell
    #[tokio::test]
    async fn a_hanging_shell_times_out_to_none() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), "sleep 5");
        let started = std::time::Instant::now();
        assert!(ShellEnv::with_shell(shell, Duration::from_millis(200)).get().await.is_none());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn a_missing_shell_is_none() {
        assert!(ShellEnv::with_shell("/nonexistent/shell".into(), Duration::from_secs(1)).get().await.is_none());
    }

    /// The capture shell inherits the app's own environment, so it starts like any other child
    /// (`openers::launch_command`): the app's hook runs on it (the session's `GDK_BACKEND` and
    /// `IBUS_ENABLE_SYNC_MODE` back, not CEF's), and what it prints is what git and editors get.
    #[cfg(unix)] // a #!/bin/sh fake shell
    #[tokio::test]
    async fn the_capture_shell_starts_through_the_child_env_hook() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), "printf '%s\\n' __GITVOLT_ENV_START__\nprintf 'GB_HOOKED=%s\\0GB_GONE=%s\\0' \"$GB_HOOKED\" \"${GB_GONE-unset}\"");
        let hook: ChildEnvHook = Arc::new(|cmd: &mut std::process::Command| {
            cmd.env("GB_HOOKED", "yes").env_remove("GB_GONE");
        });
        let vars = ShellEnv::with_shell_and_hook(shell, Duration::from_secs(2), hook).get().await.expect("captured");
        assert_eq!(get(&vars, "GB_HOOKED").unwrap(), "yes");
        assert_eq!(get(&vars, "GB_GONE").unwrap(), "unset");
    }

    /// GitVolt's own variables (`openers::PRIVATE_ENV`) never come back through the captured
    /// env, even if the user's shell setup exports them itself: the captured env is what editors
    /// start with, after `launch_command` has already stripped them.
    #[cfg(unix)] // a #!/bin/sh fake shell
    #[tokio::test]
    async fn private_app_env_is_dropped_from_the_capture() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), "printf '%s\\n' __GITVOLT_ENV_START__\nprintf 'CHROME_DEVEL_SANDBOX=/leak\\0GITVOLT_OPEN=/leak\\0GB_KEEP=1\\0'");
        let vars = ShellEnv::with_shell(shell, Duration::from_secs(2)).get().await.expect("captured");
        assert!(get(&vars, "CHROME_DEVEL_SANDBOX").is_none() && get(&vars, "GITVOLT_OPEN").is_none());
        assert_eq!(get(&vars, "GB_KEEP").unwrap(), "1");
    }

    /// The capture shell has no controlling terminal. Otherwise, in a dev run from a terminal, an
    /// interactive shell in a background process group gets SIGTTIN (or SIGTTOU) when it touches
    /// the tty and stops, and every git command waits the full timeout. With a terminal it leads
    /// a new session (its session id is its pid, `/proc/<pid>/stat` field 6); without one (CI,
    /// the Dock) its own group is enough. Either way it can't open `/dev/tty`.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_capture_shell_has_no_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(
            dir.path(),
            "printf '%s\\n' __GITVOLT_ENV_START__\nif ( : </dev/tty ) 2>/dev/null; then t=yes; else t=no; fi\nprintf 'GB_PID=%s\\0GB_SID=%s\\0GB_TTY=%s\\0' \"$$\" \"$(awk '{print $6}' /proc/$$/stat 2>/dev/null)\" \"$t\"",
        );
        let vars = ShellEnv::with_shell(shell, Duration::from_secs(2)).get().await.expect("captured");
        assert_eq!(get(&vars, "GB_TTY").and_then(|v| v.to_str()), Some("no"));
        #[cfg(target_os = "linux")]
        if crate::platform::process::detaches_by_setsid() {
            assert_eq!(get(&vars, "GB_SID").unwrap(), get(&vars, "GB_PID").unwrap());
        }
    }

    /// A real shell and the system's own `env -0` (BSD's on macOS): the capture parses.
    #[cfg(unix)]
    #[tokio::test]
    async fn captures_from_the_system_shell() {
        let vars = ShellEnv::with_shell("/bin/sh".into(), Duration::from_secs(5)).get().await.expect("captured");
        assert!(get(&vars, "PATH").is_some_and(|p| !p.is_empty()), "no PATH in the captured environment");
    }

    /// The account's own shell stands in for a missing `$SHELL` (an app started from the Dock).
    #[cfg(target_os = "macos")]
    #[test]
    fn the_account_shell_is_known() {
        assert!(account_shell().is_some_and(|s| s.is_file()));
    }

    /// A background job the rc files started that keeps the shell's stdout open doesn't stall
    /// the capture: once the shell exits, its output is read with a short grace.
    #[cfg(unix)] // a #!/bin/sh fake shell
    #[tokio::test]
    async fn a_background_job_holding_stdout_does_not_stall_the_capture() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), "sleep 3 &\nprintf '%s\\n' __GITVOLT_ENV_START__\nprintf 'GB_X=y\\0'");
        let started = std::time::Instant::now();
        let vars = ShellEnv::with_shell(shell, Duration::from_secs(2)).get().await.expect("captured");
        assert_eq!(get(&vars, "GB_X").unwrap(), "y");
        assert!(started.elapsed() < Duration::from_millis(1500), "{:?}", started.elapsed());
    }

    /// A shell started early (`start`, the top of `main`) runs with the environment it was given
    /// and is what the first `get` collects: one shell, not a second one.
    #[cfg(unix)] // a #!/bin/sh fake shell
    #[tokio::test]
    async fn a_shell_started_early_is_collected_by_the_first_get() {
        let dir = tempfile::tempdir().unwrap();
        let count = dir.path().join("runs");
        let shell = fake_shell(
            dir.path(),
            &format!(
                "echo x >> '{}'\nprintf '%s\\n' __GITVOLT_ENV_START__\nprintf 'GB_GIVEN=%s\\0GB_PRIVATE=%s\\0' \"$GB_GIVEN\" \"${{GITVOLT_OPEN-unset}}\"",
                crate::platform::fs::to_git_path(&count)
            ),
        );
        let env = ShellEnv::with_shell(shell, Duration::from_secs(2));
        let path = std::env::var_os("PATH").unwrap_or_default();
        env.start(vec![("PATH".into(), path), ("GB_GIVEN".into(), "early".into()), ("GITVOLT_OPEN".into(), "/leak".into())]);
        env.start(Vec::new()); // once only
        let vars = env.get().await.expect("captured");
        assert_eq!(get(&vars, "GB_GIVEN").unwrap(), "early");
        assert_eq!(get(&vars, "GB_PRIVATE").unwrap(), "unset", "GitVolt's own variables never reach it");
        env.start(Vec::new()); // captured: nothing more starts
        assert!(Arc::ptr_eq(&vars, &env.get().await.unwrap()));
        assert_eq!(std::fs::read_to_string(&count).unwrap().lines().count(), 1);
    }

    /// An early shell's timeout counts from its start, and one that overruns it is killed.
    #[cfg(unix)] // a #!/bin/sh fake shell
    #[tokio::test]
    async fn an_early_shell_times_out_from_its_start() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), "sleep 5");
        let env = ShellEnv::with_shell(shell, Duration::from_millis(300));
        env.start(vec![("PATH".into(), std::env::var_os("PATH").unwrap_or_default())]);
        tokio::time::sleep(Duration::from_millis(400)).await;
        let started = std::time::Instant::now();
        assert!(env.get().await.is_none());
        assert!(started.elapsed() < Duration::from_millis(250), "{:?}", started.elapsed());
    }

    /// Concurrent callers share the one capture.
    #[cfg(unix)] // a #!/bin/sh fake shell
    #[tokio::test]
    async fn concurrent_callers_wait_for_the_one_capture() {
        let dir = tempfile::tempdir().unwrap();
        let count = dir.path().join("runs");
        let shell = fake_shell(
            dir.path(),
            &format!("echo x >> '{}'\nsleep 0.2\nprintf '%s\\n' __GITVOLT_ENV_START__\nprintf 'GB_X=y\\0'", crate::platform::fs::to_git_path(&count)),
        );
        let env = ShellEnv::with_shell(shell, Duration::from_secs(2));
        let (a, b) = tokio::join!(env.get(), env.get());
        assert!(Arc::ptr_eq(&a.unwrap(), &b.unwrap()));
        assert_eq!(std::fs::read_to_string(&count).unwrap().lines().count(), 1);
    }
}
