//! `attachments.ts`: GitHub's `user-attachments` URLs, for the signed ones the full body has. Never one inside code.

use std::collections::HashSet;
use std::sync::OnceLock;

fn attachment_re() -> &'static onig::Regex {
    static RE: OnceLock<onig::Regex> = OnceLock::new();
    RE.get_or_init(|| onig::Regex::with_options(r#"https?://[^\s<>()"'\]]+?/user-attachments/assets/([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})"#, onig::RegexOptions::REGEX_OPTION_IGNORECASE, onig::Syntax::default()).expect("a valid pattern"))
}

fn signed_re() -> &'static onig::Regex {
    static RE: OnceLock<onig::Regex> = OnceLock::new();
    RE.get_or_init(|| onig::Regex::with_options(r#"(?:src|href)="(https?://[^"]+?[?&](?:amp;)?jwt=[^"]+)""#, onig::RegexOptions::REGEX_OPTION_IGNORECASE, onig::Syntax::default()).expect("a valid pattern"))
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">")
}

fn fence_of(line: &str) -> Option<(u8, usize, usize)> {
    let lead = line.bytes().take_while(|b| *b == b' ').count();
    if lead > 3 {
        return None;
    }
    let c = *line.as_bytes().get(lead)?;
    if c != b'`' && c != b'~' {
        return None;
    }
    let n = line[lead..].bytes().take_while(|b| *b == c).count();
    (n >= 3).then_some((c, n, lead + n))
}

/// `body`'s code, as sorted `[start, end)` ranges: fenced blocks to their closing fence or the end, then inline spans.
fn code_ranges(body: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut fence: Option<(u8, usize, usize)> = None;
    let mut pos = 0;
    for line in body.split('\n') {
        let m = fence_of(line);
        if let Some((ch, len, start)) = fence {
            if let Some((c, n, after)) = m
                && c == ch
                && n >= len
                && line[after..].trim().is_empty()
            {
                ranges.push((start, pos + line.len()));
                fence = None;
            }
        } else if let Some((c, n, after)) = m
            && !(c == b'`' && line[after..].contains('`'))
        {
            fence = Some((c, n, pos));
        }
        pos += line.len() + 1;
    }
    if let Some((_, _, start)) = fence {
        ranges.push((start, body.len()));
    }
    // Spans, with the fenced blocks blanked out: a run to the next run of the same length, within a paragraph.
    let in_fence = |at: usize, list: &[(usize, usize)]| list.iter().any(|(s, e)| at >= *s && at < *e);
    let fenced = ranges.clone();
    let b = body.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'`' || in_fence(i, &fenced) {
            i += 1;
            continue;
        }
        let run = b[i..].iter().take_while(|c| **c == b'`').count();
        let mut j = i + run;
        let mut close = None;
        while j < b.len() {
            if b[j] == b'\n' && body[j + 1..].trim_start_matches([' ', '\t']).starts_with('\n') {
                break;
            }
            if b[j] == b'`' {
                let n = b[j..].iter().take_while(|c| **c == b'`').count();
                if n == run && !in_fence(j, &fenced) {
                    close = Some(j + n);
                    break;
                }
                j += n;
                continue;
            }
            j += 1;
        }
        match close {
            Some(end) => {
                ranges.push((i, end));
                i = end;
            }
            None => i += run,
        }
    }
    ranges.sort();
    ranges
}

fn in_ranges(ranges: &[(usize, usize)], at: usize) -> bool {
    let i = ranges.partition_point(|r| r.0 <= at);
    i > 0 && at < ranges[i - 1].1
}

/// `body` with each `user-attachments` URL replaced by its signed URL from `body_html`: by the asset id it carries, else
/// in document order. Unchanged without a match.
pub fn signed_attachments(body: &str, body_html: Option<&str>) -> String {
    let Some(html) = body_html.filter(|h| !h.is_empty() && body.contains("/user-attachments/assets/")) else { return body.to_string() };
    let mut signed: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for caps in signed_re().captures_iter(html) {
        let u = unescape(caps.at(1).unwrap_or_default());
        if seen.insert(u.clone()) {
            signed.push(u);
        }
    }
    if signed.is_empty() {
        return body.to_string();
    }
    let code = code_ranges(body);
    let found: Vec<(usize, usize)> = attachment_re().find_iter(body).collect();
    let mut urls: Vec<&str> = Vec::new();
    for &(a, b) in &found {
        if !in_ranges(&code, a) && !urls.contains(&&body[a..b]) {
            urls.push(&body[a..b]);
        }
    }
    let mut map: Vec<(&str, String)> = Vec::new();
    let mut used = HashSet::new();
    for url in &urls {
        let id = url[url.len() - 36..].to_lowercase();
        if let Some(hit) = signed.iter().find(|s| !used.contains(*s) && s.to_lowercase().contains(&id)) {
            used.insert(hit.clone());
            map.push((url, hit.clone()));
        }
    }
    let mut rest = signed.iter().filter(|s| !used.contains(*s));
    for url in &urls {
        if !map.iter().any(|(u, _)| u == url)
            && let Some(s) = rest.next()
        {
            map.push((url, s.clone()));
        }
    }
    let mut out = String::with_capacity(body.len());
    let mut from = 0;
    for &(a, b) in &found {
        out.push_str(&body[from..a]);
        let whole = &body[a..b];
        match map.iter().find(|(u, _)| *u == whole).filter(|_| !in_ranges(&code, a)) {
            Some((_, s)) => out.push_str(s),
            None => out.push_str(whole),
        }
        from = b;
    }
    out.push_str(&body[from..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    const A: &str = "1b2c3d4e-0000-4000-8000-00000000000a";
    const B: &str = "1b2c3d4e-0000-4000-8000-00000000000b";

    fn att(u: &str) -> String {
        format!("https://github.com/user-attachments/assets/{u}")
    }

    fn signed(n: u32, u: &str) -> String {
        format!("https://private-user-images.githubusercontent.com/583231/{n}-{u}.png?jwt=eyJ.x.y")
    }

    #[test]
    fn each_attachment_takes_the_signed_url_with_its_id() {
        let body = format!("![a]({})\n\n![b]({})\n\nagain ![a]({})", att(A), att(B), att(A));
        let html = format!("<p><a href=\"{}\"><img src=\"{}\"></a><a href=\"{}\"><img src=\"{}\"></a></p>", signed(2, B), signed(2, B), signed(1, A), signed(1, A));
        assert_eq!(signed_attachments(&body, Some(&html)), format!("![a]({})\n\n![b]({})\n\nagain ![a]({})", signed(1, A), signed(2, B), signed(1, A)));
    }

    #[test]
    fn without_ids_it_goes_by_order_and_unescapes() {
        let body = format!("![a]({}) ![b]({})", att(A), att(B));
        let html = r#"<img src="https://private-user-images.githubusercontent.com/1/x.png?jwt=one&amp;v=1"><img src="https://private-user-images.githubusercontent.com/2/y.png?jwt=two">"#;
        assert_eq!(signed_attachments(&body, Some(html)), "![a](https://private-user-images.githubusercontent.com/1/x.png?jwt=one&v=1) ![b](https://private-user-images.githubusercontent.com/2/y.png?jwt=two)");
    }

    #[test]
    fn a_url_in_code_stays_as_written() {
        let body = ["Paste this:".to_string(), "```md".into(), format!("![a]({})", att(A)), "```".into(), format!("Or `{}` inline, or `` {} ``.", att(B), att(A)), "~~~~".into(), att(B), "~~~".into(), "still code".into(), "~~~~".into(), format!("Shown: ![b]({})", att(B))].join("\n");
        let html = r#"<img src="https://private-user-images.githubusercontent.com/1/x.png?jwt=one">"#;
        let want = body.replace(&format!("Shown: ![b]({})", att(B)), "Shown: ![b](https://private-user-images.githubusercontent.com/1/x.png?jwt=one)");
        assert_eq!(signed_attachments(&body, Some(html)), want);
    }

    #[test]
    fn nothing_to_do_leaves_it() {
        assert_eq!(signed_attachments("plain", Some("<img>")), "plain");
        assert_eq!(signed_attachments(&att(A), None), att(A));
    }

    #[test]
    fn long_runs_of_backticks_stay_linear() {
        let body = format!("{} {}", "` ".repeat(100_000), att(A));
        let started = std::time::Instant::now();
        let _ = signed_attachments(&body, Some(&format!("<img src=\"{}\">", signed(1, A))));
        assert!(started.elapsed().as_secs() < 30);
    }
}
