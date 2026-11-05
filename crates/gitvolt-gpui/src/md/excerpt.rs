//! `excerpt.ts`: a comment's plain-text excerpt, for a folded card's one line.

use std::sync::OnceLock;

/// How much of a body an excerpt reads: it shows on one line, cut by its box.
const READ_CHARS: usize = 2_000;

fn res() -> &'static [(onig::Regex, &'static str)] {
    static RE: OnceLock<Vec<(onig::Regex, &'static str)>> = OnceLock::new();
    RE.get_or_init(|| {
        [
            (r"^ {0,3}(?:>\s?)+", ""),
            (r"^ {0,3}#{1,6}\s+", ""),
            (r"\s+#+\s*$", ""),
            (r"^\s*(?:[-*+]|\d{1,9}[.)])\s+(?:\[[ xX]\]\s+)?", ""),
            (r"!\[([^\]]*)\]\([^)]*\)", "$1"),
            (r"\[([^\]]*)\](?:\([^)]*\)|\[[^\]]*\])", "$1"),
            (r"<((?:https?|mailto):[^>\s]+)>", "$1"),
            (r"</?[A-Za-z][^>]*>", ""),
            (r"`+([^`]*)`+", "$1"),
            (r"(\*\*|__|~~)(?=\S)(.+?)(?<=\S)\1", "$2"),
            (r"(\A|[^A-Za-z0-9_*])[*_](?=\S)(.+?)(?<=\S)[*_](?![A-Za-z0-9_*])", "$1$2"),
            (r"\\([!-/:-@\[-`{-~])", "$1"),
            (r"\s*\|\s*", " "),
        ]
        .into_iter()
        .map(|(p, r)| (onig::Regex::new(p).expect("a valid pattern"), r))
        .collect()
    })
}

fn flatten_line(line: &str) -> String {
    let mut s = line.to_string();
    for (re, with) in res() {
        s = re.replace_all(&s, |c: &onig::Captures| with.replace("$1", c.at(1).unwrap_or_default()).replace("$2", c.at(2).unwrap_or_default()));
    }
    s
}

fn fence_of(line: &str) -> Option<(char, usize, &str)> {
    let t = line.trim_start_matches(' ');
    if line.len() - t.len() > 3 {
        return None;
    }
    let c = t.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let n = t.chars().take_while(|x| *x == c).count();
    (n >= 3).then(|| (c, n, t[n..].trim_start().split(|x: char| x.is_whitespace() || x == '`').next().unwrap_or("")))
}

fn thematic(line: &str) -> bool {
    let t = line.trim_start_matches(' ');
    if line.len() - t.len() > 3 {
        return false;
    }
    let squeezed: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    squeezed.len() >= 3 && squeezed.chars().all(|c| c == squeezed.chars().next().unwrap_or(' ')) && matches!(squeezed.chars().next(), Some('-' | '*' | '_'))
}

fn delimiter_row(line: &str) -> bool {
    line.contains('|') && line.contains('-') && line.chars().all(|c| matches!(c, '|' | '-' | ':') || c.is_whitespace())
}

/// Its blocks' text, without fences or markup, joined; a suggestion reads "Suggested change".
pub fn plain_excerpt(body: &str) -> String {
    let cut = body.char_indices().nth(READ_CHARS).map_or(body.len(), |x| x.0);
    let mut blocks: Vec<String> = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut fence: Option<(char, usize, bool)> = None;
    fn end(para: &mut Vec<String>, blocks: &mut Vec<String>) {
        if !para.is_empty() {
            blocks.push(para.join(" "));
            para.clear();
        }
    }
    for line in body[..cut].split('\n').map(|l| l.trim_end_matches('\r')) {
        if let Some((c, n, skip)) = fence {
            let close = line.trim();
            if close.len() >= n && close.chars().all(|x| x == c) {
                fence = None;
                end(&mut para, &mut blocks);
                continue;
            }
            if !skip && !close.is_empty() {
                para.push(close.to_string());
            }
            continue;
        }
        if let Some((c, n, info)) = fence_of(line) {
            end(&mut para, &mut blocks);
            let suggestion = super::safe::suggestion_lang(info);
            if suggestion {
                blocks.push("Suggested change".into());
            }
            fence = Some((c, n, suggestion));
            continue;
        }
        if thematic(line) {
            end(&mut para, &mut blocks);
            continue;
        }
        if delimiter_row(line) {
            continue;
        }
        let text = flatten_line(line).split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            end(&mut para, &mut blocks);
        } else {
            para.push(text);
        }
    }
    end(&mut para, &mut blocks);
    blocks.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn a_suggestion_reads_as_one() {
        assert_eq!(plain_excerpt("```suggestion:-5+0\nfunction handle(\n  $request,\n```\n\nThis drops the unused argument."), "Suggested change · This drops the unused argument.");
        assert_eq!(plain_excerpt("Maybe:\n\n````suggestion\nconst a = \"```\";\n````"), "Maybe: · Suggested change");
    }

    #[test]
    fn other_markdown_flattens_to_its_text() {
        assert_eq!(plain_excerpt("## Why **this** _line_?\n\nSee [the docs](https://docs.example.test) and `config.yml`."), "Why this line? · See the docs and config.yml.");
        assert_eq!(plain_excerpt("> quoted\n> more\n\n- [x] done\n1. first\n\n---\n\n<b>bold</b> ~~old~~ snake_case_name \\*star"), "quoted more · done first · bold old snake_case_name *star");
        assert_eq!(plain_excerpt("```ts\nconst a = 1;\n```"), "const a = 1;");
        assert_eq!(plain_excerpt("![diagram](a.png)\n| a | b |\n|---|---|\n| 1 | 2 |"), "diagram a b 1 2");
    }

    #[test]
    fn a_blank_body_is_empty() {
        assert_eq!(plain_excerpt("\n  \n"), "");
    }
}
