//! The sanitized tree as boxes: `markdown.css`, the browser's own sheet for what it leaves to it, and what `render.tsx`
//! maps to components, as plain data. No colours (tokens, read when painting) and no shaping: this runs off the UI thread.

use super::ast::{Align, Mark, RefNode};
use super::links::is_video_src;
use super::safe::{Dom, El, Special, suggestion_lang};
use crate::theme;
use gpui::Hsla;
use std::collections::HashMap;
use std::ops::Range;

/// The longest block coloured (`HIGHLIGHT_MAX_CHARS`).
pub const HIGHLIGHT_MAX_CHARS: usize = 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    TextNormal,
    TextSelected,
    TextSecondary,
    Link,
    SectionBorder,
    PanelBg1,
    AppBg,
    ControlBg,
    InsBlock,
    DelBlock,
    Red,
}

impl Tok {
    pub fn color(self) -> Hsla {
        match self {
            Tok::TextNormal => theme::text_normal(),
            Tok::TextSelected => theme::text_selected(),
            Tok::TextSecondary => theme::text_secondary(),
            Tok::Link => theme::link(),
            Tok::SectionBorder => theme::section_border(),
            Tok::PanelBg1 => theme::panel_bg1(),
            Tok::AppBg => theme::app_bg(),
            Tok::ControlBg => theme::control_bg(),
            Tok::InsBlock => theme::md_diff_ins_block(),
            Tok::DelBlock => theme::md_diff_del_block(),
            Tok::Red => theme::red(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    /// `--reading-font`, else the UI's.
    Reading,
    /// `--md-code-font`: the editor's in a pane, else the app's monospace.
    Code,
    /// `--font-mono`.
    Mono,
    Ui,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineH {
    /// A multiple of the size.
    Num(f32),
    /// The font's own.
    Normal,
    Zero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VAlign {
    Base,
    Super,
    Sub,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub family: Family,
    pub size: f32,
    pub weight: u16,
    pub italic: bool,
    pub color: Tok,
    pub line: LineH,
    pub valign: VAlign,
    /// An index into the document's links.
    pub link: Option<u32>,
    pub strike: bool,
    pub underline: bool,
    /// Inline code's ground and its room.
    pub code: bool,
    /// A rendered diff's changed words: added, or removed.
    pub word: Option<bool>,
    /// An added or removed block's tint over its code's ground.
    pub tint: Option<Tok>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Text { range: Range<usize>, style: TextStyle },
    /// Inline padding; `end`: it closes what's before it.
    Pad { px: f32, end: bool, style: TextStyle },
    Image { slot: u32, style: TextStyle },
    Kbd { range: Range<usize>, style: TextStyle },
    Task { checked: bool, style: TextStyle },
    Break,
}

#[derive(Debug, Clone)]
pub struct InlineBlock {
    pub items: Vec<Item>,
    pub align: Align,
    pub strut: TextStyle,
    pub nowrap: bool,
    /// Where its text starts and ends in the stream.
    pub range: Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MarkerKind {
    Disc,
    Circle,
    Square,
}

#[derive(Debug, Clone)]
pub enum Marker {
    Bullet(MarkerKind),
    Number(String),
}

#[derive(Debug, Clone)]
pub struct CodeBox {
    /// The document's code-block index: where its colours land.
    pub index: u32,
    pub lang: Option<String>,
    pub lines: Vec<Range<usize>>,
    /// One of ' ', '-', '+' per line.
    pub marks: Option<Vec<u8>>,
    /// Per line, its changed words.
    pub words: Option<Vec<Vec<(usize, usize)>>>,
    pub signs: bool,
    pub style: TextStyle,
    /// The Copy control's text.
    pub copied: String,
    pub copy_button: bool,
}

#[derive(Debug, Clone)]
pub struct CellBox {
    pub content: BoxNode,
    pub align: Align,
    pub top: bool,
    pub span: u32,
    pub nowrap: bool,
    pub th: bool,
}

#[derive(Debug, Clone)]
pub struct RowBox {
    pub cells: Vec<CellBox>,
    pub bg: Option<Tok>,
    pub mark: Option<Mark>,
    pub empty: bool,
}

#[derive(Debug, Clone)]
pub struct TableBox {
    pub caption: Option<Box<BoxNode>>,
    pub rows: Vec<RowBox>,
    pub full: bool,
    pub bg: Option<Tok>,
}

#[derive(Debug, Clone)]
pub enum BoxKind {
    Flow(Vec<BoxNode>),
    Inline(InlineBlock),
    Code(CodeBox),
    Table(TableBox),
    Rule,
    Details { summary: Box<BoxNode>, body: Vec<BoxNode>, open: bool },
    /// A split row's or a changed diagram's two columns.
    Columns { cols: Vec<BoxNode>, gap: f32 },
    /// A block only the other side has: hatched.
    Hatch,
    /// A note under a mermaid block and the like: nothing to lay out but its text.
    Empty,
}

#[derive(Debug, Clone, Default)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Edges {
    pub fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self { top, right, bottom, left }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BarAt {
    /// A block's bar, in the margin.
    Block,
    /// A list item's, left of its marker.
    Item,
}

#[derive(Debug, Clone)]
pub struct BoxNode {
    pub id: u32,
    pub margin: Edges,
    pub padding: Edges,
    pub border: Edges,
    pub border_tok: Tok,
    pub bg: Option<Tok>,
    pub radius: f32,
    pub kind: BoxKind,
    pub marker: Option<Marker>,
    pub marker_style: Option<TextStyle>,
    /// A rendered diff's mark: the bar, and the tint unless `tinted` is off.
    pub mark: Option<Mark>,
    pub bar: Option<BarAt>,
    pub tinted: bool,
    /// The list's indent, for an item's bar.
    pub list_indent: f32,
    /// Where it starts in the stream.
    pub start: usize,
}

impl BoxNode {
    fn new(id: u32, kind: BoxKind, start: usize) -> Self {
        Self { id, margin: Edges::default(), padding: Edges::default(), border: Edges::default(), border_tok: Tok::SectionBorder, bg: None, radius: 0., kind, marker: None, marker_style: None, mark: None, bar: None, tinted: false, list_indent: 0., start }
    }
}

/// A link, as the views ask for its target (`links.rs`) on the UI thread.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkSrc {
    Href { href: String, title: Option<String>, old_side: bool },
    Ref { node: RefNode, old_side: bool },
}

#[derive(Debug, Clone)]
pub struct ImageSrc {
    pub src: String,
    pub alt: String,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub in_link: bool,
    pub video: bool,
    pub old_side: bool,
    /// A changed word's picture: what it's outlined in, added or removed.
    pub word: Option<bool>,
}

/// What a body is shown with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    /// `--md-font-size`: the pane's; elsewhere the reading size less 1.
    pub font_px: f32,
    pub line_height: f32,
    /// Its code in the editor's font: File View's and the diff's panes.
    pub editor_code: bool,
}

impl Default for Params {
    fn default() -> Self {
        Self { font_px: 12., line_height: 1.55, editor_code: false }
    }
}

/// A body's boxes and what they point at.
#[derive(Debug, Default)]
pub struct Built {
    pub tops: Vec<BoxNode>,
    /// All its text, in document order: what a selection takes.
    pub stream: String,
    pub links: Vec<LinkSrc>,
    pub images: Vec<ImageSrc>,
    /// A slot's or a block's id and the stream's offset an anchor goes to.
    pub anchors: HashMap<String, usize>,
    /// The fenced blocks, as colouring asks for them: language, stream range.
    pub codes: Vec<(String, Range<usize>)>,
    pub next_id: u32,
}

struct Build {
    out: Built,
    params: Params,
    /// What a suggestion replaces, when the caller knows it.
    base: Option<super::SuggestionBase>,
}

#[derive(Clone, Copy)]
struct Ctx {
    style: TextStyle,
    align: Align,
    list_depth: u32,
    pre: bool,
    pre_line: bool,
    nowrap: bool,
    old_side: bool,
    in_link: bool,
    /// Block tint for code inside an added or removed block.
    tint: Option<Tok>,
}

struct Flow {
    boxes: Vec<BoxNode>,
    run: Vec<Item>,
    /// The last thing in the run ended in a collapsible space.
    spaced: bool,
    start: usize,
}

impl Flow {
    fn new(start: usize) -> Self {
        Self { boxes: Vec::new(), run: Vec::new(), spaced: true, start }
    }
}

fn is_block(tag: &str) -> bool {
    matches!(tag, "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "li" | "blockquote" | "pre" | "hr" | "table" | "details" | "summary" | "section" | "dl" | "dt" | "dd" | "caption" | "tr" | "td" | "th" | "thead" | "tbody" | "tfoot")
}

impl Build {
    fn id(&mut self) -> u32 {
        self.out.next_id += 1;
        self.out.next_id
    }

    fn sep(&mut self, newlines: usize) {
        let s = &mut self.out.stream;
        if s.is_empty() {
            return;
        }
        let have = s.bytes().rev().take_while(|b| *b == b'\n').count();
        for _ in have..newlines {
            s.push('\n');
        }
    }

    fn push_text(&mut self, flow: &mut Flow, text: &str, ctx: &Ctx) {
        let start = self.out.stream.len();
        if ctx.pre {
            self.out.stream.push_str(text);
        } else if ctx.pre_line {
            // Lines stay; spaces collapse.
            let mut first = true;
            for line in text.split('\n') {
                if !first {
                    let at = self.out.stream.len();
                    self.out.stream.push('\n');
                    if at > start {
                        flow.run.push(Item::Text { range: start..at, style: ctx.style });
                    }
                    flow.run.push(Item::Break);
                }
                first = false;
                let mut prev_space = false;
                for c in line.chars() {
                    if c == ' ' || c == '\t' {
                        if !prev_space {
                            self.out.stream.push(' ');
                        }
                        prev_space = true;
                    } else {
                        self.out.stream.push(c);
                        prev_space = false;
                    }
                }
            }
        } else {
            let mut spaced = flow.spaced;
            for c in text.chars() {
                if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}') {
                    if !spaced {
                        self.out.stream.push(' ');
                    }
                    spaced = true;
                } else {
                    self.out.stream.push(c);
                    spaced = false;
                }
            }
            flow.spaced = spaced;
        }
        let end = self.out.stream.len();
        if end > start {
            // A run after a pre-line's break starts past the last line's newline.
            let from = if ctx.pre_line { self.out.stream[..end].rfind('\n').filter(|n| *n >= start).map_or(start, |n| n + 1) } else { start };
            if end > from {
                flow.run.push(Item::Text { range: from..end, style: ctx.style });
            }
        }
    }

    /// Ends the inline run: an anonymous block, unless it's nothing but a space.
    fn flush(&mut self, flow: &mut Flow, ctx: &Ctx) {
        // A trailing collapsible space goes.
        while let Some(Item::Text { range, .. }) = flow.run.last_mut() {
            if !ctx.pre && self.out.stream[range.clone()].ends_with(' ') {
                range.end -= 1;
                self.out.stream.truncate(range.end);
                if range.start >= range.end {
                    flow.run.pop();
                    continue;
                }
            }
            break;
        }
        if flow.run.iter().all(|i| matches!(i, Item::Pad { .. })) {
            flow.run.clear();
            flow.spaced = true;
            flow.start = self.out.stream.len();
            return;
        }
        let items = std::mem::take(&mut flow.run);
        let end = self.out.stream.len();
        let id = self.id();
        let start = items.iter().find_map(|i| match i {
            Item::Text { range, .. } | Item::Kbd { range, .. } => Some(range.start),
            _ => None,
        });
        let ib = InlineBlock { items, align: ctx.align, strut: root_strut(ctx), nowrap: ctx.nowrap, range: start.unwrap_or(end)..end };
        flow.boxes.push(BoxNode::new(id, BoxKind::Inline(ib), start.unwrap_or(end)));
        flow.spaced = true;
        flow.start = end;
    }

    fn anchor(&mut self, el: &El) {
        for name in ["id", "name"] {
            if let Some(v) = el.attr(name) {
                let at = self.out.stream.len();
                self.out.anchors.entry(v.to_string()).or_insert(at);
            }
        }
    }

    fn walk(&mut self, kids: &[Dom], ctx: &Ctx, flow: &mut Flow) {
        for k in kids {
            match k {
                Dom::Text(t) => self.push_text(flow, t, ctx),
                Dom::El(el) => self.element(el, ctx, flow),
            }
        }
    }

    fn link_index(&mut self, src: LinkSrc) -> u32 {
        self.out.links.push(src);
        self.out.links.len() as u32 - 1
    }

    fn element(&mut self, el: &El, ctx: &Ctx, flow: &mut Flow) {
        self.anchor(el);
        if let Some(s) = el.special.as_deref() {
            match s {
                Special::Reference(r) => {
                    let mut c = *ctx;
                    c.style.link = Some(self.link_index(LinkSrc::Ref { node: r.clone(), old_side: ctx.old_side }));
                    c.style.weight = 500;
                    self.push_text(flow, &r.value, &c);
                    return;
                }
                Special::Task(checked) => {
                    flow.run.push(Item::Pad { px: -1.4 * ctx.style.size, end: false, style: ctx.style });
                    flow.run.push(Item::Task { checked: *checked, style: ctx.style });
                    flow.run.push(Item::Pad { px: 6., end: true, style: ctx.style });
                    flow.spaced = false;
                    return;
                }
                Special::DiffIns | Special::DiffDel => {
                    let ins = matches!(s, Special::DiffIns);
                    let mut c = *ctx;
                    c.style.word = Some(ins);
                    c.style.color = Tok::TextSelected;
                    c.old_side = c.old_side || !ins;
                    c.style.strike = !ins || c.style.strike;
                    self.walk(&el.kids, &c, flow);
                    return;
                }
                _ => {
                    self.flush(flow, ctx);
                    if let Some(node) = self.special_block(el, s, ctx) {
                        flow.boxes.extend(node);
                    }
                    flow.start = self.out.stream.len();
                    return;
                }
            }
        }
        let tag = el.tag.as_str();
        if is_block(tag) {
            self.flush(flow, ctx);
            let node = self.block(el, ctx);
            flow.boxes.extend(node);
            flow.start = self.out.stream.len();
            return;
        }
        let mut c = *ctx;
        match tag {
            "strong" | "b" => c.style.weight = if c.style.weight >= 600 { 900 } else { 700 },
            "em" | "i" | "var" => c.style.italic = true,
            "del" => {
                c.style.strike = true;
                c.style.color = Tok::TextSecondary;
            }
            "s" | "strike" => c.style.strike = true,
            "ins" => c.style.underline = true,
            "code" | "tt" | "samp" if tag == "code" => {
                c.style.family = Family::Code;
                c.style.size *= 0.92;
                c.style.line = LineH::Normal;
                c.style.code = true;
                c.style.tint = ctx.tint;
                c.style.weight = 400;
                c.style.italic = false;
                let pad = 0.35 * c.style.size;
                flow.run.push(Item::Pad { px: pad, end: false, style: c.style });
                self.walk(&el.kids, &c, flow);
                flow.run.push(Item::Pad { px: pad, end: true, style: c.style });
                flow.spaced = false;
                return;
            }
            "tt" | "samp" => {
                c.style.family = Family::Mono;
                c.style.size *= 13. / 16.;
            }
            "kbd" => {
                let mut kc = c;
                kc.style.family = Family::Mono;
                kc.style.size *= 0.85;
                kc.style.line = LineH::Normal;
                kc.style.weight = 400;
                kc.style.italic = false;
                let start = self.out.stream.len();
                let text = dom_text(&el.kids);
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                self.out.stream.push_str(&text);
                flow.run.push(Item::Kbd { range: start..self.out.stream.len(), style: kc.style });
                flow.spaced = false;
                return;
            }
            "q" => {
                self.push_text(flow, "\u{201c}", &c);
                self.walk(&el.kids, &c, flow);
                self.push_text(flow, "\u{201d}", &c);
                return;
            }
            "sub" | "sup" => {
                c.style.size /= 1.2;
                c.style.line = LineH::Zero;
                c.style.valign = if tag == "sup" { VAlign::Super } else { VAlign::Sub };
            }
            "rp" => return,
            "rt" => c.style.size /= 2.,
            "a" => {
                if let Some(href) = el.attr("href") {
                    let src = LinkSrc::Href { href: href.to_string(), title: el.attr("title").map(str::to_string), old_side: ctx.old_side };
                    c.style.link = Some(self.link_index(src));
                    c.in_link = true;
                }
            }
            "img" => {
                self.image(el, ctx, flow);
                return;
            }
            "br" => {
                flow.run.push(Item::Break);
                self.out.stream.push('\n');
                flow.spaced = true;
                return;
            }
            "source" => return,
            _ => {}
        }
        self.walk(&el.kids, &c, flow);
    }

    fn image(&mut self, el: &El, ctx: &Ctx, flow: &mut Flow) {
        let size = |name: &str| el.attr(name).and_then(|v| v.parse::<f32>().ok()).filter(|n| *n > 0. && *n <= 10_000.);
        let src = el.attr("src").unwrap_or_default().to_string();
        let video = is_video_src(&src);
        let word = ctx.style.word;
        self.out.images.push(ImageSrc { src, alt: el.attr("alt").unwrap_or_default().to_string(), width: size("width"), height: size("height"), in_link: ctx.in_link, video, old_side: ctx.old_side, word });
        flow.run.push(Item::Image { slot: self.out.images.len() as u32 - 1, style: ctx.style });
        flow.spaced = false;
    }

    /// `kids` as boxes, inside an element whose text starts afresh.
    fn children(&mut self, kids: &[Dom], ctx: &Ctx) -> Vec<BoxNode> {
        let mut flow = Flow::new(self.out.stream.len());
        self.walk(kids, ctx, &mut flow);
        self.flush(&mut flow, ctx);
        flow.boxes
    }

    fn node_of(&mut self, kids: Vec<BoxNode>, start: usize) -> BoxNode {
        let id = self.id();
        BoxNode::new(id, BoxKind::Flow(kids), start)
    }

    /// One anonymous inline block as the box's own content.
    fn content(&mut self, kids: &[Dom], ctx: &Ctx, start: usize) -> BoxNode {
        let mut list = self.children(kids, ctx);
        if list.len() == 1 && matches!(list[0].kind, BoxKind::Inline(_)) && list[0].margin.top == 0. {
            let mut only = list.remove(0);
            only.start = start;
            return only;
        }
        self.node_of(list, start)
    }

    fn block(&mut self, el: &El, ctx: &Ctx) -> Vec<BoxNode> {
        let start = self.out.stream.len();
        let tag = el.tag.as_str();
        let mut c = *ctx;
        if let Some(a) = el.attr("align") {
            c.align = match a.to_ascii_lowercase().as_str() {
                "center" | "middle" => Align::Center,
                "right" => Align::Right,
                "left" => Align::Left,
                _ => c.align,
            };
        }
        let size = ctx.style.size;
        let node = match tag {
            "p" => {
                let mut n = self.content(&el.kids, &c, start);
                n.margin.bottom = 10.;
                self.sep(2);
                n
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let (em, border) = match tag {
                    "h1" => (1.6, true),
                    "h2" => (1.3, true),
                    "h3" => (1.15, false),
                    "h4" => (1., false),
                    "h5" => (0.9, false),
                    _ => (0.85, false),
                };
                c.style.size = size * em;
                c.style.weight = 600;
                c.style.line = LineH::Num(1.25);
                c.style.color = if tag == "h6" { Tok::TextSecondary } else { Tok::TextSelected };
                let mut n = self.content(&el.kids, &c, start);
                n.margin = Edges::new(16., 0., 8., 0.);
                if border {
                    n.padding.bottom = 0.3 * c.style.size;
                    n.border.bottom = 1.;
                }
                self.sep(2);
                n
            }
            "hr" => {
                let id = self.id();
                let mut n = BoxNode::new(id, BoxKind::Rule, start);
                n.margin = Edges::new(16., 0., 16., 0.);
                n.bg = Some(Tok::SectionBorder);
                self.sep(2);
                n
            }
            "blockquote" => {
                c.style.color = Tok::TextSecondary;
                let kids = self.children(&el.kids, &c);
                let mut n = self.node_of(kids, start);
                n.margin.bottom = 10.;
                n.padding = Edges::new(0., size, 0., size);
                n.border.left = 3.;
                self.sep(2);
                n
            }
            "ul" | "ol" => return self.list(el, &c, start),
            "li" => {
                // An item out of any list: a block with a bullet.
                let kids = self.children(&el.kids, &c);
                self.node_of(kids, start)
            }
            "pre" => {
                let text = dom_text(&el.kids);
                let code = Special::Code { lang: None, value: text.strip_suffix('\n').unwrap_or(&text).to_string(), marks: None, words: None };
                return self.special_block(el, &code, &c).unwrap_or_default();
            }
            "details" => return self.details(el, &c, start),
            "summary" => {
                let mut n = self.content(&el.kids, &c, start);
                n.padding.left = 0.;
                n
            }
            "section" if el.has("data-footnotes") => {
                c.style.size = size * 0.9;
                c.style.color = Tok::TextSecondary;
                let kids = self.children(&el.kids, &c);
                let mut n = self.node_of(kids, start);
                n.margin.top = 16.;
                n.padding.top = 8.;
                n.border.top = 1.;
                n
            }
            "dl" => {
                let kids = self.children(&el.kids, &c);
                let mut n = self.node_of(kids, start);
                n.margin = Edges::new(size, 0., size, 0.);
                n
            }
            "dd" => {
                let kids = self.children(&el.kids, &c);
                let mut n = self.node_of(kids, start);
                n.margin.left = 40.;
                self.sep(1);
                n
            }
            "dt" => {
                let n = self.content(&el.kids, &c, start);
                self.sep(1);
                n
            }
            "table" => return self.table(el, &c, start),
            // A table's part out of place: a plain block.
            _ => {
                let kids = self.children(&el.kids, &c);
                let n = self.node_of(kids, start);
                self.sep(1);
                n
            }
        };
        vec![node]
    }

    fn list(&mut self, el: &El, ctx: &Ctx, start: usize) -> Vec<BoxNode> {
        let ordered = el.tag == "ol";
        let size = ctx.style.size;
        let indent = 2. * size;
        let bullet = match ctx.list_depth {
            0 => MarkerKind::Disc,
            1 => MarkerKind::Circle,
            _ => MarkerKind::Square,
        };
        let mut c = *ctx;
        c.list_depth += 1;
        let mut number = el.attr("start").and_then(|s| s.parse::<i64>().ok()).unwrap_or(1);
        let mut items = Vec::new();
        for k in &el.kids {
            let Dom::El(li) = k else {
                // Text in a list that isn't a space: a block of its own.
                if let Dom::Text(t) = k
                    && !t.trim().is_empty()
                {
                    items.extend(self.children(std::slice::from_ref(k), &c));
                }
                continue;
            };
            if li.tag != "li" {
                items.extend(self.children(std::slice::from_ref(k), &c));
                continue;
            }
            if let Some(v) = li.attr("value").and_then(|s| s.parse::<i64>().ok()) {
                number = v;
            }
            self.anchor(li);
            let at = self.out.stream.len();
            let mark = match li.special.as_deref() {
                Some(Special::Marked(m)) => Some(*m),
                _ => None,
            };
            let mut ic = c;
            if mark == Some(Mark::Removed) {
                ic.old_side = true;
            }
            ic.tint = match mark {
                Some(Mark::Added) => Some(Tok::InsBlock),
                Some(Mark::Removed) => Some(Tok::DelBlock),
                _ => ic.tint,
            };
            let task = matches!(li.kids.first(), Some(Dom::El(e)) if matches!(e.special.as_deref(), Some(Special::Task(_))));
            let kids = self.children(&li.kids, &ic);
            let mut n = self.node_of(kids, at);
            if !items.is_empty() {
                n.margin.top = 2.;
            }
            if !task {
                n.marker = Some(if ordered { Marker::Number(format!("{number}. ")) } else { Marker::Bullet(bullet) });
                n.marker_style = Some(ic.style);
            }
            if let Some(m) = mark {
                n.mark = Some(m);
                n.bar = Some(BarAt::Item);
                n.tinted = m != Mark::Changed;
                n.list_indent = indent;
            }
            number += 1;
            self.sep(1);
            items.push(n);
        }
        let mut n = self.node_of(items, start);
        n.margin.bottom = if ctx.list_depth > 0 { 0. } else { 10. };
        if ctx.list_depth > 0 {
            // `.md li > ul`: only directly in an item; a list in a quote in an item keeps its own.
            n.margin.top = 2.;
        }
        n.padding.left = indent;
        vec![n]
    }

    fn details(&mut self, el: &El, ctx: &Ctx, start: usize) -> Vec<BoxNode> {
        let open = el.has("open");
        let summary_el = el.kids.iter().find_map(|k| match k {
            Dom::El(e) if e.tag == "summary" => Some(e),
            _ => None,
        });
        let id = self.id();
        let mut summary = match summary_el {
            Some(s) => self.content(&s.kids, ctx, start),
            None => {
                let d = [Dom::Text("Details".into())];
                self.content(&d, ctx, start)
            }
        };
        self.sep(1);
        summary.padding.left = ctx.style.size + 1.;
        // No copy of what's left: details in details would cost the square of their depth.
        let mut body_flow = Flow::new(self.out.stream.len());
        for k in &el.kids {
            if !matches!(k, Dom::El(e) if e.tag == "summary") {
                self.walk(std::slice::from_ref(k), ctx, &mut body_flow);
            }
        }
        self.flush(&mut body_flow, ctx);
        let body = body_flow.boxes;
        self.sep(2);
        let mut n = BoxNode::new(id, BoxKind::Details { summary: Box::new(summary), body, open }, start);
        n.margin.bottom = 10.;
        vec![n]
    }

    fn table(&mut self, el: &El, ctx: &Ctx, start: usize) -> Vec<BoxNode> {
        let front = matches!(el.special.as_deref(), Some(Special::Frontmatter(_)));
        let mut c = *ctx;
        if front {
            c.style.size = ctx.style.size * 0.92;
        }
        let mut caption = None;
        let mut rows = Vec::new();
        if front {
            let mut cap = c;
            cap.style.size = c.style.size * 0.85;
            cap.style.color = Tok::TextSecondary;
            let d = [Dom::Text("Front matter".into())];
            let mut n = self.content(&d, &cap, start);
            n.padding.bottom = 2.;
            caption = Some(Box::new(n));
            self.sep(1);
        }
        let row_of = |this: &mut Self, tr: &El, nth: usize, rows: &mut Vec<RowBox>| {
            this.anchor(tr);
            let mark = match tr.special.as_deref() {
                Some(Special::Marked(m)) => Some(*m),
                _ => None,
            };
            let empty = matches!(tr.special.as_deref(), Some(Special::EmptyRow));
            let mut cells = Vec::new();
            for k in &tr.kids {
                let Dom::El(td) = k else { continue };
                if td.tag != "td" && td.tag != "th" {
                    continue;
                }
                this.anchor(td);
                let th = td.tag == "th";
                let mut cc = c;
                if mark == Some(Mark::Removed) {
                    cc.old_side = true;
                }
                if th {
                    cc.style.weight = 600;
                }
                let align = match td.attr("align").map(str::to_ascii_lowercase).as_deref() {
                    Some("center") => Align::Center,
                    Some("right") => Align::Right,
                    Some("left") => Align::Left,
                    _ if th && !front => Align::Center,
                    _ => Align::Left,
                };
                cc.align = align;
                if front && th {
                    cc.style.color = Tok::TextSecondary;
                    cc.nowrap = true;
                }
                if front && !th {
                    cc.pre_line = true;
                }
                let at = this.out.stream.len();
                let mut content = this.content(&td.kids, &cc, at);
                content.padding = Edges::new(4., 10., 4., 10.);
                content.border = Edges::default();
                let span = td.attr("colspan").and_then(|s| s.parse::<u32>().ok()).filter(|n| *n >= 1 && *n <= 1000).unwrap_or(1);
                cells.push(CellBox { content, align, top: front && th, span, nowrap: front && th, th });
                this.out.stream.push('\t');
            }
            // The last tab of a row is the row's end.
            if this.out.stream.ends_with('\t') {
                this.out.stream.pop();
            }
            this.out.stream.push('\n');
            let bg = (!front && nth % 2 == 1).then_some(Tok::PanelBg1);
            rows.push(RowBox { cells, bg, mark, empty });
        };
        let mut loose = 0;
        for k in &el.kids {
            let Dom::El(part) = k else { continue };
            match part.tag.as_str() {
                "tr" => {
                    row_of(self, part, loose, &mut rows);
                    loose += 1;
                }
                "thead" | "tbody" | "tfoot" => {
                    let mut nth = 0;
                    for r in &part.kids {
                        if let Dom::El(tr) = r
                            && tr.tag == "tr"
                        {
                            row_of(self, tr, nth, &mut rows);
                            nth += 1;
                        }
                    }
                }
                "caption" => {
                    let at = self.out.stream.len();
                    let mut cap = c;
                    cap.align = Align::Center;
                    caption = Some(Box::new(self.content(&part.kids, &cap, at)));
                    self.sep(1);
                }
                _ => {}
            }
        }
        self.sep(2);
        let id = self.id();
        let mut n = BoxNode::new(id, BoxKind::Table(TableBox { caption, rows, full: front, bg: front.then_some(Tok::PanelBg1) }), start);
        n.margin.bottom = 10.;
        vec![n]
    }

    fn special_block(&mut self, el: &El, s: &Special, ctx: &Ctx) -> Option<Vec<BoxNode>> {
        let start = self.out.stream.len();
        match s {
            Special::Code { lang, value, marks, words } => {
                if lang.as_deref() == Some("mermaid") {
                    return Some(self.mermaid(value, ctx));
                }
                if lang.as_deref().is_some_and(suggestion_lang) {
                    return Some(self.suggestion(lang.as_deref().unwrap_or_default(), value, ctx));
                }
                Some(vec![self.code(lang.clone(), value, marks.as_deref(), words.as_deref(), false, ctx)])
            }
            Special::DiffBlock { mark, note } => {
                let mut c = *ctx;
                if *mark == Mark::Removed {
                    c.old_side = true;
                }
                c.tint = match mark {
                    Mark::Added => Some(Tok::InsBlock),
                    Mark::Removed => Some(Tok::DelBlock),
                    Mark::Changed => c.tint,
                };
                let mut kids = Vec::new();
                if let Some(text) = note {
                    let mut nc = c;
                    nc.style.size = ctx.style.size * 0.92;
                    nc.style.italic = true;
                    nc.style.color = Tok::TextSecondary;
                    let d = [Dom::Text(text.clone())];
                    let mut n = self.content(&d, &nc, start);
                    n.margin.bottom = 4.;
                    kids.push(n);
                    self.sep(1);
                }
                kids.extend(self.children(&el.kids, &c));
                let mut n = self.node_of(kids, start);
                n.mark = Some(*mark);
                n.bar = Some(BarAt::Block);
                n.tinted = *mark != Mark::Changed;
                Some(vec![n])
            }
            Special::DiffPair => {
                let mut cols = Vec::new();
                for k in &el.kids {
                    if let Dom::El(b) = k
                        && let Some(sp) = b.special.as_deref()
                        && let Some(mut list) = self.special_block(b, sp, ctx)
                    {
                        for n in &mut list {
                            n.bar = None;
                            n.tinted = false;
                        }
                        let at = self.out.stream.len();
                        cols.push(self.node_of(list, at));
                    }
                }
                let id = self.id();
                let mut n = BoxNode::new(id, BoxKind::Columns { cols, gap: 8. }, start);
                n.margin.bottom = 10.;
                n.mark = Some(Mark::Changed);
                Some(vec![n])
            }
            Special::SplitRow { mark, joined } => {
                let mut cols = Vec::new();
                for k in &el.kids {
                    let Dom::El(cell) = k else { continue };
                    match cell.special.as_deref() {
                        Some(Special::SplitCell { old }) => {
                            let mut c = *ctx;
                            c.old_side = *old;
                            let at = self.out.stream.len();
                            let kids = self.children(&cell.kids, &c);
                            let mut n = self.node_of(kids, at);
                            if *joined {
                                drop_last_margin(&mut n);
                            }
                            cols.push(n);
                        }
                        _ => {
                            let id = self.id();
                            let mut n = BoxNode::new(id, BoxKind::Hatch, self.out.stream.len());
                            n.margin.bottom = 10.;
                            cols.push(n);
                        }
                    }
                }
                let id = self.id();
                let mut n = BoxNode::new(id, BoxKind::Columns { cols, gap: 24. }, start);
                n.mark = *mark;
                Some(vec![n])
            }
            _ => None,
        }
    }

    fn code_style(&self, ctx: &Ctx) -> TextStyle {
        let mut s = ctx.style;
        s.family = Family::Code;
        s.size *= 0.92;
        s.line = LineH::Num(1.45);
        s.weight = 400;
        s.italic = false;
        s.code = false;
        s.link = None;
        s.word = None;
        s.strike = false;
        s
    }

    fn code(&mut self, lang: Option<String>, value: &str, marks: Option<&str>, words: Option<&str>, signs: bool, ctx: &Ctx) -> BoxNode {
        let start = self.out.stream.len();
        let mut lines = Vec::new();
        for line in value.split('\n') {
            let at = self.out.stream.len();
            self.out.stream.push_str(line);
            lines.push(at..self.out.stream.len());
            self.out.stream.push('\n');
        }
        let end = self.out.stream.len();
        // A changed block copies its new code: the lines that aren't removed; in the split view's old column, the old.
        let drop = if ctx.old_side { b'+' } else { b'-' };
        let copied = match marks {
            None => value.to_string(),
            Some(m) => value.split('\n').enumerate().filter(|(i, _)| m.as_bytes().get(*i) != Some(&drop)).map(|(_, l)| l).collect::<Vec<_>>().join("\n"),
        };
        let words = marks.and(words).map(|w| w.split(';').map(ranges_of).collect());
        let index = self.out.codes.len() as u32;
        self.out.codes.push((lang.clone().unwrap_or_default(), start..end));
        let id = self.id();
        let style = self.code_style(ctx);
        let cb = CodeBox { index, lang, lines, marks: marks.map(|m| m.as_bytes().to_vec()), words, signs, style, copied, copy_button: true };
        let mut n = BoxNode::new(id, BoxKind::Code(cb), start);
        n.padding = Edges::new(8., 10., 8., 10.);
        n.margin.bottom = 10.;
        n.bg = Some(Tok::AppBg);
        n.radius = 6.;
        self.sep(2);
        n
    }

    /// `MdMermaid`, with nothing here that draws one: its source, and why (`mermaid.rs`).
    fn mermaid(&mut self, value: &str, ctx: &Ctx) -> Vec<BoxNode> {
        let mut n = self.code(Some("mermaid".into()), value, None, None, false, ctx);
        n.margin.bottom = 0.;
        let at = self.out.stream.len();
        let mut c = *ctx;
        c.style.size = 11.;
        c.style.color = Tok::TextSecondary;
        let d = [Dom::Text(super::MERMAID_NOTE.into())];
        let mut note = self.content(&d, &c, at);
        note.margin = Edges::new(4., 0., 10., 0.);
        self.sep(2);
        vec![n, note]
    }

    /// `MdSuggestion`: a diff of the lines it replaces and the lines it puts in.
    fn suggestion(&mut self, lang: &str, code: &str, ctx: &Ctx) -> Vec<BoxNode> {
        let start = self.out.stream.len();
        let mut c = *ctx;
        c.style.size = 11.;
        c.style.color = Tok::TextSecondary;
        let d = [Dom::Text("Suggested change".into())];
        let mut caption = self.content(&d, &c, start);
        caption.padding = Edges::new(3., 8., 3., 8.);
        caption.border.bottom = 1.;
        self.sep(1);
        let replaced = self.base.as_ref().and_then(|b| b.replaced(lang));
        let file_lang = self.base.as_ref().and_then(|b| b.language.clone());
        let body = match replaced {
            None if code.is_empty() => None,
            None => Some((code.to_string(), "+".repeat(code.split('\n').count()), None)),
            Some(lines) if code.is_empty() => {
                let v = lines.join("\n");
                let n = lines.len();
                Some((v, "-".repeat(n), None))
            }
            Some(lines) => {
                let m = super::diff::code_lines(&lines.join("\n"), code, None).unwrap_or_default();
                Some((m.value, m.marks, m.words))
            }
        };
        let mut kids = vec![caption];
        match body {
            Some((value, marks, words)) => {
                let mut n = self.code(file_lang, &value, Some(&marks), words.as_deref(), true, ctx);
                n.margin.bottom = 0.;
                n.radius = 0.;
                if let BoxKind::Code(cb) = &mut n.kind {
                    cb.signs = true;
                }
                kids.push(n);
            }
            None => {
                let mut nc = *ctx;
                nc.style.italic = true;
                nc.style.color = Tok::TextSecondary;
                let d = [Dom::Text("Removes these lines".into())];
                let at = self.out.stream.len();
                let mut n = self.content(&d, &nc, at);
                n.padding = Edges::new(4., 8., 4., 8.);
                kids.push(n);
            }
        }
        let mut n = self.node_of(kids, start);
        n.margin.bottom = 8.;
        n.border = Edges::new(1., 1., 1., 1.);
        n.radius = 6.;
        self.sep(2);
        vec![n]
    }
}

fn drop_last_margin(n: &mut BoxNode) {
    if let BoxKind::Flow(kids) = &mut n.kind
        && let Some(last) = kids.last_mut()
        && last.padding.left > 0.
    {
        last.margin.bottom = 0.;
    }
}

fn root_strut(ctx: &Ctx) -> TextStyle {
    TextStyle { link: None, strike: false, underline: false, code: false, word: None, tint: None, valign: VAlign::Base, ..ctx.style }
}

fn dom_text(kids: &[Dom]) -> String {
    let mut s = String::new();
    for k in kids {
        match k {
            Dom::Text(t) => s.push_str(t),
            Dom::El(e) => s.push_str(&dom_text(&e.kids)),
        }
    }
    s
}

/// A line's word ranges (`start-end`, `,` between them), sorted; malformed ones dropped (`rangesOf`).
fn ranges_of(entry: &str) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = entry.split(',').filter_map(|r| {
        let (a, b) = r.split_once('-')?;
        let (a, b): (usize, usize) = (a.parse().ok()?, b.parse().ok()?);
        (b > a).then_some((a, b))
    }).collect();
    out.sort();
    out
}

/// The tree's boxes.
pub fn build(dom: &[Dom], params: Params, base: Option<super::SuggestionBase>) -> Built {
    let mut b = Build { out: Built::default(), params, base };
    let root = TextStyle { family: Family::Reading, size: params.font_px, weight: 400, italic: false, color: Tok::TextNormal, line: LineH::Num(params.line_height), valign: VAlign::Base, link: None, strike: false, underline: false, code: false, word: None, tint: None };
    let ctx = Ctx { style: root, align: Align::None, list_depth: 0, pre: false, pre_line: false, nowrap: false, old_side: false, in_link: false, tint: None };
    let mut tops = b.children(dom, &ctx);
    // `.md > :first-child`, `.md > :last-child`.
    if let Some(first) = tops.first_mut() {
        first.margin.top = 0.;
    }
    if let Some(last) = tops.last_mut() {
        last.margin.bottom = 0.;
    }
    let _ = b.params;
    b.out.tops = tops;
    b.out
}
