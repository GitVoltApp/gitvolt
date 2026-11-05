//! When the forge is asked (`poller.ts`) and what one poll asks (`poll.ts`). Only the shown tab polls; while nothing's due there's
//! one timer, no loop. A failed poll keeps what's shown and says so; a low rate-limit budget slows the next one.

use super::ask;
use super::store::{self, BranchMrs, CachedMrs, Fresh, MrList, RepoProjects, Refreshing, put};
use crate::app::{GitVolt, core_repo};
use gitvolt_core::api::Request;
use gitvolt_core::forge::types::{ForgeDiscussion, ForgeMr, ForgeMrDetail, MrFilter, MrState, PipelineStatus, RateLimitState};
use gpui::*;
use std::time::Duration;

pub const DETAIL_MAX_AGE_MS: i64 = 60_000;
/// A tab's `activate` polls fully at most this often, whichever poller asks.
pub const ACTIVATE_GAP_MS: i64 = 30_000;
pub const FAST_POLL_MS: u64 = 20_000;
pub const FAST_POLL_MAX_MS: u64 = 120_000;
pub const MIN_POLL_MS: u64 = 60_000;
/// A window focus polls again only once the last full poll is at least this old (or a tick was skipped meanwhile).
pub const FOCUS_GAP_MS: i64 = 10_000;
pub const JITTER: f64 = 0.2;
pub const MAX_BACKOFF_MS: u64 = 15 * 60_000;
/// Requests one poll costs, about: what a low rate-limit budget is spread over.
pub const POLL_COST: u64 = 3;
pub const LOW_BUDGET: f64 = 0.1;
pub const AFTER_PUSH_POLLS_MS: [u64; 2] = [5_000, 15_000];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Activate,
    Timer,
    Focus,
    Write,
    Fast,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    pub running_pipeline: bool,
    /// A server's interval, a rate limit's pacing or the failure backoff, in ms: a floor.
    pub server_interval_ms: Option<u64>,
    pub pipeline_key: Option<String>,
    pub failed: bool,
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn fast_poll_ms(streak: u32) -> u64 {
    FAST_POLL_MAX_MS.min(FAST_POLL_MS.saturating_mul(1u64 << streak.min(16)))
}

/// `nextPollDelay`: `None`: the fetch interval is off, so there are no timed polls (focus, activation and writes still poll).
pub fn next_poll_delay(fetch_ms: u64, last: Option<&Outcome>, streak: u32) -> Option<u64> {
    if fetch_ms == 0 {
        return None;
    }
    let interval = fetch_ms.max(MIN_POLL_MS);
    let base = if last.is_some_and(|o| o.running_pipeline) { interval.min(fast_poll_ms(streak)) } else { interval };
    Some(base.max(last.and_then(|o| o.server_interval_ms).unwrap_or(0)))
}

/// `backoffMs`: the wait after `failures` failed polls in a row.
pub fn backoff_ms(fetch_ms: u64, failures: u32) -> u64 {
    if failures == 0 {
        return 0;
    }
    MAX_BACKOFF_MS.min(fetch_ms.max(60_000).saturating_mul(1u64 << (failures - 1).min(16)))
}

/// `ratePacing`: `(floor, limited until)`, ms. Under `LOW_BUDGET` the polls left are spread until the reset.
pub fn rate_pacing(rl: Option<&RateLimitState>, now: i64) -> (u64, Option<i64>) {
    let Some(rl) = rl else { return (0, None) };
    if let Some(until) = rl.limited_until.map(|u| u * 1000).filter(|u| *u > now) {
        return ((until - now) as u64, Some(until));
    }
    let (Some(remaining), Some(limit), Some(reset)) = (rl.remaining, rl.limit.filter(|l| *l > 0), rl.reset_at) else { return (0, None) };
    if f64::from(remaining) >= f64::from(limit) * LOW_BUDGET {
        return (0, None);
    }
    let left = reset * 1000 - now;
    if left <= 0 {
        return (0, None);
    }
    if remaining == 0 {
        return (left as u64, Some(reset * 1000));
    }
    (((left as f64 * POLL_COST as f64) / f64::from(remaining)).ceil() as u64, None)
}

/// A pipeline runs, or the forge is merging it: worth a fast poll.
fn hurried(m: &ForgeMr) -> bool {
    m.pipeline.as_ref().is_some_and(|p| p.status == PipelineStatus::Running) || m.state == MrState::Merging
}

/// `runningShown`: the visible MRs/PRs whose pipeline runs: the open one's, and the list's while its section is expanded.
pub fn running_shown(f: &store::TabForge, list_shown: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |m: &ForgeMr| {
        if hurried(m) {
            let k = format!("{}:{}", m.number, m.head_sha.as_deref().unwrap_or_default());
            if !out.contains(&k) {
                out.push(k);
            }
        }
    };
    if let Some(open) = f.open_mr.and_then(|n| f.known_mr(n)) {
        add(open);
    }
    if list_shown && let Some(list) = &f.list {
        list.mrs.iter().for_each(&mut add);
    }
    out.sort();
    out
}

/// One tab's poller: `createForgePoller`, its decisions pure so a test can run them without a clock.
#[derive(Debug, Default)]
pub struct PollerState {
    pub running: bool,
    pub queued: Option<Reason>,
    pub missed: bool,
    pub last: Option<Outcome>,
    pub last_full: Option<i64>,
    pub failures: u32,
    pub streak: u32,
    pub focused: bool,
}

impl PollerState {
    pub fn new() -> Self {
        Self { focused: true, ..Default::default() }
    }

    /// Whether to run `reason` now; else it's queued (a full poll beats a fast one).
    pub fn begin(&mut self, reason: Reason, now: i64) -> bool {
        if self.running {
            self.queued = Some(match self.queued {
                None | Some(Reason::Fast) => reason,
                Some(q) if reason == Reason::Fast => q,
                Some(q) => q,
            });
            return false;
        }
        self.running = true;
        self.missed = false;
        if reason != Reason::Fast {
            self.last_full = Some(now);
        }
        true
    }

    /// A poll ended. The queued reason to run next, if any.
    pub fn finished(&mut self, outcome: Option<Outcome>) -> Option<Reason> {
        self.running = false;
        match outcome {
            Some(o) => {
                self.failures = 0;
                let same = o.running_pipeline && self.last.as_ref().is_some_and(|l| l.running_pipeline && l.pipeline_key == o.pipeline_key);
                self.streak = if same { self.streak + 1 } else { 0 };
                self.last = Some(o);
            }
            None => {
                self.failures += 1;
                let l = self.last.take().unwrap_or_default();
                self.last = Some(Outcome { running_pipeline: l.running_pipeline, server_interval_ms: l.server_interval_ms, ..Default::default() });
            }
        }
        self.queued.take()
    }

    /// The wait until the next timed poll.
    pub fn delay(&self, fetch_ms: u64, random: f64) -> Option<u64> {
        let base = next_poll_delay(fetch_ms, self.last.as_ref(), self.streak)?;
        let backoff = self.failures > 0 || self.last.as_ref().is_some_and(|o| o.failed);
        let d = base.max(backoff_ms(fetch_ms, self.failures));
        Some(if backoff { (d as f64 * (1. + JITTER * random)) as u64 } else { d })
    }

    /// A timer ran out. What to poll, or nothing: a tick while the window's away is skipped, and made up on its return.
    pub fn tick(&mut self, hidden: bool, fetch_ms: u64, now: i64) -> Option<Reason> {
        if hidden || !self.focused {
            self.missed = true;
            return None;
        }
        Some(if self.last_full.is_none_or(|l| now - l >= fetch_ms as i64) { Reason::Timer } else { Reason::Fast })
    }

    /// The window's focus changed: whether to poll.
    pub fn on_focus(&mut self, focused: bool, now: i64) -> bool {
        self.focused = focused;
        focused && (self.missed || self.last_full.is_none_or(|l| now - l >= FOCUS_GAP_MS))
    }
}

pub struct Poller {
    pub tab: u32,
    pub fetch_ms: u64,
    pub state: PollerState,
    pub epoch: u64,
    pub timer: Option<Task<()>>,
    pub pushes: Vec<Task<()>>,
}

fn fetch_interval_ms(v: &GitVolt) -> u64 {
    v.settings["fetchIntervalSecs"].as_u64().unwrap_or(60).saturating_mul(1000)
}

fn tab_forge(v: &mut GitVolt, tab: u32) -> Option<&mut store::TabForge> {
    v.tabs.iter_mut().find(|t| t.id == tab).map(|t| &mut t.forge)
}

/// `(the tab's still there, the value)` in the view.
fn with<R>(this: &WeakEntity<GitVolt>, cx: &mut AsyncApp, tab: u32, f: impl FnOnce(&mut store::TabForge) -> R) -> Option<R> {
    this.update(cx, |v, _| tab_forge(v, tab).map(f)).ok().flatten()
}

fn notify(this: &WeakEntity<GitVolt>, cx: &mut AsyncApp, tab: u32) {
    this.update(cx, |v, cx| {
        v.forge_list_changed(tab, cx);
        cx.notify();
    })
    .ok();
}

impl GitVolt {
    /// Only the shown tab's poller is live: a background tab never polls, and showing one polls at once. Called from the frame:
    /// cheap, and it does nothing unless the tab or the interval changed.
    pub fn forge_sync_poller(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = self.tabs.get(self.active) {
            super::card::set_chip_forge(t, &self.profile["hostOverrides"]);
        }
        self.forge_watch_compare(cx);
        let tab = self.tabs.get(self.active).filter(|t| !t.is_open_tab() && t.graph.is_some()).map(|t| t.id);
        let fetch_ms = fetch_interval_ms(self);
        if self.forge_poller.as_ref().map(|p| (p.tab, p.fetch_ms)) == tab.map(|t| (t, fetch_ms)) {
            return;
        }
        self.forge_poller = None;
        if let Some(tab) = tab {
            if cfg!(test) {
                return;
            }
            self.forge_read_filter(tab);
            self.forge_poller = Some(Poller { tab, fetch_ms, state: PollerState::new(), epoch: 0, timer: None, pushes: Vec::new() });
            self.forge_run(Reason::Activate, cx);
        }
    }

    /// The compare moved on: a review on it ends, and the ref kept for it goes. Nothing's held unless one's going.
    fn forge_watch_compare(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        if t.forge.review.is_none() && t.forge.compare_ref.is_none() {
            return;
        }
        let tab = t.id;
        let now = t.compare.as_ref().map(|c| (c.from.clone(), c.to.sha().map(str::to_string)));
        if t.forge.compare_seen == now {
            return;
        }
        t.forge.compare_seen = now;
        self.mr_compare_watch(tab, cx);
        self.review_settle(tab, cx);
    }

    /// The section's filter, kept per repository.
    fn forge_read_filter(&mut self, tab: u32) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let saved = self.profile["repos"][t.repo_path.as_str()]["mrFilter"].as_str().map(str::to_string);
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        if !t.forge.filter_read {
            t.forge.filter_read = true;
            t.forge.filter = match saved.as_deref() {
                Some("mine") => MrFilter::Mine,
                Some("reviewRequested") => MrFilter::ReviewRequested,
                _ => MrFilter::All,
            };
        }
    }

    pub fn forge_run(&mut self, reason: Reason, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let Some(p) = self.forge_poller.as_mut() else { return };
        if !p.state.begin(reason, now_ms()) {
            return;
        }
        p.timer = None;
        let (tab, epoch) = (p.tab, p.epoch);
        let weak = cx.weak_entity();
        cx.spawn(async move |this, cx| {
            let outcome = poll_once(weak, tab, reason, cx).await;
            this.update(cx, |v, cx| {
                // A fast poll follows a pipeline only; a review's own state goes with the full ones.
                if reason != Reason::Fast {
                    v.review_refresh(tab, cx);
                }
                let Some(p) = v.forge_poller.as_mut().filter(|p| p.tab == tab && p.epoch == epoch) else { return };
                let next = p.state.finished(outcome);
                match next {
                    Some(r) => v.forge_run(r, cx),
                    None => v.forge_schedule(cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn forge_schedule(&mut self, cx: &mut Context<Self>) {
        let random = (now_ms() % 1000) as f64 / 1000.;
        let Some(p) = self.forge_poller.as_mut() else { return };
        let Some(delay) = p.state.delay(p.fetch_ms, random) else {
            p.timer = None;
            return;
        };
        p.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(delay)).await;
            this.update(cx, |v, cx| {
                let hidden = v.window_hidden;
                let Some(p) = v.forge_poller.as_mut() else { return };
                let fetch_ms = p.fetch_ms;
                if let Some(r) = p.state.tick(hidden, fetch_ms, now_ms()) {
                    v.forge_run(r, cx);
                }
            })
            .ok();
        }));
    }

    /// The window got the focus back, or lost it.
    pub fn forge_on_focus(&mut self, focused: bool, cx: &mut Context<Self>) {
        let Some(p) = self.forge_poller.as_mut() else { return };
        if p.state.on_focus(focused, now_ms()) {
            self.forge_run(Reason::Focus, cx);
        }
    }

    /// After a GitVolt forge write: reads under way are dropped when they land, and the tab polls at once.
    pub fn forge_written(&mut self, tab: u32, cx: &mut Context<Self>) {
        if let Some(f) = tab_forge(self, tab) {
            f.write_epoch += 1;
        }
        self.forge_after_write(tab, cx);
    }

    pub fn forge_after_write(&mut self, tab: u32, cx: &mut Context<Self>) {
        if self.forge_poller.as_ref().is_some_and(|p| p.tab == tab) {
            self.forge_run(Reason::Write, cx);
        }
    }

    /// After a branch push: the forge starts its pipeline a moment after the push lands, so the tab polls now and shortly after.
    pub fn forge_branch_pushed(&mut self, tab: u32, cx: &mut Context<Self>) {
        self.forge_after_write(tab, cx);
        let Some(p) = self.forge_poller.as_mut().filter(|p| p.tab == tab) else { return };
        p.pushes = AFTER_PUSH_POLLS_MS
            .iter()
            .map(|ms| {
                let ms = *ms;
                cx.spawn(async move |this: WeakEntity<GitVolt>, cx: &mut AsyncApp| {
                    cx.background_executor().timer(Duration::from_millis(ms)).await;
                    this.update(cx, |v, cx| v.forge_after_write(tab, cx)).ok();
                })
            })
            .collect();
    }

    /// An account was added or removed: every tab's next poll asks the forges again, and the shown one polls at once.
    pub fn forge_accounts_changed(&mut self, cx: &mut Context<Self>) {
        super::search::clear_cache();
        for t in &mut self.tabs {
            t.forge.activated_at = None;
            t.forge.recheck = true;
            t.forge.limits = None;
            t.forge.limits_asked = false;
        }
        if let Some(tab) = self.forge_poller.as_ref().map(|p| p.tab) {
            self.forge_after_write(tab, cx);
        }
    }

    /// The list for the filter chosen now. An answer for a filter no longer chosen is dropped.
    pub fn forge_refresh_list(&mut self, tab: u32, cx: &mut Context<Self>) {
        let Some(f) = tab_forge(self, tab) else { return };
        if f.kind.is_none() {
            return;
        }
        let filter = f.filter;
        self.forge_ask::<MrList>(Request::ForgeMrList { repo: core_repo(tab), filter }, cx, move |v, answer, cx| {
            let Some(f) = tab_forge(v, tab) else { return };
            match answer {
                Ok(list) => {
                    if f.filter == list.filter {
                        f.list = Some(list);
                        f.error = None;
                    }
                }
                Err(e) => f.error = Some(e),
            }
            v.forge_list_changed(tab, cx);
        });
    }

    /// What a refresh button runs: nothing while the rate limit's spent; a second call meanwhile joins the first.
    fn forge_refresh_now(&mut self, tab: u32, scope: Refreshing, cx: &mut Context<Self>) {
        let Some(f) = tab_forge(self, tab) else { return };
        if f.refreshing.contains(&scope) || f.limited_until.is_some_and(|u| u > now_ms()) {
            return;
        }
        f.refreshing.push(scope);
        let weak = cx.weak_entity();
        let (api, rt) = (self.api.clone(), self.rt.clone());
        cx.spawn(async move |this, cx| {
            // The provider forgets what it kept a moment ago (a 304 still costs little), so the reads below ask the forge.
            let forgot = ask::<serde_json::Value>(api, rt, Request::ForgeRefresh { repo: core_repo(tab) }).await;
            match (scope, forgot) {
                (_, Err(e)) => {
                    with(&weak, cx, tab, |f| match scope {
                        Refreshing::List => f.error = Some(e),
                        Refreshing::Mr(n) => {
                            f.detail_errors.insert(n, e);
                        }
                    });
                }
                (Refreshing::List, Ok(_)) => {
                    poll_once(weak.clone(), tab, Reason::Write, cx).await;
                }
                (Refreshing::Mr(n), Ok(_)) => {
                    refresh_mr(&weak, tab, n, cx).await;
                }
            }
            this.update(cx, |v, cx| {
                if let Some(f) = tab_forge(v, tab) {
                    f.refreshing.retain(|s| *s != scope);
                }
                v.forge_list_changed(tab, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn forge_refresh_list_now(&mut self, tab: u32, cx: &mut Context<Self>) {
        self.forge_refresh_now(tab, Refreshing::List, cx);
    }

    pub fn forge_refresh_mr_now(&mut self, tab: u32, number: u64, cx: &mut Context<Self>) {
        self.forge_refresh_now(tab, Refreshing::Mr(number), cx);
    }

    /// An MR/PR's detail and discussion, now (the view opening). A failure is recorded for the view.
    pub fn forge_refresh_mr(&mut self, tab: u32, number: u64, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| refresh_mr(&weak, tab, number, cx).await).detach();
    }

    /// A hover card's detail: unless one younger than `DETAIL_MAX_AGE_MS` is loaded; one request at a time.
    pub fn forge_load_detail(&mut self, tab: u32, number: u64, cx: &mut Context<Self>) {
        let Some(f) = tab_forge(self, tab) else { return };
        if f.loading.contains(&number) || f.fresh_at.get(&number).is_some_and(|at| now_ms() - at < DETAIL_MAX_AGE_MS) {
            return;
        }
        f.loading.push(number);
        let epoch = f.write_epoch;
        self.forge_ask::<Fresh<ForgeMrDetail>>(Request::ForgeMrDetail { repo: core_repo(tab), number }, cx, move |v, answer, _| {
            let Some(f) = tab_forge(v, tab) else { return };
            f.loading.retain(|n| *n != number);
            f.fresh_at.insert(number, now_ms());
            match answer {
                Ok(d) if f.write_epoch == epoch => {
                    f.detail_errors.remove(&number);
                    f.details.insert(number, d.value);
                }
                Ok(_) => {}
                Err(e) => {
                    f.detail_errors.insert(number, e);
                }
            }
        });
    }
}

async fn refresh_mr(this: &WeakEntity<GitVolt>, tab: u32, number: u64, cx: &mut AsyncApp) {
    let Ok((api, rt)) = this.update(cx, |v, _| (v.api.clone(), v.rt.clone())) else { return };
    let Some(epoch) = with(this, cx, tab, |f| f.write_epoch) else { return };
    let repo = core_repo(tab);
    let detail = ask::<Fresh<ForgeMrDetail>>(api.clone(), rt.clone(), Request::ForgeMrDetail { repo, number });
    let threads = ask::<Fresh<Vec<ForgeDiscussion>>>(api, rt, Request::ForgeMrDiscussions { repo, number });
    let (detail, threads) = futures::join!(detail, threads);
    let changed = with(this, cx, tab, |f| {
        f.fresh_at.insert(number, now_ms());
        match (detail, threads) {
            (Ok(d), Ok(t)) => {
                if f.write_epoch != epoch {
                    return false;
                }
                let a = f.detail_errors.remove(&number).is_some();
                let b = f.details.get(&number) != Some(&d.value);
                if b {
                    f.details.insert(number, d.value);
                }
                let c = f.discussions.get(&number) != Some(&t.value);
                if c {
                    f.discussions.insert(number, t.value);
                }
                a || b || c
            }
            (Err(e), _) | (_, Err(e)) => f.detail_errors.insert(number, e.clone()).as_deref() != Some(e.as_str()),
        }
    });
    if changed == Some(true) {
        notify(this, cx, tab);
    }
}

/// `pollOnce`. Each part fails alone: badges that couldn't refresh keep theirs and say so, and the list is still asked. A failure
/// keeps what's shown, says so, and waits. An answer to a request that began before a write's answer is dropped.
async fn poll_once(this: WeakEntity<GitVolt>, tab: u32, reason: Reason, cx: &mut AsyncApp) -> Option<Outcome> {
    let (api, rt) = this.update(cx, |v, _| (v.api.clone(), v.rt.clone())).ok()?;
    let repo = core_repo(tab);
    if reason == Reason::Activate {
        let skip = with(&this, cx, tab, |f| {
            if f.activated_at.is_some_and(|a| now_ms() - a < ACTIVATE_GAP_MS) && f.error.is_none() {
                return true;
            }
            f.activated_at = Some(now_ms());
            false
        })?;
        if skip {
            return Some(outcome(&this, tab, None, cx));
        }
    }
    let epoch = with(&this, cx, tab, |f| f.write_epoch)?;
    let mut server_secs: u32 = 0;
    // A tab with nothing yet shows what the last session left at once (no request), marked stale.
    if with(&this, cx, tab, |f| f.updated_at.is_none() && f.list.is_none())? {
        show_cached(&this, tab, cx).await;
    }
    let poll = async {
        if reason != Reason::Fast || with(&this, cx, tab, |f| f.kind.is_none())? {
            let refresh = with(&this, cx, tab, |f| reason == Reason::Activate || std::mem::take(&mut f.recheck))?;
            let projects = match ask::<RepoProjects>(api.clone(), rt.clone(), Request::ForgeRepoProjects { repo, refresh }).await {
                Ok(p) => p,
                Err(e) => return Some(Err(e)),
            };
            let target = projects.remotes.iter().find(|r| Some(&r.remote) == projects.target.as_ref()).cloned();
            let remote_errors = projects.remotes.iter().filter_map(|r| Some((r.remote.clone(), r.error.clone()?))).collect();
            let owner_avatars = projects.remotes.iter().filter_map(|r| Some((r.remote.clone(), r.project.as_ref()?.owner_avatar_url.clone()?))).collect();
            let chosen = (projects.target.clone(), projects.target_chosen);
            with(&this, cx, tab, |f| {
                f.target = chosen.0;
                f.target_chosen = chosen.1;
                f.remote_errors = remote_errors;
                f.owner_avatars = owner_avatars;
            })?;
            notify(&this, cx, tab);
            let t = target.as_ref();
            if let Some(t) = t.filter(|t| t.project.is_none() && t.account.is_some() && t.error.is_some()) {
                // The target is on a forge but its project can't be loaded: say why, don't hide the section or show another remote.
                let (kind, remote, msg) = (t.account, t.remote.clone(), format!("{}: {}", t.path.as_deref().unwrap_or(&t.remote), t.error.as_deref().unwrap_or_default()));
                let fails = with(&this, cx, tab, |f| {
                    f.kind = kind;
                    f.remote = Some(remote);
                    f.project = None;
                    f.mapped.clear();
                    f.by_ref.clear();
                    f.history.clear();
                    f.upstreams.clear();
                    f.list = None;
                    f.error = Some(msg);
                    f.failures += 1;
                    f.updated_at = Some(now_ms());
                    f.failures
                })?;
                notify(&this, cx, tab);
                let fetch = this.update(cx, |v, _| fetch_interval_ms(v)).ok()?;
                return Some(Ok(Outcome { server_interval_ms: Some(backoff_ms(fetch, fails)), ..Default::default() }));
            }
            let Some((project, account)) = t.and_then(|t| Some((t.project.clone()?, t.account?))) else {
                with(&this, cx, tab, |f| {
                    f.kind = None;
                    f.remote = None;
                    f.project = None;
                    f.mapped.clear();
                    f.by_ref.clear();
                    f.history.clear();
                    f.upstreams.clear();
                    f.list = None;
                    f.error = None;
                    f.failures = 0;
                    f.updated_at = Some(now_ms());
                })?;
                notify(&this, cx, tab);
                return Some(Ok(Outcome::default()));
            };
            let host = project.host.clone();
            let known_me = with(&this, cx, tab, |f| f.me.clone())?;
            let me = match known_me {
                Some(m) => Some(m),
                None => ask::<Vec<super::accounts::AccountView>>(api.clone(), rt.clone(), Request::ForgeAccounts).await.ok().and_then(|a| a.into_iter().find(|x| x.account.host == host).map(|x| x.account.user.username)),
            };
            let mapped: Vec<String> = projects.remotes.iter().filter(|r| r.project.as_ref().is_some_and(|p| p.host.eq_ignore_ascii_case(&host))).map(|r| r.remote.clone()).collect();
            let remote = t.map(|t| t.remote.clone());
            with(&this, cx, tab, |f| {
                f.kind = Some(account);
                f.remote = remote;
                f.project = Some(project);
                f.mapped = mapped;
                f.me = me;
            })?;
        }
        // The badges failing (one slow request) costs only the badges: the list is still asked.
        let mut partial: Option<String> = None;
        if reason != Reason::Fast {
            let (refs, upstreams) = this
                .update(cx, |v, _| {
                    let t = v.tabs.iter().find(|t| t.id == tab)?;
                    Some(store::upstream_refs_of(&t.sidebar, Some(&t.forge)))
                })
                .ok()??;
            match ask::<BranchMrs>(api.clone(), rt.clone(), Request::ForgeBranchMrs { repo, refs }).await {
                Ok(b) => {
                    server_secs = server_secs.max(b.poll_interval_secs.unwrap_or(0));
                    with(&this, cx, tab, |f| {
                        if f.write_epoch == epoch {
                            let mut c = put(&mut f.by_ref, b.mrs.into_iter().map(|x| (x.remote_ref, x.mr)).collect());
                            c |= put(&mut f.history, b.history.into_iter().map(|x| (x.remote_ref, x.mr)).collect());
                            c |= put(&mut f.upstreams, upstreams);
                            c
                        } else {
                            false
                        }
                    })?;
                }
                Err(e) => partial = Some(e),
            }
        }
        let filter = with(&this, cx, tab, |f| f.filter)?;
        let list = match ask::<MrList>(api.clone(), rt.clone(), Request::ForgeMrList { repo, filter }).await {
            Ok(l) => l,
            Err(e) => return Some(Err(e)),
        };
        server_secs = server_secs.max(list.poll_interval_secs.unwrap_or(0));
        let rate = list.rate_limit.clone();
        with(&this, cx, tab, |f| {
            // A filter chosen meanwhile has its own list coming.
            if f.write_epoch == epoch && f.filter == filter && !f.list.as_ref().is_some_and(|l| l.same_as(&list)) {
                f.list = Some(list);
            }
        })?;
        // The open MR's own failure is in `detail_errors`; it doesn't fail the badges and the list.
        if let Some(open) = with(&this, cx, tab, |f| f.open_mr)? {
            refresh_mr(&this, tab, open, cx).await;
        }
        let (floor, limited) = rate_pacing(rate.as_ref(), now_ms());
        let floor = floor.max(u64::from(server_secs) * 1000);
        with(&this, cx, tab, |f| {
            f.updated_at = Some(now_ms());
            f.limited_until = limited;
            f.cached_at = None;
            f.error = partial.clone();
            if partial.is_none() {
                f.failures = 0;
            }
        })?;
        Some(Ok(Outcome { failed: partial.is_some(), server_interval_ms: (floor > 0).then_some(floor), ..Default::default() }))
    };
    let answer = poll.await;
    let result = match answer {
        None => return None,
        Some(Ok(mut o)) => {
            let shown = outcome(&this, tab, o.server_interval_ms, cx);
            o.running_pipeline = shown.running_pipeline;
            o.pipeline_key = shown.pipeline_key;
            o
        }
        Some(Err(e)) => {
            let fetch = this.update(cx, |v, _| fetch_interval_ms(v)).ok()?;
            let fails = with(&this, cx, tab, |f| {
                f.error = Some(e);
                f.failures += 1;
                f.failures
            })?;
            Outcome { server_interval_ms: Some(backoff_ms(fetch, fails)), failed: true, ..Default::default() }
        }
    };
    notify(&this, cx, tab);
    Some(result)
}

fn outcome(this: &WeakEntity<GitVolt>, tab: u32, server: Option<u64>, cx: &mut AsyncApp) -> Outcome {
    let running = this
        .update(cx, |v, _| {
            let Some(t) = v.tabs.iter().find(|t| t.id == tab) else { return Vec::new() };
            let shown = !t.side.collapsed.contains(&crate::sidebar::section_key(crate::sidebar::SectionKind::Mrs));
            running_shown(&t.forge, shown)
        })
        .unwrap_or_default();
    if running.is_empty() { Outcome { server_interval_ms: server, ..Default::default() } } else { Outcome { running_pipeline: true, server_interval_ms: server, pipeline_key: Some(running.join(" ")), failed: false } }
}

async fn show_cached(this: &WeakEntity<GitVolt>, tab: u32, cx: &mut AsyncApp) {
    let Ok((api, rt)) = this.update(cx, |v, _| (v.api.clone(), v.rt.clone())) else { return };
    let Ok(Some((refs, upstreams, filter))) = this.update(cx, |v, _| {
        let t = v.tabs.iter().find(|t| t.id == tab)?;
        let (r, u) = store::upstream_refs_of(&t.sidebar, Some(&t.forge));
        Some((r, u, t.forge.filter))
    }) else {
        return;
    };
    let Ok(Some(cached)) = ask::<Option<CachedMrs>>(api, rt, Request::ForgeCachedMrs { repo: core_repo(tab), refs, filter }).await else { return };
    with(this, cx, tab, |f| {
        // A poll answered meanwhile: what it said is newer.
        if f.updated_at.is_some() {
            return;
        }
        let at = cached.saved_at * 1000;
        f.kind = Some(cached.kind);
        f.remote = Some(cached.remote);
        f.project = Some(cached.project);
        if f.filter == filter {
            f.list = cached.list;
        }
        if let Some(b) = cached.badges {
            f.by_ref = b.mrs.into_iter().map(|x| (x.remote_ref, x.mr)).collect();
            f.history = b.history.into_iter().map(|x| (x.remote_ref, x.mr)).collect();
            f.upstreams = upstreams;
        }
        f.updated_at = Some(at);
        f.cached_at = Some(at);
    });
    notify(this, cx, tab);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn run(p: &mut PollerState, r: Reason, now: i64, o: Option<Outcome>) -> Option<Reason> {
        assert!(p.begin(r, now));
        p.finished(o)
    }

    #[test]
    fn timed_polls_follow_the_interval_and_never_come_under_a_minute() {
        assert_eq!(next_poll_delay(0, None, 0), None, "off: no timed polls");
        assert_eq!(next_poll_delay(30_000, None, 0), Some(60_000));
        assert_eq!(next_poll_delay(300_000, None, 0), Some(300_000));
        let running = Outcome { running_pipeline: true, ..Default::default() };
        assert_eq!(next_poll_delay(300_000, Some(&running), 0), Some(20_000));
        assert_eq!(next_poll_delay(300_000, Some(&running), 1), Some(40_000));
        assert_eq!(next_poll_delay(300_000, Some(&running), 9), Some(120_000));
        let floored = Outcome { server_interval_ms: Some(90_000), ..running };
        assert_eq!(next_poll_delay(60_000, Some(&floored), 0), Some(90_000));
    }

    #[test]
    fn failures_back_off_and_a_success_starts_over() {
        assert_eq!(backoff_ms(60_000, 0), 0);
        assert_eq!(backoff_ms(60_000, 1), 60_000);
        assert_eq!(backoff_ms(60_000, 3), 240_000);
        assert_eq!(backoff_ms(60_000, 40), MAX_BACKOFF_MS);
        let mut p = PollerState::new();
        run(&mut p, Reason::Activate, 0, None);
        assert_eq!(p.failures, 1);
        run(&mut p, Reason::Timer, 0, Some(Outcome::default()));
        assert_eq!(p.failures, 0);
    }

    #[test]
    fn a_reason_arriving_meanwhile_runs_after_and_a_full_poll_beats_a_fast_one() {
        let mut p = PollerState::new();
        assert!(p.begin(Reason::Activate, 0));
        assert!(!p.begin(Reason::Fast, 0));
        assert!(!p.begin(Reason::Write, 0));
        assert!(!p.begin(Reason::Fast, 0));
        assert_eq!(p.finished(Some(Outcome::default())), Some(Reason::Write));
    }

    #[test]
    fn a_tick_while_the_window_is_away_is_made_up_when_it_returns() {
        let mut p = PollerState::new();
        run(&mut p, Reason::Activate, 0, Some(Outcome::default()));
        assert!(!p.on_focus(false, 1_000));
        assert_eq!(p.tick(false, 60_000, 70_000), None);
        assert!(p.missed);
        assert!(p.on_focus(true, 71_000), "a skipped tick polls on the return");
        let mut q = PollerState::new();
        run(&mut q, Reason::Activate, 0, Some(Outcome::default()));
        assert!(!q.on_focus(true, 5_000), "too soon after the last full poll");
        assert!(q.on_focus(true, 10_000));
    }

    #[test]
    fn a_tick_is_full_once_the_interval_has_passed_else_only_whats_on_screen() {
        let mut p = PollerState::new();
        run(&mut p, Reason::Activate, 0, Some(Outcome::default()));
        assert_eq!(p.tick(false, 60_000, 20_000), Some(Reason::Fast));
        assert_eq!(p.tick(false, 60_000, 60_000), Some(Reason::Timer));
    }

    #[test]
    fn the_same_pipeline_running_waits_longer_and_a_new_one_starts_again() {
        let mut p = PollerState::new();
        let a = Outcome { running_pipeline: true, pipeline_key: Some("1:a".into()), ..Default::default() };
        run(&mut p, Reason::Activate, 0, Some(a.clone()));
        assert_eq!(p.delay(600_000, 0.), Some(20_000));
        run(&mut p, Reason::Fast, 0, Some(a.clone()));
        assert_eq!(p.delay(600_000, 0.), Some(40_000));
        run(&mut p, Reason::Fast, 0, Some(Outcome { pipeline_key: Some("1:b".into()), ..a }));
        assert_eq!(p.delay(600_000, 0.), Some(20_000));
    }

    #[test]
    fn a_low_budget_spreads_the_polls_left_until_the_reset() {
        let now = 1_000_000;
        assert_eq!(rate_pacing(None, now), (0, None));
        let rl = |remaining, limit, reset| RateLimitState { limited_until: None, remaining: Some(remaining), reset_at: Some(reset), limit: Some(limit) };
        assert_eq!(rate_pacing(Some(&rl(500, 1000, 2000)), now), (0, None));
        assert_eq!(rate_pacing(Some(&rl(10, 1000, 2000)), now), (300_000, None));
        assert_eq!(rate_pacing(Some(&rl(0, 1000, 2000)), now), (1_000_000, Some(2_000_000)));
        let limited = RateLimitState { limited_until: Some(3000), ..Default::default() };
        assert_eq!(rate_pacing(Some(&limited), now), (2_000_000, Some(3_000_000)));
    }
}
