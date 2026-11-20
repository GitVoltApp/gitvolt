//! The list's pipelines by commit, per account: a list poll asks the forge for pipelines only
//! when a listed head is new, one of them is still running or pending, a head without one is due
//! again (backing off), or a finished one's MR/PR changed.
//! A finished pipeline on a commit rarely changes (a re-run does), so it's kept a while.

use gitvolt_core::forge::{ForgePipeline, PipelineStatus};
use std::collections::HashMap;
use std::sync::Mutex;

/// A finished pipeline (success, failed, canceled, skipped, manual) is asked again once its
/// MR/PR's `updated_at` moved, or after this long at most (a re-run moves no `updated_at`).
pub const SETTLED_SECS: i64 = 1800;
/// A commit with no pipeline yet (just pushed: its checks may start any moment) is asked again
/// after this long, then twice as long each time it still has none (2, 4, 8, 16 min)…
pub const NONE_SECS: i64 = 120;
/// …up to this (a repository without CI: twice an hour).
pub const NONE_MAX_SECS: i64 = 1800;
/// Commits kept at most; the oldest go first.
pub const KEPT: usize = 2000;

struct Kept {
    p: Option<ForgePipeline>,
    /// When it was last asked (unix seconds).
    at: i64,
    /// Answers in a row with no pipeline (the back-off's step).
    nones: u32,
    /// The MR/PR's `updated_at` then (unix seconds).
    stamp: i64,
}

#[derive(Default)]
pub struct PipelineCache {
    by_sha: Mutex<HashMap<String, Kept>>,
}

fn running(p: &Option<ForgePipeline>) -> bool {
    p.as_ref().is_some_and(|p| matches!(p.status, PipelineStatus::Running | PipelineStatus::Pending))
}

/// How long a head that had no pipeline `nones` times in a row waits before it's asked again.
pub fn none_wait(nones: u32) -> i64 {
    NONE_SECS.saturating_mul(1 << nones.saturating_sub(1).min(16)).min(NONE_MAX_SECS)
}

impl Kept {
    fn due(&self, stamp: i64, now: i64) -> bool {
        let age = now.saturating_sub(self.at);
        match &self.p {
            p if running(p) => true,
            None => age >= none_wait(self.nones),
            Some(_) => self.stamp != stamp || age >= SETTLED_SECS,
        }
    }
}

impl PipelineCache {
    /// Some head in `heads` (its sha and its MR/PR's `updated_at`) needs asking at `now`.
    pub fn needs<'a>(&self, heads: impl IntoIterator<Item = (&'a str, i64)>, now: i64) -> bool {
        let m = self.by_sha.lock().expect("pipelines poisoned");
        heads.into_iter().any(|(s, stamp)| m.get(&s.to_ascii_lowercase()).is_none_or(|k| k.due(stamp, now)))
    }

    /// What the forge answered for `sha` (of an MR/PR updated at `stamp`). A head that still has
    /// none, answered again before it was due (another head asked for every one), keeps its
    /// back-off as it was.
    pub fn put(&self, sha: &str, p: Option<ForgePipeline>, stamp: i64, now: i64) {
        let mut m = self.by_sha.lock().expect("pipelines poisoned");
        let sha = sha.to_ascii_lowercase();
        let nones = match (m.get(&sha), &p) {
            (Some(k), None) if k.p.is_none() && !k.due(stamp, now) => return,
            (Some(k), None) if k.p.is_none() => k.nones.saturating_add(1),
            (_, None) => 1,
            (_, Some(_)) => 0,
        };
        m.insert(sha, Kept { p, at: now, nones, stamp });
        if m.len() > KEPT {
            let mut ats: Vec<i64> = m.values().map(|k| k.at).collect();
            ats.sort_unstable();
            let cut = ats[m.len() - KEPT];
            m.retain(|_, k| k.at >= cut);
        }
    }

    /// What's kept for `sha`, however old (a failed refresh keeps the last one shown).
    pub fn get(&self, sha: &str) -> Option<ForgePipeline> {
        self.by_sha.lock().expect("pipelines poisoned").get(&sha.to_ascii_lowercase()).and_then(|k| k.p.clone())
    }

    /// Whether `sha` has an answer kept (a pipeline or none).
    pub fn has(&self, sha: &str) -> bool {
        self.by_sha.lock().expect("pipelines poisoned").contains_key(&sha.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(status: PipelineStatus) -> Option<ForgePipeline> {
        Some(ForgePipeline { status, web_url: None })
    }

    #[test]
    fn a_new_or_running_head_needs_asking_and_a_settled_one_once_its_mr_changed() {
        let c = PipelineCache::default();
        assert!(c.needs([("a", 1)], 0));
        c.put("A", p(PipelineStatus::Success), 1, 0);
        c.put("b", p(PipelineStatus::Running), 1, 0);
        assert!(!c.needs([("a", 1)], SETTLED_SECS - 1), "the case of a sha doesn't matter; the same MR: kept");
        assert!(c.needs([("a", 2)], 1), "its MR/PR changed: asked again");
        assert!(c.needs([("a", 1)], SETTLED_SECS), "a re-run moves nothing: asked again after a while");
        assert!(c.needs([("b", 1)], 1), "a running one is asked every poll");
        assert_eq!(c.get("a").unwrap().status, PipelineStatus::Success);
    }

    #[test]
    fn a_head_without_a_pipeline_backs_off_2_4_8_16_then_30_minutes() {
        let c = PipelineCache::default();
        let mut now = 0;
        let mut waits = Vec::new();
        for _ in 0..7 {
            c.put("c", None, 1, now);
            assert!(c.has("c") && c.get("c").is_none());
            let wait = (1..=NONE_MAX_SECS).find(|s| c.needs([("c", 1)], now + s)).unwrap();
            assert!(!c.needs([("c", 5)], now + wait - 1), "its MR/PR's changes don't hurry it");
            waits.push(wait / 60);
            now += wait;
        }
        assert_eq!(waits, [2, 4, 8, 16, 30, 30, 30]);
        // Answered again before it was due (the roll-up asked for another head): the back-off stays.
        c.put("c", None, 1, now);
        c.put("c", None, 1, now + 10);
        assert!(c.needs([("c", 1)], now + NONE_MAX_SECS));
        // A pipeline at last, then none again (a new run's gap): the back-off starts over.
        c.put("c", p(PipelineStatus::Success), 1, now);
        c.put("c", None, 2, now);
        assert!(c.needs([("c", 2)], now + NONE_SECS) && !c.needs([("c", 2)], now + NONE_SECS - 1));
    }

    #[test]
    fn the_oldest_go_past_the_cap() {
        let c = PipelineCache::default();
        for i in 0..KEPT + 10 {
            c.put(&format!("{i}"), None, 0, i as i64);
        }
        assert!(!c.has("0") && c.has(&format!("{}", KEPT + 9)));
    }
}
