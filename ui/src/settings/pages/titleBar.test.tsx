import { act, fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';

vi.mock('../../api/client', () => ({
  api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null), fontFamilies: vi.fn(async () => []) },
  errorMessage: String,
}));

// The app's window on Linux, as its initialization script hands it over, before any page registers.
window.__GITVOLT_WINDOW__ = { frame: 'custom', platform: 'linux', layout: 'appmenu:close', doubleClick: 'toggle-maximize', middleClick: 'lower', rightClick: 'menu' };
await import('../feature');
const { DEFAULT_SETTINGS, useAppState } = await import('../../app/state');
const { settingDef } = await import('../registry');
const { useSettingsUi } = await import('../schema');
const { SettingsOverlay } = await import('../frame/SettingsOverlay');
const { systemTitleBarSetting } = await import('./appearance');

it('says what it brings back on each platform: X11 only matters on Linux', () => {
  expect(settingDef('systemTitleBar')!.description).toMatch(/desktop's own title bar.*X11/);
  const windows = systemTitleBarSetting('windows');
  expect(windows.description).toMatch(/^The system's own title bar and window frame/);
  expect(windows.description).not.toMatch(/X11/);
});

it('"Use the system title bar" is on Appearance › Window where GitVolt draws its own, off by default, and says it needs a restart', () => {
  expect(settingDef('systemTitleBar')).toMatchObject({ page: 'appearance', group: 'Window' });
  expect(settingDef('systemTitleBar')!.description).toMatch(/restarts/);
  act(() => useAppState.setState({ settings: { ...DEFAULT_SETTINGS } }));
  expect(DEFAULT_SETTINGS.systemTitleBar).toBe(false);
  render(<SettingsOverlay />);
  act(() => useSettingsUi.getState().show('systemTitleBar'));
  const toggle = screen.getByRole('switch', { name: 'Use the system title bar' });
  expect(toggle).not.toBeChecked();
  fireEvent.click(toggle);
  expect(useAppState.getState().settings.systemTitleBar).toBe(true);
});
