//! Find (`ui/src/find`): Ctrl+F's box over the graph's top-right corner, or over an open file. In the
//! graph one query matches a commit's message, its SHA and, from two characters, the paths it touched;
//! what doesn't match dims. In a file it marks the text; Monaco's own find is what the web UI shows
//! there, and this box stands in for it.

use crate::app::{GitVolt, icon};
use crate::theme;
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::tooltip::tip;
use gitvolt_core::api::Request;
use gpui::prelude::*;
use gpui::*;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

pub const DEBOUNCE: Duration = Duration::from_millis(50);
pub const PATH_MIN_CHARS: usize = 2;
const BOX_W: f32 = 380.;

#[derive(Debug, Clone, PartialEq)]
pub struct OlderHit {
    pub id: String,
    pub summary: String,
    pub author: String,
    pub time: i64,
}

/// A match in the open file: its row, side, and bytes in the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextMatch {
    pub row: usize,
    pub old_side: bool,
    pub from: usize,
    pub to: usize,
}

pub struct FindState {
    pub open: bool,
    pub input: Entity<TextInput>,
    pub query: String,
    /// Matching commits and their rows, in graph order.
    pub matches: Vec<String>,
    pub rows: Vec<usize>,
    /// The current match, and with it the selected row.
    pub index: Option<usize>,
    pub path_pending: bool,
    /// "Search older history"'s hits, once asked.
    pub older: Option<Vec<OlderHit>>,
    pub older_loading: bool,
    pub message: Option<SharedString>,
    /// The rows that stay bright; `None` dims nothing.
    pub keep: Option<Rc<HashSet<usize>>>,
    /// Bumped by each query: an answer for an older one is dropped.
    seq: u64,
    /// The open file's matches, and the current one.
    pub text_matches: Rc<Vec<TextMatch>>,
    pub text_index: Option<usize>,
    pub flags: FindFlags,
}

impl FindState {
    pub fn new(repo: u32, cx: &mut Context<GitVolt>) -> Self {
        let input = cx.new(|cx| TextInput::new(cx).placeholder("Message, SHA or path").claim_escape().claim_arrows());
        cx.subscribe(&input, move |this, _, e: &InputEvent, cx| match e {
            InputEvent::Changed => this.find_changed(repo, cx),
            InputEvent::Submit { shift, ctrl: false } => this.step_find(if *shift { -1 } else { 1 }, cx),
            InputEvent::Arrow { down, shift: false } => this.step_find(if *down { 1 } else { -1 }, cx),
            InputEvent::Escape => this.close_find(cx),
            _ => {}
        })
        .detach();
        Self { open: false, input, query: String::new(), matches: Vec::new(), rows: Vec::new(), index: None, path_pending: false, older: None, older_loading: false, message: None, keep: None, seq: 0, text_matches: Rc::new(Vec::new()), text_index: None, flags: FindFlags::default() }
    }

    fn clear(&mut self) {
        self.open = false;
        self.query.clear();
        self.matches.clear();
        self.rows.clear();
        self.index = None;
        self.path_pending = false;
        self.older = None;
        self.older_loading = false;
        self.message = None;
        self.keep = None;
        self.seq += 1;
        self.text_matches = Rc::new(Vec::new());
        self.text_index = None;
    }

    /// `3 / 12`, `0 / 0`, or nothing without a query.
    pub fn counter(&self) -> String {
        if !self.text_matches.is_empty() {
            return format!("{} / {}", self.text_index.map_or(0, |i| i + 1), self.text_matches.len());
        }
        match self.index {
            Some(i) if !self.matches.is_empty() => format!("{} / {}", i + 1, self.matches.len()),
            _ if self.query.trim().is_empty() => String::new(),
            _ => "0 / 0".to_string(),
        }
    }
}

/// `ids` in graph order, with their rows.
pub fn ordered(ids: &[String], by_sha: &HashMap<String, usize>) -> (Vec<String>, Vec<usize>) {
    let mut found: Vec<(usize, &String)> = ids.iter().filter_map(|id| by_sha.get(id).map(|r| (*r, id))).collect();
    found.sort_by_key(|f| f.0);
    found.dedup_by_key(|f| f.0);
    (found.iter().map(|f| f.1.clone()).collect(), found.iter().map(|f| f.0).collect())
}

/// Case-insensitive matches of `query` in `text`, as byte ranges.
/// Monaco's toggles.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct FindFlags {
    pub case: bool,
    pub word: bool,
    pub regex: bool,
    pub selection: bool,
}

/// What finds in a line, by the toggles: a bad pattern finds nothing.
pub enum Matcher {
    Plain(String),
    Pattern(Option<regex::Regex>),
}

impl Matcher {
    pub fn new(query: &str, flags: FindFlags) -> Self {
        if !flags.case && !flags.word && !flags.regex {
            return Self::Plain(query.to_string());
        }
        let body = if flags.regex { query.to_string() } else { regex::escape(query) };
        let body = if flags.word { format!(r"\b(?:{body})\b") } else { body };
        Self::Pattern(regex::RegexBuilder::new(&body).case_insensitive(!flags.case).size_limit(1 << 20).build().ok())
    }

    pub fn in_line(&self, text: &str) -> Vec<(usize, usize)> {
        match self {
            Self::Plain(q) => find_in_line(text, q),
            Self::Pattern(Some(re)) => re.find_iter(text).filter(|m| !m.is_empty()).map(|m| (m.start(), m.end())).collect(),
            Self::Pattern(None) => Vec::new(),
        }
    }
}

pub fn find_in_line(text: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let (hay, needle) = (text.to_lowercase(), query.to_lowercase());
    // Lowercasing can change a line's bytes: then it's matched as it is.
    if hay.len() != text.len() {
        return text.match_indices(query).map(|(i, m)| (i, i + m.len())).collect();
    }
    hay.match_indices(&needle).map(|(i, m)| (i, i + m.len())).filter(|(a, b)| text.is_char_boundary(*a) && text.is_char_boundary(*b)).collect()
}

fn is_full_hash(q: &str) -> bool {
    (q.len() == 40 || q.len() == 64) && q.bytes().all(|b| b.is_ascii_hexdigit())
}

impl GitVolt {
    /// Ctrl+F: the box takes the keyboard, its text selected, in the file when one's open.
    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        tab.find.open = true;
        let input = tab.find.input.clone();
        let placeholder = if tab.diff.is_some() { "Find" } else { "Message, SHA or path" };
        input.update(cx, |i, cx| {
            i.set_placeholder(placeholder);
            i.select_all_text(cx);
        });
        window.focus(&input.read(cx).focus.clone(), cx);
        cx.notify();
    }

    /// Closes and clears: the dimming goes with the query.
    pub fn close_find(&mut self, cx: &mut Context<Self>) {
        let focus = self.focus.clone();
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        tab.find.clear();
        if let Some(diff) = tab.diff.as_mut() {
            diff.found = Rc::new(Vec::new());
            diff.found_at = None;
        }
        let input = tab.find.input.clone();
        input.update(cx, |i, cx| i.set_text("", cx));
        self.focus_next_frame = Some(focus);
        cx.notify();
    }

    fn find_changed(&mut self, repo: u32, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) else { return };
        let query = tab.find.input.read(cx).text().to_string();
        tab.find.query = query.clone();
        tab.find.older = None;
        tab.find.older_loading = false;
        tab.find.message = None;
        tab.find.seq += 1;
        let seq = tab.find.seq;
        if tab.diff.is_some() {
            self.find_in_file(cx);
            return;
        }
        let timer = cx.background_executor().timer(DEBOUNCE);
        cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |this, cx| this.run_find(repo, seq, true, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    /// Message and SHA as you type, paths once the index answers.
    fn run_find(&mut self, repo: u32, seq: u64, select: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo) else { return };
        if tab.find.seq != seq {
            return;
        }
        let q = tab.find.query.trim().to_string();
        if q.is_empty() {
            tab.find.matches.clear();
            tab.find.rows.clear();
            tab.find.index = None;
            tab.find.path_pending = false;
            tab.find.keep = None;
            cx.notify();
            return;
        }
        if cfg!(test) {
            return;
        }
        let with_paths = q.chars().count() >= PATH_MIN_CHARS;
        tab.find.path_pending = false;
        let api = self.api.clone();
        let query = q.clone();
        let text = self.rt.spawn(async move { api.dispatch(Request::FindText { repo: crate::app::core_repo(repo), query }).await });
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(found)) = text.await else {
                this.update(cx, |this, cx| {
                    if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo).filter(|t| t.find.seq == seq) {
                        tab.find.message = Some("Search failed".into());
                        cx.notify();
                    }
                })
                .ok();
                return;
            };
            let found = ids(&found);
            this.update(cx, |this, cx| {
                this.show_matches(repo, seq, &found, select, cx);
                if with_paths {
                    if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo).filter(|t| t.find.seq == seq) {
                        tab.find.path_pending = true;
                    }
                    let (query, text_hits) = (q.clone(), found.clone());
                    let rt = this.rt.clone();
                    let paths = rt.spawn(async move { api.dispatch(Request::FindPaths { repo: crate::app::core_repo(repo), query }).await });
                    cx.spawn(async move |this, cx| {
                        let more = match paths.await {
                            Ok(Ok(v)) => ids(&v),
                            _ => Vec::new(),
                        };
                        this.update(cx, |this, cx| {
                            let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo).filter(|t| t.find.seq == seq) else { return };
                            tab.find.path_pending = false;
                            let mut all = text_hits;
                            all.extend(more);
                            this.show_matches(repo, seq, &all, select, cx);
                        })
                        .ok();
                    })
                    .detach();
                }
                // A full hash that isn't in the window: it's the commit asked for, so say so.
                if is_full_hash(&q) && !this.in_graph(&q.to_lowercase()) {
                    if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo).filter(|t| t.find.seq == seq) {
                        tab.find.message = Some("Not in the loaded history".into());
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The current match stays current if it's still one; otherwise the first becomes it.
    fn show_matches(&mut self, repo: u32, seq: u64, found: &[String], select: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == repo).filter(|t| t.find.seq == seq) else { return };
        let Some(graph) = tab.graph.clone() else { return };
        let (matches, rows) = ordered(found, &graph.by_sha);
        let was = tab.find.index.and_then(|i| tab.find.matches.get(i)).cloned();
        let keep = was.as_ref().and_then(|w| matches.iter().position(|m| m == w));
        let index = if matches.is_empty() { None } else { Some(keep.unwrap_or(0)) };
        tab.find.keep = Some(Rc::new(rows.iter().copied().collect()));
        tab.find.matches = matches;
        tab.find.rows = rows.clone();
        tab.find.index = index;
        let row = (select && keep.is_none()).then(|| index.map(|i| rows[i])).flatten();
        cx.notify();
        if let Some(row) = row {
            self.select(row, cx);
        }
    }

    /// Next or previous match; in a file, the next mark.
    pub fn step_find(&mut self, by: i64, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        if !tab.find.text_matches.is_empty() {
            let n = tab.find.text_matches.len() as i64;
            let i = tab.find.text_index.map_or(0, |i| (i as i64 + by).rem_euclid(n)) as usize;
            tab.find.text_index = Some(i);
            let m = tab.find.text_matches[i];
            if let Some(diff) = tab.diff.as_mut() {
                diff.center_on(m.row);
                diff.cursor = m.row;
                diff.found_at = Some(i);
            }
            cx.notify();
            return;
        }
        let n = tab.find.matches.len() as i64;
        if n == 0 {
            return;
        }
        let i = tab.find.index.map_or(0, |i| (i as i64 + by).rem_euclid(n)) as usize;
        tab.find.index = Some(i);
        let row = tab.find.rows[i];
        cx.notify();
        self.select(row, cx);
    }

    /// Marks the query in the open file, from the middle of what's on screen.
    pub fn find_in_file(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let Some(diff) = tab.diff.as_mut() else { return };
        let matches = diff.find_text(tab.find.query.trim(), tab.find.flags);
        let from = diff.layout.row_at(diff.scroll);
        let index = matches.iter().position(|m| m.row >= from).or(if matches.is_empty() { None } else { Some(0) });
        if let Some(m) = index.map(|i| matches[i]) {
            diff.center_on(m.row);
            diff.cursor = m.row;
        }
        tab.find.text_index = index;
        let matches = Rc::new(matches);
        diff.found = matches.clone();
        diff.found_at = index;
        tab.find.text_matches = matches;
        cx.notify();
    }

    fn search_older(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let (repo, query, seq) = (tab.id, tab.find.query.trim().to_string(), tab.find.seq);
        tab.find.older_loading = true;
        cx.notify();
        let api = self.api.clone();
        let done = self.rt.spawn(async move { api.dispatch(Request::SearchHistory { repo: crate::app::core_repo(repo), query }).await });
        cx.spawn(async move |this, cx| {
            let hits = match done.await {
                Ok(Ok(v)) => v.as_array().map_or(Vec::new(), |a| a.iter().map(|h| OlderHit { id: h["id"].as_str().unwrap_or_default().to_string(), summary: h["summary"].as_str().unwrap_or_default().to_string(), author: h["author"].as_str().unwrap_or_default().to_string(), time: h["time"].as_i64().unwrap_or(0) }).collect()),
                _ => Vec::new(),
            };
            this.update(cx, |this, cx| {
                if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo).filter(|t| t.find.seq == seq) {
                    tab.find.older = Some(hits);
                    tab.find.older_loading = false;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// An older hit: loads a window deep enough to hold it, then selects it.
    fn reveal_older(&mut self, sha: String, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let repo = tab.id;
        let api = self.api.clone();
        let id = sha.clone();
        let located = self.rt.spawn(async move { api.dispatch(Request::LocateCommit { repo: crate::app::core_repo(repo), sha: id }).await });
        cx.spawn(async move |this, cx| {
            let limit = match located.await {
                Ok(Ok(v)) if v["found"].as_bool() == Some(true) => v["limit"].as_u64(),
                _ => None,
            };
            this.update(cx, |this, cx| {
                let Some(limit) = limit else {
                    if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == repo) {
                        tab.find.message = Some("Not in the loaded history (deeper than 10,000 commits)".into());
                        cx.notify();
                    }
                    return;
                };
                this.load_deep(repo, limit as u32, Some(sha), cx);
            })
            .ok();
        })
        .detach();
    }
}

fn ids(v: &Value) -> Vec<String> {
    v.as_array().map_or(Vec::new(), |a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
}

fn find_button(id: &'static str, tooltip: &'static str, glyph: &'static str, enabled: bool, cx: &mut Context<GitVolt>, run: impl Fn(&mut GitVolt, &mut Context<GitVolt>) + 'static) -> impl IntoElement {
    tip(
        tooltip,
        div()
            .id(id)
            .flex()
            .flex_none()
            .size(px(22.))
            .items_center()
            .justify_center()
            .rounded(px(3.))
            .when(!enabled, |d| d.opacity(0.4))
            .when(enabled, |d| {
                d.cursor_pointer().hover(|d| d.bg(theme::hover_row())).on_click(cx.listener(move |this, _, _, cx| run(this, cx)))
            })
            .child(icon(glyph, 13., theme::text_secondary())),
    )
}

/// The box, over the top-right corner of the graph or the file.
/// `case`, `word`, `regex`, `selection`, by a toggle's click or Alt and its letter.
fn toggle_find(this: &mut GitVolt, which: char, cx: &mut Context<GitVolt>) {
    let Some(tab) = this.tabs.get_mut(this.active) else { return };
    let f = &mut tab.find.flags;
    match which {
        'c' => f.case = !f.case,
        'w' => f.word = !f.word,
        'r' => f.regex = !f.regex,
        _ => f.selection = !f.selection,
    }
    this.find_in_file(cx);
}

/// Monaco's find widget, over an open file: `right` is what's beside the text at the editor's right, its minimap or ruler.
pub fn editor_find(find: &FindState, right: f32, window: &Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let light = theme::light_now();
    let (bg, border, sash, input_bg, fg, focus) = if light {
        (rgb(0xf3f3f3), rgb(0xc8c8c8), rgb(0xc8c8c8), rgb(0xffffff), rgb(0x424242), rgb(0x0090f1))
    } else {
        (rgb(0x252526), rgb(0x303031), rgb(0x474747), rgb(0x3c3c3c), rgb(0xcccccc), rgb(0x007fd4))
    };
    let fg: Hsla = fg.into();
    let focused = find.input.read(cx).is_focused(window);
    let flags = find.flags;
    let toggle = |id: &'static str, letter: char, on: bool, tooltip: &'static str, face: AnyElement, cx: &mut Context<GitVolt>| {
        tip(
            tooltip,
            crate::ui::anchor(id, div()
                .id(id)
                .flex()
                .flex_none()
                .size(px(20.))
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .border_1()
                .border_color(if on { focus.into() } else { transparent_black() })
                .when(on, |d| d.bg(Hsla { a: 0.4, ..focus.into() }))
                .when(!on, |d| d.hover(|d| d.bg(rgba(if light { 0xb8b8b850 } else { 0x5a5d5e50 }))))
                .text_color(fg)
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| toggle_find(this, letter, cx)))
                .child(face)),
        )
    };
    let glyphs = |t: &'static str, underline: bool| div().text_size(px(12.)).line_height(px(14.)).when(underline, |d| d.border_b_1().border_color(fg)).child(t).into_any_element();
    let any = !find.text_matches.is_empty();
    let count = if find.query.trim().is_empty() || !any { "No results".to_string() } else { format!("{} of {}", find.text_index.map_or(0, |i| i + 1), find.text_matches.len()) };
    let button = |id: &'static str, tooltip: &'static str, glyph: &'static str, enabled: bool, cx: &mut Context<GitVolt>, run: fn(&mut GitVolt, &mut Context<GitVolt>)| {
        tip(
            tooltip,
            div()
                .id(id)
                .flex()
                .flex_none()
                .size(px(20.))
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .when(!enabled, |d| d.opacity(0.4))
                .when(enabled, |d| d.cursor_pointer().hover(|d| d.bg(rgba(if light { 0xb8b8b850 } else { 0x5a5d5e50 }))).on_click(cx.listener(move |this, _, _, cx| run(this, cx))))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(icon(glyph, 16., fg)),
        )
    };
    div()
        .id("find-box")
        .absolute()
        .top(px(74.))
        .right(px(32. + right))
        .w(px(419.))
        .h(px(33.))
        .occlude()
        .flex()
        .items_center()
        .bg(bg)
        .border_1()
        .border_color(border)
        .shadow(crate::ui::shadow(theme::shadow_popup(), 0., 8.))
        .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
            let m = e.keystroke.modifiers;
            if m.alt && !m.control {
                match e.keystroke.key.as_str() {
                    "c" => toggle_find(this, 'c', cx),
                    "w" => toggle_find(this, 'w', cx),
                    "r" => toggle_find(this, 'r', cx),
                    "l" => toggle_find(this, 's', cx),
                    _ => return,
                }
                cx.stop_propagation();
            }
        }))
        .child(div().flex_none().w(px(2.)).h_full().bg(sash))
        .child(
            div()
                .relative()
                .flex()
                .flex_none()
                .ml(px(24.))
                .w(px(215.))
                .h(px(25.))
                .items_center()
                .pl(px(6.))
                .pr(px(66.))
                .bg(input_bg)
                .border_1()
                .border_color(if focused { focus.into() } else { transparent_black() })
                .rounded(px(2.))
                .text_size(px(13.))
                .text_color(fg)
                .child(find.input.clone())
                .child(
                    div()
                        .absolute()
                        .right(px(2.))
                        .top(px(1.))
                        .flex()
                        .gap(px(2.))
                        .child(toggle("find-case", 'c', flags.case, "Match Case (Alt+C)", glyphs("Aa", false), cx))
                        .child(toggle("find-word", 'w', flags.word, "Match Whole Word (Alt+W)", glyphs("ab", true), cx))
                        .child(toggle("find-regex", 'r', flags.regex, "Use Regular Expression (Alt+R)", glyphs(".*", false), cx)),
                ),
        )
        .child(div().flex_none().ml(px(6.)).w(px(69.)).text_size(px(12.5)).whitespace_nowrap().when(!any && !find.query.trim().is_empty(), |d| d.text_color(rgb(if light { 0xa1260d } else { 0xf48771 }))).when(any || find.query.trim().is_empty(), |d| d.text_color(fg)).child(count))
        .child(div().flex().flex_none().gap(px(5.)).child(button("find-prev", "Previous Match (Shift+Enter)", "arrow-up", any, cx, |this, cx| this.step_find(-1, cx))).child(button("find-next", "Next Match (Enter)", "arrow-down", any, cx, |this, cx| this.step_find(1, cx))).child(toggle("find-selection", 's', flags.selection, "Find in Selection (Alt+L)", icon("list", 16., fg).into_any_element(), cx)).child(button("find-close", "Close (Escape)", "x", true, cx, |this, cx| this.close_find(cx))))
        .into_any_element()
}

pub fn find_box(find: &FindState, window: &Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let focused = find.input.read(cx).is_focused(window);
    let counter = find.counter();
    let any = !find.matches.is_empty() || !find.text_matches.is_empty();
    let mut card = div()
        .id("find-box")
        .absolute()
        .top(px(30.))
        .right(px(16.))
        .w(px(BOX_W))
        .occlude()
        .flex()
        .flex_col()
        .p(px(6.))
        .bg(theme::panel_bg1())
        .border_1()
        .border_color(theme::section_border())
        .rounded(px(6.))
        .shadow(crate::ui::shadow(theme::shadow_popup(), 6., 20.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(div().flex().flex_1().min_w_0().h(px(26.)).items_center().px(px(8.)).bg(theme::app_bg()).border_1().border_color(if focused { theme::focus_outline() } else { theme::section_border() }).rounded(px(4.)).text_color(theme::text_selected()).child(find.input.clone()))
                .when(find.path_pending, |d| d.child(icon("loader-circle", 13., theme::text_secondary())))
                .child(div().flex_none().min_w(px(48.)).flex().justify_end().text_size(px(11.)).text_color(theme::text_secondary()).child(counter))
                .child(find_button("find-prev", "Previous match (Shift+Enter)", "arrow-up", any, cx, |this, cx| this.step_find(-1, cx)))
                .child(find_button("find-next", "Next match (Enter)", "arrow-down", any, cx, |this, cx| this.step_find(1, cx)))
                .child(find_button("find-close", "Close (Esc)", "x", true, cx, |this, cx| this.close_find(cx))),
        );
    if let Some(message) = find.message.clone() {
        card = card.child(div().mt(px(6.)).text_size(px(12.)).text_color(theme::text_secondary()).child(message));
    }
    if !find.query.trim().is_empty() && find.text_matches.is_empty() {
        let older = match &find.older {
            None => {
                let label = if find.older_loading { "Searching…" } else { "Search older history" };
                div()
                    .id("find-older")
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .px(px(2.))
                    .py(px(1.))
                    .rounded(px(3.))
                    .cursor_pointer()
                    .text_color(theme::text_secondary())
                    .hover(|d| d.bg(theme::hover_row()).text_color(theme::text_selected()))
                    .when(!find.older_loading, |d| d.on_click(cx.listener(|this, _, _, cx| this.search_older(cx))))
                    .child(icon("history", 12., theme::text_secondary()))
                    .child(label)
                    .into_any_element()
            }
            Some(hits) if hits.is_empty() => div().text_color(theme::text_secondary()).child("No older commits match.").into_any_element(),
            Some(hits) => {
                let mut list = div().id("find-older-list").flex().flex_col().max_h(px(240.)).overflow_y_scroll();
                for (i, h) in hits.iter().enumerate() {
                    let sha = h.id.clone();
                    list = list.child(
                        div()
                            .id(("older", i))
                            .flex()
                            .flex_none()
                            .gap(px(6.))
                            .px(px(4.))
                            .py(px(2.))
                            .rounded(px(3.))
                            .whitespace_nowrap()
                            .cursor_pointer()
                            .hover(|d| d.bg(theme::hover_row()))
                            .on_click(cx.listener(move |this, _, _, cx| this.reveal_older(sha.clone(), cx)))
                            .child(div().flex_none().font_family(crate::app::MONO_FONT).text_color(theme::text_secondary()).child(h.id.chars().take(6).collect::<String>()))
                            .child(div().flex_1().min_w_0().truncate().text_color(theme::text_normal()).child(h.summary.clone()))
                            .child(div().flex_none().text_color(theme::text_secondary()).child(format!("{} · {}", h.author, crate::format::relative_time(h.time, crate::format::now())))),
                    );
                }
                list.into_any_element()
            }
        };
        card = card.child(div().mt(px(6.)).text_size(px(12.)).child(older));
    }
    card.into_any_element()
}

impl GitVolt {
    pub fn focus_graph(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.side.keys = None;
            tab.keys = crate::app::KeyArea::Graph;
            tab.diff = None;
            tab.history = None;
        }
        cx.notify();
    }

    pub fn focus_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.side.keys = None;
            tab.keys = crate::app::KeyArea::Files;
        }
        cx.notify();
    }

    pub fn set_find_text(&mut self, q: &str, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let (input, id) = (tab.find.input.clone(), tab.id);
        input.update(cx, |i, cx| i.set_text(q, cx));
        self.find_changed(id, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn matches_come_in_graph_order() {
        let by: HashMap<String, usize> = [("a".to_string(), 5), ("b".to_string(), 2), ("c".to_string(), 9)].into_iter().collect();
        let (m, r) = ordered(&["a".into(), "b".into(), "x".into(), "a".into()], &by);
        assert_eq!(m, ["b", "a"]);
        assert_eq!(r, [2, 5]);
    }

    #[test]
    fn a_line_is_searched_without_case() {
        assert_eq!(find_in_line("Foo foo", "FOO"), [(0, 3), (4, 7)]);
        assert!(find_in_line("abc", "").is_empty());
        assert_eq!(find_in_line("İx x", "x").len(), 2);
    }

    #[test]
    fn the_counter_says_where_it_is() {
        let mut s = FindCounter { index: Some(1), matches: 3, query: "a" };
        assert_eq!(s.text(), "2 / 3");
        s.index = None;
        s.matches = 0;
        assert_eq!(s.text(), "0 / 0");
        s.query = " ";
        assert_eq!(s.text(), "");
    }

    struct FindCounter {
        index: Option<usize>,
        matches: usize,
        query: &'static str,
    }

    impl FindCounter {
        fn text(&self) -> String {
            match self.index {
                Some(i) if self.matches > 0 => format!("{} / {}", i + 1, self.matches),
                _ if self.query.trim().is_empty() => String::new(),
                _ => "0 / 0".to_string(),
            }
        }
    }

    #[test]
    fn a_full_hash_is_forty_or_sixty_four_hex() {
        assert!(is_full_hash(&"a".repeat(40)));
        assert!(is_full_hash(&"0".repeat(64)));
        assert!(!is_full_hash("abc"));
        assert!(!is_full_hash(&"g".repeat(40)));
    }
}
