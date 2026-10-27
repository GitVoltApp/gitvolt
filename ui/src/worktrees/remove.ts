import { api } from '../api/client';
import { useAppState } from '../app/state';
import { useRuntime } from '../app/runtime';
import { currentOrigin } from '../ui/arm/origin';
import { confirmAction, type ConfirmRequest } from '../ui/ConfirmDialog';
import { deleteBranch } from '../branches/delete';
import { runWrite, type WriteCtx } from '../write/client';
import { closeTab, tabWorktree } from '../app/tabs';
import { mainWorktreeOf, setActiveWorktree } from './active';
import { worktreeDisplay } from './paths';

/** Remove (§11.1): always confirmed (it isn't undoable); a dirty worktree asks once more before
 * `--force`. Every tab whose active worktree it is moves to the main worktree first. Resolves
 * the tab a write that follows it runs in (ctx's, or the twin on main when ctx's was closed), or
 * `null` when nothing was removed (cancelled, or failed). `ask`: the first confirm, for a caller
 * that names more than the removal. */
export async function removeWorktree(ctx: WriteCtx, path: string, branch: string | null, ask?: (shown: string) => ConfirmRequest): Promise<string | null> {
  const origin = currentOrigin();
  const main = mainWorktreeOf(ctx.tabId) ?? ctx.worktree;
  const shown = worktreeDisplay(main, path);
  const stays = branch ? `branch ${branch.replace(/^refs\/heads\//, '')} stays` : 'its commits stay';
  if (!(await confirmAction(ask?.(shown) ?? { title: `Remove worktree ${shown}?`, body: `Its directory is deleted; ${stays}.`, confirmLabel: 'Remove', arm: `Click again to remove ${shown}: its directory is deleted`, danger: true }, origin))) return null;
  const at = { ...ctx, worktree: main };
  const onIt = () => useAppState.getState().profile.tabs.filter((t) => t.kind === 'repo' && useRuntime.getState().tabs[t.id]?.repo?.id === ctx.repoId && useRuntime.getState().tabs[t.id]?.worktree === path).map((t) => t.id);
  // A tab on the removed worktree moves to main first (§11.1). Where another tab already shows
  // main, that tab is a twin: the removed one is closed after the removal instead (and a write
  // that follows runs in the twin).
  let moved: string[] = [];
  let closing: Array<{ id: string; twin: string }> = [];
  const moveAway = () => {
    for (const id of onIt()) {
      const tabs = useAppState.getState().profile.tabs;
      const me = tabs.find((x) => x.id === id);
      const twin = tabs.find((t) => t.id !== id && t.kind === 'repo' && t.path === me?.path && tabWorktree(t) === main)?.id;
      if (twin) closing.push({ id, twin });
      else { setActiveWorktree(id, main); moved.push(id); }
    }
  };
  const putBack = () => { for (const id of moved) setActiveWorktree(id, path); moved = []; closing = []; };
  const send = (force: boolean) => { moveAway(); return runWrite(at, () => api.worktreeRemove(ctx.repoId, main, path, force), { origin }); };
  const done = () => {
    for (const { id } of closing) { const app = useAppState.getState(); app.setProfile(closeTab(app.profile, id)); }
    return closing.find((c) => c.id === ctx.tabId)?.twin ?? ctx.tabId;
  };
  const out = await send(false);
  if (out === null) { putBack(); return null; }
  if (out.status !== 'needsForce') return done();
  putBack();
  const body = `${shown} has changes that aren't committed. They're lost: this can't be undone.`;
  const again = await confirmAction({ title: 'Remove it anyway?', body, confirmLabel: 'Remove', arm: `Click again to remove ${shown}: its uncommitted changes are lost`, caption: body, danger: true }, origin);
  if (!again) return null;
  const forced = await send(true);
  if (forced === null) { putBack(); return null; }
  return done();
}

/** Remove with its branch (#9): one confirm names both; then the removal (with its dirty
 * question), then the local branch's delete from the main worktree (undoable, with its unmerged
 * question: news the first confirm didn't have). A cancelled or failed removal leaves the branch. */
export async function removeWorktreeAndBranch(ctx: WriteCtx, path: string, branch: { name: string; oid: string }): Promise<void> {
  const origin = currentOrigin();
  const main = mainWorktreeOf(ctx.tabId) ?? ctx.worktree;
  const tabId = await removeWorktree(ctx, path, branch.name, (shown) => ({
    title: `Remove worktree ${shown} and delete ${branch.name}?`, body: `Its directory is deleted (that can't be undone); you can undo deleting the branch.`,
    confirmLabel: 'Remove and delete', arm: `Click again to remove ${shown} and delete ${branch.name}`, danger: true,
  }));
  if (tabId === null) return;
  await deleteBranch({ ...ctx, tabId, worktree: main }, { branch: branch.name, local: { oid: branch.oid }, remote: null }, false, origin);
}
