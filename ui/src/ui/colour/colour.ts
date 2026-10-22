export interface Rgba { r: number; g: number; b: number; a: number }

const round2 = (n: number) => Math.round(n * 100) / 100;
const HEX = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i;
const FN = /^(rgba?)\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})\s*(?:,\s*(\d*\.?\d+)\s*)?\)$/i;

/** `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb()` and `rgba()` (numbers in range); anything else is null. */
export function parseColour(css: string): Rgba | null {
  const s = css.trim();
  const hex = HEX.exec(s);
  if (hex) {
    let h = hex[1]!;
    if (h.length === 3) h = [...h].map((c) => c + c).join('');
    const n = (i: number) => parseInt(h.slice(i, i + 2), 16);
    return { r: n(0), g: n(2), b: n(4), a: h.length === 8 ? round2(n(6) / 255) : 1 };
  }
  const fn = FN.exec(s);
  if (!fn) return null;
  const [r, g, b] = [Number(fn[2]), Number(fn[3]), Number(fn[4])];
  if ((fn[1]!.toLowerCase() === 'rgba') !== (fn[5] !== undefined)) return null;
  const a = fn[5] === undefined ? 1 : Number(fn[5]);
  if (r > 255 || g > 255 || b > 255 || !(a >= 0 && a <= 1)) return null;
  return { r, g, b, a: round2(a) };
}

/** `#rrggbb` when opaque, else `rgba(r, g, b, a)`: the format themes.ts writes. */
export function formatColour({ r, g, b, a }: Rgba): string {
  if (a === 1) return '#' + [r, g, b].map((n) => n.toString(16).padStart(2, '0')).join('');
  return `rgba(${r}, ${g}, ${b}, ${round2(a)})`;
}

export const isCssColour = (v: unknown): v is string => typeof v === 'string' && parseColour(v) !== null;

const byte = (n: number) => Math.max(0, Math.min(255, Math.round(n)));

function hueOf(r: number, g: number, b: number, max: number, d: number): number {
  if (d === 0) return 0;
  const h = max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return (h * 60 + 360) % 360;
}

/** Unrounded (h 0-360, s/v 0-100) so a round trip is exact; round for display. */
export function toHsv(c: Rgba): { h: number; s: number; v: number } {
  const [r, g, b] = [c.r / 255, c.g / 255, c.b / 255];
  const max = Math.max(r, g, b), d = max - Math.min(r, g, b);
  return { h: hueOf(r, g, b, max, d), s: max === 0 ? 0 : (d / max) * 100, v: max * 100 };
}

export function fromHsv(h: number, s: number, v: number, a: number): Rgba {
  const [S, V] = [s / 100, v / 100];
  const f = (n: number) => { const k = (n + h / 60) % 6; return V - V * S * Math.max(0, Math.min(k, 4 - k, 1)); };
  return { r: byte(f(5) * 255), g: byte(f(3) * 255), b: byte(f(1) * 255), a };
}

/** Unrounded (h 0-360, s/l 0-100), as `toHsv`. */
export function toHsl(c: Rgba): { h: number; s: number; l: number } {
  const [r, g, b] = [c.r / 255, c.g / 255, c.b / 255];
  const max = Math.max(r, g, b), min = Math.min(r, g, b), d = max - min;
  const l = (max + min) / 2;
  return { h: hueOf(r, g, b, max, d), s: d === 0 ? 0 : (d / (1 - Math.abs(2 * l - 1))) * 100, l: l * 100 };
}

export function fromHsl(h: number, s: number, l: number, a: number): Rgba {
  const [S, L] = [s / 100, l / 100];
  const k = S * Math.min(L, 1 - L);
  const f = (n: number) => { const m = (n + h / 30) % 12; return L - k * Math.max(-1, Math.min(m - 3, 9 - m, 1)); };
  return { r: byte(f(0) * 255), g: byte(f(8) * 255), b: byte(f(4) * 255), a };
}
