import { useCallback, useMemo, useRef } from 'react';
import { create } from 'zustand';

/** Folders opened (true) or closed (false) by hand, per repository (`RepoViewState.repoPath`); in memory only. */
export const useFolderMemory = create<{ byRepo: Record<string, Record<string, boolean>>; remember(repo: string, changes: Record<string, boolean>): void }>((set) => ({
  byRepo: {},
  remember: (repo, changes) => set((s) => ({ byRepo: { ...s.byRepo, [repo]: { ...s.byRepo[repo], ...changes } } })),
}));

export const NEVER_COLLAPSED = (_folder: string): boolean => false;

/** The folders that are closed: a remembered state wins, else the folder's default. */
export function collapsedSet(folders: readonly string[], memory: Record<string, boolean> | undefined, startsCollapsed: (folder: string) => boolean): ReadonlySet<string> {
  const out = new Set<string>();
  for (const f of folders) {
    const open = memory?.[f];
    if (open === undefined ? startsCollapsed(f) : !open) out.add(f);
  }
  return out;
}

type Next = ReadonlySet<string> | ((c: ReadonlySet<string>) => ReadonlySet<string>);

/** The list's collapsed folders, and a setter that remembers what changed (accepts a set or an updater, like useState's). */
export function useFolderCollapse(repo: string, folders: readonly string[], startsCollapsed: (folder: string) => boolean = NEVER_COLLAPSED): [ReadonlySet<string>, (next: Next) => void] {
  const memory = useFolderMemory((s) => s.byRepo[repo]);
  const collapsed = useMemo(() => collapsedSet(folders, memory, startsCollapsed), [folders, memory, startsCollapsed]);
  const latest = useRef({ collapsed, folders });
  latest.current = { collapsed, folders };
  const setCollapsed = useCallback((next: Next) => {
    const { collapsed: cur, folders: all } = latest.current;
    const to = typeof next === 'function' ? next(cur) : next;
    const changes: Record<string, boolean> = {};
    for (const f of all) if (to.has(f) !== cur.has(f)) changes[f] = !to.has(f);
    if (Object.keys(changes).length > 0) useFolderMemory.getState().remember(repo, changes);
  }, [repo]);
  return [collapsed, setCollapsed];
}
