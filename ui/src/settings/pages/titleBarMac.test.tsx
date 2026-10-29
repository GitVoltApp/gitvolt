import { expect, it, vi } from 'vitest';

vi.mock('../../api/client', () => ({
  api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null), fontFamilies: vi.fn(async () => []) },
  errorMessage: String,
}));

// The app's window on macOS, as its initialization script hands it over, before any page registers.
window.__GITVOLT_WINDOW__ = { frame: 'custom', platform: 'macos', layout: '', doubleClick: 'toggle-maximize', middleClick: 'none', rightClick: 'none' };
await import('../feature');
const { settingDef } = await import('../registry');

it('"Use the system title bar" is on Appearance › Window on macOS too, in macOS\'s terms', () => {
  const def = settingDef('systemTitleBar')!;
  expect(def).toMatchObject({ page: 'appearance', group: 'Window' });
  expect(def.description).toMatch(/^macOS's standard title bar .*restarts\.$/);
  expect(def.description).not.toMatch(/X11/);
});
