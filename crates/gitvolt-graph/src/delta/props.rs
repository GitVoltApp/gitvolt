//! `apply(old, diff(old, new)) == new` over random repositories changing at random: commits,
//! rebases (a new chain and the branch moved onto it), ref moves, stashes, worktrees and their
//! WIP, the pin on and off, branch-preset views, the window's size and the active worktree. A
//! seeded generator, so a failure names its seed and replays.

use super::{apply, diff, GraphDelta};
use crate::input::{GraphInput, GraphRequest, HeadIn, RefIn, RefKind, RefsIn, StashIn, WorktreeIn};
use crate::payload::{GraphPayload, InProgress, WipCounts};
use crate::presets::{BranchPicks, CollapsePlacement, Matcher, PresetAction, Rule};
use crate::view::ViewSpec;
use crate::walk::CommitMeta;
use crate::{build, Oid};
use std::collections::{BTreeMap, HashMap};

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        (!items.is_empty()).then(|| &items[self.below(items.len())])
    }
}

const AUTHORS: [&str; 3] = ["mara@example.com", "tobin@example.com", "ilse@example.com"];
const MAIN: &str = "/demo/x";

/// A repository and how the graph is asked for, as plain data.
#[derive(Clone)]
struct World {
    commits: HashMap<Oid, CommitMeta>,
    next: u32,
    time: i64,
    branches: BTreeMap<String, Oid>,
    /// `origin/<name>`.
    remotes: BTreeMap<String, Oid>,
    tags: BTreeMap<String, Oid>,
    /// The main worktree's branch; `None`: detached at `detached`.
    head: Option<String>,
    detached: Oid,
    /// Newest first: (stash commit, its base).
    stashes: Vec<(Oid, Oid)>,
    /// Linked worktrees: (path, branch).
    linked: Vec<(String, String)>,
    /// Per worktree path, its WIP counts.
    wip: BTreeMap<String, WipCounts>,
    rebasing: bool,
    no_pin: bool,
    pinned_ref: Option<String>,
    view: Option<ViewSpec>,
    limit: usize,
    active: Option<String>,
}

impl World {
    fn oid(&mut self) -> Oid {
        self.next += 1;
        let mut b = [0x5a; 20];
        b[..4].copy_from_slice(&self.next.to_be_bytes());
        Oid(b)
    }

    fn commit(&mut self, rng: &mut Rng, parents: &[Oid], summary: &str) -> Oid {
        let id = self.oid();
        // Now and then a clock behind its parent's: the walk puts children first anyway.
        self.time += if rng.one_in(12) { -500 } else { 60 };
        let email = AUTHORS[rng.below(AUTHORS.len())];
        let name = email.split('@').next().unwrap().to_string();
        let message = if rng.one_in(3) { format!("{summary}\n\nWhy: see !{}", rng.below(90) + 1) } else { summary.to_string() };
        self.commits.insert(id, CommitMeta::from_message(id, parents.to_vec(), name, email.into(), self.time, self.time, &message));
        id
    }

    fn head_tip(&self) -> Oid {
        self.head.as_ref().map_or(self.detached, |b| self.branches[b])
    }

    fn all_commits(&self) -> Vec<Oid> {
        let mut v: Vec<Oid> = self.commits.keys().copied().collect();
        v.sort();
        v
    }

    fn branch_names(&self) -> Vec<String> {
        self.branches.keys().cloned().collect()
    }

    fn checked_out(&self, b: &str) -> bool {
        self.head.as_deref() == Some(b) || self.linked.iter().any(|(_, l)| l == b)
    }

    fn new(rng: &mut Rng) -> World {
        let mut w = World {
            commits: HashMap::new(),
            next: 0,
            time: 1_700_000_000,
            branches: BTreeMap::new(),
            remotes: BTreeMap::new(),
            tags: BTreeMap::new(),
            head: Some("main".into()),
            detached: Oid::NULL,
            stashes: vec![],
            linked: vec![],
            wip: BTreeMap::new(),
            rebasing: false,
            no_pin: false,
            pinned_ref: None,
            view: None,
            limit: 200,
            active: None,
        };
        let root = w.commit(rng, &[], "Initial import");
        w.branches.insert("main".into(), root);
        for i in 0..10 + rng.below(40) {
            match rng.below(20) {
                0..8 => {
                    let tip = w.branches["main"];
                    let c = w.commit(rng, &[tip], &format!("Main step {i}"));
                    w.branches.insert("main".into(), c);
                }
                8..15 if w.branches.len() > 1 => {
                    let names: Vec<String> = w.branches.keys().filter(|b| *b != "main").cloned().collect();
                    let b = rng.pick(&names).unwrap().clone();
                    let c = w.commit(rng, &[w.branches[&b]], &format!("{b}: part {i}"));
                    w.branches.insert(b, c);
                }
                15..18 if w.branches.len() > 1 => {
                    let names: Vec<String> = w.branches.keys().filter(|b| *b != "main").cloned().collect();
                    let b = rng.pick(&names).unwrap().clone();
                    let parents = [w.branches["main"], w.branches[&b]];
                    let c = w.commit(rng, &parents, &format!("Merge branch '{b}'"));
                    w.branches.insert("main".into(), c);
                    if rng.one_in(2) {
                        w.branches.remove(&b);
                    }
                }
                _ => {
                    let from = *rng.pick(&w.all_commits()).unwrap();
                    w.branches.insert(format!("feat/{i}"), from);
                }
            }
        }
        for (b, &t) in &w.branches {
            if rng.one_in(2) {
                w.remotes.insert(b.clone(), t);
            }
        }
        w.remotes.insert("main".into(), w.branches["main"]);
        w
    }

    /// One random change, of what the repository holds or of how the graph is asked for.
    fn change(&mut self, rng: &mut Rng) -> &'static str {
        match rng.below(17) {
            0 | 1 => {
                // A commit on HEAD (as each pick of a rebase makes one).
                let tip = self.head_tip();
                let c = self.commit(rng, &[tip], "On top");
                match self.head.clone() {
                    Some(b) => {
                        self.branches.insert(b, c);
                    }
                    None => self.detached = c,
                }
                "commit"
            }
            2 => {
                // A rebase: the branch's newest commits copied onto main, the branch moved there.
                let names: Vec<String> = self.branches.keys().filter(|b| *b != "main").cloned().collect();
                let Some(b) = rng.pick(&names).cloned() else { return "no rebase" };
                let mut chain = vec![];
                let mut at = self.branches[&b];
                for _ in 0..1 + rng.below(4) {
                    chain.push(at);
                    match self.commits[&at].parents.first() {
                        Some(&p) => at = p,
                        None => break,
                    }
                }
                let mut onto = self.branches.get("main").copied().unwrap_or_else(|| self.head_tip());
                for old in chain.into_iter().rev() {
                    let summary = self.commits[&old].summary.clone();
                    onto = self.commit(rng, &[onto], &summary);
                }
                self.branches.insert(b, onto);
                "rebase"
            }
            3 => {
                let names = self.branch_names();
                let b = rng.pick(&names).unwrap().clone();
                let to = *rng.pick(&self.all_commits()).unwrap();
                self.branches.insert(b, to);
                "ref move"
            }
            4 => {
                let names: Vec<String> = self.branches.keys().filter(|b| !self.checked_out(b)).cloned().collect();
                if let Some(b) = rng.pick(&names).cloned() {
                    self.branches.remove(&b);
                } else {
                    let to = *rng.pick(&self.all_commits()).unwrap();
                    self.branches.insert(format!("new/{}", self.next), to);
                }
                "branch delete or create"
            }
            5 => {
                let to = *rng.pick(&self.all_commits()).unwrap();
                if rng.one_in(2) && !self.tags.is_empty() {
                    let names: Vec<String> = self.tags.keys().cloned().collect();
                    self.tags.remove(rng.pick(&names).unwrap());
                } else {
                    self.tags.insert(format!("v0.{}", self.next), to);
                }
                "tag"
            }
            6 => {
                // A push or a fetch: a remote branch follows its local one, or moves on.
                let names = self.branch_names();
                let b = rng.pick(&names).unwrap().clone();
                if rng.one_in(2) {
                    let t = self.branches[&b];
                    self.remotes.insert(b, t);
                } else {
                    let base = self.remotes.get(&b).copied().unwrap_or(self.branches[&b]);
                    let c = self.commit(rng, &[base], "Fetched");
                    self.remotes.insert(b, c);
                }
                "remote"
            }
            7 => {
                if rng.one_in(3) && !self.stashes.is_empty() {
                    self.stashes.remove(0);
                } else {
                    let base = self.head_tip();
                    let index = self.commit(rng, &[base], "index on main");
                    let stash = self.commit(rng, &[base, index], "WIP on main");
                    self.stashes.insert(0, (stash, base));
                }
                "stash"
            }
            8 => {
                let free: Vec<String> = self.branches.keys().filter(|b| !self.checked_out(b)).cloned().collect();
                if rng.one_in(2) && !self.linked.is_empty() {
                    let k = rng.below(self.linked.len());
                    let (path, _) = self.linked.remove(k);
                    self.wip.remove(&path);
                    if self.active.as_deref() == Some(path.as_str()) {
                        self.active = None;
                    }
                } else if let Some(b) = rng.pick(&free).cloned() {
                    self.linked.push((format!("/demo/wt-{}", self.next), b));
                    self.next += 1;
                }
                "worktree"
            }
            9 => {
                let paths: Vec<String> = std::iter::once(MAIN.to_string()).chain(self.linked.iter().map(|(p, _)| p.clone())).collect();
                let p = rng.pick(&paths).unwrap().clone();
                if rng.one_in(3) {
                    self.wip.remove(&p);
                } else {
                    self.wip.insert(p, WipCounts { modified: rng.below(4) as u32, added: rng.below(3) as u32, conflicted: rng.below(2) as u32, ..Default::default() });
                }
                "wip"
            }
            10 => {
                self.no_pin = rng.one_in(2);
                let mut refs: Vec<String> = self.branches.keys().map(|b| format!("refs/heads/{b}")).collect();
                refs.extend(self.remotes.keys().map(|b| format!("refs/remotes/origin/{b}")));
                self.pinned_ref = if rng.one_in(2) { None } else { rng.pick(&refs).cloned() };
                "pin"
            }
            11 | 12 => {
                self.view = (!rng.one_in(4)).then(|| self.random_view(rng));
                "view"
            }
            13 => {
                self.limit = [3, 8, 15, 40, 200][rng.below(5)];
                "limit"
            }
            14 => {
                let paths: Vec<String> = self.linked.iter().map(|(p, _)| p.clone()).collect();
                self.active = if rng.one_in(2) { None } else { rng.pick(&paths).cloned() };
                "active"
            }
            15 => {
                // A checkout: another free branch, or HEAD detached.
                let free: Vec<String> = self.branches.keys().filter(|b| !self.checked_out(b)).cloned().collect();
                if rng.one_in(3) || free.is_empty() {
                    self.detached = *rng.pick(&self.all_commits()).unwrap();
                    self.head = None;
                } else {
                    self.head = rng.pick(&free).cloned();
                }
                "checkout"
            }
            _ => {
                self.rebasing = !self.rebasing;
                "in progress"
            }
        }
    }

    fn random_view(&self, rng: &mut Rng) -> ViewSpec {
        let actions = [PresetAction::Show, PresetAction::Hide, PresetAction::Collapse];
        let matchers = [Matcher::NameMatches { patterns: "feat/*".into() }, Matcher::Remote, Matcher::Local, Matcher::OlderThan { days: 1 }, Matcher::MergedIntoTrunk, Matcher::CheckedOut];
        let rules = (0..rng.below(3)).map(|_| Rule { action: actions[rng.below(3)], matcher: matchers[rng.below(matchers.len())].clone() }).collect();
        let names = self.branch_names();
        let picks = BranchPicks {
            hidden: (0..rng.below(2)).filter_map(|_| rng.pick(&names).cloned()).collect(),
            shown: (0..rng.below(2)).filter_map(|_| rng.pick(&names).cloned()).collect(),
            ..Default::default()
        };
        ViewSpec {
            rules,
            otherwise: actions[rng.below(3)],
            picks,
            fold_merged: rng.one_in(2),
            ghost_lines: rng.one_in(2),
            ghost_picks: rng.one_in(2),
            expanded: (0..rng.below(2)).filter_map(|_| rng.pick(&names).map(|n| format!("branch:{n}"))).collect(),
            placement: if rng.one_in(2) { CollapsePlacement::KeepDateOrder } else { CollapsePlacement::PullTogether },
            now: self.time,
            ..Default::default()
        }
    }

    fn graph(&self) -> GraphPayload {
        let local = |b: &str, t: Oid| RefIn {
            full_name: format!("refs/heads/{b}"),
            short_name: b.into(),
            kind: RefKind::Local,
            target: t,
            upstream: self.remotes.contains_key(b).then(|| format!("refs/remotes/origin/{b}")),
            upstream_mismatch: None,
            annotation: None,
        };
        let mut refs: Vec<RefIn> = self.branches.iter().map(|(b, &t)| local(b, t)).collect();
        refs.extend(self.remotes.iter().map(|(b, &t)| RefIn { full_name: format!("refs/remotes/origin/{b}"), short_name: format!("origin/{b}"), kind: RefKind::Remote { remote: "origin".into() }, upstream: None, ..local(b, t) }));
        refs.extend(self.tags.iter().map(|(t, &at)| RefIn { full_name: format!("refs/tags/{t}"), short_name: t.clone(), kind: RefKind::Tag, upstream: None, ..local(t, at) }));
        let head = HeadIn { branch: self.head.as_ref().map(|b| format!("refs/heads/{b}")), target: Some(self.head_tip()), detached: self.head.is_none(), unborn: false };
        let stashes = self.stashes.iter().enumerate().map(|(i, &(id, _))| StashIn { index: i, id, message: format!("WIP on main: stash {i}"), time: self.commits[&id].committer_time }).collect();
        let mut worktrees = vec![WorktreeIn { path: MAIN.into(), canonical: MAIN.into(), head: head.target, branch: head.branch.clone(), is_main: true, bare: false, prunable: false, locked: false }];
        worktrees.extend(self.linked.iter().map(|(p, b)| WorktreeIn { path: p.clone(), canonical: p.clone(), head: Some(self.branches[b]), branch: Some(format!("refs/heads/{b}")), is_main: false, bare: false, prunable: false, locked: false }));
        let wip = worktrees.iter().enumerate().filter_map(|(i, w)| self.wip.get(&w.path).map(|c| (i, *c))).collect();
        let mut in_progress = BTreeMap::new();
        if self.rebasing {
            in_progress.insert(MAIN.to_string(), InProgress::Other { what: "am".into() });
        }
        let input = GraphInput {
            refs: RefsIn { head, refs, remote_heads: HashMap::from([("origin".to_string(), "main".to_string())]), stashes, ..Default::default() },
            worktrees,
            wip,
            in_progress,
            workdir: MAIN.into(),
            active: self.active.clone(),
            pinned_ref: self.pinned_ref.clone(),
            no_pin: self.no_pin,
            forks: Default::default(),
            view: self.view.clone(),
        };
        build(&GraphRequest { input, limit: self.limit }, &self.commits).expect("a laid out graph")
    }
}

fn json(g: &GraphPayload) -> serde_json::Value {
    serde_json::to_value(g).unwrap()
}

/// `diff` then `apply`, checked; the delta.
fn round_trip(old: &GraphPayload, new: &GraphPayload, what: &str) -> GraphDelta {
    let d = diff(old, new, 1, 2);
    let applied = apply(old, &d).unwrap_or_else(|| panic!("{what}: the delta doesn't apply to the payload it was made against"));
    assert!(json(&applied) == json(new), "{what}: apply(old, diff(old, new)) != new");
    d
}

#[test]
fn applying_the_diff_of_two_payloads_gives_the_new_one() {
    // What the deltas did, so a generator that stopped making some change shows.
    let (mut inserted, mut removed, mut patched, mut viewed) = (0, 0, 0, 0);
    for seed in 0..120u64 {
        let mut rng = Rng::new(seed);
        let mut world = World::new(&mut rng);
        let mut old = world.graph();
        for step in 0..10 {
            let changes: Vec<&str> = (0..1 + rng.below(2)).map(|_| world.change(&mut rng)).collect();
            let new = world.graph();
            let d = round_trip(&old, &new, &format!("seed {seed}, step {step} ({changes:?})"));
            inserted += usize::from(d.splices.iter().any(|s| !s.insert.is_empty()));
            removed += usize::from(d.splices.iter().any(|s| s.remove > 0));
            patched += usize::from(!d.patched.is_empty());
            viewed += usize::from(new.view.is_some());
            // And from a payload that has nothing in common.
            round_trip(&new, &super::tests::empty(), &format!("seed {seed}, step {step}, to empty"));
            old = new;
        }
    }
    for (what, n) in [("rows inserted", inserted), ("rows removed", removed), ("rows patched", patched), ("under a preset", viewed)] {
        assert!(n > 100, "{what}: {n} of 1200 deltas");
    }
}

/// The golden triples the UI's applier replays (ui/src/graph/delta.test.ts): JSON `[{old, delta,
/// new}]` from a few seeded changes, written to `ui/src/graph/delta.golden.json` when
/// `GITVOLT_WRITE_GOLDEN` is set, and else compared with it.
#[test]
fn golden_triples_for_the_ui() {
    let mut out = String::from("[\n");
    let mut first = true;
    for seed in [3u64, 7, 11, 19, 23, 42] {
        let mut rng = Rng::new(seed);
        let mut world = World::new(&mut rng);
        world.limit = 8;
        let mut old = world.graph();
        let mut kept = 0;
        while kept < 2 {
            world.change(&mut rng);
            world.limit = world.limit.min(8);
            let new = world.graph();
            let d = round_trip(&old, &new, &format!("golden seed {seed}"));
            // Only deltas that change rows: the rest are the payloads again.
            if d.splices.is_empty() && d.patched.is_empty() {
                old = new;
                continue;
            }
            kept += 1;
            let triple = serde_json::json!({ "old": old, "delta": super::GraphReply::Delta(d), "new": new });
            if !first {
                out.push_str(",\n");
            }
            first = false;
            out.push_str(&serde_json::to_string(&triple).unwrap());
            old = new;
        }
    }
    out.push_str("\n]\n");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/graph/delta.golden.json");
    if std::env::var_os("GITVOLT_WRITE_GOLDEN").is_some() {
        std::fs::write(&path, &out).unwrap();
    } else {
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(have == out, "{} is out of date: rerun with GITVOLT_WRITE_GOLDEN=1", path.display());
    }
}
