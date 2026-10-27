//! Branch presets as they are stored (the settings files' shape) and sent back in a `ViewSpec`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A named, coloured view of the graph: rules deciding each branch, and two options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BranchPreset {
    pub id: String,
    pub name: String,
    /// A tab-group palette colour's name (`GROUP_COLORS`: `blue`, `purple`, …).
    pub color: String,
    pub scope: PresetScope,
    /// The first rule that matches a branch decides.
    pub rules: Vec<Rule>,
    /// "Everything else": what a branch no rule matches gets.
    pub otherwise: PresetAction,
    /// Fold branches merged into the trunk into their merge commit.
    pub fold_merged: bool,
    /// A stub where branches its rules hide fork (named for the ghost lines it once drew).
    pub ghost_lines: bool,
    /// The preset button's click steps through it.
    pub in_cycle: bool,
}

/// Where a preset is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PresetScope {
    All,
    Repo,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PresetAction {
    #[default]
    Show,
    Hide,
    Collapse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Rule {
    pub action: PresetAction,
    pub matcher: Matcher,
}

/// What a rule matches a branch by. "In the loaded commits" for `CommitsBy` and `OlderThan`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Matcher {
    /// One of the repository's trunk branches.
    Trunk,
    /// Checked out in any worktree.
    CheckedOut,
    /// Has a commit not on the trunk by this author.
    CommitsBy { author: Author },
    /// Comma-separated globs (`*` crosses `/`), against the branch name or any `remote/name`.
    NameMatches { patterns: String },
    /// Has no local branch.
    Remote,
    /// Has a local branch.
    Local,
    /// Its newest commit is older than this many days.
    OlderThan { days: u32 },
    /// Its tip is reachable from a trunk branch.
    MergedIntoTrunk,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum Author {
    /// Whoever uses the preset: the repository's `user.email` and the profile's forge accounts' emails.
    Me,
    /// An email or a name, compared case-insensitively.
    Person { who: String },
}

/// Branches hidden or shown by hand (unit names), applied before the rules. A name with a trailing
/// slash (`deployed-commit/`) is a folder: it picks every branch under it, also those created later,
/// unless a pick nearer the branch (its own, or a deeper folder's) says otherwise.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BranchPicks {
    pub shown: Vec<String>,
    pub hidden: Vec<String>,
}

/// Where a collapsed branch's oldest commit goes (Settings › Graph › "Collapsed branches").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CollapsePlacement {
    /// At its own date, the ⋮ row's dotted lane running down to it.
    #[default]
    KeepDateOrder,
    /// Right under the ⋮ row: the branch is three adjacent rows.
    PullTogether,
}

/// A built-in preset's place in the list: its order and whether it's in the cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PresetSlot {
    pub id: String,
    pub in_cycle: bool,
}
