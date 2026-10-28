import { Palette } from 'lucide-react';
import { useEffect, useState, type CSSProperties } from 'react';
import { useAppState } from '../../app/state';
import { DEFAULT_DENSITY, DENSITIES, useDensity, type Density } from '../../theme/density';
import { AppearanceBody } from '../../theme/editor/AppearanceBody';
import { ThemeSelect } from '../../theme/editor/ThemeSelect';
import {
  clampReadingFont, clampReadingLineHeight, clampReadingWidth, DEFAULT_FONT_SETTINGS as F, DEFAULT_UI_FONT, proseFontFamily, READING_FONT_MAX, READING_FONT_MIN,
  READING_LINE_HEIGHT_MAX, READING_LINE_HEIGHT_MIN, READING_WIDTH_MAX, READING_WIDTH_MIN, readingFontFamily,
} from '../../theme/fontSettings';
import { displayChord } from '../../ui/platformKeys';
import { appSetting, storeBinding } from '../bindings';
import { NumberField, TextField } from '../frame/fields';
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

/** The font families' preview: Vietnamese stacked marks (drawn over the wrong letter by some
 * fonts) beside plain Latin. */
export const PROSE_PREVIEW = 'Đảo chiều tối thiểu — The quick brown fox';

const save = (patch: Partial<typeof F>) => useAppState.getState().setSettings(patch);

/** A family's box, and under it a line in what it types, as it types. */
function FamilyField({ id, value, placeholder, preview }: { id: 'uiFontFamily' | 'readingFontFamily'; value: string; placeholder: string; preview(draft: string): CSSProperties }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  return (
    <div className="setting-font-family">
      <TextField id={id} value={value} placeholder={placeholder} onDraft={setDraft} onCommit={(v) => save(id === 'uiFontFamily' ? { uiFontFamily: v } : { readingFontFamily: v })} />
      <div className="setting-font-preview" data-testid={`${id}-preview`} aria-hidden style={preview(draft)}>{PROSE_PREVIEW}</div>
    </div>
  );
}

function UiFontControl() {
  const value = useAppState((s) => s.settings.uiFontFamily);
  return <FamilyField id="uiFontFamily" value={value} placeholder="System UI font" preview={(d) => ({ fontFamily: proseFontFamily(d) || DEFAULT_UI_FONT })} />;
}

/** The reading font's preview is at the reading size and line height. */
function ReadingFontControl() {
  const value = useAppState((s) => s.settings.readingFontFamily);
  const size = useAppState((s) => s.settings.readingFontSize);
  const lineHeight = useAppState((s) => s.settings.readingLineHeight);
  return <FamilyField id="readingFontFamily" value={value} placeholder="UI font" preview={(d) => ({ fontFamily: readingFontFamily(d), fontSize: clampReadingFont(size), lineHeight: clampReadingLineHeight(lineHeight) })} />;
}

function ReadingSizeControl() {
  const px = useAppState((s) => s.settings.readingFontSize);
  return <NumberField id="readingFontSize" value={clampReadingFont(px)} min={READING_FONT_MIN} max={READING_FONT_MAX} step={1} onCommit={(n) => save({ readingFontSize: n })} />;
}

function ReadingLineHeightControl() {
  const n = useAppState((s) => s.settings.readingLineHeight);
  return <NumberField id="readingLineHeight" value={clampReadingLineHeight(n)} min={READING_LINE_HEIGHT_MIN} max={READING_LINE_HEIGHT_MAX} step={0.05} decimals={2} onCommit={(v) => save({ readingLineHeight: v })} />;
}

function ReadingWidthControl() {
  const ch = useAppState((s) => s.settings.readingLineWidth);
  return <NumberField id="readingLineWidth" value={clampReadingWidth(ch)} min={READING_WIDTH_MIN} max={READING_WIDTH_MAX} step={1} blank={0} placeholder="None" onCommit={(v) => save({ readingLineWidth: v })} />;
}

const offs = [
  registerSettingsPage({ id: 'appearance', label: 'Appearance', scope: 'app', order: 20, icon: Palette, summary: 'Density, fonts, theme and colours. The zoom is in the status bar; the editor font is under Diffs and editors.', Body: AppearanceBody }),
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
    {
      id: 'uiFontFamily', label: 'UI font', page: 'appearance', group: 'Fonts',
      keywords: 'font family face typeface interface chrome menus lists graph dialogs css',
      description: 'A CSS font-family list for the menus, lists, graph, settings and dialogs, such as "Inter", sans-serif. Empty: the system\'s UI font. Its size follows the zoom and the density.',
      Control: UiFontControl, binding: appSetting('uiFontFamily', F.uiFontFamily),
    },
    {
      id: 'readingFontFamily', label: 'Reading font', page: 'appearance', group: 'Fonts',
      keywords: 'font family face typeface markdown prose rendered readme description comment serif css',
      description: 'A CSS font-family list for rendered Markdown: the Markdown view, the rendered diff, MR/PR descriptions and comments. Empty: the UI font. Code in it keeps the editor font.',
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
      description: 'The longest line in the Markdown view and the rendered diff, in characters, 20 to 240. Empty: none (their usual 880 px column).',
      Control: ReadingWidthControl, binding: appSetting('readingLineWidth', F.readingLineWidth),
    },
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
