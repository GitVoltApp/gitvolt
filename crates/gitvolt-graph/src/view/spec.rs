//! What a graph request carries to apply a preset (`GraphInput::view`): the resolved preset,
//! this repository's picks and trunk patterns, and the session's expansions.

use crate::presets::{BranchPicks, CollapsePlacement, PresetAction, Rule};
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
    /// Session expansions: "branch:<unit name>" or "fold:<merge sha>".
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
