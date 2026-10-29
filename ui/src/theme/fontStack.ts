/**
 * The font settings' stored values (`editorFontFamily`, `readingFontFamily`, `uiFontFamily`): empty
 * for the default, a family's bare name when the picker chose one, or a CSS font-family list typed
 * under "Custom font list…" (and every value from before the picker, which may be either).
 *
 * Only the family is stored: the fallbacks after it (diff/options.ts `editorFontFamily`,
 * fontSettings.ts `proseFontFamily`) are added when the setting applies, so they can improve
 * without rewriting anyone's settings, and a value synced from a machine with other fonts still
 * falls back well.
 */

export const GENERIC_FAMILY = /^(monospace|ui-monospace|serif|sans-serif|ui-serif|ui-sans-serif|ui-rounded|system-ui|cursive|fantasy|math|emoji|fangsong)$/i;

/** A CSS font-family list's names, unquoted (commas inside quotes don't split). */
export function parseFamilyList(list: string): string[] {
  const out: string[] = [];
  let cur = '';
  let quote: string | null = null;
  let quoted = false;
  const push = () => {
    const name = quoted ? cur : cur.trim().replace(/\s+/g, ' ');
    if (name.trim()) out.push(name);
    cur = '';
    quoted = false;
  };
  for (let i = 0; i < list.length; i++) {
    const c = list[i]!;
    if (quote) {
      if (c === '\\' && i + 1 < list.length) cur += list[++i];
      else if (c === quote) quote = null;
      else cur += c;
    } else if (c === '"' || c === "'") {
      quote = c;
      quoted = true;
      cur = '';
    } else if (c === ',') push();
    else if (!quoted) cur += c;
  }
  push();
  return out;
}

/** The one family `value` names (not a generic one), or null: empty, a list, or a generic. */
export function singleFamily(value: string): string | null {
  const names = parseFamilyList(value);
  return names.length === 1 && !GENERIC_FAMILY.test(names[0]!) ? names[0]! : null;
}

/** `name` as CSS: quoted, unless it's a generic family (which must not be). */
export const cssFamily = (name: string): string => (GENERIC_FAMILY.test(name) ? name : `"${name.replace(/["\\]/g, '\\$&')}"`);

/** What the picker stores for `name`: the bare name, quoted only when it holds a comma or a quote
 * (which a bare name would split or end). */
export const storedFamily = (name: string): string => (/[,"'\\]/.test(name) ? cssFamily(name) : name);

/** `name` first, then `fallbacks` (a CSS list) without another mention of it. */
export function withFallbacks(name: string, fallbacks: string): string {
  const key = name.toLowerCase();
  const rest = parseFamilyList(fallbacks).filter((f) => f.toLowerCase() !== key).map(cssFamily);
  return [cssFamily(name), ...rest].join(', ');
}
