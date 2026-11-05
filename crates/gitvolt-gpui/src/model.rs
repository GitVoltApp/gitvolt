//! What the GPUI views draw, from the core's payloads: pure, so it's tested without a window. Ports
//! `ui/src/graph/{segments,geometry,columns}.ts`, `ui/src/avatars/color.ts`, `ui/src/format/*.ts`
//! and the sidebar's branch tree.

use chrono::{Datelike, Local, TimeZone, Timelike};
use gitvolt_graph::{GraphPayload, NodeKind, RefLabel, RowPayload};
use gpui::SharedString;
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
    pub labels: f32,
    pub graph: f32,
    pub message: f32,
    pub author: f32,
    pub date: f32,
    pub sha: f32,
    pub total: f32,
    pub clipped: bool,
}

/// The graph table's columns, left to right (`ui/src/graph/columns.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Col {
    Labels,
    Graph,
    Message,
    Author,
    Date,
    Sha,
}

pub const HIDEABLE: [(Col, &str); 4] = [(Col::Labels, "Branch / Tag"), (Col::Author, "Author"), (Col::Date, "Commit date / time"), (Col::Sha, "SHA")];
const MIN_LABELS: f32 = 80.;
const MIN_AUTHOR: f32 = 60.;
const MIN_DATE: f32 = 90.;
const SHA_MAX: f32 = 326.;
const AUTHOR_DATE_MAX: f32 = 800.;
const MESSAGE_MAX: f32 = 4000.;

fn col_min(col: Col) -> f32 {
    match col {
        Col::Labels => MIN_LABELS,
        Col::Graph => GRAPH_MIN,
        Col::Message => MESSAGE_MIN,
        Col::Author => MIN_AUTHOR,
        Col::Date => MIN_DATE,
        Col::Sha => SHA_W,
    }
}

/// The user's widths. `graph`: `None` fits the lanes; `message`: `None` fills the table.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnPrefs {
    pub labels: f32,
    pub graph: Option<f32>,
    pub author: f32,
    pub date: f32,
    pub sha: f32,
    pub message: Option<f32>,
}

impl Default for ColumnPrefs {
    fn default() -> Self {
        Self { labels: LABELS_W, graph: None, author: AUTHOR_W, date: DATE_W, sha: SHA_W, message: None }
    }
}

/// `isCollapsed`: a column at its minimum shows an icon instead of its title.
pub fn is_collapsed(col: Col, width: f32) -> bool {
    col != Col::Sha && width <= col_min(col)
}

pub fn lanes_width(max_lanes: u16) -> f32 {
    LANE_W * (f32::from(max_lanes) + 1.)
}

/// `allocateColumns`: Branch/Tag, Graph and SHA always get their width; Message fills what's left, and
/// at its minimum squeezes Author and Date in proportion; a hand-set Message squeezes nothing, and the
/// table runs past the window. With no hand-set width, the Graph fits its lanes while Message has room.
pub fn allocate_columns(prefs: &ColumnPrefs, available: f32, hidden: &std::collections::HashSet<Col>, max_lanes: u16) -> Columns {
    let off = |c: Col| hidden.contains(&c);
    let labels = if off(Col::Labels) { 0. } else { prefs.labels.max(MIN_LABELS) };
    let sha = if off(Col::Sha) { 0. } else { prefs.sha.clamp(SHA_W, SHA_MAX) };
    let mut author = if off(Col::Author) { 0. } else { prefs.author.max(MIN_AUTHOR) };
    let mut date = if off(Col::Date) { 0. } else { prefs.date.max(MIN_DATE) };
    let wanted = lanes_width(max_lanes);
    let graph = match prefs.graph {
        Some(g) => g.max(GRAPH_MIN),
        None => wanted.min((available - labels - author - date - sha - MESSAGE_MIN).max(GRAPH_MIN)),
    };
    let clipped = graph < wanted;
    if let Some(m) = prefs.message {
        let message = m.max(MESSAGE_MIN);
        return Columns { labels, graph, message, author, date, sha, total: labels + graph + message + author + date + sha, clipped };
    }
    let leftover = available - (labels + graph + author + date + sha);
    if leftover >= MESSAGE_MIN {
        return Columns { labels, graph, message: leftover, author, date, sha, total: available, clipped };
    }
    let deficit = MESSAGE_MIN - leftover;
    let room_a = if off(Col::Author) { 0. } else { author - MIN_AUTHOR };
    let room_d = if off(Col::Date) { 0. } else { date - MIN_DATE };
    if deficit >= room_a + room_d {
        if !off(Col::Author) {
            author = MIN_AUTHOR;
        }
        if !off(Col::Date) {
            date = MIN_DATE;
        }
    } else {
        let from_a = (deficit * room_a / (room_a + room_d)).round();
        author -= from_a;
        date -= deficit - from_a;
    }
    Columns { labels, graph, message: MESSAGE_MIN, author, date, sha, total: labels + graph + MESSAGE_MIN + author + date + sha, clipped }
}

/// The default widths' columns for a table `total` px wide.
#[cfg(test)]
pub fn columns(total: f32, max_lanes: u16) -> Columns {
    allocate_columns(&ColumnPrefs::default(), total, &std::collections::HashSet::new(), max_lanes)
}

/// `handleRange`: how far a column's right edge may go.
pub fn handle_range(col: Col, w: &Columns, available: f32, graph_max: f32) -> (f32, f32) {
    let total_min = MIN_LABELS + GRAPH_MIN + MESSAGE_MIN + MIN_AUTHOR + MIN_DATE + SHA_W;
    match col {
        Col::Labels => (MIN_LABELS, (available - (total_min - MIN_LABELS)).max(MIN_LABELS).max(w.labels)),
        Col::Graph => (GRAPH_MIN, (available - (total_min - GRAPH_MIN)).max(GRAPH_MIN).max(w.graph).min(graph_max.max(GRAPH_MIN))),
        Col::Message => (MESSAGE_MIN, w.message.max(MESSAGE_MAX)),
        Col::Author => (MIN_AUTHOR, w.author.max(AUTHOR_DATE_MAX)),
        Col::Date => (MIN_DATE, w.date.max(AUTHOR_DATE_MAX)),
        Col::Sha => (SHA_W, w.sha.max(SHA_MAX)),
    }
}

/// `resizeColumn`: the prefs after moving `col`'s edge `dx` px from where a drag started.
pub fn resize_column(col: Col, start: &Columns, prefs: &ColumnPrefs, dx: f32, available: f32, graph_max: f32) -> ColumnPrefs {
    let (min, max) = handle_range(col, start, available, graph_max);
    let width_of = |w: f32| (w + dx.round()).max(min).min(max);
    let mut out = prefs.clone();
    match col {
        Col::Labels => out.labels = width_of(start.labels),
        Col::Graph => out.graph = Some(width_of(start.graph)),
        Col::Message => {
            out.author = start.author;
            out.date = start.date;
            out.message = Some(width_of(start.message));
        }
        Col::Author | Col::Date | Col::Sha => {
            let was = match col {
                Col::Author => start.author,
                Col::Date => start.date,
                _ => start.sha,
            };
            let width = width_of(was);
            let grow = width - was;
            let fill = prefs.message.is_none();
            out.author = start.author;
            out.date = start.date;
            out.message = if !fill || grow < 0. {
                Some(start.message)
            } else if grow <= start.message - MESSAGE_MIN {
                None
            } else {
                Some(MESSAGE_MIN)
            };
            match col {
                Col::Author => out.author = width,
                Col::Date => out.date = width,
                _ => out.sha = width,
            }
        }
    }
    out
}

/// `branchRows`: the rows a branch's tip reaches through parents.
pub fn branch_rows(g: &GraphView, tip: usize) -> std::collections::HashSet<usize> {
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![tip];
    while let Some(i) = stack.pop() {
        if !seen.insert(i) {
            continue;
        }
        if let Some(r) = g.rows.get(i) {
            stack.extend(r.parents.iter().filter_map(|p| g.by_sha.get(p).copied()));
        }
    }
    seen
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
    /// Its remote branches: `(remote, full ref)`.
    pub remotes: Vec<(String, String)>,
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
    /// A WIP row's worktree.
    pub worktree: Option<String>,
    pub email: String,
    pub parents: Vec<String>,
    /// A stash's branch, dimmed after its text.
    pub stash_branch: Option<SharedString>,
    pub wip: Option<WipCounts>,
}

/// A WIP row's change counts, and its worktree's name when it isn't the main one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WipCounts {
    pub name: Option<SharedString>,
    pub modified: u32,
    pub added: u32,
    pub deleted: u32,
    pub renamed: u32,
    pub conflicted: u32,
}

impl WipCounts {
    pub fn files(&self) -> u32 {
        self.modified + self.added + self.deleted + self.renamed + self.conflicted
    }
}

pub struct GraphView {
    pub rows: Vec<RowView>,
    pub max_lanes: u16,
    pub head_row: Option<usize>,
    pub head_branch: SharedString,
    pub pinned: SharedString,
    /// More history than this window holds.
    pub truncated: bool,
    pub by_sha: HashMap<String, usize>,
}

const LANES: usize = 10;

fn chip(l: &RefLabel) -> Chip {
    Chip { name: l.name.clone().into(), head: l.is_head, local: l.local.is_some(), remote: !l.remotes.is_empty(), tag: l.tag, remotes: l.remotes.iter().map(|r| (r.remote.clone(), r.full_name.clone())).collect() }
}

fn row_view(r: &RowPayload, chips: Vec<Chip>) -> RowView {
    let (summary, stash_branch) = match r.kind {
        NodeKind::Wip => ("// WIP".to_string(), None),
        NodeKind::Stash => crate::sidebar::stash_label(&r.summary),
        _ => (r.summary.clone(), None),
    };
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
        worktree: r.wip.as_ref().map(|w| w.worktree_path.clone()),
        email: r.author_email.clone(),
        parents: r.parents.clone(),
        stash_branch: stash_branch.map(Into::into),
        wip: r.wip.as_ref().map(|w| WipCounts { name: w.worktree_name.clone().map(Into::into), modified: w.modified, added: w.added, deleted: w.deleted, renamed: w.renamed, conflicted: w.conflicted }),
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
        truncated: g.truncated,
        by_sha,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let wide = columns(1349., 22);
        assert_eq!((wide.graph, wide.message, wide.clipped), (506., 253., false));
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

    // Web: graph/columnsFit.test.ts and columns.test.ts.
    #[test]
    fn message_fills_then_squeezes_author_and_date_by_their_room() {
        let none = std::collections::HashSet::new();
        let c = allocate_columns(&ColumnPrefs { graph: Some(100.), ..ColumnPrefs::default() }, 1000., &none, 2);
        assert_eq!((c.message, c.author, c.date), (310., 160., 170.));
        let tight = allocate_columns(&ColumnPrefs { graph: Some(100.), ..ColumnPrefs::default() }, 700., &none, 2);
        assert_eq!(tight.message, 160.);
        assert_eq!(tight.author + tight.date, 180. + 0., "what's left past the fixed columns");
        let floor = allocate_columns(&ColumnPrefs { graph: Some(100.), ..ColumnPrefs::default() }, 300., &none, 2);
        assert_eq!((floor.author, floor.date), (60., 90.));
    }

    #[test]
    fn a_hidden_column_takes_no_room_and_a_hand_set_message_stays() {
        let mut hidden = std::collections::HashSet::new();
        hidden.insert(Col::Author);
        hidden.insert(Col::Labels);
        let c = allocate_columns(&ColumnPrefs { graph: Some(100.), ..ColumnPrefs::default() }, 1000., &hidden, 2);
        assert_eq!((c.labels, c.author, c.message), (0., 0., 670.));
        let set = allocate_columns(&ColumnPrefs { message: Some(900.), graph: Some(100.), ..ColumnPrefs::default() }, 800., &std::collections::HashSet::new(), 2);
        assert_eq!(set.message, 900.);
        assert!(set.total > 800., "the table runs past the window");
    }

    #[test]
    fn a_drag_stops_at_a_columns_limits() {
        let none = std::collections::HashSet::new();
        let prefs = ColumnPrefs::default();
        let c = allocate_columns(&prefs, 1400., &none, 2);
        assert_eq!(resize_column(Col::Labels, &c, &prefs, 40., 1400., 66.).labels, 240.);
        assert_eq!(resize_column(Col::Labels, &c, &prefs, -500., 1400., 66.).labels, 80.);
        assert_eq!(resize_column(Col::Graph, &c, &prefs, 500., 1400., 88.).graph, Some(88.), "never past its lanes");
        assert_eq!(resize_column(Col::Sha, &c, &prefs, 900., 1400., 66.).sha, 326.);
        let grown = resize_column(Col::Author, &c, &prefs, 30., 1400., 66.);
        assert_eq!((grown.author, grown.message), (190., None), "Message keeps filling");
        assert_eq!(resize_column(Col::Message, &c, &prefs, 10., 1400., 66.).message, Some(c.message + 10.));
    }

    #[test]
    fn a_branch_reaches_its_ancestors_only() {
        let row = |id: &str, parents: &[&str]| RowView { id: id.to_string().into(), kind: NodeKind::Commit, lane: 0, color: 0, segments: Vec::new(), summary: SharedString::default(), author: SharedString::default(), date: SharedString::default(), sha: SharedString::default(), initials: SharedString::default(), avatar: 0, chips: Vec::new(), worktree: None, email: String::new(), parents: parents.iter().map(|p| p.to_string()).collect(), stash_branch: None, wip: None };
        let rows = vec![row("d", &["b"]), row("c", &["b"]), row("b", &["a"]), row("a", &[])];
        let by_sha: HashMap<String, usize> = rows.iter().enumerate().map(|(i, r)| (r.id.to_string(), i)).collect();
        let g = GraphView { rows, max_lanes: 2, head_row: None, head_branch: SharedString::default(), pinned: SharedString::default(), truncated: false, by_sha };
        let mut reach: Vec<usize> = branch_rows(&g, 1).into_iter().collect();
        reach.sort_unstable();
        assert_eq!(reach, [1, 2, 3]);
    }
}
