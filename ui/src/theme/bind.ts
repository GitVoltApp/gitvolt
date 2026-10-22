import type { AppSettings } from '../api/gen/AppSettings';
import type { CustomTheme } from '../api/gen/CustomTheme';
import { useAppState } from '../app/state';
import { isHexColor, readThemeMirror, writeThemeMirror } from './apply';
import { customThemeDef, isCustomThemeId, themeDef } from './custom';
import { migrateLaneOverrides } from './migrate';
import { useTheme } from './store';
import { DEFAULT_THEME_ID, isThemeId, THEMES, type ThemeDef } from './themes';

/**
 * Keeps the applied theme in step with the saved settings (`AppSettings.theme`, resolved against
 * `customThemes`). Call once, before the first render:
 *
 * - First paint: the localStorage mirror of the last saved theme (ruling R3), so a light-theme
 *   user never sees Default Dark while the settings load. Until they have loaded, the store's
 *   placeholder `DEFAULT_SETTINGS` is ignored.
 * - The first loaded settings move any old lane overrides into custom themes (migrate.ts).
 * - Then every settings change that affects the shown theme (its id, or the current custom
 *   theme's contents; anything else is skipped, so canvases don't redraw for an unrelated
 *   setting), mirrored for the next start.
 * - `?theme=<id>` overrides the saved theme for this page load only (the screenshot tests use
 *   it): a built-in, or a custom theme the settings have. It's never saved or mirrored.
 */
export function bindThemeToSettings(): () => void {
  const fromUrl = new URLSearchParams(location.search).get('theme');
  let lastKey = '';
  const apply = (def: ThemeDef, custom: CustomTheme | undefined, extra = '') => {
    const key = `${def.id}|${JSON.stringify(custom ?? null)}|${extra}`;
    if (key === lastKey) return false;
    lastKey = key;
    useTheme.getState().set(def);
    return true;
  };

  const mirror = readThemeMirror();
  if (isThemeId(fromUrl)) apply(THEMES[fromUrl], undefined);
  else if (mirror?.custom) apply(customThemeDef(mirror.custom), mirror.custom);
  else if (mirror) {
    // A mirror from before custom themes, with the theme's lane overrides: the same lanes until
    // the settings load and the migration makes them a custom theme.
    const own = THEMES[mirror.id as keyof typeof THEMES];
    const graph = own.graph.map((c, i) => (isHexColor(mirror.graph?.[i]) ? mirror.graph[i]!.toLowerCase() : c));
    apply({ ...own, graph }, undefined, mirror.graph ? JSON.stringify(graph) : '');
  } else apply(THEMES[DEFAULT_THEME_ID], undefined);

  let migrated = false;
  const sync = ({ loaded, settings }: { loaded: boolean; settings: AppSettings }) => {
    if (!loaded) return;
    if (!migrated) {
      migrated = true;
      const patch = migrateLaneOverrides(settings);
      // setSettings calls this again with the migrated settings.
      if (patch) return useAppState.getState().setSettings(patch);
    }
    const customs = settings.customThemes ?? [];
    const forced = fromUrl && (isThemeId(fromUrl) || (isCustomThemeId(fromUrl) && customs.some((c) => c.id === fromUrl))) ? fromUrl : null;
    const id = forced ?? settings.theme;
    const def = themeDef(id, customs);
    const custom = customs.find((c) => c.id === def.id);
    if (apply(def, custom) && !forced) writeThemeMirror(def, custom);
  };
  sync(useAppState.getState());
  return useAppState.subscribe(sync);
}
