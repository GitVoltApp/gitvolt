import { GitMerge } from 'lucide-react';
import { checkout } from '../branches/checkout';
import type { DroppedLabel } from '../graph/branchDrag';
import { ontoRows } from '../irebase/menus';
import { branchRefOf, commitTargetOf, fileMenuEnv, type MenuEnv } from '../menu/menuEnv';
import type { MenuRow } from '../menu/types';
import { predictedHeadOf } from '../pending/head';
import type { RepoViewStore } from '../repo/store';
import { journalKey, useJournal } from '../undo/store';
import type { WriteCtx } from '../write/client';
import { integrateRows, startIntegrate } from './integrate';

/**
 * The menu of a local branch chip A dropped on another, B: it acts on B, as the right-click
 * Integrate rows of A's chip do with B checked out. Merge A into B (or Fast-forward B to A),
 * Rebase B onto A, Interactive rebase B onto A. "Fast-forward A to B" is left out: it moves A,
 * the dragged branch. When A is already in B, the merge row stays, greyed, so a drop never looks
 * like it did nothing.
 *
 * B not checked out: the same rows, worked out as if B were HEAD, each checking B out first and
 * running only once it landed (you end up on B). B checked out in another worktree: every row
 * greyed with where; an operation in progress here greys them with its own reason first.
 */
export function branchDropRows(from: DroppedLabel, onto: DroppedLabel, env: MenuEnv): MenuRow[] {
  const ctx = env.write;
  const local = onto.label.local;
  const b = local?.replace(/^refs\/heads\//, '');
  if (!ctx || !local || !b || !from.label.local || from.label.local === local) return [];
  const here = env.headBranch === b;
  const asB: MenuEnv = here ? env : { ...env, headBranch: b, headSha: onto.row.id };
  const a = from.label.name;
  const t = commitTargetOf(from.row, branchRefOf(from.label));
  const busy = env.inProgress ? `Finish or abort the ${env.inProgress} first` : null;
  const integrate = integrateRows(t, asB, busy, { ff: () => {}, go: (kind, y, ffOnly) => void startIntegrate(ctx, kind, y, b, ffOnly) })
    .filter((r) => r.kind !== 'action' || r.id !== 'integrate.ff');
  const contained = !!asB.headSha && asB.isAncestor?.(t.sha, asB.headSha) === true;
  const merged: MenuRow[] = contained && integrate.length === 0
    ? [{ kind: 'action', id: 'integrate.merge', label: `Merge ${a} into ${b}`, icon: GitMerge, tooltip: `git merge ${a} on ${b}`, disabledReason: busy ?? `${b} already contains ${a}`, run: () => {} }]
    : [];
  const rows = [...merged, ...integrate, ...ontoRows(t, asB)];
  if (here) return rows;
  // The label's worktree is as of the last refresh: a queued checkout may already be leaving it.
  const away = onto.label.checkedOut && onto.label.checkedOut !== env.activeWorktree ? onto.label.checkedOut : null;
  const where = away && !busy ? `${b} is checked out in ${env.worktreeShown(away)}` : null;
  return rows.map((r) => {
    if (r.kind !== 'action') return r;
    if (where) return { ...r, disabledReason: where };
    if (r.disabledReason) return r;
    return {
      ...r,
      tooltip: `${r.tooltip}. Checks out ${b} first.`,
      run: () => void (async () => {
        if (await checkedOut(ctx, b, local, onto.row.id)) r.run();
      })(),
    };
  });
}

const banners = (ctx: WriteCtx) => useJournal.getState().states[journalKey(ctx.repoId, ctx.worktree)]?.banners ?? [];

/** Checks B out, and says whether the action can follow: HEAD landed and the changes it carried
 * over came back clean. Restored changes that conflict leave an unmerged index the action would
 * fail on: the user stays on B with the conflict banner, as after a checkout by hand. */
async function checkedOut(ctx: WriteCtx, b: string, local: string, tip: string): Promise<boolean> {
  const before = new Set(banners(ctx).map((x) => x.entry));
  if (!(await checkout(ctx, { kind: 'branch', name: b }, { head: predictedHeadOf(ctx.tabId).sha, refs: { [local]: tip } }))) return false;
  return !banners(ctx).some((x) => x.kind === 'autostashConflicts' && !before.has(x.entry));
}

/** The drop menu's builder, from the tab's store (re-run by the menu's refresh). */
export const branchDropMenu = (store: RepoViewStore, from: DroppedLabel, onto: DroppedLabel) => (): MenuRow[] =>
  branchDropRows(from, onto, fileMenuEnv(store));
