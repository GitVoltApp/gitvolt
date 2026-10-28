import type { AppSettings } from '../api/gen/AppSettings';
import { useAppState } from '../app/state';

/**
 * Sticky scroll (H7) was the one editor setting kept in localStorage, under this key; it's an app
 * setting now (`editorStickyScroll`, options.ts). The first loaded settings take the old value
 * over, once, and the key goes.
 */
export const EDITOR_SETTINGS_STORAGE_KEY = 'gitvolt.editorSettings.v1';

/** The old key's sticky scroll, if it holds a valid one; the key is removed either way. Blocked
 * storage reads as nothing. */
export function takeLegacyStickyScroll(storage: Pick<Storage, 'getItem' | 'removeItem'> | undefined = globalThis.localStorage): boolean | null {
  try {
    const raw = storage?.getItem(EDITOR_SETTINGS_STORAGE_KEY);
    if (raw == null) return null;
    storage!.removeItem(EDITOR_SETTINGS_STORAGE_KEY);
    const v = (JSON.parse(raw) as { stickyScroll?: unknown } | null)?.stickyScroll;
    return typeof v === 'boolean' ? v : null;
  } catch {
    return null;
  }
}

/** The settings change that moves the old value in, or null when there's nothing to change. */
export function migrateStickyScroll(settings: AppSettings, storage?: Pick<Storage, 'getItem' | 'removeItem'>): Partial<AppSettings> | null {
  const old = takeLegacyStickyScroll(storage);
  return old === null || old === settings.editorStickyScroll ? null : { editorStickyScroll: old };
}

/** Runs the migration on the first loaded settings (the store's placeholder defaults before that
 * would be overwritten by the load). Call once, before the first render. */
export function installEditorSettingsMigration(): () => void {
  let off = () => {};
  const run = ({ loaded, settings }: { loaded: boolean; settings: AppSettings }) => {
    if (!loaded) return false;
    off();
    const patch = migrateStickyScroll(settings);
    if (patch) useAppState.getState().setSettings(patch);
    return true;
  };
  if (run(useAppState.getState())) return off;
  off = useAppState.subscribe((s) => { run(s); });
  return () => off();
}
