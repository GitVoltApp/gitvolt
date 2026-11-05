//! The + Add search (`create/SearchPicker.tsx`, `pickerCache.ts`, PeopleCard's popover): a box and its matches, debounced; only the newest query's
//! answer shows. ↑ ↓ move, Enter or a press picks, Esc closes. A floating panel with no shade, its own entity: it's built where the app can't be read.

use crate::theme;
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::modal;
use gitvolt_core::api::{Api, Request};
use gitvolt_core::forge::types::{ForgeLabel, ForgeUser};
use gpui::prelude::*;
use gpui::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_millis(200);
pub const STALE: Duration = Duration::from_secs(5 * 60);
pub const POP_W: f32 = 280.;

#[derive(Debug, Clone, PartialEq)]
pub struct SearchOption {
    pub key: String,
    pub label: String,
    pub detail: Option<String>,
    pub color: Option<String>,
    pub user: Option<ForgeUser>,
}

thread_local! {
    static CACHE: RefCell<HashMap<String, (Instant, Vec<SearchOption>)>> = RefCell::new(HashMap::new());
}

/// When a forge account changes, the project may resolve differently.
pub fn clear_cache() {
    CACHE.with(|c| c.borrow_mut().clear());
}

fn peek(key: &str) -> Option<(Vec<SearchOption>, bool)> {
    CACHE.with(|c| c.borrow().get(key).map(|(at, v)| (v.clone(), at.elapsed() > STALE)))
}

/// What a search asks: people or labels of a remote's project.
#[derive(Clone)]
pub enum Source {
    People { repo: u32, remote: String },
    /// GitHub lists up to 3 pages of 100 labels, GitLab one page of 50: a list under that is whole, and filtered here.
    Labels { repo: u32, remote: String, limit: usize },
}

impl Source {
    fn key(&self, q: &str) -> String {
        match self {
            Source::People { repo, remote } => format!("{repo}\0{remote}\0people\0{q}"),
            Source::Labels { repo, remote, .. } => format!("{repo}\0{remote}\0labels\0{q}"),
        }
    }

    fn filter(items: Vec<SearchOption>, q: &str) -> Vec<SearchOption> {
        let n = q.to_lowercase();
        items.into_iter().filter(|o| n.is_empty() || o.label.to_lowercase().contains(&n)).collect()
    }

    fn peek(&self, q: &str) -> Option<(Vec<SearchOption>, bool)> {
        match self {
            Source::People { .. } => peek(&self.key(q)),
            Source::Labels { limit, .. } => {
                if let Some((all, stale)) = peek(&self.key(""))
                    && all.len() < *limit
                {
                    return Some((Self::filter(all, q), stale));
                }
                peek(&self.key(q))
            }
        }
    }

    async fn fetch(&self, api: Arc<Api>, rt: tokio::runtime::Handle, q: String) -> Result<Vec<SearchOption>, String> {
        match self {
            Source::People { repo, remote } => {
                let users = super::ask::<Vec<ForgeUser>>(api, rt, Request::ForgeSearchUsers { repo: *repo, remote: remote.clone(), query: q }).await?;
                Ok(users.into_iter().map(|u| SearchOption { key: u.id.to_string(), label: u.name.clone(), detail: Some(format!("@{}", u.username)), color: None, user: Some(u) }).collect())
            }
            Source::Labels { repo, remote, .. } => {
                let labels = super::ask::<Vec<ForgeLabel>>(api, rt, Request::ForgeLabels { repo: *repo, remote: remote.clone(), query: q }).await?;
                Ok(labels.into_iter().map(|l| SearchOption { key: l.name.clone(), label: l.name, detail: None, color: l.color, user: None }).collect())
            }
        }
    }

    /// The answer for `q`, cached by its query; labels ask for the whole list first.
    async fn load(&self, api: Arc<Api>, rt: tokio::runtime::Handle, q: String) -> Result<Vec<SearchOption>, String> {
        if let Source::Labels { limit, .. } = self {
            let all = self.fetch(api.clone(), rt.clone(), String::new()).await?;
            let whole = all.len() < *limit;
            let key = self.key("");
            CACHE.with(|c| c.borrow_mut().insert(key, (Instant::now(), all.clone())));
            if whole {
                return Ok(Self::filter(all, &q));
            }
            if q.is_empty() {
                return Ok(all);
            }
        }
        let found = self.fetch(api, rt, q.clone()).await?;
        let key = self.key(&q);
        CACHE.with(|c| c.borrow_mut().insert(key, (Instant::now(), found.clone())));
        Ok(found)
    }
}

pub type OnPick = Rc<dyn Fn(&SearchOption, &mut App)>;

pub struct SearchPop {
    input: Entity<TextInput>,
    source: Source,
    api: Arc<Api>,
    rt: tokio::runtime::Handle,
    options: Option<Vec<SearchOption>>,
    error: Option<String>,
    cursor: usize,
    seq: u64,
    /// What's chosen already: left out of the matches.
    taken: Rc<RefCell<Vec<String>>>,
    on_pick: OnPick,
    id: SharedString,
    label: SharedString,
}

impl SearchPop {
    fn go(&mut self, cx: &mut Context<Self>) {
        let q = self.input.read(cx).text().trim().to_string();
        self.seq += 1;
        let mine = self.seq;
        let hit = self.source.peek(&q);
        let fresh = hit.as_ref().is_some_and(|h| !h.1);
        match hit {
            Some((options, _)) => {
                self.options = Some(options);
                self.error = None;
                self.cursor = 0;
            }
            None => self.options = None,
        }
        cx.notify();
        if fresh || cfg!(test) {
            return;
        }
        let (source, api, rt) = (self.source.clone(), self.api.clone(), self.rt.clone());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            if this.read_with(cx, |p, _| p.seq != mine).unwrap_or(true) {
                return;
            }
            let answer = source.load(api, rt, q).await;
            this.update(cx, |p, cx| {
                if p.seq != mine {
                    return;
                }
                match answer {
                    Ok(o) => {
                        p.options = Some(o);
                        p.error = None;
                    }
                    Err(e) => {
                        p.options = Some(Vec::new());
                        p.error = Some(e);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn shown(&self) -> Vec<SearchOption> {
        let taken = self.taken.borrow();
        self.options.iter().flatten().filter(|o| !taken.contains(&o.key)).cloned().collect()
    }

    fn pick(&mut self, o: &SearchOption, cx: &mut Context<Self>) {
        (self.on_pick)(o, cx);
        self.taken.borrow_mut().push(o.key.clone());
        self.input.update(cx, |i, cx| i.set_text("", cx));
        self.go(cx);
    }
}

impl Render for SearchPop {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.shown();
        let mut list = div().id("pick-list").flex().flex_col().mt(px(4.)).max_h(px(220.)).overflow_y_scroll();
        for (i, o) in shown.iter().enumerate() {
            let on = i == self.cursor;
            let opt = o.clone();
            let mut row = div().id(("pick-opt", i)).flex().items_center().px(px(8.)).py(px(3.)).rounded(px(4.)).cursor_pointer().when(on, |d| d.bg(theme::selected_row()).text_color(theme::text_selected())).hover(|d| d.bg(theme::hover_row()));
            if let Some(c) = o.color.as_deref().and_then(colour_of) {
                row = row.child(div().flex_none().size(px(10.)).mr(px(6.)).rounded_full().bg(c));
            }
            row = row.child(div().min_w_0().truncate().child(o.label.clone()));
            if let Some(d) = &o.detail {
                row = row.child(div().flex_none().ml(px(6.)).text_color(theme::text_secondary()).child(d.clone()));
            }
            list = list.child(crate::ui::anchor(format!("pick-opt:{}", o.label), row.on_mouse_down(MouseButton::Left, cx.listener(move |p, _, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
                p.pick(&opt, cx);
            }))));
        }
        let note = match (&self.options, &self.error) {
            (_, Some(e)) => Some((e.clone(), theme::red())),
            (Some(_), None) if shown.is_empty() => Some(("No matches".to_string(), theme::text_secondary())),
            (None, None) => Some(("Searching…".to_string(), theme::text_secondary())),
            _ => None,
        };
        div()
            .id(self.id.clone())
            .w(px(POP_W - 14.))
            .flex()
            .flex_col()
            .child(div().flex().items_center().h(px(28.)).px(px(6.)).rounded(px(6.)).border_1().border_color(theme::section_border()).bg(theme::wip_input_bg()).child(self.input.clone()))
            .child(list)
            .when_some(note, |d, (t, c)| d.child(div().mt(px(4.)).px(px(8.)).py(px(3.)).text_color(c).child(t)))
            .into_any_element()
    }
}

/// `#rrggbb` as a colour.
pub fn colour_of(s: &str) -> Option<Hsla> {
    let hex = s.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some(gpui::rgb(n).into())
}

/// Opens the search under `anchor`: `compact`: its right edge at the button's. `taken`: what's chosen already.
#[allow(clippy::too_many_arguments)]
pub fn open(id: SharedString, label: SharedString, anchor: Bounds<Pixels>, compact: bool, source: Source, taken: Vec<String>, api: Arc<Api>, rt: tokio::runtime::Handle, on_pick: OnPick, window: &mut Window, cx: &mut App) {
    let input = cx.new(|cx| TextInput::new(cx).placeholder(label.clone()).claim_arrows().enter_submits());
    let focus = input.read(cx).focus.clone();
    let closing = id.clone();
    let pop = cx.new(|_| SearchPop { input: input.clone(), source, api, rt, options: None, error: None, cursor: 0, seq: 0, taken: Rc::new(RefCell::new(taken)), on_pick, id: id.clone(), label: label.clone() });
    let weak = pop.downgrade();
    cx.subscribe(&input, move |_, e: &InputEvent, cx| {
        let Some(pop) = weak.upgrade() else { return };
        match e {
            InputEvent::Changed => pop.update(cx, |p, cx| {
                p.cursor = 0;
                p.go(cx);
            }),
            InputEvent::Arrow { down, .. } => pop.update(cx, |p, cx| {
                let n = p.shown().len();
                p.cursor = if *down { (p.cursor + 1).min(n.saturating_sub(1)) } else { p.cursor.saturating_sub(1) };
                cx.notify();
            }),
            InputEvent::Submit { .. } => pop.update(cx, |p, cx| {
                let shown = p.shown();
                if let Some(o) = shown.get(p.cursor).cloned() {
                    p.pick(&o, cx);
                }
            }),
            _ => {}
        }
    })
    .detach();
    pop.update(cx, |p, cx| p.go(cx));
    let x = if compact { (anchor.right().as_f32() - POP_W).max(4.) } else { anchor.left().as_f32() };
    let bounds = Bounds::new(point(px(x), anchor.top()), size(px(POP_W), anchor.size.height));
    let shown = pop.clone();
    modal::open(modal::Modal::new(closing, move |_, _| shown.clone().into_any_element()).width(POP_W).anchored(Some(bounds)).focus(focus), window, cx);
}
