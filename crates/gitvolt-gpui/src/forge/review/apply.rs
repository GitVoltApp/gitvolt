//! A suggestion's Apply (`forge/review/applySuggestion.ts`): the suggested lines go into the working copy, as an edit, never a
//! commit or a forge call; only where it lands as the forge meant it. The file is read when it's clicked.

use crate::app::{GitVolt, Tab, core_repo};
use crate::md::SuggestionBase;
use crate::md::ast::Flavor;
use crate::write::{Write, WriteCtx, call};
use gitvolt_core::api::Request;
use gitvolt_core::forge::ForgeKind;
use gitvolt_core::forge::types::{DiffPosition, ForgeMr};
use gpui::*;
use serde_json::{Value, json};

pub const ALREADY_APPLIED: &str = "Already applied";
pub const LINES_CHANGED: &str = "The lines changed since; apply it by hand";
pub const OLDER_VERSION: &str = "This comment is on an older version of the file";

/// A note's suggestions: each fence's language and its lines. Bounded: nobody writes more in one comment.
pub fn suggestions_of(text: &str) -> Vec<(String, Vec<String>)> {
    const MAX: usize = 20;
    let mut out = Vec::new();
    let mut open: Option<(char, usize, String, Vec<String>)> = None;
    for line in text.lines() {
        let t = line.trim_start();
        let c = t.chars().next().unwrap_or(' ');
        let n = if c == '`' || c == '~' { t.chars().take_while(|x| *x == c).count() } else { 0 };
        match &mut open {
            None if n >= 3 => {
                let lang = t[n..].split_whitespace().next().unwrap_or_default().to_string();
                open = Some((c, n, lang, Vec::new()));
            }
            Some((oc, on, lang, lines)) => {
                if c == *oc && n >= *on && t[n..].trim().is_empty() {
                    if crate::md::safe::suggestion_lang(lang) && out.len() < MAX {
                        out.push((lang.clone(), std::mem::take(lines)));
                    }
                    open = None;
                } else {
                    lines.push(line.trim_end_matches('\r').to_string());
                }
            }
            None => {}
        }
    }
    out
}

/// `snippetBase`: what a note's suggestion replaces, from its snippet's new-side lines. A note on a removed line has none.
pub fn snippet_base(flavor: Flavor, pos: Option<&DiffPosition>) -> Option<SuggestionBase> {
    let pos = pos?;
    let line = pos.line?;
    let raw: Vec<&str> = pos.snippet.as_deref()?.split('\n').collect();
    if raw.last()?.starts_with('-') {
        return None;
    }
    let new_side: Vec<String> = raw.iter().filter(|l| !l.starts_with('-') && !l.starts_with('\\')).map(|l| l.get(1..).unwrap_or_default().to_string()).collect();
    let first = (line + 1).checked_sub(new_side.len() as u32)?;
    let start = pos.start_line.filter(|s| *s < line).unwrap_or(line);
    Some(SuggestionBase {
        flavor,
        path: Some(pos.path.clone()),
        language: None,
        span: line - start,
        line,
        lines: std::sync::Arc::new(move |from, to| (from >= first && to <= line).then(|| new_side[(from - first) as usize..=(to - first) as usize].to_vec())),
    })
}

/// A text's lines and each one's ending (none for a last line without one).
fn lines_of(text: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        match rest.find('\n') {
            None => {
                out.push((rest, ""));
                break;
            }
            Some(n) => {
                let cr = n > 0 && rest.as_bytes()[n - 1] == b'\r';
                out.push((&rest[..if cr { n - 1 } else { n }], if cr { "\r\n" } else { "\n" }));
                rest = &rest[n + 1..];
            }
        }
    }
    out
}

/// `applyCheck`: why it can't go into `text` at line `from`. The suggested lines found there say it's in, unless the replaced ones
/// are too and the suggestion is the shorter (it takes lines off their end).
pub fn apply_check(text: &str, from: u32, replaced: &[String], suggested: &[String]) -> Option<&'static str> {
    let lines: Vec<&str> = lines_of(text).into_iter().map(|l| l.0).collect();
    let at = |n: usize| -> Vec<&str> { if from >= 1 { lines.iter().skip(from as usize - 1).take(n).copied().collect() } else { Vec::new() } };
    let want: Vec<&str> = replaced.iter().map(|l| l.trim_end_matches('\r')).collect();
    let sug: Vec<&str> = suggested.iter().map(String::as_str).collect();
    let has_old = at(want.len()) == want;
    if want == sug {
        return Some(if has_old { ALREADY_APPLIED } else { LINES_CHANGED });
    }
    let has_new = !sug.is_empty() && at(sug.len()) == sug;
    if has_new && (!has_old || sug.len() > want.len()) {
        return Some(ALREADY_APPLIED);
    }
    (!has_old).then_some(LINES_CHANGED)
}

/// `replaceLines`: in the file's line endings, and a file without a final newline stays without one.
pub fn replace_lines(text: &str, from: u32, count: usize, new_lines: &[String]) -> Option<String> {
    let all = lines_of(text);
    let (from, to) = (from as usize, from as usize + count - 1);
    if from < 1 || count < 1 || to > all.len() {
        return None;
    }
    let eol = if !all[from - 1].1.is_empty() { all[from - 1].1 } else { all.iter().map(|l| l.1).find(|e| !e.is_empty()).unwrap_or("\n") };
    let last = all[to - 1].1;
    let mut out: Vec<(&str, &str)> = all[..from - 1].to_vec();
    let put = new_lines.len();
    out.extend(new_lines.iter().enumerate().map(|(i, b)| (b.as_str(), if i + 1 == put { last } else { eol })));
    out.extend_from_slice(&all[to..]);
    if put == 0
        && last.is_empty()
        && let Some(l) = out.last_mut()
    {
        l.1 = "";
    }
    Some(out.iter().map(|(b, e)| format!("{b}{e}")).collect())
}

/// `applyGate`: why it can't run against what the tab has checked out, or where it writes and what it expects there.
pub fn apply_gate(t: &Tab, kind: ForgeKind, mr: &ForgeMr, head_sha: Option<&str>, path: &str, position: Option<&DiffPosition>) -> Result<(WriteCtx, Value), String> {
    if position.is_some_and(|p| p.outdated || p.head_sha.as_deref().is_some_and(|h| !h.is_empty() && Some(h) != head_sha)) {
        return Err(OLDER_VERSION.to_string());
    }
    let first = format!("Check out {} at the {}'s head to apply it", mr.source_branch, super::super::text::mr_noun(kind));
    let (Some(branch), Some(head), Some(want)) = (t.head_branch(), t.head_sha(), head_sha) else { return Err(first) };
    if head != want {
        return Err(first);
    }
    let on_it = branch == mr.source_branch
        || crate::forge::mrview::checkout::source_remote(t, mr).is_some_and(|r| {
            let upstream = format!("refs/remotes/{r}/{}", mr.source_branch);
            t.locals().iter().any(|l| l.name == branch && l.upstream.as_deref() == Some(upstream.as_str()))
        });
    if !on_it {
        return Err(first);
    }
    if t.diff.as_ref().is_some_and(|d| d.edit.as_ref().is_some_and(|e| e.dirty()) && d.file.path.as_ref() == path) {
        return Err(format!("Save your edits to {path} first"));
    }
    Ok((t.write_target(), json!({ "head": head, "refs": { format!("refs/heads/{branch}"): head } })))
}

impl GitVolt {
    #[allow(clippy::too_many_arguments)]
    pub fn mr_apply_suggestion(&mut self, tab: u32, kind: ForgeKind, mr: ForgeMr, path: String, from: u32, replaced: Vec<String>, suggested: Vec<String>, position: Option<DiffPosition>, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let head = t.forge.details.get(&mr.number).and_then(|d| d.mr.head_sha.clone()).or(mr.head_sha.clone());
        let (ctx, expect) = match apply_gate(t, kind, &mr, head.as_deref(), &path, position.as_ref()) {
            Ok(g) => g,
            Err(why) => return crate::ui::toast::show(why, cx),
        };
        let (api, rt) = (self.api.clone(), self.rt.clone());
        let (read_path, worktree) = (path.clone(), ctx.worktree.clone());
        let repo = core_repo(tab);
        let old = json!({ "kind": "absent" });
        let new = json!({ "kind": "worktree", "worktree": worktree });
        cx.spawn(async move |this, cx| {
            let req = Request::DiffContents { repo, path: read_path, old: serde_json::from_value(old).expect("a side"), new: serde_json::from_value(new).expect("a side"), force: false, old_path: None };
            let read = rt.spawn(async move { api.dispatch(req).await }).await;
            this.update(cx, |v, cx| {
                let cant = format!("Can't read {path} here");
                let Ok(Ok(c)) = read else { return crate::ui::toast::show(cant, cx) };
                let n = &c["new"];
                let (Some(text), Some(base)) = (n["text"].as_str(), n["hash"].as_str()) else { return crate::ui::toast::show(cant, cx) };
                if c["tooLarge"].as_bool() == Some(true) || n["binary"].as_bool() == Some(true) || n["filtered"].as_bool() == Some(true) {
                    return crate::ui::toast::show(cant, cx);
                }
                if let Some(why) = apply_check(text, from, &replaced, &suggested) {
                    return crate::ui::toast::show(why, cx);
                }
                let Some(next) = replace_lines(text, from, replaced.len(), &suggested) else { return crate::ui::toast::show(LINES_CHANGED, cx) };
                let (w, p, b, e) = (ctx.worktree.clone(), path.clone(), base.to_string(), expect.clone());
                let (done, undo_ctx) = (path.clone(), ctx.clone());
                let write = Write::new(&ctx, move |_| call("applySuggestion", json!({ "repo": repo, "worktree": w, "path": p, "text": next, "base": b, "expect": e }))).on_success(move |_, o, cx| {
                    let entry = o["journal"]["undo"]["entry"].as_u64();
                    let actions = entry.map(|_| {
                        let weak = cx.weak_entity();
                        let ctx = undo_ctx.clone();
                        crate::ui::toast::ToastAction {
                            label: "Undo".into(),
                            run: std::rc::Rc::new(move |_: &mut Window, cx: &mut App| {
                                let ctx = ctx.clone();
                                weak.update(cx, |v, cx| v.undo(ctx, crate::ui::arm::Origin::None, cx)).ok();
                            }),
                        }
                    });
                    crate::ui::toast::show_with(format!("Applied the suggestion to {done}"), crate::ui::toast::Options { actions: actions.into_iter().collect(), ..Default::default() }, cx);
                });
                v.write(write, cx);
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn v(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn it_finds_suggestions_and_leaves_other_code() {
        let t = "Maybe:\n\n```suggestion\nnew one\nnew two\n```\n\n```rust\nx\n```\n\n````suggestion:-1+0\n```\n````\n";
        let s = suggestions_of(t);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0], ("suggestion".to_string(), v(&["new one", "new two"])));
        assert_eq!(s[1], ("suggestion:-1+0".to_string(), v(&["```"])));
    }

    #[test]
    fn the_check_says_why() {
        let text = "a\nb\nc\n";
        assert_eq!(apply_check(text, 2, &v(&["b"]), &v(&["B"])), None);
        assert_eq!(apply_check(text, 2, &v(&["x"]), &v(&["B"])), Some(LINES_CHANGED));
        assert_eq!(apply_check("a\nB\nc\n", 2, &v(&["b"]), &v(&["B"])), Some(ALREADY_APPLIED));
        assert_eq!(apply_check(text, 2, &v(&["b"]), &v(&["b"])), Some(ALREADY_APPLIED));
        // Takes the second line off: not applied yet, though the first is what it leaves.
        assert_eq!(apply_check(text, 1, &v(&["a", "b"]), &v(&["a"])), None);
    }

    #[test]
    fn replacing_keeps_the_files_endings() {
        assert_eq!(replace_lines("a\r\nb\r\nc", 2, 1, &v(&["x", "y"])).as_deref(), Some("a\r\nx\r\ny\r\nc"));
        assert_eq!(replace_lines("a\nb", 2, 1, &v(&["x"])).as_deref(), Some("a\nx"));
        assert_eq!(replace_lines("a\nb", 2, 1, &[]).as_deref(), Some("a"));
        assert_eq!(replace_lines("a\n", 3, 1, &v(&["x"])), None);
    }

    #[test]
    fn a_snippets_base_gives_its_new_lines() {
        let pos: DiffPosition = serde_json::from_value(json!({"path": "a", "oldPath": null, "line": 5, "oldLine": null, "snippet": " three\n-gone\n+four\n+five", "startLine": 4, "startOldLine": null, "outdated": false})).unwrap();
        let b = snippet_base(Flavor::Github, Some(&pos)).unwrap();
        assert_eq!(b.span, 1);
        assert_eq!(b.replaced("suggestion"), Some(v(&["four", "five"])));
        assert_eq!(b.start("suggestion"), 4);
        let gl = snippet_base(Flavor::Gitlab, Some(&pos)).unwrap();
        assert_eq!(gl.replaced("suggestion"), Some(v(&["five"])));
        assert_eq!(gl.replaced("suggestion:-9+0"), None);
    }
}
