//! What a text field completes (`emojiComplete.ts`, `gitmoji.ts`, `mentions.ts`): pure. The emoji popup is the input's
//! (`ui::input`), the mentions' `field.rs`'s.

use super::emoji::{self, Entry};
use std::io::Read;
use std::sync::OnceLock;

pub const MAX_HITS: usize = 8;

/// The emoji popup lists this many (it scrolls): all the gitmojis (74 today) for a bare `:`.
pub const EMOJI_MAX: usize = 100;

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

/// Where the run after the last character `stop` takes starts: past that character, however many bytes it is (`€`).
fn after_last(text: &str, stop: impl Fn(char) -> bool) -> usize {
    text.char_indices().rev().find(|(_, c)| stop(*c)).map_or(0, |(i, c)| i + c.len_utf8())
}

fn opens(c: Option<char>) -> bool {
    c.is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '[' | '{'))
}

/// The shortcode being typed before `caret`: the colon starts the text or follows whitespace or an opening bracket (so
/// not `http://`, `12:30` or `a:b`), the caret is after it or after word characters following it, and it's not in code.
pub fn find_trigger(text: &str, caret: usize) -> Option<Trigger> {
    let before = text.get(..caret)?;
    let tail = after_last(before, |c| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')));
    if !before[..tail].ends_with(':') {
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
    let tail = after_last(before, |c| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')));
    if !before[..tail].ends_with('@') {
        return None;
    }
    let start = tail - 1;
    if !opens(before[..start].chars().next_back()) || in_code(before, start) {
        return None;
    }
    Some(Trigger { start, query: before[tail..].to_ascii_lowercase() })
}

/// `description`: a gitmoji's meaning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmojiHit {
    pub emoji: String,
    pub name: String,
    pub description: Option<String>,
}

/// A gitmoji (gitmoji.dev): `code` without its colons, as typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gitmoji {
    pub emoji: String,
    pub code: String,
    pub description: String,
}

/// The gitmojis, embedded as `assets/gitmojis.json.gz` (`generate.mjs`): read once, on the first `:`.
pub fn gitmojis() -> &'static [Gitmoji] {
    static LIST: OnceLock<Vec<Gitmoji>> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut json = String::new();
        flate2::read::GzDecoder::new(&include_bytes!("../../assets/gitmojis.json.gz")[..])
            .read_to_string(&mut json)
            .ok()
            .and_then(|_| serde_json::from_str::<Vec<(String, String, String)>>(&json).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|(emoji, code, description)| Gitmoji { emoji, code, description })
            .collect()
    })
}

/// `rankGitmojis`: the gitmojis whose code starts with `query`, in gitmoji's own order.
fn rank_gitmojis<'a>(list: &'a [Gitmoji], query: &str) -> impl Iterator<Item = &'a Gitmoji> {
    list.iter().filter(move |g| g.code.starts_with(query))
}

/// The same emoji, with or without the variation selector (gitmoji's 🏗️, gemoji's 🏗).
fn bare(emoji: &str) -> String {
    emoji.replace('\u{fe0f}', "")
}

/// `rankHits`: up to `limit` hits, the gitmojis whose code starts with the query (with their meaning) first, then
/// `rank_emoji`'s, less the emojis already listed.
pub fn rank_hits(gitmojis: &[Gitmoji], emojis: &[Entry], query: &str, limit: usize) -> Vec<EmojiHit> {
    let q = query.to_lowercase();
    let first: Vec<EmojiHit> = rank_gitmojis(gitmojis, &q).map(|g| EmojiHit { emoji: g.emoji.clone(), name: g.code.clone(), description: Some(g.description.clone()) }).collect();
    let seen: std::collections::HashSet<String> = first.iter().map(|h| bare(&h.emoji)).collect();
    let rest = rank_emoji_in(emojis, &q, limit).into_iter().filter(|h| !seen.contains(&bare(&h.emoji)));
    first.into_iter().chain(rest).take(limit).collect()
}

/// `rank_hits` on the embedded lists: what a `:` offers.
pub fn emoji_hits(query: &str) -> Vec<EmojiHit> {
    rank_hits(gitmojis(), emoji::list(), query, EMOJI_MAX)
}

/// Names starting with the query, then those containing it, each group with the common ones first and otherwise in
/// gemoji's order.
pub fn rank_emoji(query: &str, limit: usize) -> Vec<EmojiHit> {
    rank_emoji_in(emoji::list(), query, limit)
}

fn rank_emoji_in(list: &[Entry], query: &str, limit: usize) -> Vec<EmojiHit> {
    let q = query.to_lowercase();
    let rank = |names: &[String]| names.iter().filter_map(|n| COMMON.iter().position(|c| c == n)).min().unwrap_or(99);
    let (mut prefix, mut inner) = (Vec::new(), Vec::new());
    for (i, e) in list.iter().enumerate() {
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
        (if pre { &mut prefix } else { &mut inner }).push((rank(&e.names), i, EmojiHit { emoji: e.emoji.clone(), name: name.clone(), description: None }));
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
    fn a_shortcode_starts_after_a_space_and_out_of_code() {
        let at = |s: &str| find_trigger(s, s.len());
        assert_eq!(at("hi :sm"), Some(Trigger { start: 3, query: "sm".into() }));
        assert_eq!(at(":SM"), Some(Trigger { start: 0, query: "sm".into() }));
        assert_eq!(at("(:sm"), Some(Trigger { start: 1, query: "sm".into() }));
        assert_eq!(at(":s"), Some(Trigger { start: 0, query: "s".into() }));
        assert_eq!(at("http://x:sm"), None);
        assert_eq!(at("12:30"), None);
        assert_eq!(at("`:sm"), None, "in a span");
        assert_eq!(at("```\n:sm"), None, "in a fence");
        assert_eq!(at("```\nx\n```\n:sm"), Some(Trigger { start: 10, query: "sm".into() }));
    }

    #[test]
    fn a_character_of_several_bytes_before_the_caret_is_no_trigger_and_no_panic() {
        for text in ["€", "@[]{}\\|~€", "öäüß", ":ö", "@€"] {
            assert_eq!(find_trigger(text, text.len()), None, "{text}");
            assert_eq!(find_mention(text, text.len()), None, "{text}");
        }
        assert_eq!(find_trigger("€ :sm", "€ :sm".len()), Some(Trigger { start: 4, query: "sm".into() }));
    }

    #[test]
    fn it_opens_on_the_colon_itself() {
        let at = |s: &str| find_trigger(s, s.len());
        assert_eq!(at(":"), Some(Trigger { start: 0, query: String::new() }));
        assert_eq!(at("fix :B"), Some(Trigger { start: 4, query: "b".into() }));
        assert_eq!(at("one\n:"), Some(Trigger { start: 4, query: String::new() }));
        assert_eq!(at("fix : it"), None);
        assert_eq!(at("Note:"), None);
        assert_eq!(at(""), None);
    }

    fn lists() -> (Vec<Gitmoji>, Vec<Entry>) {
        let g = |emoji: &str, code: &str, description: &str| Gitmoji { emoji: emoji.into(), code: code.into(), description: description.into() };
        let e = |emoji: &str, name: &str| Entry { emoji: emoji.into(), names: vec![name.into()] };
        (
            vec![g("🐛", "bug", "Fix a bug."), g("🏗\u{fe0f}", "building_construction", "Make architectural changes."), g("✨", "sparkles", "Introduce new features.")],
            vec![e("🚌", "bus"), e("🐛", "bug"), e("🏗", "building_construction"), e("😄", "smile"), e("🦋", "butterfly")],
        )
    }

    #[test]
    fn the_gitmojis_come_first_with_their_meaning_then_the_other_emojis_without_repeats() {
        let (g, e) = lists();
        let hit = |emoji: &str, name: &str, description: Option<&str>| EmojiHit { emoji: emoji.into(), name: name.into(), description: description.map(str::to_string) };
        assert_eq!(rank_hits(&g, &e, "bu", EMOJI_MAX), [hit("🐛", "bug", Some("Fix a bug.")), hit("🏗\u{fe0f}", "building_construction", Some("Make architectural changes.")), hit("🚌", "bus", None), hit("🦋", "butterfly", None)]);
        let names = |h: Vec<EmojiHit>| h.into_iter().map(|h| h.name).collect::<Vec<_>>();
        assert_eq!(names(rank_hits(&g, &e, "", EMOJI_MAX))[..3], ["bug", "building_construction", "sparkles"], "a bare colon: every gitmoji first");
        assert_eq!(names(rank_hits(&[], &e, "bu", EMOJI_MAX)), ["bug", "bus", "building_construction", "butterfly"]);
        assert_eq!(names(rank_hits(&g, &[], "sp", EMOJI_MAX)), ["sparkles"]);
        assert_eq!(rank_hits(&g, &e, "", 2).len(), 2);
    }

    #[test]
    fn the_embedded_gitmojis_have_their_codes_unwrapped() {
        let list = gitmojis();
        assert_eq!(list.iter().find(|g| g.code == "white_check_mark").map(|g| g.emoji.as_str()), Some("✅"));
        assert!(list.iter().all(|g| g.code.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')));
        assert!(list.len() >= 70);
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
