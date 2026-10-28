// @vitest-environment happy-dom
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { act } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AppSettings } from '../api/gen/AppSettings';

vi.mock('../api/client', () => ({ api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null) } }));

const { DEFAULT_SETTINGS, useAppState } = await import('../app/state');
const { DEFAULT_FONT_SETTINGS, DEFAULT_UI_FONT, clampReadingLineHeight, clampReadingWidth, fontVars, proseFontFamily, readingFontFamily, useUiFont } = await import('./fontSettings');
const { applyFonts, bindFontsToSettings, UI_FONT_STORAGE_KEY } = await import('./fonts');

const here = dirname(fileURLToPath(import.meta.url));
const css = (path: string) => readFileSync(join(here, path), 'utf8');
const rule = (text: string, selector: string) => text.match(new RegExp(`(?:^|\\n)${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`))?.[1] ?? '';
const EMOJI = '"Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji"';
const root = document.documentElement;
const v = (name: string) => root.style.getPropertyValue(name);

describe('the font settings', () => {
  it("default to today's look: no family of their own, 13 px at 1.55, the panes' own column", () => {
    expect(DEFAULT_SETTINGS).toMatchObject(DEFAULT_FONT_SETTINGS);
    expect(DEFAULT_FONT_SETTINGS).toEqual({ readingFontFamily: '', readingFontSize: 13, readingLineHeight: 1.55, readingLineWidth: 0, uiFontFamily: '' });
    expect(fontVars(DEFAULT_FONT_SETTINGS)).toEqual({
      '--font-ui': null, '--reading-font': null, '--reading-size': '13px', '--reading-line-height': '1.55', '--md-measure': null, '--md-measure-split': null,
    });
  });

  it('with no generic family, a list gets sans-serif after it, then the emoji fonts; empty is none', () => {
    expect(proseFontFamily('Inter')).toBe(`Inter, sans-serif, ${EMOJI}`);
    expect(proseFontFamily(" 'Noto Serif', serif, ")).toBe(`'Noto Serif', serif, ${EMOJI}`);
    expect(proseFontFamily('Charter, ui-serif')).toBe(`Charter, ui-serif, ${EMOJI}`);
    expect(proseFontFamily('  ')).toBe('');
    // The reading font falls back to the UI font, whichever that is.
    expect(readingFontFamily('')).toBe('var(--font-ui)');
    expect(readingFontFamily('Literata')).toBe(`Literata, sans-serif, ${EMOJI}`);
  });

  it('clamp the line height to 1.2-2.4 and the line width to 20-240 ch, 0 being none', () => {
    expect(clampReadingLineHeight(1)).toBe(1.2);
    expect(clampReadingLineHeight(3)).toBe(2.4);
    expect(clampReadingLineHeight(1.678)).toBe(1.68);
    expect(clampReadingLineHeight(Number.NaN)).toBe(1.55);
    expect(clampReadingWidth(0)).toBe(0);
    expect(clampReadingWidth(-5)).toBe(0);
    expect(clampReadingWidth(5)).toBe(20);
    expect(clampReadingWidth(72.4)).toBe(72);
    expect(clampReadingWidth(999)).toBe(240);
  });

  it('set the variables markdown.css and diff.css read, and the split diff two columns and their gap', () => {
    expect(fontVars({ readingFontFamily: 'Literata', readingFontSize: 40, readingLineHeight: 1.8, readingLineWidth: 72, uiFontFamily: 'Inter' })).toEqual({
      '--font-ui': `Inter, sans-serif, ${EMOJI}`,
      '--reading-font': `Literata, sans-serif, ${EMOJI}`,
      '--reading-size': '32px',
      '--reading-line-height': '1.8',
      '--md-measure': '72ch',
      '--md-measure-split': 'calc(144ch + 24px)',
    });
  });

  it("are what the CSS reads, its fallbacks today's values: prose in the UI font at 1.55, 12 px outside the panes, 880 px columns", () => {
    expect(css('tokens.css')).toContain(`--font-ui: ${DEFAULT_UI_FONT};`);
    const md = rule(css('../markdown/markdown.css'), '.md');
    expect(md).toContain('font-family: var(--reading-font, var(--font-ui));');
    // The panes set --md-font-size (the reading size); elsewhere 1 px less: 12 px by default.
    expect(md).toContain('font-size: var(--md-font-size, max(8px, calc(var(--reading-size, 13px) - 1px)));');
    expect(md).toContain('line-height: var(--reading-line-height, 1.55);');
    // Code keeps its ratio to the prose, in the editor's family where a pane sets it.
    expect(rule(css('../markdown/markdown.css'), '.md code')).toContain('font: 0.92em var(--md-code-font, var(--font-mono));');
    const diff = css('../diff/diff.css');
    expect(rule(diff, '.md-rendered > *')).toContain('max-width: var(--md-measure, 880px); margin-inline: auto;');
    expect(rule(diff, '.md-rendered > .md-diff-split')).toContain('max-width: var(--md-measure-split, 1784px);');
  });
});

describe('applyFonts', () => {
  afterEach(() => {
    applyFonts(root, DEFAULT_FONT_SETTINGS);
  });

  it('writes the variables on the root, removes the unset ones, and tells useUiFont of a UI font change', () => {
    applyFonts(root, { ...DEFAULT_FONT_SETTINGS, uiFontFamily: 'Inter', readingFontFamily: 'Literata', readingLineWidth: 60 });
    expect(v('--font-ui')).toBe(`Inter, sans-serif, ${EMOJI}`);
    expect(v('--reading-font')).toBe(`Literata, sans-serif, ${EMOJI}`);
    expect(v('--md-measure')).toBe('60ch');
    expect(root.dataset.uiFont).toBe('');
    expect(useUiFont.getState().family).toBe(`Inter, sans-serif, ${EMOJI}`);
    applyFonts(root, DEFAULT_FONT_SETTINGS);
    expect(v('--font-ui')).toBe('');
    expect(v('--reading-font')).toBe('');
    expect(v('--md-measure')).toBe('');
    expect(v('--reading-size')).toBe('13px');
    expect(root.dataset.uiFont).toBeUndefined();
    expect(useUiFont.getState().family).toBe('');
  });
});

describe('bindFontsToSettings', () => {
  const settle = (patch: Partial<AppSettings>, loaded = true) => act(() => useAppState.setState((s) => ({ loaded, settings: { ...s.settings, ...patch } })));
  let unbind: (() => void) | undefined;
  beforeEach(() => {
    localStorage.clear();
    useAppState.setState({ loaded: false, settings: DEFAULT_SETTINGS });
  });
  afterEach(() => {
    unbind?.();
    unbind = undefined;
    applyFonts(root, DEFAULT_FONT_SETTINGS);
  });

  it('shows the mirrored UI font until the settings load, then follows them and mirrors the UI font', () => {
    localStorage.setItem(UI_FONT_STORAGE_KEY, 'Inter');
    unbind = bindFontsToSettings(root);
    expect(v('--font-ui')).toBe(`Inter, sans-serif, ${EMOJI}`);
    settle({ uiFontFamily: '', readingFontSize: 15 });
    expect(v('--font-ui')).toBe('');
    expect(v('--reading-size')).toBe('15px');
    expect(localStorage.getItem(UI_FONT_STORAGE_KEY)).toBeNull();
    settle({ uiFontFamily: 'Cantarell', readingLineHeight: 2 });
    expect(v('--font-ui')).toBe(`Cantarell, sans-serif, ${EMOJI}`);
    expect(v('--reading-line-height')).toBe('2');
    expect(localStorage.getItem(UI_FONT_STORAGE_KEY)).toBe('Cantarell');
  });
});
