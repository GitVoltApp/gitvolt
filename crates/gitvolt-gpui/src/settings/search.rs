//! `search.ts`: every whitespace-separated word must occur, case- and accent-insensitive, in a setting's label,
//! description or keywords.

use std::ops::Range;

/// What a character folds to: lower case, without its accent (the Latin ones; other scripts stay as they are).
fn fold_char(c: char) -> String {
    let base = match c {
        'À'..='Å' | 'à'..='å' | 'Ā'..='ą' => 'a',
        'Ç' | 'ç' | 'Ć'..='č' => 'c',
        'È'..='Ë' | 'è'..='ë' | 'Ē'..='ě' => 'e',
        'Ì'..='Ï' | 'ì'..='ï' | 'Ĩ'..='ı' => 'i',
        'Ñ' | 'ñ' | 'Ń'..='ň' => 'n',
        'Ò'..='Ö' | 'ò'..='ö' | 'Ō'..='ő' => 'o',
        'Ù'..='Ü' | 'ù'..='ü' | 'Ũ'..='ų' => 'u',
        'Ý' | 'ý' | 'ÿ' | 'Ŷ'..='Ÿ' => 'y',
        c => return c.to_lowercase().collect(),
    };
    base.to_string()
}

pub fn fold(s: &str) -> String {
    s.chars().map(fold_char).collect()
}

pub fn words(query: &str) -> Vec<String> {
    fold(query).split_whitespace().map(str::to_string).collect()
}

pub fn matches(words: &[String], label: &str, description: &str, keywords: &str) -> bool {
    let hay = fold(&format!("{label}\n{description}\n{keywords}"));
    !words.is_empty() && words.iter().all(|w| hay.contains(w.as_str()))
}

/// Each word's places in `text`, as byte ranges of `text`, merged.
pub fn highlights(text: &str, words: &[String]) -> Vec<Range<usize>> {
    let mut folded = String::new();
    let mut at: Vec<(usize, usize)> = Vec::new();
    for (i, ch) in text.char_indices() {
        let f = fold_char(ch);
        for _ in 0..f.len() {
            at.push((i, i + ch.len_utf8()));
        }
        folded.push_str(&f);
    }
    let mut found: Vec<Range<usize>> = Vec::new();
    for w in words {
        let mut from = 0;
        while let Some(i) = folded[from..].find(w.as_str()) {
            let start = from + i;
            found.push(at[start].0..at[start + w.len() - 1].1);
            from = folded[start..].char_indices().nth(1).map_or(folded.len(), |c| start + c.0);
        }
    }
    found.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for h in found {
        match merged.last_mut() {
            Some(prev) if h.start <= prev.end => prev.end = prev.end.max(h.end),
            _ => merged.push(h),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn every_word_must_occur_folded() {
        let w = words("DATE format");
        assert!(matches(&w, "Date format", "", ""));
        assert!(matches(&w, "x", "the format", "date"));
        assert!(!matches(&w, "Date", "", ""));
        assert!(matches(&words("cafe"), "Café", "", ""));
        assert!(!matches(&[], "x", "", ""), "an empty query matches nothing");
    }

    #[test]
    fn a_match_marks_the_original_text() {
        assert_eq!(highlights("Café date", &words("cafe")), vec![0..5]);
        assert_eq!(highlights("Font size", &words("fo on")), vec![0..3]);
        assert_eq!(highlights("Date", &words("zz")), [] as [Range<usize>; 0]);
    }
}
