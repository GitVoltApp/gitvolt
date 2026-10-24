import { ListMinus, ListPlus, Trash2 } from 'lucide-react';
import type { FileChange } from '../api/gen/FileChange';
import type { FileListPayload } from '../api/gen/FileListPayload';
import type { MenuRow } from '../menu/types';
import type { WriteCtx } from '../write/client';
import { discardAll, discardUnstaged, pathsOf, stageAll, stagePaths, unstageAll } from './actions';
import { COMMIT_QUEUED, stagingKey, useStaging } from './store';

// The WIP lists' right-click rows that stand for their buttons: a section header's (and its
// list's empty space), and the WIP header's Discard all (also on the graph's WIP row). Each
// runs what its button runs; a confirm arms the menu row in place (`confirmAction`'s origin).

const row = (r: Omit<Extract<MenuRow, { kind: 'action' }>, 'kind'>): MenuRow => ({ kind: 'action', ...r });

/** What a Discard all confirms: the changed files shown (`count`, each path once) and their
 * paths, a rename's both (UX R1 C.2). */
export interface DiscardAllScope { count: number; confirmed?: string[]; /** A merge, rebase… in the worktree (§7.2: Abort instead). */ inProgress: boolean }

/** The scope of the lists shown, or `null` while none is loaded. */
export function discardScopeOf(lists: readonly FileListPayload[]): Pick<DiscardAllScope, 'count' | 'confirmed'> | null {
  if (lists.length === 0) return null;
  const shown = new Set<string>();
  const paths = new Set<string>();
  for (const l of lists) for (const f of l.files) { shown.add(f.path); paths.add(f.path); if (f.oldPath) paths.add(f.oldPath); }
  return { count: shown.size, confirmed: [...paths] };
}

/** `Discard all changes…`: the WIP header's Discard all, staged, unstaged and untracked. */
export function discardAllRow(ctx: WriteCtx, scope: DiscardAllScope): MenuRow {
  const st = useStaging.getState();
  const key = stagingKey(ctx.repoId, ctx.worktree);
  const reason = st.committing[key] ? COMMIT_QUEUED : st.discarding[key] ? 'Discarding…' : scope.inProgress ? 'Abort instead' : scope.count === 0 ? 'Nothing to discard' : undefined;
  return row({
    id: 'wip.discardAll', label: 'Discard all changes…', icon: Trash2,
    tooltip: 'Discard every staged, unstaged and untracked change, back to HEAD (you can undo this)',
    run: () => void discardAll(ctx, scope.count, scope.confirmed), disabledReason: reason,
  });
}

/** A section's own rows, as its header's buttons: Unstaged has Stage all and Discard unstaged,
 * Staged has Unstage all; then Discard all. `guardConflicts`: Stage all names the other unstaged
 * paths (`git add -A` would mark every conflict resolved). */
export function sectionRows(ctx: WriteCtx, which: 'unstaged' | 'staged', files: readonly FileChange[], o: { guardConflicts: boolean; all: DiscardAllScope }): MenuRow[] {
  const queued = useStaging.getState().committing[stagingKey(ctx.repoId, ctx.worktree)] ? COMMIT_QUEUED : undefined;
  const own: MenuRow[] = [];
  if (files.length > 0 && which === 'unstaged') {
    own.push(row({ id: 'wip.stageAll', label: 'Stage all changes', icon: ListPlus, tooltip: 'Stage every unstaged change', run: () => void (o.guardConflicts ? stagePaths(ctx, pathsOf(files)) : stageAll(ctx)), disabledReason: queued }));
  } else if (files.length > 0) {
    own.push(row({ id: 'wip.unstageAll', label: 'Unstage all changes', icon: ListMinus, tooltip: 'Move every staged change back to Unstaged', run: () => void unstageAll(ctx), disabledReason: queued }));
  }
  const discard: MenuRow[] = [];
  if (which === 'unstaged' && files.length > 0 && !files.every((f) => f.submodule)) {
    discard.push(row({ id: 'wip.discardUnstaged', label: 'Discard unstaged changes', icon: Trash2, tooltip: 'Discard every unstaged change and delete the untracked files; the staged ones stay (you can undo this)', run: () => void discardUnstaged(ctx), disabledReason: queued }));
  }
  discard.push(discardAllRow(ctx, o.all));
  return [...own, ...(own.length ? [{ kind: 'separator' } as const] : []), ...discard];
}
