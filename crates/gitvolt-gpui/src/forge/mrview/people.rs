//! The view's reviewers, assignees and labels (`MrPeople.tsx`, `PeopleCard.tsx`, `peopleLimits.ts`). + and × change the reviewers and assignees at once:
//! shown before the forge answers, put back if it refuses, one change at a time per MR. A project that allows one makes + a swap.

use super::header::{card_btn, fact, fact_k};
use crate::app::{GitVolt, core_repo, icon};
use crate::forge::card::forge_avatar;
use crate::forge::search::{self, SearchOption, Source};
use crate::forge::text::{chip_text_is_black, mr_ref};
use crate::theme;
use crate::ui::tooltip::tip;
use gitvolt_core::api::Request;
use gitvolt_core::forge::types::{ForgeMr, ForgeMrDetail, ForgeUser, MrEdit, MrState, PeopleEdit, PeopleLimits, ReviewState};
use gitvolt_core::forge::ForgeKind;
use gpui::prelude::*;
use gpui::*;
use std::rc::Rc;

/// An open search: only one at a time.
pub struct Pop;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Reviewers,
    Assignees,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Reviewers => "reviewers",
            Role::Assignees => "assignees",
        }
    }

    pub fn noun(self) -> &'static str {
        match self {
            Role::Reviewers => "reviewer",
            Role::Assignees => "assignee",
        }
    }
}

pub fn max_of(limits: Option<&PeopleLimits>, role: Role) -> Option<u32> {
    limits.and_then(|l| if role == Role::Reviewers { l.max_reviewers } else { l.max_assignees })
}

/// The + button's tooltip at the cap.
pub fn limit_tip(kind: ForgeKind, role: Role, max: u32) -> String {
    if max == 1 {
        return format!("This project allows one {}", role.noun());
    }
    format!("{} allows up to {max} {}s", if kind == ForgeKind::GitHub { "GitHub" } else { "This project" }, role.noun())
}

pub fn people_of(d: &mut ForgeMrDetail, role: Role) -> &mut Vec<ForgeUser> {
    if role == Role::Reviewers { &mut d.reviewers } else { &mut d.assignees }
}

impl GitVolt {
    fn mr_load_limits(&mut self, tab: u32, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        if t.forge.limits_asked {
            return;
        }
        let Some(remote) = t.forge.remote.clone() else { return };
        t.forge.limits_asked = true;
        self.forge_ask::<PeopleLimits>(Request::ForgePeopleLimits { repo: core_repo(tab), remote }, cx, move |v, answer, _| {
            let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) else { return };
            match answer {
                // A limit learned meanwhile stays.
                Ok(l) => {
                    let cur = t.forge.limits.as_ref();
                    t.forge.limits = Some(PeopleLimits { max_reviewers: cur.and_then(|c| c.max_reviewers).or(l.max_reviewers), max_assignees: cur.and_then(|c| c.max_assignees).or(l.max_assignees) });
                }
                Err(_) => t.forge.limits_asked = false,
            }
        });
    }

    /// `changeMrPeople`: shown at once in the detail, then sent; a refusal puts it back and a toast says why. No confirm: it's undone the same way.
    pub fn mr_change_people(&mut self, tab: u32, kind: ForgeKind, number: u64, role: Role, add: Option<ForgeUser>, remove: Option<ForgeUser>, replace: Vec<ForgeUser>, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        let Some(d) = t.forge.details.get_mut(&number) else { return };
        let list = people_of(d, role);
        let before = list.clone();
        let who = add.as_ref().or(remove.as_ref()).cloned();
        let Some(who) = who else { return };
        let adding = add.is_some();
        let replaced: Vec<ForgeUser> = if adding { replace.into_iter().filter(|u| u.id != who.id).collect() } else { Vec::new() };
        if adding {
            if !list.iter().any(|u| u.id == who.id) {
                list.retain(|u| !replaced.iter().any(|r| r.id == u.id));
                list.push(who.clone());
            }
        } else {
            list.retain(|u| u.id != who.id);
        }
        let r = mr_ref(kind, number);
        let failure = if adding { format!("Couldn't add {} as {} on {r}", who.name, if role == Role::Reviewers { "a reviewer" } else { "an assignee" }) } else { format!("Couldn't remove {} from the {} of {r}", who.name, role.name()) };
        let people = if adding { PeopleEdit { add: vec![who.id], remove: replaced.iter().map(|u| u.id).collect() } } else { PeopleEdit { add: Vec::new(), remove: vec![who.id] } };
        let edit = MrEdit { title: None, description: None, labels: None, reviewers: (role == Role::Reviewers).then(|| people.clone()), assignees: (role == Role::Assignees).then_some(people) };
        // One after the other: GitLab takes the whole lists, made from the MR as the forge has it then.
        t.forge.people_queue.push_back(super::actions::PeopleWrite { number, edit, failure, role, before, who_id: who.id, adding });
        self.mr_people_next(tab, cx);
        cx.notify();
    }

    fn mr_people_next(&mut self, tab: u32, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        if t.forge.people_busy {
            return;
        }
        let Some(w) = t.forge.people_queue.pop_front() else { return };
        t.forge.people_busy = true;
        let (number, edit) = (w.number, w.edit.clone());
        self.forge_ask::<ForgeMr>(Request::ForgeEditMr { repo: core_repo(tab), number, edit }, cx, move |v, answer, cx| {
            if let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) {
                t.forge.people_busy = false;
            }
            match answer {
                Ok(m) => {
                    v.forge_written(tab, cx);
                    v.forge_put_mr(tab, m, cx);
                }
                Err(e) => {
                    if let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab)
                        && let Some(d) = t.forge.details.get_mut(&number)
                    {
                        let list = people_of(d, w.role);
                        if w.adding {
                            list.retain(|u| u.id != w.who_id);
                        }
                        *list = w.before.clone();
                    }
                    crate::ui::toast::show_error(format!("{}: {e}", w.failure), cx);
                }
            }
            v.mr_people_next(tab, cx);
        });
    }
}

/// A person's chip.
pub fn person_chip(user: &ForgeUser, label: &str, remove: Option<AnyElement>) -> Div {
    div().flex().flex_none().items_center().gap(px(5.)).h(px(24.)).max_w_full().pl(px(3.)).pr(px(if remove.is_some() { 4. } else { 8. })).rounded(px(12.)).bg(theme::control_bg()).text_size(px(12.)).text_color(theme::text_normal()).child(forge_avatar(user, 18.)).child(div().min_w_0().truncate().child(label.to_string())).children(remove)
}

/// A label's pill, in the forge's colour with the text that reads on it.
pub fn label_pill(label: &str, color: Option<&str>, remove: Option<AnyElement>) -> Div {
    let bg = color.and_then(search::colour_of);
    let fg = color.and_then(chip_text_is_black).map(|black| if black { gpui::black() } else { gpui::white() });
    div().flex().flex_none().items_center().gap(px(4.)).h(px(22.)).max_w_full().pl(px(9.)).pr(px(if remove.is_some() { 4. } else { 9. })).rounded(px(11.)).bg(bg.unwrap_or(theme::control_bg())).text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(fg.unwrap_or(theme::text_normal())).child(crate::md::emoji::emojify(label)).children(remove)
}

pub fn rm_button(key: String, name: &str, run: impl Fn(&mut GitVolt, &mut Context<GitVolt>) + 'static, cx: &mut Context<GitVolt>) -> AnyElement {
    crate::ui::anchor(key.clone(), div().id(ElementId::Name(key.into())).flex().flex_none().size(px(16.)).items_center().justify_center().rounded_full().cursor_pointer().hover(|d| d.bg(theme::control_hover_bg())).child(icon("x", 11., theme::text_disabled())).on_click(cx.listener(move |v, _, _, cx| {
        cx.stop_propagation();
        run(v, cx);
        cx.notify();
    })))
    .into_any_element()
    .tap_name(name)
}

trait TapName {
    fn tap_name(self, name: &str) -> Self;
}

impl TapName for AnyElement {
    fn tap_name(self, _: &str) -> Self {
        self
    }
}

/// `+` in a card's head: opens the search for reviewers or assignees.
fn add_button(this: &mut GitVolt, tab: u32, kind: ForgeKind, mr: &ForgeMr, detail: &ForgeMrDetail, role: Role, cx: &mut Context<GitVolt>) -> AnyElement {
    let t = this.tabs.iter().find(|t| t.id == tab);
    let max = max_of(t.and_then(|t| t.forge.limits.as_ref()), role);
    let list = if role == Role::Reviewers { &detail.reviewers } else { &detail.assignees };
    let count = list.len() as u32;
    let swap = max == Some(1) && count >= 1;
    let full = !swap && max.is_some_and(|m| count >= m);
    let name = format!("{} {}", if swap { "Replace" } else { "Add" }, role.noun());
    let text = if swap || full { max.map_or(name.clone(), |m| limit_tip(kind, role, m)) } else { name };
    let key = format!("mr-add-{}", role.name());
    let glyph = if swap { "arrow-left-right" } else { "plus" };
    let (number, current, api, rt) = (mr.number, list.clone(), this.api.clone(), this.rt.clone());
    let remote = t.and_then(|t| t.forge.remote.clone());
    let weak = cx.weak_entity();
    let open_key = key.clone();
    tip(
        text,
        crate::ui::anchor(
            key.clone(),
            card_btn(ElementId::Name(key.into())).when(full, |d| d.opacity(0.55)).child(icon(glyph, 13., theme::text_secondary())).when(!full, |d| {
                d.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    let Some(remote) = remote.clone() else { return };
                    let Some(at) = crate::ui::Ui::get(cx).anchor(&open_key) else { return };
                    let (w, current2) = (weak.clone(), current.clone());
                    let taken = current.iter().map(|u| u.id.to_string()).collect();
                    let source = Source::People { repo: core_repo(tab), remote };
                    let on_pick = Rc::new(move |o: &SearchOption, cx: &mut App| {
                        let Some(user) = o.user.clone() else { return };
                        let replace = if max == Some(1) { current2.clone() } else { Vec::new() };
                        w.update(cx, |v, cx| v.mr_change_people(tab, kind, number, role, Some(user), None, replace, cx)).ok();
                    });
                    let label = if role == Role::Reviewers { "Reviewers" } else { "Assignees" };
                    search::open(format!("mr-search-{}", role.name()).into(), label.into(), at, true, source, taken, api.clone(), rt.clone(), on_pick, window, cx);
                })
            }),
        ),
    )
    .into_any_element()
}

/// The three cards. `editLabels`: the Labels card's pencil opens the Edit form with its picker open.
pub fn people_cards(this: &mut GitVolt, tab: u32, kind: ForgeKind, mr: &ForgeMr, detail: Option<&ForgeMrDetail>, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    this.mr_load_limits(tab, cx);
    let number = mr.number;
    let live = matches!(mr.state, MrState::Open | MrState::Draft);
    let body = || div().flex().flex_1().items_center().min_w_0().pt(px(4.));
    let chips = || div().flex().flex_wrap().items_center().gap(px(4.)).min_w_0();
    let none = |t: &'static str| div().text_size(px(12.)).text_color(theme::text_disabled()).child(t);
    let mut out = Vec::new();
    for (role, id, label) in [(Role::Reviewers, "mr-card-reviewers", "Reviewers"), (Role::Assignees, "mr-card-assignees", "Assignees")] {
        let add = detail.filter(|_| live).map(|d| add_button(this, tab, kind, mr, d, role, cx));
        let list = detail.map(|d| if role == Role::Reviewers { d.reviewers.clone() } else { d.assignees.clone() });
        let mut c = chips();
        match &list {
            None => c = c.child(none("Loading…")),
            Some(l) if l.is_empty() => c = c.child(none("None")),
            Some(l) => {
                for u in l {
                    // GitHub (and Forgejo) keeps a submitted review: only a request still open can be withdrawn.
                    let keeps = role == Role::Reviewers && kind != ForgeKind::GitLab && detail.is_some_and(|d| d.mr.review.reviews.iter().any(|r| r.user.id == u.id && r.state != ReviewState::Pending && !r.requested));
                    let rm = (live && detail.is_some() && !keeps).then(|| {
                        let who = u.clone();
                        rm_button(format!("mr-rm-{}-{}", role.name(), u.username), &format!("Remove {}", u.name), move |v, cx| v.mr_change_people(tab, kind, number, role, None, Some(who.clone()), Vec::new(), cx), cx)
                    });
                    c = c.child(person_chip(u, &u.name, rm));
                }
            }
        }
        out.push(fact(id).child(div().flex().items_center().gap(px(6.)).child(fact_k(label)).children(add)).child(body().child(c)).into_any_element());
    }
    let mut c = chips();
    if mr.labels.is_empty() {
        c = c.child(none("None"));
    }
    for l in &mr.labels {
        c = c.child(label_pill(l, mr.label_colors.get(l).map(String::as_str), None));
    }
    let pencil = live.then(|| {
        tip(
            "Edit labels",
            crate::ui::anchor(
                "mr-edit-labels",
                card_btn("mr-edit-labels").child(icon("pencil", 12., theme::text_secondary())).on_click(cx.listener(move |v, _, window, cx| v.mr_open_edit(tab, number, true, window, cx))),
            ),
        )
    });
    out.push(fact("mr-card-labels").child(div().flex().items_center().gap(px(6.)).child(fact_k("Labels")).children(pencil)).child(body().child(c)).into_any_element());
    out
}
