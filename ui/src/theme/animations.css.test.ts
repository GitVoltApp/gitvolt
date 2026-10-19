import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// jsdom doesn't load CSS: this reads every stylesheet under src/. An animation that never ends runs
// for as long as its element is shown, so it must be one the compositor runs alone (transform,
// opacity). Anything else restyles and repaints on the main thread every frame: the update pill's
// box-shadow glow kept the renderer busy for as long as an update waited.
const src = join(dirname(fileURLToPath(import.meta.url)), '..');
const sheets = readdirSync(src, { recursive: true, encoding: 'utf8' })
  .filter((f) => f.endsWith('.css'))
  .map((f) => ({ file: f, css: readFileSync(join(src, f), 'utf8') }));

/** `@keyframes name { … }`: the properties its frames set. */
function keyframes(): Map<string, Set<string>> {
  const out = new Map<string, Set<string>>();
  for (const { css } of sheets) {
    for (const m of css.matchAll(/@keyframes\s+([\w-]+)\s*\{/g)) {
      const start = m.index + m[0].length;
      let depth = 1;
      let i = start;
      for (; i < css.length && depth > 0; i++) {
        if (css[i] === '{') depth++;
        else if (css[i] === '}') depth--;
      }
      const body = css.slice(start, i - 1);
      out.set(m[1], new Set([...body.matchAll(/([\w-]+)\s*:/g)].map((p) => p[1])));
    }
  }
  return out;
}

describe('animations', () => {
  it('every infinite animation is compositor-only (transform, opacity)', () => {
    const frames = keyframes();
    const infinite = sheets.flatMap(({ file, css }) =>
      [...css.matchAll(/animation:\s*([\w-]+)[^;}]*\binfinite\b/g)].map((m) => ({ file, name: m[1] })));
    expect(infinite.length).toBeGreaterThan(0);
    const offenders = infinite.flatMap(({ file, name }) => {
      const props = frames.get(name);
      if (!props) return [`${file}: ${name} has no @keyframes`];
      const bad = [...props].filter((p) => p !== 'transform' && p !== 'opacity');
      return bad.length ? [`${file}: ${name} animates ${bad.join(', ')}`] : [];
    });
    expect(offenders).toEqual([]);
  });
});
