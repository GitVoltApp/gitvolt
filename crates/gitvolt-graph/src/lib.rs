//! The commit graph's pure pipeline (website demo spec §4.2): from plain data (the walked commits,
//! the refs, the worktrees and their WIP) to `GraphPayload`. No git access and no I/O: gitvolt-core
//! feeds it what gix reads, and the website demo (through gitvolt-graph-wasm) what isomorphic-git
//! reads, so both lay a repository out identically.

mod ascii;
mod assemble;
#[cfg(any(test, feature = "testing"))]
mod check;
mod input;
mod labels;
pub mod layout;
mod message;
mod message_refs;
mod oid;
mod payload;
mod pins;
// The rule engine's and the view transform's crate-internal parts are stubs that lanes A and B
// fill in and call from `assemble`: drop these `allow`s then.
#[allow(dead_code, unused_imports)]
mod presets;
#[cfg(test)]
mod testkit;
#[allow(dead_code)]
mod view;
mod walk;

pub use ascii::{render_ascii, AsciiRow};
pub use assemble::{assemble, build, build_hidden_owner, build_preview, build_reveal, graph_tips, walk_order};
#[cfg(any(test, feature = "testing"))]
pub use check::check_continuity;
pub use input::{CommitIn, ForkFacts, GraphInput, GraphRequest, HeadIn, RefIn, RefKind, RefsIn, StashIn, WorktreeIn};
pub use layout::{layout, GraphRow, Half, Layout, LayoutNode, LayoutState, NodeKind, Parent, Segment, LANE_COLORS};
pub use message::split_message;
pub use message_refs::{parse_message_refs, MAX_MESSAGE_REFS};
pub use oid::Oid;
pub use payload::{
    BranchState, BranchView, CollapsedRow, DecidedBy, FoldMark, ForkDetection, ForkPayload, GhostBranch, GraphPayload, GraphWorktree, HeadPayload, HostKind, InProgress, PresetPreview, RefLabel,
    RemoteRefLabel, RevealPayload, RevealTag, RowPayload, TagAnnotation, ViewPayload, WipCounts, WipPayload,
};
pub use presets::{Author, BranchPicks, BranchPreset, CollapsePlacement, Matcher, PresetAction, PresetScope, PresetSlot, Rule};
pub use view::{hidden_owner, preview, reveal, ViewSpec};
pub use walk::{walk, walk_by_date, CommitMeta, WalkOptions, WalkResult};
