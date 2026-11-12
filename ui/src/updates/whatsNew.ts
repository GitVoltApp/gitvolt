import { create } from 'zustand';
import { useAppInfo } from '../app/appInfo';
import { useAppState } from '../app/state';
import { compareVersions } from './changelog';

/** The notes What's new shows: `since` (exclusive) up to `version`; `since` null: `version`'s alone. */
export interface WhatsNewRange { version: string; since: string | null }

interface WhatsNewStore {
  /** "Updated to GitVolt …" in the notification bell, until cleared: this session's update. */
  notice: WhatsNewRange | null;
  /** The notice hasn't been seen in the bell yet (it counts in the bell's badge). */
  unread: boolean;
  /** The version this session updated from, kept when the notice is cleared (Help → What's new). */
  updatedFrom: string | null;
  /** The What's new panel, open on these notes. */
  open: WhatsNewRange | null;
  show(range: WhatsNewRange): void;
  close(): void;
  markRead(): void;
  clearNotice(): void;
}

export const useWhatsNew = create<WhatsNewStore>((set) => ({
  notice: null,
  unread: false,
  updatedFrom: null,
  open: null,
  show: (open) => set({ open }),
  close: () => set({ open: null }),
  markRead: () => set((s) => (s.unread ? { unread: false } : s)),
  clearNotice: () => set({ notice: null, unread: false }),
}));

/**
 * What a start of `current` makes of the version that last ran: `updated` only for a newer
 * version; a first run (or settings from before the setting existed) and a downgrade only store
 * `current`, and the same version (build metadata aside) does nothing.
 */
export function lastRunChange(last: string | null, current: string): 'updated' | 'store' | 'same' {
  if (!last) return 'store';
  const c = compareVersions(current, last);
  if (last === current || c === 0) return 'same';
  return c !== null && c > 0 ? 'updated' : 'store';
}

let checked = false;

/**
 * Once per session, after the settings loaded: an update since the last run puts an unread
 * "Updated to GitVolt …" in the bell, and the running version becomes the last-run one.
 */
export async function checkLastRun(): Promise<void> {
  if (checked) return;
  checked = true;
  await useAppInfo.getState().load();
  const current = useAppInfo.getState().info?.appVersion;
  if (!current) return;
  const app = useAppState.getState();
  const last = app.settings.lastRunVersion;
  const change = lastRunChange(last, current);
  if (change === 'same') return;
  if (change === 'updated' && last) useWhatsNew.setState({ notice: { version: current, since: last }, unread: true, updatedFrom: last });
  app.setSettings({ lastRunVersion: current });
}

/** Tests only. */
export function resetWhatsNewForTest(): void {
  checked = false;
  useWhatsNew.setState({ notice: null, unread: false, updatedFrom: null, open: null });
}
