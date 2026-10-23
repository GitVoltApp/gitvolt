import { Palette } from 'lucide-react';
import type { CustomTheme } from '../api/gen/CustomTheme';
import { registerActions, type Action } from '../app/actions';
import { useAppState } from '../app/state';
import { registerSettingsFloat } from '../settings/frame/floating';
import { useSettingsUi } from '../settings/schema';
import { isCustomThemeId } from './custom';
import { FloatingThemeEditor } from './editor/FloatingThemeEditor';
import { THEME_IDS, THEMES } from './themes';

/**
 * The theme actions (ruling R4; its own feature module, imported from `app/features.ts`): View →
 * "Theme…" opens Settings on the theme picker, and one palette-only "Theme: <Name>" per theme
 * (`menu: false`), so Ctrl+P `>Theme: Nord` switches in one go. Custom themes get theirs too,
 * registered again whenever the list changes. Settings › Appearance's "Edit mode" turns
 * Settings into the floating theme editor registered here.
 */
const themeAction = (id: string, label: string): Action => ({
  id: `view.theme.${id}`, label: `Theme: ${label}`, group: 'View', icon: Palette, tooltip: `Switch to the ${label} theme`, menu: false,
  run: () => useAppState.getState().setSettings({ theme: id }),
});

const offActions = registerActions([
  { id: 'view.theme', label: 'Theme…', group: 'View', icon: Palette, tooltip: 'Choose the colour theme', run: () => useSettingsUi.getState().show('theme') },
  ...THEME_IDS.map((id) => themeAction(id, THEMES[id].label)),
]);

const offFloat = registerSettingsFloat('theme-editor', FloatingThemeEditor);

let customs: readonly CustomTheme[] | undefined;
let offCustoms = () => {};
const syncCustoms = ({ settings }: { settings: { customThemes?: CustomTheme[] } }) => {
  const list = settings.customThemes ?? [];
  if (list === customs) return;
  customs = list;
  offCustoms();
  // A hand-edited file's duplicate or odd ids get no action rather than a registry error.
  const ok = list.filter((t, i) => isCustomThemeId(t.id) && list.findIndex((o) => o.id === t.id) === i);
  offCustoms = registerActions(ok.map((t) => themeAction(t.id, t.name)));
};
syncCustoms(useAppState.getState());
const offSync = useAppState.subscribe(syncCustoms);

// A dev-server hot update re-runs this module: release the old registrations first.
import.meta.hot?.dispose(() => {
  offActions();
  offFloat();
  offSync();
  offCustoms();
});
