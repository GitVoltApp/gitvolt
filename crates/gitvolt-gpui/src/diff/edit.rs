//! The working copy, edited in place (`ui/src/diff/workingCopy.ts`): the new side of a WIP diff and File View
//! of a working file take the keyboard. A plain editor on the text field's `Buffer`: caret, selection, typing,
//! undo, save behind the hash it loaded, the dirty mark, and the question on leaving unsaved.

use super::view::{DiffData, DiffState, ViewKind};
use crate::app::GitVolt;
use crate::details::Origin;
use crate::ui::input::{self, Buffer, EditKind};
use crate::ui::popover::{self, Choice, ChoiceRequest};
use crate::write::{Write, WriteCtx, call};
use gitvolt_core::error::GbErrorKind;
use gpui::*;
use serde_json::{Value, json};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

pub struct EditState {
    pub buffer: Buffer,
    /// The loaded bytes' hash: a save goes through only if the file still has it.
    pub base: String,
    /// The text as loaded: what it's dirty against.
    pub loaded: String,
    /// The file ends its lines with CRLF: a save writes them so. As Monaco, one kind per file.
    pub crlf: bool,
    pub old_text: Option<String>,
    pub focus: FocusHandle,
    pub worktree: String,
    /// Each line's start in the buffer.
    pub starts: Vec<usize>,
    /// The column Up and Down keep.
    pub want_col: Option<usize>,
    /// Colours follow typing after a breath.
    pub recolour: u64,
}

fn line_starts(text: &str) -> Vec<usize> {
    let mut out = vec![0];
    out.extend(text.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(i, _)| i + 1));
    out
}

impl EditState {
    pub fn new(text: &str, old_text: Option<String>, base: String, worktree: String, cx: &mut App) -> Self {
        let crlf = text.contains("\r\n") && text.matches("\r\n").count() * 2 >= text.matches('\n').count();
        let mut buffer = Buffer::new(true);
        buffer.set(text);
        let loaded = buffer.text.clone();
        buffer.move_to(0);
        let starts = line_starts(&loaded);
        Self { buffer, base, loaded, crlf, old_text, focus: cx.focus_handle(), worktree, starts, want_col: None, recolour: 0 }
    }

    pub fn dirty(&self) -> bool {
        self.buffer.text != self.loaded
    }

    /// What a save writes.
    pub fn text_to_save(&self) -> String {
        if self.crlf { self.buffer.text.replace('\n', "\r\n") } else { self.buffer.text.clone() }
    }

    /// A buffer offset as its line and column.
    pub fn line_col(&self, at: usize) -> (usize, usize) {
        let n = self.starts.partition_point(|s| *s <= at).saturating_sub(1);
        (n, at - self.starts[n])
    }

    fn line_range(&self, n: usize) -> Range<usize> {
        let start = self.starts[n];
        let end = self.starts.get(n + 1).map_or(self.buffer.text.len(), |s| s - 1);
        start..end
    }

    /// The offset on line `n` nearest column `col`, on a character's boundary.
    fn at_line(&self, n: usize, col: usize) -> usize {
        let r = self.line_range(n);
        let mut at = (r.start + col).min(r.end);
        while !self.buffer.text.is_char_boundary(at) {
            at -= 1;
        }
        at
    }

    /// Reloaded: the text is what's on disk now, the caret stays near.
    pub fn reload(&mut self, text: &str, base: String, old_text: Option<String>) {
        let at = self.buffer.caret();
        let same = {
            let mut b = Buffer::new(true);
            b.set(text);
            b.text == self.buffer.text
        };
        self.base = base;
        self.old_text = old_text;
        self.crlf = text.contains("\r\n") && text.matches("\r\n").count() * 2 >= text.matches('\n').count();
        if same {
            self.loaded = self.buffer.text.clone();
            return;
        }
        self.buffer.set(text);
        self.loaded = self.buffer.text.clone();
        let mut at = at.min(self.buffer.text.len());
        while !self.buffer.text.is_char_boundary(at) {
            at -= 1;
        }
        self.buffer.move_to(at);
        self.starts = line_starts(&self.buffer.text);
    }
}

/// `Worktree file` targets: File View of a WIP file, unstaged or staged, shows its working-tree file. A
/// staged one loads it as its new side, its staged version as the old one.
pub fn sides_for(file: &crate::details::FileChange, view: ViewKind) -> (Value, Value) {
    if view == ViewKind::File
        && let Origin::Wip { worktree, .. } = &file.origin
        && file.new["kind"] != "worktree"
        && file.new["kind"] != "absent"
        && file.status != 'U'
        && file.status != 'X'
    {
        return (file.new.clone(), json!({ "kind": "worktree", "worktree": worktree }));
    }
    (file.old.clone(), file.new.clone())
}

/// A WIP diff whose new side is the working-tree file, as loaded text: binaries, large files, images,
/// deletions and conflicted files stay read-only, and so does a file shown in its clean form.
pub fn is_editable(file: &crate::details::FileChange, new_side: &Value, contents: &Value) -> bool {
    matches!(file.origin, Origin::Wip { .. }) && new_side["kind"] == "worktree" && file.status != 'U' && file.status != 'X' && contents["tooLarge"].as_bool() != Some(true) && contents["image"].as_bool() != Some(true) && contents["new"].is_object() && contents["new"]["binary"].as_bool() != Some(true) && contents["new"]["text"].is_string() && contents["new"]["filtered"].as_bool() != Some(true)
}

impl DiffState {
    /// The file's text changed under the caret: it's aligned and laid out anew, the caret in view.
    fn after_edit(&mut self) {
        let Some(mut edit) = self.edit.take() else { return };
        edit.starts = line_starts(&edit.buffer.text);
        let (encoding, eol_only) = self.data.as_ref().map_or((SharedString::default(), false), |d| (d.encoding.clone(), d.eol_only));
        let data = DiffData::build(edit.old_text.as_deref(), Some(&edit.buffer.text), self.ignore_whitespace, encoding, eol_only);
        self.data = Some(Arc::new(data));
        self.fold_ends = Rc::new(Vec::new());
        self.edit = Some(edit);
        self.relayout();
        self.show_caret();
    }

    /// The buffer's selection, as the painter draws it; the caret's row in view.
    pub fn show_caret(&mut self) {
        self.sync_selection();
        self.reveal_caret = true;
        self.layout_again();
    }

    /// The row a line of the new side is on, opening what folds it.
    fn row_of_line(&mut self, n: usize) -> Option<usize> {
        let mut found = self.rows.iter().position(|r| r.new == Some(n as u32));
        if found.is_none() {
            // Hunk mode folded it, or File View did.
            let region = self.rows.iter().find_map(|r| match r.kind {
                super::model::RowKind::Hidden { count, region } if r.new.is_some_and(|s| (s as usize) <= n && n < s as usize + count as usize) => Some(region),
                _ => None,
            });
            if let Some(region) = region {
                self.expanded.insert(region);
            } else if !self.folded.is_empty() {
                self.folded.clear();
            } else {
                return None;
            }
            self.relayout();
            found = self.rows.iter().position(|r| r.new == Some(n as u32));
        }
        found
    }

    fn pos_of(&mut self, at: usize) -> Option<super::view::TextPos> {
        let (n, col) = self.edit.as_ref()?.line_col(at);
        let row = self.row_of_line(n)?;
        let line = self.data.as_ref()?.new.lines.get(n)?;
        Some(super::view::TextPos { row, byte: line.display(col as u32) as usize })
    }

    pub fn sync_selection(&mut self) {
        let Some(edit) = self.edit.as_ref() else { return };
        let (caret, other) = if edit.buffer.reversed { (edit.buffer.sel.start, edit.buffer.sel.end) } else { (edit.buffer.sel.end, edit.buffer.sel.start) };
        let (Some(head), Some(anchor)) = (self.pos_of(caret), self.pos_of(other)) else { return };
        self.caret = Some(head);
        self.cursor = head.row;
        self.selection = (head != anchor).then_some(super::view::Selection { anchor, head, old_side: false });
    }

    /// The buffer offset of a place in the shown text; `None` on what isn't the new side.
    pub fn offset_of(&self, pos: super::view::TextPos) -> Option<usize> {
        let edit = self.edit.as_ref()?;
        let n = self.rows.get(pos.row)?.new? as usize;
        let line = self.data.as_ref()?.new.lines.get(n)?;
        let col = line.raw(pos.byte as u32) as usize;
        Some(edit.at_line(n, col))
    }
}

impl GitVolt {
    fn with_edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut EditState) -> bool) {
        let Some(diff) = self.tabs.get_mut(self.active).and_then(|t| t.diff.as_mut()) else { return };
        let Some(edit) = diff.edit.as_mut() else { return };
        let (before, changed) = (edit.buffer.text.len(), f(edit));
        let _ = before;
        if changed {
            diff.after_edit();
            self.colour_after_edit(cx);
        } else {
            diff.show_caret();
        }
        cx.notify();
    }

    fn move_caret(&mut self, select: bool, cx: &mut Context<Self>, to: impl FnOnce(&EditState) -> usize) {
        self.with_edit(cx, |e| {
            let at = to(e);
            if select {
                e.buffer.select_to(at);
            } else {
                e.buffer.move_to(at);
            }
            e.want_col = None;
            false
        });
    }

    fn vertical(&mut self, down: bool, select: bool, cx: &mut Context<Self>) {
        self.with_edit(cx, |e| {
            let (n, col) = e.line_col(e.buffer.caret());
            let col = *e.want_col.get_or_insert(col);
            let want = e.want_col;
            let at = if down {
                if n + 1 < e.starts.len() { e.at_line(n + 1, col) } else { e.buffer.text.len() }
            } else if n > 0 {
                e.at_line(n - 1, col)
            } else {
                0
            };
            if select {
                e.buffer.select_to(at);
            } else {
                e.buffer.move_to(at);
            }
            e.want_col = want;
            false
        });
    }

    fn insert(&mut self, text: &str, kind: EditKind, cx: &mut Context<Self>) {
        self.with_edit(cx, |e| {
            e.buffer.replace(None, text, kind);
            e.want_col = None;
            true
        });
    }

    fn delete(&mut self, cx: &mut Context<Self>, to: impl FnOnce(&Buffer, usize) -> usize) {
        self.with_edit(cx, |e| {
            let range = if e.buffer.sel.is_empty() {
                let caret = e.buffer.caret();
                let other = to(&e.buffer, caret);
                caret.min(other)..caret.max(other)
            } else {
                e.buffer.sel.clone()
            };
            if range.is_empty() {
                return false;
            }
            e.buffer.replace(Some(range), "", EditKind::Deleting);
            e.want_col = None;
            true
        });
    }

    /// Typing done: its colours, once it pauses.
    fn colour_after_edit(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let repo = tab.id;
        let Some(diff) = tab.diff.as_mut() else { return };
        let Some(edit) = diff.edit.as_mut() else { return };
        edit.recolour += 1;
        let at = edit.recolour;
        let timer = cx.background_executor().timer(std::time::Duration::from_millis(120));
        cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |this, cx| {
                let Some(diff) = this.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.diff.as_mut()) else { return };
                let (Some(edit), Some(data)) = (diff.edit.as_ref(), diff.data.clone()) else { return };
                if edit.recolour != at {
                    return;
                }
                let texts = super::view::Texts::new_side(diff.file.path.to_string(), edit.buffer.text.clone());
                diff.stop_colours.store(true, std::sync::atomic::Ordering::Relaxed);
                diff.stop_colours = Arc::default();
                let (stop, generation) = (diff.stop_colours.clone(), diff.generation);
                this.color(repo, generation, data, texts, stop, cx);
            })
            .ok();
        })
        .detach();
    }

    /// A press in the text: the caret goes there, and the keyboard's the editor's.
    pub fn edit_press(&mut self, at: super::view::TextPos, extend: bool, unit: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(diff) = self.tabs.get_mut(self.active).and_then(|t| t.diff.as_mut()) else { return };
        let Some(offset) = diff.offset_of(at) else { return };
        let Some(edit) = diff.edit.as_mut() else { return };
        window.focus(&edit.focus, cx);
        match unit {
            2 => {
                edit.buffer.sel = edit.buffer.word_at(offset);
                edit.buffer.reversed = false;
            }
            n if n >= 3 => {
                let (line, _) = edit.line_col(offset);
                edit.buffer.sel = edit.line_range(line);
                edit.buffer.reversed = false;
            }
            _ if extend => edit.buffer.select_to(offset),
            _ => edit.buffer.move_to(offset),
        }
        edit.want_col = None;
        diff.sync_selection();
        cx.notify();
    }

    /// The press drags: the selection's head follows.
    pub fn edit_drag(&mut self, at: super::view::TextPos, cx: &mut Context<Self>) {
        let Some(diff) = self.tabs.get_mut(self.active).and_then(|t| t.diff.as_mut()) else { return };
        let Some(offset) = diff.offset_of(at) else { return };
        let Some(edit) = diff.edit.as_mut() else { return };
        edit.buffer.select_to(offset);
        diff.sync_selection();
        cx.notify();
    }

    /// Ctrl+S: behind the loaded base. A `Stale` save keeps the text and asks.
    pub fn save_working_copy(&mut self, then: Option<crate::write::After>, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let repo = tab.id;
        let Some(diff) = tab.diff.as_ref() else { return };
        let Some(edit) = diff.edit.as_ref() else { return };
        // Nothing unsaved: no write, so no new mtime to wake a watcher.
        if !edit.dirty() {
            if let Some(then) = then {
                then(self, cx);
            }
            return;
        }
        let (path, worktree, text, base) = (diff.file.path.to_string(), edit.worktree.clone(), edit.text_to_save(), edit.base.clone());
        self.send_save(repo, worktree, path, text, base, then, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn send_save(&mut self, repo: u32, worktree: String, path: String, text: String, base: String, then: Option<crate::write::After>, cx: &mut Context<Self>) {
        let ctx = WriteCtx { repo, worktree: worktree.clone() };
        let (saved, handled_path, saved_path, send_path, send_text) = (text.clone(), path.clone(), path.clone(), path.clone(), text.clone());
        let handled_then = then.clone();
        let (retry_worktree, retry_text) = (worktree.clone(), text);
        let w = Write::new(&ctx, move |_| call("writeWorktreeFile", json!({ "repo": repo, "worktree": worktree, "path": send_path, "text": send_text, "base": base })))
            .on_success(move |this, out, cx| {
                let hash = out["hash"].as_str().unwrap_or_default().to_string();
                if let Some(diff) = this.tabs.iter_mut().find(|t| t.id == repo).and_then(|t| t.diff.as_mut()).filter(|d| d.file.path.as_ref() == saved_path)
                    && let Some(edit) = diff.edit.as_mut()
                {
                    edit.base = hash;
                    // Typed after the text was read: those edits stay, still dirty.
                    let mut written = Buffer::new(true);
                    written.set(&saved);
                    edit.loaded = written.text;
                    diff.keep_scroll();
                    diff.generation += 1;
                }
                this.read_contents(false, cx);
                this.reread_wip(repo, cx);
                if let Some(then) = &then {
                    then(this, cx);
                }
                cx.notify();
            })
            .handle(move |this, err, cx| {
                if err.kind != GbErrorKind::Stale {
                    return false;
                }
                this.ask_stale(repo, retry_worktree.clone(), handled_path.clone(), retry_text.clone(), err.message.clone(), handled_then.clone(), cx);
                true
            });
        self.write(w, cx);
    }

    /// The file changed on disk since it loaded: Reload (edits lost) or Overwrite with this text.
    #[allow(clippy::too_many_arguments)]
    fn ask_stale(&mut self, repo: u32, worktree: String, path: String, text: String, title: String, then: Option<crate::write::After>, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        let req = ChoiceRequest {
            title: title.into(),
            body: Some("Reload it (your edits are lost), or overwrite it with your text.".into()),
            note: None,
            choices: vec![Choice { id: "reload".into(), label: "Reload".into(), primary: true, ..Default::default() }, Choice { id: "overwrite".into(), label: "Overwrite".into(), danger: true, ..Default::default() }],
        };
        self.with_window(cx, move |window, cx| {
            popover::ask_choice(
                req,
                None,
                move |pick, _, cx| {
                    this.update(cx, |v, cx| match pick.as_deref() {
                        Some("reload") => {
                            v.discard_edits();
                            v.with_diff(cx, |d| d.generation += 1);
                            v.read_contents(false, cx);
                        }
                        Some("overwrite") => v.overwrite(repo, worktree.clone(), path.clone(), text.clone(), then.clone(), cx),
                        _ => {}
                    })
                    .ok();
                },
                window,
                cx,
            );
        });
    }

    /// Reads the disk's hash now, then saves over it.
    #[allow(clippy::too_many_arguments)]
    fn overwrite(&mut self, repo: u32, worktree: String, path: String, text: String, then: Option<crate::write::After>, cx: &mut Context<Self>) {
        if cfg!(test) {
            self.sent.borrow_mut().push(call("overwrite", json!({ "path": path })));
            return;
        }
        let api = self.api.clone();
        let (w, p) = (worktree.clone(), path.clone());
        let body = call("diffContents", json!({ "repo": repo, "path": p, "old": { "kind": "absent" }, "new": { "kind": "worktree", "worktree": w }, "force": true }));
        let read = self.rt.spawn(async move { api.dispatch(crate::write::request_of(body)?).await });
        cx.spawn(async move |this, cx| {
            let now = read.await;
            this.update(cx, |this, cx| {
                let hash = match now {
                    Ok(Ok(v)) => v["new"]["hash"].as_str().map(str::to_string),
                    _ => None,
                };
                match hash {
                    Some(h) => this.send_save(repo, worktree, path.clone(), text, h, then, cx),
                    None => this.show_toast(format!("{path} was deleted on disk; reload to start over"), cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Unsaved typing goes.
    pub fn discard_edits(&mut self) {
        if let Some(diff) = self.tabs.get_mut(self.active).and_then(|t| t.diff.as_mut())
            && let Some(edit) = diff.edit.as_mut()
        {
            let text = edit.loaded.clone();
            edit.buffer.set(&text);
            edit.buffer.move_to(0);
            edit.starts = line_starts(&text);
            diff.after_edit();
        }
    }

    /// Runs `f` with the window, once what's running now is done.
    pub fn with_window(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut Window, &mut App) + 'static) {
        let Some(window) = self.window else { return };
        cx.defer(move |cx| {
            window.update(cx, |_, window, cx| f(window, cx)).ok();
        });
    }

    /// Leaving the file with unsaved edits asks [Save] [Discard edits] [Cancel]; `go` runs once it's settled.
    /// Whether it asked: the caller stops there.
    pub fn ask_unsaved(&mut self, go: impl Fn(&mut Self, &mut Context<Self>) + 'static, cx: &mut Context<Self>) -> bool {
        let Some(diff) = self.tabs.get(self.active).and_then(|t| t.diff.as_ref()) else { return false };
        if !diff.editing_dirty() {
            return false;
        }
        let path = diff.file.path.to_string();
        let this = cx.weak_entity();
        let go: crate::write::After = Rc::new(go);
        let req = ChoiceRequest { title: format!("Save your changes to {path}?").into(), body: None, note: None, choices: vec![Choice { id: "save".into(), label: "Save".into(), primary: true, ..Default::default() }, Choice { id: "discard".into(), label: "Discard edits".into(), danger: true, ..Default::default() }] };
        self.with_window(cx, move |window, cx| {
            popover::ask_choice(
                req,
                None,
                move |pick, _, cx| {
                    let go = go.clone();
                    this.update(cx, |v, cx| match pick.as_deref() {
                        Some("save") => v.save_working_copy(Some(go), cx),
                        Some("discard") => {
                            v.discard_edits();
                            go(v, cx);
                        }
                        _ => {}
                    })
                    .ok();
                },
                window,
                cx,
            );
        });
        true
    }

    /// Tab: as the line's indent is made, a tab or four spaces.
    pub fn edit_tab(&mut self, cx: &mut Context<Self>) {
        let tabs = self.tabs.get(self.active).and_then(|t| t.diff.as_ref()).and_then(|d| d.edit.as_ref()).is_some_and(|e| e.buffer.text.lines().any(|l| l.starts_with('\t')));
        self.insert(if tabs { "\t" } else { "    " }, EditKind::Other, cx);
    }

    /// The editor's key bindings: the text field's, while its keyboard is on the diff's text.
    pub fn edit_keys_static(el: Stateful<Div>, cx: &mut Context<GitVolt>) -> Stateful<Div> {
        use input::*;
        el.key_context("TextInput")
            .on_action(cx.listener(|this, _: &Backspace, _, cx| this.delete(cx, Buffer::prev_grapheme)))
            .on_action(cx.listener(|this, _: &Delete, _, cx| this.delete(cx, Buffer::next_grapheme)))
            .on_action(cx.listener(|this, _: &DeleteWordLeft, _, cx| this.delete(cx, Buffer::word_left)))
            .on_action(cx.listener(|this, _: &DeleteWordRight, _, cx| this.delete(cx, Buffer::word_right)))
            .on_action(cx.listener(|this, _: &DeleteToStart, _, cx| this.delete(cx, |b, at| b.line_at(at).start)))
            .on_action(cx.listener(|this, _: &DeleteToEnd, _, cx| this.delete(cx, |b, at| b.line_at(at).end)))
            .on_action(cx.listener(|this, _: &Left, _, cx| {
                this.move_caret(false, cx, |e| if e.buffer.sel.is_empty() { e.buffer.prev_grapheme(e.buffer.caret()) } else { e.buffer.sel.start });
            }))
            .on_action(cx.listener(|this, _: &Right, _, cx| {
                this.move_caret(false, cx, |e| if e.buffer.sel.is_empty() { e.buffer.next_grapheme(e.buffer.caret()) } else { e.buffer.sel.end });
            }))
            .on_action(cx.listener(|this, _: &Up, _, cx| this.vertical(false, false, cx)))
            .on_action(cx.listener(|this, _: &Down, _, cx| this.vertical(true, false, cx)))
            .on_action(cx.listener(|this, _: &SelectUp, _, cx| this.vertical(false, true, cx)))
            .on_action(cx.listener(|this, _: &SelectDown, _, cx| this.vertical(true, true, cx)))
            .on_action(cx.listener(|this, _: &WordLeft, _, cx| this.move_caret(false, cx, |e| e.buffer.word_left(e.buffer.caret()))))
            .on_action(cx.listener(|this, _: &WordRight, _, cx| this.move_caret(false, cx, |e| e.buffer.word_right(e.buffer.caret()))))
            .on_action(cx.listener(|this, _: &SelectLeft, _, cx| this.move_caret(true, cx, |e| e.buffer.prev_grapheme(e.buffer.caret()))))
            .on_action(cx.listener(|this, _: &SelectRight, _, cx| this.move_caret(true, cx, |e| e.buffer.next_grapheme(e.buffer.caret()))))
            .on_action(cx.listener(|this, _: &SelectWordLeft, _, cx| this.move_caret(true, cx, |e| e.buffer.word_left(e.buffer.caret()))))
            .on_action(cx.listener(|this, _: &SelectWordRight, _, cx| this.move_caret(true, cx, |e| e.buffer.word_right(e.buffer.caret()))))
            .on_action(cx.listener(|this, _: &input::SelectAll, _, cx| {
                this.with_edit(cx, |e| {
                    e.buffer.select_all();
                    false
                });
            }))
            .on_action(cx.listener(|this, _: &Home, _, cx| this.move_caret(false, cx, |e| e.buffer.line_at(e.buffer.caret()).start)))
            .on_action(cx.listener(|this, _: &End, _, cx| this.move_caret(false, cx, |e| e.buffer.line_at(e.buffer.caret()).end)))
            .on_action(cx.listener(|this, _: &SelectHome, _, cx| this.move_caret(true, cx, |e| e.buffer.line_at(e.buffer.caret()).start)))
            .on_action(cx.listener(|this, _: &SelectEnd, _, cx| this.move_caret(true, cx, |e| e.buffer.line_at(e.buffer.caret()).end)))
            .on_action(cx.listener(|this, _: &DocStart, _, cx| this.move_caret(false, cx, |_| 0)))
            .on_action(cx.listener(|this, _: &DocEnd, _, cx| this.move_caret(false, cx, |e| e.buffer.text.len())))
            .on_action(cx.listener(|this, _: &SelectDocStart, _, cx| this.move_caret(true, cx, |_| 0)))
            .on_action(cx.listener(|this, _: &SelectDocEnd, _, cx| this.move_caret(true, cx, |e| e.buffer.text.len())))
            .on_action(cx.listener(|this, _: &Paste, _, cx| {
                if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    this.insert(&text, EditKind::Other, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &input::Copy, _, cx| this.copy_selection(cx)))
            .on_action(cx.listener(|this, _: &Cut, _, cx| {
                this.copy_selection(cx);
                this.insert("", EditKind::Other, cx);
            }))
            .on_action(cx.listener(|this, _: &Undo, _, cx| this.with_edit(cx, |e| e.buffer.undo())))
            .on_action(cx.listener(|this, _: &Redo, _, cx| this.with_edit(cx, |e| e.buffer.redo())))
            .on_action(cx.listener(|this, _: &Enter, _, cx| this.insert("\n", EditKind::Other, cx)))
            .on_action(cx.listener(|this, _: &ShiftEnter, _, cx| this.insert("\n", EditKind::Other, cx)))
            // Monaco's "insert line below".
            .on_action(cx.listener(|this, _: &CtrlEnter, _, cx| {
                this.with_edit(cx, |e| {
                    let end = e.buffer.line_at(e.buffer.caret()).end;
                    e.buffer.move_to(end);
                    e.buffer.replace(None, "\n", EditKind::Other);
                    true
                });
            }))
            .on_action(cx.listener(|_, _: &input::Escape, _, cx| cx.propagate()))
    }
}

impl EntityInputHandler for GitVolt {
    fn text_for_range(&mut self, range: Range<usize>, actual: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let b = &self.tabs.get(self.active)?.diff.as_ref()?.edit.as_ref()?.buffer;
        let r = b.range_from_utf16(&range);
        actual.replace(b.range_to_utf16(&r));
        Some(b.text[r].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let b = &self.tabs.get(self.active)?.diff.as_ref()?.edit.as_ref()?.buffer;
        Some(UTF16Selection { range: b.range_to_utf16(&b.sel), reversed: b.reversed })
    }

    fn set_selected_text_range(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) {
        self.with_edit(cx, |e| {
            e.buffer.sel = e.buffer.range_from_utf16(&range);
            e.buffer.reversed = false;
            false
        });
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.tabs.get(self.active)?.diff.as_ref()?.edit.as_ref()?.buffer.text.encode_utf16().count())
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let b = &self.tabs.get(self.active)?.diff.as_ref()?.edit.as_ref()?.buffer;
        b.marked.as_ref().map(|r| b.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.with_edit(cx, |e| {
            e.buffer.marked = None;
            false
        });
    }

    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        // A character with the control or platform key isn't text; the platform may still offer it.
        if text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
            return;
        }
        self.with_edit(cx, |e| {
            let range = range.map(|r| e.buffer.range_from_utf16(&r));
            e.buffer.replace(range, text, EditKind::Typing);
            e.want_col = None;
            true
        });
    }

    fn replace_and_mark_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, selected: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        self.with_edit(cx, |e| {
            let range = range.map(|r| e.buffer.range_from_utf16(&r));
            let map = |u: usize| {
                let (mut u8s, mut u16s) = (0, 0);
                for c in text.chars() {
                    if u16s >= u {
                        break;
                    }
                    u16s += c.len_utf16();
                    u8s += c.len_utf8();
                }
                u8s
            };
            e.buffer.replace_marked(range, text, selected.map(|s| map(s.start)..map(s.end)));
            true
        });
    }

    fn bounds_for_range(&mut self, _: Range<usize>, bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let diff = self.tabs.get(self.active)?.diff.as_ref()?;
        let at = diff.caret?;
        let y = diff.layout.offsets.get(at.row).copied().unwrap_or(0.) - diff.scroll;
        Some(Bounds::new(bounds.origin + point(px(diff.gutter_w() + at.byte as f32 * 7.8 - diff.hscroll), px(y)), size(px(1.), px(super::view::LINE_H))))
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::testing::Fixture;
    use std::prelude::v1::test;

    #[test]
    fn a_staged_files_file_view_shows_the_working_tree_over_its_staged_version() {
        let staged = crate::details::FileChange { path: "a".into(), old_path: None, status: 'M', additions: None, deletions: None, old: json!({ "kind": "head" }), new: json!({ "kind": "index" }), conflict: None, origin: Origin::Wip { worktree: "/w".into(), staged: true } };
        let (old, new) = sides_for(&staged, ViewKind::File);
        assert_eq!((old["kind"].as_str(), new["kind"].as_str()), (Some("index"), Some("worktree")));
        assert_eq!(sides_for(&staged, ViewKind::Diff).1["kind"], "index");
        let conflicted = crate::details::FileChange { status: 'U', ..staged };
        assert_eq!(sides_for(&conflicted, ViewKind::File).1["kind"], "index");
    }

    #[test]
    fn only_a_plain_working_file_can_be_edited() {
        let file = crate::details::FileChange { path: "a".into(), old_path: None, status: 'M', additions: None, deletions: None, old: Value::Null, new: Value::Null, conflict: None, origin: Origin::Wip { worktree: "/w".into(), staged: false } };
        let wt = json!({ "kind": "worktree" });
        let text = json!({ "new": { "text": "x", "binary": false } });
        assert!(is_editable(&file, &wt, &text));
        assert!(!is_editable(&file, &json!({ "kind": "index" }), &text));
        assert!(!is_editable(&file, &wt, &json!({ "new": { "text": null, "binary": true } })));
        assert!(!is_editable(&file, &wt, &json!({ "new": { "text": "x", "filtered": true } })));
        assert!(!is_editable(&file, &wt, &json!({ "tooLarge": true, "new": { "text": "x" } })));
        assert!(!is_editable(&crate::details::FileChange { status: 'U', ..file.clone() }, &wt, &text));
        assert!(!is_editable(&crate::details::FileChange { origin: Origin::Commit { id: "c".into() }, ..file }, &wt, &text));
    }

    #[gpui::test]
    fn the_buffer_keeps_a_crlf_files_ends_and_knows_its_lines(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let e = EditState::new("one\r\ntwo\r\n", None, "h".into(), "/w".into(), cx);
            assert_eq!(e.buffer.text, "one\ntwo\n");
            assert!(e.crlf);
            assert_eq!(e.text_to_save(), "one\r\ntwo\r\n");
            assert!(!e.dirty());
            assert_eq!(e.line_col(5), (1, 1));
            assert_eq!(e.at_line(1, 99), 7, "past the line's end, its end");
        });
    }

    fn real_file(text: &str) -> crate::details::FileChange {
        let _ = text;
        crate::details::FileChange { path: "a".into(), old_path: None, status: 'M', additions: None, deletions: None, old: Value::Null, new: json!({ "kind": "worktree", "worktree": "/w" }), conflict: None, origin: Origin::Wip { worktree: "/w".into(), staged: false } }
    }

    #[gpui::test]
    fn typing_aligns_the_diff_again_and_marks_it_dirty(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut d = DiffState::new(real_file("x"));
            d.view = ViewKind::Diff;
            d.set_data(Arc::new(DiffData::build(Some("a\nb\n"), Some("a\nb\n"), false, SharedString::default(), false)));
            let mut e = EditState::new("a\nb\n", Some("a\nb\n".into()), "h".into(), "/w".into(), cx);
            e.buffer.move_to(2);
            e.buffer.replace(None, "new\n", EditKind::Other);
            d.edit = Some(e);
            d.after_edit();
            assert!(d.editing_dirty());
            assert_eq!(d.changes.len(), 1, "the new line is a change");
            assert_eq!(d.data.as_ref().unwrap().new.lines[1].text.as_ref(), "new");
            // The caret's on the line after it.
            assert_eq!(d.caret.map(|c| d.rows[c.row].new), Some(Some(2)));
            // A press on a line's text maps to its offset.
            let row = d.rows.iter().position(|r| r.new == Some(1)).unwrap();
            assert_eq!(d.offset_of(super::super::view::TextPos { row, byte: 1 }), Some(3));
        });
    }

    #[tokio::test]
    async fn a_save_writes_behind_its_base_and_a_stale_one_refuses() {
        let f = Fixture::new().await;
        f.write("a.txt", "edited\n");
        let contents = f.ok(call("diffContents", json!({ "repo": f.repo, "path": "a.txt", "old": { "kind": "absent" }, "new": { "kind": "worktree", "worktree": f.worktree }, "force": false }))).await;
        let base = contents["new"]["hash"].as_str().unwrap().to_string();
        let save = |text: &str, base: &str| call("writeWorktreeFile", json!({ "repo": f.repo, "worktree": f.worktree, "path": "a.txt", "text": text, "base": base }));
        let r = f.ok(save("saved\n", &base)).await;
        assert_eq!(f.read("a.txt"), "saved\n");
        assert_ne!(r["outcome"]["hash"].as_str().unwrap(), base);
        // The file moved on since: refused, and the file stays.
        let e = f.send(save("late\n", &base)).await.unwrap_err();
        assert_eq!(e.kind, GbErrorKind::Stale);
        assert_eq!(f.read("a.txt"), "saved\n");
    }
}
