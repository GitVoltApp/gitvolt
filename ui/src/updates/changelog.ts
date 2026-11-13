/**
 * CHANGELOG.md (Keep a Changelog) for What's new: its released sections (parsed at build time,
 * build/changelog.ts), and the ones between the version that last ran and this one. No network:
 * the notes are the ones this build was made with.
 */

export interface ChangelogEntry {
  version: string;
  /** `YYYY-MM-DD`, or empty when the heading has none. */
  date: string;
  /** The section's Markdown, without its heading or surrounding blank lines. */
  body: string;
}

type Ident = number | string;
interface Version { core: [number, number, number]; pre: Ident[] }

const NUM = /^(0|[1-9]\d*)$/;
const IDENT = /^[0-9A-Za-z-]+$/;

/** `1.2.3`, `v1.2.3-rc.1`, `1.2.3+202610051325.ab4dbf9e` (build metadata ignored, as in the
 * core's `updates/version.rs`); null for anything else. */
function parseVersion(s: string): Version | null {
  const m = /^v?(\d+)\.(\d+)\.(\d+)(?:-([^+]+))?(?:\+[0-9A-Za-z.-]+)?$/.exec(s.trim());
  if (!m || ![m[1], m[2], m[3]].every((n) => NUM.test(n))) return null;
  const pre = m[4] === undefined ? [] : m[4].split('.');
  if (!pre.every((p) => IDENT.test(p) && (!/^\d+$/.test(p) || NUM.test(p)))) return null;
  return { core: [Number(m[1]), Number(m[2]), Number(m[3])], pre: pre.map((p) => (/^\d+$/.test(p) ? Number(p) : p)) };
}

function compareIdent(a: Ident, b: Ident): number {
  if (typeof a === 'number' && typeof b === 'number') return a - b;
  if (typeof a === 'number') return -1;
  if (typeof b === 'number') return 1;
  return a < b ? -1 : a > b ? 1 : 0;
}

/** SemVer order of two versions (negative: `a` is older); null when either isn't a version. */
export function compareVersions(a: string, b: string): number | null {
  const x = parseVersion(a);
  const y = parseVersion(b);
  if (!x || !y) return null;
  for (let i = 0; i < 3; i++) if (x.core[i] !== y.core[i]) return x.core[i] - y.core[i];
  // A pre-release comes before its release.
  if (!x.pre.length || !y.pre.length) return y.pre.length - x.pre.length;
  for (let i = 0; i < Math.min(x.pre.length, y.pre.length); i++) {
    const c = compareIdent(x.pre[i], y.pre[i]);
    if (c) return c;
  }
  return x.pre.length - y.pre.length;
}

const HEADING = /^## \[([^\]]+)\](?:\s+-\s*(\S*))?\s*$/;
const LINK_REF = /^\[[^\]]+\]:\s+\S/;

/** The released sections (`## [1.6.1] - 2026-11-12`), in file order: `[Unreleased]` and any
 * heading that isn't a version are skipped, and a section ends at the next `## ` heading. */
export function parseChangelog(text: string): ChangelogEntry[] {
  const lines = text.replace(/\r\n?/g, '\n').split('\n');
  const out: ChangelogEntry[] = [];
  let current: { version: string; date: string; lines: string[] } | null = null;
  const close = () => {
    if (!current) return;
    const body = current.lines;
    while (body.length && (!body[body.length - 1].trim() || LINK_REF.test(body[body.length - 1]))) body.pop();
    while (body.length && !body[0].trim()) body.shift();
    out.push({ version: current.version, date: current.date, body: body.join('\n') });
    current = null;
  };
  for (const line of lines) {
    if (line.startsWith('## ')) {
      close();
      const m = HEADING.exec(line);
      if (m && parseVersion(m[1])) current = { version: m[1], date: m[2] ?? '', lines: [] };
    } else if (current) {
      current.lines.push(line);
    }
  }
  close();
  return out;
}

/**
 * The notes to show for `current`, newest first: every version after `since` up to `current`
 * (an update from 1.5.0 to 1.6.1: 1.6.1, 1.6.0, 1.5.x), or `current`'s alone when `since` is null.
 * Versions newer than `current` (its own unreleased-but-numbered sections) are left out.
 */
export function notesBetween(entries: ChangelogEntry[], since: string | null, current: string): ChangelogEntry[] {
  const picked = entries.filter((e) => {
    const upTo = compareVersions(e.version, current);
    if (upTo === null || upTo > 0) return false;
    if (since === null) return upTo === 0;
    const after = compareVersions(e.version, since);
    return after !== null && after > 0;
  });
  return picked.sort((a, b) => compareVersions(b.version, a.version) ?? 0);
}
