import { api } from '../api/client';
import type { PinSetting } from '../api/gen/PinSetting';
import type { ViewSpec } from '../api/gen/ViewSpec';
import { useRuntime } from '../app/runtime';
import { useAppState } from '../app/state';
import { tabStore } from '../app/tabStores';
import { expand, viewSpecFor } from './store';

/** What a read about the tab's graph under its preset sends, as its `graph` request does (the
 * window, the pin, the active worktree, the view); null under All or without a repository. */
export function viewRequest(tabId: string): { repo: number; limit: number; extra: { pin?: PinSetting; active?: string }; view: ViewSpec } | null {
  const rt = useRuntime.getState().tabs[tabId];
  const view = viewSpecFor(tabId);
  if (!rt?.repo || !view) return null;
  const { settings, profile } = useAppState.getState();
  const pin = profile.repos[rt.repo.path]?.pin ?? undefined;
  return { repo: rt.repo.id, limit: rt.limit ?? settings.commitLimit, extra: { ...(pin ? { pin } : {}), ...(rt.worktree ? { active: rt.worktree } : {}) }, view };
}

/**
 * Selects commit `sha` in the tab's graph, also when the branch preset hides it (Review Focus 1):
 * the collapsed run, folded merge or hidden branch that holds it is expanded for the session
 * (until the tab switches presets), and once the graph has reloaded, its row is selected. False
 * when it isn't in the loaded window at all: the caller's own "not loaded" path runs (it may
 * load a deeper window and call again).
 */
export async function revealCommit(tabId: string, sha: string): Promise<boolean> {
  if (tabStore(tabId)?.getState().selectCommitById(sha)) return true;
  const req = viewRequest(tabId);
  if (!req) return false;
  const key = await api.graphHiddenOwner(req.repo, req.limit, req.extra, req.view, sha).catch(() => null);
  if (!key) return false;
  await expand(tabId, key);
  return tabStore(tabId)?.getState().selectCommitById(sha) ?? false;
}
