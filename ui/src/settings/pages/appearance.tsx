import { Type } from 'lucide-react';
import { useAppState } from '../../app/state';
import { DEFAULT_DENSITY, DENSITIES, useDensity, type Density } from '../../theme/density';
import {
  clampReadingFont, clampReadingLineHeight, clampReadingWidth, DEFAULT_FONT_SETTINGS as F, DEFAULT_UI_FONT, READING_FONT_MAX, READING_FONT_MIN, READING_LINE_HEIGHT_MAX,
  READING_LINE_HEIGHT_MIN, READING_WIDTH_MAX, READING_WIDTH_MIN,
} from '../../theme/fontSettings';
import { displayChord } from '../../ui/platformKeys';
import { appSetting, storeBinding } from '../bindings';
import { switchControl } from '../frame/controls';
import { NumberField } from '../frame/fields';
import type { FontRole } from '../frame/FontPicker';
import { FontSetting, ReadingFontPreview, UiFontPreview } from '../frame/FontSetting';
import { registerSettings, registerSettingsPage, type SettingDef } from '../registry';

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

const save = (patch: Partial<typeof F>) => useAppState.getState().setSettings(patch);

/** The UI font's picker: a family gets the platform UI font's stack after it. */
export const UI_FONT_ROLE: FontRole = { id: 'uiFontFamily', defaultName: 'system UI font', defaultStack: DEFAULT_UI_FONT, fallback: DEFAULT_UI_FONT };
/** The reading font's: its default is the UI font, whichever is set. */
export const READING_FONT_ROLE: FontRole = { id: 'readingFontFamily', defaultName: 'the UI font', defaultStack: 'var(--font-ui)', fallback: DEFAULT_UI_FONT };

function UiFontControl() {
  return <FontSetting role={UI_FONT_ROLE} placeholder={'"Inter", sans-serif'} preview={(v) => <UiFontPreview family={v} />} />;
}

/** The reading font's preview is at the reading size and line height. */
function ReadingFontControl() {
  return <FontSetting role={READING_FONT_ROLE} placeholder={'"Literata", Georgia, serif'} preview={(v) => <ReadingFontPreview family={v} />} />;
}

function ReadingSizeControl() {
  const px = useAppState((s) => s.settings.readingFontSize);
  return <NumberField id="readingFontSize" value={clampReadingFont(px)} min={READING_FONT_MIN} max={READING_FONT_MAX} step={1} onCommit={(n) => save({ readingFontSize: n })} />;
}

function ReadingLineHeightControl() {
  const n = useAppState((s) => s.settings.readingLineHeight);
  return <NumberField id="readingLineHeight" value={clampReadingLineHeight(n)} min={READING_LINE_HEIGHT_MIN} max={READING_LINE_HEIGHT_MAX} step={0.05} decimals={2} onCommit={(v) => save({ readingLineHeight: v })} />;
}

/** + from None starts at 80 characters, a common measure for prose. */
export const READING_WIDTH_START = 80;

function ReadingWidthControl() {
  const ch = useAppState((s) => s.settings.readingLineWidth);
  return <NumberField id="readingLineWidth" value={clampReadingWidth(ch)} min={READING_WIDTH_MIN} max={READING_WIDTH_MAX} step={1} blank={0} blankStart={READING_WIDTH_START} placeholder="None" onCommit={(v) => save({ readingLineWidth: v })} />;
}

/** The platform of the app's window, which can draw its own title bar; none in a browser. */
const WINDOW_PLATFORM = typeof window === 'undefined' ? undefined : window.__GITVOLT_WINDOW__?.platform;

const SYSTEM_TITLE_BAR_DESCRIPTIONS: Record<'linux' | 'windows' | 'macos', string> = {
  linux: 'The desktop\'s own title bar and window frame instead of GitVolt\'s, with ☰ back in the tab bar. Takes effect when GitVolt restarts. An X11 session always has the desktop\'s.',
  windows: 'The system\'s own title bar and window frame instead of GitVolt\'s, with ☰ back in the tab bar. Takes effect when GitVolt restarts.',
  macos: 'macOS\'s standard title bar instead of GitVolt\'s, with ☰ back in the tab bar. Takes effect when GitVolt restarts.',
};

/** "Use the system title bar": only where GitVolt draws its own (the app's window, on Linux,
 * Windows and macOS). */
export function systemTitleBarSetting(platform: 'linux' | 'windows' | 'macos'): SettingDef {
  return {
    id: 'systemTitleBar', label: 'Use the system title bar', page: 'appearance', group: 'Window',
    keywords: 'title bar titlebar window frame decorations buttons minimize maximize close native gnome desktop windows macos traffic lights',
    description: SYSTEM_TITLE_BAR_DESCRIPTIONS[platform],
    Control: switchControl('systemTitleBar'), binding: appSetting('systemTitleBar', false),
  };
}

const offs = [
  registerSettingsPage({ id: 'appearance', label: 'Appearance', scope: 'app', order: 20, icon: Type, summary: 'Density and fonts. The theme and colours are under Theme, the editor font under Diffs and editors, the zoom in the status bar.' }),
  registerSettings([
    {
      id: 'density', label: 'Density', page: 'appearance', group: 'Density', labelFor: 'group',
      keywords: 'compact standard comfortable row height spacing padding',
      description: 'Row height and padding of the graph and the file lists.',
      Control: DensityControl,
      binding: storeBinding(useDensity, () => useDensity.getState().density, (d) => useDensity.getState().setDensity(d), DEFAULT_DENSITY),
    },
    {
      id: 'uiFontFamily', label: 'UI font', page: 'appearance', group: 'Fonts',
      keywords: 'font family face typeface interface chrome menus lists graph dialogs installed picker custom list css',
      description: 'The menus, lists, graph, settings and dialogs. Default: the system\'s UI font, which also draws what a chosen font lacks. "Custom font list…" takes a CSS font-family list. Its size follows the zoom and the density.',
      Control: UiFontControl, binding: appSetting('uiFontFamily', F.uiFontFamily),
    },
    {
      id: 'readingFontFamily', label: 'Reading font', page: 'appearance', group: 'Fonts',
      keywords: 'font family face typeface markdown prose rendered readme description comment serif installed picker custom list css',
      description: 'Rendered Markdown: the Markdown view, the rendered diff, MR/PR descriptions and comments. Default: the UI font. Code in it keeps the editor font.',
      Control: ReadingFontControl, binding: appSetting('readingFontFamily', F.readingFontFamily),
    },
    {
      id: 'readingFontSize', label: 'Reading text size', page: 'appearance', group: 'Fonts',
      keywords: 'font size markdown prose rendered px zoom',
      description: `Rendered Markdown in the Markdown view and the rendered diff, 8 to 32 px; descriptions and comments show it 1 px smaller. Over it, ${displayChord('Mod+=')} and ${displayChord('Mod+-')} change it, ${displayChord('Mod+0')} resets it.`,
      Control: ReadingSizeControl, binding: appSetting('readingFontSize', F.readingFontSize),
    },
    {
      id: 'readingLineHeight', label: 'Reading line height', page: 'appearance', group: 'Fonts',
      keywords: 'markdown prose line spacing leading',
      description: 'A multiple of the text size, 1.2 to 2.4.',
      Control: ReadingLineHeightControl, binding: appSetting('readingLineHeight', F.readingLineHeight),
    },
    {
      id: 'readingLineWidth', label: 'Reading line width', page: 'appearance', group: 'Fonts',
      keywords: 'markdown prose measure column max width characters ch',
      description: `The longest line in the Markdown view and the rendered diff, in characters, 20 to 240. Empty: none (their usual 880 px column); + from there starts at ${READING_WIDTH_START}.`,
      Control: ReadingWidthControl, binding: appSetting('readingLineWidth', F.readingLineWidth),
    },
    ...(WINDOW_PLATFORM ? [systemTitleBarSetting(WINDOW_PLATFORM)] : []),
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
