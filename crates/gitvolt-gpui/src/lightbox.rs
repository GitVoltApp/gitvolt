//! The image viewer (`ui/src/lightbox/`): one for the app, over everything but tooltips and toasts. It shows the image the
//! element already holds, and fetches nothing. The wheel zooms around the pointer, a drag pans, a click off the picture
//! closes. Dark in every theme, like a photo viewer.

use crate::app::{ClearSelection, SelectNext, SelectPrevious};
use crate::diff::image::{View, ZOOM_STEPS, centered, clamp_view, fit_scale, next_step_index, zoom_around};
use crate::theme;
use crate::ui::Ui;
use crate::ui::tooltip::tip;
use gitvolt_core::api::{Api, Request};
use gpui::prelude::*;
use gpui::*;
use std::sync::Arc;

/// How far an arrow key pans, in screen px.
const PAN_STEP: f32 = 50.;

pub struct Item {
    pub image: Arc<RenderImage>,
    pub w: f32,
    pub h: f32,
    pub browser_url: Option<String>,
    pub api: Arc<Api>,
    pub rt: tokio::runtime::Handle,
}

pub struct State {
    item: Item,
    view: View,
    focus: FocusHandle,
    return_to: Option<FocusHandle>,
    drag: Option<Point<Pixels>>,
    down_on_backdrop: bool,
    /// The wheel's little moves, adding up to a step.
    wheel: f32,
    size: (f32, f32),
}

fn box_size(window: &Window) -> (f32, f32) {
    let v = window.viewport_size();
    (v.width.as_f32(), v.height.as_f32())
}

/// Opens at 100%, or fitted when that's smaller.
pub fn open(item: Item, window: &mut Window, cx: &mut App) {
    crate::ui::tooltip::hide(cx);
    let focus = cx.focus_handle();
    let (bw, bh) = box_size(window);
    let view = centered(fit_scale(item.w, item.h, bw, bh).min(1.), item.w, item.h, bw, bh);
    let return_to = window.focused(cx);
    window.focus(&focus, cx);
    Ui::get_mut(cx).lightbox = Some(Box::new(State { item, view, focus, return_to, drag: None, down_on_backdrop: false, wheel: 0., size: (bw, bh) }));
    window.refresh();
}

pub fn close(window: &mut Window, cx: &mut App) {
    if let Some(s) = Ui::get_mut(cx).lightbox.take() {
        if let Some(back) = s.return_to {
            window.focus(&back, cx);
        }
        window.refresh();
    }
}

fn with(window: &mut Window, cx: &mut App, f: impl FnOnce(&mut State, (f32, f32))) {
    let size = box_size(window);
    if let Some(s) = Ui::get_mut(cx).lightbox.as_mut() {
        f(s, size);
        window.refresh();
    }
}

fn clamp(s: &State, v: View, size: (f32, f32)) -> View {
    clamp_view(v, s.item.w, s.item.h, size.0, size.1)
}

fn fit(window: &mut Window, cx: &mut App) {
    with(window, cx, |s, (w, h)| s.view = centered(fit_scale(s.item.w, s.item.h, w, h), s.item.w, s.item.h, w, h));
}

fn zoom_to(scale: f32, at: Option<(f32, f32)>, window: &mut Window, cx: &mut App) {
    with(window, cx, |s, size| {
        let (x, y) = at.unwrap_or((size.0 / 2., size.1 / 2.));
        s.view = clamp(s, zoom_around(s.view, scale, x, y), size);
    });
}

fn step(up: bool, at: Option<(f32, f32)>, window: &mut Window, cx: &mut App) {
    let Some(scale) = Ui::get(cx).lightbox.as_ref().map(|s| s.view.scale) else { return };
    zoom_to(ZOOM_STEPS[next_step_index(scale, up)], at, window, cx);
}

fn pan(dx: f32, dy: f32, window: &mut Window, cx: &mut App) {
    with(window, cx, |s, size| s.view = clamp(s, View { x: s.view.x + dx, y: s.view.y + dy, ..s.view }, size));
}

fn on_key(e: &KeyDownEvent, window: &mut Window, cx: &mut App) {
    let k = &e.keystroke;
    if k.modifiers.control || k.modifiers.alt || k.modifiers.platform {
        return;
    }
    match k.key.as_str() {
        "0" => fit(window, cx),
        "1" => zoom_to(1., None, window, cx),
        "+" | "=" => step(true, None, window, cx),
        "-" | "_" => step(false, None, window, cx),
        "left" => pan(PAN_STEP, 0., window, cx),
        "right" => pan(-PAN_STEP, 0., window, cx),
        "up" => pan(0., PAN_STEP, window, cx),
        "down" => pan(0., -PAN_STEP, window, cx),
        "escape" => close(window, cx),
        _ => return,
    }
    cx.stop_propagation();
}

fn copy_image(cx: &mut App) {
    let Some((image, rt)) = Ui::get(cx).lightbox.as_ref().map(|s| (s.item.image.clone(), s.item.rt.clone())) else { return };
    let job = rt.spawn_blocking(move || {
        let s = image.size(0);
        let mut bytes = image.as_bytes(0)?.to_vec();
        for px in bytes.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
        let rgba = image::RgbaImage::from_raw(s.width.0 as u32, s.height.0 as u32, bytes)?;
        let mut out = std::io::Cursor::new(Vec::new());
        rgba.write_to(&mut out, image::ImageFormat::Png).ok()?;
        Some(out.into_inner())
    });
    cx.spawn(async move |cx| {
        let made = job.await.ok().flatten();
        cx.update(|cx| match made {
            Some(bytes) => {
                cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(ImageFormat::Png, bytes)));
                crate::ui::toast::show("Image copied", cx);
            }
            None => crate::ui::toast::show_with("Couldn't copy the image", crate::ui::toast::Options { error: true, ..Default::default() }, cx),
        });
    })
    .detach();
}

fn open_browser(cx: &mut App) {
    let Some((url, api, rt)) = Ui::get(cx).lightbox.as_ref().and_then(|s| Some((s.item.browser_url.clone()?, s.item.api.clone(), s.item.rt.clone()))) else { return };
    rt.spawn(async move {
        let _ = api.dispatch(Request::OpenUrl { url }).await;
    });
}

fn text_button(id: &'static str, label: &'static str, tooltip: &'static str, run: impl Fn(&mut Window, &mut App) + 'static) -> AnyElement {
    tip(
        tooltip,
        div()
            .id(id)
            .px(px(8.))
            .py(px(2.))
            .rounded(px(4.))
            .text_color(theme::text_secondary())
            .cursor_pointer()
            .hover(|d| d.bg(theme::hover_row()).text_color(theme::text_selected()))
            .on_click(move |_, window, cx| run(window, cx))
            .child(label),
    )
    .into_any_element()
}

fn icon_button(id: &'static str, name: &'static str, tooltip: &'static str, run: impl Fn(&mut Window, &mut App) + 'static) -> AnyElement {
    tip(
        tooltip,
        div()
            .id(id)
            .flex()
            .flex_none()
            .size(px(22.))
            .items_center()
            .justify_center()
            .rounded(px(3.))
            .cursor_pointer()
            .hover(|d| d.bg(theme::hover_row()))
            .on_click(move |_, window, cx| run(window, cx))
            .child(crate::app::icon(name, 14., theme::text_secondary())),
    )
    .into_any_element()
}

/// The viewer, while it's open.
pub fn element(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let (image, view, w, h, focus, has_url, pannable) = {
        let (bw, bh) = box_size(window);
        let s = Ui::get_mut(cx).lightbox.as_mut()?;
        // The window resized: the same zoom, kept centred and in bounds.
        if s.size != (bw, bh) {
            let v = View { x: s.view.x + (bw - s.size.0) / 2., y: s.view.y + (bh - s.size.1) / 2., ..s.view };
            s.view = clamp_view(v, s.item.w, s.item.h, bw, bh);
            s.size = (bw, bh);
        }
        let pannable = s.item.w * s.view.scale > bw + 0.5 || s.item.h * s.view.scale > bh + 0.5;
        (s.item.image.clone(), s.view, s.item.w, s.item.h, s.focus.clone(), s.item.browser_url.is_some(), pannable)
    };
    let percent: SharedString = format!("{}%", (view.scale * 100.).round() as u32).into();
    let picture = canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let at = Bounds::new(b.origin + point(px(view.x), px(view.y)), size(px(w * view.scale), px(h * view.scale)));
            window.paint_image(b, at, Corners::default(), image.clone(), 0, false).ok();
            if pannable && at.contains(&window.mouse_position()) {
                window.set_window_cursor_style(CursorStyle::OpenHand);
            }
        },
    )
    .absolute()
    .size_full();
    let on_picture = move |p: Point<Pixels>| {
        let (x, y) = (p.x.as_f32(), p.y.as_f32());
        x >= view.x && x < view.x + w * view.scale && y >= view.y && y < view.y + h * view.scale
    };
    let toolbar = div()
        .id("lightbox-toolbar")
        .absolute()
        .top(px(10.))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            div()
                .id("lightbox-bar")
                .occlude()
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(6.))
                .py(px(3.))
                .rounded(px(6.))
                .bg(theme::panel_bg1())
                .border_1()
                .border_color(theme::section_border())
                .shadow(crate::ui::shadow(theme::shadow_popup(), 6., 20.))
                .text_size(px(12.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().min_w(px(36.)).px(px(4.)).flex().justify_center().text_color(theme::text_secondary()).child(percent))
                .child(text_button("lightbox-fit", "Fit", "Fit to the window (0)", fit))
                .child(text_button("lightbox-100", "100%", "Actual size (1)", |window, cx| zoom_to(1., None, window, cx)))
                .child(icon_button("lightbox-copy", "copy", "Copy the image to the clipboard as a PNG", |_, cx| copy_image(cx)))
                .when(has_url, |d| d.child(icon_button("lightbox-open", "external-link", "Open in browser", |_, cx| open_browser(cx))))
                .child(icon_button("lightbox-close", "x", "Close (Esc)", close)),
        );
    Some(
        div()
            .id("lightbox")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .key_context("Lightbox")
            .track_focus(&focus)
            // Theme-free: a dark backdrop in every theme.
            .bg(rgba(0x000000d9))
            .on_key_down(on_key)
            .on_action(|_: &SelectNext, window, cx| pan(0., -PAN_STEP, window, cx))
            .on_action(|_: &SelectPrevious, window, cx| pan(0., PAN_STEP, window, cx))
            .on_action(|_: &ClearSelection, window, cx| close(window, cx))
            .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                let on = on_picture(e.position);
                if let Some(s) = Ui::get_mut(cx).lightbox.as_mut() {
                    s.down_on_backdrop = !on;
                    if on && pannable {
                        s.drag = Some(e.position);
                    }
                }
                cx.stop_propagation();
            })
            .on_mouse_move(|e, window, cx| {
                let Some(last) = Ui::get(cx).lightbox.as_ref().and_then(|s| s.drag) else { return };
                if e.pressed_button == Some(MouseButton::Left) {
                    pan((e.position.x - last.x).as_f32(), (e.position.y - last.y).as_f32(), window, cx);
                    if let Some(s) = Ui::get_mut(cx).lightbox.as_mut() {
                        s.drag = Some(e.position);
                    }
                }
            })
            .on_mouse_up(MouseButton::Left, |_, window, cx| {
                let back = Ui::get_mut(cx).lightbox.as_mut().is_some_and(|s| {
                    s.drag = None;
                    std::mem::take(&mut s.down_on_backdrop)
                });
                if back {
                    close(window, cx);
                }
            })
            .on_scroll(|e, window, cx| {
                let dy = e.delta.pixel_delta(window.line_height()).y.as_f32();
                let at = (e.position.x.as_f32(), e.position.y.as_f32());
                let Some(s) = Ui::get_mut(cx).lightbox.as_mut() else { return };
                s.wheel += dy;
                if s.wheel.abs() < 40. {
                    return;
                }
                let up = s.wheel > 0.;
                s.wheel = 0.;
                step(up, Some(at), window, cx);
                cx.stop_propagation();
            })
            .child(picture)
            .child(toolbar)
            .into_any_element(),
    )
}
