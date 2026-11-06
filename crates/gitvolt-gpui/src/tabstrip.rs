//! The tab strip (`TabBar.tsx`, `useTabDrag.ts`, `tabGroups.ts`): a drag previews the strip a release would give, the others
//! sliding aside over 150 ms; a tab goes into a group, out of one, or onto another to make one; a group moves by its chip. The
//! strip scrolls under the wheel; a chip's rest shows its tabs; Rename is the inline field.
//!
//! What's pure here works on `Strip`, the tabs' ids and their groups, as the profile holds them.

use crate::app::{GitVolt, icon};
use crate::theme;
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::menu;
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const ONTO_EDGE: f32 = 0.3;
pub const ZONE: f32 = 0.4;
pub const ZONE_MAX: f32 = 44.;
pub const HYSTERESIS: f32 = 10.;
const THRESHOLD: f32 = 4.;
const SLIDE: Duration = Duration::from_millis(150);
/// `GROUP_LIST_DELAY_MS`, the strip's hover delay.
const LIST_DELAY: Duration = Duration::from_millis(400);
const LEAVE_GRACE: Duration = Duration::from_millis(120);

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Tab(Option<String>),
    Chip(String),
    /// A collapsed group: passed whole, never entered.
    Block,
}

#[derive(Debug, Clone)]
pub struct DragBox {
    pub kind: Kind,
    pub left: f32,
    pub width: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TabDrop {
    Gap { gap: usize, group: Option<String> },
    Onto(usize),
}

#[derive(Debug, Clone)]
struct Step {
    x: f32,
    drop: TabDrop,
}

struct Steps {
    right: Vec<Step>,
    left: Vec<Step>,
    zone: f32,
    edges: Vec<(TabDrop, bool)>,
}

fn is_tab_of(others: &[(DragBox, usize)], j: usize, g: &str) -> bool {
    others.get(j).is_some_and(|(o, _)| matches!(&o.kind, Kind::Tab(Some(x)) if x == g))
}

/// A slot at a group's edge: its first (after its chip) or its last, or the one just outside it.
fn at_edge(others: &[(DragBox, usize)], d: &TabDrop) -> bool {
    let TabDrop::Gap { gap, group } = d else { return false };
    let prev = gap.checked_sub(1).and_then(|p| others.get(p));
    if let Some(g) = group {
        return prev.is_some_and(|(p, _)| matches!(&p.kind, Kind::Chip(c) if c == g)) || !is_tab_of(others, *gap, g);
    }
    prev.is_some_and(|(p, _)| matches!(&p.kind, Kind::Tab(Some(_)))) || others.get(*gap).is_some_and(|(n, _)| matches!(n.kind, Kind::Chip(_)))
}

fn tab_steps(items: &[DragBox], from: usize) -> Steps {
    let dragged = &items[from];
    let w = dragged.width;
    let own = if let Kind::Tab(g) = &dragged.kind { g.clone() } else { None };
    let others: Vec<(DragBox, usize)> = items.iter().enumerate().filter(|(i, _)| *i != from).map(|(i, b)| (DragBox { left: if i > from { b.left - w } else { b.left }, ..b.clone() }, i)).collect();
    let zone = (ZONE * w).min(ZONE_MAX);
    let min_of = |d: &TabDrop| if at_edge(&others, d) { zone } else { 2. * HYSTERESIS };
    let rest_min = if at_edge(&others, &TabDrop::Gap { gap: from, group: own.clone() }) { zone } else { 0. };

    let mut right = Vec::new();
    let mut last_x = dragged.left;
    let mut min = rest_min;
    let mut k = from;
    let mut grp = own.clone();
    macro_rules! push_right {
        ($x:expr, $d:expr) => {{
            let d: TabDrop = $d;
            last_x = ($x as f32).max(last_x + min);
            min = min_of(&d);
            right.push(Step { x: last_x, drop: d });
        }};
    }
    loop {
        if let Some(g) = grp.clone()
            && !is_tab_of(&others, k, &g)
        {
            let Some((last, _)) = k.checked_sub(1).and_then(|p| others.get(p)) else { break };
            push_right!(last.left + last.width, TabDrop::Gap { gap: k, group: None });
            grp = None;
            continue;
        }
        let Some((next, index)) = others.get(k) else { break };
        match &next.kind {
            Kind::Tab(None) => {
                push_right!(next.left + next.width * ONTO_EDGE, TabDrop::Onto(*index));
                push_right!(next.left + next.width * (1. - ONTO_EDGE), TabDrop::Gap { gap: k + 1, group: None });
            }
            kind => {
                grp = match kind {
                    Kind::Block => None,
                    Kind::Chip(g) => Some(g.clone()),
                    Kind::Tab(g) => g.clone(),
                };
                push_right!(next.left + next.width / 2., TabDrop::Gap { gap: k + 1, group: grp.clone() });
            }
        }
        k += 1;
    }

    let mut left = Vec::new();
    let mut last_x = dragged.left;
    let mut min = rest_min;
    let mut k = from;
    let mut grp = own;
    macro_rules! push_left {
        ($x:expr, $d:expr) => {{
            let d: TabDrop = $d;
            last_x = ($x as f32).min(last_x - min);
            min = min_of(&d);
            left.push(Step { x: last_x, drop: d });
        }};
    }
    while let Some((prev, index)) = k.checked_sub(1).and_then(|p| others.get(p)) {
        if let Kind::Tab(Some(g)) = &prev.kind
            && Some(g) != grp.as_ref()
        {
            grp = Some(g.clone());
            push_left!(prev.left + prev.width * (1. - ONTO_EDGE), TabDrop::Gap { gap: k, group: grp.clone() });
            continue;
        }
        match &prev.kind {
            Kind::Chip(g) if Some(g) == grp.as_ref() => {
                grp = None;
                push_left!(prev.left, TabDrop::Gap { gap: k - 1, group: None });
            }
            Kind::Tab(None) => {
                push_left!(prev.left + prev.width * (1. - ONTO_EDGE), TabDrop::Onto(*index));
                push_left!(prev.left + prev.width * ONTO_EDGE, TabDrop::Gap { gap: k - 1, group: None });
            }
            kind => {
                if !matches!(kind, Kind::Tab(_)) {
                    grp = None;
                }
                push_left!(prev.left + prev.width / 2., TabDrop::Gap { gap: k - 1, group: grp.clone() });
            }
        }
        k -= 1;
    }
    let edges = right.iter().chain(left.iter()).map(|s| (s.drop.clone(), at_edge(&others, &s.drop))).collect();
    Steps { right, left, zone, edges }
}

/// Where a tab dragged from `items[from]` by `dx` lands; `held`, where it was previewed last, stays until `dx` is `HYSTERESIS` past its zone.
pub fn tab_drop_at(items: &[DragBox], from: usize, dx: f32, held: Option<&TabDrop>) -> TabDrop {
    let steps = tab_steps(items, from);
    let b = &items[from];
    let own = if let Kind::Tab(g) = &b.kind { g.clone() } else { None };
    let at = |d: f32| {
        let x = b.left + d;
        let mut drop = TabDrop::Gap { gap: from, group: own.clone() };
        if d > 0. {
            for s in &steps.right {
                if x > s.x {
                    drop = s.drop.clone();
                } else {
                    break;
                }
            }
        }
        if d < 0. {
            for s in &steps.left {
                if x < s.x {
                    drop = s.drop.clone();
                } else {
                    break;
                }
            }
        }
        drop
    };
    let drop = at(dx);
    let Some(held) = held.filter(|h| **h != drop) else { return drop };
    if at(dx - HYSTERESIS) == *held || at(dx + HYSTERESIS) == *held { held.clone() } else { drop }
}

/// How far a tab dragged from `items[from]` can go.
pub fn tab_drag_range(items: &[DragBox], from: usize, strip_left: f32, strip_right: f32) -> (f32, f32) {
    let steps = tab_steps(items, from);
    let b = &items[from];
    let reach = |s: &Step| if steps.edges.iter().any(|(d, e)| *d == s.drop && *e) { steps.zone / 2. } else { 1. };
    let lo = strip_left.min(steps.left.last().map_or(f32::INFINITY, |l| l.x - reach(l))) - b.left;
    let hi = (strip_right - b.width).max(steps.right.last().map_or(f32::NEG_INFINITY, |r| r.x + reach(r))) - b.left;
    (lo.min(0.), hi.max(0.))
}

/// A unit moving as a block among others passes each at its midpoint, by its leading edge.
pub fn target_index(lefts: &[f32], widths: &[f32], from: usize, dx: f32) -> usize {
    let mut to = from;
    if dx > 0. {
        let edge = lefts[from] + widths[from] + dx;
        for j in from + 1..lefts.len() {
            if edge > lefts[j] + widths[j] * 0.5 {
                to = j;
            } else {
                break;
            }
        }
    } else if dx < 0. {
        let edge = lefts[from] + dx;
        for j in (0..from).rev() {
            if edge < lefts[j] + widths[j] * 0.5 {
                to = j;
            } else {
                break;
            }
        }
    }
    to
}

/// The tabs and their groups, as the profile holds them.
#[derive(Clone, Debug, PartialEq)]
pub struct Strip {
    pub tabs: Vec<String>,
    pub groups: Vec<Value>,
}

#[derive(Clone, Debug)]
pub enum Place {
    Before(String),
    After(String),
}

impl Strip {
    fn group_of(&self, tab: &str) -> Option<&Value> {
        self.groups.iter().find(|g| g["tabs"].as_array().is_some_and(|t| t.iter().any(|x| x == tab)))
    }

    fn group(&self, id: &str) -> Option<&Value> {
        self.groups.iter().find(|g| g["id"] == id)
    }

    fn members(&self, id: &str) -> Vec<String> {
        self.group(id).and_then(|g| g["tabs"].as_array()).map_or(Vec::new(), |t| t.iter().filter_map(|x| x.as_str().map(String::from)).collect())
    }

    fn normalized(mut self) -> Self {
        crate::tabs::normalize_groups(&mut self.groups, &self.tabs);
        self.tabs = crate::tabs::contiguous(&self.tabs, &self.groups);
        crate::tabs::normalize_groups(&mut self.groups, &self.tabs);
        self
    }

    fn leave(&mut self, tab: &str) {
        for g in &mut self.groups {
            if let Some(t) = g["tabs"].as_array_mut() {
                t.retain(|x| x != tab);
            }
        }
    }

    fn move_to(&mut self, tab: &str, at: &Place) {
        let Some(from) = self.tabs.iter().position(|x| x == tab) else { return };
        let moved = self.tabs.remove(from);
        let reference = match at {
            Place::Before(r) | Place::After(r) => self.tabs.iter().position(|x| x == r),
        };
        let Some(r) = reference else {
            self.tabs.insert(from, moved);
            return;
        };
        self.tabs.insert(if matches!(at, Place::Before(_)) { r } else { r + 1 }, moved);
    }

    fn join(&mut self, tab: &str, id: &str) {
        self.leave(tab);
        if let Some(t) = self.groups.iter_mut().find(|g| g["id"] == id).and_then(|g| g["tabs"].as_array_mut()) {
            t.push(json!(tab));
        }
    }

    /// `placeTab`: dragged to `at`, in `group`.
    pub fn place_tab(&self, tab: &str, at: Option<&Place>, group: Option<&str>) -> Self {
        let mut q = self.clone();
        if let Some(at) = at {
            q.move_to(tab, at);
        }
        if !q.tabs.iter().any(|x| x == tab) {
            return self.clone();
        }
        match group.filter(|g| q.group(g).is_some()) {
            Some(g) => q.join(tab, g),
            None => q.leave(tab),
        }
        q.normalized()
    }

    /// `groupOnto`: beside the target on the side it came from, in its group, or a new one of the two.
    pub fn group_onto(&self, tab: &str, target: &str, id: &str, color: &str) -> Self {
        let (Some(from), Some(to)) = (self.tabs.iter().position(|x| x == tab), self.tabs.iter().position(|x| x == target)) else { return self.clone() };
        if from == to {
            return self.clone();
        }
        let mut q = self.clone();
        q.move_to(tab, &if from < to { Place::Before(target.to_string()) } else { Place::After(target.to_string()) });
        q.leave(tab);
        q.groups.retain(|g| g["tabs"].as_array().is_some_and(|t| !t.is_empty()));
        if let Some(g) = q.group_of(target).map(|g| g["id"].as_str().unwrap_or_default().to_string()) {
            q.join(tab, &g);
            return q.normalized();
        }
        q.groups.push(json!({ "id": id, "name": "", "color": color, "collapsed": false, "tabs": [tab, target] }));
        q.normalized()
    }

    /// `moveGroup`: a whole group, past another whole.
    pub fn move_group(&self, id: &str, at: &Place) -> Self {
        let mine = self.members(id);
        let (Place::Before(reference) | Place::After(reference)) = at;
        if mine.is_empty() || mine.contains(reference) {
            return self.clone();
        }
        let other = self.group_of(reference).map(|g| g["id"].as_str().unwrap_or_default().to_string()).map(|g| self.members(&g));
        let before = matches!(at, Place::Before(_));
        let target = match &other {
            Some(o) if before => o.first().cloned(),
            Some(o) => o.last().cloned(),
            None => Some(reference.clone()),
        };
        let rest: Vec<String> = self.tabs.iter().filter(|t| !mine.contains(t)).cloned().collect();
        let Some(r) = target.and_then(|t| rest.iter().position(|x| *x == t)) else { return self.clone() };
        let i = if before { r } else { r + 1 };
        let moving: Vec<String> = self.tabs.iter().filter(|t| mine.contains(t)).cloned().collect();
        let tabs = rest[..i].iter().chain(&moving).chain(&rest[i..]).cloned().collect();
        Self { tabs, groups: self.groups.clone() }.normalized()
    }

    /// `stripItems`: each group's chip before its first tab, what a collapsed group hides left out.
    pub fn items(&self, active: Option<&str>) -> Vec<String> {
        let mut out = Vec::new();
        for t in &self.tabs {
            let g = self.group_of(t);
            if let Some(g) = g
                && g["tabs"][0] == t.as_str()
            {
                out.push(format!("chip:{}", g["id"].as_str().unwrap_or_default()));
            }
            if g.is_some_and(|g| g["collapsed"] == true) && active != Some(t.as_str()) {
                continue;
            }
            out.push(format!("tab:{t}"));
        }
        out
    }
}

/// What a release at some offset gives.
#[derive(Clone, Debug)]
pub struct Preview {
    pub order: Vec<String>,
    pub color: Option<String>,
    pub onto: Option<String>,
    pub result: Option<Strip>,
}

#[derive(Clone)]
enum Plan {
    Tab { id: String, boxes: Vec<DragBox>, from: usize, entries: Vec<(Option<String>, Option<String>)> },
    Group { id: String, firsts: Vec<(String, String)>, from: usize, lefts: Vec<f32>, widths: Vec<f32> },
}

struct Slide {
    from: f32,
    to: f32,
    start: Instant,
}

impl Slide {
    fn now(&self, at: Instant) -> f32 {
        let t = (at - self.start).as_secs_f32() / SLIDE.as_secs_f32();
        if t >= 1. {
            return self.to;
        }
        // CSS's ease-out, near enough.
        let e = 1. - (1. - t) * (1. - t);
        self.from + (self.to - self.from) * e
    }
    fn running(&self, at: Instant) -> bool {
        at - self.start < SLIDE
    }
}

pub struct StripDrag {
    key: String,
    moving: Vec<String>,
    press_x: f32,
    active: bool,
    dx: f32,
    range: (f32, f32),
    strip: Strip,
    plan: Option<Plan>,
    lefts: HashMap<String, f32>,
    widths: HashMap<String, f32>,
    start: f32,
    held: Option<TabDrop>,
    preview: Option<Preview>,
    slides: HashMap<String, Slide>,
    /// Released: what's lifted slides to its slot, then the strip takes `result`.
    settle: Option<(Slide, Option<Strip>)>,
    /// The tab's id, when it's the tab that was pressed: a click activates it.
    click: Option<usize>,
}

fn offsets(order: &[String], lefts: &HashMap<String, f32>, widths: &HashMap<String, f32>, start: f32) -> HashMap<String, f32> {
    let mut out = HashMap::new();
    let mut x = start;
    for k in order {
        let (Some(l), Some(w)) = (lefts.get(k), widths.get(k)) else { continue };
        out.insert(k.clone(), x - l);
        x += w;
    }
    out
}

#[derive(Default)]
pub struct StripState {
    pub drag: Option<StripDrag>,
    pub handle: ScrollHandle,
    /// The tab being renamed, by its id, and its field.
    pub renaming: Option<(u32, Entity<TextInput>)>,
    /// The chip whose list of tabs shows.
    pub list: Option<String>,
    list_gen: u64,
    revealed: Option<u32>,
}

impl GitVolt {
    fn strip(&self) -> Strip {
        Strip { tabs: self.tabs.iter().map(|t| t.state_id.clone()).collect(), groups: self.profile["tabGroups"].as_array().cloned().unwrap_or_default() }
    }

    /// A press on a tab (`click`: its index) or a chip: a drag if it goes on past the threshold.
    fn strip_press(&mut self, key: String, click: Option<usize>, x: f32) {
        self.strip_ui.drag = Some(StripDrag { key: key.clone(), moving: vec![key], press_x: x, active: false, dx: 0., range: (0., 0.), strip: self.strip(), plan: None, lefts: HashMap::new(), widths: HashMap::new(), start: 0., held: None, preview: None, slides: HashMap::new(), settle: None, click });
    }

    /// What the strip is when a drag starts, measured from where it was last drawn.
    fn strip_begin(&mut self, cx: &mut Context<Self>) -> bool {
        let active_id = self.tabs.get(self.active).map(|t| t.state_id.clone());
        let ui = crate::ui::Ui::get(cx);
        let Some(d) = self.strip_ui.drag.as_mut() else { return false };
        let items = d.strip.items(active_id.as_deref());
        for k in &items {
            let Some(b) = ui.anchor(&format!("strip:{k}")) else { return false };
            d.lefts.insert(k.clone(), b.left().as_f32());
            d.widths.insert(k.clone(), b.size.width.as_f32());
        }
        d.start = items.first().and_then(|k| d.lefts.get(k)).copied().unwrap_or(0.);
        let Some(view) = ui.anchor("strip-view") else { return false };
        let (strip_left, strip_right) = (view.left().as_f32(), view.right().as_f32());
        let content_right = items.last().map_or(strip_right, |k| d.lefts[k] + d.widths[k]);
        let group_tab = |s: &Strip, t: &str| s.group_of(t).map(|g| g["id"].as_str().unwrap_or_default().to_string());
        if let Some(gid) = d.key.strip_prefix("chip:").map(String::from) {
            let mut units: Vec<Vec<String>> = Vec::new();
            let mut firsts = Vec::new();
            let mut from = 0;
            for t in &d.strip.tabs {
                let key = format!("tab:{t}");
                match group_tab(&d.strip, t) {
                    Some(g) => {
                        let tabs = d.strip.members(&g);
                        if tabs.first() != Some(t) {
                            if items.contains(&key)
                                && let Some(u) = units.last_mut()
                            {
                                u.push(key);
                            }
                            continue;
                        }
                        if g == gid {
                            from = units.len();
                        }
                        let mut unit = vec![format!("chip:{g}")];
                        if items.contains(&key) {
                            unit.push(key);
                        }
                        units.push(unit);
                        firsts.push((t.clone(), tabs.last().cloned().unwrap_or_else(|| t.clone())));
                    }
                    None => {
                        units.push(vec![key]);
                        firsts.push((t.clone(), t.clone()));
                    }
                }
            }
            let lefts: Vec<f32> = units.iter().map(|u| d.lefts[&u[0]]).collect();
            let widths: Vec<f32> = units.iter().map(|u| u.iter().map(|k| d.widths[k]).sum()).collect();
            let n = lefts.len();
            let lo = strip_left.max(lefts[0]) - lefts[from];
            let hi = strip_right.min(lefts[n - 1] + widths[n - 1]) - (lefts[from] + widths[from]);
            d.range = (lo.min(0.), hi.max(0.));
            d.moving = units[from].clone();
            d.plan = Some(Plan::Group { id: gid, firsts, from, lefts, widths });
            return true;
        }
        let Some(tab_id) = d.key.strip_prefix("tab:").map(String::from) else { return false };
        let own = group_tab(&d.strip, &tab_id);
        let mut entries: Vec<(Vec<String>, Kind, Option<String>)> = Vec::new();
        for k in &items {
            if let Some(g) = k.strip_prefix("chip:") {
                let collapsed = d.strip.group(g).is_some_and(|x| x["collapsed"] == true);
                let whole = collapsed && own.as_deref() != Some(g);
                entries.push((vec![k.clone()], if whole { Kind::Block } else { Kind::Chip(g.to_string()) }, None));
            } else if let Some(t) = k.strip_prefix("tab:") {
                let g = group_tab(&d.strip, t);
                let whole = g.as_ref().is_some_and(|x| d.strip.group(x).is_some_and(|x| x["collapsed"] == true)) && g != own;
                if whole {
                    if let Some(e) = entries.last_mut() {
                        e.0.push(k.clone());
                    }
                } else {
                    entries.push((vec![k.clone()], Kind::Tab(g), Some(t.to_string())));
                }
            }
        }
        let Some(from) = entries.iter().position(|e| e.2.as_deref() == Some(&tab_id)) else { return false };
        let boxes: Vec<DragBox> = entries.iter().map(|(keys, kind, _)| DragBox { kind: kind.clone(), left: d.lefts[&keys[0]], width: keys.iter().map(|k| d.widths[k]).sum() }).collect();
        d.range = tab_drag_range(&boxes, from, strip_left, strip_right.max(content_right));
        let plan_entries = entries.iter().map(|(_, kind, tab)| (tab.clone(), match kind { Kind::Tab(g) => g.clone(), Kind::Chip(g) => Some(g.clone()), Kind::Block => None })).collect();
        d.plan = Some(Plan::Tab { id: tab_id, boxes, from, entries: plan_entries });
        true
    }

    pub fn on_strip_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(d) = self.strip_ui.drag.as_ref() else { return };
        if d.settle.is_some() {
            return;
        }
        if e.pressed_button != Some(MouseButton::Left) {
            self.strip_ui.drag = None;
            cx.notify();
            return;
        }
        let dx = e.position.x.as_f32() - d.press_x;
        if !d.active {
            if dx.abs() < THRESHOLD {
                return;
            }
            if !self.strip_begin(cx) {
                self.strip_ui.drag = None;
                return;
            }
        }
        let active_id = self.tabs.get(self.active).map(|t| t.state_id.clone());
        let new_group = (crate::tabs::new_tab_state_id(), self.next_group_color().to_string());
        let Some(d) = self.strip_ui.drag.as_mut() else { return };
        d.active = true;
        d.dx = dx.clamp(d.range.0, d.range.1);
        let preview = d.locate(active_id.as_deref(), &new_group);
        let changed = d.preview.as_ref().is_none_or(|p| p.order != preview.order);
        if changed {
            let now = Instant::now();
            let to = offsets(&preview.order, &d.lefts, &d.widths, d.start);
            let from_now: HashMap<String, f32> = d.slides.iter().map(|(k, s)| (k.clone(), s.now(now))).collect();
            d.slides = to.into_iter().map(|(k, to)| { let from = from_now.get(&k).copied().unwrap_or(0.); (k, Slide { from, to, start: now }) }).collect();
        }
        d.preview = Some(preview);
        cx.notify();
    }

    /// Esc during a drag that's begun: everything slides back.
    pub fn cancel_strip_drag(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(d) = self.strip_ui.drag.as_mut() else { return false };
        if !d.active || d.settle.is_some() {
            return false;
        }
        let now = Instant::now();
        let from_now: HashMap<String, f32> = d.slides.iter().map(|(k, s)| (k.clone(), s.now(now))).collect();
        d.slides = from_now.into_iter().map(|(k, from)| (k, Slide { from, to: 0., start: now })).collect();
        d.settle = Some((Slide { from: d.dx, to: 0., start: now }, None));
        d.preview = None;
        cx.notify();
        true
    }

    /// The release: a click on what wasn't dragged; what's dragged slides into its slot, and the strip takes the order after that.
    pub fn on_strip_release(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.strip_ui.drag.as_mut() else { return };
        if d.settle.is_some() {
            return;
        }
        if !d.active {
            let (click, key) = (d.click, d.key.clone());
            self.strip_ui.drag = None;
            match click {
                Some(i) => {
                    self.active = i;
                    self.save_tabs(cx);
                }
                None => {
                    if let Some(id) = key.strip_prefix("chip:") {
                        self.toggle_tab_group(id, cx);
                    }
                }
            }
            cx.notify();
            return;
        }
        let order_offset = d.preview.as_ref().map(|p| offsets(&p.order, &d.lefts, &d.widths, d.start)).unwrap_or_default();
        let to = order_offset.get(&d.key).copied().unwrap_or(0.);
        let result = d.preview.as_ref().and_then(|p| p.result.clone());
        d.settle = Some((Slide { from: d.dx, to, start: Instant::now() }, result));
        cx.notify();
    }

    fn finish_strip_drag(&mut self, result: Option<Strip>, cx: &mut Context<Self>) {
        self.strip_ui.drag = None;
        let Some(s) = result else { return };
        let shown = self.tabs.get(self.active).map(|t| t.id);
        self.tabs.sort_by_key(|t| s.tabs.iter().position(|x| *x == t.state_id));
        self.active = shown.and_then(|id| self.tabs.iter().position(|t| t.id == id)).unwrap_or(self.active);
        let groups = s.groups;
        self.update_profile(move |p| p["tabGroups"] = json!(groups), cx);
        self.save_tabs(cx);
        cx.notify();
    }

    fn toggle_tab_group(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = id.to_string();
        self.edit_groups(move |g| {
            if let Some(x) = g.iter_mut().find(|x| x["id"] == id.as_str()) {
                x["collapsed"] = json!(x["collapsed"] != true);
            }
        }, cx);
    }

    /// Rename, in place: `RenameInput`.
    pub fn start_tab_rename(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        let id = tab.id;
        let alias = tab.alias.clone().unwrap_or_default();
        let input = cx.new(|cx| {
            let mut i = TextInput::new(cx).claim_escape();
            i.set_text(&alias, cx);
            i
        });
        input.update(cx, |i, cx| i.select_all_text(cx));
        cx.subscribe(&input, move |this, input, e: &InputEvent, cx| match e {
            InputEvent::Submit { .. } | InputEvent::Blurred => this.end_tab_rename(id, Some(input.read(cx).text().to_string()), cx),
            InputEvent::Escape => this.end_tab_rename(id, None, cx),
            _ => {}
        })
        .detach();
        window.focus(&input.read(cx).focus_handle(cx), cx);
        self.strip_ui.renaming = Some((id, input));
        cx.notify();
    }

    fn end_tab_rename(&mut self, id: u32, value: Option<String>, cx: &mut Context<Self>) {
        if self.strip_ui.renaming.as_ref().is_none_or(|(r, _)| *r != id) {
            return;
        }
        self.strip_ui.renaming = None;
        self.focus_next_frame = Some(self.focus.clone());
        if let (Some(value), Some(tab)) = (value, self.tabs.iter_mut().find(|t| t.id == id)) {
            let name = value.trim().to_string();
            tab.alias = (!name.is_empty()).then(|| name.clone());
            tab.name = if name.is_empty() { tab.repo_name.clone() } else { name.into() };
            self.save_tabs(cx);
        }
        cx.notify();
    }

    fn chip_hover(&mut self, id: String, on: bool, cx: &mut Context<Self>) {
        self.strip_ui.list_gen += 1;
        let at = self.strip_ui.list_gen;
        let wait = if on { LIST_DELAY } else { LEAVE_GRACE };
        if on && self.strip_ui.list.as_deref() == Some(id.as_str()) {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                if this.strip_ui.list_gen != at {
                    return;
                }
                if on {
                    if this.strip_ui.drag.is_none() {
                        this.strip_ui.list = Some(id);
                    }
                } else {
                    this.strip_ui.list = None;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl StripDrag {
    fn locate(&mut self, active: Option<&str>, new_group: &(String, String)) -> Preview {
        let keys = |s: &Strip| s.items(active);
        let color_of = |s: &Strip, t: &str| s.group_of(t).and_then(|g| g["color"].as_str().map(String::from));
        let group_of = |s: &Strip, t: &str| s.group_of(t).and_then(|g| g["id"].as_str().map(String::from));
        let strip = self.strip.clone();
        match self.plan.clone() {
            Some(Plan::Tab { id, boxes, from, entries }) => {
                let drop = tab_drop_at(&boxes, from, self.dx, self.held.as_ref());
                self.held = Some(drop.clone());
                match drop {
                    TabDrop::Onto(j) => {
                        let target = entries[j].0.clone().unwrap_or_default();
                        let q = strip.group_onto(&id, &target, &new_group.0, &new_group.1);
                        Preview { order: keys(&q), color: color_of(&q, &id), onto: Some(format!("tab:{target}")), result: Some(q) }
                    }
                    TabDrop::Gap { gap, group } => {
                        let own = group_of(&strip, &id);
                        if gap == from && group == own {
                            return Preview { order: keys(&strip), color: color_of(&strip, &id), onto: None, result: None };
                        }
                        let others: Vec<&(Option<String>, Option<String>)> = entries.iter().enumerate().filter(|(i, _)| *i != from).map(|(_, e)| e).collect();
                        let prev = gap.checked_sub(1).and_then(|p| others.get(p));
                        let next = others.get(gap);
                        let members = |g: &str| strip.members(g).into_iter().filter(|t| *t != id).collect::<Vec<_>>();
                        let at = if group.is_some() {
                            match (prev.and_then(|p| p.0.clone()), next.and_then(|n| n.0.clone())) {
                                (Some(p), _) => Some(Place::After(p)),
                                (None, Some(n)) => Some(Place::Before(n)),
                                _ => None,
                            }
                        } else if let Some(g) = prev.and_then(|p| p.1.clone()) {
                            members(&g).last().cloned().map(Place::After)
                        } else if let Some(p) = prev.and_then(|p| p.0.clone()) {
                            Some(Place::After(p))
                        } else {
                            next.and_then(|n| n.0.clone().or_else(|| n.1.as_ref().and_then(|g| members(g).first().cloned()))).map(Place::Before)
                        };
                        let q = strip.place_tab(&id, at.as_ref(), group.as_deref());
                        Preview { order: keys(&q), color: color_of(&q, &id), onto: None, result: Some(q) }
                    }
                }
            }
            Some(Plan::Group { id, firsts, from, lefts, widths }) => {
                let to = target_index(&lefts, &widths, from, self.dx);
                let color = strip.group(&id).and_then(|g| g["color"].as_str().map(String::from));
                if to == from {
                    return Preview { order: keys(&strip), color, onto: None, result: None };
                }
                let place = if to > from { Place::After(firsts[to].1.clone()) } else { Place::Before(firsts[to].0.clone()) };
                let q = strip.move_group(&id, &place);
                Preview { order: keys(&q), color, onto: None, result: Some(q) }
            }
            None => Preview { order: keys(&strip), color: None, onto: None, result: None },
        }
    }
}

/// `.tab`: one, as the strip or the lifted copy draws it.
fn tab_element(this: &GitVolt, i: usize, group_color: Option<Hsla>, separator: bool, cx: &mut Context<GitVolt>) -> Stateful<Div> {
    let tab = &this.tabs[i];
    let active = i == this.active;
    let outlined = menu::is_target(&format!("tab:{}", tab.id), cx);
    let key: SharedString = format!("tab:{}", tab.id).into();
    let renaming = this.strip_ui.renaming.as_ref().filter(|(id, _)| *id == tab.id).map(|(_, input)| input.clone());
    let (fg, state_key) = (if active { theme::text_selected() } else { theme::text_secondary() }, format!("tab:{}", tab.state_id));
    let path = (!tab.is_open_tab()).then(|| tab.repo_path.clone());
    let label = div().flex_1().min_w_0().truncate().child(tab.name.clone());
    let group_id: SharedString = format!("tab-close-group-{i}").into();
    div()
        .id(("tab", i))
        .role(Role::Tab)
        .aria_label(tab.name.clone())
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .h(px(crate::app::TAB_H))
        // The CSS's 90 and 220 are the content's: padding and the border are on top of them.
        .min_w(px(109.))
        .max_w(px(239.))
        .pl(px(12.))
        .pr(px(6.))
        .border_r_1()
        .border_color(if separator { theme::section_border() } else { transparent_black() })
        .when(active, |d| d.bg(theme::action_bar_bg()).text_color(theme::text_selected()))
        .when(!active, |d| d.text_color(theme::text_secondary()).hover(|d| d.bg(theme::hover_row())))
        .relative()
        .when(outlined, |d| d.child(crate::ui::outline()))
        .when_some(group_color, |d, c| d.child(div().absolute().top_0().left_0().right(px(-1.)).h(px(2.)).bg(c)))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            if e.click_count == 2 {
                this.strip_ui.drag = None;
                this.start_tab_rename(i, window, cx);
                return;
            }
            let Some(t) = this.tabs.get(i) else { return };
            this.strip_press(format!("tab:{}", t.state_id), Some(i), e.position.x.as_f32());
        }))
        .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, _, cx| this.close_tab(i, cx)))
        .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            let weak = cx.weak_entity();
            let rows = this.tab_rows(i, &weak);
            menu::open_context_for(key.clone(), rows, e.position, window, cx);
        }))
        .map(|d| {
            let _ = &state_key;
            match renaming {
                Some(input) => d.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .px(px(4.))
                        .py(px(1.))
                        .border_1()
                        .border_color(theme::blue())
                        .rounded(px(3.))
                        .bg(theme::panel_bg1())
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(input),
                ),
                None => match path {
                    Some(p) => d.child(div().flex_1().min_w_0().child(crate::ui::tooltip::tip(p, label).delay(400))),
                    None => d.child(label),
                },
            }
        })
        .child(
            div()
                .id(("tab-close", i))
                .role(Role::Button)
                .aria_label(format!("Close {}", tab.name))
                .group(group_id.clone())
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(18.))
                .rounded(px(3.))
                .hover(|d| d.bg(theme::control_hover_bg()))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.close_tab(i, cx);
                }))
                .child(icon("x", 12., Hsla { a: fg.a * 0.6, ..fg }).group_hover(group_id, move |d| d.text_color(fg))),
        )
}

/// `.tab-group-chip`: its colour, its name in the ink that reads on it, a 2 px line from its pill along its tabs.
fn chip_element(this: &GitVolt, g: &Value, cx: &mut Context<GitVolt>) -> AnyElement {
    let id = g["id"].as_str().unwrap_or_default().to_string();
    let collapsed = g["collapsed"] == true;
    let color = theme::tag_group(g["color"].as_str().unwrap_or("gray"));
    let name = g["name"].as_str().unwrap_or_default().to_string();
    let key: SharedString = format!("tab-group:{id}").into();
    let (i_menu, k_menu, i_hover) = (id.clone(), key.clone(), id.clone());
    let press_key = format!("chip:{id}");
    let _ = this;
    crate::ui::anchor(
        key,
        div()
            .id(ElementId::Name(format!("tab-group-{id}").into()))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .h_full()
            .px(px(6.))
            .cursor_pointer()
            .on_hover(cx.listener(move |this, on: &bool, _, cx| this.chip_hover(i_hover.clone(), *on, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.strip_ui.list = None;
                this.strip_ui.list_gen += 1;
                this.strip_press(press_key.clone(), None, e.position.x.as_f32());
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(move |v, _, window, cx| {
                cx.stop_propagation();
                v.open_group_menu(i_menu.clone(), k_menu.clone(), window, cx);
            }))
            .when(!collapsed, |d| d.child(div().absolute().top_0().left(px(6.)).right_0().h(px(2.)).bg(color)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(22.))
                    .max_w(px(140.))
                    .h(px(22.))
                    .when(!name.is_empty(), |d| d.px(px(9.)))
                    .border_2()
                    .border_color(color)
                    .rounded_tr(px(11.))
                    .rounded_br(px(11.))
                    .rounded_bl(px(11.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(!collapsed, |d| d.bg(color).text_color(theme::tg_ink()))
                    .when(collapsed, |d| d.text_color(color))
                    .when(!name.is_empty(), |d| d.child(div().min_w_0().truncate().child(name))),
            ),
    )
    .into_any_element()
}

/// `GroupTabList`: under the chip while the pointer's on the chip or the list.
fn group_list(this: &GitVolt, id: &str, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let g = this.profile["tabGroups"].as_array()?.iter().find(|g| g["id"] == id)?;
    let color = theme::tag_group(g["color"].as_str().unwrap_or("gray"));
    let chip = crate::ui::Ui::get(cx).anchor(&format!("tab-group:{id}"))?;
    let tabs: Vec<(usize, SharedString, bool)> = g["tabs"].as_array()?.iter().filter_map(|t| {
        let i = this.tabs.iter().position(|x| t == x.state_id.as_str())?;
        Some((i, this.tabs[i].name.clone(), i == this.active))
    }).collect();
    let (hover_id, list_id) = (id.to_string(), id.to_string());
    let mut list = div()
        .id("tg-list")
        .flex()
        .flex_col()
        .min_w(px(180.))
        .max_w(px(320.))
        .py(px(4.))
        .bg(theme::panel_bg1())
        .border_1()
        .border_t_2()
        .border_color(theme::section_border())
        .border_t_2()
        .rounded(px(6.))
        .shadow(vec![BoxShadow { color: theme::shadow_popup(), offset: point(px(0.), px(6.)), blur_radius: px(20.), spread_radius: px(0.), inset: false }])
        .text_color(theme::text_normal())
        .occlude()
        .on_hover(cx.listener(move |this, on: &bool, _, cx| {
            if *on {
                this.strip_ui.list_gen += 1;
            } else {
                this.chip_hover(hover_id.clone(), false, cx);
            }
        }))
        .child(div().absolute().top(px(-2.)).left(px(-1.)).right(px(-1.)).h(px(2.)).bg(color));
    for (i, name, active) in tabs {
        let _ = &list_id;
        list = list.child(
            div()
                .id(("tg-row", i))
                .flex()
                .items_center()
                .gap(px(8.))
                .pl(px(8.))
                .pr(px(12.))
                .py(px(4.))
                .cursor_pointer()
                .hover(|d| d.bg(theme::hover_row()).text_color(theme::text_selected()))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.active = i;
                    this.strip_ui.list = None;
                    this.save_tabs(cx);
                    cx.notify();
                }))
                .child(if active { icon("check", 12., theme::text_normal()).into_any_element() } else { div().flex_none().size(px(12.)).into_any_element() })
                .child(div().min_w_0().truncate().child(name)),
        );
    }
    Some(deferred(anchored().position(point(chip.left(), chip.bottom())).snap_to_window().child(list)).with_priority(1).into_any_element())
}

/// `.tabs`: the strip, a child of the tab bar.
pub fn strip(this: &mut GitVolt, window: &mut Window, cx: &mut Context<GitVolt>) -> AnyElement {
    let now = Instant::now();
    let done = this.strip_ui.drag.as_ref().and_then(|d| d.settle.as_ref()).filter(|(s, _)| !s.running(now)).map(|(_, r)| r.clone());
    if let Some(result) = done {
        this.finish_strip_drag(result, cx);
    }
    let reveal = this.tabs.get(this.active).map(|t| t.id);
    let handle = this.strip_ui.handle.clone();
    let groups = this.profile["tabGroups"].as_array().cloned().unwrap_or_default();
    let strip_state = this.strip();
    let active_state = this.tabs.get(this.active).map(|t| t.state_id.clone());
    let drag = this.strip_ui.drag.as_ref();
    let animating = drag.is_some_and(|d| d.settle.as_ref().is_some_and(|(s, _)| s.running(now)) || d.slides.values().any(|s| s.running(now)));
    let order: Vec<String> = drag.filter(|d| d.active).map(|d| d.strip.items(active_state.as_deref())).unwrap_or_else(|| strip_state.items(active_state.as_deref()));
    let moving: Vec<String> = drag.filter(|d| d.active).map(|d| d.moving.clone()).unwrap_or_default();
    let lifted_dx = drag.map_or(0., |d| d.settle.as_ref().map_or(d.dx, |(s, _)| s.now(now)));
    let (mut row, mut lifted_children) = (div().id("tabs").role(Role::TabList).aria_label("Repositories").flex().size_full().overflow_x_scroll().track_scroll(&handle), Vec::new());
    let mut reveal_child = None;
    let view = crate::ui::Ui::get(cx).anchor("strip-view");
    for (child, key) in order.iter().enumerate() {
        let is_moving = moving.contains(key);
        let offset = drag.and_then(|d| d.slides.get(key)).map_or(0., |s| s.now(now));
        let width = drag.and_then(|d| d.widths.get(key)).copied();
        let home = drag.and_then(|d| d.lefts.get(key)).copied();
        let element = if let Some(id) = key.strip_prefix("chip:") {
            let Some(g) = groups.iter().find(|g| g["id"] == id) else { continue };
            chip_element(this, g, cx)
        } else {
            let Some(state_id) = key.strip_prefix("tab:") else { continue };
            let Some(i) = this.tabs.iter().position(|t| t.state_id == state_id) else { continue };
            if reveal == Some(this.tabs[i].id) {
                reveal_child = Some(child);
            }
            let in_order = |q: usize| order.get(q).and_then(|k| k.strip_prefix("tab:")).and_then(|s| this.tabs.iter().position(|t| t.state_id == s));
            let next_is_active = in_order(child + 1) == Some(this.active);
            let separator = i != this.active && !next_is_active;
            // What a drag previews on the tab that's dragged and the one it would drop onto, else its group's.
            let previewed = drag.and_then(|d| d.preview.as_ref().filter(|p| is_moving || p.onto.as_deref() == Some(key)).map(|p| p.color.clone()));
            let colour = match previewed {
                Some(c) => c,
                None => groups.iter().find(|g| g["tabs"].as_array().is_some_and(|t| t.iter().any(|x| x == state_id))).and_then(|g| g["color"].as_str().map(String::from)),
            };
            crate::ui::anchor(format!("tabx:{i}"), tab_element(this, i, colour.map(|c| theme::tag_group(&c)), separator, cx)).into_any_element()
        };
        if is_moving {
            let (Some(w), Some(l)) = (width, home) else { continue };
            row = row.child(div().flex_none().w(px(w)).h_full());
            let origin = view.map_or(0., |v| v.left().as_f32());
            lifted_children.push(div().absolute().top_0().left(px(l - origin + lifted_dx)).h_full().bg(theme::tab_bar_bg()).shadow(vec![BoxShadow { color: theme::shadow_modal(), offset: point(px(0.), px(2.)), blur_radius: px(10.), spread_radius: px(0.), inset: false }]).child(div().h_full().bg(theme::hover_row()).child(element)));
            continue;
        }
        let strip_key = format!("strip:{key}");
        row = row.child(crate::ui::anchor(strip_key, div().flex_none().relative().h_full().left(px(offset)).child(element)));
    }
    if this.strip_ui.revealed != reveal && drag.is_none() {
        this.strip_ui.revealed = reveal;
        if let Some(c) = reveal_child {
            handle.scroll_to_item(c);
        }
    }
    if animating {
        window.request_animation_frame();
    }
    let list = this.strip_ui.list.clone().filter(|_| this.strip_ui.drag.is_none()).and_then(|id| group_list(this, &id, cx));
    let row = row
        .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, window, cx| {
            // Empty space only, as a browser's tab strip: a new tab on the Open repository screen; else it moves the window.
            if e.click_count == 2 {
                this.open_blank_tab(cx);
            } else if !this.system_title_bar {
                window.start_window_move();
            }
        }))
        .on_mouse_down(MouseButton::Right, cx.listener(|this, e: &MouseDownEvent, window, cx| {
            let weak = cx.weak_entity();
            let rows = this.tabbar_rows(&weak);
            menu::open_context(rows, e.position, window, cx);
        }));
    crate::ui::anchor(
        "strip-view",
        div().relative().flex().flex_1().min_w_0().h_full().overflow_hidden().child(row).children(lifted_children).children(list),
    )
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn tab(g: Option<&str>, left: f32) -> DragBox {
        DragBox { kind: Kind::Tab(g.map(String::from)), left, width: 100. }
    }

    #[test]
    fn a_tab_passes_another_past_its_far_edge_and_drops_onto_its_middle() {
        let items = [tab(None, 0.), tab(None, 100.), tab(None, 200.)];
        assert_eq!(tab_drop_at(&items, 0, 10., None), TabDrop::Gap { gap: 0, group: None });
        // Its leading edge at 40 is in the next one's middle.
        assert_eq!(tab_drop_at(&items, 0, 40., None), TabDrop::Onto(1));
        assert_eq!(tab_drop_at(&items, 0, 80., None), TabDrop::Gap { gap: 1, group: None });
        assert_eq!(tab_drop_at(&items, 2, -80., None), TabDrop::Gap { gap: 1, group: None });
    }

    #[test]
    fn a_tab_goes_into_a_group_past_its_chip_and_out_past_its_edge() {
        let chip = DragBox { kind: Kind::Chip("g".into()), left: 100., width: 34. };
        let items = [tab(None, 0.), chip, tab(Some("g"), 134.), tab(Some("g"), 234.)];
        let into = tab_drop_at(&items, 0, 45., None);
        assert_eq!(into, TabDrop::Gap { gap: 1, group: Some("g".into()) });
        // From inside: past the chip's far side it's out.
        assert_eq!(tab_drop_at(&items, 2, -50., None), TabDrop::Gap { gap: 1, group: None });
    }

    #[test]
    fn a_zone_is_held_past_its_edge() {
        let items = [tab(None, 0.), tab(None, 100.)];
        let held = TabDrop::Onto(1);
        // 28 is before the onto zone's start, but within hysteresis of it.
        assert_eq!(tab_drop_at(&items, 0, 28., Some(&held)), held);
        assert_eq!(tab_drop_at(&items, 0, 15., Some(&held)), TabDrop::Gap { gap: 0, group: None });
    }

    fn strip() -> Strip {
        Strip { tabs: vec!["a".into(), "b".into(), "c".into(), "d".into()], groups: vec![json!({ "id": "g", "name": "", "color": "blue", "collapsed": false, "tabs": ["b", "c"] })] }
    }

    #[test]
    fn placing_a_tab_leaves_and_joins_as_one_change() {
        let q = strip().place_tab("a", Some(&Place::After("b".into())), Some("g"));
        assert_eq!(q.tabs, ["b", "a", "c", "d"]);
        assert_eq!(q.groups[0]["tabs"], json!(["b", "a", "c"]));
        let q = strip().place_tab("c", Some(&Place::After("d".into())), None);
        assert_eq!(q.tabs, ["a", "b", "d", "c"]);
        assert_eq!(q.groups[0]["tabs"], json!(["b"]));
    }

    #[test]
    fn a_group_of_one_that_empties_goes() {
        let s = Strip { tabs: vec!["a".into(), "b".into()], groups: vec![json!({ "id": "g", "name": "", "color": "blue", "collapsed": false, "tabs": ["a"] })] };
        assert_eq!(s.place_tab("a", None, None).groups.len(), 0);
    }

    #[test]
    fn dropping_onto_a_tab_makes_a_group_or_joins_its() {
        let q = strip().group_onto("a", "d", "new", "red");
        assert_eq!(q.tabs, ["b", "c", "a", "d"]);
        assert_eq!(q.groups[1]["tabs"], json!(["a", "d"]));
        let q = strip().group_onto("d", "c", "new", "red");
        assert_eq!(q.groups.len(), 1);
        assert_eq!(q.groups[0]["tabs"], json!(["b", "c", "d"]));
    }

    #[test]
    fn a_group_moves_whole_past_others() {
        let q = strip().move_group("g", &Place::After("d".into()));
        assert_eq!(q.tabs, ["a", "d", "b", "c"]);
        let q = strip().move_group("g", &Place::Before("a".into()));
        assert_eq!(q.tabs, ["b", "c", "a", "d"]);
    }

    #[test]
    fn a_collapsed_groups_tabs_are_left_out_but_the_active_one() {
        let mut s = strip();
        s.groups[0]["collapsed"] = json!(true);
        assert_eq!(s.items(Some("c")), ["tab:a", "chip:g", "tab:c", "tab:d"]);
        assert_eq!(s.items(Some("a")), ["tab:a", "chip:g", "tab:d"]);
    }
}
