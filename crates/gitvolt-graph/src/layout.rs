use crate::Oid;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum NodeKind {
    Commit,
    Merge,
    Stash,
    Wip,
    /// A collapsed branch's ⋮ row: no node of its own, its first parent's lane dotted.
    Collapsed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Parent {
    Row(u32),
    Outside(Oid),
    /// A folded merge's tail: its folded branch's line, kept as a parent edge so the lanes route
    /// around it as they would around the branch, ending undrawn in row `n` (the row under the
    /// merge). Its lane is taken through all of row `n`, where its end is dotted.
    Tail(u32),
}

#[derive(Debug, Clone)]
pub struct LayoutNode {
    pub parents: Vec<Parent>,
    pub kind: NodeKind,
    /// The pinned lane this node is forced into: 0 for the trunk's first-parent chain, 1 for a
    /// pinned pair's second chain (a remote branch diverged from its local one).
    pub pinned: Option<u16>,
    /// Committer time (`i64::MAX` for a WIP row). Only the stash rule reads it: a commit
    /// a stash reached first goes to a branch whose chain is newer than the stash.
    pub time: i64,
    /// Hidden branches fork from this commit: a stub comes into it from the row above, in a lane
    /// of its own, as a child's line would (`LayoutState::push`'s `next`). Never on row 0.
    pub stub: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Half {
    Top = 0,
    Bottom = 1,
    Full = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub from_lane: u16,
    pub to_lane: u16,
    pub half: Half,
    pub color: u8,
    pub dashed: bool,
    /// A collapsed run's lane, from its ⋮ row down to its oldest commit.
    pub dotted: bool,
}

const MAX_PACKED_LANE: u16 = 1023;
pub const LANE_COLORS: u8 = 10;

impl Segment {
    pub fn pack(self) -> u32 {
        let from = u32::from(self.from_lane.min(MAX_PACKED_LANE));
        let to = u32::from(self.to_lane.min(MAX_PACKED_LANE));
        from | (to << 10) | ((self.half as u32) << 20) | (u32::from(self.color & 0xf) << 22) | (u32::from(self.dashed) << 26) | (u32::from(self.dotted) << 27)
    }

    pub fn unpack(v: u32) -> Self {
        let half = match (v >> 20) & 0x3 {
            0 => Half::Top,
            1 => Half::Bottom,
            _ => Half::Full,
        };
        Segment {
            from_lane: (v & 0x3ff) as u16,
            to_lane: ((v >> 10) & 0x3ff) as u16,
            half,
            color: ((v >> 22) & 0xf) as u8,
            dashed: (v >> 26) & 1 == 1,
            dotted: (v >> 27) & 1 == 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow {
    pub lane: u16,
    pub color: u8,
    pub segments: Vec<Segment>,
    /// The lane of the hidden-branch stub coming into this row's node (`LayoutNode::stub`).
    pub stub: Option<u16>,
    /// The lane of the fold tail this row's node opens (`Parent::Tail`).
    pub tail: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct Layout {
    pub rows: Vec<GraphRow>,
    pub max_lanes: u16,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Wait {
    Row(u32),
    Outside(Oid),
    /// A fold tail, ending in row `n`.
    Tail(u32),
    /// A hidden-branch stub, ending in row `n`'s node.
    Stub(u32),
}

/// `Lane::newest` when the chain that set the home recorded no time.
const NO_TIME: i64 = i64::MIN;

/// One lane: the parent it waits for. Every awaited parent has exactly one *home* among the
/// lanes waiting for it: the lane it will land in (its column reservation).
#[derive(Clone, Copy)]
struct Lane {
    wait: Wait,
    /// The committer time of the newest commit on the chain that set the home, or `NO_TIME`.
    newest: i64,
    dashed: bool,
    /// A collapsed run's lane: set by a ⋮ row for its first parent (its oldest commit).
    dotted: bool,
    /// `wait` lands in this lane.
    home: bool,
    /// A merge has `wait` as a parent, so its home never moves again: merge lines run straight
    /// into it, and a trunk of merges keeps its lane.
    locked: bool,
    /// The home was set by a stash row (a newer branch takes it over).
    by_stash: bool,
}

impl Lane {
    fn new(wait: Wait, dashed: bool, dotted: bool) -> Self {
        Lane { wait, newest: NO_TIME, dashed, dotted, home: false, locked: false, by_stash: false }
    }
}

fn color_of(lane: usize) -> u8 {
    (lane % usize::from(LANE_COLORS)) as u8
}

fn first_free(lanes: &[Option<Lane>], min: usize) -> usize {
    (min..lanes.len()).find(|&i| lanes[i].is_none()).unwrap_or(lanes.len().max(min))
}

/// The layout's whole state between two rows: the lanes (each with the parent it waits for and,
/// for that parent's home lane, its lock, stash flag and chain time). Laying rows out one chunk
/// at a time from one `LayoutState` gives exactly the single pass (spec §8.2 "Continuation").
pub struct LayoutState {
    lanes: Vec<Option<Lane>>,
    /// The pinned lanes (see `layout`): lane `k` is kept for its pinned chain through row
    /// `reserved[k]`.
    reserved: Vec<u32>,
    max_lanes: usize,
}

impl LayoutState {
    /// `reserved`: see `layout`.
    pub fn new(reserved: &[u32]) -> Self {
        LayoutState { lanes: Vec::new(), reserved: reserved.to_vec(), max_lanes: 0 }
    }

    /// The widest the lanes have been so far.
    pub fn max_lanes(&self) -> u16 {
        self.max_lanes as u16
    }

    /// Load more: parents that were outside the window are now loaded rows. Lanes waiting for
    /// them keep their home, lock, stash flag and chain time, so the takeover and lock rules
    /// carry on across the boundary.
    ///
    /// Test-only for now (1C review M9): its owner is the deferred "load more" plan, which will
    /// extend a built layout instead of rebuilding it; the code and its tests are kept for that.
    pub fn resolve_outside(&mut self, row_of: impl Fn(&Oid) -> Option<u32>) {
        for l in self.lanes.iter_mut().flatten() {
            if let Wait::Outside(id) = l.wait
                && let Some(r) = row_of(&id)
            {
                l.wait = Wait::Row(r);
            }
        }
    }

    /// A line already running down into the rows to come, as a graph row's line runs on into a
    /// reveal panel under it: lane `lane` waits for row `r` as its home, locked so no later child
    /// moves it. Before the first `push`, one line per row.
    pub(crate) fn enter(&mut self, lane: u16, r: u32) {
        let lane = usize::from(lane);
        if self.lanes.len() <= lane {
            self.lanes.resize(lane + 1, None);
        }
        self.lanes[lane] = Some(Lane { home: true, locked: true, ..Lane::new(Wait::Row(r), false, false) });
    }

    /// Lays out row `r`, which must come after every row pushed so far. `next` is row `r + 1`'s
    /// node: when it has a stub (`LayoutNode::stub`), the stub starts in this row.
    pub fn push(&mut self, r: u32, node: &LayoutNode, next: Option<&LayoutNode>) -> GraphRow {
        // The lanes still kept for their pinned chains: nothing else opens in them.
        let min_free = self.min_free(r);
        let next_min_free = self.min_free(r + 1);
        let lanes = &mut self.lanes;
        let me = Wait::Row(r);
        let mut home = None;
        let mut awaited = false;
        for (i, l) in lanes.iter().enumerate() {
            if let Some(l) = l
                && l.wait == me
            {
                awaited = true;
                if l.home {
                    home = Some((i, *l));
                    break;
                }
            }
        }
        debug_assert_eq!(home.is_some(), awaited, "row {r}: an awaited commit has exactly one home lane");
        let lane = if let Some(pinned) = node.pinned {
            usize::from(pinned)
        } else if let Some((h, _)) = home {
            h
        } else {
            first_free(lanes, min_free)
        };
        // The chain's newest time: inherited when the commit lands in
        // its home, and the commit's own time otherwise.
        let own_newest = home.map_or(NO_TIME, |(_, l)| l.newest);
        let newest = match home {
            Some((h, l)) if h == lane => l.newest,
            _ => node.time,
        };
        if lanes.len() <= lane {
            lanes.resize(lane + 1, None);
        }

        let mut segments = Vec::with_capacity(lanes.len() + node.parents.len());
        // A fold tail's or a stub's lane ends in this row but stays taken through it, so nothing
        // this row opens runs on from where it ends (two folds in a row alternate lanes).
        let mut ending = Vec::new();
        let mut stub = None;
        for (i, slot) in lanes.iter_mut().enumerate() {
            let Some(l) = *slot else { continue };
            if l.wait == me {
                segments.push(Segment { from_lane: i as u16, to_lane: lane as u16, half: Half::Top, color: color_of(i), dashed: l.dashed, dotted: l.dotted });
                *slot = None;
            } else if l.wait == Wait::Tail(r) {
                // Off the node: the tail's dotted end, in its own lane.
                segments.push(Segment { from_lane: i as u16, to_lane: i as u16, half: Half::Top, color: color_of(i), dashed: false, dotted: true });
                ending.push(i);
            } else if l.wait == Wait::Stub(r) {
                // The stub's fork curve into the node, solid as any child's.
                segments.push(Segment { from_lane: i as u16, to_lane: lane as u16, half: Half::Top, color: color_of(i), dashed: false, dotted: false });
                ending.push(i);
                stub = Some(i as u16);
            } else {
                segments.push(Segment { from_lane: i as u16, to_lane: i as u16, half: Half::Full, color: color_of(i), dashed: l.dashed, dotted: l.dotted });
            }
        }

        let dashed = node.kind == NodeKind::Wip;
        let merge = node.kind == NodeKind::Merge;
        let stash = node.kind == NodeKind::Stash;
        let mut tail = None;
        for (k, parent) in node.parents.iter().enumerate() {
            // A ⋮ row's line down to its run's oldest commit.
            let dotted = node.kind == NodeKind::Collapsed && k == 0;
            let wait = match *parent {
                Parent::Row(p) => Wait::Row(p),
                Parent::Outside(id) => Wait::Outside(id),
                Parent::Tail(n) => Wait::Tail(n),
            };
            // One scan: the parent's home, the first solid lane waiting for it, the first free lane.
            // It stops at the home when that settles the target (a first parent's target is this
            // lane; a merge line goes into a solid home).
            let (mut held, mut solid, mut free) = (None, None, None);
            for (j, slot) in lanes.iter().enumerate() {
                match slot {
                    None if j >= min_free && free.is_none() => free = Some(j),
                    Some(l) if l.wait == wait => {
                        if !l.dashed && solid.is_none() {
                            solid = Some(j);
                        }
                        if l.home {
                            held = Some((j, *l));
                            if k == 0 || !l.dashed {
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
            let target = if k == 0 {
                debug_assert!(
                    lanes.get(lane).copied().flatten().is_none_or(|l| l.wait == wait),
                    "lane {lane} already waits for a different parent; each pinned lane's nodes must form one first-parent chain"
                );
                lane
            } else {
                // Into the parent's home when it's solid. A solid merge line never joins a
                // dashed WIP lane that happens to be waiting on the same parent: it opens or
                // reuses its own lane instead, and the two converge at the parent node via
                // separate Top curves.
                match held {
                    Some((h, l)) if !l.dashed => h,
                    _ => solid.or(free).unwrap_or(lanes.len().max(min_free)),
                }
            };
            if matches!(parent, Parent::Tail(_)) {
                tail = Some(target as u16);
            }
            if lanes.len() <= target {
                lanes.resize(target + 1, None);
            }
            if lanes[target].is_none() {
                lanes[target] = Some(Lane::new(wait, dashed, dotted));
            }
            segments.push(Segment { from_lane: lane as u16, to_lane: target as u16, half: Half::Bottom, color: color_of(target), dashed, dotted });

            match held {
                None => {
                    let l = lanes[target].as_mut().expect("bound above");
                    *l = Lane { home: true, locked: merge, by_stash: stash, newest, ..*l };
                }
                Some((h, l)) => {
                    let mut home_now = h;
                    if k == 0 && h != lane && !l.locked && !merge && !dashed {
                        let stash_steal = l.by_stash && !stash && own_newest != NO_TIME && l.newest != NO_TIME && own_newest > l.newest;
                        if lane < h || stash_steal {
                            lanes[h] = Some(Lane { home: false, ..l });
                            let mine = lanes[lane].as_mut().expect("bound above");
                            *mine = Lane { home: true, locked: false, by_stash: stash, newest: own_newest, ..*mine };
                            home_now = lane;
                        }
                    }
                    if merge {
                        lanes[home_now].as_mut().expect("home lane is bound").locked = true;
                    }
                }
            }
        }
        for i in ending {
            lanes[i] = None;
        }

        // The next row's stub starts here, dotted from this row's centre down, in the first lane
        // nothing in this row touches, and not where the next commit lands when nothing awaits it.
        if let Some(next) = next.filter(|n| n.stub) {
            let awaited = next.pinned.is_some() || lanes.iter().flatten().any(|l| l.wait == Wait::Row(r + 1));
            let lands = (!awaited).then(|| first_free(lanes, next_min_free));
            let touched = |j: usize| j == lane || segments.iter().any(|s| usize::from(s.from_lane) == j || usize::from(s.to_lane) == j);
            let j = (min_free..).find(|&j| lanes.get(j).is_none_or(Option::is_none) && !touched(j) && Some(j) != lands).expect("lanes past the last are free");
            if lanes.len() <= j {
                lanes.resize(j + 1, None);
            }
            lanes[j] = Some(Lane::new(Wait::Stub(r + 1), false, true));
            segments.push(Segment { from_lane: j as u16, to_lane: j as u16, half: Half::Bottom, color: color_of(j), dashed: false, dotted: true });
        }

        self.max_lanes = self.max_lanes.max(lanes.len());
        while matches!(lanes.last(), Some(None)) {
            lanes.pop();
        }
        GraphRow { lane: lane as u16, color: color_of(lane), segments, stub, tail }
    }

    /// Row `r`'s first lane not kept for a pinned chain.
    fn min_free(&self, r: u32) -> usize {
        self.reserved.iter().take_while(|&&until| r <= until).count()
    }
}

/// The column rule, in one top-to-bottom pass whose whole state is the lane vector (`LayoutState`):
///
/// - A commit lands in its home lane: the lane of the first child that reached it. A commit no
///   lane waits for (a tip) takes the left-most free lane.
/// - A later first-parent child in a lower lane takes the home over, unless that child is a
///   merge or a WIP row, or the home is locked. Every parent of a merge is locked, so a trunk of
///   merges keeps its lane and the branches forked from it curve in from either side.
/// - A home a stash set goes to a later first-parent child whose chain is newer than the stash.
///
/// More rules on top: pinned commits always take their pinned lane; a solid merge line never
/// joins a dashed WIP lane; a root commit's lane is freed for the next branch; lines to parents
/// outside the window run off the bottom.
///
/// Branch presets' two marks are lines the allocator places like real ones, so nothing overlaps
/// them: a folded merge's tail (`Parent::Tail`) opens as any merge line does and ends dotted in
/// the row under it; a hidden-branch stub (`LayoutNode::stub`) starts dotted in the row above its
/// commit, in the first lane nothing in that row touches, and curves into the node as a fork does.
/// Each keeps its lane taken through the whole row it ends in.
///
/// `reserved` keeps the pinned lanes left of every other: lane `k` is reserved through row
/// `reserved[k]` (inclusive), so no other branch opens in it. The trunk's lane 0 is reserved for
/// the whole window (`u32::MAX`), from the pinned-ref choice rather than the window, so a
/// truncated window lays out as the prefix of a longer one; a diverged pair's lane 1 through its
/// chain's last row, after which the lane frees up. Empty: nothing is pinned.
///
/// Rows must be in display order with every parent row below its children.
///
/// Invariant: each pinned lane's nodes form a single first-parent chain (each one's first
/// parent is the next one, down to the root, an outside parent, or, for lane 1, a lane-0
/// node). The first-parent binding debug-asserts this: a pinned commit's own lane must be
/// free, or already waiting for that same parent, when it binds its first parent.
pub fn layout(nodes: &[LayoutNode], reserved: &[u32]) -> Layout {
    let mut state = LayoutState::new(reserved);
    let rows = nodes.iter().enumerate().map(|(r, n)| state.push(r as u32, n, nodes.get(r + 1))).collect();
    Layout { rows, max_lanes: state.max_lanes() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dotted_segment_round_trips_through_pack() {
        let s = Segment { from_lane: 3, to_lane: 3, half: Half::Full, color: 3, dashed: false, dotted: true };
        assert_eq!(Segment::unpack(s.pack()), s);
        assert_eq!(s.pack() >> 27 & 1, 1);
        let plain = Segment { dotted: false, ..s };
        assert_eq!(plain.pack() >> 27 & 1, 0, "a plain segment packs as before");
    }

    #[test]
    fn only_a_collapsed_rows_first_parent_lane_is_dotted() {
        let node = |parents: &[u32], kind| LayoutNode { parents: parents.iter().map(|&p| Parent::Row(p)).collect(), kind, pinned: None, time: 0, stub: false };
        // tip → ⋮ → oldest, and another branch onto the oldest beside the dotted lane.
        let nodes = [node(&[1], NodeKind::Commit), node(&[3], NodeKind::Collapsed), node(&[3], NodeKind::Commit), node(&[], NodeKind::Commit)];
        let lay = layout(&nodes, &[]);
        let dotted = |r: usize| -> Vec<(u16, Half)> { lay.rows[r].segments.iter().filter(|s| s.dotted).map(|s| (s.from_lane, s.half)).collect() };
        assert_eq!(dotted(0), []);
        assert_eq!(dotted(1), [(0, Half::Bottom)]);
        assert_eq!(dotted(2), [(0, Half::Full)], "lane 1's own line is solid");
        assert_eq!(dotted(3), [(0, Half::Top)]);
    }

    fn node(parents: &[u32], kind: NodeKind) -> LayoutNode {
        LayoutNode { parents: parents.iter().map(|&p| Parent::Row(p)).collect(), kind, pinned: None, time: 0, stub: false }
    }

    /// The rows' dotted segments: (row, lane, half).
    fn dotted_segments(lay: &Layout) -> Vec<(usize, u16, Half)> {
        lay.rows.iter().enumerate().flat_map(|(r, row)| row.segments.iter().filter(|s| s.dotted).map(move |s| (r, s.from_lane, s.half))).collect()
    }

    #[test]
    fn a_collapsed_row_draws_a_dotted_lane_down_to_its_oldest_commit() {
        // main2 (pinned) on lane 0; tip → ⋮ → (a main commit between) → oldest → main2.
        let mut nodes = vec![node(&[1], NodeKind::Commit), node(&[3], NodeKind::Collapsed), node(&[4], NodeKind::Commit), node(&[4], NodeKind::Commit), node(&[], NodeKind::Commit)];
        for n in [2, 4] {
            nodes[n].pinned = Some(0);
        }
        let lay = layout(&nodes, &[u32::MAX]);
        crate::check_continuity(&lay, &nodes).unwrap();
        let l = lay.rows[1].lane;
        assert_eq!((lay.rows[0].lane, l, lay.rows[3].lane), (1, 1, 1));
        assert_eq!(dotted_segments(&lay), [(1, l, Half::Bottom), (2, l, Half::Full), (3, l, Half::Top)], "the ⋮ lane only, from the ⋮ row to the oldest");
    }

    #[test]
    fn a_collapsed_row_with_extra_parents_draws_solid_merge_lines() {
        // ⋮'s parents: its oldest (row 2), then a trunk commit (row 3) a middle commit merged.
        let mut nodes = vec![node(&[1], NodeKind::Commit), node(&[2, 3], NodeKind::Collapsed), node(&[3], NodeKind::Commit), node(&[], NodeKind::Commit)];
        nodes[3].pinned = Some(0);
        let lay = layout(&nodes, &[u32::MAX]);
        crate::check_continuity(&lay, &nodes).unwrap();
        let bottoms: Vec<(u16, bool)> = lay.rows[1].segments.iter().filter(|s| s.half == Half::Bottom).map(|s| (s.to_lane, s.dotted)).collect();
        assert_eq!(bottoms, [(1, true), (2, false)], "only the first parent's line is dotted");
    }

    // Branch presets' marks: a folded merge's tail and a hidden-branch stub, in the edge cases of
    // the design's spike. The trunk is pinned to lane 0 throughout, as the graph pins it.

    fn on_trunk(parents: &[u32]) -> LayoutNode {
        LayoutNode { pinned: Some(0), ..node(parents, if parents.len() > 1 { NodeKind::Merge } else { NodeKind::Commit }) }
    }

    fn side(parents: &[u32]) -> LayoutNode {
        node(parents, NodeKind::Commit)
    }

    /// A trunk merge at row `r` holding a folded branch: its first parent `p`, its tail into row
    /// `r + 1`.
    fn fold(r: u32, p: u32) -> LayoutNode {
        LayoutNode { parents: vec![Parent::Row(p), Parent::Tail(r + 1)], ..on_trunk(&[p, p]) }
    }

    fn stubbed(n: LayoutNode) -> LayoutNode {
        LayoutNode { stub: true, ..n }
    }

    /// `nodes` laid out: continuous, and each tail's and stub's lane used by nothing else in either
    /// row it's drawn in (the merge's and the one under it; the fork commit's and the one above).
    fn lay_marks(nodes: &[LayoutNode]) -> Layout {
        let lay = layout(nodes, &[u32::MAX]);
        crate::check_continuity(&lay, nodes).unwrap_or_else(|e| panic!("{e}"));
        let alone = |r: usize, lane: u16| {
            let segments = &lay.rows[r].segments;
            assert_eq!(segments.iter().filter(|s| s.from_lane == lane || s.to_lane == lane).count(), 1, "row {r}: lane {lane} is the mark's alone: {segments:?}");
        };
        for (r, row) in lay.rows.iter().enumerate() {
            if let Some(t) = row.tail {
                alone(r, t);
                alone(r + 1, t);
            }
            if let Some(s) = row.stub {
                alone(r - 1, s);
                alone(r, s);
            }
        }
        lay
    }

    fn tails(lay: &Layout) -> Vec<(usize, u16)> {
        lay.rows.iter().enumerate().filter_map(|(r, row)| Some((r, row.tail?))).collect()
    }

    fn stubs(lay: &Layout) -> Vec<(usize, u16)> {
        lay.rows.iter().enumerate().filter_map(|(r, row)| Some((r, row.stub?))).collect()
    }

    #[test]
    fn a_fold_tail_is_a_merge_curve_then_a_dotted_end_in_its_own_lane() {
        // (a) routes 2, 1 (forked from station), main's c13, the fold, station, the first commit:
        // the routes line holds lane 1 past the merge, so the tail opens in lane 2.
        let nodes = [side(&[1]), side(&[4]), on_trunk(&[3]), fold(3, 4), on_trunk(&[5]), on_trunk(&[])];
        let lay = lay_marks(&nodes);
        assert_eq!(lay.rows[1].lane, 1);
        assert_eq!(tails(&lay), [(3, 2)]);
        let merge: Vec<(u16, u16, Half, bool)> = lay.rows[3].segments.iter().map(|s| (s.from_lane, s.to_lane, s.half, s.dotted)).collect();
        assert_eq!(merge, [(0, 0, Half::Top, false), (1, 1, Half::Full, false), (0, 0, Half::Bottom, false), (0, 2, Half::Bottom, false)], "the ordinary merge curve, solid");
        let end: Vec<(u16, u16, Half, bool)> = lay.rows[4].segments.iter().filter(|s| s.from_lane == 2).map(|s| (s.from_lane, s.to_lane, s.half, s.dotted)).collect();
        assert_eq!(end, [(2, 2, Half::Top, true)], "a dotted top half, off the node");
        assert_eq!(lay.rows[3].segments[3].color, lay.rows[4].segments.iter().find(|s| s.from_lane == 2).unwrap().color, "in its lane's colour");
        assert_eq!(lay.max_lanes, 3);
        assert!(lay.rows[5].segments.iter().all(|s| s.from_lane != 2 && s.to_lane != 2), "and nothing below");
    }

    #[test]
    fn adjacent_fold_tails_alternate_lanes() {
        // (b) Three folds in a row, a commit, then two more: each tail's lane is held through the
        // row it ends in, so the next merge's opens beside it.
        let nodes = [on_trunk(&[1]), fold(1, 2), fold(2, 3), fold(3, 4), on_trunk(&[5]), fold(5, 6), fold(6, 7), on_trunk(&[])];
        let lay = lay_marks(&nodes);
        assert_eq!(tails(&lay), [(1, 1), (2, 2), (3, 1), (5, 1), (6, 2)]);
        assert_eq!(lay.max_lanes, 3);
    }

    #[test]
    fn a_fold_tail_beside_a_collapsed_run_crosses_its_dotted_lane() {
        // (c) main's c13; payments' receipts, its ⋮ row and its ledger model (from station); the
        // fold between the ⋮ row and the ledger; station; the first commit.
        let nodes = [on_trunk(&[3]), side(&[2]), node(&[4], NodeKind::Collapsed), fold(3, 5), side(&[5]), on_trunk(&[6]), on_trunk(&[])];
        let lay = lay_marks(&nodes);
        assert_eq!((lay.rows[2].lane, lay.rows[4].lane), (1, 1));
        assert_eq!(tails(&lay), [(3, 2)]);
        assert!(lay.rows[3].segments.iter().any(|s| s.from_lane == 1 && s.half == Half::Full && s.dotted), "the ⋮ lane runs past the merge");
    }

    #[test]
    fn a_fold_tail_takes_the_lane_the_branch_would_and_no_more() {
        // (d) Three branches from the first commit, lanes 1-3 busy on the fold's row and the next.
        let folded = [side(&[6]), side(&[6]), side(&[6]), on_trunk(&[4]), fold(4, 5), on_trunk(&[6]), on_trunk(&[])];
        let lay = lay_marks(&folded);
        assert_eq!(tails(&lay), [(4, 4)]);
        // The folded branch shown: its one commit between the merge and station.
        let shown = [side(&[7]), side(&[7]), side(&[7]), on_trunk(&[4]), on_trunk(&[6, 5]), side(&[7]), on_trunk(&[7]), on_trunk(&[])];
        let real = layout(&shown, &[u32::MAX]);
        assert_eq!(real.rows[5].lane, 4, "the branch's own lane");
        assert_eq!(lay.max_lanes, real.max_lanes);
    }

    #[test]
    fn a_stub_takes_the_first_lane_nothing_in_the_row_above_touches() {
        // (h1) cargo and routes fork from station, as do the hidden branches: the stub comes in
        // from lane 3, right of both lines.
        let lay = lay_marks(&[side(&[3]), side(&[3]), on_trunk(&[3]), stubbed(on_trunk(&[4])), on_trunk(&[])]);
        assert_eq!(stubs(&lay), [(3, 3)]);
        let start: Vec<(u16, Half, bool)> = lay.rows[2].segments.iter().filter(|s| s.from_lane == 3).map(|s| (s.to_lane, s.half, s.dotted)).collect();
        assert_eq!(start, [(3, Half::Bottom, true)], "a dotted bottom half in the row above, off its node");
        let into: Vec<(u16, u16, bool)> = lay.rows[3].segments.iter().filter(|s| s.half == Half::Top).map(|s| (s.from_lane, s.to_lane, s.dotted)).collect();
        assert_eq!(into, [(0, 0, false), (1, 0, false), (2, 0, false), (3, 0, false)], "then the ordinary fork curve into the node, solid");
        assert_eq!(lay.max_lanes, 4);
        // (h5) A shown branch passing in lane 1, another's tip right above in lane 2: lane 3.
        let lay = lay_marks(&[side(&[4]), on_trunk(&[3]), side(&[3]), stubbed(on_trunk(&[4])), on_trunk(&[])]);
        assert_eq!(lay.rows[2].lane, 2);
        assert_eq!(stubs(&lay), [(3, 3)]);
    }

    #[test]
    fn stubs_on_adjacent_rows_alternate_lanes() {
        // (h3) Hidden branches fork from two commits in a row.
        let lay = lay_marks(&[on_trunk(&[1]), stubbed(on_trunk(&[2])), stubbed(on_trunk(&[3])), on_trunk(&[])]);
        assert_eq!(stubs(&lay), [(1, 1), (2, 2)]);
    }

    #[test]
    fn a_stub_on_a_folded_merge_and_under_it_clear_its_tail() {
        // (h4) A stub into the merge, its tail, and a stub into the commit under it.
        let lay = lay_marks(&[on_trunk(&[1]), stubbed(fold(1, 2)), stubbed(on_trunk(&[3])), on_trunk(&[])]);
        assert_eq!((stubs(&lay), tails(&lay)), (vec![(1, 1), (2, 3)], vec![(1, 2)]));
    }

    #[test]
    fn a_stub_never_takes_the_lane_its_commit_lands_in() {
        // A branch tip nothing awaits lands in the first free lane, 1: its stub comes from lane 2.
        let lay = lay_marks(&[on_trunk(&[2]), stubbed(side(&[2])), on_trunk(&[])]);
        assert_eq!((lay.rows[1].lane, stubs(&lay)), (1, vec![(1, 2)]));
        // A pinned commit lands in its own lane: the first free one is the stub's (as in h3).
        let lay = lay_marks(&[on_trunk(&[1]), stubbed(on_trunk(&[2])), on_trunk(&[])]);
        assert_eq!(stubs(&lay), [(1, 1)]);
    }

    #[test]
    fn the_continuity_check_follows_tails_and_stubs() {
        let nodes = [on_trunk(&[1]), stubbed(fold(1, 2)), stubbed(on_trunk(&[3])), on_trunk(&[])];
        let lay = lay_marks(&nodes);
        let broken = |r: usize, f: &dyn Fn(&mut Vec<Segment>)| {
            let mut bad = lay.clone();
            f(&mut bad.rows[r].segments);
            crate::check_continuity(&bad, &nodes).expect_err("broken")
        };
        // The tail's dotted end gone, or running on into the next row's node.
        assert!(broken(2, &|s| s.retain(|s| !(s.from_lane == 2 && s.half == Half::Top))).contains("broke"));
        assert!(broken(2, &|s| s.iter_mut().filter(|s| s.from_lane == 2).for_each(|s| s.to_lane = 0)).contains("the fold tail on lane 2 enters the node"));
        // The second stub's start missing.
        assert!(broken(1, &|s| s.retain(|s| s.from_lane != 3)).contains("0 stubs start here"));
    }
}
