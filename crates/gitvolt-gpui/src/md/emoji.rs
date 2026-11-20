//! gemoji's shortcodes (`forge/emoji.ts`, `emojiComplete.ts`), embedded as `assets/emoji.json.gz`: each emoji with its
//! names, in gemoji's order. Read once, on the first shortcode.

use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

pub struct Entry {
    pub emoji: String,
    pub names: Vec<String>,
}

struct Table {
    list: Vec<Entry>,
    by_name: HashMap<String, usize>,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut json = String::new();
        let list: Vec<Entry> = flate2::read::GzDecoder::new(&include_bytes!("../../assets/emoji.json.gz")[..])
            .read_to_string(&mut json)
            .ok()
            .and_then(|_| serde_json::from_str::<Vec<(String, Vec<String>)>>(&json).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|(emoji, names)| Entry { emoji, names })
            .collect();
        let by_name = list.iter().enumerate().flat_map(|(i, e)| e.names.iter().map(move |n| (n.clone(), i))).collect();
        Table { list, by_name }
    })
}

pub fn list() -> &'static [Entry] {
    &table().list
}

/// A lone text-presentation emoji (⚙ U+2699) gets U+FE0F, or it draws monochrome.
fn colourful(e: &str) -> String {
    let mut chars = e.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_ascii() && !default_emoji(c) => format!("{e}\u{fe0f}"),
        _ => e.to_string(),
    }
}

/// `Emoji_Presentation`, as far as a lone code point in gemoji's list goes: the wide pictographs.
fn default_emoji(c: char) -> bool {
    matches!(c as u32, 0x1F300..=0x1F5FF if !matches!(c as u32, 0x1F321..=0x1F32C | 0x1F336 | 0x1F37D | 0x1F396..=0x1F39F | 0x1F3CB..=0x1F3CE | 0x1F3D4..=0x1F3DF | 0x1F3F3 | 0x1F3F5 | 0x1F3F7 | 0x1F43F | 0x1F441 | 0x1F4FD | 0x1F549..=0x1F54A | 0x1F56F..=0x1F570 | 0x1F573..=0x1F579 | 0x1F587 | 0x1F58A..=0x1F58D | 0x1F590 | 0x1F5A5 | 0x1F5A8 | 0x1F5B1..=0x1F5B2 | 0x1F5BC | 0x1F5C2..=0x1F5C4 | 0x1F5D1..=0x1F5D3 | 0x1F5DC..=0x1F5DE | 0x1F5E1 | 0x1F5E3 | 0x1F5E8 | 0x1F5EF | 0x1F5F3 | 0x1F5FA))
        || matches!(c as u32, 0x1F600..=0x1F64F | 0x1F680..=0x1F6C5 | 0x1F6CC | 0x1F6D0..=0x1F6D2 | 0x1F6D5..=0x1F6D7 | 0x1F6DC..=0x1F6DF | 0x1F6EB..=0x1F6EC | 0x1F6F4..=0x1F6FC | 0x1F7E0..=0x1F7EB | 0x1F7F0 | 0x1F90C..=0x1F9FF | 0x1FA70..=0x1FAFF | 0x231A..=0x231B | 0x23E9..=0x23EC | 0x23F0 | 0x23F3 | 0x25FD..=0x25FE | 0x2614..=0x2615 | 0x2648..=0x2653 | 0x267F | 0x2693 | 0x26A1 | 0x26AA..=0x26AB | 0x26BD..=0x26BE | 0x26C4..=0x26C5 | 0x26CE | 0x26D4 | 0x26EA | 0x26F2..=0x26F3 | 0x26F5 | 0x26FA | 0x26FD | 0x2705 | 0x270A..=0x270B | 0x2728 | 0x274C | 0x274E | 0x2753..=0x2755 | 0x2757 | 0x2795..=0x2797 | 0x27B0 | 0x27BF | 0x2B1B..=0x2B1C | 0x2B50 | 0x2B55)
}

fn lookup(name: &str) -> Option<String> {
    let t = table();
    let i = t.by_name.get(name).or_else(|| t.by_name.get(&name.to_lowercase()))?;
    Some(colourful(&t.list[*i].emoji))
}

/// `text` with each known `:shortcode:` replaced by its emoji.
pub fn emojify(text: &str) -> String {
    let is_name = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-');
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(':') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let len = after.find(|c: char| !is_name(c)).unwrap_or(after.len());
        if len > 0 && after[len..].starts_with(':') {
            if let Some(e) = lookup(&after[..len]) {
                out.push_str(&e);
                rest = &after[len + 1..];
                continue;
            }
            // As JavaScript's regex: the second colon can start the next one only when this one matched nothing known.
            out.push_str(&rest[at..at + 1 + len + 1]);
            rest = &after[len + 1..];
            continue;
        }
        out.push(':');
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn shortcodes_turn_into_glyphs() {
        assert_eq!(emojify("a :rocket: b"), "a 🚀 b");
        assert_eq!(emojify(":ROCKET:"), "🚀");
        assert_eq!(emojify(":+1: and :nope: and 12:30"), "👍 and :nope: and 12:30");
        assert_eq!(emojify(":gear:"), "⚙\u{fe0f}", "a text-presentation one is asked for in colour");
    }
}
