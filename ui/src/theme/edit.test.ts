import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../api/client', () => ({ api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null) } }));

const { DEFAULT_SETTINGS, useAppState } = await import('../app/state');
const { deleteTheme, ensureEditable, moveLane, renameTheme, setLaneColour, setThemeColour } = await import('./edit');
const { THEMES } = await import('./themes');

const settings = () => useAppState.getState().settings;
const current = () => settings().customThemes.find((t) => t.id === settings().theme)!;

describe('editing themes', () => {
  beforeEach(() => useAppState.setState({ settings: { ...DEFAULT_SETTINGS, theme: 'nord', customThemes: [] } }));

  it('an edit of a built-in makes "<theme> (Custom)", selected, and leaves the built-in alone', () => {
    const nordBg = THEMES.nord.colors['app-bg0'];
    setThemeColour('app-bg0', '#202020');
    expect(settings().customThemes).toHaveLength(1);
    expect(current()).toMatchObject({ name: 'Nord (Custom)', base: 'nord', colors: { 'app-bg0': '#202020' }, graph: [] });
    expect(current().id).toMatch(/^custom-[0-9a-f]{8}$/);
    expect(THEMES.nord.colors['app-bg0']).toBe(nordBg);
  });

  it('a second edit edits the same custom theme', () => {
    setThemeColour('app-bg0', '#202020');
    const id = settings().theme;
    setThemeColour('blue', '#0000ff');
    expect(settings().customThemes).toHaveLength(1);
    expect(settings().theme).toBe(id);
    expect(current().colors).toEqual({ 'app-bg0': '#202020', blue: '#0000ff' });
    expect(ensureEditable()).toBe(id);
  });

  it('another copy of the same built-in takes a free name', () => {
    setThemeColour('app-bg0', '#202020');
    useAppState.getState().setSettings({ theme: 'nord' });
    setThemeColour('app-bg0', '#303030');
    expect(settings().customThemes.map((t) => t.name)).toEqual(['Nord (Custom)', 'Nord (Custom) (2)']);
    expect(current().name).toBe('Nord (Custom) (2)');
  });

  it('sets and reorders lanes, writing all ten', () => {
    setLaneColour(1, '#ABCDEF');
    const lanes = THEMES.nord.graph.map((c, i) => (i === 1 ? '#abcdef' : c));
    expect(current().graph).toEqual(lanes);
    moveLane(0, 2);
    expect(current().graph).toEqual([lanes[1], lanes[2], lanes[0], ...lanes.slice(3)]);
    moveLane(0, 10);
    expect(current().graph).toEqual([lanes[1], lanes[2], lanes[0], ...lanes.slice(3)]);
  });

  it('renames: trimmed; empty or unchanged does nothing; a clash takes a suffix', () => {
    setThemeColour('app-bg0', '#202020');
    const id = settings().theme;
    renameTheme(id, '  ');
    expect(current().name).toBe('Nord (Custom)');
    renameTheme(id, '  Night  ');
    expect(current().name).toBe('Night');
    useAppState.getState().setSettings({ customThemes: [...settings().customThemes, { id: 'custom-00000002', name: 'Dusk', base: 'light', colors: {}, graph: [] }] });
    renameTheme(id, 'dusk');
    expect(current().name).toBe('dusk (2)');
  });

  it('deleting the current custom theme selects its base; another one leaves the theme alone', () => {
    setThemeColour('app-bg0', '#202020');
    const id = settings().theme;
    useAppState.getState().setSettings({ customThemes: [...settings().customThemes, { id: 'custom-00000002', name: 'Dusk', base: 'light', colors: {}, graph: [] }] });
    deleteTheme('custom-00000002');
    expect(settings().theme).toBe(id);
    deleteTheme(id);
    expect(settings().theme).toBe('nord');
    expect(settings().customThemes).toEqual([]);
  });
});
