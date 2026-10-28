import type { AppSettings } from '../api/gen/AppSettings';
import { useAppState } from '../app/state';
import { DEFAULT_FONT_SETTINGS, fontVars, useUiFont, type FontSettings } from './fontSettings';

/** The UI font family setting's first-paint mirror, as the theme's (bind.ts): the app's chrome
 * shows before the settings load. `AppSettings.uiFontFamily` stays the source of truth. */
export const UI_FONT_STORAGE_KEY = 'gitvolt.uiFont.v1';

const readMirror = (): string => {
  try {
    return globalThis.localStorage.getItem(UI_FONT_STORAGE_KEY) ?? '';
  } catch {
    return '';
  }
};
const writeMirror = (family: string) => {
  try {
    if (family) globalThis.localStorage.setItem(UI_FONT_STORAGE_KEY, family);
    else globalThis.localStorage.removeItem(UI_FONT_STORAGE_KEY);
  } catch {
    // Blocked storage: the default font until the settings load.
  }
};

const pick = (s: AppSettings): FontSettings => ({
  readingFontFamily: s.readingFontFamily, readingFontSize: s.readingFontSize, readingLineHeight: s.readingLineHeight, readingLineWidth: s.readingLineWidth, uiFontFamily: s.uiFontFamily,
});

/** Writes `s` onto `root` as custom properties (fontSettings.ts `fontVars`); a UI font change
 * also marks `data-ui-font` (form controls take it: tokens.css) and tells `useUiFont`. */
export function applyFonts(root: HTMLElement, s: FontSettings): void {
  for (const [name, value] of Object.entries(fontVars(s))) {
    if (value === null) root.style.removeProperty(name);
    else if (root.style.getPropertyValue(name) !== value) root.style.setProperty(name, value);
  }
  const family = root.style.getPropertyValue('--font-ui');
  if (family) root.dataset.uiFont = '';
  else delete root.dataset.uiFont;
  if (useUiFont.getState().family !== family) useUiFont.setState({ family });
}

/** Keeps the font settings applied: the mirrored UI font from the first paint, then the loaded
 * settings and every change to them. Call once, before the first render. */
export function bindFontsToSettings(root: HTMLElement = document.documentElement): () => void {
  applyFonts(root, { ...DEFAULT_FONT_SETTINGS, uiFontFamily: readMirror() });
  let last = '';
  const sync = ({ loaded, settings }: { loaded: boolean; settings: AppSettings }) => {
    if (!loaded) return;
    const s = pick(settings);
    const key = JSON.stringify(s);
    if (key === last) return;
    last = key;
    applyFonts(root, s);
    writeMirror(s.uiFontFamily.trim());
  };
  sync(useAppState.getState());
  return useAppState.subscribe(sync);
}
