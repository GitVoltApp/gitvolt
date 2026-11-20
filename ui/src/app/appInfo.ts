import { create } from 'zustand';
import { api } from '../api/client';
import type { AppInfoPayload } from '../api/gen/AppInfoPayload';
import { tildePath } from './osPath';

/** One `appInfo` request at a time: the components that load it at startup share it. */
let loading: Promise<void> | null = null;

/** The About dialog's version info (`api.appInfo`), loaded once on first use (again after a
 * failure: the git check's retry). */
export const useAppInfo = create<{ info: AppInfoPayload | null; load(): Promise<void> }>((set, get) => ({
  info: null,
  load() {
    if (get().info) return Promise.resolve();
    loading ??= (async () => set({ info: await api.appInfo() }))().finally(() => { loading = null; });
    return loading;
  },
}));

/** A function writing a path with the home folder as `~`, for display (titles keep the full path). */
export function useTilde(): (path: string) => string {
  const home = useAppInfo((s) => s.info?.home);
  return (path) => tildePath(path, home);
}
