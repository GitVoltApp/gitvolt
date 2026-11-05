//! Add remote's list of the target project's forks (`remotes/AddRemoteDialog.tsx`): pages of ten, the next one as the end of the
//! list nears view. Nothing is asked until the dialog opens.

use super::store::RepoProjects;
use crate::app::{GitVolt, core_repo};
use crate::format::relative_time;
use crate::theme;
use gitvolt_core::api::Request;
use gitvolt_core::forge::types::{ForgeProject, ForkPage};
use gpui::prelude::*;
use gpui::*;
use std::cell::RefCell;
use std::rc::Rc;

const FORKS_PAGE: u32 = 10;
/// No project has a list worth more pages from one dialog: a bound on what's held.
const MAX_FORKS: usize = 1000;

#[derive(Clone, PartialEq)]
pub enum More {
    Idle,
    Loading,
    Error(String),
}

pub enum Forks {
    /// Nothing known yet; `looked` once the repo turned out to have no project.
    None { looked: bool },
    Loading { of: ForgeProject },
    Ready { of: ForgeProject, remote: String, list: Vec<ForgeProject>, next: Option<u32>, more: More },
    Error { of: ForgeProject, message: String },
}

pub struct ForksUi {
    pub tab: u32,
    pub state: Rc<RefCell<Forks>>,
    /// Decided at open from what the tab knows: the box is there from the first frame.
    pub expect: bool,
    pub added: Rc<Vec<(String, String, String)>>,
    pub scroll: ScrollHandle,
    /// One Tab stop per button, kept across frames.
    pub stops: RefCell<std::collections::HashMap<String, FocusHandle>>,
    pub this: WeakEntity<GitVolt>,
}

fn host_name(h: &str) -> String {
    h.rsplit_once(':').filter(|(_, p)| p.bytes().all(|b| b.is_ascii_digit())).map_or(h, |(a, _)| a).to_lowercase()
}

/// `addedAs`: the remote already on this project.
pub fn added_as(added: &[(String, String, String)], fork: &ForgeProject) -> Option<String> {
    added.iter().find(|(_, host, path)| host_name(host) == host_name(&fork.host) && path.eq_ignore_ascii_case(&fork.path)).map(|r| r.0.clone())
}

/// A page appended: what's there already isn't listed twice, and the list stops at its bound.
pub fn merged(list: &mut Vec<ForgeProject>, page: Vec<ForgeProject>) {
    for f in page {
        if list.len() < MAX_FORKS && !list.iter().any(|x| x.path == f.path) {
            list.push(f);
        }
    }
}

impl GitVolt {
    /// The dialog's state, and the first page asked. `None`: a repository not read yet.
    pub fn forks_open(&mut self, cx: &mut Context<Self>) -> Option<ForksUi> {
        let t = self.tabs.get(self.active)?;
        let tab = t.id;
        let expect = t.forge.kind.is_some() || !t.forge.mapped.is_empty();
        let added = t.info["remotes"].as_array().map_or(Vec::new(), |l| l.iter().filter_map(|r| Some((r["name"].as_str()?.to_string(), r["host"].as_str()?.to_string(), r["path"].as_str()?.to_string()))).collect());
        let state = Rc::new(RefCell::new(Forks::None { looked: false }));
        let ui = ForksUi { tab, state: state.clone(), expect, added: Rc::new(added), scroll: ScrollHandle::new(), stops: RefCell::default(), this: cx.weak_entity() };
        if cfg!(test) {
            return Some(ui);
        }
        let (api, rt) = (self.api.clone(), self.rt.clone());
        cx.spawn(async move |_, cx| {
            let projects = super::ask::<RepoProjects>(api.clone(), rt.clone(), Request::ForgeRepoProjects { repo: core_repo(tab), refresh: false }).await.ok();
            let target = projects.as_ref().and_then(|p| p.remotes.iter().find(|r| Some(&r.remote) == p.target.as_ref()));
            let found = target.and_then(|t| Some((t.remote.clone(), t.project.clone()?)));
            let Some((remote, of)) = found else {
                *state.borrow_mut() = Forks::None { looked: true };
                cx.update(|cx| cx.refresh_windows());
                return;
            };
            *state.borrow_mut() = Forks::Loading { of: of.clone() };
            cx.update(|cx| cx.refresh_windows());
            let page = super::ask::<ForkPage>(api, rt, Request::ForgeForks { repo: core_repo(tab), remote: remote.clone(), page: 1, per_page: FORKS_PAGE }).await;
            *state.borrow_mut() = match page {
                Ok(p) => Forks::Ready { of, remote, list: p.forks, next: p.next, more: More::Idle },
                Err(message) => Forks::Error { of, message },
            };
            cx.update(|cx| cx.refresh_windows());
        })
        .detach();
        Some(ui)
    }

    fn forks_more(&mut self, tab: u32, state: Rc<RefCell<Forks>>, cx: &mut Context<Self>) {
        let (remote, next) = {
            let mut s = state.borrow_mut();
            let Forks::Ready { remote, next: Some(next), more, .. } = &mut *s else { return };
            if *more == More::Loading {
                return;
            }
            *more = More::Loading;
            (remote.clone(), *next)
        };
        let (api, rt) = (self.api.clone(), self.rt.clone());
        cx.spawn(async move |_, cx| {
            let page = super::ask::<ForkPage>(api, rt, Request::ForgeForks { repo: core_repo(tab), remote, page: next, per_page: FORKS_PAGE }).await;
            if let Forks::Ready { list, next, more, .. } = &mut *state.borrow_mut() {
                match page {
                    Ok(p) => {
                        merged(list, p.forks);
                        *next = p.next.filter(|_| list.len() < MAX_FORKS);
                        *more = More::Idle;
                    }
                    Err(e) => *more = More::Error(e),
                }
            }
            cx.update(|cx| cx.refresh_windows());
        })
        .detach();
    }
}

fn small_button(id: impl Into<ElementId>, label: impl Into<SharedString>, tip: impl Into<SharedString>, focus: FocusHandle, run: impl Fn(&mut Window, &mut App) + 'static + Clone) -> impl IntoElement {
    let on_click = run.clone();
    crate::ui::tooltip::tip(tip, crate::ui::controls::tabbable(
        div()
            .id(id)
            .flex_none()
            .px(px(10.))
            .py(px(2.))
            .rounded(px(4.))
            .border_1()
            .border_color(theme::section_border())
            .text_color(theme::text_normal())
            .hover(|s| s.bg(theme::hover_row()).text_color(theme::text_selected()))
            .cursor_pointer()
            .child(label.into())
            .on_click(move |_, window, cx| on_click(window, cx)),
        &focus,
        run,
    ))
}

/// `add-remote-forks`: one box of fixed height, so the dialog never resizes as forks arrive.
pub fn section(ui: &ForksUi, modal_id: &'static str, window: &mut Window, cx: &mut App) -> AnyElement {
    let state = ui.state.borrow();
    if !ui.expect && matches!(&*state, Forks::None { .. }) {
        return div().into_any_element();
    }
    let title = match &*state {
        Forks::None { .. } => "Forks".to_string(),
        Forks::Loading { of } | Forks::Ready { of, .. } | Forks::Error { of, .. } => format!("Forks of {}", of.path),
    };
    let stop = |key: String, cx: &mut App| ui.stops.borrow_mut().entry(key).or_insert_with(|| cx.focus_handle().tab_stop(true)).clone();
    let dim = |s: SharedString| div().text_color(theme::text_normal()).child(s);
    let body: AnyElement = match &*state {
        Forks::None { looked } => dim(if *looked { "No forks to show" } else { "Loading forks…" }.into()).into_any_element(),
        Forks::Loading { .. } => dim("Loading forks…".into()).into_any_element(),
        Forks::Error { message, .. } => div().text_color(theme::red()).child(message.clone()).into_any_element(),
        Forks::Ready { list, .. } if list.is_empty() => dim("No forks yet".into()).into_any_element(),
        Forks::Ready { list, next, more, .. } => {
            let now = crate::format::now();
            let rows: Vec<_> = list.iter().enumerate().map(|(i, f)| {
                let meta = format!("{}{}", f.default_branch.as_deref().unwrap_or("empty"), f.updated_at.map_or(String::new(), |t| format!(" · updated {}", relative_time(t, now))));
                let owner = f.owner.rsplit('/').next().filter(|o| !o.is_empty()).unwrap_or(&f.owner).to_string();
                let trailing = match added_as(&ui.added, f) {
                    Some(name) => dim(format!("Added as {name}").into()).into_any_element(),
                    None => {
                        let (this, tab, fork) = (ui.this.clone(), ui.tab, f.clone());
                        crate::ui::anchor(
                            format!("fork-add:{}", f.path),
                            small_button(("fork-add", i), "Add", format!("Add {owner}'s fork"), stop(f.path.clone(), cx), move |window, cx| {
                                crate::ui::modal::close(modal_id, window, cx);
                                let fork = fork.clone();
                                this.update(cx, |v, cx| v.add_fork_remote(tab, fork, cx, |_, _, _| {})).ok();
                            }),
                        )
                        .into_any_element()
                    }
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(4.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .hover(|s| s.bg(theme::hover_row()))
                    .child(div().flex_1().min_w_0().truncate().text_color(theme::text_selected()).child(f.path.clone()))
                    .child(div().flex_none().text_size(px(12.)).whitespace_nowrap().text_color(theme::text_normal()).child(meta))
                    .child(trailing)
            }).collect();
            let tail: Option<AnyElement> = match (next, more) {
                (_, More::Loading) => Some(dim("Loading…".into()).into_any_element()),
                (_, More::Error(e)) => {
                    let (this, tab, st) = (ui.this.clone(), ui.tab, ui.state.clone());
                    Some(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_color(theme::red())
                            .child(format!("Couldn't load more forks: {e}"))
                            .child(small_button("fork-retry", "Retry", "Load the next page again", stop("retry".to_string(), cx), move |_, cx| {
                                let st = st.clone();
                                this.update(cx, |v, cx| v.forks_more(tab, st, cx)).ok();
                            }))
                            .into_any_element(),
                    )
                }
                (Some(_), More::Idle) => {
                    let (this, tab, st) = (ui.this.clone(), ui.tab, ui.state.clone());
                    Some(
                        small_button("fork-more", "Load more forks", "Load the next page", stop("more".to_string(), cx), move |_, cx| {
                            let st = st.clone();
                            this.update(cx, |v, cx| v.forks_more(tab, st, cx)).ok();
                        })
                        .into_any_element(),
                    )
                }
                (None, More::Idle) => None,
            };
            // The next page as the end of the list nears view; loaded by no loop, only on a frame that shows it.
            let offset = -ui.scroll.offset().y.as_f32();
            let view = ui.scroll.bounds().size.height.as_f32();
            let content = ui.scroll.max_offset().y.as_f32() + view;
            if next.is_some() && *more == More::Idle && view > 0.0 && offset + view + 80.0 >= content {
                let (this, tab, st) = (ui.this.clone(), ui.tab, ui.state.clone());
                window.defer(cx, move |_, cx| {
                    this.update(cx, |v, cx| v.forks_more(tab, st, cx)).ok();
                });
            }
            div()
                .id("fork-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&ui.scroll)
                .children(rows)
                .when_some(tail, |d, t| d.child(div().min_h(px(28.)).flex().items_center().justify_center().child(t)))
                .into_any_element()
        }
    };
    div()
        .mt(px(2.))
        .pt(px(10.))
        .border_t_1()
        .border_color(theme::section_border())
        .child(div().mb(px(6.)).text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_selected()).child(title))
        .child(div().flex().flex_col().h(px(240.)).child(body))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn fork(path: &str, host: &str) -> ForgeProject {
        serde_json::from_value(serde_json::json!({"kind": "gitlab", "id": 1, "host": host, "path": path, "name": "p", "owner": "alice", "webUrl": "", "defaultBranch": null, "cloneHttps": "", "cloneSsh": "", "forkOf": null, "updatedAt": null, "archived": false})).unwrap()
    }

    #[test]
    fn a_fork_already_added_goes_by_host_without_port_and_path_without_case() {
        let added = vec![("alice".to_string(), "forge.example.com".to_string(), "Alice/Project".to_string())];
        assert_eq!(added_as(&added, &fork("alice/project", "forge.example.com:8443")).as_deref(), Some("alice"));
        assert_eq!(added_as(&added, &fork("bob/project", "forge.example.com")), None);
    }

    #[test]
    fn pages_merge_without_doubles() {
        let mut list = vec![fork("a/p", "h")];
        merged(&mut list, vec![fork("a/p", "h"), fork("b/p", "h")]);
        assert_eq!(list.len(), 2);
    }
}
