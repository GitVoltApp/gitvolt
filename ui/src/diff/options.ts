import type { AppSettings } from '../api/gen/AppSettings';
import type { EditorWhitespace } from '../api/gen/EditorWhitespace';
import type { EditorDiffPrefs } from './diffPrefs';

/** Hunk mode's collapsed unchanged regions (spec §10.2). */
export const HUNK_REGIONS = { enabled: true, contextLineCount: 3, minimumLineCount: 3, revealLineCount: 20 } as const;
/** Split falls back to Inline only below this width. Monaco's own 900 px would flip it in a
 * normal window, because the center panel sits beside the details panel. */
export const INLINE_BREAKPOINT_PX = 600;
export const EDITOR_FONT_SIZE = 13;
export const EDITOR_FONT_MIN = 8;
export const EDITOR_FONT_MAX = 32;
/** The saved editor font size, whole px within 8-32 (13 for anything that isn't a number). */
export const clampEditorFont = (px: number): number => (Number.isFinite(px) ? Math.min(EDITOR_FONT_MAX, Math.max(EDITOR_FONT_MIN, Math.round(px))) : EDITOR_FONT_SIZE);

/** The app settings every editor is drawn with (Settings › Diffs and editors). */
export type EditorSettings = Pick<AppSettings, 'editorFontSize' | 'editorFontFamily' | 'editorFontLigatures' | 'editorLineHeight' | 'editorTabSize' | 'editorWhitespace' | 'editorMinimap' | 'editorStickyScroll'>;
/** `AppSettings::default()`'s. Sticky scroll is off (H7): Monaco's pops in after a file opens and
 * is revealed, and with no language service its scopes come from indentation, so with PHP's
 * brace-on-next-line style the pinned line is the lone `{` under `function bar()`. */
export const DEFAULT_EDITOR_SETTINGS: EditorSettings = {
  editorFontSize: EDITOR_FONT_SIZE, editorFontFamily: '', editorFontLigatures: false, editorLineHeight: 0, editorTabSize: 4, editorWhitespace: 'none', editorMinimap: true, editorStickyScroll: false,
};
const EDITOR_KEYS = Object.keys(DEFAULT_EDITOR_SETTINGS) as (keyof EditorSettings)[];
/** Whether a settings change touches the editors (anything else, they skip). */
export const editorSettingsChanged = (a: EditorSettings, b: EditorSettings): boolean => EDITOR_KEYS.some((k) => a[k] !== b[k]);

/** The bundled JetBrains Mono (theme/editorFont.ts) first, on every platform; then fonts that draw
 * what it lacks (CJK and other scripts) well. Never Monaco's Linux default, which falls back to
 * DejaVu Sans Mono: it draws Vietnamese stacked marks over the next letter. */
export const DEFAULT_EDITOR_FONT_FAMILY = "'JetBrains Mono', 'Noto Sans Mono', 'Ubuntu Sans Mono', 'Liberation Mono', monospace";
export const GENERIC_FAMILY = /^(monospace|ui-monospace|serif|sans-serif|ui-serif|ui-sans-serif|ui-rounded|system-ui|cursive|fantasy|math|emoji|fangsong)$/i;
/** The font-family list the editors use: the default for an empty setting, else the user's list,
 * with `monospace` after it when it names no generic family, so a font that isn't installed never
 * falls back to the browser's proportional default. */
export function editorFontFamily(list: string): string {
  const s = list.trim().replace(/[\s,]+$/, '');
  if (!s) return DEFAULT_EDITOR_FONT_FAMILY;
  return s.split(',').some((f) => GENERIC_FAMILY.test(f.trim())) ? s : `${s}, monospace`;
}

export const EDITOR_TAB_MIN = 1;
export const EDITOR_TAB_MAX = 16;
/** A tab's width in columns, whole, 1-16 (4 for anything that isn't a number). */
export const clampTabSize = (n: number): number => (Number.isFinite(n) ? Math.min(EDITOR_TAB_MAX, Math.max(EDITOR_TAB_MIN, Math.round(n))) : DEFAULT_EDITOR_SETTINGS.editorTabSize);

/** Line height: 0 is automatic (Monaco's, from the font size); else whole px within 8-64. Below 8,
 * Monaco would read it as a multiple of the font size. */
export const EDITOR_LINE_HEIGHT_MIN = 8;
export const EDITOR_LINE_HEIGHT_MAX = 64;
export const clampLineHeight = (px: number): number => (!Number.isFinite(px) || px <= 0 ? 0 : Math.min(EDITOR_LINE_HEIGHT_MAX, Math.max(EDITOR_LINE_HEIGHT_MIN, Math.round(px))));

/** "Show whitespace" as Monaco's `renderWhitespace` (the same names; anything else is none). */
export const renderWhitespaceOf = (w: EditorWhitespace): 'none' | 'boundary' | 'all' => (w === 'boundary' || w === 'all' ? w : 'none');

/** The options every code editor takes from the settings. A set line height is never below the
 * font size, so a zoomed-in editor's lines don't overlap. */
export function editorDisplayOptions(s: EditorSettings) {
  const fontSize = clampEditorFont(s.editorFontSize);
  const lineHeight = clampLineHeight(s.editorLineHeight);
  return {
    fontSize,
    fontFamily: editorFontFamily(s.editorFontFamily),
    fontLigatures: s.editorFontLigatures,
    lineHeight: lineHeight && Math.max(lineHeight, fontSize),
    renderWhitespace: renderWhitespaceOf(s.editorWhitespace),
    minimap: { enabled: s.editorMinimap },
    stickyScroll: { enabled: s.editorStickyScroll },
  };
}

/**
 * A model's indentation options for the tab width setting. Monaco guesses each model's indentation
 * from its text (`detectIndentation`) and ties the tab's width to it; the setting takes the width
 * over, display only. A model indented with spaces keeps its guessed indent size, so Tab and
 * Shift+Tab in an editable one still indent as the file does; one indented with tabs indents a
 * tab at a time whatever its width. `insertSpaces` stays the guess: the text is never rewritten.
 */
export function modelTabOptions(guessed: { insertSpaces: boolean; indentSize: number }, tabSize: number): { tabSize: number; indentSize?: number } {
  return guessed.insertSpaces ? { tabSize: clampTabSize(tabSize), indentSize: guessed.indentSize } : { tabSize: clampTabSize(tabSize) };
}
/** The app's slim scrollbars (tokens.css; F31), in Monaco's own: 10 px, no shadow. Monaco draws
 * no arrow buttons unless asked, and its colours come from the theme (monaco/theme.ts). */
/** Unicode highlighting: invisible characters stay marked (a hidden bidi or zero-width character is
 * worth seeing in a diff), but "ambiguous" ones (box drawing, curly quotes, non-Latin letters) don't.
 * Their banner ("This document contains many ambiguous unicode characters") popped in and out
 * on every reload of a changing file, such as a growing log. */
export const UNICODE_HIGHLIGHT = { ambiguousCharacters: false, invisibleCharacters: true, nonBasicASCII: false } as const;

export const EDITOR_SCROLLBAR = { verticalScrollbarSize: 10, horizontalScrollbarSize: 10, useShadows: false } as const;

/**
 * Monaco DiffEditor options. Every diff feature stays on (the user's decision): overview ruler,
 * +/- indicators, moved-code detection, split resizing, code lens, folding.
 * Except the margin's revert arrow and the gutter menu that draws it in newer Monaco (the user's call: it discarded working-copy lines with no
 * tooltip; the hunk/line actions and Discard say what they do, and undo). The font, whitespace,
 * minimap and sticky scroll are the user's settings (`editorDisplayOptions`). The only fixed settings are the spec'd read-only and
 * no-language-service ones. It returns plain data (no Monaco import), so it's unit-testable.
 */
export function diffEditorOptions(p: EditorDiffPrefs, contextMenu: boolean, s: EditorSettings = DEFAULT_EDITOR_SETTINGS) {
  return {
    readOnly: true,
    automaticLayout: false,
    renderValidationDecorations: 'off' as const,
    contextmenu: contextMenu,
    ...editorDisplayOptions(s),
    scrollbar: { ...EDITOR_SCROLLBAR },
    fixedOverflowWidgets: true,
    unicodeHighlight: { ...UNICODE_HIGHLIGHT },
    renderOverviewRuler: true,
    renderIndicators: true,
    // The +/- indicators sit in this column, between the line number and the text: Monaco's 10 px
    // left them touching both. The icon is centred in it (diff.css).
    lineDecorationsWidth: 18,
    renderMarginRevertIcon: false,
    // Monaco 0.4x+ draws its revert arrow as the gutter menu (renderMarginRevertIcon alone is the old
    // icon): both off, so no unlabeled arrow can discard working-copy lines.
    renderGutterMenu: false,
    diffCodeLens: true,
    enableSplitViewResizing: true,
    useInlineViewWhenSpaceIsLimited: true,
    renderSideBySideInlineBreakpoint: INLINE_BREAKPOINT_PX,
    diffAlgorithm: 'advanced' as const,
    experimental: { showMoves: true, showEmptyDecorations: true },
    folding: true,
    renderSideBySide: p.mode === 'split',
    hideUnchangedRegions: p.mode === 'hunk' ? { ...HUNK_REGIONS } : { enabled: false },
    ignoreTrimWhitespace: p.ignoreWhitespace,
    wordWrap: p.wordWrap ? ('on' as const) : ('off' as const),
    diffWordWrap: 'inherit' as const,
  };
}

/** Options for File View's read-only editor (spec §10.1). */
export function fileViewOptions(wordWrap: boolean, contextMenu: boolean, s: EditorSettings = DEFAULT_EDITOR_SETTINGS) {
  return {
    readOnly: true,
    automaticLayout: false,
    renderValidationDecorations: 'off' as const,
    contextmenu: contextMenu,
    ...editorDisplayOptions(s),
    scrollbar: { ...EDITOR_SCROLLBAR },
    fixedOverflowWidgets: true,
    unicodeHighlight: { ...UNICODE_HIGHLIGHT },
    folding: true,
    wordWrap: wordWrap ? ('on' as const) : ('off' as const),
  };
}
