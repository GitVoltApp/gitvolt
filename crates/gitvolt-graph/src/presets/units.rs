//! Branch units: a local branch together with its remote-tracking refs, or a remote-only
//! branch, keyed by the short branch name; and which of them are the trunk.

use crate::input::{RefKind, RefsIn};
use crate::payload::DecidedBy;
use crate::presets::{name_matches, PresetAction};
use crate::Oid;
use std::collections::BTreeMap;

/// One branch as a chip shows it: `refs/heads/<name>` and every `refs/remotes/<r>/<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BranchUnit {
    pub name: String,
    /// Full ref names.
    pub refs: Vec<String>,
    pub tips: Vec<Oid>,
    pub has_local: bool,
}

/// What a preset does with one unit, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Decision {
    pub action: PresetAction,
    pub by: DecidedBy,
    /// The first rule that matched, in rule order (for the settings page's counts); `None`:
    /// Everything else.
    pub rule: Option<u16>,
}

/// The repository's branch units, sorted by name: each one's local ref first, then its remote
/// refs in the order `refs` lists them, and its distinct tips. Tags and `refs/remotes/<r>/HEAD`
/// aren't units.
pub(crate) fn branch_units(refs: &RefsIn) -> Vec<BranchUnit> {
    let mut units: BTreeMap<&str, BranchUnit> = BTreeMap::new();
    for r in &refs.refs {
        let name = match &r.kind {
            RefKind::Local => r.short_name.as_str(),
            RefKind::Remote { remote } => match r.short_name.get(remote.len() + 1..) {
                Some(name) if !name.is_empty() && name != "HEAD" => name,
                _ => continue,
            },
            RefKind::Tag => continue,
        };
        let unit = units.entry(name).or_insert_with(|| BranchUnit { name: name.to_string(), refs: Vec::new(), tips: Vec::new(), has_local: false });
        if r.kind == RefKind::Local {
            unit.has_local = true;
            unit.refs.insert(0, r.full_name.clone());
        } else {
            unit.refs.push(r.full_name.clone());
        }
        if !unit.tips.contains(&r.target) {
            unit.tips.push(r.target);
        }
    }
    units.into_values().collect()
}

/// Per unit: is it a trunk branch (its refs include a `pinned` name, or its name matches one of
/// the repository's extra trunk `patterns`).
pub(crate) fn trunk_flags(units: &[BranchUnit], pinned: &[String], patterns: &[String]) -> Vec<bool> {
    units.iter().map(|u| u.refs.iter().any(|r| pinned.contains(r)) || patterns.iter().any(|p| name_matches(p, u))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::glob_match;
    use crate::testkit::{commit, local, remote, request, tag, worktree};

    fn units_of(refs: impl IntoIterator<Item = crate::RefIn>) -> Vec<BranchUnit> {
        branch_units(&RefsIn { refs: refs.into_iter().collect(), ..Default::default() })
    }

    fn unit<'a>(units: &'a [BranchUnit], name: &str) -> &'a BranchUnit {
        units.iter().find(|u| u.name == name).unwrap()
    }

    #[test]
    fn a_local_branch_and_its_remote_twins_are_one_unit() {
        let units = units_of([remote("origin", "x", 2), local("x", 1), remote("upstream", "x", 2), remote("origin", "y", 3), remote("origin", "HEAD", 2), tag("v1", 1)]);
        assert_eq!(units.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(), ["x", "y"], "no HEAD, no tag");
        let x = unit(&units, "x");
        assert_eq!(x.refs, ["refs/heads/x", "refs/remotes/origin/x", "refs/remotes/upstream/x"]);
        assert_eq!(x.tips, [crate::testkit::id(2), crate::testkit::id(1)], "distinct tips");
        assert!(x.has_local);
        let y = unit(&units, "y");
        assert_eq!((y.refs.len(), y.has_local), (1, false));
    }

    #[test]
    fn globs_cross_slashes_and_match_remote_names() {
        assert!(glob_match("dependabot/*", "dependabot/npm/a"));
        assert!(glob_match("feat?", "feat1"));
        assert!(!glob_match("Main", "main"), "case-sensitive");
        assert!(glob_match("*", "") && glob_match("a*b*c", "a/xb/yc") && glob_match("*-wip", "ui-wip-wip"));
        assert!(!glob_match("feat?", "feat") && !glob_match("a*b", "a/c") && !glob_match("", "x"));
        assert!(glob_match("caf?/*", "café/menu"), "? is one character, not one byte");

        let units = units_of([local("x", 1), remote("origin", "y", 2), local("release/1.2", 3)]);
        assert!(name_matches("origin/*", unit(&units, "y")));
        assert!(!name_matches("origin/*", unit(&units, "x")), "a local-only branch has no remote name");
        assert!(name_matches(" bot/* , release/*", unit(&units, "release/1.2")));
        assert!(!name_matches("", unit(&units, "x")) && !name_matches(" , ", unit(&units, "x")));
    }

    #[test]
    fn the_trunk_is_the_pin_pair_plus_patterns() {
        let units = units_of([local("main", 1), remote("origin", "main", 1), local("release/1.2", 2), local("x", 3)]);
        let flags = trunk_flags(&units, &["refs/heads/main".into(), "refs/remotes/origin/main".into()], &["release/*".into()]);
        let by_name: Vec<(&str, bool)> = units.iter().map(|u| u.name.as_str()).zip(flags).collect();
        assert_eq!(by_name, [("main", true), ("release/1.2", true), ("x", false)]);
    }

    #[test]
    fn with_the_pin_off_the_trunk_is_still_the_default_pair() {
        let main = crate::RefIn { upstream: Some("refs/remotes/origin/main".into()), ..local("main", 1) };
        let (mut req, _) = request([main, remote("origin", "main", 1), local("x", 2)], [worktree("/demo/x", 1, "main", true)], [commit(1, &[], 100, "ada@example.com", "Initial commit"), commit(2, &[1], 200, "ada@example.com", "X")], 10);
        req.input.refs.remote_hosts.insert("origin".into(), crate::HostKind::Generic);
        req.input.no_pin = true;
        assert_eq!(crate::pins::trunk_names(&req.input, None), ["refs/heads/main", "refs/remotes/origin/main"]);
        req.input.no_pin = false;
        req.input.pinned_ref = Some("refs/heads/x".into());
        assert_eq!(crate::pins::trunk_names(&req.input, None), ["refs/heads/x"], "an explicit pin is the trunk");
    }
}
