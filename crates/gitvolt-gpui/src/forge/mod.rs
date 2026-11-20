//! The forge's views (`ui/src/forge`): accounts, the MR/PR list and its polling, the badges and cards, the MR/PR view,
//! reviews, creating one and stacks. Everything goes through the core's `Request`s, as `ui/src/api/client.ts` makes them.
//! What a forge sends is a stranger's text: a body goes through `crate::md`, and nothing here runs or loads anything else.

pub mod accounts;
pub mod accounts_forks;
pub mod card;
pub mod create;
pub mod goto;
pub mod keys;
pub mod menu;
pub mod mrview;
pub mod poll;
pub mod review;
pub mod search;
pub mod section;
pub mod stack;
pub mod store;
pub mod text;
pub mod urls;

use crate::app::{GitVolt, core_repo as app_repo};
use gitvolt_core::api::{Api, Request};
use gitvolt_core::forge::ForgeKind;
use gpui::*;
use serde::de::DeserializeOwned;
use std::sync::Arc;

/// A request through the core, off the UI thread, its answer typed. The error is what the UI may show: the core's message,
/// through its redaction filter.
pub async fn ask<T: DeserializeOwned + Send + 'static>(api: Arc<Api>, rt: tokio::runtime::Handle, req: Request) -> Result<T, String> {
    let job = rt.spawn(async move { api.dispatch(req).await });
    match job.await {
        Ok(Ok(v)) => serde_json::from_value(v).map_err(|e| format!("An answer that can't be read: {e}")),
        Ok(Err(e)) => Err(e.message),
        Err(e) => Err(e.to_string()),
    }
}

impl GitVolt {
    /// A reload of the tab, and `then` once it's landed.
    pub fn reload_then(&mut self, tab: u32, cx: &mut Context<Self>, then: impl FnOnce(&mut GitVolt, &mut Context<GitVolt>) + 'static) {
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
            t.after_reload.push(Box::new(then));
        }
        self.reload(tab, cx);
    }

    /// Draft ⇄ ready: a toast as it starts, the answer into the store, and a poll.
    pub fn mr_toggle_draft(&mut self, tab: u32, kind: ForgeKind, mr: gitvolt_core::forge::types::ForgeMr, cx: &mut Context<Self>) {
        let draft = mr.state != gitvolt_core::forge::types::MrState::Draft;
        let r = text::mr_ref(kind, mr.number);
        crate::ui::toast::show(if draft { format!("Marking {r} as a draft…") } else { format!("Marking {r} as ready…") }, cx);
        let failure = if draft { format!("Couldn't mark {r} as a draft") } else { format!("Couldn't mark {r} as ready") };
        self.forge_write::<gitvolt_core::forge::types::ForgeMr>(tab, failure, Request::ForgeSetDraft { repo: app_repo(tab), number: mr.number, draft }, cx, move |v, answer, cx| v.forge_put_mr(tab, answer, cx));
    }

    /// One forge write (`forgeWrite`): a remote action, not journaled. It works: the tab polls at once, and `then` gets the answer.
    /// It fails: a toast says why and nothing in the store changes.
    pub fn forge_write<T: DeserializeOwned + Send + 'static>(&mut self, tab: u32, failure: String, req: Request, cx: &mut Context<Self>, then: impl FnOnce(&mut GitVolt, T, &mut Context<GitVolt>) + 'static) {
        self.forge_ask::<T>(req, cx, move |v, answer, cx| match answer {
            Ok(value) => {
                v.forge_written(tab, cx);
                then(v, value, cx);
            }
            Err(e) => crate::ui::toast::show_error(format!("{failure}: {e}"), cx),
        });
    }

    /// `putMr`: the MR/PR a write answered with, into the list and its detail. A write's answer has no review (and GitLab's no
    /// pipeline): the loaded ones stay until the next poll.
    pub fn forge_put_mr(&mut self, tab: u32, mr: gitvolt_core::forge::types::ForgeMr, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        if let Some(list) = t.forge.list.as_mut()
            && let Some(m) = list.mrs.iter_mut().find(|m| m.number == mr.number)
        {
            let (review, pipeline) = (m.review.clone(), mr.pipeline.clone().or(m.pipeline.clone()));
            *m = gitvolt_core::forge::types::ForgeMr { review, pipeline, ..mr.clone() };
        }
        if let Some(d) = t.forge.details.get_mut(&mr.number) {
            let (review, pipeline) = (d.mr.review.clone(), mr.pipeline.clone().or(d.mr.pipeline.clone()));
            d.mr = gitvolt_core::forge::types::ForgeMr { review, pipeline, ..mr };
        }
        self.forge_list_changed(tab, cx);
    }

    /// `ask`, its answer back on the view.
    pub fn forge_ask<T: DeserializeOwned + Send + 'static>(&self, req: Request, cx: &mut Context<Self>, then: impl FnOnce(&mut GitVolt, Result<T, String>, &mut Context<GitVolt>) + 'static) {
        if cfg!(test) {
            return;
        }
        let (api, rt) = (self.api.clone(), self.rt.clone());
        cx.spawn(async move |this, cx| {
            let answer = ask::<T>(api, rt, req).await;
            this.update(cx, |v, cx| {
                then(v, answer, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// What a script's `dump` says of the tab's forge: never a token, which nothing here holds.
pub fn dump(t: &crate::app::Tab) -> serde_json::Value {
    let f = &t.forge;
    let r = f.review.as_ref();
    serde_json::json!({
        "kind": f.kind,
        "remote": f.remote,
        "me": f.me,
        "accountId": f.account_id,
        "target": f.target,
        "list": f.list.as_ref().map(|l| l.mrs.iter().map(|m| m.number).collect::<Vec<_>>()),
        "badges": f.by_ref.iter().map(|(k, m)| format!("{k}={}", m.number)).collect::<Vec<_>>(),
        "openMr": f.open_mr,
        "create": t.create.as_ref().map(|u| serde_json::json!({ "branch": u.branch, "problem": u.problem, "busy": u.busy, "error": u.error, "draft": u.draft.as_ref().map(|d| serde_json::json!({ "title": d.title, "description": d.description, "reviewers": d.reviewers.len(), "assignees": d.assignees.len(), "labels": d.labels, "asDraft": d.draft, "target": d.target_branch })) })),
        "error": f.error,
        "failures": f.failures,
        "state": f.open_mr.and_then(|n| f.known_mr(n)).map(|m| serde_json::to_value(m.state).unwrap_or_default()),
        "threads": f.open_mr.and_then(|n| f.discussions.get(&n)).map(Vec::len),
        "threadIds": f.open_mr.and_then(|n| f.discussions.get(&n)).map(|l| l.iter().filter(|d| d.notes.iter().any(|n| !n.system)).take(50).map(|d| d.id.clone()).collect::<Vec<_>>()),
        "resolved": f.open_mr.and_then(|n| f.discussions.get(&n)).map(|l| l.iter().filter(|d| d.resolved).count()),
        "review": r.map(|s| serde_json::json!({ "number": s.number, "drafts": s.drafts.len(), "pending": s.pending_review.is_some(), "files": s.files.len(), "refs": s.refs.is_some(), "error": s.error })),
    })
}
