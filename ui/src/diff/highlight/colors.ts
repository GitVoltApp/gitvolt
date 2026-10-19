/** Monaco's token colour syntax (its ColorMap's): six hex digits, an alpha it ignores. */
const HEX = /^#?([0-9a-f]{6})([0-9a-f]{2})?$/i;

/** `#rgb`/`#rgba` → `#rrggbb`/`#rrggbbaa`. */
const expand = (c: string) => (/^#?[0-9a-f]{3,4}$/i.test(c) ? `#${[...c.replace('#', '')].map((x) => x + x).join('')}` : c);

/**
 * A Shiki theme's colour map (index → colour, from 1) as Monaco's token colours: the list to give
 * a Monaco theme as `encodedTokensColors` (it numbers them 1… in order, before any rule's colour),
 * and the remap from Shiki's ids to those, for the worker's token metadata. Monaco keys colours by
 * their six hex digits alone, so two Shiki colours that differ only in alpha share one id; a colour
 * it can't parse becomes the default foreground. `remap` is `null` when the ids already agree.
 */
export function monacoColors(colorMap: readonly string[]): { colors: string[]; remap: Uint16Array | null } {
  const colors: string[] = [];
  const ids = new Map<string, number>();
  const remap = new Uint16Array(Math.max(colorMap.length, 3));
  let same = true;
  for (let i = 1; i < colorMap.length; i++) {
    const m = HEX.exec(expand(colorMap[i] ?? ''));
    let id = 1;
    if (m) {
      const key = m[1].toUpperCase();
      id = ids.get(key) ?? 0;
      if (!id) {
        colors.push(`#${key}`);
        id = colors.length;
        ids.set(key, id);
      }
    }
    remap[i] = id;
    if (id !== i) same = false;
  }
  return { colors, remap: same ? null : remap };
}
