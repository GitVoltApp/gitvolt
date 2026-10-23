//! Branch name globs for presets' "name matches" and the trunk patterns: `*` matches any run of
//! characters, `/` included, and `?` one character; everything else literally, case-sensitively.

use crate::presets::BranchUnit;

/// Whether `pattern` matches all of `text`. Two pointers, backtracking only to the last `*` (a
/// later `*` supersedes an earlier one), so it's linear-ish with no recursion or allocation.
pub(crate) fn glob_match(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.as_bytes(), text.as_bytes());
    // A `?` or a `*`'s retry steps over a whole character; literals compare byte by byte, which
    // keeps the text on character boundaries wherever those two land.
    let step = |i: usize| text.get(i..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
    let (mut pi, mut ti) = (0, 0);
    // The pattern just past the last `*`, and where in the text that `*` stopped matching.
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        match p.get(pi) {
            Some(b'*') => {
                pi += 1;
                star = Some((pi, ti));
            }
            Some(b'?') => {
                pi += 1;
                ti += step(ti);
            }
            Some(&c) if c == t[ti] => {
                pi += 1;
                ti += 1;
            }
            _ => match star {
                Some((sp, st)) => {
                    let st = st + step(st);
                    star = Some((sp, st));
                    (pi, ti) = (sp, st);
                }
                None => return false,
            },
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}

/// Whether one of the comma-separated `patterns` (trimmed, empty ones ignored) matches the unit's
/// name or one of its remote refs as `remote/name` (`origin/*`).
pub(crate) fn name_matches(patterns: &str, unit: &BranchUnit) -> bool {
    let remote_names = || unit.refs.iter().filter_map(|r| r.strip_prefix("refs/remotes/"));
    patterns.split(',').map(str::trim).filter(|p| !p.is_empty()).any(|p| glob_match(p, &unit.name) || remote_names().any(|r| glob_match(p, r)))
}
