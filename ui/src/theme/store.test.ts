// @vitest-environment happy-dom
import { beforeEach, describe, expect, it } from 'vitest';
import { applyTheme, readThemeMirror, resolveColors, THEME_STORAGE_KEY, writeThemeMirror } from './apply';
import { customThemeDef } from './custom';
import { useTheme } from './store';
import { COLOR_TOKENS, THEMES } from './themes';

const custom = { id: 'custom-0a1b2c3d', name: 'Nord (Custom)', base: 'nord', colors: { 'app-bg0': '#202020' }, graph: [] };

describe('useTheme', () => {
  beforeEach(() => useTheme.getState().set(THEMES['default-dark']));

  it('applies every color token and lane color to :root', () => {
    useTheme.getState().set(THEMES.nord);
    const style = document.documentElement.style;
    expect(style.getPropertyValue('--app-bg0')).toBe('#2e3440');
    expect(style.getPropertyValue('--graph-0')).toBe('#88c0d0');
    for (const t of COLOR_TOKENS) expect(style.getPropertyValue(`--${t}`), t).toBe(THEMES.nord.colors[t]);
    expect(document.documentElement.dataset.theme).toBe('nord');
    expect(style.colorScheme).toBe('dark');
    useTheme.getState().set(THEMES.light);
    expect(style.colorScheme).toBe('light');
  });

  it('bumps version on every change so canvases redraw', () => {
    const v = useTheme.getState().version;
    useTheme.getState().set(THEMES.monokai);
    expect(useTheme.getState().version).toBe(v + 1);
  });

  it('applies a custom theme like a built-in', () => {
    useTheme.getState().set(customThemeDef(custom));
    expect(useTheme.getState().id).toBe('custom-0a1b2c3d');
    expect(document.documentElement.style.getPropertyValue('--app-bg0')).toBe('#202020');
    expect(document.documentElement.style.getPropertyValue('--panel-bg0')).toBe(THEMES.nord.colors['panel-bg0']);
  });

  it('previews colours and lanes without changing the applied theme, and restores it', () => {
    useTheme.getState().set(THEMES.nord);
    const style = document.documentElement.style;
    const v = useTheme.getState().version;
    useTheme.getState().preview({ colors: { blue: '#ff0000' }, graph: ['#010101', ...THEMES.nord.graph.slice(1)] });
    expect(style.getPropertyValue('--blue')).toBe('#ff0000');
    expect(style.getPropertyValue('--graph-0')).toBe('#010101');
    expect(useTheme.getState().colors.graph[0]).toBe('#010101');
    expect(useTheme.getState().def.colors.blue).toBe('#ff0000');
    expect(useTheme.getState().version).toBe(v + 1);
    useTheme.getState().preview(null);
    expect(style.getPropertyValue('--blue')).toBe(THEMES.nord.colors.blue);
    expect(useTheme.getState().def).toBe(THEMES.nord);
    // `set` replaces any preview.
    useTheme.getState().preview({ colors: { blue: '#ff0000' } });
    useTheme.getState().set(THEMES.light);
    useTheme.getState().preview(null);
    expect(style.getPropertyValue('--blue')).toBe(THEMES.light.colors.blue);
  });

  it("resolves the canvas colors from the theme's tokens", () => {
    expect(resolveColors(THEMES.light)).toEqual({
      nodeFill: THEMES.light.colors['node-fill'], avatarBackdrop: THEMES.light.colors['avatar-backdrop'], nodeText: THEMES.light.colors['node-text'], collapseStrip: THEMES.light.colors['collapse-strip'], graph: [...THEMES.light.graph],
      laneText: expect.any(Array),
    });
  });

  it('picks avatar initials per lane by contrast; Default Dark keeps white on every lane', () => {
    expect(resolveColors(THEMES['default-dark']).laneText).toEqual(Array(10).fill('#ffffff'));
    const graph = [...THEMES.darcula.graph];
    graph[4] = '#ffffff';
    graph[5] = '#000000';
    const d = resolveColors({ ...THEMES.darcula, graph });
    expect(d.laneText[4]).toBe(THEMES.darcula.colors['app-bg0']);
    expect(d.laneText[5]).toBe('#ffffff');
    // #ffc66d, Darcula's yellow lane: dark initials.
    expect(resolveColors(THEMES.darcula).laneText[4]).toBe('#2b2b2b');
  });
});

describe('the first-paint mirror (R3)', () => {
  beforeEach(() => localStorage.clear());

  it('round-trips a built-in, with the kind and background index.html paints', () => {
    writeThemeMirror(THEMES.nord);
    expect(JSON.parse(localStorage.getItem(THEME_STORAGE_KEY)!)).toEqual({ id: 'nord', kind: 'dark', bg: '#2e3440' });
    expect(readThemeMirror()).toEqual({ id: 'nord', graph: undefined });
  });

  it('carries a custom theme itself, so it resolves before the settings load', () => {
    writeThemeMirror(customThemeDef(custom), custom);
    expect(JSON.parse(localStorage.getItem(THEME_STORAGE_KEY)!)).toEqual({ id: custom.id, kind: 'dark', bg: '#202020', custom });
    expect(readThemeMirror()).toEqual({ id: custom.id, custom });
  });

  it('writes the background as #rrggbb whatever form the theme gives it', () => {
    writeThemeMirror(customThemeDef({ ...custom, colors: { 'app-bg0': 'rgba(16, 32, 48, 0.5)' } }), custom);
    expect(JSON.parse(localStorage.getItem(THEME_STORAGE_KEY)!).bg).toBe('#102030');
  });

  it('reads the lane overrides an older mirror carries', () => {
    localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify({ id: 'nord', kind: 'dark', bg: '#2e3440', graph: ['#123456', null, 'x'] }));
    expect(readThemeMirror()).toEqual({ id: 'nord', graph: ['#123456', null, null] });
  });

  it('ignores a missing, corrupt or unknown mirror, or a custom one without its theme', () => {
    expect(readThemeMirror()).toBeNull();
    localStorage.setItem(THEME_STORAGE_KEY, '{nope');
    expect(readThemeMirror()).toBeNull();
    localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify({ id: 'solarized' }));
    expect(readThemeMirror()).toBeNull();
    localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify({ id: 'custom-0a1b2c3d' }));
    expect(readThemeMirror()).toBeNull();
    localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify({ id: 'custom-0a1b2c3d', custom: { ...custom, id: 'custom-ffffffff' } }));
    expect(readThemeMirror()).toBeNull();
    localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify({ id: 'nord', graph: 'x' }));
    expect(readThemeMirror()).toEqual({ id: 'nord', graph: undefined });
  });

  it('applyTheme writes onto the element it is given', () => {
    const el = document.createElement('div');
    applyTheme(el, THEMES.dracula, resolveColors(THEMES.dracula));
    expect(el.style.getPropertyValue('--app-bg0')).toBe('#282a36');
    expect(el.dataset.theme).toBe('dracula');
  });
});
