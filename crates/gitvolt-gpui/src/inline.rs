//! The inline editor in a graph row's Branch/Tag cell (`graph/rowEditor.ts`, `branches/BranchNameInput.tsx`,
//! `tags/TagNameInput.tsx`): Create branch here, the toolbar's Branch and Create tag here. It floats over the chips, which
//! stay, so nothing moves; its hint or error floats under it. Where there's no row to put it in (the commit isn't loaded,
//! a file's on top, Branch/Tag is hidden), the dialog in `refs.rs` instead.

use crate::app::GitVolt;
use crate::model::{CHIP_H, Col};
use crate::refs::{branch_name_error, tag_name_error};
use crate::theme;
use crate::ui::input::{InputEvent, TextInput};
use crate::write::WriteCtx;
use gpui::prelude::*;
use gpui::*;

/// An inline editor's narrowest: room for a name even in a narrow Branch/Tag column.
const EDITOR_MIN_W: f32 = 200.;
const NOTE_MAX_W: f32 = 420.;

#[derive(Clone, Debug, PartialEq)]
pub enum EditorKind {
    /// Where the branch starts; the ref a remote-tracking start sets as its upstream.
    Branch { start_ref: Option<String> },
    Tag { annotated: bool, on_message: bool },
}

pub struct RowEditor {
    pub row_id: String,
    pub kind: EditorKind,
    pub ctx: WriteCtx,
    pub name: Entity<TextInput>,
    pub message: Option<Entity<TextInput>>,
    pub taken: Vec<String>,
}

/// `BRANCH_INPUT_HINT`.
pub fn branch_hint() -> String {
    format!("Enter: create and check out · {}: create only · Esc: cancel", crate::commands::display_chord("Mod+Enter"))
}

pub const TAG_INPUT_HINT: &str = "Enter: create the tag · Esc: cancel";
pub const TAG_NAME_NEXT_HINT: &str = "Enter: write its message · Esc: cancel";
pub const TAG_MESSAGE_HINT: &str = "Enter: create the tag · Shift+Enter: new line · Esc: cancel";

impl RowEditor {
    /// Why the name can't be used; an empty one says nothing.
    pub fn error(&self, cx: &App) -> Option<String> {
        let v = self.name.read(cx).text();
        if v.is_empty() {
            return None;
        }
        match self.kind {
            EditorKind::Branch { .. } => branch_name_error(v).or_else(|| self.taken.iter().any(|t| t == v).then(|| format!("A branch named {v} already exists"))),
            EditorKind::Tag { .. } => tag_name_error(v).or_else(|| self.taken.iter().any(|t| t == v).then(|| format!("A tag named {v} already exists"))),
        }
    }
}

impl GitVolt {
    /// Whether the graph can show an editor on this commit: else it's the dialog.
    fn row_for_editor(&self, sha: &str) -> bool {
        self.tabs.get(self.active).is_some_and(|t| t.diff.is_none() && t.history.is_none() && !t.hidden_columns.contains(&Col::Labels) && t.graph.as_ref().is_some_and(|g| g.by_sha.contains_key(sha)))
    }

    fn open_editor(&mut self, sha: String, kind: EditorKind, placeholder: &'static str, taken: Vec<String>, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let ctx = tab.write_target();
        if let Some(i) = tab.graph.as_ref().and_then(|g| g.by_sha.get(&sha).copied())
            && tab.selected != Some(i)
        {
            self.select(i, cx);
        }
        let id = self.tabs[self.active].id;
        let name = cx.new(|cx| TextInput::new(cx).placeholder(placeholder).claim_escape());
        cx.subscribe(&name, move |this, _, e: &InputEvent, cx| this.editor_event(id, e, false, cx)).detach();
        self.focus_next_frame = Some(name.read(cx).focus.clone());
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.row_editor = Some(RowEditor { row_id: sha, kind, ctx, name, message: None, taken });
        }
        cx.notify();
    }

    /// Create branch here and the toolbar's Branch. False: no row to put it in.
    pub fn open_branch_editor(&mut self, sha: &str, start_ref: Option<String>, cx: &mut Context<Self>) -> bool {
        if !self.row_for_editor(sha) {
            return false;
        }
        let taken = self.tabs[self.active].locals().into_iter().map(|b| b.name).collect();
        self.open_editor(sha.to_string(), EditorKind::Branch { start_ref }, "enter branch name", taken, cx);
        true
    }

    /// Create tag here. False: no row to put it in.
    pub fn open_tag_editor(&mut self, sha: &str, annotated: bool, cx: &mut Context<Self>) -> bool {
        if !self.row_for_editor(sha) {
            return false;
        }
        let taken = self.tabs[self.active].sidebar["tags"].as_array().map_or(Vec::new(), |t| t.iter().filter_map(|x| x["name"].as_str().map(str::to_string)).collect());
        self.open_editor(sha.to_string(), EditorKind::Tag { annotated, on_message: false }, if annotated { "enter annotated tag name" } else { "enter tag name" }, taken, cx);
        true
    }

    pub fn close_editor(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active)
            && tab.row_editor.take().is_some()
        {
            self.focus_next_frame = Some(self.focus.clone());
            cx.notify();
        }
    }

    fn editor_event(&mut self, repo: u32, e: &InputEvent, in_message: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) else { return };
        let Some(ed) = tab.row_editor.as_ref() else { return };
        match e {
            InputEvent::Changed => cx.notify(),
            InputEvent::Escape => self.close_editor(cx),
            // Leaving it empty cancels; a typed name stays to come back to.
            InputEvent::Blurred if !in_message => {
                let on_message = matches!(ed.kind, EditorKind::Tag { on_message: true, .. });
                if ed.name.read(cx).text().is_empty() && !on_message {
                    self.close_editor(cx);
                } else {
                    cx.notify();
                }
            }
            InputEvent::Focused | InputEvent::Blurred => cx.notify(),
            InputEvent::Submit { ctrl, shift } => {
                if in_message {
                    if !*shift {
                        self.submit_tag_message(cx);
                    }
                } else {
                    self.submit_editor(*ctrl, cx);
                }
            }
            _ => {}
        }
    }

    fn submit_editor(&mut self, ctrl: bool, cx: &mut Context<Self>) {
        let Some(ed) = self.tabs.get(self.active).and_then(|t| t.row_editor.as_ref()) else { return };
        let name = ed.name.read(cx).text().to_string();
        if name.is_empty() || ed.error(cx).is_some() {
            return;
        }
        let (ctx, sha, kind) = (ed.ctx.clone(), ed.row_id.clone(), ed.kind.clone());
        match kind {
            EditorKind::Branch { start_ref } => {
                self.close_editor(cx);
                self.create_branch(ctx, sha, start_ref, name, !ctrl, cx);
            }
            EditorKind::Tag { annotated: false, .. } => {
                self.close_editor(cx);
                self.create_tag(ctx, sha, name, None, cx);
            }
            EditorKind::Tag { annotated: true, .. } => {
                let repo = ctx.repo;
                let message = cx.new(|cx| TextInput::multiline(cx).placeholder("tag message").claim_escape().enter_submits());
                cx.subscribe(&message, move |this, _, e: &InputEvent, cx| this.editor_event(repo, e, true, cx)).detach();
                self.focus_next_frame = Some(message.read(cx).focus.clone());
                if let Some(ed) = self.tabs.get_mut(self.active).and_then(|t| t.row_editor.as_mut()) {
                    ed.kind = EditorKind::Tag { annotated: true, on_message: true };
                    ed.message = Some(message);
                }
                cx.notify();
            }
        }
    }

    fn submit_tag_message(&mut self, cx: &mut Context<Self>) {
        let Some(ed) = self.tabs.get(self.active).and_then(|t| t.row_editor.as_ref()) else { return };
        let Some(text) = ed.message.as_ref().map(|m| m.read(cx).text().to_string()).filter(|m| !m.trim().is_empty()) else { return };
        let (ctx, sha, name) = (ed.ctx.clone(), ed.row_id.clone(), ed.name.read(cx).text().to_string());
        self.close_editor(cx);
        self.create_tag(ctx, sha, name, Some(text), cx);
    }
}

/// The hint or the error, floated under the field.
fn note(text: String, error: bool) -> Div {
    div().max_w(px(NOTE_MAX_W)).px(px(6.)).py(px(2.)).rounded(px(3.)).text_size(px(11.)).bg(theme::panel_bg1()).shadow(crate::ui::shadow(theme::shadow_popup(), 1., 4.)).text_color(if error { theme::red() } else { theme::text_normal() }).child(text)
}

/// Over the row's chips: in its cell, floated above the rows.
pub fn editor_element(ed: &RowEditor, labels_w: f32, window: &Window, cx: &App) -> AnyElement {
    let width = EDITOR_MIN_W.max(labels_w - 8.);
    let error = ed.error(cx);
    let focused = ed.name.read(cx).is_focused(window);
    let border = if error.is_some() { theme::red() } else if focused { theme::blue() } else { theme::section_border() };
    let on_message = matches!(ed.kind, EditorKind::Tag { on_message: true, .. });
    let hint = match ed.kind {
        EditorKind::Branch { .. } => branch_hint(),
        EditorKind::Tag { on_message: true, .. } => TAG_MESSAGE_HINT.to_string(),
        EditorKind::Tag { annotated: true, .. } => TAG_NAME_NEXT_HINT.to_string(),
        EditorKind::Tag { .. } => TAG_INPUT_HINT.to_string(),
    };
    let shown_note = match error {
        Some(e) => Some(note(e, true)),
        None => (focused || on_message).then(|| note(hint, false)),
    };
    let below = match ed.message.as_ref().filter(|_| on_message) {
        Some(m) => Some(div().flex().flex_col().gap(px(3.)).w(px(width.max(320.))).child(div().h(px(62.)).px(px(6.)).py(px(4.)).bg(theme::app_bg()).border_1().border_color(theme::blue()).rounded(px(3.)).shadow(crate::ui::shadow(theme::shadow_popup(), 1., 4.)).text_color(theme::text_selected()).child(m.clone())).children(shown_note)),
        None => shown_note.map(|n| div().flex().w(px(NOTE_MAX_W)).items_start().child(n.flex_none())),
    };
    deferred(
        div()
            .id("row-editor")
            .absolute()
            .left(px(4.))
            .top_0()
            .bottom_0()
            .w(px(width))
            .flex()
            .items_center()
            .occlude()
            .cursor_default()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .child(div().relative().flex().flex_1().min_w_0().child(div().flex().flex_1().min_w_0().items_center().h(px(CHIP_H + 4.)).px(px(6.)).bg(theme::app_bg()).border_1().border_color(border).rounded(px(3.)).text_color(theme::text_selected()).child(ed.name.clone())).children(below.map(|b| div().absolute().left_0().top(px(CHIP_H + 7.)).child(b)))),
    )
    .with_priority(1)
    .into_any_element()
}
