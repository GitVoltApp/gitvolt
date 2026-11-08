//! Merge and the source branch's update (`MergeBox.tsx`, `branchUpdate.ts`, `projectSettings.ts`). Each click arms in place; a merge sends the head the
//! view loaded, so a moved head is refused. No optimistic UI: the store changes only with the answer.

use super::header::{mr_button, mr_primary, remote_tip};
use crate::app::{GitVolt, core_repo, icon};
use crate::forge::card::forge_avatar;
use crate::forge::text::{forge_name, kind_of, mr_ref, pipeline_text};
use crate::theme;
use crate::ui::arm::{self, ArmRequest, Origin, Tone};
use crate::ui::controls;
use crate::ui::menu::{self, MenuRow};
use crate::ui::tooltip::tip;
use gitvolt_core::api::Request;
use gitvolt_core::forge::types::{BranchUpdate, ForgeMr, ForgeMrDetail, ForgeProjectSettings, MergeMethod, MergeStatus, MrState, PipelineStatus, SquashOption};
use gitvolt_core::forge::ForgeKind;
use gpui::prelude::*;
use gpui::*;
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMode {
    Merge,
    Auto,
    Cancel,
    None,
}

#[derive(Default)]
pub struct MergeUi {
    pub method: Option<MergeMethod>,
    pub squash: Option<bool>,
    pub del: Option<bool>,
    pub busy: Option<MergeMode>,
    pub update_busy: Option<BranchUpdate>,
    pub update_error: Option<String>,
}

pub fn method_label(kind: ForgeKind, m: MergeMethod) -> &'static str {
    if kind == ForgeKind::Forgejo {
        match m {
            MergeMethod::Merge => "Create merge commit",
            MergeMethod::Squash => "Squash",
            MergeMethod::Rebase => "Rebase then fast-forward",
            MergeMethod::SemiLinear => "Rebase then create merge commit",
            MergeMethod::FastForward => "Fast-forward only",
        }
    } else {
        match m {
            MergeMethod::Merge => "Merge commit",
            MergeMethod::Squash => "Squash and merge",
            MergeMethod::Rebase => "Rebase and merge",
            MergeMethod::SemiLinear => "Merge commit with semi-linear history",
            MergeMethod::FastForward => "Fast-forward merge",
        }
    }
}

/// `filter: brightness(1.1)`, the merge button's hover.
fn brighter(c: Hsla) -> Hsla {
    Hsla { l: (c.l * 1.1).min(1.), ..c }
}

/// `.mr-merge-pick`'s left border over the button's ground, between a split button's parts: GPUI has one border colour to an
/// element, and draws it over the element's children.
fn pick_divider(ground: Option<Hsla>) -> Div {
    div().flex_none().w(px(1.)).bg(ground.map_or(theme::section_border(), |g| theme::mix(theme::text_on_accent(), 0.35, g)))
}

fn method_tip(m: MergeMethod) -> &'static str {
    match m {
        MergeMethod::Merge => "Merges with a merge commit",
        MergeMethod::Squash => "Squashes the commits into one, then merges",
        MergeMethod::Rebase => "Rebases the commits onto the target, then merges",
        MergeMethod::SemiLinear => "Merges with a merge commit, keeping the history semi-linear",
        MergeMethod::FastForward => "Merges by fast-forwarding",
    }
}

/// An update's button label, what it says while it runs, and its tooltip, as each forge's merge box names it.
pub fn update_label(kind: ForgeKind, how: BranchUpdate, target: &str) -> Option<(String, &'static str, String)> {
    Some(match (kind, how) {
        (ForgeKind::GitLab, BranchUpdate::Rebase) => ("Rebase".into(), "Rebasing…", "GitLab rebases the source branch onto the target, then runs its pipeline".into()),
        (ForgeKind::GitLab, BranchUpdate::RebaseSkipCi) => ("Rebase without pipeline".into(), "Rebasing…", "GitLab rebases the source branch onto the target, without running a pipeline".into()),
        (ForgeKind::GitHub, BranchUpdate::Merge) => (format!("Merge {target} in"), "Merging…", "GitHub merges the base branch into this branch".into()),
        (ForgeKind::GitHub, BranchUpdate::Rebase) => (format!("Rebase onto {target}"), "Rebasing…", "GitHub rebases this branch onto the base branch".into()),
        (ForgeKind::Forgejo, BranchUpdate::Merge) => (format!("Merge {target} in"), "Merging…", "Forgejo merges the target branch into this branch".into()),
        (ForgeKind::Forgejo, BranchUpdate::Rebase) => (format!("Rebase onto {target}"), "Rebasing…", "Forgejo rebases this branch onto the target branch".into()),
        _ => return None,
    })
}

/// `defaultUpdate`: the way last chosen here, when the forge offers it; else on GitHub and Forgejo a rebase when the repository allows rebase merges, else a merge; on GitLab, Rebase.
pub fn default_update(kind: ForgeKind, kinds: &[BranchUpdate], saved: Option<BranchUpdate>, methods: &[MergeMethod]) -> Option<BranchUpdate> {
    if let Some(s) = saved.filter(|s| kinds.contains(s)) {
        return Some(s);
    }
    let want = if kind != ForgeKind::GitLab && !methods.contains(&MergeMethod::Rebase) { BranchUpdate::Merge } else { BranchUpdate::Rebase };
    if kinds.contains(&want) { Some(want) } else { kinds.first().copied() }
}

/// Why Merge is disabled; `None`: it isn't.
pub fn merge_block(kind: ForgeKind, detail: Option<&ForgeMrDetail>) -> Option<String> {
    let Some(d) = detail else { return Some("Loading…".into()) };
    match &d.merge_status {
        MergeStatus::Checking => Some(format!("{} is still checking whether it can merge", forge_name(kind))),
        MergeStatus::Blocked { reason } => Some(reason.clone()),
        MergeStatus::Mergeable => None,
    }
}

fn lower_has(s: &str, words: &[&str]) -> bool {
    let l = s.to_lowercase();
    words.iter().any(|w| l.contains(w))
}

/// Blocks no wait for the checks lifts: auto-merge isn't offered (the forge would refuse it too).
fn hard_block(s: &str) -> bool {
    lower_has(s, &["conflict", "draft", "closed", "merged already", "being merged", "isn't open"])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeTone {
    Ready,
    Wait,
    Bad,
}

/// The icon's colour, the bold title and the line under it (`statusOf`); `by`: who set auto-merge, drawn after the title.
#[allow(clippy::too_many_arguments)]
pub fn status_of(kind: ForgeKind, mr: &ForgeMr, mode: MergeMode, blocked: Option<&str>, detail: Option<&ForgeMrDetail>, loaded: bool) -> (MergeTone, String, Option<String>) {
    let failed_text = if kind == ForgeKind::GitLab { "The pipeline failed" } else { "Some checks were not successful" };
    let failed = mr.pipeline.as_ref().is_some_and(|p| p.status == PipelineStatus::Failed);
    let checking = detail.is_none_or(|d| matches!(d.merge_status, MergeStatus::Checking));
    if mr.state == MrState::Merging {
        return (MergeTone::Wait, "Merging…".into(), Some(format!("{} is merging it into {}", forge_name(kind), mr.target_branch)));
    }
    if mode == MergeMode::Cancel && mr.auto_merge.is_some() {
        return if failed {
            (MergeTone::Bad, "Auto-merge set".into(), Some(format!("{failed_text}: it won't merge")))
        } else {
            let pipe = mr.pipeline.as_ref().map_or(String::new(), |p| format!(" · {}", pipeline_text(kind, Some(p))));
            (MergeTone::Wait, "Auto-merge set".into(), Some(format!("Will merge when checks pass{pipe}")))
        };
    }
    if mode == MergeMode::Auto {
        if let Some(b) = blocked {
            if !checking && loaded {
                return (if lower_has(b, &["conflict"]) { MergeTone::Bad } else { MergeTone::Wait }, "Merge is blocked".into(), Some(b.to_string()));
            }
            return (MergeTone::Wait, "Checking…".into(), Some(b.to_string()));
        }
        return (MergeTone::Wait, pipeline_text(kind, mr.pipeline.as_ref()), Some("Merge when all checks pass".into()));
    }
    let Some(b) = blocked else { return (MergeTone::Ready, "Ready to merge".into(), ready_summary(mr)) };
    let mergeable = detail.is_some_and(|d| matches!(d.merge_status, MergeStatus::Mergeable));
    if checking || (!loaded && mergeable) {
        return (MergeTone::Wait, "Checking…".into(), Some(b.to_string()));
    }
    if failed {
        return (MergeTone::Bad, "Merge is blocked".into(), Some(failed_text.into()));
    }
    if lower_has(b, &["schedul"]) {
        return (MergeTone::Wait, "Merge is scheduled".into(), Some(b.to_string()));
    }
    if lower_has(b, &["conflict", "fail", "couldn't", "can't", "cannot", "merged already"]) {
        return (MergeTone::Bad, "Merge is blocked".into(), Some(b.to_string()));
    }
    (MergeTone::Wait, "Merge is blocked".into(), Some(b.to_string()))
}

/// "Pipeline passed · approved": what makes it ready, when the forge says.
fn ready_summary(mr: &ForgeMr) -> Option<String> {
    use gitvolt_core::forge::types::ReviewDecision;
    let mut bits = Vec::new();
    if mr.pipeline.as_ref().is_some_and(|p| p.status == PipelineStatus::Success) {
        bits.push("Pipeline passed");
    }
    if mr.review.decision == ReviewDecision::Approved {
        bits.push("approved");
    }
    if bits.is_empty() {
        return None;
    }
    let s = bits.join(" · ");
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str())
}

fn behind_text(kind: ForgeKind, behind: Option<u32>, target: &str) -> String {
    if kind != ForgeKind::GitLab {
        return format!("This branch is out of date with {target}");
    }
    match behind {
        None => format!("The source branch is behind {target}"),
        Some(n) => format!("The source branch is {n} {} behind {target}", if n == 1 { "commit" } else { "commits" }),
    }
}

fn update_of(s: &str) -> Option<BranchUpdate> {
    match s {
        "rebase" => Some(BranchUpdate::Rebase),
        "rebaseSkipCi" => Some(BranchUpdate::RebaseSkipCi),
        "merge" => Some(BranchUpdate::Merge),
        _ => None,
    }
}

fn update_name(u: BranchUpdate) -> &'static str {
    match u {
        BranchUpdate::Rebase => "rebase",
        BranchUpdate::RebaseSkipCi => "rebaseSkipCi",
        BranchUpdate::Merge => "merge",
    }
}

impl GitVolt {
    /// The target project's merge settings, asked once per repository and remote for the session; a failure is asked again after the next poll.
    pub fn merge_load_settings(&mut self, tab: u32, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some(remote) = t.forge.remote.clone() else { return };
        let polled = t.forge.updated_at;
        if t.forge.settings.contains_key(&remote) || t.forge.settings_asked.get(&remote).is_some_and(|at| *at == polled) {
            return;
        }
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        t.forge.settings_asked.insert(remote.clone(), polled);
        let r = remote.clone();
        self.forge_ask::<ForgeProjectSettings>(Request::ForgeProjectSettings { repo: core_repo(tab), remote }, cx, move |v, answer, _| {
            let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) else { return };
            match answer {
                Ok(s) => {
                    t.forge.settings.insert(r.clone(), s);
                    t.forge.settings_error = None;
                }
                Err(e) => t.forge.settings_error = Some(e),
            }
        });
    }

    fn merge_options(&self, kind: ForgeKind, mr: &ForgeMr, detail: Option<&ForgeMrDetail>, s: Option<&ForgeProjectSettings>, ui: &MergeUi) -> serde_json::Value {
        let methods = s.map_or(&[][..], |s| &s.merge_methods[..]);
        let chosen = if kind == ForgeKind::GitLab { None } else { ui.method.or(methods.first().copied()) };
        let opt = s.map_or(SquashOption::DefaultOff, |s| s.squash);
        let squash_on = match opt {
            SquashOption::Always => true,
            SquashOption::Never => false,
            _ => ui.squash.or(detail.and_then(|d| d.squash)).unwrap_or(opt == SquashOption::DefaultOn),
        };
        let del_on = ui.del.or(detail.and_then(|d| d.delete_source_branch)).or(s.map(|s| s.delete_source_branch)).unwrap_or(false);
        if kind == ForgeKind::GitLab {
            json!({ "method": null, "squash": if opt == SquashOption::Never { None } else { Some(squash_on) }, "deleteSourceBranch": del_on, "expectedSha": mr.head_sha })
        } else {
            json!({ "method": chosen, "squash": null, "deleteSourceBranch": null, "expectedSha": mr.head_sha })
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn merge_run(&mut self, tab: u32, kind: ForgeKind, mr: &ForgeMr, detail: Option<&ForgeMrDetail>, mode: MergeMode, origin: Origin, window: &mut Window, cx: &mut Context<Self>) {
        let number = mr.number;
        let r = mr_ref(kind, number);
        let target = mr.target_branch.clone();
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let settings = t.forge.remote.as_ref().and_then(|x| t.forge.settings.get(x)).cloned();
        let ui = t.mr_views.get(&number);
        let options = ui.map_or(json!(null), |u| self.merge_options(kind, mr, detail, settings.as_ref(), &u.merge));
        let (from, _) = crate::forge::text::branches_parts(mr);
        let body = format!("{from} → {target}");
        let req = match mode {
            MergeMode::Merge => {
                let mut q = ArmRequest::new(Tone::Positive, format!("Click again to merge {r} into {target}"), format!("Merge {r}?"), "Merge");
                q.body = body.into();
                q
            }
            MergeMode::Auto => {
                let mut q = ArmRequest::new(Tone::Positive, format!("Click again to set {r} to auto-merge"), format!("Set {r} to auto-merge?"), "Set to auto-merge");
                q.body = "It merges when all checks pass".into();
                q
            }
            MergeMode::Cancel => ArmRequest::new(Tone::Warn, format!("Click again to cancel auto-merge of {r}"), format!("Cancel auto-merge of {r}?"), "Cancel auto-merge"),
            MergeMode::None => return,
        };
        let weak = cx.weak_entity();
        arm::request(
            req,
            origin,
            move |answer, _, cx| {
                if !answer.ok {
                    return;
                }
                weak.update(cx, |v, cx| {
                    if let Some(ui) = v.mr_ui(tab, number) {
                        ui.merge.busy = Some(mode);
                    }
                    cx.notify();
                    let repo = core_repo(tab);
                    let parsed = serde_json::from_value(options.clone()).ok();
                    let Some(options) = parsed else { return };
                    let (req, failure) = match mode {
                        MergeMode::Merge => (Request::ForgeMerge { repo, number, options }, format!("Couldn't merge {r}")),
                        MergeMode::Auto => (Request::ForgeSetAutoMerge { repo, number, options }, format!("Couldn't set {r} to auto-merge")),
                        _ => (Request::ForgeCancelAutoMerge { repo, number }, format!("Couldn't cancel auto-merge of {r}")),
                    };
                    let (r2, target) = (r.clone(), target.clone());
                    v.forge_ask::<ForgeMr>(req, cx, move |v, answer, cx| {
                        if let Some(ui) = v.mr_ui(tab, number) {
                            ui.merge.busy = None;
                        }
                        match answer {
                            Err(e) => crate::ui::toast::show_error(format!("{failure}: {e}"), cx),
                            Ok(m) => {
                                v.forge_written(tab, cx);
                                let state = m.state;
                                v.forge_put_mr(tab, m, cx);
                                crate::ui::toast::show(
                                    if mode == MergeMode::Cancel {
                                        format!("Auto-merge of {r2} cancelled")
                                    } else if state == MrState::Merged {
                                        format!("Merged {r2} into {target}")
                                    } else if state == MrState::Merging {
                                        format!("Merging {r2} into {target}…")
                                    } else {
                                        format!("{r2} will merge when all checks pass")
                                    },
                                    cx,
                                );
                                v.stack_after_merge(tab, number, cx);
                            }
                        }
                    });
                })
                .ok();
            },
            window,
            cx,
        );
    }

    /// Not destructive (the forge keeps the old head in the MR's versions): no arming. The way clicked becomes the repository's main way.
    pub fn merge_update(&mut self, tab: u32, kind: ForgeKind, mr: &ForgeMr, how: BranchUpdate, settled: bool, cx: &mut Context<Self>) {
        let number = mr.number;
        let r = mr_ref(kind, number);
        let target = mr.target_branch.clone();
        let head = mr.head_sha.clone();
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let path = t.repo_path.clone();
        if settled {
            self.update_repo(&path, move |x| x["branchUpdate"] = json!(update_name(how)), cx);
        }
        if let Some(ui) = self.mr_ui(tab, number) {
            ui.merge.update_error = None;
            ui.merge.update_busy = Some(how);
        }
        cx.notify();
        self.forge_ask::<ForgeMr>(Request::ForgeUpdateBranch { repo: core_repo(tab), number, how, expected_sha: head }, cx, move |v, answer, cx| {
            match answer {
                Ok(m) => {
                    v.forge_written(tab, cx);
                    v.forge_put_mr(tab, m, cx);
                    v.forge_refresh_mr(tab, number, cx);
                    crate::ui::toast::show(if kind == ForgeKind::GitLab || how == BranchUpdate::Rebase { format!("Rebased {r} onto {target}") } else { format!("Merged {target} into {r}") }, cx);
                }
                Err(e) => {
                    if let Some(ui) = v.mr_ui(tab, number) {
                        ui.merge.update_error = Some(e);
                    }
                }
            }
            if let Some(ui) = v.mr_ui(tab, number) {
                ui.merge.update_busy = None;
            }
        });
    }

    /// What the commits say about being behind (`useLocalBehind`): `None`: can't tell or not answered yet. Kept per repo, tip and head.
    fn merge_local_behind(&mut self, tab: u32, target: &str, head: Option<&str>, cx: &mut Context<Self>) -> Option<bool> {
        let t = self.tabs.iter().find(|t| t.id == tab)?;
        let tip = t.forge.remote.as_deref().and_then(|r| remote_tip(t, r, target))?;
        let head = head?.to_string();
        let key = format!("{tip}\0{head}");
        if let Some(known) = t.forge.behind.get(&key) {
            return *known;
        }
        let k2 = key.clone();
        let t = self.tabs.iter_mut().find(|t| t.id == tab)?;
        t.forge.behind.insert(key, None);
        let t3 = tip.clone();
        self.forge_ask::<Option<String>>(Request::MergeBase { repo: core_repo(tab), a: tip, b: head }, cx, move |v, answer, _| {
            let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) else { return };
            let behind = answer.ok().flatten().map(|base| base != t3);
            t.forge.behind.insert(k2, behind);
        });
        None
    }
}

pub fn merge_box(this: &mut GitVolt, tab: u32, kind: ForgeKind, listed: &ForgeMr, detail: Option<&ForgeMrDetail>, cx: &mut Context<GitVolt>) -> Option<AnyElement> {
    let mr = detail.map_or(listed, |d| &d.mr);
    let number = mr.number;
    let live = matches!(mr.state, MrState::Open | MrState::Draft);
    if !live && mr.state != MrState::Merging {
        return None;
    }
    this.merge_load_settings(tab, cx);
    let offer = if live { detail.and_then(|d| d.update.clone()) } else { None };
    let local_behind = if offer.is_some() { this.merge_local_behind(tab, &mr.target_branch, mr.head_sha.as_deref(), cx) } else { None };
    let t = this.tabs.iter().find(|t| t.id == tab)?;
    let settings = t.forge.remote.as_ref().and_then(|x| t.forge.settings.get(x)).cloned();
    let settings_error = t.forge.settings_error.clone();
    let saved_update = this.profile["repos"][t.repo_path.as_str()]["branchUpdate"].as_str().and_then(update_of);
    let ui = t.mr_views.get(&number)?;
    let (busy, update_busy, update_error, chosen_method, squash, del) = (ui.merge.busy, ui.merge.update_busy, ui.merge.update_error.clone(), ui.merge.method, ui.merge.squash, ui.merge.del);
    let blocked_raw = merge_block(kind, detail).or_else(|| match (&settings, &settings_error) {
        (Some(_), _) => None,
        (None, Some(e)) => Some(format!("Couldn't load merge options: {e}")),
        (None, None) => Some("Loading merge options…".to_string()),
    });
    let pipe = mr.pipeline.as_ref().map(|p| p.status);
    let checks_run = matches!(pipe, Some(PipelineStatus::Running | PipelineStatus::Pending));
    let auto = mr.auto_merge.clone();
    let offer_auto = live && auto.is_none() && checks_run;
    let auto_block = if !offer_auto {
        None
    } else if detail.is_none_or(|d| matches!(d.merge_status, MergeStatus::Checking)) || settings.is_none() {
        Some(blocked_raw.clone().unwrap_or_else(|| "Loading…".to_string()))
    } else if mr.state == MrState::Draft {
        Some("Mark it ready first: it's a draft".to_string())
    } else {
        blocked_raw.clone().filter(|b| hard_block(b))
    };
    let mode = if !live {
        MergeMode::None
    } else if auto.is_some() {
        MergeMode::Cancel
    } else if offer_auto {
        MergeMode::Auto
    } else {
        MergeMode::Merge
    };
    let blocked = match mode {
        MergeMode::Auto => auto_block,
        MergeMode::Cancel => None,
        _ => blocked_raw,
    };
    let offered: Vec<BranchUpdate> = offer.as_ref().map_or(Vec::new(), |o| o.kinds.iter().copied().filter(|k| update_label(kind, *k, "").is_some()).collect());
    let methods: Vec<MergeMethod> = settings.as_ref().map_or(Vec::new(), |s| s.merge_methods.clone());
    let main = default_update(kind, &offered, saved_update, &methods);
    let settled = kind == ForgeKind::GitLab || settings.is_some() || saved_update.is_some_and(|s| offered.contains(&s));
    let updating = update_busy.or(if offer.as_ref().is_some_and(|o| o.in_progress) { main } else { None });
    let behind_now = offer.as_ref().is_some_and(|o| local_behind == Some(true) || o.forge_behind);
    let show_update = main.is_some() && offer.is_some() && (behind_now || updating.is_some() || update_error.is_some());
    let ready = mode != MergeMode::None && busy.is_none() && updating.is_none() && blocked.is_none() && (mode == MergeMode::Cancel || settings.is_some());
    this.mr_lend_merge(tab, number, ready && mode != MergeMode::Cancel, show_update && settled && updating.is_none() && busy.is_none());
    let r = mr_ref(kind, number);
    let chosen = if kind == ForgeKind::GitLab { methods.first().copied() } else { chosen_method.or(methods.first().copied()) };
    let squash_opt = settings.as_ref().map_or(SquashOption::DefaultOff, |s| s.squash);
    let squash_on = match squash_opt {
        SquashOption::Always => true,
        SquashOption::Never => false,
        _ => squash.or(detail.and_then(|d| d.squash)).unwrap_or(squash_opt == SquashOption::DefaultOn),
    };
    let del_on = del.or(detail.and_then(|d| d.delete_source_branch)).or(settings.as_ref().map(|s| s.delete_source_branch)).unwrap_or(false);
    let (tone, title, note) = status_of(kind, mr, mode, blocked.as_deref(), detail, settings.is_some() || settings_error.is_some());
    let glyph = match tone {
        MergeTone::Ready => "check",
        MergeTone::Bad => "x",
        _ if mr.state == MrState::Merging => "loader-circle",
        _ => "clock",
    };
    let tone_color = match tone {
        MergeTone::Ready => theme::green(),
        MergeTone::Wait => theme::orange(),
        MergeTone::Bad => theme::red(),
    };
    let note_color = match tone {
        MergeTone::Ready => theme::text_secondary(),
        MergeTone::Wait => theme::orange(),
        MergeTone::Bad => theme::red(),
    };
    let dot_icon: AnyElement = if mr.state == MrState::Merging { crate::commit_box::spinner("mr-merge-spin", 13., tone_color).into_any_element() } else { icon(glyph, 13., tone_color).into_any_element() };
    let label = match busy {
        Some(MergeMode::Merge) => "Merging…",
        Some(MergeMode::Auto) => "Setting…",
        Some(MergeMode::Cancel) => "Cancelling…",
        _ => match mode {
            MergeMode::Auto => "Set to auto-merge",
            MergeMode::Cancel => "Cancel auto-merge",
            _ => "Merge",
        },
    };
    let pick = auto.is_none() && kind != ForgeKind::GitLab && methods.len() > 1 && chosen.is_some();
    let go_tip = if matches!(mode, MergeMode::Cancel | MergeMode::None) { None } else { chosen.map(|m| format!("{}{}", method_tip(m), if mode == MergeMode::Auto { " once all checks pass" } else { "" })) };
    let (m_run, d_run) = (mr.clone(), detail.cloned());
    let go = {
        let base = mr_button("mr-merge-go", ready).font_weight(if ready && mode != MergeMode::Cancel { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM }).child(label);
        let styled = if ready && mode != MergeMode::Cancel { base.bg(theme::green()).border_color(theme::green()).text_color(theme::text_on_accent()).hover(|d| d.bg(brighter(theme::green()))) } else { base };
        let styled = if pick { styled.rounded_r(px(0.)) } else { styled };
        styled.when(ready, |d| {
            d.on_click(cx.listener(move |v, _, window, cx| v.merge_run(tab, kind, &m_run, d_run.as_ref(), mode, Origin::control("mr-merge-act"), window, cx)))
        })
    };
    let go: AnyElement = match go_tip.filter(|_| ready) {
        Some(t) => tip(t, go).into_any_element(),
        None => go.into_any_element(),
    };
    let chevron = pick.then(|| {
        let key = "mr-merge-method";
        let rows_methods = methods.clone();
        let weak = cx.weak_entity();
        crate::ui::anchor(key, mr_button(key, busy.is_none()).px(px(5.)).border_l_0().rounded_l(px(0.)).bg(if ready { theme::green() } else { theme::control_bg() }).border_color(if ready { theme::green() } else { theme::section_border() }).when(ready, |d| d.hover(|d| d.bg(brighter(theme::green())))).child(icon("chevron-down", 14., if ready { theme::text_on_accent() } else { theme::text_disabled() })).on_mouse_down(MouseButton::Left, cx.listener(move |_, _, window, cx| {
            cx.stop_propagation();
            let rows: Vec<MenuRow> = rows_methods.iter().map(|m| {
                let (w, m) = (weak.clone(), *m);
                MenuRow::action(format!("merge.method.{m:?}"), method_label(kind, m), if Some(m) == chosen { "check" } else { "" }, method_tip(m), move |_, cx| {
                    w.update(cx, |v, cx| {
                        if let Some(ui) = v.mr_ui(tab, number) {
                            ui.merge.method = Some(m);
                        }
                        cx.notify();
                    }).ok();
                })
            }).collect();
            menu::open_below(key, rows, Default::default(), window, cx);
        })))
    });
    let mut title_row = div().flex().flex_wrap().items_center().gap_x(px(4.)).min_w_0().line_height(px(18.)).child(div().font_weight(FontWeight::BOLD).text_color(theme::text_selected()).child(title));
    if mode == MergeMode::Cancel
        && let Some(by) = auto.as_ref().and_then(|a| a.enabled_by.as_ref())
    {
        title_row = title_row.child(div().child("by")).child(forge_avatar(by, 16.)).child(div().font_weight(FontWeight::BOLD).text_color(theme::text_selected()).child(by.name.clone()));
    }
    let top = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .py(px(10.))
        .bg(theme::panel_bg1())
        .child(div().flex().flex_none().size(px(22.)).items_center().justify_center().rounded_full().bg(theme::mix(tone_color, 0.18, transparent_black())).child(dot_icon))
        .child(div().flex().flex_col().flex_1().min_w_0().child(title_row).child(div().min_h(px(17.)).text_size(px(12.)).text_color(note_color).child(note.unwrap_or_default())))
        // Merge alone arms, as the web's button does: the chevron stays beside its label.
        .child(crate::ui::anchor("mr-merge-buttons", div().flex().flex_none().items_stretch().child(crate::ui::anchor("mr-merge-act", go)).when(pick, |d| d.child(pick_divider(ready.then(theme::green)))).children(chevron)));
    let mut section = div().id("mr-merge").flex().flex_col().flex_none().rounded(px(8.)).border_1().border_color(theme::section_border()).overflow_hidden().child(top);
    if let (true, Some(main), Some(o)) = (show_update, main, offer.as_ref()) {
        let others: Vec<BranchUpdate> = offered.iter().copied().filter(|k| *k != main).collect();
        let note = match updating {
            Some(u) => {
                let t = &mr.target_branch;
                let text = if kind == ForgeKind::GitLab { format!("GitLab is rebasing the source branch onto {t}") } else if u == BranchUpdate::Rebase { format!("{} is rebasing the branch onto {t}", forge_name(kind)) } else { format!("{} is merging {t} into the branch", forge_name(kind)) };
                div().flex().items_center().gap(px(6.)).child(crate::commit_box::spinner("mr-update-spin", 12., theme::text_secondary())).child(text)
            }
            None => div().text_color(if update_error.is_some() { theme::red() } else { theme::text_secondary() }).child(update_error.clone().unwrap_or_else(|| behind_text(kind, o.behind, &mr.target_branch))),
        };
        let (label, busy_label, tip_text) = update_label(kind, updating.unwrap_or(main), &mr.target_branch).unwrap_or_default();
        let m_up = mr.clone();
        let enabled = updating.is_none() && busy.is_none();
        let btn = mr_primary("mr-update", enabled).when(!others.is_empty(), |d| d.rounded_r(px(0.))).child(if updating.is_some() { busy_label.to_string() } else { label }).when(enabled, |d| d.on_click(cx.listener(move |v, _, _, cx| v.merge_update(tab, kind, &m_up, main, settled, cx))));
        let btn: AnyElement = if updating.is_none() { tip(tip_text, btn).into_any_element() } else { btn.into_any_element() };
        let has_more = !others.is_empty();
        let more = has_more.then(|| {
            let key = "mr-update-more";
            let (m3, weak, target) = (mr.clone(), cx.weak_entity(), mr.target_branch.clone());
            crate::ui::anchor(key, mr_primary(key, enabled).px(px(5.)).border_l_0().rounded_l(px(0.)).child(icon("chevron-down", 14., theme::text_on_accent())).on_mouse_down(MouseButton::Left, cx.listener(move |_, _, window, cx| {
                cx.stop_propagation();
                let rows: Vec<MenuRow> = others.iter().filter_map(|k| {
                    let (l, _, t) = update_label(kind, *k, &target)?;
                    let (w, k2, m) = (weak.clone(), *k, m3.clone());
                    Some(MenuRow::action(format!("merge.update.{k:?}"), l, if *k == BranchUpdate::Merge { "git-merge" } else { "git-pull-request-arrow" }, t, move |_, cx| {
                        let m = m.clone();
                        w.update(cx, |v, cx| v.merge_update(tab, kind, &m, k2, settled, cx)).ok();
                    }))
                }).collect();
                menu::open_below(key, rows, Default::default(), window, cx);
            })))
        });
        section = section.child(div().flex().flex_wrap().items_center().gap_x(px(12.)).gap_y(px(4.)).px(px(12.)).py(px(8.)).min_h(px(38.)).border_t_1().border_color(theme::section_border()).child(div().flex_1().min_w(px(200.)).text_size(px(12.)).child(note)).child(div().flex().ml_auto().items_stretch().child(btn).when(has_more, |d| d.child(pick_divider(Some(theme::button_primary())))).children(more)));
    }
    if live && auto.is_none() && (kind == ForgeKind::GitLab || settings.as_ref().is_some_and(|s| s.delete_source_branch)) {
        let mut opts = div().flex().flex_wrap().items_center().gap(px(16.)).px(px(12.)).py(px(8.)).min_h(px(38.)).border_t_1().border_color(theme::section_border());
        if kind == ForgeKind::GitLab {
            let loaded = settings.is_some();
            let w1 = cx.weak_entity();
            let w2 = cx.weak_entity();
            let del_switch = controls::switch("mr-merge-del", del_on, "Delete source branch", Some("After the merge".into()), !loaded, move |on, _, cx| {
                w1.update(cx, |v, cx| {
                    if let Some(ui) = v.mr_ui(tab, number) {
                        ui.merge.del = Some(on);
                    }
                    cx.notify();
                }).ok();
            });
            let squash_desc: SharedString = if squash_opt == SquashOption::Always { "This project always squashes".into() } else { format!("One commit on {}", mr.target_branch).into() };
            let squash_switch = controls::switch("mr-merge-squash", squash_on, "Squash commits", Some(squash_desc), !loaded || squash_opt == SquashOption::Always, move |on, _, cx| {
                w2.update(cx, |v, cx| {
                    if let Some(ui) = v.mr_ui(tab, number) {
                        ui.merge.squash = Some(on);
                    }
                    cx.notify();
                }).ok();
            });
            opts = opts.child(div().flex().items_center().w_full().child(div().flex_1().when(!loaded, |d| d.invisible()).child(del_switch)).child(div().flex_1().when(!loaded || squash_opt == SquashOption::Never, |d| d.invisible()).child(squash_switch)));
        } else {
            opts = opts.child(div().text_size(px(11.)).text_color(theme::text_secondary()).child(format!("{} deletes the branch after merging (repository setting)", forge_name(kind))));
        }
        section = section.child(opts);
    }
    let _ = (kind_of, r);
    Some(section.into_any_element())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn the_main_update_way_follows_the_saved_one_then_the_forges() {
        use BranchUpdate::*;
        assert_eq!(default_update(ForgeKind::GitLab, &[Rebase, RebaseSkipCi], None, &[]), Some(Rebase));
        assert_eq!(default_update(ForgeKind::GitLab, &[Rebase, RebaseSkipCi], Some(RebaseSkipCi), &[]), Some(RebaseSkipCi));
        assert_eq!(default_update(ForgeKind::GitHub, &[Merge, Rebase], None, &[MergeMethod::Merge]), Some(Merge));
        assert_eq!(default_update(ForgeKind::GitHub, &[Merge, Rebase], None, &[MergeMethod::Rebase]), Some(Rebase));
        assert_eq!(default_update(ForgeKind::GitHub, &[], None, &[]), None);
    }

    #[test]
    fn a_block_no_wait_lifts_rules_out_auto_merge() {
        assert!(hard_block("It has merge conflicts"));
        assert!(hard_block("it's a Draft"));
        assert!(!hard_block("It needs approval first"));
    }
}
