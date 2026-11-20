//! The keyboard in a flyout and on a review's cards: Tab walks the controls that are in view, Enter or Space presses the one it's on,
//! Esc closes. They're the same controls a pointer presses, by their anchors, so none is left out as rows and states come and go.

use crate::app::GitVolt;
use crate::theme;
use gpui::*;

/// In reading order; a control out of view isn't a stop.
pub fn stops(mut found: Vec<(String, Bounds<Pixels>)>, area: Bounds<Pixels>) -> Vec<String> {
    found.retain(|(_, b)| {
        let c = b.center();
        b.size.width > px(0.) && area.contains(&c)
    });
    found.sort_by(|a, b| (a.1.origin.y.as_f32() / 4.).floor().total_cmp(&(b.1.origin.y.as_f32() / 4.).floor()).then(a.1.origin.x.as_f32().total_cmp(&b.1.origin.x.as_f32())));
    found.dedup_by(|a, b| a.1 == b.1);
    found.into_iter().map(|f| f.0).collect()
}

pub fn step(list: &[String], at: Option<&str>, back: bool) -> Option<String> {
    let n = list.len();
    if n == 0 {
        return None;
    }
    let i = at.and_then(|a| list.iter().position(|k| k == a));
    Some(list[match (i, back) {
        (None, false) => 0,
        (None, true) => n - 1,
        (Some(i), false) => (i + 1) % n,
        (Some(i), true) => (i + n - 1) % n,
    }]
    .clone())
}

impl GitVolt {
    fn flyout_open(&self) -> bool {
        self.tabs.get(self.active).is_some_and(|t| (t.create.is_some() || t.forge.open_mr.is_some()) && t.diff.is_none() && t.history.is_none())
    }

    fn reviewing_in_diff(&self) -> bool {
        self.tabs.get(self.active).is_some_and(|t| t.forge.review.is_some() && t.diff.is_some())
    }

    fn flyout_stops(&self, cx: &App) -> Vec<String> {
        let ui = crate::ui::Ui::get(cx);
        if self.flyout_open() {
            let Some(area) = ui.anchor("flyout-frame") else { return Vec::new() };
            // What's in the frame, and only what's a control: not its head's title, not the frame itself.
            let all: Vec<(String, Bounds<Pixels>)> = ui.anchors_starting(&["mr-", "create-", "fork-", "side:"]).into_iter().filter(|(k, _)| !k.starts_with("side:")).collect();
            return stops(all, area);
        }
        if self.reviewing_in_diff() {
            let Some(area) = ui.anchor("diff-area") else { return stops(ui.anchors_starting(&["rv-"]), Bounds::new(point(px(0.), px(0.)), size(px(100_000.), px(100_000.)))) };
            return stops(ui.anchors_starting(&["rv-"]), area);
        }
        Vec::new()
    }

    /// Tab, Enter and Space, when no field has the keys.
    pub fn flyout_key(&mut self, k: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.flyout_open() && !self.reviewing_in_diff() {
            return false;
        }
        match k.key.as_str() {
            "tab" if !k.modifiers.control && !k.modifiers.alt => {
                let list = self.flyout_stops(cx);
                self.flyout_kb = step(&list, self.flyout_kb.as_deref(), k.modifiers.shift);
                window.refresh();
                self.flyout_kb.is_some()
            }
            "enter" | "space" if !k.modifiers.modified() => {
                let Some(key) = self.flyout_kb.clone().filter(|key| self.flyout_stops(cx).contains(key)) else { return false };
                let Some(b) = crate::ui::Ui::get(cx).anchor(&key) else { return false };
                press_at(b.center(), self.window, cx);
                true
            }
            "pagedown" | "pageup" if self.flyout_open() => {
                let Some(t) = self.tabs.get(self.active) else { return false };
                let handle = if let Some(u) = &t.create { u.scroll.clone() } else { t.forge.open_mr.and_then(|n| t.mr_views.get(&n)).map_or_else(ScrollHandle::new, |u| u.scroll.clone()) };
                let page = handle.bounds().size.height * 0.9;
                let o = handle.offset();
                let y = if k.key == "pagedown" { o.y - page } else { o.y + page };
                handle.set_offset(point(o.x, y.clamp(-handle.max_offset().y, px(0.))));
                window.refresh();
                cx.notify();
                true
            }
            _ => false,
        }
    }

    /// Esc: a flyout's first; what it holds open goes before it does.
    pub fn flyout_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.flyout_open() {
            return false;
        }
        self.flyout_kb = None;
        let Some(t) = self.tabs.get(self.active) else { return false };
        let tab = t.id;
        if t.create.is_some() {
            self.forge_close_create(tab, cx);
        } else if let Some(n) = t.forge.open_mr {
            self.mr_escape(tab, n, window, cx);
        }
        true
    }

    /// The ring on the control Tab is on: over everything, taking no presses.
    pub fn flyout_ring(&self, cx: &App) -> Option<AnyElement> {
        let key = self.flyout_kb.as_deref().filter(|_| self.flyout_open() || self.reviewing_in_diff())?;
        let b = crate::ui::Ui::get(cx).anchor(key)?;
        Some(deferred(div().absolute().left(b.origin.x - px(2.)).top(b.origin.y - px(2.)).w(b.size.width + px(4.)).h(b.size.height + px(4.)).rounded(px(5.)).border_2().border_color(theme::focus_outline())).with_priority(5).into_any_element())
    }
}

/// A press, as a pointer makes it: the control can't tell.
fn press_at(position: Point<Pixels>, handle: Option<AnyWindowHandle>, cx: &mut Context<GitVolt>) {
    let events = vec![
        PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() }),
        PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position, modifiers: Modifiers::default(), click_count: 1, first_mouse: false }),
        PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position, modifiers: Modifiers::default(), click_count: 1 }),
    ];
    let Some(handle) = handle else { return };
    cx.spawn(async move |_, cx| {
        for e in events {
            handle.update(cx, |_, window, cx| window.dispatch_event(e, cx)).ok();
            cx.background_executor().timer(std::time::Duration::from_millis(16)).await;
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn b(x: f32, y: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(20.), px(20.)))
    }

    #[test]
    fn stops_go_in_reading_order_and_only_in_view() {
        let area = Bounds::new(point(px(0.), px(0.)), size(px(100.), px(100.)));
        let found = vec![("c".to_string(), b(10., 50.)), ("a".to_string(), b(40., 10.)), ("b".to_string(), b(10., 11.)), ("off".to_string(), b(10., 500.))];
        assert_eq!(stops(found, area), ["b", "a", "c"]);
    }

    #[test]
    fn tab_wraps_both_ways() {
        let l: Vec<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        assert_eq!(step(&l, None, false).as_deref(), Some("a"));
        assert_eq!(step(&l, None, true).as_deref(), Some("b"));
        assert_eq!(step(&l, Some("b"), false).as_deref(), Some("a"));
        assert_eq!(step(&l, Some("a"), true).as_deref(), Some("b"));
        assert_eq!(step(&[], Some("a"), true), None);
    }
}
