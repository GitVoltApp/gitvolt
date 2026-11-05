//! File colours (`ui/src/fileColours/`): a repository's scopes, each a tint and `.gitignore`-style patterns; a file list's rows
//! take the first matching scope's tint, a folder's when a scope covers all of it. The scopes live in the profile, by the
//! repository's common directory. Settings' page isn't here yet: a file's menu adds to a scope through a picker.

use crate::app::GitVolt;
use crate::ui::menu::MenuRow;
use crate::ui::picker::{self, PickItem};
use gpui::prelude::*;
use gpui::*;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::HashMap;

pub const TINT_PRESETS: [(&str, u32); 7] = [("green", 0x3fb950), ("blue", 0x4d88ff), ("orange", 0xf0883e), ("violet", 0xa371f7), ("rose", 0xf778ba), ("yellow", 0xd29922), ("gray", 0x8b949e)];
/// A dark theme's fill strength.
const TINT_STRENGTH: f32 = 0.1;

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Lit(char),
    /// `*`: anything but a slash.
    Star,
    /// `**` not before a slash.
    Any,
    /// `**/`: zero or more folders.
    Dirs,
    /// `?`: one character but a slash.
    One,
    Class { neg: bool, items: Vec<(char, char)> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub source: String,
    pub exclude: bool,
    anchored: bool,
    literal_prefix: String,
    toks: Vec<Tok>,
    inside: bool,
}

fn parse_class(p: &[char], i: usize) -> Option<(Tok, usize)> {
    let mut j = i + 1;
    let mut neg = false;
    if matches!(p.get(j), Some('!' | '^')) {
        neg = true;
        j += 1;
    }
    let mut items = Vec::new();
    let mut first = true;
    while j < p.len() {
        let c = p[j];
        if c == ']' && !first {
            return Some((Tok::Class { neg, items }, j + 1));
        }
        first = false;
        let lo = if c == '\\' && j + 1 < p.len() {
            j += 1;
            p[j]
        } else {
            c
        };
        if p.get(j + 1) == Some(&'-') && p.get(j + 2).is_some_and(|x| *x != ']') {
            items.push((lo, p[j + 2]));
            j += 3;
        } else {
            items.push((lo, lo));
            j += 1;
        }
    }
    None
}

fn body(p: &[char]) -> Option<Vec<Tok>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < p.len() {
        match p[i] {
            '*' => {
                let mut j = i;
                while p.get(j) == Some(&'*') {
                    j += 1;
                }
                if j - i >= 2 {
                    let at_start = i == 0 || p[i - 1] == '/';
                    if at_start && p.get(j) == Some(&'/') {
                        out.push(Tok::Dirs);
                        i = j + 1;
                    } else {
                        out.push(Tok::Any);
                        i = j;
                    }
                } else {
                    out.push(Tok::Star);
                    i += 1;
                }
            }
            '?' => {
                out.push(Tok::One);
                i += 1;
            }
            '[' => {
                let (t, end) = parse_class(p, i)?;
                out.push(t);
                i = end;
            }
            '\\' if i + 1 < p.len() => {
                out.push(Tok::Lit(p[i + 1]));
                i += 2;
            }
            c => {
                out.push(Tok::Lit(c));
                i += 1;
            }
        }
    }
    Some(out)
}

/// `compilePattern`: anything that can't compile is `None`.
pub fn compile(pattern: &str) -> Option<Pattern> {
    let source = pattern.trim().to_string();
    if source.is_empty() || source.starts_with('#') {
        return None;
    }
    let (exclude, mut p) = match source.strip_prefix('!') {
        Some(rest) => (true, rest.trim().to_string()),
        None => (false, source.clone()),
    };
    if p.is_empty() {
        return None;
    }
    let mut inside = false;
    if p.ends_with("/**") && !p.ends_with("\\/**") {
        p.truncate(p.len() - 3);
        inside = true;
    } else if p.ends_with('/') {
        p.pop();
        inside = true;
    }
    let lead = p.starts_with('/');
    if lead {
        p.remove(0);
    }
    if p.is_empty() {
        return None;
    }
    let chars: Vec<char> = p.chars().collect();
    let has_slash = chars.iter().enumerate().any(|(i, c)| *c == '/' && (i == 0 || chars[i - 1] != '\\'));
    let anchored = lead || has_slash;
    let mut toks = body(&chars)?;
    if !anchored {
        toks.insert(0, Tok::Dirs);
    }
    let mut literal_prefix = String::new();
    if anchored {
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if matches!(c, '*' | '?' | '[') {
                break;
            }
            if c == '\\' && i + 1 < chars.len() {
                i += 1;
                literal_prefix.push(chars[i]);
            } else {
                literal_prefix.push(c);
            }
            i += 1;
        }
    }
    Some(Pattern { source, exclude, anchored, literal_prefix, toks, inside })
}

/// Where the tokens can end along `s` from `at`, as `Tail` wants.
fn walk(t: &[Tok], s: &[char], at: usize, done: &dyn Fn(usize) -> bool) -> bool {
    let Some(tok) = t.first() else { return done(at) };
    match tok {
        Tok::Lit(c) => s.get(at) == Some(c) && walk(&t[1..], s, at + 1, done),
        Tok::One => s.get(at).is_some_and(|c| *c != '/') && walk(&t[1..], s, at + 1, done),
        Tok::Class { neg, items } => s.get(at).is_some_and(|c| *c != '/' && (items.iter().any(|(a, b)| a <= c && c <= b) != *neg)) && walk(&t[1..], s, at + 1, done),
        Tok::Star => {
            let mut i = at;
            loop {
                if walk(&t[1..], s, i, done) {
                    return true;
                }
                if s.get(i).is_none_or(|c| *c == '/') {
                    return false;
                }
                i += 1;
            }
        }
        Tok::Any => (at..=s.len()).any(|i| walk(&t[1..], s, i, done)),
        Tok::Dirs => walk(&t[1..], s, at, done) || (at..s.len()).any(|i| s[i] == '/' && walk(&t[1..], s, i + 1, done)),
    }
}

impl Pattern {
    /// Tests a repository-relative file path.
    pub fn file(&self, path: &str) -> bool {
        let s: Vec<char> = path.chars().collect();
        let len = s.len();
        if self.inside {
            let tail = s.clone();
            walk(&self.toks, &s, 0, &move |at| tail.get(at) == Some(&'/') && at + 1 < len)
        } else {
            let tail = s.clone();
            walk(&self.toks, &s, 0, &move |at| at == len || tail.get(at) == Some(&'/'))
        }
    }

    /// Tests a folder's path: the pattern covers everything below it.
    pub fn folder(&self, path: &str) -> bool {
        let s: Vec<char> = path.chars().collect();
        let (len, tail) = (s.len(), s.clone());
        walk(&self.toks, &s, 0, &move |at| at == len || tail.get(at) == Some(&'/'))
    }

    /// Could this exclude match a path below `folder`?
    fn could_exclude(&self, folder: &str) -> bool {
        if !self.anchored {
            return true;
        }
        let dir = format!("{folder}/");
        dir.starts_with(&self.literal_prefix) || self.literal_prefix.starts_with(&dir)
    }
}

/// `escapeGlob`: the text matches only itself.
pub fn escape_glob(literal: &str) -> String {
    let mut out = String::new();
    for c in literal.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    if out.starts_with(['!', '#']) {
        out.insert(0, '\\');
    }
    out
}

/// `typePatternOf`: a file's last extension; a name with none, a dotfile or a trailing dot stands for itself.
pub fn type_pattern(path: &str) -> (String, String) {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < name.len() => {
            let ext = &name[dot + 1..];
            (format!("*.{}", escape_glob(ext)), format!("*.{ext}"))
        }
        _ => (escape_glob(name), name.to_string()),
    }
}

/// `folderPatternOf`: `<dir>/**`, the file's own folder or the folder itself; none for a file at the root.
pub fn folder_pattern(path: &str, is_folder: bool) -> Option<(String, String)> {
    let dir = if is_folder { path } else { path.rsplit_once('/').map_or("", |x| x.0) };
    (!dir.is_empty()).then(|| (format!("{}/**", escape_glob(dir)), format!("{dir}/")))
}

/// `tintHex`: a preset's colour, or a valid `#rrggbb`.
pub fn tint_hex(tint: &str) -> Option<u32> {
    if let Some((_, h)) = TINT_PRESETS.iter().find(|(n, _)| *n == tint) {
        return Some(*h);
    }
    let h = tint.strip_prefix('#').filter(|h| h.len() == 6)?;
    u32::from_str_radix(h, 16).ok()
}

fn rgb(h: u32) -> [f32; 3] {
    [((h >> 16) & 255) as f32, ((h >> 8) & 255) as f32, (h & 255) as f32]
}

fn luminance(c: [f32; 3]) -> f32 {
    let f = |v: f32| {
        let s = v / 255.;
        if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
}

fn contrast(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn over(top: [f32; 3], alpha: f32, base: [f32; 3]) -> [f32; 3] {
    [top[0] * alpha + base[0] * (1. - alpha), top[1] * alpha + base[1] * (1. - alpha), top[2] * alpha + base[2] * (1. - alpha)]
}

/// `rowTint`: the tint at the theme's strength, lowered until the file names keep 4.5:1 (or no worse than they read). The
/// default dark theme's ground: `--panel-bg0`, with white text at 75% and 60%.
pub fn row_tint(tint: &str) -> Option<(u32, f32)> {
    let hex = tint_hex(tint)?;
    let color = rgb(hex);
    let bg = rgb(0x262930);
    let white = [255., 255., 255.];
    let texts = [over(white, 0.75, bg), over(white, 0.6, bg)];
    let targets = [contrast(texts[0], bg).min(4.5), contrast(texts[1], bg).min(4.5)];
    let mut steps = (TINT_STRENGTH / 0.005).round() as i32;
    while steps > 0 {
        let fill = over(color, steps as f32 * 0.005, bg);
        let ok = [0.75_f32, 0.6].iter().enumerate().all(|(i, a)| contrast(over(white, *a, fill), fill) >= targets[i] - 1e-6);
        if ok {
            break;
        }
        steps -= 1;
    }
    Some((hex, steps as f32 * 0.005))
}

struct Scope {
    positives: Vec<Pattern>,
    excludes: Vec<Pattern>,
    fill: Option<Hsla>,
}

/// `matcherFor`: per path, cached, until the scopes change.
pub struct Matcher {
    scopes: Vec<Scope>,
    files: RefCell<HashMap<String, i32>>,
    folders: RefCell<HashMap<String, i32>>,
}

impl Matcher {
    pub fn new(scopes: &Value) -> Self {
        let list = scopes.as_array().map_or(&[][..], Vec::as_slice);
        let scopes = list
            .iter()
            .map(|s| {
                let all: Vec<Pattern> = s["patterns"].as_array().map_or(Vec::new(), |p| p.iter().filter_map(|x| compile(x.as_str()?)).collect());
                let (excludes, positives) = all.into_iter().partition(|p| p.exclude);
                let fill = s["tint"].as_str().and_then(row_tint).map(|(hex, a)| {
                    let c: Hsla = rgba((hex << 8) | ((a * 255.).round() as u32)).into();
                    c
                });
                Scope { positives, excludes, fill }
            })
            .collect();
        Self { scopes, files: RefCell::default(), folders: RefCell::default() }
    }

    /// The first matching scope's index, -1 for none.
    pub fn scope_of(&self, path: &str) -> i32 {
        if let Some(h) = self.files.borrow().get(path) {
            return *h;
        }
        let hit = self.scopes.iter().position(|s| s.positives.iter().any(|p| p.file(path)) && !s.excludes.iter().any(|p| p.file(path))).map_or(-1, |i| i as i32);
        self.files.borrow_mut().insert(path.to_string(), hit);
        hit
    }

    /// The first scope covering the whole folder, -1 for none.
    pub fn folder_scope_of(&self, folder: &str) -> i32 {
        if let Some(h) = self.folders.borrow().get(folder) {
            return *h;
        }
        let hit = self.scopes.iter().position(|s| s.positives.iter().any(|p| p.folder(folder)) && !s.excludes.iter().any(|e| e.could_exclude(folder))).map_or(-1, |i| i as i32);
        self.folders.borrow_mut().insert(folder.to_string(), hit);
        hit
    }

    fn fill(&self, i: i32) -> Option<Hsla> {
        usize::try_from(i).ok().and_then(|i| self.scopes.get(i)).and_then(|s| s.fill)
    }

    pub fn file_tint(&self, path: &str) -> Option<Hsla> {
        self.fill(self.scope_of(path))
    }

    pub fn folder_tint(&self, folder: &str) -> Option<Hsla> {
        self.fill(self.folder_scope_of(folder))
    }

    /// The winning scope's index and its first positive pattern that matched.
    pub fn explain(&self, path: &str) -> Option<(usize, String)> {
        let i = usize::try_from(self.scope_of(path)).ok()?;
        self.scopes[i].positives.iter().find(|p| p.file(path)).map(|p| (i, p.source.clone()))
    }
}

/// `nextTint`: the first preset no scope uses, else gray.
pub fn next_tint(scopes: &[Value]) -> &'static str {
    TINT_PRESETS.iter().map(|p| p.0).find(|n| !scopes.iter().any(|s| s["tint"] == *n)).unwrap_or("gray")
}

/// `uniqueName`: `name`, else `name (2)`, `name (3)`…
fn unique_name(taken: &[Value], name: &str) -> String {
    let free = |n: &str| !taken.iter().any(|s| s["name"] == n);
    if free(name) {
        return name.to_string();
    }
    (2..).map(|i| format!("{name} ({i})")).find(|n| free(n)).unwrap_or_else(|| name.to_string())
}

impl GitVolt {
    /// The tab's repository's scopes, as a matcher: the same one until the profile changes.
    pub fn colours(&self) -> Option<std::rc::Rc<Matcher>> {
        let tab = self.tabs.get(self.active)?;
        let dir = tab.info["commonDir"].as_str()?;
        let scopes = &self.profile["fileColours"][dir];
        if !scopes.is_array() {
            return None;
        }
        let mut cache = self.colours_cache.borrow_mut();
        if let Some(m) = cache.get(dir) {
            return Some(m.clone());
        }
        let m = std::rc::Rc::new(Matcher::new(scopes));
        cache.insert(dir.to_string(), m.clone());
        Some(m)
    }

    fn common_dir(&self) -> Option<String> {
        self.tabs.get(self.active)?.info["commonDir"].as_str().map(str::to_string)
    }

    /// Adds `pattern` to a scope, or a new one at the end. Saved in the profile, as the web UI's page does.
    pub fn add_colour_pattern(&mut self, dir: String, scope: Option<usize>, pattern: String, cx: &mut Context<Self>) {
        self.colours_cache.borrow_mut().clear();
        self.update_profile(
            move |p| {
                if !p["fileColours"].is_object() {
                    p["fileColours"] = json!({});
                }
                if !p["fileColours"][&dir].is_array() {
                    p["fileColours"][&dir] = json!([]);
                }
                let list = p["fileColours"][&dir].as_array_mut().expect("just made");
                match scope.filter(|i| *i < list.len()) {
                    Some(i) => {
                        if let Some(ps) = list[i]["patterns"].as_array_mut()
                            && !ps.iter().any(|x| x == &json!(pattern))
                        {
                            ps.push(json!(pattern));
                        }
                    }
                    None => {
                        let id = format!("scope-{}", chrono::Utc::now().timestamp_millis());
                        let (name, tint) = (unique_name(list, "New scope"), next_tint(list));
                        list.push(json!({ "id": id, "name": name, "tint": tint, "patterns": [pattern], "startCollapsed": false }));
                    }
                }
            },
            cx,
        );
    }

    /// "Colour …": which scope takes the pattern; the web UI's Settings page isn't here yet.
    fn pick_colour_scope(&mut self, pattern: String, window: &mut Window, cx: &mut Context<Self>) {
        const NEW: &str = "\u{0}new";
        let Some(dir) = self.common_dir() else { return };
        let list = self.profile["fileColours"][&dir].as_array().cloned().unwrap_or_default();
        let mut items: Vec<PickItem> = list.iter().enumerate().map(|(i, s)| PickItem { id: i.to_string().into(), label: s["name"].as_str().unwrap_or_default().to_string().into(), detail: s["tint"].as_str().map(|t| t.to_string().into()), icon: Some("palette"), current: false, tooltip: Some(format!("Add {pattern} to this scope").into()) }).collect();
        items.push(PickItem { id: NEW.into(), label: "New scope".into(), detail: None, icon: Some("plus"), current: false, tooltip: Some(format!("A new scope for {pattern}").into()) });
        let at = crate::ui::menu::last_origin(cx);
        let anchor = Bounds::new(point(at.x, at.y - px(4.)), size(px(0.), px(0.)));
        let weak = cx.weak_entity();
        let on_pick = move |item: &PickItem, _: &mut Window, cx: &mut App| {
            let scope = item.id.parse::<usize>().ok();
            let (dir, pattern) = (dir.clone(), pattern.clone());
            weak.update(cx, |v, cx| v.add_colour_pattern(dir, scope, pattern, cx)).ok();
        };
        picker::open(picker::PickRequest { key: None, anchor, items, placeholder: "Colour with a scope".into(), toggle: None, on_pick: std::rc::Rc::new(on_pick) }, window, cx);
    }

    fn colour_item(&self, id: &'static str, label: String, pattern: String, this: &WeakEntity<GitVolt>) -> MenuRow {
        let (w, p) = (this.clone(), pattern.clone());
        MenuRow::action(id, label, "palette", format!("Add {pattern} to a scope"), move |window, cx| {
            let p = p.clone();
            w.update(cx, |v, cx| v.pick_colour_scope(p, window, cx)).ok();
        })
    }

    /// A file row's `Scope: <name>` and "Colour …".
    pub fn file_colour_rows(&self, path: &str, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        if self.common_dir().is_none() {
            return Vec::new();
        }
        let mut rows = Vec::new();
        if let (Some(m), Some(dir)) = (self.colours(), self.common_dir())
            && let Some((i, pattern)) = m.explain(path)
            && let Some(name) = self.profile["fileColours"][&dir][i]["name"].as_str()
        {
            rows.push(MenuRow::header("file.colour.scope", format!("Scope: {name} · {pattern}")));
        }
        let (pattern, label) = type_pattern(path);
        rows.push(self.colour_item("file.colour.type", format!("Colour {label} files…"), pattern, this));
        if let Some((pattern, label)) = folder_pattern(path, false) {
            rows.push(self.colour_item("file.colour.folder", format!("Colour folder {label}…"), pattern, this));
        }
        rows
    }

    /// A folder row's.
    pub fn folder_colour_rows(&self, path: &str, this: &WeakEntity<GitVolt>) -> Vec<MenuRow> {
        if self.common_dir().is_none() {
            return Vec::new();
        }
        let mut rows = Vec::new();
        if let (Some(m), Some(dir)) = (self.colours(), self.common_dir())
            && let Ok(i) = usize::try_from(m.folder_scope_of(path))
            && let Some(name) = self.profile["fileColours"][&dir][i]["name"].as_str()
        {
            rows.push(MenuRow::header("folder.colour.scope", format!("Scope: {name}")));
        }
        if let Some((pattern, label)) = folder_pattern(path, true) {
            rows.push(self.colour_item("folder.colour.folder", format!("Colour folder {label}…"), pattern, this));
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn hit(p: &str, path: &str) -> bool {
        compile(p).is_some_and(|c| c.file(path))
    }

    #[test]
    fn patterns_match_as_gitignores_do() {
        assert!(hit("*.ts", "a/b/c.ts"));
        assert!(hit("*.ts", "c.ts"));
        assert!(!hit("*.ts", "c.tsx"));
        assert!(hit("docs/**", "docs/a/b.md"));
        assert!(hit("docs/**", "other/docs/b.md"), "only a slash before its end anchors it, as the web UI's compile has it");
        assert!(!hit("a/docs/**", "other/a/docs/b.md"), "anchored by its slash");
        assert!(hit("/top.txt", "top.txt"));
        assert!(!hit("/top.txt", "a/top.txt"));
        assert!(hit("**/tests/**", "a/tests/b.rs"));
        assert!(hit("**/tests/**", "tests/b.rs"));
        assert!(!hit("**/tests/**", "tests"));
        assert!(hit("build/", "x/build/o.o"));
        assert!(!hit("build/", "x/build"));
        assert!(hit("dist", "x/dist/o.o"), "a name covers what's under it");
        assert!(hit("a?c", "abc"));
        assert!(!hit("a?c", "a/c"));
        assert!(hit("[ab]x", "bx"));
        assert!(hit("[!ab]x", "cx"));
        assert!(!hit("[!ab]x", "ax"));
        assert!(hit("a\\*b", "a*b"));
        assert!(!hit("a\\*b", "axb"));
        assert!(compile("# comment").is_none());
        assert!(compile("[oops").is_none());
        assert!(compile("  ").is_none());
    }

    #[test]
    fn the_first_scope_wins_and_an_exclude_leaves_it() {
        let m = Matcher::new(&json!([{ "tint": "green", "patterns": ["*.test.*", "!skip.test.ts"] }, { "tint": "blue", "patterns": ["*.ts", "docs/**"] }]));
        assert_eq!(m.scope_of("a.test.ts"), 0);
        assert_eq!(m.scope_of("skip.test.ts"), 1);
        assert_eq!(m.scope_of("a.rs"), -1);
        assert_eq!(m.explain("b.ts"), Some((1, "*.ts".to_string())));
        assert_eq!(m.folder_scope_of("docs"), 1);
        assert_eq!(m.folder_scope_of("src"), -1);
        assert!(m.file_tint("b.ts").is_some());
    }

    #[test]
    fn a_files_patterns_come_from_its_type_and_folder() {
        assert_eq!(type_pattern("a/b.test.ts").0, "*.ts");
        assert_eq!(type_pattern("Makefile").0, "Makefile");
        assert_eq!(type_pattern(".env").0, ".env");
        assert_eq!(folder_pattern("a/b/c.ts", false).map(|x| x.0).as_deref(), Some("a/b/**"));
        assert_eq!(folder_pattern("c.ts", false), None);
        assert_eq!(folder_pattern("a/b", true).map(|x| x.1).as_deref(), Some("a/b/"));
        assert_eq!(escape_glob("a*b"), "a\\*b");
        assert_eq!(escape_glob("!x"), "\\!x");
    }

    #[test]
    fn a_tint_gives_way_to_keep_the_text_readable() {
        let (hex, a) = row_tint("green").unwrap();
        assert_eq!(hex, 0x3fb950);
        assert!(a > 0. && a <= TINT_STRENGTH);
        assert_eq!(row_tint("#ABCDEF").map(|x| x.0), Some(0xabcdef));
        assert_eq!(row_tint("nope"), None);
        assert_eq!(next_tint(&[json!({ "tint": "green" })]), "blue");
    }
}
