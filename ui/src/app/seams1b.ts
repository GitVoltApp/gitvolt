/**
 * The app's dependency surface on the repo view (details panel, diff takeover, focus zones):
 * app features import the repo view from here, and tests `vi.mock` this file. Every seam acts on a tab's own
 * `RepoViewStore` and `RepoServices` (`tabStores.ts`), in `fileMenuEnv(store)`
 * style. The contracts:
 * - opening a file takes a `DiffTarget` (its two `BlobSource`s), not a (sha, path) pair;
 * - commit messages are the per-repo `CommitMessageCache` (details carry no message);
 * - the sidebar's focus zone is the `useFocusZone('sidebar', ref)` hook, under the tab's
 *   `RepoViewContext` (RepoTab provides it);
 * - the diff editor's context menu is `MonacoHost.setContextMenuHandler` (`diff/monaco/host.ts`),
 *   installed by the menus while its tab is active; there's no seam for it here.
 */
import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';
import { fileViewTarget, openWorktree, type DiffTarget, type RepoViewState, type RepoViewStore } from '../repo/store';
import { tabView, useTabView } from './tabStores';

export { Avatar as AuthorAvatar } from '../avatars/Avatar';
export { useFocusZone } from '../repo/focus';
export { RepoView } from '../repo/RepoView';
export { fileViewTarget, RepoViewContext, targetFor, useRepoView, useRepoViewStore, type DiffTarget, type FocusZone, type RepoViewStore } from '../repo/store';

/** Stands in for a tab with no view yet (no graph loaded). */
const NO_VIEW = createStore(() => ({ diff: null })) as unknown as RepoViewStore;

/** True while the tab's center panel shows a file (the diff takeover). */
export function useDiffOpen(tabId: string): boolean {
  return useStore(useTabView(tabId)?.store ?? NO_VIEW, (s: RepoViewState) => s.diff !== null);
}

const storeOf = (tabId: string) => tabView(tabId)?.store;

/** Opens `target` in the tab's center panel (Diff View, or File View per `target.view`). */
export function openDiff(tabId: string, target: DiffTarget): boolean {
  const store = storeOf(tabId);
  store?.getState().openFile(target);
  return !!store;
}

/** File View of `path` as of commit `sha` ("View all files"). */
export function openFileView(tabId: string, sha: string, path: string): boolean {
  return openDiff(tabId, fileViewTarget(path, sha, { kind: 'commit', id: sha, parent: 0 }));
}

/**
 * Compare mode: `from` → `to` (two commits, FROM then TO), or a commit with a
 * working tree (`'worktree'`: the tab's own). As a click then a Ctrl+click would. False when a
 * commit isn't in the loaded graph.
 */
export function startCompare(tabId: string, from: string, to: string | 'worktree' | { worktree: string }): boolean {
  const store = storeOf(tabId);
  if (!store) return false;
  const s = store.getState();
  const i = s.indexById.get(from);
  if (i === undefined) return false;
  if (to === 'worktree' || typeof to === 'object') {
    s.compareWithWorktree(from, typeof to === 'object' ? to.worktree : openWorktree(s));
    return true;
  }
  const j = s.indexById.get(to);
  if (j === undefined) return false;
  s.selectRow(i);
  store.getState().selectRow(j, { ctrl: true });
  return true;
}

function servicesOf(tabId: string) {
  const v = tabView(tabId);
  if (!v) throw new Error(`no repository view for tab ${tabId}`);
  return v.services;
}

/** The commit's full message: summary, a blank line, then the body. */
export async function fetchCommitMessage(tabId: string, sha: string): Promise<string> {
  const m = await servicesOf(tabId).messages.get(sha);
  return m.body ? `${m.summary}\n\n${m.body}` : m.summary;
}

/** Every path at a commit ("View all files"; the palette). */
export async function listTreeFiles(tabId: string, sha: string): Promise<string[]> {
  return servicesOf(tabId).treeFiles.get(sha);
}
