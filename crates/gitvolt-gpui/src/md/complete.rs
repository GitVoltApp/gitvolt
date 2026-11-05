//! What the Markdown field completes (`emojiComplete.ts`, `mentions.ts`): pure, the popup is `field.rs`'s.

use super::emoji;

pub const MAX_HITS: usize = 8;

/// gemoji has no popularity order, so the everyday ones are named here and sort first.
const COMMON: [&str; 20] = ["+1", "thumbsup", "-1", "thumbsdown", "smile", "tada", "heart", "eyes", "rocket", "white_check_mark", "warning", "bug", "fire", "pray", "joy", "thinking", "clap", "sparkles", "x", "memo"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    /// Where the `:` or `@` is.
    pub start: usize,
    pub query: String,
}

fn in_fence(before: &str) -> bool {
    let mut open = false;
    let mut lines: Vec<&str> = before.split('\n').collect();
    lines.pop();
    for line in lines {
        let t = line.trim_start_matches(char::is_whitespace);
        if line.len() - t.len() <= 3 && (t.starts_with("```") || t.starts_with("~~~")) {
            open = !open;
        }
    }
    open
}

/// Whether what's at `start` is inside fenced code or an inline code span.
pub fn in_code(before: &str, start: usize) -> bool {
    if in_fence(before) {
        return true;
    }
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    before[line_start..start].matches('`').count() % 2 == 1
}

fn opens(c: Option<char>) -> bool {
    c.is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '[' | '{'))
}

/// The shortcode being typed before `caret`: the colon starts the text or follows whitespace or an opening bracket (so
/// not `http://`, `12:30` or `a:b`), at least two word characters follow it, and it's not in code.
pub fn find_trigger(text: &str, caret: usize) -> Option<Trigger> {
    let before = text.get(..caret)?;
    let tail = before.rfind(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-'))).map_or(0, |i| i + 1);
    if before.len() - tail < 2 || !before[..tail].ends_with(':') {
        return None;
    }
    let start = tail - 1;
    if !opens(before[..start].chars().next_back()) || in_code(before, start) {
        return None;
    }
    Some(Trigger { start, query: before[tail..].to_ascii_lowercase() })
}

/// The `@handle` being typed before `caret`: the @ starts the text or follows whitespace or an opening bracket (so not
/// an email's `a@b`), and isn't in code. The query may be empty: a bare @ lists the MR's people.
pub fn find_mention(text: &str, caret: usize) -> Option<Trigger> {
    let before = text.get(..caret)?;
    let tail = before.rfind(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))).map_or(0, |i| i + 1);
    if !before[..tail].ends_with('@') {
        return None;
    }
    let start = tail - 1;
    if !opens(before[..start].chars().next_back()) || in_code(before, start) {
        return None;
    }
    Some(Trigger { start, query: before[tail..].to_ascii_lowercase() })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmojiHit {
    pub emoji: String,
    pub name: String,
}

/// Names starting with the query, then those containing it, each group with the common ones first and otherwise in
/// gemoji's order.
pub fn rank_emoji(query: &str, limit: usize) -> Vec<EmojiHit> {
    let q = query.to_lowercase();
    let rank = |names: &[String]| names.iter().filter_map(|n| COMMON.iter().position(|c| c == n)).min().unwrap_or(99);
    let (mut prefix, mut inner) = (Vec::new(), Vec::new());
    for (i, e) in emoji::list().iter().enumerate() {
        let mut best: Option<(&String, bool)> = None;
        for name in &e.names {
            match name.find(&q) {
                Some(0) => {
                    best = Some((name, true));
                    break;
                }
                Some(_) => best = best.or(Some((name, false))),
                None => {}
            }
        }
        let Some((name, pre)) = best else { continue };
        (if pre { &mut prefix } else { &mut inner }).push((rank(&e.names), i, EmojiHit { emoji: e.emoji.clone(), name: name.clone() }));
    }
    prefix.sort_by_key(|x| (x.0, x.1));
    inner.sort_by_key(|x| (x.0, x.1));
    prefix.into_iter().chain(inner).take(limit).map(|x| x.2).collect()
}

/// `text` with the typed trigger and query replaced by `token`, and the caret after it.
pub fn insert_token(text: &str, caret: usize, trigger: &Trigger, token: &str) -> (String, usize) {
    (format!("{}{token}{}", &text[..trigger.start], &text[caret..]), trigger.start + token.len())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub id: u64,
    pub username: String,
    pub name: String,
}

/// The people who match `query`, by username or display name: prefix matches, then substrings, each group in the order
/// given. Everyone for an empty query.
pub fn rank_people(users: &[Person], query: &str) -> Vec<Person> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return users.to_vec();
    }
    let rank = |u: &Person| {
        let (a, b) = (u.username.to_lowercase(), u.name.to_lowercase());
        if a.starts_with(&q) || b.starts_with(&q) || b.split_whitespace().any(|w| w.starts_with(&q)) {
            0
        } else if a.contains(&q) || b.contains(&q) {
            1
        } else {
            2
        }
    };
    let mut hits: Vec<(i32, usize, &Person)> = users.iter().enumerate().map(|(i, u)| (rank(u), i, u)).filter(|x| x.0 < 2).collect();
    hits.sort_by_key(|x| (x.0, x.1));
    hits.into_iter().map(|x| x.2.clone()).collect()
}

/// `first` then `more` without repeats, up to `limit`.
pub fn merge_people(first: &[Person], more: &[Person], limit: usize) -> Vec<Person> {
    let mut seen = std::collections::HashSet::new();
    first.iter().chain(more).filter(|u| seen.insert(u.id)).take(limit).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_shortcode_starts_after_a_space_with_two_characters_and_out_of_code() {
        let at = |s: &str| find_trigger(s, s.len());
        assert_eq!(at("hi :sm"), Some(Trigger { start: 3, query: "sm".into() }));
        assert_eq!(at(":SM"), Some(Trigger { start: 0, query: "sm".into() }));
        assert_eq!(at("(:sm"), Some(Trigger { start: 1, query: "sm".into() }));
        assert_eq!(at(":s"), None, "two at least");
        assert_eq!(at("http://x:sm"), None);
        assert_eq!(at("12:30"), None);
        assert_eq!(at("`:sm"), None, "in a span");
        assert_eq!(at("```\n:sm"), None, "in a fence");
        assert_eq!(at("```\nx\n```\n:sm"), Some(Trigger { start: 10, query: "sm".into() }));
    }

    #[test]
    fn a_mention_starts_after_a_space_and_may_be_bare() {
        let at = |s: &str| find_mention(s, s.len());
        assert_eq!(at("@"), Some(Trigger { start: 0, query: String::new() }));
        assert_eq!(at("hi @Ad"), Some(Trigger { start: 3, query: "ad".into() }));
        assert_eq!(at("a@b"), None, "an email");
        assert_eq!(at("`@b"), None);
    }

    #[test]
    fn common_emoji_sort_first_prefixes_before_substrings() {
        let names: Vec<String> = rank_emoji("rock", 8).into_iter().map(|h| h.name).collect();
        assert_eq!(names[0], "rocket");
        assert!(rank_emoji("heart", 8).iter().any(|h| h.name == "heart"));
        assert_eq!(rank_emoji("nosuchthing", 8), Vec::new());
        assert_eq!(rank_emoji("a", 8).len(), 8);
    }

    #[test]
    fn a_token_takes_the_trigger_and_query() {
        let t = Trigger { start: 3, query: "sm".into() };
        assert_eq!(insert_token("hi :sm!", 6, &t, ":smile: "), ("hi :smile: !".to_string(), 11));
    }

    #[test]
    fn people_rank_by_prefix_then_substring_in_the_order_given() {
        let p = |id, u: &str, n: &str| Person { id, username: u.into(), name: n.into() };
        let list = [p(1, "zed", "Ada Lane"), p(2, "bada", "Bo"), p(3, "ada", "Cy")];
        let names = |q: &str| rank_people(&list, q).into_iter().map(|u| u.username).collect::<Vec<_>>();
        assert_eq!(names("ada"), ["zed", "ada", "bada"]);
        assert_eq!(names(""), ["zed", "bada", "ada"]);
        assert_eq!(names("nobody"), Vec::<String>::new());
        assert_eq!(merge_people(&list[..1], &list, 2).iter().map(|u| u.id).collect::<Vec<_>>(), [1, 2]);
    }
}
