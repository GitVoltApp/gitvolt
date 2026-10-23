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

/** OKLCH as the picker edits it: lightness 0-100 (%), chroma 0 to about 0.4, hue 0-360 (degrees). */
export interface Oklch { l: number; c: number; h: number }

const toLinear = (n: number) => { const v = n / 255; return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
const toByte = (v: number) => { const x = Math.max(0, Math.min(1, v)); return byte(255 * (x <= 0.0031308 ? 12.92 * x : 1.055 * x ** (1 / 2.4) - 0.055)); };

/** sRGB to OKLab (Björn Ottosson's matrices), `L` 0-1. */
export function toOklab({ r, g, b }: Rgba): { L: number; a: number; b: number } {
  const [R, G, B] = [toLinear(r), toLinear(g), toLinear(b)];
  const l = Math.cbrt(0.4122214708 * R + 0.5363325363 * G + 0.0514459929 * B);
  const m = Math.cbrt(0.2119034982 * R + 0.6806995451 * G + 0.1073969566 * B);
  const s = Math.cbrt(0.0883024619 * R + 0.2817188376 * G + 0.6299787005 * B);
  return {
    L: 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    a: 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    b: 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  };
}

/** OKLab to linear sRGB, unclamped: a channel outside 0-1 is outside the sRGB gamut. */
export function oklabToLinear(L: number, a: number, b: number): [number, number, number] {
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}

const RAD = Math.PI / 180;
const linearOf = (l: number, c: number, h: number) => oklabToLinear(l / 100, c * Math.cos(h * RAD), c * Math.sin(h * RAD));
// The matrices round-trip to within ~1e-7, so white and the primaries count as in gamut.
const inGamut = (rgb: readonly number[]) => rgb.every((v) => v >= -1e-6 && v <= 1 + 1e-6);

/** Unrounded, so a round trip is exact; round for display. A grey (chroma below rounding noise)
 * reads chroma 0, hue 0. */
export function toOklch(c: Rgba): Oklch {
  const { L, a, b } = toOklab(c);
  const C = Math.hypot(a, b);
  return C < 1e-4 ? { l: L * 100, c: 0, h: 0 } : { l: L * 100, c: C, h: (Math.atan2(b, a) / RAD + 360) % 360 };
}

/** The most chroma, up to `limit`, sRGB holds at lightness `l` (0-100) and hue `h`. At a fixed
 * lightness and hue the colours in gamut run from chroma 0 to one edge, so a binary search finds it. */
export function maxChroma(l: number, h: number, limit: number): number {
  if (inGamut(linearOf(l, limit, h))) return limit;
  let [lo, hi] = [0, limit];
  for (let i = 0; i < 24; i++) {
    const mid = (lo + hi) / 2;
    if (inGamut(linearOf(l, mid, h))) lo = mid;
    else hi = mid;
  }
  return lo;
}

/** An OKLCH as sRGB bytes. Outside the gamut it gives up chroma at the same lightness and hue
 * until it fits: clipping channels instead would shift its hue. */
export function fromOklch({ l, c, h }: Oklch, a: number): Rgba {
  const L = Math.max(0, Math.min(100, l));
  const [r, g, b] = linearOf(L, maxChroma(L, h, Math.max(0, c)), h).map(toByte) as [number, number, number];
  return { r, g, b, a };
}

/** An OKLCH's sRGB bytes, or `null` outside the gamut: the picker's square, pixel by pixel. */
export function oklchPixel(l: number, c: number, h: number): [number, number, number] | null {
  const rgb = linearOf(l, c, h);
  return inGamut(rgb) ? (rgb.map(toByte) as [number, number, number]) : null;
}
