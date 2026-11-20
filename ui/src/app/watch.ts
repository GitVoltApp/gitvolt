import { api, errorMessage } from '../api/client';

const desired = new Map<number, boolean>();
const running = new Map<number, Promise<void>>();
/** The tab that last asked to watch each repo: it holds the repo before each watch. */
const holders = new Map<number, string>();

/**
 * Tab `tabId` holds `repo` in the core, which drops a handle no tab holds (and won't watch it).
 * Idempotent; retried once, then logged: a lost hold leaves the tab unwatched until it's shown
 * again, when its watch holds it again.
 */
export async function holdRepo(tabId: string, repo: number): Promise<void> {
  try {
    await api.holdRepo(tabId, repo);
  } catch {
    await api.holdRepo(tabId, repo).catch((e: unknown) => console.warn('[gitvolt] holdRepo failed', errorMessage(e)));
  }
}

/**
 * The backend outlives a webview reload: forget what the previous page watched and held, before
 * this one watches its active tab and its tabs hold their repositories again.
 */
export async function forgetPreviousPage(): Promise<void> {
  await api.unwatchAll().catch((e: unknown) => console.warn('[gitvolt] unwatchAll failed', errorMessage(e)));
  await api.releaseAll().catch((e: unknown) => console.warn('[gitvolt] releaseAll failed', errorMessage(e)));
}

/**
 * Watch or unwatch a repo (only the active tab is watched). Calls for one repo are
 * applied one at a time and coalesce to the latest wish, so an effect's quick
 * mount → unmount → mount (StrictMode, fast tab switching) can't end with the repo unwatched.
 * Resolves once the repo's latest wish has been applied (a failure is logged, not thrown): a tab
 * refreshes after its `watchRepo` returns, since the watcher's first pass has then refreshed the
 * backend's status cache (watch, then a plain `graph`). A watch for tab `holder` holds the
 * repo first: the core watches only a repo some tab holds.
 */
export function setWatched(repo: number, on: boolean, holder?: string): Promise<void> {
  desired.set(repo, on);
  if (on && holder) holders.set(repo, holder);
  const cur = running.get(repo);
  if (cur) return cur;
  const run = (async () => {
    let applied: boolean | undefined;
    while (desired.get(repo) !== applied) {
      const want = desired.get(repo)!;
      const holder = want ? holders.get(repo) : undefined;
      if (holder) await holdRepo(holder, repo);
      try {
        await (want ? api.watchRepo(repo) : api.unwatchRepo(repo));
      } catch (e) {
        console.warn('[gitvolt] watch update failed', e);
      }
      applied = want;
    }
    running.delete(repo);
  })();
  running.set(repo, run);
  return run;
}
