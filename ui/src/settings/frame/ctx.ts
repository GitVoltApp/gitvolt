import { useMemo } from 'react';
import { useRuntime } from '../../app/runtime';
import { useAppState } from '../../app/state';
import type { SettingsCtx } from '../registry';

/** What Settings is about: the shown tab, and its repository (the Repository pages', and the
 * Hosts list's remotes). Stable while those don't change. */
export function useSettingsCtx(): SettingsCtx {
  const tabId = useAppState((s) => s.profile.activeTab);
  const repoPath = useRuntime((s) => (tabId ? s.tabs[tabId]?.repo?.path : undefined)) ?? null;
  const repoName = useRuntime((s) => (tabId ? s.tabs[tabId]?.repo?.name : undefined)) ?? null;
  return useMemo(() => ({ repoPath, repoName, tabId }), [repoPath, repoName, tabId]);
}
