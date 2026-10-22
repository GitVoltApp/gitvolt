import { useMemo } from 'react';
import { matcherFor } from './match';
import { useRepoScopes, type FileScope } from './store';
import { useScopeTints } from './useTints';

export interface ListColours {
  fileTint(path: string): string | null;
  folderTint(folder: string): string | null;
  /** Only the outermost covered folder row of a start-collapsed scope starts closed. `parentRow`:
   * the nearest folder above it that is a row of its own (a merged chain `admin/dist/assets` is one
   * row, so `admin/dist` isn't), null at the top. */
  startsCollapsed(folder: string, parentRow: string | null): boolean;
}

/** The list's colours from scopes and their fills (one per scope, null when unusable). */
export function listColoursOf(scopes: readonly FileScope[], tints: readonly (string | null)[]): ListColours {
  const m = matcherFor(scopes);
  const fill = (i: number): string | null => (i < 0 ? null : (tints[i] ?? null));
  return {
    fileTint: (path) => fill(m.scopeOf(path)),
    folderTint: (folder) => fill(m.folderScopeOf(folder)),
    startsCollapsed: (folder, parentRow) => {
      const i = m.folderScopeOf(folder);
      if (i < 0 || !scopes[i].startCollapsed) return false;
      return parentRow === null || m.folderScopeOf(parentRow) !== i;
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
