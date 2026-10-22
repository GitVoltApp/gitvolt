/** `.gitignore`-style patterns, compiled to regular expressions by hand (no dependency). Every
 * regex metacharacter outside glob syntax is escaped, so a pattern never throws or matches by
 * accident; anything that can't compile is `null`. */

export interface CompiledPattern {
  source: string;      // as typed (trimmed)
  exclude: boolean;    // leading `!`
  anchored: boolean;   // has a `/` other than a trailing one (a leading `/` is stripped)
  literalPrefix: string; // anchored: the text before the first glob character
  file: RegExp;        // tests a repository-relative file path
  folder: RegExp;      // tests a folder path: the pattern covers everything below it
}

const reEscape = (s: string) => s.replace(/[.*+?^${}()|[\]\\/]/g, '\\$&');

/** A `[...]` class starting at `i` (pointing at `[`): its regex and the index after it, or null if unclosed. */
function parseClass(p: string, i: number): { re: string; end: number } | null {
  let j = i + 1;
  let neg = false;
  if (p[j] === '!' || p[j] === '^') { neg = true; j++; }
  let body = '';
  let first = true;
  for (; j < p.length; j++) {
    const c = p[j];
    if (c === ']' && !first) return { re: `(?!/)[${neg ? '^' : ''}${body}]`, end: j + 1 };
    first = false;
    if (c === '\\' && j + 1 < p.length) { body += reEscape(p[++j]); continue; }
    if (c === '-') { body += '-'; continue; }
    body += c === ']' || c === '[' || c === '^' || c === '\\' ? `\\${c}` : c;
  }
  return null;
}

/** Glob text (no leading `/`, no trailing `/` or `/**`) to a regex source, or null. */
function body(p: string): string | null {
  let out = '';
  for (let i = 0; i < p.length; ) {
    const c = p[i];
    if (c === '*') {
      let j = i;
      while (p[j] === '*') j++;
      if (j - i >= 2) {
        const atStart = i === 0 || p[i - 1] === '/';
        if (atStart && p[j] === '/') { // `**/`: zero or more folders
          out += '(?:.*/)?';
          i = j + 1;
        } else {
          out += '.*';
          i = j;
        }
      } else {
        out += '[^/]*';
        i++;
      }
    } else if (c === '?') {
      out += '[^/]';
      i++;
    } else if (c === '[') {
      const cls = parseClass(p, i);
      if (!cls) return null;
      out += cls.re;
      i = cls.end;
    } else if (c === '\\' && i + 1 < p.length) {
      out += reEscape(p[i + 1]);
      i += 2;
    } else {
      out += reEscape(c);
      i++;
    }
  }
  return out;
}

function literalPrefixOf(p: string): string {
  let out = '';
  for (let i = 0; i < p.length; i++) {
    const c = p[i];
    if (c === '*' || c === '?' || c === '[') break;
    if (c === '\\' && i + 1 < p.length) { out += p[++i]; continue; }
    out += c;
  }
  return out;
}

export function compilePattern(pattern: string): CompiledPattern | null {
  const source = pattern.trim();
  if (source === '' || source.startsWith('#')) return null;
  let p = source;
  let exclude = false;
  if (p.startsWith('!')) { exclude = true; p = p.slice(1).trim(); }
  if (p === '') return null;
  let inside = false; // trailing `/` or `/**`: everything inside the folder
  if (p.endsWith('/**') && !p.endsWith('\\/**')) { p = p.slice(0, -3); inside = true; }
  else if (p.endsWith('/')) { p = p.slice(0, -1); inside = true; }
  const lead = p.startsWith('/');
  if (lead) p = p.slice(1);
  if (p === '') return null;
  const anchored = lead || /(^|[^\\])\//.test(p);
  const b = body(p);
  if (b === null) return null;
  const full = anchored ? b : `(?:.*/)?${b}`;
  try {
    return {
      source,
      exclude,
      anchored,
      literalPrefix: anchored ? literalPrefixOf(p) : '',
      file: new RegExp(inside ? `^(?:${full})/.+$` : `^(?:${full})(?:/.*)?$`),
      folder: new RegExp(`^(?:${full})(?:/.*)?$`),
    };
  } catch {
    return null;
  }
}

/** Positive match only: ignores a leading `!`. */
export function matchesPattern(pattern: string, path: string): boolean {
  const c = compilePattern(pattern);
  return c !== null && c.file.test(path);
}

/** Escapes `* ? [ ] \` and a leading `!` or `#`, so the text matches only itself. */
export function escapeGlob(literal: string): string {
  const s = literal.replace(/[*?[\]\\]/g, '\\$&');
  return /^[!#]/.test(s) ? `\\${s}` : s;
}
