import { describe, expect, it } from 'vitest';
import type { CustomTheme } from '../api/gen/CustomTheme';
import { customThemeDef } from './custom';
import { themeFromJson, themeToJson } from './themeJson';
import { THEMES } from './themes';

const lanes = THEMES.nord.graph.map((c, i) => (i === 0 ? '#123456' : c));
const mine: CustomTheme = { id: 'custom-0a1b2c3d', name: 'Nord (Custom)', base: 'nord', colors: { 'app-bg0': '#202020', blue: 'rgba(1, 2, 3, 0.5)' }, graph: lanes };
const json = (body: Record<string, unknown>) => JSON.stringify({ gitvolt: 'theme', version: 1, name: 'Shared', base: 'nord', ...body });
const fresh = () => 'custom-00000009';

describe('theme JSON', () => {
  it('round-trips a custom theme: a new id, a free name, the same colours and lanes', () => {
    const text = themeToJson(customThemeDef(mine), mine);
    expect(JSON.parse(text)).toEqual({ gitvolt: 'theme', version: 1, name: 'Nord (Custom)', base: 'nord', colors: mine.colors, graph: lanes });
    const back = themeFromJson(text, [mine], fresh)!;
    expect(back).toEqual({ ...mine, id: 'custom-00000009', name: 'Nord (Custom) (2)' });
    expect(customThemeDef(back).colors).toEqual(customThemeDef(mine).colors);
  });

  it('writes a built-in as itself with nothing set', () => {
    expect(JSON.parse(themeToJson(THEMES.light))).toEqual({ gitvolt: 'theme', version: 1, name: 'Light', base: 'light', colors: {}, graph: [] });
    expect(themeFromJson(themeToJson(THEMES.light), [], fresh)).toEqual({ id: 'custom-00000009', name: 'Light', base: 'light', colors: {}, graph: [] });
  });

  it("refuses what isn't a theme", () => {
    expect(themeFromJson('{"gitvolt":"branch-preset","version":1,"name":"x"}', [])).toBeNull();
    expect(themeFromJson(JSON.stringify({ gitvolt: 'theme', version: 2, name: 'x', base: 'nord' }), [])).toBeNull();
    expect(themeFromJson('not json', [])).toBeNull();
    expect(themeFromJson('[1, 2]', [])).toBeNull();
    expect(themeFromJson(json({ name: '   ' }), [])).toBeNull();
    expect(themeFromJson(json({ name: 7 }), [])).toBeNull();
    expect(themeFromJson(json({ pad: 'x'.repeat(70 * 1024) }), [])).toBeNull();
  });

  it('keeps only known tokens with plain colours: nothing else reaches a custom property', () => {
    const t = themeFromJson(json({ colors: { 'app-bg0': 'red; background:url(https://example.com/x)', blue: '#00f', 'panel-bg0': 'var(--x)', 'text-normal': 'rgb(300, 0, 0)', bogus: '#fff' } }), [], fresh)!;
    expect(t.colors).toEqual({ blue: '#00f' });
    expect(themeFromJson(json({ colors: ['#fff'] }), [], fresh)!.colors).toEqual({});
  });

  it('keeps lanes only when all ten are valid; an unknown base is Default Dark; a long name is cut', () => {
    expect(themeFromJson(json({ graph: lanes.slice(0, 9) }), [], fresh)!.graph).toEqual([]);
    expect(themeFromJson(json({ graph: [...lanes.slice(0, 9), 'red'] }), [], fresh)!.graph).toEqual([]);
    expect(themeFromJson(json({ graph: lanes.map((c) => c.toUpperCase()) }), [], fresh)!.graph).toEqual(lanes);
    expect(themeFromJson(json({ base: 'gone' }), [], fresh)!.base).toBe('default-dark');
    expect(themeFromJson(json({ name: `  ${'n'.repeat(100)}  ` }), [], fresh)!.name).toBe('n'.repeat(80));
  });

  it('takes a pasted theme with a ```json fence around it', () => {
    expect(themeFromJson(`\`\`\`json\n${json({})}\n\`\`\``, [], fresh)?.name).toBe('Shared');
  });
});
