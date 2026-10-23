//! What rules match a unit on, from the loaded window, and which units reach each commit.

use crate::input::WorktreeIn;
use crate::presets::BranchUnit;
use crate::walk::CommitMeta;
use crate::Oid;
use std::collections::{HashMap, HashSet};

/// One unit's facts, within the loaded commits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UnitFacts {
    pub trunk: bool,
    pub checked_out: bool,
    pub local: bool,
    /// The newest committer time among its tips in the window; `None` when none is loaded.
    pub tip_time: Option<i64>,
    pub merged: bool,
    /// Lower-cased author emails and names of the unit's own commits (reachable from it, not from the trunk).
    pub authors: HashSet<String>,
    pub mine: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Facts {
    pub units: Vec<UnitFacts>,
    pub reach: ReachSets,
}

/// Per commit (walk index): the units that reach it, as a bitset over unit indices (empty for
/// trunk-reachable commits), and whether a trunk branch reaches it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ReachSets {
    bits: Vec<Vec<u64>>,
    trunk: Vec<bool>,
}

impl ReachSets {
    /// `bits`: one word vector per commit (bit `u % 64` of word `u / 64` for unit `u`), empty for
    /// trunk-reachable commits; `trunk`: per commit, whether a trunk branch reaches it.
    pub(crate) fn new(bits: Vec<Vec<u64>>, trunk: Vec<bool>) -> Self {
        ReachSets { bits, trunk }
    }

    /// The units reaching `commit`, ascending.
    pub(crate) fn units_reaching(&self, commit: usize) -> impl Iterator<Item = usize> + '_ {
        self.bits.get(commit).into_iter().flatten().enumerate().flat_map(|(w, &word)| (0..64).filter(move |b| word >> b & 1 == 1).map(move |b| w * 64 + b))
    }

    pub(crate) fn trunk_reachable(&self, commit: usize) -> bool {
        self.trunk.get(commit).copied().unwrap_or(false)
    }
}

/// Every unit's facts and the reach sets, in one pass over `commits` in walk order (children
/// before parents; `index` maps an id to its position). A commit is trunk-reachable when it is a
/// trunk unit's tip or a parent of a trunk-reachable commit, and then no unit "reaches" it; any
/// other commit is reached by the units whose tip it is and every unit reaching one of its
/// children. Tips and parents outside the window count for nothing. `me`: who "me" is (emails or
/// names, any case). How old a tip is is `decide`'s to judge, against the spec's `now`.
pub(crate) fn unit_facts(units: &[BranchUnit], trunk: &[bool], commits: &[CommitMeta], index: &HashMap<Oid, usize>, worktrees: &[WorktreeIn], me: &[String], _now: i64) -> Facts {
    let words = units.len().div_ceil(64);
    let mut trunk_reach = vec![false; commits.len()];
    let mut bits: Vec<Vec<u64>> = vec![Vec::new(); commits.len()];
    for (u, unit) in units.iter().enumerate() {
        for &c in unit.tips.iter().filter_map(|t| index.get(t)) {
            if trunk[u] {
                trunk_reach[c] = true;
            } else {
                let w = &mut bits[c];
                w.resize(words, 0);
                w[u / 64] |= 1 << (u % 64);
            }
        }
    }
    for (c, commit) in commits.iter().enumerate() {
        let parents = commit.parents.iter().filter_map(|p| index.get(p).copied());
        if trunk_reach[c] {
            bits[c] = Vec::new();
            parents.for_each(|p| trunk_reach[p] = true);
            continue;
        }
        let own = std::mem::take(&mut bits[c]);
        if !own.is_empty() {
            for p in parents {
                let w = &mut bits[p];
                w.resize(words, 0);
                w.iter_mut().zip(&own).for_each(|(w, o)| *w |= o);
            }
        }
        bits[c] = own;
    }
    let reach = ReachSets::new(bits, trunk_reach);

    let mut authors: Vec<HashSet<String>> = vec![HashSet::new(); units.len()];
    for (c, commit) in commits.iter().enumerate() {
        let mut reaching = reach.units_reaching(c).peekable();
        if reaching.peek().is_none() {
            continue;
        }
        let (email, name) = (commit.author_email.to_lowercase(), commit.author_name.to_lowercase());
        for u in reaching {
            authors[u].insert(email.clone());
            authors[u].insert(name.clone());
        }
    }

    let me: Vec<String> = me.iter().map(|m| m.to_lowercase()).collect();
    let units = units
        .iter()
        .zip(trunk)
        .zip(authors)
        .map(|((u, &trunk), authors)| {
            let loaded = || u.tips.iter().filter_map(|t| index.get(t).copied());
            UnitFacts {
                trunk,
                checked_out: worktrees.iter().any(|w| w.branch.as_deref().and_then(|b| b.strip_prefix("refs/heads/")) == Some(u.name.as_str())),
                local: u.has_local,
                tip_time: loaded().map(|c| commits[c].committer_time).max(),
                merged: loaded().any(|c| reach.trunk_reachable(c)),
                mine: me.iter().any(|m| authors.contains(m)),
                authors,
            }
        })
        .collect();
    Facts { units, reach }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::input::RefIn;
    use crate::presets::{branch_units, trunk_flags};
    use crate::testkit::{commit, local, worktree};

    /// The units of `refs` and their facts over `commits` (newest first, children before
    /// parents), with `pinned` the trunk's refs.
    pub(crate) fn facts_for(refs: Vec<RefIn>, worktrees: &[WorktreeIn], commits: &[CommitMeta], pinned: &[&str], me: &[&str]) -> (Vec<BranchUnit>, Facts) {
        let units = branch_units(&crate::input::RefsIn { refs, ..Default::default() });
        let trunk = trunk_flags(&units, &pinned.iter().map(|p| p.to_string()).collect::<Vec<_>>(), &[]);
        let index = commits.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
        let me: Vec<String> = me.iter().map(|m| m.to_string()).collect();
        let facts = unit_facts(&units, &trunk, commits, &index, worktrees, &me, 0);
        (units, facts)
    }

    pub(crate) fn of<'a>(units: &[BranchUnit], facts: &'a Facts, name: &str) -> &'a UnitFacts {
        &facts.units[units.iter().position(|u| u.name == name).unwrap()]
    }

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn own_commits_stop_at_the_trunk() {
        // main: 1 ← 2; x from 1: 3 ← 4.
        let commits = [commit(4, &[3], 400, "ada@example.com", "X two"), commit(3, &[1], 300, "ada@example.com", "X one"), commit(2, &[1], 200, "bo@example.com", "Main"), commit(1, &[], 100, "bo@example.com", "Initial commit")];
        let (units, facts) = facts_for(vec![local("main", 2), local("x", 4)], &[], &commits, &["refs/heads/main"], &[]);
        let x = of(&units, &facts, "x");
        assert_eq!(x.authors, set(&["ada@example.com", "ada"]));
        assert_eq!((x.trunk, x.merged, x.local, x.tip_time), (false, false, true, Some(400)));
        assert!(of(&units, &facts, "main").trunk);
        let x_unit = units.iter().position(|u| u.name == "x").unwrap();
        assert_eq!(facts.reach.units_reaching(1).collect::<Vec<_>>(), [x_unit], "4 and 3 are x's");
        assert!(facts.reach.trunk_reachable(3) && facts.reach.units_reaching(3).count() == 0, "1 is the trunk's");
    }

    #[test]
    fn a_merged_branch_is_merged_and_keeps_its_authors_out() {
        // main: 1 ← 2 ← 5 (merging x: 3 ← 4, from 1).
        let commits = [commit(5, &[2, 4], 500, "bo@example.com", "Merge branch 'x'"), commit(4, &[3], 400, "ada@example.com", "X two"), commit(3, &[1], 300, "ada@example.com", "X one"), commit(2, &[1], 200, "bo@example.com", "Main"), commit(1, &[], 100, "bo@example.com", "Initial commit")];
        let (units, facts) = facts_for(vec![local("main", 5), local("x", 4)], &[], &commits, &["refs/heads/main"], &[]);
        let x = of(&units, &facts, "x");
        assert!(x.merged);
        assert!(x.authors.is_empty(), "all its commits are the trunk's");
        assert!((0..commits.len()).all(|c| facts.reach.trunk_reachable(c) && facts.reach.units_reaching(c).count() == 0));
    }

    #[test]
    fn a_stacked_branch_counts_the_commits_below_it() {
        // main: 1; x: 2 by ada; y on x: 3 by cy.
        let commits = [commit(3, &[2], 300, "cy@example.com", "Y"), commit(2, &[1], 200, "ada@example.com", "X"), commit(1, &[], 100, "bo@example.com", "Initial commit")];
        let (units, facts) = facts_for(vec![local("main", 1), local("x", 2), local("y", 3)], &[], &commits, &["refs/heads/main"], &[]);
        assert_eq!(of(&units, &facts, "y").authors, set(&["cy@example.com", "cy", "ada@example.com", "ada"]));
        assert_eq!(of(&units, &facts, "x").authors, set(&["ada@example.com", "ada"]));
        assert_eq!(facts.reach.units_reaching(1).count(), 2, "x and y both reach 2");
    }

    #[test]
    fn mine_matches_email_or_name_case_insensitively() {
        let commits = [commit(3, &[1], 300, "Robin@Example.com", "Mine"), commit(2, &[1], 200, "ada@example.com", "Theirs"), commit(1, &[], 100, "bo@example.com", "Initial commit")];
        let refs = || vec![local("main", 1), local("mine", 3), local("theirs", 2)];
        let (units, facts) = facts_for(refs(), &[], &commits, &["refs/heads/main"], &["ROBIN@example.com"]);
        assert!(of(&units, &facts, "mine").mine && !of(&units, &facts, "theirs").mine);
        // The test commit's name is the part of its email before the `@`.
        let (units, facts) = facts_for(refs(), &[], &commits, &["refs/heads/main"], &["ADA"]);
        assert!(of(&units, &facts, "theirs").mine && !of(&units, &facts, "mine").mine);
        let (_, facts) = facts_for(refs(), &[], &commits, &["refs/heads/main"], &[]);
        assert!(facts.units.iter().all(|f| !f.mine), "no one is me");
    }

    #[test]
    fn a_tip_outside_the_window_is_old_has_no_own_commits_and_is_not_merged() {
        // gone's tip (9) isn't loaded; x's is, its base (1) isn't.
        let commits = [commit(3, &[2], 300, "ada@example.com", "X"), commit(2, &[1], 200, "bo@example.com", "Main")];
        let (units, facts) = facts_for(vec![local("main", 2), local("gone", 9), local("x", 3)], &[], &commits, &["refs/heads/main"], &[]);
        let gone = of(&units, &facts, "gone");
        assert_eq!((gone.tip_time, gone.merged), (None, false));
        assert!(gone.authors.is_empty());
        assert_eq!(of(&units, &facts, "x").tip_time, Some(300));
        assert_eq!(of(&units, &facts, "x").authors, set(&["ada@example.com", "ada"]));
        // A trunk whose tip isn't loaded reaches nothing: the window's commits are the branches'.
        let (units, facts) = facts_for(vec![local("main", 9), local("x", 3)], &[], &commits, &["refs/heads/main"], &[]);
        let x = of(&units, &facts, "x");
        assert_eq!((x.merged, x.authors.len()), (false, 4));
    }

    #[test]
    fn checked_out_in_any_worktree() {
        let commits = [commit(3, &[1], 300, "ada@example.com", "X"), commit(2, &[1], 200, "ada@example.com", "Y"), commit(1, &[], 100, "bo@example.com", "Initial commit")];
        let worktrees = [worktree("/demo/x", 1, "main", true), worktree("/demo/x-wt", 3, "x", false)];
        let (units, facts) = facts_for(vec![local("main", 1), local("x", 3), local("y", 2)], &worktrees, &commits, &["refs/heads/main"], &[]);
        assert!(of(&units, &facts, "x").checked_out && of(&units, &facts, "main").checked_out);
        assert!(!of(&units, &facts, "y").checked_out);
    }
}
