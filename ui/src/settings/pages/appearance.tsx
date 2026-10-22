import { Palette } from 'lucide-react';
import { useAppState } from '../../app/state';
import { clampEditorFont, EDITOR_FONT_MAX, EDITOR_FONT_MIN } from '../../diff/options';
import { AppearanceSection, ThemeSelect } from '../../theme/AppearanceSection';
import { DEFAULT_DENSITY, DENSITIES, useDensity, type Density } from '../../theme/density';
import { appSetting, storeBinding } from '../bindings';
import { NumberField } from '../frame/fields';
import { PageGroups } from '../frame/SettingsPage';
import { registerSettings, registerSettingsPage, type SettingsCtx } from '../registry';

const DENSITY_LABELS: Record<Density, string> = { compact: 'Compact', standard: 'Standard', comfortable: 'Comfortable' };

function DensityControl() {
  const density = useDensity((s) => s.density);
  return (
    <div role="radiogroup" aria-labelledby="label-density" className="radio-group">
      {DENSITIES.map((d) => (
        <label key={d} className="radio"><input type="radio" name="density" checked={density === d} onChange={() => useDensity.getState().setDensity(d)} />{DENSITY_LABELS[d]}</label>
      ))}
    </div>
  );
}

function EditorFontControl() {
  const size = useAppState((s) => s.settings.editorFontSize);
  return <NumberField id="editorFontSize" value={clampEditorFont(size)} min={EDITOR_FONT_MIN} max={EDITOR_FONT_MAX} step={1} onCommit={(n) => useAppState.getState().setSettings({ editorFontSize: n })} />;
}

function AppearanceBody({ ctx }: { ctx: SettingsCtx }) {
  return <PageGroups pageId="appearance" ctx={ctx} render={(group, rows) => (group === 'Theme' ? <AppearanceSection /> : rows)} />;
}

const offs = [
  registerSettingsPage({ id: 'appearance', label: 'Appearance', scope: 'app', order: 20, icon: Palette, summary: 'Theme, colours and sizes. The zoom is in the status bar.', Body: AppearanceBody }),
  registerSettings([
    {
      id: 'theme', label: 'Theme', page: 'appearance', group: 'Theme', labelFor: 'group',
      keywords: 'colour color scheme dark light default monokai darcula dracula one dark solarized github nord',
      description: 'The colours of the app and the commit graph.',
      // The page's Theme group is AppearanceSection's; this is the theme's control in search results.
      Control: ThemeSelect, binding: appSetting('theme', 'default-dark'),
    },
    {
      id: 'graphColors', label: 'Graph lane colors', page: 'appearance', group: 'Theme', labelFor: 'group',
      keywords: 'colour branch lane graph palette override',
      description: "The commit graph's ten lane colours. Changing one on a built-in theme makes a custom copy of it.",
    },
    {
      id: 'density', label: 'Density', page: 'appearance', group: 'Sizes', labelFor: 'group',
      keywords: 'compact standard comfortable row height spacing padding',
      description: 'Row height and padding of the graph and the file lists.',
      Control: DensityControl,
      binding: storeBinding(useDensity, () => useDensity.getState().density, (d) => useDensity.getState().setDensity(d), DEFAULT_DENSITY),
    },
    {
      id: 'editorFontSize', label: 'Editor font size', page: 'appearance', group: 'Sizes',
      keywords: 'monaco diff text size px zoom',
      description: 'Text in the diff and file views, 8 to 32 px. The zoom in the status bar scales everything else.',
      Control: EditorFontControl, binding: appSetting('editorFontSize', 13),
    },
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
