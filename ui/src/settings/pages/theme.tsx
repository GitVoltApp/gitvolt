import { Palette } from 'lucide-react';
import { ThemeBody } from '../../theme/editor/ThemeBody';
import { ThemeSelect } from '../../theme/editor/ThemeSelect';
import { appSetting } from '../bindings';
import { registerSettings, registerSettingsPage } from '../registry';

const offs = [
  registerSettingsPage({ id: 'theme', label: 'Theme', scope: 'app', order: 25, icon: Palette, summary: 'The colours of the app, the commit graph and the editors. Density and fonts are under Appearance.', Body: ThemeBody }),
  registerSettings([
    {
      id: 'theme', label: 'Theme', page: 'theme', group: 'Theme', labelFor: 'group',
      keywords: 'colour color scheme dark light default monokai darcula dracula one dark solarized github nord',
      description: 'The colours of the app and the commit graph.',
      // The page shows the theme cards (ThemeBody); this is the theme's control in search results.
      Control: ThemeSelect, binding: appSetting('theme', 'default-dark'),
    },
    {
      id: 'graphColors', label: 'Graph lanes', page: 'theme', group: 'Graph lanes',
      keywords: 'colour color branch lane graph palette override reorder',
      description: "The commit graph's ten lane colours. Changing one on a built-in theme makes a custom copy of it.",
    },
    {
      id: 'themeColours', label: 'Colours', page: 'theme', group: 'Colours',
      keywords: 'colour color token background text accent diff custom',
      description: "The theme's colours, each with a picker. Changing one on a built-in theme makes a custom copy of it.",
    },
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
