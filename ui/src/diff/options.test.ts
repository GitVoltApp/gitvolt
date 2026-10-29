import { describe, expect, it } from 'vitest';
import {
  clampEditorFont, clampLineHeight, clampTabSize, DEFAULT_EDITOR_FONT_FAMILY, DEFAULT_EDITOR_SETTINGS, diffEditorOptions, EDITOR_SCROLLBAR, editorDisplayOptions, editorFontFamily, editorSettingsChanged, fileViewOptions,
  HUNK_REGIONS, INLINE_BREAKPOINT_PX, modelTabOptions, renderWhitespaceOf,
} from './options';

const D = DEFAULT_EDITOR_SETTINGS;

describe('Monaco options', () => {
  it('keeps every diff feature on but sticky scroll, which is off by default (H7)', () => {
    const o = diffEditorOptions({ mode: 'split', ignoreWhitespace: false, wordWrap: false }, true);
    expect(o.minimap).toEqual({ enabled: true });
    expect([o.renderOverviewRuler, o.renderIndicators, o.enableSplitViewResizing, o.useInlineViewWhenSpaceIsLimited, o.diffCodeLens, o.folding]).toEqual([true, true, true, true, true, true]);
    // Monaco's revert arrow is off: it discarded working-copy lines with no tooltip.
    expect(o.renderMarginRevertIcon).toBe(false);
    expect(o.renderGutterMenu).toBe(false);
    // No ambiguous-unicode banner popping in and out on a changing file; invisible characters stay marked.
    expect(o.unicodeHighlight).toEqual({ ambiguousCharacters: false, invisibleCharacters: true, nonBasicASCII: false });
    expect(o.experimental).toEqual({ showMoves: true, showEmptyDecorations: true });
    expect(o.stickyScroll).toEqual({ enabled: false });
    expect(o.renderSideBySideInlineBreakpoint).toBe(INLINE_BREAKPOINT_PX);
    expect([o.readOnly, o.automaticLayout, o.renderValidationDecorations]).toEqual([true, false, 'off']);
  });

  it('maps the modes and toggles', () => {
    const hunk = diffEditorOptions({ mode: 'hunk', ignoreWhitespace: true, wordWrap: true }, false);
    expect([hunk.renderSideBySide, hunk.hideUnchangedRegions, hunk.ignoreTrimWhitespace, hunk.wordWrap, hunk.contextmenu]).toEqual([false, { ...HUNK_REGIONS }, true, 'on', false]);
    expect(HUNK_REGIONS).toEqual({ enabled: true, contextLineCount: 3, minimumLineCount: 3, revealLineCount: 20 });
    const inline = diffEditorOptions({ mode: 'inline', ignoreWhitespace: false, wordWrap: false }, true);
    expect([inline.renderSideBySide, inline.hideUnchangedRegions, inline.ignoreTrimWhitespace, inline.wordWrap]).toEqual([false, { enabled: false }, false, 'off']);
    expect(diffEditorOptions({ mode: 'split', ignoreWhitespace: false, wordWrap: false }, true).renderSideBySide).toBe(true);
    expect(fileViewOptions(true, true)).toMatchObject({ readOnly: true, wordWrap: 'on', minimap: { enabled: true }, contextmenu: true });
  });

  it('sticky scroll follows its setting in both editors (H7)', () => {
    const p = { mode: 'inline', ignoreWhitespace: false, wordWrap: false } as const;
    expect(diffEditorOptions(p, true, { ...D, editorStickyScroll: true }).stickyScroll).toEqual({ enabled: true });
    expect(diffEditorOptions(p, true, D).stickyScroll).toEqual({ enabled: false });
    expect(fileViewOptions(false, true).stickyScroll).toEqual({ enabled: false });
    expect(fileViewOptions(false, true, { ...D, editorStickyScroll: true }).stickyScroll).toEqual({ enabled: true });
  });

  it("both editors use the app's slim, shadowless scrollbars (no arrows: Monaco's default)", () => {
    expect(EDITOR_SCROLLBAR).toEqual({ verticalScrollbarSize: 10, horizontalScrollbarSize: 10, useShadows: false });
    expect(diffEditorOptions({ mode: 'inline', ignoreWhitespace: false, wordWrap: false }, true).scrollbar).toEqual(EDITOR_SCROLLBAR);
    expect(fileViewOptions(false, true).scrollbar).toEqual(EDITOR_SCROLLBAR);
  });
});

describe('overflow widgets (G.1)', () => {
  it('every editor places its hovers and messages in viewport coordinates (the overflow layer)', () => {
    expect(diffEditorOptions({ mode: 'split', ignoreWhitespace: false, wordWrap: false }, true).fixedOverflowWidgets).toBe(true);
    expect(fileViewOptions(false, true).fixedOverflowWidgets).toBe(true);
  });
});

describe('editor font size', () => {
  it('clamps to 8-32 whole px, 13 for a non-number, and reaches both editors', () => {
    expect([clampEditorFont(4), clampEditorFont(13.4), clampEditorFont(99), clampEditorFont(Number.NaN)]).toEqual([8, 13, 32, 13]);
    expect(diffEditorOptions({ mode: 'split', ignoreWhitespace: false, wordWrap: false }, true, { ...D, editorFontSize: 16 }).fontSize).toBe(16);
    expect(fileViewOptions(false, true, { ...D, editorFontSize: 99 }).fontSize).toBe(32);
    expect(fileViewOptions(false, true).fontSize).toBe(13);
  });
});

describe('the editor settings', () => {
  const p = { mode: 'split', ignoreWhitespace: false, wordWrap: false } as const;

  it("by default: JetBrains Mono's stack, no ligatures, automatic line height, no whitespace marks, the minimap on", () => {
    expect(DEFAULT_EDITOR_FONT_FAMILY).toBe("'JetBrains Mono', 'Noto Sans Mono', 'Ubuntu Sans Mono', 'Liberation Mono', monospace");
    const want = { fontSize: 13, fontFamily: DEFAULT_EDITOR_FONT_FAMILY, fontLigatures: false, lineHeight: 0, renderWhitespace: 'none', minimap: { enabled: true }, stickyScroll: { enabled: false } };
    expect(editorDisplayOptions(D)).toEqual(want);
    expect(diffEditorOptions(p, true)).toMatchObject(want);
    expect(fileViewOptions(false, true)).toMatchObject(want);
    expect(D.editorTabSize).toBe(4);
  });

  it('each one reaches the diff and file options', () => {
    const s = { ...D, editorFontSize: 15, editorFontFamily: "'Fira Code', monospace", editorFontLigatures: true, editorLineHeight: 22, editorWhitespace: 'boundary' as const, editorMinimap: false };
    const want = { fontSize: 15, fontFamily: "'Fira Code', monospace", fontLigatures: true, lineHeight: 22, renderWhitespace: 'boundary', minimap: { enabled: false } };
    expect(diffEditorOptions(p, true, s)).toMatchObject(want);
    expect(fileViewOptions(false, true, s)).toMatchObject(want);
  });

  it("one family gets the default's fallbacks after it; a list without a generic family monospace; empty is the default stack", () => {
    expect(editorFontFamily('')).toBe(DEFAULT_EDITOR_FONT_FAMILY);
    expect(editorFontFamily('   ')).toBe(DEFAULT_EDITOR_FONT_FAMILY);
    expect(editorFontFamily('Iosevka')).toBe('"Iosevka", "JetBrains Mono", "Noto Sans Mono", "Ubuntu Sans Mono", "Liberation Mono", monospace');
    // The picker's family (old settings' bare names too), quoted or not; not twice in the stack.
    expect(editorFontFamily("'Noto Sans Mono'")).toBe('"Noto Sans Mono", "JetBrains Mono", "Ubuntu Sans Mono", "Liberation Mono", monospace');
    expect(editorFontFamily('"Cascadia Code, Italic"')).toBe('"Cascadia Code, Italic", "JetBrains Mono", "Noto Sans Mono", "Ubuntu Sans Mono", "Liberation Mono", monospace');
    expect(editorFontFamily("'Fira Code', 'Noto Sans Mono',")).toBe("'Fira Code', 'Noto Sans Mono', monospace");
    expect(editorFontFamily("'Fira Code', monospace")).toBe("'Fira Code', monospace");
    expect(editorFontFamily('Menlo, ui-monospace')).toBe('Menlo, ui-monospace');
    expect(editorFontFamily('Georgia, serif')).toBe('Georgia, serif');
    // A quoted "monospace" names a font, not the generic family.
    expect(editorFontFamily('"monospace"')).toBe('"monospace", monospace');
  });

  it('line height: 0 is automatic; else whole px within 8-64, and never below the font size', () => {
    expect([clampLineHeight(0), clampLineHeight(-3), clampLineHeight(Number.NaN), clampLineHeight(3), clampLineHeight(19.6), clampLineHeight(200)]).toEqual([0, 0, 0, 8, 20, 64]);
    expect(editorDisplayOptions({ ...D, editorLineHeight: 0 }).lineHeight).toBe(0);
    expect(editorDisplayOptions({ ...D, editorLineHeight: 10, editorFontSize: 18 }).lineHeight).toBe(18);
    expect(editorDisplayOptions({ ...D, editorLineHeight: 24, editorFontSize: 18 }).lineHeight).toBe(24);
  });

  it('show whitespace maps to renderWhitespace (none, boundary, all; anything else none)', () => {
    expect([renderWhitespaceOf('none'), renderWhitespaceOf('boundary'), renderWhitespaceOf('all'), renderWhitespaceOf('selection' as never)]).toEqual(['none', 'boundary', 'all', 'none']);
    expect(diffEditorOptions(p, true, { ...D, editorWhitespace: 'all' }).renderWhitespace).toBe('all');
  });

  it('tab width: whole columns within 1-16, 4 for a non-number; display only, keeping the guessed indentation', () => {
    expect([clampTabSize(0), clampTabSize(2.4), clampTabSize(99), clampTabSize(Number.NaN)]).toEqual([1, 2, 16, 4]);
    // Indented with spaces: the guessed indent size stays, so Tab indents as the file does.
    expect(modelTabOptions({ insertSpaces: true, indentSize: 2 }, 8)).toEqual({ tabSize: 8, indentSize: 2 });
    // Indented with tabs: a tab at a time, whatever its width. Never `insertSpaces`: the text isn't rewritten.
    expect(modelTabOptions({ insertSpaces: false, indentSize: 4 }, 8)).toEqual({ tabSize: 8 });
    expect(modelTabOptions({ insertSpaces: false, indentSize: 4 }, 40)).toEqual({ tabSize: 16 });
  });

  it('only a change to one of them counts', () => {
    expect(editorSettingsChanged(D, { ...D })).toBe(false);
    expect(editorSettingsChanged(D, { ...D, editorTabSize: 2 })).toBe(true);
    expect(editorSettingsChanged(D, { ...D, editorFontFamily: 'x' })).toBe(true);
    expect(editorSettingsChanged({ ...D, theme: 'a' } as never, { ...D, theme: 'b' } as never)).toBe(false);
  });
});
