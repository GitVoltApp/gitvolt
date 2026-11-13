//! What's new (`ui/src/updates/whatsNew.ts`, `changelog.ts`, `WhatsNew.tsx`, `WhatsNewDialog.tsx`): the first start of a newer
//! version puts "Updated to GitVolt …" in the bell, which opens the notes of every version since the one that last ran, newest
//! first; Help › What's new shows them again. The notes are CHANGELOG.md's released sections, built in: nothing is fetched.

use crate::app::GitVolt;
use crate::md::view::{Content, MdOptions, MdView};
use crate::theme;
use crate::ui::controls::{self, ButtonKind};
use crate::ui::modal::{self, Modal};
use gpui::prelude::*;
use gpui::*;
use std::sync::{Arc, OnceLock};

const DIALOG: &str = "whats-new";

/// A released section of CHANGELOG.md.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub version: String,
    /// `YYYY-MM-DD`, or empty when the heading has none.
    pub date: String,
    /// The section's Markdown, without its heading or surrounding blank lines.
    pub body: String,
}

/// The notes What's new shows: after `since` up to `version`; `since` none: `version`'s alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Range {
    pub version: String,
    pub since: Option<String>,
}

/// This session's What's new.
#[derive(Default)]
pub struct WhatsNew {
    /// "Updated to GitVolt …" in the bell, until cleared: this session's update.
    pub notice: Option<Range>,
    /// The notice hasn't been seen in the bell yet: it counts in the bell's badge.
    pub unread: bool,
    /// The version this session updated from, kept when the notice is cleared (Help › What's new).
    pub updated_from: Option<String>,
    checked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ident {
    Num(u64),
    Text(String),
}

struct Version {
    core: [u64; 3],
    pre: Vec<Ident>,
}

/// `1.2.3`, `v1.2.3-rc.1`, `1.2.3+202610051325.ab4dbf9e` (build metadata ignored, as in the core's `updates/version.rs`);
/// none for anything else.
fn parse_version(s: &str) -> Option<Version> {
    let s = s.trim();
    let s = s.strip_prefix('v').unwrap_or(s);
    let (s, build) = s.split_once('+').map_or((s, None), |(a, b)| (a, Some(b)));
    if build.is_some_and(|b| b.is_empty() || !b.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')) {
        return None;
    }
    let (core, pre) = s.split_once('-').map_or((s, None), |(a, b)| (a, Some(b)));
    let num = |p: &str| (!p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()) && (p == "0" || !p.starts_with('0'))).then(|| p.parse::<u64>().ok()).flatten();
    let parts: Vec<&str> = core.split('.').collect();
    let [a, b, c] = parts[..] else { return None };
    let core = [num(a)?, num(b)?, num(c)?];
    let pre = match pre {
        None => Vec::new(),
        Some(p) => p
            .split('.')
            .map(|i| {
                if i.is_empty() || !i.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
                    None
                } else if i.bytes().all(|c| c.is_ascii_digit()) {
                    num(i).map(Ident::Num)
                } else {
                    Some(Ident::Text(i.to_string()))
                }
            })
            .collect::<Option<Vec<_>>>()?,
    };
    Some(Version { core, pre })
}

/// SemVer order of two versions; none when either isn't a version.
pub fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    let (x, y) = (parse_version(a)?, parse_version(b)?);
    let core = x.core.cmp(&y.core);
    if core != Ordering::Equal {
        return Some(core);
    }
    // A pre-release comes before its release.
    if x.pre.is_empty() || y.pre.is_empty() {
        return Some(y.pre.len().cmp(&x.pre.len()));
    }
    for (p, q) in x.pre.iter().zip(&y.pre) {
        let c = match (p, q) {
            (Ident::Num(m), Ident::Num(n)) => m.cmp(n),
            (Ident::Num(_), Ident::Text(_)) => Ordering::Less,
            (Ident::Text(_), Ident::Num(_)) => Ordering::Greater,
            (Ident::Text(m), Ident::Text(n)) => m.cmp(n),
        };
        if c != Ordering::Equal {
            return Some(c);
        }
    }
    Some(x.pre.len().cmp(&y.pre.len()))
}

/// The released sections (`## [1.6.1] - 2026-11-12`), in file order: `[Unreleased]` and any heading that isn't a version
/// are skipped, and a section ends at the next `## ` heading.
pub fn parse_changelog(text: &str) -> Vec<Entry> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = Vec::new();
    let mut current: Option<(String, String, Vec<&str>)> = None;
    let link_ref = |l: &str| l.starts_with('[') && l.split_once("]:").is_some_and(|(label, rest)| label.len() > 1 && rest.starts_with(char::is_whitespace) && !rest.trim().is_empty());
    let mut close = |current: &mut Option<(String, String, Vec<&str>)>| {
        let Some((version, date, mut body)) = current.take() else { return };
        while body.last().is_some_and(|l| l.trim().is_empty() || link_ref(l)) {
            body.pop();
        }
        let first = body.iter().position(|l| !l.trim().is_empty()).unwrap_or(body.len());
        out.push(Entry { version, date, body: body[first..].join("\n") });
    };
    for line in text.split('\n') {
        if let Some(rest) = line.strip_prefix("## ") {
            close(&mut current);
            // `[version]`, then nothing, or ` - date` (the date may be missing).
            let Some((version, after)) = rest.strip_prefix('[').and_then(|r| r.split_once(']')) else { continue };
            let after = after.trim();
            let date = match after.strip_prefix('-') {
                Some(d) if !d.trim().contains(char::is_whitespace) => d.trim().to_string(),
                None if after.is_empty() => String::new(),
                _ => continue,
            };
            if parse_version(version).is_some() {
                current = Some((version.to_string(), date, Vec::new()));
            }
        } else if let Some((_, _, body)) = current.as_mut() {
            body.push(line);
        }
    }
    close(&mut current);
    out
}

/// The notes for `current`, newest first: every version after `since` up to `current`, or `current`'s alone with no `since`.
/// Versions newer than `current` are left out.
pub fn notes_between<'a>(entries: &'a [Entry], since: Option<&str>, current: &str) -> Vec<&'a Entry> {
    use std::cmp::Ordering;
    let mut picked: Vec<&Entry> = entries
        .iter()
        .filter(|e| {
            let Some(up_to) = compare_versions(&e.version, current) else { return false };
            if up_to == Ordering::Greater {
                return false;
            }
            match since {
                None => up_to == Ordering::Equal,
                Some(s) => compare_versions(&e.version, s) == Some(Ordering::Greater),
            }
        })
        .collect();
    picked.sort_by(|a, b| compare_versions(&b.version, &a.version).unwrap_or(Ordering::Equal));
    picked
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LastRun {
    Updated,
    Store,
    Same,
}

/// What a start of `current` makes of the version that last ran: `Updated` only for a newer version; a first run (or
/// settings from before the setting) and a downgrade only store `current`; the same version (build metadata aside) nothing.
pub fn last_run_change(last: Option<&str>, current: &str) -> LastRun {
    let Some(last) = last else { return LastRun::Store };
    let c = compare_versions(current, last);
    if last == current || c == Some(std::cmp::Ordering::Equal) {
        return LastRun::Same;
    }
    if c == Some(std::cmp::Ordering::Greater) { LastRun::Updated } else { LastRun::Store }
}

/// This build's CHANGELOG.md, parsed on first need.
fn changelog() -> &'static [Entry] {
    static LIST: OnceLock<Vec<Entry>> = OnceLock::new();
    LIST.get_or_init(|| parse_changelog(include_str!("../../../CHANGELOG.md")))
}

/// `Nov 12, 2026` for a section's `2026-11-12`; empty when it has none.
fn day(date: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let parts: Vec<&str> = date.split('-').collect();
    match parts[..] {
        [y, m, d] if y.len() == 4 && m.len() == 2 && d.len() == 2 => match (y.parse::<u32>(), m.parse::<usize>(), d.parse::<u32>()) {
            (Ok(y), Ok(m @ 1..=12), Ok(d @ 1..=31)) => format!("{} {d}, {y}", MONTHS[m - 1]),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

/// A section's body one heading level down, under its version's heading.
fn demoted(body: &str) -> String {
    body.split('\n').map(|l| if l.starts_with("### ") { format!("#{l}") } else { l.to_string() }).collect::<Vec<_>>().join("\n")
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

impl GitVolt {
    /// `checkLastRun`: once per session, after the settings loaded. An update since the last run puts an unread "Updated to
    /// GitVolt …" in the bell, and the running version becomes the last-run one.
    pub fn check_last_run(&mut self, cx: &mut Context<Self>) {
        if self.whats_new.checked {
            return;
        }
        self.whats_new.checked = true;
        let current = current_version();
        let last = self.settings["lastRunVersion"].as_str().map(str::to_string);
        match last_run_change(last.as_deref(), current) {
            LastRun::Same => return,
            LastRun::Updated => {
                self.whats_new.notice = Some(Range { version: current.to_string(), since: last.clone() });
                self.whats_new.unread = true;
                self.whats_new.updated_from = last;
            }
            LastRun::Store => {}
        }
        self.set_settings(|s| s["lastRunVersion"] = current.into(), cx);
    }

    /// Help › What's new: this version's notes, every version since the last one when this session updated.
    pub fn show_whats_new_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let range = Range { version: current_version().to_string(), since: self.whats_new.updated_from.clone() };
        self.open_whats_new(range, window, cx);
    }

    /// The What's new panel: the notes for `range`, newest first, each under its version's heading.
    pub fn open_whats_new(&mut self, range: Range, window: &mut Window, cx: &mut Context<Self>) {
        let entries = notes_between(changelog(), range.since.as_deref(), &range.version);
        let sections: Vec<(String, Entity<MdView>)> = entries
            .iter()
            .map(|e| {
                let heading = [e.version.clone(), day(&e.date)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                let opts = MdOptions::for_comment(0, Default::default(), crate::md::ast::Flavor::Github, Default::default());
                let (api, rt, retired) = (self.api.clone(), self.rt.clone(), self.retired_images.clone());
                let text: Arc<str> = demoted(&e.body).into();
                let view = cx.new(|cx| {
                    let mut v = MdView::new(opts, api, rt, retired, cx);
                    v.set_content(Content::Text(text), false, cx);
                    v
                });
                (heading, view)
            })
            .collect();
        let m = Modal::new(DIALOG, move |_, _| {
            let mut notes = div().id("whats-new-notes").flex().flex_col().max_h(px(460.)).min_h(px(60.)).overflow_y_scroll().px(px(12.)).pb(px(8.)).rounded(px(6.)).border_1().border_color(theme::section_border()).bg(theme::app_bg());
            if sections.is_empty() {
                notes = notes.child(div().py(px(8.)).italic().text_color(theme::text_secondary()).child("No notes for this version."));
            }
            for (heading, view) in &sections {
                let (version, date) = heading.split_once(" · ").map_or((heading.as_str(), None), |(v, d)| (v, Some(d)));
                notes = notes
                    .child(
                        div()
                            .mx(px(-12.))
                            .px(px(12.))
                            .pt(px(8.))
                            .pb(px(6.))
                            .border_b_1()
                            .border_color(theme::section_border())
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text_selected())
                            .child(version.to_string())
                            .when_some(date, |d, date| d.child(div().font_weight(FontWeight::NORMAL).text_color(theme::text_secondary()).child(format!(" · {date}"))).flex()),
                    )
                    .child(view.clone());
            }
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(modal::title(format!("What's new in GitVolt {}", range.version)))
                .when_some(range.since.clone(), |d, since| d.child(div().mt(px(-4.)).text_color(theme::text_secondary()).child(format!("Every change since {since}, the version you used before."))))
                .child(notes)
                .child(modal::actions_row().child(crate::ui::anchor("whats-new-close", controls::button("whats-new-close", "Close", ButtonKind::Plain, true).on_click(|_, window, cx| modal::close(DIALOG, window, cx)))))
                .into_any_element()
        })
        .width(680f32.min(window.viewport_size().width.as_f32() - 32.));
        modal::open(m, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;
    use std::prelude::v1::test;

    const FIXTURE: &str = "# Changelog\n\nIntro text.\n\n## [Unreleased]\n\n### Added\n\n- **Not yet.** Unreleased work.\n\n## [1.6.1] - 2026-11-12\n\n### Fixed\n\n- **Clone on Windows.** A drive path works (#26).\n\n## [1.6.0] - 2026-11-06\n\n### Added\n\n- **Emoji completion.** In the commit summary.\n\n## [1.6.0-rc.2] - 2026-11-04\n\n- Second candidate.\n\n## [1.6.0-rc.1] - 2026-11-03\n\n- First candidate.\n\n## Not a version\n\n- Ignored.\n\n## [1.5.1] - 2026-11-05\n\n- A patch.\n\n## [1.5.0] - 2026-11-01\n\n- The base.\n\n[1.5.0]: https://forge.example/compare/v1.4.0...v1.5.0\n";

    #[test]
    fn released_sections_only_with_their_dates_and_trimmed_bodies() {
        let entries = parse_changelog(FIXTURE);
        assert_eq!(entries.iter().map(|e| e.version.as_str()).collect::<Vec<_>>(), ["1.6.1", "1.6.0", "1.6.0-rc.2", "1.6.0-rc.1", "1.5.1", "1.5.0"]);
        assert_eq!(entries[0], Entry { version: "1.6.1".into(), date: "2026-11-12".into(), body: "### Fixed\n\n- **Clone on Windows.** A drive path works (#26).".into() });
        assert!(!entries.iter().any(|e| e.body.contains("Not yet")));
        assert_eq!(entries[3].body, "- First candidate.", "a section ends at any ## heading");
        assert_eq!(entries[5].body, "- The base.", "trailing link references go");
    }

    #[test]
    fn malformed_headings_and_crlf() {
        let text = "## [1.2]\n- bad version\n## [x.y.z] - 2026-01-01\n- bad\n## [1.0.0]\r\n- no date\r\n## [0.9.0] -\n- dash, no date\n";
        let e = |v: &str, b: &str| Entry { version: v.into(), date: String::new(), body: b.into() };
        assert_eq!(parse_changelog(text), [e("1.0.0", "- no date"), e("0.9.0", "- dash, no date")]);
        assert_eq!(parse_changelog(""), []);
    }

    #[test]
    fn the_built_in_changelog_has_every_released_section_dated_with_notes() {
        let list = changelog();
        assert!(list.len() > 10);
        assert!(list.iter().all(|e| parse_version(&e.version).is_some() && !day(&e.date).is_empty() && !e.body.trim().is_empty()));
    }

    #[test]
    fn versions_order_as_semver_ignoring_build_metadata() {
        let c = |a, b| compare_versions(a, b);
        assert_eq!(c("1.6.1", "1.6.0"), Some(Ordering::Greater));
        assert_eq!(c("1.10.0", "1.9.9"), Some(Ordering::Greater));
        assert_eq!(c("1.6.0-rc.1", "1.6.0"), Some(Ordering::Less));
        assert_eq!(c("1.6.0-rc.10", "1.6.0-rc.2"), Some(Ordering::Greater));
        assert_eq!(c("1.6.0-rc", "1.6.0-rc.1"), Some(Ordering::Less));
        assert_eq!(c("1.6.0-1", "1.6.0-alpha"), Some(Ordering::Less));
        assert_eq!(c("v1.6.1+202611121200.abc1234", "1.6.1"), Some(Ordering::Equal));
        assert_eq!(c("1.6", "1.6.0"), None);
        assert_eq!(c("1.06.0", "1.6.0"), None);
        assert_eq!(c("dev", "1.6.0"), None);
    }

    #[test]
    fn the_notes_between_two_versions_newest_first() {
        let entries = parse_changelog(FIXTURE);
        let versions = |since: Option<&str>, current: &str| notes_between(&entries, since, current).into_iter().map(|e| e.version.clone()).collect::<Vec<_>>();
        assert_eq!(versions(Some("1.5.0"), "1.6.1"), ["1.6.1", "1.6.0", "1.6.0-rc.2", "1.6.0-rc.1", "1.5.1"]);
        assert_eq!(versions(Some("1.6.0"), "1.6.1"), ["1.6.1"]);
        assert_eq!(versions(Some("1.6.0-rc.1"), "1.6.0"), ["1.6.0", "1.6.0-rc.2"]);
        assert_eq!(versions(Some("1.5.0"), "1.6.0"), ["1.6.0", "1.6.0-rc.2", "1.6.0-rc.1", "1.5.1"]);
        let shuffled = [entries[4].clone(), entries[0].clone(), entries[2].clone(), entries[1].clone()];
        assert_eq!(notes_between(&shuffled, Some("1.5.0"), "1.6.1").into_iter().map(|e| e.version.as_str()).collect::<Vec<_>>(), ["1.6.1", "1.6.0", "1.6.0-rc.2", "1.5.1"]);
        assert_eq!(versions(None, "1.6.1"), ["1.6.1"]);
        assert_eq!(versions(None, "1.7.0"), Vec::<String>::new());
        assert_eq!(versions(Some("garbage"), "1.6.1"), Vec::<String>::new());
    }

    #[test]
    fn only_a_newer_version_counts_as_an_update() {
        assert_eq!(last_run_change(Some("1.5.0"), "1.6.1"), LastRun::Updated);
        assert_eq!(last_run_change(Some("1.6.0-rc.1"), "1.6.0"), LastRun::Updated);
        assert_eq!(last_run_change(None, "1.6.1"), LastRun::Store);
        assert_eq!(last_run_change(Some("1.7.0"), "1.6.1"), LastRun::Store);
        assert_eq!(last_run_change(Some("garbage"), "1.6.1"), LastRun::Store);
        assert_eq!(last_run_change(Some("1.6.1"), "1.6.1"), LastRun::Same);
        assert_eq!(last_run_change(Some("1.6.1+202611121200.abc1234"), "1.6.1"), LastRun::Same);
    }

    #[test]
    fn a_sections_day_and_its_headings_one_level_down() {
        assert_eq!(day("2026-11-12"), "Nov 12, 2026");
        assert_eq!(day(""), "");
        assert_eq!(day("2026-13-01"), "");
        assert_eq!(demoted("### Fixed\n\n- ### not a heading"), "#### Fixed\n\n- ### not a heading");
    }
}
