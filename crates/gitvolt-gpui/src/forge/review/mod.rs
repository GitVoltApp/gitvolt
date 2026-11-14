//! Review comments (`forge/review/`): the tab's session, which lines take a comment, where a new one goes, and where the MR's threads and the
//! user's drafts sit. The session's writes go through the core; a write that works updates it at once, then reads the forge back; one the forge
//! refuses changes nothing and gives the reason to the box that asked.

pub mod apply;
pub mod zones;

use crate::app::{GitVolt, core_repo};
use gitvolt_core::api::Request;
use gitvolt_core::forge::types::{DiffPosition, DiffRefs, DiffSide, ForgeDiscussion, ForgeMr, LineKind, MrState, NewReviewComment, ReviewAnchor, ReviewDiff, ReviewDraft, ReviewDrafts, ReviewFile, ReviewLine, SubmitOutcome};
use gitvolt_core::forge::ForgeKind;
use gpui::*;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, PartialEq)]
pub struct CommentableFile {
    pub path: String,
    pub old_path: String,
    pub too_large: bool,
    pub lines: Vec<ReviewLine>,
    pub old: HashMap<u32, ReviewLine>,
    pub new: HashMap<u32, ReviewLine>,
}

pub fn commentable_index(file: &ReviewFile) -> CommentableFile {
    let mut out = CommentableFile { path: file.path.clone(), old_path: file.old_path.clone(), too_large: file.too_large, lines: file.lines.clone(), old: HashMap::new(), new: HashMap::new() };
    for l in &file.lines {
        if l.kind != LineKind::Added {
            out.old.insert(l.old_line, *l);
        }
        if l.kind != LineKind::Removed {
            out.new.insert(l.new_line, *l);
        }
    }
    out
}

impl CommentableFile {
    pub fn at(&self, side: DiffSide, line: u32) -> Option<&ReviewLine> {
        match side {
            DiffSide::Old => self.old.get(&line),
            DiffSide::New => self.new.get(&line),
        }
    }
}

/// Two lines in a row are in one hunk: neither counter jumps.
fn same_hunk(a: &ReviewLine, b: &ReviewLine) -> bool {
    b.old_line.saturating_sub(a.old_line) <= 1 && b.new_line.saturating_sub(a.new_line) <= 1
}

/// `anchorFor`: where a comment on lines `from`..`to` (either order) of `side` goes: the first and last of them that take a comment, within the
/// hunk of the last one (a range can't span lines the diff doesn't show).
pub fn anchor_for(file: &CommentableFile, side: DiffSide, from: u32, to: u32) -> Option<ReviewAnchor> {
    let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
    let at = |l: &ReviewLine| if side == DiffSide::Old { l.old_line } else { l.new_line };
    let mut hunk = 0;
    let mut picked: Vec<(&ReviewLine, usize)> = Vec::new();
    for (i, l) in file.lines.iter().enumerate() {
        if i > 0 && !same_hunk(&file.lines[i - 1], l) {
            hunk += 1;
        }
        if file.at(side, at(l)) == Some(l) && (lo..=hi).contains(&at(l)) {
            picked.push((l, hunk));
        }
    }
    let last = picked.last()?;
    let run: Vec<&(&ReviewLine, usize)> = picked.iter().filter(|p| p.1 == last.1).collect();
    Some(ReviewAnchor { path: file.path.clone(), old_path: file.old_path.clone(), start: if run.len() > 1 { Some(*run[0].0) } else { None }, end: *last.0 })
}

/// `anchorSpan`: the lines a comment on `a` covers on the side it lands on, in that side's numbers: the last line decides the side, as the forge places it.
pub fn anchor_span(file: &CommentableFile, a: &ReviewAnchor) -> (DiffSide, u32, u32) {
    let side = a.end.side();
    let to = a.end.number();
    let Some(start) = a.start else { return (side, to, to) };
    if side == DiffSide::Old {
        return (side, start.old_line, to);
    }
    if start.kind != LineKind::Removed {
        return (side, start.new_line, to);
    }
    let i = file.lines.iter().position(|l| l.kind == start.kind && l.old_line == start.old_line);
    let first = i.and_then(|i| file.lines[i..].iter().find(|l| l.kind != LineKind::Removed));
    (side, first.map(|f| f.new_line).filter(|n| *n <= to).unwrap_or(to), to)
}

/// Where a thread or a draft sits in the diff.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub path: String,
    pub side: DiffSide,
    pub line: u32,
    pub start_line: Option<u32>,
    pub outdated: bool,
}

pub fn is_outdated(pos: &DiffPosition, refs: Option<&DiffRefs>) -> bool {
    pos.outdated || refs.is_some_and(|r| pos.head_sha.as_ref().is_some_and(|h| *h != r.head_sha))
}

/// `placeOf`: an outdated published thread sits nowhere (it stays on the timeline); an outdated draft sits at its line, marked, while that's one of the diff's.
pub fn place_of(pos: &DiffPosition, refs: Option<&DiffRefs>, file: Option<&CommentableFile>, draft: bool) -> Option<Placement> {
    let side = if pos.line.is_some() { DiffSide::New } else { DiffSide::Old };
    let line = if side == DiffSide::New { pos.line } else { pos.old_line }?;
    let outdated = is_outdated(pos, refs);
    if outdated && (!draft || file.and_then(|f| f.at(side, line)).is_none()) {
        return None;
    }
    let start = if side == DiffSide::New { pos.start_line } else { pos.start_old_line };
    Some(Placement { path: pos.path.clone(), side, line, start_line: start.filter(|s| *s < line), outdated })
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlacedItem {
    Thread { thread: ForgeDiscussion, at: Placement },
    Draft { draft: ReviewDraft, at: Placement },
}

impl PlacedItem {
    pub fn at(&self) -> &Placement {
        match self {
            PlacedItem::Thread { at, .. } | PlacedItem::Draft { at, .. } => at,
        }
    }

    /// A card shows for it: a draft, or a thread with a comment.
    pub fn shown(&self) -> bool {
        match self {
            PlacedItem::Draft { .. } => true,
            PlacedItem::Thread { thread, .. } => thread.notes.iter().any(|n| !n.system),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReviewPlacements {
    /// By file (its new path), in line order.
    pub by_path: BTreeMap<String, Vec<PlacedItem>>,
    /// Drafts on no line of this diff: Submit review… lists them, the chip counts them, and the review sends them.
    pub unplaced: Vec<ReviewDraft>,
}

pub fn place_review(s: &ReviewSession, threads: &[ForgeDiscussion]) -> ReviewPlacements {
    let mut out = ReviewPlacements::default();
    for t in threads {
        let Some(p) = t.notes.iter().find_map(|n| n.position.as_ref()) else { continue };
        if let Some(at) = place_of(p, s.refs.as_ref(), s.files.get(&p.path), false) {
            out.by_path.entry(at.path.clone()).or_default().push(PlacedItem::Thread { thread: t.clone(), at });
        }
    }
    for d in &s.drafts {
        let at = d.position.as_ref().and_then(|p| place_of(p, s.refs.as_ref(), s.files.get(&p.path), true));
        match at.filter(|a| s.diff_head.is_none() || s.files.contains_key(&a.path)) {
            Some(at) => out.by_path.entry(at.path.clone()).or_default().push(PlacedItem::Draft { draft: d.clone(), at }),
            None => out.unplaced.push(d.clone()),
        }
    }
    for items in out.by_path.values_mut() {
        items.sort_by_key(|i| (i.at().line, i.at().side != DiffSide::Old));
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewSession {
    pub number: u64,
    pub kind: ForgeKind,
    pub compare: Option<(String, String)>,
    pub refs: Option<DiffRefs>,
    pub files: HashMap<String, CommentableFile>,
    pub diff_head: Option<String>,
    pub drafts: Vec<ReviewDraft>,
    pub pending_review: Option<String>,
    pub can_draft: bool,
    pub closed: bool,
    pub error: Option<String>,
    pub loaded: bool,
}

pub fn new_session(kind: ForgeKind, number: u64, compare: Option<(String, String)>) -> ReviewSession {
    ReviewSession { number, kind, compare, refs: None, files: HashMap::new(), diff_head: None, drafts: Vec::new(), pending_review: None, can_draft: true, closed: false, error: None, loaded: false }
}

/// Whether a session stays: until its first refresh answers (unless it failed); while a review is pending; else not once its MR merged or closed, and otherwise while its Compare is shown.
pub fn review_alive(s: &ReviewSession, compare_open: bool, mr: Option<&ForgeMr>) -> bool {
    if (!s.loaded && s.error.is_none()) || !s.drafts.is_empty() || s.pending_review.is_some() {
        return true;
    }
    if mr.is_some_and(|m| matches!(m.state, MrState::Merged | MrState::Closed)) {
        return false;
    }
    compare_open
}

pub fn is_pending(s: Option<&ReviewSession>) -> bool {
    s.is_some_and(|s| !s.drafts.is_empty() || s.pending_review.is_some())
}

/// "3 pending", or "review pending" (GitHub's pending review with no comment yet).
pub fn pending_label(s: &ReviewSession) -> String {
    if s.drafts.is_empty() { "review pending".to_string() } else { format!("{} pending", s.drafts.len()) }
}

pub fn pending_text(n: usize) -> String {
    match n {
        0 => "your pending review".to_string(),
        1 => "1 pending comment".to_string(),
        _ => format!("{n} pending comments"),
    }
}

/// What went in before a part-way review: the summary note, the event.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Sent {
    pub body_posted: bool,
    pub event_sent: bool,
}

/// `retryPlan`: what a retry of a part-way review sends, so nothing goes in twice.
pub fn retry_plan(event: &str, body: &str, sent: Option<Sent>) -> (String, String) {
    if let Some(s) = sent {
        if s.body_posted && !s.event_sent {
            return (event.to_string(), String::new());
        }
        if s.event_sent && !s.body_posted && !body.trim().is_empty() {
            return ("comment".to_string(), body.to_string());
        }
    }
    (event.to_string(), body.to_string())
}

pub fn retry_note(sent: Option<Sent>) -> &'static str {
    match sent {
        Some(s) if s.body_posted && !s.event_sent => " Your summary is already posted, so trying again sends only the rest.",
        Some(s) if s.event_sent && !s.body_posted => " Your approval went in, so trying again posts only your summary.",
        _ => "",
    }
}

/// A file's badge in the file list.
#[derive(Debug, Clone, PartialEq)]
pub struct FileBadge {
    pub count: usize,
    pub threads: usize,
    pub drafts: usize,
    pub unresolved: usize,
    pub stops: Vec<Placement>,
    pub sig: String,
}

pub fn file_badge(all: Option<&Vec<PlacedItem>>) -> Option<FileBadge> {
    let items: Vec<&PlacedItem> = all?.iter().filter(|i| i.shown()).collect();
    if items.is_empty() {
        return None;
    }
    fn unresolved(i: &PlacedItem) -> bool {
        matches!(i, PlacedItem::Thread { thread, .. } if thread.resolvable && !thread.resolved)
    }
    let open: Vec<&PlacedItem> = items.iter().filter(|i| unresolved(i)).copied().collect();
    let threads = items.iter().filter(|i| matches!(i, PlacedItem::Thread { .. })).count();
    let mut order = open.clone();
    order.extend(items.iter().filter(|i| !unresolved(i)).copied());
    let sig = order.iter().map(|i| match i {
        PlacedItem::Thread { thread, at } => format!("t{}{}@{:?}{}", thread.id, if thread.resolved { "+" } else { "" }, at.side, at.line),
        PlacedItem::Draft { draft, at } => format!("d{}@{:?}{}", draft.id, at.side, at.line),
    }).collect::<Vec<_>>().join(" ");
    Some(FileBadge { count: items.len(), threads, drafts: items.len() - threads, unresolved: open.len(), stops: order.iter().map(|i| i.at().clone()).collect(), sig })
}

pub fn badge_label(b: &FileBadge) -> String {
    let mut parts = Vec::new();
    if b.threads > 0 {
        parts.push(if b.threads == 1 { "1 thread".to_string() } else { format!("{} threads", b.threads) });
    }
    if b.unresolved > 0 {
        parts.push(format!("{} unresolved", b.unresolved));
    }
    if b.drafts > 0 {
        parts.push(format!("{} pending", b.drafts));
    }
    parts.join(", ")
}

impl GitVolt {
    fn compare_shown(&self, tab: u32, s: &ReviewSession) -> bool {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return false };
        let (Some((from, to)), Some(c)) = (&s.compare, &t.compare) else { return false };
        let Some(c_to) = c.to.sha() else { return false };
        (c.from == *from && c_to == to) || (c.from == *to && c_to == from)
    }

    /// `startReview`: starts (or for the same MR re-anchors) the tab's session on its Compare, and reads it. Another MR's session gives way: its drafts stay on the forge, and its Compare finds them again.
    pub fn review_start(&mut self, tab: u32, kind: ForgeKind, number: u64, compare: (String, String), cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        match t.forge.review.as_mut().filter(|r| r.number == number) {
            Some(r) => r.compare = Some(compare),
            None => {
                t.forge.review_refs = None;
                t.forge.review = Some(new_session(kind, number, Some(compare)));
            }
        }
        self.review_refresh(tab, cx);
    }

    /// `resumeReview`: a review left pending outside this session: read once per open, and any drafts start a session without a Compare.
    pub fn review_resume(&mut self, tab: u32, number: u64, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some(kind) = t.forge.kind else { return };
        let taken = move |v: &GitVolt| -> bool {
            let Some(cur) = v.tabs.iter().find(|t| t.id == tab).and_then(|t| t.forge.review.clone()) else { return false };
            cur.number == number || !cur.drafts.is_empty() || cur.pending_review.is_some() || v.compare_shown(tab, &cur)
        };
        if taken(self) {
            return;
        }
        self.forge_ask::<ReviewDrafts>(Request::ForgeReviewDrafts { repo: core_repo(tab), number }, cx, move |v, answer, _| {
            let Ok(state) = answer else { return };
            if (state.drafts.is_empty() && state.pending_review.is_none()) || taken(v) {
                return;
            }
            let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) else { return };
            t.forge.review_refs = None;
            t.forge.review = Some(ReviewSession { drafts: state.drafts, pending_review: state.pending_review, can_draft: state.can_draft, loaded: true, ..new_session(kind, number, None) });
        });
    }

    /// `refreshReview`: the user's drafts every time, the diff's refs and commentable lines once per set of refs. A failure is the session's `error`, what's shown stays. An answer that may predate a write's is dropped.
    pub fn review_refresh(&mut self, tab: u32, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some(s) = t.forge.review.as_ref() else { return };
        let (number, epoch, seen) = (s.number, t.forge.write_epoch, t.forge.review_refs.clone());
        let (api, rt, repo) = (self.api.clone(), self.rt.clone(), core_repo(tab));
        let weak = cx.weak_entity();
        if cfg!(test) {
            return;
        }
        cx.spawn(async move |_, cx| {
            let state = super::ask::<ReviewDrafts>(api.clone(), rt.clone(), Request::ForgeReviewDrafts { repo, number }).await;
            let asked = state.as_ref().ok().and_then(|s| s.refs.as_ref()).map(|r| format!("{number} {} {} {}", r.base_sha, r.start_sha, r.head_sha));
            let (mut diff, mut diff_error) = (None, None);
            if asked.is_some() && asked != seen {
                match super::ask::<ReviewDiff>(api, rt, Request::ForgeReviewDiff { repo, number }).await {
                    Ok(d) => diff = Some(d),
                    Err(e) => diff_error = Some(e),
                }
            }
            weak.update(cx, |v, cx| {
                let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) else { return };
                if t.forge.write_epoch != epoch {
                    return;
                }
                let Some(cur) = t.forge.review.as_mut().filter(|r| r.number == number) else { return };
                match state {
                    Err(e) => cur.error = Some(e),
                    Ok(state) => {
                        cur.drafts = state.drafts;
                        cur.pending_review = state.pending_review;
                        cur.can_draft = state.can_draft;
                        cur.error = diff_error;
                        cur.loaded = true;
                        if let Some(d) = diff {
                            t.forge.review_refs = asked;
                            cur.files = d.files.iter().map(|f| (f.path.clone(), commentable_index(f))).collect();
                            cur.diff_head = Some(d.refs.head_sha.clone());
                            cur.refs = Some(d.refs);
                        }
                    }
                }
                v.review_settle(tab, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// `settleReview`: ends the session once nothing keeps it; else notes its MR merged or closed under pending drafts.
    pub fn review_settle(&mut self, tab: u32, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some(s) = t.forge.review.clone() else { return };
        let mr = t.forge.known_mr(s.number).cloned();
        let shown = self.compare_shown(tab, &s);
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        if !review_alive(&s, shown, mr.as_ref()) {
            t.forge.review = None;
            t.forge.review_refs = None;
            cx.notify();
            return;
        }
        if let Some(r) = t.forge.review.as_mut() {
            r.closed = mr.is_some_and(|m| matches!(m.state, MrState::Merged | MrState::Closed));
        }
    }

    /// A review write through the session. The answer's reason goes to `then`.
    fn review_write<T: serde::de::DeserializeOwned + Send + 'static>(&mut self, tab: u32, req: Request, cx: &mut Context<Self>, then: impl FnOnce(&mut GitVolt, Result<T, String>, &mut Context<GitVolt>) + 'static) {
        self.forge_ask::<T>(req, cx, move |v, answer, cx| {
            if answer.is_ok()
                && let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab)
            {
                t.forge.write_epoch += 1;
            }
            then(v, answer, cx);
        });
    }

    pub fn review_add_draft(&mut self, tab: u32, anchor: ReviewAnchor, body: String, cx: &mut Context<Self>, then: impl FnOnce(&mut GitVolt, Result<(), String>, &mut Context<GitVolt>) + 'static) {
        let Some((number, refs)) = self.tabs.iter().find(|t| t.id == tab).and_then(|t| t.forge.review.as_ref()).and_then(|s| Some((s.number, s.refs.clone()?))) else {
            return then(self, Err("The review's diff isn't loaded yet".to_string()), cx);
        };
        self.review_write::<ReviewDraft>(tab, Request::ForgeAddDraft { repo: core_repo(tab), number, comment: NewReviewComment { anchor, body, refs } }, cx, move |v, answer, cx| match answer {
            Err(e) => then(v, Err(e), cx),
            Ok(d) => {
                if let Some(s) = v.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.forge.review.as_mut()).filter(|s| s.number == number) {
                    s.drafts.retain(|x| x.id != d.id);
                    s.drafts.push(d);
                }
                v.review_refresh(tab, cx);
                then(v, Ok(()), cx);
            }
        });
    }

    pub fn review_comment_now(&mut self, tab: u32, anchor: ReviewAnchor, body: String, cx: &mut Context<Self>, then: impl FnOnce(&mut GitVolt, Result<(), String>, &mut Context<GitVolt>) + 'static) {
        let Some((number, refs)) = self.tabs.iter().find(|t| t.id == tab).and_then(|t| t.forge.review.as_ref()).and_then(|s| Some((s.number, s.refs.clone()?))) else {
            return then(self, Err("The review's diff isn't loaded yet".to_string()), cx);
        };
        self.review_write::<ForgeDiscussion>(tab, Request::ForgeCommentNow { repo: core_repo(tab), number, comment: NewReviewComment { anchor, body, refs } }, cx, move |v, answer, cx| match answer {
            Err(e) => then(v, Err(e), cx),
            Ok(d) => {
                if let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) {
                    t.forge.discussions.entry(number).or_default().push(d);
                }
                v.forge_after_write(tab, cx);
                then(v, Ok(()), cx);
            }
        });
    }

    pub fn review_edit_draft(&mut self, tab: u32, id: String, body: String, cx: &mut Context<Self>, then: impl FnOnce(&mut GitVolt, Result<(), String>, &mut Context<GitVolt>) + 'static) {
        let Some(number) = self.tabs.iter().find(|t| t.id == tab).and_then(|t| t.forge.review.as_ref()).map(|s| s.number) else { return };
        self.review_write::<ReviewDraft>(tab, Request::ForgeEditDraft { repo: core_repo(tab), number, id: id.clone(), body }, cx, move |v, answer, cx| match answer {
            Err(e) => then(v, Err(e), cx),
            Ok(d) => {
                if let Some(s) = v.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.forge.review.as_mut()).filter(|s| s.number == number)
                    && let Some(x) = s.drafts.iter_mut().find(|x| x.id == id)
                {
                    x.body = d.body;
                }
                v.review_refresh(tab, cx);
                then(v, Ok(()), cx);
            }
        });
    }

    pub fn review_delete_draft(&mut self, tab: u32, id: String, cx: &mut Context<Self>) {
        let Some(number) = self.tabs.iter().find(|t| t.id == tab).and_then(|t| t.forge.review.as_ref()).map(|s| s.number) else { return };
        self.review_write::<serde_json::Value>(tab, Request::ForgeDeleteDraft { repo: core_repo(tab), number, id: id.clone() }, cx, move |v, answer, cx| match answer {
            Err(e) => crate::ui::toast::show_error(format!("Couldn't delete the pending comment: {e}"), cx),
            Ok(_) => {
                if let Some(s) = v.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.forge.review.as_mut()).filter(|s| s.number == number) {
                    s.drafts.retain(|x| x.id != id);
                }
                v.review_refresh(tab, cx);
            }
        });
    }

    fn review_cleared(&mut self, tab: u32, number: u64, cx: &mut Context<Self>) {
        if let Some(s) = self.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.forge.review.as_mut()).filter(|s| s.number == number) {
            s.drafts.clear();
            s.pending_review = None;
        }
        self.forge_after_write(tab, cx);
        self.review_settle(tab, cx);
    }

    /// Sends the review with its drafts. A GitLab that published the drafts but refused the event still answers, with `eventError` saying what didn't go through.
    pub fn review_submit(&mut self, tab: u32, review: serde_json::Value, cx: &mut Context<Self>, then: impl FnOnce(&mut GitVolt, Result<SubmitOutcome, String>, &mut Context<GitVolt>) + 'static) {
        let Some(number) = self.tabs.iter().find(|t| t.id == tab).and_then(|t| t.forge.review.as_ref()).map(|s| s.number) else {
            return then(self, Err("No review is open in this tab".into()), cx);
        };
        let Ok(review) = serde_json::from_value(review) else { return };
        self.review_write::<SubmitOutcome>(tab, Request::ForgeSubmitReview { repo: core_repo(tab), number, review }, cx, move |v, answer, cx| {
            if answer.is_ok() {
                v.review_cleared(tab, number, cx);
            }
            then(v, answer, cx);
        });
    }

    /// `discardPending`: the control arms in place first; then the forge deletes it, and a toast says what went.
    pub fn review_discard(&mut self, tab: u32, kind: ForgeKind, origin: crate::ui::arm::Origin, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.tabs.iter().find(|t| t.id == tab).and_then(|t| t.forge.review.clone()) else { return };
        let what = pending_text(s.drafts.len());
        let r = super::text::mr_ref(kind, s.number);
        let number = s.number;
        let weak = cx.weak_entity();
        let req = crate::ui::arm::ArmRequest::danger(format!("Click again to discard {what}"), format!("Discard {what} on {r}?"), "Discard");
        crate::ui::arm::request(
            req,
            origin,
            move |answer, _, cx| {
                if !answer.ok {
                    return;
                }
                weak.update(cx, |v, cx| {
                    v.review_write::<serde_json::Value>(tab, Request::ForgeDiscardReview { repo: core_repo(tab), number }, cx, move |v, answer, cx| match answer {
                        Ok(_) => {
                            v.review_cleared(tab, number, cx);
                            crate::ui::toast::show(format!("Discarded {what} on {r}"), cx);
                        }
                        Err(e) => crate::ui::toast::show_error(format!("Couldn't discard the review: {e}"), cx),
                    });
                })
                .ok();
            },
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn file(lines: Vec<ReviewLine>) -> CommentableFile {
        commentable_index(&ReviewFile { path: "a.rs".into(), old_path: "a.rs".into(), lines, too_large: false })
    }

    fn l(kind: LineKind, old_line: u32, new_line: u32) -> ReviewLine {
        ReviewLine { kind, old_line, new_line }
    }

    #[test]
    fn a_range_stays_in_the_hunk_of_its_last_line() {
        let f = file(vec![l(LineKind::Context, 1, 1), l(LineKind::Added, 2, 2), l(LineKind::Context, 50, 51), l(LineKind::Added, 51, 52)]);
        let a = anchor_for(&f, DiffSide::New, 1, 52).unwrap();
        assert_eq!(a.end.new_line, 52);
        assert_eq!(a.start.unwrap().new_line, 51, "the first hunk's lines aren't in the range");
        let one = anchor_for(&f, DiffSide::New, 2, 2).unwrap();
        assert_eq!(one.start, None);
        assert!(anchor_for(&f, DiffSide::New, 10, 20).is_none());
    }

    #[test]
    fn a_range_is_on_the_side_of_its_last_line() {
        let f = file(vec![l(LineKind::Removed, 1, 1), l(LineKind::Added, 2, 1), l(LineKind::Added, 2, 2)]);
        let a = ReviewAnchor { path: "a.rs".into(), old_path: "a.rs".into(), start: Some(f.lines[0]), end: f.lines[2] };
        assert_eq!(anchor_span(&f, &a), (DiffSide::New, 1, 2), "a removed line it starts on is left out of the numbers");
        let b = ReviewAnchor { path: "a.rs".into(), old_path: "a.rs".into(), start: None, end: f.lines[0] };
        assert_eq!(anchor_span(&f, &b), (DiffSide::Old, 1, 1));
    }

    fn pos(line: Option<u32>, old_line: Option<u32>, head: Option<&str>) -> DiffPosition {
        DiffPosition { path: "a.rs".into(), old_path: Some("a.rs".into()), line, old_line, snippet: None, start_line: None, start_old_line: None, head_sha: head.map(str::to_string), outdated: false }
    }

    #[test]
    fn an_outdated_thread_stays_on_the_timeline_and_an_outdated_draft_stays_on_its_line() {
        let refs = DiffRefs { base_sha: "b".into(), start_sha: "s".into(), head_sha: "new".into() };
        let f = file(vec![l(LineKind::Added, 1, 5)]);
        assert!(place_of(&pos(Some(5), None, Some("old")), Some(&refs), Some(&f), false).is_none());
        let d = place_of(&pos(Some(5), None, Some("old")), Some(&refs), Some(&f), true).unwrap();
        assert!(d.outdated);
        assert!(place_of(&pos(Some(9), None, Some("old")), Some(&refs), Some(&f), true).is_none());
        assert_eq!(place_of(&pos(None, Some(3), None), Some(&refs), None, false).unwrap().side, DiffSide::Old);
    }

    #[test]
    fn a_session_lasts_while_something_keeps_it() {
        let mut s = new_session(ForgeKind::GitLab, 1, None);
        assert!(review_alive(&s, false, None), "not read yet");
        s.loaded = true;
        assert!(!review_alive(&s, false, None));
        assert!(review_alive(&s, true, None));
        s.pending_review = Some("x".into());
        assert!(review_alive(&s, false, None));
    }

    #[test]
    fn a_retry_sends_what_didnt_go_in() {
        assert_eq!(retry_plan("approve", "hi", None), ("approve".to_string(), "hi".to_string()));
        assert_eq!(retry_plan("approve", "hi", Some(Sent { body_posted: true, event_sent: false })), ("approve".to_string(), String::new()));
        assert_eq!(retry_plan("approve", "hi", Some(Sent { body_posted: false, event_sent: true })), ("comment".to_string(), "hi".to_string()));
    }
}
