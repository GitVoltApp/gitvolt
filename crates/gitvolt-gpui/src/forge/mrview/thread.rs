//! The description and the activity (`Thread.tsx`, `NoteActions.tsx`, `noteActionsStore.ts`, `ReplyBox.tsx`): tabs over a vertical timeline of forge events and comment-thread cards.

use super::header::mr_button;
use super::system_note::{NotePart, SystemKind, parse_system_note};
use super::{ActivityTab, FieldKey};
use crate::app::{GitVolt, core_repo, icon};
use crate::forge::card::forge_avatar;
use crate::forge::text::mr_ref;
use crate::md::attachments::signed_attachments;
use crate::md::field::MarkdownField;
use crate::theme;
use crate::ui::arm::{self, ArmRequest, Origin};
use crate::ui::menu::{self, MenuRow};
use crate::ui::tooltip::tip;
use gitvolt_core::api::Request;
use gitvolt_core::forge::types::{DiffPosition, ForgeDiscussion, ForgeMr, ForgeMrDetail, ForgeNote, ForgeReaction, ForgeReview, NoteRef, ReviewState, ThreadState};
use gitvolt_core::forge::ForgeKind;
use gpui::prelude::*;
use gpui::*;

/// A body over this renders as plain text.
pub const MR_BODY_MAX_BYTES: usize = 1024 * 1024;

pub const GITHUB_REACTIONS: [(&str, &str); 8] = [("+1", "👍"), ("-1", "👎"), ("laugh", "😄"), ("hooray", "🎉"), ("confused", "😕"), ("heart", "❤️"), ("rocket", "🚀"), ("eyes", "👀")];
pub const GITLAB_COMMON: [(&str, &str); 8] = [("thumbsup", "👍"), ("thumbsdown", "👎"), ("smile", "😄"), ("tada", "🎉"), ("confused", "😕"), ("heart", "❤️"), ("rocket", "🚀"), ("eyes", "👀")];

pub fn known_glyph(kind: ForgeKind, name: &str) -> Option<&'static str> {
    let set = if kind == ForgeKind::GitLab { &GITLAB_COMMON } else { &GITHUB_REACTIONS };
    set.iter().find(|(n, _)| *n == name).map(|(_, g)| *g)
}

/// A comment's web address: GitHub's and Forgejo's own, GitLab's MR address + `#note_<id>`.
pub fn note_permalink(kind: ForgeKind, mr_url: &str, n: &ForgeNote) -> Option<String> {
    if kind != ForgeKind::GitLab {
        return n.web_url.clone();
    }
    (!mr_url.is_empty() && !n.id.is_empty() && n.id.bytes().all(|b| b.is_ascii_digit())).then(|| format!("{mr_url}#note_{}", n.id))
}

/// GitHub's and Forgejo's review summaries take no reactions and can't be changed here.
pub fn changeable_thread(kind: ForgeKind, d: &ForgeDiscussion) -> bool {
    !(kind != ForgeKind::GitLab && d.id.starts_with("review-"))
}

/// Forgejo deletes conversation comments only, not a review's code comments.
pub fn deletable_note(kind: ForgeKind, d: &ForgeDiscussion) -> bool {
    kind != ForgeKind::Forgejo || d.id.starts_with("issue-")
}

/// `list` with the user's `name` reaction added (`on`) or taken away.
pub fn toggled(list: &[ForgeReaction], name: &str, on: bool, who: Option<&str>) -> Vec<ForgeReaction> {
    let at = list.iter().position(|r| r.name == name);
    let mut out = list.to_vec();
    if on {
        match at {
            None => out.push(ForgeReaction { name: name.to_string(), count: 1, mine: true, users: who.map(str::to_string).into_iter().collect() }),
            Some(i) if !out[i].mine => {
                out[i].count += 1;
                out[i].mine = true;
                if let Some(w) = who {
                    out[i].users.push(w.to_string());
                }
            }
            _ => {}
        }
    } else if let Some(i) = at.filter(|i| out[*i].mine) {
        if out[i].count <= 1 {
            out.remove(i);
        } else {
            out[i].count -= 1;
            out[i].mine = false;
            if let Some(w) = who {
                out[i].users.retain(|u| u != w);
            }
        }
    }
    out
}

/// The note in its discussion (a new one at the end), as the forge answered it.
pub fn append_note(list: &mut Vec<ForgeDiscussion>, discussion: Option<&str>, note: ForgeNote) {
    if let Some(d) = discussion.and_then(|id| list.iter_mut().find(|d| d.id == id)) {
        d.notes.push(note);
        return;
    }
    list.push(ForgeDiscussion { id: format!("new-{}", note.id), notes: vec![note], resolvable: false, resolved: false, resolved_by: None, resolved_at: None });
}

/// `list` with a note removed, and its discussion with it once no comment is left.
pub fn without_note(list: &mut Vec<ForgeDiscussion>, discussion: &str, id: &str) {
    if let Some(i) = list.iter().position(|d| d.id == discussion) {
        list[i].notes.retain(|n| n.id != id);
        if !list[i].notes.iter().any(|n| !n.system) {
            list.remove(i);
        }
    }
}

/// Quote reply's text: the comment as a Markdown quote, after what's there.
pub fn quoted(now: &str, body: &str) -> String {
    let quote = body.trim_end().split('\n').map(|l| if l.is_empty() { ">".to_string() } else { format!("> {l}") }).collect::<Vec<_>>().join("\n");
    if now.is_empty() { format!("{quote}\n\n") } else { format!("{}\n\n{quote}\n\n", now.trim_end()) }
}

type FoldFn = Box<dyn Fn(&mut GitVolt, bool, &mut Context<GitVolt>)>;

/// Where a note opens the diff: its new line, else its old one; a range's lines, from its first, when that has a number on the same side.
pub fn note_line(pos: &DiffPosition) -> Option<(bool, u32, Option<u32>)> {
    let (new_side, start, end) = match (pos.line, pos.old_line) {
        (Some(l), _) => (true, pos.start_line, l),
        (None, l) => (false, pos.start_old_line, l?),
    };
    Some(match start.filter(|s| *s < end) {
        Some(s) => (new_side, s, Some(end)),
        None => (new_side, end, None),
    })
}

/// A note's `path:line`, or `path:start-end`.
pub fn note_where(pos: &DiffPosition) -> String {
    match note_line(pos) {
        Some((_, l, Some(e))) => format!("{}:{l}-{e}", pos.path),
        Some((_, l, None)) => format!("{}:{l}", pos.path),
        None => pos.path.clone(),
    }
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Entry {
    System(ForgeNote),
    Thread(ForgeDiscussion),
    Review(ForgeReview, i64),
}

/// `entriesOf`: the timeline's entries by time; a thread sits at its first comment's time. GitHub's and Forgejo's reviews are events too.
pub fn entries_of(discussions: &[ForgeDiscussion], reviews: &[ForgeReview]) -> Vec<Entry> {
    let mut out: Vec<(i64, Entry)> = Vec::new();
    for d in discussions {
        if !d.notes.is_empty() && d.notes.iter().all(|n| n.system) {
            out.extend(d.notes.iter().map(|n| (n.created_at, Entry::System(n.clone()))));
        } else if let Some(first) = d.notes.iter().find(|n| !n.system) {
            out.push((first.created_at, Entry::Thread(d.clone())));
        }
    }
    for r in reviews {
        if let Some(at) = r.submitted_at.filter(|_| matches!(r.state, ReviewState::Approved | ReviewState::ChangesRequested | ReviewState::Commented)) {
            out.push((at, Entry::Review(r.clone(), at)));
        }
    }
    out.sort_by_key(|e| e.0);
    out.into_iter().map(|e| e.1).collect()
}

/// A relative date, its precise one on hover.
pub fn rel_time(at: i64, text: String, color: Hsla, size: f32, this: &GitVolt) -> AnyElement {
    let full = crate::format::date_with_zone(at, this.settings["dateFormat"].as_str().unwrap_or("ymd12h"));
    tip(full, div().flex_none().text_size(px(size)).text_color(color).child(text)).into_any_element()
}

fn when(this: &GitVolt, at: i64) -> AnyElement {
    rel_time(at, crate::format::relative_time(at, crate::format::now()), theme::text_disabled(), 12., this)
}

fn node_icon(kind: SystemKind) -> (&'static str, Hsla) {
    match kind {
        SystemKind::Commits => ("git-commit-horizontal", theme::blue()),
        SystemKind::Edit => ("pencil", theme::text_secondary()),
        SystemKind::Title => ("type", theme::orange()),
        SystemKind::Mention => ("at-sign", theme::purple()),
        SystemKind::Approved => ("check", theme::green()),
        SystemKind::Unapproved => ("circle-alert", theme::orange()),
        SystemKind::Merged => ("git-merge", theme::purple()),
        SystemKind::Closed => ("circle-x", theme::red()),
        SystemKind::Reopened => ("circle-dot", theme::green()),
        SystemKind::Draft => ("git-pull-request-draft", theme::text_secondary()),
        SystemKind::Label => ("tag", theme::text_secondary()),
        SystemKind::Other => ("circle-dot", theme::text_secondary()),
        SystemKind::Review => ("message-square", theme::text_secondary()),
    }
}

fn node(kind: SystemKind) -> Div {
    let (glyph, color) = node_icon(kind);
    div().flex().flex_none().size(px(22.)).items_center().justify_center().rounded_full().bg(theme::panel_bg()).border_2().border_color(theme::panel_bg1()).child(icon(glyph, 12., color))
}

/// A system note's parts as text and elements: nothing is ever parsed as HTML.
fn note_parts(parts: &[NotePart], key: &str, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    let mut out = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        out.push(match p {
            NotePart::Text(t) => div().child(crate::md::emoji::emojify(t)).into_any_element(),
            NotePart::Code(t) => div().px(px(5.)).rounded(px(3.)).bg(theme::control_bg()).font_family(crate::app::MONO_FONT).text_size(px(11.)).text_color(theme::text_normal()).child(crate::md::emoji::emojify(t)).into_any_element(),
            NotePart::Del(t) => div().text_color(theme::red()).opacity(0.85).line_through().child(t.clone()).into_any_element(),
            NotePart::Ins(t) => div().text_color(theme::green()).child(t.clone()).into_any_element(),
            NotePart::Link { text, url } => {
                let url = url.clone();
                div().id(ElementId::Name(format!("{key}:link:{i}").into())).cursor_pointer().text_color(theme::link()).hover(|d| d.underline()).child(text.clone()).on_click(cx.listener(move |v, _, _, _| v.forge_open_url(url.clone()))).into_any_element()
            }
            NotePart::Commits { items, more } => {
                let mut list = div().flex().flex_col().gap(px(3.)).mt(px(5.)).w_full();
                for c in items {
                    list = list.child(div().flex().items_baseline().gap(px(8.)).text_color(theme::text_normal()).when(!c.sha.is_empty(), |d| d.child(div().flex_none().px(px(5.)).rounded(px(3.)).bg(theme::mix(theme::link(), 0.08, transparent_black())).font_family(crate::app::MONO_FONT).text_size(px(11.)).text_color(theme::link()).child(c.sha.clone()))).child(div().min_w_0().child(c.subject.clone())));
                }
                if *more > 0 {
                    list = list.child(div().text_color(theme::text_disabled()).child(format!("and {more} more commit{}", if *more == 1 { "" } else { "s" })));
                }
                list.into_any_element()
            }
        });
    }
    out
}

fn event_line(this: &GitVolt, name: &str, parts: Vec<AnyElement>, at: i64) -> Div {
    div().flex().flex_wrap().items_baseline().gap_x(px(3.)).pt(px(3.)).line_height(px(16.)).text_size(px(12.)).text_color(theme::text_secondary()).child(div().font_weight(FontWeight::SEMIBOLD).text_color(theme::text_normal()).child(name.to_string())).children(parts).child(div().ml(px(6.)).child(when(this, at)))
}

impl GitVolt {
    /// A file of the shown compare, at a note's line; or as soon as its files are read.
    pub fn open_compare_file_at(&mut self, tab: u32, path: &str, line: Option<(bool, u32, Option<u32>)>, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        let Some(files) = t.files.as_ref() else {
            t.forge.note_after_compare = Some((path.to_string(), line));
            return;
        };
        let at = files.changes.iter().position(|f| f.path.as_ref() == path || f.old_path.as_deref() == Some(path));
        match at {
            Some(i) => {
                self.open_file(i, cx);
                if let Some((_, l, _)) = line {
                    self.with_diff(cx, |d| d.pending_line = Some(l.saturating_sub(1)));
                }
            }
            None => crate::ui::toast::show(format!("{path} isn't changed in this merge request any more"), cx),
        }
    }

    fn mr_patch_threads(&mut self, tab: u32, number: u64, change: impl FnOnce(&mut Vec<ForgeDiscussion>)) {
        if let Some(l) = self.tabs.iter_mut().find(|t| t.id == tab).and_then(|t| t.forge.discussions.get_mut(&number)) {
            change(l);
        }
    }

    fn mr_note_mut<R>(&mut self, tab: u32, number: u64, discussion: &str, id: &str, f: impl FnOnce(&mut ForgeNote) -> R) -> Option<R> {
        let t = self.tabs.iter_mut().find(|t| t.id == tab)?;
        let d = t.forge.discussions.get_mut(&number)?.iter_mut().find(|d| d.id == discussion)?;
        d.notes.iter_mut().find(|n| n.id == id).map(f)
    }

    /// `toggleReaction`: shown at once, then sent; the forge's answer replaces it, a refusal puts it back and a toast says why.
    pub fn mr_toggle_reaction(&mut self, tab: u32, number: u64, discussion: String, id: String, name: String, cx: &mut Context<Self>) {
        let me = self.tabs.iter().find(|t| t.id == tab).and_then(|t| t.forge.me.clone());
        let Some(before) = self.mr_note_mut(tab, number, &discussion, &id, |n| n.reactions.clone()) else { return };
        let on = !before.iter().any(|r| r.name == name && r.mine);
        let next = toggled(&before, &name, on, me.as_deref());
        self.mr_note_mut(tab, number, &discussion, &id, |n| n.reactions = next);
        let note = NoteRef { discussion: discussion.clone(), note: id.clone() };
        let (d2, id2, name2) = (discussion, id, name.clone());
        self.forge_ask::<Vec<ForgeReaction>>(Request::ForgeReact { repo: core_repo(tab), number, note, name, on }, cx, move |v, answer, cx| match answer {
            Ok(r) => {
                v.forge_written(tab, cx);
                v.mr_note_mut(tab, number, &d2, &id2, |n| n.reactions = r);
            }
            Err(e) => {
                // Refused: this one change undone, whatever else changed meanwhile.
                let now = v.mr_note_mut(tab, number, &d2, &id2, |n| n.reactions.clone()).unwrap_or_default();
                let back = toggled(&now, &name2, !on, me.as_deref());
                v.mr_note_mut(tab, number, &d2, &id2, |n| n.reactions = back);
                crate::ui::toast::show_error(format!("{}: {e}", if on { "Couldn't add the reaction" } else { "Couldn't remove the reaction" }), cx);
            }
        });
    }

    /// `resolveThread`: shown at once, put back (with the forge's reason) if refused.
    pub fn mr_resolve(&mut self, tab: u32, number: u64, discussion: String, resolved: bool, failure: Option<String>, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some(before) = t.forge.discussions.get(&number).and_then(|l| l.iter().find(|d| d.id == discussion)).cloned() else { return };
        let me = t.forge.me.clone();
        let now = (t.forge.kind == Some(ForgeKind::GitLab)).then(|| super::super::poll::now_ms() / 1000);
        let d2 = discussion.clone();
        self.mr_patch_threads(tab, number, move |l| {
            if let Some(d) = l.iter_mut().find(|d| d.id == d2) {
                d.resolved = resolved;
                d.resolved_by = if resolved { me } else { None };
                d.resolved_at = if resolved { now } else { None };
            }
        });
        cx.notify();
        let failure = failure.unwrap_or_else(|| if resolved { "Couldn't resolve the thread".to_string() } else { "Couldn't unresolve the thread".to_string() });
        let d3 = discussion.clone();
        self.forge_ask::<ThreadState>(Request::ForgeResolve { repo: core_repo(tab), number, discussion, resolved }, cx, move |v, answer, cx| match answer {
            Ok(s) => {
                v.forge_written(tab, cx);
                v.mr_patch_threads(tab, number, |l| {
                    if let Some(d) = l.iter_mut().find(|d| d.id == d3) {
                        d.resolved = s.resolved;
                        d.resolved_by = s.resolved_by;
                        d.resolved_at = if s.resolved { now } else { None };
                    }
                });
            }
            Err(e) => {
                v.mr_patch_threads(tab, number, |l| {
                    if let Some(d) = l.iter_mut().find(|d| d.id == d3) {
                        *d = before;
                    }
                });
                crate::ui::toast::show_error(format!("{failure}: {e}"), cx);
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn mr_delete_note(&mut self, tab: u32, number: u64, discussion: String, id: String, origin: Origin, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        let mut req = ArmRequest::danger("Click again to delete the comment", "Delete this comment?", "Delete");
        req.body = "It's deleted on the forge, for everyone.".into();
        arm::request(
            req,
            origin,
            move |answer, _, cx| {
                if !answer.ok {
                    return;
                }
                weak.update(cx, |v, cx| {
                    let note = NoteRef { discussion: discussion.clone(), note: id.clone() };
                    let (d, n) = (discussion.clone(), id.clone());
                    v.forge_ask::<serde_json::Value>(Request::ForgeDeleteNote { repo: core_repo(tab), number, note }, cx, move |v, answer, cx| match answer {
                        Ok(_) => {
                            v.forge_written(tab, cx);
                            v.mr_patch_threads(tab, number, |l| without_note(l, &d, &n));
                        }
                        Err(e) => crate::ui::toast::show_error(format!("Couldn't delete the comment: {e}"), cx),
                    });
                })
                .ok();
            },
            window,
            cx,
        );
    }

    fn mr_save_note(&mut self, tab: u32, number: u64, discussion: String, id: String, cx: &mut Context<Self>) {
        let Some(field) = self.mr_ui(tab, number).and_then(|u| u.note_edit.get(&id).cloned()) else { return };
        let body = field.read(cx).text(cx);
        if body.trim().is_empty() {
            return;
        }
        if let Some(ui) = self.mr_ui(tab, number) {
            ui.note_busy = Some(id.clone());
        }
        let note = NoteRef { discussion: discussion.clone(), note: id.clone() };
        self.forge_ask::<ForgeNote>(Request::ForgeEditNote { repo: core_repo(tab), number, note, body }, cx, move |v, answer, cx| {
            if let Some(ui) = v.mr_ui(tab, number) {
                ui.note_busy = None;
            }
            match answer {
                Ok(n) => {
                    v.forge_written(tab, cx);
                    v.mr_note_mut(tab, number, &discussion, &id, |x| {
                        x.body = n.body;
                        x.body_html = n.body_html;
                    });
                    if let Some(ui) = v.mr_ui(tab, number) {
                        ui.note_edit.remove(&id);
                    }
                }
                Err(e) => crate::ui::toast::show_error(format!("Couldn't save the comment: {e}"), cx),
            }
        });
    }

    /// Quote reply: into the reply box its answer goes to (the thread's; GitHub's and Forgejo's conversation: the new comment's).
    #[allow(clippy::too_many_arguments)]
    fn mr_quote(&mut self, tab: u32, kind: ForgeKind, number: u64, d: &ForgeDiscussion, body: &str, window: &mut Window, cx: &mut Context<Self>) {
        let target = if kind != ForgeKind::GitLab && !d.id.starts_with("thread-") { None } else { Some(d.id.clone()) };
        let key = target.clone().map_or(FieldKey::Comment, FieldKey::Reply);
        let Some(field) = self.mr_field(tab, number, key, cx) else { return };
        if let (Some(id), Some(ui)) = (&target, self.mr_ui(tab, number)) {
            ui.reply_open.insert(id.clone());
        }
        let text = quoted(&field.read(cx).text(cx), body);
        field.update(cx, |f, cx| {
            f.set_text(&text, cx);
            f.focus(window, cx);
        });
        cx.notify();
    }

    /// Ctrl+Enter from a comment's box.
    pub fn mr_field_submit(&mut self, tab: u32, number: u64, key: &FieldKey, cx: &mut Context<Self>) {
        match key {
            FieldKey::Comment => self.mr_send_reply(tab, number, None, false, cx),
            FieldKey::Reply(d) => self.mr_send_reply(tab, number, Some(d.clone()), false, cx),
            FieldKey::NoteEdit(n) => {
                let d = self.tabs.iter().find(|t| t.id == tab).and_then(|t| t.forge.discussions.get(&number)).and_then(|l| l.iter().find(|d| d.notes.iter().any(|x| &x.id == n)).map(|d| d.id.clone()));
                if let Some(d) = d {
                    self.mr_save_note(tab, number, d, n.clone(), cx);
                }
            }
        }
    }

    /// A comment or a reply in a discussion. `toggle`: the reply, then the thread's resolve (no forge does both in one request).
    pub fn mr_send_reply(&mut self, tab: u32, number: u64, discussion: Option<String>, toggle: bool, cx: &mut Context<Self>) {
        let key = discussion.clone().map_or(FieldKey::Comment, FieldKey::Reply);
        let Some(field) = self.mr_field(tab, number, key, cx) else { return };
        let text = field.read(cx).text(cx);
        let kind = self.tabs.iter().find(|t| t.id == tab).and_then(|t| t.forge.kind).unwrap_or(ForgeKind::GitLab);
        let was_resolved = discussion.as_ref().and_then(|id| self.tabs.iter().find(|t| t.id == tab)?.forge.discussions.get(&number)?.iter().find(|d| &d.id == id).map(|d| d.resolved));
        let Some(ui) = self.mr_ui(tab, number) else { return };
        let busy_key = discussion.clone().unwrap_or_default();
        if text.trim().is_empty() || ui.reply_busy.is_some() {
            return;
        }
        ui.reply_busy = Some(busy_key);
        cx.notify();
        let r = mr_ref(kind, number);
        let failure = if discussion.is_some() { format!("Couldn't reply on {r}") } else { format!("Couldn't comment on {r}") };
        let d2 = discussion.clone();
        self.forge_ask::<ForgeNote>(Request::ForgeReply { repo: core_repo(tab), number, discussion, body: text.clone() }, cx, move |v, answer, cx| {
            if let Some(ui) = v.mr_ui(tab, number) {
                ui.reply_busy = None;
            }
            match answer {
                Err(e) => crate::ui::toast::show_error(format!("{failure}: {e}"), cx),
                Ok(n) => {
                    v.forge_written(tab, cx);
                    if field.read(cx).text(cx) == text {
                        field.update(cx, |f, cx| f.set_text("", cx));
                    }
                    if let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) {
                        append_note(t.forge.discussions.entry(number).or_default(), d2.as_deref(), n);
                    }
                    if let (Some(id), Some(ui)) = (&d2, v.mr_ui(tab, number)) {
                        ui.reply_open.remove(id);
                    }
                    if let (true, Some(id), Some(was)) = (toggle, d2, was_resolved) {
                        let flip = if was { "unresolve" } else { "resolve" };
                        v.mr_resolve(tab, number, id, !was, Some(format!("Replied, but couldn't {flip} the thread")), cx);
                    }
                }
            }
        });
    }

    /// A diff-line note's `file:line`: that file in the local compare of merge-base(the target's remote tip, the head) → the head, as the MR's diff is, at its line or lines.
    pub fn mr_open_note_file(&mut self, tab: u32, kind: ForgeKind, mr: &ForgeMr, pos: &DiffPosition, cx: &mut Context<Self>) {
        let noun = crate::forge::text::mr_name(kind).to_lowercase();
        let not_loaded = format!("The {noun}'s commits aren't in the loaded history: fetch, or check it out first");
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let (Some(remote), Some(head)) = (t.forge.remote.clone(), mr.head_sha.clone()) else { return crate::ui::toast::show(not_loaded, cx) };
        let Some(tip_sha) = super::header::remote_tip(t, &remote, &mr.target_branch) else { return crate::ui::toast::show(not_loaded, cx) };
        let (path, line) = (pos.path.clone(), note_line(pos));
        let repo = core_repo(tab);
        self.forge_ask::<Option<String>>(Request::MergeBase { repo, a: tip_sha, b: head.clone() }, cx, move |v, base, cx| {
            let Some(base) = base.ok().flatten().filter(|b| v.tabs.iter().find(|t| t.id == tab).and_then(|t| t.graph.as_ref()).is_some_and(|g| g.by_sha.contains_key(b) && g.by_sha.contains_key(&head))) else {
                return crate::ui::toast::show(not_loaded, cx);
            };
            v.compare_commits(base, head, cx);
            v.open_compare_file_at(tab, &path, line, cx);
        });
    }
}

/// `Comment`, `Reply`, `Reply and resolve`.
#[allow(clippy::too_many_arguments)]
pub fn reply_box(this: &mut GitVolt, tab: u32, kind: ForgeKind, number: u64, discussion: Option<&ForgeDiscussion>, field: Entity<MarkdownField>, on_cancel: Option<String>, cx: &mut Context<GitVolt>) -> Div {
    let id = discussion.map(|d| d.id.clone());
    let text_empty = field.read(cx).text(cx).trim().is_empty();
    let busy_here = this.tabs.iter().find(|t| t.id == tab).and_then(|t| t.mr_views.get(&number)).and_then(|u| u.reply_busy.clone()).is_some_and(|b| b == id.clone().unwrap_or_default());
    let any_busy = this.tabs.iter().find(|t| t.id == tab).and_then(|t| t.mr_views.get(&number)).is_some_and(|u| u.reply_busy.is_some());
    let ready = !text_empty && !any_busy;
    let action = if id.is_some() { "Reply" } else { "Comment" };
    let key = id.clone().unwrap_or_else(|| "new".to_string());
    let toggle = discussion.filter(|d| d.resolvable).map(|d| if d.resolved { "unresolve" } else { "resolve" });
    let mut row = div().flex().justify_end().gap(px(6.));
    if let Some(cancel) = on_cancel {
        let d = cancel.clone();
        row = row.child(crate::ui::anchor(format!("mr-reply-cancel:{key}"), mr_button(ElementId::Name(format!("mr-reply-cancel:{key}").into()), true).child("Cancel").on_click(cx.listener(move |v, _, _, cx| {
            if let Some(ui) = v.mr_ui(tab, number) {
                ui.reply_open.remove(&d);
                if let Some(f) = ui.replies.remove(&d) {
                    f.update(cx, |f, cx| f.set_text("", cx));
                }
            }
            cx.notify();
        }))));
    }
    if let Some(flip) = toggle {
        let id2 = id.clone();
        row = row.child(crate::ui::anchor(format!("mr-reply-toggle:{key}"), mr_button(ElementId::Name(format!("mr-reply-toggle:{key}").into()), ready).child(format!("Reply and {flip}")).when(ready, |d| d.on_click(cx.listener(move |v, _, _, cx| v.mr_send_reply(tab, number, id2.clone(), true, cx))))));
    }
    let id3 = id.clone();
    row = row.child(crate::ui::anchor(format!("mr-reply-send:{key}"), super::header::mr_primary(ElementId::Name(format!("mr-reply-send:{key}").into()), ready).child(if busy_here { "Sending…" } else { action }).when(ready, |d| d.on_click(cx.listener(move |v, _, _, cx| v.mr_send_reply(tab, number, id3.clone(), false, cx))))));
    let _ = kind;
    div().flex().flex_col().gap(px(6.)).child(crate::ui::anchor(format!("mr-field:{key}"), div().child(field))).child(row)
}

/// A suggestion's Apply. Greyed, with why, where it wouldn't land; a line that's moved since says so on the click.
#[allow(clippy::too_many_arguments)]
fn apply_buttons(this: &GitVolt, tab: u32, kind: ForgeKind, number: u64, text: &str, base: Option<&crate::md::SuggestionBase>, position: Option<&gitvolt_core::forge::types::DiffPosition>, id: &str, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    let (Some(base), Some(pos), Some(t)) = (base, position, this.tabs.iter().find(|t| t.id == tab)) else { return Vec::new() };
    let Some(mr) = t.forge.known_mr(number).cloned() else { return Vec::new() };
    let head = t.forge.details.get(&number).and_then(|d| d.mr.head_sha.clone()).or(mr.head_sha.clone());
    let gate = crate::forge::review::apply::apply_gate(t, kind, &mr, head.as_deref(), &pos.path, Some(pos));
    let found = crate::forge::review::apply::suggestions_of(text);
    let several = found.len() > 1;
    found
        .into_iter()
        .enumerate()
        .filter_map(|(i, (lang, suggested))| {
            let replaced = base.replaced(&lang)?;
            let from = base.start(&lang);
            let key = format!("mr-apply:{id}:{i}");
            let on = gate.is_ok();
            let label = if several { format!("Apply suggestion {}", i + 1) } else { "Apply suggestion".to_string() };
            let tip_text = gate.as_ref().err().cloned().unwrap_or_else(|| format!("Apply it to {} in the working copy", pos.path));
            let (m, path, position) = (mr.clone(), pos.path.clone(), pos.clone());
            let btn = super::header::mr_button(ElementId::Name(key.clone().into()), on).child(label).when(on, |d| {
                d.on_click(cx.listener(move |v, _, _, cx| v.mr_apply_suggestion(tab, kind, m.clone(), path.clone(), from, replaced.clone(), suggested.clone(), Some(position.clone()), cx)))
            });
            Some(div().flex().mt(px(4.)).child(tip(tip_text, crate::ui::anchor(key, btn))).into_any_element())
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn note_view(this: &mut GitVolt, tab: u32, kind: ForgeKind, mr: &ForgeMr, d: &ForgeDiscussion, n: &ForgeNote, first: bool, avatar: bool, in_card: bool, lead: Option<AnyElement>, where_: Option<AnyElement>, cx: &mut Context<GitVolt>) -> AnyElement {
    let number = mr.number;
    let key = format!("mr-note:{}", n.id);
    let me = this.tabs.iter().find(|t| t.id == tab).and_then(|t| t.forge.me.clone());
    let mine = me.as_deref() == Some(n.author.username.as_str());
    let reactable = changeable_thread(kind, d);
    let link = note_permalink(kind, &mr.web_url, n);
    let editing = this.tabs.iter().find(|t| t.id == tab).and_then(|t| t.mr_views.get(&number)).is_some_and(|u| u.note_edit.contains_key(&n.id));
    let picker_here = this.tabs.iter().find(|t| t.id == tab).and_then(|t| t.mr_views.get(&number)).and_then(|u| u.reaction_picker.as_ref()).is_some_and(|p| p.note == n.id);
    let group: SharedString = key.clone().into();
    let when_el = {
        let full = crate::format::date_with_zone(n.created_at, this.settings["dateFormat"].as_str().unwrap_or("ymd12h"));
        let text = crate::format::relative_time(n.created_at, crate::format::now());
        let label = div().flex_none().text_size(px(12.)).text_color(theme::text_disabled());
        match link.clone() {
            Some(url) => {
                let mkey = format!("{key}:when");
                let (w, u2) = (cx.weak_entity(), url.clone());
                let menu_key = mkey.clone();
                tip(full, crate::ui::anchor(mkey.clone(), label.id(ElementId::Name(mkey.into())).cursor_pointer().hover(|d| d.underline()).child(text).on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    let (w2, u3, u4) = (w.clone(), u2.clone(), u2.clone());
                    let rows = vec![
                        MenuRow::action("note.copyLink", "Copy link", "link", format!("Copy {u3}"), move |_, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(u3.clone()));
                            crate::ui::toast::show("Copied the link", cx);
                        }),
                        MenuRow::action("note.openInBrowser", "Open in browser", "external-link", u4.clone(), move |_, cx| {
                            let u = u4.clone();
                            w2.update(cx, |v, _| v.forge_open_url(u)).ok();
                        }),
                    ];
                    menu::open_below(menu_key.clone(), rows, Default::default(), window, cx);
                }))).into_any_element()
            }
            None => tip(full, label.child(text)).into_any_element(),
        }
    };
    // The header's actions: shown on hover, and while their picker or menu is open.
    let more_key = format!("{key}:more");
    let (w, d2, n2, link2, more2) = (cx.weak_entity(), d.clone(), n.clone(), link.clone(), more_key.clone());
    let menu_open = menu::is_target(&more_key, cx);
    let actions = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.))
        .when(!picker_here && !menu_open, |d| d.invisible().group_hover(group.clone(), |d| d.visible()))
        .when(reactable, |e| {
            let (ping_d, ping_n) = (d.id.clone(), n.id.clone());
            e.child(tip("Add reaction", crate::ui::anchor(format!("{key}:react"), div().id(ElementId::Name(format!("{key}:react").into())).flex().size(px(22.)).items_center().justify_center().rounded(px(4.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).child(icon("smile-plus", 14., theme::text_secondary())).on_click(cx.listener(move |v, _, _, cx| {
                if let Some(ui) = v.mr_ui(tab, number) {
                    ui.reaction_picker = if ui.reaction_picker.as_ref().is_some_and(|p| p.note == ping_n) { None } else { Some(PickerAt { discussion: ping_d.clone(), note: ping_n.clone() }) };
                }
                cx.notify();
            })))))
        })
        .child(crate::ui::anchor(more_key.clone(), div().id(ElementId::Name(more_key.clone().into())).flex().size(px(22.)).items_center().justify_center().rounded(px(4.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).child(icon("ellipsis-vertical", 14., theme::text_secondary())).on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            let mut rows = Vec::new();
            if mine && reactable {
                let (w2, id, body) = (w.clone(), n2.id.clone(), n2.body.clone());
                rows.push(MenuRow::action("note.edit", "Edit", "pencil", "Edit your comment", move |window, cx| {
                    let (id, body) = (id.clone(), body.clone());
                    w2.update(cx, |v, cx| {
                        if let Some(f) = v.mr_field(tab, number, FieldKey::NoteEdit(id), cx) {
                            f.update(cx, |f, cx| {
                                f.set_text(&body, cx);
                                f.focus(window, cx);
                            });
                        }
                    }).ok();
                }));
            }
            if mine && reactable && deletable_note(kind, &d2) {
                let (w2, did, id) = (w.clone(), d2.id.clone(), n2.id.clone());
                rows.push(MenuRow::action("note.delete", "Delete", "trash-2", "Delete your comment, for everyone", move |window, cx| {
                    let (did, id, origin) = (did.clone(), id.clone(), Origin::current(cx));
                    w2.update(cx, |v, cx| v.mr_delete_note(tab, number, did, id, origin, window, cx)).ok();
                }));
            }
            if let Some(url) = link2.clone() {
                rows.push(MenuRow::action("note.copyLink", "Copy link", "link", format!("Copy {url}"), move |_, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                    crate::ui::toast::show("Copied the link", cx);
                }));
            }
            let (w2, d3, body) = (w.clone(), d2.clone(), n2.body.clone());
            rows.push(MenuRow::action("note.quote", "Quote reply", "quote", "Quote this comment in a reply", move |window, cx| {
                let (d3, body) = (d3.clone(), body.clone());
                w2.update(cx, |v, cx| v.mr_quote(tab, kind, number, &d3, &body, window, cx)).ok();
            }));
            menu::open_below(more2.clone(), rows, Default::default(), window, cx);
        })));
    let resolve = (first && d.resolvable).then(|| {
        let text = if d.resolved { format!("Unresolve thread{}", d.resolved_by.as_ref().map_or(String::new(), |b| format!(" · resolved by {b}"))) } else { "Resolve thread".to_string() };
        let (did, resolved) = (d.id.clone(), d.resolved);
        tip(text, crate::ui::anchor(format!("mr-resolve:{}", d.id), div().id(ElementId::Name(format!("mr-resolve:{}", d.id).into())).flex().size(px(22.)).items_center().justify_center().rounded_full().cursor_pointer().hover(|d| d.bg(theme::hover_row())).when(resolved, |d| d.bg(theme::mix(theme::green(), 0.16, transparent_black()))).child(icon("circle-check", 15., if resolved { theme::green() } else { theme::text_secondary() })).on_click(cx.listener(move |v, _, _, cx| v.mr_resolve(tab, number, did.clone(), !resolved, None, cx)))))
    });
    let head = div().flex().items_center().gap(px(6.)).min_h(px(22.)).text_size(px(12.)).children(lead).when(!first || avatar, |e| e.child(forge_avatar(&n.author, 18.))).child(div().font_weight(FontWeight::BOLD).text_color(theme::text_selected()).child(n.author.name.clone())).child(when_el).child(div().flex_1()).child(actions).children(resolve);
    let body: AnyElement = if editing {
        let field = this.tabs.iter().find(|t| t.id == tab).and_then(|t| t.mr_views.get(&number)).and_then(|u| u.note_edit.get(&n.id).cloned());
        match field {
            Some(f) => {
                let ready = !f.read(cx).text(cx).trim().is_empty() && f.read(cx).text(cx) != n.body;
                let busy = this.tabs.iter().find(|t| t.id == tab).and_then(|t| t.mr_views.get(&number)).is_some_and(|u| u.note_busy.as_ref() == Some(&n.id));
                let (id1, id2, did) = (n.id.clone(), n.id.clone(), d.id.clone());
                div().flex().flex_col().gap(px(6.)).child(f).child(div().flex().justify_end().gap(px(6.)).child(crate::ui::anchor(format!("{key}:cancel"), mr_button(ElementId::Name(format!("{key}:cancel").into()), true).child("Cancel").on_click(cx.listener(move |v, _, _, cx| {
                    if let Some(ui) = v.mr_ui(tab, number) {
                        ui.note_edit.remove(&id1);
                    }
                    cx.notify();
                })))).child(crate::ui::anchor(format!("{key}:save"), super::header::mr_primary(ElementId::Name(format!("{key}:save").into()), ready && !busy).child(if busy { "Saving…" } else { "Save" }).when(ready && !busy, |e| e.on_click(cx.listener(move |v, _, _, cx| v.mr_save_note(tab, number, did.clone(), id2.clone(), cx))))))).into_any_element()
            }
            None => div().into_any_element(),
        }
    } else {
        let text = signed_attachments(&n.body, n.body_html.as_deref());
        if text.len() > MR_BODY_MAX_BYTES {
            div().whitespace_normal().text_size(px(12.)).child(text).into_any_element()
        } else {
            let position = d.notes.iter().find_map(|x| x.position.clone());
            let base = crate::forge::review::apply::snippet_base(super::flavor_of(kind), position.as_ref());
            let applies = apply_buttons(this, tab, kind, number, &text, base.as_ref(), position.as_ref(), &n.id, cx);
            match this.mr_body_on(tab, number, &n.id, &text, base, cx) {
                Some(view) => div().flex().flex_col().child(view).children(applies).into_any_element(),
                None => div().into_any_element(),
            }
        }
    };
    let pills = reaction_pills(tab, kind, number, d, n, cx);
    let picker = picker_here.then(|| reaction_picker(tab, kind, number, d, n, cx));
    div()
        .group(group)
        .relative()
        .flex()
        .flex_col()
        .min_w_0()
        .when(!(avatar || in_card), |e| e.px(px(12.)))
        .py(px(if first { 8. } else { 6. }))
        .when(!first, |e| e.border_t_1().border_color(theme::section_border()))
        .child(head)
        .children(where_)
        .child(div().mt(px(2.)).min_w_0().text_size(px(12.)).child(body))
        .children(pills)
        .children(picker)
        .into_any_element()
}

fn reaction_pills(tab: u32, kind: ForgeKind, number: u64, d: &ForgeDiscussion, n: &ForgeNote, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    if n.reactions.is_empty() {
        return None;
    }
    let mut row = div().flex().flex_wrap().gap(px(4.)).mt(px(6.));
    for r in &n.reactions {
        let glyph = known_glyph(kind, &r.name).map_or_else(|| crate::md::emoji::emojify(&format!(":{}:", r.name)), str::to_string);
        let (did, nid, name) = (d.id.clone(), n.id.clone(), r.name.clone());
        let pill = div()
            .id(ElementId::Name(format!("mr-pill:{}:{}", n.id, r.name).into()))
            .flex()
            .items_center()
            .gap(px(4.))
            .h(px(22.))
            .px(px(8.))
            .rounded(px(11.))
            .border_1()
            .border_color(if r.mine { theme::mix(theme::blue(), 0.6, transparent_black()) } else { theme::section_border() })
            .bg(if r.mine { theme::mix(theme::blue(), 0.14, transparent_black()) } else { theme::panel_bg() })
            .text_size(px(12.))
            .text_color(theme::text_normal())
            .cursor_pointer()
            .hover(|d| d.border_color(theme::text_disabled()))
            .child(glyph)
            .child(r.count.to_string())
            .on_click(cx.listener(move |v, _, _, cx| v.mr_toggle_reaction(tab, number, did.clone(), nid.clone(), name.clone(), cx)));
        let anchored = crate::ui::anchor(format!("mr-pill:{}:{}", n.id, r.name), pill);
        row = row.child(if r.users.is_empty() { anchored.into_any_element() } else { tip(format!("{} reacted with :{}:", r.users.join(", "), r.name), anchored).into_any_element() });
    }
    Some(row.into_any_element())
}

/// Where the reaction picker is open.
#[derive(Debug, Clone, PartialEq)]
pub struct PickerAt {
    pub discussion: String,
    pub note: String,
}

fn reaction_picker(tab: u32, kind: ForgeKind, number: u64, d: &ForgeDiscussion, n: &ForgeNote, cx: &mut Context<GitVolt>) -> AnyElement {
    let set: &[(&str, &str)] = if kind == ForgeKind::GitLab { &GITLAB_COMMON } else { &GITHUB_REACTIONS };
    let mut grid = div().flex().flex_wrap().gap(px(2.)).w(px(8. * 28. + 7. * 2.));
    for (name, glyph) in set {
        let mine = n.reactions.iter().any(|r| r.name == *name && r.mine);
        let (did, nid, nm) = (d.id.clone(), n.id.clone(), name.to_string());
        grid = grid.child(crate::ui::anchor(format!("mr-pick:{name}"), div().id(ElementId::Name(format!("mr-pick:{name}").into())).flex().size(px(28.)).items_center().justify_center().rounded(px(4.)).text_size(px(16.)).cursor_pointer().when(mine, |d| d.bg(theme::mix(theme::blue(), 0.18, transparent_black()))).hover(|d| d.bg(theme::control_bg())).child(glyph.to_string()).on_click(cx.listener(move |v, _, _, cx| {
            if let Some(ui) = v.mr_ui(tab, number) {
                ui.reaction_picker = None;
            }
            v.mr_toggle_reaction(tab, number, did.clone(), nid.clone(), nm.clone(), cx);
        }))));
    }
    let search = (kind == ForgeKind::GitLab).then(|| reaction_search(tab, kind, number, d, n, cx));
    let hits = search.as_ref().map_or(Vec::new(), |(_, h)| h.clone());
    let search = search.map(|(el, _)| el);
    let mut found = div().flex().flex_wrap().gap(px(2.)).w(px(8. * 28. + 7. * 2.));
    for (name, glyph) in hits.iter().take(24) {
        let mine = n.reactions.iter().any(|r| r.name == *name && r.mine);
        let (did, nid, nm) = (d.id.clone(), n.id.clone(), name.clone());
        found = found.child(crate::ui::anchor(format!("mr-pick:{name}"), div().id(ElementId::Name(format!("mr-pick:{name}").into())).flex().size(px(28.)).items_center().justify_center().rounded(px(4.)).cursor_pointer().when(mine, |e| e.bg(theme::selected_row())).hover(|e| e.bg(theme::control_hover_bg())).child(glyph.clone()).on_click(cx.listener(move |v, _, _, cx| {
            if let Some(ui) = v.mr_ui(tab, number) {
                ui.reaction_picker = None;
            }
            STATE_SEARCH.with_borrow_mut(|s| *s = None);
            v.mr_toggle_reaction(tab, number, did.clone(), nid.clone(), nm.clone(), cx);
        }))));
    }
    let searching = !hits.is_empty();
    deferred(div().id("mr-reaction-picker").occlude().absolute().right(px(12.)).top(px(32.)).flex().flex_col().gap(px(6.)).p(px(6.)).rounded(px(8.)).border_1().border_color(theme::section_border()).bg(theme::panel_bg()).shadow(crate::ui::shadow(theme::shadow_popup(), 4., 16.)).on_mouse_down_out(cx.listener(move |v, _, _, cx| {
        if let Some(ui) = v.mr_ui(tab, number) {
            ui.reaction_picker = None;
        }
        cx.notify();
    })).children(search).when(searching, |e| e.child(found)).when(!searching, |e| e.child(grid))).with_priority(2).into_any_element()
}

/// GitLab takes any emoji: a search of every name. The hits come with the field that makes them.
fn reaction_search(tab: u32, _kind: ForgeKind, number: u64, _d: &ForgeDiscussion, _n: &ForgeNote, cx: &mut Context<GitVolt>) -> (AnyElement, Vec<(String, String)>) {
    let existing = STATE_SEARCH.with_borrow(|s| s.clone());
    let input = existing.unwrap_or_else(|| {
        let input = cx.new(|cx| crate::ui::input::TextInput::new(cx).placeholder("Search emoji"));
        cx.subscribe(&input, move |_, _, e: &crate::ui::input::InputEvent, cx| {
            if matches!(e, crate::ui::input::InputEvent::Changed) {
                cx.notify();
            }
        })
        .detach();
        STATE_SEARCH.with_borrow_mut(|s| *s = Some(input.clone()));
        input
    });
    let _ = (tab, number);
    let q = input.read(cx).text().trim().trim_matches(':').to_lowercase();
    let hits = if q.is_empty() { Vec::new() } else { crate::md::complete::rank_emoji(&q, 24).into_iter().map(|h| (h.name, h.emoji)).collect() };
    let el = div().px(px(8.)).py(px(4.)).rounded(px(4.)).border_1().border_color(theme::wip_input_border()).bg(theme::wip_input_bg()).text_size(px(12.)).child(input).into_any_element();
    (el, hits)
}

thread_local! {
    /// The picker's search; one picker's open at a time.
    static STATE_SEARCH: std::cell::RefCell<Option<Entity<crate::ui::input::TextInput>>> = const { std::cell::RefCell::new(None) };
}

pub fn description(this: &mut GitVolt, tab: u32, kind: ForgeKind, mr: &ForgeMr, detail: Option<&ForgeMrDetail>, cx: &mut Context<GitVolt>) -> AnyElement {
    let _ = kind;
    let body: AnyElement = match detail {
        None => div().text_size(px(11.)).text_color(theme::text_secondary()).child("Loading…").into_any_element(),
        Some(d) if d.description.trim().is_empty() => div().text_size(px(11.)).text_color(theme::text_secondary()).child("No description").into_any_element(),
        Some(d) => {
            let text = signed_attachments(&d.description, d.body_html.as_deref());
            if text.len() > MR_BODY_MAX_BYTES {
                div().text_size(px(12.)).child(text).into_any_element()
            } else {
                this.mr_body(tab, mr.number, "description", &text, cx).map_or(div().into_any_element(), |v| v.into_any_element())
            }
        }
    };
    let merge_above = matches!(mr.state, gitvolt_core::forge::types::MrState::Open | gitvolt_core::forge::types::MrState::Draft | gitvolt_core::forge::types::MrState::Merging);
    div().id("mr-description").flex().flex_col().gap(px(4.)).flex_none().min_w_0().text_size(px(12.)).when(!merge_above, |d| d.pt(px(8.)).border_t_1().border_color(theme::section_border())).child(body).into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn discussion_card(this: &mut GitVolt, tab: u32, kind: ForgeKind, mr: &ForgeMr, d: &ForgeDiscussion, roll_resolved: bool, show_where: bool, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let number = mr.number;
    let notes: Vec<&ForgeNote> = d.notes.iter().filter(|n| !n.system).collect();
    let first = *notes.first()?;
    let replies: Vec<&ForgeNote> = notes[1..].to_vec();
    let chosen = this.tabs.iter().find(|t| t.id == tab).and_then(|t| t.mr_views.get(&number)).and_then(|u| u.folds.get(&d.id).copied());
    let rolls = roll_resolved && d.resolved;
    let rolled = rolls && !chosen.unwrap_or(false);
    let open = replies.is_empty() || if rolls { !rolled } else { chosen.unwrap_or(!d.resolved) };
    let pos = d.notes.iter().find_map(|n| n.position.as_ref());
    // In a card at its line in the diff, the card is its frame.
    let card = || div().flex().flex_col().min_w_0().flex_1().when(show_where, |d| d.rounded(px(8.)).border_1().border_color(theme::section_border()).bg(theme::panel_bg1())).text_color(theme::text_normal());
    let side_pad = if show_where { 12. } else { 0. };
    let did = d.id.clone();
    let set_fold = move |v: &mut GitVolt, on: bool, cx: &mut Context<GitVolt>| {
        if let Some(ui) = v.mr_ui(tab, number) {
            ui.folds.insert(did.clone(), on);
        }
        cx.notify();
    };
    let fold_a = set_fold.clone();
    let roll_btn = |unroll: bool, fold: FoldFn, cx: &mut Context<GitVolt>| {
        let id = format!("mr-roll:{}:{unroll}", d.id);
        crate::ui::anchor(format!("mr-roll:{}", d.id), div().id(ElementId::Name(id.into())).flex().flex_none().size(px(22.)).items_center().justify_center().ml(px(if unroll { -4. } else { 0. })).rounded(px(4.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).child(icon(if unroll { "chevron-right" } else { "chevron-down" }, 14., theme::text_secondary())).on_click(cx.listener(move |v, _, _, cx| {
            cx.stop_propagation();
            fold(v, unroll, cx);
        })))
        .into_any_element()
    };
    if rolled {
        let lead = roll_btn(true, Box::new(fold_a.clone()), cx);
        let (m2, p2) = (mr.clone(), pos.cloned());
        let fold_b = fold_a.clone();
        let line = div()
            .flex()
            .flex_wrap()
            .flex_1()
            .min_w_0()
            .items_baseline()
            .gap_x(px(3.))
            .text_color(theme::text_secondary())
            .child(div().font_weight(FontWeight::BOLD).text_color(theme::text_selected()).child(first.author.name.clone()))
            .child("started a thread")
            .when_some(pos.filter(|_| show_where), |e, p| {
                let text = note_where(p);
                e.child("on").child(div().id(ElementId::Name(format!("mr-where:{}", d.id).into())).font_family(crate::app::MONO_FONT).text_size(px(11.)).text_color(theme::link()).cursor_pointer().hover(|d| d.underline()).child(text).on_click(cx.listener(move |v, _, _, cx| {
                    cx.stop_propagation();
                    if let Some(p) = &p2 {
                        v.mr_open_note_file(tab, kind, &m2, p, cx);
                    }
                })))
            })
            .child(div().ml(px(6.)).child(when(this, first.created_at)));
        let resolved_at = d.resolved_at.map(|at| crate::format::relative_time(at, crate::format::now()));
        let resolved = div().flex().flex_wrap().gap_x(px(3.)).pl(px(24.)).text_size(px(11.)).text_color(theme::text_secondary()).child("Resolved").when_some(resolved_at, |e, w| e.child(w)).when_some(d.resolved_by.clone(), |e, b| e.child("by").child(div().text_color(theme::text_normal()).child(b)));
        let mut c = card().child(div().id(ElementId::Name(format!("mr-rolled:{}", d.id).into())).flex().flex_col().px(px(12.)).py(px(8.)).cursor_pointer().on_click(cx.listener(move |v, _, _, cx| fold_b(v, true, cx))).child(div().flex().items_center().gap(px(6.)).min_h(px(22.)).text_size(px(12.)).child(lead).child(line)).child(resolved));
        if !replies.is_empty() {
            c = c.child(replies_row(this, &replies, false, Box::new(fold_a.clone()), &d.id, side_pad, cx));
        }
        return Some(c.into_any_element());
    }
    let lead = rolls.then(|| roll_btn(false, Box::new(fold_a.clone()), cx));
    let where_ = pos.filter(|_| show_where).map(|p| {
        let (m2, p2) = (mr.clone(), p.clone());
        let over = first.body.lines().any(|l| l.trim_start().starts_with("```suggestion"));
        div()
            .flex()
            .flex_col()
            .pt(px(2.))
            .child(div().id(ElementId::Name(format!("mr-where:{}", d.id).into())).font_family(crate::app::MONO_FONT).text_size(px(11.)).text_color(theme::link()).cursor_pointer().hover(|d| d.underline()).child(note_where(p)).on_click(cx.listener(move |v, _, _, cx| v.mr_open_note_file(tab, kind, &m2, &p2, cx))))
            .when_some(p.snippet.clone().filter(|_| !over), |e, s| e.child(div().mt(px(6.)).px(px(8.)).py(px(5.)).rounded(px(4.)).bg(theme::app_bg()).font_family(crate::app::MONO_FONT).text_size(px(11.)).line_height(px(16.5)).text_color(theme::text_secondary()).overflow_hidden().whitespace_nowrap().children(s.lines().map(|l| div().child(if l.is_empty() { " ".to_string() } else { l.to_string() })).collect::<Vec<_>>())))
            .into_any_element()
    });
    let mut c = card().child(note_view(this, tab, kind, mr, d, first, true, !show_where, !show_where, lead, where_, cx));
    if !replies.is_empty() {
        c = c.child(replies_row(this, &replies, open, Box::new(fold_a.clone()), &d.id, side_pad, cx));
    }
    if open {
        for n in d.notes.iter().filter(|n| n.id != first.id) {
            if n.system {
                let p = parse_system_note(&n.body, &mr.web_url);
                let parts = note_parts(&p.parts, &format!("mr-sys:{}", n.id), cx);
                c = c.child(div().flex().items_start().gap(px(8.)).px(px(side_pad)).py(px(4.)).border_t_1().border_color(theme::section_border()).text_size(px(12.)).text_color(theme::text_secondary()).min_w_0().child(node(p.kind)).child(event_line(this, &n.author.name, parts, n.created_at).flex_1().min_w_0()));
            } else {
                c = c.child(note_view(this, tab, kind, mr, d, n, false, false, !show_where, None, None, cx));
            }
        }
        // A discussion's own reply (GitLab: any; GitHub, Forgejo: review threads). It opens by itself when a draft waits in it.
        if kind == ForgeKind::GitLab || d.id.starts_with("thread-") {
            let has_draft = this.tabs.iter().find(|t| t.id == tab).and_then(|t| t.mr_views.get(&number)).is_some_and(|u| u.reply_open.contains(&d.id) || u.replies.get(&d.id).is_some_and(|f| !f.read(cx).text(cx).is_empty()));
            let foot = div().flex().flex_col().items_start().gap(px(6.)).px(px(side_pad)).py(px(8.)).border_t_1().border_color(theme::section_border());
            if has_draft {
                if let Some(field) = this.mr_field(tab, number, FieldKey::Reply(d.id.clone()), cx) {
                    c = c.child(foot.child(div().w_full().child(reply_box(this, tab, kind, number, Some(d), field, Some(d.id.clone()), cx))));
                }
            } else {
                let did = d.id.clone();
                c = c.child(foot.child(crate::ui::anchor(format!("mr-reply-open:{}", d.id), mr_button(ElementId::Name(format!("mr-reply-open:{}", d.id).into()), true).child(icon("corner-down-left", 12., theme::text_normal())).child("Reply").on_click(cx.listener(move |v, _, window, cx| {
                    if let Some(ui) = v.mr_ui(tab, number) {
                        ui.reply_open.insert(did.clone());
                    }
                    if let Some(f) = v.mr_field(tab, number, FieldKey::Reply(did.clone()), cx) {
                        f.update(cx, |f, cx| f.focus(window, cx));
                    }
                    cx.notify();
                })))));
            }
        }
    }
    Some(c.into_any_element())
}

#[allow(clippy::too_many_arguments)]
fn replies_row(this: &GitVolt, replies: &[&ForgeNote], open: bool, fold: FoldFn, id: &str, side_pad: f32, cx: &mut Context<GitVolt>) -> AnyElement {
    let n = replies.len();
    let mut people: Vec<&gitvolt_core::forge::types::ForgeUser> = Vec::new();
    for r in replies {
        if !people.iter().any(|p| p.username == r.author.username) && people.len() < 3 {
            people.push(&r.author);
        }
    }
    let last = replies[n - 1];
    let toggle = div().flex().items_center().gap(px(6.)).py(px(2.)).text_color(theme::text_normal()).child(icon(if open { "chevron-down" } else { "chevron-right" }, 14., theme::text_normal())).when(open, |d| d.child("Collapse replies")).when(!open, |d| {
        d.child(div().flex().children(people.iter().enumerate().map(|(i, u)| div().when(i > 0, |d| d.ml(px(-6.))).child(forge_avatar(u, 18.))))).child(format!("{n} {}", if n == 1 { "reply" } else { "replies" }))
    });
    div()
        .id(ElementId::Name(format!("mr-replies:{id}").into()))
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(side_pad))
        .py(px(4.))
        .border_t_1()
        .border_color(theme::section_border())
        .text_size(px(12.))
        .text_color(theme::text_secondary())
        .cursor_pointer()
        .on_click(cx.listener(move |v, _, _, cx| fold(v, !open, cx)))
        .child(toggle)
        .when(!open, |d| d.child(div().flex().gap(px(6.)).child(format!("Last reply by {}", last.author.name)).child(when(this, last.created_at))))
        .into_any_element()
}

pub fn activity(this: &mut GitVolt, tab: u32, kind: ForgeKind, mr: &ForgeMr, detail: Option<&ForgeMrDetail>, _window: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let number = mr.number;
    let t = this.tabs.iter().find(|t| t.id == tab);
    let discussions = t.and_then(|t| t.forge.discussions.get(&number).cloned());
    let shown_tab = t.and_then(|t| t.mr_views.get(&number)).map_or(ActivityTab::Activity, |u| u.tab);
    let reviews = if kind == ForgeKind::GitLab { Vec::new() } else { detail.map_or(Vec::new(), |d| d.mr.review.reviews.clone()) };
    let entries = entries_of(discussions.as_deref().unwrap_or_default(), &reviews);
    let threads: Vec<&Entry> = entries.iter().filter(|e| matches!(e, Entry::Thread(_))).collect();
    let diffs: Vec<&Entry> = threads.iter().filter(|e| matches!(e, Entry::Thread(d) if d.notes.iter().any(|n| n.position.is_some()))).copied().collect();
    let counts = (threads.len(), diffs.len());
    let shown: Vec<Entry> = match shown_tab {
        ActivityTab::Activity => entries.clone(),
        ActivityTab::Comments => threads.iter().map(|e| (*e).clone()).collect(),
        ActivityTab::Diff => diffs.iter().map(|e| (*e).clone()).collect(),
    };
    let mut tabs = div().flex().gap(px(2.)).border_b_1().border_color(theme::section_border());
    for (id, label, count, name) in [(ActivityTab::Activity, "Activity", None, "activity"), (ActivityTab::Comments, "Comments", Some(counts.0), "comments"), (ActivityTab::Diff, "Diff notes", Some(counts.1), "diff")] {
        let on = shown_tab == id;
        tabs = tabs.child(crate::ui::anchor(format!("mr-tab:{name}"), div().id(ElementId::Name(format!("mr-tab:{name}").into())).flex().items_center().px(px(10.)).py(px(6.)).border_b_2().border_color(if on { theme::blue() } else { transparent_black() }).text_size(px(12.)).text_color(if on { theme::text_selected() } else { theme::text_secondary() }).cursor_pointer().hover(|d| d.text_color(theme::text_normal())).child(label).when_some(count, |d, c| d.child(div().ml(px(4.)).px(px(6.)).rounded(px(8.)).bg(theme::control_bg()).text_size(px(11.)).child(c.to_string()))).on_click(cx.listener(move |v, _, _, cx| {
            if let Some(ui) = v.mr_ui(tab, number) {
                ui.tab = id;
            }
            cx.notify();
        }))));
    }
    let mut timeline = div().relative().flex().flex_col().gap(px(10.)).pl(px(26.)).child(div().absolute().left(px(10.)).top(px(6.)).bottom(px(6.)).w(px(2.)).bg(theme::section_border()));
    if discussions.is_none() {
        timeline = timeline.child(div().flex().justify_center().py(px(12.)).child(crate::commit_box::spinner("mr-threads-spin", 14., theme::text_secondary())));
    } else if shown.is_empty() {
        timeline = timeline.child(div().text_size(px(11.)).text_color(theme::text_secondary()).child(if shown_tab == ActivityTab::Diff { "No diff notes" } else { "No comments yet" }));
    }
    for e in &shown {
        let ev = div().relative().min_w_0().text_size(px(12.)).text_color(theme::text_secondary());
        match e {
            Entry::System(n) => {
                let p = parse_system_note(&n.body, &mr.web_url);
                let parts = note_parts(&p.parts, &format!("mr-sys:{}", n.id), cx);
                timeline = timeline.child(ev.child(div().absolute().left(px(-26.)).top_0().child(node(p.kind))).child(event_line(this, &n.author.name, parts, n.created_at)));
            }
            Entry::Review(r, at) => {
                let (k, text) = match r.state {
                    ReviewState::Approved => (SystemKind::Approved, "approved these changes"),
                    ReviewState::ChangesRequested => (SystemKind::Unapproved, "requested changes"),
                    _ => (SystemKind::Review, "reviewed"),
                };
                timeline = timeline.child(ev.child(div().absolute().left(px(-26.)).top_0().child(node(k))).child(event_line(this, &r.user.name, vec![div().child(text).into_any_element()], *at)));
            }
            Entry::Thread(d) => {
                let Some(first) = d.notes.iter().find(|n| !n.system) else { continue };
                let author = first.author.clone();
                let Some(card) = discussion_card(this, tab, kind, mr, d, true, true, cx) else { continue };
                timeline = timeline.child(ev.flex().child(div().absolute().left(px(-26.)).top(px(9.)).child(div().rounded_full().overflow_hidden().bg(theme::panel_bg()).child(forge_avatar(&author, 22.)))).child(card));
            }
        }
    }
    div().id("mr-activity").flex().flex_col().flex_none().gap(px(10.)).min_w_0().child(tabs).child(timeline).into_any_element()
}

/// A thread drawn at its line in the diff (`showWhere` off).
pub fn thread_in_diff(this: &mut GitVolt, tab: u32, kind: ForgeKind, mr: &ForgeMr, d: &ForgeDiscussion, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    discussion_card(this, tab, kind, mr, d, false, false, cx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn pos(line: Option<u32>, start: Option<u32>, old: Option<u32>, start_old: Option<u32>) -> DiffPosition {
        DiffPosition { path: "a.rs".into(), old_path: None, line, old_line: old, snippet: None, start_line: start, start_old_line: start_old, head_sha: None, outdated: false }
    }

    #[test]
    fn a_note_says_where_it_is() {
        assert_eq!(note_where(&pos(Some(5), None, None, None)), "a.rs:5");
        assert_eq!(note_where(&pos(Some(7), Some(5), None, None)), "a.rs:5-7");
        assert_eq!(note_where(&pos(None, None, Some(3), None)), "a.rs:3");
        assert_eq!(note_where(&pos(None, None, None, None)), "a.rs");
        // A range from a removed line to an added one is its last line alone.
        assert_eq!(note_line(&pos(Some(7), None, Some(2), Some(1))), Some((true, 7, None)));
    }

    #[test]
    fn a_reaction_toggles_on_and_off() {
        let mine = toggled(&[], "heart", true, Some("ada"));
        assert_eq!(mine[0].count, 1);
        assert!(mine[0].mine);
        assert!(toggled(&mine, "heart", false, Some("ada")).is_empty());
        let others = vec![ForgeReaction { name: "heart".into(), count: 2, mine: false, users: vec!["x".into(), "y".into()] }];
        let two = toggled(&others, "heart", true, Some("ada"));
        assert_eq!(two[0].count, 3);
        let back = toggled(&two, "heart", false, Some("ada"));
        assert_eq!(back, others);
    }

    #[test]
    fn a_quote_goes_after_whats_there() {
        assert_eq!(quoted("", "a\n\nb\n"), "> a\n>\n> b\n\n");
        assert_eq!(quoted("hi\n", "a"), "hi\n\n> a\n\n");
    }

    #[test]
    fn only_what_can_change_says_so() {
        let d = |id: &str| ForgeDiscussion { id: id.into(), notes: vec![], resolvable: false, resolved: false, resolved_by: None, resolved_at: None };
        assert!(!changeable_thread(ForgeKind::GitHub, &d("review-1")));
        assert!(changeable_thread(ForgeKind::GitLab, &d("review-1")));
        assert!(deletable_note(ForgeKind::Forgejo, &d("issue-1")));
        assert!(!deletable_note(ForgeKind::Forgejo, &d("thread-1")));
    }
}
