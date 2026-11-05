//! The Keyboard Shortcuts panel (`ui/src/shortcuts/`): every chord in the command table and every key the views handle
//! themselves, by section, with a filter box. The web UI has no rebinding, so neither has this.

use crate::app::{GitVolt, icon};
use crate::commands::{self, Group};
use crate::theme;
use crate::ui::controls::{self, keycaps};
use crate::ui::input::TextInput;
use crate::ui::modal::{self, Modal};
use crate::ui::tooltip::tip;
use gpui::prelude::*;
use gpui::*;
use std::rc::Rc;

struct H {
    id: &'static str,
    section: &'static str,
    label: &'static str,
    keys: &'static [&'static str],
    context: &'static str,
}

/// The hints: `registerKeyHints` across the web UI, for keys that aren't app actions.
const HINTS: &[H] = &[
    H { id: "key.commit", section: "Commit message", label: "Commit", keys: &["Mod+Enter"], context: "(when in message input)" },
    H { id: "key.msgNext", section: "Commit message", label: "Summary to description", keys: &["Enter"], context: "(when in the summary)" },
    H { id: "key.msgBlur", section: "Commit message", label: "Leave the message input", keys: &["Esc"], context: "(when in message input)" },
    H { id: "key.textSizeIn", section: "Diff", label: "Larger text (over code: the code; over rendered Markdown: the reading text; also Ctrl+wheel)", keys: &["Mod+="], context: "" },
    H { id: "key.textSizeOut", section: "Diff", label: "Smaller text (over a file or diff)", keys: &["Mod+-"], context: "" },
    H { id: "key.textSizeReset", section: "Diff", label: "Reset the text size (over a file or diff)", keys: &["Mod+0"], context: "" },
    H { id: "key.mdPreview", section: "Markdown", label: "Switch between Write and Preview", keys: &["Mod+Shift+P"], context: "(when writing a comment or description)" },
    H { id: "lightbox.zoom", section: "Image viewer", label: "Zoom in / out", keys: &["+", "-"], context: "(or the mouse wheel)" },
    H { id: "lightbox.fit", section: "Image viewer", label: "Fit to the window", keys: &["0"], context: "" },
    H { id: "lightbox.actual", section: "Image viewer", label: "Actual size (100%)", keys: &["1"], context: "" },
    H { id: "lightbox.pan", section: "Image viewer", label: "Pan a zoomed image", keys: &["Left", "Right", "Up", "Down"], context: "(or drag it)" },
    H { id: "lightbox.close", section: "Image viewer", label: "Close", keys: &["Esc"], context: "" },
    H { id: "lightbox.open", section: "Image viewer", label: "View an image full size", keys: &["Enter"], context: "(when an image in a comment has focus; or click it)" },
    H { id: "key.reviewCardLeave", section: "Merge request", label: "Leave the comment card for the diff", keys: &["Esc"], context: "(on a comment card in the diff)" },
    H { id: "key.reviewIconExpand", section: "Merge request", label: "Expand a collapsed thread", keys: &["Enter", "Space"], context: "" },
    H { id: "key.reviewIconLeave", section: "Merge request", label: "Leave a collapsed thread for the diff", keys: &["Esc"], context: "" },
    H { id: "key.reviewAdd", section: "Merge request", label: "Add the comment to your review", keys: &["Mod+Enter"], context: "(when writing a comment in the diff)" },
    H { id: "key.reviewCancel", section: "Merge request", label: "Cancel the comment (arms first when it has text)", keys: &["Esc"], context: "(when writing a comment in the diff)" },
    H { id: "key.reviewDraftSave", section: "Merge request", label: "Save the edited pending comment", keys: &["Mod+Enter"], context: "(when editing a pending comment in the diff)" },
    H { id: "key.reviewDraftCancel", section: "Merge request", label: "Leave the edit", keys: &["Esc"], context: "(when editing a pending comment in the diff)" },
    H { id: "key.renderedReviewCardLeave", section: "Merge request", label: "Leave the comment card for the rendered diff", keys: &["Esc"], context: "(on a comment card in the rendered Markdown diff)" },
    H { id: "key.fc.move", section: "Settings", label: "Move the focused scope up or down", keys: &["Alt+↑", "Alt+↓"], context: "on the File colours page" },
    H { id: "key.fc.paste", section: "Settings", label: "Paste scopes from the clipboard", keys: &["Mod+V"], context: "on the File colours page" },
    H { id: "key.mrCreate", section: "Merge request", label: "Create the merge/pull request", keys: &["Mod+Enter"], context: "(when writing the title or description)" },
    H { id: "key.mrReview", section: "Merge request", label: "Submit the review (Comment, Approve or Request changes)", keys: &["Mod+Enter"], context: "(in the review composer)" },
    H { id: "key.mrNoteSave", section: "Merge request", label: "Save the edited comment", keys: &["Mod+Enter"], context: "(when editing a comment)" },
    H { id: "key.mrNoteCancel", section: "Merge request", label: "Cancel the edit, or close the reaction picker", keys: &["Esc"], context: "(when editing a comment or picking a reaction)" },
    H { id: "key.mrReply", section: "Merge request", label: "Send the comment or reply", keys: &["Mod+Enter"], context: "(when writing a comment)" },
    H { id: "key.mrReplyResolve", section: "Merge request", label: "Reply and resolve the thread (or unresolve it)", keys: &["Mod+Shift+Enter"], context: "(when replying in a resolvable thread)" },
    H { id: "key.stackCreate", section: "Merge request", label: "Create the stack of merge/pull requests", keys: &["Mod+Enter"], context: "(in the Create stack dialog)" },
    H { id: "key.reveal.open", section: "Graph", label: "Show the commits a collapsed branch or folded merge holds", keys: &["Enter"], context: "(on a ⋮ row or a folded merge)" },
    H { id: "key.reveal.move", section: "Graph", label: "Select the previous / next hidden commit", keys: &["Up", "Down"], context: "(in a reveal panel)" },
    H { id: "key.reveal.close", section: "Graph", label: "Close the reveal panel", keys: &["Esc"], context: "(while one is open)" },
    H { id: "key.fh.close", section: "File history", label: "Close file history", keys: &["Esc"], context: "(when not typing)" },
    H { id: "key.rb.action", section: "Rebase editor", label: "Pick / Reword / Squash / Fixup / Drop / Edit", keys: &["P", "R", "S", "F", "D", "E"], context: "(when a row is selected)" },
    H { id: "key.rb.move", section: "Rebase editor", label: "Move row", keys: &["Mod+Up", "Mod+Down"], context: "" },
    H { id: "key.rb.msg", section: "Rebase editor", label: "Edit message", keys: &["Enter"], context: "(when a row is selected)" },
    H { id: "key.rb.save", section: "Rebase editor", label: "Save the plan", keys: &["Mod+Enter"], context: "" },
    H { id: "key.rb.cancel", section: "Rebase editor", label: "Cancel", keys: &["Esc"], context: "" },
    H { id: "lightbox.zoom", section: "Image viewer", label: "Zoom in / out", keys: &["+", "-"], context: "(or the mouse wheel)" },
    H { id: "lightbox.fit", section: "Image viewer", label: "Fit to the window", keys: &["0"], context: "" },
    H { id: "lightbox.actual", section: "Image viewer", label: "Actual size (100%)", keys: &["1"], context: "" },
    H { id: "lightbox.pan", section: "Image viewer", label: "Pan a zoomed image", keys: &["Left", "Right", "Up", "Down"], context: "(or drag it)" },
    H { id: "lightbox.close", section: "Image viewer", label: "Close", keys: &["Esc"], context: "" },
    H { id: "lightbox.open", section: "Image viewer", label: "View an image full size", keys: &["Enter"], context: "(when an image in a comment has focus; or click it)" },
    H { id: "key.mdPreview", section: "Markdown", label: "Switch between Write and Preview", keys: &["Mod+Shift+P"], context: "(when writing a comment or description)" },
    H { id: "key.ghost.open", section: "Navigation", label: "List the hidden branches that start at a commit", keys: &["Enter"], context: "on a commit hidden branches fork from" },
    H { id: "key.ghost.close", section: "Navigation", label: "Close the hidden branches list", keys: &["Esc"], context: "when it is open" },
    H { id: "key.altclick.branch", section: "Navigation", label: "Hide or show a branch, tag or folder", keys: &["Alt+Click"], context: "branch or tag chip, sidebar branch, tag or folder" },
    H { id: "key.bp.move", section: "Settings", label: "Move the focused preset or rule up or down", keys: &["Alt+↑", "Alt+↓"], context: "on the Branch presets page" },
    H { id: "key.bp.paste", section: "Settings", label: "Paste a branch preset from the clipboard", keys: &["Mod+V"], context: "on the Branch presets page" },
    H { id: "key.esc.file", section: "Navigation", label: "Close the open file or leave compare", keys: &["Esc"], context: "(when not typing)" },
    H { id: "graph.move", section: "Graph", label: "Select previous / next commit", keys: &["Up", "Down"], context: "(when the graph has focus)" },
    H { id: "graph.page", section: "Graph", label: "Page up / down", keys: &["PageUp", "PageDown"], context: "(when the graph has focus)" },
    H { id: "graph.ends", section: "Graph", label: "First / last commit", keys: &["Home", "End"], context: "(when the graph has focus)" },
    H { id: "graph.range", section: "Graph", label: "Extend the selection", keys: &["Shift+Up", "Shift+Down"], context: "(when the graph has focus)" },
    H { id: "graph.open", section: "Graph", label: "Open the selected commit's first changed file", keys: &["Right", "Enter"], context: "(when the graph has focus)" },
    H { id: "graph.menu", section: "Graph", label: "Context menu", keys: &["Shift+F10", "Menu"], context: "(when the graph has focus)" },
    H { id: "graph.draft", section: "Graph", label: "Leave the WIP summary box", keys: &["Esc", "Enter"], context: "(when in the WIP summary input)" },
    H { id: "files.move", section: "File list", label: "Previous / next file", keys: &["Up", "Down"], context: "(when the file list has focus)" },
    H { id: "files.page", section: "File list", label: "Page up / down", keys: &["PageUp", "PageDown"], context: "(when the file list has focus)" },
    H { id: "files.ends", section: "File list", label: "First / last file", keys: &["Home", "End"], context: "(when the file list has focus)" },
    H { id: "files.expand", section: "File list", label: "Expand folder, or open the file", keys: &["Right"], context: "(when the file list has focus)" },
    H { id: "files.collapse", section: "File list", label: "Collapse folder, or close the diff", keys: &["Left"], context: "(when the file list has focus)" },
    H { id: "files.toggle", section: "File list", label: "Open or close the file, toggle a folder", keys: &["Enter", "Space"], context: "(when the file list has focus)" },
    H { id: "files.menu", section: "File list", label: "Context menu", keys: &["Shift+F10", "Menu"], context: "(when the file list has focus)" },
    H { id: "files.filter", section: "File list", label: "Close the file filter", keys: &["Esc"], context: "(when in the filter input)" },
    H { id: "files.filterLeave", section: "File list", label: "Clear the file filter", keys: &["Esc"], context: "(when in the filter input)" },
    H { id: "files.create", section: "File list", label: "Create the file / cancel", keys: &["Enter", "Esc"], context: "(when naming a new file)" },
    H { id: "stage.undo", section: "Staging", label: "Undo the last staging action", keys: &["Mod+Z"], context: "(when in the diff view or WIP file list)" },
    H { id: "stage.redo", section: "Staging", label: "Redo the staging action", keys: &["Mod+Shift+Z"], context: "(when in the diff view or WIP file list)" },
    H { id: "sb.move", section: "Sidebar", label: "Previous / next row", keys: &["Up", "Down"], context: "(when the sidebar has focus)" },
    H { id: "sb.ends", section: "Sidebar", label: "First / last row", keys: &["Home", "End"], context: "(when the sidebar has focus)" },
    H { id: "sb.open", section: "Sidebar", label: "Activate the row", keys: &["Enter", "Space"], context: "(when the sidebar has focus)" },
    H { id: "sb.expand", section: "Sidebar", label: "Expand / collapse a group", keys: &["Right", "Left"], context: "(when the sidebar has focus)" },
    H { id: "sb.hide", section: "Sidebar", label: "Hide or show the branch or folder in the graph", keys: &["H"], context: "(when the sidebar has focus)" },
    H { id: "sb.filter", section: "Sidebar", label: "Clear the filter / move into the list", keys: &["Esc", "Down"], context: "(when in the sidebar filter)" },
    H { id: "sb.resize", section: "Sidebar", label: "Reset the width", keys: &["Enter"], context: "(when the resize handle has focus)" },
    H { id: "find.next", section: "Find", label: "Next match", keys: &["Enter", "Down"], context: "(when in the find box)" },
    H { id: "find.prev", section: "Find", label: "Previous match", keys: &["Shift+Enter", "Up"], context: "(when in the find box)" },
    H { id: "find.close", section: "Find", label: "Close find", keys: &["Esc"], context: "(when in the find box)" },
    H { id: "pal.move", section: "Command palette", label: "Previous / next result", keys: &["Up", "Down"], context: "(when the palette is open)" },
    H { id: "pal.run", section: "Command palette", label: "Run the result", keys: &["Enter"], context: "(when the palette is open)" },
    H { id: "pal.close", section: "Command palette", label: "Close", keys: &["Esc"], context: "(when the palette is open)" },
    H { id: "merge.step", section: "Merge tool", label: "Next / previous conflict", keys: &["F7", "Shift+F7"], context: "" },
    H { id: "merge.save", section: "Merge tool", label: "Save the result", keys: &["Mod+S"], context: "" },
    H { id: "merge.toggle", section: "Merge tool", label: "Toggle the conflict at the cursor", keys: &["Space"], context: "(when the cursor is in a conflict)" },
    H { id: "tabs.rename", section: "Navigation", label: "Commit / cancel a tab rename", keys: &["Enter", "Esc"], context: "(when renaming a tab)" },
    H { id: "tabs.activate", section: "Navigation", label: "Activate the focused tab", keys: &["Enter", "Space"], context: "(when a tab has focus)" },
    H { id: "tabs.groupToggle", section: "Navigation", label: "Collapse / expand the tab group", keys: &["Enter", "Space"], context: "(when a group chip has focus)" },
    H { id: "tabs.groupMenu", section: "Navigation", label: "Tab group menu (colour, name, actions)", keys: &["Shift+F10", "Menu"], context: "(when a group chip has focus)" },
    H { id: "tabs.groupList", section: "Navigation", label: "List the group's tabs; move, switch, close", keys: &["Down", "Up", "Enter", "Esc"], context: "(when a group chip has focus)" },
    H { id: "dlg.close", section: "Dialogs and menus", label: "Close the dialog / menu", keys: &["Esc"], context: "" },
    H { id: "menu.move", section: "Dialogs and menus", label: "Move in a menu", keys: &["Up", "Down", "Left", "Right", "Home", "End", "Enter"], context: "(when a menu is open)" },
    H { id: "img.swipe", section: "Diff", label: "Move the swipe handle", keys: &["Left", "Right"], context: "(when the swipe handle has focus)" },
    H { id: "diff.save", section: "Diff", label: "Save the working copy", keys: &["Mod+S"], context: "(when editing a file)" },
    H { id: "themeFloat.move", section: "Theme editor", label: "Move the floating theme editor", keys: &["Alt+Left", "Alt+Right", "Alt+Up", "Alt+Down"], context: "(when its title bar has focus; or drag it)" },
    H { id: "themeFloat.done", section: "Theme editor", label: "Back to Settings", keys: &["Esc"], context: "(when the floating theme editor has focus)" },
    H { id: "key.zoomIn", section: "Navigation", label: "Zoom in", keys: &["Mod+="], context: "" },
    H { id: "key.zoomOut", section: "Navigation", label: "Zoom out", keys: &["Mod+-"], context: "" },
    H { id: "key.zoomReset", section: "Navigation", label: "Reset zoom", keys: &["Mod+0"], context: "" },
];

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub label: String,
    pub keys: Vec<String>,
    pub context: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: String,
    pub rows: Vec<Row>,
}

const ORDER: [&str; 10] = ["Repo actions", "Navigation", "Edit and search", "Staging", "Commit message", "Diff", "File history", "Merge request", "Rebase editor", "Help"];

fn section_of(group: Group) -> &'static str {
    match group {
        Group::File | Group::Repository => "Repo actions",
        Group::Edit => "Edit and search",
        Group::View => "Navigation",
        Group::Help => "Help",
    }
}

/// `shortcutSections`: every real binding, usable now or not: the panel is the reference.
pub fn sections() -> Vec<Section> {
    let mut by: Vec<(String, Vec<Row>)> = Vec::new();
    let mut add = |section: &str, row: Row| match by.iter_mut().find(|s| s.0 == section) {
        Some(s) => s.1.push(row),
        None => by.push((section.to_string(), vec![row])),
    };
    let mut seen: Vec<String> = Vec::new();
    for c in commands::table().iter() {
        if c.shortcuts.is_empty() {
            continue;
        }
        let key = format!("{}|{}", c.label, c.shortcuts.join(","));
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let section = match c.id.split('.').next().unwrap_or_default() {
            "irebase" => "Rebase editor",
            "stage" => "Staging",
            "commit" => "Commit message",
            "diff" => "Diff",
            "history" => "File history",
            _ => section_of(c.group),
        };
        add(section, Row { id: c.id.to_string(), label: c.label.to_string(), keys: c.shortcuts.iter().map(|s| commands::display_chord(s)).collect(), context: String::new() });
    }
    for h in HINTS {
        add(h.section, Row { id: h.id.to_string(), label: h.label.to_string(), keys: h.keys.iter().map(|k| commands::display_chord(k)).collect(), context: h.context.to_string() });
    }
    let rank = |t: &str| ORDER.iter().position(|o| *o == t).unwrap_or(ORDER.len());
    by.sort_by(|x, y| rank(&x.0).cmp(&rank(&y.0)).then_with(|| x.0.cmp(&y.0)));
    by.into_iter().map(|(title, rows)| Section { title, rows }).collect()
}

/// `filterSections`: rows whose section, label, context or keys contain every word.
pub fn filter(all: &[Section], q: &str) -> Vec<Section> {
    let q = q.to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    if words.is_empty() {
        return all.to_vec();
    }
    all.iter()
        .filter_map(|s| {
            let rows: Vec<Row> = s.rows.iter().filter(|r| {
                let hay = format!("{} {} {} {}", s.title, r.label, r.context, r.keys.join(" ")).to_lowercase();
                words.iter().all(|w| hay.contains(w))
            }).cloned().collect();
            (!rows.is_empty()).then(|| Section { title: s.title.clone(), rows })
        })
        .collect()
}

fn list_view(input: &Entity<TextInput>, window: &Window, cx: &App) -> AnyElement {
    let q = input.read(cx).text().to_string();
    let shown = filter(&sections(), &q);
    let mut body = div().id("sc-body").flex_1().min_h_0().overflow_y_scroll();
    if shown.is_empty() {
        body = body.child(div().px(px(4.)).py(px(16.)).text_color(theme::text_secondary()).child("No shortcuts match."));
    }
    for s in shown {
        body = body.child(div().mt(px(14.)).mb(px(4.)).text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_secondary()).child(s.title.to_uppercase()));
        for r in s.rows {
            let mut keys = div().flex().flex_none().items_center().gap(px(6.));
            for (i, k) in r.keys.iter().enumerate() {
                if i > 0 {
                    keys = keys.child(div().text_size(px(11.)).text_color(theme::text_secondary()).child("or"));
                }
                keys = keys.child(keycaps(k));
            }
            body = body.child(div().flex().items_center().justify_between().gap(px(12.)).px(px(4.)).py(px(5.)).border_b_1().border_color(theme::section_border()).child(div().min_w_0().flex().flex_wrap().gap(px(4.)).child(r.label).when(!r.context.is_empty(), |d| d.child(div().text_size(px(11.)).text_color(theme::text_secondary()).child(r.context)))).child(keys));
        }
    }
    let i2 = input.clone();
    div().flex().flex_col().gap(px(6.)).flex_1().min_h_0().child(div().flex().w(px(220.)).child(controls::clearable("sc-filter", input, move |_, cx| i2.update(cx, |i, cx| i.set_text("", cx)), window, cx))).child(body).into_any_element()
}

impl GitVolt {
    /// Ctrl+/: the panel, or closes it.
    pub fn open_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        const ID: &str = "shortcuts";
        if modal::is_open(ID, cx) {
            return modal::close(ID, window, cx);
        }
        let placeholder: SharedString = format!("Filter shortcuts ({})", commands::display_chord("Mod+F")).into();
        let input = cx.new(|cx| TextInput::new(cx).placeholder(placeholder).claim_escape_with_text());
        let focus = input.read(cx).focus.clone();
        let sub = cx.subscribe(&input, |_, input, e: &crate::ui::input::InputEvent, cx| {
            if matches!(e, crate::ui::input::InputEvent::Escape) {
                input.update(cx, |i, cx| i.set_text("", cx));
            }
            cx.notify();
        });
        let view = input.clone();
        let height = window.viewport_size().height.as_f32() * 0.8;
        let mut m = Modal::new(ID, move |window, cx| {
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .h(px(height - 34.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(modal::title("Keyboard Shortcuts").flex_1().child(icon("keyboard", 16., theme::text_selected())).child(keycaps(&commands::display_chord("Mod+/"))))
                        .child(tip("Close shortcuts", div().id("sc-close").flex().size(px(22.)).items_center().justify_center().rounded(px(3.)).cursor_pointer().hover(|d| d.bg(theme::hover_row())).on_click(|_, window, cx| modal::close(ID, window, cx)).child(icon("x", 14., theme::text_secondary())))),
                )
                .child(list_view(&view, window, cx))
                .into_any_element()
        })
        .width(640.)
        .toggle(commands::resolve_chord("Mod+/"))
        .focus(focus);
        m.keep.push(sub);
        modal::open(m, window, cx);
    }
}

/// Settings › Keyboard's body: the same list, scrolling with the page.
pub fn settings_body(this: &mut GitVolt, window: &mut Window, cx: &mut Context<GitVolt>) -> Vec<AnyElement> {
    let input = this.settings_ui.keyboard_filter.get_or_insert_with(|| {
        let input = cx.new(|cx| TextInput::new(cx).placeholder("Filter shortcuts"));
        cx.subscribe(&input, |_, _, _: &crate::ui::input::InputEvent, cx| cx.notify()).detach();
        input
    }).clone();
    let list = list_view(&input, window, cx);
    vec![div().mt(px(16.)).flex().flex_col().child(list).into_any_element()]
}

#[allow(dead_code)]
fn _keep(_: Rc<()>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn every_chord_and_the_rebase_editors_bare_keys_are_listed() {
        let all = sections();
        assert_eq!(all[0].title, "Repo actions");
        let rebase = all.iter().find(|s| s.title == "Rebase editor").expect("the rebase editor's section");
        assert!(rebase.rows.iter().any(|r| r.keys.iter().any(|k| k.len() == 1)), "its letters: {:?}", rebase.rows);
        assert!(all.iter().flat_map(|s| &s.rows).any(|r| r.id == "file.settings"));
        assert!(all.iter().flat_map(|s| &s.rows).any(|r| r.id == "graph.move"));
    }

    #[test]
    fn a_filter_keeps_rows_with_every_word() {
        let all = sections();
        let only = filter(&all, "rebase move");
        assert!(only.iter().all(|s| s.rows.iter().all(|r| format!("{} {} {}", s.title, r.label, r.context).to_lowercase().contains("move") || r.keys.join(" ").to_lowercase().contains("move"))));
        assert_eq!(filter(&all, "zzz nothing"), []);
        assert_eq!(filter(&all, "").len(), all.len());
    }
}
