import type { AppSettings } from '../api/gen/AppSettings';
import type { CustomTheme } from '../api/gen/CustomTheme';
import { uniqueName } from '../settings/share';
import { isHexColor } from './apply';
import { newCustomThemeId } from './custom';
import { THEME_IDS, THEMES } from './themes';

type Lanes = Pick<AppSettings, 'theme' | 'customThemes' | 'graphColorOverrides'>;

/**
 * Today's per-theme lane overrides (`graphColorOverrides`) as custom themes: one "<theme> (Custom)"
 * per built-in with a valid override, its lanes the theme's with the overrides applied (the
 * colours the graph showed), and the current theme switched to its copy, so nothing looks
 * different after the update. The overrides are cleared in the same patch, so it runs once.
 * `null` when there's nothing to do.
 */
export function migrateLaneOverrides(s: Lanes, newId: () => string = newCustomThemeId): Partial<AppSettings> | null {
  const overrides = s.graphColorOverrides ?? {};
  if (Object.keys(overrides).length === 0) return null;
  const customThemes: CustomTheme[] = [...(s.customThemes ?? [])];
  let theme: string | undefined;
  // In the built-ins' order, so the result doesn't depend on the file's key order.
  for (const id of THEME_IDS) {
    const lanes = overrides[id];
    if (!Array.isArray(lanes)) continue;
    const base = THEMES[id].graph;
    if (!lanes.slice(0, base.length).some(isHexColor)) continue;
    const graph = base.map((c, i) => (isHexColor(lanes[i]) ? lanes[i].toLowerCase() : c));
    const ct: CustomTheme = { id: newId(), name: uniqueName(`${THEMES[id].label} (Custom)`, customThemes.map((c) => c.name)), base: id, colors: {}, graph };
    customThemes.push(ct);
    if (s.theme === id) theme = ct.id;
  }
  return { customThemes, graphColorOverrides: {}, ...(theme && { theme }) };
}
