//! The badge and the hover card (`MrBadge.tsx`, `MrCard.tsx`, `MrCardLive.tsx`): the card shows what the badge's data has at once, and
//! asks for the rest while it shows.

use super::section::{pipeline_icon, state_colour, state_icon, state_icon_coloured};
use super::text::{branches_parts, mr_ref, pipeline_text, review_text, state_label};
use crate::app::{GitVolt, icon};
use crate::theme;
use crate::ui::tooltip::{Anchor, Placement};
use gitvolt_core::forge::types::{ForgeMr, ForgeMrDetail, ForgeUser, MrState, ReviewDecision, ReviewState};
use gitvolt_core::forge::ForgeKind;
use gpui::prelude::*;
use gpui::*;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

/// What the graph's chips read of the shown tab's forge, kept between frames: a chip's width is measured where there's no tab.
#[derive(Default)]
struct ChipForge {
    tab: u32,
    kind: Option<ForgeKind>,
    remote_kinds: HashMap<String, ForgeKind>,
    by_ref: BTreeMap<String, ForgeMr>,
    upstreams: BTreeMap<String, String>,
    details: HashMap<u64, ForgeMrDetail>,
    errors: HashMap<u64, String>,
    fresh_at: HashMap<u64, i64>,
}

thread_local! {
    static CHIPS: RefCell<ChipForge> = RefCell::new(ChipForge::default());
}

pub fn set_chip_forge(tab: &crate::app::Tab, overrides: &serde_json::Value) {
    CHIPS.with(|c| {
        let mut c = c.borrow_mut();
        c.tab = tab.id;
        c.kind = tab.forge.kind;
        c.remote_kinds = super::section::remote_kinds(tab, overrides);
        if c.by_ref != tab.forge.by_ref {
            c.by_ref = tab.forge.by_ref.clone();
        }
        if c.upstreams != tab.forge.upstreams {
            c.upstreams = tab.forge.upstreams.clone();
        }
        if c.details != tab.forge.details {
            c.details = tab.forge.details.clone();
        }
        if c.errors != tab.forge.detail_errors {
            c.errors = tab.forge.detail_errors.clone();
        }
        if c.fresh_at != tab.forge.fresh_at {
            c.fresh_at = tab.forge.fresh_at.clone();
        }
    });
}

pub fn chip_remote_kind(remote: &str) -> Option<ForgeKind> {
    CHIPS.with(|c| c.borrow().remote_kinds.get(remote).copied())
}

/// A chip's MR/PR: through one of its remote refs, else through its local branch's upstream.
pub fn chip_badge(chip: &crate::model::Chip) -> Option<(u32, ForgeKind, ForgeMr)> {
    if chip.tag {
        return None;
    }
    CHIPS.with(|c| {
        let c = c.borrow();
        let kind = c.kind?;
        let mr = chip.remotes.iter().find_map(|(_, full)| c.by_ref.get(full)).or_else(|| {
            let local = format!("refs/heads/{}", chip.name);
            c.upstreams.get(&local).and_then(|u| c.by_ref.get(u))
        })?;
        Some((c.tab, kind, mr.clone()))
    })
}

/// A forge user's avatar: the picture the forge links to, else the email's, else initials.
pub fn forge_avatar(user: &ForgeUser, size: f32) -> Div {
    let url = user.avatar_url.as_deref().map(str::trim).filter(|u| !u.is_empty());
    if let Some(picture) = url.and_then(crate::avatars::image_for_url) {
        return div().flex_none().size(px(size)).rounded_full().overflow_hidden().bg(theme::avatar_backdrop()).child(img(picture).size(px(size)).rounded_full());
    }
    crate::details::avatar(&SharedString::from(user.name.clone()), user.email.as_deref().unwrap_or_default(), size)
}

/// `MrStateChip`: the state as a coloured chip, icon and word.
pub fn state_chip(s: MrState, font: f32, pad_x: f32, pad_y: f32) -> Div {
    let (bg, fg) = match s {
        MrState::Open => (theme::mix(theme::green(), 0.15, transparent_black()), theme::green()),
        MrState::Merged | MrState::Merging => (theme::mix(theme::purple(), 0.15, transparent_black()), theme::purple()),
        MrState::Closed => (theme::mix(theme::red(), 0.15, transparent_black()), theme::red()),
        MrState::Draft => (theme::control_bg(), theme::text_secondary()),
    };
    let _ = state_colour;
    div().flex().flex_none().items_center().gap(px(5.)).px(px(pad_x)).py(px(pad_y)).rounded(px(11.)).font_weight(FontWeight::SEMIBOLD).bg(bg).text_size(px(font)).text_color(fg).child(state_icon_coloured(s, 12., fg)).child(state_label(s))
}

fn has_review_news(d: &ForgeMrDetail) -> bool {
    let r = &d.mr.review;
    r.decision != ReviewDecision::None || r.approvals > 0 || r.approvals_required.unwrap_or(0) > 0 || r.reviews.iter().any(|x| x.state == ReviewState::Commented)
}

/// The card's body: state chip, number and title, the author, the branches, then only what needs a look.
pub fn mr_card(kind: ForgeKind, mr: &ForgeMr, detail: Option<&ForgeMrDetail>, error: Option<&str>, hint: Option<&str>, extra: Option<AnyElement>) -> Div {
    let m = detail.map_or(mr, |d| &d.mr);
    let meta = || div().text_size(px(11.)).text_color(theme::text_secondary());
    let (from, to) = branches_parts(m);
    let wait = error.map_or("Loading…".to_string(), |e| format!("Couldn't load: {e}"));
    // The reference and the title are one text, wrapping in the card beside the chip: as items of their own, a long title
    // took a line of its own and ran past the card. (The web's wraps under the chip too; here it hangs beside it.) The
    // title is the highlight: one on the reference, from the text's start, kept the title's colour.
    let r = mr_ref(kind, mr.number);
    let text = format!("{r} {}", crate::md::emoji::emojify(&m.title));
    let name = HighlightStyle { color: Some(theme::text_selected()), font_weight: Some(FontWeight::SEMIBOLD), ..Default::default() };
    let title = StyledText::new(text.clone()).with_highlights([(r.len()..text.len(), name)]);
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .max_w(px(320.))
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(4.))
                .text_color(theme::text_secondary())
                .child(state_chip(m.state, 10., 7., 0.).mr(px(2.)))
                .child(div().flex_1().min_w_0().child(title)),
        )
        .child(meta().flex().items_center().gap(px(5.)).min_h(px(16.)).child(forge_avatar(&m.author, 16.)).child(m.author.name.clone()))
        .child(meta().flex().items_center().gap(px(3.)).child(from).child(icon("arrow-right", 11., theme::text_secondary())).child(to))
        .when_some(m.pipeline.as_ref(), |d, p| d.child(meta().flex().items_center().gap(px(4.)).child(pipeline_icon(p, 12.)).child(pipeline_text(kind, Some(p)))))
        .when(detail.is_some_and(has_review_news), |d| d.child(meta().child(review_text(&detail.map_or(mr, |x| &x.mr).review))))
        .when(m.conflicts == Some(true), |d| d.child(div().text_size(px(11.)).text_color(theme::red()).child("Has conflicts")))
        .when(detail.is_none(), |d| d.child(meta().child(wait)))
        .children(extra)
        .when_some(hint, |d, h| d.child(div().mt(px(2.)).text_size(px(11.)).text_color(theme::text_disabled()).child(h.to_string())))
}

/// The hover card, with the MR/PR's detail loaded while it shows. Read inside a card's build: the view may be mid-update, so what
/// it needs was mirrored into `CardData` before the frame.
pub fn mr_card_live(tab: u32, kind: ForgeKind, mr: &ForgeMr, hint: Option<&str>, this: &WeakEntity<GitVolt>, cx: &mut App) -> AnyElement {
    let (detail, error) = crate::forge::card::card_data(tab, mr.number, this, cx);
    let stack = crate::forge::stack::stack_line(tab, mr, this, cx);
    mr_card(kind, mr, detail.as_ref(), error.as_deref(), hint, stack).into_any_element()
}

/// What a card reads of the tab, from what the frame copied: a card's built while the view's being updated. It asks for the detail
/// once the card's up, after the update.
pub fn card_data(tab: u32, number: u64, this: &WeakEntity<GitVolt>, cx: &mut App) -> (Option<ForgeMrDetail>, Option<String>) {
    let (detail, error, fresh) = CHIPS.with(|c| {
        let c = c.borrow();
        (c.details.get(&number).cloned(), c.errors.get(&number).cloned(), c.fresh_at.get(&number).is_some_and(|at| super::poll::now_ms() - at < super::poll::DETAIL_MAX_AGE_MS))
    });
    if !fresh {
        let w = this.clone();
        cx.defer(move |cx| {
            w.update(cx, |v, cx| v.forge_load_detail(tab, number, cx)).ok();
        });
    }
    (detail, error)
}

/// The sidebar hover card's MR/PR block.
pub fn branch_mr_block(tab: u32, upstream: Option<&str>, this: &WeakEntity<GitVolt>, cx: &mut App) -> Option<AnyElement> {
    let (kind, mr) = CHIPS.with(|c| {
        let c = c.borrow();
        Some((c.kind?, c.by_ref.get(upstream?)?.clone()))
    })?;
    Some(div().mt(px(6.)).pt(px(6.)).border_t_1().border_color(theme::section_border()).child(mr_card_live(tab, kind, &mr, None, this, cx)).into_any_element())
}

/// The badge: a state-coloured icon. A click opens the MR/PR's view and does nothing else (the row isn't selected, a double-click
/// doesn't check out). With `card`, hovering shows its card.
pub fn mr_badge(tab: u32, kind: ForgeKind, mr: &ForgeMr, size: f32, card: bool, cx: &mut Context<GitVolt>) -> AnyElement {
    let number = mr.number;
    let key = format!("mr-badge:{tab}:{number}");
    let el = div()
        .id(ElementId::Name(key.clone().into()))
        .flex()
        .flex_none()
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.forge_open_mr(tab, number, cx);
        }))
        .child(state_icon(mr.state, size));
    let anchored = crate::ui::anchor(key.clone(), el);
    if card {
        let (mr, this) = (mr.clone(), cx.weak_entity());
        Anchor::new(anchored).card(key, move |_, cx| {
            let shell = div().flex_none().w(px(320.)).px(px(10.)).py(px(8.)).bg(theme::panel_bg1()).border_1().border_color(theme::section_border()).rounded(px(6.)).shadow(crate::ui::shadow(theme::shadow_popup(), 6., 20.)).text_color(theme::text_normal());
            shell.child(mr_card_live(tab, kind, &mr, Some("Click to open"), &this, cx)).into_any_element()
        }).placement(Placement::Below).into_any_element()
    } else {
        anchored.into_any_element()
    }
}

/// `@user`'s card (`userCard`): looked up once a session, per repository and remote. A failure isn't kept, so the next hover asks again.
type Looked = Option<Option<ForgeUser>>;
thread_local! {
    static USERS: std::cell::RefCell<std::collections::HashMap<String, Looked>> = std::cell::RefCell::default();
}
const MAX_USERS: usize = 500;

pub fn show_user_card(env: (u32, String), user: String, bounds: Bounds<Pixels>, api: std::sync::Arc<gitvolt_core::api::Api>, rt: tokio::runtime::Handle, cx: &mut App) {
    let (repo, remote) = env;
    let key = format!("{repo}\0{remote}\0{}", user.to_lowercase());
    let known = USERS.with_borrow(|u| u.contains_key(&key));
    if !known {
        USERS.with_borrow_mut(|u| {
            if u.len() >= MAX_USERS {
                u.clear();
            }
            u.insert(key.clone(), None);
        });
        let (k, name) = (key.clone(), user.clone());
        cx.spawn(async move |cx| {
            let answer = super::ask::<Vec<ForgeUser>>(api, rt, gitvolt_core::api::Request::ForgeSearchUsers { repo, remote, query: name.clone() }).await;
            USERS.with_borrow_mut(|u| match answer {
                Ok(list) => {
                    let found = list.into_iter().find(|f| f.username.eq_ignore_ascii_case(&name));
                    u.insert(k, Some(found));
                }
                Err(_) => {
                    u.remove(&k);
                }
            });
            cx.update(|cx| cx.refresh_windows());
        })
        .detach();
    }
    let (k, shown) = (key.clone(), user);
    let build: crate::ui::tooltip::Build = std::rc::Rc::new(move |_, _| {
        let found = USERS.with_borrow(|u| u.get(&k).cloned().flatten().flatten());
        let card = div().p(px(8.)).rounded(px(6.)).bg(theme::panel_bg1()).border_1().border_color(theme::section_border()).text_size(px(12.));
        match found {
            Some(u) => card
                .flex()
                .items_center()
                .gap(px(8.))
                .child(forge_avatar(&u, 24.))
                .child(div().flex().gap(px(4.)).child(div().font_weight(FontWeight::BOLD).text_color(theme::text_selected()).child(u.name.clone())).child(div().text_color(theme::text_normal()).child(format!("@{}", u.username))))
                .into_any_element(),
            None => card.text_color(theme::text_selected()).child(format!("@{shown}")).into_any_element(),
        }
    });
    crate::ui::tooltip::show_card_at(format!("user:{key}"), build, bounds, crate::ui::tooltip::Placement::Below, cx);
}
