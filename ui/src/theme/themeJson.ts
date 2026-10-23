import type { CustomTheme } from '../api/gen/CustomTheme';
import { parseShareJson, toShareJson, uniqueName } from '../settings/share';
import { isTokenColour, newCustomThemeId } from './custom';
import { COLOR_TOKENS, DEFAULT_THEME_ID, isThemeId, type ThemeDef } from './themes';

/** A theme as shared JSON: `{"gitvolt": "theme", "version": 1, name, base, colors, graph}`. */
const VERSION = 1;
const MAX_NAME = 80;

const isLane = (v: unknown): v is string => typeof v === 'string' && /^#[0-9a-f]{6}$/i.test(v);
const lanesOf = (v: unknown): string[] => (Array.isArray(v) && v.length === 10 && v.every(isLane) ? v.map((c) => c.toLowerCase()) : []);

/** Only known tokens with a plain colour (Review Focus 1: nothing else may reach a custom property). */
function coloursOf(v: unknown): Record<string, string> {
  if (!v || typeof v !== 'object' || Array.isArray(v)) return {};
  return Object.fromEntries(Object.entries(v).filter(([t, c]) => (COLOR_TOKENS as readonly string[]).includes(t) && isTokenColour(c)));
}

/** `def` as JSON: a custom theme (`custom`, the one `def` resolves) with the colours and lanes it
 * set; a built-in as itself, nothing set. */
export function themeToJson(def: ThemeDef, custom?: CustomTheme): string {
  const own = def.base && custom?.id === def.id ? custom : undefined;
  return toShareJson('theme', VERSION, {
    name: def.label,
    base: def.base ?? def.id,
    colors: own ? coloursOf(own.colors) : {},
    graph: own ? lanesOf(own.graph) : [],
  });
}

/** A pasted or imported theme as a new custom theme (a fresh id, a free name), or `null` for
 * anything that isn't one. An unknown base is Default Dark; invalid colours and lanes are dropped. */
export function themeFromJson(text: string, existing: readonly CustomTheme[], newId: () => string = newCustomThemeId): CustomTheme | null {
  const parsed = parseShareJson(text, 'theme', VERSION);
  if (!parsed.ok) return null;
  const { name, base, colors, graph } = parsed.body;
  if (typeof name !== 'string' || !name.trim()) return null;
  return {
    id: newId(),
    name: uniqueName(name.trim().slice(0, MAX_NAME).trim(), existing.map((t) => t.name)),
    base: isThemeId(base) ? base : DEFAULT_THEME_ID,
    colors: coloursOf(colors),
    graph: lanesOf(graph),
  };
}
