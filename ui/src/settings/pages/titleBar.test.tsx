import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';

vi.mock('../../api/client', () => ({
  api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null), fontFamilies: vi.fn(async () => []), relaunchApp: vi.fn(async () => null) },
  errorMessage: String,
}));
const choose = vi.hoisted(() => vi.fn());
vi.mock('../../ui/ConfirmDialog', async (orig) => ({ ...(await orig<typeof import('../../ui/ConfirmDialog')>()), chooseAction: choose }));

// The app's window on Linux, as its initialization script hands it over, before any page registers.
const LINUX = { frame: 'custom', platform: 'linux', layout: 'appmenu:close', doubleClick: 'toggle-maximize', middleClick: 'lower', rightClick: 'menu' } as const;
window.__GITVOLT_WINDOW__ = { ...LINUX };
await import('../feature');
const { api } = await import('../../api/client');
const { DEFAULT_SETTINGS, useAppState } = await import('../../app/state');
const { useWindowChrome } = await import('../../chrome/windowChrome');
const { allSettings, settingDef } = await import('../registry');
const { useSettingsUi } = await import('../schema');
const { SettingsOverlay } = await import('../frame/SettingsOverlay');
const { titleBarChoice, titleBarSetting } = await import('./appearance');
const { useWorkingCopy } = await import('../../diff/workingCopy');

beforeEach(() => {
  act(() => {
    useAppState.setState({ settings: { ...DEFAULT_SETTINGS } });
    useWindowChrome.setState({ info: { ...LINUX } });
  });
  useWorkingCopy.setState({ copies: {} });
  vi.mocked(api.relaunchApp).mockClear();
  choose.mockReset();
});

const show = () => {
  render(<SettingsOverlay />);
  act(() => useSettingsUi.getState().show('titleBar'));
};
const radio = (name: string) => within(screen.getByRole('radiogroup', { name: 'Title bar' })).getByRole('radio', { name });
const restartButton = () => screen.queryByRole('button', { name: 'Restart GitVolt' });

it('"Title bar" is on Appearance › Window, before Fonts, and lists what each choice is on each platform', () => {
  expect(settingDef('titleBar')).toMatchObject({ page: 'appearance', group: 'Window', label: 'Title bar' });
  const groups = [...new Set(allSettings().filter((d) => d.page === 'appearance').map((d) => d.group))];
  expect(groups).toEqual(['Density', 'Window', 'Fonts']);
  expect(settingDef('titleBar')!.description!.split('\n')).toEqual([
    expect.stringMatching(/^• Separate: /),
    expect.stringMatching(/^• Combined: /),
    expect.stringMatching(/^• System: the desktop's own title bar/),
    'Takes effect when GitVolt restarts.',
  ]);
  const windows = titleBarSetting('windows').description!;
  expect(windows).toMatch(/• System: Windows' own title bar and frame/);
  expect(windows).not.toMatch(/X11/);
  // Found by the old switches' words too.
  expect(settingDef('titleBar')!.keywords).toMatch(/compact.*system.*decorations.*traffic lights/);
  // Shown as a list, then the notes.
  show();
  const row = document.getElementById('setting-titleBar')!;
  expect([...row.querySelectorAll('.setting-desc li')].map((li) => li.textContent)).toEqual([
    expect.stringMatching(/^Separate: /), expect.stringMatching(/^Combined: /), expect.stringMatching(/^System: /),
  ]);
  expect([...row.querySelectorAll('.setting-desc p')].map((p) => p.textContent)).toEqual(['Takes effect when GitVolt restarts.']);
});

it('maps the two stored switches: Separate by default, Combined, System, and both on reads as System', () => {
  expect(titleBarChoice({ systemTitleBar: false, compactTitleBar: false })).toBe('separate');
  expect(titleBarChoice({ systemTitleBar: false, compactTitleBar: true })).toBe('combined');
  expect(titleBarChoice({ systemTitleBar: true, compactTitleBar: false })).toBe('system');
  expect(titleBarChoice({ systemTitleBar: true, compactTitleBar: true })).toBe('system');
  show();
  expect(radio('Separate')).toBeChecked();
  fireEvent.click(radio('Combined'));
  expect(useAppState.getState().settings).toMatchObject({ compactTitleBar: true, systemTitleBar: false });
  fireEvent.click(radio('System'));
  expect(useAppState.getState().settings).toMatchObject({ compactTitleBar: false, systemTitleBar: true });
  expect(radio('System')).toBeChecked();
  fireEvent.click(radio('Separate'));
  expect(useAppState.getState().settings).toMatchObject({ compactTitleBar: false, systemTitleBar: false });
  act(() => useAppState.getState().setSettings({ systemTitleBar: true, compactTitleBar: true }));
  expect(radio('System')).toBeChecked();
});

it('offers Restart GitVolt once the choice isn\'t what the window was built with, and hides it when it is again', async () => {
  show();
  expect(restartButton()).toBeNull();
  fireEvent.click(radio('Combined'));
  expect(restartButton()).toHaveClass('primary');
  expect(screen.getByText('Takes effect after a restart.')).toBeVisible();
  fireEvent.click(radio('Separate'));
  expect(restartButton()).toBeNull();
  fireEvent.click(radio('System'));
  await act(async () => { fireEvent.click(restartButton()!); });
  expect(api.relaunchApp).toHaveBeenCalledTimes(1);
  expect(api.saveSettings).toHaveBeenCalled();
  // The running window is combined: Separate needs a restart too, and Combined doesn't.
  act(() => useWindowChrome.setState({ info: { ...LINUX, compact: true } }));
  expect(restartButton()).not.toBeNull();
  fireEvent.click(radio('Combined'));
  expect(restartButton()).toBeNull();
});

it('Restart GitVolt with unsaved edits asks first: Cancel stays, Discard edits restarts', async () => {
  useWorkingCopy.setState({ copies: { t: { key: 'k', path: 'a.txt', worktree: '/r', repo: 1, base: 'h0', dirty: true, view: 'diff' } } });
  show();
  fireEvent.click(radio('Combined'));
  choose.mockResolvedValueOnce(null);
  await act(async () => { fireEvent.click(restartButton()!); });
  expect(choose).toHaveBeenCalledWith(expect.objectContaining({ title: 'Discard unsaved changes?' }));
  expect(api.relaunchApp).not.toHaveBeenCalled();
  choose.mockResolvedValueOnce('discard');
  await act(async () => { fireEvent.click(restartButton()!); });
  expect(choose).toHaveBeenCalledTimes(2);
  expect(api.relaunchApp).toHaveBeenCalledTimes(1);
});

it('in an X11 session it\'s System, fixed, with nothing to reset, and says why', () => {
  act(() => {
    useWindowChrome.setState({ info: { ...LINUX, frame: 'system', systemOnly: true } });
    useAppState.getState().setSettings({ compactTitleBar: true });
  });
  show();
  expect(radio('System')).toBeChecked();
  for (const name of ['Separate', 'Combined', 'System']) expect(radio(name)).toBeDisabled();
  expect(screen.getByText('An X11 session always has the desktop\'s title bar.')).toBeVisible();
  expect(restartButton()).toBeNull();
  expect(settingDef('titleBar')!.binding!({ repoPath: null, repoName: null, tabId: null })).toBeNull();
});
