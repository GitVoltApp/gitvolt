import { useRuntime } from '../app/runtime';
import type { WriteCtx } from './client';

/** What a write from tab `tabId` targets: its repo, and `worktree` or the tab's
 * active worktree (`rt.repo.path` is the repository's main worktree, shared by its tabs). */
export function writeCtx(tabId: string, worktree?: string): WriteCtx | null {
  const rt = useRuntime.getState().tabs[tabId];
  return rt?.repo ? { tabId, repoId: rt.repo.id, worktree: worktree ?? rt.worktree ?? rt.repo.path } : null;
}
