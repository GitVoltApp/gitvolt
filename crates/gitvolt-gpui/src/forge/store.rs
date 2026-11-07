//! One tab's forge state (`mrStore.ts`), kept in memory: the poller fills it. Answers go in only when they differ, so a poll
//! that found nothing new draws nothing.

use crate::model::Chip;
use gitvolt_core::forge::types::{ForgeDiscussion, ForgeMr, ForgeMrDetail, ForgeProject, MrFilter, RateLimitState};
use gitvolt_core::forge::ForgeKind;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrList {
    pub kind: ForgeKind,
    pub remote: String,
    pub project: ForgeProject,
    pub filter: MrFilter,
    pub mrs: Vec<ForgeMr>,
    pub fetched_at: i64,
    pub poll_interval_secs: Option<u32>,
    #[serde(default)]
    pub rate_limit: Option<RateLimitState>,
}

impl MrList {
    /// The same list, whatever it says about when it was read.
    pub fn same_as(&self, other: &MrList) -> bool {
        self.kind == other.kind && self.remote == other.remote && self.project == other.project && self.filter == other.filter && self.mrs == other.mrs && self.poll_interval_secs == other.poll_interval_secs
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefMr {
    pub remote_ref: String,
    pub mr: ForgeMr,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchMrs {
    pub kind: ForgeKind,
    pub remote: String,
    pub mrs: Vec<RefMr>,
    #[serde(default)]
    pub history: Vec<RefMr>,
    #[serde(default)]
    pub poll_interval_secs: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedMrs {
    pub kind: ForgeKind,
    pub remote: String,
    pub project: ForgeProject,
    pub list: Option<MrList>,
    pub badges: Option<BranchMrs>,
    pub saved_at: i64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteProject {
    pub remote: String,
    pub host: Option<String>,
    pub path: Option<String>,
    pub account: Option<ForgeKind>,
    /// Which of the host's accounts is used (`ForgeAccount.id`).
    pub account_id: Option<String>,
    pub project: Option<ForgeProject>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoProjects {
    pub remotes: Vec<RemoteProject>,
    pub target: Option<String>,
    pub target_chosen: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Fresh<T> {
    pub value: T,
}

/// What a refresh button is fetching now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Refreshing {
    List,
    Mr(u64),
}

/// A file, and where in it: `note_line`.
pub type NoteAt = (String, Option<(bool, u32, Option<u32>)>);

pub struct TabForge {
    /// The kind of the account behind the repo's target project; `None`: no MR/PR UI for this repo.
    pub kind: Option<ForgeKind>,
    pub remote: Option<String>,
    pub project: Option<ForgeProject>,
    /// The remotes with a project on the target's host.
    pub mapped: Vec<String>,
    pub target: Option<String>,
    pub target_chosen: bool,
    pub remote_errors: HashMap<String, String>,
    pub owner_avatars: HashMap<String, String>,
    pub me: Option<String>,
    /// The account behind the target project (`ForgeAccount.id`): the repository's chosen one on the host, else the host's default.
    pub account_id: Option<String>,
    /// Remote-tracking ref → its MR/PR: the badges.
    pub by_ref: BTreeMap<String, ForgeMr>,
    /// Merged or closed ones that badge nothing: a stack's walk reads them.
    pub history: BTreeMap<String, ForgeMr>,
    /// Local branch ref → its upstream, as the badges were read.
    pub upstreams: BTreeMap<String, String>,
    pub filter: MrFilter,
    pub filter_read: bool,
    pub list: Option<MrList>,
    pub details: HashMap<u64, ForgeMrDetail>,
    pub detail_errors: HashMap<u64, String>,
    pub discussions: HashMap<u64, Vec<ForgeDiscussion>>,
    /// When a detail was last read or failed, ms: a hover card asks again once it's old.
    pub fresh_at: HashMap<u64, i64>,
    pub loading: Vec<u64>,
    /// GitVolt's forge writes answered so far: a read that began before one may predate it.
    pub write_epoch: u64,
    pub activated_at: Option<i64>,
    /// The next poll asks the forges again (an account was added or removed).
    pub recheck: bool,
    /// The MR/PR the tab's view shows.
    pub open_mr: Option<u64>,
    pub updated_at: Option<i64>,
    pub error: Option<String>,
    pub limited_until: Option<i64>,
    pub cached_at: Option<i64>,
    pub failures: u32,
    pub refreshing: Vec<Refreshing>,
    pub review: Option<super::review::ReviewSession>,
    /// The refs a review's diff was last read for.
    pub review_refs: Option<String>,
    /// A head Compare fetched into a ref of its own: the MR/PR, the head, and whether its compare has shown.
    /// Prompted this session, and running now: merged MRs/PRs' numbers.
    pub stack_prompted: std::collections::HashSet<u64>,
    pub stack_running: std::collections::HashSet<u64>,
    /// The compare last looked at, for `forge_watch_compare`.
    pub compare_seen: Option<(String, Option<String>)>,
    pub compare_ref: Option<(u64, String, bool)>,
    /// The project's merge settings by remote; a failure is asked again after the next poll.
    pub settings: HashMap<String, gitvolt_core::forge::types::ForgeProjectSettings>,
    pub settings_asked: HashMap<String, Option<i64>>,
    pub settings_error: Option<String>,
    /// Behind its target, as the commits say, by target tip and head: `None`: can't tell.
    pub behind: HashMap<String, Option<bool>>,
    /// How many reviewers and assignees the target's MRs may have.
    pub limits: Option<gitvolt_core::forge::types::PeopleLimits>,
    pub limits_asked: bool,
    pub people_queue: std::collections::VecDeque<super::mrview::actions::PeopleWrite>,
    pub people_busy: bool,
    /// A note's `file:line` asked to open once the compare's files are read.
    pub note_after_compare: Option<NoteAt>,
    pub review_ui: super::review::zones::ReviewUi,
    pub lent_merge: Option<u64>,
    pub lent_update: Option<u64>,
}

impl Default for TabForge {
    fn default() -> Self {
        Self { stack_prompted: Default::default(), stack_running: Default::default(), compare_seen: None, account_id: None, kind: None, remote: None, project: None, mapped: Vec::new(), target: None, target_chosen: false, remote_errors: HashMap::new(), owner_avatars: HashMap::new(), me: None, by_ref: BTreeMap::new(), history: BTreeMap::new(), upstreams: BTreeMap::new(), filter: MrFilter::All, filter_read: false, list: None, details: HashMap::new(), detail_errors: HashMap::new(), discussions: HashMap::new(), fresh_at: HashMap::new(), loading: Vec::new(), write_epoch: 0, activated_at: None, recheck: false, open_mr: None, updated_at: None, error: None, limited_until: None, cached_at: None, failures: 0, refreshing: Vec::new(), review: None, review_refs: None, compare_ref: None, settings: HashMap::new(), settings_asked: HashMap::new(), settings_error: None, behind: HashMap::new(), limits: None, limits_asked: false, people_queue: Default::default(), people_busy: false, note_after_compare: None, review_ui: Default::default(), lent_merge: None, lent_update: None }
    }
}

/// `v`, set if it differs; whether it did.
pub fn put<T: PartialEq>(at: &mut T, v: T) -> bool {
    if *at == v {
        false
    } else {
        *at = v;
        true
    }
}

impl TabForge {
    /// A chip's or a branch's MR/PR: through one of its remote refs (`via` that ref), else through its local branch's
    /// upstream (`via` "local"). The badge takes the place of that ref's icon.
    pub fn mr_for_chip(&self, c: &Chip) -> Option<(&ForgeMr, String)> {
        for (_, full) in &c.remotes {
            if let Some(mr) = self.by_ref.get(full) {
                return Some((mr, full.clone()));
            }
        }
        if c.local {
            let local = format!("refs/heads/{}", c.name);
            if let Some(mr) = self.upstreams.get(&local).and_then(|u| self.by_ref.get(u)) {
                return Some((mr, "local".to_string()));
            }
        }
        None
    }


    /// An MR/PR by number from what's loaded: its detail, the list, a badge.
    pub fn known_mr(&self, n: u64) -> Option<&ForgeMr> {
        self.details.get(&n).map(|d| &d.mr).or_else(|| self.list.as_ref()?.mrs.iter().find(|m| m.number == n)).or_else(|| self.by_ref.values().find(|m| m.number == n))
    }
}

/// `upstreamRefsOf`: the local branches' upstreams, newest tip first; first of all, the target branches of open MRs/PRs from
/// local branches that have no local branch (a stack's merged bottom whose branch was deleted after the merge).
pub fn upstream_refs_of(sidebar: &Value, f: Option<&TabForge>) -> (Vec<String>, BTreeMap<String, String>) {
    let locals = sidebar["locals"].as_array().map_or(&[][..], Vec::as_slice);
    let asked = |l: &Value| l["upstream"].as_str().map(str::to_string).or_else(|| l["pushTarget"].as_str().map(|p| format!("refs/remotes/{p}")));
    let mut with_ref: Vec<(&Value, String)> = locals.iter().filter_map(|l| Some((l, asked(l)?))).collect();
    with_ref.sort_by_key(|(l, _)| std::cmp::Reverse(l["tipTime"].as_i64().unwrap_or(0)));
    let mut refs: Vec<String> = Vec::new();
    if let Some(f) = f
        && let Some(remote) = &f.remote
    {
        for l in locals {
            let name = l["name"].as_str().unwrap_or_default();
            let full = l["fullName"].as_str().unwrap_or_default();
            let mr = f.upstreams.get(full).and_then(|u| f.by_ref.get(u)).or_else(|| f.by_ref.get(&format!("refs/remotes/{remote}/{name}")));
            let Some(mr) = mr.filter(|m| matches!(m.state, gitvolt_core::forge::types::MrState::Open | gitvolt_core::forge::types::MrState::Draft)) else { continue };
            let t = &mr.target_branch;
            let has_local = locals.iter().any(|x| x["name"].as_str() == Some(t));
            if !has_local && f.project.as_ref().and_then(|p| p.default_branch.as_ref()) != Some(t) {
                refs.push(format!("refs/remotes/{remote}/{t}"));
            }
        }
    }
    for (_, r) in &with_ref {
        if !refs.contains(r) {
            refs.push(r.clone());
        }
    }
    let upstreams = with_ref.iter().map(|(l, r)| (l["fullName"].as_str().unwrap_or_default().to_string(), r.clone())).collect();
    (refs, upstreams)
}

/// `staleText`: why what's shown may be old; `None` when the last poll worked.
pub fn stale_text(f: &TabForge, now_ms: i64) -> Option<String> {
    let ago = |ms: i64| crate::format::relative_time(ms / 1000, now_ms as f64 / 1000.);
    let updated = f.updated_at.map_or(String::new(), |u| format!(" Last updated {}", ago(u)));
    if let Some(until) = f.limited_until.filter(|u| *u > now_ms) {
        use chrono::TimeZone;
        let hhmm = chrono::Local.timestamp_millis_opt(until).single().map_or(String::new(), |t| t.format("%H:%M").to_string());
        return Some(format!("Rate limited until {hhmm}.{updated}"));
    }
    if let Some(e) = &f.error {
        return Some(format!("Couldn't refresh: {e}.{updated}"));
    }
    f.cached_at.map(|c| format!("Updated {}", ago(c)))
}

/// A section's words when it has no rows.
pub fn empty_text(kind: ForgeKind, f: &TabForge) -> String {
    if f.list.is_none() {
        return f.error.as_ref().map_or("Loading…".to_string(), |e| format!("Couldn't load: {e}"));
    }
    match f.filter {
        MrFilter::Mine => "None of yours are open".to_string(),
        MrFilter::ReviewRequested => "No reviews requested from you".to_string(),
        MrFilter::All => format!("No open {}", super::text::mr_section_label(kind).to_lowercase()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::prelude::v1::test;

    #[test]
    fn the_badges_ask_for_upstreams_newest_first_and_a_pushed_branch_too() {
        let sidebar = json!({ "locals": [
            { "name": "old", "fullName": "refs/heads/old", "upstream": "refs/remotes/origin/old", "tipTime": 1 },
            { "name": "new", "fullName": "refs/heads/new", "upstream": "refs/remotes/origin/new", "tipTime": 9 },
            { "name": "pushed", "fullName": "refs/heads/pushed", "pushTarget": "origin/pushed", "tipTime": 5 },
            { "name": "alone", "fullName": "refs/heads/alone", "tipTime": 7 },
        ] });
        let (refs, up) = upstream_refs_of(&sidebar, None);
        assert_eq!(refs, ["refs/remotes/origin/new", "refs/remotes/origin/pushed", "refs/remotes/origin/old"]);
        assert_eq!(up["refs/heads/pushed"], "refs/remotes/origin/pushed");
        assert!(!up.contains_key("refs/heads/alone"));
    }

    #[test]
    fn a_stale_note_says_why_or_nothing() {
        let mut f = TabForge::default();
        assert_eq!(stale_text(&f, 1000), None);
        f.error = Some("down".into());
        f.updated_at = Some(0);
        assert!(stale_text(&f, 120_000).unwrap().starts_with("Couldn't refresh: down. Last updated"));
        f.error = None;
        f.cached_at = Some(0);
        assert!(stale_text(&f, 120_000).unwrap().starts_with("Updated "));
    }
}
