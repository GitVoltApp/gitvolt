import { SettingsGroup } from '../../settings/frame/SettingsPage';
import { useSettingsPaste } from '../../settings/paste';
import { pasteTheme } from './actions';
import { EditingStrip } from './EditingStrip';
import { LaneColours } from './LaneColours';
import { LivePreview } from './LivePreview';
import { ThemeCards } from './ThemeCards';
import { ThemeColourList } from './ThemeColourList';
import './editor.css';

/** Settings › Theme: the theme cards and the editing strip, the lane colours, and the colours
 * beside a live preview. Ctrl+V on the page pastes a theme. */
export function ThemeBody() {
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
    </>
  );
}
