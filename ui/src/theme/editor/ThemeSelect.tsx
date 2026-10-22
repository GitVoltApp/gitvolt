import { useAppState } from '../../app/state';
import { Select } from '../../ui/Select';
import { THEME_IDS, THEMES } from '../themes';
import { useEditedTheme } from './actions';

const BUILT_IN_CHOICES = THEME_IDS.map((id) => [id, THEMES[id].label] as const);

/** The theme as a dropdown (the app's own, never a native `<select>`, K92): the built-ins, then
 * the custom themes. The theme's control in the settings search results; the page has the cards. */
export function ThemeSelect() {
  const { def, customs } = useEditedTheme();
  const choices = [...BUILT_IN_CHOICES, ...customs.map((t) => [t.id, t.name] as const)];
  return <Select<string> id="input-theme" aria-labelledby="label-theme" value={def.id} onChange={(v) => useAppState.getState().setSettings({ theme: v })} options={choices} />;
}
