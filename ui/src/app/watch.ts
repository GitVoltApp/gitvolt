import { api, errorMessage } from '../api/client';

const desired = new Map<number, boolean>();
const running = new Map<number, Promise<void>>();
/** The tab that last asked to watch each repo: it holds the repo before each watch. */
const holders = new Map<number, string>();

/** Each tab's hold, sent or in flight: a tab's open and then its watch send one `holdRepo`. */
const holds = new Map<string, { repo: number; done: Promise<void> }>();

/**
 * Tab `tabId` holds `repo` in the core, which drops a handle no tab holds (and won't watch it).
 * Sent once per tab and repository (until `releaseRepo`): a second call waits on the first.
 * Retried once, then logged: a lost hold is forgotten, so the tab's watch holds it again the next
 * time it's shown.
 */
export function holdRepo(tabId: string, repo: number): Promise<void> {
  const held = holds.get(tabId);
  if (held?.repo === repo) return held.done;
  const done = (async () => {
    try {
      await api.holdRepo(tabId, repo);
    } catch {
      await api.holdRepo(tabId, repo).catch((e: unknown) => {
        console.warn('[gitvolt] holdRepo failed', errorMessage(e));
        if (holds.get(tabId)?.done === done) holds.delete(tabId);
      });
    }
  })();
  holds.set(tabId, { repo, done });
  return done;
}

/** Tab `tabId` closed: the core lets go of its repository unless another tab holds it. A lost
 * release only keeps the handle. */
export function releaseRepo(tabId: string): void {
  holds.delete(tabId);
  api.releaseRepo(tabId).catch(() => {});
}

/**
 * The backend outlives a webview reload: forget what the previous page watched and held, before
 * this one watches its active tab and its tabs hold their repositories again.
 */
export async function forgetPreviousPage(): Promise<void> {
  holds.clear();
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
 * repo first: the core watches only a repo some tab holds. `held`: the caller has just held it,
 * so `watchRepo` goes out at once (ahead of a request sent right after it).
 */
export function setWatched(repo: number, on: boolean, holder?: string, held = false): Promise<void> {
  desired.set(repo, on);
  if (on && holder) holders.set(repo, holder);
  const cur = running.get(repo);
  if (cur) return cur;
  const run = (async () => {
    let applied: boolean | undefined;
    let skipHold = held;
    while (desired.get(repo) !== applied) {
      const want = desired.get(repo)!;
      const holder = want && !skipHold ? holders.get(repo) : undefined;
      skipHold = false;
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
