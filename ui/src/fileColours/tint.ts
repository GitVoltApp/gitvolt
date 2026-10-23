import { contrastRatio } from '../theme/contrast';

export const TINT_PRESETS = {
  green: '#3fb950',
  blue: '#4d88ff',
  orange: '#f0883e',
  violet: '#a371f7',
  rose: '#f778ba',
  yellow: '#d29922',
  gray: '#8b949e',
} as const;
export type TintPreset = keyof typeof TINT_PRESETS;

/** Fill strength by theme kind; light themes need more to show against a pale ground. */
export const TINT_STRENGTH = { dark: 0.1, light: 0.14 } as const;

/** A preset's hex, a valid `#rrggbb` lowercased, else null. */
export function tintHex(tint: string): string | null {
  if (Object.hasOwn(TINT_PRESETS, tint)) return TINT_PRESETS[tint as TintPreset];
  return /^#[0-9a-f]{6}$/i.test(tint) ? tint.toLowerCase() : null;
}

/** What a row sits on: an opaque background and the text colours that must stay readable on it. */
export interface TintGround {
  kind: 'dark' | 'light';
  bg: string;
  texts: readonly string[];
}

function rgbOf(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/**
 * The fill for a row: the tint at the theme's strength, lowered until every file-name colour
 * keeps 4.5:1 (or, if it already read worse, no worse than it did) over the composited fill.
 * The colour is kept; only the strength gives way.
 */
export function rowTint(tint: string, ground: TintGround): string | null {
  const hex = tintHex(tint);
  if (!hex || !/^#[0-9a-f]{6}$/i.test(ground.bg.trim())) return null;
  const rgb = rgbOf(hex);
  const base = rgbOf(ground.bg.trim());
  let targets: number[];
  try {
    targets = ground.texts.map((t) => Math.min(4.5, contrastRatio(t, ground.bg)));
  } catch {
    return null;
  }
  const over = (a: number) => `rgb(${rgb.map((v, i) => Math.round(v * a + base[i] * (1 - a))).join(', ')})`;
  const ok = (a: number) => {
    const bg = over(a);
    return ground.texts.every((t, i) => contrastRatio(t, bg) >= targets[i] - 1e-9);
  };
  let steps = Math.round(TINT_STRENGTH[ground.kind] / 0.005);
  while (steps > 0 && !ok(steps * 0.005)) steps--;
  return `rgba(${rgb.join(', ')}, ${Number((steps * 0.005).toFixed(3))})`;
}
