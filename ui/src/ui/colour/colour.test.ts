import { describe, expect, it } from 'vitest';
import { formatColour, fromOklch, maxChroma, oklchPixel, parseColour, toOklch } from './colour';

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
});

describe('OKLCH', () => {
  it('gives the known values for white, black and red', () => {
    const white = toOklch(parseColour('#ffffff')!);
    expect(white.l).toBeCloseTo(100, 3);
    expect(white.c).toBe(0);
    expect(toOklch(parseColour('#000000')!)).toEqual({ l: 0, c: 0, h: 0 });
    const red = toOklch(parseColour('#ff0000')!);
    expect(red.l).toBeCloseTo(62.8, 1);
    expect(red.c).toBeCloseTo(0.258, 3);
    expect(red.h).toBeCloseTo(29.2, 1);
    expect(toOklch(parseColour('#0000ff')!).h).toBeCloseTo(264.05, 1);
  });

  it('round-trips sRGB through OKLCH without drift, greys and the gamut corners included', () => {
    for (const v of ['#de9b43', '#4d88ff', '#1c1e23', '#ff0000', '#00ff00', '#0000ff', '#ffffff', '#000000', '#808080', '#010101', '#fefefe', '#00ffff'])
      expect(formatColour(fromOklch(toOklch(parseColour(v)!), 1)), v).toBe(v);
  });

  it('keeps alpha as given', () => {
    expect(fromOklch({ l: 50, c: 0, h: 0 }, 0.4).a).toBe(0.4);
  });

  it('brings a colour outside sRGB in by chroma alone: its hue and lightness stay', () => {
    // Red's hue at full chroma lies outside sRGB; the result is the most saturated sRGB colour there.
    const asked = { l: 62.8, c: 0.4, h: 29.2 };
    const got = toOklch(fromOklch(asked, 1));
    expect(got.l).toBeCloseTo(62.8, 0);
    expect(got.h).toBeCloseTo(29.2, 0);
    expect(got.c).toBeLessThan(0.4);
    expect(got.c).toBeCloseTo(maxChroma(62.8, 29.2, 0.4), 2);
    // Clipping channels would have given another hue; chroma reduction never does, even far out.
    for (const h of [0, 90, 150, 200, 264, 330]) {
      const c = toOklch(fromOklch({ l: 55, c: 0.4, h }, 1));
      expect(Math.abs(((c.h - h + 540) % 360) - 180), `hue ${h}`).toBeLessThan(1.5);
    }
  });

  it('white and black have no chroma to give', () => {
    expect(formatColour(fromOklch({ l: 100, c: 0.3, h: 120 }, 1))).toBe('#ffffff');
    expect(formatColour(fromOklch({ l: 0, c: 0.3, h: 120 }, 1))).toBe('#000000');
    expect(maxChroma(100, 120, 0.4)).toBeLessThan(1e-3);
  });

  it('a pixel outside sRGB is null, one inside its bytes', () => {
    expect(oklchPixel(62.8, 0.4, 29.2)).toBeNull();
    expect(oklchPixel(62.8, 0.1, 29.2)).not.toBeNull();
    expect(oklchPixel(100, 0, 0)).toEqual([255, 255, 255]);
  });
});
