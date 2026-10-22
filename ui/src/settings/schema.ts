import { create } from 'zustand';
import { settingDef } from './registry';

/** The fetch interval the scheduler runs: 0 (or less, or not a number) is off; anything else is
 * between a minute and a day, so a hand-edited settings file can't make it hammer a remote. */
export function clampFetchInterval(secs: number): number {
  if (!(secs > 0)) return 0;
  return Math.min(86_400, Math.max(60, Math.round(secs)));
}

interface SettingsUi {
  open: boolean;
  /** The page shown: kept for the session (the overlay remounts on each open), set by a deep link. */
  page: string;
  query: string;
  /** A setting to focus and flash (deep link). */
  focus: string | null;
  /** A floating panel shown instead of the overlay ('theme-editor'), or null. */
  floating: string | null;
  show(focus?: string): void;
  setPage(id: string): void;
  setQuery(q: string): void;
  setFloating(id: string | null): void;
  close(): void;
}

/** Settings' state: open, on which page, the search query, a deep link. `show(id)` opens it on
 * the page of setting `id` and focuses that setting; the settings themselves are in `registry.ts`. */
export const useSettingsUi = create<SettingsUi>((set) => ({
  open: false,
  page: 'general',
  query: '',
  focus: null,
  floating: null,
  show: (focus) => set((st) => ({ open: true, focus: focus ?? null, query: '', page: (focus && settingDef(focus)?.page) || st.page })),
  setPage: (page) => set({ page, query: '', focus: null }),
  setQuery: (query) => set({ query }),
  setFloating: (floating) => set({ floating }),
  close: () => set({ open: false, focus: null, floating: null }),
}));
