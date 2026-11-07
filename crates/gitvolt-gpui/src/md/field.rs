//! The Markdown field (`MarkdownField.tsx`, `useCompletion.tsx`, `field.css`): a textarea with Write and Preview tabs in
//! one editor box, completions at the caret, a "Markdown supported" footer. Preview renders the text, debounced, over the
//! textarea, which stays and keeps its size, text and caret. Ctrl+Shift+P switches while the field has the keyboard.

use super::boxes::Params;
use super::complete::*;
use super::view::{Content, MdOptions, MdView};
use crate::theme;
use crate::ui::input::{InputEvent, TextInput};
use gitvolt_core::api::Api;
use gpui::prelude::*;
use gpui::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(150);
const REMOTE_DEBOUNCE: Duration = Duration::from_millis(150);
const LINE: f32 = 12.5 * 1.55;

/// `@username` for a tab with a forge: the MR/PR's own people first, then what the forge's search finds. The forge views
/// give one; without it, only `:emoji:` completes.
#[derive(Clone)]
pub struct PeopleSource {
    pub participants: Vec<Person>,
    pub search: Option<PeopleSearch>,
    pub avatar: Option<PersonAvatar>,
}

pub type PeopleSearch = Rc<dyn Fn(String, &mut App) -> Task<Vec<Person>>>;
pub type PersonAvatar = Rc<dyn Fn(&Person, f32) -> AnyElement>;

pub struct FieldOptions {
    pub md: MdOptions,
    /// Its accessible name, and what its preview says it is.
    pub label: SharedString,
    pub placeholder: SharedString,
    pub rows: u32,
    /// Preview grows to its content, up to 60% of the window, in the textarea's place: a review comment's box.
    pub grow_preview: bool,
    pub people: Option<PeopleSource>,
}

#[derive(Debug, Clone)]
pub enum FieldEvent {
    Changed,
    /// Ctrl+Enter, from the textarea or the preview.
    Submit,
    Blurred,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Write,
    Preview,
}

#[derive(Clone)]
enum Hit {
    Emoji(EmojiHit),
    Person(Person),
}

impl Hit {
    fn key(&self) -> String {
        match self {
            Hit::Emoji(e) => e.name.clone(),
            Hit::Person(p) => p.id.to_string(),
        }
    }

    fn token(&self) -> String {
        match self {
            Hit::Emoji(e) => format!(":{}: ", e.name),
            Hit::Person(p) => format!("@{} ", p.username),
        }
    }
}

pub type Toolbar = Rc<dyn Fn(&mut App) -> AnyElement>;

pub struct MarkdownField {
    toolbar: Option<Toolbar>,
    pub input: Entity<TextInput>,
    preview: Entity<MdView>,
    preview_focus: FocusHandle,
    pub mode: Mode,
    shown: String,
    label: SharedString,
    rows: u32,
    grow: bool,
    floor: f32,
    disabled: bool,
    people: Option<PeopleSource>,
    active: usize,
    closed_at: Option<usize>,
    /// The search's answer, for the trigger it was asked for.
    more: (String, Vec<Person>),
    asked: String,
    last_tag: String,
    debounce: u64,
    open_items: Rc<RefCell<(Vec<Hit>, Option<Trigger>)>>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<FieldEvent> for MarkdownField {}

impl MarkdownField {
    pub fn new(mut opts: FieldOptions, api: Arc<Api>, rt: tokio::runtime::Handle, retired: Rc<RefCell<Vec<Arc<RenderImage>>>>, cx: &mut Context<Self>) -> Self {
        let placeholder = opts.placeholder.clone();
        let input = cx.new(|cx| TextInput::multiline(cx).placeholder(placeholder));
        opts.md.pane = true;
        opts.md.inset = Some((12., 10.));
        if opts.md.params == Params::default() {
            opts.md.params.font_px = 12.;
        }
        let md = opts.md;
        let preview = cx.new(|cx| MdView::new(md, api, rt, retired, cx));
        let subs = vec![cx.subscribe(&input, |this, _, e: &InputEvent, cx| this.on_input(e, cx))];
        Self { toolbar: None, input, preview, preview_focus: cx.focus_handle(), mode: Mode::Write, shown: String::new(), label: opts.label, rows: opts.rows.max(1), grow: opts.grow_preview, floor: 0., disabled: false, people: opts.people, active: 0, closed_at: None, more: (String::new(), Vec::new()), asked: String::new(), last_tag: String::new(), debounce: 0, open_items: Rc::default(), _subs: subs }
    }

    pub fn text(&self, cx: &App) -> String {
        self.input.read(cx).text().to_string()
    }

    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.set_text(text, cx));
        cx.notify();
    }

    /// What sits at the end of the tabs' row: a template's picker.
    pub fn set_toolbar(&mut self, build: Toolbar) {
        self.toolbar = Some(build);
    }

    pub fn set_disabled(&mut self, on: bool, cx: &mut Context<Self>) {
        self.disabled = on;
        self.input.update(cx, |i, cx| i.set_disabled(on, cx));
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        let h = self.input.read(cx).focus.clone();
        window.focus(&h, cx);
    }

    fn on_input(&mut self, e: &InputEvent, cx: &mut Context<Self>) {
        match e {
            InputEvent::Changed => {
                cx.emit(FieldEvent::Changed);
                cx.notify();
            }
            InputEvent::Blurred => cx.emit(FieldEvent::Blurred),
            InputEvent::Submit { ctrl: true, .. } => cx.emit(FieldEvent::Submit),
            InputEvent::Submit { .. } => self.insert_current(cx),
            InputEvent::Arrow { down, .. } => {
                let n = self.open_items.borrow().0.len();
                if n > 0 {
                    self.active = if *down { (self.active + 1) % n } else { (self.active + n - 1) % n };
                    cx.notify();
                }
            }
            InputEvent::Escape => {
                self.closed_at = self.open_items.borrow().1.as_ref().map(|t| t.start);
                cx.notify();
            }
            InputEvent::Focused => cx.notify(),
        }
    }

    /// `true`: the popup's open, with these.
    fn insert_current(&mut self, cx: &mut Context<Self>) {
        let hit = {
            let items = self.open_items.borrow();
            items.0.get(self.active.min(items.0.len().saturating_sub(1))).cloned()
        };
        if let Some(hit) = hit {
            self.insert(hit, cx);
        }
    }

    fn insert(&mut self, hit: Hit, cx: &mut Context<Self>) {
        let trigger = self.open_items.borrow().1.clone();
        let Some(trigger) = trigger else { return };
        let token = hit.token();
        self.input.update(cx, |i, cx| {
            let caret = i.buffer.caret();
            let (text, at) = insert_token(i.text(), caret, &trigger, &token);
            i.set_text_caret(&text, at, cx);
        });
        cx.notify();
    }

    /// Switches: Preview keeps the textarea's text and caret; its text goes in at once, later typing after a pause.
    pub fn show(&mut self, next: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if next == self.mode {
            return;
        }
        if next == Mode::Preview {
            self.shown = self.text(cx);
            self.floor = self.input_height();
            let text: Arc<str> = self.shown.as_str().into();
            self.preview.update(cx, |p, cx| p.set_content(Content::Text(text), false, cx));
        }
        self.mode = next;
        match next {
            Mode::Preview => window.focus(&self.preview_focus, cx),
            Mode::Write => {
                let h = self.input.read(cx).focus.clone();
                window.focus(&h, cx);
            }
        }
        cx.notify();
    }

    fn input_height(&self) -> f32 {
        self.rows as f32 * LINE + 20.
    }

    /// What it completes, at the caret: `(hits, trigger)`, the first provider whose trigger is there.
    fn candidates(&mut self, cx: &mut Context<Self>) -> (Vec<Hit>, Option<Trigger>) {
        let (text, caret) = {
            let i = self.input.read(cx);
            (i.text().to_string(), i.buffer.caret())
        };
        if let Some(t) = find_trigger(&text, caret) {
            let hits = rank_emoji(&t.query, MAX_HITS).into_iter().map(Hit::Emoji).collect();
            return (hits, Some(t));
        }
        let Some(people) = self.people.clone() else { return (Vec::new(), None) };
        let Some(t) = find_mention(&text, caret) else { return (Vec::new(), None) };
        let tag = t.query.clone();
        if self.asked != tag {
            self.asked = tag.clone();
            if let Some(search) = people.search.clone() {
                self.debounce += 1;
                let (job, query) = (self.debounce, tag.clone());
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(REMOTE_DEBOUNCE).await;
                    let Ok(Some(found)) = this.update(cx, |this, cx| (this.debounce == job).then(|| search(query.clone(), cx))) else { return };
                    let found = found.await;
                    this.update(cx, |this, cx| {
                        if this.debounce == job {
                            this.more = (query, found);
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
        }
        let mut local = rank_people(&people.participants, &t.query);
        if self.more.0 == tag {
            local = merge_people(&local, &rank_people(&self.more.1, &t.query), MAX_HITS);
        }
        local.truncate(MAX_HITS);
        (local.into_iter().map(Hit::Person).collect(), Some(t))
    }

    fn tab(&self, id: &'static str, label: &'static str, mode: Mode, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let on = self.mode == mode;
        div()
            .id(id)
            .mb(px(-1.))
            .px(px(10.))
            .pt(px(4.))
            .pb(px(6.))
            .rounded_t(px(5.))
            .border_t_1()
            .border_l_1()
            .border_r_1()
            .text_size(px(12.))
            .cursor_pointer()
            .when(on, |d| d.text_color(theme::text_selected()).bg(theme::wip_input_bg()).border_color(theme::wip_input_border()))
            .when(!on, |d| d.text_color(theme::text_secondary()).border_color(transparent_black()).hover(|d| d.text_color(theme::text_normal())))
            .on_click(cx.listener(move |this, _, window, cx| this.show(mode, window, cx)))
            .child(label)
    }

    fn popup(&mut self, hits: &[Hit], trigger: &Trigger, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (at, line) = self.input.read(cx).point_of(trigger.start)?;
        let current = self.active.min(hits.len().saturating_sub(1));
        let avatar = self.people.as_ref().and_then(|p| p.avatar.clone());
        let rows: Vec<AnyElement> = hits
            .iter()
            .enumerate()
            .map(|(i, hit)| {
                let on = i == current;
                let insert = hit.clone();
                div()
                    .id(SharedString::from(format!("md-opt-{}", hit.key())))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_color(if on { theme::text_selected() } else { theme::text_normal() })
                    .when(on, |d| d.bg(theme::selected_row()))
                    .on_mouse_move(cx.listener(move |this, _, _, cx| {
                        if this.active != i {
                            this.active = i;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| this.insert(insert.clone(), cx)))
                    .map(|d| match hit {
                        Hit::Emoji(e) => d.child(div().flex_none().w(px(19.6)).flex().justify_center().text_size(px(14.)).child(e.emoji.clone())).child(format!(":{}:", e.name)),
                        Hit::Person(p) => d
                            .when_some(avatar.as_ref(), |d, a| d.child(a(p, 18.)))
                            .child(p.name.clone())
                            .child(div().when(!on, |d| d.text_color(theme::text_secondary())).when(on, |d| d.opacity(0.8)).child(format!("@{}", p.username))),
                    })
                    .into_any_element()
            })
            .collect();
        let list = div()
            .id("md-emoji-pop")
            .occlude()
            .min_w(px(180.))
            .max_h(px(260.))
            .overflow_y_scroll()
            .p(px(4.))
            .rounded(px(6.))
            .border_1()
            .border_color(theme::wip_input_border())
            .bg(theme::panel_bg1())
            .shadow(crate::ui::shadow(theme::shadow_popup(), 6., 18.))
            .text_size(px(12.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(rows);
        Some(deferred(anchored().position(point(at.x, at.y + line + px(2.))).snap_to_window().child(list)).with_priority(1).into_any_element())
    }
}

/// The Markdown mark: an M and a down arrow in a box, in the text's colour.
fn markdown_mark(color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let (x, y) = (b.origin.x.as_f32(), b.origin.y.as_f32());
            window.paint_quad(PaintQuad { bounds: Bounds::new(point(px(x), px(y)), size(px(14.), px(10.))), corner_radii: Corners::all(px(1.)), background: transparent_black().into(), border_widths: Edges::all(px(1.)), border_color: color, border_style: BorderStyle::Solid });
            let bar = |window: &mut Window, bx: f32, by: f32, w: f32, h: f32| window.paint_quad(fill(Bounds::new(point(px(x + bx), px(y + by)), size(px(w), px(h))), color));
            bar(window, 2., 2.5, 1., 5.);
            bar(window, 6., 2.5, 1., 5.);
            bar(window, 3., 2.5, 1., 1.5);
            bar(window, 5., 2.5, 1., 1.5);
            bar(window, 4., 3.5, 1., 1.5);
            bar(window, 10., 2.5, 1., 3.);
            let mut arrow = Path::new(point(px(x + 8.5), px(y + 5.)));
            arrow.line_to(point(px(x + 12.5), px(y + 5.)));
            arrow.line_to(point(px(x + 10.5), px(y + 7.5)));
            window.paint_path(arrow, color);
        },
    )
    .flex_none()
    .w(px(14.))
    .h(px(10.))
}

impl Render for MarkdownField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Later typing shows after a pause.
        if self.mode == Mode::Preview && self.text(cx) != self.shown {
            let text = self.text(cx);
            self.debounce += 1;
            let job = self.debounce;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(PREVIEW_DEBOUNCE).await;
                this.update(cx, |this, cx| {
                    if this.debounce == job {
                        this.shown = text.clone();
                        let t: Arc<str> = text.as_str().into();
                        this.preview.update(cx, |p, cx| p.set_content(Content::Text(t), true, cx));
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
            self.shown = self.text(cx);
        }
        let (hits, trigger) = if self.mode == Mode::Write { self.candidates(cx) } else { (Vec::new(), None) };
        // A new query's list starts at its top; a trigger gone ends what closed it.
        let tag = trigger.as_ref().map_or(String::new(), |t| format!("{}:{}", t.start, t.query));
        if tag != self.last_tag {
            self.active = 0;
            self.last_tag = tag;
        }
        if trigger.is_none() {
            self.closed_at = None;
        }
        let open = trigger.as_ref().is_some_and(|t| !hits.is_empty() && self.closed_at != Some(t.start));
        self.input.update(cx, |i, _| i.set_popup_keys(open));
        *self.open_items.borrow_mut() = if open { (hits.clone(), trigger.clone()) } else { (Vec::new(), None) };
        let popup = if open { trigger.as_ref().and_then(|t| self.popup(&hits, t, cx)) } else { None };
        let focused = self.input.read(cx).is_focused(window) || self.preview_focus.is_focused(window);
        let preview_on = self.mode == Mode::Preview;
        let empty = self.shown.trim().is_empty();
        let height = self.input_height();
        let vh = window.viewport_size().height.as_f32();
        let grown = (self.floor.max(self.preview.read(cx).state.borrow().total_height())).min(vh * 0.6);
        let toggle_keys = |this: &mut Self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>| {
            let k = &e.keystroke;
            let m = k.modifiers;
            if (m.control || m.platform) && m.shift && !m.alt && k.key.eq_ignore_ascii_case("p") {
                let next = if this.mode == Mode::Write { Mode::Preview } else { Mode::Write };
                this.show(next, window, cx);
                cx.stop_propagation();
            } else if (m.control || m.platform) && !m.shift && !m.alt && k.key == "enter" && this.mode == Mode::Preview {
                cx.emit(FieldEvent::Submit);
                cx.stop_propagation();
            } else if k.key == "tab" && !m.control && !m.alt && !this.open_items.borrow().0.is_empty() {
                this.insert_current(cx);
                cx.stop_propagation();
            }
        };
        let tool = self.toolbar.clone().map(|b| b(cx));
        let foot = format!("Markdown supported · {} to preview", crate::commands::display_chord("Mod+Shift+P"));
        div()
            .id("md-field")
            .key_context("MarkdownField")
            .flex()
            .flex_col()
            .min_w_0()
            .rounded(px(6.))
            .border_1()
            .overflow_hidden()
            .bg(theme::wip_input_bg())
            .border_color(if focused { theme::blue() } else { theme::wip_input_border() })
            .when(focused, |d| d.shadow(vec![BoxShadow { color: theme::focus_outline(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(2.), inset: false }]))
            .on_key_down(cx.listener(toggle_keys))
            .capture_action(cx.listener(|this, _: &crate::ui::modal::FocusNext, _, cx| {
                if !this.open_items.borrow().0.is_empty() {
                    this.insert_current(cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_end()
                    .gap(px(2.))
                    .min_h(px(30.))
                    .pt(px(4.))
                    .px(px(6.))
                    .bg(theme::panel_bg1())
                    .border_b_1()
                    .border_color(theme::wip_input_border())
                    .justify_between()
                    .items_center()
                    .child(div().flex().gap(px(2.)).child(self.tab("md-write", "Write", Mode::Write, cx)).child(self.tab("md-preview", "Preview", Mode::Preview, cx)))
                    .children(tool),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .h(px(if preview_on && self.grow { grown.max(height) } else { height }))
                    // Kept in place under Preview: its size, text and caret stay.
                    .child(div().size_full().px(px(12.)).py(px(10.)).font_family("Noto Sans Mono").text_size(px(12.5)).line_height(px(LINE)).when(!(preview_on && self.grow), |d| d.child(self.input.clone())))
                    .when(preview_on, |d| {
                        d.child(
                            div()
                                .id("md-field-preview")
                                .track_focus(&self.preview_focus)
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .flex()
                                .bg(theme::wip_input_bg())
                                .when(empty, |d| d.px(px(12.)).py(px(10.)).text_size(px(12.)).text_color(theme::text_secondary()).child("Nothing to preview"))
                                .when(!empty, |d| d.child(self.preview.clone())),
                        )
                    })
                    .children(popup),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .px(px(10.))
                    .py(px(4.))
                    .border_t_1()
                    .border_color(theme::wip_input_border())
                    .text_size(px(11.))
                    .text_color(theme::text_disabled())
                    .child(markdown_mark(theme::text_disabled()))
                    .child(foot),
            )
    }
}
