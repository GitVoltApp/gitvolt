//! Rendered Markdown (`ui/src/markdown/`). `ast` is the tree (remark and its plugins), `safe` what's let through
//! (rehype-raw and rehype-sanitize), `boxes` its boxes (`markdown.css`), `layout` their places, `view` the element that
//! paints them and takes the mouse and the keyboard. `diff` is the rendered diff and `field` the comment box.

// What the forge views will use is here before they do.
#![allow(dead_code)]

pub mod ast;
pub mod boxes;
pub mod diff;
pub mod emoji;
pub mod frontmatter;
pub mod layout;
pub mod links;
pub mod safe;

pub const MERMAID_NOTE: &str = "This build shows a Mermaid diagram as its source";

/// What a suggestion replaces (`MdSuggestionBase`), from whoever renders the note.
#[derive(Clone)]
pub struct SuggestionBase {
    pub flavor: ast::Flavor,
    pub path: Option<String>,
    /// The language of the commented file.
    pub language: Option<String>,
    /// How many lines the comment's range has above its last one.
    pub span: u32,
    /// The comment's last line, 1-based.
    pub line: u32,
    pub lines: std::sync::Arc<dyn Fn(u32, u32) -> Option<Vec<String>> + Send + Sync>,
}

impl std::fmt::Debug for SuggestionBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SuggestionBase({:?}:{})", self.path, self.line)
    }
}

impl SuggestionBase {
    /// `suggestionOffset`: GitLab's `suggestion:-N+M`; a plain one on GitHub and Forgejo the commented lines, on GitLab
    /// the last one alone.
    pub fn offset(&self, lang: &str) -> (u32, u32) {
        if let Some((a, b)) = lang.strip_prefix("suggestion:-").and_then(|r| r.split_once('+'))
            && let (Ok(a), Ok(b)) = (a.parse(), b.parse())
        {
            return (a, b);
        }
        (if self.flavor == ast::Flavor::Gitlab { 0 } else { self.span }, 0)
    }

    pub fn replaced(&self, lang: &str) -> Option<Vec<String>> {
        let (above, below) = self.offset(lang);
        let (from, to) = (self.line.checked_sub(above).filter(|f| *f >= 1)?, self.line + below);
        (self.lines)(from, to).filter(|l| l.len() as u32 == to - from + 1)
    }

    pub fn start(&self, lang: &str) -> u32 {
        self.line.saturating_sub(self.offset(lang).0)
    }
}
