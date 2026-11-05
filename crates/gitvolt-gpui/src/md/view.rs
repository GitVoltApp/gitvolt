//! The element: `MdView` holds a body's boxes, lays out what's near the view, paints it glyph by glyph and takes the
//! mouse and the keyboard. It's an entity, as `TextInput` is: its owner makes one, shows it as a child and hears what
//! leaves it through `MdEvent`. In a pane (`MdOptions::pane`) it scrolls itself; otherwise it's as tall as its content.

#![allow(clippy::too_many_arguments)]

use super::ast::{self, Flavor, Mark};
use super::boxes::{self, *};
// What's in both this crate's and GPUI's preludes, by name.
use super::boxes::TextStyle;
use super::links::ImageSource;
use gpui::Edges;
use super::layout::*;
use super::links::*;
use super::safe;
use crate::diff::textmate::{FONT_BOLD, FONT_ITALIC, Span, Tokenizer};
use crate::theme;
use crate::ui::menu::{self, MenuRow};
use crate::ui::tooltip;
use gitvolt_core::api::{Api, Request};
use gpui::prelude::*;
use gpui::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// `DEFAULT_MAX_BYTES`: bodies over this render as written.
pub const DEFAULT_MAX_BYTES: usize = 5_000_000;
/// `MR_BODY_MAX_BYTES`.
pub const MR_BODY_MAX_BYTES: usize = 1_000_000;
/// `SYNC_PARSE_CHARS`: up to here it's rendered in the first paint.
pub const SYNC_PARSE_CHARS: usize = 16_384;
const COPY_SIZE: f32 = 20.;
const MEASURE_SLICE_MS: u128 = 4;
const BAR: f32 = 10.;
/// The pane's padding, and the fifth of a pixel Chromium's toolbar leaves over it: a baseline rounds as it does there.
const PANE_PAD_TOP: f32 = 16.19;

/// What leaves the element: what only its owner can do. Links out and copies it does itself.
#[derive(Debug, Clone)]
pub enum MdEvent {
    OpenFile { path: String, commit: String, anchor: Option<String> },
    ShowCommit { sha: String },
    OpenMr { number: u64, web_url: String },
    /// The body's over what renders, or the parse gave up: its owner shows Source.
    TooLarge,
    /// Ctrl+wheel over it: the reading size, a step.
    Zoom(i32),
}

#[derive(Clone)]
pub struct MdOptions {
    pub ctx: MdContext,
    /// A rendered diff's old side: what removed parts resolve against.
    pub old_ctx: Option<MdContext>,
    pub env: LinkEnv,
    pub flavor: Flavor,
    pub frontmatter: bool,
    pub params: Params,
    /// It scrolls itself, in a reading column.
    pub pane: bool,
    pub base: Option<super::SuggestionBase>,
    /// Where `worktree` is, for a relative image.
    pub worktree: String,
    pub reading_font: Option<String>,
    pub max_bytes: usize,
    /// `--md-measure`: in characters.
    pub measure_ch: Option<f32>,
    /// A rendered diff's pane: an overview ruler stands in for its scrollbar, and the text keeps clear of it.
    pub ruled: bool,
    /// Side by side: two reading columns and the gap between them.
    pub split: bool,
    /// A pane with this padding, sides and top, and its whole width: the field's preview.
    pub inset: Option<(f32, f32)>,
}

impl MdOptions {
    pub fn for_comment(repo: u32, env: LinkEnv, flavor: Flavor, params: Params) -> Self {
        Self { ctx: MdContext::Forge { repo }, old_ctx: None, env, flavor, frontmatter: false, params, pane: false, base: None, worktree: String::new(), reading_font: None, max_bytes: MR_BODY_MAX_BYTES, measure_ch: None, ruled: false, split: false, inset: None }
    }
}

/// What it shows.
#[derive(Clone)]
pub enum Content {
    Text(Arc<str>),
    /// A rendered diff.
    Diff { old: Arc<str>, new: Arc<str>, split: bool },
}

enum SlotState {
    Loading,
    Shown { image: Arc<RenderImage>, w: f32, h: f32, thumb: Option<Arc<RenderImage>> },
    Ask { host: String },
    Broken { why: String },
    /// A video, fetched and unplayable here: what can be done with it.
    Video { why: String, url: Option<String> },
}

struct Slot {
    src: ImageSrc,
    source: ImageSource,
    state: SlotState,
    asked: bool,
    top: usize,
}

struct Top {
    laid: Option<Rc<Laid>>,
    h: f32,
    mt: f32,
    mb: f32,
    measured: bool,
    /// Its changes: top inside it, height, kind.
    marks: Vec<(f32, f32, Mark)>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Focus {
    Link(u32),
    Image(u32),
    Summary(u32),
    Copy(u32),
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    Text,
    Pane { grab: f32 },
    Bar { id: u32, start_x: f32, start: f32, ratio: f32 },
}

/// A top-level block's place in the stream, for what asks by offset.
struct Doc {
    built: Built,
    targets: Vec<LinkTarget>,
    slots: Vec<Slot>,
    tops: Vec<Top>,
    offsets: Vec<f32>,
    width: f32,
    /// By code block: each line's colours.
    colours: Vec<Option<Arc<Vec<Vec<Span>>>>>,
    measured: usize,
    focusables: Vec<(Focus, usize)>,
}

pub struct MdState {
    pub opts: MdOptions,
    diagrams: Arc<super::mermaid::Diagrams>,
    /// Each diagram's picture or error, by source and dark or not, so a rebuild doesn't draw them again.
    drawn: HashMap<(String, bool), Result<super::mermaid::Drawn, String>>,
    mermaid_waits: bool,
    api: Arc<Api>,
    rt: tokio::runtime::Handle,
    retired: Rc<RefCell<Vec<Arc<RenderImage>>>>,
    content: Option<Content>,
    dom: Option<Arc<Vec<safe::Dom>>>,
    doc: Option<Doc>,
    too_large: bool,
    generation: u64,
    stop: Arc<AtomicBool>,
    theme_version: u64,
    pub scroll: f32,
    view_h: f32,
    bounds: Bounds<Pixels>,
    selection: Option<(usize, usize)>,
    drag: Drag,
    hover_link: Option<u32>,
    hover_code: Option<u32>,
    hover_copy: Option<u32>,
    hover_load: Option<u32>,
    hscroll: HashMap<u32, f32>,
    focused: Option<Focus>,
    cursor: CursorStyle,
    /// A click on an anchor, or a heading asked for: where to scroll once it's laid out.
    pending_anchor: Option<String>,
    /// Keeps a rendered diff's first change at the top once it's opened.
    pub reveal_first_change: bool,
    bar_hover: bool,
    /// The change marks, for the ruler: top, height, kind.
    pub marks: Rc<Vec<(f32, f32, Mark)>>,
    consented: Vec<String>,
    pending_load: Vec<u32>,
    press_hit: Option<Hit>,
    press_at: Point<Pixels>,
    last_step: Option<(usize, f32)>,
    pub ruler_drag: bool,
}

pub struct MdView {
    pub state: Rc<RefCell<MdState>>,
    pub focus: FocusHandle,
}

impl EventEmitter<MdEvent> for MdView {}

fn utf8_over(text: &str, max: usize) -> bool {
    text.len() > max
}

fn sized_lh(style: &TextStyle) -> f32 {
    match style.line {
        LineH::Num(k) => lu(style.size * k),
        _ => style.size * 1.32,
    }
}

/// About how tall a block will be: a character is half an em, lines are as wide as `w`. Close enough to scroll by,
/// until it's reached and laid out.
fn estimate(node: &BoxNode, w: f32, stream: &str) -> f32 {
    let inner = (w - node.margin.left - node.margin.right - node.padding.left - node.padding.right - node.border.left - node.border.right).max(40.);
    let body = match &node.kind {
        BoxKind::Inline(ib) => {
            let chars = stream.get(ib.range.clone()).map_or(0, str::len) as f32;
            let lines = (chars * ib.strut.size * 0.5 / inner).ceil().max(1.);
            lines * sized_lh(&ib.strut)
        }
        BoxKind::Code(c) => c.lines.len() as f32 * sized_lh(&c.style),
        BoxKind::Flow(kids) => kids.iter().map(|k| estimate(k, inner, stream) + k.margin.top.max(k.margin.bottom)).sum(),
        BoxKind::Details { summary, body, open } => estimate(summary, inner, stream) + if *open { body.iter().map(|k| estimate(k, inner, stream)).sum() } else { 0. },
        BoxKind::Table(t) => t.rows.iter().map(|r| r.cells.iter().map(|c| estimate(&c.content, inner / r.cells.len().max(1) as f32, stream)).fold(0., f32::max) + 1.).sum::<f32>(),
        BoxKind::Columns { cols, .. } => cols.iter().map(|k| estimate(k, inner / 2., stream)).fold(0., f32::max),
        BoxKind::Rule => 2.,
        _ => 0.,
    };
    body + node.padding.top + node.padding.bottom + node.border.top + node.border.bottom
}

fn walk_focus(node: &BoxNode, out: &mut Vec<(Focus, usize)>) {
    match &node.kind {
        BoxKind::Inline(ib) => {
            let mut last = None;
            for it in &ib.items {
                match it {
                    Item::Text { range, style } | Item::Kbd { range, style } => {
                        if let Some(l) = style.link.filter(|l| last != Some(*l)) {
                            out.push((Focus::Link(l), range.start));
                            last = Some(l);
                        }
                    }
                    Item::Image { slot, style } => {
                        if let Some(l) = style.link {
                            if last != Some(l) {
                                out.push((Focus::Link(l), ib.range.start));
                                last = Some(l);
                            }
                        } else {
                            out.push((Focus::Image(*slot), ib.range.start));
                        }
                    }
                    _ => {}
                }
            }
        }
        BoxKind::Flow(kids) => kids.iter().for_each(|k| walk_focus(k, out)),
        BoxKind::Details { summary, body, open } => {
            out.push((Focus::Summary(node.id), node.start));
            walk_focus(summary, out);
            if *open {
                body.iter().for_each(|k| walk_focus(k, out));
            }
        }
        BoxKind::Table(t) => t.rows.iter().for_each(|r| r.cells.iter().for_each(|c| walk_focus(&c.content, out))),
        BoxKind::Columns { cols, .. } => cols.iter().for_each(|k| walk_focus(k, out)),
        BoxKind::Code(c) if c.copy_button => out.push((Focus::Copy(c.index), node.start)),
        _ => {}
    }
}

fn slots_of(node: &BoxNode, top: usize, out: &mut Vec<(u32, usize)>) {
    match &node.kind {
        BoxKind::Inline(ib) => {
            for it in &ib.items {
                if let Item::Image { slot, .. } = it {
                    out.push((*slot, top));
                }
            }
        }
        BoxKind::Flow(kids) => kids.iter().for_each(|k| slots_of(k, top, out)),
        BoxKind::Details { summary, body, .. } => {
            slots_of(summary, top, out);
            body.iter().for_each(|k| slots_of(k, top, out));
        }
        BoxKind::Table(t) => t.rows.iter().for_each(|r| r.cells.iter().for_each(|c| slots_of(&c.content, top, out))),
        BoxKind::Columns { cols, .. } => cols.iter().for_each(|k| slots_of(k, top, out)),
        _ => {}
    }
}

/// What a diagram's box holds now.
fn set_kids(node: &mut BoxNode, id: u32, new: Vec<BoxNode>) -> bool {
    fn go(node: &mut BoxNode, id: u32, new: &mut Option<Vec<BoxNode>>) -> bool {
        if node.id == id
            && let BoxKind::Flow(kids) = &mut node.kind
        {
            *kids = new.take().unwrap_or_default();
            return true;
        }
        match &mut node.kind {
            BoxKind::Flow(kids) => kids.iter_mut().any(|k| go(k, id, new)),
            BoxKind::Details { summary, body, .. } => go(summary, id, new) || body.iter_mut().any(|k| go(k, id, new)),
            BoxKind::Table(t) => t.rows.iter_mut().any(|r| r.cells.iter_mut().any(|c| go(&mut c.content, id, new))),
            BoxKind::Columns { cols, .. } => cols.iter_mut().any(|k| go(k, id, new)),
            _ => false,
        }
    }
    go(node, id, &mut Some(new))
}

fn toggle_in(node: &mut BoxNode, id: u32) -> bool {
    if node.id == id
        && let BoxKind::Details { open, .. } = &mut node.kind
    {
        *open = !*open;
        return true;
    }
    match &mut node.kind {
        BoxKind::Flow(kids) => kids.iter_mut().any(|k| toggle_in(k, id)),
        BoxKind::Details { summary, body, .. } => toggle_in(summary, id) || body.iter_mut().any(|k| toggle_in(k, id)),
        BoxKind::Table(t) => t.rows.iter_mut().any(|r| r.cells.iter_mut().any(|c| toggle_in(&mut c.content, id))),
        BoxKind::Columns { cols, .. } => cols.iter_mut().any(|k| toggle_in(k, id)),
        _ => false,
    }
}

fn contains_id(node: &BoxNode, id: u32) -> bool {
    if node.id == id {
        return true;
    }
    match &node.kind {
        BoxKind::Flow(kids) => kids.iter().any(|k| contains_id(k, id)),
        BoxKind::Details { summary, body, .. } => contains_id(summary, id) || body.iter().any(|k| contains_id(k, id)),
        BoxKind::Table(t) => t.rows.iter().any(|r| r.cells.iter().any(|c| contains_id(&c.content, id))),
        BoxKind::Columns { cols, .. } => cols.iter().any(|k| contains_id(k, id)),
        _ => false,
    }
}

/// What the window's text system answers.
struct WinShaper<'a> {
    sys: &'a WindowTextSystem,
    base: Font,
    reading: Option<&'a str>,
    code_font: Font,
    slots: &'a mut Vec<Slot>,
    colours: &'a [Option<Arc<Vec<Vec<Span>>>>],
    wanted: Vec<u32>,
    metrics: HashMap<(Family, u32, u16, bool), (Font, FontM)>,
    font_px: f32,
}

/// `--font-ui`'s end: the emoji fonts, in its order.
fn emoji_fallbacks(mut names: Vec<String>) -> Option<FontFallbacks> {
    names.push("Noto Sans".into());
    names.push("Segoe UI Emoji".into());
    names.push("Noto Color Emoji".into());
    Some(FontFallbacks::from_fonts(names))
}

impl WinShaper<'_> {
    fn font_of(&mut self, style: &TextStyle) -> (Font, FontM) {
        let key = (style.family, style.size.to_bits(), style.weight, style.italic);
        if let Some(hit) = self.metrics.get(&key) {
            return hit.clone();
        }
        let mut f = match style.family {
            Family::Reading => match self.reading {
                Some(name) => Font { family: name.to_string().into(), fallbacks: emoji_fallbacks(vec![self.base.family.to_string()]), ..self.base.clone() },
                None => Font { fallbacks: emoji_fallbacks(Vec::new()), ..self.base.clone() },
            },
            Family::Ui => self.base.clone(),
            Family::Code => self.code_font.clone(),
            Family::Mono => Font { family: "Noto Sans Mono".into(), fallbacks: emoji_fallbacks(vec!["monospace".into()]), features: FontFeatures::default(), weight: FontWeight::NORMAL, style: FontStyle::Normal },
        };
        f.weight = FontWeight(style.weight as f32);
        f.style = if style.italic { FontStyle::Italic } else { FontStyle::Normal };
        let id = self.sys.resolve_font(&f);
        let size = px(style.size);
        // Chromium rounds them.
        let m = FontM { ascent: self.sys.ascent(id, size).as_f32().round(), descent: self.sys.descent(id, size).as_f32().abs().round(), gap: 0., x_height: self.sys.x_height(id, size).as_f32() };
        self.metrics.insert(key, (f.clone(), m));
        (f, m)
    }

    fn flatten(layout: &LineLayout) -> Shaped {
        let mut glyphs = Vec::with_capacity(layout.runs.iter().map(|r| r.glyphs.len()).sum());
        for r in &layout.runs {
            for g in &r.glyphs {
                glyphs.push(Glyph { id: g.id, font: r.font_id, index: g.index as u32, x: g.position.x.as_f32(), emoji: g.is_emoji });
            }
        }
        // Advances in whole pixels, as Chromium's are with hinting on.
        let mut at = 0.;
        let mut out_w = 0.;
        let count = glyphs.len();
        for i in 0..count {
            let next = glyphs.get(i + 1).map_or(layout.width.as_f32(), |g| g.x);
            let adv = (next - glyphs[i].x).round();
            glyphs[i].x = at;
            at += adv;
            out_w = at;
        }
        Shaped { glyphs, width: out_w }
    }

    fn image_box(&mut self, slot: u32) -> ImageState {
        let Some(s) = self.slots.get_mut(slot as usize) else { return ImageState::Boxed { w: 18., h: 24. } };
        if !s.asked {
            s.asked = true;
            self.wanted.push(slot);
        }
        let (aw, ah) = (s.src.width, s.src.height);
        let boxed = |w: f32, h: f32| {
            let w = aw.unwrap_or(w);
            let h = match (aw, ah) {
                (Some(a), Some(b)) => (w * b / a).max(24.),
                _ => h,
            };
            ImageState::Boxed { w, h }
        };
        match &s.state {
            SlotState::Shown { w, h, .. } => {
                let width = aw.unwrap_or(*w);
                let height = if *w > 0. { width * *h / *w } else { *h };
                ImageState::Shown { w: width, h: height }
            }
            SlotState::Broken { why } if s.src.mermaid => {
                let text = format!("Couldn't draw the diagram: {why}");
                let w = self.small_width(&text);
                ImageState::Boxed { w, h: 17. }
            }
            SlotState::Loading if s.src.video => boxed(240., 135.),
            SlotState::Loading => boxed(18., 24.),
            SlotState::Ask { host } => {
                let what = if s.src.video { "video" } else { "image" };
                let alt = if s.src.alt.is_empty() { None } else { Some(s.src.alt.clone()) };
                let label = format!("Load {what} from {host}");
                let (w, h) = self.ask_size(alt.as_deref(), &label);
                boxed(w, h)
            }
            SlotState::Broken { .. } => {
                let alt = s.src.alt.clone();
                let w = if alt.is_empty() { 14. } else { 14. + 6. + self.small_width(&alt) };
                boxed(w + 18., 26_f32.max(24.))
            }
            SlotState::Video { .. } => boxed(240., 135.),
        }
    }

    fn small_style(&self) -> TextStyle {
        TextStyle { family: Family::Reading, size: 11., weight: 400, italic: false, color: Tok::TextSecondary, line: LineH::Num(1.55), valign: VAlign::Base, link: None, strike: false, underline: false, code: false, word: None, tint: None }
    }

    fn small_width(&mut self, text: &str) -> f32 {
        let st = self.small_style();
        self.shape(text, &st).width
    }

    fn ask_size(&mut self, alt: Option<&str>, label: &str) -> (f32, f32) {
        let lh = lu(11. * self.line_height());
        let button = self.small_width(label) + 18.;
        let w = alt.map_or(0., |a| self.small_width(a) + 6.) + button + 18.;
        (w, lh + 6. + 10.)
    }

    fn line_height(&self) -> f32 {
        1.55
    }
}

impl Shaper for WinShaper<'_> {
    fn shape(&mut self, text: &str, style: &TextStyle) -> Shaped {
        if text.is_empty() {
            return Shaped::default();
        }
        let (font, _) = self.font_of(style);
        let run = TextRun { len: text.len(), font, color: black(), background_color: None, underline: None, strikethrough: None };
        let layout = self.sys.layout_line(text, px(style.size), &[run], None);
        Self::flatten(&layout)
    }

    fn metrics(&mut self, style: &TextStyle) -> FontM {
        self.font_of(style).1
    }

    fn image(&mut self, slot: u32) -> ImageState {
        let _ = self.font_px;
        self.image_box(slot)
    }

    fn code_line(&mut self, block: u32, line: usize, text: &str, style: &TextStyle) -> Shaped {
        let spans = self.colours.get(block as usize).and_then(Option::as_ref).and_then(|c| c.get(line));
        let Some(spans) = spans.filter(|s| s.iter().any(|x| x.font & (FONT_BOLD | FONT_ITALIC) != 0)) else { return self.shape(text, style) };
        if text.is_empty() {
            return Shaped::default();
        }
        let (font, _) = self.font_of(style);
        let mut runs = Vec::with_capacity(spans.len() * 2 + 1);
        let mut at = 0usize;
        for s in spans {
            let (a, b) = ((s.start as usize).min(text.len()), (s.end as usize).min(text.len()));
            if a > at {
                runs.push(TextRun { len: a - at, font: font.clone(), color: black(), background_color: None, underline: None, strikethrough: None });
            }
            if b > a {
                let f = Font { weight: if s.font & FONT_BOLD != 0 { FontWeight::BOLD } else { font.weight }, style: if s.font & FONT_ITALIC != 0 { FontStyle::Italic } else { font.style }, ..font.clone() };
                runs.push(TextRun { len: b - a, font: f, color: black(), background_color: None, underline: None, strikethrough: None });
            }
            at = at.max(b);
        }
        if at < text.len() {
            runs.push(TextRun { len: text.len() - at, font, color: black(), background_color: None, underline: None, strikethrough: None });
        }
        let layout = self.sys.layout_line(text, px(style.size), &runs, None);
        Self::flatten(&layout)
    }
}

/// What parsing needs of the options: what can leave the UI thread.
#[derive(Clone)]
struct Prep {
    flavor: Flavor,
    frontmatter: bool,
    max_bytes: usize,
    params: Params,
    base: Option<super::SuggestionBase>,
}

/// Parse to boxes, off the UI thread when it's long.
fn prepare(content: &Content, opts: &Prep) -> Result<(Arc<Vec<safe::Dom>>, Built), bool> {
    let dom = match content {
        Content::Text(text) => {
            if utf8_over(text, opts.max_bytes) {
                return Err(true);
            }
            let parsed = ast::parse(text, opts.flavor, opts.frontmatter);
            if parsed.too_large {
                return Err(true);
            }
            safe::to_dom(&parsed.root)
        }
        Content::Diff { old, new, split } => {
            if utf8_over(old, opts.max_bytes) || utf8_over(new, opts.max_bytes) {
                return Err(true);
            }
            let result = super::diff::diff_markdown(old, new, opts.flavor);
            if result.gave_up {
                return Err(true);
            }
            let root = if *split { super::diff::split_tree(result.root) } else { result.root };
            safe::to_dom(&root)
        }
    };
    let built = boxes::build(&dom, opts.params, opts.base.clone());
    Ok((Arc::new(dom), built))
}

impl MdState {
    fn link_env_side(&self, old_side: bool) -> &MdContext {
        if old_side { self.opts.old_ctx.as_ref().unwrap_or(&self.opts.ctx) } else { &self.opts.ctx }
    }

    fn adopt(&mut self, built: Built) {
        let env = self.opts.env.clone();
        let targets: Vec<LinkTarget> = built
            .links
            .iter()
            .map(|l| match l {
                LinkSrc::Href { href, old_side, .. } => target_of_href(&env, self.link_env_side(*old_side), href),
                LinkSrc::Ref { node, .. } => target_of_reference(&env, node),
            })
            .collect();
        let mut tops_of = Vec::new();
        for (i, t) in built.tops.iter().enumerate() {
            slots_of(t, i, &mut tops_of);
        }
        let mut slots: Vec<Slot> = built
            .images
            .iter()
            .map(|src| {
                let source = resolve_image(&env, self.link_env_side(src.old_side), &src.src);
                Slot { src: src.clone(), source, state: SlotState::Loading, asked: false, top: 0 }
            })
            .collect();
        for (slot, top) in tops_of {
            if let Some(s) = slots.get_mut(slot as usize) {
                s.top = top;
            }
        }
        let mut focusables = Vec::new();
        for t in &built.tops {
            walk_focus(t, &mut focusables);
        }
        focusables.retain(|(f, _)| !matches!(f, Focus::Link(l) if targets.get(*l as usize) == Some(&LinkTarget::Inert)) && !matches!(f, Focus::Image(s) if slots.get(*s as usize).is_some_and(|s| s.src.mermaid)));
        let tops = built.tops.iter().map(|_| Top { laid: None, h: 0., mt: 0., mb: 0., measured: false, marks: Vec::new() }).collect();
        let colours = vec![None; built.codes.len()];
        self.doc = Some(Doc { built, targets, slots, tops, offsets: Vec::new(), width: -1., colours, measured: 0, focusables });
        self.selection = None;
        self.hover_link = None;
        self.focused = None;
        self.hscroll.clear();
    }

    fn content_h(&self) -> f32 {
        self.doc.as_ref().and_then(|d| d.offsets.last().copied()).unwrap_or(0.)
    }

    /// The column's left and its width, in a pane `w` wide.
    fn column(&self, w: f32) -> (f32, f32) {
        if !self.opts.pane {
            return (0., w);
        }
        if let Some((x, _)) = self.opts.inset {
            return (x, (w - 2. * x).max(0.));
        }
        let room = if self.opts.ruled { (w - 24. - 54.).max(0.) } else { (w - 48. - BAR).max(0.) };
        let ch = (self.opts.params.font_px * 0.5).round();
        let measure = match (self.opts.split, self.opts.measure_ch) {
            (true, Some(n)) => 2. * n * ch + 24.,
            (true, None) => 1784.,
            (false, Some(n)) => n * ch,
            (false, None) => 880.,
        };
        let cw = room.min(measure);
        (24. + (room - cw) / 2., cw)
    }

    fn pad_top(&self) -> f32 {
        match (self.opts.pane, self.opts.inset) {
            (true, Some((_, y))) => y,
            (true, None) => PANE_PAD_TOP,
            _ => 0.,
        }
    }

    pub fn max_scroll(&self) -> f32 {
        (self.content_h() + 2. * self.pad_top() - self.view_h).max(0.)
    }

    fn top_at(d: &Doc, y: f32) -> usize {
        d.offsets.partition_point(|o| *o <= y).saturating_sub(1).min(d.tops.len().saturating_sub(1))
    }

    /// Where each block lies: the margins between them collapse, as they do in a flow.
    fn reflow(d: &mut Doc) {
        let mut y = 0.;
        let mut pending: f32 = 0.;
        d.offsets.clear();
        let mut first = true;
        for t in &d.tops {
            // An empty block's margins collapse through it.
            if t.h == 0. {
                d.offsets.push(y + pending.max(t.mt));
                pending = pending.max(t.mt).max(t.mb);
                continue;
            }
            y += if first { t.mt.max(pending) } else { pending.max(t.mt) };
            first = false;
            d.offsets.push(y);
            y += t.h;
            pending = t.mb;
        }
        d.offsets.push(y + if d.tops.is_empty() { 0. } else { pending.max(0.) });
    }

    /// Lays out what's near the view, and, up to a slice of time, what isn't yet. `true`: there's more for the next frame.
    fn ensure(&mut self, bounds: Bounds<Pixels>, window: &mut Window) -> bool {
        self.bounds = bounds;
        let (_, cw) = self.column(bounds.size.width.as_f32());
        self.view_h = bounds.size.height.as_f32();
        let pane = self.opts.pane;
        let (pad_top, view_h) = (self.pad_top(), self.view_h);
        let Some(d) = self.doc.as_mut() else { return false };
        let stream = std::mem::take(&mut d.built.stream);
        if d.width != cw {
            d.width = cw;
            d.measured = 0;
            for (t, node) in d.tops.iter_mut().zip(&d.built.tops) {
                t.laid = None;
                t.measured = false;
                t.h = estimate(node, cw, &stream);
                t.mt = node.margin.top;
                t.mb = node.margin.bottom;
            }
            Self::reflow(d);
        }
        let code_font = {
            let mut f = crate::diff::view::mono();
            if !self.opts.params.editor_code {
                f.family = "Noto Sans Mono".into();
            }
            f
        };
        let base = window.text_style().font();
        let sys = window.text_system().clone();
        let colours = std::mem::take(&mut d.colours);
        let mut slots = std::mem::take(&mut d.slots);
        let mut shaper = WinShaper { sys: &sys, base, reading: self.opts.reading_font.as_deref(), code_font, slots: &mut slots, colours: &colours, wanted: Vec::new(), metrics: HashMap::new(), font_px: self.opts.params.font_px };
        let mut more = false;
        let measure_one = |i: usize, d: &mut Doc, s: &mut WinShaper| -> f32 {
            let laid = layout_box(&d.built.tops[i], cw, &stream, s);
            let t = &mut d.tops[i];
            let change = laid.h - t.h;
            if !t.measured {
                d.measured += 1;
            }
            t.h = laid.h;
            t.mt = laid.mt;
            t.mb = laid.mb;
            t.measured = true;
            t.marks.clear();
            MdState::collect_marks(&d.built.tops[i], &laid, 0., &mut t.marks);
            t.laid = Some(Rc::new(laid));
            change
        };
        if pane {
            // What's on screen and a screen each way, at their real heights; a block above that changed moves the rest,
            // and the view moves with it, so nothing on screen jumps.
            for _ in 0..4 {
                let (from, to) = {
                    let top = self.scroll - pad_top;
                    (Self::top_at(d, top - view_h), Self::top_at(d, top + 2. * view_h))
                };
                let mut changed = false;
                let anchor_block = Self::top_at(d, self.scroll - pad_top);
                for i in from..=to.min(d.tops.len().saturating_sub(1)) {
                    if d.tops.is_empty() || d.tops[i].laid.is_some() {
                        continue;
                    }
                    let change = measure_one(i, d, &mut shaper);
                    if change != 0. {
                        changed = true;
                        if i < anchor_block {
                            self.scroll += change;
                        }
                    }
                }
                if changed {
                    Self::reflow(d);
                } else {
                    break;
                }
            }
            // A slice of the rest, so the scrollbar comes true.
            if d.measured < d.tops.len() {
                let started = std::time::Instant::now();
                let anchor_block = Self::top_at(d, self.scroll - pad_top);
                let mut changed = false;
                for i in 0..d.tops.len() {
                    if d.tops[i].measured {
                        continue;
                    }
                    let change = measure_one(i, d, &mut shaper);
                    // Only its height is kept: what's far from the view is laid out again when it's reached.
                    d.tops[i].laid = None;
                    changed = true;
                    if i < anchor_block {
                        self.scroll += change;
                    }
                    if started.elapsed().as_millis() >= MEASURE_SLICE_MS {
                        break;
                    }
                }
                if changed {
                    Self::reflow(d);
                }
                more = d.measured < d.tops.len();
            }
            // What's far from the view lets its glyphs go.
            let (near_from, near_to) = (Self::top_at(d, self.scroll - pad_top - 3. * view_h), Self::top_at(d, self.scroll - pad_top + 4. * view_h));
            for (i, t) in d.tops.iter_mut().enumerate() {
                if (i < near_from || i > near_to) && t.laid.is_some() {
                    t.laid = None;
                }
            }
        } else {
            let mut changed = false;
            for i in 0..d.tops.len() {
                if d.tops[i].laid.is_none() {
                    measure_one(i, d, &mut shaper);
                    changed = true;
                }
            }
            if changed {
                Self::reflow(d);
            }
        }
        let wanted = std::mem::take(&mut shaper.wanted);
        drop(shaper);
        d.colours = colours;
        d.slots = slots;
        d.built.stream = stream;
        let mut marks = Vec::new();
        for (t, y) in d.tops.iter().zip(&d.offsets) {
            marks.extend(t.marks.iter().map(|(top, h, m)| (y + top, *h, *m)));
        }
        if marks != *self.marks {
            self.marks = Rc::new(marks);
        }
        self.pending_images(wanted);
        // A file's rendered diff opens at its first change, once it's where it will stay.
        if self.reveal_first_change
            && let Some(first) = self.marks.first().copied()
        {
            let (fits, pad) = (first.0 + first.1 + 16. + self.pad_top() <= self.view_h, self.pad_top());
            let settled = self.doc.as_ref().is_some_and(|d| d.measured == d.tops.len());
            self.scroll = if fits { 0. } else { Self::reveal_top(first.0 + pad, first.1, self.view_h) };
            if settled {
                self.reveal_first_change = false;
            }
        }
        if let Some(id) = self.pending_anchor.clone()
            && self.scroll_to_anchor(&id, bounds, window)
        {
            self.pending_anchor = None;
        }
        self.scroll = self.scroll.clamp(0., self.max_scroll());
        more
    }

    fn pending_images(&mut self, wanted: Vec<u32>) {
        self.pending_load.extend(wanted);
    }
}

/// A press's or a hover's find.
#[derive(Debug, Clone, Default)]
struct Hit {
    offset: Option<usize>,
    link: Option<u32>,
    slot: Option<u32>,
    load: bool,
    summary: Option<u32>,
    code: Option<u32>,
    copy: Option<u32>,
    bar: Option<(u32, f32, f32, f32)>,
    /// A block under it that scrolls sideways, and how far it can.
    hblock: Option<(u32, f32)>,
    text: bool,
    /// How far into an image's box.
    ask_x: Option<f32>,
}

fn frag_text_offset(item: &Item, sh: &Shaped, frag: &Frag, rel_x: f32) -> usize {
    let range = match item {
        Item::Text { range, .. } | Item::Kbd { range, .. } => range.clone(),
        _ => return 0,
    };
    let (from, to) = (frag.from as usize, frag.to as usize);
    let base = sh.glyphs.get(from).map_or(0., |g| g.x);
    for i in from..to {
        let g = &sh.glyphs[i];
        let x = g.x - base;
        let next = sh.glyphs.get(i + 1).map_or(sh.width, |n| n.x) - base;
        if rel_x < (x + next) / 2. {
            return range.start + g.index as usize;
        }
    }
    range.start + sh.glyphs.get(to).map_or(range.end - range.start, |g| g.index as usize)
}

impl MdState {
    fn hit_inline(ib: &InlineBlock, l: &LaidInline, x: f32, y: f32, hit: &mut Hit) {
        let Some(line) = l.lines.iter().find(|line| y < line.y + line.h).or(l.lines.last()) else {
            hit.offset = Some(ib.range.start);
            return;
        };
        let rel = x - line.x;
        let inside_y = y >= line.y && y < line.y + line.h;
        hit.offset = line.frags.first().map(|f| {
            let it = &ib.items[f.item as usize];
            frag_text_offset(it, &l.shaped[f.item as usize], f, 0.).max(ib.range.start)
        });
        for f in &line.frags {
            let it = &ib.items[f.item as usize];
            if rel >= f.x + f.w {
                if matches!(it, Item::Text { .. } | Item::Kbd { .. }) {
                    hit.offset = Some(frag_text_offset(it, &l.shaped[f.item as usize], f, f.w + 1.));
                }
                continue;
            }
            if rel < f.x {
                break;
            }
            match it {
                Item::Text { style, .. } => {
                    hit.offset = Some(frag_text_offset(it, &l.shaped[f.item as usize], f, rel - f.x));
                    if inside_y {
                        hit.link = style.link;
                        hit.text = true;
                    }
                }
                Item::Kbd { style, .. } => {
                    hit.offset = Some(frag_text_offset(it, &l.shaped[f.item as usize], f, rel - f.x - 6.));
                    if inside_y {
                        hit.link = style.link;
                        hit.text = true;
                    }
                }
                Item::Image { slot, style } => {
                    let top = line.baseline - f.above;
                    if y >= line.y + top && y < line.y + top + f.h {
                        hit.slot = Some(*slot);
                        hit.link = style.link;
                        hit.ask_x = Some(rel - f.x);
                    }
                }
                _ => {}
            }
            break;
        }
    }

    fn hit_node(&self, node: &BoxNode, laid: &Laid, x: f32, y: f32, hit: &mut Hit) {
        let (inner_x, inner_y) = (node.padding.left + node.border.left, node.padding.top + node.border.top);
        match (&node.kind, &laid.kind) {
            (BoxKind::Inline(ib), LaidKind::Inline(l)) => Self::hit_inline(ib, l, x - inner_x, y - inner_y, hit),
            (BoxKind::Flow(kids), LaidKind::Flow(p)) => self.hit_list(kids.iter(), p, x, y, hit),
            (BoxKind::Columns { cols, .. }, LaidKind::Flow(p)) => self.hit_list(cols.iter(), p, x, y, hit),
            (BoxKind::Details { summary, body, open }, LaidKind::Flow(p)) => {
                if let Some(first) = p.first()
                    && y >= first.y
                    && y < first.y + first.laid.h
                {
                    hit.summary = Some(node.id);
                }
                let list: Vec<&BoxNode> = if *open { std::iter::once(&**summary).chain(body.iter()).collect() } else { vec![&**summary] };
                self.hit_list(list.into_iter(), p, x, y, hit);
            }
            (BoxKind::Code(c), LaidKind::Code(l)) => {
                hit.code = Some(c.index);
                let view_w = laid.w;
                if c.copy_button && (view_w - 4. - COPY_SIZE..view_w - 4.).contains(&x) && (4. ..4. + COPY_SIZE).contains(&y) {
                    hit.copy = Some(c.index);
                }
                let scroll = self.hscroll.get(&node.id).copied().unwrap_or(0.);
                if l.overflow {
                    let (track, content) = (view_w, l.width + 20.);
                    hit.hblock = Some((node.id, content - track));
                    if y >= laid.h - BAR {
                        hit.bar = Some((node.id, scroll, track, content));
                    }
                }
                let row = ((y - inner_y) / l.line_h).floor().clamp(0., (l.lines.len().max(1) - 1) as f32) as usize;
                if let (Some(sh), Some(r)) = (l.lines.get(row), c.lines.get(row)) {
                    let sign_w = if c.signs { (c.style.size * 0.6).round() * 2. } else { 0. };
                    let rx = x - inner_x + scroll - sign_w;
                    let i = sh.glyphs.iter().position(|g| rx < g.x + (c.style.size * 0.3)).map_or(r.len(), |k| sh.glyphs[k].index as usize);
                    hit.offset = Some(r.start + i);
                    hit.text = true;
                }
            }
            (BoxKind::Table(t), LaidKind::Table(l)) => {
                let scroll = self.hscroll.get(&node.id).copied().unwrap_or(0.);
                if l.overflow {
                    hit.hblock = Some((node.id, l.width - l.view_w));
                    if y >= laid.h - BAR {
                        hit.bar = Some((node.id, scroll, l.view_w, l.width));
                    }
                }
                let (rx, ry) = (x - inner_x + scroll, y - inner_y);
                let Some((i, row)) = l.rows.iter().enumerate().find(|(_, r)| ry < r.y + r.h).or(l.rows.iter().enumerate().next_back()) else { return };
                let Some(rb) = t.rows.get(i) else { return };
                let at = row.boxes.iter().position(|(cx, w)| rx < cx + w).unwrap_or(row.boxes.len().saturating_sub(1));
                if let (Some(cell), Some(p)) = (rb.cells.get(at), row.cells.get(at)) {
                    self.hit_node(&cell.content, &p.laid, rx - p.x, ry - row.y - p.y, hit);
                }
            }
            _ => {}
        }
    }

    fn hit_list<'a>(&self, kids: impl Iterator<Item = &'a BoxNode>, placed: &[Placed], x: f32, y: f32, hit: &mut Hit) {
        let mut chosen: Option<(&BoxNode, &Placed)> = None;
        for (k, p) in kids.zip(placed) {
            if y >= p.y {
                if p.laid.h > 0. || chosen.is_none() {
                    chosen = Some((k, p));
                }
            } else {
                if chosen.is_none() {
                    chosen = Some((k, p));
                }
                break;
            }
        }
        if let Some((k, p)) = chosen {
            self.hit_node(k, &p.laid, x - p.x, y - p.y, hit);
        }
    }

    /// `pos`, in the window.
    fn hit(&self, pos: Point<Pixels>) -> Hit {
        let mut hit = Hit::default();
        let Some(d) = self.doc.as_ref() else { return hit };
        let (left, _) = self.column(self.bounds.size.width.as_f32());
        let x = (pos.x - self.bounds.origin.x).as_f32() - left;
        let y = (pos.y - self.bounds.origin.y).as_f32() + self.scroll - self.pad_top();
        if d.tops.is_empty() {
            return hit;
        }
        let i = Self::top_at(d, y);
        if let Some(laid) = d.tops[i].laid.as_ref() {
            self.hit_node(&d.built.tops[i], laid, x, y - d.offsets[i], &mut hit);
        }
        // An image waiting to be asked for: all of its box is the button's way in, past its alt text.
        if let Some(slot) = hit.slot
            && let Some(s) = d.slots.get(slot as usize)
            && matches!(s.state, SlotState::Ask { .. })
        {
            hit.load = true;
            hit.link = None;
        }
        hit
    }

    fn word_at(stream: &str, at: usize) -> (usize, usize) {
        let at = at.min(stream.len());
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let start = stream[..at].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(at, |(i, _)| i);
        let end = stream[at..].char_indices().find(|(_, c)| !is_word(*c)).map_or(stream.len(), |(i, _)| at + i);
        (start, end)
    }

    pub fn selected_text(&self) -> Option<String> {
        let (a, b) = self.selection?;
        let (a, b) = (a.min(b), a.max(b));
        let d = self.doc.as_ref()?;
        let text = d.built.stream.get(a..b)?.trim_end_matches('\n');
        (!text.is_empty()).then(|| text.to_string())
    }

    pub fn view_height(&self) -> f32 {
        self.view_h
    }

    pub fn change_marks(&self) -> Rc<Vec<(f32, f32, Mark)>> {
        self.marks.clone()
    }

    /// The pane's content height, what a ruler maps its marks against.
    pub fn total_height(&self) -> f32 {
        self.content_h() + 2. * self.pad_top()
    }

    pub fn focused_name(&self) -> Option<String> {
        self.focused.map(|f| format!("{f:?}"))
    }

    pub fn hscrolled(&self) -> f32 {
        self.hscroll.values().fold(0., |a, b| a.max(*b))
    }

    /// For a script: where each top-level block lies, in the document.
    pub fn top_offsets(&self) -> Vec<f32> {
        self.doc.as_ref().map_or(Vec::new(), |d| d.offsets.clone())
    }

    pub fn set_scroll(&mut self, y: f32) {
        self.scroll = y.clamp(0., self.max_scroll());
    }

    pub fn select_all(&mut self) {
        if let Some(d) = self.doc.as_ref() {
            self.selection = Some((0, d.built.stream.len()));
        }
    }

    pub fn scroll_by(&mut self, px: f32) {
        self.scroll = (self.scroll + px).clamp(0., self.max_scroll());
    }

    fn anchor_offset(&self, id: &str) -> Option<usize> {
        self.doc.as_ref()?.built.anchors.get(id).copied()
    }

    /// The y of `offset`, in the document: the top of its line when it's laid out.
    fn y_of_offset(d: &Doc, offset: usize) -> Option<(usize, f32)> {
        let i = d.built.tops.partition_point(|t| t.start <= offset).saturating_sub(1);
        let base = *d.offsets.get(i)?;
        let inner = d.tops.get(i)?.laid.as_ref().and_then(|l| Self::y_in(&d.built.tops[i], l, offset));
        Some((i, base + inner.unwrap_or(0.)))
    }

    fn y_in(node: &BoxNode, laid: &Laid, offset: usize) -> Option<f32> {
        let pad = node.padding.top + node.border.top;
        match (&node.kind, &laid.kind) {
            (BoxKind::Inline(ib), LaidKind::Inline(l)) => {
                for line in &l.lines {
                    for f in &line.frags {
                        if let Item::Text { range, .. } = &ib.items[f.item as usize]
                            && offset <= range.end
                        {
                            return Some(pad + line.y);
                        }
                    }
                }
                Some(pad)
            }
            (BoxKind::Flow(kids), LaidKind::Flow(p)) => Self::y_in_list(kids.iter(), p, offset),
            (BoxKind::Columns { cols, .. }, LaidKind::Flow(p)) => Self::y_in_list(cols.iter(), p, offset),
            (BoxKind::Details { summary, body, open }, LaidKind::Flow(p)) => {
                let list: Vec<&BoxNode> = if *open { std::iter::once(&**summary).chain(body.iter()).collect() } else { vec![&**summary] };
                Self::y_in_list(list.into_iter(), p, offset)
            }
            (BoxKind::Code(c), LaidKind::Code(l)) => {
                let row = c.lines.partition_point(|r| r.end < offset);
                Some(pad + row as f32 * l.line_h)
            }
            (BoxKind::Table(t), LaidKind::Table(l)) => {
                for (rb, row) in t.rows.iter().zip(&l.rows) {
                    let end = rb.cells.last().map_or(0, |c| c.content.start);
                    if offset <= end + 1 {
                        return Some(pad + row.y);
                    }
                }
                Some(pad)
            }
            _ => Some(0.),
        }
    }

    fn y_in_list<'a>(kids: impl Iterator<Item = &'a BoxNode>, p: &[Placed], offset: usize) -> Option<f32> {
        let mut chosen = None;
        for (k, placed) in kids.zip(p) {
            if k.start <= offset {
                chosen = Some((k, placed));
            } else {
                break;
            }
        }
        let (k, placed) = chosen?;
        Some(placed.y + Self::y_in(k, &placed.laid, offset).unwrap_or(0.))
    }

    /// `true`: it's done, or there's no such place.
    fn scroll_to_anchor(&mut self, id: &str, bounds: Bounds<Pixels>, _: &mut Window) -> bool {
        let Some(offset) = self.anchor_offset(id) else { return true };
        let _ = bounds;
        let pad = self.pad_top();
        let Some(d) = self.doc.as_ref() else { return false };
        let Some((i, y)) = Self::y_of_offset(d, offset) else { return true };
        if !d.tops[i].measured || d.tops[i].laid.is_none() {
            // Near enough to be laid out next frame.
            self.scroll = (y - pad).max(0.);
            return false;
        }
        self.scroll = (y + pad - pad).clamp(0., self.max_scroll());
        true
    }

    /// The change marks, top-level: a rendered diff's stepper and ruler go by them.
    fn collect_marks(node: &BoxNode, laid: &Laid, y: f32, out: &mut Vec<(f32, f32, Mark)>) {
        if let Some(m) = node.mark {
            out.push((y, laid.h.max(1.), m));
            return;
        }
        match (&node.kind, &laid.kind) {
            (BoxKind::Flow(kids), LaidKind::Flow(p)) => kids.iter().zip(p).for_each(|(k, pl)| Self::collect_marks(k, &pl.laid, y + pl.y, out)),
            (BoxKind::Details { summary, body, open }, LaidKind::Flow(p)) => {
                let list: Vec<&BoxNode> = if *open { std::iter::once(&**summary).chain(body.iter()).collect() } else { vec![&**summary] };
                list.into_iter().zip(p).for_each(|(k, pl)| Self::collect_marks(k, &pl.laid, y + pl.y, out));
            }
            (BoxKind::Table(t), LaidKind::Table(l)) => {
                for (rb, row) in t.rows.iter().zip(&l.rows) {
                    if let Some(m) = rb.mark {
                        out.push((y + row.y, row.h, m));
                    }
                }
            }
            (BoxKind::Columns { cols, .. }, LaidKind::Flow(p)) => cols.iter().zip(p).for_each(|(k, pl)| Self::collect_marks(k, &pl.laid, y + pl.y, out)),
            _ => {}
        }
    }

    /// `revealTop`: a change centred, or its top in view when it's taller than the pane.
    fn reveal_top(top: f32, h: f32, view: f32) -> f32 {
        const STEP_MARGIN: f32 = 16.;
        if h + 2. * STEP_MARGIN >= view { top - STEP_MARGIN } else { top + h / 2. - view / 2. }
    }

    /// Next, the first change starting below the pane's centre line; Previous, the last one ending above it. Past the
    /// last it wraps to the first, and before the first to the last. Right after a step, on from the one it went to.
    pub fn step_change(&mut self, forward: bool) {
        let marks = self.marks.clone();
        if marks.is_empty() {
            return;
        }
        let pad = self.pad_top();
        let centre = self.scroll + self.view_h / 2.;
        let current = self.last_step.filter(|(i, top)| *i < marks.len() && (self.scroll - top).abs() <= 1.).map(|x| x.0);
        let i = match (current, forward) {
            (Some(i), true) => (i + 1) % marks.len(),
            (Some(i), false) => (i + marks.len() - 1) % marks.len(),
            (None, true) => marks.iter().position(|m| m.0 + pad > centre).unwrap_or(0),
            (None, false) => marks.iter().rposition(|m| m.0 + pad + m.1 < centre).unwrap_or(marks.len() - 1),
        };
        self.scroll = Self::reveal_top(marks[i].0 + pad, marks[i].1, self.view_h).clamp(0., self.max_scroll());
        self.last_step = Some((i, self.scroll));
    }
}

struct PaintCx<'a> {
    st: &'a MdState,
    d: &'a Doc,
    band: (f32, f32),
    origin: Point<Pixels>,
    selection: Option<(usize, usize)>,
    /// `Family::Code`'s colours fall back to the pre's.
    editor_fg: Hsla,
}

fn q(window: &mut Window, x: f32, y: f32, w: f32, h: f32, color: Hsla) {
    window.paint_quad(fill(Bounds::new(point(px(x), px(y)), size(px(w), px(h))), color));
}

fn rq(window: &mut Window, x: f32, y: f32, w: f32, h: f32, r: f32, color: Hsla) {
    window.paint_quad(fill(Bounds::new(point(px(x), px(y)), size(px(w), px(h))), color).corner_radii(px(r)));
}

fn border_box(window: &mut Window, x: f32, y: f32, w: f32, h: f32, r: f32, bg: Hsla, border: Hsla, style: BorderStyle) {
    window.paint_quad(PaintQuad { bounds: Bounds::new(point(px(x), px(y)), size(px(w), px(h))), corner_radii: Corners::all(px(r)), background: bg.into(), border_widths: Edges::all(px(1.)), border_color: border, border_style: style });
}

fn paint_small_text(text: &str, x: f32, baseline: f32, color: Hsla, size_px: f32, window: &mut Window, cx: &mut App) -> f32 {
    let font = window.text_style().font();
    let run = TextRun { len: text.len(), font: font.clone(), color, background_color: None, underline: None, strikethrough: None };
    let line = window.text_system().shape_line(text.to_string().into(), px(size_px), &[run], None);
    let id = window.text_system().resolve_font(&font);
    let asc = window.text_system().ascent(id, px(size_px));
    line.paint(point(px(x), px(baseline) - asc), px(size_px * 1.3), TextAlign::Left, None, window, cx).ok();
    line.width.as_f32()
}

fn paint_shaped(sh: &Shaped, from: usize, to: usize, x: f32, baseline: f32, size_px: f32, color: &mut dyn FnMut(usize) -> Hsla, window: &mut Window, cx: &mut App) {
    let base = sh.glyphs.get(from).map_or(0., |g| g.x);
    let y = baseline.round();
    let size_px = px(size_px);
    let mask = window.content_mask().bounds;
    let (left, right) = (mask.origin.x.as_f32() - 40., (mask.origin.x + mask.size.width).as_f32() + 40.);
    for i in from..to {
        let g = &sh.glyphs[i];
        let gx = (x + g.x - base).round();
        if gx < left {
            continue;
        }
        if gx > right {
            break;
        }
        let origin = point(px(gx), px(y));
        if g.emoji {
            window.paint_emoji(origin, g.font, g.id, size_px).ok();
        } else {
            window.paint_glyph(origin, g.font, g.id, size_px, color(g.index as usize)).ok();
        }
    }
    let _ = cx;
}

fn sel_bg() -> Hsla {
    theme::selection_bg()
}

impl PaintCx<'_> {
    fn target(&self, link: u32) -> Option<&LinkTarget> {
        self.d.targets.get(link as usize).filter(|t| **t != LinkTarget::Inert)
    }

    fn text_color(&self, style: &TextStyle) -> Hsla {
        if style.link.is_some_and(|l| self.target(l).is_some()) { theme::link() } else { style.color.color() }
    }

    fn paint_inline(&self, node: &BoxNode, ib: &InlineBlock, l: &LaidInline, ox: f32, oy: f32, window: &mut Window, cx: &mut App) {
        let (x0, y0) = (ox + node.padding.left + node.border.left, oy + node.padding.top + node.border.top);
        for line in &l.lines {
            let top = y0 + line.y;
            if top + line.h < self.band.0 || top > self.band.1 {
                continue;
            }
            let lx = x0 + line.x;
            let base = top + line.baseline;
            // Grounds first: inline code's and a changed word's, then the selection, then the text.
            for (k, f) in line.frags.iter().enumerate() {
                let it = &ib.items[f.item as usize];
                let (Item::Text { style, .. } | Item::Pad { style, .. }) = it else { continue };
                if style.code || style.word.is_some() {
                    let m = code_metrics(style, l, f);
                    let (asc, desc, pad) = m;
                    let next_same = line.frags.get(k + 1).is_some_and(|n| matches!(&ib.items[n.item as usize], Item::Text { style: s, .. } | Item::Pad { style: s, .. } if s.code == style.code && s.word == style.word));
                    let prev_same = k > 0 && matches!(&ib.items[line.frags[k - 1].item as usize], Item::Text { style: s, .. } | Item::Pad { style: s, .. } if s.code == style.code && s.word == style.word);
                    let radii = Corners { top_left: if prev_same { px(0.) } else { px(if style.code { 4. } else { 2. }) }, bottom_left: if prev_same { px(0.) } else { px(if style.code { 4. } else { 2. }) }, top_right: if next_same { px(0.) } else { px(if style.code { 4. } else { 2. }) }, bottom_right: if next_same { px(0.) } else { px(if style.code { 4. } else { 2. }) } };
                    let b = Bounds::new(point(px(lx + f.x), px(base - asc - pad)), size(px(f.w.max(0.)), px(asc + desc + 2. * pad)));
                    if style.code {
                        window.paint_quad(fill(b, Tok::AppBg.color()).corner_radii(radii));
                        if let Some(t) = style.tint {
                            window.paint_quad(fill(b, t.color()).corner_radii(radii));
                        }
                    }
                    if let Some(ins) = style.word {
                        let tint = if ins { theme::md_diff_ins_text() } else { theme::md_diff_del_text() };
                        window.paint_quad(fill(b, tint).corner_radii(radii));
                    }
                }
            }
            if let Some((a, b)) = self.selection {
                for f in &line.frags {
                    let it = &ib.items[f.item as usize];
                    let range = match it {
                        Item::Text { range, .. } | Item::Kbd { range, .. } => range,
                        _ => continue,
                    };
                    let sh = &l.shaped[f.item as usize];
                    let at = |g: u32| sh.glyphs.get(g as usize).map_or(range.end - range.start, |x| x.index as usize) + range.start;
                    let (fa, fb) = (at(f.from), at(f.to));
                    if fb <= a || fa >= b || fa >= fb {
                        continue;
                    }
                    let base_x = sh.glyphs.get(f.from as usize).map_or(0., |g| g.x);
                    let (ra, rb) = (a.max(fa), b.min(fb));
                    let sa = sh.x_at_byte(ra - range.start) - base_x;
                    let sb = sh.x_at_byte(rb - range.start) - base_x;
                    // A selection running on past this line shows its end.
                    let extra = if b > fb && f.item as usize + 1 >= ib.items.len() { 4. } else { 0. };
                    q(window, lx + f.x + sa, top, sb - sa + extra, line.h, sel_bg());
                }
            }
            for f in &line.frags {
                let it = &ib.items[f.item as usize];
                match it {
                    Item::Text { style, .. } => {
                        let sh = &l.shaped[f.item as usize];
                        let color = self.text_color(style);
                        let up = match style.valign {
                            VAlign::Base => 0.,
                            VAlign::Super => (style.size * 1.2 / 3. + 1.).floor(),
                            VAlign::Sub => -((style.size * 1.2 / 5. + 1.).floor()),
                        };
                        let by = base - up;
                        paint_shaped(sh, f.from as usize, f.to as usize, lx + f.x, by, style.size, &mut |_| color, window, cx);
                        let hovered = style.link.is_some() && style.link == self.st.hover_link && self.target(style.link.unwrap_or(0)).is_some();
                        if style.strike {
                            q(window, lx + f.x, by.round() - (style.size * 0.29).round(), f.w, (style.size / 13.).max(1.).round(), color);
                        }
                        if style.underline || hovered {
                            q(window, lx + f.x, by.round() + (style.size * 0.12).round().max(1.), f.w, (style.size / 13.).max(1.).round(), color);
                        }
                        if let (Some(Focus::Link(fl)), Some(sl)) = (self.st.focused, style.link)
                            && fl == sl
                        {
                            let fm = (style.size * 1.1).round();
                            window.paint_quad(PaintQuad { bounds: Bounds::new(point(px(lx + f.x - 2.), px(by - fm - 1.)), size(px(f.w + 4.), px(fm + 6.))), corner_radii: Corners::all(px(2.)), background: gpui::transparent_black().into(), border_widths: Edges::all(px(1.)), border_color: theme::focus_outline(), border_style: BorderStyle::Solid });
                        }
                    }
                    Item::Kbd { style, .. } => {
                        let (bx, by) = (lx + f.x, base - f.above);
                        rq(window, bx, by, f.w, f.h, 4., theme::section_border());
                        window.paint_quad(fill(Bounds::new(point(px(bx + 1.), px(by + 1.)), size(px(f.w - 2.), px(f.h - 3.))), theme::panel_bg1()).corner_radii(px(3.)));
                        let color = style.color.color();
                        let sh = &l.shaped[f.item as usize];
                        paint_shaped(sh, 0, sh.glyphs.len(), bx + 6., base, style.size, &mut |_| color, window, cx);
                    }
                    Item::Task { checked, .. } => paint_task(lx + f.x, base - f.above, *checked, window, cx),
                    Item::Image { slot, style } => self.paint_image(*slot, style, lx + f.x, base - f.above, f.w, f.h, window, cx),
                    _ => {}
                }
            }
        }
    }

    fn paint_image(&self, slot: u32, style: &TextStyle, x: f32, y: f32, w: f32, h: f32, window: &mut Window, cx: &mut App) {
        let Some(s) = self.d.slots.get(slot as usize) else { return };
        let _ = style;
        let dashed = |window: &mut Window, bg: Hsla| border_box(window, x, y, w, h, 4., bg, theme::section_border(), BorderStyle::Dashed);
        match &s.state {
            SlotState::Shown { image, thumb, .. } => {
                let b = Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
                window.paint_image(b, b, Corners::default(), thumb.as_ref().unwrap_or(image).clone(), 0, false).ok();
                if let Some(ins) = s.src.word {
                    // An outline: outside the picture.
                    window.paint_quad(PaintQuad { bounds: b.dilate(px(2.)), corner_radii: Corners::default(), background: gpui::transparent_black().into(), border_widths: Edges::all(px(2.)), border_color: if ins { theme::status_added() } else { theme::status_deleted() }, border_style: BorderStyle::Solid });
                }
                if self.st.focused == Some(Focus::Image(slot)) {
                    window.paint_quad(PaintQuad { bounds: b.dilate(px(4.)), corner_radii: Corners::default(), background: gpui::transparent_black().into(), border_widths: Edges::all(px(2.)), border_color: theme::blue(), border_style: BorderStyle::Solid });
                }
            }
            SlotState::Loading => {
                dashed(window, theme::panel_bg1());
                if s.src.video {
                    let name = video_name(&s.src);
                    let cy = y + h / 2.;
                    let tw = paint_small_text("", 0., 0., theme::text_secondary(), 11., window, cx);
                    let _ = tw;
                    window.paint_svg(Bounds::new(point(px(x + 9.), px(cy - 7.)), size(px(14.), px(14.))), "icons/play.svg".into(), None, TransformationMatrix::unit(), theme::text_secondary(), cx).ok();
                    paint_small_text(&name, x + 29., cy + 4., theme::text_secondary(), 11., window, cx);
                }
            }
            SlotState::Ask { host } => {
                dashed(window, gpui::transparent_black());
                let what = if s.src.video { "video" } else { "image" };
                let base = y + (h + 8.) / 2.;
                let mut bx = x + 9.;
                if !s.src.alt.is_empty() {
                    let tw = paint_small_text(&s.src.alt, bx, base, theme::text_secondary(), 11., window, cx);
                    bx += tw + 6.;
                }
                let label = format!("Load {what} from {host}");
                let bh = h - 10.;
                let run_w = {
                    let font = window.text_style().font();
                    let run = TextRun { len: label.len(), font, color: theme::text_normal(), background_color: None, underline: None, strikethrough: None };
                    window.text_system().shape_line(label.clone().into(), px(11.), &[run], None).width.as_f32()
                };
                let hovered = self.st.hover_load == Some(slot);
                border_box(window, bx, y + 5., run_w + 18., bh, 4., if hovered { theme::control_hover_bg() } else { theme::control_bg() }, theme::section_border(), BorderStyle::Solid);
                paint_small_text(&label, bx + 9., base, theme::text_normal(), 11., window, cx);
            }
            SlotState::Broken { why } if s.src.mermaid => {
                paint_small_text(&format!("Couldn't draw the diagram: {why}"), x, y + 13., theme::red(), 11., window, cx);
            }
            SlotState::Broken { .. } => {
                dashed(window, gpui::transparent_black());
                window.paint_svg(Bounds::new(point(px(x + 9.), px(y + (h - 14.) / 2.)), size(px(14.), px(14.))), "icons/image-off.svg".into(), None, TransformationMatrix::unit(), theme::text_secondary(), cx).ok();
                if !s.src.alt.is_empty() {
                    paint_small_text(&s.src.alt, x + 29., y + h / 2. + 4., theme::text_secondary(), 11., window, cx);
                }
            }
            SlotState::Video { why, .. } => {
                dashed(window, gpui::transparent_black());
                paint_small_text(why, x + 9., y + h / 2. + 4., theme::text_secondary(), 11., window, cx);
            }
        }
    }

    fn paint_code(&self, node: &BoxNode, c: &CodeBox, l: &LaidCode, ox: f32, oy: f32, w: f32, h: f32, window: &mut Window, cx: &mut App) {
        let scroll = self.st.hscroll.get(&node.id).copied().unwrap_or(0.).clamp(0., (l.width + 20. - w).max(0.));
        let (ix, iy) = (ox + node.padding.left, oy + node.padding.top);
        let colours = self.d.colours.get(c.index as usize).and_then(Option::as_ref);
        let clip = Bounds::new(point(px(ox), px(oy)), size(px(w), px(h)));
        let hl = window.content_mask().bounds.intersect(&clip);
        let default = if colours.is_some() { self.editor_fg } else { c.style.color.color() };
        window.with_content_mask(Some(ContentMask { bounds: hl }), |window| {
            for (i, sh) in l.lines.iter().enumerate() {
                let y = iy + i as f32 * l.line_h;
                if y + l.line_h < self.band.0 || y > self.band.1 {
                    continue;
                }
                let mark = c.marks.as_ref().and_then(|m| m.get(i)).copied().unwrap_or(b' ');
                let tx = ix - scroll;
                if mark == b'+' {
                    q(window, ox, y, w.max(l.width + 20.), l.line_h, theme::md_diff_ins_block());
                } else if mark == b'-' {
                    q(window, ox, y, w.max(l.width + 20.), l.line_h, theme::md_diff_del_block());
                }
                let sign_w = if c.signs { (c.style.size * 0.6).round() * 2. } else { 0. };
                let text_x = tx + sign_w;
                if let Some(ranges) = c.words.as_ref().and_then(|w| w.get(i)).filter(|_| mark != b' ') {
                    let line_text = &self.d.built.stream[c.lines[i].clone()];
                    let tint = if mark == b'+' { theme::md_diff_ins_text() } else { theme::md_diff_del_text() };
                    for &(a, b) in ranges {
                        // Ranges are in UTF-16 units there, and a line's mostly ASCII here.
                        let (ba, bb) = (utf16_to_byte(line_text, a), utf16_to_byte(line_text, b));
                        rq(window, text_x + sh.x_at_byte(ba), y, sh.x_at_byte(bb) - sh.x_at_byte(ba), l.line_h, 2., tint);
                    }
                }
                if let Some((a, b)) = self.selection {
                    let r = &c.lines[i];
                    if r.end >= a && r.start < b {
                        let (ra, rb) = (a.max(r.start), b.min(r.end));
                        let (sa, sb) = (sh.x_at_byte(ra - r.start), sh.x_at_byte(rb - r.start));
                        let extra = if b > r.end { 6. } else { 0. };
                        q(window, text_x + sa, y, sb - sa + extra, l.line_h, sel_bg());
                    }
                }
                if c.signs {
                    let sign = match mark {
                        b'+' => "+",
                        b'-' => "-",
                        _ => "",
                    };
                    if !sign.is_empty() {
                        paint_small_text(sign, tx, y + l.baseline, theme::text_secondary(), c.style.size, window, cx);
                    }
                }
                let spans: &[Span] = colours.and_then(|p| p.get(i)).map_or(&[], Vec::as_slice);
                let mut at = 0;
                let mut color = |byte: usize| -> Hsla {
                    while at < spans.len() && (spans[at].end as usize) <= byte {
                        at += 1;
                    }
                    match spans.get(at) {
                        Some(s) if (s.start as usize) <= byte => theme::c(s.color),
                        _ => default,
                    }
                };
                paint_shaped(sh, 0, sh.glyphs.len(), text_x, y + l.baseline, c.style.size, &mut color, window, cx);
            }
        });
        if l.overflow {
            let by = oy + h - BAR;
            paint_hbar(ox, by, w, l.width + 20., scroll, window);
        }
        // Copy, over the block, on hover or with the keyboard on it.
        let hovered = self.st.hover_code == Some(c.index);
        let focused = self.st.focused == Some(Focus::Copy(c.index));
        if c.copy_button && (hovered || focused) {
            let (bx, by) = (ox + w - 4. - COPY_SIZE, oy + 4.);
            let over = self.st.hover_copy == Some(c.index);
            border_box(window, bx, by, COPY_SIZE, COPY_SIZE, 4., if over { theme::control_hover_bg() } else { theme::control_bg() }, if focused { theme::focus_outline() } else { theme::section_border() }, BorderStyle::Solid);
            window.paint_svg(Bounds::new(point(px(bx + 4.), px(by + 4.)), size(px(12.), px(12.))), "icons/copy.svg".into(), None, TransformationMatrix::unit(), if over { theme::text_normal() } else { theme::text_secondary() }, cx).ok();
        }
    }

    fn paint_table(&self, node: &BoxNode, t: &TableBox, l: &LaidTable, ox: f32, oy: f32, w: f32, window: &mut Window, cx: &mut App) {
        let (ix, iy) = (ox + node.padding.left + node.border.left, oy + node.padding.top + node.border.top);
        if let (Some(cap), Some(p)) = (t.caption.as_deref(), l.caption.as_deref()) {
            self.paint_node(cap, &p.laid, ix + p.x, iy + p.y, window, cx);
        }
        let scroll = self.st.hscroll.get(&node.id).copied().unwrap_or(0.).clamp(0., (l.width - l.view_w).max(0.));
        let clip = Bounds::new(point(px(ix), px(iy)), size(px(l.view_w.max(1.)), px(1e6)));
        let mask = window.content_mask().bounds.intersect(&clip);
        window.with_content_mask(Some(ContentMask { bounds: mask }), |window| {
            let tx = ix - scroll;
            let (table_top, table_bottom) = (iy + l.top, iy + l.rows.last().map_or(l.top, |r| r.y + r.h));
            if let Some(bg) = t.bg {
                q(window, tx, table_top, l.width, table_bottom - table_top + 0.5, bg.color());
            }
            for (rb, row) in t.rows.iter().zip(&l.rows) {
                let ry = iy + row.y;
                if ry + row.h < self.band.0 || ry > self.band.1 {
                    continue;
                }
                let row_x = tx + 0.5;
                let row_w = l.width - 1.;
                // An added or removed row's tint takes the zebra's place.
                if let Some(bg) = rb.bg.filter(|_| !matches!(rb.mark, Some(Mark::Added | Mark::Removed))) {
                    q(window, row_x, ry, row_w, row.h, bg.color());
                }
                if let Some(m) = rb.mark {
                    match m {
                        Mark::Added => q(window, row_x, ry, row_w, row.h, theme::md_diff_ins_block()),
                        Mark::Removed => q(window, row_x, ry, row_w, row.h, theme::md_diff_del_block()),
                        Mark::Changed => {}
                    }
                }
                if rb.empty {
                    window.paint_quad(fill(Bounds::new(point(px(row_x), px(ry)), size(px(row_w), px(row.h))), pattern_slash(theme::diff_diagonal(), 2., 6.)));
                }
                let first_row = std::ptr::eq(row, &l.rows[0]);
                let color = theme::section_border();
                // Collapsed: each line once, its colour has an alpha. Chromium's snap to whole pixels.
                let (line_top, line_bottom) = ((ry - 0.5).round(), (ry + row.h - 0.5).round());
                for (k, ((cell, placed), (cx_, cw))) in rb.cells.iter().zip(&row.cells).zip(&row.boxes).enumerate() {
                    self.paint_node(&cell.content, &placed.laid, tx + placed.x, ry + placed.y, window, cx);
                    let (bx, right) = ((tx + cx_ - 0.5).round(), (tx + cx_ + cw - 0.5).round());
                    if first_row {
                        q(window, bx, line_top, right - bx + 1., 1., color);
                    }
                    q(window, bx, line_bottom, right - bx + 1., 1., color);
                    let (from, to) = (line_top + 1., line_bottom);
                    if k == 0 {
                        q(window, bx, from, 1., to - from, color);
                    }
                    q(window, right, from, 1., to - from, color);
                }
                if let Some(m) = rb.mark {
                    let c = mark_color(m);
                    q(window, row_x, ry, 3., row.h, c);
                }
            }
        });
        if l.overflow {
            paint_hbar(ix, oy + node.padding.top + l.rows.last().map_or(0., |r| r.y + r.h) + 0.5, l.view_w.min(w), l.width, scroll, window);
        }
    }

    fn paint_marker(&self, node: &BoxNode, laid: &Laid, ox: f32, oy: f32, window: &mut Window, cx: &mut App) {
        let (Some(marker), Some(style)) = (&node.marker, &node.marker_style) else { return };
        let Some(base) = first_baseline(node, laid) else { return };
        let y = (oy + base).round();
        let color = style.color.color();
        match marker {
            Marker::Bullet(kind) => {
                // Blink's bullet: a third of the ascent and a pixel across, two thirds of the ascent and two more from the text.
                let ascent = (style.size * 14. / 13.).round();
                let d = ((ascent * 2. / 3. + 1.) / 2.).floor();
                let (x, top) = ((ox - (ascent * 2. / 3.).floor() - d - 2.).round(), y - (ascent / 2.).round());
                match kind {
                    MarkerKind::Disc => rq(window, x, top, d, d, d / 2., color),
                    MarkerKind::Circle => window.paint_quad(PaintQuad { bounds: Bounds::new(point(px(x), px(top)), size(px(d), px(d))), corner_radii: Corners::all(px(d / 2.)), background: gpui::transparent_black().into(), border_widths: Edges::all(px(1.)), border_color: color, border_style: BorderStyle::Solid }),
                    MarkerKind::Square => q(window, x, top, d, d, color),
                }
            }
            Marker::Number(text) => {
                let font = window.text_style().font();
                let run = TextRun { len: text.len(), font: font.clone(), color, background_color: None, underline: None, strikethrough: None };
                let line = window.text_system().shape_line(text.clone().into(), px(style.size), &[run], None);
                let id = window.text_system().resolve_font(&font);
                let asc = window.text_system().ascent(id, px(style.size));
                let (mut w, mut prev) = (0., 0.);
                for g in line.runs.iter().flat_map(|r| &r.glyphs) {
                    w += (g.position.x.as_f32() - prev).round();
                    prev = g.position.x.as_f32();
                }
                w += (line.width.as_f32() - prev).round();
                line.paint(point(px((ox - w).round()), px(y) - asc), px(style.size * 1.3), TextAlign::Left, None, window, cx).ok();
            }
        }
    }

    fn paint_node(&self, node: &BoxNode, laid: &Laid, ox: f32, oy: f32, window: &mut Window, cx: &mut App) {
        let (w, h) = (laid.w, laid.h);
        if oy > self.band.1 || oy + h < self.band.0 {
            return;
        }
        if let Some(bg) = node.bg {
            if node.radius > 0. {
                rq(window, ox, oy, w, h, node.radius, bg.color());
            } else if matches!(node.kind, BoxKind::Rule) {
                q(window, ox, oy, w, 2., bg.color());
            } else {
                q(window, ox, oy, w, h, bg.color());
            }
        }
        if node.tinted
            && let Some(m) = node.mark
        {
            let tint = if m == Mark::Added { theme::md_diff_ins_block() } else { theme::md_diff_del_block() };
            q(window, ox, oy, w, h, tint);
        }
        if let (Some(bar), Some(m)) = (node.bar, node.mark) {
            let x = match bar {
                BarAt::Block => ox - 12.,
                BarAt::Item => ox - node.list_indent - 12.,
            };
            rq(window, x, oy, 3., h, 2., mark_color(m));
        }
        let border = node.border_tok.color();
        if node.border.top > 0. && node.border.left > 0. && node.border.right > 0. && node.border.bottom > 0. {
            border_box(window, ox, oy, w, h, node.radius, gpui::transparent_black(), border, BorderStyle::Solid);
        } else {
            if node.border.bottom > 0. {
                q(window, ox, oy + h - node.border.bottom, w, node.border.bottom, border);
            }
            if node.border.top > 0. {
                q(window, ox, oy, w, node.border.top, border);
            }
            if node.border.left > 0. {
                q(window, ox, oy, node.border.left, h, border);
            }
        }
        self.paint_marker(node, laid, ox, oy, window, cx);
        match (&node.kind, &laid.kind) {
            (BoxKind::Inline(ib), LaidKind::Inline(l)) => self.paint_inline(node, ib, l, ox, oy, window, cx),
            (BoxKind::Flow(kids), LaidKind::Flow(p)) => {
                for (k, pl) in kids.iter().zip(p) {
                    self.paint_node(k, &pl.laid, ox + pl.x, oy + pl.y, window, cx);
                }
            }
            (BoxKind::Columns { cols, gap }, LaidKind::Flow(p)) => {
                for (i, (k, pl)) in cols.iter().zip(p).enumerate() {
                    self.paint_node(k, &pl.laid, ox + pl.x, oy + pl.y, window, cx);
                    // A changed diagram's two sides have their frames; a split row's old cell is on the left.
                    if *gap == 8. {
                        let c = if i == 0 { theme::status_deleted() } else { theme::status_added() };
                        window.paint_quad(PaintQuad { bounds: Bounds::new(point(px(ox + pl.x), px(oy + pl.y)), size(px(pl.laid.w), px(pl.laid.h))), corner_radii: Corners::all(px(6.)), background: gpui::transparent_black().into(), border_widths: Edges::all(px(2.)), border_color: c, border_style: BorderStyle::Solid });
                    }
                }
                if *gap > 8.
                    && let Some(m) = node.mark
                {
                    let c = mark_color(m);
                    if m != Mark::Added {
                        rq(window, ox - 12., oy, 3., h, 2., c);
                    }
                    if m != Mark::Removed {
                        rq(window, ox + w / 2. + 12. - 12., oy, 3., h, 2., c);
                    }
                }
            }
            (BoxKind::Details { summary, body, open }, LaidKind::Flow(p)) => {
                let list: Vec<&BoxNode> = if *open { std::iter::once(&**summary).chain(body.iter()).collect() } else { vec![&**summary] };
                for (k, pl) in list.into_iter().zip(p) {
                    self.paint_node(k, &pl.laid, ox + pl.x, oy + pl.y, window, cx);
                }
                if let (Some(pl), Some(style)) = (p.first(), inline_style(summary)) {
                    paint_triangle(ox, oy + pl.y, first_baseline(summary, &pl.laid).unwrap_or(0.), style.size, *open, style.color.color(), window);
                }
                if self.st.focused == Some(Focus::Summary(node.id))
                    && let Some(pl) = p.first()
                {
                    window.paint_quad(PaintQuad { bounds: Bounds::new(point(px(ox), px(oy + pl.y)), size(px(w), px(pl.laid.h))), corner_radii: Corners::all(px(2.)), background: gpui::transparent_black().into(), border_widths: Edges::all(px(1.)), border_color: theme::focus_outline(), border_style: BorderStyle::Solid });
                }
            }
            (BoxKind::Code(c), LaidKind::Code(l)) => self.paint_code(node, c, l, ox, oy, w, h, window, cx),
            (BoxKind::Table(t), LaidKind::Table(l)) => self.paint_table(node, t, l, ox, oy, w, window, cx),
            (BoxKind::Hatch, _) => {
                window.paint_quad(fill(Bounds::new(point(px(ox), px(oy)), size(px(w), px(h.max(1.)))), pattern_slash(theme::diff_diagonal(), 2., 6.)));
            }
            _ => {}
        }
    }
}

fn mark_color(m: Mark) -> Hsla {
    match m {
        Mark::Added => theme::status_added(),
        Mark::Removed => theme::status_deleted(),
        Mark::Changed => theme::status_modified(),
    }
}

/// A character range from JavaScript: UTF-16 units to this line's bytes.
pub fn utf16_to_byte(line: &str, units: usize) -> usize {
    let mut n = 0;
    for (i, c) in line.char_indices() {
        if n >= units {
            return i;
        }
        n += c.len_utf16();
    }
    line.len()
}

fn video_name(src: &ImageSrc) -> String {
    let path = src.src.split(['?', '#']).next().unwrap_or("");
    let name = path.rsplit('/').next().unwrap_or("");
    if name.is_empty() { "Video".to_string() } else { name.to_string() }
}

/// Inline code's ascent, descent and vertical padding: a font of its own, on the line's baseline.
fn code_metrics(style: &TextStyle, _l: &LaidInline, _f: &Frag) -> (f32, f32, f32) {
    let (asc, desc) = ((style.size * 1.02).round(), (style.size * 0.3).round());
    (asc, desc, if style.code { 0.15 * style.size } else { 0. })
}

fn inline_style(node: &BoxNode) -> Option<&TextStyle> {
    if let BoxKind::Inline(ib) = &node.kind { Some(&ib.strut) } else { None }
}

/// The first line's baseline in a box.
fn first_baseline(node: &BoxNode, laid: &Laid) -> Option<f32> {
    let pad = node.padding.top + node.border.top;
    match (&node.kind, &laid.kind) {
        (BoxKind::Inline(_), LaidKind::Inline(l)) => l.lines.first().map(|line| pad + line.y + line.baseline),
        (BoxKind::Flow(kids), LaidKind::Flow(p)) => kids.iter().zip(p).find_map(|(k, pl)| first_baseline(k, &pl.laid).map(|b| b + pl.y)),
        (BoxKind::Code(_), LaidKind::Code(l)) => Some(pad + l.baseline),
        (BoxKind::Table(_), LaidKind::Table(l)) => l.rows.first().and_then(|r| r.cells.first()).map(|_| pad + 15.),
        (BoxKind::Details { summary, .. }, LaidKind::Flow(p)) => p.first().and_then(|pl| first_baseline(summary, &pl.laid).map(|b| b + pl.y)),
        _ => None,
    }
}

fn paint_triangle(x: f32, y: f32, baseline: f32, size_px: f32, open: bool, color: Hsla, window: &mut Window) {
    // Blink's disclosure triangles, in its marker's box left of the text.
    let (w, h) = (size_px * 0.68, size_px * 0.68);
    let top = y + baseline - size_px * 0.8;
    let left = x;
    let mut path = Path::new(if open { point(px(left), px(top + h * 0.12)) } else { point(px(left), px(top)) });
    if open {
        path.line_to(point(px(left + w), px(top + h * 0.12)));
        path.line_to(point(px(left + w / 2.), px(top + h)));
    } else {
        path.line_to(point(px(left + w * 0.88), px(top + h / 2.)));
        path.line_to(point(px(left), px(top + h)));
    }
    window.paint_path(path, color);
}

fn paint_task(x: f32, y: f32, checked: bool, window: &mut Window, cx: &mut App) {
    // A disabled checkbox on Chromium's dark controls.
    let (x, y) = (x.round(), y.round());
    let ground = theme::text_secondary();
    if checked {
        rq(window, x, y, 13., 13., 2.5, Hsla { a: ground.a * 0.8, ..ground });
        window.paint_svg(Bounds::new(point(px(x + 1.), px(y + 1.)), size(px(11.), px(11.))), "icons/check.svg".into(), None, TransformationMatrix::unit(), theme::app_bg(), cx).ok();
    } else {
        window.paint_quad(PaintQuad { bounds: Bounds::new(point(px(x), px(y)), size(px(13.), px(13.))), corner_radii: Corners::all(px(2.5)), background: Hsla { a: 0.05, ..ground }.into(), border_widths: Edges::all(px(1.)), border_color: Hsla { a: ground.a * 0.7, ..ground }, border_style: BorderStyle::Solid });
    }
}

/// The app's scrollbar along the bottom of what scrolls: ten pixels with a six pixel thumb, rounded.
fn paint_hbar(x: f32, y: f32, track: f32, content: f32, scroll: f32, window: &mut Window) {
    let thumb = (track * track / content.max(1.)).max(24.);
    let max = (content - track).max(1.);
    let tx = x + (track - thumb) * (scroll / max).clamp(0., 1.);
    rq(window, tx + 2., y + 2., thumb - 4., BAR - 4., 3., theme::scroll_thumb());
}

impl MdView {
    pub fn new(opts: MdOptions, api: Arc<Api>, rt: tokio::runtime::Handle, retired: Rc<RefCell<Vec<Arc<RenderImage>>>>, cx: &mut Context<Self>) -> Self {
        let state = MdState {
            opts,
            api,
            rt,
            retired,
            content: None,
            dom: None,
            doc: None,
            too_large: false,
            generation: 0,
            diagrams: Arc::default(),
            drawn: HashMap::new(),
            mermaid_waits: false,
            stop: Arc::default(),
            theme_version: theme::version(),
            scroll: 0.,
            view_h: 0.,
            bounds: Bounds::default(),
            selection: None,
            drag: Drag::None,
            hover_link: None,
            hover_code: None,
            hover_copy: None,
            hover_load: None,
            hscroll: HashMap::new(),
            focused: None,
            cursor: CursorStyle::Arrow,
            pending_anchor: None,
            reveal_first_change: false,
            bar_hover: false,
            marks: Rc::default(),
            consented: Vec::new(),
            pending_load: Vec::new(),
            press_hit: None,
            press_at: Point::default(),
            last_step: None,
            ruler_drag: false,
        };
        Self { state: Rc::new(RefCell::new(state)), focus: cx.focus_handle() }
    }

    /// What it shows now. `None` from `too_large` afterwards: the owner shows Source.
    pub fn set_content(&mut self, content: Content, keep_scroll: bool, cx: &mut Context<Self>) {
        let (opts, generation, small) = {
            let mut st = self.state.borrow_mut();
            st.generation += 1;
            st.stop.store(true, Ordering::Relaxed);
            st.stop = Arc::default();
            if !keep_scroll {
                st.scroll = 0.;
            }
            st.too_large = false;
            let small = match &content {
                Content::Text(t) => t.len() <= SYNC_PARSE_CHARS,
                Content::Diff { old, new, .. } => old.len() + new.len() <= SYNC_PARSE_CHARS,
            };
            st.content = Some(content.clone());
            let o = &st.opts;
            (Prep { flavor: o.flavor, frontmatter: o.frontmatter, max_bytes: o.max_bytes, params: o.params, base: o.base.clone() }, st.generation, small)
        };
        if small || cfg!(test) {
            let result = prepare(&content, &opts);
            self.adopt(result, cx);
            return;
        }
        let rt = self.state.borrow().rt.clone();
        let job = rt.spawn_blocking(move || prepare(&content, &opts));
        cx.spawn(async move |this, cx| {
            let Ok(result) = job.await else { return };
            this.update(cx, |this, cx| {
                if this.state.borrow().generation == generation {
                    this.adopt(result, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn adopt(&mut self, result: Result<(Arc<Vec<safe::Dom>>, Built), bool>, cx: &mut Context<Self>) {
        match result {
            Ok((dom, built)) => {
                let mut st = self.state.borrow_mut();
                st.dom = Some(dom);
                st.adopt(built);
                drop(st);
                self.colour(cx);
                self.draw_mermaid(0, cx);
            }
            Err(_) => {
                self.state.borrow_mut().too_large = true;
                cx.emit(MdEvent::TooLarge);
            }
        }
        cx.notify();
    }

    pub fn too_large(&self) -> bool {
        self.state.borrow().too_large
    }

    /// Goes to a heading, as a link to it does.
    pub fn go_to_anchor(&mut self, anchor: &str, cx: &mut Context<Self>) {
        let id = if anchor.starts_with("user-content-") { anchor.to_string() } else { format!("user-content-{anchor}") };
        self.state.borrow_mut().pending_anchor = Some(id);
        cx.notify();
    }

    pub fn is_ready(&self) -> bool {
        self.state.borrow().doc.is_some()
    }

    /// Its first blocks have been laid out for a frame.
    pub fn is_painted(&self) -> bool {
        let st = self.state.borrow();
        st.too_large || st.doc.as_ref().is_some_and(|d| d.tops.first().is_none_or(|t| t.measured))
    }

    /// The reading size or the line height changed: its boxes anew, what it scrolled to where it was.
    pub fn set_params(&mut self, params: Params, reading_font: Option<String>, measure_ch: Option<f32>, cx: &mut Context<Self>) {
        let mut st = self.state.borrow_mut();
        if st.opts.params == params && st.opts.reading_font == reading_font && st.opts.measure_ch == measure_ch {
            return;
        }
        st.opts.params = params;
        st.opts.reading_font = reading_font;
        st.opts.measure_ch = measure_ch;
        if let Some(dom) = st.dom.clone() {
            let share = if st.max_scroll() > 0. { st.scroll / st.max_scroll() } else { 0. };
            let built = boxes::build(&dom, params, st.opts.base.clone());
            let colours = st.doc.as_mut().map(|d| std::mem::take(&mut d.colours));
            let images: Vec<SlotState> = st.doc.as_mut().map(|d| d.slots.drain(..).map(|s| s.state).collect()).unwrap_or_default();
            st.adopt(built);
            if let Some(d) = st.doc.as_mut() {
                if let Some(c) = colours {
                    d.colours = c;
                }
                for (s, state) in d.slots.iter_mut().zip(images) {
                    s.asked = !matches!(state, SlotState::Loading);
                    s.state = state;
                }
            }
            st.scroll = share * st.max_scroll();
        }
        drop(st);
        self.draw_mermaid(0, cx);
        cx.notify();
    }

    /// Each fenced block's colours, off the UI thread: the near ones show first as they're all done in a blink.
    fn colour(&mut self, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let (rt, stop, generation, jobs) = {
            let st = self.state.borrow();
            let Some(d) = st.doc.as_ref() else { return };
            let jobs: Vec<(usize, &'static str, String)> = d
                .built
                .codes
                .iter()
                .enumerate()
                .filter_map(|(i, (lang, range))| {
                    if lang.is_empty() || range.len() > HIGHLIGHT_MAX_CHARS {
                        return None;
                    }
                    let id = crate::diff::language::by_fence(lang)?;
                    let text = d.built.stream.get(range.clone())?.strip_suffix('\n').unwrap_or_default().to_string();
                    Some((i, id, text))
                })
                .collect();
            (st.rt.clone(), st.stop.clone(), st.generation, jobs)
        };
        if jobs.is_empty() {
            return;
        }
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        rt.spawn_blocking(move || {
            for (i, id, text) in jobs {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let lines: Vec<&str> = text.split('\n').collect();
                if let Some(mut t) = Tokenizer::new(id)
                    && let Some(spans) = t.lines(&lines)
                    && tx.send((i, spans)).is_err()
                {
                    return;
                }
            }
        });
        cx.spawn(async move |this, cx| {
            while let Some((i, spans)) = rx.recv().await {
                // What's ready together lands together: a frame for them all.
                let mut batch = vec![(i, spans)];
                while let Ok(more) = rx.try_recv() {
                    batch.push(more);
                }
                let live = this.update(cx, |this, cx| {
                    let mut st = this.state.borrow_mut();
                    if st.generation != generation {
                        return false;
                    }
                    let Some(d) = st.doc.as_mut() else { return false };
                    for (i, spans) in batch {
                        // Bold or italic take their own fonts, so the lines are shaped again.
                        let restyled = spans.iter().flatten().any(|s| s.font & (FONT_BOLD | FONT_ITALIC) != 0);
                        d.colours[i] = Some(Arc::new(spans));
                        if restyled {
                            for t in &mut d.tops {
                                t.laid = None;
                            }
                        }
                    }
                    drop(st);
                    cx.notify();
                    true
                });
                if !matches!(live, Ok(true)) {
                    return;
                }
            }
        })
        .detach();
    }

    fn load_images(&mut self, cx: &mut Context<Self>) {
        let wanted: Vec<u32> = std::mem::take(&mut self.state.borrow_mut().pending_load);
        for slot in wanted {
            self.load_image(slot, false, cx);
        }
    }

    /// A picture much larger than its column is drawn from a copy at the column's size: the GPU's sampling alone leaves
    /// a moiré where Chromium's is smooth.
    fn shrink_image(&mut self, slot: u32, cx: &mut Context<Self>) {
        let (image, want, rt, generation) = {
            let st = self.state.borrow();
            let Some(d) = st.doc.as_ref() else { return };
            let Some(SlotState::Shown { image, w, thumb: None, .. }) = d.slots.get(slot as usize).map(|s| &s.state) else { return };
            if *w < d.width * 1.25 || d.width <= 0. {
                return;
            }
            (image.clone(), d.width, st.rt.clone(), st.generation)
        };
        let job = rt.spawn_blocking(move || {
            let s = image.size(0);
            let (w, h) = (s.width.0 as u32, s.height.0 as u32);
            let rgba = image::RgbaImage::from_raw(w, h, image.as_bytes(0)?.to_vec())?;
            let tw = (want.ceil() as u32).clamp(1, w);
            let th = ((h as f32 * tw as f32 / w as f32).round() as u32).max(1);
            // Its bytes are BGRA already: a resize doesn't mind.
            let small = image::imageops::resize(&rgba, tw, th, image::imageops::FilterType::Triangle);
            Some(Arc::new(RenderImage::new(vec![image::Frame::new(small)])))
        });
        cx.spawn(async move |this, cx| {
            let Ok(Some(made)) = job.await else { return };
            this.update(cx, |this, cx| {
                let mut st = this.state.borrow_mut();
                if st.generation != generation {
                    return;
                }
                if let Some(SlotState::Shown { thumb, .. }) = st.doc.as_mut().and_then(|d| d.slots.get_mut(slot as usize)).map(|s| &mut s.state) {
                    *thumb = Some(made);
                }
                drop(st);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn set_slot(&mut self, slot: u32, state: SlotState, cx: &mut Context<Self>) {
        let shown = matches!(state, SlotState::Shown { .. });
        self.set_slot_state(slot, state, cx);
        if shown {
            self.shrink_image(slot, cx);
        }
    }

    fn set_slot_state(&mut self, slot: u32, state: SlotState, cx: &mut Context<Self>) {
        let mut st = self.state.borrow_mut();
        let Some(d) = st.doc.as_mut() else { return };
        let Some(s) = d.slots.get_mut(slot as usize) else { return };
        s.state = state;
        let top = s.top;
        if let Some(t) = d.tops.get_mut(top) {
            t.laid = None;
            if t.measured {
                t.measured = false;
                d.measured -= 1;
            }
        }
        drop(st);
        cx.notify();
    }

    fn load_image(&mut self, slot: u32, consent: bool, cx: &mut Context<Self>) {
        let (source, video, repo, api, rt, worktree, generation) = {
            let st = self.state.borrow();
            let Some(s) = st.doc.as_ref().and_then(|d| d.slots.get(slot as usize)) else { return };
            (s.source.clone(), s.src.video, st.opts.ctx.repo(), st.api.clone(), st.rt.clone(), st.opts.worktree.clone(), st.generation)
        };
        let consent = consent || matches!(&source, ImageSource::Remote { url, .. } | ImageSource::Forge { url } if self.state.borrow().consented.contains(url));
        if video {
            let state = match &source {
                ImageSource::Remote { host, .. } if !consent => SlotState::Ask { host: host.clone() },
                ImageSource::Remote { url, .. } | ImageSource::Forge { url } => SlotState::Video { why: "This build can't play a video: open it in the browser".into(), url: Some(url.clone()) },
                _ => SlotState::Broken { why: "Not loaded: GitVolt plays videos from the forge only".into() },
            };
            return self.set_slot(slot, state, cx);
        }
        match source {
            ImageSource::None => self.set_slot(slot, SlotState::Broken { why: "Not loaded: GitVolt doesn't load images from this address".into() }, cx),
            ImageSource::Remote { host, .. } if !consent => self.set_slot(slot, SlotState::Ask { host }, cx),
            ImageSource::Data { url } => {
                let job = rt.spawn_blocking(move || {
                    let b64 = url.split_once(',').map(|x| x.1).unwrap_or_default().split_whitespace().collect::<String>();
                    crate::diff::image::decode_raster(&b64)
                });
                self.finish_image(slot, generation, job, cx);
            }
            ImageSource::Repo { path, commit } => {
                let svg = cx.svg_renderer();
                let side = if commit == "worktree" { serde_json::json!({ "kind": "worktree", "worktree": worktree }) } else { serde_json::json!({ "kind": "atCommit", "commit": commit }) };
                let job = rt.spawn(async move {
                    let (Ok(old), Ok(new)) = (serde_json::from_value(serde_json::json!({ "kind": "absent" })), serde_json::from_value(side)) else { return Err("the repository isn't available here".to_string()) };
                    let v = api.dispatch(Request::DiffContents { repo, path: path.clone(), old, new, force: false, old_path: None }).await.map_err(|e| e.message)?;
                    let known = v["image"].as_bool() == Some(true) || path.to_ascii_lowercase().ends_with(".svg");
                    if !known || v["tooLarge"].as_bool() == Some(true) {
                        return Err("not found in the repository".to_string());
                    }
                    tokio::task::spawn_blocking(move || match crate::diff::image::decode_side(&v["new"], &svg) {
                        crate::diff::image::Side::Image { decoded, .. } => Ok(decoded),
                        crate::diff::image::Side::None => Err("not found in the repository".to_string()),
                        crate::diff::image::Side::Failed { .. } => Err("the image didn't decode".to_string()),
                    })
                    .await
                    .map_err(|e| e.to_string())?
                });
                cx.spawn(async move |this, cx| {
                    let result = job.await;
                    this.update(cx, |this, cx| {
                        if this.state.borrow().generation != generation {
                            return;
                        }
                        let state = match result {
                            Ok(Ok(d)) => SlotState::Shown { image: d.image.clone(), w: d.w, h: d.h, thumb: None },
                            Ok(Err(why)) => SlotState::Broken { why: format!("Couldn't load: {why}") },
                            Err(e) => SlotState::Broken { why: format!("Couldn't load: {e}") },
                        };
                        this.set_slot(slot, state, cx);
                    })
                    .ok();
                })
                .detach();
            }
            ImageSource::Forge { url } | ImageSource::Remote { url, .. } => {
                let job = rt.spawn(async move {
                    let v = api.dispatch(Request::ForgeImage { repo, url, user_allowed: consent }).await.map_err(|e| e.message)?;
                    match v["kind"].as_str() {
                        Some("found") => {
                            let b64 = v["base64"].as_str().unwrap_or_default().to_string();
                            tokio::task::spawn_blocking(move || crate::diff::image::decode_raster(&b64).map(Ok).unwrap_or(Err("the image didn't decode".to_string()))).await.map_err(|e| e.to_string())?.map(FetchedImage::Image)
                        }
                        Some("ask") => Ok(FetchedImage::Ask(v["host"].as_str().unwrap_or_default().to_string())),
                        Some("expired") => Err("its link expired".to_string()),
                        _ => Err(v["reason"].as_str().unwrap_or("not found").to_string()),
                    }
                });
                cx.spawn(async move |this, cx| {
                    let result = job.await;
                    this.update(cx, |this, cx| {
                        if this.state.borrow().generation != generation {
                            return;
                        }
                        let state = match result {
                            Ok(Ok(FetchedImage::Image(d))) => SlotState::Shown { w: d.w, h: d.h, image: d.image, thumb: None },
                            Ok(Ok(FetchedImage::Ask(host))) => SlotState::Ask { host },
                            Ok(Err(why)) => SlotState::Broken { why: format!("Couldn't load: {why}") },
                            Err(e) => SlotState::Broken { why: format!("Couldn't load: {e}") },
                        };
                        this.set_slot(slot, state, cx);
                    })
                    .ok();
                })
                .detach();
            }
        }
    }

    fn finish_image(&mut self, slot: u32, generation: u64, job: tokio::task::JoinHandle<Option<crate::diff::image::Decoded>>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = job.await.ok().flatten();
            this.update(cx, |this, cx| {
                if this.state.borrow().generation != generation {
                    return;
                }
                let state = match result {
                    Some(d) => SlotState::Shown { w: d.w, h: d.h, image: d.image, thumb: None },
                    None => SlotState::Broken { why: "Couldn't load: the image didn't decode".into() },
                };
                this.set_slot(slot, state, cx);
            })
            .ok();
        })
        .detach();
    }

    fn open_external(&self, url: String, cx: &mut Context<Self>) {
        let (api, rt) = {
            let st = self.state.borrow();
            (st.api.clone(), st.rt.clone())
        };
        let job = rt.spawn(async move { api.dispatch(Request::OpenUrl { url }).await });
        cx.spawn(async move |_, cx| {
            if let Ok(Err(e)) = job.await {
                cx.update(|cx| crate::ui::toast::show_with(e.message, crate::ui::toast::Options { error: true, ..Default::default() }, cx));
            }
        })
        .detach();
    }

    fn browser_url(&self, link: u32) -> Option<String> {
        let st = self.state.borrow();
        let d = st.doc.as_ref()?;
        let old = matches!(d.built.links.get(link as usize)?, LinkSrc::Href { old_side: true, .. } | LinkSrc::Ref { old_side: true, .. });
        browser_url_for(&st.opts.env, st.link_env_side(old), d.targets.get(link as usize)?)
    }

    /// A plain click on a link (`openLinkTarget`). Nothing here navigates by itself.
    fn activate(&mut self, link: u32, modifiers: Modifiers, cx: &mut Context<Self>) {
        if modifiers.control || modifiers.platform {
            if let Some(url) = self.browser_url(link) {
                self.open_external(url, cx);
            }
            return;
        }
        let (target, ctx) = {
            let st = self.state.borrow();
            let Some(t) = st.doc.as_ref().and_then(|d| d.targets.get(link as usize)).cloned() else { return };
            (t, st.opts.ctx.clone())
        };
        match target {
            LinkTarget::Inert => {}
            LinkTarget::Anchor { id } => {
                self.state.borrow_mut().pending_anchor = Some(id);
                cx.notify();
            }
            LinkTarget::External { url } => self.open_external(url, cx),
            LinkTarget::Mr { number, web_url } => cx.emit(MdEvent::OpenMr { number, web_url }),
            LinkTarget::Commit { sha, web_url } => match web_url {
                Some(url) => self.open_external(url, cx),
                None => cx.emit(MdEvent::ShowCommit { sha }),
            },
            LinkTarget::File { path, commit, anchor } => match &ctx {
                MdContext::File { path: here, .. } if *here == path => {
                    if let Some(a) = anchor {
                        self.state.borrow_mut().pending_anchor = Some(if a.starts_with("user-content-") { a } else { format!("user-content-{a}") });
                        cx.notify();
                    }
                }
                MdContext::File { .. } => cx.emit(MdEvent::OpenFile { path, commit, anchor }),
                MdContext::Forge { .. } => {
                    if let Some(url) = self.browser_url(link) {
                        self.open_external(url, cx);
                    }
                }
            },
        }
    }

    fn toggle_details(&mut self, id: u32, cx: &mut Context<Self>) {
        let mut st = self.state.borrow_mut();
        let Some(d) = st.doc.as_mut() else { return };
        let Some(i) = d.built.tops.iter().position(|t| contains_id(t, id)) else { return };
        toggle_in(&mut d.built.tops[i], id);
        d.tops[i].laid = None;
        d.focusables.clear();
        for t in &d.built.tops {
            walk_focus(t, &mut d.focusables);
        }
        let targets = d.targets.clone();
        let slots = &d.slots;
        d.focusables.retain(|(f, _)| !matches!(f, Focus::Link(l) if targets.get(*l as usize) == Some(&LinkTarget::Inert)) && !matches!(f, Focus::Image(s) if slots.get(*s as usize).is_some_and(|s| s.src.mermaid)));
        drop(st);
        cx.notify();
    }

    /// The document's diagrams, in turn, each by what's cached of it or the worker; the next starts when one's done.
    fn draw_mermaid(&mut self, index: usize, cx: &mut Context<Self>) {
        if cfg!(test) || index >= super::mermaid::MAX_DIAGRAMS {
            return;
        }
        let dark = !theme::applied().light;
        let (source, width, generation, rt, diagrams, cached) = {
            let st = self.state.borrow();
            let Some(d) = st.doc.as_ref() else { return };
            let Some(m) = d.built.mermaids.get(index) else { return };
            let key = (m.source.clone(), dark);
            (m.source.clone(), d.width, st.generation, st.rt.clone(), st.diagrams.clone(), st.drawn.get(&key).cloned())
        };
        if let Some(done) = cached {
            self.show_mermaid(index, done, cx);
            return self.draw_mermaid(index + 1, cx);
        }
        // The column's width isn't known until the first layout: its paint asks again.
        if width <= 0. {
            self.state.borrow_mut().mermaid_waits = true;
            return;
        }
        let svg = cx.svg_renderer();
        let key = (source.clone(), dark);
        let job = rt.spawn_blocking(move || diagrams.draw(&source, dark, width, &svg));
        cx.spawn(async move |this, cx| {
            let result = job.await.unwrap_or_else(|e| Err(e.to_string()));
            this.update(cx, |this, cx| {
                if this.state.borrow().generation != generation {
                    return;
                }
                this.state.borrow_mut().drawn.insert(key, result.clone());
                this.show_mermaid(index, result, cx);
                this.draw_mermaid(index + 1, cx);
            })
            .ok();
        })
        .detach();
    }

    fn show_mermaid(&mut self, index: usize, result: Result<super::mermaid::Drawn, String>, cx: &mut Context<Self>) {
        let mut st = self.state.borrow_mut();
        let Some(d) = st.doc.as_mut() else { return };
        let Some(m) = d.built.mermaids.get(index).cloned() else { return };
        let Some(i) = d.built.tops.iter().position(|t| contains_id(t, m.id)) else { return };
        let (kids, state) = match result {
            Ok(done) => (vec![m.drawn], SlotState::Shown { image: done.image, w: done.w, h: done.h, thumb: None }),
            Err(why) => {
                let mut note = m.drawn;
                note.margin.top = 4.;
                let why = why.split('\n').next().unwrap_or_default().chars().take(200).collect::<String>();
                (vec![m.code, note], SlotState::Broken { why })
            }
        };
        let shown = matches!(state, SlotState::Shown { .. });
        set_kids(&mut d.built.tops[i], m.id, kids);
        if let Some(s) = d.slots.get_mut(m.slot as usize) {
            s.state = state;
            s.asked = true;
            s.top = i;
        }
        d.tops[i].laid = None;
        if d.tops[i].measured {
            d.tops[i].measured = false;
            d.measured -= 1;
        }
        drop(st);
        if shown {
            self.shrink_image(m.slot, cx);
        }
        cx.notify();
    }

    fn open_slot(&mut self, slot: u32, modifiers: Modifiers, window: &mut Window, cx: &mut Context<Self>) {
        let item = {
            let st = self.state.borrow();
            let Some(s) = st.doc.as_ref().and_then(|d| d.slots.get(slot as usize)).filter(|s| !s.src.mermaid) else { return };
            let SlotState::Shown { image, w, h, .. } = &s.state else { return };
            let browser_url = match &s.source {
                ImageSource::Forge { url } | ImageSource::Remote { url, .. } => Some(url.clone()),
                _ => None,
            };
            crate::lightbox::Item { image: image.clone(), w: *w, h: *h, browser_url, api: st.api.clone(), rt: st.rt.clone() }
        };
        if modifiers.control || modifiers.platform {
            if let Some(url) = item.browser_url {
                self.open_external(url, cx);
            }
            return;
        }
        crate::lightbox::open(item, window, cx);
    }

    fn copy_text(text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        crate::ui::toast::show("Copied", cx);
    }

    fn copy_code(&mut self, index: u32, cx: &mut Context<Self>) {
        let text = {
            let st = self.state.borrow();
            let Some(d) = st.doc.as_ref() else { return };
            fn find(n: &BoxNode, index: u32) -> Option<String> {
                match &n.kind {
                    BoxKind::Code(c) if c.index == index => Some(c.copied.clone()),
                    BoxKind::Flow(k) => k.iter().find_map(|x| find(x, index)),
                    BoxKind::Details { summary, body, .. } => find(summary, index).or_else(|| body.iter().find_map(|x| find(x, index))),
                    BoxKind::Table(t) => t.rows.iter().flat_map(|r| &r.cells).find_map(|c| find(&c.content, index)),
                    BoxKind::Columns { cols, .. } => cols.iter().find_map(|x| find(x, index)),
                    _ => None,
                }
            }
            d.built.tops.iter().find_map(|t| find(t, index))
        };
        if let Some(t) = text {
            Self::copy_text(t, cx);
        }
    }

    fn on_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let hit = {
            let st = self.state.borrow();
            let (bx, by) = (st.bounds.origin.x, st.bounds.origin.y);
            let _ = by;
            let w = st.bounds.size.width.as_f32();
            let x = (e.position.x - bx).as_f32();
            if st.opts.pane && !st.opts.ruled && x >= w - BAR && st.max_scroll() > 0. {
                drop(st);
                self.pane_bar_press(e.position.y, cx);
                return;
            }
            st.hit(e.position)
        };
        let mut st = self.state.borrow_mut();
        st.press_hit = Some(hit.clone());
        st.press_at = e.position;
        st.focused = None;
        if let Some((id, scroll, track, content)) = hit.bar {
            let ratio = (content - track).max(1.) / (track - (track * track / content.max(1.)).max(24.)).max(1.);
            st.drag = Drag::Bar { id, start_x: e.position.x.as_f32(), start: scroll, ratio };
            st.press_hit = None;
            drop(st);
            cx.notify();
            return;
        }
        if hit.copy.is_some() || hit.load {
            drop(st);
            cx.notify();
            return;
        }
        if let Some(at) = hit.offset {
            let stream_len = st.doc.as_ref().map_or(0, |d| d.built.stream.len());
            let selection = match e.click_count {
                2 => st.doc.as_ref().map(|d| MdState::word_at(&d.built.stream, at)),
                n if n >= 3 => st.doc.as_ref().map(|d| {
                    let s = &d.built.stream;
                    let from = s[..at.min(stream_len)].rfind('\n').map_or(0, |k| k + 1);
                    let to = s[at.min(stream_len)..].find('\n').map_or(stream_len, |k| at + k);
                    (from, to)
                }),
                _ if e.modifiers.shift => Some((st.selection.map_or(at, |s| s.0), at)),
                _ => Some((at, at)),
            };
            st.selection = selection;
            st.drag = Drag::Text;
        } else {
            st.selection = None;
        }
        drop(st);
        cx.notify();
    }

    fn pane_bar_press(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let mut st = self.state.borrow_mut();
        let (h, total) = (st.view_h, st.max_scroll() + st.view_h);
        let thumb = (h * h / total.max(1.)).max(24.);
        let top = (h - thumb) * (st.scroll / st.max_scroll().max(1.));
        let at = (y - st.bounds.origin.y).as_f32();
        let grab = if at >= top && at < top + thumb { at - top } else { thumb / 2. };
        st.drag = Drag::Pane { grab };
        let share = ((at - grab) / (h - thumb).max(1.)).clamp(0., 1.);
        st.scroll = share * st.max_scroll();
        drop(st);
        cx.notify();
    }

    fn on_up(&mut self, e: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (hit, moved, collapsed) = {
            let mut st = self.state.borrow_mut();
            st.drag = Drag::None;
            let moved = (e.position.x - st.press_at.x).abs() > px(3.) || (e.position.y - st.press_at.y).abs() > px(3.);
            let collapsed = st.selection.is_none_or(|(a, b)| a == b);
            (st.press_hit.take(), moved, collapsed)
        };
        let Some(hit) = hit else {
            cx.notify();
            return;
        };
        if moved || !collapsed {
            cx.notify();
            return;
        }
        if let Some(index) = hit.copy {
            self.copy_code(index, cx);
        } else if hit.load {
            if let Some(slot) = hit.slot {
                let mut st = self.state.borrow_mut();
                let url = st.doc.as_ref().and_then(|d| d.slots.get(slot as usize)).and_then(|s| match &s.source {
                    ImageSource::Remote { url, .. } | ImageSource::Forge { url } => Some(url.clone()),
                    _ => None,
                });
                if let Some(u) = url {
                    st.consented.push(u);
                }
                drop(st);
                self.set_slot(slot, SlotState::Loading, cx);
                self.load_image(slot, true, cx);
            }
        } else if let Some(link) = hit.link {
            self.activate(link, e.modifiers, cx);
        } else if let Some(slot) = hit.slot {
            self.open_slot(slot, e.modifiers, window, cx);
        } else if let Some(id) = hit.summary {
            self.toggle_details(id, cx);
        }
        cx.notify();
    }

    fn on_move(&mut self, e: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let mut st = self.state.borrow_mut();
        let pressed = e.pressed_button == Some(MouseButton::Left);
        match st.drag {
            Drag::Text if pressed => {
                let (b, y) = (st.bounds, e.position.y);
                if st.opts.pane {
                    let above = (b.origin.y - y).as_f32();
                    let below = (y - b.origin.y - b.size.height).as_f32();
                    if above > 0. {
                        st.scroll_by(-above.min(54.));
                    } else if below > 0. {
                        st.scroll_by(below.min(54.));
                    }
                }
                if let Some(at) = st.hit(e.position).offset
                    && let Some(sel) = st.selection.as_mut()
                {
                    sel.1 = at;
                }
                drop(st);
                cx.notify();
                return;
            }
            Drag::Pane { grab } if pressed => {
                let at = (e.position.y - st.bounds.origin.y).as_f32();
                let (h, total) = (st.view_h, st.max_scroll() + st.view_h);
                let thumb = (h * h / total.max(1.)).max(24.);
                let share = ((at - grab) / (h - thumb).max(1.)).clamp(0., 1.);
                st.scroll = share * st.max_scroll();
                drop(st);
                cx.notify();
                return;
            }
            Drag::Bar { id, start_x, start, ratio } if pressed => {
                st.hscroll.insert(id, (start + (e.position.x.as_f32() - start_x) * ratio).max(0.));
                drop(st);
                cx.notify();
                return;
            }
            _ => {}
        }
        let inside = st.bounds.contains(&e.position);
        let hit = if inside { st.hit(e.position) } else { Hit::default() };
        let (link, code, copy, load) = (hit.link.filter(|l| st.doc.as_ref().is_some_and(|d| d.targets.get(*l as usize).is_some_and(|t| *t != LinkTarget::Inert))), hit.code, hit.copy, hit.load.then_some(hit.slot).flatten());
        let changed = link != st.hover_link || code != st.hover_code || copy != st.hover_copy || load != st.hover_load;
        let link_changed = link != st.hover_link;
        st.hover_link = link;
        st.hover_code = code;
        st.hover_copy = copy;
        st.hover_load = load;
        st.cursor = if link.is_some() || copy.is_some() || load.is_some() || hit.summary.is_some() {
            CursorStyle::PointingHand
        } else if hit.slot.is_some() {
            CursorStyle::Arrow
        } else if hit.text {
            CursorStyle::IBeam
        } else {
            CursorStyle::Arrow
        };
        let tip = link.and_then(|l| {
            let d = st.doc.as_ref()?;
            let old = matches!(d.built.links.get(l as usize)?, LinkSrc::Href { old_side: true, .. } | LinkSrc::Ref { old_side: true, .. });
            link_tooltip(&st.opts.env, st.link_env_side(old), d.targets.get(l as usize)?)
        });
        let broken = hit.slot.and_then(|s| st.doc.as_ref()?.slots.get(s as usize)).and_then(|s| if let SlotState::Broken { why } = &s.state { Some(why.clone()) } else { None });
        let mention = link.and_then(|l| match st.doc.as_ref()?.built.links.get(l as usize)? {
            LinkSrc::Ref { node, .. } => node.user.clone().zip(st.opts.env.users.clone()),
            _ => None,
        });
        let core = (st.api.clone(), st.rt.clone());
        drop(st);
        if link_changed && let Some((user, env)) = mention {
            crate::forge::card::show_user_card(env, user, Bounds::new(e.position, size(px(1.), px(14.))), core.0, core.1, cx);
        } else if link_changed {
            match tip {
                Some(text) => tooltip::show_at(text, Bounds::new(e.position, size(px(1.), px(14.))), tooltip::Placement::Below, cx),
                None => tooltip::hide(cx),
            }
        }
        if let Some(why) = broken {
            tooltip::show_at(why, Bounds::new(e.position, size(px(1.), px(14.))), tooltip::Placement::Below, cx);
        }
        if changed {
            cx.notify();
        }
    }

    fn on_wheel(&mut self, e: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        if e.modifiers.control {
            cx.emit(MdEvent::Zoom(if e.delta.pixel_delta(window.line_height()).y.as_f32() > 0. { 1 } else { -1 }));
            cx.stop_propagation();
            return;
        }
        let delta = e.delta.pixel_delta(window.line_height());
        let (dx, dy) = if e.modifiers.shift { (delta.y, px(0.)) } else { (delta.x, delta.y) };
        let mut st = self.state.borrow_mut();
        if dx.as_f32() != 0.
            && let Some(block) = st.hit(e.position).hblock
        {
            let now = st.hscroll.get(&block.0).copied().unwrap_or(0.);
            st.hscroll.insert(block.0, (now - dx.as_f32()).clamp(0., block.1));
            drop(st);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if st.opts.pane {
            st.scroll_by(-dy.as_f32());
            drop(st);
            cx.stop_propagation();
            cx.notify();
        }
    }

    /// Tab and Shift+Tab, among what's focusable; Enter or Space on it.
    pub fn move_focus(&mut self, back: bool, cx: &mut Context<Self>) -> bool {
        let mut st = self.state.borrow_mut();
        let Some(d) = st.doc.as_ref() else { return false };
        if d.focusables.is_empty() {
            return false;
        }
        let at = st.focused.and_then(|f| d.focusables.iter().position(|(x, _)| *x == f));
        let next = match (at, back) {
            (None, false) => 0,
            (None, true) => d.focusables.len() - 1,
            (Some(i), false) if i + 1 < d.focusables.len() => i + 1,
            (Some(i), true) if i > 0 => i - 1,
            _ => {
                st.focused = None;
                drop(st);
                cx.notify();
                return false;
            }
        };
        let (f, offset) = d.focusables[next];
        st.focused = Some(f);
        if let Some(d) = st.doc.as_ref()
            && let Some((_, y)) = MdState::y_of_offset(d, offset)
        {
            let (scroll, h, pad) = (st.scroll, st.view_h, st.pad_top());
            if y + pad < scroll || y + pad + 40. > scroll + h {
                st.scroll = (y + pad - h / 3.).clamp(0., st.max_scroll());
            }
        }
        drop(st);
        cx.notify();
        true
    }

    pub fn activate_focused(&mut self, modifiers: Modifiers, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let f = self.state.borrow().focused;
        match f {
            Some(Focus::Link(l)) => self.activate(l, modifiers, cx),
            Some(Focus::Image(s)) => self.open_slot(s, modifiers, window, cx),
            Some(Focus::Summary(id)) => self.toggle_details(id, cx),
            Some(Focus::Copy(i)) => self.copy_code(i, cx),
            None => return false,
        }
        true
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let plain = !k.modifiers.control && !k.modifiers.alt && !k.modifiers.platform;
        match k.key.as_str() {
            "tab" if plain => {
                if self.move_focus(k.modifiers.shift, cx) {
                    cx.stop_propagation();
                }
            }
            "enter" | "space" if plain && !k.modifiers.shift => {
                if self.activate_focused(k.modifiers, window, cx) {
                    cx.stop_propagation();
                }
            }
            "home" | "end" if plain && self.state.borrow().opts.pane => {
                let mut st = self.state.borrow_mut();
                st.scroll = if k.key == "home" { 0. } else { st.max_scroll() };
                drop(st);
                cx.stop_propagation();
                cx.notify();
            }
            _ => {}
        }
    }

    fn on_right(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let entity = cx.entity();
        let (hit, selected) = {
            let st = self.state.borrow();
            (st.hit(e.position), st.selected_text())
        };
        let mut rows = Vec::new();
        if let Some(l) = hit.link {
            rows = self.link_menu(l, &hit, cx);
        }
        if let Some(text) = selected {
            if !rows.is_empty() {
                rows.push(MenuRow::Separator);
            }
            rows.push(MenuRow::action("md.copySelection", "Copy", "copy", "Copy the selected text", move |_, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
            }));
        }
        let _ = entity;
        if !rows.is_empty() {
            menu::open_context(rows, e.position, window, cx);
            cx.stop_propagation();
        }
    }

    /// The `link` menu (`linkMenu.ts`).
    fn link_menu(&self, link: u32, hit: &Hit, cx: &mut Context<Self>) -> Vec<MenuRow> {
        let entity = cx.entity();
        let st = self.state.borrow();
        let Some(d) = st.doc.as_ref() else { return Vec::new() };
        let Some(target) = d.targets.get(link as usize).cloned() else { return Vec::new() };
        let src = d.built.links.get(link as usize);
        let old = matches!(src, Some(LinkSrc::Href { old_side: true, .. } | LinkSrc::Ref { old_side: true, .. }));
        let ctx = st.link_env_side(old).clone();
        let in_app = match &target {
            LinkTarget::Mr { .. } => true,
            LinkTarget::Commit { web_url, .. } => web_url.is_none(),
            LinkTarget::File { .. } => matches!(ctx, MdContext::File { .. }),
            _ => false,
        };
        let mut open = Vec::new();
        if in_app {
            let icon = match &target {
                LinkTarget::Commit { .. } => "git-commit-horizontal",
                LinkTarget::File { .. } => "file-text",
                _ => "git-pull-request",
            };
            let tip = link_tooltip(&st.opts.env, &ctx, &target).unwrap_or_else(|| "Open in GitVolt".into());
            let e = entity.clone();
            open.push(MenuRow::action("link.openInApp", "Open in GitVolt", icon, tip, move |_, cx| {
                e.update(cx, |v, cx| v.activate(link, Modifiers::default(), cx));
            }));
        }
        let url = browser_url_for(&st.opts.env, &ctx, &target);
        if let Some(u) = url.clone() {
            let e = entity.clone();
            let u2 = u.clone();
            open.push(MenuRow::action("link.openInBrowser", "Open in browser", "globe", u, move |_, cx| {
                let u = u2.clone();
                e.update(cx, |v, cx| v.open_external(u, cx));
            }));
        }
        let href = match src {
            Some(LinkSrc::Href { href, .. }) => href.trim().to_string(),
            _ => String::new(),
        };
        let link_text = url.or(Some(href).filter(|h| !h.is_empty()));
        let shown = hit.offset.map_or(String::new(), |at| link_text_at(d, link, at));
        let mut copy = Vec::new();
        if let Some(l) = link_text {
            let tip = format!("Copy {l}");
            copy.push(MenuRow::action("link.copyLink", "Copy link", "link-2", tip, move |_, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(l.clone()));
                crate::ui::toast::show("Copied", cx);
            }));
        }
        let tip = format!("Copy “{shown}”");
        copy.push(MenuRow::action("link.copyText", "Copy text", "copy", tip, move |_, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(shown.clone()));
            crate::ui::toast::show("Copied", cx);
        }));
        menu::grouped(vec![open, copy])
    }
}

/// A link's whole text: its items, around where it was pressed.
fn link_text_at(d: &Doc, link: u32, at: usize) -> String {
    fn walk(n: &BoxNode, link: u32, stream: &str, out: &mut String) {
        match &n.kind {
            BoxKind::Inline(ib) => {
                for it in &ib.items {
                    if let Item::Text { range, style } = it
                        && style.link == Some(link)
                    {
                        out.push_str(&stream[range.clone()]);
                    }
                }
            }
            BoxKind::Flow(k) => k.iter().for_each(|x| walk(x, link, stream, out)),
            BoxKind::Details { summary, body, .. } => {
                walk(summary, link, stream, out);
                body.iter().for_each(|x| walk(x, link, stream, out));
            }
            BoxKind::Table(t) => t.rows.iter().flat_map(|r| &r.cells).for_each(|c| walk(&c.content, link, stream, out)),
            BoxKind::Columns { cols, .. } => cols.iter().for_each(|x| walk(x, link, stream, out)),
            _ => {}
        }
    }
    let i = d.built.tops.partition_point(|t| t.start <= at).saturating_sub(1);
    let mut out = String::new();
    if let Some(t) = d.built.tops.get(i) {
        walk(t, link, &d.built.stream, &mut out);
    }
    out
}

enum FetchedImage {
    Image(crate::diff::image::Decoded),
    Ask(String),
}

/// The element: lays out in its prepaint, as tall as its content when it isn't a pane.
struct MdElement {
    view: Entity<MdView>,
    state: Rc<RefCell<MdState>>,
}

impl IntoElement for MdElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for MdElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let pane = self.state.borrow().opts.pane;
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if pane {
            style.size.height = relative(1.).into();
            return (window.request_layout(style, [], cx), ());
        }
        let state = self.state.clone();
        let id = window.request_measured_layout(style, move |known, available, window, _| {
            let width = known.width.unwrap_or(match available.width {
                AvailableSpace::Definite(w) => w,
                _ => px(600.),
            });
            let mut st = state.borrow_mut();
            let b = Bounds::new(point(px(0.), px(0.)), size(width, px(0.)));
            st.ensure(b, window);
            size(width, px(st.content_h()))
        });
        (id, ())
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) {
        let (more, marks_stale) = {
            let mut st = self.state.borrow_mut();
            let more = st.ensure(bounds, window);
            let settled = st.doc.as_ref().is_some_and(|d| d.measured == d.tops.len());
            (more, settled)
        };
        let _ = marks_stale;
        let diagrams = {
            let mut st = self.state.borrow_mut();
            let ready = st.doc.as_ref().is_some_and(|d| d.width > 0.);
            ready && std::mem::take(&mut st.mermaid_waits)
        };
        if diagrams {
            self.view.update(cx, |v, cx| v.draw_mermaid(0, cx));
        }
        let want = !self.state.borrow().pending_load.is_empty();
        if want {
            self.view.update(cx, |v, cx| v.load_images(cx));
        }
        if more {
            // The rest's heights, a slice a frame, until they're known: it ends by itself.
            let view = self.view.clone();
            window.on_next_frame(move |_, cx| view.update(cx, |_, cx| cx.notify()));
        }
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), _: &mut (), window: &mut Window, cx: &mut App) {
        let st = self.state.borrow();
        let Some(d) = st.doc.as_ref() else { return };
        let (left, _) = st.column(bounds.size.width.as_f32());
        let origin_x = bounds.origin.x.as_f32() + left;
        let origin_y = bounds.origin.y.as_f32() + st.pad_top() - st.scroll;
        let mask = if st.opts.pane { bounds.intersect(&window.content_mask().bounds) } else { window.content_mask().bounds };
        let band = (mask.origin.y.as_f32() - 8., (mask.origin.y + mask.size.height).as_f32() + 8.);
        let pc = PaintCx { st: &st, d, band, origin: bounds.origin, selection: st.selection.map(|(a, b)| (a.min(b), a.max(b))).filter(|(a, b)| a != b), editor_fg: theme::editor().foreground };
        let _ = pc.origin;
        window.with_content_mask(Some(ContentMask { bounds: mask }), |window| {
            if d.tops.is_empty() {
                return;
            }
            let first = MdState::top_at(d, band.0 - origin_y);
            for i in first..d.tops.len() {
                let y = origin_y + d.offsets[i];
                if y > band.1 {
                    break;
                }
                if let Some(laid) = d.tops[i].laid.as_ref() {
                    pc.paint_node(&d.built.tops[i], laid, origin_x, y, window, cx);
                }
            }
            if st.opts.pane && !st.opts.ruled && st.max_scroll() > 0. {
                let (h, total) = (st.view_h, st.max_scroll() + st.view_h);
                let thumb = (h * h / total.max(1.)).max(24.);
                let top = (h - thumb) * (st.scroll / st.max_scroll().max(1.));
                let x = (bounds.origin.x + bounds.size.width).as_f32() - BAR + 2.;
                rq(window, x, bounds.origin.y.as_f32() + top + 2., BAR - 4., thumb - 4., 3., if st.bar_hover { theme::scroll_thumb_hover() } else { theme::scroll_thumb() });
            }
            if bounds.contains(&window.mouse_position()) {
                window.set_window_cursor_style(st.cursor);
            }
        });
    }
}

impl Render for MdView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A theme switch: the code's colours again, by the new editor theme.
        let stale = {
            let mut st = self.state.borrow_mut();
            let stale = st.theme_version != theme::version();
            st.theme_version = theme::version();
            stale
        };
        if stale {
            self.colour(cx);
            self.draw_mermaid(0, cx);
        }
        let (pane, inset) = (self.state.borrow().opts.pane, self.state.borrow().opts.inset.is_some());
        div()
            .id("md-view")
            .key_context("Markdown")
            .track_focus(&self.focus)
            .relative()
            .w_full()
            .when(pane, |d| d.h_full().flex_1().min_h_0().min_w_0().overflow_hidden())
            .when(pane && !inset, |d| d.bg(theme::app_bg()))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right))
            .on_mouse_move(cx.listener(Self::on_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_up))
            .on_scroll(cx.listener(Self::on_wheel))
            .on_key_down(cx.listener(Self::on_key))
            .child(MdElement { view: cx.entity(), state: self.state.clone() })
    }
}

impl Drop for MdState {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(d) = self.doc.as_ref() {
            let mut retired = self.retired.borrow_mut();
            for s in &d.slots {
                if let SlotState::Shown { image, thumb, .. } = &s.state {
                    retired.push(image.clone());
                    retired.extend(thumb.clone());
                }
            }
        }
    }
}
