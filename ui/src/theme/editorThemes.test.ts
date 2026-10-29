// @vitest-environment happy-dom
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { editorColors } from '../diff/monaco/theme';
import exceptions from '../../build/license-exceptions.json';
import { customThemeDef } from './custom';
import { bindEditorTheme, bindTokenTheme, currentEditorTheme, currentTokenTheme, EDITOR_THEME_THROTTLE_MS, editorThemeName, editorThemeRegistrations, SHIKI_THEMES } from './editorThemes';
import { useTheme } from './store';
import { SHIKI_BUNDLED_THEMES, THEME_IDS, THEMES } from './themes';

describe('editor themes', () => {
  it("bundles only the Shiki themes GitVolt uses, never Shiki's whole registry or a dropped theme", () => {
    expect(Object.keys(SHIKI_THEMES).sort()).toEqual([...SHIKI_BUNDLED_THEMES].sort());
    const dropped = Object.keys(exceptions.dropped.themes);
    expect(dropped).toContain('aurora-x');
    for (const id of dropped) expect(Object.keys(SHIKI_THEMES)).not.toContain(id);
    const src = readFileSync(join(process.cwd(), 'src/theme/editorThemes.ts'), 'utf8');
    expect(src).not.toMatch(/from 'shiki\/themes'/);
  });

  it('registers a theme for every ThemeDef, named as it names it, each with its own overlay', async () => {
    const regs = await Promise.all(editorThemeRegistrations());
    expect(regs.map((r) => r.name)).toEqual(THEME_IDS.map((id) => THEMES[id].editorTheme));
    expect(new Set(regs.map((r) => r.name)).size).toBe(THEME_IDS.length);
    regs.forEach((r, i) => {
      const def = THEMES[THEME_IDS[i]];
      expect(r.colors).toMatchObject(editorColors(def));
      expect(r.colors?.['editor.background']).toBe(def.colors['app-bg0'].toLowerCase());
    });
  });

  it('derives the overlay from the theme tokens, and Default Dark keeps its tuned values', () => {
    const d = editorColors(THEMES['default-dark']);
    expect(d['editor.background']).toBe('#1c1e23');
    expect(d['scrollbarSlider.background']).toBe('#ffffff26');
    expect(d['scrollbarSlider.hoverBackground']).toBe('#ffffff40');
    expect(d['diffEditor.removedTextBackground']).toBe('#d9413d33');
    expect(editorColors(THEMES.darcula)['editor.background']).toBe(THEMES.darcula.colors['app-bg0'].toLowerCase());
    expect(editorColors(THEMES.light)['editor.selectionBackground']).toBe(THEMES.light.colors['selection-bg'].toLowerCase());
  });

  it('pushes the editor theme on bind and on every app theme change, once per change', () => {
    act(() => useTheme.getState().set(THEMES['default-dark']));
    const setTheme = vi.fn();
    const unbind = bindEditorTheme(setTheme);
    expect(setTheme).toHaveBeenLastCalledWith('dark-plus');
    act(() => useTheme.getState().set(THEMES.darcula));
    expect(setTheme).toHaveBeenLastCalledWith('darcula');
    act(() => useTheme.getState().set({ ...THEMES.darcula, graph: ['#123456', ...THEMES.darcula.graph.slice(1)] }));
    expect(setTheme).toHaveBeenCalledTimes(2);
    expect(currentEditorTheme()).toBe('darcula');
    unbind();
  });
});

describe('custom themes in the editor', () => {
  const mine = (colors: Record<string, string>) => customThemeDef({ id: 'custom-0a1b2c3d', name: 'Nord (Custom)', base: 'nord', colors, graph: [] });
  afterEach(() => { vi.useRealTimers(); act(() => useTheme.getState().set(THEMES['default-dark'])); });

  it("colour tokens with the base's Shiki theme; Monaco gets a name per overlay", () => {
    act(() => useTheme.getState().set(mine({})));
    expect(currentTokenTheme()).toBe('nord');
    const first = currentEditorTheme();
    expect(first).toMatch(/^nord--[0-9a-z]+$/);
    act(() => useTheme.getState().set(mine({ 'forge-github': '#000000' })));
    expect(currentEditorTheme()).toBe(first);
    act(() => useTheme.getState().set(mine({ 'app-bg0': '#202020' })));
    expect(currentEditorTheme()).not.toBe(first);
    expect(currentEditorTheme()).toMatch(/^nord--/);
    act(() => useTheme.getState().set(THEMES.light));
    expect([currentTokenTheme(), currentEditorTheme()]).toEqual(['light-plus', 'light-plus']);
  });

  it("names every built-in's and custom theme's Monaco theme as Monaco's defineTheme allows (#14)", () => {
    // standaloneThemeService.js `defineTheme`: anything else is 'Illegal theme name!'.
    const monacoName = /^[a-z0-9-]+$/i;
    const builtIns = THEME_IDS.map((id) => THEMES[id].editorTheme);
    for (const id of THEME_IDS) {
      expect(editorThemeName(THEMES[id])).toMatch(monacoName);
      expect(THEMES[id].editorTheme).not.toContain('--');
      const custom = editorThemeName(customThemeDef({ id: 'custom-0a1b2c3d', name: 'Mine', base: id, colors: { 'app-bg0': '#202020' }, graph: [] }));
      expect(custom).toMatch(monacoName);
      // Its base's family, and never a built-in's name.
      expect(custom.startsWith(`${THEMES[id].editorTheme}--`)).toBe(true);
      expect(builtIns).not.toContain(custom);
    }
  });

  it("the token theme doesn't move for an edit that keeps the base", () => {
    act(() => useTheme.getState().set(THEMES.nord));
    const setTheme = vi.fn();
    const unbind = bindTokenTheme(setTheme);
    act(() => useTheme.getState().set(mine({ 'app-bg0': '#202020' })));
    act(() => useTheme.getState().preview({ colors: { blue: '#ff0000' } }));
    expect(setTheme.mock.calls).toEqual([['nord']]);
    act(() => useTheme.getState().set(THEMES.light));
    expect(setTheme).toHaveBeenLastCalledWith('light-plus');
    unbind();
  });

  it('Monaco follows colour edits at most every 150 ms, the last one landing; a theme switch at once', () => {
    vi.useFakeTimers();
    act(() => useTheme.getState().set(mine({})));
    const setTheme = vi.fn();
    const unbind = bindEditorTheme(setTheme);
    expect(setTheme).toHaveBeenCalledTimes(1);
    act(() => useTheme.getState().preview({ colors: { 'app-bg0': '#101010' } }));
    expect(setTheme).toHaveBeenCalledTimes(2);
    act(() => useTheme.getState().preview({ colors: { 'app-bg0': '#111111' } }));
    act(() => useTheme.getState().preview({ colors: { 'app-bg0': '#121212' } }));
    expect(setTheme).toHaveBeenCalledTimes(2);
    vi.advanceTimersByTime(EDITOR_THEME_THROTTLE_MS);
    expect(setTheme).toHaveBeenCalledTimes(3);
    expect(setTheme).toHaveBeenLastCalledWith(currentEditorTheme());
    vi.advanceTimersByTime(EDITOR_THEME_THROTTLE_MS);
    expect(setTheme).toHaveBeenCalledTimes(3);
    expect(vi.getTimerCount()).toBe(0);
    act(() => useTheme.getState().preview({ colors: { 'app-bg0': '#131313' } }));
    act(() => useTheme.getState().set(THEMES.light));
    expect(setTheme).toHaveBeenLastCalledWith('light-plus');
    unbind();
    expect(vi.getTimerCount()).toBe(0);
  });
});
