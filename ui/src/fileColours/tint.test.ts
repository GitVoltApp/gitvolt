import { describe, expect, it } from 'vitest';
import { contrastRatio } from '../theme/contrast';
import { THEMES } from '../theme/themes';
import { rowTint, TINT_PRESETS, tintHex, type TintGround } from './tint';

const groundOf = (id: keyof typeof THEMES): TintGround => {
  const t = THEMES[id];
  return { kind: t.kind, bg: t.colors['panel-bg0'], texts: [t.colors['text-normal'], t.colors['text-secondary']] };
};

function composite(rgba: string, bg: string) {
  const m = rgba.match(/^rgba\((\d+), (\d+), (\d+), ([\d.]+)\)$/)!;
  const n = parseInt(bg.slice(1), 16);
  const base = [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  const a = Number(m[4]);
  const c = [1, 2, 3].map((i, k) => Math.round(Number(m[i]) * a + base[k] * (1 - a)));
  return { a, bg: `rgb(${c.join(', ')})` };
}

describe('tint', () => {
  it('tintHex accepts the seven presets and #rrggbb only', () => {
    for (const [k, v] of Object.entries(TINT_PRESETS)) expect(tintHex(k)).toBe(v);
    expect(tintHex('teal')).toBeNull();
    expect(tintHex('#abc')).toBeNull();
    expect(tintHex('red')).toBeNull();
    expect(tintHex('#FF0000')).toBe('#ff0000');
  });

  it("a preset is drawn at the theme's strength", () => {
    expect(rowTint('green', groundOf('default-dark'))).toBe('rgba(63, 185, 80, 0.1)');
    expect(rowTint('green', groundOf('light'))).toBe('rgba(63, 185, 80, 0.14)');
  });

  it('returns null for an unknown tint or unparsable ground', () => {
    expect(rowTint('teal', groundOf('light'))).toBeNull();
    expect(rowTint('green', { kind: 'dark', bg: 'nope', texts: [] })).toBeNull();
  });

  it('the strength is capped so file names keep 4.5:1', () => {
    const ground: TintGround = { kind: 'dark', bg: '#000000', texts: ['#767676'] };
    const before = contrastRatio('#767676', '#000000');
    expect(before).toBeGreaterThan(4.5);
    expect(before).toBeLessThan(4.7);
    const c = composite(rowTint('#ffffff', ground)!, '#000000');
    expect(c.a).toBeLessThan(0.1);
    expect(contrastRatio('#767676', c.bg)).toBeGreaterThanOrEqual(4.5);
  });

  it('a text already under 4.5:1 is kept no worse than it was', () => {
    const ground: TintGround = { kind: 'dark', bg: '#000000', texts: ['#555555'] };
    const before = contrastRatio('#555555', '#000000');
    expect(before).toBeLessThan(4.5);
    const c = composite(rowTint('#ffffff', ground)!, '#000000');
    expect(contrastRatio('#555555', c.bg)).toBeGreaterThanOrEqual(before - 1e-9);
  });

  it('every preset on every built-in theme keeps file names readable', () => {
    for (const id of Object.keys(THEMES) as (keyof typeof THEMES)[]) {
      const g = groundOf(id);
      for (const p of Object.keys(TINT_PRESETS)) {
        const c = composite(rowTint(p, g)!, g.bg);
        for (const t of g.texts) {
          const target = Math.min(4.5, contrastRatio(t, g.bg));
          expect(contrastRatio(t, c.bg)).toBeGreaterThanOrEqual(target - 1e-9);
        }
      }
    }
  });
});
