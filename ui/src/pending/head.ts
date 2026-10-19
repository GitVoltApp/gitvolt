import { create } from 'zustand';
import type { LocalBranch } from '../api/gen/LocalBranch';
import type { RefLabel } from '../api/gen/RefLabel';
import type { SidebarPayload } from '../api/gen/SidebarPayload';
import { useRuntime, worktreeOf } from '../app/runtime';
import type { WriteCtx } from '../write/client';

/** A worktree's HEAD: its branch by short name (`null`: detached) and its commit. */
export interface HeadAt { branch: string | null; sha: string | null }

/** Where a write leaves HEAD: on `branch` (absent: the branch it's on, as a reset does) at `sha`
 * (absent: not known ahead). */
export interface HeadMove { branch?: string | null; sha?: string }

interface HeadMoves {
  /** Per `repo|worktree`, the queued and running writes that move HEAD, in click order. */
  byWorktree: Record<string, { id: number; move: HeadMove }[]>;
}

/**
 * The action queue runs writes in click order (spec #2 §3.6), and the graph shows HEAD only once
 * they ran. What the user queues next is for HEAD as every write before it leaves it: the menus,
 * double-click, the toolbar's Push and Pull and the palette read `predictedHeadOf`, which folds
 * each queued or running checkout and reset into the graph's HEAD. Writes whose result isn't known
 * ahead (pull, commit, merge…) aren't folded in; the backend carries their moves forward.
 */
export const useHeadMoves = create<HeadMoves>(() => ({ byWorktree: {} }));

const keyOf = (repoId: number, worktree: string) => `${repoId}|${worktree}`;
let seq = 0;

/** Runs `fn` (a write that moves HEAD) with its move predicted until it ends however it ends. */
export async function withHeadMove<T>(ctx: WriteCtx, move: HeadMove, fn: () => Promise<T>): Promise<T> {
  const key = keyOf(ctx.repoId, ctx.worktree);
  const id = ++seq;
  useHeadMoves.setState((s) => ({ byWorktree: { ...s.byWorktree, [key]: [...(s.byWorktree[key] ?? []), { id, move }] } }));
  try {
    return await fn();
  } finally {
    useHeadMoves.setState((s) => {
      const { [key]: moves = [], ...rest } = s.byWorktree;
      const left = moves.filter((m) => m.id !== id);
      return { byWorktree: left.length ? { ...rest, [key]: left } : rest };
    });
  }
}

/** HEAD `now` with every queued or running move of `worktree`'s HEAD applied, in order. */
export function predictHead(repoId: number, worktree: string, now: HeadAt): HeadAt {
  const moves = useHeadMoves.getState().byWorktree[keyOf(repoId, worktree)];
  return moves ? moves.reduce<HeadAt>((h, { move }) => ({ branch: move.branch === undefined ? h.branch : move.branch, sha: move.sha ?? h.sha }), now) : now;
}

/** The HEAD of tab `tabId`'s active worktree once the queue has run: the graph's, moved. */
export function predictedHeadOf(tabId: string): HeadAt {
  const rt = useRuntime.getState().tabs[tabId];
  const active = worktreeOf(rt);
  const wt = rt?.graph?.worktrees.find((w) => w.path === active);
  const ref = wt ? wt.branch : rt?.graph?.head.branch ?? null;
  const now = { branch: ref?.replace(/^refs\/heads\//, '') ?? null, sha: (wt ? wt.head : rt?.graph?.head.target) ?? null };
  return rt?.repo && active ? predictHead(rt.repo.id, active, now) : now;
}

/** Labels marked for HEAD on `branch`: the local branch chips' `isHead` (the same array when
 * nothing changes). */
export function withHeadLabels(labels: RefLabel[], branch: string | null): RefLabel[] {
  const full = branch === null ? null : `refs/heads/${branch}`;
  return labels.some((l) => l.local !== null && l.isHead !== (l.local === full)) ? labels.map((l) => (l.local === null ? l : { ...l, isHead: l.local === full })) : labels;
}

/** The sidebar marked for HEAD on `branch` (the same payload when nothing changes). */
export function withHeadSidebar(s: SidebarPayload, branch: string | null): SidebarPayload {
  const mark = (b: LocalBranch) => b.isHead !== (b.name === branch);
  return s.locals.some(mark) ? { ...s, locals: s.locals.map((b) => (mark(b) ? { ...b, isHead: b.name === branch } : b)) } : s;
}
