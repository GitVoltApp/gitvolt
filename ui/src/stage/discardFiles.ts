import type { FileChange } from '../api/gen/FileChange';
import { confirmAction } from '../ui/ConfirmDialog';
import type { WriteCtx } from '../write/client';
import { discardPaths, pathsOf } from './actions';

/** The files of a row (a file, or every file under a folder) a Discard takes: not a submodule,
 * whose own changes are discarded inside it (T4). Untracked files are among them (deleted). */
export const discardableOf = (files: readonly FileChange[]) => files.filter((f) => !f.submodule);

/** A row's Discard, from its hover button or its menu: arms in place where it started ("Click
 * again to discard 3 files in src"), then discards those files' unstaged changes (undoable). */
export async function discardFiles(ctx: WriteCtx, files: readonly FileChange[], name: string): Promise<boolean> {
  const n = files.length;
  const arm = n === 1 ? `Click again to discard ${name}` : `Click again to discard ${n} files in ${name}`;
  if (!(await confirmAction({ title: `Discard ${name}?`, body: 'Its changes are removed. You can undo this.', confirmLabel: 'Discard', arm, danger: true }))) return false;
  return discardPaths(ctx, pathsOf(files));
}
