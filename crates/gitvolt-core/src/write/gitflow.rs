//! Git flow: its settings, kept where the `git flow` command line keeps them (`gitflow.*` in the
//! repository's config), so either tool picks up the other's branches.
//! - Init: the settings, and the development branch at the production one when it's missing.
//!   Journaled (MoveRefs, with the config).
//! - Start is an ordinary create-and-checkout (`CreateBranch`) from the base branch: the UI's.
//! - Finish: `merge --no-ff` into each target in turn (a feature into development; a release or
//!   hotfix into production, then development), a tag on production for a release or hotfix,
//!   then the branch's delete. It ends on the development branch wherever it started, as `git
//!   flow` does. One write, one Undo (Switch: back to where HEAD was).
//!   A merge that would conflict isn't started: the finish stops before it, having done what
//!   came first, and says which merge is left. Once the user has made that merge, Finish again
//!   skips every target the branch is already in.

use crate::error::{gix_err, GbError, GbErrorKind};
use crate::events::{ChangeKind, OpKind};
use crate::git::{GitCli, GitInvocation};
use crate::journal::{RefMove, UndoKind};
use crate::write::config::{self, quiet_exit, BranchConfig};
use crate::write::names::{branch_name_error, tag_name_error};
use crate::write::precheck::display_worktree;
use crate::write::refs::read_ref;
use crate::write::{is_ancestor, Plan, Pre, Staging, WriteCx, WriteIntent};
use serde::{Deserialize, Serialize};
use gix::bstr::ByteSlice;
use std::path::Path;
use std::sync::OnceLock;
use ts_rs::TS;

/// A repository's git flow settings (`gitflow.branch.*`, `gitflow.prefix.*`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitflowConfig {
    /// The production branch (`gitflow.branch.master`).
    pub master: String,
    /// The development branch (`gitflow.branch.develop`).
    pub develop: String,
    pub feature: String,
    pub release: String,
    pub hotfix: String,
    /// Put before a release's or hotfix's version to name its tag (`v`: `v1.2.0`); may be empty.
    pub version_tag: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum GitflowKind {
    Feature,
    Release,
    Hotfix,
}

impl GitflowConfig {
    /// The kind of a branch (short name) and what follows its prefix (a release's version), when
    /// it's a git flow branch.
    pub(crate) fn classify<'b>(&self, branch: &'b str) -> Option<(GitflowKind, &'b str)> {
        [(GitflowKind::Feature, &self.feature), (GitflowKind::Release, &self.release), (GitflowKind::Hotfix, &self.hotfix)]
            .into_iter()
            .find_map(|(kind, prefix)| branch.strip_prefix(prefix.as_str()).filter(|rest| !rest.is_empty()).map(|rest| (kind, rest)))
    }

    /// The branches a finish merges into, in order.
    fn targets(&self, kind: GitflowKind) -> Vec<&str> {
        match kind {
            GitflowKind::Feature => vec![&self.develop],
            GitflowKind::Release | GitflowKind::Hotfix => vec![&self.master, &self.develop],
        }
    }

    /// The keys Init writes, with their values.
    fn keys(&self) -> [(&'static str, &str); 6] {
        [
            ("gitflow.branch.master", &self.master),
            ("gitflow.branch.develop", &self.develop),
            ("gitflow.prefix.feature", &self.feature),
            ("gitflow.prefix.release", &self.release),
            ("gitflow.prefix.hotfix", &self.hotfix),
            ("gitflow.prefix.versiontag", &self.version_tag),
        ]
    }

    /// Why these settings can't be used, for the dialog.
    pub(crate) fn error(&self) -> Option<String> {
        for (what, name) in [("production", &self.master), ("development", &self.develop)] {
            if let Some(why) = branch_name_error(name) {
                return Some(format!("The {what} branch: {why}"));
            }
        }
        if self.master == self.develop {
            return Some("The production and development branches must differ".into());
        }
        let prefixes = [("feature", &self.feature), ("release", &self.release), ("hotfix", &self.hotfix)];
        for (i, (what, prefix)) in prefixes.iter().enumerate() {
            if prefix.is_empty() {
                return Some(format!("Enter a {what} prefix"));
            }
            if let Some(why) = branch_name_error(&format!("{prefix}x")) {
                return Some(format!("The {what} prefix: {why}"));
            }
            if let Some((other, _)) = prefixes[..i].iter().find(|(_, p)| p.starts_with(prefix.as_str()) || prefix.starts_with(p.as_str())) {
                return Some(format!("The {what} and {other} prefixes overlap"));
            }
            if let Some(b) = [&self.master, &self.develop].into_iter().find(|b| b.starts_with(prefix.as_str())) {
                return Some(format!("The {what} prefix would make {b} a {what} branch"));
            }
        }
        if !self.version_tag.is_empty()
            && let Some(why) = tag_name_error(&format!("{}1.0", self.version_tag))
        {
            return Some(format!("The version tag prefix: {why}"));
        }
        None
    }
}

fn heads(name: &str) -> String {
    format!("refs/heads/{name}")
}

/// `gitflow.*` as `git config --null --get-regexp` lists it (`key\nvalue\0`), each key's values
/// in config order.
fn parse_entries(raw: &[u8]) -> BranchConfig {
    let mut map = BranchConfig::new();
    for rec in raw.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let rec = String::from_utf8_lossy(rec);
        let (key, value) = rec.split_once('\n').unwrap_or((rec.as_ref(), "true"));
        if key.starts_with("gitflow.") {
            map.entry(key.to_string()).or_default().push(value.to_string());
        }
    }
    map
}

/// The repository's own `gitflow.*`: all Init writes, and so all its journal entry records.
async fn entries(cli: &GitCli, root: &Path) -> Result<BranchConfig, GbError> {
    match cli.run(GitInvocation::new(root, ["config", "--local", "--null", "--get-regexp", r"^gitflow\."])).await {
        Ok(out) => Ok(parse_entries(&out.stdout)),
        Err(e) if quiet_exit(&e) => Ok(BranchConfig::new()),
        Err(e) => Err(e),
    }
}

/// The repository's git flow settings, or `None` until it's set up here: its own config names
/// the production and development branches (a `git flow init --global` names them for every
/// repository) and both exist, as `git flow` requires. A prefix that isn't set has `git flow`'s
/// default. In process, from the config file as it is now: the sidebar reads it on every refresh.
pub(crate) fn read(repo: &gix::Repository) -> Option<GitflowConfig> {
    let c = from_config(&crate::shelldata::fresh_config(repo))?;
    let exists = |b: &str| read_ref(repo, &heads(b)).ok().flatten().is_some();
    (exists(&c.master) && exists(&c.develop)).then_some(c)
}

/// `read`'s settings, from the repository's own config file only (`fresh_config` falls back on
/// the handle's snapshot, which has the global and system files too).
fn from_config(config: &gix::config::File) -> Option<GitflowConfig> {
    let one = |k: &str| config.string_filter(k, |m| m.source == gix::config::Source::Local).map(|v| v.to_str_lossy().into_owned());
    let (master, develop) = (one("gitflow.branch.master")?, one("gitflow.branch.develop")?);
    let or = |k: &str, default: &str| one(k).unwrap_or_else(|| default.to_string());
    Some(GitflowConfig {
        master,
        develop,
        feature: or("gitflow.prefix.feature", "feature/"),
        release: or("gitflow.prefix.release", "release/"),
        hotfix: or("gitflow.prefix.hotfix", "hotfix/"),
        version_tag: or("gitflow.prefix.versiontag", ""),
    })
}

/// The tag an earlier finish of a release or hotfix put on production, which a finish again
/// keeps: one on a commit of `master` that holds `branch`, the nearest. `None`: there's none, or
/// `branch` isn't a release or hotfix.
pub(crate) fn finished_tag(repo: &gix::Repository, config: &GitflowConfig, branch: &str) -> Result<Option<String>, GbError> {
    if !matches!(config.classify(branch), Some((GitflowKind::Release | GitflowKind::Hotfix, _))) {
        return Ok(None);
    }
    let tip = |name: &str| -> Result<Option<gix::ObjectId>, GbError> { read_ref(repo, &heads(name))?.map(|o| gix::ObjectId::from_hex(o.as_bytes()).map_err(gix_err)).transpose() };
    let (Some(tip), Some(master)) = (tip(branch)?, tip(&config.master)?) else { return Ok(None) };
    holding_tag(repo, tip, master)
}

/// The tag nearest `tip` among those on a commit of `master`'s that holds `tip`. Only the commits
/// of `master` that `tip` doesn't have are walked: those that hold it descend from it in there.
fn holding_tag(repo: &gix::Repository, tip: gix::ObjectId, master: gix::ObjectId) -> Result<Option<String>, GbError> {
    let mut children: std::collections::HashMap<gix::ObjectId, Vec<gix::ObjectId>> = std::collections::HashMap::new();
    for info in repo.rev_walk([master]).with_hidden([tip]).all().map_err(gix_err)? {
        let info = info.map_err(gix_err)?;
        for p in info.parent_ids {
            children.entry(p).or_default().push(info.id);
        }
    }
    // Breadth first from `tip`: how far each commit that holds it is.
    let mut depth = std::collections::HashMap::new();
    let mut queue = std::collections::VecDeque::from([(tip, 0u32)]);
    while let Some((id, d)) = queue.pop_front() {
        for &child in children.get(&id).into_iter().flatten() {
            if let std::collections::hash_map::Entry::Vacant(e) = depth.entry(child) {
                e.insert(d + 1);
                queue.push_back((child, d + 1));
            }
        }
    }
    if depth.is_empty() {
        return Ok(None);
    }
    let mut best: Option<(u32, String)> = None;
    for r in repo.references().map_err(gix_err)?.tags().map_err(gix_err)? {
        let Ok(mut r) = r else { continue };
        let Ok(id) = r.peel_to_id() else { continue };
        let Some(&d) = depth.get(&id.detach()) else { continue };
        let name = r.name().shorten().to_str_lossy().into_owned();
        if best.as_ref().is_none_or(|(bd, bn)| (d, &name) < (*bd, bn)) {
            best = Some((d, name));
        }
    }
    Ok(best.map(|(_, name)| name))
}

pub(crate) struct GitflowInit {
    pub config: GitflowConfig,
}

impl WriteIntent for GitflowInit {
    type Outcome = ();
    fn kind(&self) -> OpKind {
        OpKind::Branch
    }
    fn label(&self) -> String {
        "initialize git flow".into()
    }
    fn undo(&self) -> Option<UndoKind> {
        Some(UndoKind::MoveRefs)
    }
    fn staging(&self) -> Staging {
        Staging::Keep
    }
    fn refs(&self) -> Vec<String> {
        if self.config.error().is_some() {
            return Vec::new();
        }
        vec![heads(&self.config.master), heads(&self.config.develop)]
    }
    async fn plan(&self, pre: &Pre<'_>) -> Result<Plan, GbError> {
        if let Some(why) = self.config.error() {
            return Err(GbError::new(GbErrorKind::InvalidInput, why));
        }
        if pre.before.refs.get(&heads(&self.config.master)).cloned().flatten().is_none() {
            return Err(GbError::new(GbErrorKind::NotFound, format!("There's no branch {}: create it first", self.config.master)));
        }
        Ok(Plan::default())
    }
    async fn run(&self, cx: &mut WriteCx<'_>) -> Result<(), GbError> {
        let c = &self.config;
        if cx.before.refs.get(&heads(&c.develop)).cloned().flatten().is_none() {
            let start = cx.before.refs.get(&heads(&c.master)).cloned().flatten();
            cx.cas(&[RefMove { name: heads(&c.develop), old: None, new: start }], &format!("branch: Created from {}", c.master)).await?;
            cx.touch(ChangeKind::Refs);
        }
        let before = entries(&cx.api.cli, cx.root).await?;
        for (key, value) in c.keys() {
            if before.get(key).is_some_and(|v| v.len() == 1 && v[0] == value) {
                continue;
            }
            let inv = cx.git(["config", "--local", "--replace-all", key, value]);
            cx.run_git(inv).await?;
        }
        let after = entries(&cx.api.cli, cx.root).await?;
        let changes = config::changes(&before, &after);
        if !changes.is_empty() {
            cx.touch(ChangeKind::Config);
        }
        cx.record_config(changes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum GitflowFinishOutcome {
    /// Merged into `merged`, in order (a target that already had the branch isn't listed),
    /// tagged `tag` (one it made), and the branch deleted unless it was kept.
    Done { merged: Vec<String>, tag: Option<String>, deleted: bool },
    /// Stopped before the merge into `into`, which would conflict in `files`; `merged` and `tag`
    /// are what it did before that. The branch stays.
    Conflicts { into: String, files: Vec<String>, merged: Vec<String>, tag: Option<String> },
}

/// What `plan` decided, for `run`.
#[derive(Debug, Default)]
struct Steps {
    /// Each target, in order, with whether it merges (`false`: the branch is already in it).
    targets: Vec<(String, bool)>,
    /// The merge it stops before: the target, and its conflicted paths.
    stop: Option<(String, Vec<String>)>,
    /// The tag it makes on the production branch, right after that branch's merge.
    tag: Option<String>,
    delete: bool,
}

pub(crate) struct GitflowFinish {
    /// Short name (`feature/login`).
    pub branch: String,
    /// The settings as the request found them (`read`).
    pub config: GitflowConfig,
    /// A release's or hotfix's tag name, as the user chose it; `None`: the version tag prefix
    /// and what follows the branch's prefix (`git flow`'s).
    pub tag: Option<String>,
    /// A release's or hotfix's tag: annotated with this message, else lightweight.
    pub tag_message: Option<String>,
    /// Keep the branch once it's finished.
    pub keep: bool,
    steps: OnceLock<Steps>,
}

impl GitflowFinish {
    pub(crate) fn new(branch: String, config: GitflowConfig, tag: Option<String>, tag_message: Option<String>, keep: bool) -> Self {
        Self { branch, config, tag, tag_message, keep, steps: OnceLock::new() }
    }

    fn kind(&self) -> Result<(GitflowKind, &str), GbError> {
        self.config.classify(&self.branch).ok_or_else(|| GbError::new(GbErrorKind::InvalidInput, format!("{} isn't a git flow feature, release or hotfix branch", self.branch)))
    }

    fn tag_name(&self) -> Option<String> {
        match self.kind().ok()? {
            (GitflowKind::Feature, _) => None,
            (_, version) => Some(self.tag.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| format!("{}{version}", self.config.version_tag))),
        }
    }
}

impl WriteIntent for GitflowFinish {
    type Outcome = GitflowFinishOutcome;
    fn kind(&self) -> OpKind {
        OpKind::Merge
    }
    fn label(&self) -> String {
        format!("finish {}", self.branch)
    }
    /// A finish that completes ends on the development branch: Undo switches back to where it
    /// started (or rewinds that branch, when it started there), then puts every ref back.
    fn undo(&self) -> Option<UndoKind> {
        Some(UndoKind::Switch)
    }
    /// Merges run their hooks and may sign; so may an annotated tag; switches run `post-checkout`.
    fn runs_hooks(&self) -> bool {
        true
    }
    fn refs(&self) -> Vec<String> {
        let Ok((kind, _)) = self.kind() else { return Vec::new() };
        if branch_name_error(&self.branch).is_some() || self.config.error().is_some() {
            return Vec::new();
        }
        let mut refs = vec![heads(&self.branch)];
        refs.extend(self.config.targets(kind).into_iter().map(heads));
        refs.extend(self.tag_name().filter(|t| tag_name_error(t).is_none()).map(|t| format!("refs/tags/{t}")));
        refs
    }
    async fn plan(&self, pre: &Pre<'_>) -> Result<Plan, GbError> {
        let (kind, _) = self.kind()?;
        if let Some(why) = self.config.error() {
            return Err(GbError::new(GbErrorKind::InvalidInput, why));
        }
        let tip = pre.before.refs.get(&heads(&self.branch)).cloned().flatten().ok_or_else(|| GbError::new(GbErrorKind::NotFound, format!("No branch {}", self.branch)))?;
        let oid = |o: &str| gix::ObjectId::from_hex(o.as_bytes()).map_err(gix_err);
        let tip_id = oid(&tip)?;
        // Every merge runs in this worktree: a target some other one has checked out can't be.
        // Nor can the branch be deleted there; kept, it's only read.
        let targets = self.config.targets(kind);
        let worktrees = crate::worktree::list_worktrees(pre.root).await?;
        for name in std::iter::once(self.branch.as_str()).filter(|_| !self.keep).chain(targets.iter().copied()) {
            if let Some(w) = worktrees.iter().find(|w| w.branch.as_deref() == Some(heads(name).as_str()))
                && crate::platform::fs::canonicalize(&w.path).unwrap_or(w.path.clone()) != pre.root
            {
                let main = worktrees.iter().find(|m| m.is_main).map(|m| m.path.clone()).unwrap_or_else(|| pre.h.workdir.clone());
                return Err(GbError::checked_out_elsewhere(name, &display_worktree(&main, &w.path)));
            }
        }
        // Each step switches branches: the changes would have to travel through every merge.
        let dirty = crate::write::precheck::dirty(&pre.api.cli, pre.root).await?;
        if !dirty.tracked.is_empty() {
            return Err(GbError::new(GbErrorKind::InvalidInput, format!("Commit or stash your changes before finishing {}", self.branch)));
        }
        let repo = gix::open(pre.root).map_err(gix_err)?;
        let mut steps = Steps { delete: !self.keep, ..Steps::default() };
        for target in &targets {
            let at = pre.before.refs.get(&heads(target)).cloned().flatten().ok_or_else(|| GbError::new(GbErrorKind::NotFound, format!("No branch {target}")))?;
            let at = oid(&at)?;
            if is_ancestor(&repo, tip_id, at) {
                steps.targets.push((target.to_string(), false));
                continue;
            }
            let files = crate::write::integrate::predicted_conflicts(&repo, at, tip_id)?;
            if !files.is_empty() {
                steps.stop = Some((target.to_string(), files));
                steps.delete = false;
                break;
            }
            steps.targets.push((target.to_string(), true));
        }
        // The tag goes on production once the branch is in it. When production has it already
        // (an earlier finish stopped before development), a tag there that holds the branch is
        // that finish's, kept whatever name this one was given; so is a tag of this name that
        // holds it.
        let on_master = steps.targets.iter().any(|(t, _)| *t == self.config.master);
        let merges_master = steps.targets.iter().any(|(t, merges)| *t == self.config.master && *merges);
        let tagged = on_master && !merges_master && {
            let at = pre.before.refs.get(&heads(&self.config.master)).cloned().flatten().ok_or_else(|| GbError::new(GbErrorKind::NotFound, format!("No branch {}", self.config.master)))?;
            holding_tag(&repo, tip_id, oid(&at)?)?.is_some()
        };
        if let Some(name) = self.tag_name().filter(|_| on_master && !tagged) {
            if let Some(why) = tag_name_error(&name) {
                return Err(GbError::new(GbErrorKind::InvalidInput, format!("The tag {name}: {why}")));
            }
            let held = match read_ref(&repo, &format!("refs/tags/{name}"))? {
                None => false,
                Some(_) => {
                    let tagged = repo.rev_parse_single(format!("refs/tags/{name}^{{commit}}").as_str()).map_err(gix_err)?.detach();
                    if !is_ancestor(&repo, tip_id, tagged) {
                        return Err(GbError::new(GbErrorKind::InvalidInput, format!("A tag named {name} already exists")));
                    }
                    true
                }
            };
            if !held {
                if self.tag_message.as_deref().is_some_and(|m| m.trim().is_empty()) {
                    return Err(GbError::new(GbErrorKind::InvalidInput, "Enter a tag message"));
                }
                steps.tag = Some(name);
            }
        }
        // An untracked file one of its checkouts would write over: git would refuse part-way,
        // so it's refused here, before anything runs. What it checks out: each target it merges
        // into, then the merge (which brings the branch's files), and development, where it ends.
        let merges: Vec<&str> = steps.targets.iter().filter(|(_, m)| *m).map(|(t, _)| t.as_str()).collect();
        let mut commits = if merges.is_empty() { Vec::new() } else { vec![tip_id] };
        let ends = steps.stop.is_none().then_some(self.config.develop.as_str());
        for name in merges.into_iter().chain(ends) {
            commits.extend(pre.before.refs.get(&heads(name)).cloned().flatten().map(|o| oid(&o)).transpose()?);
        }
        let blocked = in_the_way(&repo, &commits, &dirty.untracked)?;
        if !blocked.is_empty() {
            let shown = blocked.iter().take(3).map(String::as_str).collect::<Vec<_>>().join(", ");
            let more = if blocked.len() > 3 { format!(" and {} more", blocked.len() - 3) } else { String::new() };
            let it = if blocked.len() == 1 { "it" } else { "them" };
            return Err(GbError::new(GbErrorKind::DirtyWorktree, format!("Move or delete {shown}{more} before finishing {}: it would overwrite {it}", self.branch)));
        }
        let _ = self.steps.set(steps);
        Ok(Plan::default())
    }
    async fn run(&self, cx: &mut WriteCx<'_>) -> Result<GitflowFinishOutcome, GbError> {
        let steps = self.steps.get().ok_or_else(|| GbError::other("the finish wasn't planned"))?;
        let mut on = cx.before.head.branch.clone();
        let mut merged = Vec::new();
        let mut tag = None;
        for (target, merges) in &steps.targets {
            if *merges {
                switch_to(cx, &mut on, target).await?;
                let inv = cx.git(["merge", "--no-ff", "--no-edit", self.branch.as_str()]);
                let res = cx.run_git(inv).await;
                for k in [ChangeKind::Worktree, ChangeKind::Index, ChangeKind::Refs, ChangeKind::State] {
                    cx.touch(k);
                }
                if let Err(e) = res {
                    // Predicted clean, but git stopped: it's undone. On conflicts (a merge driver,
                    // a rename it judged otherwise) the finish stops as if they'd been predicted;
                    // anything else (a hook's refusal) is the merge's error.
                    let files = abort_merge(cx).await?;
                    if files.is_empty() {
                        return Err(e);
                    }
                    return Ok(GitflowFinishOutcome::Conflicts { into: target.clone(), files, merged, tag });
                }
                merged.push(target.clone());
            }
            if *target == self.config.master
                && let Some(name) = &steps.tag
            {
                make_tag(cx, name, &self.config.master, self.tag_message.as_deref()).await?;
                tag = Some(name.clone());
            }
        }
        if let Some((into, files)) = &steps.stop {
            return Ok(GitflowFinishOutcome::Conflicts { into: into.clone(), files: files.clone(), merged, tag });
        }
        // Done: it ends on the development branch, as `git flow` does, even when no merge took
        // it there (a release finished from production that development already had). Off the
        // branch before its delete; in this write, so its Undo puts HEAD back too.
        switch_to(cx, &mut on, &self.config.develop).await?;
        if steps.delete {
            let old = cx.before.refs.get(&heads(&self.branch)).cloned().flatten();
            let changes = crate::write::branch_delete::delete_local_branch(cx, &self.branch, old).await?;
            cx.record_config(changes)?;
        }
        Ok(GitflowFinishOutcome::Done { merged, tag, deleted: steps.delete })
    }
}

/// `git switch` to `target` unless HEAD is on it; a failed or cancelled one puts back the files
/// it rewrote, as a checkout's does.
async fn switch_to(cx: &mut WriteCx<'_>, on: &mut Option<String>, target: &str) -> Result<(), GbError> {
    if on.as_deref() == Some(target) {
        return Ok(());
    }
    let to = read_ref(&gix::open(cx.root).map_err(gix_err)?, &heads(target))?.ok_or_else(|| GbError::new(GbErrorKind::NotFound, format!("No branch {target}")))?;
    let inv = cx.git(["switch", "--no-guess", target]);
    let res = cx.run_git(inv).await;
    for k in [ChangeKind::Head, ChangeKind::Index, ChangeKind::Worktree] {
        cx.touch(k);
    }
    crate::write::checkout::failed_move(cx, res, &to).await?;
    *on = Some(target.to_string());
    Ok(())
}

/// The untracked files among `untracked` that checking out one of `commits` would write over:
/// it has the file, or a file where one of its directories is.
fn in_the_way(repo: &gix::Repository, commits: &[gix::ObjectId], untracked: &std::collections::BTreeSet<String>) -> Result<Vec<String>, GbError> {
    if untracked.is_empty() {
        return Ok(Vec::new());
    }
    let trees = commits.iter().map(|c| repo.find_commit(*c).map_err(gix_err)?.tree().map_err(gix_err)).collect::<Result<Vec<_>, _>>()?;
    Ok(untracked
        .iter()
        .filter(|path| {
            let dirs: Vec<&str> = path.match_indices('/').map(|(i, _)| &path[..i]).collect();
            trees.iter().any(|tree| {
                tree.lookup_entry_by_path(path.as_str()).ok().flatten().is_some() || dirs.iter().any(|d| tree.lookup_entry_by_path(*d).ok().flatten().is_some_and(|e| !e.mode().is_tree()))
            })
        })
        .cloned()
        .collect())
}

/// A merge git stopped: its conflicted paths (none: it stopped for another reason), once `git
/// merge --abort` has put the branch back. Nothing to abort when it never started.
async fn abort_merge(cx: &mut WriteCx<'_>) -> Result<Vec<String>, GbError> {
    if !matches!(crate::in_progress::read(cx.root).ok().flatten(), Some(crate::in_progress::InProgress::Merge { .. })) {
        return Ok(Vec::new());
    }
    let files = crate::write::precheck::dirty(&cx.api.cli, cx.root).await?.unmerged;
    let inv = cx.git(["merge", "--abort"]);
    cx.run_git(inv).await?;
    Ok(files)
}

/// The release's or hotfix's tag on `branch`'s tip: annotated (`git tag -a`, which signs per
/// `tag.gpgSign`) with a message, else a lightweight CAS.
async fn make_tag(cx: &mut WriteCx<'_>, name: &str, branch: &str, message: Option<&str>) -> Result<(), GbError> {
    let at = read_ref(&gix::open(cx.root).map_err(gix_err)?, &heads(branch))?.ok_or_else(|| GbError::new(GbErrorKind::NotFound, format!("No branch {branch}")))?;
    match message {
        None => cx.cas(&[RefMove { name: format!("refs/tags/{name}"), old: None, new: Some(at) }], &format!("tag: {name}")).await?,
        Some(m) => {
            let inv = cx.git(["tag", "-a", "-F", "-", "--", name, at.as_str()]).stdin(format!("{}\n", m.trim_end()).into_bytes());
            cx.run_git(inv).await?;
        }
    }
    cx.touch(ChangeKind::Refs);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::state::RepoState;
    use crate::testing::write::{identity, open, send, wt, WriteEnv};
    use crate::testing::TestRepo;
    use serde_json::json;

    fn config() -> GitflowConfig {
        GitflowConfig { master: "main".into(), develop: "develop".into(), feature: "feature/".into(), release: "release/".into(), hotfix: "hotfix/".into(), version_tag: "v".into() }
    }

    /// A repository with git flow set up: `main`, and `develop` one commit ahead of it.
    fn repo() -> TestRepo {
        let r = TestRepo::new();
        identity(&r);
        r.commit("one");
        r.switch_new("develop");
        r.commit("dev");
        for (k, v) in config().keys() {
            r.git(&["config", k, v]);
        }
        r
    }

    fn oid(r: &TestRepo, rev: &str) -> String {
        r.git(&["rev-parse", rev])
    }

    fn exists(r: &TestRepo, name: &str) -> bool {
        r.try_git(&["rev-parse", "--verify", "-q", name]).is_ok()
    }

    async fn undo_top(api: &crate::api::Api, id: u32, r: &TestRepo) -> serde_json::Value {
        let s = send(api, json!({"method": "journalState", "params": {"repo": id, "worktree": wt(r)}})).await.unwrap();
        send(api, json!({"method": "undo", "params": {"repo": id, "worktree": wt(r), "entry": s["undo"]["entry"]}})).await.unwrap()
    }

    async fn redo_top(api: &crate::api::Api, id: u32, r: &TestRepo) -> serde_json::Value {
        let s = send(api, json!({"method": "journalState", "params": {"repo": id, "worktree": wt(r)}})).await.unwrap();
        send(api, json!({"method": "redo", "params": {"repo": id, "worktree": wt(r), "entry": s["redo"]["entry"]}})).await.unwrap()
    }

    fn finish(id: u32, r: &TestRepo, branch: &str, message: Option<&str>) -> serde_json::Value {
        json!({"method": "gitflowFinish", "params": {"repo": id, "worktree": wt(r), "branch": branch, "tagMessage": message, "keep": false}})
    }

    #[test]
    fn branches_are_classified_by_their_prefix() {
        let c = config();
        assert_eq!(c.classify("feature/login"), Some((GitflowKind::Feature, "login")));
        assert_eq!(c.classify("release/1.2.0"), Some((GitflowKind::Release, "1.2.0")));
        assert_eq!(c.classify("hotfix/1.2.1"), Some((GitflowKind::Hotfix, "1.2.1")));
        assert_eq!(c.classify("feature/"), None, "a bare prefix names no branch");
        assert_eq!(c.classify("develop"), None);
        assert_eq!(c.classify("features/x"), None);
    }

    #[test]
    fn settings_that_cant_work_say_why() {
        assert_eq!(config().error(), None);
        let with = |f: fn(&mut GitflowConfig)| {
            let mut c = config();
            f(&mut c);
            c.error()
        };
        assert_eq!(with(|c| c.develop = "main".into()).as_deref(), Some("The production and development branches must differ"));
        assert_eq!(with(|c| c.feature = String::new()).as_deref(), Some("Enter a feature prefix"));
        assert_eq!(with(|c| c.hotfix = "feature/".into()).as_deref(), Some("The hotfix and feature prefixes overlap"));
        assert_eq!(with(|c| c.feature = "dev".into()).as_deref(), Some("The feature prefix would make develop a feature branch"));
        assert!(with(|c| c.release = "rel ease/".into()).unwrap().starts_with("The release prefix: "));
        assert!(with(|c| c.version_tag = "v..".into()).unwrap().starts_with("The version tag prefix: "));
    }

    #[test]
    fn only_gitflow_keys_are_parsed() {
        let raw = b"gitflow.branch.master\nmain\0gitflow.prefix.versiontag\n\0other.key\nx\0";
        let e = parse_entries(raw);
        assert_eq!(e.len(), 2);
        assert_eq!(e["gitflow.prefix.versiontag"], [""], "an empty value is a value");
    }

    #[test]
    fn read_is_none_before_init_and_fills_in_default_prefixes() {
        let r = TestRepo::new();
        r.commit("one");
        r.git(&["branch", "dev"]);
        assert_eq!(read(&gix::open(r.path()).unwrap()), None);
        r.git(&["config", "gitflow.branch.master", "main"]);
        r.git(&["config", "gitflow.branch.develop", "dev"]);
        r.git(&["config", "gitflow.prefix.feature", "feat/"]);
        let c = read(&gix::open(r.path()).unwrap()).unwrap();
        assert_eq!((c.develop.as_str(), c.feature.as_str(), c.release.as_str(), c.version_tag.as_str()), ("dev", "feat/", "release/", ""));
    }

    /// `git flow` refuses a repository whose production or development branch is missing.
    #[test]
    fn it_isnt_set_up_until_both_branches_exist() {
        let r = TestRepo::new();
        r.commit("one");
        r.git(&["config", "gitflow.branch.master", "main"]);
        r.git(&["config", "gitflow.branch.develop", "develop"]);
        assert_eq!(read(&gix::open(r.path()).unwrap()), None);
        r.git(&["branch", "develop"]);
        assert_eq!(read(&gix::open(r.path()).unwrap()).map(|c| c.develop), Some("develop".into()));
    }

    /// `git flow init --global` would otherwise light the panel up in every repository.
    #[test]
    fn settings_from_the_global_config_dont_set_it_up() {
        let file = |text: &str, source: gix::config::Source| gix::config::File::from_bytes_owned(&mut text.as_bytes().to_vec(), gix::config::file::Metadata::from(source), Default::default()).unwrap();
        let global = "[gitflow \"branch\"]\n\tmaster = main\n\tdevelop = develop\n[gitflow \"prefix\"]\n\tfeature = f/\n";
        assert_eq!(from_config(&file(global, gix::config::Source::User)), None);
        let mut both = file(global, gix::config::Source::User);
        both.append(file("[gitflow \"branch\"]\n\tmaster = prod\n\tdevelop = dev\n", gix::config::Source::Local)).unwrap();
        let c = from_config(&both).unwrap();
        assert_eq!((c.master.as_str(), c.develop.as_str(), c.feature.as_str()), ("prod", "dev", "feature/"), "only the repository's own keys count");
    }

    /// The sidebar reads the settings on every refresh: in process, not a `git config` that
    /// would fill the Activity log.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_sidebar_reads_the_settings_without_a_git_process() {
        let env = WriteEnv::new();
        let r = repo();
        let id = open(&env.api, &r).await;
        let before = env.api.cli.log().entries().len();
        let s = send(&env.api, json!({"method": "sidebar", "params": {"repo": id}})).await.unwrap();
        assert_eq!(s["gitflow"]["develop"], "develop");
        let ran: Vec<_> = env.api.cli.log().entries()[before..].iter().map(|e| e.args.join(" ")).collect();
        assert!(ran.is_empty(), "{ran:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn init_creates_develop_and_writes_the_settings_and_undoes_both() {
        let env = WriteEnv::new();
        let r = TestRepo::new();
        identity(&r);
        r.commit("one");
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        send(&env.api, json!({"method": "gitflowInit", "params": {"repo": id, "worktree": wt(&r), "config": config()}})).await.unwrap();
        assert_eq!(oid(&r, "develop"), oid(&r, "main"));
        assert_eq!(r.git(&["config", "gitflow.prefix.versiontag"]), "v");
        assert_eq!(r.git(&["branch", "--show-current"]), "main", "Init doesn't check develop out");
        undo_top(&env.api, id, &r).await;
        assert!(!exists(&r, "refs/heads/develop"));
        assert!(r.try_git(&["config", "gitflow.branch.master"]).is_err());
        assert_eq!(RepoState::capture(&r).refs, before.refs);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn init_refuses_a_production_branch_that_doesnt_exist() {
        let env = WriteEnv::new();
        let r = TestRepo::new();
        identity(&r);
        r.commit("one");
        let id = open(&env.api, &r).await;
        let c = GitflowConfig { master: "master".into(), ..config() };
        let e = send(&env.api, json!({"method": "gitflowInit", "params": {"repo": id, "worktree": wt(&r), "config": c}})).await.unwrap_err();
        assert_eq!(e.message, "There's no branch master: create it first");
        assert!(!exists(&r, "refs/heads/develop"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_feature_merges_into_develop_and_goes_and_round_trips() {
        let env = WriteEnv::new();
        let r = repo();
        r.switch_new("feature/login");
        r.commit("f1");
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        let (dev, feat) = (oid(&r, "develop"), oid(&r, "feature/login"));
        let out = send(&env.api, finish(id, &r, "feature/login", None)).await.unwrap();
        assert_eq!(out["outcome"], json!({"status": "done", "merged": ["develop"], "tag": null, "deleted": true}));
        assert_eq!(r.git(&["branch", "--show-current"]), "develop");
        assert_eq!(r.git(&["rev-parse", "develop^1", "develop^2"]), format!("{dev}\n{feat}"), "a merge commit, even where a fast-forward would do");
        assert_eq!(r.git(&["log", "-1", "--format=%s", "develop"]), "Merge branch 'feature/login' into develop");
        assert!(!exists(&r, "refs/heads/feature/login"));
        undo_top(&env.api, id, &r).await;
        let undone = RepoState::capture(&r);
        assert_eq!((undone.refs.clone(), r.git(&["branch", "--show-current"])), (before.refs.clone(), "feature/login".to_string()));
        redo_top(&env.api, id, &r).await;
        assert!(!exists(&r, "refs/heads/feature/login"));
        assert_eq!((oid(&r, "develop^2"), r.git(&["branch", "--show-current"])), (feat, "develop".to_string()));
        assert_eq!(r.git(&["status", "--porcelain"]), "", "the worktree follows every move");
    }

    /// Started on the development branch itself, Undo rewinds it rather than switching.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_feature_finished_from_develop_undoes_onto_develop() {
        let env = WriteEnv::new();
        let r = repo();
        r.switch_new("feature/login");
        r.commit("f1");
        r.switch("develop");
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        send(&env.api, finish(id, &r, "feature/login", None)).await.unwrap();
        undo_top(&env.api, id, &r).await;
        assert_eq!(RepoState::capture(&r).refs, before.refs);
        assert_eq!(r.git(&["branch", "--show-current"]), "develop");
        assert_eq!(r.git(&["status", "--porcelain"]), "");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_release_merges_into_main_tags_it_then_develop_and_round_trips() {
        let env = WriteEnv::new();
        let r = repo();
        r.switch_new("release/1.2.0");
        r.commit("bump");
        r.switch("main");
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        let rel = oid(&r, "release/1.2.0");
        let out = send(&env.api, finish(id, &r, "release/1.2.0", Some("Release 1.2.0"))).await.unwrap();
        assert_eq!(out["outcome"], json!({"status": "done", "merged": ["main", "develop"], "tag": "v1.2.0", "deleted": true}));
        assert_eq!(r.git(&["rev-parse", "v1.2.0^{commit}"]), oid(&r, "main"));
        assert_eq!(r.git(&["for-each-ref", "--format=%(contents:subject)", "refs/tags/v1.2.0"]), "Release 1.2.0");
        assert_eq!((oid(&r, "main^2"), oid(&r, "develop^2")), (rel.clone(), rel));
        assert_eq!(r.git(&["branch", "--show-current"]), "develop");
        // It started on main, which it moved: Undo puts main back before switching onto it.
        undo_top(&env.api, id, &r).await;
        assert_eq!(RepoState::capture(&r).refs, before.refs);
        assert_eq!(r.git(&["branch", "--show-current"]), "main");
        assert_eq!(r.git(&["status", "--porcelain"]), "");
        redo_top(&env.api, id, &r).await;
        assert!(exists(&r, "refs/tags/v1.2.0"));
        assert_eq!(r.git(&["branch", "--show-current"]), "develop");
        assert_eq!(r.git(&["status", "--porcelain"]), "");
    }

    /// No merge takes it to develop (develop has the release already): it switches there anyway,
    /// as `git flow` does, and Undo goes back to main.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_release_develop_already_has_finished_from_main_ends_on_develop_and_round_trips() {
        let env = WriteEnv::new();
        let r = repo();
        r.switch_new("release/1.2.0");
        r.commit("bump");
        r.switch("develop");
        r.git(&["merge", "-q", "--no-ff", "--no-edit", "release/1.2.0"]);
        r.switch("main");
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        let (dev, rel) = (oid(&r, "develop"), oid(&r, "release/1.2.0"));
        let out = send(&env.api, finish(id, &r, "release/1.2.0", Some("Release 1.2.0"))).await.unwrap();
        assert_eq!(out["outcome"], json!({"status": "done", "merged": ["main"], "tag": "v1.2.0", "deleted": true}));
        assert_eq!(r.git(&["branch", "--show-current"]), "develop");
        assert_eq!((oid(&r, "develop"), oid(&r, "main^2")), (dev, rel));
        assert_eq!(r.git(&["status", "--porcelain"]), "");
        let after = RepoState::capture(&r);
        undo_top(&env.api, id, &r).await;
        assert_eq!(RepoState::capture(&r).refs, before.refs);
        assert_eq!(r.git(&["branch", "--show-current"]), "main");
        assert_eq!(r.git(&["status", "--porcelain"]), "");
        redo_top(&env.api, id, &r).await;
        assert_eq!(RepoState::capture(&r).refs, after.refs);
        assert_eq!(r.git(&["branch", "--show-current"]), "develop");
        assert_eq!(r.git(&["status", "--porcelain"]), "");
    }

    /// Nothing to merge, only a delete: it still ends on develop, and Undo goes back to the
    /// branch it started on.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_merged_feature_finished_from_another_branch_ends_on_develop_and_round_trips() {
        let env = WriteEnv::new();
        let r = repo();
        r.switch_new("feature/x");
        r.commit("f");
        r.switch("develop");
        r.git(&["merge", "-q", "--no-ff", "--no-edit", "feature/x"]);
        r.git(&["switch", "-q", "-c", "other", "main"]);
        r.commit("o");
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        let out = send(&env.api, finish(id, &r, "feature/x", None)).await.unwrap();
        assert_eq!(out["outcome"], json!({"status": "done", "merged": [], "tag": null, "deleted": true}));
        assert_eq!(r.git(&["branch", "--show-current"]), "develop");
        assert!(!exists(&r, "refs/heads/feature/x"));
        assert_eq!(r.git(&["status", "--porcelain"]), "");
        let after = RepoState::capture(&r);
        undo_top(&env.api, id, &r).await;
        assert_eq!(RepoState::capture(&r).refs, before.refs);
        assert_eq!(r.git(&["branch", "--show-current"]), "other");
        assert_eq!(r.git(&["status", "--porcelain"]), "");
        redo_top(&env.api, id, &r).await;
        assert_eq!(RepoState::capture(&r).refs, after.refs);
        assert_eq!(r.git(&["branch", "--show-current"]), "develop");
        assert_eq!(r.git(&["status", "--porcelain"]), "");
    }

    /// An untracked file develop, or a merge, would overwrite: git would refuse it part-way, so
    /// the finish refuses it before anything runs.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_untracked_file_a_checkout_would_overwrite_refuses_the_finish() {
        let env = WriteEnv::new();
        let r = repo();
        r.switch_new("feature/x");
        r.commit("f");
        r.switch("develop");
        r.git(&["merge", "-q", "--no-ff", "--no-edit", "feature/x"]);
        r.git(&["switch", "-q", "-c", "other", "main"]);
        // develop's "dev" commit added file_1.txt.
        r.write("file_1.txt", "mine\n");
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        let e = send(&env.api, finish(id, &r, "feature/x", None)).await.unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (GbErrorKind::DirtyWorktree, "Move or delete file_1.txt before finishing feature/x: it would overwrite it"));
        assert_eq!(RepoState::capture(&r).refs, before.refs);
        assert_eq!(r.git(&["branch", "--show-current"]), "other");
        assert_eq!(std::fs::read_to_string(r.path().join("file_1.txt")).unwrap(), "mine\n");
        let s = send(&env.api, json!({"method": "journalState", "params": {"repo": id, "worktree": wt(&r)}})).await.unwrap();
        assert!(s["undo"].is_null(), "nothing to undo");
        // The branch's own file, which its merge into develop writes.
        std::fs::remove_file(r.path().join("file_1.txt")).unwrap();
        r.git(&["switch", "-q", "-c", "feature/y", "develop"]);
        r.write("y/new.txt", "feature\n");
        r.git(&["add", "y/new.txt"]);
        r.git(&["commit", "-q", "-m", "y"]);
        r.switch("other");
        r.write("y/new.txt", "mine\n");
        let e = send(&env.api, finish(id, &r, "feature/y", None)).await.unwrap_err();
        assert_eq!(e.message, "Move or delete y/new.txt before finishing feature/y: it would overwrite it");
        assert_eq!(r.git(&["branch", "--show-current"]), "other");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_conflicting_merge_stops_the_finish_before_it_and_finish_again_picks_up() {
        let env = WriteEnv::new();
        let r = repo();
        r.switch_new("hotfix/1.0.1");
        r.write("version.txt", "1.0.1\n");
        r.git(&["add", "version.txt"]);
        r.git(&["commit", "-q", "-m", "fix"]);
        r.switch("develop");
        r.write("version.txt", "2.0.0-dev\n");
        r.git(&["add", "version.txt"]);
        r.git(&["commit", "-q", "-m", "next"]);
        let id = open(&env.api, &r).await;
        let out = send(&env.api, finish(id, &r, "hotfix/1.0.1", None)).await.unwrap();
        assert_eq!(out["outcome"], json!({"status": "conflicts", "into": "develop", "files": ["version.txt"], "merged": ["main"], "tag": "v1.0.1"}));
        assert_eq!(r.git(&["branch", "--show-current"]), "main", "it stops where the last merge left it");
        assert!(exists(&r, "refs/heads/hotfix/1.0.1"), "the branch stays");
        assert_eq!(r.git(&["status", "--porcelain"]), "");
        // The user's own merge, conflict resolved, then Finish again: main and the tag are done.
        r.switch("develop");
        let _ = r.try_git(&["merge", "--no-ff", "hotfix/1.0.1"]);
        r.write("version.txt", "2.0.0-dev\n");
        r.git(&["add", "version.txt"]);
        r.git(&["commit", "-q", "--no-edit"]);
        let main = oid(&r, "main");
        let out = send(&env.api, finish(id, &r, "hotfix/1.0.1", None)).await.unwrap();
        assert_eq!(out["outcome"], json!({"status": "done", "merged": [], "tag": null, "deleted": true}));
        assert_eq!(oid(&r, "main"), main);
        assert!(!exists(&r, "refs/heads/hotfix/1.0.1"));
    }

    /// The first finish named the tag itself (`3.0.0`, not `vspring`) and stopped before develop:
    /// the dialog offers that tag again, and the second finish makes no other.
    #[tokio::test(flavor = "multi_thread")]
    async fn finishing_again_keeps_the_tag_the_first_finish_made() {
        let env = WriteEnv::new();
        let r = repo();
        r.switch_new("release/spring");
        r.write("version.txt", "3.0.0\n");
        r.git(&["add", "version.txt"]);
        r.git(&["commit", "-q", "-m", "bump"]);
        r.switch("develop");
        r.write("version.txt", "3.1.0-dev\n");
        r.git(&["add", "version.txt"]);
        r.git(&["commit", "-q", "-m", "next"]);
        let id = open(&env.api, &r).await;
        let tag_of = |branch: &str| json!({"method": "gitflowFinishedTag", "params": {"repo": id, "branch": branch}});
        assert_eq!(send(&env.api, tag_of("release/spring")).await.unwrap(), json!(null), "not finished yet");
        let req = json!({"method": "gitflowFinish", "params": {"repo": id, "worktree": wt(&r), "branch": "release/spring", "tag": "3.0.0", "tagMessage": "Release 3.0.0"}});
        let out = send(&env.api, req).await.unwrap();
        assert_eq!((out["outcome"]["status"].as_str(), out["outcome"]["tag"].as_str()), (Some("conflicts"), Some("3.0.0")));
        // A later tag on main holds the release too; the nearer one is the finish's.
        r.switch("main");
        r.commit("later");
        r.git(&["tag", "3.0.1"]);
        assert_eq!(send(&env.api, tag_of("release/spring")).await.unwrap(), "3.0.0");
        assert_eq!(send(&env.api, tag_of("feature/x")).await.unwrap(), json!(null));
        r.switch("develop");
        let _ = r.try_git(&["merge", "--no-ff", "release/spring"]);
        r.write("version.txt", "3.1.0-dev\n");
        r.git(&["add", "version.txt"]);
        r.git(&["commit", "-q", "--no-edit"]);
        // Asked for the prefixes' tag (`vspring`) this time: the one main has stays the only one.
        let out = send(&env.api, finish(id, &r, "release/spring", Some("Release spring"))).await.unwrap();
        assert_eq!(out["outcome"], json!({"status": "done", "merged": [], "tag": null, "deleted": true}));
        assert_eq!(r.git(&["tag", "--list"]), "3.0.0\n3.0.1");
    }

    /// A pre-merge-commit hook that refuses isn't a conflict: the merge is undone and its error
    /// (the hook's output) is the finish's.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_hook_that_refuses_the_merge_is_an_error_not_a_conflict() {
        let env = WriteEnv::new();
        let r = repo();
        r.switch_new("feature/x");
        r.commit("f");
        r.hook("pre-merge-commit", "#!/bin/sh\necho 'merges are frozen' >&2\nexit 1\n");
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        let e = send(&env.api, finish(id, &r, "feature/x", None)).await.unwrap_err();
        assert!(e.stderr.as_deref().unwrap_or_default().contains("merges are frozen"), "{e:?}");
        assert!(!r.path().join(".git/MERGE_HEAD").exists(), "the merge is aborted");
        assert_eq!(r.git(&["status", "--porcelain"]), "");
        assert_eq!(RepoState::capture(&r).refs, before.refs);
        assert!(exists(&r, "refs/heads/feature/x"));
    }

    /// Checked out in another worktree, the branch can still be merged; only its delete can't.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_branch_checked_out_elsewhere_is_finished_when_it_is_kept() {
        let env = WriteEnv::new();
        let r = repo();
        r.switch_new("feature/x");
        r.commit("f");
        r.switch("develop");
        r.add_worktree("x", "feature/x");
        let id = open(&env.api, &r).await;
        let e = send(&env.api, finish(id, &r, "feature/x", None)).await.unwrap_err();
        assert!(e.message.contains("feature/x"), "{}", e.message);
        let req = json!({"method": "gitflowFinish", "params": {"repo": id, "worktree": wt(&r), "branch": "feature/x", "keep": true}});
        let out = send(&env.api, req).await.unwrap();
        assert_eq!(out["outcome"], json!({"status": "done", "merged": ["develop"], "tag": null, "deleted": false}));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_first_merge_that_would_conflict_changes_nothing() {
        let env = WriteEnv::new();
        let r = repo();
        r.switch_new("feature/x");
        r.write("a.txt", "feature\n");
        r.git(&["add", "a.txt"]);
        r.git(&["commit", "-q", "-m", "f"]);
        r.switch("develop");
        r.write("a.txt", "develop\n");
        r.git(&["add", "a.txt"]);
        r.git(&["commit", "-q", "-m", "d"]);
        r.switch("feature/x");
        let id = open(&env.api, &r).await;
        let before = RepoState::capture(&r);
        let out = send(&env.api, finish(id, &r, "feature/x", None)).await.unwrap();
        assert_eq!(out["outcome"]["status"], "conflicts");
        assert_eq!(RepoState::capture(&r), before);
        let s = send(&env.api, json!({"method": "journalState", "params": {"repo": id, "worktree": wt(&r)}})).await.unwrap();
        assert!(s["undo"].is_null(), "nothing to undo");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn finish_refuses_changes_and_branches_it_doesnt_know() {
        let env = WriteEnv::new();
        let r = repo();
        r.switch_new("feature/x");
        r.commit("f");
        r.write("file_0.txt", "edited\n");
        let id = open(&env.api, &r).await;
        let e = send(&env.api, finish(id, &r, "feature/x", None)).await.unwrap_err();
        assert_eq!(e.message, "Commit or stash your changes before finishing feature/x");
        let e = send(&env.api, finish(id, &r, "develop", None)).await.unwrap_err();
        assert_eq!(e.message, "develop isn't a git flow feature, release or hotfix branch");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn finish_before_init_says_so() {
        let env = WriteEnv::new();
        let r = TestRepo::new();
        identity(&r);
        r.commit("one");
        let id = open(&env.api, &r).await;
        let e = send(&env.api, finish(id, &r, "feature/x", None)).await.unwrap_err();
        assert_eq!(e.message, "Git flow isn't set up in this repository");
    }

    /// The dialog's tag name wins over the one the prefixes make.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_release_tagged_with_the_name_the_user_chose() {
        let env = WriteEnv::new();
        let r = repo();
        r.switch_new("release/spring");
        r.commit("bump");
        let id = open(&env.api, &r).await;
        let req = json!({"method": "gitflowFinish", "params": {"repo": id, "worktree": wt(&r), "branch": "release/spring", "tag": "3.0.0", "tagMessage": "Release 3.0.0"}});
        let out = send(&env.api, req).await.unwrap();
        assert_eq!(out["outcome"]["tag"], "3.0.0");
        assert_eq!(r.git(&["rev-parse", "3.0.0^{commit}"]), oid(&r, "main"));
        assert!(!exists(&r, "refs/tags/vspring"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_kept_branch_stays_and_an_existing_tag_that_isnt_the_release_refuses() {
        let env = WriteEnv::new();
        let r = repo();
        r.switch_new("release/2.0");
        r.commit("bump");
        r.switch("develop");
        r.git(&["tag", "v2.0", "main"]);
        let id = open(&env.api, &r).await;
        let e = send(&env.api, finish(id, &r, "release/2.0", Some("Release 2.0"))).await.unwrap_err();
        assert_eq!(e.message, "A tag named v2.0 already exists");
        r.git(&["tag", "-d", "v2.0"]);
        let req = json!({"method": "gitflowFinish", "params": {"repo": id, "worktree": wt(&r), "branch": "release/2.0", "keep": true}});
        let out = send(&env.api, req).await.unwrap();
        assert_eq!(out["outcome"], json!({"status": "done", "merged": ["main", "develop"], "tag": "v2.0", "deleted": false}));
        assert!(exists(&r, "refs/heads/release/2.0"));
        assert_eq!(r.git(&["cat-file", "-t", "v2.0"]), "commit", "no message: a lightweight tag");
    }
}
