import { FileDiff } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useAppState } from '../../app/state';
import {
  clampEditorFont, clampLineHeight, clampTabSize, DEFAULT_EDITOR_SETTINGS, EDITOR_FONT_MAX, EDITOR_FONT_MIN, EDITOR_LINE_HEIGHT_MAX, EDITOR_TAB_MAX, EDITOR_TAB_MIN, editorFontFamily,
} from '../../diff/options';
import { displayChord } from '../../ui/platformKeys';
import { appSetting } from '../bindings';
import { selectControl, switchControl } from '../frame/controls';
import { NumberField, TextField } from '../frame/fields';
import { registerSettings, registerSettingsPage } from '../registry';

/** The font family's preview: Vietnamese stacked marks (drawn over the wrong letter by some
 * fallback fonts), the look-alike characters, and code's brackets. */
export const FONT_PREVIEW = 'const tối = "thiểu"; // 0O1lI {}';

function FontSizeControl() {
  const size = useAppState((s) => s.settings.editorFontSize);
  return <NumberField id="editorFontSize" value={clampEditorFont(size)} min={EDITOR_FONT_MIN} max={EDITOR_FONT_MAX} step={1} onCommit={(n) => useAppState.getState().setSettings({ editorFontSize: n })} />;
}

/** The family's box, and under it a line of code in what it types, as it types. */
function FontFamilyControl() {
  const value = useAppState((s) => s.settings.editorFontFamily);
  const ligatures = useAppState((s) => s.settings.editorFontLigatures);
  const size = useAppState((s) => s.settings.editorFontSize);
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  return (
    <div className="setting-font-family">
      <TextField id="editorFontFamily" value={value} placeholder="JetBrains Mono" onDraft={setDraft} onCommit={(v) => useAppState.getState().setSettings({ editorFontFamily: v })} />
      <div className="setting-font-preview" data-testid="font-preview" aria-hidden style={{ fontFamily: editorFontFamily(draft), fontSize: clampEditorFont(size), fontVariantLigatures: ligatures ? 'normal' : 'none' }}>{FONT_PREVIEW}</div>
    </div>
  );
}

function LineHeightControl() {
  const px = useAppState((s) => s.settings.editorLineHeight);
  return <NumberField id="editorLineHeight" value={clampLineHeight(px)} min={0} max={EDITOR_LINE_HEIGHT_MAX} step={1} blank={0} placeholder="Auto" onCommit={(n) => useAppState.getState().setSettings({ editorLineHeight: clampLineHeight(n) })} />;
}

function TabSizeControl() {
  const n = useAppState((s) => s.settings.editorTabSize);
  return <NumberField id="editorTabSize" value={clampTabSize(n)} min={EDITOR_TAB_MIN} max={EDITOR_TAB_MAX} step={1} onCommit={(v) => useAppState.getState().setSettings({ editorTabSize: v })} />;
}

const D = DEFAULT_EDITOR_SETTINGS;
const offs = [
  registerSettingsPage({ id: 'diffs', label: 'Diffs and editors', scope: 'app', order: 50, icon: FileDiff, summary: 'The diff and file views, the working copy and the merge editor.' }),
  registerSettings([
    {
      id: 'editorFontSize', label: 'Font size', page: 'diffs', group: 'Editor',
      keywords: 'monaco diff editor text size px zoom',
      description: `Text in the diffs, file views and editors, 8 to 32 px. Over one, ${displayChord('Mod+=')} and ${displayChord('Mod+-')} change it, ${displayChord('Mod+0')} resets it.`,
      Control: FontSizeControl, binding: appSetting('editorFontSize', D.editorFontSize),
    },
    {
      id: 'editorFontFamily', label: 'Font family', page: 'diffs', group: 'Editor',
      keywords: 'monaco editor font face typeface monospace jetbrains mono css',
      description: 'A CSS font-family list, such as "Fira Code", monospace. Empty: JetBrains Mono, which comes with GitVolt.',
      Control: FontFamilyControl, binding: appSetting('editorFontFamily', D.editorFontFamily),
    },
    {
      id: 'editorFontLigatures', label: 'Font ligatures', page: 'diffs', group: 'Editor',
      keywords: 'monaco editor font ligature arrows => != calt',
      description: 'Draws character sequences such as => and != as one symbol, in a font that has them.',
      Control: switchControl('editorFontLigatures'), binding: appSetting('editorFontLigatures', D.editorFontLigatures),
    },
    {
      id: 'editorLineHeight', label: 'Line height', page: 'diffs', group: 'Editor',
      keywords: 'monaco editor line spacing height px',
      description: 'In px, at least the font size. Empty: automatic, from the font size.',
      Control: LineHeightControl, binding: appSetting('editorLineHeight', D.editorLineHeight),
    },
    {
      id: 'editorTabSize', label: 'Tab width', page: 'diffs', group: 'Editor',
      keywords: 'monaco editor tab size indent columns spaces',
      description: "How many columns a tab character takes, 1 to 16. It changes how files look, never what they hold.",
      Control: TabSizeControl, binding: appSetting('editorTabSize', D.editorTabSize),
    },
    {
      id: 'editorWhitespace', label: 'Show whitespace', page: 'diffs', group: 'Editor', labelFor: 'group',
      keywords: 'monaco editor whitespace spaces tabs dots render invisible',
      description: 'Marks spaces and tabs: at boundaries leaves out single spaces between words.',
      Control: selectControl('editorWhitespace', [['none', 'None'], ['boundary', 'At boundaries'], ['all', 'Everywhere']] as const),
      binding: appSetting('editorWhitespace', D.editorWhitespace),
    },
    {
      id: 'editorMinimap', label: 'Minimap', page: 'diffs', group: 'Editor',
      keywords: 'monaco editor minimap overview scroll',
      description: 'A small view of the whole file beside the scrollbar.',
      Control: switchControl('editorMinimap'), binding: appSetting('editorMinimap', D.editorMinimap),
    },
    {
      id: 'stickyScroll', label: 'Sticky scroll', page: 'diffs', group: 'Editor',
      keywords: 'monaco pin scope header',
      description: "Pins the enclosing scope's first line at the top. With brace-on-next-line code (common in PHP) it may show only the brace.",
      Control: switchControl('stickyScroll'), binding: appSetting('editorStickyScroll', D.editorStickyScroll),
    },
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
