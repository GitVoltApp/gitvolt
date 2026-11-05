//! A forge's own note ("added 1 commit", "changed title from …") as structured parts (`systemNote.ts`). GitLab's system notes arrive as
//! Markdown with a little HTML; they're untrusted, so they're tokenized into plain data and drawn as text: nothing is ever parsed as
//! HTML. Unknown tags are dropped (their text stays); a link is kept only when it resolves to http(s). Every scan is bounded by the
//! body's cap and the token limits, so no input costs more than linear time.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteCommit {
    pub sha: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotePart {
    Text(String),
    Code(String),
    Link { text: String, url: String },
    Del(String),
    Ins(String),
    Commits { items: Vec<NoteCommit>, more: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemKind {
    Commits,
    Edit,
    Title,
    Mention,
    Approved,
    Unapproved,
    Merged,
    Closed,
    Reopened,
    Draft,
    Label,
    Other,
    /// Not GitLab's: a review on GitHub's or Forgejo's timeline.
    Review,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemNote {
    pub kind: SystemKind,
    pub parts: Vec<NotePart>,
}

const MAX_BODY: usize = 20_000;
pub const MAX_COMMITS: usize = 50;

fn floor_boundary(s: &str, mut at: usize) -> usize {
    at = at.min(s.len());
    while !s.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The HTML entities in text about to be shown as plain text (once, never nested).
pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let semi = rest[..floor_boundary(rest, 12)].find(';');
        let decoded = semi.and_then(|j| {
            let name = &rest[1..j];
            let c = if let Some(n) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                u32::from_str_radix(n, 16).ok().filter(|_| n.len() <= 6 && !n.is_empty()).and_then(|c| char::from_u32(c).filter(|c| *c != '\0'))
            } else if let Some(n) = name.strip_prefix('#') {
                n.parse::<u32>().ok().filter(|_| n.len() <= 7 && !n.is_empty()).and_then(|c| char::from_u32(c).filter(|c| *c != '\0'))
            } else {
                match name {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some(' '),
                    _ => None,
                }
            };
            c.map(|c| (c, j))
        });
        match decoded {
            Some((c, j)) => {
                out.push(c);
                rest = &rest[j + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn origin(url: &str) -> Option<&str> {
    let (scheme, rest) = url.split_once("://")?;
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    Some(&url[..scheme.len() + 3 + end])
}

/// An http(s) URL, or a `/path` resolved against the MR's web URL and staying on its origin; anything else: `None`. Backslashes, control
/// characters and whitespace are refused: a URL parser treats a backslash as `/` and drops tabs and newlines.
pub fn safe_url(raw: &str, base: &str) -> Option<String> {
    let url = decode_entities(raw.trim());
    if url.chars().any(|c| c == '\\' || c.is_whitespace() || c.is_control()) {
        return None;
    }
    if origin(&url).is_some_and(|o| o.len() > url.split_once("://").map_or(0, |x| x.0.len()) + 3) {
        return Some(url);
    }
    if url.starts_with('/') && !url.starts_with("//") {
        return origin(base).map(|o| format!("{o}{url}"));
    }
    None
}

pub fn classify(text: &str) -> SystemKind {
    let t = text.trim().to_lowercase();
    let added_commits = t.strip_prefix("added ").is_some_and(|r| {
        let digits = r.chars().take_while(char::is_ascii_digit).count();
        digits > 0 && r[digits..].trim_start().starts_with("commit") && r[digits..].starts_with(' ')
    });
    if added_commits {
        return SystemKind::Commits;
    }
    if t.starts_with("changed title") {
        return SystemKind::Title;
    }
    if ["changed the description", "edited the description", "updated the description", "description changed", "description edited", "changed this line"].iter().any(|p| t.starts_with(p)) {
        return SystemKind::Edit;
    }
    if t.starts_with("mentioned in") {
        return SystemKind::Mention;
    }
    if t.starts_with("unapproved") || t.starts_with("requested changes") || ["removed their approval", "removed an approval", "removed a approval", "removed approval", "revoked their approval", "revoked an approval", "revoked a approval", "revoked approval"].iter().any(|p| t.starts_with(p)) {
        return SystemKind::Unapproved;
    }
    if t.starts_with("approved") {
        return SystemKind::Approved;
    }
    let word = |w: &str| t.starts_with(w) && t[w.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_');
    if word("merged") {
        return SystemKind::Merged;
    }
    if word("closed") {
        return SystemKind::Closed;
    }
    if word("reopened") {
        return SystemKind::Reopened;
    }
    let has_word = |w: &str| t.match_indices(w).any(|(i, _)| {
        let before = t[..i].chars().next_back().is_none_or(|c| !c.is_alphanumeric() && c != '_');
        let after = t[i + w.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_');
        before && after
    });
    if (t.starts_with("marked ") && (has_word("draft") || has_word("ready"))) || ((t.starts_with("set ") || t.starts_with("removed ")) && has_word("draft")) {
        return SystemKind::Draft;
    }
    if has_word("label") || has_word("labels") {
        return SystemKind::Label;
    }
    SystemKind::Other
}

fn squash(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    out
}

fn is_sha_range(s: &str) -> bool {
    let one = |p: &str| (7..=40).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    for sep in ["…", "..."] {
        if let Some((a, b)) = s.split_once(sep) {
            return one(a) && one(b);
        }
    }
    one(s)
}

fn commit_line(text: &str) -> NoteCommit {
    if let Some((sha, rest)) = text.split_once(" - ")
        && is_sha_range(sha)
    {
        return NoteCommit { sha: sha.to_string(), subject: rest.to_string() };
    }
    NoteCommit { sha: String::new(), subject: text.to_string() }
}

/// GitLab's Markdown title change: the two bold groups are the whole old and new titles.
fn title_change(body: &str) -> Option<(String, String)> {
    let t = body.trim().strip_prefix("changed title from **")?;
    let (a, b) = t.split_once("** to **")?;
    let b = b.strip_suffix("**")?;
    if a.len() > 500 || b.len() > 500 || a.contains('\n') || b.contains('\n') {
        return None;
    }
    let unmark = |s: &str| decode_entities(&s.replace("{-", "").replace("-}", "").replace("{+", "").replace("+}", ""));
    Some((unmark(a), unmark(b)))
}

pub fn parse_system_note(raw: &str, base: &str) -> SystemNote {
    let body = &raw[..floor_boundary(raw, MAX_BODY)];
    if let Some((old, new)) = title_change(body) {
        return SystemNote { kind: SystemKind::Title, parts: vec![NotePart::Text("changed title from ".into()), NotePart::Del(old), NotePart::Text(" to ".into()), NotePart::Ins(new)] };
    }
    let no_links = body.trim_start().to_lowercase().starts_with("changed title");
    // Markdown's commit bullets as the HTML form.
    let mut src = String::with_capacity(body.len());
    for (i, line) in body.replace("**", "").split('\n').enumerate() {
        if i > 0 {
            src.push('\n');
        }
        let t = line.trim_start_matches([' ', '\t']);
        let bullet = t.strip_prefix("* ").or_else(|| t.strip_prefix("- "));
        match bullet.filter(|b| b.split_once(" - ").is_some_and(|(s, _)| is_sha_range(s))) {
            Some(b) => {
                src.push_str("<li>");
                src.push_str(b);
                src.push_str("</li>");
            }
            None => src.push_str(line),
        }
    }
    let mut parts: Vec<NotePart> = Vec::new();
    let mut li: Option<String> = None;
    let mut cap: Option<(bool, String)> = None;
    let mut idiffs = 0;
    fn add_text(t: &str, parts: &mut Vec<NotePart>, li: &mut Option<String>, cap: &mut Option<(bool, String)>) {
        if t.is_empty() {
            return;
        }
        if let Some((_, c)) = cap {
            c.push_str(t);
        } else if let Some(l) = li {
            l.push_str(t);
        } else if t.trim().is_empty() && matches!(parts.last(), Some(NotePart::Commits { .. })) {
            // The gap after a list.
        } else {
            parts.push(NotePart::Text(t.to_string()));
        }
    }
    fn flush_li(parts: &mut Vec<NotePart>, li: &mut Option<String>) {
        let Some(text) = li.take() else { return };
        let text = squash(&text);
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let item = commit_line(text);
        if let Some(NotePart::Commits { items, more }) = parts.last_mut() {
            if items.len() < MAX_COMMITS {
                items.push(item);
            } else {
                *more += 1;
            }
        } else {
            parts.push(NotePart::Commits { items: vec![item], more: 0 });
        }
    }
    let bytes = src.as_bytes();
    let (mut i, mut at) = (0usize, 0usize);
    while i < bytes.len() {
        let c = bytes[i];
        let flush = |parts: &mut Vec<NotePart>, li: &mut Option<String>, cap: &mut Option<(bool, String)>, end: usize| add_text(&decode_entities(&squash(&src[at..end])), parts, li, cap);
        if c == b'<' {
            // <(/?)name attrs{0,200}>
            let mut j = i + 1;
            let closing = bytes.get(j) == Some(&b'/');
            if closing {
                j += 1;
            }
            let name_start = j;
            while j < bytes.len() && bytes[j].is_ascii_alphanumeric() {
                j += 1;
            }
            let name_ok = j > name_start && bytes[name_start].is_ascii_alphabetic();
            let attrs_start = j;
            let limit = (attrs_start + 201).min(bytes.len());
            let gt = bytes[attrs_start..limit].iter().position(|b| *b == b'>').map(|p| attrs_start + p);
            let boundary = name_ok && bytes.get(attrs_start).is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'_'));
            if let (true, Some(gt)) = (boundary, gt) {
                flush(&mut parts, &mut li, &mut cap, i);
                let tag = src[name_start..attrs_start].to_ascii_lowercase();
                let attrs = &src[attrs_start..gt];
                match tag.as_str() {
                    "li" => {
                        flush_li(&mut parts, &mut li);
                        if !closing {
                            li = Some(String::new());
                        }
                    }
                    "ul" | "ol" => flush_li(&mut parts, &mut li),
                    "code" if !closing => cap = Some((attrs.contains("idiff"), String::new())),
                    "code" => {
                        if let Some((idiff, text)) = cap.take() {
                            if idiff {
                                if !text.is_empty() {
                                    parts.push(if idiffs == 0 { NotePart::Del(text) } else { NotePart::Ins(text) });
                                    idiffs += 1;
                                }
                            } else if let Some(l) = li.as_mut() {
                                l.push_str(&text);
                            } else if !text.is_empty() {
                                parts.push(NotePart::Code(text));
                            }
                        }
                    }
                    "br" => add_text(" ", &mut parts, &mut li, &mut cap),
                    // Every other tag (span, p, a, strong, script, …) is dropped; its text stays.
                    _ => {}
                }
                i = gt + 1;
                at = i;
                continue;
            }
        } else if c == b'[' {
            let end = (i + 1 + 301).min(bytes.len());
            let close = bytes[i + 1..end].iter().position(|b| *b == b']' || *b == b'\n').map(|p| i + 1 + p);
            if let Some(k) = close.filter(|k| bytes[*k] == b']' && bytes.get(*k + 1) == Some(&b'(')) {
                let url_start = k + 2;
                let limit = (url_start + 501).min(bytes.len());
                let paren = bytes[url_start..limit].iter().position(|b| *b == b')' || b.is_ascii_whitespace()).map(|p| url_start + p);
                if let Some(p) = paren.filter(|p| bytes[*p] == b')') {
                    flush(&mut parts, &mut li, &mut cap, i);
                    let text = decode_entities(&src[i + 1..k]);
                    match safe_url(&src[url_start..p], base).filter(|_| !no_links && cap.is_none() && li.is_none()) {
                        Some(url) => {
                            let t = if text.is_empty() { url.clone() } else { text };
                            parts.push(NotePart::Link { text: t, url });
                        }
                        None => add_text(&text, &mut parts, &mut li, &mut cap),
                    }
                    i = p + 1;
                    at = i;
                    continue;
                }
            }
        } else if (c == b'{') && matches!(bytes.get(i + 1), Some(b'-' | b'+')) {
            let sign = bytes[i + 1];
            let end = (i + 2 + 303).min(bytes.len());
            let close = (i + 2..end.saturating_sub(1)).find(|k| (bytes[*k] == sign && bytes[*k + 1] == b'}') || bytes[*k] == b'\n').filter(|k| bytes[*k] != b'\n');
            if let Some(k) = close {
                flush(&mut parts, &mut li, &mut cap, i);
                let text = decode_entities(&src[i + 2..k]);
                parts.push(if sign == b'-' { NotePart::Del(text) } else { NotePart::Ins(text) });
                i = k + 2;
                at = i;
                continue;
            }
        } else if c == b'`' {
            let end = (i + 1 + 201).min(bytes.len());
            let close = bytes[i + 1..end].iter().position(|b| *b == b'`' || *b == b'\n').map(|p| i + 1 + p);
            if let Some(k) = close.filter(|k| bytes[*k] == b'`' && *k > i + 1) {
                flush(&mut parts, &mut li, &mut cap, i);
                let text = decode_entities(&src[i + 1..k]);
                if cap.is_some() || li.is_some() {
                    add_text(&text, &mut parts, &mut li, &mut cap);
                } else {
                    parts.push(NotePart::Code(text));
                }
                i = k + 1;
                at = i;
                continue;
            }
        }
        i += 1;
    }
    add_text(&decode_entities(&squash(&src[at..])), &mut parts, &mut li, &mut cap);
    if let Some((_, rest)) = cap.take()
        && !rest.is_empty()
    {
        parts.push(NotePart::Text(rest));
    }
    flush_li(&mut parts, &mut li);
    if let Some(NotePart::Text(t)) = parts.first_mut() {
        *t = t.trim_start().to_string();
    }
    if let Some(NotePart::Text(t)) = parts.last_mut() {
        *t = t.trim_end().to_string();
    }
    parts.retain(|p| !matches!(p, NotePart::Text(t) if t.is_empty()));
    let lead: String = parts.iter().map(|p| match p {
        NotePart::Text(t) | NotePart::Code(t) => t.as_str(),
        _ => "",
    }).collect();
    SystemNote { kind: classify(&lead), parts }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    const BASE: &str = "https://gitlab.example.com/g/p/-/merge_requests/1";

    #[test]
    fn it_reads_the_forms_gitlab_sends() {
        let n = parse_system_note("added 2 commits\n\n<ul><li>abc1234 - One &amp; two</li><li>def5678 - Three</li></ul>\n\n[Compare with previous version](/g/p/-/merge_requests/1/diffs)", BASE);
        assert_eq!(n.kind, SystemKind::Commits);
        assert_eq!(n.parts[0], NotePart::Text("added 2 commits".into()));
        assert_eq!(n.parts[1], NotePart::Commits { items: vec![NoteCommit { sha: "abc1234".into(), subject: "One & two".into() }, NoteCommit { sha: "def5678".into(), subject: "Three".into() }], more: 0 });
        assert_eq!(n.parts[2], NotePart::Link { text: "Compare with previous version".into(), url: "https://gitlab.example.com/g/p/-/merge_requests/1/diffs".into() });
    }

    #[test]
    fn a_title_change_says_both_titles_and_never_links() {
        let n = parse_system_note("changed title from **Old {-one-}** to **New {+two+}**", BASE);
        assert_eq!(n.kind, SystemKind::Title);
        assert_eq!(n.parts, vec![NotePart::Text("changed title from ".into()), NotePart::Del("Old one".into()), NotePart::Text(" to ".into()), NotePart::Ins("New two".into())]);
    }

    #[test]
    fn nothing_in_a_note_can_leave_as_markup_or_a_bad_link() {
        let n = parse_system_note("<script>alert(1)</script> approved <img src=x onerror=y> [click](javascript:alert(1)) [out](//evil.example/a) [slash](/\\evil.example)", BASE);
        let text: String = n.parts.iter().map(|p| match p {
            NotePart::Text(t) => t.clone(),
            NotePart::Link { url, .. } => panic!("{url}"),
            _ => String::new(),
        }).collect();
        assert!(!text.contains('<'), "{text}");
        assert_eq!(safe_url("https://a.example/x", BASE).as_deref(), Some("https://a.example/x"));
        assert_eq!(safe_url("ftp://a.example/x", BASE), None);
        assert_eq!(safe_url("/x y", BASE), None);
    }

    #[test]
    fn its_cost_is_bounded() {
        let long = "<".repeat(200_000) + &"[".repeat(200_000) + &"`".repeat(200_000);
        let started = std::time::Instant::now();
        let n = parse_system_note(&long, BASE);
        assert!(started.elapsed().as_secs() < 5);
        assert!(n.parts.iter().map(|p| if let NotePart::Text(t) = p { t.len() } else { 0 }).sum::<usize>() <= MAX_BODY);
    }

    #[test]
    fn it_classifies_by_what_the_text_says() {
        assert_eq!(classify("mentioned in commit abc"), SystemKind::Mention);
        assert_eq!(classify("approved this merge request"), SystemKind::Approved);
        assert_eq!(classify("unapproved this merge request"), SystemKind::Unapproved);
        assert_eq!(classify("merged"), SystemKind::Merged);
        assert_eq!(classify("marked this merge request as ready"), SystemKind::Draft);
        assert_eq!(classify("added ~bug label"), SystemKind::Label);
        assert_eq!(classify("assigned to @ada"), SystemKind::Other);
    }
}
