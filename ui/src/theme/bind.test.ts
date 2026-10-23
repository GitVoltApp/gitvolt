// @vitest-environment happy-dom
import { act } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AppSettings } from '../api/gen/AppSettings';
import type { CustomTheme } from '../api/gen/CustomTheme';

vi.mock('../api/client', () => ({ api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null) } }));

const { DEFAULT_SETTINGS, useAppState } = await import('../app/state');
const { THEME_STORAGE_KEY } = await import('./apply');
const { bindThemeToSettings } = await import('./bind');
const { useTheme } = await import('./store');
const { COLOR_TOKENS, THEMES } = await import('./themes');

const settle = (patch: Partial<AppSettings>, loaded = true) => act(() => useAppState.setState((s) => ({ loaded, settings: { ...s.settings, ...patch } })));
const mine: CustomTheme = { id: 'custom-0a1b2c3d', name: 'Nord (Custom)', base: 'nord', colors: { 'app-bg0': '#202020' }, graph: [] };
const mirror = () => JSON.parse(localStorage.getItem(THEME_STORAGE_KEY)!);

describe('bindThemeToSettings', () => {
  let unbind: (() => void) | undefined;
  beforeEach(() => {
    localStorage.clear();
    useAppState.setState({ loaded: false, settings: DEFAULT_SETTINGS });
    act(() => useTheme.getState().set(THEMES['default-dark']));
  });
  afterEach(() => { unbind?.(); unbind = undefined; history.replaceState(null, '', '/'); });

  it('applies the saved theme at startup and follows later changes', () => {
    settle({ theme: 'dracula', graphColorOverrides: {} });
    unbind = bindThemeToSettings();
    expect(useTheme.getState().id).toBe('dracula');
    settle({ theme: 'nord' });
    expect(useTheme.getState().id).toBe('nord');
  });

  it('does not bump the theme version for unrelated settings changes, but does for an edit of the current custom theme', () => {
    settle({ theme: mine.id, customThemes: [mine] });
    unbind = bindThemeToSettings();
    const v = useTheme.getState().version;
    settle({ editorFontSize: 14 });
    settle({ customThemes: [mine, { ...mine, id: 'custom-00000002', name: 'Other' }] });
    expect(useTheme.getState().version).toBe(v);
    settle({ customThemes: [{ ...mine, colors: { 'app-bg0': '#303030' } }] });
    expect(useTheme.getState().version).toBe(v + 1);
    expect(document.documentElement.style.getPropertyValue('--app-bg0')).toBe('#303030');
  });

  it('falls back to Default Dark for an unknown id, and for a custom id the settings lack, with every token set', () => {
    settle({ theme: 'no-such-theme' });
    unbind = bindThemeToSettings();
    expect(useTheme.getState().id).toBe('default-dark');
    const error = vi.spyOn(console, 'error');
    settle({ theme: 'custom-ffffffff', customThemes: [mine] });
    expect(useTheme.getState().id).toBe('default-dark');
    for (const t of COLOR_TOKENS) expect(document.documentElement.style.getPropertyValue(`--${t}`), t).not.toBe('');
    expect(error).not.toHaveBeenCalled();
    error.mockRestore();
  });

  it('a deleted current custom theme leaves its base selected (edit.ts) and applied', () => {
    settle({ theme: mine.id, customThemes: [mine] });
    unbind = bindThemeToSettings();
    expect(useTheme.getState().id).toBe(mine.id);
    settle({ theme: 'nord', customThemes: [] });
    expect(useTheme.getState().def).toBe(THEMES.nord);
  });

  it('lets a ?theme= URL parameter override the saved theme without saving it', () => {
    history.replaceState(null, '', '/?theme=solarized-light');
    settle({ theme: 'nord' });
    unbind = bindThemeToSettings();
    expect(useTheme.getState().id).toBe('solarized-light');
    expect(useAppState.getState().settings.theme).toBe('nord');
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBeNull();
  });

  it('takes a custom theme the settings have from ?theme=', () => {
    history.replaceState(null, '', `/?theme=${mine.id}`);
    settle({ theme: 'light', customThemes: [mine] });
    unbind = bindThemeToSettings();
    expect(useTheme.getState().id).toBe(mine.id);
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBeNull();
  });

  it('paints the mirrored theme before the settings load, and keeps it while they load (R3: no dark flash)', () => {
    localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify({ id: mine.id, kind: 'dark', bg: '#202020', custom: mine }));
    unbind = bindThemeToSettings();
    // Not loaded yet: DEFAULT_SETTINGS (Default Dark) must not be applied over the mirror.
    expect(useTheme.getState().id).toBe(mine.id);
    expect(document.documentElement.style.getPropertyValue('--app-bg0')).toBe('#202020');
    const v = useTheme.getState().version;
    settle({ theme: mine.id, customThemes: [mine] });
    expect(useTheme.getState().version).toBe(v);
  });

  it('mirrors the saved theme, and a custom theme with it, for the next start', () => {
    unbind = bindThemeToSettings();
    settle({ theme: mine.id, customThemes: [mine] });
    expect(mirror()).toEqual({ id: mine.id, kind: 'dark', bg: '#202020', custom: mine });
    settle({ theme: 'default-dark' });
    expect(mirror()).toEqual({ id: 'default-dark', kind: 'dark', bg: '#1c1e23' });
  });

  it('on upgrade: the old lane overrides become the current custom theme, the same lanes as before, at first paint and after', () => {
    // What 0.7.8 left: Nord with lane 1 overridden, in the settings and in the mirror.
    const before = THEMES.nord.graph.map((c, i) => (i === 0 ? '#123456' : c));
    localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify({ id: 'nord', kind: 'dark', bg: '#2e3440', graph: ['#123456', null] }));
    unbind = bindThemeToSettings();
    expect(useTheme.getState().colors.graph).toEqual(before);
    settle({ theme: 'nord', graphColorOverrides: { nord: ['#123456', null] } });
    const s = useAppState.getState().settings;
    expect(s.graphColorOverrides).toEqual({});
    expect(s.customThemes.map((t) => t.name)).toEqual(['Nord (Custom)']);
    expect(s.theme).toBe(s.customThemes[0]!.id);
    expect(useTheme.getState().id).toBe(s.theme);
    expect(useTheme.getState().colors.graph).toEqual(before);
    expect(useTheme.getState().def.colors).toEqual(THEMES.nord.colors);
    expect(mirror().custom.graph).toEqual(before);
  });
});
