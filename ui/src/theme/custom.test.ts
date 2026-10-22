import { describe, expect, it } from 'vitest';
import type { CustomTheme } from '../api/gen/CustomTheme';
import { CORE_COLOURS, customThemeDef, isCustomThemeId, isTokenColour, newCustomThemeId, themeDef, TOKEN_GROUPS, TOKEN_LABELS } from './custom';
import { COLOR_TOKENS, THEME_IDS, THEMES } from './themes';

const ct = (over: Partial<CustomTheme> = {}): CustomTheme => ({ id: 'custom-00000000', name: 'X', base: 'nord', colors: {}, graph: [], ...over });

describe('customThemeDef', () => {
  it('with nothing set, is its base exactly', () => {
    for (const id of THEME_IDS) {
      const def = customThemeDef(ct({ base: id }));
      expect(def.colors, id).toEqual(THEMES[id].colors);
      expect(def.graph, id).toEqual(THEMES[id].graph);
      expect(def).toMatchObject({ id: 'custom-00000000', label: 'X', kind: THEMES[id].kind, editorTheme: THEMES[id].editorTheme, base: id });
    }
  });

  it('lets derived tokens follow an edit, but keeps the ones the base set by hand', () => {
    const light = customThemeDef(ct({ base: 'light', colors: { blue: '#ff0000' } })).colors;
    expect(light.blue).toBe('#ff0000');
    expect(light['hover-row']).not.toBe(THEMES.light.colors['hover-row']);
    expect(light['selected-row']).toBe('rgba(255, 0, 0, 0.16)');
    const nord = customThemeDef(ct({ colors: { red: '#ff0000' } })).colors;
    expect(nord['stats-deleted-bright']).toBe('#e5a0a6');
    expect(nord['diff-removed-line']).toBe('rgba(255, 0, 0, 0.15)');
  });

  it('an explicit derived token wins over derivation', () => {
    expect(customThemeDef(ct({ colors: { blue: '#ff0000', 'hover-row': '#010203' } })).colors['hover-row']).toBe('#010203');
  });

  it('ignores unknown tokens and anything that is not a plain colour', () => {
    const def = customThemeDef(ct({ colors: { 'app-bg0': 'url(x)', bogus: '#fff', 'panel-bg0': 'var(--x)', 'text-normal': 'red; background: url(https://example.com/x)' } }));
    expect(def.colors).toEqual(THEMES.nord.colors);
    expect(def.colors).not.toHaveProperty('bogus');
  });

  it('derives from a set colour in any accepted form', () => {
    const def = customThemeDef(ct({ base: 'light', colors: { blue: 'rgba(255, 0, 0, 0.5)', green: '#0f0' } }));
    expect(def.colors.blue).toBe('rgba(255, 0, 0, 0.5)');
    expect(def.colors['selected-row']).toBe('rgba(255, 0, 0, 0.16)');
    expect(def.colors['diff-inserted-line']).toBe('rgba(0, 255, 0, 0.1)');
  });

  it('an unknown base resolves from Default Dark', () => {
    const def = customThemeDef(ct({ base: 'gone' }));
    expect(def.colors).toEqual(THEMES['default-dark'].colors);
    expect(def.base).toBe('default-dark');
  });

  it('takes its own lanes only when all ten are valid', () => {
    const ten = THEMES.light.graph.map((c) => c.toUpperCase());
    expect(customThemeDef(ct({ graph: ten })).graph).toEqual(THEMES.light.graph);
    expect(customThemeDef(ct({ graph: ten.slice(0, 9) })).graph).toEqual(THEMES.nord.graph);
    expect(customThemeDef(ct({ graph: [...ten.slice(0, 9), 'red'] })).graph).toEqual(THEMES.nord.graph);
  });
});

describe('themeDef', () => {
  it('a built-in by id, a custom by id, else Default Dark', () => {
    expect(themeDef('nord', [])).toBe(THEMES.nord);
    expect(themeDef('custom-0a1b2c3d', [ct({ id: 'custom-0a1b2c3d', name: 'Mine' })]).label).toBe('Mine');
    expect(themeDef('custom-ffffffff', [])).toBe(THEMES['default-dark']);
    expect(themeDef('no-such-theme', [])).toBe(THEMES['default-dark']);
  });
});

describe('ids, names and colours', () => {
  it('makes and recognises custom ids', () => {
    const id = newCustomThemeId();
    expect(isCustomThemeId(id)).toBe(true);
    expect(isCustomThemeId('nord')).toBe(false);
    expect(isCustomThemeId('custom-0A1B2C3D')).toBe(false);
  });

  it('accepts only plain colours in range', () => {
    for (const ok of ['#abc', '#4D88FF', '#a3be8c55', 'rgb(1, 2, 3)', 'rgba(92, 184, 92, 0.12)']) expect(isTokenColour(ok), ok).toBe(true);
    for (const bad of ['red', 'var(--x)', 'rgba(1,2,3)', 'rgb(1, 2, 3, 0.5)', 'rgb(300, 0, 0)', '#12345', 'rgba(1, 2, 3, 2)', '#fff; background: url(x)', '', 7])
      expect(isTokenColour(bad), String(bad)).toBe(false);
  });
});

describe('the colour lists', () => {
  it('labels every token and groups each exactly once', () => {
    for (const t of COLOR_TOKENS) expect(TOKEN_LABELS[t], t).toBeTruthy();
    expect(TOKEN_GROUPS.flatMap((g) => g.tokens).sort()).toEqual([...COLOR_TOKENS].sort());
  });

  it('shows the twelve core colours under four groups', () => {
    expect(CORE_COLOURS).toHaveLength(12);
    expect([...new Set(CORE_COLOURS.map((c) => c.group))]).toEqual(['Surfaces', 'Text', 'Diff', 'Status']);
  });
});
