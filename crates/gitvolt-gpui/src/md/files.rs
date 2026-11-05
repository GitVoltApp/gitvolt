//! A Markdown file in the diff panel (`diff/markdownFiles.ts`, `FileBody.tsx`, `DiffTextBody.tsx`): `Source | Rendered`,
//! the rendered view in the editor's place, and in Diff View the rendered diff with its overview ruler (`mdRuler.ts`).

use super::ast::{Flavor, Mark};
use super::boxes::Params;
use super::links::{LinkEnv, MdContext};
use super::view::{Content, DEFAULT_MAX_BYTES, MdEvent, MdOptions, MdView};
use crate::app::GitVolt;
use crate::details::{FileChange, segment, segmented};
use crate::diff::view::{DiffState, ViewKind};
use crate::theme;
use crate::ui::tooltip::tip;
use gpui::prelude::*;
use gpui::*;
use std::rc::Rc;
use std::sync::Arc;

pub const TOO_LARGE_TO_RENDER: &str = "Too large to render";
/// Why the text-diff tools are off while the rendered diff shows.
pub const RENDERED_HUNK_TIP: &str = "The rendered diff shows the whole document: pick Inline or Split, or Source for Hunk";
pub const RENDERED_WHITESPACE_TIP: &str = "The rendered diff already ignores whitespace that doesn't show";
pub const RENDERED_WRAP_TIP: &str = "Rendered text always wraps";
/// Narrower than this, Split shows as one column (`INLINE_BREAKPOINT_PX`).
const INLINE_BREAKPOINT: f32 = 600.;
/// Monaco's width of a diff's overview ruler.
const RULER_W: f32 = 30.;
const LANE_W: f32 = 15.;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MdPick {
    Rendered,
    Source,
}

pub struct MdPane {
    pub view: Entity<MdView>,
    pub state: Rc<std::cell::RefCell<super::view::MdState>>,
    pub key: (String, ViewKind, bool, Option<gitvolt_core::forge::ForgeKind>),
    pub split: bool,
    _events: Subscription,
}

pub fn is_markdown_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, ext)) => !stem.is_empty() && ["md", "markdown", "mdx"].contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}

/// Either side's path: a rename to or from a Markdown name still renders.
pub fn is_markdown(file: &FileChange) -> bool {
    is_markdown_path(&file.path) || file.old_path.as_deref().is_some_and(is_markdown_path)
}

/// A side's commit, for its links and images.
fn commit_of(side: &serde_json::Value) -> Option<String> {
    match side["kind"].as_str() {
        Some("atCommit") => side["commit"].as_str().map(str::to_string),
        Some("absent") => None,
        _ => Some("worktree".to_string()),
    }
}

/// Split shows side by side, unless the pane's too narrow.
pub fn wants_split(diff: &DiffState) -> bool {
    diff.mode == crate::diff::model::Mode::Split && diff.view == ViewKind::Diff && diff.md_width.is_none_or(|w| w >= INLINE_BREAKPOINT)
}

impl DiffState {
    /// The rendered view's on: not while it's too large, an image or a binary.
    pub fn shows_rendered(&self, cx: &App) -> bool {
        self.markdown == MdPick::Rendered && is_markdown(&self.file) && self.md.as_ref().is_some_and(|m| !m.view.read(cx).too_large()) && self.hex.is_none() && self.image.is_none() && self.notice.is_none()
    }
}

impl GitVolt {
    /// The tab's rendered view, while it's what shows.
    pub fn shown_md(&self) -> Option<Entity<MdView>> {
        let diff = self.tabs.get(self.active)?.diff.as_ref()?;
        (diff.markdown == MdPick::Rendered && diff.hex.is_none() && diff.image.is_none() && diff.notice.is_none()).then(|| diff.md.as_ref().map(|m| m.view.clone())).flatten()
    }

    fn reading_params(&self) -> (Params, Option<String>, Option<f32>) {
        let size = self.settings["readingFontSize"].as_f64().unwrap_or(13.).clamp(8., 32.) as f32;
        let lh = self.settings["readingLineHeight"].as_f64().unwrap_or(1.55).clamp(1.2, 2.4) as f32;
        let family = self.settings["readingFontFamily"].as_str().unwrap_or("");
        let reading = crate::settings::fonts::families_of(family).into_iter().next();
        let width = self.settings["readingLineWidth"].as_f64().unwrap_or(0.) as f32;
        (Params { font_px: size, line_height: lh, editor_code: true }, reading, (width > 0.).then_some(width))
    }

    /// The open file's rendered view follows what's read and what's picked.
    pub fn sync_markdown(&mut self, cx: &mut Context<Self>) {
        let (params, reading, measure) = self.reading_params();
        let (api, rt, retired) = (self.api.clone(), self.rt.clone(), self.retired_images.clone());
        let pick_source = self.prefs["markdownView"] == "source";
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let (id, worktree) = (tab.id, tab.worktree.clone());
        let by_sha: Rc<Vec<String>> = Rc::new(tab.graph.as_ref().map_or(Vec::new(), |g| g.by_sha.keys().map(|k| k.to_string()).collect()));
        let forge = tab.forge.kind.map(|k| (crate::forge::mrview::link_env(tab), k));
        let Some(diff) = tab.diff.as_mut() else { return };
        diff.markdown = if pick_source { MdPick::Source } else { MdPick::Rendered };
        if !is_markdown(&diff.file) || diff.markdown != MdPick::Rendered {
            return;
        }
        let Some((old, new)) = diff.data.as_ref().and_then(|d| d.raw.clone()) else { return };
        // What's shown of a file being edited is its buffer, unsaved typing included.
        let new: Arc<str> = diff.edit.as_ref().map_or(new, |e| e.buffer.text.as_str().into());
        let file_view = diff.view == ViewKind::File;
        let (old_side, new_side) = crate::diff::edit::sides_for(&diff.file, diff.view);
        let new_commit = commit_of(&new_side);
        let old_commit = commit_of(&old_side);
        let path = diff.file.path.to_string();
        let old_path = diff.file.old_path.clone().unwrap_or_else(|| path.clone());
        let repo = crate::app::core_repo(id);
        let ctx = MdContext::File { repo, commit: new_commit.clone().or(old_commit.clone()).unwrap_or_else(|| "worktree".into()), path: path.clone() };
        let old_ctx = MdContext::File { repo, commit: old_commit.or(new_commit).unwrap_or_else(|| "worktree".into()), path: old_path };
        let split = wants_split(diff);
        let key = (path.clone(), diff.view, split, forge.as_ref().map(|f| f.1));
        let content = if file_view {
            let shown = if new.is_empty() && !old.is_empty() { old } else { new };
            Content::Text(shown)
        } else {
            Content::Diff { old, new, split }
        };
        let env = LinkEnv {
            users: None,
            kind: None,
            project: None,
            full_sha: Some(Rc::new(move |prefix: &str| {
                let p = prefix.to_ascii_lowercase();
                by_sha.iter().find(|s| s.starts_with(&p)).cloned()
            })),
        };
        // On a forge, a file's references and flavour are its; loaded commits as ever.
        let (env, flavor) = match forge {
            Some((forge_env, kind)) => (LinkEnv { full_sha: env.full_sha.clone(), ..forge_env }, crate::forge::mrview::flavor_of(kind)),
            None => (env, Flavor::Github),
        };
        let opts = MdOptions { ctx, old_ctx: Some(old_ctx), env, flavor, frontmatter: true, params, pane: true, base: None, worktree, reading_font: reading, max_bytes: DEFAULT_MAX_BYTES, measure_ch: measure, ruled: !file_view, split, inset: None };
        let same_place = diff.md.as_ref().is_some_and(|m| m.key == key);
        let keep = same_place;
        if !same_place {
            let view = cx.new(|cx| MdView::new(opts, api, rt, retired, cx));
            let events = cx.subscribe(&view, |this, _, e: &MdEvent, cx| this.on_md_event(e, cx));
            let state = view.read(cx).state.clone();
            diff.md = Some(MdPane { view, state, key, split, _events: events });
        }
        let anchor = diff.md_anchor.take();
        if let Some(m) = diff.md.as_ref() {
            let reveal = !file_view && !keep;
            m.view.update(cx, |v, cx| {
                v.state.borrow_mut().reveal_first_change = reveal;
                v.set_content(content, keep, cx);
                if let Some(a) = anchor {
                    v.go_to_anchor(&a, cx);
                }
            });
        }
        cx.notify();
    }

    /// The settings' reading rows, on what's open.
    pub fn apply_reading_settings(&mut self, cx: &mut Context<Self>) {
        let (params, reading, measure) = self.reading_params();
        for tab in &mut self.tabs {
            if let Some(m) = tab.diff.as_ref().and_then(|d| d.md.as_ref()) {
                m.view.update(cx, |v, cx| v.set_params(params, reading.clone(), measure, cx));
            }
        }
    }

    fn on_md_event(&mut self, e: &MdEvent, cx: &mut Context<Self>) {
        match e {
            MdEvent::OpenFile { path, commit, anchor } => {
                let at = if commit == "worktree" { None } else { Some(commit.clone()) };
                match at {
                    Some(sha) => self.open_path_at(path, &sha, false, cx),
                    None => self.open_worktree_file(path, cx),
                }
                if let Some(diff) = self.tabs.get_mut(self.active).and_then(|t| t.diff.as_mut()) {
                    diff.md_anchor = anchor.clone();
                }
            }
            MdEvent::ShowCommit { sha } => self.go_to_commit(sha.clone().into(), cx),
            MdEvent::OpenMr { number, web_url } => self.forge_open_or_browse(*number, web_url.clone(), cx),
            MdEvent::TooLarge => cx.notify(),
            MdEvent::Zoom(dir) => {
                let dir = *dir;
                self.reading_zoom(dir, cx);
            }
        }
    }

    /// Ctrl+wheel and Ctrl+= / - / 0 over a rendered pane: the reading size (`nextReadingFont`).
    pub fn reading_zoom(&mut self, dir: i32, cx: &mut Context<Self>) {
        let now = self.settings["readingFontSize"].as_f64().unwrap_or(13.);
        let next = if dir == 0 { 13. } else { (now + dir as f64).clamp(8., 32.) };
        if next != now {
            self.set_settings(move |s| s["readingFontSize"] = serde_json::json!(next as u32), cx);
        }
        self.apply_reading_settings(cx);
        crate::ui::toast::show(format!("Reading text {} px", next as u32), cx);
    }

    /// `GITVOLT_SHOW`'s and a script's `mdfield`: a field in a dialog, as a comment box has it.
    pub fn open_field_demo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use super::field::{FieldOptions, MarkdownField, PeopleSource};
        use super::complete::Person;
        let repo = self.tabs.get(self.active).map_or(0, |t| crate::app::core_repo(t.id));
        let md = MdOptions::for_comment(repo, LinkEnv::default(), Flavor::Github, Params::default());
        let people = PeopleSource { participants: vec![Person { id: 1, username: "ada".into(), name: "Ada Fixture".into() }, Person { id: 2, username: "ben".into(), name: "Ben Harbour".into() }], search: None, avatar: None };
        let opts = FieldOptions { md, label: "Write a comment".into(), placeholder: "Leave a comment".into(), rows: 6, grow_preview: false, people: Some(people) };
        let (api, rt, retired) = (self.api.clone(), self.rt.clone(), self.retired_images.clone());
        let field = cx.new(|cx| MarkdownField::new(opts, api, rt, retired, cx));
        self.field_demo = Some(field.clone());
        let focus = field.read(cx).input.read(cx).focus.clone();
        let shown = field.clone();
        crate::ui::modal::open(crate::ui::modal::Modal::new("md-field-demo", move |_, _| div().w(px(560.)).child(shown.clone()).into_any_element()).width(600.).focus(focus), window, cx);
    }

    fn pick_markdown(&mut self, pick: MdPick, cx: &mut Context<Self>) {
        self.prefs["markdownView"] = serde_json::json!(if pick == MdPick::Rendered { "rendered" } else { "source" });
        self.save_layout();
        self.with_diff(cx, |d| d.markdown = pick);
        self.sync_markdown(cx);
    }
}

/// `MarkdownViewToggle`.
pub fn view_toggle(diff: &DiffState, cx: &mut Context<GitVolt>) -> AnyElement {
    let too_large = diff.md.as_ref().is_some_and(|m| m.view.read(cx).too_large());
    let source = diff.markdown == MdPick::Source || too_large;
    let rendered = segment("md-rendered", "Rendered", !source, !too_large).when(!too_large, |d| d.on_click(cx.listener(|this, _, _, cx| this.pick_markdown(MdPick::Rendered, cx))));
    let rendered = if too_large { tip(TOO_LARGE_TO_RENDER, rendered).into_any_element() } else { rendered.into_any_element() };
    segmented(vec![segment("md-source", "Source", source, true).on_click(cx.listener(|this, _, _, cx| this.pick_markdown(MdPick::Source, cx))).into_any_element(), rendered]).into_any_element()
}

pub fn too_large_banner(diff: &DiffState, cx: &App) -> Option<AnyElement> {
    let large = diff.markdown == MdPick::Rendered && diff.md.as_ref().is_some_and(|m| m.view.read(cx).too_large());
    large.then(|| div().flex_none().px(px(12.)).py(px(4.)).bg(theme::yellow_banner()).text_color(theme::text_selected()).child(TOO_LARGE_TO_RENDER).into_any_element())
}

/// The rendered view in the editor's place, when that's what shows. A diff has its overview ruler.
pub fn rendered_body(diff: &DiffState, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    if !diff.shows_rendered(cx) {
        return None;
    }
    let pane = diff.md.as_ref()?;
    let view = pane.view.clone();
    if diff.view == ViewKind::File {
        return Some(div().flex().flex_1().min_w_0().min_h_0().child(view).into_any_element());
    }
    let (split, st) = (pane.split, view.read(cx).state.clone());
    let marks = st.borrow().change_marks();
    let rule_view = view.clone();
    Some(
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .relative()
            .child(view)
            .child(div().id("md-ruler").absolute().top_0().bottom_0().right_0().w(px(RULER_W)).child(canvas(
                |_, _, _| {},
                move |b, _, window, cx| paint_ruler(b, &rule_view, &marks, split, window, cx),
            )
            .size_full()))
            .into_any_element(),
    )
}

fn mark_tint(m: Mark) -> Hsla {
    match m {
        Mark::Added => theme::ruler_added(),
        Mark::Removed => theme::ruler_removed(),
        Mark::Changed => Hsla { a: theme::ruler_added().a, ..theme::status_modified() },
    }
}

/// `markRects`: each change at least 2 px, centred on it and kept on the strip; split, removed on the left lane, added on
/// the right, changed across. The slider's Monaco's `ScrollbarState`. A press moves the view and a drag follows.
fn paint_ruler(b: Bounds<Pixels>, view: &Entity<MdView>, marks: &[(f32, f32, Mark)], split: bool, window: &mut Window, cx: &mut App) {
    let st = view.read(cx).state.clone();
    let (content, visible, scroll, max) = {
        let s = st.borrow();
        (s.total_height().max(s.view_height()), s.view_height(), s.scroll, s.max_scroll())
    };
    let strip = b.size.height.as_f32();
    if content <= 0. || strip <= 0. {
        return;
    }
    let ratio = strip / content;
    for (top, h, kind) in marks {
        let (y1, y2) = ((top * ratio).floor(), ((top + h) * ratio).floor());
        let half = ((y2 - y1) / 2.).max(1.);
        let centre = ((y1 + y2) / 2.).max(half).min(strip - half);
        let (x, w) = match (split, kind) {
            (false, _) | (_, Mark::Changed) => (0., RULER_W),
            (true, Mark::Removed) => (0., LANE_W),
            (true, Mark::Added) => (LANE_W, LANE_W),
        };
        window.paint_quad(fill(Bounds::new(point(b.origin.x + px(x), b.origin.y + px(centre - half)), size(px(w), px(2. * half))), mark_tint(*kind)));
    }
    let slider_h = if content > visible { ((visible * strip / content).floor().max(20.)).round() } else { strip };
    if content > visible {
        let slide = (strip - slider_h) / (content - visible);
        window.paint_quad(fill(Bounds::new(point(b.origin.x, b.origin.y + px((scroll * slide).round())), size(px(RULER_W), px(slider_h))), theme::scroll_thumb()));
    }
    let to_scroll = move |y: f32| ((y - slider_h / 2.) / (strip - slider_h).max(1.)).clamp(0., 1.) * max;
    let (v1, v2) = (view.clone(), view.clone());
    let (s1, s2) = (st.clone(), st);
    let dragging = Rc::new(std::cell::Cell::new(false));
    let d2 = dragging.clone();
    window.on_mouse_event(move |e: &MouseDownEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble && b.contains(&e.position) && e.button == MouseButton::Left {
            s1.borrow_mut().set_scroll(to_scroll((e.position.y - b.origin.y).as_f32()));
            s1.borrow_mut().ruler_drag = true;
            v1.update(cx, |_, cx| cx.notify());
        }
    });
    let _ = d2;
    window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        let drag = s2.borrow().ruler_drag;
        if drag {
            if e.pressed_button == Some(MouseButton::Left) {
                s2.borrow_mut().set_scroll(to_scroll((e.position.y - b.origin.y).as_f32()));
            } else {
                s2.borrow_mut().ruler_drag = false;
            }
            v2.update(cx, |_, cx| cx.notify());
        }
    });
    let _ = dragging;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_markdown_file_is_told_by_its_extension() {
        assert!(is_markdown_path("docs/READ.ME.MD"));
        assert!(is_markdown_path("a.mdx"));
        assert!(!is_markdown_path(".md"), "a dot file's name, not an extension");
        assert!(!is_markdown_path("a.md.txt"));
    }
}
