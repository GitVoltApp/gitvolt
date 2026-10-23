import { PageGroups, SettingsGroup } from '../../settings/frame/SettingsPage';
import { useSettingsPaste } from '../../settings/paste';
import type { SettingsCtx } from '../../settings/registry';
import { pasteTheme } from './actions';
import { EditingStrip } from './EditingStrip';
import { LaneColours } from './LaneColours';
import { LivePreview } from './LivePreview';
import { ThemeCards } from './ThemeCards';
import { ThemeColourList } from './ThemeColourList';
import './editor.css';

/** The groups this body draws itself; any other group on the page (Sizes) is rows as usual. */
const OWN = new Set(['Theme', 'Graph lanes', 'Colours']);

/** Settings › Appearance: the theme cards and the editing strip, the lane colours, the colours
 * beside a live preview, then the size settings. Ctrl+V on the page pastes a theme. */
export function AppearanceBody({ ctx }: { ctx: SettingsCtx }) {
  useSettingsPaste(pasteTheme);
  return (
    <>
      <SettingsGroup title="Theme" bare>
        <div data-setting-id="theme" id="setting-theme">
          <ThemeCards />
          <EditingStrip />
        </div>
      </SettingsGroup>
      <SettingsGroup title="Graph lanes" hint="drag to reorder, click to change" bare>
        <div data-setting-id="graphColors" id="setting-graphColors"><LaneColours /></div>
      </SettingsGroup>
      <SettingsGroup title="Colours" bare>
        <div data-setting-id="themeColours" id="setting-themeColours" className="theme-colours-page">
          <ThemeColourList />
          <LivePreview />
        </div>
      </SettingsGroup>
      <PageGroups pageId="appearance" ctx={ctx} skip={OWN} />
    </>
  );
}
