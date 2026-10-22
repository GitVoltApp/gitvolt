import type { CustomTheme } from '../api/gen/CustomTheme';
import { useAppState } from '../app/state';
import { uniqueName } from '../settings/share';
import { newCustomThemeId, themeDef } from './custom';
import { DEFAULT_THEME_ID, isThemeId, type ColorToken } from './themes';

/**
 * Editing themes. Editing a built-in makes "<theme> (Custom)" at once and selects it, leaving the
 * built-in untouched; editing a custom theme edits it. Everything saves through the settings, and
 * bind.ts applies it.
 */

const settings = () => useAppState.getState().settings;
const customs = () => settings().customThemes ?? [];

function update(id: string, fn: (t: CustomTheme) => CustomTheme): void {
  useAppState.getState().setSettings({ customThemes: customs().map((t) => (t.id === id ? fn(t) : t)) });
}

/** The current theme's id when it's a custom theme; else a new custom copy of it, selected. */
export function ensureEditable(): string {
  const { theme } = settings();
  if (customs().some((t) => t.id === theme)) return theme;
  const def = themeDef(theme, customs());
  const base = isThemeId(def.id) ? def.id : DEFAULT_THEME_ID;
  const ct: CustomTheme = { id: newCustomThemeId(), name: uniqueName(`${def.label} (Custom)`, customs().map((t) => t.name)), base, colors: {}, graph: [] };
  useAppState.getState().setSettings({ customThemes: [...customs(), ct], theme: ct.id });
  return ct.id;
}

export function setThemeColour(token: ColorToken, value: string): void {
  update(ensureEditable(), (t) => ({ ...t, colors: { ...t.colors, [token]: value } }));
}

/** The current theme's ten lanes, as shown (its own, else its base's). */
const lanes = (id: string) => [...themeDef(id, customs()).graph];

export function setLaneColour(index: number, hex: string): void {
  const id = ensureEditable();
  const graph = lanes(id);
  if (index < 0 || index >= graph.length) return;
  graph[index] = hex.toLowerCase();
  update(id, (t) => ({ ...t, graph }));
}

export function moveLane(from: number, to: number): void {
  const id = ensureEditable();
  const graph = lanes(id);
  if (from === to || from < 0 || to < 0 || from >= graph.length || to >= graph.length) return;
  const [lane] = graph.splice(from, 1);
  graph.splice(to, 0, lane!);
  update(id, (t) => ({ ...t, graph }));
}

/** Trimmed; empty or unchanged does nothing; a clash with another theme's name takes a suffix. */
export function renameTheme(id: string, name: string): void {
  const t = customs().find((c) => c.id === id);
  const next = name.trim();
  if (!t || !next || next === t.name) return;
  const named = uniqueName(next, customs().filter((c) => c.id !== id).map((c) => c.name));
  update(id, (c) => ({ ...c, name: named }));
}

/** Removes a custom theme; when it's the current one, its base is selected. */
export function deleteTheme(id: string): void {
  const t = customs().find((c) => c.id === id);
  if (!t) return;
  const patch: Partial<{ customThemes: CustomTheme[]; theme: string }> = { customThemes: customs().filter((c) => c.id !== id) };
  if (settings().theme === id) patch.theme = isThemeId(t.base) ? t.base : DEFAULT_THEME_ID;
  useAppState.getState().setSettings(patch);
}
