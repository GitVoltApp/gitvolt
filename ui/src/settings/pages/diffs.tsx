import { FileDiff } from 'lucide-react';
import { useAppState } from '../../app/state';
import {
  autoLineHeight, clampEditorFont, clampLineHeight, clampTabSize, DEFAULT_EDITOR_FONT_FAMILY, DEFAULT_EDITOR_SETTINGS, EDITOR_FONT_MAX, EDITOR_FONT_MIN, EDITOR_LINE_HEIGHT_MAX,
  EDITOR_LINE_HEIGHT_MIN, EDITOR_TAB_MAX, EDITOR_TAB_MIN,
} from '../../diff/options';
import { EDITOR_FONT_NAME } from '../../theme/editorFont';
import { displayChord } from '../../ui/platformKeys';
import { appSetting } from '../bindings';
import { selectControl, switchControl } from '../frame/controls';
import { NumberField } from '../frame/fields';
import type { FontRole } from '../frame/FontPicker';
import { EditorFontPreview, FontSetting } from '../frame/FontSetting';
import { registerSettings, registerSettingsPage } from '../registry';

function FontSizeControl() {
  const size = useAppState((s) => s.settings.editorFontSize);
  return <NumberField id="editorFontSize" value={clampEditorFont(size)} min={EDITOR_FONT_MIN} max={EDITOR_FONT_MAX} step={1} onCommit={(n) => useAppState.getState().setSettings({ editorFontSize: n })} />;
}

/** The editor font's picker (monospace families first), its fallbacks the default's. */
export const EDITOR_FONT_ROLE: FontRole = { id: 'editorFontFamily', noun: 'editor', defaultName: EDITOR_FONT_NAME, defaultStack: DEFAULT_EDITOR_FONT_FAMILY, fallback: DEFAULT_EDITOR_FONT_FAMILY, monospace: true };

function FontFamilyControl() {
  return <FontSetting role={EDITOR_FONT_ROLE} placeholder={'"Fira Code", monospace'} preview={(v) => <EditorFontPreview family={v} />} />;
}

function LineHeightControl() {
  const px = useAppState((s) => s.settings.editorLineHeight);
  const size = useAppState((s) => s.settings.editorFontSize);
  // + from Auto starts at Auto's own height for the font size: the first step changes nothing on screen.
  return <NumberField id="editorLineHeight" value={clampLineHeight(px)} min={EDITOR_LINE_HEIGHT_MIN} max={EDITOR_LINE_HEIGHT_MAX} step={1} blank={0} blankStart={autoLineHeight(clampEditorFont(size))} placeholder="Auto" onCommit={(n) => useAppState.getState().setSettings({ editorLineHeight: clampLineHeight(n) })} />;
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
      keywords: 'monaco editor font face typeface monospace jetbrains mono css installed picker custom list',
      description: 'Default: JetBrains Mono, which comes with GitVolt. What a chosen font lacks draws in it or another monospace font. "Custom font list…" takes a CSS font-family list. The reading and UI fonts are under Appearance.',
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
      description: 'In px, at least the font size. Empty: automatic, from the font size; + from there starts at the automatic height.',
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
