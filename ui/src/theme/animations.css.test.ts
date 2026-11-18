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
  return new Map([...keyframeBodies()].map(([name, { body }]) => [name, new Set([...body.matchAll(/([\w-]+)\s*:/g)].map((p) => p[1]))]));
}

/** `@keyframes name { … }`: its frames' text, and its file. */
function keyframeBodies(): Map<string, { file: string; body: string }> {
  const out = new Map<string, { file: string; body: string }>();
  for (const { file, css } of sheets) {
    for (const m of css.matchAll(/@keyframes\s+([\w-]+)\s*\{/g)) {
      const start = m.index + m[0].length;
      let depth = 1;
      let i = start;
      for (; i < css.length && depth > 0; i++) {
        if (css[i] === '{') depth++;
        else if (css[i] === '}') depth--;
      }
      out.set(m[1], { file, body: css.slice(start, i - 1) });
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

  // Each iteration of an animation fires `animationiteration`, which React listens for at the root,
  // and Chromium runs main-thread frames around every one it has a listener for: a spinner turning
  // once an iteration cost ~5 frames a second (style, pre-paint, commit over the whole page). The
  // shared `spinner-turns` (tokens.css) is sixty turns an iteration.
  it('every endless turn is the shared spinner-turns, an iteration lasting at least half a minute', () => {
    const turns = [...keyframeBodies()].filter(([, k]) => k.body.includes('rotate(')).map(([name, k]) => `${k.file}: ${name}`);
    expect(turns).toEqual(['theme/tokens.css: spinner-turns']);
    const spins = sheets.flatMap(({ file, css }) =>
      [...css.matchAll(/animation:\s*spinner-turns\s+([\d.]+)(m?s)\b[^;}]*/g)].map((m) => ({ file, ms: Number(m[1]) * (m[2] === 's' ? 1000 : 1), infinite: /\binfinite\b/.test(m[0]) })));
    expect(spins.length).toBeGreaterThan(5);
    expect(spins.filter((s) => s.ms < 30_000 || !s.infinite)).toEqual([]);
  });
});
