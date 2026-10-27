//! Deciding each unit from a preset's rules, its picks and the session's expansions.

use crate::payload::DecidedBy;
use crate::presets::{name_matches, Author, BranchUnit, Decision, Facts, Matcher, PresetAction, UnitFacts};
use crate::view::{branch_key, ViewSpec};

/// One decision per unit. The trunk shows; then an expansion this session shows; then a pick
/// shows or hides; then the first matching rule decides; then Everything else. A unit checked out
/// in any worktree that a rule or Everything else would hide shows instead (it may still
/// collapse); one hidden by hand hides, unless it's the tab's own worktree's. `rule` is the first
/// rule that matches whatever decided, for the settings page's counts.
pub(crate) fn decide(spec: &ViewSpec, units: &[BranchUnit], facts: &Facts) -> Vec<Decision> {
    units
        .iter()
        .zip(&facts.units)
        .map(|(unit, f)| {
            let rule = spec.rules.iter().position(|r| matches(&r.matcher, unit, f, spec.now)).map(|i| i as u16);
            let (action, by) = if f.trunk {
                (PresetAction::Show, DecidedBy::Trunk)
            } else if spec.expanded.iter().any(|k| branch_key(k).is_some_and(|(name, _)| name == unit.name)) {
                (PresetAction::Show, DecidedBy::Expanded)
            } else if spec.picks.shown.contains(&unit.name) {
                (PresetAction::Show, DecidedBy::Picked)
            } else if spec.picks.hidden.contains(&unit.name) {
                (PresetAction::Hide, DecidedBy::Picked)
            } else if let Some(index) = rule {
                (spec.rules[index as usize].action, DecidedBy::Rule { index })
            } else {
                (spec.otherwise, DecidedBy::Otherwise)
            };
            let kept = f.open || (f.checked_out && by != DecidedBy::Picked);
            let (action, by) = if action == PresetAction::Hide && kept { (PresetAction::Show, DecidedBy::CheckedOut) } else { (action, by) };
            Decision { action, by, rule }
        })
        .collect()
}

fn matches(matcher: &Matcher, unit: &BranchUnit, f: &UnitFacts, now: i64) -> bool {
    match matcher {
        Matcher::Trunk => f.trunk,
        Matcher::CheckedOut => f.checked_out,
        Matcher::CommitsBy { author: Author::Me } => f.mine,
        Matcher::CommitsBy { author: Author::Person { who } } => f.authors.contains(&who.trim().to_lowercase()),
        Matcher::NameMatches { patterns } => name_matches(patterns, unit),
        Matcher::Remote => !f.local,
        Matcher::Local => f.local,
        // A tip outside the window is older than any N days.
        Matcher::OlderThan { days } => f.tip_time.is_none_or(|t| now - t > i64::from(*days) * 86_400),
        Matcher::MergedIntoTrunk => f.merged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::facts::tests::facts_for;
    use crate::presets::{BranchPicks, Rule};
    use crate::testkit::{commit, local, remote, worktree};
    use crate::walk::CommitMeta;
    use PresetAction::{Collapse, Hide, Show};

    fn rule(action: PresetAction, matcher: Matcher) -> Rule {
        Rule { action, matcher }
    }

    fn spec(rules: Vec<Rule>, otherwise: PresetAction) -> ViewSpec {
        ViewSpec { rules, otherwise, ..Default::default() }
    }

    fn named(patterns: &str) -> Matcher {
        Matcher::NameMatches { patterns: patterns.into() }
    }

    /// main: 1 ← 2; x: 3 by ada, from 1; y: 4 by cy, from 2, remote only; z: 5 by ada, from 2,
    /// checked out in a linked worktree.
    fn window() -> (Vec<BranchUnit>, Facts) {
        let commits = [
            commit(5, &[2], 500, "ada@example.com", "Z"),
            commit(4, &[2], 400, "cy@example.com", "Y"),
            commit(3, &[1], 300, "ada@example.com", "X"),
            commit(2, &[1], 200, "bo@example.com", "Main"),
            commit(1, &[], 100, "bo@example.com", "Initial commit"),
        ];
        let worktrees = [worktree("/demo/x", 2, "main", true), worktree("/demo/x-z", 5, "z", false)];
        facts_for(vec![local("main", 2), local("x", 3), remote("origin", "y", 4), local("z", 5)], &worktrees, &commits, &["refs/heads/main"], &[])
    }

    /// (unit name, action, by, rule) per unit, by name.
    fn decided(spec: &ViewSpec, units: &[BranchUnit], facts: &Facts) -> Vec<(String, PresetAction, DecidedBy, Option<u16>)> {
        units.iter().zip(decide(spec, units, facts)).map(|(u, d)| (u.name.clone(), d.action, d.by, d.rule)).collect()
    }

    fn of(spec: &ViewSpec, units: &[BranchUnit], facts: &Facts, name: &str) -> Decision {
        decide(spec, units, facts)[units.iter().position(|u| u.name == name).unwrap()]
    }

    #[test]
    fn first_matching_rule_wins_and_everything_else_has_the_last_word() {
        let (units, facts) = window();
        let s = spec(vec![rule(Collapse, named("x, y")), rule(Hide, named("x")), rule(Show, Matcher::Remote)], Hide);
        let d = decided(&s, &units, &facts);
        assert_eq!(
            d,
            [
                ("main".into(), Show, DecidedBy::Trunk, None),
                ("x".into(), Collapse, DecidedBy::Rule { index: 0 }, Some(0)),
                ("y".into(), Collapse, DecidedBy::Rule { index: 0 }, Some(0)),
                // Hidden by Everything else, but checked out.
                ("z".into(), Show, DecidedBy::CheckedOut, None),
            ]
        );
        let s = spec(vec![rule(Collapse, Matcher::Local)], Hide);
        assert_eq!(of(&s, &units, &facts, "y"), Decision { action: Hide, by: DecidedBy::Otherwise, rule: None }, "no rule matches a remote-only branch");
    }

    #[test]
    fn the_trunk_is_never_hidden_or_collapsed() {
        let (units, facts) = window();
        for action in [Hide, Collapse] {
            let s = spec(vec![rule(action, Matcher::Trunk)], action);
            assert_eq!(of(&s, &units, &facts, "main"), Decision { action: Show, by: DecidedBy::Trunk, rule: Some(0) });
        }
        let s = ViewSpec { picks: BranchPicks { shown: vec![], hidden: vec!["main".into()] }, ..spec(vec![], Collapse) };
        assert_eq!(of(&s, &units, &facts, "main").by, DecidedBy::Trunk, "nor hidden by a pick");
    }

    #[test]
    fn a_checked_out_branch_hidden_by_a_rule_shows_anyway() {
        let (units, facts) = window();
        let s = spec(vec![rule(Hide, named("z"))], Show);
        assert_eq!(of(&s, &units, &facts, "z"), Decision { action: Show, by: DecidedBy::CheckedOut, rule: Some(0) });
        assert_eq!(of(&spec(vec![], Hide), &units, &facts, "z"), Decision { action: Show, by: DecidedBy::CheckedOut, rule: None }, "or by Everything else");
    }

    #[test]
    fn a_branch_hidden_by_hand_hides_unless_the_tab_has_it_checked_out() {
        // z is checked out in another worktree than the tab's.
        let (units, facts) = window();
        let s = ViewSpec { picks: BranchPicks { shown: vec![], hidden: vec!["z".into()] }, ..spec(vec![rule(Show, Matcher::CheckedOut)], Show) };
        assert_eq!(of(&s, &units, &facts, "z"), Decision { action: Hide, by: DecidedBy::Picked, rule: Some(0) });
        // The tab shows x's worktree: its branch stays.
        let commits = [commit(3, &[1], 300, "ada@example.com", "X"), commit(1, &[], 100, "bo@example.com", "Initial commit")];
        let worktrees = [worktree("/demo/x", 3, "x", true), worktree("/demo/x-main", 1, "main", false)];
        let (units, facts) = facts_for(vec![local("main", 1), local("x", 3)], &worktrees, &commits, &["refs/heads/main"], &[]);
        let s = ViewSpec { picks: BranchPicks { shown: vec![], hidden: vec!["x".into()] }, ..spec(vec![], Show) };
        assert_eq!(of(&s, &units, &facts, "x"), Decision { action: Show, by: DecidedBy::CheckedOut, rule: None });
    }

    #[test]
    fn a_checked_out_branch_may_collapse() {
        let (units, facts) = window();
        let s = spec(vec![rule(Collapse, Matcher::CheckedOut)], Hide);
        assert_eq!(of(&s, &units, &facts, "z"), Decision { action: Collapse, by: DecidedBy::Rule { index: 0 }, rule: Some(0) });
        assert_eq!(of(&spec(vec![], Collapse), &units, &facts, "z").action, Collapse);
    }

    #[test]
    fn picks_apply_before_rules() {
        let (units, facts) = window();
        let s = ViewSpec { picks: BranchPicks { shown: vec!["x".into()], hidden: vec!["y".into()] }, ..spec(vec![rule(Hide, named("x")), rule(Show, named("y"))], Collapse) };
        assert_eq!(of(&s, &units, &facts, "x"), Decision { action: Show, by: DecidedBy::Picked, rule: Some(0) });
        assert_eq!(of(&s, &units, &facts, "y"), Decision { action: Hide, by: DecidedBy::Picked, rule: Some(1) });
        // An expansion this session beats a pick.
        let s = ViewSpec { expanded: vec!["branch:y".into(), "fold:abc".into()], ..s };
        assert_eq!(of(&s, &units, &facts, "y"), Decision { action: Show, by: DecidedBy::Expanded, rule: Some(1) });
    }

    #[test]
    fn unknown_picked_names_are_ignored() {
        let (units, facts) = window();
        let s = ViewSpec { picks: BranchPicks { shown: vec!["gone".into(), "origin/y".into()], hidden: vec!["also-gone".into()] }, expanded: vec!["branch:gone".into()], ..spec(vec![], Collapse) };
        let d = decided(&s, &units, &facts);
        assert_eq!(d.len(), units.len());
        assert_eq!(d[1..].iter().map(|(_, a, by, _)| (*a, *by)).collect::<Vec<_>>(), [(Collapse, DecidedBy::Otherwise); 3], "picks name units, not refs");
    }

    #[test]
    fn older_than_counts_days_from_now() {
        let day = 86_400;
        let commits: Vec<CommitMeta> = vec![commit(3, &[1], 10 * day, "ada@example.com", "New"), commit(2, &[1], 3 * day, "ada@example.com", "Old"), commit(1, &[], day, "bo@example.com", "Initial commit")];
        let (units, facts) = facts_for(vec![local("main", 1), local("new", 3), local("old", 2), local("unloaded", 9)], &[], &commits, &["refs/heads/main"], &[]);
        let s = ViewSpec { now: 12 * day, ..spec(vec![rule(Hide, Matcher::OlderThan { days: 7 })], Show) };
        assert_eq!(of(&s, &units, &facts, "new").action, Show, "2 days old");
        assert_eq!(of(&s, &units, &facts, "old").action, Hide, "9 days old");
        assert_eq!(of(&s, &units, &facts, "unloaded").action, Hide, "its tip isn't loaded");
        let s = ViewSpec { now: 17 * day, ..s };
        assert_eq!(of(&s, &units, &facts, "new").action, Show, "exactly 7 days isn't older than 7");
        let s = ViewSpec { now: 17 * day + 1, ..s };
        assert_eq!(of(&s, &units, &facts, "new").action, Hide);
    }

    #[test]
    fn commits_by_a_person_matches_their_email_or_name_in_any_case() {
        let (units, facts) = window();
        for who in ["ADA@example.com", "Ada", " ada "] {
            let s = spec(vec![rule(Show, Matcher::CommitsBy { author: Author::Person { who: who.into() } })], Hide);
            assert_eq!(of(&s, &units, &facts, "x").action, Show, "{who}");
            assert_eq!(of(&s, &units, &facts, "y").action, Hide, "{who}");
        }
        let s = spec(vec![rule(Show, Matcher::CommitsBy { author: Author::Person { who: "bo".into() } })], Hide);
        assert_eq!(of(&s, &units, &facts, "x").action, Hide, "bo's commits are the trunk's");
    }

    #[test]
    fn the_built_in_presets_decide_as_specified() {
        // Shaped like the `presets` fixture: main 1 ← 2 ← 3 ← 6 (merging feature/docs 4 ← 5, kept);
        // feature/payments 7 ← 8 ← 9 from 6; dependabot 10 from 6, remote only; spike 11 ← 12
        // from 3; mine/dock-panel 13 ← 14 from 6, checked out in wt-dock; mine/fuel-gauge 15 from
        // 6; release/1.2 16 from 3.
        let c = |n: u8, parents: &[u8], email: &str| commit(n, parents, i64::from(n) * 100, email, "A commit");
        let mut commits = vec![
            c(1, &[], "kasimir@example.com"),
            c(2, &[1], "kasimir@example.com"),
            c(3, &[2], "wen@example.com"),
            c(4, &[3], "wen@example.com"),
            c(5, &[4], "wen@example.com"),
            c(6, &[3, 5], "wen@example.com"),
            c(7, &[6], "teodor@example.com"),
            c(8, &[7], "noor@example.com"),
            c(9, &[8], "teodor@example.com"),
            c(10, &[6], "bot@example.com"),
            c(11, &[3], "rafe@example.com"),
            c(12, &[11], "rafe@example.com"),
            c(13, &[6], "you@example.com"),
            c(14, &[13], "you@example.com"),
            c(15, &[6], "you@example.com"),
            c(16, &[3], "wen@example.com"),
        ];
        commits.reverse();
        let refs = vec![
            local("main", 6),
            remote("origin", "main", 6),
            local("feature/docs", 5),
            remote("origin", "feature/docs", 5),
            local("feature/payments", 9),
            remote("origin", "feature/payments", 9),
            remote("origin", "dependabot/npm/lodash-4.17.21", 10),
            local("spike/warp-gates", 12),
            local("mine/dock-panel", 14),
            local("mine/fuel-gauge", 15),
            local("release/1.2", 16),
            remote("origin", "release/1.2", 16),
        ];
        let worktrees = [worktree("/demo/presets", 6, "main", true), worktree("/demo/wt-dock", 14, "mine/dock-panel", false)];
        let (units, facts) = facts_for(refs, &worktrees, &commits, &["refs/heads/main", "refs/remotes/origin/main"], &["you@example.com"]);

        let mine = spec(vec![rule(Show, Matcher::Trunk), rule(Show, Matcher::CheckedOut), rule(Show, Matcher::CommitsBy { author: Author::Me })], Hide);
        let shown: Vec<String> = decided(&mine, &units, &facts).into_iter().filter(|d| d.1 == Show).map(|d| format!("{} {:?}", d.0, d.2)).collect();
        assert_eq!(shown, ["main Trunk", "mine/dock-panel Rule { index: 1 }", "mine/fuel-gauge Rule { index: 2 }"]);
        assert!(decided(&mine, &units, &facts).iter().all(|d| d.1 == Show || d.2 == DecidedBy::Otherwise));

        let all_collapsed = spec(vec![rule(Show, Matcher::Trunk), rule(Show, Matcher::CheckedOut)], Collapse);
        let d = decided(&all_collapsed, &units, &facts);
        let shown: Vec<&str> = d.iter().filter(|d| d.1 == Show).map(|d| d.0.as_str()).collect();
        assert_eq!(shown, ["main", "mine/dock-panel"]);
        assert_eq!(d.iter().filter(|d| d.1 == Collapse && d.2 == DecidedBy::Otherwise).count(), units.len() - 2);
        assert!(of(&all_collapsed, &units, &facts, "feature/docs").action == Collapse && facts.units[units.iter().position(|u| u.name == "feature/docs").unwrap()].merged);
    }
}
