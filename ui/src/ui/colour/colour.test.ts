import { describe, expect, it } from 'vitest';
import { formatColour, fromHsl, fromHsv, parseColour, toHsl, toHsv } from './colour';

describe('colour maths', () => {
  it('parses every accepted form and refuses the rest', () => {
    expect(parseColour('#4D88FF')).toEqual({ r: 77, g: 136, b: 255, a: 1 });
    expect(parseColour('#abc')).toEqual({ r: 170, g: 187, b: 204, a: 1 });
    expect(parseColour('#a3be8c55')).toEqual({ r: 163, g: 190, b: 140, a: 0.33 });
    expect(parseColour('rgba(92, 184, 92, 0.12)')).toEqual({ r: 92, g: 184, b: 92, a: 0.12 });
    for (const bad of ['red', 'var(--x)', 'rgba(1,2,3)', 'rgb(300, 0, 0)', '#12345', 'rgba(1, 2, 3, 2)', '#fff; background: url(x)', ''])
      expect(parseColour(bad)).toBeNull();
  });
  it('formats opaque as hex and translucent as rgba, round-tripping themes.ts values', () => {
    expect(formatColour({ r: 77, g: 136, b: 255, a: 1 })).toBe('#4d88ff');
    expect(formatColour({ r: 92, g: 184, b: 92, a: 0.12 })).toBe('rgba(92, 184, 92, 0.12)');
    for (const v of ['#1c1e23', 'rgba(255, 255, 255, 0.75)']) expect(formatColour(parseColour(v)!)).toBe(v);
  });
  it('converts through HSV and HSL without drift', () => {
    const c = parseColour('#de9b43')!;
    const { h, s, v } = toHsv(c);
    expect(formatColour(fromHsv(h, s, v, 1))).toBe('#de9b43');
    const l = toHsl(c);
    expect(formatColour(fromHsl(l.h, l.s, l.l, 1))).toBe('#de9b43');
  });
});
