//! The graph's payload types, serialized to the UI (TypeScript from ts-rs, `just gen-types`, into
//! ui/src/api/gen/ like the core's), and the per-worktree data it carries.

use crate::layout::{GraphRow, NodeKind, Segment};
use crate::walk::CommitMeta;
use crate::{render_ascii, AsciiRow};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GraphPayload {
    pub rows: Vec<RowPayload>,
    pub labels: Vec<RefLabel>,
    pub max_lanes: u16,
    /// The pinned refs, the one leading lane 0 first: the chosen pin (the default trunk or an
    /// explicit one) and its pair, a local branch's upstream or the local branch tracking a remote
    /// one, local first; in a fork by default, the original project's trunk then the local main.
    /// Their commits are laid out at the left (lane 0, and lane 1 for a diverged pair's second).
    /// Only refs whose commit is in the window; empty when nothing is pinned.
    pub pinned_refs: Vec<String>,
    pub head: HeadPayload,
    pub truncated: bool,
    /// The open worktree (the one this tab opened), spelled as its WIP row's
    /// `WipPayload::worktree_path` would be, dirty or not: what "Compare with working tree"
    /// targets. Absent if it isn't among `git worktree list`'s usable ones.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub open_worktree: Option<String>,
    /// Every usable worktree of the repository (spec #2 §11.2).
    pub worktrees: Vec<GraphWorktree>,
    // 2D-T6 begin
    /// Each worktree's merge, rebase or other in-progress operation (spec #2 §13.2), keyed by its
    /// path as its WIP row's `worktreePath` spells it (Deviation 1). Absent when none is mid-operation.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    #[ts(optional, as = "Option<std::collections::BTreeMap<String, crate::InProgress>>")]
    pub in_progress: std::collections::BTreeMap<String, InProgress>,
    // 2D-T6 end
    /// The repository is a fork (`snapshot::fork_of`): which remote is your fork, which the
    /// original project, and how that was told. Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fork: Option<ForkPayload>,
    /// Under a branch preset: every branch's state and the trunk. Absent without one, so All's
    /// payload is the plain graph's.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub view: Option<ViewPayload>,
}

/// A fork's remotes as the graph sees them: the default pin is the original project's trunk,
/// and a commit on a local branch standing for a trunk here gets a notice (the commit panel).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ForkPayload {
    /// Your fork's remote (`origin`).
    pub fork: String,
    /// The original project's remote (`upstream`).
    pub original: String,
    /// The original remote's host, and the forge it's detected as (the UI applies the
    /// profile's host-type overrides by host): what a change for it is called, a merge request
    /// on GitLab.
    pub host: Option<String>,
    pub host_kind: HostKind,
    pub detected_by: ForkDetection,
    /// The original project's trunk (`refs/remotes/upstream/main`); `None` until it's fetched.
    pub trunk: Option<String>,
    /// Your fork's own trunk (`refs/remotes/origin/main`), never pinned by default.
    pub fork_trunk: Option<String>,
    /// The local branches a commit on which goes to a trunk rather than a branch of its own:
    /// `trunk`'s local counterpart (the default pin's local branch) and every local branch whose
    /// upstream is `fork_trunk`. Full names, sorted.
    pub trunk_branches: Vec<String>,
}

/// How a fork was told (`ForkPayload::detected_by`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ForkDetection {
    /// The forge's cached data: the fork remote's project is a fork of the original remote's.
    Forge,
    /// No forge data: a remote named `upstream` next to `origin` (GitHub's fork workflow).
    UpstreamRemote,
}

/// One usable worktree as the graph sees it (spec #2 §11.2): the UI moves the HEAD marker and
/// the "checked out elsewhere" chips to the tab's active worktree from these, with no request.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GraphWorktree {
    /// Spelled as its WIP row's `WipPayload::worktree_path`.
    pub path: String,
    /// The full ref its HEAD names; `None` when detached or bare.
    pub branch: Option<String>,
    pub head: Option<String>,
    pub is_main: bool,
    pub locked: bool,
    /// The operation in progress there, by kind (`merge`, `rebase`, or what git calls the other
    /// one: `cherry-pick`, `revert`, `am`); the detail is in `GraphPayload.in_progress`, keyed by
    /// this path.
    pub in_progress: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RowPayload {
    pub id: String,
    pub kind: NodeKind,
    pub lane: u16,
    pub color: u8,
    /// Packed segments; see `graph::Segment::pack`.
    pub segments: Vec<u32>,
    pub summary: String,
    pub body_first_line: String,
    pub author_name: String,
    pub author_email: String,
    #[ts(type = "number")]
    pub author_time: i64,
    /// Committer timestamp (unix seconds): what the Date/Time column shows. It
    /// differs from `author_time` once a commit is amended, rebased or cherry-picked. 0 on WIP rows.
    #[ts(type = "number")]
    pub committer_time: i64,
    pub parents: Vec<String>,
    /// MR/PR/issue references in the summary and body, as written (`!42`, `group/project!7`,
    /// `#12`, `owner/repo#3`), first occurrence first, at most 20 (`message_refs`, §9.2). The
    /// host decides what each means. Empty on WIP rows.
    pub mr_refs: Vec<String>,
    pub wip: Option<WipPayload>,
    /// A `collapsed` row (the ⋮ row): its branch and what it holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub collapsed: Option<CollapsedRow>,
    /// A merge on the trunk that holds a folded branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fold: Option<FoldMark>,
    /// Hidden branches forking from this commit, newest tip first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ghosts: Vec<GhostBranch>,
    /// The lane of the stub marking where `ghosts` fork: a dotted line from the row above's centre
    /// that curves into this row's node (the layout's `GraphRow::stub`). None on row 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stub_lane: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WipPayload {
    pub worktree_path: String,
    pub worktree_name: Option<String>,
    pub modified: u32,
    pub added: u32,
    pub deleted: u32,
    pub renamed: u32,
    pub conflicted: u32,
}

#[derive(Debug, Clone, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HeadPayload {
    pub branch: Option<String>,
    pub target: Option<String>,
    pub detached: bool,
    pub unborn: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RefLabel {
    pub row: u32,
    pub name: String,
    pub local: Option<String>,
    pub remotes: Vec<RemoteRefLabel>,
    pub tag: bool,
    pub is_head: bool,
    pub worktree: Option<String>,
    /// The worktree (any, the open one included) whose HEAD is this local branch; on a worktree's
    /// detached-HEAD chip (no local, remote or tag), that worktree.
    pub checked_out: Option<String>,
    /// A local branch whose upstream has another branch name (`feature/a` tracking
    /// `origin/feature/b`): that upstream's short name, for the chip's warning (UX round 3, M.1).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub upstream_mismatch: Option<String>,
    /// An annotated tag's message and tagger (UX round 3, M.2); absent on a lightweight tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub annotation: Option<TagAnnotation>,
}

/// An annotated tag's message and tagger, for the tag chip's and the sidebar row's tooltip (UX
/// round 3, M.2). Read in process with the refs, cached by tag object (`refs::read_tag`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TagAnnotation {
    /// The message's first `refs::TAG_MESSAGE_LINES` lines (any signature left out), trimmed.
    pub message: String,
    /// More of the message follows.
    pub truncated: bool,
    /// The tagger's name.
    pub tagger: Option<String>,
    /// When it was tagged (unix seconds); 0 without a tagger.
    #[ts(type = "number")]
    pub time: i64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoteRefLabel {
    pub full_name: String,
    pub remote: String,
    /// The remote's host name, so the UI can apply the profile's host-type override.
    pub host: Option<String>,
    pub host_kind: HostKind,
}

impl GraphPayload {
    /// ASCII dump: layout grid, then the row summary (WIP rows show their worktree).
    pub fn ascii(&self) -> String {
        let segs: Vec<Vec<Segment>> = self.rows.iter().map(|r| r.segments.iter().map(|&s| Segment::unpack(s)).collect()).collect();
        let labels: Vec<String> = self
            .rows
            .iter()
            .map(|r| match &r.wip {
                Some(w) => format!("// WIP {}", w.worktree_name.as_deref().unwrap_or("(main)")),
                None => r.summary.clone(),
            })
            .collect();
        let rows: Vec<AsciiRow> = self
            .rows
            .iter()
            .zip(&segs)
            .zip(&labels)
            .map(|((r, s), l)| AsciiRow { lane: r.lane, kind: r.kind, segments: s, label: l })
            .collect();
        render_ascii(&rows, self.max_lanes)
    }
}

impl RowPayload {
    /// A commit's row, laid out as `g` with its packed `segments`: no WIP, fold mark or ghosts.
    pub(crate) fn commit(c: &CommitMeta, kind: NodeKind, g: &GraphRow, segments: Vec<u32>) -> Self {
        RowPayload {
            id: c.id.to_string(),
            kind,
            lane: g.lane,
            color: g.color,
            segments,
            summary: c.summary.clone(),
            body_first_line: c.body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string(),
            author_name: c.author_name.clone(),
            author_email: c.author_email.clone(),
            author_time: c.author_time,
            committer_time: c.committer_time,
            parents: c.parents.iter().map(|p| p.to_string()).collect(),
            // Parsed by the walk from the full message: no second object lookup.
            mr_refs: c.mr_refs.clone(),
            wip: None,
            collapsed: None,
            fold: None,
            ghosts: vec![],
            stub_lane: None,
        }
    }

    /// The ⋮ row of a collapsed run: its id `collapsed:<branch>@<newest sha>` (its key's), no
    /// summary, author or parents, times 0.
    pub fn collapsed(row: CollapsedRow, lane: u16, color: u8, segments: Vec<u32>) -> Self {
        RowPayload {
            id: format!("collapsed:{}", row.key.strip_prefix("branch:").unwrap_or(&row.key)),
            kind: NodeKind::Collapsed,
            lane,
            color,
            segments,
            summary: String::new(),
            body_first_line: String::new(),
            author_name: String::new(),
            author_email: String::new(),
            author_time: 0,
            committer_time: 0,
            parents: vec![],
            mr_refs: vec![],
            wip: None,
            collapsed: Some(row),
            fold: None,
            ghosts: vec![],
            stub_lane: None,
        }
    }
}

// `Deserialize` and `Ord`: the profile's host-type overrides are a map keyed by host (§14.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum HostKind {
    GitLab,
    GitHub,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum InProgress {
    /// `MERGE_HEAD` exists. `message`: `MERGE_MSG`, line endings normalised (§8.2).
    Merge { merge_head: String, message: String, conflicted: u32 },
    /// `rebase-merge/` (or `rebase-apply/`). `onto` and `stopped_at` are oids; `head_name` is the
    /// branch being rebased (`refs/heads/main`). `message`: what Continue commits the stopped
    /// pick with (`rebase-merge/message`, `rebase-apply/final-commit`), line endings normalised;
    /// empty when git wrote none.
    Rebase {
        onto: String,
        head_name: String,
        step: u32,
        total: u32,
        stopped_at: Option<String>,
        // --- 3C T4 ---
        /// At an `edit` stop: the commit git made there (`rebase-merge/amend`). `None` at a
        /// conflict, or a stop of another kind.
        edit_stop: Option<String>,
        // --- end 3C T4 ---
        /// UX L: GitVolt's Edit stop is "about to commit": right after git stopped, HEAD was
        /// soft-reset to the edited commit's parent (this oid), so its changes are staged and its
        /// message (`message`) is the commit box's. `None`: git's own stop (HEAD on `edit_stop`),
        /// as a terminal's rebase has it.
        edit_base: Option<String>,
        /// UX L, with `edit_base`: the paths the stopped commit adds. One left untracked is a
        /// piece of it not staged yet: Continue waits for it (any other untracked file doesn't).
        edit_added: Vec<String>,
        /// UX L, with `edit_base`: the index isn't the stopped commit's tree (changed, or some of
        /// it unstaged): an Abort keeps it.
        edit_changed: bool,
        /// 3C final fix (I1): the stopped pick is an `edit` line that stopped before git made its
        /// commit (a conflict): git won't stop again once it's resolved, so this stop is the Edit's.
        edit_conflict: bool,
        /// 3C final fix (M1, M2): GitVolt couldn't apply this stop's new message (a commit-msg
        /// hook refused it): why. HEAD keeps the old one.
        message_failed: Option<String>,
        /// 3C fix round 2: GitVolt's own interactive rebase (its session is there): Commit at
        /// its Edit stops is GitVolt's. `false` for one started in a terminal.
        gitvolt: bool,
        conflicted: u32,
        message: String,
    },
    /// `CHERRY_PICK_HEAD` (a single pick, or a range's, `.git/sequencer/` too). `head`: the commit
    /// being picked; `message`: `MERGE_MSG`, what Continue commits it with. A range stopped
    /// between picks (`sequencer/` alone) isn't an operation here, as for gix.
    CherryPick { head: Option<String>, message: String, conflicted: u32 },
    /// `REVERT_HEAD`: as `CherryPick`, for the commit being reverted.
    Revert { head: Option<String>, message: String, conflicted: u32 },
    /// `git am`, started outside GitVolt; #3 adds its actions.
    Other { what: String },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, TS)]
#[serde(default)]
#[ts(export, export_to = "../../demo/gen/")]
pub struct WipCounts {
    pub modified: u32,
    pub added: u32,
    pub deleted: u32,
    /// Renamed in the index (`R`); one per path, so a staged and unstaged edit of the same path
    /// is one file, as in the WIP panel's "N file changes".
    pub renamed: u32,
    pub conflicted: u32,
}

impl WipCounts {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

// Branch presets (the view transform, `crate::view`).

/// Why a branch got its state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum DecidedBy {
    Trunk,
    /// Checked out in a worktree: shown although a rule or Everything else hides it (or a pick,
    /// in the tab's own worktree).
    CheckedOut,
    /// Hidden or shown by hand.
    Picked,
    /// "Expand in graph", or a navigation to one of its commits, this session.
    Expanded,
    /// The preset's rule at `index`.
    Rule { index: u16 },
    /// Everything else.
    Otherwise,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BranchState {
    Shown,
    Hidden,
    Collapsed,
    /// Merged into the trunk, its commits folded into the merge.
    Folded,
}

/// One branch unit under a preset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BranchView {
    /// The unit's name: the short branch name (`feature/x`), whichever remotes carry it.
    pub name: String,
    /// Its full ref names.
    pub refs: Vec<String>,
    pub state: BranchState,
    pub by: DecidedBy,
}

/// `GraphPayload::view`: every branch unit's state, sorted by name, and the trunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ViewPayload {
    pub branches: Vec<BranchView>,
    /// The trunk units' names.
    pub trunk: Vec<String>,
}

/// The ⋮ row of a collapsed branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CollapsedRow {
    /// "branch:<name>@<newest sha>": its reveal panel's key (a branch can have several runs, one
    /// per line of history). Its branch's `expanded` key, for all of them, is "branch:<name>".
    pub key: String,
    pub branch: String,
    /// The commits folded into the row.
    pub count: u32,
    /// Distinct authors among them.
    pub authors: u32,
}

/// A trunk merge's folded branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FoldMark {
    /// "fold:<merge sha>".
    pub key: String,
    /// The merged branch: its ref's name, else the one the merge message gives, else "merged branch".
    pub branch: String,
    pub count: u32,
    /// The lane of its tail: the merge's curve into it, then its dotted end in the row under the
    /// merge (the layout's `GraphRow::tail`). None when the merge is the window's last row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tail_lane: Option<u16>,
}

/// A hidden branch forking from a shown commit, where the row's stub is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GhostBranch {
    pub branch: String,
    /// Its lane colour index: the stub's lane's, the same for every hidden branch of the row.
    pub color: u8,
    /// Its newest tip in the window.
    pub tip: String,
    pub summary: String,
    pub author_name: String,
    #[ts(type = "number")]
    pub author_time: i64,
}

/// A small dim badge after a reveal row's message (`main`, `parent`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RevealTag {
    pub row: u32,
    pub text: String,
}

/// The reveal panel's rows (`graphReveal`), laid out on the graph's lanes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RevealPayload {
    pub key: String,
    pub branch: String,
    pub count: u32,
    pub rows: Vec<RowPayload>,
    pub tags: Vec<RevealTag>,
    /// The lane the panel's branch line meets the graph in.
    pub entry_lane: u16,
    pub max_lanes: u16,
}

/// The settings page's live summary of a preset over the open repository's window.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PresetPreview {
    /// The branches each rule decides, then Everything else's last.
    pub rules: Vec<u32>,
    pub shown: u32,
    pub collapsed: u32,
    pub hidden: u32,
    /// Merges holding a folded branch.
    pub folded: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> RowPayload {
        RowPayload {
            id: "a".repeat(40),
            kind: NodeKind::Commit,
            lane: 0,
            color: 0,
            segments: vec![],
            summary: "Initial commit".into(),
            body_first_line: String::new(),
            author_name: "Mara Quill".into(),
            author_email: "mara@example.com".into(),
            author_time: 100,
            committer_time: 100,
            parents: vec![],
            mr_refs: vec![],
            wip: None,
            collapsed: None,
            fold: None,
            ghosts: vec![],
            stub_lane: None,
        }
    }

    #[test]
    fn a_row_without_view_extras_serializes_as_before() {
        let json = serde_json::to_value(row()).unwrap();
        let keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        for extra in ["collapsed", "fold", "ghosts"] {
            assert!(!keys.contains(&extra), "{extra} in {keys:?}");
        }
        assert_eq!(serde_json::from_value::<RowPayload>(json).unwrap(), row(), "and reads back");
        let g = GraphPayload {
            rows: vec![row()],
            labels: vec![],
            max_lanes: 1,
            pinned_refs: vec![],
            head: HeadPayload::default(),
            truncated: false,
            open_worktree: None,
            worktrees: vec![],
            in_progress: Default::default(),
            fork: None,
            view: None,
        };
        assert!(serde_json::to_value(&g).unwrap().get("view").is_none());
    }

    #[test]
    fn a_collapsed_row_carries_its_extras() {
        let r = RowPayload { kind: NodeKind::Collapsed, collapsed: Some(CollapsedRow { key: "branch:feature/x".into(), branch: "feature/x".into(), count: 4, authors: 2 }), ..row() };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["kind"], "collapsed");
        assert_eq!(json["collapsed"], serde_json::json!({"key": "branch:feature/x", "branch": "feature/x", "count": 4, "authors": 2}));
        assert_eq!(serde_json::to_value(DecidedBy::Rule { index: 2 }).unwrap(), serde_json::json!({"kind": "rule", "index": 2}));
        assert_eq!(serde_json::to_value(BranchState::Folded).unwrap(), "folded");
    }
}
