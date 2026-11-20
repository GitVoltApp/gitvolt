/**
 * Caches kept per tab or per repository by modules the runtime doesn't import (a blame, the
 * palette's file list, the find and reveal state, the forge pickers' answers): each registers a
 * cleaner here, and the runtime's `drop` runs them, so a closed tab doesn't leave them behind.
 */
const tabCleaners = new Set<(tabId: string) => void>();
const repoCleaners = new Set<(repoId: number) => void>();

/** `clean` runs when a tab closes, with its id. Returns the unregister. */
export function onTabClosed(clean: (tabId: string) => void): () => void {
  tabCleaners.add(clean);
  return () => { tabCleaners.delete(clean); };
}

/** `clean` runs when the last tab showing a repository closes, with the repository's id. */
export function onRepoClosed(clean: (repoId: number) => void): () => void {
  repoCleaners.add(clean);
  return () => { repoCleaners.delete(clean); };
}

/** `tabId` is closing; `tabs`: the open tabs' runtimes, it included (its repository, and whether
 * another tab still shows it). */
export function cleanClosedTab(tabId: string, tabs: Readonly<Record<string, { repo?: { id: number } | null }>>): void {
  for (const clean of tabCleaners) clean(tabId);
  const repo = tabs[tabId]?.repo?.id;
  if (repo === undefined) return;
  for (const [id, t] of Object.entries(tabs)) if (id !== tabId && t.repo?.id === repo) return;
  for (const clean of repoCleaners) clean(repo);
}
