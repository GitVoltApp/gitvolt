//! The only forge-specific words the UI uses (`labels.ts`), what an MR/PR's badge, card and view say (`mrText.ts`), and a
//! host's forge by its name (`hostKind.ts`, `urls.ts`).

use gitvolt_core::forge::types::{ForgeMr, ForgePipeline, MrState, PipelineStatus, ReviewDecision, ReviewState, ReviewSummary};
use gitvolt_core::forge::ForgeKind;

pub fn forge_name(k: ForgeKind) -> &'static str {
    k.name()
}

pub fn mr_noun(k: ForgeKind) -> &'static str {
    if k == ForgeKind::GitLab { "MR" } else { "PR" }
}

pub fn mr_ref(k: ForgeKind, n: u64) -> String {
    format!("{}{n}", if k == ForgeKind::GitLab { '!' } else { '#' })
}

pub fn mr_section_label(k: ForgeKind) -> &'static str {
    if k == ForgeKind::GitLab { "Merge requests" } else { "Pull requests" }
}

pub fn mr_name(k: ForgeKind) -> &'static str {
    if k == ForgeKind::GitLab { "Merge request" } else { "Pull request" }
}

pub fn pipeline_noun(k: ForgeKind) -> &'static str {
    if k == ForgeKind::GitLab { "Pipeline" } else { "Checks" }
}

pub fn mr_long_noun(k: ForgeKind) -> &'static str {
    if k == ForgeKind::GitLab { "merge request" } else { "pull request" }
}

pub fn state_label(s: MrState) -> &'static str {
    match s {
        MrState::Open => "Open",
        MrState::Draft => "Draft",
        MrState::Merged => "Merged",
        MrState::Merging => "Merging…",
        MrState::Closed => "Closed",
    }
}

fn pipeline_word_of(s: PipelineStatus) -> &'static str {
    match s {
        PipelineStatus::Pending => "pending",
        PipelineStatus::Running => "running",
        PipelineStatus::Success => "passed",
        PipelineStatus::Failed => "failed",
        PipelineStatus::Canceled => "canceled",
        PipelineStatus::Skipped => "skipped",
        PipelineStatus::Manual => "waiting for a manual job",
    }
}

pub fn pipeline_text(k: ForgeKind, p: Option<&ForgePipeline>) -> String {
    match p {
        Some(p) => format!("{} {}", pipeline_noun(k), pipeline_word_of(p.status)),
        None => format!("No {}", pipeline_noun(k).to_lowercase()),
    }
}

/// A pipeline's one word for the header's fact tile.
pub fn pipeline_word(p: Option<&ForgePipeline>) -> String {
    let Some(p) = p else { return "None".to_string() };
    let w = pipeline_word_of(p.status);
    let mut c = w.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
}

fn names(r: &ReviewSummary, state: ReviewState) -> String {
    r.reviews.iter().filter(|x| x.state == state).map(|x| x.user.name.as_str()).collect::<Vec<_>>().join(", ")
}

pub fn review_text(r: &ReviewSummary) -> String {
    let count = r.approvals_required.filter(|n| *n > 0).map(|n| format!("{} of {n} approval{}", r.approvals, if n == 1 { "" } else { "s" }));
    match r.decision {
        ReviewDecision::ChangesRequested => {
            let who = names(r, ReviewState::ChangesRequested);
            format!("Changes requested by {}", if who.is_empty() { "a reviewer" } else { &who })
        }
        ReviewDecision::Approved => count.unwrap_or_else(|| format!("Approved by {}", names(r, ReviewState::Approved))),
        ReviewDecision::ReviewRequired => {
            if let Some(c) = count {
                return c;
            }
            let (pending, reviewed) = (names(r, ReviewState::Pending), names(r, ReviewState::Commented));
            if !pending.is_empty() {
                return if reviewed.is_empty() { format!("Review requested from {pending}") } else { format!("Reviewed by {reviewed}; review requested from {pending}") };
            }
            // A comment doesn't meet the rule: the merge still waits for a review.
            if reviewed.is_empty() { "Review required".to_string() } else { format!("Reviewed by {reviewed}; review required") }
        }
        ReviewDecision::None => {
            if r.approvals > 0 {
                return format!("Approved by {}", names(r, ReviewState::Approved));
            }
            // A review that only commented decides nothing, but it was given.
            let reviewed = names(r, ReviewState::Commented);
            if reviewed.is_empty() { "No reviews yet".to_string() } else { format!("Reviewed by {reviewed}") }
        }
    }
}

/// A project path's owner: its namespace (`group/sub`), or a GitHub login.
pub fn owner_of(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |x| x.0)
}

/// The two sides of `dev → main`; from a fork, `alice:fix → main`.
pub fn branches_parts(mr: &ForgeMr) -> (String, String) {
    let fork = !mr.source_project.is_empty() && mr.source_project != mr.target_project;
    (if fork { format!("{}:{}", owner_of(&mr.source_project), mr.source_branch) } else { mr.source_branch.clone() }, mr.target_branch.clone())
}

/// The host a typed `input` names, as the core's `normalize_host` reads it.
pub fn bare_host(input: &str) -> String {
    let s = input.trim();
    let rest = s.split_once("://").map_or(s, |x| x.1);
    rest.split(['/', '?', '#']).next().unwrap_or_default().to_lowercase()
}

/// A host's forge by its name: `"gitlab" | "github" | "forgejo" | "generic"`, the core's `host_kind`.
pub fn detect_host_kind(host: &str) -> &'static str {
    let h = host.to_lowercase();
    if h == "github.com" || h.ends_with(".github.com") {
        return "github";
    }
    if h == "gitlab.com" || h.split('.').any(|l| l.contains("gitlab")) {
        return "gitlab";
    }
    let name = h.split(':').next().unwrap_or_default();
    if name == "codeberg.org" || name.split('.').any(|l| l.contains("forgejo") || l.contains("gitea")) {
        return "forgejo";
    }
    "generic"
}

/// The profile's override for a host (hosts compare case-insensitively), else what was detected.
pub fn effective_kind<'a>(host: &str, detected: &'a str, overrides: &'a serde_json::Value) -> &'a str {
    overrides[host].as_str().or_else(|| overrides[host.to_lowercase().as_str()].as_str()).unwrap_or(detected)
}

pub fn kind_of(s: &str) -> Option<ForgeKind> {
    match s {
        "gitlab" => Some(ForgeKind::GitLab),
        "github" => Some(ForgeKind::GitHub),
        "forgejo" => Some(ForgeKind::Forgejo),
        _ => None,
    }
}

/// `chipText`: black or white, whichever reads better on `#rrggbb` (WCAG relative luminance, as GitHub picks it).
pub fn chip_text_is_black(color: &str) -> Option<bool> {
    let hex = color.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let lin = |i: usize| {
        let c = u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0) as f32 / 255.;
        if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    Some(0.2126 * lin(0) + 0.7152 * lin(2) + 0.0722 * lin(4) > 0.4)
}

/// A host's forge mark icon.
pub fn mark_icon(k: ForgeKind) -> &'static str {
    match k {
        ForgeKind::GitLab => "mark-gitlab",
        ForgeKind::GitHub => "mark-github",
        ForgeKind::Forgejo => "mark-forgejo",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_hosts_forge_is_read_from_its_name() {
        assert_eq!(detect_host_kind("github.com"), "github");
        assert_eq!(detect_host_kind("gitlab.example.com"), "gitlab");
        assert_eq!(detect_host_kind("git.example.dev"), "generic");
        assert_eq!(detect_host_kind("codeberg.org"), "forgejo");
        assert_eq!(detect_host_kind("forgejo.example.com:3000"), "forgejo");
        assert_eq!(bare_host(" https://Git.Example.dev/a?b "), "git.example.dev");
    }

    #[test]
    fn a_label_chip_takes_the_text_that_reads_on_it() {
        assert_eq!(chip_text_is_black("#d73a4a"), Some(false));
        assert_eq!(chip_text_is_black("ededed"), Some(true));
        assert_eq!(chip_text_is_black("nope"), None);
    }
}
