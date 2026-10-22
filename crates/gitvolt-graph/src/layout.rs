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

    /// Lays out row `r`, which must come after every row pushed so far.
    pub fn push(&mut self, r: u32, node: &LayoutNode) -> GraphRow {
        // The lanes still kept for their pinned chains: nothing else opens in them.
        let min_free = self.reserved.iter().take_while(|&&until| r <= until).count();
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
        for (i, slot) in lanes.iter_mut().enumerate() {
            let Some(l) = *slot else { continue };
            if l.wait == me {
                segments.push(Segment { from_lane: i as u16, to_lane: lane as u16, half: Half::Top, color: color_of(i), dashed: l.dashed, dotted: l.dotted });
                *slot = None;
            } else {
                segments.push(Segment { from_lane: i as u16, to_lane: i as u16, half: Half::Full, color: color_of(i), dashed: l.dashed, dotted: l.dotted });
            }
        }

        let dashed = node.kind == NodeKind::Wip;
        let merge = node.kind == NodeKind::Merge;
        let stash = node.kind == NodeKind::Stash;
        for (k, parent) in node.parents.iter().enumerate() {
            // A ⋮ row's line down to its run's oldest commit.
            let dotted = node.kind == NodeKind::Collapsed && k == 0;
            let wait = match *parent {
                Parent::Row(p) => Wait::Row(p),
                Parent::Outside(id) => Wait::Outside(id),
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

        self.max_lanes = self.max_lanes.max(lanes.len());
        while matches!(lanes.last(), Some(None)) {
            lanes.pop();
        }
        GraphRow { lane: lane as u16, color: color_of(lane), segments }
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
    let rows = nodes.iter().enumerate().map(|(r, n)| state.push(r as u32, n)).collect();
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
        let node = |parents: &[u32], kind| LayoutNode { parents: parents.iter().map(|&p| Parent::Row(p)).collect(), kind, pinned: None, time: 0 };
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
        LayoutNode { parents: parents.iter().map(|&p| Parent::Row(p)).collect(), kind, pinned: None, time: 0 }
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
}
