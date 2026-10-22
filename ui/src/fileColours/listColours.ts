import { useMemo } from 'react';
import { matcherFor } from './match';
import { useRepoScopes, type FileScope } from './store';
import { useScopeTints } from './useTints';

export interface ListColours {
  fileTint(path: string): string | null;
  folderTint(folder: string): string | null;
  /** Only the outermost covered folder of a start-collapsed scope starts closed. */
  startsCollapsed(folder: string): boolean;
}

const parentOf = (folder: string): string | null => {
  const i = folder.lastIndexOf('/');
  return i < 0 ? null : folder.slice(0, i);
};

/** The list's colours from scopes and their fills (one per scope, null when unusable). */
export function listColoursOf(scopes: readonly FileScope[], tints: readonly (string | null)[]): ListColours {
  const m = matcherFor(scopes);
  const fill = (i: number): string | null => (i < 0 ? null : (tints[i] ?? null));
  return {
    fileTint: (path) => fill(m.scopeOf(path)),
    folderTint: (folder) => fill(m.folderScopeOf(folder)),
    startsCollapsed: (folder) => {
      const i = m.folderScopeOf(folder);
      if (i < 0 || !scopes[i].startCollapsed) return false;
      const parent = parentOf(folder);
      return parent === null || m.folderScopeOf(parent) !== i;
    },
  };
}

const NONE: ListColours = { fileTint: () => null, folderTint: () => null, startsCollapsed: () => false };

/** The current repository's colours; stable until its scopes or the theme change. */
export function useListColours(): ListColours {
  const { scopes } = useRepoScopes();
  const tints = useScopeTints(scopes);
  return useMemo(() => (scopes.length === 0 ? NONE : listColoursOf(scopes, tints)), [scopes, tints]);
}
