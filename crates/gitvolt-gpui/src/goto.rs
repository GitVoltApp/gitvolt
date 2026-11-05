//! Paste anywhere to go to it (`ui/src/goto`): Ctrl+V outside a text field reads the clipboard and goes
//! to the commit, file, folder, branch, forge page or MR/PR number it names. Forge targets are in `forge::goto`.

use crate::app::GitVolt;
use gitvolt_core::api::Request;
use gpui::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `loose`: all digits or all letters; not found, it's ignored.
    Commit { sha: String, ref_name: Option<String>, loose: bool },
    /// `loose`: one relative name with no folder, which could as well be any word.
    Path { path: String, absolute: bool, line: Option<u32>, loose: bool, ref_name: Option<String> },
    Ref(String),
    Forge(crate::forge::goto::ForgeTarget),
    None,
}

fn is_hash(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn ref_name(s: &str) -> Option<String> {
    let bad = s.len() > 255
        || s.chars().any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c))
        || s.contains("..")
        || s.contains("@{")
        || s.contains("//")
        || s.starts_with(['-', '/', '.'])
        || s.ends_with(['/', '.'])
        || s.ends_with(".lock")
        || s == "@";
    (!bad).then(|| s.to_string())
}

fn path_of(s: &str) -> Option<Target> {
    let (mut p, mut line) = (s, None);
    if let Some((rest, n)) = s.rsplit_once(':')
        && let Ok(n) = n.split('-').next().unwrap_or(n).trim_start_matches('L').parse::<u32>()
        && n >= 1
    {
        p = rest;
        line = Some(n);
    } else if let Some((rest, n)) = s.rsplit_once("#L")
        && let Ok(n) = n.split('-').next().unwrap_or(n).parse::<u32>()
    {
        p = rest;
        line = Some(n);
    }
    if p.is_empty() || p.chars().any(|c| "\0<>|\"*?".contains(c)) {
        return None;
    }
    let absolute = p.starts_with('/');
    if !absolute && p.chars().any(char::is_whitespace) {
        return None;
    }
    let trimmed = p.trim_end_matches('/').trim_start_matches("./");
    if trimmed.is_empty() {
        return None;
    }
    let loose = !absolute && !trimmed.contains('/');
    Some(Target::Path { path: trimmed.to_string(), absolute, line, loose, ref_name: if line.is_none() { ref_name(s) } else { None } })
}

/// `classify`: what the clipboard's first line names.
pub fn classify(text: &str) -> Target {
    if text.len() > 4096 {
        return Target::None;
    }
    let Some(s) = text.lines().map(str::trim).find(|l| !l.is_empty()) else { return Target::None };
    if let Some(p) = s.strip_prefix("file://") {
        return path_of(p).unwrap_or(Target::None);
    }
    if s.contains("://") {
        return crate::forge::goto::classify_url(s).map_or(Target::None, Target::Forge);
    }
    if let Some(scheme) = s.split_once(':').map(|p| p.0)
        && scheme.len() > 1
        && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        && !s[scheme.len() + 1..].starts_with(|c: char| c.is_ascii_digit())
    {
        return Target::None;
    }
    let rest = s.strip_prefix("commit ").map_or(s, str::trim);
    let word = rest.split_whitespace().next().unwrap_or(rest);
    let tail_ok = rest == word || rest[word.len()..].trim_start().starts_with('(');
    if is_hash(word) && tail_ok {
        let digits = word.bytes().all(|b| b.is_ascii_digit());
        let letters = word.bytes().all(|b| matches!(b, b'a'..=b'f' | b'A'..=b'F'));
        return Target::Commit { sha: word.to_lowercase(), ref_name: ref_name(s), loose: digits || letters };
    }
    // `!12`, `#12`, `group/project!12` are MRs.
    if let Some(t) = crate::forge::goto::classify_short(s) {
        return Target::Forge(t);
    }
    if let Some(t) = path_of(s) {
        return t;
    }
    ref_name(s).map_or(Target::None, Target::Ref)
}

impl GitVolt {
    pub fn go_to_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else { return };
        self.go_to_text(&text, cx);
    }

    pub fn go_to_text(&mut self, text: &str, cx: &mut Context<Self>) {
        match classify(text) {
            Target::None => {}
            Target::Forge(t) => self.go_to_forge(t, cx),
            Target::Ref(name) => {
                if let Some(tip) = self.ref_tip(&name) {
                    self.go_to_sha(&tip, cx);
                }
            }
            Target::Commit { sha, ref_name, loose } => self.go_to_commit_text(sha, ref_name, loose, cx),
            Target::Path { path, absolute, line, loose, ref_name } => self.go_to_path(path, absolute, line, loose, ref_name, cx),
        }
    }

    /// The commit a local branch, remote branch or tag of the tab points at.
    fn ref_tip(&self, name: &str) -> Option<String> {
        let tab = self.tabs.get(self.active)?;
        tab.side.sections.iter().flat_map(|s| &s.items).find(|i| i.name == name || matches!(&i.kind, crate::sidebar::ItemKind::Remote { remote, .. } if format!("{remote}/{}", i.name) == name)).and_then(|i| i.target.clone())
    }

    fn go_to_commit_text(&mut self, sha: String, ref_name: Option<String>, loose: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        if let Some(tip) = ref_name.as_ref().and_then(|r| self.ref_tip(r)) {
            return self.go_to_sha(&tip, cx);
        }
        let (repo, name, api, id) = (tab.id, tab.name.clone(), self.api.clone(), sha.clone());
        let located = self.rt.spawn(async move { api.dispatch(Request::LocateCommit { repo: crate::app::core_repo(repo), sha: id }).await });
        cx.spawn(async move |this, cx| {
            let v = match located.await {
                Ok(Ok(v)) => v,
                _ => serde_json::Value::Null,
            };
            this.update(cx, |this, cx| {
                if v["ambiguous"].as_bool() == Some(true) {
                    return this.show_toast(format!("{sha} is ambiguous in {name}: paste more of the hash"), cx);
                }
                if v["found"].as_bool() == Some(true) {
                    let full = v["id"].as_str().unwrap_or(&sha).to_string();
                    if this.in_graph(&full) {
                        return this.go_to_sha(&full, cx);
                    }
                    if let Some(limit) = v["limit"].as_u64() {
                        return this.load_deep(repo, limit as u32, Some(full), cx);
                    }
                    return this.show_toast(format!("{} isn't in {name}'s graph (deeper than 10,000 commits, or on no branch)", &full[..full.len().min(6)]), cx);
                }
                if !loose {
                    this.show_toast(format!("No commit {sha} in {name}"), cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn go_to_path(&mut self, path: String, absolute: bool, line: Option<u32>, loose: bool, ref_name: Option<String>, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let worktree = tab.worktree.clone();
        let rel = if absolute {
            match path.strip_prefix(&format!("{}/", worktree.trim_end_matches('/'))) {
                Some(r) => r.to_string(),
                None => return self.show_toast(format!("{path} isn't in an open repository"), cx),
            }
        } else {
            path.clone()
        };
        let (repo, name, api, at, query) = (tab.id, tab.name.clone(), self.api.clone(), rel.clone(), worktree.clone());
        let kind = self.rt.spawn(async move { api.dispatch(Request::WorktreePathKind { repo: crate::app::core_repo(repo), worktree: query, path: at }).await });
        cx.spawn(async move |this, cx| {
            let kind = match kind.await {
                Ok(Ok(v)) => v.as_str().map(str::to_string),
                _ => None,
            };
            this.update(cx, |this, cx| match kind.as_deref() {
                Some("file") => {
                    this.open_worktree_file(&rel, cx);
                    if let Some(line) = line {
                        this.with_diff(cx, |d| d.pending_line = Some(line.saturating_sub(1)));
                    }
                }
                Some(_) => this.show_toast(format!("{rel} is a folder"), cx),
                None => {
                    if let Some(tip) = ref_name.as_ref().and_then(|r| this.ref_tip(r)) {
                        return this.go_to_sha(&tip, cx);
                    }
                    if !loose {
                        this.show_toast(format!("No file {rel} in {name}"), cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Opens a file of the working tree, in its File View.
    pub fn open_worktree_file(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let file = crate::details::file_in_worktree(path, &tab.worktree);
        tab.history = None;
        tab.diff = Some(crate::diff::view::DiffState::carrying(file, tab.diff.as_ref()));
        if let Some(d) = tab.diff.as_mut() {
            d.view = crate::diff::view::ViewKind::File;
        }
        tab.sidebar_open = false;
        self.read_contents(false, cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn it_reads_what_the_clipboard_names() {
        assert_eq!(classify("5df155c1"), Target::Commit { sha: "5df155c1".into(), ref_name: Some("5df155c1".into()), loose: false });
        assert_eq!(classify("commit ABCDEF0"), Target::Commit { sha: "abcdef0".into(), ref_name: None, loose: false });
        assert!(matches!(classify("deadbeef"), Target::Commit { loose: true, .. }));
        assert!(matches!(classify("1234567"), Target::Commit { loose: true, .. }));
        assert_eq!(classify("src/main.rs:12"), Target::Path { path: "src/main.rs".into(), absolute: false, line: Some(12), loose: false, ref_name: None });
        assert_eq!(classify("feature/x"), Target::Path { path: "feature/x".into(), absolute: false, line: None, loose: false, ref_name: Some("feature/x".into()) });
        assert!(matches!(classify("README"), Target::Path { loose: true, .. }));
        assert_eq!(classify("https://example.com/a/b"), Target::None);
        assert!(matches!(classify("!12"), Target::Forge(_)));
        assert!(matches!(classify("group/project#3"), Target::Forge(_)));
        assert_eq!(classify("mailto:a@example.com"), Target::None);
        assert_eq!(classify("   \n  "), Target::None);
        assert_eq!(classify("a sentence with words"), Target::None);
    }
}
