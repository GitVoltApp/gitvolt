import { revealCommit } from '../presets/reveal';
import { tabStore } from './tabStores';

/**
 * Selects `sha` in the tab's graph, as a click would (the details panel follows, an open diff
 * closes), and the graph scrolls it into view. `focus` also moves the keyboard to the graph (the
 * sidebar's Enter, spec §11.1). False if the commit isn't in the loaded rows (or the tab has no
 * graph yet): the caller may load a deeper window first.
 */
export function selectCommit(tabId: string, sha: string, opts: { focus?: boolean } = {}): boolean {
  const s = tabStore(tabId)?.getState();
  if (!s?.selectCommitById(sha)) return false;
  if (opts.focus) s.setFocus('graph');
  return true;
}

/**
 * Goes to commit `sha` in the tab's graph (a parent link, "Show in graph", a sidebar row): as
 * `selectCommit`, and when it isn't a row because the branch preset hides it, what hides it is
 * expanded and it's selected once the graph has reloaded (presets/reveal.ts). `missing` runs
 * when it isn't in the loaded window at all.
 */
export function goToCommit(tabId: string, sha: string, opts: { focus?: boolean }, missing: () => void): void {
  if (selectCommit(tabId, sha, opts)) return;
  void revealCommit(tabId, sha).then((ok) => {
    if (!ok) missing();
    else if (opts.focus) tabStore(tabId)?.getState().setFocus('graph');
  });
}
