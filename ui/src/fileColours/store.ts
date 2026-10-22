import { useMemo } from 'react';
import type { FileScope } from '../api/gen/FileScope';
import type { Profile } from '../api/gen/Profile';
import { useRuntime } from '../app/runtime';
import { useAppState } from '../app/state';
import { tabIdOf } from '../app/tabStores';
import { useRepoViewStore } from '../repo/store';

export type { FileScope } from '../api/gen/FileScope';

export const NO_SCOPES: readonly FileScope[] = Object.freeze([]);

/** A repository's scopes, in order: the stored array itself, so its identity is the matcher's cache key. */
export function scopesOf(profile: Profile, commonDir: string | null): readonly FileScope[] {
  return (commonDir && profile.fileColours[commonDir]) || NO_SCOPES;
}

/** Saves a repository's scopes; an empty list removes its key. */
export function setScopes(commonDir: string, scopes: FileScope[]): void {
  useAppState.getState().updateProfile((p) => {
    const fileColours = { ...p.fileColours };
    if (scopes.length) fileColours[commonDir] = scopes;
    else delete fileColours[commonDir];
    return { ...p, fileColours };
  });
}

/** The tab's repository (the same for every worktree of it); null until its repo info has loaded. */
export function commonDirOfTab(tabId: string | null): string | null {
  return (tabId && useRuntime.getState().tabs[tabId]?.info?.commonDir) || null;
}

export function useTabCommonDir(tabId: string | null): string | null {
  return useRuntime((s) => (tabId ? s.tabs[tabId]?.info?.commonDir : null) || null);
}

/** The scopes of the repository behind the tab this RepoView store belongs to. */
export function useRepoScopes(): { commonDir: string | null; scopes: readonly FileScope[] } {
  const store = useRepoViewStore();
  const commonDir = useTabCommonDir(tabIdOf(store));
  const scopes = useAppState((s) => scopesOf(s.profile, commonDir));
  return useMemo(() => ({ commonDir, scopes }), [commonDir, scopes]);
}

export const newScopeId = (): string => crypto.randomUUID();
