import { describe, expect, it } from 'vitest';
import { customThemeDef } from './custom';
import { resolveColors } from './apply';
import { migrateLaneOverrides } from './migrate';
import { THEMES } from './themes';

const ids = (...list: string[]) => {
  let i = 0;
  return () => list[i++]!;
};

describe('migrateLaneOverrides', () => {
  it('turns per-theme lane overrides into custom themes and switches to the current one', () => {
    const out = migrateLaneOverrides({ theme: 'nord', customThemes: [], graphColorOverrides: { nord: ['#123456', null], light: [null, '#abcdef'] } }, ids('custom-00000001', 'custom-00000002'))!;
    expect(out.customThemes!.map((t) => [t.name, t.base])).toEqual([['Light (Custom)', 'light'], ['Nord (Custom)', 'nord']]);
    expect(out.customThemes!.find((t) => t.base === 'nord')!.graph[0]).toBe('#123456');
    expect(out.theme).toBe(out.customThemes!.find((t) => t.base === 'nord')!.id);
    expect(out.graphColorOverrides).toEqual({});
  });

  it('creates nothing for empty, invalid or unknown overrides, but clears them', () => {
    expect(migrateLaneOverrides({ theme: 'nord', customThemes: [], graphColorOverrides: { nord: [null, null], gone: ['#123456'], light: ['red'] } }))
      .toEqual({ customThemes: [], graphColorOverrides: {} });
  });

  it('does nothing when there are no overrides (so a restart never duplicates)', () => {
    expect(migrateLaneOverrides({ theme: 'nord', customThemes: [], graphColorOverrides: {} })).toBeNull();
  });

  it('shows exactly the colours the overrides showed: valid ones (any case) on the theme lanes, the rest and anything past lane 10 ignored', () => {
    const overrides = ['#ABCDEF', 'nope', null, '#000000', ...Array(6).fill(null), '#ffffff'];
    const out = migrateLaneOverrides({ theme: 'darcula', customThemes: [], graphColorOverrides: { darcula: overrides } }, ids('custom-00000001'))!;
    const def = customThemeDef(out.customThemes![0]!);
    // What 0.7.8's resolveColors drew for these overrides.
    const before = THEMES.darcula.graph.map((c, i) => (i === 0 ? '#abcdef' : i === 3 ? '#000000' : c));
    expect(resolveColors(def)).toEqual(resolveColors({ ...THEMES.darcula, graph: before }));
    expect(def.colors).toEqual(THEMES.darcula.colors);
    expect(out.theme).toBe('custom-00000001');
  });

  it('keeps existing custom themes and avoids their names; leaves the theme when it had no overrides', () => {
    const mine = { id: 'custom-0000000a', name: 'Nord (Custom)', base: 'nord', colors: {}, graph: [] };
    const out = migrateLaneOverrides({ theme: 'light', customThemes: [mine], graphColorOverrides: { nord: ['#123456'] } }, ids('custom-00000001'))!;
    expect(out.customThemes!.map((t) => t.name)).toEqual(['Nord (Custom)', 'Nord (Custom) (2)']);
    expect(out).not.toHaveProperty('theme');
  });
});
