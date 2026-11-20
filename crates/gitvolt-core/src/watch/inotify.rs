//! The file watcher's Linux backend: inotify, subscribed only to what can change a file's
//! contents, name or mode. `notify`'s own inotify backend also subscribes to `IN_OPEN` and has
//! no way to turn it off, so every file opened in a watched directory (git reading `.git`, an
//! editor, a build, GitVolt's own reads) woke its thread for an event the watcher then dropped.
//! Here reads wake nothing, and there is no thread: the inotify descriptor is read by a tokio
//! task. Events come out as `notify` events, as `notify`'s backend spells them.

use futures_util::StreamExt;
use ::inotify::{EventMask, Inotify, WatchDescriptor, WatchMask, Watches};
use notify::event::{CreateKind, DataChange, MetadataKind, ModifyKind, RemoveKind, RenameMode};
use notify::{Config, Error, ErrorKind, Event, EventHandler, EventKind, RecursiveMode, Result, Watcher, WatcherKind};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Writes (`CLOSE_WRITE`, and `MODIFY` for a file kept open, like a growing log), creates,
/// deletes, renames and mode changes. Never `OPEN`, `ACCESS` or `CLOSE_NOWRITE`.
const MASK: WatchMask = WatchMask::ATTRIB
    .union(WatchMask::CREATE)
    .union(WatchMask::DELETE)
    .union(WatchMask::CLOSE_WRITE)
    .union(WatchMask::MODIFY)
    .union(WatchMask::MOVED_FROM)
    .union(WatchMask::MOVED_TO)
    .union(WatchMask::DONT_FOLLOW);
/// A path watched itself (not a subfolder a recursive watch added) also reports its own
/// deletion or move.
const SELF_MASK: WatchMask = MASK.union(WatchMask::DELETE_SELF).union(WatchMask::MOVE_SELF);

#[derive(Default)]
struct State {
    /// Each watched folder: its descriptor, and whether its new subfolders are watched too.
    paths: HashMap<PathBuf, (WatchDescriptor, bool)>,
    wds: HashMap<WatchDescriptor, PathBuf>,
}

impl State {
    /// Watches `path` (and, if `recursive`, every folder under it).
    fn add(&mut self, watches: &mut Watches, path: &Path, recursive: bool, own: bool) -> Result<()> {
        let mask = if own { SELF_MASK } else { MASK };
        let wd = watches.add(path, mask).map_err(|e| {
            // ENOSPC: `fs.inotify.max_user_watches` reached.
            if e.kind() == std::io::ErrorKind::StorageFull {
                Error::new(ErrorKind::MaxFilesWatch)
            } else if e.kind() == std::io::ErrorKind::NotFound {
                Error::new(ErrorKind::PathNotFound)
            } else {
                Error::io(e)
            }
            .add_path(path.to_path_buf())
        })?;
        self.wds.insert(wd.clone(), path.to_path_buf());
        self.paths.insert(path.to_path_buf(), (wd, recursive));
        if recursive && let Ok(dir) = std::fs::read_dir(path) {
            for entry in dir.flatten() {
                // Not through symlinks, like `DONT_FOLLOW`.
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    match self.add(watches, &entry.path(), true, false) {
                        Err(e) if matches!(e.kind, ErrorKind::MaxFilesWatch) => return Err(e),
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }

    /// Unwatches `path` and every watched folder under it. `false` if it wasn't watched.
    fn remove(&mut self, watches: &mut Watches, path: &Path) -> bool {
        let under: Vec<PathBuf> = self.paths.keys().filter(|p| p.starts_with(path)).cloned().collect();
        for p in &under {
            if let Some((wd, _)) = self.paths.remove(p) {
                self.wds.remove(&wd);
                let _ = watches.remove(wd);
            }
        }
        !under.is_empty()
    }

    fn recursive_parent(&self, path: &Path) -> bool {
        path.parent().and_then(|p| self.paths.get(p)).is_some_and(|(_, r)| *r)
    }
}

pub struct QuietInotify {
    watches: Watches,
    state: Arc<Mutex<State>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for QuietInotify {
    fn drop(&mut self) {
        // The task owns the stream: aborting it closes the descriptor, and with it every watch.
        self.task.abort();
    }
}

impl Watcher for QuietInotify {
    /// Must be called within a tokio runtime (the descriptor is read by a task on it).
    fn new<F: EventHandler>(handler: F, _config: Config) -> Result<Self> {
        let rt = tokio::runtime::Handle::try_current().map_err(|e| Error::generic(&e.to_string()))?;
        let _guard = rt.enter();
        let inotify = Inotify::init().map_err(Error::io)?;
        let watches = inotify.watches();
        let stream = inotify.into_event_stream([0u8; 4096]).map_err(Error::io)?;
        let state = Arc::new(Mutex::new(State::default()));
        let task = rt.spawn(read_events(stream, watches.clone(), state.clone(), handler));
        Ok(Self { watches, state, task })
    }

    fn watch(&mut self, path: &Path, mode: RecursiveMode) -> Result<()> {
        let mut state = self.state.lock().expect("inotify state poisoned");
        state.add(&mut self.watches, path, mode == RecursiveMode::Recursive && path.is_dir(), true)
    }

    fn unwatch(&mut self, path: &Path) -> Result<()> {
        let mut state = self.state.lock().expect("inotify state poisoned");
        if state.remove(&mut self.watches, path) { Ok(()) } else { Err(Error::watch_not_found().add_path(path.to_path_buf())) }
    }

    fn kind() -> WatcherKind {
        WatcherKind::Inotify
    }
}

async fn read_events<F: EventHandler>(mut stream: ::inotify::EventStream<[u8; 4096]>, mut watches: Watches, state: Arc<Mutex<State>>, mut handler: F) {
    // The last `MOVED_FROM`, for a `MOVED_TO` with its cookie to pair with.
    let mut moved_from: Option<(u32, PathBuf)> = None;
    while let Some(ev) = stream.next().await {
        let ev = match ev {
            Ok(ev) => ev,
            Err(e) => {
                handler.handle_event(Err(Error::io(e)));
                continue;
            }
        };
        for out in translate(&ev, &mut watches, &state, &mut moved_from, &mut handler) {
            handler.handle_event(Ok(out));
        }
    }
}

/// `ev` as `notify` events, keeping the watches in step: folders created or moved under a
/// recursive watch are watched, folders deleted or moved away are unwatched.
fn translate<F: EventHandler>(ev: &::inotify::Event<OsString>, watches: &mut Watches, state: &Mutex<State>, moved_from: &mut Option<(u32, PathBuf)>, handler: &mut F) -> Vec<Event> {
    let m = ev.mask;
    if m.contains(EventMask::Q_OVERFLOW) {
        return vec![Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan)];
    }
    let mut state = state.lock().expect("inotify state poisoned");
    if m.contains(EventMask::IGNORED) {
        // The watch is gone (its folder deleted, or unwatched): forget it, unless re-added.
        if let Some(p) = state.wds.remove(&ev.wd)
            && state.paths.get(&p).is_some_and(|(wd, _)| *wd == ev.wd)
        {
            state.paths.remove(&p);
        }
        return Vec::new();
    }
    let Some(dir) = state.wds.get(&ev.wd).cloned() else { return Vec::new() };
    let path = match &ev.name {
        Some(name) => dir.join(name),
        None => dir,
    };
    let is_dir = m.contains(EventMask::ISDIR);
    let mut out = Vec::new();
    let mut watch_new = |state: &mut State, watches: &mut Watches| {
        if is_dir && state.recursive_parent(&path) {
            match state.add(watches, &path, true, false) {
                Err(e) if matches!(e.kind, ErrorKind::MaxFilesWatch) => handler.handle_event(Err(e)),
                _ => {}
            }
        }
    };
    if m.contains(EventMask::MOVED_FROM) {
        if state.paths.contains_key(&path) {
            state.remove(watches, &path);
        }
        *moved_from = Some((ev.cookie, path.clone()));
        out.push(Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::From))).add_path(path.clone()).set_tracker(ev.cookie as usize));
    } else if m.contains(EventMask::MOVED_TO) {
        out.push(Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::To))).add_path(path.clone()).set_tracker(ev.cookie as usize));
        if let Some((_, from)) = moved_from.take_if(|(cookie, _)| *cookie == ev.cookie) {
            out.push(Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both))).add_path(from).add_path(path.clone()).set_tracker(ev.cookie as usize));
        }
        watch_new(&mut state, watches);
    }
    if m.contains(EventMask::MOVE_SELF) {
        out.push(Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::From))).add_path(path.clone()));
    }
    if m.contains(EventMask::CREATE) {
        out.push(Event::new(EventKind::Create(if is_dir { CreateKind::Folder } else { CreateKind::File })).add_path(path.clone()));
        watch_new(&mut state, watches);
    }
    if m.contains(EventMask::DELETE) {
        out.push(Event::new(EventKind::Remove(if is_dir { RemoveKind::Folder } else { RemoveKind::File })).add_path(path.clone()));
        if state.paths.contains_key(&path) {
            state.remove(watches, &path);
        }
    }
    if m.contains(EventMask::DELETE_SELF) {
        out.push(Event::new(EventKind::Remove(RemoveKind::Folder)).add_path(path.clone()));
        state.remove(watches, &path);
    }
    if m.contains(EventMask::MODIFY) {
        out.push(Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Any))).add_path(path.clone()));
    }
    if m.contains(EventMask::CLOSE_WRITE) {
        out.push(Event::new(EventKind::Access(notify::event::AccessKind::Close(notify::event::AccessMode::Write))).add_path(path.clone()));
    }
    if m.contains(EventMask::ATTRIB) {
        out.push(Event::new(EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any))).add_path(path));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::mpsc;
    use std::time::Duration;

    /// A watcher of `dirs` on its own runtime, and its events.
    fn watching(dirs: &[(&Path, RecursiveMode)]) -> (tokio::runtime::Runtime, QuietInotify, mpsc::Receiver<Event>) {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let (tx, rx) = mpsc::channel();
        let mut w = {
            let _rt = rt.enter();
            QuietInotify::new(move |e: Result<Event>| tx.send(e.expect("no watch error")).unwrap(), Config::default()).unwrap()
        };
        for (d, mode) in dirs {
            w.watch(d, *mode).unwrap();
        }
        (rt, w, rx)
    }

    /// Every event until `quiet` passes without one.
    fn events(rx: &mpsc::Receiver<Event>, quiet: Duration) -> Vec<Event> {
        std::iter::from_fn(|| rx.recv_timeout(quiet).ok()).collect()
    }

    fn kinds_for(evs: &[Event], path: &Path) -> Vec<EventKind> {
        evs.iter().filter(|e| e.paths.iter().any(|p| p == path)).map(|e| e.kind).collect()
    }

    const QUIET: Duration = Duration::from_millis(200);

    /// The point of this backend: opening, reading and listing files (git reading refs, an
    /// editor, GitVolt itself) wakes nothing.
    #[test]
    fn reads_wake_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, refs) = (tmp.path().join("work"), tmp.path().join("refs"));
        std::fs::create_dir_all(refs.join("heads/feature")).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        let files: Vec<PathBuf> = (0..10).map(|i| root.join(format!("f{i}.txt"))).chain((0..10).map(|i| refs.join(format!("heads/feature/t{i}")))).collect();
        for f in &files {
            std::fs::write(f, "x\n").unwrap();
        }
        let (_rt, _w, rx) = watching(&[(&root, RecursiveMode::NonRecursive), (&refs, RecursiveMode::Recursive)]);
        for _ in 0..50 {
            for f in &files {
                std::fs::read(f).unwrap();
                std::fs::metadata(f).unwrap();
            }
            for d in [&root, &refs, &refs.join("heads"), &refs.join("heads/feature")] {
                std::fs::read_dir(d).unwrap().for_each(drop);
            }
        }
        assert_eq!(events(&rx, QUIET), Vec::new());
        std::fs::write(&files[0], "y\n").unwrap();
        let write = EventKind::Access(notify::event::AccessKind::Close(notify::event::AccessMode::Write));
        assert!(kinds_for(&events(&rx, QUIET), &files[0]).contains(&write));
    }

    #[test]
    fn writes_creates_renames_mode_changes_and_deletes_are_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, sub) = (tmp.path().to_path_buf(), tmp.path().join("sub"));
        let (a, b) = (root.join("a.txt"), root.join("b.txt"));
        let (_rt, _w, rx) = watching(&[(&root, RecursiveMode::NonRecursive)]);
        std::fs::write(&a, "x\n").unwrap();
        let evs = events(&rx, QUIET);
        assert!(kinds_for(&evs, &a).contains(&EventKind::Create(CreateKind::File)), "{evs:?}");
        assert!(kinds_for(&evs, &a).contains(&EventKind::Modify(ModifyKind::Data(DataChange::Any))), "{evs:?}");
        std::fs::set_permissions(&a, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(kinds_for(&events(&rx, QUIET), &a), vec![EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any))]);
        std::fs::rename(&a, &b).unwrap();
        let evs = events(&rx, QUIET);
        assert_eq!(evs.iter().map(|e| e.kind).collect::<Vec<_>>(), [RenameMode::From, RenameMode::To, RenameMode::Both].map(|m| EventKind::Modify(ModifyKind::Name(m))));
        assert_eq!(evs[2].paths, vec![a.clone(), b.clone()]);
        std::fs::remove_file(&b).unwrap();
        assert_eq!(kinds_for(&events(&rx, QUIET), &b), vec![EventKind::Remove(RemoveKind::File)]);
        std::fs::create_dir(&sub).unwrap();
        assert_eq!(kinds_for(&events(&rx, QUIET), &sub), vec![EventKind::Create(CreateKind::Folder)]);
        // Flat: what happens inside the new folder isn't watched.
        std::fs::write(sub.join("inner.txt"), "x\n").unwrap();
        assert_eq!(events(&rx, QUIET), Vec::new());
    }

    /// Under a recursive watch, new folders (with those made inside them before their own watch
    /// was added) are watched; a folder moved away is unwatched, so nothing under it is reported.
    #[test]
    fn recursive_watches_follow_new_and_moved_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let (refs, elsewhere) = (tmp.path().join("refs"), tmp.path().join("elsewhere"));
        std::fs::create_dir_all(&refs).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let (_rt, w, rx) = watching(&[(&refs, RecursiveMode::Recursive)]);
        std::fs::create_dir_all(refs.join("heads/feature")).unwrap();
        events(&rx, QUIET);
        let deep = refs.join("heads/feature/topic");
        std::fs::write(&deep, "x\n").unwrap();
        assert!(kinds_for(&events(&rx, QUIET), &deep).contains(&EventKind::Create(CreateKind::File)));
        std::fs::rename(refs.join("heads"), elsewhere.join("heads")).unwrap();
        events(&rx, QUIET);
        assert!(!w.state.lock().unwrap().paths.keys().any(|p| p.starts_with(refs.join("heads"))), "the moved folders' watches are dropped");
        std::fs::write(elsewhere.join("heads/feature/topic"), "y\n").unwrap();
        assert_eq!(events(&rx, QUIET), Vec::new());
    }

    #[test]
    fn a_missing_folder_is_not_found_and_so_is_one_not_watched() {
        let tmp = tempfile::tempdir().unwrap();
        let (_rt, mut w, _rx) = watching(&[]);
        let err = w.watch(&tmp.path().join("nope"), RecursiveMode::NonRecursive).unwrap_err();
        assert!(matches!(err.kind, ErrorKind::PathNotFound), "{err:?}");
        w.watch(tmp.path(), RecursiveMode::NonRecursive).unwrap();
        w.unwatch(tmp.path()).unwrap();
        assert!(matches!(w.unwatch(tmp.path()).unwrap_err().kind, ErrorKind::WatchNotFound));
    }

    /// Dropping the watcher ends its task (dropping the handler) and closes the descriptor.
    #[test]
    fn dropping_it_ends_the_task() {
        let tmp = tempfile::tempdir().unwrap();
        let (_rt, w, rx) = watching(&[(tmp.path(), RecursiveMode::NonRecursive)]);
        drop(w);
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Err(mpsc::RecvTimeoutError::Disconnected));
    }
}
