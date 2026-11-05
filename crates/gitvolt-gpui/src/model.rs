//! What the GPUI views draw, from the core's payloads: pure, so it's tested without a window. Ports
//! `ui/src/graph/{segments,geometry,columns}.ts`, `ui/src/avatars/color.ts`, `ui/src/format/*.ts`
//! and the sidebar's branch tree.

use chrono::{Datelike, Local, TimeZone, Timelike};
use gitvolt_graph::{GraphPayload, NodeKind, RefLabel, RowPayload};
use gpui::SharedString;
use serde_json::Value;
use std::collections::HashMap;

pub const ROW_H: f32 = 28.;
pub const LANE_W: f32 = 22.;
pub const PAD_X: f32 = LANE_W / 2.;
pub const BAND_INSET: f32 = 3.;
pub const CHIP_H: f32 = 22.;
pub const CELL_PAD: f32 = 8.;
pub const LINE_W: f32 = 2.;
pub const RAIL_W: f32 = 2.;
pub const NODE_R: f32 = ROW_H * 0.36;
pub const MERGE_R: f32 = ROW_H * 0.18;
pub const BAND_ALPHA: f32 = 0.18;
pub const SELECTED_BAND_ALPHA: f32 = 0.5;
pub const CONNECTOR_ALPHA: f32 = 0.25;
const ZONE_GAP: f32 = 3.;
pub const SHORT_SHA: usize = 6;

pub const LABELS_W: f32 = 200.;
pub const AUTHOR_W: f32 = 160.;
pub const DATE_W: f32 = 170.;
pub const SHA_W: f32 = 60.;
const MESSAGE_MIN: f32 = 160.;
const GRAPH_MIN: f32 = 48.;

pub const HALF_TOP: u8 = 0;
#[cfg(test)]
pub const HALF_BOTTOM: u8 = 1;
pub const HALF_FULL: u8 = 2;

/// `graph::Segment::pack` unpacked. Bits: from 0-9, to 10-19, half 20-21, color 22-25, dashed 26, dotted 27.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seg {
    pub from: u16,
    pub to: u16,
    pub half: u8,
    pub color: u8,
    pub dashed: bool,
    pub dotted: bool,
}

pub fn decode_segment(p: u32) -> Seg {
    Seg {
        from: (p & 0x3ff) as u16,
        to: ((p >> 10) & 0x3ff) as u16,
        half: ((p >> 20) & 0x3) as u8,
        color: ((p >> 22) & 0xf) as u8,
        dashed: (p >> 26) & 1 == 1,
        dotted: (p >> 27) & 1 == 1,
    }
}

pub fn lane_x(lane: u16) -> f32 {
    PAD_X + f32::from(lane) * LANE_W + LANE_W / 2.
}

/// The picks a person's initials avatar: FNV-1a over the UTF-16 of the trimmed, lowercased email
/// (the name when there's none), as `avatarLane`, so a person has the same colour in both UIs.
pub fn avatar_lane(name: &str, email: &str, lanes: usize) -> usize {
    let key = email.trim().to_lowercase();
    let text = if key.is_empty() { name } else { &key };
    let mut h: u32 = 2_166_136_261;
    for unit in text.encode_utf16() {
        h ^= u32::from(unit);
        h = h.wrapping_mul(16_777_619);
    }
    h as usize % lanes
}

pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    if words.is_empty() {
        return "?".into();
    }
    words.iter().take(2).filter_map(|w| w.chars().next()).flat_map(char::to_uppercase).collect()
}

/// `formatDate`'s default preset: `2023-11-17 @ 5:53 PM`, local time.
pub fn format_date(unix_seconds: i64) -> String {
    let Some(d) = Local.timestamp_opt(unix_seconds, 0).single() else { return String::new() };
    let hour = d.hour();
    let h12 = if hour % 12 == 0 { 12 } else { hour % 12 };
    format!("{}-{:02}-{:02} @ {h12}:{:02} {}", d.year(), d.month(), d.day(), d.minute(), if hour < 12 { "AM" } else { "PM" })
}

/// The graph's column widths at `total` px of row, and whether its lanes need more than they get.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Columns {
    pub graph: f32,
    pub message: f32,
    pub clipped: bool,
}

pub fn columns(total: f32, max_lanes: u16) -> Columns {
    let fixed = LABELS_W + AUTHOR_W + DATE_W + SHA_W;
    let wanted = LANE_W * (f32::from(max_lanes) + 1.);
    let room = (total - fixed - MESSAGE_MIN).max(GRAPH_MIN);
    let graph = wanted.min(room);
    Columns { graph, message: (total - fixed - graph).max(MESSAGE_MIN), clipped: wanted > room }
}

/// The collapse zone's width: a node, a gap either side and the rail.
pub fn zone_width() -> f32 {
    (2. * NODE_R).ceil() + 2. * ZONE_GAP + RAIL_W
}

/// A row's node x in a graph column `width` wide: `None` while its lane is past the lane area
/// (`clipped`), when it's packed into the zone. As `laneNodeX`, by the lane's centre.
pub fn node_x(lane: u16, width: f32, clipped: bool) -> Option<f32> {
    let x = lane_x(lane);
    (!clipped || x < width - zone_width()).then_some(x)
}

/// The packed column's centre x: mid-zone, left of the rail.
pub fn packed_x(width: f32) -> f32 {
    width - zone_width() + (zone_width() - RAIL_W) / 2.
}

/// A branch or tag chip on a row.
#[derive(Debug, Clone, PartialEq)]
pub struct Chip {
    pub name: SharedString,
    pub head: bool,
    pub local: bool,
    pub remote: bool,
    pub tag: bool,
}

/// One graph row, ready to draw: nothing here is formatted during a frame.
pub struct RowView {
    pub id: SharedString,
    pub kind: NodeKind,
    pub lane: u16,
    pub color: u8,
    pub segments: Vec<Seg>,
    pub summary: SharedString,
    pub author: SharedString,
    pub date: SharedString,
    pub sha: SharedString,
    pub initials: SharedString,
    pub avatar: usize,
    pub chips: Vec<Chip>,
}

pub struct GraphView {
    pub rows: Vec<RowView>,
    pub max_lanes: u16,
    pub head_row: Option<usize>,
    pub head_branch: SharedString,
    pub pinned: SharedString,
    pub by_sha: HashMap<String, usize>,
}

const LANES: usize = 10;

fn chip(l: &RefLabel) -> Chip {
    Chip { name: l.name.clone().into(), head: l.is_head, local: l.local.is_some(), remote: !l.remotes.is_empty(), tag: l.tag }
}

fn row_view(r: &RowPayload, chips: Vec<Chip>) -> RowView {
    let summary = if r.kind == NodeKind::Wip { "// WIP".to_string() } else { r.summary.clone() };
    RowView {
        id: r.id.clone().into(),
        kind: r.kind,
        lane: r.lane,
        color: r.color,
        segments: r.segments.iter().map(|p| decode_segment(*p)).collect(),
        summary: summary.into(),
        author: r.author_name.clone().into(),
        date: if r.committer_time > 0 { format_date(r.committer_time).into() } else { SharedString::default() },
        sha: if r.kind == NodeKind::Commit || r.kind == NodeKind::Merge { r.id.chars().take(SHORT_SHA).collect::<String>().into() } else { SharedString::default() },
        initials: initials(&r.author_name).into(),
        avatar: avatar_lane(&r.author_name, &r.author_email, LANES),
        chips,
    }
}

pub fn graph_view(g: &GraphPayload) -> GraphView {
    let mut chips: HashMap<u32, Vec<Chip>> = HashMap::new();
    let mut head_row = None;
    for l in &g.labels {
        chips.entry(l.row).or_default().push(chip(l));
        if l.is_head {
            head_row = Some(l.row as usize);
        }
    }
    for list in chips.values_mut() {
        // The checked-out branch's chip is the one that shows; the rest fold into "+N".
        list.sort_by_key(|c| !c.head);
    }
    let rows: Vec<RowView> = g.rows.iter().enumerate().map(|(i, r)| row_view(r, chips.remove(&(i as u32)).unwrap_or_default())).collect();
    let by_sha = rows.iter().enumerate().map(|(i, r)| (r.id.to_string(), i)).collect();
    let short = |name: &str| name.trim_start_matches("refs/heads/").trim_start_matches("refs/remotes/").to_string();
    GraphView {
        rows,
        max_lanes: g.max_lanes,
        head_row,
        head_branch: g.head.branch.as_deref().map(short).unwrap_or_else(|| "detached".into()).into(),
        pinned: g.pinned_refs.first().map(|r| short(r)).unwrap_or_default().into(),
        by_sha,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideIcon {
    Folder,
    Branch,
    Tag,
    Stash,
    Home,
}

/// A row of a sidebar panel.
#[derive(Debug, Clone, PartialEq)]
pub struct SideRow {
    pub label: SharedString,
    pub depth: u8,
    pub icon: SideIcon,
    /// The checked-out branch, the open worktree: the green row.
    pub current: bool,
    /// The commit it points at; a folder has none.
    pub target: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelKind {
    Local,
    Remote,
    Worktrees,
    Stashes,
    Tags,
}

pub struct Panel {
    pub kind: PanelKind,
    pub title: &'static str,
    pub count: usize,
    pub rows: Vec<SideRow>,
}

/// `(name, target, current)`s as a tree by `/`: a folder before its branches, each level sorted.
fn branch_tree(mut items: Vec<(String, String, bool)>, icon: SideIcon) -> Vec<SideRow> {
    items.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = Vec::new();
    let mut open: Vec<String> = Vec::new();
    // Folders first at each level: a folder sorts by its name with a low byte in front.
    items.sort_by_key(|(name, _, _)| {
        let parts: Vec<&str> = name.split('/').collect();
        let last = parts.len() - 1;
        parts.iter().enumerate().map(|(i, p)| format!("{}{p}", if i < last { '\u{1}' } else { '\u{2}' })).collect::<Vec<_>>()
    });
    for (name, target, current) in items {
        let parts: Vec<&str> = name.split('/').collect();
        let (leaf, folders) = parts.split_last().expect("split gives one part at least");
        let shared = open.iter().zip(folders.iter()).take_while(|(a, b)| a.as_str() == **b).count();
        open.truncate(shared);
        for f in &folders[shared..] {
            out.push(SideRow { label: (*f).to_string().into(), depth: open.len() as u8, icon: SideIcon::Folder, current: false, target: None });
            open.push((*f).to_string());
        }
        out.push(SideRow { label: (*leaf).to_string().into(), depth: open.len() as u8, icon, current, target: Some(target) });
    }
    out
}

fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

/// The `sidebar` read's five panels, in the UI's order.
pub fn panels(v: &Value) -> Vec<Panel> {
    let list = |key: &str| v[key].as_array().cloned().unwrap_or_default();
    let locals = list("locals");
    let local_rows = branch_tree(locals.iter().map(|b| (text(b, "name"), text(b, "target"), b["isHead"].as_bool() == Some(true))).collect(), SideIcon::Branch);
    let remotes = list("remotes");
    let mut remote_rows = Vec::new();
    let mut remote_count = 0;
    for r in &remotes {
        let branches = r["branches"].as_array().cloned().unwrap_or_default();
        remote_count += branches.len();
        remote_rows.push(SideRow { label: text(r, "name").into(), depth: 0, icon: SideIcon::Folder, current: false, target: None });
        let prefix = format!("{}/", text(r, "name"));
        for mut row in branch_tree(branches.iter().map(|b| (text(b, "name").trim_start_matches(&prefix).to_string(), text(b, "target"), false)).collect(), SideIcon::Branch) {
            row.depth += 1;
            remote_rows.push(row);
        }
    }
    let worktrees = list("worktrees");
    let worktree_rows = worktrees.iter().map(|w| SideRow { label: text(w, "name").into(), depth: 0, icon: SideIcon::Home, current: w["isCurrent"].as_bool() == Some(true), target: w["head"].as_str().map(str::to_string) }).collect();
    let stashes = list("stashes");
    let stash_rows = stashes.iter().map(|s| SideRow { label: text(s, "message").into(), depth: 0, icon: SideIcon::Stash, current: false, target: Some(text(s, "id")) }).collect();
    let tags = list("tags");
    let tag_rows = tags.iter().map(|t| SideRow { label: text(t, "name").into(), depth: 0, icon: SideIcon::Tag, current: false, target: Some(text(t, "target")) }).collect();
    vec![
        Panel { kind: PanelKind::Local, title: "LOCAL", count: locals.len(), rows: local_rows },
        Panel { kind: PanelKind::Remote, title: "REMOTE", count: remote_count, rows: remote_rows },
        Panel { kind: PanelKind::Worktrees, title: "WORKTREES", count: worktrees.len(), rows: worktree_rows },
        Panel { kind: PanelKind::Stashes, title: "STASHES", count: stashes.len(), rows: stash_rows },
        Panel { kind: PanelKind::Tags, title: "TAGS", count: tags.len(), rows: tag_rows },
    ]
}

/// The `commitDetails` read, as the details panel shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    pub id: SharedString,
    pub author: SharedString,
    pub email: SharedString,
    pub date: SharedString,
    pub parents: Vec<SharedString>,
    pub signed: bool,
}

pub fn details(v: &Value) -> Details {
    Details {
        id: text(v, "id").into(),
        author: text(&v["author"], "name").into(),
        email: text(&v["author"], "email").into(),
        date: format_date(v["committer"]["time"].as_i64().unwrap_or(0)).into(),
        parents: v["parents"].as_array().map(|p| p.iter().filter_map(|x| x.as_str()).map(|s| s.chars().take(SHORT_SHA).collect::<String>().into()).collect()).unwrap_or_default(),
        signed: v["signed"].as_bool() == Some(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_segment_unpacks_as_the_layout_packs_it() {
        let packed = 3 | (5 << 10) | (u32::from(HALF_BOTTOM) << 20) | (7 << 22) | (1 << 26);
        assert_eq!(decode_segment(packed), Seg { from: 3, to: 5, half: HALF_BOTTOM, color: 7, dashed: true, dotted: false });
        assert!(decode_segment(1 << 27).dotted);
    }

    #[test]
    fn a_person_has_the_web_uis_avatar_colour_and_initials() {
        // `hash('dev1@example.com') % 10` in ui/src/avatars/color.ts, by FNV-1a.
        let mut h: u32 = 2_166_136_261;
        for b in "dev1@example.com".bytes() {
            h ^= u32::from(b);
            h = h.wrapping_mul(16_777_619);
        }
        assert_eq!(avatar_lane("Dev", "  DEV1@example.com ", 10), h as usize % 10, "trimmed and lowercased");
        assert_ne!(avatar_lane("Ada", "", 1000), avatar_lane("Bob", "", 1000), "the name, without an email");
        assert_eq!(initials("Dev Person35"), "DP");
        assert_eq!(initials("  ada  "), "A");
        assert_eq!(initials(""), "?");
        assert_eq!(initials("Ada B Cole"), "AB");
    }

    #[test]
    fn the_graph_column_takes_what_its_lanes_want_until_the_message_needs_the_room() {
        // The web UI's measured layout: 22 lanes at a 1349 px row.
        assert_eq!(columns(1349., 22), Columns { graph: 506., message: 253., clipped: false });
        let tight = columns(1000., 22);
        assert_eq!((tight.graph, tight.message, tight.clipped), (250., 160., true));
        assert_eq!(columns(400., 22).graph, 48., "never under its minimum");
    }

    #[test]
    fn a_lane_past_the_lane_area_packs_into_the_zone() {
        assert_eq!(lane_x(0), 22.);
        assert_eq!(node_x(3, 100., false), Some(88.));
        assert_eq!(node_x(0, 100., true), Some(22.));
        assert_eq!(node_x(3, 100., true), None);
        assert!(packed_x(100.) > 100. - zone_width());
    }

    #[test]
    fn branches_nest_by_slash_with_folders_first() {
        let v = json!({"locals": [
            {"name": "main", "target": "a", "isHead": true},
            {"name": "feature/b10", "target": "c"},
            {"name": "feature/b1", "target": "b"},
            {"name": "fix/deep/x", "target": "d"},
        ], "remotes": [{"name": "origin", "branches": [{"name": "origin/main", "target": "a"}]}], "worktrees": [{"name": "w", "isCurrent": true, "head": "a"}], "stashes": [], "tags": [{"name": "v1", "target": "t"}]});
        let p = panels(&v);
        let shown: Vec<(&str, u8, bool)> = p[0].rows.iter().map(|r| (r.label.as_ref(), r.depth, r.current)).collect();
        assert_eq!(shown, [("feature", 0, false), ("b1", 1, false), ("b10", 1, false), ("fix", 0, false), ("deep", 1, false), ("x", 2, false), ("main", 0, true)]);
        assert_eq!(p[0].count, 4);
        assert_eq!(p[1].rows.iter().map(|r| r.label.as_ref()).collect::<Vec<_>>(), ["origin", "main"]);
        assert_eq!((p[1].count, p[1].rows[1].depth), (1, 1));
        assert!(p[2].rows[0].current);
        assert_eq!(p[4].rows[0].target.as_deref(), Some("t"));
    }

    #[test]
    fn details_read_the_camel_case_payload() {
        let d = details(&json!({"id": "abcdef123", "parents": ["1234567890"], "author": {"name": "Ada", "email": "a@example.com", "time": 1}, "committer": {"time": 1_700_000_000}, "signed": true}));
        assert_eq!((d.author.as_ref(), d.parents[0].as_ref(), d.signed), ("Ada", "123456", true));
        assert!(d.date.contains(" @ "));
    }
}
