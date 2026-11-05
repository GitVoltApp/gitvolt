//! Review mode in the source diff (`diff/review/`, `monaco/reviewZones.ts`, `reviewGutter.ts`): the gutter's "+", cards under their lines, F9 between them.
//!
//! A card takes its space among the diff's rows as a `RowKind::Zone` row, which the painter leaves blank, and the card itself is an ordinary element laid over
//! that row (as a hunk row's buttons are), with a real width and its own keyboard. Its height is what it measures: a change comes back through
//! `DiffState::zones` and lays the rows out again, keeping the lines in view where they are.

use super::{CommentableFile, PlacedItem, ReviewSession, anchor_for, anchor_span, is_pending, pending_label, pending_text, place_review};
use crate::app::{GitVolt, core_repo, icon};
use crate::diff::model::{Row, RowKind};
use crate::diff::view::{DiffState, ViewKind};
use crate::forge::card::forge_avatar;
use crate::forge::mrview::header::{mr_button, mr_primary};
use crate::forge::mrview::{flavor_of, link_env};
use crate::forge::text::{forge_name, mr_ref};
use crate::md::field::{FieldEvent, FieldOptions, MarkdownField};
use crate::md::view::MdOptions;
use crate::theme;
use crate::ui::arm::{self, ArmRequest, Origin};
use crate::ui::menu::{self, MenuRow};
use crate::ui::tooltip::{Placement, tip};
use gitvolt_core::forge::types::{DiffSide, ForgeDiscussion, LineKind, ReviewAnchor, ReviewDraft};
use gitvolt_core::forge::ForgeKind;
use gpui::prelude::*;
use gpui::*;
use std::collections::HashMap;

pub const NO_SUGGESTION: &str = "Suggestions are for the new side's lines";
pub const NO_DRAFTS: &str = "This GitLab keeps no pending comments (16.3 or later does): use Comment now";
pub const NOT_COMMENTABLE: &str = "No review comments here: the forge only takes them on the merge request's changed lines and the lines around them";
pub const GLYPH_TIP: &str = "Comment on this line (drag for several)";
const CARD_RIGHT: f32 = 12.;
const CARD_MARGIN: f32 = 12.;
/// A card's height until it's measured.
const FIRST_H: f32 = 80.;

#[derive(Debug, Clone, PartialEq)]
pub struct ZoneSlot {
    pub key: String,
    pub new_side: bool,
    /// 1-based, on its side.
    pub line: u32,
    pub height: f32,
}

/// `zoneTarget`: the rows with a card's zone after its line. In Split each side has its own; in Inline and Hunk an old line's card goes under its removed block.
pub fn with_zones(rows: Vec<Row>, zones: &[ZoneSlot], split: bool) -> Vec<Row> {
    if zones.is_empty() {
        return rows;
    }
    let mut places: Vec<(usize, usize)> = zones
        .iter()
        .enumerate()
        .map(|(i, z)| {
            let line = z.line.saturating_sub(1);
            let found = rows.iter().position(|r| !matches!(r.kind, RowKind::Hidden { .. }) && if z.new_side { r.new == Some(line) } else { r.old == Some(line) });
            let at = match found {
                Some(mut p) => {
                    if !z.new_side && !split {
                        while rows.get(p + 1).is_some_and(|r| r.new.is_none() && r.old.is_some() && !matches!(r.kind, RowKind::Hidden { .. })) {
                            p += 1;
                        }
                    }
                    p + 1
                }
                // Folded away: below the fold.
                None => rows.iter().rposition(|r| if z.new_side { r.new.is_some_and(|n| n <= line) } else { r.old.is_some_and(|n| n <= line) }).map_or(0, |p| p + 1),
            };
            (at, i)
        })
        .collect();
    places.sort_unstable();
    let mut out = Vec::with_capacity(rows.len() + places.len());
    let mut next = places.into_iter().peekable();
    for (i, r) in rows.into_iter().enumerate() {
        while let Some((_, zone)) = next.next_if(|p| p.0 <= i) {
            out.push(Row { old: None, new: None, kind: RowKind::Zone { zone: zone as u32 } });
        }
        out.push(r);
    }
    for (_, zone) in next {
        out.push(Row { old: None, new: None, kind: RowKind::Zone { zone: zone as u32 } });
    }
    out
}

/// What a card under a line is.
#[derive(Clone)]
pub enum Entry {
    Thread { thread: ForgeDiscussion, outdated: bool },
    Draft { draft: ReviewDraft, outdated: bool },
    Box(OpenBox),
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenBox {
    pub key: String,
    pub anchor: ReviewAnchor,
    pub suggestion: Option<String>,
}

pub fn box_key(a: &ReviewAnchor) -> String {
    format!("{}:{:?}:{}-{}", a.path, a.end.side(), a.start.map_or(String::new(), |s| s.number().to_string()), a.end.number())
}

/// Suggest change's block: GitHub's `suggestion` replaces the commented lines; GitLab's `suggestion:-N+0` the commented line and the N above it. A longer fence when the code has one.
pub fn suggestion_block(kind: ForgeKind, lines: &[String]) -> String {
    let fence = if lines.iter().any(|l| l.contains("```")) { "````" } else { "```" };
    let info = if kind == ForgeKind::GitLab { format!("suggestion:-{}+0", lines.len().saturating_sub(1)) } else { "suggestion".to_string() };
    format!("{fence}{info}\n{}\n{fence}", lines.join("\n"))
}

/// The session's diff refs aren't its Compare's head: the forge's lines and positions aren't the Compare's.
pub fn compare_stale(s: &ReviewSession) -> bool {
    match (&s.compare, &s.refs) {
        (Some(c), Some(r)) => r.head_sha != c.1,
        _ => false,
    }
}

/// `reviewModeOf`: on when the open diff is the session's Compare for a file of the MR. `file`: the lines that take a comment, once read.
#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Off,
    On { stale: bool, updating: bool, too_large: bool, file: Option<CommentableFile> },
}

pub fn review_mode(s: Option<&ReviewSession>, compare: Option<(&str, &str)>, path: &str, view_is_diff: bool, mr_head: Option<&str>) -> Mode {
    let Some(s) = s else { return Mode::Off };
    let Some(c) = &s.compare else { return Mode::Off };
    if !view_is_diff || compare != Some((c.0.as_str(), c.1.as_str())) {
        return Mode::Off;
    }
    if compare_stale(s) {
        return Mode::On { stale: true, updating: mr_head == Some(c.1.as_str()), too_large: false, file: None };
    }
    let read = s.diff_head.as_deref() == Some(c.1.as_str());
    let file = if read { s.files.get(path) } else { None };
    if read && file.is_none() {
        return Mode::Off;
    }
    Mode::On { stale: false, updating: false, too_large: file.is_some_and(|f| f.too_large), file: file.filter(|f| !f.too_large).cloned() }
}

/// `reviewEntries`: one file's cards, in line order (at one line: its threads, then drafts, then boxes), with whether a thread's folded to the gutter.
pub fn entries(placed: &[PlacedItem], boxes: &[OpenBox]) -> Vec<(String, bool, u32, Entry)> {
    let mut out: Vec<(u8, String, bool, u32, Entry)> = Vec::new();
    for p in placed.iter().filter(|p| p.shown()) {
        match p {
            PlacedItem::Draft { draft, at } => out.push((1, format!("d:{}", draft.id), at.side == DiffSide::New, at.line, Entry::Draft { draft: draft.clone(), outdated: at.outdated })),
            PlacedItem::Thread { thread, at } => out.push((0, format!("t:{}", thread.id), at.side == DiffSide::New, at.line, Entry::Thread { thread: thread.clone(), outdated: at.outdated })),
        }
    }
    for b in boxes {
        out.push((2, format!("b:{}", b.key), b.anchor.end.side() == DiffSide::New, b.anchor.end.number(), Entry::Box(b.clone())));
    }
    out.sort_by_key(|e| (e.3, e.0));
    out.into_iter().map(|e| (e.1, e.2, e.3, e.4)).collect()
}

/// What review mode keeps in a tab, for the session.
#[derive(Default)]
pub struct ReviewUi {
    pub boxes: Vec<OpenBox>,
    pub folds: HashMap<String, bool>,
    pub fields: HashMap<String, Entity<MarkdownField>>,
    pub errors: HashMap<String, String>,
    pub busy: Option<String>,
    /// Where each file's badge went last.
    pub turns: HashMap<String, (String, usize)>,
    pub submit_open: Option<u64>,
    /// A press on the "+": the side, and the lines it's gone over.
    pub drag: Option<(bool, u32, u32)>,
    pub stop: Option<String>,
}

impl GitVolt {
    fn review_field(&mut self, tab: u32, key: &str, label: &'static str, placeholder: &'static str, grow: bool, submit: impl Fn(&mut GitVolt, &mut Context<GitVolt>) + 'static, cx: &mut Context<Self>) -> Option<Entity<MarkdownField>> {
        let t = self.tabs.iter().find(|t| t.id == tab)?;
        if let Some(f) = t.forge.review_ui.fields.get(key) {
            return Some(f.clone());
        }
        let kind = t.forge.kind?;
        let number = t.forge.review.as_ref()?.number;
        let md = MdOptions::for_comment(core_repo(tab), link_env(t), flavor_of(kind), Default::default());
        let people = self.mr_people_source(tab, number);
        let opts = FieldOptions { md, label: label.into(), placeholder: placeholder.into(), rows: 4, grow_preview: grow, people: Some(people) };
        let (api, rt, retired) = (self.api.clone(), self.rt.clone(), self.retired_images.clone());
        let field = cx.new(|cx| MarkdownField::new(opts, api, rt, retired, cx));
        cx.subscribe(&field, move |v, _, e: &FieldEvent, cx| match e {
            FieldEvent::Submit => submit(v, cx),
            FieldEvent::Changed => cx.notify(),
            FieldEvent::Blurred => {}
        })
        .detach();
        let t = self.tabs.iter_mut().find(|t| t.id == tab)?;
        t.forge.review_ui.fields.insert(key.to_string(), field.clone());
        Some(field)
    }

    /// Opens a box under `anchor` (the one already there, if any), and gives it the keyboard.
    pub fn review_open_box(&mut self, tab: u32, anchor: ReviewAnchor, suggestion: Option<String>, cx: &mut Context<Self>) {
        let key = box_key(&anchor);
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        if !t.forge.review_ui.boxes.iter().any(|b| b.key == key) {
            t.forge.review_ui.boxes.push(OpenBox { key: key.clone(), anchor, suggestion });
        }
        t.forge.review_ui.stop = Some(format!("b:{key}"));
        cx.notify();
    }

    fn review_send_box(&mut self, tab: u32, key: String, now: bool, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some(b) = t.forge.review_ui.boxes.iter().find(|b| b.key == key).cloned() else { return };
        let Some(field) = t.forge.review_ui.fields.get(&format!("b:{key}")).cloned() else { return };
        let can_draft = t.forge.review.as_ref().is_none_or(|s| s.can_draft);
        let stale = t.forge.review.as_ref().is_some_and(compare_stale);
        let text = field.read(cx).text(cx);
        if text.trim().is_empty() || t.forge.review_ui.busy.is_some() || stale {
            return;
        }
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        if !now && !can_draft {
            t.forge.review_ui.errors.insert(key, NO_DRAFTS.to_string());
            cx.notify();
            return;
        }
        t.forge.review_ui.busy = Some(key.clone());
        t.forge.review_ui.errors.remove(&key);
        cx.notify();
        let done = move |v: &mut GitVolt, out: Result<(), String>, cx: &mut Context<GitVolt>| {
            let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) else { return };
            t.forge.review_ui.busy = None;
            match out {
                Err(e) => {
                    t.forge.review_ui.errors.insert(key.clone(), e);
                }
                Ok(()) => {
                    t.forge.review_ui.boxes.retain(|b| b.key != key);
                    t.forge.review_ui.fields.remove(&format!("b:{key}"));
                    v.focus_next_frame = Some(v.focus.clone());
                }
            }
            cx.notify();
        };
        if now {
            self.review_comment_now(tab, b.anchor, text, cx, done);
        } else {
            self.review_add_draft(tab, b.anchor, text, cx, done);
        }
    }

    fn review_close_box(&mut self, tab: u32, key: &str, cx: &mut Context<Self>) {
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
            t.forge.review_ui.boxes.retain(|b| b.key != key);
            t.forge.review_ui.fields.remove(&format!("b:{key}"));
            t.forge.review_ui.errors.remove(key);
        }
        self.focus_next_frame = Some(self.focus.clone());
        cx.notify();
    }

    /// Cancel: arms first when there's text.
    fn review_cancel_box(&mut self, tab: u32, key: String, window: &mut Window, cx: &mut Context<Self>) {
        let empty = self.tabs.iter().find(|t| t.id == tab).and_then(|t| t.forge.review_ui.fields.get(&format!("b:{key}"))).is_none_or(|f| f.read(cx).text(cx).trim().is_empty());
        if empty {
            return self.review_close_box(tab, &key, cx);
        }
        let weak = cx.weak_entity();
        let req = ArmRequest::danger("Click again to discard the comment", "Discard this comment?", "Discard");
        arm::request(req, Origin::control(format!("rv-cancel:{key}")), move |answer, _, cx| {
            if answer.ok {
                weak.update(cx, |v, cx| v.review_close_box(tab, &key, cx)).ok();
            }
        }, window, cx);
    }

    fn review_start_edit(&mut self, tab: u32, id: String, body: String, window: &mut Window, cx: &mut Context<Self>) {
        let key = format!("e:{id}");
        let (i2, k2) = (id.clone(), key.clone());
        let Some(f) = self.review_field(tab, &key, "Edit pending comment", "", false, move |v, cx| v.review_save_edit(tab, i2.clone(), k2.clone(), cx), cx) else { return };
        f.update(cx, |f, cx| {
            f.set_text(&body, cx);
            f.focus(window, cx);
        });
        cx.notify();
    }

    fn review_save_edit(&mut self, tab: u32, id: String, key: String, cx: &mut Context<Self>) {
        let Some(f) = self.tabs.iter().find(|t| t.id == tab).and_then(|t| t.forge.review_ui.fields.get(&key).cloned()) else { return };
        let body = f.read(cx).text(cx);
        if body.trim().is_empty() {
            return;
        }
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
            t.forge.review_ui.busy = Some(key.clone());
        }
        self.review_edit_draft(tab, id, body, cx, move |v, out, cx| {
            let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) else { return };
            t.forge.review_ui.busy = None;
            match out {
                Ok(()) => {
                    t.forge.review_ui.fields.remove(&key);
                    t.forge.review_ui.errors.remove(&key);
                }
                Err(e) => {
                    t.forge.review_ui.errors.insert(key.clone(), e);
                }
            }
            cx.notify();
        });
    }

    fn review_remove_draft(&mut self, tab: u32, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        let req = ArmRequest::danger("Click again to delete the pending comment", "Delete this pending comment?", "Delete");
        arm::request(req, Origin::control(format!("rv-delete:{id}")), move |answer, _, cx| {
            if answer.ok {
                weak.update(cx, |v, cx| v.review_delete_draft(tab, id.clone(), cx)).ok();
            }
        }, window, cx);
    }

    /// The diff's lines under a comment, from the shown file: what Suggest change puts in.
    fn review_new_lines(&self, tab: u32, from: u32, to: u32) -> Option<Vec<String>> {
        let d = self.tabs.iter().find(|t| t.id == tab)?.diff.as_ref()?.data.as_ref()?.clone();
        let lines = &d.new.lines;
        let (a, b) = (from.checked_sub(1)? as usize, to.checked_sub(1)? as usize);
        (a <= b && b < lines.len()).then(|| lines[a..=b].iter().map(|l| l.text.to_string()).collect())
    }

    /// A click or a drag from the "+".
    pub fn review_pick(&mut self, tab: u32, new_side: bool, from: u32, to: u32, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some(path) = t.diff.as_ref().map(|d| d.file.path.to_string()) else { return };
        let Some(file) = t.forge.review.as_ref().and_then(|s| s.files.get(&path)).cloned() else { return };
        let side = if new_side { DiffSide::New } else { DiffSide::Old };
        let Some(anchor) = anchor_for(&file, side, from, to) else {
            return crate::ui::toast::show(NOT_COMMENTABLE, cx);
        };
        let (s, a, b) = anchor_span(&file, &anchor);
        let suggestion = if s == DiffSide::New { self.review_new_lines(tab, a, b).map(|l| l.join("\n")) } else { None };
        self.review_open_box(tab, anchor, suggestion, cx);
    }

    /// Mod+Alt+C: a comment on the selected lines, or the caret's.
    pub fn review_comment_key(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get(self.active) else { return };
        let tab = t.id;
        let Some(d) = t.diff.as_ref() else { return };
        let Some(r) = d.rows.get(d.cursor) else { return };
        let (new_side, line) = match (r.new, r.old) {
            (Some(n), _) => (true, n + 1),
            (None, Some(n)) => (false, n + 1),
            _ => return,
        };
        let (a, b) = d.selection_lines(new_side).unwrap_or((line, line));
        self.review_pick(tab, new_side, a, b, cx);
    }

    /// F9, Shift+F9: between the threads and pending comments of the file; the card's first control takes the keyboard.
    pub fn review_step(&mut self, next: bool, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        let Some(d) = t.diff.as_mut() else { return };
        let stops: Vec<(usize, String)> = d.rows.iter().enumerate().filter_map(|(i, r)| match r.kind {
            RowKind::Zone { zone } => d.zones.get(zone as usize).filter(|z| !z.key.starts_with("b:")).map(|z| (i, z.key.clone())),
            _ => None,
        }).collect();
        if stops.is_empty() {
            return;
        }
        let top = d.layout.row_at(d.scroll);
        let current = t.forge.review_ui.stop.clone();
        let at = current.as_ref().and_then(|c| stops.iter().position(|s| &s.1 == c));
        let to = match (at, next) {
            (Some(i), true) => (i + 1) % stops.len(),
            (Some(i), false) => (i + stops.len() - 1) % stops.len(),
            (None, true) => stops.iter().position(|s| s.0 >= top).unwrap_or(0),
            (None, false) => stops.iter().rposition(|s| s.0 < top).unwrap_or(stops.len() - 1),
        };
        let (row, key) = stops[to].clone();
        d.center_on(row);
        t.forge.review_ui.stop = Some(key);
        cx.notify();
    }

    /// Before the frame: the open diff's zones from the session, and its cards built, the view being free to read here.
    pub fn review_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get(self.active) else { return };
        let tab = t.id;
        let Some(diff) = t.diff.as_ref() else { return };
        let path = diff.file.path.to_string();
        let compare = t.compare.as_ref().and_then(|c| Some((c.from.as_str(), c.to.sha()?)));
        let session = t.forge.review.as_ref();
        let head = session.and_then(|s| t.forge.details.get(&s.number).map(|d| &d.mr).or_else(|| t.forge.known_mr(s.number))).and_then(|m| m.head_sha.as_deref());
        let mode = review_mode(session, compare, &path, diff.view == ViewKind::Diff, head);
        let Mode::On { stale, updating, too_large, file } = mode else {
            if !diff.zones.is_empty() || diff.review.is_some() {
                let t = self.tabs.get_mut(self.active).expect("the tab");
                let d = t.diff.as_mut().expect("the diff");
                d.zones.clear();
                d.review = None;
                d.relayout();
            }
            return;
        };
        let Some(s) = session.cloned() else { return };
        let number = s.number;
        let kind = s.kind;
        let threads = t.forge.discussions.get(&number).cloned().unwrap_or_default();
        let placed = if stale { Default::default() } else { place_review(&s, &threads) };
        let boxes: Vec<OpenBox> = t.forge.review_ui.boxes.iter().filter(|b| b.anchor.path == path).cloned().collect();
        let list = entries(placed.by_path.get(&path).map_or(&[][..], Vec::as_slice), &boxes);
        let folds = t.forge.review_ui.folds.clone();
        // A folded thread (a resolved one starts folded) takes no room among the lines: it's an icon in the gutter.
        let open_of = |id: &str, resolved: bool| folds.get(id).copied().unwrap_or(!resolved);
        let heights = diff.zone_heights.borrow().clone();
        let mut slots = Vec::new();
        let mut shown = Vec::new();
        let mut icons: HashMap<(bool, u32), Vec<ForgeDiscussion>> = HashMap::new();
        for (key, new_side, line, e) in list {
            if let Entry::Thread { thread, .. } = &e
                && !open_of(&thread.id, thread.resolved)
            {
                icons.entry((new_side, line)).or_default().push(thread.clone());
                continue;
            }
            slots.push(ZoneSlot { height: heights.get(&key).copied().unwrap_or(FIRST_H), key: key.clone(), new_side, line });
            shown.push((key, e));
        }
        let review = crate::diff::view::ReviewState {
            old: file.as_ref().map_or(Default::default(), |f| f.old.keys().copied().collect()),
            new: file.as_ref().map_or(Default::default(), |f| f.new.keys().copied().collect()),
            icons,
            banner: stale.then(|| if updating { format!("{} is still catching up with the latest push: comments are off until it has", forge_name(kind)) } else { "This merge request moved on: compare again to review its latest changes".to_string() }).or(too_large.then(|| format!("{} sent no diff for this file: it takes no comments here", forge_name(kind)))),
            tab,
        };
        let t = self.tabs.get_mut(self.active).expect("the tab");
        let d = t.diff.as_mut().expect("the diff");
        let same_set = d.zones.len() == slots.len() && d.zones.iter().zip(&slots).all(|(a, b)| a.key == b.key && a.new_side == b.new_side && a.line == b.line);
        d.review = Some(review);
        if !same_set {
            d.zones = slots;
            d.relayout();
        } else if d.zones.iter().zip(&slots).any(|(a, b)| a.height != b.height) {
            // Cards above the view that change height keep the lines in view where they are.
            let mut shift = 0.;
            for (ix, r) in d.rows.iter().enumerate() {
                if let RowKind::Zone { zone } = r.kind
                    && let (Some(a), Some(b)) = (d.zones.get(zone as usize), slots.get(zone as usize))
                    && d.layout.offsets.get(ix).is_some_and(|top| top + a.height <= d.scroll)
                {
                    shift += b.height - a.height;
                }
            }
            d.zones = slots;
            d.scroll += shift;
            d.layout_again();
        }
        // The cards.
        let mut built = Vec::new();
        for (key, e) in shown {
            let el = match e {
                Entry::Thread { thread, outdated } => thread_card(self, tab, kind, number, &thread, outdated, cx),
                Entry::Draft { draft, outdated } => draft_card(self, tab, kind, &draft, outdated, cx),
                Entry::Box(b) => comment_box(self, tab, kind, &b, stale, window, cx),
            };
            built.push((key, el));
        }
        let t = self.tabs.get_mut(self.active).expect("the tab");
        let d = t.diff.as_mut().expect("the diff");
        *d.zone_elements.borrow_mut() = built;
    }
}

/// The small icon button of a card's head.
fn review_icon(id: String, glyph: &'static str) -> Stateful<Div> {
    div().id(ElementId::Name(id.into())).flex().flex_none().size(px(22.)).items_center().justify_center().rounded(px(4.)).cursor_pointer().hover(|d| d.bg(theme::control_hover_bg())).child(icon(glyph, 14., theme::text_secondary()))
}

fn chip(text: &'static str, bg: Hsla, fg: Hsla) -> Div {
    div().flex_none().px(px(6.)).rounded(px(11.)).text_size(px(11.)).font_weight(FontWeight::MEDIUM).bg(bg).text_color(fg).child(text)
}

fn pending_chip() -> Div {
    chip("Pending", theme::mix(theme::yellow(), 0.22, transparent_black()), theme::text_selected())
}

fn outdated_chip() -> Div {
    chip("Outdated", theme::control_bg(), theme::text_secondary())
}

fn card_shell() -> Div {
    div().flex().flex_col().gap(px(6.)).px(px(10.)).py(px(8.)).rounded(px(8.)).border_1().border_color(theme::section_border()).bg(theme::panel_bg1()).text_color(theme::text_normal()).text_size(px(12.)).cursor_default()
}

/// `CommentBox`: a new comment under lines, then Cancel, Suggest change, Comment now and Add to review (the default, Mod+Enter).
pub fn comment_box(this: &mut GitVolt, tab: u32, kind: ForgeKind, b: &OpenBox, stale: bool, window: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let key = b.key.clone();
    let k2 = key.clone();
    let Some(field) = this.review_field(tab, &format!("b:{key}"), "Comment", "Leave a comment", true, move |v, cx| v.review_send_box(tab, k2.clone(), false, cx), cx) else { return div().into_any_element() };
    // A new box takes the keyboard when it mounts.
    if this.tabs.iter().find(|t| t.id == tab).is_some_and(|t| t.forge.review_ui.stop.as_deref() == Some(&format!("b:{key}"))) {
        field.update(cx, |f, cx| f.focus(window, cx));
        if let Some(t) = this.tabs.iter_mut().find(|t| t.id == tab) {
            t.forge.review_ui.stop = None;
        }
    }
    let t = this.tabs.iter().find(|t| t.id == tab);
    let busy = t.is_some_and(|t| t.forge.review_ui.busy.as_deref() == Some(key.as_str()));
    let error = t.and_then(|t| t.forge.review_ui.errors.get(&key).cloned());
    let can_draft = t.and_then(|t| t.forge.review.as_ref()).is_none_or(|s| s.can_draft);
    let empty = field.read(cx).text(cx).trim().is_empty();
    let blocked = stale;
    let reason = error.or(blocked.then(|| "This comment can't go now: compare again".to_string()));
    let (k_cancel, k_now, k_add) = (key.clone(), key.clone(), key.clone());
    let cancel = crate::ui::anchor(format!("rv-cancel:{key}"), mr_button(ElementId::Name(format!("rv-cancel:{key}").into()), true).child("Cancel").on_click(cx.listener(move |v, _, window, cx| v.review_cancel_box(tab, k_cancel.clone(), window, cx))));
    let suggest_on = b.suggestion.is_some();
    let suggestion = b.suggestion.clone();
    let f2 = field.clone();
    let suggest = mr_button(ElementId::Name(format!("rv-suggest:{key}").into()), suggest_on && !busy).child("Suggest change").when(suggest_on && !busy, |d| {
        d.on_click(cx.listener(move |_, _, _, cx| {
            let Some(s) = &suggestion else { return };
            let block = suggestion_block(kind, &s.split('\n').map(str::to_string).collect::<Vec<_>>());
            let now = f2.read(cx).text(cx);
            let text = if now.trim().is_empty() { block } else { format!("{}\n\n{block}", now.trim_end()) };
            f2.update(cx, |f, cx| f.set_text(&text, cx));
            cx.notify();
        }))
    });
    let suggest: AnyElement = if suggest_on { crate::ui::anchor(format!("rv-suggest:{key}"), suggest).into_any_element() } else { tip(NO_SUGGESTION, suggest).into_any_element() };
    let enabled = !empty && !busy && !blocked;
    let now = crate::ui::anchor(format!("rv-now:{key}"), mr_button(ElementId::Name(format!("rv-now:{key}").into()), enabled).child("Comment now").when(enabled, |d| d.on_click(cx.listener(move |v, _, _, cx| v.review_send_box(tab, k_now.clone(), true, cx)))));
    let add = crate::ui::anchor(format!("rv-add:{key}"), mr_primary(ElementId::Name(format!("rv-add:{key}").into()), enabled).child("Add to review").when(enabled, |d| d.on_click(cx.listener(move |v, _, _, cx| v.review_send_box(tab, k_add.clone(), false, cx)))));
    let k_esc = key.clone();
    let _ = can_draft;
    card_shell()
        .id(ElementId::Name(format!("rv-box:{key}").into()))
        .on_key_down(cx.listener(move |v, e: &KeyDownEvent, window, cx| {
            if e.keystroke.key == "escape" && !e.keystroke.modifiers.modified() {
                cx.stop_propagation();
                v.review_cancel_box(tab, k_esc.clone(), window, cx);
            }
        }))
        .child(crate::ui::anchor(format!("rv-field:{key}"), div().child(field)))
        .when_some(reason, |d, r| d.child(div().text_color(theme::red()).child(r)))
        .child(div().flex().justify_end().gap(px(6.)).child(cancel).child(suggest).child(now).child(add))
        .into_any_element()
}

/// `DraftCard`: a Pending mark (and Outdated, at an older head), its text, Edit and Delete, which arms in place first.
pub fn draft_card(this: &mut GitVolt, tab: u32, kind: ForgeKind, draft: &ReviewDraft, outdated: bool, cx: &mut Context<GitVolt>) -> AnyElement {
    let id = draft.id.clone();
    let key = format!("e:{id}");
    let t = this.tabs.iter().find(|t| t.id == tab);
    let editing = t.and_then(|t| t.forge.review_ui.fields.get(&key).cloned());
    let error = t.and_then(|t| t.forge.review_ui.errors.get(&key).cloned());
    let busy = t.is_some_and(|t| t.forge.review_ui.busy.as_deref() == Some(key.as_str()));
    let number = t.and_then(|t| t.forge.review.as_ref()).map_or(0, |s| s.number);
    let mut head = div().flex().items_center().gap(px(6.)).min_w_0().child(pending_chip()).when(outdated, |d| d.child(outdated_chip())).child(div().flex_1());
    if editing.is_none() {
        let (i1, body) = (id.clone(), draft.body.clone());
        let i2 = id.clone();
        head = head
            .child(tip("Edit", crate::ui::anchor(format!("rv-edit:{id}"), review_icon(format!("rv-edit:{id}"), "pencil").on_click(cx.listener(move |v, _, window, cx| v.review_start_edit(tab, i1.clone(), body.clone(), window, cx))))))
            .child(tip("Delete", crate::ui::anchor(format!("rv-delete:{id}"), review_icon(format!("rv-delete:{id}"), "trash-2").on_click(cx.listener(move |v, _, window, cx| v.review_remove_draft(tab, i2.clone(), window, cx))))));
    }
    let body: AnyElement = match editing {
        Some(f) => {
            let text = f.read(cx).text(cx);
            let changed = !text.trim().is_empty() && text != draft.body && !busy;
            let (i1, k1, k2) = (id.clone(), key.clone(), key.clone());
            div().flex().flex_col().gap(px(6.)).child(f).child(div().flex().justify_end().gap(px(6.)).child(crate::ui::anchor(format!("rv-edit-cancel:{id}"), mr_button(ElementId::Name(format!("rv-edit-cancel:{id}").into()), true).child("Cancel").on_click(cx.listener(move |v, _, _, cx| {
                if let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) {
                    t.forge.review_ui.fields.remove(&k2);
                }
                cx.notify();
            })))).child(crate::ui::anchor(format!("rv-edit-save:{id}"), mr_primary(ElementId::Name(format!("rv-edit-save:{id}").into()), changed).child("Save").when(changed, |d| d.on_click(cx.listener(move |v, _, _, cx| v.review_save_edit(tab, i1.clone(), k1.clone(), cx))))))).into_any_element()
        }
        None => match this.mr_body(tab, number, &format!("draft:{id}"), &draft.body, cx) {
            Some(v) => v.into_any_element(),
            None => div().child(draft.body.clone()).into_any_element(),
        },
    };
    let _ = kind;
    card_shell().id(ElementId::Name(format!("rv-draft:{id}").into())).child(head).child(body).when_some(error, |d, e| d.child(div().text_color(theme::red()).child(e))).into_any_element()
}

/// `ThreadCard`: the MR view's thread under a header that folds it to the gutter.
fn thread_card(this: &mut GitVolt, tab: u32, kind: ForgeKind, number: u64, thread: &ForgeDiscussion, outdated: bool, cx: &mut Context<GitVolt>) -> AnyElement {
    let t = this.tabs.iter().find(|t| t.id == tab);
    let mr = t.and_then(|t| t.forge.details.get(&number).map(|d| d.mr.clone()).or_else(|| t.forge.known_mr(number).cloned()));
    let Some(mr) = mr else { return div().into_any_element() };
    let Some(first) = thread.notes.iter().find(|n| !n.system) else { return div().into_any_element() };
    let name = first.author.name.clone();
    let id = thread.id.clone();
    let toggle = crate::ui::anchor(format!("rv-fold:{id}"), tip("Minimize to the gutter", div().id(ElementId::Name(format!("rv-fold:{id}").into())).flex().flex_none().size(px(18.)).items_center().justify_center().rounded(px(3.)).ml(px(-2.)).cursor_pointer().hover(|d| d.bg(theme::control_hover_bg())).child(icon("minus", 14., theme::text_secondary())).on_click(cx.listener(move |v, _, _, cx| {
        if let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) {
            t.forge.review_ui.folds.insert(id.clone(), false);
        }
        cx.notify();
    }))));
    let _ = name;
    let head = div().flex().items_center().gap(px(6.)).min_w_0().child(toggle).child(div().flex_1()).when(thread.resolved, |d| d.child(chip("Resolved", theme::mix(theme::green(), 0.16, transparent_black()), theme::green()))).when(outdated, |d| d.child(outdated_chip()));
    let body = crate::forge::mrview::thread::thread_in_diff(this, tab, kind, &mr, thread, cx);
    card_shell().id(ElementId::Name(format!("rv-thread:{}", thread.id).into())).child(head).children(body).into_any_element()
}

/// The zones' elements, the "+" and the folded threads' icons, over the rows.
pub fn overlays(diff: &DiffState, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    let mut out = Vec::new();
    let Some(review) = diff.review.as_ref() else { return out };
    let tab = review.tab;
    let scroll = diff.scroll;
    let height = diff.layout.height;
    let width = diff.body.get().size.width.as_f32();
    let split = diff.split_shown();
    let gutter = diff.gutter_w();
    let elements = std::mem::take(&mut *diff.zone_elements.borrow_mut());
    let heights = diff.zone_heights.clone();
    let mut by_key: HashMap<String, AnyElement> = elements.into_iter().collect();
    for (ix, r) in diff.rows.iter().enumerate() {
        let RowKind::Zone { zone } = r.kind else { continue };
        let Some(top) = diff.layout.offsets.get(ix).map(|t| t - scroll) else { continue };
        let Some(z) = diff.zones.get(zone as usize) else { continue };
        let Some(el) = by_key.remove(&z.key) else { continue };
        let measured = heights.borrow().contains_key(&z.key);
        // What's far from the view keeps its last height, and draws nothing.
        if measured && (top > height + 600. || top + z.height < -600.) {
            continue;
        }
        let (left, w) = if split {
            let half = width / 2.;
            if z.new_side { (half + gutter - crate::diff::view::SPLIT_LEFT_MARGIN, half - gutter + crate::diff::view::SPLIT_LEFT_MARGIN - CARD_RIGHT) } else { (gutter, half - gutter - CARD_RIGHT) }
        } else {
            (gutter, width - gutter - CARD_RIGHT)
        };
        let key = z.key.clone();
        let cell = heights.clone();
        out.push(
            div()
                .on_children_prepainted(move |bounds, window, _| {
                    let Some(b) = bounds.first() else { return };
                    let h = b.size.height.as_f32() + CARD_MARGIN;
                    let was = cell.borrow().get(&key).copied();
                    if was.is_none_or(|w| (w - h).abs() >= 0.5) {
                        cell.borrow_mut().insert(key.clone(), h);
                        window.refresh();
                    }
                })
                .id(ElementId::Name(format!("zone:{}", z.key).into()))
                .absolute()
                .left(px(left))
                .top(px(top + 4.))
                .w(px(w.max(120.)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .child(el)
                .into_any_element(),
        );
    }
    let (x_old, x_new) = diff.glyph_columns();
    // Folded threads: their first author's avatar at the line, a check on a resolved one, a count when several fold there.
    for ((new_side, line), threads) in &review.icons {
        let row = diff.rows.iter().position(|r| !matches!(r.kind, RowKind::Hidden { .. } | RowKind::Zone { .. }) && if *new_side { r.new == Some(line - 1) } else { r.old == Some(line - 1) });
        let Some(row) = row else { continue };
        let Some(top) = diff.layout.offsets.get(row).map(|t| t - scroll) else { continue };
        if top < -30. || top > height {
            continue;
        }
        let d = &threads[0];
        let Some(first) = d.notes.iter().find(|n| !n.system) else { continue };
        let excerpt = first.body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
        let more = threads.len() - 1;
        let text = format!("{} by {}{}{}", if d.resolved { "Resolved thread" } else { "Thread" }, first.author.name, if excerpt.is_empty() { String::new() } else { format!(": {excerpt}") }, if more > 0 { format!(" (and {more} more {})", if more == 1 { "thread" } else { "threads" }) } else { String::new() });
        let ids: Vec<String> = threads.iter().map(|t| t.id.clone()).collect();
        let x = if *new_side { x_new } else { x_old };
        let size = (crate::metrics::line_h() - 2.).max(12.);
        out.push(
            tip(text, crate::ui::anchor(format!("rv-icon:{new_side}:{line}"), div()
                .id(ElementId::Name(format!("rv-icon:{new_side}:{line}").into()))
                .absolute()
                .left(px(x))
                .top(px(top + 1.))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(cx.listener(move |v, _, _, cx| {
                    if let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) {
                        for id in &ids {
                            t.forge.review_ui.folds.insert(id.clone(), true);
                        }
                    }
                    cx.notify();
                }))
                .child(forge_avatar(&first.author, size))
                .when(d.resolved, |e| e.child(div().absolute().right(px(-2.)).bottom(px(-1.)).flex().size(px(10.)).items_center().justify_center().rounded_full().bg(theme::green()).child(icon("check", 7., theme::text_on_accent()))))
                .when(threads.len() > 1, |e| e.child(div().absolute().right(px(-3.)).top(px(-2.)).flex().min_w(px(11.)).h(px(11.)).px(px(2.)).items_center().justify_center().rounded(px(6.)).bg(theme::blue()).text_size(px(8.)).font_weight(FontWeight::BOLD).text_color(theme::text_on_accent()).child(threads.len().to_string()))))).placement(Placement::Right).into_any_element(),
        );
    }
    // The "+": on the hovered line that takes a comment; a click picks its line, a drag from it the lines it goes over, on its side.
    if let Some((row, _)) = diff.hover
        && let Some(r) = diff.rows.get(row).filter(|r| !matches!(r.kind, RowKind::Zone { .. } | RowKind::Hidden { .. }))
    {
        let top = diff.layout.offsets.get(row).map_or(-1000., |t| t - scroll);
        let at = |new_side: bool, line: u32| review.icons.contains_key(&(new_side, line));
        let old_on = r.old.map(|n| n + 1).filter(|n| review.old.contains(n) && !at(false, *n) && (split || r.new.is_none()));
        let new_on = r.new.map(|n| n + 1).filter(|n| review.new.contains(n) && !at(true, *n));
        let mut glyph = |new_side: bool, line: u32, x: f32, cx: &mut Context<GitVolt>| {
            out.push(
                tip(GLYPH_TIP, crate::ui::anchor(if new_side { "rv-plus-new" } else { "rv-plus-old" }, div()
                    .id(if new_side { "rv-plus-new" } else { "rv-plus-old" })
                    .absolute()
                    .left(px(x))
                    .top(px(top + (crate::metrics::line_h() - 14.) / 2.))
                    .flex()
                    .size(px(14.))
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .bg(theme::blue())
                    .text_color(theme::text_on_accent())
                    .text_size(px(13.))
                    .font_weight(FontWeight::BOLD)
                    .cursor_pointer()
                    .child("+")
                    .on_mouse_down(MouseButton::Left, cx.listener(move |v, _, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                        v.review_press(new_side, line, cx);
                    }))
                    .on_mouse_up(MouseButton::Left, cx.listener(|v, _, _, cx| {
                        cx.stop_propagation();
                        v.review_release(cx);
                    })))).placement(Placement::Right).into_any_element(),
            );
        };
        if let Some(n) = old_on {
            glyph(false, n, x_old, cx);
        }
        if let Some(n) = new_on {
            glyph(true, n, x_new, cx);
        }
    }
    if let Some(banner) = &review.banner {
        out.push(div().absolute().top(px(6.)).left(px(12.)).max_w(px(width.max(200.) - 470.)).flex().items_center().gap(px(8.)).px(px(10.)).py(px(4.)).rounded(px(6.)).border_1().border_color(theme::section_border()).bg(theme::panel_bg1()).text_size(px(12.)).text_color(theme::orange()).child(icon("triangle-alert", 12., theme::orange())).child(banner.clone()).into_any_element());
    }
    out
}

impl GitVolt {
    fn review_press(&mut self, new_side: bool, line: u32, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        t.forge.review_ui.drag = Some((new_side, line, line));
        if let Some(d) = t.diff.as_mut() {
            d.review_drag = Some((new_side, line, line));
        }
        cx.notify();
    }

    fn review_release(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        let tab = t.id;
        let Some((new_side, a, b)) = t.forge.review_ui.drag.take() else { return };
        if let Some(d) = t.diff.as_mut() {
            d.review_drag = None;
        }
        self.review_pick(tab, new_side, a, b, cx);
    }

    /// The pointer moved with a press from the "+": the lines it's gone over; or it was let go anywhere.
    pub fn review_drag_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) -> bool {
        let Some(t) = self.tabs.get_mut(self.active) else { return false };
        let Some((new_side, from, _)) = t.forge.review_ui.drag else { return false };
        if e.pressed_button != Some(MouseButton::Left) {
            self.review_release(cx);
            return true;
        }
        let Some(d) = t.diff.as_mut() else { return false };
        let b = d.body.get();
        let row = d.layout.row_at((e.position.y - b.origin.y).as_f32() + d.scroll);
        let line = d.rows.get(row).and_then(|r| if new_side { r.new } else { r.old }).map(|n| n + 1);
        if let Some(line) = line {
            t.forge.review_ui.drag = Some((new_side, from, line));
            d.review_drag = Some((new_side, from, line));
            cx.notify();
        }
        true
    }
}

/// The top bar's chip: "Reviewing !12 · 3 pending", while the session has drafts or a pending review. A click goes back to the MR's view and Compare; its caret submits or discards.
pub fn review_chip(this: &GitVolt, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let t = this.tabs.get(this.active)?;
    let s = t.forge.review.as_ref().filter(|s| is_pending(Some(s)))?;
    let kind = t.forge.kind?;
    let tab = t.id;
    let number = s.number;
    let mr = t.forge.known_mr(number);
    let r = mr_ref(kind, number);
    let ended = if s.closed { mr.and_then(|m| match m.state { gitvolt_core::forge::types::MrState::Merged => Some("merged"), gitvolt_core::forge::types::MrState::Closed => Some("closed"), _ => None }) } else { None };
    let value = format!("{r}{} · {}", ended.map_or(String::new(), |e| format!(" {e}")), pending_label(s));
    let problem = ended.map(|e| format!("{r} was {e} with your review pending: submit it anyway, or discard it")).or(s.error.as_ref().map(|e| format!("Couldn't read your pending review: {e}")));
    let tooltip = problem.clone().unwrap_or_else(|| format!("Back to {r}'s changes and your pending review"));
    let color = if problem.is_some() { theme::orange() } else { theme::review_pending() };
    let what = pending_text(s.drafts.len());
    let name = forge_name(kind);
    let key = "tb-review-caret";
    let weak = cx.weak_entity();
    let m2 = mr.cloned();
    Some(
        div()
            .flex()
            .flex_none()
            .items_stretch()
            .h(px(38.))
            .rounded(px(6.))
            .border_1()
            .border_color(if problem.is_some() { theme::mix(theme::orange(), 0.45, transparent_black()) } else { theme::mix(theme::review_pending(), 0.4, transparent_black()) })
            .when(problem.is_none(), |d| d.bg(theme::review_pending_bg()))
            .child(tip(tooltip, crate::ui::anchor("tb-review", div().id("tb-review").flex().flex_col().justify_center().px(px(10.)).rounded_l(px(4.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).child(div().text_size(px(10.)).line_height(px(14.)).text_color(if problem.is_some() { theme::text_secondary() } else { color }).child("REVIEWING")).child(div().flex().items_center().gap(px(3.)).line_height(px(18.)).font_weight(FontWeight::SEMIBOLD).text_color(color).child(icon("message-square", 12., color)).child(value).when(problem.is_some(), |d| d.child(icon("triangle-alert", 12., color)))).on_click(cx.listener(move |v, _, _, cx| {
                v.forge_open_mr(tab, number, cx);
                if let Some(m) = m2.clone() {
                    let detail = v.tabs.iter().find(|t| t.id == tab).and_then(|t| t.forge.details.get(&number).cloned());
                    v.mr_comparing(tab, number, true);
                    v.mr_compare(tab, kind, m, detail, cx);
                }
            })))))
            .child(crate::ui::anchor(key, div().id(key).flex().flex_none().w(px(16.)).items_center().justify_center().rounded_r(px(4.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).child(icon("chevron-down", 12., color)).on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                let (w1, w2, what2) = (weak.clone(), weak.clone(), what.clone());
                let r2 = r.clone();
                let rows = vec![
                    MenuRow::action("review.submitMenu", "Submit review…", "send", format!("Send {what} on {r2} as one review: Comment, Approve or Request changes"), move |_, cx| {
                        w1.update(cx, |v, cx| v.review_toggle_submit(cx)).ok();
                    }),
                    MenuRow::action("review.discard", "Discard pending review", "trash-2", format!("Delete {what2} on {r2} from {name}"), move |window, cx| {
                        let origin = Origin::current(cx);
                        w2.update(cx, |v, cx| v.review_discard(tab, kind, origin, window, cx)).ok();
                    }),
                ];
                menu::open_below(key, rows, Default::default(), window, cx);
            })))
            .into_any_element(),
    )
}

impl GitVolt {
    /// Mod+Alt+R, and the chip's Submit review…: again, it closes.
    pub fn review_toggle_submit(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        let Some(n) = t.forge.review.as_ref().map(|s| s.number) else { return };
        t.forge.review_ui.submit_open = if t.forge.review_ui.submit_open.is_some() { None } else { Some(n) };
        let tab = t.id;
        t.mr_views.entry(n).or_default();
        if self.tabs.iter().find(|t| t.id == tab).is_some_and(|t| t.forge.review_ui.submit_open.is_some()) {
            let (input, _) = self.mr_composer(tab, n, cx);
            self.focus_next_frame = Some(input.read(cx).focus.clone());
        }
        cx.notify();
    }
}

/// Submit review…: the MR/PR view's review composer, under the chip. Above it, the pending comments on no line of the diff.
pub fn submit_popover(this: &mut GitVolt, window: &mut Window, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let t = this.tabs.get(this.active)?;
    let tab = t.id;
    let number = t.forge.review_ui.submit_open?;
    let kind = t.forge.kind?;
    let session = t.forge.review.clone();
    let threads = t.forge.discussions.get(&number).cloned().unwrap_or_default();
    let at = crate::ui::Ui::get(cx).anchor("tb-review").map_or(point(px(190.), px(80.)), |b| point(b.left(), b.bottom() + px(4.)));
    let loose = session.as_ref().filter(|s| s.number == number).map_or(Vec::new(), |s| place_review(s, &threads).unplaced);
    let cards: Vec<AnyElement> = loose.iter().map(|d| draft_card(this, tab, kind, d, false, cx)).collect();
    let composer = crate::forge::mrview::actions::review_composer(this, tab, kind, number, false, window, cx);
    let sending = this.tabs.get(this.active).and_then(|t| t.mr_views.get(&number)).is_some_and(|u| u.review.busy);
    Some(
        deferred(
            div()
                .id("review-submit")
                .occlude()
                .absolute()
                .left(at.x)
                .top(at.y)
                .w(px(440.))
                .p(px(10.))
                .bg(theme::panel_bg1())
                .border_1()
                .border_color(theme::section_border())
                .rounded(px(8.))
                .shadow(crate::ui::shadow(theme::shadow_modal(), 8., 24.))
                .text_color(theme::text_normal())
                .on_mouse_down_out(cx.listener(move |v, _, _, cx| {
                    if !sending && let Some(t) = v.tabs.get_mut(v.active) {
                        t.forge.review_ui.submit_open = None;
                        cx.notify();
                    }
                }))
                .on_key_down(cx.listener(move |v, e: &KeyDownEvent, _, cx| {
                    if e.keystroke.key == "escape" && !sending {
                        cx.stop_propagation();
                        if let Some(t) = v.tabs.get_mut(v.active) {
                            t.forge.review_ui.submit_open = None;
                        }
                        cx.notify();
                    }
                }))
                .when(!cards.is_empty(), |d| d.child(div().id("review-loose").flex().flex_col().max_h(px(300.)).overflow_y_scroll().mb(px(8.)).pb(px(4.)).border_b_1().border_color(theme::section_border()).children(cards)))
                .child(composer),
        )
        .with_priority(3)
        .into_any_element(),
    )
}

/// A file row's review badge: a bubble and a count, highlighted while a thread on the file is unresolved. A press opens the file at its next thread.
pub fn row_badge(this: &GitVolt, path: &str, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let t = this.tabs.get(this.active)?;
    let s = t.forge.review.as_ref()?;
    let c = s.compare.as_ref()?;
    let shown = t.compare.as_ref().is_some_and(|x| x.from == c.0 && x.to.sha() == Some(c.1.as_str()));
    if !shown || compare_stale(s) {
        return None;
    }
    let threads = t.forge.discussions.get(&s.number).cloned().unwrap_or_default();
    let placed = place_review(s, &threads);
    let badge = super::file_badge(placed.by_path.get(path))?;
    let on = badge.unresolved > 0;
    let count = badge.count;
    let label = super::badge_label(&badge);
    let (tab, path2) = (t.id, path.to_string());
    Some(
        tip(label, crate::ui::anchor(format!("rv-badge:{path}"), div().id(ElementId::Name(format!("rv-badge:{path}").into())).flex().flex_none().items_center().gap(px(2.)).ml(px(6.)).px(px(5.)).h(px(16.)).rounded(px(8.)).text_size(px(10.)).bg(if on { theme::blue() } else { theme::control_bg() }).text_color(if on { theme::text_on_accent() } else { theme::text_secondary() }).child(icon("message-square", 11., if on { theme::text_on_accent() } else { theme::text_secondary() })).child(count.to_string()).on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(cx.listener(move |v, _, _, cx| {
            cx.stop_propagation();
            let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) else { return };
            let was = t.forge.review_ui.turns.get(&path2).cloned();
            let at = match was {
                Some((sig, at)) if sig == badge.sig => (at + 1) % badge.stops.len(),
                _ => 0,
            };
            t.forge.review_ui.turns.insert(path2.clone(), (badge.sig.clone(), at));
            let stop = badge.stops[at].clone();
            let line = (stop.side == DiffSide::New, stop.start_line.unwrap_or(stop.line), stop.start_line.map(|_| stop.line));
            v.open_compare_file_at(tab, &path2, Some(line), cx);
        })))).into_any_element(),
    )
}

/// The lines a drag from the "+" has gone over: their tint, under the rows' text.
pub fn drag_marks(diff: &DiffState) -> Vec<AnyElement> {
    let Some((new_side, a, b)) = diff.review_drag else { return Vec::new() };
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let mut out = Vec::new();
    for (ix, r) in diff.rows.iter().enumerate() {
        let n = if new_side { r.new } else { r.old }.map(|n| n + 1);
        if n.is_some_and(|n| (lo..=hi).contains(&n)) {
            let (Some(top), Some(end)) = (diff.layout.offsets.get(ix).map(|t| t - diff.scroll), diff.layout.offsets.get(ix + 1).map(|t| t - diff.scroll)) else { continue };
            out.push(div().absolute().left_0().right_0().top(px(top)).h(px(end - top)).bg(Hsla { a: 0.15, ..theme::blue() }).into_any_element());
        }
    }
    out
}

/// What a comment's kind says, for a test of the words.
pub fn kind_word(k: LineKind) -> &'static str {
    match k {
        LineKind::Added => "added",
        LineKind::Removed => "removed",
        LineKind::Context => "context",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn row(old: Option<u32>, new: Option<u32>) -> Row {
        Row { old, new, kind: RowKind::Same }
    }

    fn slot(key: &str, new_side: bool, line: u32) -> ZoneSlot {
        ZoneSlot { key: key.into(), new_side, line, height: 40. }
    }

    #[test]
    fn a_zone_goes_under_its_line_and_an_old_lines_under_its_removed_block() {
        let rows = vec![row(Some(0), Some(0)), row(Some(1), None), row(Some(2), None), row(None, Some(1)), row(Some(3), Some(2))];
        let out = with_zones(rows.clone(), &[slot("n", true, 2)], false);
        assert!(matches!(out[4].kind, RowKind::Zone { zone: 0 }), "under the added line");
        let out = with_zones(rows.clone(), &[slot("o", false, 2)], false);
        assert!(matches!(out[3].kind, RowKind::Zone { zone: 0 }), "under the whole removed block");
        let out = with_zones(rows, &[slot("o", false, 2)], true);
        assert!(matches!(out[2].kind, RowKind::Zone { zone: 0 }), "split: right under its line");
    }

    #[test]
    fn a_suggestion_says_what_each_forge_replaces() {
        let lines = vec!["a".to_string(), "b".to_string()];
        assert_eq!(suggestion_block(ForgeKind::GitHub, &lines), "```suggestion\na\nb\n```");
        assert_eq!(suggestion_block(ForgeKind::GitLab, &lines), "```suggestion:-1+0\na\nb\n```");
        assert_eq!(suggestion_block(ForgeKind::Forgejo, &["```".to_string()]), "````suggestion\n```\n````");
    }
}
