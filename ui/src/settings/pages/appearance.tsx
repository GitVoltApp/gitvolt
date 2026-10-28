import { Palette } from 'lucide-react';
import { DEFAULT_DENSITY, DENSITIES, useDensity, type Density } from '../../theme/density';
import { AppearanceBody } from '../../theme/editor/AppearanceBody';
import { ThemeSelect } from '../../theme/editor/ThemeSelect';
import { appSetting, storeBinding } from '../bindings';
import { registerSettings, registerSettingsPage } from '../registry';

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

const offs = [
  registerSettingsPage({ id: 'appearance', label: 'Appearance', scope: 'app', order: 20, icon: Palette, summary: 'Density, theme and colours. The zoom is in the status bar; the editor font is under Diffs and editors.', Body: AppearanceBody }),
  registerSettings([
    {
      id: 'theme', label: 'Theme', page: 'appearance', group: 'Theme', labelFor: 'group',
      keywords: 'colour color scheme dark light default monokai darcula dracula one dark solarized github nord',
      description: 'The colours of the app and the commit graph.',
      // The page shows the theme cards (AppearanceBody); this is the theme's control in search results.
      Control: ThemeSelect, binding: appSetting('theme', 'default-dark'),
    },
    {
      id: 'graphColors', label: 'Graph lanes', page: 'appearance', group: 'Graph lanes',
      keywords: 'colour color branch lane graph palette override reorder',
      description: "The commit graph's ten lane colours. Changing one on a built-in theme makes a custom copy of it.",
    },
    {
      id: 'themeColours', label: 'Colours', page: 'appearance', group: 'Colours',
      keywords: 'colour color token background text accent diff custom',
      description: "The theme's colours, each with a picker. Changing one on a built-in theme makes a custom copy of it.",
    },
    {
      id: 'density', label: 'Density', page: 'appearance', group: 'Density', labelFor: 'group',
      keywords: 'compact standard comfortable row height spacing padding',
      description: 'Row height and padding of the graph and the file lists.',
      Control: DensityControl,
      binding: storeBinding(useDensity, () => useDensity.getState().density, (d) => useDensity.getState().setDensity(d), DEFAULT_DENSITY),
    },
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
