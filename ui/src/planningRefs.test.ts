import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

// The design notes the code was planned from are local-only, so a section number (`§12.3`,
// `spec #2 §8.1`), a task id (`2C T3`) or a review/feedback id (`K68`, `review I1`) in a comment, a
// test title or a doc means nothing to a reader of the repository. Say the reason instead. The scan
// is over whole lines, so a string that only looks like one goes in ALLOWED.
const PATTERNS: RegExp[] = [
  /§/,
  /\bspec #\d/i,
  /\b\d[A-E] T\d+\b/,
  /\b\d[A-E] (?:Task|fix|final|Deviation|review|re-review)\b/,
  /\bplan \d[A-E]\b/i,
  /\b\d[A-E]'s\b/,
  /\bK\d{1,3}\b/,
  /\bJ\d{1,2}\b/,
  /\bH(?:[7-9]|\d{2,3})\b/,
  /\bfix round\b/i,
  /\bdeviation \d/i,
  /\bamendment \d/i,
  /\bruling [A-Z]?\d/i,
  /\bUX ?(?:round \d|\d? ?[A-Z]\.\d|[A-Z]\b)/i,
  /\b(?:re-)?review [ICMNP]?\d/i,
  /\bfeedback [A-Z]\d/i,
  /\bfinal [ICMN]\d/,
  /\blane (?:[A-Z]\d?(?:-[A-Z])?|W\d-[A-Z])\b/,
  /\breview focus\b/i,
  /\bminor #\d/i,
  /\bphase \d/i,
  /\bUX\d?\b/,
  /\(R\d+(?:\.\d+)?[),:]/,
  /\bplan \d\b/i,
];

// Exceptions: `[path, text]`, the text a substring of the offending line (a public standard's
// section, or a word that only looks like an id).
const ALLOWED: [string, string][] = [
  ['ui/src/planningRefs.test.ts', ''],
  ['crates/gitvolt-core/src/graph/tests.rs', 'lane L is the node'], // a graph lane
  ['ui/src/demo/engine/writeSeam.ts', "Symbol('phase 3')"], // a symbol's description
];

const repo = join(__dirname, '../..');
const ROOTS = ['ui/src', 'ui/e2e', 'ui/e2e-demo', 'ui/scripts', 'ui/build', 'crates', 'vendor/tauri-runtime-cef/src', 'scripts', 'docs'];
const ROOT_FILES = [
  'justfile', 'Cargo.toml', 'ui/index.html', 'ui/demo.html', 'ui/vite.config.ts', 'ui/playwright.config.ts',
  'ARCHITECTURE.md', 'CONTRIBUTING.md', 'SECURITY.md', 'PRIVACY.md', 'README.md', 'vendor/tauri-runtime-cef/GITVOLT-PATCH.md',
];
const SOURCE = /\.(rs|ts|tsx|js|mjs|cjs|css|html|sh|toml|md)$|^[^.]+$/;
const SKIP_DIRS = new Set(['node_modules', 'target', 'dist', 'wasm', 'test-results', 'snapshots', '__snapshots__']);

function files(dir: string, out: string[] = []): string[] {
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const e of entries) {
    if (e.isDirectory()) {
      if (!SKIP_DIRS.has(e.name)) files(join(dir, e.name), out);
    } else if (SOURCE.test(e.name)) out.push(join(dir, e.name));
  }
  return out;
}

/** The tracked files under ROOTS: what the repository publishes, not what's ignored locally
 * (`docs/superpowers/` holds the local-only notes themselves). Null without git. */
function tracked(): string[] | null {
  try {
    const out = execFileSync('git', ['ls-files', '-z', '--', ...ROOTS], { cwd: repo, encoding: 'utf8', maxBuffer: 64 << 20 });
    return out.split('\0').filter((f) => f && SOURCE.test(f.split('/').pop()!) && !f.split('/').some((d) => SKIP_DIRS.has(d))).map((f) => join(repo, f));
  } catch {
    return null;
  }
}

describe('planning references', () => {
  it('no comment or test title points at the local-only design notes', () => {
    const all = [...tracked() ?? ROOTS.flatMap((r) => files(join(repo, r))), ...ROOT_FILES.map((f) => join(repo, f))];
    const hits: string[] = [];
    for (const path of all) {
      let text: string;
      try {
        text = readFileSync(path, 'utf8');
      } catch {
        continue;
      }
      const rel = relative(repo, path);
      text.split('\n').forEach((line, i) => {
        if (!PATTERNS.some((p) => p.test(line))) return;
        if (!ALLOWED.some(([p, t]) => rel === p && line.includes(t))) hits.push(`${rel}:${i + 1}: ${line.trim()}`);
      });
    }
    expect(hits).toEqual([]);
  });
});
