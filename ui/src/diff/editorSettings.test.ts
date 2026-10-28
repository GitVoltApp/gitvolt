// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS, useAppState } from '../app/state';
import { EDITOR_SETTINGS_STORAGE_KEY, installEditorSettingsMigration, migrateStickyScroll, takeLegacyStickyScroll } from './editorSettings';

vi.mock('../api/client', () => ({ api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null) }, errorMessage: String }));

const KEY = 'gitvolt.editorSettings.v1';

beforeEach(() => {
  localStorage.clear();
  useAppState.setState({ loaded: false, settings: { ...DEFAULT_SETTINGS } });
});
afterEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
});

describe('sticky scroll moves from localStorage into the app settings, once', () => {
  it('reads the old key once and removes it, whatever it held', () => {
    expect(EDITOR_SETTINGS_STORAGE_KEY).toBe(KEY);
    localStorage.setItem(KEY, '{"stickyScroll":true}');
    expect(takeLegacyStickyScroll()).toBe(true);
    expect(localStorage.getItem(KEY)).toBeNull();
    expect(takeLegacyStickyScroll()).toBeNull();
    for (const raw of ['{not json', 'null', '"x"', '[]', '{"stickyScroll":"yes"}']) {
      localStorage.setItem(KEY, raw);
      expect(takeLegacyStickyScroll(), raw).toBeNull();
      expect(localStorage.getItem(KEY), raw).toBeNull();
    }
  });

  it('a change only when the old value differs from the setting', () => {
    localStorage.setItem(KEY, '{"stickyScroll":true}');
    expect(migrateStickyScroll(DEFAULT_SETTINGS)).toEqual({ editorStickyScroll: true });
    localStorage.setItem(KEY, '{"stickyScroll":false}');
    expect(migrateStickyScroll(DEFAULT_SETTINGS)).toBeNull();
    localStorage.setItem(KEY, '{"stickyScroll":true}');
    expect(migrateStickyScroll({ ...DEFAULT_SETTINGS, editorStickyScroll: true })).toBeNull();
    expect(migrateStickyScroll(DEFAULT_SETTINGS)).toBeNull();
  });

  it('blocked storage migrates nothing and throws nothing', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('blocked'); });
    expect(takeLegacyStickyScroll()).toBeNull();
  });

  it('waits for the loaded settings, then moves the value in and stops listening', () => {
    localStorage.setItem(KEY, '{"stickyScroll":true}');
    const off = installEditorSettingsMigration();
    // The store's placeholder settings: the load would overwrite a change made now.
    expect(useAppState.getState().settings.editorStickyScroll).toBe(false);
    expect(localStorage.getItem(KEY)).not.toBeNull();
    useAppState.setState({ loaded: true, settings: { ...DEFAULT_SETTINGS, editorFontSize: 15 } });
    expect(useAppState.getState().settings).toMatchObject({ editorStickyScroll: true, editorFontSize: 15 });
    expect(localStorage.getItem(KEY)).toBeNull();
    // Turned off later, it stays off: the migration is done.
    localStorage.setItem(KEY, '{"stickyScroll":true}');
    useAppState.getState().setSettings({ editorStickyScroll: false });
    expect(useAppState.getState().settings.editorStickyScroll).toBe(false);
    off();
  });
});
