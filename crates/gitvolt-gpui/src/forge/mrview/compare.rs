//! The branch card's Compare (`compare.ts`, `compareRef.ts`): the MR/PR's changes as a compare in the graph, exactly as clicking its base then
//! Ctrl+clicking its head. A head that isn't in the repository is fetched: its ref alone, into a ref of its own that lives while that compare does.

use super::super::text::mr_name;
use crate::app::{GitVolt, core_repo};
use crate::write::{Write, call};
use gitvolt_core::api::Request;
use gitvolt_core::forge::types::{ForgeMr, ForgeMrDetail, MrState};
use gitvolt_core::forge::ForgeKind;
use gpui::*;
use serde_json::json;

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(6)]
}

impl GitVolt {
    pub fn mr_compare(&mut self, tab: u32, kind: ForgeKind, mr: ForgeMr, detail: Option<ForgeMrDetail>, cx: &mut Context<Self>) {
        let noun = mr_name(kind).to_lowercase();
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let session_refs = t.forge.review.as_ref().filter(|r| r.number == mr.number).and_then(|r| r.refs.clone());
        let head = session_refs.as_ref().map(|r| r.head_sha.clone()).or_else(|| detail.as_ref().and_then(|d| d.mr.head_sha.clone())).or_else(|| mr.head_sha.clone());
        let merged = detail.as_ref().map_or(mr.state, |d| d.mr.state) == MrState::Merged;
        let landed = if merged { detail.as_ref().and_then(|d| d.merged_sha.clone()) } else { None };
        let remote = t.forge.remote.clone();
        let say = |m: String, v: &mut GitVolt, cx: &mut Context<GitVolt>| {
            crate::ui::toast::show(m, cx);
            v.mr_comparing(tab, mr.number, false);
        };
        if let Some(landed) = landed {
            let in_repo = t.graph.as_ref().is_some_and(|g| g.by_sha.contains_key(&landed));
            if in_repo {
                return self.mr_compare_landed(tab, mr.number, landed, noun, cx);
            }
            return match remote {
                Some(remote) => {
                    let number = mr.number;
                    self.mr_fetch_then(tab, remote, None, cx, move |v, cx| v.mr_compare_landed(tab, number, landed, noun, cx))
                }
                None => {
                    self.mr_comparing(tab, mr.number, false);
                    crate::ui::toast::show(format!("The {noun} was merged as {}, which isn't in this repository", short(&landed)), cx);
                }
            };
        }
        let Some(head) = head else { return say(format!("The {noun}'s head isn't known yet"), self, cx) };
        if t.graph.as_ref().is_some_and(|g| g.by_sha.contains_key(&head)) {
            return self.mr_compare_with_head(tab, kind, mr, detail, head, session_refs, cx);
        }
        // In the repository but older than the loaded window: a deeper one, no network.
        let (number, repo) = (mr.number, core_repo(tab));
        let at_head = head.clone();
        self.forge_ask::<serde_json::Value>(Request::LocateCommit { repo, sha: head.clone() }, cx, move |v, answer, cx| {
            let found = answer.as_ref().is_ok_and(|l| l["found"].as_bool() == Some(true));
            if found {
                let limit = answer.as_ref().ok().and_then(|l| l["limit"].as_u64()).unwrap_or(0) as u32;
                v.mr_deepen_then(tab, limit, cx, move |v, cx| v.mr_compare_with_head(tab, kind, mr, detail, at_head, session_refs, cx));
                return;
            }
            let Some(remote) = remote else {
                v.mr_comparing(tab, number, false);
                crate::ui::toast::show(format!("The {}'s commits aren't in this repository", mr_name(kind).to_lowercase()), cx);
                return;
            };
            let head_arg = (!merged).then(|| json!({ "kind": kind, "number": number }));
            let (m2, d2, h2, r2) = (mr, detail, at_head, session_refs);
            let from = remote.clone();
            let held = !merged;
            v.mr_fetch_then(tab, remote, head_arg, cx, move |v, cx| {
                let there = v.tabs.iter().find(|t| t.id == tab).and_then(|t| t.graph.as_ref()).is_some_and(|g| g.by_sha.contains_key(&h2));
                if there {
                    if held && let Some(t) = v.tabs.iter_mut().find(|t| t.id == tab) {
                        t.forge.compare_ref = Some((m2.number, h2.clone(), false));
                    }
                    v.mr_compare_with_head(tab, kind, m2, d2, h2, r2, cx);
                } else {
                    v.mr_comparing(tab, number, false);
                    let noun = mr_name(kind).to_lowercase();
                    crate::ui::toast::show(if merged { format!("The merged {noun}'s commits aren't on {from}") } else { format!("The {noun}'s commits couldn't be fetched from {from}") }, cx);
                    v.mr_release_compare_ref(tab, cx);
                }
            });
        });
    }

    pub fn mr_comparing(&mut self, tab: u32, number: u64, on: bool) {
        if let Some(ui) = self.mr_ui(tab, number) {
            ui.comparing = on;
        }
    }

    fn mr_fetch_then(&mut self, tab: u32, remote: String, mr_head: Option<serde_json::Value>, cx: &mut Context<Self>, then: impl FnOnce(&mut GitVolt, &mut Context<GitVolt>) + 'static) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let ctx = t.write_target();
        let repo = core_repo(tab);
        let then = std::cell::RefCell::new(Some(then));
        let w = Write::new(&ctx, move |_| {
            let mut p = json!({ "repo": repo, "background": false, "remote": remote });
            if let Some(h) = &mr_head {
                p["mrHead"] = h.clone();
            }
            call("fetch", p)
        })
        .on_end(move |v, _, cx| {
            if let Some(then) = then.borrow_mut().take() {
                v.reload_then(tab, cx, then);
            }
        });
        self.write(w, cx);
    }

    fn mr_deepen_then(&mut self, tab: u32, limit: u32, cx: &mut Context<Self>, then: impl FnOnce(&mut GitVolt, &mut Context<GitVolt>) + 'static) {
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab)
            && limit > t.limit
        {
            t.limit = limit;
        }
        self.reload_then(tab, cx, then);
    }

    #[allow(clippy::too_many_arguments)]
    fn mr_compare_with_head(&mut self, tab: u32, kind: ForgeKind, mr: ForgeMr, detail: Option<ForgeMrDetail>, head: String, refs: Option<gitvolt_core::forge::types::DiffRefs>, cx: &mut Context<Self>) {
        let noun = mr_name(kind).to_lowercase();
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let tip = t.forge.remote.as_deref().and_then(|r| crate::forge::mrview::header::remote_tip(t, r, &mr.target_branch));
        let mut froms: Vec<String> = Vec::new();
        froms.extend(refs.as_ref().map(|r| r.base_sha.clone()));
        froms.extend(detail.as_ref().and_then(|d| d.base_sha.clone()));
        froms.extend(tip);
        let (api, rt, repo) = (self.api.clone(), self.rt.clone(), core_repo(tab));
        let number = mr.number;
        let h = head.clone();
        cx.spawn(async move |this, cx| {
            let mut base: Option<String> = None;
            for from in froms.into_iter().take(3) {
                base = super::super::ask::<Option<String>>(api.clone(), rt.clone(), Request::MergeBase { repo, a: from, b: h.clone() }).await.ok().flatten();
                if base.is_some() {
                    break;
                }
            }
            this.update(cx, |v, cx| {
                v.mr_comparing(tab, number, false);
                let Some(base) = base else {
                    crate::ui::toast::show(format!("The {noun}'s base isn't in this repository: fetch first"), cx);
                    v.mr_release_compare_ref(tab, cx);
                    return;
                };
                if !v.mr_compare_commits(tab, &base, &head, cx) {
                    crate::ui::toast::show(format!("The {noun}'s base isn't in the graph's history"), cx);
                    v.mr_release_compare_ref(tab, cx);
                    return;
                }
                // The MR's Compare is where its review happens.
                v.review_start(tab, kind, number, (base, head), cx);
            })
            .ok();
        })
        .detach();
    }

    fn mr_compare_landed(&mut self, tab: u32, number: u64, landed: String, noun: String, cx: &mut Context<Self>) {
        self.mr_comparing(tab, number, false);
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return };
        let Some(g) = t.graph.as_ref() else { return };
        let Some(row) = g.by_sha.get(&landed).copied() else {
            crate::ui::toast::show(format!("The {noun} was merged as {}, which isn't in this repository", short(&landed)), cx);
            return;
        };
        let Some(parent) = g.rows[row].parents.first().map(|p| p.to_string()) else {
            crate::ui::toast::show(format!("{}, the {noun}'s merge, has no parent to compare with", short(&landed)), cx);
            return;
        };
        if !self.mr_compare_commits(tab, &parent, &landed, cx) {
            crate::ui::toast::show(format!("The {noun}'s merge isn't in the graph's history"), cx);
        }
    }

    /// As clicking the base then Ctrl+clicking the head; whether both are in the graph.
    fn mr_compare_commits(&mut self, tab: u32, from: &str, to: &str, cx: &mut Context<Self>) -> bool {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else { return false };
        if !t.graph.as_ref().is_some_and(|g| g.by_sha.contains_key(from) && g.by_sha.contains_key(to)) {
            return false;
        }
        self.compare_commits(from.to_string(), to.to_string(), cx);
        true
    }

    /// The hold on a head Compare fetched ends: its ref goes, and the graph stops drawing commits no branch holds.
    pub fn mr_release_compare_ref(&mut self, tab: u32, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        if t.forge.compare_ref.take().is_none() {
            return;
        }
        // Housekeeping: one left by a failure goes at the repository's next open.
        let (api, repo) = (self.api.clone(), core_repo(tab));
        let job = self.rt.spawn(async move { api.dispatch(Request::DropCompareRefs { repo }).await });
        cx.spawn(async move |this, cx| {
            job.await.ok();
            this.update(cx, |v, cx| v.reload(tab, cx)).ok();
        })
        .detach();
    }

    /// The tab's compare moved on: what a fetched head's hold watches.
    pub fn mr_compare_watch(&mut self, tab: u32, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        let Some((_, head, shown)) = t.forge.compare_ref.clone() else { return };
        let comparing = t.compare.as_ref().is_some_and(|c| c.from == head || c.to.sha() == Some(head.as_str()));
        if comparing {
            if !shown && let Some(h) = t.forge.compare_ref.as_mut() {
                h.2 = true;
            }
        } else if shown {
            self.mr_release_compare_ref(tab, cx);
        }
    }
}
