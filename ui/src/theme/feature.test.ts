import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../api/client', () => ({ api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null) } }));

await import('./feature');
await import('../settings/pages');
const { availableActions, hamburgerRows, runAction } = await import('../app/actions');
const { DEFAULT_SETTINGS, useAppState } = await import('../app/state');
const { useSettingsUi } = await import('../settings/schema');
const { THEME_IDS, THEMES } = await import('./themes');

describe('theme actions (R4)', () => {
  beforeEach(() => {
    useAppState.setState({ settings: DEFAULT_SETTINGS });
    useSettingsUi.setState({ open: false, focus: null, page: 'general' });
  });

  it('one palette action per theme, "Theme: <Name>", which saves that theme', () => {
    const ids = availableActions().filter((a) => a.id.startsWith('view.theme.'));
    expect(ids.map((a) => a.label)).toEqual(THEME_IDS.map((id) => `Theme: ${THEMES[id].label}`));
    expect(runAction('view.theme.solarized-dark')).toBe(true);
    expect(useAppState.getState().settings.theme).toBe('solarized-dark');
  });

  it('a custom theme gets its "Theme: <name>" while it exists', () => {
    const mine = { id: 'custom-0a1b2c3d', name: 'Nord (Custom)', base: 'nord', colors: {}, graph: [] };
    useAppState.getState().setSettings({ customThemes: [mine] });
    expect(availableActions().find((a) => a.id === `view.theme.${mine.id}`)?.label).toBe('Theme: Nord (Custom)');
    expect(runAction(`view.theme.${mine.id}`)).toBe(true);
    expect(useAppState.getState().settings.theme).toBe(mine.id);
    useAppState.getState().setSettings({ customThemes: [{ ...mine, name: 'Night' }] });
    expect(availableActions().find((a) => a.id === `view.theme.${mine.id}`)?.label).toBe('Theme: Night');
    // A hand-edited file's duplicate id registers once, without throwing.
    useAppState.getState().setSettings({ customThemes: [mine, mine] });
    expect(availableActions().filter((a) => a.id === `view.theme.${mine.id}`)).toHaveLength(1);
    useAppState.getState().setSettings({ customThemes: [] });
    expect(availableActions().some((a) => a.id === `view.theme.${mine.id}`)).toBe(false);
  });

  it('the menu shows only "Theme…" under View, which opens Settings on Theme', () => {
    const view = hamburgerRows().find((r) => r.kind === 'submenu' && r.label === 'View');
    const ids = view?.kind === 'submenu' ? view.rows.map((r) => (r.kind === 'action' ? r.id : '-')) : [];
    expect(ids).toEqual(['view.theme']);
    expect(runAction('view.theme')).toBe(true);
    expect(useSettingsUi.getState()).toMatchObject({ open: true, page: 'theme', focus: 'theme' });
  });
});
