//! A child's own process group, and stopping all of it (git and whatever it started: ssh, a
//! credential helper, a hook).
//!
//! Unix: the child leads a new process group (or a new session, so it has no controlling
//! terminal), and a stop signals the whole group: SIGTERM first for a write (git removes its
//! `.lock` files), SIGKILL. Windows: the child starts without a console window
//! (`CREATE_NO_WINDOW`, so none flashes up and nothing can prompt on one) and is put in a job
//! object right after it starts; a stop terminates the job, which holds everything it started.
//! There's no SIGTERM for a windowless process there, so [`Group::terminate`] kills too: a
//! write stopped mid-way may leave a `.lock` file (Remove stale lock offers to clear it).
//!
//! Unix: a child must start without `fork`. std starts it with `posix_spawn` unless it has to
//! run code in the child first: a `pre_exec` closure, or a `PATH` search or relative program it
//! must do in the child's own environment and directory. Then it forks, and in the CEF runtime
//! a forked child can deadlock before `exec`: the atfork handlers (macOS `xpc_atfork_child`)
//! take a lock another thread held at the fork, and the parent's `spawn` waits on it for good.
//! [`spawnable_program`] does that lookup here; [`own_session`] needs no `setsid` when the
//! app has no controlling terminal.

use std::path::PathBuf;
use std::process::Command;

/// Starts the child in its own process group (Unix), so a stop reaches its children too.
pub fn own_group(cmd: &mut Command) {
    imp::own_group(cmd);
}

/// Starts the child with no controlling terminal, so ssh or a credential helper can't open
/// `/dev/tty` to prompt. It leads its own group as well. Unix: when the app has a controlling
/// terminal (a dev run from a shell) the child gets a new session (`setsid`, which forks: see
/// the module docs); without one (started from the Dock or a desktop session) its own group is
/// enough, as a child that isn't a session leader can't gain a terminal.
pub fn own_session(cmd: &mut Command) {
    imp::own_session(cmd);
}

/// The absolute program to start `cmd` with, when std would otherwise fork to find it (see the
/// module docs); `None` when std starts it as it is. `env_cleared`: whether `cmd`'s environment
/// was cleared (`Command` can't tell). Resolves as `execvp` would in the child: `PATH` (`cmd`'s
/// own, else the system default when cleared or removed) in order, an empty or relative entry
/// against `cmd`'s directory, the first executable regular file. None found: `None`, so the
/// spawn fails as before. The caller starts the result with `arg0` set to the original name.
/// Windows: always `None` (no fork there).
pub fn spawnable_program(cmd: &Command, env_cleared: bool) -> Option<PathBuf> {
    imp::spawnable_program(cmd, env_cleared)
}

/// Whether [`own_session`] gives a child a new session now (the app has a terminal).
#[cfg(all(unix, test))]
pub(crate) fn detaches_by_setsid() -> bool {
    imp::needs_new_session(imp::has_terminal_probe())
}

/// A started child's process group (Unix) or job (Windows).
#[derive(Debug, Clone)]
pub struct Group(imp::Group);

impl Group {
    /// The group of `child`, started with [`own_group`] or [`own_session`].
    pub fn of(child: &tokio::process::Child) -> Self {
        Self(imp::Group::of(child))
    }

    /// Asks every process in it to stop (SIGTERM). Windows: kills them (see the module docs).
    pub fn terminate(&self) {
        self.0.terminate();
    }

    /// Kills every process in it (SIGKILL).
    pub fn kill(&self) {
        self.0.kill();
    }
}

#[cfg(unix)]
mod imp {
    use nix::sys::signal::Signal;
    use std::ffi::OsStr;
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    pub fn own_group(cmd: &mut Command) {
        cmd.process_group(0);
    }

    pub fn own_session(cmd: &mut Command) {
        if !needs_new_session(has_terminal_probe()) {
            cmd.process_group(0);
            return;
        }
        // SAFETY: `setsid` is async-signal-safe and the closure touches nothing else.
        unsafe {
            cmd.pre_exec(|| nix::unistd::setsid().map(drop).map_err(std::io::Error::from));
        }
    }

    /// Opens the app's controlling terminal (without making it one), checked at each start: the
    /// app has none when that fails with `ENXIO`.
    pub(super) fn has_terminal_probe() -> std::io::Result<()> {
        std::fs::OpenOptions::new().read(true).write(true).custom_flags(nix::fcntl::OFlag::O_NOCTTY.bits()).open("/dev/tty").map(drop)
    }

    /// Whether a child needs `setsid` to lose the terminal. Fails closed: only `ENXIO` ("no
    /// controlling terminal") spares it.
    pub(super) fn needs_new_session(probe: std::io::Result<()>) -> bool {
        !matches!(probe, Err(e) if e.raw_os_error() == Some(nix::errno::Errno::ENXIO as i32))
    }

    /// Where `execvp` looks when there's no `PATH` (`_PATH_DEFPATH`; glibc's own order).
    #[cfg(not(target_os = "linux"))]
    const DEFAULT_PATH: &str = "/usr/bin:/bin";
    #[cfg(target_os = "linux")]
    const DEFAULT_PATH: &str = "/bin:/usr/bin";

    pub fn spawnable_program(cmd: &Command, env_cleared: bool) -> Option<PathBuf> {
        let program = Path::new(cmd.get_program());
        if program.is_absolute() {
            return None;
        }
        // The child's directory, absolute (a relative `cwd` is relative to ours).
        let base = || match cmd.get_current_dir() {
            Some(dir) if dir.is_absolute() => Some(dir.to_path_buf()),
            dir => std::env::current_dir().ok().map(|ours| dir.map_or_else(|| ours.clone(), |d| ours.join(d))),
        };
        if program.components().count() > 1 {
            // A relative path is the child's directory's: std forks for it only with one set.
            return cmd.get_current_dir().and(base()).map(|dir| dir.join(program));
        }
        let path = match cmd.get_envs().find(|(k, _)| *k == OsStr::new("PATH")) {
            Some((_, Some(path))) => path.to_owned(),
            Some((_, None)) => DEFAULT_PATH.into(),
            None if env_cleared => DEFAULT_PATH.into(),
            // `PATH` is ours, so `posix_spawnp` searches it as the child would.
            None => return None,
        };
        let base = base()?;
        std::env::split_paths(&path).map(|dir| if dir.is_relative() { base.join(dir) } else { dir }).map(|dir| dir.join(program)).find(|p| is_executable(p))
    }

    /// A regular file (following links) this process may execute, as `execvp` tries it.
    fn is_executable(path: &Path) -> bool {
        path.is_file() && nix::unistd::access(path, nix::unistd::AccessFlags::X_OK).is_ok()
    }

    /// The leader's pid, which is the group's id. Accepted risk (1C review M7): if the leader was
    /// already reaped, its pid could in theory be reused and `killpg` would hit a stranger's
    /// group. The window is tiny (we kill on timeout/cancel while the child is still ours,
    /// before `wait` reaps it, and a zombie leader keeps its pid and group alive), so no extra
    /// guard is added.
    #[derive(Debug, Clone)]
    pub struct Group {
        pid: Option<u32>,
    }

    impl Group {
        pub fn of(child: &tokio::process::Child) -> Self {
            Self { pid: child.id() }
        }

        fn signal(&self, signal: Signal) {
            if let Some(pid) = self.pid {
                let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid as i32), signal);
            }
        }

        pub fn terminate(&self) {
            self.signal(Signal::SIGTERM);
        }

        pub fn kill(&self) {
            self.signal(Signal::SIGKILL);
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use std::sync::Arc;
    use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, TerminateJobObject};
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    pub fn own_group(cmd: &mut Command) {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    pub fn own_session(cmd: &mut Command) {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    pub fn spawnable_program(_cmd: &Command, _env_cleared: bool) -> Option<std::path::PathBuf> {
        None
    }

    /// The job the child was put in right after it started (what it starts joins it too). If
    /// that failed, `taskkill /T` on its pid (its tree, while the parent still runs).
    #[derive(Debug, Clone)]
    pub struct Group {
        job: Option<Arc<OwnedHandle>>,
        pid: Option<u32>,
    }

    impl Group {
        pub fn of(child: &tokio::process::Child) -> Self {
            Self { job: child.raw_handle().and_then(job_for), pid: child.id() }
        }

        pub fn terminate(&self) {
            self.kill();
        }

        pub fn kill(&self) {
            match &self.job {
                // SAFETY: a job handle we own.
                Some(job) => unsafe {
                    TerminateJobObject(job.as_raw_handle(), 1);
                },
                None => {
                    if let Some(pid) = self.pid {
                        let _ = Command::new("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).creation_flags(CREATE_NO_WINDOW).output();
                    }
                }
            }
        }
    }

    fn job_for(process: RawHandle) -> Option<Arc<OwnedHandle>> {
        // SAFETY: an unnamed job with default security; the handle is checked, then owned.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return None;
        }
        // SAFETY: `job` is a fresh valid handle that nothing else owns.
        let job = unsafe { OwnedHandle::from_raw_handle(job) };
        // SAFETY: both handles are valid; the process one is the child's, still open.
        (unsafe { AssignProcessToJobObject(job.as_raw_handle(), process) } != 0).then(|| Arc::new(job))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A child that starts a grandchild and waits: killing the group stops both, so the pipe
    /// they share closes and the read ends.
    #[tokio::test]
    async fn killing_the_group_stops_the_children_too() {
        #[cfg(unix)]
        let mut cmd = {
            let mut c = Command::new("sh");
            c.args(["-c", "sleep 30 & wait"]);
            c
        };
        #[cfg(windows)]
        let mut cmd = {
            let mut c = Command::new("cmd");
            c.args(["/C", "ping -n 30 127.0.0.1 >NUL & ping -n 30 127.0.0.1"]);
            c
        };
        cmd.stdout(std::process::Stdio::piped()).stdin(std::process::Stdio::null());
        own_group(&mut cmd);
        let mut child = tokio::process::Command::from(cmd).spawn().unwrap();
        let group = Group::of(&child);
        let mut out = child.stdout.take().unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        group.kill();
        let status = tokio::time::timeout(Duration::from_secs(10), child.wait()).await.expect("the child stops").unwrap();
        assert!(!status.success());
        let mut rest = Vec::new();
        let read = tokio::time::timeout(Duration::from_secs(10), tokio::io::AsyncReadExt::read_to_end(&mut out, &mut rest)).await;
        assert!(read.is_ok(), "no grandchild keeps the pipe open");
    }

    #[cfg(unix)]
    mod spawnable {
        use super::super::imp::needs_new_session;
        use super::super::spawnable_program;
        use std::path::Path;
        use std::process::Command;

        fn exe(dir: &Path, name: &str, mode: u32) {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(name), "#!/bin/sh\n").unwrap();
            crate::platform::fs::set_mode(dir.join(name), mode).unwrap();
        }

        fn path_of(dirs: &[&Path]) -> std::ffi::OsString {
            std::env::join_paths(dirs).unwrap()
        }

        #[test]
        fn the_first_executable_in_path_wins_skipping_what_cant_run() {
            let t = tempfile::tempdir().unwrap();
            let (none, plain, dir, good, later) = (t.path().join("none"), t.path().join("plain"), t.path().join("dir"), t.path().join("good"), t.path().join("later"));
            exe(&plain, "tool", 0o644);
            std::fs::create_dir_all(dir.join("tool")).unwrap();
            exe(&good, "tool", 0o755);
            exe(&later, "tool", 0o755);
            let mut cmd = Command::new("tool");
            cmd.env("PATH", path_of(&[&none, &plain, &dir, &good, &later]));
            assert_eq!(spawnable_program(&cmd, false), Some(good.join("tool")));
        }

        /// As `execvp` in the child: an empty or relative entry is the child's directory's.
        #[test]
        fn empty_and_relative_entries_are_the_childs_directorys() {
            let t = tempfile::tempdir().unwrap();
            exe(t.path(), "here", 0o755);
            exe(&t.path().join("bin"), "below", 0o755);
            let mut cmd = Command::new("here");
            cmd.env("PATH", "/nonexistent::").current_dir(t.path());
            assert_eq!(spawnable_program(&cmd, false), Some(t.path().join("here")));
            let mut cmd = Command::new("below");
            cmd.env("PATH", "bin").current_dir(t.path());
            assert_eq!(spawnable_program(&cmd, false), Some(t.path().join("bin").join("below")));
        }

        #[test]
        fn a_cleared_or_removed_path_searches_the_system_default() {
            let mut cleared = Command::new("sh");
            cleared.env_clear();
            let mut removed = Command::new("sh");
            removed.env_remove("PATH");
            for cmd in [cleared, removed] {
                let found = spawnable_program(&cmd, true).expect("sh in the default PATH");
                assert!(found.is_absolute() && found.ends_with("sh"), "{found:?}");
            }
        }

        /// std starts these with `posix_spawn` as they are.
        #[test]
        fn what_std_spawns_itself_is_left_alone() {
            assert_eq!(spawnable_program(&Command::new("sh"), false), None, "our own PATH");
            assert_eq!(spawnable_program(Command::new("/bin/sh").env("PATH", "/x"), false), None, "absolute");
            assert_eq!(spawnable_program(Command::new("sub/tool").env("PATH", "/x"), false), None, "relative, our directory");
            assert_eq!(spawnable_program(Command::new("no-such-tool-anywhere").env("PATH", "/nonexistent"), false), None, "not found");
        }

        #[test]
        fn a_relative_program_is_the_childs_directorys() {
            let mut cmd = Command::new("sub/tool");
            cmd.current_dir("/some/repo");
            assert_eq!(spawnable_program(&cmd, false), Some(Path::new("/some/repo/sub/tool").to_path_buf()));
        }

        /// Fails closed: only "no controlling terminal" spares the child its own session.
        #[test]
        fn only_enxio_skips_setsid() {
            let err = |e: nix::errno::Errno| Err(std::io::Error::from_raw_os_error(e as i32));
            assert!(!needs_new_session(err(nix::errno::Errno::ENXIO)));
            assert!(needs_new_session(Ok(())));
            assert!(needs_new_session(err(nix::errno::Errno::EACCES)));
            assert!(needs_new_session(err(nix::errno::Errno::EMFILE)));
        }
    }
}
