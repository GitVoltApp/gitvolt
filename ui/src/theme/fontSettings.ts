import { create } from 'zustand';
import type { AppSettings } from '../api/gen/AppSettings';
import { EDITOR_FONT_MAX, EDITOR_FONT_MIN, GENERIC_FAMILY } from '../diff/options';

/**
 * Settings › Appearance › Fonts: the reading font (rendered Markdown's prose, wherever it shows)
 * and the UI font. Their defaults are today's look: prose in the UI font at 13 px (the Markdown
 * view's and the rendered diff's; MR/PR descriptions and comments are 1 px smaller) and 1.55, in
 * the panes' own 880 px column; the platform's UI font (`--font-ui`, tokens.css).
 *
 * They apply as custom properties on the root (theme/fonts.ts): `--font-ui`, `--reading-font`,
 * `--reading-size`, `--reading-line-height`, and the panes' `--md-measure` (markdown.css,
 * diff.css).
 */
export type FontSettings = Pick<AppSettings, 'readingFontFamily' | 'readingFontSize' | 'readingLineHeight' | 'readingLineWidth' | 'uiFontFamily'>;

/** The size the rendered panes' prose had (the editor's 13): same range as the editor's. */
export const READING_FONT_SIZE = 13;
export const READING_FONT_MIN = EDITOR_FONT_MIN;
export const READING_FONT_MAX = EDITOR_FONT_MAX;
/** markdown.css's line height before the setting. */
export const READING_LINE_HEIGHT = 1.55;
export const READING_LINE_HEIGHT_MIN = 1.2;
export const READING_LINE_HEIGHT_MAX = 2.4;
export const READING_WIDTH_MIN = 20;
export const READING_WIDTH_MAX = 240;

/** `AppSettings::default()`'s. */
export const DEFAULT_FONT_SETTINGS: FontSettings = {
  readingFontFamily: '', readingFontSize: READING_FONT_SIZE, readingLineHeight: READING_LINE_HEIGHT, readingLineWidth: 0, uiFontFamily: '',
};

/** Whole px within 8-32 (13 for anything that isn't a number). */
export const clampReadingFont = (px: number): number => (Number.isFinite(px) ? Math.min(READING_FONT_MAX, Math.max(READING_FONT_MIN, Math.round(px))) : READING_FONT_SIZE);
/** A multiple of the text size within 1.2-2.4, to the hundredth (1.55 for anything that isn't a number). */
export const clampReadingLineHeight = (n: number): number => (Number.isFinite(n) ? Math.min(READING_LINE_HEIGHT_MAX, Math.max(READING_LINE_HEIGHT_MIN, Math.round(n * 100) / 100)) : READING_LINE_HEIGHT);
/** Whole `ch` within 20-240; 0 (and anything not above 0) is none. */
export const clampReadingWidth = (ch: number): number => (!Number.isFinite(ch) || ch <= 0 ? 0 : Math.min(READING_WIDTH_MAX, Math.max(READING_WIDTH_MIN, Math.round(ch))));

/** The emoji fonts `--font-ui` ends with (tokens.css): a user's family keeps colour emoji. */
const EMOJI_FAMILIES = '"Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji"';
/** tokens.css's `--font-ui`, for what shows it while a set one applies (the setting's preview). */
export const DEFAULT_UI_FONT = `system-ui, "Segoe UI", Ubuntu, Cantarell, sans-serif, ${EMOJI_FAMILIES}`;

/** A user's font-family list as CSS, with `sans-serif` after it when it names no generic family
 * (a font that isn't installed never falls back to the browser's serif default), then the emoji
 * fonts; '' when it's empty. */
export function proseFontFamily(list: string): string {
  const s = list.trim().replace(/[\s,]+$/, '');
  if (!s) return '';
  return `${s.split(',').some((f) => GENERIC_FAMILY.test(f.trim())) ? s : `${s}, sans-serif`}, ${EMOJI_FAMILIES}`;
}

/** The reading font as CSS: the user's list, or the UI font's (`--font-ui`) when it's empty. */
export const readingFontFamily = (list: string): string => proseFontFamily(list) || 'var(--font-ui)';

/** The root's custom properties for `s`; null removes one, so its CSS fallback applies (tokens.css's
 * `--font-ui`, the panes' 880 px column). The split diff holds two columns and its 24 px gap. */
export function fontVars(s: FontSettings): Record<string, string | null> {
  const width = clampReadingWidth(s.readingLineWidth);
  return {
    '--font-ui': proseFontFamily(s.uiFontFamily) || null,
    '--reading-font': proseFontFamily(s.readingFontFamily) || null,
    '--reading-size': `${clampReadingFont(s.readingFontSize)}px`,
    '--reading-line-height': String(clampReadingLineHeight(s.readingLineHeight)),
    '--md-measure': width ? `${width}ch` : null,
    '--md-measure-split': width ? `calc(${2 * width}ch + 24px)` : null,
  };
}

/**
 * The UI font applied (`--font-ui`'s value, '' for tokens.css's own). What measures UI text
 * (the graph's chip fit and its node initials, the CSV table's columns) listens, and measures
 * again on a change: their canvases and caches don't see a CSS change by themselves.
 */
export const useUiFont = create<{ family: string }>(() => ({ family: '' }));
