//! A file's grammar: `ui/src/diff/language.ts`, by name, extension or shebang.

use super::languages::LANGS;

const HIGHLIGHT_MAX_BYTES: usize = 1_048_576;
const HIGHLIGHT_MAX_LINES: usize = 20_000;

const EXTENSIONS: &[(&str, &str)] = &[
    ("phtml", "php"), ("inc", "php"), ("h", "c"), ("hpp", "cpp"), ("hh", "cpp"), ("cc", "cpp"), ("cxx", "cpp"), ("htm", "html"), ("xhtml", "html"),
    ("svg", "xml"), ("xsd", "xml"), ("plist", "xml"), ("bash", "shellscript"), ("zsh", "shellscript"), ("ps1", "powershell"), ("kt", "kotlin"),
    ("kts", "kotlin"), ("rb", "ruby"), ("pl", "perl"), ("pm", "perl"), ("cs", "csharp"), ("ex", "elixir"), ("exs", "elixir"), ("erl", "erlang"),
    ("hs", "haskell"), ("ml", "ocaml"), ("tf", "terraform"), ("gql", "graphql"), ("cfg", "ini"), ("conf", "ini"), ("neon", "yaml"), ("markdown", "markdown"),
    ("vert", "glsl"), ("frag", "glsl"), ("geom", "glsl"), ("tesc", "glsl"), ("tese", "glsl"), ("comp", "glsl"), ("vsh", "glsl"), ("fsh", "glsl"), ("gsh", "glsl"),
    ("vshader", "glsl"), ("fshader", "glsl"), ("gshader", "glsl"),
];

const FILENAMES: &[(&str, &str)] = &[
    ("dockerfile", "docker"), ("makefile", "make"), ("gnumakefile", "make"), ("cmakelists.txt", "cmake"), ("composer.lock", "json"),
    ("cargo.lock", "toml"), (".editorconfig", "ini"), (".gitconfig", "ini"), ("justfile", "just"),
];

const SHEBANGS: &[(&str, &str)] = &[
    ("bash", "shellscript"), ("sh", "shellscript"), ("zsh", "shellscript"), ("dash", "shellscript"), ("node", "javascript"), ("deno", "typescript"),
    ("python", "python"), ("python3", "python"), ("php", "php"), ("ruby", "ruby"), ("perl", "perl"), ("tclsh", "tcl"),
];

fn table(list: &'static [(&str, &'static str)], key: &str) -> Option<&'static str> {
    list.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn by_name(name: &str) -> Option<&'static str> {
    LANGS.iter().find(|l| l.id == name || l.aliases.contains(&name)).map(|l| l.id)
}

/// A fence's language: Shiki's id or alias.
pub fn by_fence(name: &str) -> Option<&'static str> {
    by_name(&name.to_ascii_lowercase())
}

fn interpreter(first_line: &str) -> Option<&str> {
    let rest = first_line.strip_prefix("#!")?.trim_start();
    let mut words = rest.split_ascii_whitespace();
    let mut word = words.next()?;
    if word.rsplit('/').next() == Some("env") {
        word = words.find(|w| !w.starts_with('-'))?;
    }
    let name = word.rsplit('/').next()?;
    let end = name.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(name.len());
    (end > 0).then(|| &name[..end])
}

/// The grammar for `path`, whose first line is `first_line`; `None` is plain text.
pub fn detect(path: &str, first_line: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    if let Some(found) = table(FILENAMES, &name) {
        return Some(found);
    }
    if name == ".env" || name.starts_with(".env.") {
        return by_name("dotenv");
    }
    if let Some(dot) = name.rfind('.')
        && dot > 0
        && let Some(found) = by_name(&name[dot + 1..]).or_else(|| table(EXTENSIONS, &name[dot + 1..]))
    {
        return Some(found);
    }
    interpreter(first_line).and_then(|i| table(SHEBANGS, i))
}

/// `detect`, or plain text above the highlighting limit (1 MiB or 20k lines).
pub fn highlight_language(path: &str, text: &str) -> Option<&'static str> {
    if text.len() > HIGHLIGHT_MAX_BYTES || text.bytes().filter(|b| *b == b'\n').count() > HIGHLIGHT_MAX_LINES {
        return None;
    }
    detect(path, text.split('\n').next().unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_finds_its_grammar_by_name_extension_or_shebang() {
        assert_eq!(detect("src/a.rs", ""), Some("rust"));
        assert_eq!(detect("a/B.TSX", ""), Some("tsx"));
        assert_eq!(detect("x.yml", ""), Some("yaml"), "an alias");
        assert_eq!(detect("x.kt", ""), Some("kotlin"), "the table");
        assert_eq!(detect("dir/Dockerfile", ""), Some("docker"));
        assert_eq!(detect(".env.local", ""), Some("dotenv"));
        assert_eq!(detect("run", "#!/usr/bin/env -S python3 -u"), Some("python"));
        assert_eq!(detect("run", "#!/bin/bash"), Some("shellscript"));
        assert_eq!(detect("notes", "hello"), None);
        assert_eq!(detect(".rs", ""), None, "a dotfile has no extension");
    }

    #[test]
    fn a_huge_file_stays_plain() {
        assert_eq!(highlight_language("a.rs", &"x\n".repeat(20_001)), None);
        assert_eq!(highlight_language("a.rs", &"x".repeat(1_048_577)), None);
        assert_eq!(highlight_language("a.rs", "x\n"), Some("rust"));
    }
}
