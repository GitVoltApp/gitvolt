//! People's pictures (`ui/src/avatars`): asked of the core once per email, when a row or a panel first
//! shows it; initials until one arrives, and when there's none. Everything here stays on the UI thread.

use crate::app::GitVolt;
use gitvolt_core::api::Request;
use gpui::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
enum Avatar {
    Loading,
    Missing,
    Image(Arc<RenderImage>),
}

thread_local! {
    static STORE: RefCell<HashMap<String, Avatar>> = RefCell::new(HashMap::new());
    static WANTED: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

fn key(email: &str) -> String {
    email.trim().to_lowercase()
}

/// The picture for `email`, if it's here; the first ask queues it for the next frame.
pub fn image_for(email: &str, name: &str) -> Option<Arc<RenderImage>> {
    let k = key(email);
    if k.is_empty() {
        return None;
    }
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        match s.get(&k) {
            Some(Avatar::Image(i)) => Some(i.clone()),
            Some(_) => None,
            None => {
                s.insert(k.clone(), Avatar::Loading);
                WANTED.with(|w| w.borrow_mut().push((k, name.to_string())));
                None
            }
        }
    })
}

/// Whether `bytes` decode; what the core gave is PNG or JPEG.
fn decode(base64_text: &str) -> Option<image::RgbaImage> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(base64_text).ok()?;
    Some(image::load_from_memory(&bytes).ok()?.to_rgba8())
}

impl GitVolt {
    /// What the frame before asked for.
    /// `resetAvatars`: a setting changed what the core answers, so everyone's asked again.
    pub fn reset_avatars(&mut self, cx: &mut Context<Self>) {
        STORE.with(|s| s.borrow_mut().clear());
        cx.notify();
    }

    pub fn fetch_avatars(&mut self, cx: &mut Context<Self>) {
        let wanted = WANTED.with(|w| std::mem::take(&mut *w.borrow_mut()));
        if wanted.is_empty() || cfg!(test) {
            return;
        }
        let repo = self.tabs.get(self.active).map(|t| t.id);
        for (email, name) in wanted {
            let api = self.api.clone();
            let asked = email.clone();
            let done = self.rt.spawn(async move {
                let v = api.dispatch(Request::Avatar { email: asked, repo, name: Some(name) }).await.ok()?;
                decode(v["base64"].as_str()?)
            });
            cx.spawn(async move |this, cx| {
                let picture = done.await.ok().flatten();
                this.update(cx, |_, cx| {
                    let value = picture.map_or(Avatar::Missing, |rgba| Avatar::Image(Arc::new(crate::diff::image::render_image(rgba))));
                    STORE.with(|s| s.borrow_mut().insert(email, value));
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn an_email_is_asked_for_once_in_any_case() {
        assert!(image_for("  Ada@Example.com ", "Ada").is_none());
        assert!(image_for("ada@example.com", "Ada").is_none());
        assert_eq!(WANTED.with(|w| w.borrow().len()), 1);
        assert!(image_for("", "Nobody").is_none());
        assert_eq!(WANTED.with(|w| w.borrow().len()), 1);
    }

    #[test]
    fn what_doesnt_decode_is_no_picture() {
        assert!(decode("not base64!").is_none());
        assert!(decode("aGVsbG8=").is_none());
    }
}
