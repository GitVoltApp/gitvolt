//! `fuzzysort` 4.0's ranking, as the palette uses it: `search` matches a query's characters in order,
//! scored as the library does (word beginnings count, a substring counts more, late and long is
//! worse; spaces split the query into parts), `threshold` on its normalized 0..1 score.

const INF: f64 = f64::INFINITY;
const BACKTRACK_LIMIT: usize = 200;

/// `fuzzysort.prepare`: what a target needs for every query.
#[derive(Debug, Clone)]
pub struct Target {
    lower: Vec<u16>,
    next: Vec<usize>,
    beginnings: usize,
    has_space: bool,
}

/// The same character folding as the library's `remapChars` for what matters here: an accent drops,
/// and the UTF-16 units are what indexes count.
fn remap(c: char) -> char {
    match c {
        'à'..='å' | 'À'..='Å' => 'a',
        'è'..='ë' | 'È'..='Ë' => 'e',
        'ì'..='ï' | 'Ì'..='Ï' => 'i',
        'ò'..='ö' | 'Ò'..='Ö' => 'o',
        'ù'..='ü' | 'Ù'..='Ü' => 'u',
        'ç' | 'Ç' => 'c',
        'ñ' | 'Ñ' => 'n',
        c => c,
    }
}

fn codes(text: &str) -> (Vec<u16>, Vec<u16>) {
    let mut original = Vec::with_capacity(text.len());
    let mut buf = [0u16; 2];
    for c in text.chars() {
        original.extend_from_slice(remap(c).encode_utf16(&mut buf));
    }
    let lower = original.iter().map(|&u| if (65..=90).contains(&u) { u + 32 } else { char::from_u32(u32::from(u)).map_or(u, |c| c.to_lowercase().next().map_or(u, |l| l as u16)) }).collect();
    (original, lower)
}

impl Target {
    pub fn new(text: &str) -> Self {
        let (original, lower) = codes(text);
        let mut next = Vec::with_capacity(original.len());
        let (mut was_upper, mut was_alnum, mut fill, mut count) = (false, false, 0, 0);
        for (i, &code) in original.iter().enumerate() {
            let upper = (65..=90).contains(&code);
            let alnum = upper || (97..=122).contains(&code) || (48..=57).contains(&code);
            let beginning = (upper && !was_upper) || !was_alnum || !alnum;
            was_upper = upper;
            was_alnum = alnum;
            if beginning {
                count += 1;
                while fill < i {
                    next.push(i);
                    fill += 1;
                }
            }
        }
        while fill < original.len() {
            next.push(original.len());
            fill += 1;
        }
        Self { has_space: lower.contains(&32), lower, next, beginnings: count }
    }
}

fn find(hay: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    if needle.is_empty() || from > hay.len() {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|i| i + from)
}

fn calculate(matches: &[usize], target_len: usize, strict: bool, beginnings: usize, substring: bool, start: bool, end: bool) -> f64 {
    let search_len = matches.len();
    let (mut score, mut groups) = (0f64, 0f64);
    for i in 1..search_len {
        if matches[i] - matches[i - 1] != 1 {
            score -= matches[i] as f64;
            groups += 1.;
        }
    }
    let unmatched = (matches[search_len - 1] - matches[0] + 1 - search_len) as f64;
    score -= (12. + unmatched) * groups;
    if matches[0] != 0 {
        score -= (matches[0] * matches[0]) as f64 * 0.2;
    }
    if !strict {
        score *= 1000.;
    } else if beginnings > 24 {
        score *= ((beginnings - 24) * 10) as f64;
    }
    let penalty = (target_len as f64 - search_len as f64) / 2.;
    score -= penalty;
    let bonus = 1. + (search_len * search_len) as f64;
    if substring {
        score /= bonus;
    }
    if start {
        score /= bonus;
    }
    score -= penalty;
    if end { score + 4f64.min(-score / 2.) } else { score }
}

/// One part's match, or none. `next`: the target's beginnings, with what earlier parts changed.
fn algorithm(search: &[u16], t: &Target, next: &[usize]) -> Option<(f64, Vec<usize>)> {
    let (slen, tlen) = (search.len(), t.lower.len());
    if slen == 0 {
        return None;
    }
    let mut simple = Vec::with_capacity(slen);
    let mut at = 0;
    for &c in search {
        loop {
            if at >= tlen {
                return None;
            }
            if t.lower[at] == c {
                simple.push(at);
                at += 1;
                break;
            }
            at += 1;
        }
    }
    if slen == 1 {
        let code = search[0];
        let mut i = if simple[0] == 0 { 0 } else { next[simple[0] - 1] };
        while i < tlen && t.lower[i] != code {
            i = next[i];
        }
        let strict = i < tlen;
        let m = if strict { i } else { simple[0] };
        let mut score = if m == 0 { 0. } else { -((m * m) as f64) * 0.2 };
        if !strict {
            score *= 1000.;
        } else if t.beginnings > 24 {
            score *= ((t.beginnings - 24) * 10) as f64;
        }
        return Some((score - (tlen as f64 - 1.), vec![m]));
    }
    let mut strict_match: Vec<usize> = Vec::with_capacity(slen);
    let mut strict = false;
    let mut substring_i = find(&t.lower, search, simple[0]);
    let is_substring = substring_i.is_some();
    let mut substring_start = substring_i.is_some_and(|i| i == 0 || next[i - 1] == i);
    if substring_start && substring_i == Some(simple[0]) {
        strict = true;
    } else {
        let mut ti = if simple[0] == 0 { 0 } else { next[simple[0] - 1] };
        let (mut si, mut backtracks) = (0, 0);
        if ti != tlen {
            loop {
                if ti >= tlen {
                    if si == 0 {
                        break;
                    }
                    backtracks += 1;
                    if backtracks > BACKTRACK_LIMIT {
                        break;
                    }
                    si -= 1;
                    let previous = strict_match.pop().expect("one per matched part");
                    ti = next[previous];
                } else if search[si] == t.lower[ti] {
                    strict_match.push(ti);
                    si += 1;
                    if si == slen {
                        strict = true;
                        break;
                    }
                    ti += 1;
                } else {
                    ti = next[ti];
                }
            }
        }
    }
    if is_substring && !substring_start {
        let found = substring_i.unwrap_or(0);
        let mut i = 0;
        while i < next.len() {
            if i > found && t.lower[i..].starts_with(search) {
                substring_i = Some(i);
                substring_start = true;
                break;
            }
            i = next[i];
        }
    }
    let matches: Vec<usize> = if !strict {
        match substring_i {
            Some(s) => (0..slen).map(|i| s + i).collect(),
            None => simple,
        }
    } else if substring_start {
        let s = substring_i.unwrap_or(0);
        (0..slen).map(|i| s + i).collect()
    } else {
        strict_match
    };
    let end = matches[slen - 1] + 1;
    let substring_end = is_substring && end - matches[0] == slen && (end == tlen || next[end - 1] == end);
    Some((calculate(&matches, tlen, strict, t.beginnings, is_substring, substring_start, substring_end), matches))
}

pub struct Query {
    lower: Vec<u16>,
    parts: Vec<Vec<u16>>,
    has_space: bool,
}

impl Query {
    pub fn new(text: &str) -> Self {
        let trimmed = text.trim();
        let (_, lower) = codes(trimmed);
        let has_space = lower.contains(&32);
        let mut parts: Vec<Vec<u16>> = Vec::new();
        if has_space {
            for p in trimmed.split_whitespace() {
                let (_, l) = codes(p);
                if !parts.contains(&l) {
                    parts.push(l);
                }
            }
        }
        Self { lower, parts, has_space }
    }
}

/// `algorithmSpaces`: every part in turn, a consecutive part's end counting as a beginning for the
/// next, and the whole query with its spaces when the target has them and that scores better.
fn spaces(q: &Query, t: &Target) -> Option<f64> {
    let mut next = t.next.clone();
    let (mut score, mut previous_first) = (0f64, 0usize);
    let n = q.parts.len() as f64;
    let count = q.parts.len();
    for (i, part) in q.parts.iter().enumerate() {
        let (s, indexes) = algorithm(part, t, &next)?;
        if i + 1 < count && indexes.windows(2).all(|w| w[1] - w[0] == 1) {
            let beginning = indexes[indexes.len() - 1] + 1;
            if beginning <= next.len() {
                let previous = next[beginning - 1];
                let mut j = beginning as i64 - 1;
                while j >= 0 && next[j as usize] == previous {
                    next[j as usize] = beginning;
                    j -= 1;
                }
            }
        }
        score += s / n;
        if indexes[0] < previous_first {
            score -= ((previous_first - indexes[0]) * 2) as f64;
        }
        previous_first = indexes[0];
    }
    if t.has_space
        && let Some((whole, _)) = algorithm(&q.lower, t, &t.next)
        && whole > score
    {
        return Some(whole);
    }
    Some(score)
}

/// The raw score of `t` for `q`, or none.
pub fn score(q: &Query, t: &Target) -> Option<f64> {
    if q.has_space { spaces(q, t) } else { algorithm(&q.lower, t, &t.next).map(|r| r.0) }
}

/// The library's `normalizeScore`.
pub fn normalize(value: f64) -> f64 {
    if value == -INF {
        0.
    } else if value > 1. {
        value
    } else {
        std::f64::consts::E.powf(((-value + 1.).powf(0.04307) - 1.) * -2.)
    }
}

/// `fuzzysort.go`: the indexes of `targets` that match, best first, at most `limit`, above `threshold`
/// (normalized). Equal scores keep no particular order in the library; here, the targets' order.
pub fn go<'a>(query: &str, targets: impl Iterator<Item = &'a Target>, limit: usize, threshold: f64) -> Vec<usize> {
    let q = Query::new(query);
    let mut out: Vec<(f64, usize)> = targets.enumerate().filter_map(|(i, t)| score(&q, t).filter(|s| normalize(*s) >= threshold).map(|s| (s, i))).collect();
    out.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    out.truncate(limit);
    out.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn ranked(q: &str, list: &[&str]) -> Vec<String> {
        let targets: Vec<Target> = list.iter().map(|s| Target::new(s)).collect();
        go(q, targets.iter(), 50, 0.3).into_iter().map(|i| list[i].to_string()).collect()
    }

    // Values from the library itself: `fuzzysort.single(q, t).score` in Node.
    #[test]
    fn it_scores_as_the_library() {
        let close = |q: &str, t: &str, want: f64| {
            let got = normalize(score(&Query::new(q), &Target::new(t)).unwrap());
            assert!((got - want).abs() < 1e-6, "{q} in {t}: {got} vs {want}");
        };
        close("fet", "Fetch all", 0.8836132781655108);
        close("fa", "Fetch all", 0.7270222893385028);
        close("zz", "fuzz", 0.6142065597655855);
        close("f", "Fetch all", 0.8199483535594311);
        close("he wo", "hello world", 0.8541186278834809);
        close("cp", "Scope", 0.35478039925289334);
        close("cp", "Command palette", 0.7068836086003896);
        close("cp", "Compare", 0.3525438820776805);
        close("graph", "Find in graph", 0.9072996944371321);
        close("graph", "Graph", 1.);
        close("all fet", "Fetch all", 0.779363674820767);
        close("fpg", "feature/page.ts", 0.29888108083637255);
        close("main", "src/main.rs", 0.9144394810453171);
        close("fb", "fooBar", 0.7521648683612847);
    }

    #[test]
    fn a_word_start_outranks_the_middle_and_a_miss_is_out() {
        assert_eq!(ranked("cp", &["Scope", "Command palette", "Compare"]), ["Command palette", "Scope", "Compare"]);
        assert!(ranked("xyz", &["Fetch"]).is_empty());
        assert_eq!(ranked("graph", &["Find in graph", "Graph"]), ["Graph", "Find in graph"]);
    }

    #[test]
    fn spaces_split_the_query() {
        assert_eq!(ranked("all fet", &["Fetch all"]), ["Fetch all"]);
        assert!(ranked("all xyz", &["Fetch all"]).is_empty());
    }

    #[test]
    fn camel_case_counts_as_beginnings() {
        let t = Target::new("fooBar");
        assert_eq!(t.beginnings, 2);
        assert_eq!(t.next[0], 3);
    }
}
