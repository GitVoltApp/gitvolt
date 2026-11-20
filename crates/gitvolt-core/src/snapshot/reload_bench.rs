//! A timing of one HEAD-only graph reload on a generated repository (20k commits, 50k files,
//! window 2000), stage by stage. Ignored by default; in release mode:
//! `cargo nextest run --release -p gitvolt-core --run-ignored only --no-capture reload_bench`.
//! The test build's layout runs the continuity check (`gitvolt-graph/testing`): `assemble` is
//! timed without it, as the app runs it, and with it; `build_graph_with_text` here
//! includes it. Also the delta a tab holding the last payload is sent instead (`GraphReply`).

use super::*;
use crate::log::CommandLog;
use crate::testing::{isolated_git_env, TestRepo};
use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const COMMITS: usize = 20_000;
const FILES: usize = 50_000;
const STEPS: usize = 15;

const AUTHORS: [(&str, &str); 6] = [("Mara Quill", "mara@example.com"), ("Tobin Reyes", "tobin@example.com"), ("Ilse Varga", "ilse@example.com"), ("Oren Patel", "oren@example.com"), ("Juno Achebe", "juno@example.com"), ("Pell Sorensen", "pell@example.com")];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn path_of(i: usize) -> String {
    format!("src/m{:02}/d{:02}/f{:03}.rs", i / 1000, (i / 100) % 10, i % 100)
}

/// A main line with up to six topic branches at once, each merged back after 5–60 commits.
fn generate(r: &TestRepo) {
    let mut s: Vec<u8> = Vec::with_capacity(64 << 20);
    let mut rng = Rng(0x5eed_cafe);
    let mut mark = 0usize;
    let mut time = 1_700_000_000i64;
    let mut commit = |s: &mut Vec<u8>, rng: &mut Rng, parents: &[usize], files: &[usize], summary: &str| -> usize {
        mark += 1;
        time += 60;
        let (name, email) = AUTHORS[rng.below(AUTHORS.len())];
        let body = if rng.below(3) == 0 { format!("\n\nWhy: the {} path needed it.\nSee !{}\n", rng.below(500), rng.below(900) + 1) } else { String::new() };
        let msg = format!("{summary}{body}");
        write!(s, "commit refs/gen/tmp\nmark :{mark}\nauthor {name} <{email}> {time} +0000\ncommitter {name} <{email}> {time} +0000\ndata {}\n{msg}\n", msg.len()).unwrap();
        if let Some(p) = parents.first() {
            writeln!(s, "from :{p}").unwrap();
        }
        for p in parents.iter().skip(1) {
            writeln!(s, "merge :{p}").unwrap();
        }
        for &f in files {
            let content = format!("// file {f} at {mark}\n");
            write!(s, "M 100644 inline {}\ndata {}\n{content}\n", path_of(f), content.len()).unwrap();
        }
        mark
    };
    let all: Vec<usize> = (0..FILES).collect();
    let mut main = commit(&mut s, &mut rng, &[], &all, "Initial import");
    let mut topics: Vec<(usize, usize, usize, usize)> = vec![]; // (number, tip, length, target)
    let mut kept: Vec<(usize, usize)> = vec![];
    let mut tags = vec![];
    let mut next_topic = 0;
    let mut made = 1;
    while made < COMMITS {
        while topics.len() < 6 {
            next_topic += 1;
            topics.push((next_topic, main, 0, 5 + rng.below(56)));
        }
        let files = [rng.below(FILES), rng.below(FILES)];
        if rng.below(10) < 4 {
            main = commit(&mut s, &mut rng, &[main], &files, &format!("Tune the main loop, step {made}"));
            if made % 400 == 0 {
                tags.push((made / 400, main));
            }
        } else {
            let t = rng.below(topics.len());
            let (n, tip, len, target) = topics[t];
            let tip = commit(&mut s, &mut rng, &[tip], &files, &format!("Topic {n}: part {}", len + 1));
            topics[t] = (n, tip, len + 1, target);
            if len + 1 >= target {
                made += 1;
                main = commit(&mut s, &mut rng, &[main, tip], &[], &format!("Merge branch 'topic/{n}'"));
                kept.push((n, tip));
                topics.remove(t);
            }
        }
        made += 1;
    }
    for (n, tip, ..) in &topics {
        writeln!(s, "reset refs/heads/topic/{n}\nfrom :{tip}").unwrap();
    }
    for (n, tip) in kept.iter().rev().take(30) {
        writeln!(s, "reset refs/heads/topic/{n}\nfrom :{tip}").unwrap();
    }
    for (n, at) in &tags {
        writeln!(s, "reset refs/tags/v0.{n}\nfrom :{at}").unwrap();
    }
    writeln!(s, "reset refs/remotes/origin/main\nfrom :{main}\nreset refs/heads/main\nfrom :{main}").unwrap();
    let mut child = Command::new("git").current_dir(r.path()).args(["fast-import", "--quiet"]).envs(isolated_git_env()).stdin(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(&s).unwrap();
    assert!(child.wait().unwrap().success());
    r.git(&["update-ref", "-d", "refs/gen/tmp"]);
}

/// A new commit on main (its tree unchanged), as a rebase step or a commit moves HEAD.
fn step(r: &TestRepo, i: usize) {
    let head = r.git(&["rev-parse", "main"]);
    let new = r.git(&["commit-tree", "main^{tree}", "-p", &head, "-m", &format!("Step {i}")]);
    r.git(&["update-ref", "refs/heads/main", &new]);
}

#[derive(Default)]
struct Times(BTreeMap<&'static str, Vec<Duration>>);

impl Times {
    fn time<T>(&mut self, stage: &'static str, f: impl FnOnce() -> T) -> T {
        let t = Instant::now();
        let v = f();
        self.0.entry(stage).or_default().push(t.elapsed());
        v
    }
    fn median(&self, stage: &str) -> Duration {
        let mut v = self.0[stage].clone();
        v.sort();
        v[v.len() / 2]
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "a timing: run in release with --run-ignored only"]
async fn reload_bench() {
    let r = TestRepo::new();
    let t = Instant::now();
    generate(&r);
    eprintln!("generated {COMMITS} commits, {FILES} files in {:?}", t.elapsed());
    let workdir = r.path().to_path_buf();
    let ts = gix::ThreadSafeRepository::open(&workdir).unwrap();
    let cli = GitCli::new(Arc::new(CommandLog::new(100))).with_env(isolated_git_env());
    let wip = Arc::new(WipCache::default());
    wip.put(&workdir, wip.stamp(), WipEntry::from_raw(b""));
    let opts = BuildOptions { wip_cache: Some(wip), walk_cache: Some(Arc::default()), ..Default::default() };

    let mut times = Times::default();
    let t = Instant::now();
    let (mut last, _) = build_graph_with_text(ts.clone(), workdir.clone(), cli.clone(), opts.clone()).await.unwrap();
    eprintln!("cold build: {:?}", t.elapsed());
    let (mut bytes, mut delta_bytes) = (0, 0);
    for i in 0..STEPS {
        // Stage by stage, as `build_graph_with_text` runs them.
        step(&r, 2 * i);
        let t = Instant::now();
        let worktrees = listed_worktrees(&workdir, &opts).await.unwrap();
        times.0.entry("1 worktrees").or_default().push(t.elapsed());
        let wip = cached_wip(&worktrees, opts.wip_cache.as_deref());
        let repo = ts.to_thread_local();
        let (mut refs, tips) = times.time("2 refs", || {
            let refs = refs_input(read_refs(&repo).unwrap());
            let tips = gitvolt_graph::graph_tips(&refs, &worktrees_input(&worktrees));
            (refs, tips)
        });
        refs.remote_tags = remote_tags_input(&opts.remote_tags);
        let stash_ids: HashSet<Oid> = refs.stashes.iter().map(|s| s.id).collect();
        let walk_last = opts.walk_cache.as_ref().unwrap().lock().unwrap().as_ref().map(|w| w.walked.clone());
        let walked = times.time("3 walk (with find texts)", || cached_walk(&repo, tips, &stash_ids, &opts).unwrap());
        // `cached_walk` made them: again, on their own.
        times.time("5 find texts (inside 3)", || find_texts(&walked.result, walk_last.as_deref()));
        let input = GraphInput {
            refs,
            worktrees: worktrees_input(&worktrees),
            wip,
            in_progress: in_progress_of(&worktrees),
            workdir: canonical(&workdir).display().to_string(),
            active: None,
            pinned_ref: None,
            no_pin: false,
            forks: opts.forks.clone(),
            view: None,
        };
        // The app's build has no continuity check; the tests' (and so `build_graph_with_text`
        // here) runs it on every layout.
        let payload = times.time("4 assemble", || gitvolt_graph::without_continuity_check(|| gitvolt_graph::assemble(&input, &walked.result).unwrap()));
        times.time("4b assemble, with the tests' check", || gitvolt_graph::assemble(&input, &walked.result).unwrap());
        let json = times.time("6 json (whole payload)", || serde_json::to_vec(&payload).unwrap());
        bytes = json.len();
        // What a tab holding the last payload is sent instead: one commit on top.
        let delta = times.time("8 delta diff", || gitvolt_graph::diff_payloads(&last, &payload, 1, 2));
        let json = times.time("9 json (delta)", || serde_json::to_vec(&gitvolt_graph::GraphReply::Delta(delta)).unwrap());
        delta_bytes = json.len();
        // And the whole of it, after another step.
        step(&r, 2 * i + 1);
        let t = Instant::now();
        last = build_graph_with_text(ts.clone(), workdir.clone(), cli.clone(), opts.clone()).await.unwrap().0;
        times.0.entry("7 build_graph_with_text").or_default().push(t.elapsed());
    }
    eprintln!("\nHEAD-only reload, median of {STEPS} (window {DEFAULT_COMMIT_LIMIT}):");
    for stage in times.0.keys() {
        eprintln!("  {stage:<34} {:>9.2} ms", times.median(stage).as_secs_f64() * 1e3);
    }
    eprintln!("  payload (whole)                    {:>9.1} KB", bytes as f64 / 1024.0);
    eprintln!("  payload (delta, a commit on top)   {:>9.1} KB", delta_bytes as f64 / 1024.0);
}

/// A 20-commit rebase on the generated repository, its graph followed as the UI follows it: on
/// each `refsUpdated` one `graph` read naming the version held (a delta), none while one runs.
/// Counts the picks the graph showed on their own (a distinct HEAD in a read during the run).
/// `cargo nextest run --release -p gitvolt-core --run-ignored only --no-capture rebase_stream_bench`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "a timing: run in release with --run-ignored only"]
async fn rebase_stream_bench() {
    use crate::api::{Api, Request};
    use crate::events::AppEvent;
    use crate::write::integrate::IntegrateKind;
    const PICKS: usize = 20;
    let r = TestRepo::new();
    generate(&r);
    // A branch of 20 commits off an older main, each adding a file of its own: no conflicts.
    r.git(&["branch", "stream", "main~300"]);
    r.git(&["checkout", "-q", "-f", "stream"]);
    for i in 0..PICKS {
        r.write(&format!("stream/s{i:02}.txt"), &format!("step {i}\n"));
        r.git(&["add", "--", &format!("stream/s{i:02}.txt")]);
        r.git(&["commit", "-q", "-m", &format!("Stream step {i}")]);
    }
    let data = tempfile::tempdir().unwrap();
    let api = Api::new(GitCli::new(Arc::new(CommandLog::new(5000))).with_env(isolated_git_env()), None).with_data_dir(data.path().to_path_buf());
    let path = canonical(r.path()).display().to_string();
    let id = api.dispatch(Request::OpenRepo { path: path.clone() }).await.unwrap()["id"].as_u64().unwrap() as u32;
    // Watched, as the active tab is: a graph read takes the watcher's status, no `git status`.
    api.dispatch(Request::WatchRepo { repo: id }).await.unwrap();
    let graph = |since: u32| Request::Graph { repo: id, limit: None, pin: None, rescan: None, active: None, view: None, since: Some(since) };
    let first = api.dispatch(graph(0)).await.unwrap();
    let mut version = first["version"].as_u64().unwrap() as u32;
    let mut rx = api.subscribe();
    let started = Instant::now();
    let run = api.dispatch(Request::Integrate { repo: id, worktree: path, kind: IntegrateKind::Rebase, target: "main".into(), update_refs: Some(false), expect: Default::default(), ff_only: None, confirm: Default::default() });
    tokio::pin!(run);
    let (mut events, mut reads, mut deltas, mut bytes, mut read_ms) = (0, 0, 0, 0, Vec::new());
    let mut heads: Vec<String> = vec![];
    let res = loop {
        let mut dirty = false;
        tokio::select! {
            res = &mut run => break res,
            Ok(ev) = rx.recv() => if matches!(ev, AppEvent::RefsUpdated { .. }) {
                events += 1;
                dirty = true;
            },
        }
        // Events that came meanwhile coalesce into this one read, as the UI's refresh does.
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, AppEvent::RefsUpdated { .. }) {
                events += 1;
                dirty = true;
            }
        }
        if dirty {
            let t = Instant::now();
            let reply = api.dispatch(graph(version)).await.unwrap();
            read_ms.push(t.elapsed().as_secs_f64() * 1e3);
            reads += 1;
            deltas += usize::from(reply["kind"] == "delta");
            bytes += reply.to_string().len();
            version = reply["version"].as_u64().unwrap() as u32;
            let head = if reply["kind"] == "delta" { &reply["rest"]["head"]["target"] } else { &reply["graph"]["head"]["target"] };
            if let Some(h) = head.as_str()
                && heads.last().map(String::as_str) != Some(h)
            {
                heads.push(h.to_string());
            }
        }
    };
    let took = started.elapsed();
    assert_eq!(res.unwrap()["outcome"]["status"], "done");
    read_ms.sort_by(f64::total_cmp);
    let median = read_ms.get(read_ms.len() / 2).copied().unwrap_or(0.0);
    eprintln!(
        "\n{PICKS}-commit rebase in {took:?}: {events} refsUpdated, {reads} graph reads ({deltas} deltas, {:.1} KB each on average, median {median:.1} ms), {} distinct HEADs shown during the run",
        bytes as f64 / 1024.0 / reads.max(1) as f64,
        heads.len()
    );
}
