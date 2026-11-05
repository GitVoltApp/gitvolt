//! A colour's short transition, as CSS's `transition: color`: what asks for a colour by a key gets where it is on its way there.
//! Nothing runs unless a target changed, and then only for the transition's length.

use gpui::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
pub enum Curve {
    EaseIn,
    EaseOut,
}

struct Entry {
    target: Hsla,
    from: Hsla,
    start: Instant,
    length: Duration,
    curve: Curve,
}

thread_local! {
    static COLOURS: RefCell<HashMap<u64, Entry>> = RefCell::new(HashMap::new());
}

/// Past this many remembered colours, those at rest go: a row off the screen has nothing to ease from.
const KEPT: usize = 4000;

fn lerp(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (x, y) = (Rgba::from(a), Rgba::from(b));
    let m = |p: f32, q: f32| p + (q - p) * t;
    Rgba { r: m(x.r, y.r), g: m(x.g, y.g), b: m(x.b, y.b), a: m(x.a, y.a) }.into()
}

fn value(e: &Entry, now: Instant) -> Hsla {
    let t = ((now - e.start).as_secs_f32() / e.length.as_secs_f32()).min(1.);
    if t >= 1. {
        return e.target;
    }
    let shaped = match e.curve {
        Curve::EaseIn => t * t,
        Curve::EaseOut => 1. - (1. - t) * (1. - t),
    };
    lerp(e.from, e.target, shaped)
}

/// The colour to draw for `key`, going to `target` over `length` once it changes; `running` is set while one is on its way.
pub fn colour(key: u64, target: Hsla, length: Duration, curve: Curve, running: &mut bool) -> Hsla {
    if cfg!(test) {
        return target;
    }
    let now = Instant::now();
    COLOURS.with_borrow_mut(|map| {
        if map.len() > KEPT {
            map.retain(|_, e| now - e.start < e.length);
        }
        let Some(e) = map.get_mut(&key) else {
            map.insert(key, Entry { target, from: target, start: now - length, length, curve });
            return target;
        };
        if e.target != target {
            *e = Entry { from: value(e, now), target, start: now, length, curve };
        }
        let shown = value(e, now);
        if now - e.start < e.length {
            *running = true;
        }
        shown
    })
}

thread_local! {
    static NUMBERS: RefCell<HashMap<u64, (f32, f32, Instant)>> = RefCell::new(HashMap::new());
}

/// A commit's new order: what slid is where it belongs, with no slide back.
pub fn forget(key: u64) {
    NUMBERS.with_borrow_mut(|map| {
        map.remove(&key);
    });
}

/// An offset's slide, as CSS's `transition: transform`, ease-out; what's not asked for lately starts where it's asked.
pub fn number(key: u64, target: f32, length: Duration, running: &mut bool) -> f32 {
    if cfg!(test) {
        return target;
    }
    let now = Instant::now();
    let at = |from: f32, to: f32, start: Instant| {
        let t = ((now - start).as_secs_f32() / length.as_secs_f32()).min(1.);
        from + (to - from) * (1. - (1. - t) * (1. - t))
    };
    NUMBERS.with_borrow_mut(|map| {
        if map.len() > KEPT {
            map.clear();
        }
        let e = map.entry(key).or_insert((target, target, now - length));
        if e.1 != target {
            *e = (at(e.0, e.1, e.2), target, now);
        }
        if now - e.2 < length {
            *running = true;
        }
        at(e.0, e.1, e.2)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn halfway_is_between() {
        let (a, b) = (rgb(0x000000).into(), rgb(0xffffff).into());
        let e = Entry { target: b, from: a, start: Instant::now() - Duration::from_millis(100), length: Duration::from_millis(200), curve: Curve::EaseOut };
        let v: Rgba = value(&e, Instant::now()).into();
        assert!(v.r > 0.6 && v.r < 0.9, "{}", v.r);
    }
}
