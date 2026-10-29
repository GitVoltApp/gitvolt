import { useMemo, type CSSProperties } from 'react';
import { useAppState } from '../app/state';
import { registerKeyHints } from '../shortcuts/hints';
import { clampReadingFont, READING_FONT_SIZE } from '../theme/fontSettings';
import { registerKeys } from '../ui/keyRouter';
import { useToast } from '../ui/toastStore';
import { fontZoomOf, zoomDirection, type ZoomedText } from '../ui/zoom';
import { clampEditorFont, EDITOR_FONT_SIZE, editorFontFamily } from './options';

/**
 * The text size of a file's or a diff's body. Over such a body (`[data-font-zoom]`,
 * `FONT_ZOOM_ATTR`), Ctrl+wheel and Ctrl+= / Ctrl+- / Ctrl+0 change it (1 px a step, 8-32, 0
 * resets to 13) instead of the app zoom (ui/zoom.ts), which they keep everywhere else. Which size
 * follows what has focus (the keys) or what the pointer is over (the wheel), the innermost marked
 * box deciding (`fontZoomOf`):
 *
 * - a rendered Markdown pane (`data-font-zoom="reading"`: File View's and the diff's, the review
 *   cards in it included): the reading size, `readingFontSize` (Settings › Appearance › Fonts);
 * - an editor or a diff (Monaco), the CSV table: the code size, `editorFontSize`.
 *
 * A rendered diff's pane and its Source editor are separate boxes, one of them hidden, so code
 * blocks in the rendered pane go with its prose. The new size shows in a toast ("Reading text
 * 15 px", "Code text 14 px") and is saved like any setting.
 */

/** The size one step in `dir` from `px` (1 in, -1 out, 0 back to Monaco's 13). */
export function nextEditorFont(px: number, dir: 1 | -1 | 0): number {
  return dir === 0 ? EDITOR_FONT_SIZE : clampEditorFont(clampEditorFont(px) + dir);
}

/** The reading size one step in `dir` from `px` (0: back to its default, 13). */
export function nextReadingFont(px: number, dir: 1 | -1 | 0): number {
  return dir === 0 ? READING_FONT_SIZE : clampReadingFont(clampReadingFont(px) + dir);
}

/** The editor's font family setting and ligatures, which the rendered panes' code takes too. */
export interface CodeFont { family: string; ligatures: boolean }

/** A rendered pane's style: its Markdown at the reading size `px` and, given `code`, its code in
 * the editor's font (`--md-code-font`, `--md-code-ligatures`: markdown.css). */
export const readingFontVar = (px: number, code?: CodeFont) =>
  ({ '--md-font-size': `${clampReadingFont(px)}px`, ...(code && { '--md-code-font': editorFontFamily(code.family), '--md-code-ligatures': code.ligatures ? 'normal' : 'none' }) }) as CSSProperties;

/** The editor's code font settings (`readingFontVar`'s `code`). */
export function useEditorCodeFont(): CodeFont {
  const family = useAppState((s) => s.settings.editorFontFamily);
  const ligatures = useAppState((s) => s.settings.editorFontLigatures);
  return useMemo(() => ({ family, ligatures }), [family, ligatures]);
}

/** The editor font size, clamped (the CSV table's text size). */
export function useEditorFontPx(): number {
  return clampEditorFont(useAppState((s) => s.settings.editorFontSize));
}

/** The reading size, clamped: the rendered panes' text size (`MdFontPx`, their `--md-font-size`). */
export function useReadingFontPx(): number {
  return clampReadingFont(useAppState((s) => s.settings.readingFontSize));
}


/** A wheel's travel per step, in px: one mouse notch (100 px in Chromium) is one step; a
 * touchpad's (or a pinch's) small deltas add up to one. */
const WHEEL_STEP_PX = 50;
const LINE_PX = 40;

function step(text: ZoomedText, dir: 1 | -1 | 0) {
  const { settings, setSettings } = useAppState.getState();
  if (text === 'reading') {
    const px = nextReadingFont(settings.readingFontSize, dir);
    if (px !== settings.readingFontSize) setSettings({ readingFontSize: px });
    useToast.getState().show(`Reading text ${px} px`);
    return;
  }
  const px = nextEditorFont(settings.editorFontSize, dir);
  if (px !== settings.editorFontSize) setSettings({ editorFontSize: px });
  useToast.getState().show(`Code text ${px} px`);
}

/** Installs the keys (the key router's app layer, beside the app zoom's) and Ctrl+wheel on
 * `window`. Returns the uninstaller. */
export function installFontZoom(): () => void {
  const offKeys = registerKeys('app', (e) => {
    const dir = zoomDirection(e);
    const text = dir === null ? null : fontZoomOf(e.target);
    if (dir === null || text === null) return;
    e.preventDefault();
    step(text, dir);
    return 'handled';
  });
  let travel = 0;
  const onWheel = (e: WheelEvent) => {
    const text = e.ctrlKey ? fontZoomOf(e.target) : null;
    if (text === null) { travel = 0; return; }
    // Monaco (and the pane) never see it: no scroll, and no zoom of their own.
    e.preventDefault();
    e.stopPropagation();
    travel += e.deltaMode === 1 ? e.deltaY * LINE_PX : e.deltaY;
    if (Math.abs(travel) < WHEEL_STEP_PX) return;
    step(text, travel < 0 ? 1 : -1);
    travel = 0;
  };
  window.addEventListener('wheel', onWheel, { capture: true, passive: false });
  return () => {
    offKeys();
    window.removeEventListener('wheel', onWheel, { capture: true });
  };
}

// Shown in the Keyboard Shortcuts panel (Ctrl+/); metadata only.
registerKeyHints([
  { id: 'key.textSizeIn', section: 'Diff', label: 'Larger text (over code: the code; over rendered Markdown: the reading text; also Ctrl+wheel)', keys: ['Mod+='], source: 'diff/fontZoom.ts' },
  { id: 'key.textSizeOut', section: 'Diff', label: 'Smaller text (over a file or diff)', keys: ['Mod+-'], source: 'diff/fontZoom.ts' },
  { id: 'key.textSizeReset', section: 'Diff', label: 'Reset the text size (over a file or diff)', keys: ['Mod+0'], source: 'diff/fontZoom.ts' },
]);
