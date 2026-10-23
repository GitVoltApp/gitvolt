//! What a graph request carries to apply a preset (`GraphInput::view`): the resolved preset,
//! this repository's picks and trunk patterns, and the session's expansions.

use crate::presets::{BranchPicks, CollapsePlacement, PresetAction, Rule};
use crate::Oid;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ViewSpec {
    pub rules: Vec<Rule>,
    pub otherwise: PresetAction,
    #[serde(default)]
    pub picks: BranchPicks,
    pub fold_merged: bool,
    pub ghost_lines: bool,
    /// The repository's extra trunk branches (globs), beside the pinned pair.
    #[serde(default)]
    pub trunk_patterns: Vec<String>,
    /// Session expansions: "branch:<unit name>" (all of its runs) or "fold:<merge sha>".
    #[serde(default)]
    pub expanded: Vec<String>,
    #[serde(default)]
    pub placement: CollapsePlacement,
    /// Filled by the core (and the demo), never by the UI: who "me" is, lower-cased emails.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub me: Vec<String>,
    /// Filled by the core (and the demo), never by the UI: unix seconds, for "older than N days".
    #[serde(default, skip_serializing_if = "is_zero")]
    #[ts(type = "number")]
    pub now: i64,
}

/// A collapsed run's key, its reveal panel's: `"branch:<unit name>@<newest commit's sha>"` (a unit
/// can have several runs, one per line of history). Its unit's `expanded` key is `"branch:<name>"`.
pub(crate) fn run_key(unit: &str, newest: &Oid) -> String {
    format!("branch:{unit}@{newest}")
}

/// A `"branch:…"` key's unit name and, for a run's key, its newest commit. A name may hold `@`;
/// only a full sha after the last one makes it a run's key.
pub(crate) fn branch_key(key: &str) -> Option<(&str, Option<Oid>)> {
    let rest = key.strip_prefix("branch:")?;
    Some(match rest.rsplit_once('@').and_then(|(name, sha)| Some((name, Oid::from_hex(sha)?))) {
        Some((name, newest)) => (name, Some(newest)),
        None => (rest, None),
    })
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::Matcher;

    #[test]
    fn a_view_spec_reads_from_the_uis_json() {
        let json = r#"{"rules":[{"action":"hide","matcher":{"kind":"nameMatches","patterns":"bot/*"}}],"otherwise":"show",
                       "foldMerged":true,"ghostLines":true}"#;
        let v: ViewSpec = serde_json::from_str(json).unwrap();
        assert_eq!(v.rules, [Rule { action: PresetAction::Hide, matcher: Matcher::NameMatches { patterns: "bot/*".into() } }]);
        assert_eq!((v.otherwise, v.fold_merged, v.ghost_lines), (PresetAction::Show, true, true));
        assert_eq!(v.picks, BranchPicks::default());
        assert!(v.expanded.is_empty() && v.me.is_empty() && v.trunk_patterns.is_empty());
        assert_eq!((v.placement, v.now), (CollapsePlacement::KeepDateOrder, 0));
    }

    #[test]
    fn a_run_key_names_its_unit_and_newest_commit() {
        let sha = Oid::from_hex(&"ab".repeat(20)).unwrap();
        let key = run_key("feature/x@2", &sha);
        assert_eq!(key, format!("branch:feature/x@2@{}", "ab".repeat(20)));
        assert_eq!(branch_key(&key), Some(("feature/x@2", Some(sha))));
        assert_eq!(branch_key("branch:feature/x@2"), Some(("feature/x@2", None)), "a unit's key");
        assert_eq!(branch_key("branch:x"), Some(("x", None)));
        assert_eq!(branch_key(&format!("fold:{sha}")), None);
    }

    #[test]
    fn matchers_and_authors_spell_their_kinds_in_camel_case() {
        let rules = vec![
            Rule { action: PresetAction::Collapse, matcher: Matcher::CommitsBy { author: crate::presets::Author::Person { who: "ada@example.com".into() } } },
            Rule { action: PresetAction::Show, matcher: Matcher::OlderThan { days: 30 } },
            Rule { action: PresetAction::Hide, matcher: Matcher::MergedIntoTrunk },
        ];
        let json = serde_json::to_value(&rules).unwrap();
        assert_eq!(
            json,
            serde_json::json!([
                {"action": "collapse", "matcher": {"kind": "commitsBy", "author": {"kind": "person", "who": "ada@example.com"}}},
                {"action": "show", "matcher": {"kind": "olderThan", "days": 30}},
                {"action": "hide", "matcher": {"kind": "mergedIntoTrunk"}},
            ])
        );
        assert_eq!(serde_json::from_value::<Vec<Rule>>(json).unwrap(), rules);
    }
}
