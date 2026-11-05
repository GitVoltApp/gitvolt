//! Git LFS (`ui/src/lfs/`): per repository, whether it's used and git-lfs's version, read once and again after a Track, a
//! checkout, a pull or `.gitattributes` changing; which file rows LFS stores, asked in batches as rows show; Pull, Fetch,
//! Track and Stop tracking; the status bar's "isn't installed"; a diff's "not downloaded" in place of its file.

use crate::app::{GitVolt, icon};
use crate::details::FileChange;
use crate::theme;
use crate::ui::menu::MenuRow;
use crate::ui::toast::{self, Options};
use crate::ui::tooltip::tip;
use crate::write::{Write, WriteCtx, call};
use gitvolt_core::events::{AppEvent, OpKind};
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// git-lfs is installed but `git lfs install` wasn't run for this repository.
const NOT_SET_UP: &str = "git-lfs isn't set up in this repository, so the files stay pointers: run git lfs install";
const INSTALL_COMMAND: &str = "git lfs install";
/// How long a burst of changes settles before the facts are read again.
const REFRESH_MS: u64 = 1000;
/// How often the window's focus may ask git-lfs's version again.
const FOCUS_RECHECK: Duration = Duration::from_secs(60);

/// `lfsStatus`' answer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LfsStatus {
    pub used: bool,
    pub version: Option<String>,
    pub install_url: String,
    pub patterns: Vec<String>,
    pub filter: bool,
}

fn status_of(v: &Value) -> LfsStatus {
    LfsStatus { used: v["used"] == true, version: v["version"].as_str().map(str::to_string), install_url: v["installUrl"].as_str().unwrap_or_default().to_string(), patterns: v["patterns"].as_array().map_or(Vec::new(), |p| p.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()), filter: v["filter"] == true }
}

#[derive(Default)]
pub struct Lfs {
    pub status: HashMap<u32, LfsStatus>,
    failed: HashSet<u32>,
    loading: HashSet<u32>,
    pub tracked: HashMap<(u32, String), HashMap<String, bool>>,
    queued: HashMap<(u32, String), HashSet<String>>,
    flushing: bool,
    refresh_seq: HashMap<u32, u64>,
    rechecks: HashSet<u32>,
    last_focus: Option<Instant>,
}

/// An LFS side of a diff whose file isn't downloaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfsSide {
    pub oid: String,
    pub size: u64,
    pub pointer: String,
}

/// `lfsMissing`: the diff's LFS sides whose real file isn't downloaded, old first.
pub fn lfs_missing(v: &Value) -> Vec<LfsSide> {
    ["old", "new"].iter().filter_map(|s| v[*s]["lfs"].as_object()).filter(|l| l.get("downloaded").and_then(Value::as_bool) != Some(true)).map(|l| LfsSide { oid: l.get("oid").and_then(Value::as_str).unwrap_or_default().to_string(), size: l.get("size").and_then(Value::as_u64).unwrap_or(0), pointer: l.get("pointer").and_then(Value::as_str).unwrap_or_default().to_string() }).collect()
}

/// `extPattern`: `*.<ext>` for a file with an extension git-lfs can track as a pattern.
pub fn ext_pattern(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let dot = name.rfind('.')?;
    if dot == 0 || dot + 1 == name.len() {
        return None;
    }
    let ext = &name[dot + 1..];
    (!ext.chars().any(|c| c.is_whitespace() || "/\\*?[]!#\"'".contains(c))).then(|| format!("*.{ext}"))
}

fn is_commit(c: &str) -> bool {
    (c.len() == 40 || c.len() == 64) && c.chars().all(|x| x.is_ascii_hexdigit() && !x.is_ascii_uppercase())
}

impl GitVolt {
    /// Reads repo `repo`'s status, once (`force`: again). A failure leaves it unknown, so nothing about LFS shows.
    pub fn load_lfs(&mut self, repo: u32, force: bool, recheck: bool, cx: &mut Context<Self>) {
        if cfg!(test) || (self.lfs.loading.contains(&repo) && !force) || (!force && (self.lfs.status.contains_key(&repo) || self.lfs.failed.contains(&repo))) {
            return;
        }
        self.lfs.failed.remove(&repo);
        self.lfs.loading.insert(repo);
        let api = self.api.clone();
        let body = call("lfsStatus", if recheck { json!({ "repo": crate::app::core_repo(repo), "recheck": true }) } else { json!({ "repo": crate::app::core_repo(repo) }) });
        let read = self.rt.spawn(async move { api.dispatch(crate::write::request_of(body)?).await });
        cx.spawn(async move |this, cx| {
            let answer = read.await;
            this.update(cx, |this, cx| {
                this.lfs.loading.remove(&repo);
                match answer {
                    Ok(Ok(v)) => {
                        let s = status_of(&v);
                        if this.lfs.status.get(&repo) != Some(&s) {
                            this.lfs.status.insert(repo, s);
                            cx.notify();
                        }
                    }
                    _ => {
                        this.lfs.failed.insert(repo);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn ask_tracked(&mut self, repo: u32, worktree: &str, path: &str, cx: &mut Context<Self>) {
        let key = (repo, worktree.to_string());
        if !self.lfs.queued.entry(key).or_default().insert(path.to_string()) {
            return;
        }
        if !self.lfs.flushing {
            self.lfs.flushing = true;
            cx.spawn(async move |this, cx| {
                this.update(cx, |this, cx| this.flush_tracked(cx)).ok();
            })
            .detach();
        }
    }

    fn flush_tracked(&mut self, cx: &mut Context<Self>) {
        self.lfs.flushing = false;
        for ((repo, worktree), paths) in std::mem::take(&mut self.lfs.queued) {
            let asked: Vec<String> = paths.into_iter().collect();
            let api = self.api.clone();
            let body = call("lfsTracked", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "paths": asked }));
            let read = self.rt.spawn(async move { api.dispatch(crate::write::request_of(body)?).await });
            cx.spawn(async move |this, cx| {
                let Ok(Ok(found)) = read.await else { return };
                let yes: HashSet<String> = found.as_array().map_or(HashSet::new(), |a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect());
                this.update(cx, |this, cx| {
                    let known = this.lfs.tracked.entry((repo, worktree)).or_default();
                    for p in asked {
                        let on = yes.contains(&p);
                        known.insert(p, on);
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    /// A file row's mark: `None` in a repository without LFS; `Some(false)` keeps its space until it's known to be LFS.
    pub fn lfs_mark(&mut self, path: &str, cx: &mut Context<Self>) -> Option<bool> {
        let tab = self.tabs.get(self.active)?;
        let (repo, worktree) = (tab.id, tab.write_target().worktree);
        if !self.lfs.status.get(&repo).is_some_and(|s| s.used) {
            return None;
        }
        let known = self.lfs.tracked.get(&(repo, worktree.clone())).and_then(|k| k.get(path)).copied();
        if known.is_none() {
            self.ask_tracked(repo, &worktree, path, cx);
        }
        Some(known == Some(true))
    }

    /// Status and every known path, read again. `recheck`: git-lfs's version too.
    pub fn refresh_lfs(&mut self, repo: u32, recheck: bool, cx: &mut Context<Self>) {
        let keys: Vec<(u32, String)> = self.lfs.tracked.keys().filter(|k| k.0 == repo).cloned().collect();
        for key in keys {
            let paths: Vec<String> = self.lfs.tracked[&key].keys().cloned().collect();
            for p in paths {
                self.ask_tracked(repo, &key.1, &p, cx);
            }
        }
        self.load_lfs(repo, true, recheck, cx);
    }

    fn refresh_lfs_soon(&mut self, repo: u32, recheck: bool, cx: &mut Context<Self>) {
        if recheck {
            self.lfs.rechecks.insert(repo);
        }
        let seq = self.lfs.refresh_seq.entry(repo).or_default();
        *seq += 1;
        let at = *seq;
        let timer = cx.background_executor().timer(Duration::from_millis(REFRESH_MS));
        cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |this, cx| {
                if this.lfs.refresh_seq.get(&repo) == Some(&at) {
                    let recheck = this.lfs.rechecks.remove(&repo);
                    this.refresh_lfs(repo, recheck, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// A repository that uses LFS changed on disk, or an LFS, pull or checkout write finished.
    pub fn lfs_on_event(&mut self, e: &AppEvent, cx: &mut Context<Self>) {
        match e {
            AppEvent::RepoChanged { repo, .. } if self.lfs.status.get(repo).is_some_and(|s| s.used) => self.refresh_lfs_soon(*repo, false, cx),
            AppEvent::OpFinished { repo: Some(repo), kind, outcome, .. } if matches!(kind, OpKind::Lfs | OpKind::Pull | OpKind::Checkout) => {
                let failed = *kind == OpKind::Lfs && format!("{outcome:?}").to_lowercase() == "failed";
                self.refresh_lfs_soon(*repo, failed, cx);
            }
            _ => {}
        }
    }

    /// git-lfs may have been installed meanwhile: on the window's focus, at most once a minute, the repositories that use
    /// LFS but were read without it are read again. One asks git; the core keeps the answer for the rest.
    pub fn lfs_on_focus(&mut self, cx: &mut Context<Self>) {
        if self.lfs.last_focus.is_some_and(|t| t.elapsed() < FOCUS_RECHECK) {
            return;
        }
        let repos: Vec<u32> = self.lfs.status.iter().filter(|(_, s)| s.used && s.version.is_none()).map(|(r, _)| *r).collect();
        if repos.is_empty() {
            return;
        }
        self.lfs.last_focus = Some(Instant::now());
        for (i, r) in repos.into_iter().enumerate() {
            self.load_lfs(r, true, i == 0, cx);
        }
    }

    /// After a download, fetch or pull: the open file reads its sides again.
    fn lfs_downloaded(&mut self, cx: &mut Context<Self>) {
        if self.tabs.get(self.active).is_some_and(|t| t.diff.is_some()) {
            self.read_contents(true, cx);
        }
    }

    /// `git lfs pull` or `git lfs fetch` for the tab, a queued write with Activity output.
    pub fn lfs_transfer(&mut self, pull: bool, cx: &mut Context<Self>) {
        let Some(ctx) = self.tabs.get(self.active).map(|t| t.write_target()) else { return };
        let (repo, worktree) = (ctx.repo, ctx.worktree.clone());
        let w = Write::new(&ctx, move |_| call(if pull { "lfsPull" } else { "lfsFetch" }, json!({ "repo": crate::app::core_repo(repo), "worktree": worktree }))).on_success(move |this, out, cx| {
            this.lfs_downloaded(cx);
            if out["notSetUp"] == true {
                toast::show_with(format!("Fetched the LFS files. {NOT_SET_UP}"), Options { warning: true, detail: Some(INSTALL_COMMAND.into()), ..Default::default() }, cx);
            } else {
                toast::show(if pull { "Pulled the LFS files" } else { "Fetched the LFS files" }, cx);
            }
        });
        self.write(w, cx);
    }

    /// Track or stop tracking `pattern`: `.gitattributes` changes, a WIP change to commit, undoable.
    pub fn lfs_set_tracked(&mut self, ctx: WriteCtx, pattern: String, track: bool, cx: &mut Context<Self>) {
        let (repo, worktree, p) = (ctx.repo, ctx.worktree.clone(), pattern.clone());
        let w = Write::new(&ctx, move |_| call(if track { "lfsTrack" } else { "lfsUntrack" }, json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "pattern": p }))).on_success(move |this, _, cx| {
            let set_up = this.lfs.status.get(&repo).is_none_or(|s| s.filter);
            this.refresh_lfs(repo, false, cx);
            if !track {
                toast::show(format!("Stopped tracking {pattern} with Git LFS"), cx);
            } else if !set_up {
                toast::show_with(format!("Tracking {pattern} with Git LFS, but git-lfs isn't set up in this repository yet: run git lfs install before you commit"), Options { warning: true, detail: Some(INSTALL_COMMAND.into()), ..Default::default() }, cx);
            } else {
                toast::show(format!("Tracking {pattern} with Git LFS: commit .gitattributes to share it"), cx);
            }
        });
        self.write(w, cx);
    }

    /// A file row's Track `*.<ext>` with LFS (git-lfs installed), or Stop tracking for a file LFS stores.
    pub fn lfs_rows(&self, file: &FileChange, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        let Some(tab) = self.tabs.get(self.active) else { return Vec::new() };
        let ctx = tab.write_target();
        let Some(pattern) = ext_pattern(&file.path) else { return Vec::new() };
        let Some(status) = self.lfs.status.get(&ctx.repo).filter(|s| s.version.is_some()) else { return Vec::new() };
        let tracked = if status.used { self.lfs.tracked.get(&(ctx.repo, ctx.worktree.clone())).and_then(|k| k.get(file.path.as_ref())).copied() } else { Some(false) };
        let Some(tracked) = tracked else { return Vec::new() };
        if tracked && !status.patterns.contains(&pattern) {
            return Vec::new();
        }
        let (weak, p) = (this.clone(), pattern.clone());
        let run = move |_: &mut Window, cx: &mut App| {
            let (ctx, p) = (ctx.clone(), p.clone());
            weak.update(cx, |v, cx| v.lfs_set_tracked(ctx, p, !tracked, cx)).ok();
        };
        vec![if tracked {
            MenuRow::action("file.lfsUntrack", format!("Stop tracking {pattern}"), "file-minus", format!("Stop storing {pattern} files with Git LFS (git lfs untrack): .gitattributes changes, and you can undo this"), run)
        } else {
            MenuRow::action("file.lfsTrack", format!("Track {pattern} with LFS"), "file-plus", format!("Store {pattern} files with Git LFS (git lfs track): .gitattributes changes, and you can undo this"), run)
        }]
    }

    pub fn open_install_url(&mut self, repo: u32, cx: &mut Context<Self>) {
        let Some(url) = self.lfs.status.get(&repo).map(|s| s.install_url.clone()) else { return };
        cx.open_url(&url);
    }

    /// Download: fetches that file at the diff's commits (checks it out where the working tree has it as a pointer), then
    /// the view reads it again.
    pub fn lfs_download(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let ctx = tab.write_target();
        let Some(diff) = tab.diff.as_mut() else { return };
        let Some(crate::diff::view::Notice::Lfs(missing)) = diff.notice.as_ref() else { return };
        let oids: Vec<String> = missing.iter().map(|l| l.oid.clone()).collect();
        let path = diff.file.path.to_string();
        let mut refs: Vec<String> = [&diff.file.old, &diff.file.new].iter().filter_map(|s| s["commit"].as_str()).filter(|c| is_commit(c)).map(str::to_string).collect();
        refs.dedup();
        diff.lfs_busy = true;
        let (repo, worktree, p) = (ctx.repo, ctx.worktree.clone(), path.clone());
        let w = Write::new(&ctx, move |_| call("lfsDownload", json!({ "repo": crate::app::core_repo(repo), "worktree": worktree, "path": p, "refs": refs, "oids": oids })))
            .on_success(move |this, out, cx| {
                this.lfs_downloaded(cx);
                if out["missing"].as_array().is_some_and(|m| !m.is_empty()) {
                    toast::show_with(format!("git-lfs didn't find {path} on the remote"), Options { error: true, ..Default::default() }, cx);
                } else if out["notSetUp"] == true {
                    toast::show_with(format!("Downloaded {path}. {NOT_SET_UP}"), Options { warning: true, detail: Some(INSTALL_COMMAND.into()), ..Default::default() }, cx);
                }
            })
            .on_end(|this, _, cx| {
                if let Some(d) = this.tabs.get_mut(this.active).and_then(|t| t.diff.as_mut()) {
                    d.lfs_busy = false;
                }
                cx.notify();
            });
        self.write(w, cx);
    }
}

/// `LfsMark`: a file row's mark; every row of a repository that uses LFS keeps its space.
pub fn lfs_mark_element(on: bool) -> AnyElement {
    let mark = div().flex_none().px(px(4.)).border_1().border_color(theme::section_border()).rounded(px(3.)).text_size(px(9.)).line_height(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_secondary()).child("LFS");
    if on { tip("Stored with Git LFS", mark).into_any_element() } else { mark.invisible().into_any_element() }
}

/// `LfsNotice`: the status bar's warning for a repository that uses LFS without git-lfs installed.
pub fn lfs_notice(v: &GitVolt, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let repo = v.tabs.get(v.active).map(|t| t.id)?;
    v.lfs.status.get(&repo).filter(|s| s.used && s.version.is_none())?;
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .text_color(theme::orange())
            .child(icon("triangle-alert", 12., theme::orange()))
            .child("This repository uses Git LFS, but git-lfs isn't installed: files check out as pointers")
            .child(div().id("lfs-install").text_color(theme::blue()).cursor_pointer().hover(|d| d.underline()).on_click(cx.listener(move |this, _, _, cx| this.open_install_url(repo, cx))).child("Install git-lfs"))
            .into_any_element(),
    )
}

/// `LfsPlaceholder`, in place of a diff or a file whose LFS file isn't downloaded. The pointer text is never shown as if it
/// were the file.
pub fn lfs_placeholder(missing: &[LfsSide], pointer: bool, busy: bool, no_git_lfs: bool, cx: &mut Context<GitVolt>) -> AnyElement {
    let sizes: Vec<String> = missing.iter().map(|l| crate::format::format_bytes(Some(l.size))).collect();
    let two = sizes.len() == 2 && sizes[0] != sizes[1];
    let first = sizes.first().cloned().unwrap_or_default();
    let line = div().flex().items_center().when(two, |d| d.child(format!("LFS file ({first}")).child(div().mx(px(4.)).child(icon("arrow-right", 12., theme::text_secondary()))).child(format!("{}), not downloaded", sizes[1]))).when(!two, |d| d.child(format!("LFS file ({first}), not downloaded")));
    let off = busy || no_git_lfs;
    let text_button = |id: &'static str, label: &'static str| div().id(id).flex().justify_center().px(px(10.)).py(px(3.)).rounded(px(4.)).bg(theme::blue()).text_color(theme::text_on_accent()).cursor_pointer().child(label);
    div()
        .flex()
        .flex_col()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .text_color(theme::text_secondary())
        .child(line)
        .child(
            div()
                .flex()
                .gap(px(8.))
                .child(crate::ui::anchor("lfs-download", text_button("lfs-download", if busy { "Downloading…" } else { "Download" }).min_w(px(81.)).when(off, |d| d.opacity(0.5)).when(!off, |d| d.on_click(cx.listener(|this, _, _, cx| this.lfs_download(cx))))))
                .child(crate::ui::anchor(
                    "lfs-pointer",
                    text_button("lfs-pointer", if pointer { "Hide pointer" } else { "Show pointer" }).on_click(cx.listener(|this, _, _, cx| {
                        this.with_diff(cx, |d| d.lfs_pointer = !d.lfs_pointer);
                    })),
                )),
        )
        .when(no_git_lfs, |d| d.child(div().flex().gap(px(6.)).text_color(theme::text_disabled()).child("git-lfs isn't installed, so it can't be downloaded.").child(div().id("lfs-install-hint").text_color(theme::blue()).cursor_pointer().hover(|d| d.underline()).on_click(cx.listener(|this, _, _, cx| {
            if let Some(repo) = this.tabs.get(this.active).map(|t| t.id) {
                this.open_install_url(repo, cx);
            }
        })).child("Install git-lfs"))))
        .when(pointer, |d| d.children(missing.iter().map(|l| div().mt(px(8.)).px(px(8.)).py(px(6.)).font_family("JetBrains Mono").text_size(px(11.)).text_color(theme::text_secondary()).bg(theme::toolbar_bg()).border_1().border_color(theme::section_border()).rounded(px(4.)).children(l.pointer.lines().map(|t| div().child(t.to_string()))))))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_pattern_comes_from_an_extension_git_lfs_can_track() {
        assert_eq!(ext_pattern("a/b/pic.PSD").as_deref(), Some("*.PSD"));
        assert_eq!(ext_pattern("x.tar.gz").as_deref(), Some("*.gz"));
        assert_eq!(ext_pattern(".gitignore"), None);
        assert_eq!(ext_pattern("Makefile"), None);
        assert_eq!(ext_pattern("trail."), None);
        assert_eq!(ext_pattern("odd.a[b"), None);
    }

    #[test]
    fn only_sides_not_downloaded_are_missing() {
        let v = json!({ "old": { "lfs": { "oid": "a", "size": 3, "downloaded": true, "pointer": "p" } }, "new": { "lfs": { "oid": "b", "size": 4, "downloaded": false, "pointer": "q" } } });
        assert_eq!(lfs_missing(&v), [LfsSide { oid: "b".into(), size: 4, pointer: "q".into() }]);
        assert!(lfs_missing(&json!({ "old": { "text": "x" }, "new": null })).is_empty());
    }

    #[test]
    fn status_reads_as_the_core_says_it() {
        let s = status_of(&json!({ "used": true, "version": null, "installUrl": "https://example.com", "patterns": ["*.bin"], "filter": false }));
        assert!(s.used && s.version.is_none() && !s.filter);
        assert_eq!(s.patterns, ["*.bin"]);
    }
}
