//! A text field and a textarea: caret, selection, mouse and keyboard editing, word movement,
//! clipboard, undo, placeholder and IME through GPUI's `InputHandler` (`<input>` and `<textarea>` as
//! the web UI uses them). No caret blink: an idle app costs nothing.
//!
//! ```ignore
//! let input = cx.new(|cx| TextInput::new(cx).placeholder("Filter"));
//! cx.subscribe(&input, |this: &mut GitVolt, _, e: &InputEvent, cx| { .. }).detach();
//! div().child(input.clone())
//! ```
//!
//! `Buffer` is the editing model, pure and tested; `TextInput` is the entity that draws it and takes
//! the keys. Up, Down and Esc go on to the parent unless the owner claims them.

use crate::theme;
use gpui::prelude::*;
use gpui::*;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        DeleteWordRight,
        DeleteToStart,
        DeleteToEnd,
        Left,
        Right,
        Up,
        Down,
        WordLeft,
        WordRight,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectWordLeft,
        SelectWordRight,
        SelectAll,
        Home,
        End,
        SelectHome,
        SelectEnd,
        DocStart,
        DocEnd,
        SelectDocStart,
        SelectDocEnd,
        Paste,
        Cut,
        Copy,
        Undo,
        Redo,
        Enter,
        ShiftEnter,
        CtrlEnter,
        Escape,
    ]
);

pub fn bind_keys(cx: &mut App) {
    let c = Some("TextInput");
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("shift-backspace", Backspace, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, c),
        KeyBinding::new("ctrl-delete", DeleteWordRight, c),
        KeyBinding::new("left", Left, c),
        KeyBinding::new("right", Right, c),
        KeyBinding::new("up", Up, c),
        KeyBinding::new("down", Down, c),
        KeyBinding::new("ctrl-left", WordLeft, c),
        KeyBinding::new("ctrl-right", WordRight, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("shift-up", SelectUp, c),
        KeyBinding::new("shift-down", SelectDown, c),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, c),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, c),
        KeyBinding::new("ctrl-a", SelectAll, c),
        KeyBinding::new("home", Home, c),
        KeyBinding::new("end", End, c),
        KeyBinding::new("shift-home", SelectHome, c),
        KeyBinding::new("shift-end", SelectEnd, c),
        KeyBinding::new("ctrl-home", DocStart, c),
        KeyBinding::new("ctrl-end", DocEnd, c),
        KeyBinding::new("ctrl-shift-home", SelectDocStart, c),
        KeyBinding::new("ctrl-shift-end", SelectDocEnd, c),
        KeyBinding::new("ctrl-v", Paste, c),
        KeyBinding::new("shift-insert", Paste, c),
        KeyBinding::new("ctrl-x", Cut, c),
        KeyBinding::new("ctrl-c", Copy, c),
        KeyBinding::new("ctrl-z", Undo, c),
        KeyBinding::new("ctrl-shift-z", Redo, c),
        KeyBinding::new("ctrl-y", Redo, c),
        KeyBinding::new("enter", Enter, c),
        KeyBinding::new("shift-enter", ShiftEnter, c),
        KeyBinding::new("ctrl-enter", CtrlEnter, c),
        KeyBinding::new("escape", Escape, c),
    ]);
}

/// Whether a field has the keyboard: the keys that mean something to text are its.
pub fn is_typing(window: &Window) -> bool {
    window.context_stack().iter().any(|c| c.contains("TextInput"))
}

/// What an edit is, for undo: typing runs together; anything else stands alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Deleting,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    text: String,
    sel: Range<usize>,
    reversed: bool,
}

/// A word character, as the browser's word movement sees one.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The text, the selection, what the IME is composing, and undo. All offsets are UTF-8 bytes.
#[derive(Debug, Clone, Default)]
pub struct Buffer {
    pub text: String,
    pub sel: Range<usize>,
    /// The caret is at `sel.start`.
    pub reversed: bool,
    pub marked: Option<Range<usize>>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    run: Option<EditKind>,
    multiline: bool,
}

impl Buffer {
    pub fn new(multiline: bool) -> Self {
        Self { multiline, ..Self::default() }
    }

    pub fn caret(&self) -> usize {
        if self.reversed { self.sel.start } else { self.sel.end }
    }

    pub fn selected(&self) -> &str {
        &self.text[self.sel.clone()]
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { text: self.text.clone(), sel: self.sel.clone(), reversed: self.reversed }
    }

    fn restore(&mut self, s: Snapshot) {
        self.text = s.text;
        self.sel = s.sel;
        self.reversed = s.reversed;
        self.marked = None;
    }

    /// Before an edit: a new undo step, unless it goes on the run before it.
    fn checkpoint(&mut self, kind: EditKind) {
        if kind == EditKind::Other || self.run != Some(kind) {
            self.undo.push(self.snapshot());
            self.redo.clear();
        }
        self.run = Some(kind);
    }

    /// Replaces the whole text: a program setting it, not an edit (undo starts over).
    pub fn set(&mut self, text: &str) {
        self.text = self.clean(text);
        self.sel = self.text.len()..self.text.len();
        self.reversed = false;
        self.marked = None;
        self.undo.clear();
        self.redo.clear();
        self.run = None;
    }

    /// A field holds one line: a pasted line break becomes a space, as in a browser's `<input>`.
    fn clean(&self, text: &str) -> String {
        if self.multiline {
            return text.replace("\r\n", "\n").replace('\r', "\n");
        }
        let mut out = String::with_capacity(text.len());
        let mut breaking = false;
        for c in text.chars() {
            if c == '\n' || c == '\r' {
                if !breaking {
                    out.push(' ');
                }
                breaking = true;
            } else {
                out.push(c);
                breaking = false;
            }
        }
        out
    }

    /// Replaces `range` (default what's composed, else the selection) with `new`; the caret after it.
    pub fn replace(&mut self, range: Option<Range<usize>>, new: &str, kind: EditKind) {
        let range = range.or_else(|| self.marked.clone()).unwrap_or_else(|| self.sel.clone());
        let new = self.clean(new);
        if range.is_empty() && new.is_empty() {
            return;
        }
        self.checkpoint(kind);
        self.text.replace_range(range.clone(), &new);
        let at = range.start + new.len();
        self.sel = at..at;
        self.reversed = false;
        self.marked = None;
    }

    /// Composition: `new` stands in `range`, marked; `selected` (within `new`) or the caret after it.
    pub fn replace_marked(&mut self, range: Option<Range<usize>>, new: &str, selected: Option<Range<usize>>) {
        let range = range.or_else(|| self.marked.clone()).unwrap_or_else(|| self.sel.clone());
        self.checkpoint(EditKind::Typing);
        self.text.replace_range(range.clone(), new);
        self.marked = (!new.is_empty()).then(|| range.start..range.start + new.len());
        self.sel = match selected {
            Some(s) => range.start + s.start.min(new.len())..range.start + s.end.min(new.len()),
            None => range.start + new.len()..range.start + new.len(),
        };
        self.reversed = false;
    }

    pub fn move_to(&mut self, at: usize) {
        self.sel = at..at;
        self.reversed = false;
        self.run = None;
    }

    pub fn select_to(&mut self, at: usize) {
        if self.reversed {
            self.sel.start = at;
        } else {
            self.sel.end = at;
        }
        if self.sel.end < self.sel.start {
            self.reversed = !self.reversed;
            self.sel = self.sel.end..self.sel.start;
        }
        self.run = None;
    }

    pub fn select_all(&mut self) {
        self.sel = 0..self.text.len();
        self.reversed = false;
        self.run = None;
    }

    pub fn prev_grapheme(&self, at: usize) -> usize {
        self.text[..at].grapheme_indices(true).next_back().map_or(0, |(i, _)| i)
    }

    pub fn next_grapheme(&self, at: usize) -> usize {
        self.text[at..].grapheme_indices(true).nth(1).map_or(self.text.len(), |(i, _)| at + i)
    }

    /// Ctrl+←: the start of the word before the caret.
    pub fn word_left(&self, at: usize) -> usize {
        let mut chars = self.text[..at].char_indices().rev().peekable();
        while chars.peek().is_some_and(|(_, c)| !is_word(*c)) {
            chars.next();
        }
        let mut start = chars.peek().map_or(0, |(i, _)| *i);
        for (i, c) in chars {
            if !is_word(c) {
                break;
            }
            start = i;
        }
        start
    }

    /// Ctrl+→: the end of the word after the caret, as a browser does on Linux and macOS.
    pub fn word_right(&self, at: usize) -> usize {
        let mut chars = self.text[at..].char_indices().peekable();
        while chars.peek().is_some_and(|(_, c)| !is_word(*c)) {
            chars.next();
        }
        for (i, c) in chars {
            if !is_word(c) {
                return at + i;
            }
        }
        self.text.len()
    }

    /// What a double click takes: the word, the run of spaces or the punctuation mark at `at`.
    pub fn word_at(&self, at: usize) -> Range<usize> {
        let class = |c: char| if is_word(c) { 0 } else if c.is_whitespace() { 1 } else { 2 };
        let Some(c) = self.text[at..].chars().next().or_else(|| self.text[..at].chars().next_back()) else { return at..at };
        let of = class(c);
        if of == 2 {
            let start = if self.text[at..].starts_with(c) { at } else { at - c.len_utf8() };
            return start..start + c.len_utf8();
        }
        let mut start = at;
        for (i, c) in self.text[..at].char_indices().rev() {
            if class(c) != of || c == '\n' {
                break;
            }
            start = i;
        }
        let mut end = at;
        for (i, c) in self.text[at..].char_indices() {
            if class(c) != of || c == '\n' {
                break;
            }
            end = at + i + c.len_utf8();
        }
        start..end
    }

    /// The start and the end of the logical line around `at`.
    pub fn line_at(&self, at: usize) -> Range<usize> {
        let start = self.text[..at].rfind('\n').map_or(0, |i| i + 1);
        let end = self.text[at..].find('\n').map_or(self.text.len(), |i| at + i);
        start..end
    }

    pub fn undo(&mut self) -> bool {
        let Some(s) = self.undo.pop() else { return false };
        self.redo.push(self.snapshot());
        self.restore(s);
        self.run = None;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(s) = self.redo.pop() else { return false };
        self.undo.push(self.snapshot());
        self.restore(s);
        self.run = None;
        true
    }

    pub fn to_utf16(&self, offset: usize) -> usize {
        self.text[..offset.min(self.text.len())].encode_utf16().count()
    }

    pub fn utf8_of(&self, offset: usize) -> usize {
        let (mut u8s, mut u16s) = (0, 0);
        for c in self.text.chars() {
            if u16s >= offset {
                break;
            }
            u16s += c.len_utf16();
            u8s += c.len_utf8();
        }
        u8s
    }

    fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.to_utf16(r.start)..self.to_utf16(r.end)
    }

    fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.utf8_of(r.start)..self.utf8_of(r.end)
    }
}

/// What a `TextInput` tells its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    /// The text changed, by the user.
    Changed,
    /// Enter (a field), or Ctrl+Enter. `shift`: Shift+Enter in a field.
    Submit { shift: bool, ctrl: bool },
    /// Esc, when the owner claims it.
    Escape,
    /// ↑ or ↓, when the owner claims them.
    Arrow { down: bool, shift: bool },
    Focused,
    Blurred,
}

/// One shaped logical line and where it starts, in the text and on the element.
struct Laid {
    start: usize,
    y: Pixels,
    line: WrappedLine,
}

pub struct TextInput {
    pub focus: FocusHandle,
    pub buffer: Buffer,
    placeholder: SharedString,
    multiline: bool,
    claim_escape: Claim,
    claim_arrows: bool,
    was_focused: bool,
    /// A monospace field, a size or colour other than what it inherits.
    text_color: Option<Hsla>,
    placeholder_color: Hsla,
    disabled: bool,
    selecting: bool,
    /// The press's granularity while it drags: a double click extends by words, a triple by lines.
    drag_unit: u8,
    drag_origin: Range<usize>,
    /// Scrolled so the caret stays in view: x in a field, y in a textarea.
    scroll: Pixels,
    laid: Vec<Laid>,
    origin: Point<Pixels>,
    line_h: Pixels,
    viewport: Size<Pixels>,
    last_caret: usize,
    follow_caret: bool,
}

/// Whether Esc is the owner's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Claim {
    Never,
    Always,
    /// While there's text to clear; an empty field's Esc goes on.
    WhenText,
}

impl EventEmitter<InputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TextInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self::with_lines(false, cx)
    }

    pub fn multiline(cx: &mut Context<Self>) -> Self {
        Self::with_lines(true, cx)
    }

    fn with_lines(multiline: bool, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        Self {
            focus,
            buffer: Buffer::new(multiline),
            placeholder: SharedString::default(),
            multiline,
            claim_escape: Claim::Never,
            claim_arrows: false,
            was_focused: false,
            text_color: None,
            placeholder_color: theme::text_disabled(),
            disabled: false,
            selecting: false,
            drag_unit: 1,
            drag_origin: 0..0,
            scroll: px(0.),
            laid: Vec::new(),
            origin: Point::default(),
            line_h: px(18.),
            viewport: Size::default(),
            last_caret: 0,
            follow_caret: true,
        }
    }

    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = text.into();
        self
    }

    pub fn placeholder_color(mut self, color: Hsla) -> Self {
        self.placeholder_color = color;
        self
    }

    pub fn text_color(mut self, color: Hsla) -> Self {
        self.text_color = Some(color);
        self
    }

    /// Esc is the owner's: `InputEvent::Escape`, and nothing behind the field sees it.
    pub fn claim_escape(mut self) -> Self {
        self.claim_escape = Claim::Always;
        self
    }

    /// Esc is the owner's while there's text (to clear); an empty field's goes on behind it.
    pub fn claim_escape_with_text(mut self) -> Self {
        self.claim_escape = Claim::WhenText;
        self
    }

    /// ↑ and ↓ are the owner's in a field: `InputEvent::Arrow`.
    pub fn claim_arrows(mut self) -> Self {
        self.claim_arrows = true;
        self
    }

    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        cx.notify();
    }

    pub fn text(&self) -> &str {
        &self.buffer.text
    }

    /// Sets the text, the caret after it. No `Changed`: the owner knows.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.buffer.text != text {
            self.buffer.set(text);
            self.follow_caret = true;
            cx.notify();
        }
    }

    pub fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.buffer.select_all();
        cx.notify();
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    fn moved(&mut self, cx: &mut Context<Self>) {
        self.follow_caret = true;
        if !self.buffer.sel.is_empty() {
            cx.write_to_primary(ClipboardItem::new_string(self.buffer.selected().to_string()));
        }
        cx.notify();
    }

    fn edited(&mut self, cx: &mut Context<Self>) {
        self.follow_caret = true;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn edit(&mut self, range: Option<Range<usize>>, new: &str, kind: EditKind, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let before = self.buffer.text.len();
        let had = self.buffer.sel.len();
        self.buffer.replace(range, new, kind);
        if before != self.buffer.text.len() || had > 0 || !new.is_empty() {
            self.edited(cx);
        }
    }

    fn collapse_or(&mut self, to_end: bool, step: impl FnOnce(&Buffer, usize) -> usize, cx: &mut Context<Self>) {
        let at = if self.buffer.sel.is_empty() {
            step(&self.buffer, self.buffer.caret())
        } else if to_end {
            self.buffer.sel.end
        } else {
            self.buffer.sel.start
        };
        self.buffer.move_to(at);
        self.moved(cx);
    }

    fn jump(&mut self, select: bool, at: usize, cx: &mut Context<Self>) {
        if select {
            self.buffer.select_to(at);
        } else {
            self.buffer.move_to(at);
        }
        self.moved(cx);
    }

    fn delete_back(&mut self, to: impl FnOnce(&Buffer, usize) -> usize, cx: &mut Context<Self>) {
        if self.buffer.sel.is_empty() {
            let caret = self.buffer.caret();
            let from = to(&self.buffer, caret);
            self.edit(Some(from..caret), "", EditKind::Deleting, cx);
        } else {
            self.edit(None, "", EditKind::Deleting, cx);
        }
    }

    fn delete_forward(&mut self, to: impl FnOnce(&Buffer, usize) -> usize, cx: &mut Context<Self>) {
        if self.buffer.sel.is_empty() {
            let caret = self.buffer.caret();
            let until = to(&self.buffer, caret);
            self.edit(Some(caret..until), "", EditKind::Deleting, cx);
        } else {
            self.edit(None, "", EditKind::Deleting, cx);
        }
    }

    /// The visual row of `at` and its x, from the last layout.
    fn position_of(&self, at: usize) -> Point<Pixels> {
        for l in self.laid.iter().rev() {
            if at >= l.start {
                let p = l.line.position_for_index(at - l.start, self.line_h).unwrap_or_default();
                return point(p.x, l.y + p.y);
            }
        }
        Point::default()
    }

    /// The index under a point in the text's own space (scrolled).
    fn index_at_local(&self, p: Point<Pixels>) -> usize {
        for (i, l) in self.laid.iter().enumerate() {
            let bottom = l.y + l.line.size(self.line_h).height;
            if p.y < bottom || i + 1 == self.laid.len() {
                let local = point(p.x.max(px(0.)), (p.y - l.y).max(px(0.)).min(l.line.size(self.line_h).height - px(1.)));
                let index = match l.line.closest_index_for_position(local, self.line_h) {
                    Ok(i) | Err(i) => i,
                };
                return l.start + index.min(l.line.len());
            }
        }
        0
    }

    fn index_at(&self, position: Point<Pixels>) -> usize {
        let p = position - self.origin;
        if self.multiline { self.index_at_local(point(p.x, p.y + self.scroll)) } else { self.index_at_local(point(p.x + self.scroll, self.line_h / 2.)) }
    }

    /// The caret one visual row up or down, its x kept; past the ends, the text's ends.
    fn vertical(&self, down: bool) -> usize {
        let p = self.position_of(self.buffer.caret());
        let y = if down { p.y + self.line_h } else { p.y - self.line_h };
        if y < px(0.) {
            return 0;
        }
        let total = self.laid.last().map_or(px(0.), |l| l.y + l.line.size(self.line_h).height);
        if y >= total {
            return self.buffer.text.len();
        }
        self.index_at_local(point(p.x, y))
    }

    /// Home and End: the visual row's ends in a textarea, the text's in a field.
    fn row_end(&self, end: bool) -> usize {
        if !self.multiline {
            return if end { self.buffer.text.len() } else { 0 };
        }
        let caret = self.buffer.caret();
        let p = self.position_of(caret);
        let at = self.index_at_local(point(if end { px(1_000_000.) } else { px(0.) }, p.y));
        let line = self.buffer.line_at(caret);
        at.clamp(line.start, line.end)
    }

    fn on_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        window.focus(&self.focus, cx);
        window.prevent_default();
        let at = self.index_at(e.position);
        self.selecting = true;
        self.drag_unit = e.click_count.clamp(1, 3) as u8;
        if e.modifiers.shift {
            self.buffer.select_to(at);
            self.drag_origin = self.buffer.sel.clone();
        } else {
            match self.drag_unit {
                1 => self.buffer.move_to(at),
                2 => {
                    let w = self.buffer.word_at(at);
                    self.buffer.sel = w;
                    self.buffer.reversed = false;
                }
                _ => {
                    let l = if self.multiline { self.buffer.line_at(at) } else { 0..self.buffer.text.len() };
                    self.buffer.sel = l;
                    self.buffer.reversed = false;
                }
            }
            self.drag_origin = self.buffer.sel.clone();
        }
        self.moved(cx);
    }

    fn on_mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || e.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let at = self.index_at(e.position);
        match self.drag_unit {
            1 if !e.modifiers.shift => self.buffer.select_to(at),
            1 => self.buffer.select_to(at),
            unit => {
                let around = if unit == 2 { self.buffer.word_at(at) } else if self.multiline { self.buffer.line_at(at) } else { 0..self.buffer.text.len() };
                let o = self.drag_origin.clone();
                self.buffer.sel = o.start.min(around.start)..o.end.max(around.end);
                self.buffer.reversed = at < o.start;
            }
        }
        self.moved(cx);
    }

    fn on_middle_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        window.focus(&self.focus, cx);
        let at = self.index_at(e.position);
        self.buffer.move_to(at);
        if let Some(text) = cx.read_from_primary().and_then(|i| i.text()) {
            self.edit(None, &text, EditKind::Other, cx);
        }
    }

    fn escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        // Esc ends a composition first, as a browser does.
        if self.buffer.marked.take().is_some() {
            window.notify_text_input_changed(&self.focus);
            cx.notify();
        } else if self.claim_escape == Claim::Always || (self.claim_escape == Claim::WhenText && !self.buffer.text.is_empty()) {
            cx.emit(InputEvent::Escape);
        } else {
            cx.propagate();
        }
    }

    fn arrow(&mut self, down: bool, shift: bool, cx: &mut Context<Self>) {
        if self.multiline {
            let at = self.vertical(down);
            self.jump(shift, at, cx);
        } else if self.claim_arrows {
            cx.emit(InputEvent::Arrow { down, shift });
        } else {
            cx.propagate();
        }
    }

    fn enter(&mut self, shift: bool, cx: &mut Context<Self>) {
        if self.multiline {
            self.edit(None, "\n", EditKind::Other, cx);
        } else {
            cx.emit(InputEvent::Submit { shift, ctrl: false });
        }
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(&mut self, range: Range<usize>, actual: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let r = self.buffer.range_from_utf16(&range);
        actual.replace(self.buffer.range_to_utf16(&r));
        Some(self.buffer.text[r].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.buffer.range_to_utf16(&self.buffer.sel), reversed: self.buffer.reversed })
    }

    fn set_selected_text_range(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.buffer.sel = self.buffer.range_from_utf16(&range);
        self.buffer.reversed = false;
        window.notify_text_selection_changed(&self.focus);
        self.moved(cx);
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.buffer.text.encode_utf16().count())
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.buffer.marked.as_ref().map(|r| self.buffer.range_to_utf16(r))
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.buffer.marked = None;
        window.notify_text_input_changed(&self.focus);
        cx.notify();
    }

    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let range = range.map(|r| self.buffer.range_from_utf16(&r));
        // A character with the control or platform key isn't text; the platform may still offer it.
        if text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
            return;
        }
        self.edit(range, text, EditKind::Typing, cx);
        window.notify_text_input_changed(&self.focus);
    }

    fn replace_and_mark_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, selected: Option<Range<usize>>, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let range = range.map(|r| self.buffer.range_from_utf16(&r));
        // `selected` counts UTF-16 units within `text`.
        let selected = selected.map(|s| {
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
            map(s.start)..map(s.end)
        });
        self.buffer.replace_marked(range, text, selected);
        self.follow_caret = true;
        window.notify_text_input_changed(&self.focus);
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn bounds_for_range(&mut self, range: Range<usize>, bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let r = self.buffer.range_from_utf16(&range);
        let (a, b) = (self.position_of(r.start), self.position_of(r.end));
        let shift = if self.multiline { point(px(0.), -self.scroll) } else { point(-self.scroll, px(0.)) };
        Some(Bounds::from_corners(bounds.origin + a + shift, bounds.origin + b + shift + point(px(0.), self.line_h)))
    }

    fn selection_bounds_for_range(&mut self, range: Range<usize>, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> Vec<Bounds<Pixels>> {
        self.bounds_for_range(range, bounds, window, cx).into_iter().collect()
    }

    fn character_index_for_point(&mut self, p: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let at = self.index_at(p);
        Some(self.buffer.to_utf16(at))
    }
}

struct InputElement {
    input: Entity<TextInput>,
}

struct Prepainted {
    lines: Vec<Laid>,
    placeholder: bool,
    selection: Vec<Bounds<Pixels>>,
    caret: Option<Bounds<Pixels>>,
    scroll: Pixels,
    line_h: Pixels,
}

impl IntoElement for InputElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

/// The visual rows of a shaped line, as index ranges.
fn rows_of(line: &WrappedLine) -> Vec<Range<usize>> {
    let mut out = Vec::with_capacity(line.wrap_boundaries.len() + 1);
    let mut start = 0;
    for b in line.wrap_boundaries.iter() {
        let end = line.unwrapped_layout.runs[b.run_ix].glyphs[b.glyph_ix].index;
        out.push(start..end);
        start = end;
    }
    out.push(start..line.len());
    out
}

impl Element for InputElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepainted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let multiline = self.input.read(cx).multiline;
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = if multiline { relative(1.).into() } else { window.line_height().into() };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) -> Prepainted {
        let input = self.input.read(cx);
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_h = window.line_height();
        let empty = input.buffer.text.is_empty();
        let shown: SharedString = if empty { input.placeholder.clone() } else { input.buffer.text.clone().into() };
        let color = if empty { input.placeholder_color } else { input.text_color.unwrap_or(style.color) };
        let run = TextRun { len: shown.len(), font: style.font(), color, background_color: None, underline: None, strikethrough: None };
        let runs: Vec<TextRun> = match input.buffer.marked.as_ref().filter(|_| !empty) {
            Some(m) => vec![
                TextRun { len: m.start, ..run.clone() },
                TextRun { len: m.end - m.start, underline: Some(UnderlineStyle { color: Some(run.color), thickness: px(1.), wavy: false }), ..run.clone() },
                TextRun { len: shown.len() - m.end, ..run },
            ],
            None => vec![run],
        };
        let wrap = input.multiline.then_some(bounds.size.width);
        let shaped = window.text_system().shape_text(shown.clone(), font_size, &runs, wrap, None).unwrap_or_default();
        let mut lines = Vec::with_capacity(shaped.len());
        let (mut start, mut y) = (0, px(0.));
        for line in shaped {
            let h = line.size(line_h).height;
            let len = line.len();
            lines.push(Laid { start, y, line });
            start += len + 1;
            y += h;
        }
        let total = y;
        let locate = |at: usize| -> Point<Pixels> {
            for l in lines.iter().rev() {
                if at >= l.start {
                    let p = l.line.position_for_index(at - l.start, line_h).unwrap_or_default();
                    return point(p.x, l.y + p.y);
                }
            }
            Point::default()
        };
        let caret_at = if empty { Point::default() } else { locate(input.buffer.caret()) };
        // Keep the caret in view, only when it moved: a wheel over a textarea stays where it went.
        let mut scroll = input.scroll;
        let follow = input.follow_caret || input.last_caret != input.buffer.caret();
        if input.multiline {
            if follow {
                if caret_at.y < scroll {
                    scroll = caret_at.y;
                } else if caret_at.y + line_h > scroll + bounds.size.height {
                    scroll = caret_at.y + line_h - bounds.size.height;
                }
            }
            scroll = scroll.clamp(px(0.), (total - bounds.size.height).max(px(0.)));
        } else {
            let width = lines.first().map_or(px(0.), |l| l.line.size(line_h).width);
            if follow {
                let room = (bounds.size.width - px(1.)).max(px(0.));
                if caret_at.x < scroll {
                    scroll = caret_at.x;
                } else if caret_at.x > scroll + room {
                    scroll = caret_at.x - room;
                }
            }
            scroll = scroll.clamp(px(0.), (width - bounds.size.width + px(1.)).max(px(0.)));
        }
        let shift = if input.multiline { point(px(0.), -scroll) } else { point(-scroll, px(0.)) };
        let mut selection = Vec::new();
        let sel = input.buffer.sel.clone();
        if !sel.is_empty() && !empty {
            for l in &lines {
                let line_end = l.start + l.line.len();
                if sel.end < l.start || sel.start > line_end {
                    continue;
                }
                for (ix, row) in rows_of(&l.line).into_iter().enumerate() {
                    let (a, b) = (sel.start.max(l.start + row.start), sel.end.min(l.start + row.end));
                    if a > b || (a == b && !(b == line_end && sel.end > line_end)) {
                        continue;
                    }
                    let x0 = l.line.unwrapped_layout.x_for_index(row.start);
                    let x_a = l.line.unwrapped_layout.x_for_index(a - l.start) - x0;
                    let x_b = l.line.unwrapped_layout.x_for_index(b - l.start) - x0;
                    // A selected line break shows as a sliver, as a browser draws it.
                    let extra = if sel.end > line_end && b == line_end { px(5.) } else { px(0.) };
                    let top = l.y + line_h * ix as f32;
                    selection.push(Bounds::from_corners(bounds.origin + shift + point(x_a, top), bounds.origin + shift + point(x_b + extra, top + line_h)));
                }
            }
        }
        let caret = sel.is_empty().then(|| Bounds::new(bounds.origin + shift + caret_at, size(px(1.), line_h)));
        self.input.update(cx, |i, _| {
            i.scroll = scroll;
            i.follow_caret = false;
            i.last_caret = i.buffer.caret();
            i.viewport = bounds.size;
            i.origin = bounds.origin;
            i.line_h = line_h;
        });
        Prepainted { lines, placeholder: empty, selection, caret, scroll, line_h }
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), p: &mut Prepainted, window: &mut Window, cx: &mut App) {
        let input = self.input.read(cx);
        let (focus, multiline, disabled) = (input.focus.clone(), input.multiline, input.disabled);
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.input.clone()), cx);
        let focused = focus.is_focused(window);
        let shift = if multiline { point(px(0.), -p.scroll) } else { point(-p.scroll, px(0.)) };
        let line_h = p.line_h;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if focused {
                for s in &p.selection {
                    window.paint_quad(fill(*s, theme::selection_bg()));
                }
            }
            for l in &p.lines {
                let y = bounds.origin.y + shift.y + l.y;
                if y + l.line.size(line_h).height >= bounds.top() && y <= bounds.bottom() {
                    l.line.paint(point(bounds.origin.x + shift.x, y), line_h, TextAlign::Left, None, window, cx).ok();
                }
            }
            if focused && !disabled && let Some(c) = p.caret {
                window.paint_quad(fill(c, theme::text_selected()));
            }
        });
        let lines = std::mem::take(&mut p.lines);
        let placeholder = p.placeholder;
        self.input.update(cx, |i, _| i.laid = if placeholder { Vec::new() } else { lines });
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let multiline = self.multiline;
        let focused = self.focus.is_focused(window);
        if focused != self.was_focused {
            self.was_focused = focused;
            if !focused {
                self.selecting = false;
            }
            cx.emit(if focused { InputEvent::Focused } else { InputEvent::Blurred });
        }
        div()
            .id("text-input")
            .key_context("TextInput")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .items_center()
            .overflow_hidden()
            .when(!self.disabled, |d| d.cursor(CursorStyle::IBeam))
            .on_action(cx.listener(|this, _: &Backspace, _, cx| this.delete_back(Buffer::prev_grapheme, cx)))
            .on_action(cx.listener(|this, _: &Delete, _, cx| this.delete_forward(Buffer::next_grapheme, cx)))
            .on_action(cx.listener(|this, _: &DeleteWordLeft, _, cx| this.delete_back(Buffer::word_left, cx)))
            .on_action(cx.listener(|this, _: &DeleteWordRight, _, cx| this.delete_forward(Buffer::word_right, cx)))
            .on_action(cx.listener(|this, _: &DeleteToStart, _, cx| this.delete_back(|_, _| 0, cx)))
            .on_action(cx.listener(|this, _: &DeleteToEnd, _, cx| this.delete_forward(|b, _| b.text.len(), cx)))
            .on_action(cx.listener(|this, _: &Left, _, cx| this.collapse_or(false, Buffer::prev_grapheme, cx)))
            .on_action(cx.listener(|this, _: &Right, _, cx| this.collapse_or(true, Buffer::next_grapheme, cx)))
            .on_action(cx.listener(|this, _: &Up, _, cx| this.arrow(false, false, cx)))
            .on_action(cx.listener(|this, _: &Down, _, cx| this.arrow(true, false, cx)))
            .on_action(cx.listener(|this, _: &SelectUp, _, cx| this.arrow(false, true, cx)))
            .on_action(cx.listener(|this, _: &SelectDown, _, cx| this.arrow(true, true, cx)))
            .on_action(cx.listener(|this, _: &WordLeft, _, cx| this.jump(false, this.buffer.word_left(this.buffer.caret()), cx)))
            .on_action(cx.listener(|this, _: &WordRight, _, cx| this.jump(false, this.buffer.word_right(this.buffer.caret()), cx)))
            .on_action(cx.listener(|this, _: &SelectLeft, _, cx| this.jump(true, this.buffer.prev_grapheme(this.buffer.caret()), cx)))
            .on_action(cx.listener(|this, _: &SelectRight, _, cx| this.jump(true, this.buffer.next_grapheme(this.buffer.caret()), cx)))
            .on_action(cx.listener(|this, _: &SelectWordLeft, _, cx| this.jump(true, this.buffer.word_left(this.buffer.caret()), cx)))
            .on_action(cx.listener(|this, _: &SelectWordRight, _, cx| this.jump(true, this.buffer.word_right(this.buffer.caret()), cx)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                this.buffer.select_all();
                this.moved(cx);
            }))
            .on_action(cx.listener(|this, _: &Home, _, cx| this.jump(false, this.row_end(false), cx)))
            .on_action(cx.listener(|this, _: &End, _, cx| this.jump(false, this.row_end(true), cx)))
            .on_action(cx.listener(|this, _: &SelectHome, _, cx| this.jump(true, this.row_end(false), cx)))
            .on_action(cx.listener(|this, _: &SelectEnd, _, cx| this.jump(true, this.row_end(true), cx)))
            .on_action(cx.listener(|this, _: &DocStart, _, cx| this.jump(false, 0, cx)))
            .on_action(cx.listener(|this, _: &DocEnd, _, cx| this.jump(false, this.buffer.text.len(), cx)))
            .on_action(cx.listener(|this, _: &SelectDocStart, _, cx| this.jump(true, 0, cx)))
            .on_action(cx.listener(|this, _: &SelectDocEnd, _, cx| this.jump(true, this.buffer.text.len(), cx)))
            .on_action(cx.listener(|this, _: &Paste, _, cx| {
                if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    this.edit(None, &text, EditKind::Other, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Copy, _, cx| {
                if !this.buffer.sel.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(this.buffer.selected().to_string()));
                }
            }))
            .on_action(cx.listener(|this, _: &Cut, _, cx| {
                if !this.buffer.sel.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(this.buffer.selected().to_string()));
                    this.edit(None, "", EditKind::Other, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Undo, _, cx| {
                if this.buffer.undo() {
                    this.edited(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Redo, _, cx| {
                if this.buffer.redo() {
                    this.edited(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Enter, _, cx| this.enter(false, cx)))
            .on_action(cx.listener(|this, _: &ShiftEnter, _, cx| this.enter(true, cx)))
            .on_action(cx.listener(|_, _: &CtrlEnter, _, cx| cx.emit(InputEvent::Submit { shift: false, ctrl: true })))
            .on_action(cx.listener(Self::escape))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_middle_down))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, _| this.selecting = false))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, _| this.selecting = false))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .when(multiline, |d| {
                d.on_scroll(cx.listener(|this, e: &ScrollEvent, _, cx| {
                    let total = this.laid.last().map_or(px(0.), |l| l.y + l.line.size(this.line_h).height);
                    let dy = e.delta.pixel_delta(this.line_h).y;
                    this.scroll = (this.scroll - dy).clamp(px(0.), (total - this.viewport.height).max(px(0.)));
                    cx.notify();
                }))
            })
            .child(InputElement { input: cx.entity() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn buf(text: &str) -> Buffer {
        let mut b = Buffer::new(false);
        b.set(text);
        b
    }

    #[test]
    fn typing_replaces_the_selection_and_runs_together_for_undo() {
        let mut b = buf("hello");
        b.select_all();
        b.replace(None, "a", EditKind::Typing);
        b.replace(None, "b", EditKind::Typing);
        assert_eq!(b.text, "ab");
        assert_eq!(b.caret(), 2);
        assert!(b.undo());
        assert_eq!(b.text, "hello");
        assert_eq!(b.sel, 0..5, "the selection comes back with it");
        assert!(b.redo());
        assert_eq!(b.text, "ab");
    }

    #[test]
    fn a_caret_move_ends_the_undo_run() {
        let mut b = buf("");
        b.replace(None, "a", EditKind::Typing);
        b.move_to(0);
        b.replace(None, "b", EditKind::Typing);
        assert_eq!(b.text, "ba");
        b.undo();
        assert_eq!(b.text, "a");
        b.undo();
        assert_eq!(b.text, "");
        assert!(!b.undo());
    }

    #[test]
    fn a_field_takes_one_line_a_textarea_keeps_them() {
        let mut b = buf("");
        b.replace(None, "a\r\nb\n\nc", EditKind::Other);
        assert_eq!(b.text, "a b c");
        let mut t = Buffer::new(true);
        t.replace(None, "a\r\nb", EditKind::Other);
        assert_eq!(t.text, "a\nb");
    }

    #[test]
    fn words_move_as_in_a_browser_on_linux() {
        let b = buf("foo-bar  baz");
        assert_eq!(b.word_right(0), 3);
        assert_eq!(b.word_right(3), 7);
        assert_eq!(b.word_right(7), 12);
        assert_eq!(b.word_left(12), 9);
        assert_eq!(b.word_left(9), 4);
        assert_eq!(b.word_left(4), 0);
        assert_eq!(b.word_left(0), 0);
        assert_eq!(b.word_right(12), 12);
    }

    #[test]
    fn a_double_click_takes_the_word_the_spaces_or_the_mark() {
        let b = buf("foo-bar  baz");
        assert_eq!(b.word_at(1), 0..3);
        assert_eq!(b.word_at(3), 3..4);
        assert_eq!(b.word_at(8), 7..9);
        assert_eq!(b.word_at(12), 9..12, "at the end, the word before it");
    }

    #[test]
    fn graphemes_and_utf16_are_counted_right() {
        let b = buf("a\u{1F600}e\u{301}");
        assert_eq!(b.next_grapheme(0), 1);
        assert_eq!(b.next_grapheme(1), 5);
        assert_eq!(b.next_grapheme(5), 8);
        assert_eq!(b.prev_grapheme(8), 5);
        assert_eq!(b.prev_grapheme(5), 1);
        assert_eq!(b.to_utf16(5), 3);
        assert_eq!(b.utf8_of(3), 5);
    }

    #[test]
    fn a_composition_stands_marked_and_commits_on_replace() {
        let mut b = buf("x");
        b.replace_marked(None, "ka", None);
        assert_eq!(b.marked, Some(1..3));
        b.replace_marked(None, "か", None);
        assert_eq!(b.text, "xか");
        assert_eq!(b.marked, Some(1..4));
        b.replace(None, "火", EditKind::Typing);
        assert_eq!(b.text, "x火");
        assert_eq!(b.marked, None);
        assert_eq!(b.caret(), 4);
        b.undo();
        assert_eq!(b.text, "x", "one undo takes the whole composition");
    }

    #[test]
    fn selecting_past_the_anchor_turns_it_around() {
        let mut b = buf("abcdef");
        b.move_to(3);
        b.select_to(5);
        assert_eq!((b.sel.clone(), b.reversed), (3..5, false));
        b.select_to(1);
        assert_eq!((b.sel.clone(), b.reversed), (1..3, true));
        assert_eq!(b.caret(), 1);
        assert_eq!(b.selected(), "bc");
    }

    struct Host {
        input: Entity<TextInput>,
        events: Vec<InputEvent>,
    }
    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(300.)).h(px(40.)).text_size(px(13.)).line_height(px(18.)).child(self.input.clone())
        }
    }

    fn host(cx: &mut TestAppContext, multi: bool) -> (Entity<Host>, VisualTestContext) {
        cx.update(bind_keys);
        let window = cx
            .update(|cx| {
                cx.open_window(WindowOptions::default(), |window, cx| {
                    let input = cx.new(|cx| if multi { TextInput::multiline(cx) } else { TextInput::new(cx).claim_escape() });
                    window.focus(&input.read(cx).focus.clone(), cx);
                    cx.new(|cx| {
                        cx.subscribe(&input, |this: &mut Host, _, e: &InputEvent, _| this.events.push(*e)).detach();
                        Host { input, events: Vec::new() }
                    })
                })
            })
            .unwrap();
        let view = window.root(cx).unwrap();
        let visual = VisualTestContext::from_window(window.into(), cx);
        (view, visual)
    }

    #[gpui::test]
    fn keys_edit_the_field(cx: &mut TestAppContext) {
        let (view, mut cx) = host(cx, false);
        let text = |view: &Entity<Host>, cx: &mut VisualTestContext| view.read_with(cx, |h, cx| h.input.read(cx).text().to_string());
        cx.simulate_input("hello world");
        assert_eq!(text(&view, &mut cx), "hello world");
        cx.simulate_keystrokes("ctrl-backspace");
        assert_eq!(text(&view, &mut cx), "hello ");
        cx.simulate_keystrokes("ctrl-a backspace");
        assert_eq!(text(&view, &mut cx), "");
        cx.simulate_keystrokes("ctrl-z");
        assert_eq!(text(&view, &mut cx), "hello ");
        cx.simulate_keystrokes("home delete");
        assert_eq!(text(&view, &mut cx), "ello ");
        cx.simulate_keystrokes("enter escape");
        cx.run_until_parked();
        let events = view.read_with(&cx, |h, _| h.events.clone());
        assert!(events.contains(&InputEvent::Changed));
        assert!(events.contains(&InputEvent::Submit { shift: false, ctrl: false }));
        assert!(events.contains(&InputEvent::Escape));
    }

    #[gpui::test]
    fn a_textarea_breaks_lines_and_moves_between_them(cx: &mut TestAppContext) {
        let (view, mut cx) = host(cx, true);
        cx.simulate_input("ab");
        cx.simulate_keystrokes("enter");
        cx.simulate_input("cd");
        cx.run_until_parked();
        view.read_with(&cx, |h, cx| assert_eq!(h.input.read(cx).text(), "ab\ncd"));
        cx.simulate_keystrokes("up");
        cx.run_until_parked();
        view.read_with(&cx, |h, cx| assert!(h.input.read(cx).buffer.caret() <= 2));
        cx.simulate_keystrokes("ctrl-end");
        view.read_with(&cx, |h, cx| assert_eq!(h.input.read(cx).buffer.caret(), 5));
    }

    #[gpui::test]
    fn a_click_places_the_caret_and_a_drag_selects(cx: &mut TestAppContext) {
        let (view, mut cx) = host(cx, false);
        cx.simulate_input("abcdefghij");
        cx.run_until_parked();
        let y = px(10.);
        cx.simulate_mouse_down(point(px(0.), y), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(point(px(0.), y), MouseButton::Left, Modifiers::none());
        view.read_with(&cx, |h, cx| assert_eq!(h.input.read(cx).buffer.caret(), 0));
        cx.simulate_mouse_down(point(px(0.), y), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(point(px(280.), y), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(point(px(280.), y), MouseButton::Left, Modifiers::none());
        view.read_with(&cx, |h, cx| assert_eq!(h.input.read(cx).buffer.sel, 0..10));
    }
}
