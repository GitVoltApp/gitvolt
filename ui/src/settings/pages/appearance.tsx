import { Type } from 'lucide-react';
import { api, errorMessage } from '../../api/client';
import { flushSaves, useAppState } from '../../app/state';
import { useWindowChrome, type WindowChromeInfo } from '../../chrome/windowChrome';
import { confirmDiscardUnsaved } from '../../diff/workingCopy';
import { DEFAULT_DENSITY, DENSITIES, useDensity, type Density } from '../../theme/density';
import {
  clampReadingFont, clampReadingLineHeight, clampReadingWidth, DEFAULT_FONT_SETTINGS as F, DEFAULT_UI_FONT, READING_FONT_MAX, READING_FONT_MIN, READING_LINE_HEIGHT_MAX,
  READING_LINE_HEIGHT_MIN, READING_WIDTH_MAX, READING_WIDTH_MIN,
} from '../../theme/fontSettings';
import { displayChord } from '../../ui/platformKeys';
import { useToast } from '../../ui/toastStore';
import { appSetting, storeBinding } from '../bindings';
import { settingDef, type SettingsCtx } from '../registry';
import { ChoiceGroup, choiceControl } from '../frame/controls';
import { NumberField } from '../frame/fields';
import type { FontRole } from '../frame/FontPicker';
import { FontSetting, ReadingFontPreview, UiFontPreview } from '../frame/FontSetting';
import { registerSettings, registerSettingsPage, type SettingDef } from '../registry';

const DENSITY_LABELS: Record<Density, string> = { compact: 'Compact', standard: 'Standard', comfortable: 'Comfortable' };
const DENSITY_CHOICES = DENSITIES.map((d) => [d, DENSITY_LABELS[d]] as const);

const save = (patch: Partial<typeof F>) => useAppState.getState().setSettings(patch);

/** The UI font's picker: a family gets the platform UI font's stack after it. */
export const UI_FONT_ROLE: FontRole = { id: 'uiFontFamily', noun: 'UI', defaultName: 'system UI font', defaultStack: DEFAULT_UI_FONT, fallback: DEFAULT_UI_FONT };
/** The reading font's: its default is the UI font, whichever is set. */
export const READING_FONT_ROLE: FontRole = { id: 'readingFontFamily', noun: 'reading', defaultName: 'the UI font', defaultStack: 'var(--font-ui)', fallback: DEFAULT_UI_FONT };

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

const RESTART_NOTE = 'Takes effect when GitVolt restarts.';
const TITLE_BAR_DESCRIPTIONS: Record<'linux' | 'windows' | 'macos', string> = {
  linux: [
    '• Separate: GitVolt\'s title bar, above the tabs.',
    '• Combined: the tabs in the title bar, with ☰ and the window buttons.',
    '• System: the desktop\'s own title bar and frame, ☰ in the tab bar.',
    RESTART_NOTE,
  ].join('\n'),
  windows: [
    '• Separate: GitVolt\'s title bar, above the tabs.',
    '• Combined: the tabs in the title bar, with ☰ and the window buttons.',
    '• System: Windows\' own title bar and frame, ☰ in the tab bar.',
    RESTART_NOTE,
  ].join('\n'),
  macos: [
    '• Separate: GitVolt\'s title bar with the traffic lights, above the tabs.',
    '• Combined: the tabs in the title bar, after the traffic lights and ☰.',
    '• System: macOS\'s standard title bar, ☰ in the tab bar.',
    RESTART_NOTE,
  ].join('\n'),
};

export type TitleBarChoice = 'separate' | 'combined' | 'system';
const TITLE_BAR_CHOICES: readonly (readonly [TitleBarChoice, string])[] = [['separate', 'Separate'], ['combined', 'Combined'], ['system', 'System']];

/** The choice the two stored switches make: the system title bar wins over compact mode
 * (Combined), which needs GitVolt's own. */
export function titleBarChoice(s: { systemTitleBar: boolean; compactTitleBar: boolean }): TitleBarChoice {
  return s.systemTitleBar ? 'system' : s.compactTitleBar ? 'combined' : 'separate';
}

/** The choice the window was built with, from the window chrome the app handed over. */
function runningChoice(info: WindowChromeInfo | null): TitleBarChoice {
  return info?.frame === 'system' ? 'system' : info?.compact ? 'combined' : 'separate';
}

const restart = async () => {
  try {
    // Unsaved edits ask first, as the window's close does (cancelled: no restart). The choice is
    // saved before this process goes; the window's placement and tabs as at quit.
    if (!(await confirmDiscardUnsaved())) return;
    await flushSaves();
    await api.relaunchApp();
  } catch (e) {
    useToast.getState().show(errorMessage(e), { error: true });
  }
};

/** Separate, Combined or System, and under it Restart GitVolt once the choice isn't what the
 * window was built with. That line keeps its space while hidden, so nothing moves. In an X11
 * session it's System, fixed, and the same line says why. */
function TitleBarControl({ ctx }: { ctx: SettingsCtx }) {
  const systemOnly = useWindowChrome((s) => !!s.info?.systemOnly);
  const running = useWindowChrome((s) => runningChoice(s.info));
  const def = settingDef('titleBar')!;
  const stored = useAppState((s) => titleBarChoice(s.settings));
  const choice = systemOnly ? 'system' : stored;
  const set = (c: TitleBarChoice) => def.binding?.(ctx)?.set(c);
  const restartShown = !systemOnly && choice !== running;
  return (
    <div className="title-bar-choice">
      <ChoiceGroup<TitleBarChoice> id="titleBar" value={choice} options={TITLE_BAR_CHOICES} onChange={set} disabled={systemOnly} />
      <div className="title-bar-restart" style={{ visibility: systemOnly || restartShown ? 'visible' : 'hidden' }}>
        {systemOnly ? <span className="setting-hint">An X11 session always has the desktop's title bar.</span> : (
          <>
            <span className="setting-hint">Takes effect after a restart.</span>
            <button type="button" className="primary" onClick={() => void restart()}>Restart GitVolt</button>
          </>
        )}
      </div>
    </div>
  );
}

/** "Title bar": where GitVolt can draw its own (the app's window, on Linux, Windows and macOS),
 * over the two stored switches. In an X11 session there's nothing to change, so nothing to reset. */
export function titleBarSetting(platform: 'linux' | 'windows' | 'macos'): SettingDef {
  return {
    id: 'titleBar', label: 'Title bar', page: 'appearance', group: 'Window', labelFor: 'group',
    keywords: 'title bar titlebar separate combined compact system gitvolt tabs space height row browser window frame decorations buttons minimize maximize close native gnome desktop windows macos traffic lights restart',
    description: TITLE_BAR_DESCRIPTIONS[platform],
    Control: TitleBarControl,
    binding: () => useWindowChrome.getState().info?.systemOnly ? null : {
      get: () => titleBarChoice(useAppState.getState().settings),
      set: (c: TitleBarChoice) => useAppState.getState().setSettings({ systemTitleBar: c === 'system', compactTitleBar: c === 'combined' }),
      defaultValue: 'separate',
      subscribe: (cb) => useAppState.subscribe(cb),
    },
  };
}

const offs = [
  registerSettingsPage({ id: 'appearance', label: 'Appearance', scope: 'app', order: 20, icon: Type, summary: 'Density, the window\'s title bar and fonts. The theme and colours are under Theme, the editor font under Diffs and editors, the zoom in the status bar.' }),
  registerSettings([
    {
      id: 'density', label: 'Density', page: 'appearance', group: 'Density', labelFor: 'group',
      keywords: 'compact standard comfortable row height spacing padding',
      description: 'Row height and padding of the graph and the file lists.',
      Control: choiceControl<Density>('density', DENSITY_CHOICES),
      binding: storeBinding(useDensity, () => useDensity.getState().density, (d) => useDensity.getState().setDensity(d), DEFAULT_DENSITY),
    },
    // Groups show in the order their first setting registers: Window before Fonts.
    ...(WINDOW_PLATFORM ? [titleBarSetting(WINDOW_PLATFORM)] : []),
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
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
