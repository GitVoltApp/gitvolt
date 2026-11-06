import { create } from 'zustand';
import type { StagingUndoState } from '../api/gen/StagingUndoState';

/** One worktree of one open repo. */
export const stagingKey = (repo: number, worktree: string) => `${repo}\u0000${worktree}`;

export const COMMIT_QUEUED = 'Commit queued';
/** A control's reason while a Discard of its files runs. */
export const DISCARDING = 'Discarding…';

interface StagingStore {
  /** The staging undo log's state (spec #2 §7.6), from every write's answer (`applyResult`). */
  states: Record<string, StagingUndoState>;
  /** A commit for the worktree is queued or running: staging and discards wait (§3.6). */
  committing: Record<string, boolean>;
  /** UX R1 C.2: a Discard all of the worktree is running: another waits for its answer. */
  discarding: Record<string, boolean>;
  /** The paths a file, folder or hunk Discard of the worktree is running on. Another Discard of
   * any of them waits for its answer: sent meanwhile, it found nothing left and failed. */
  discardingPaths: Record<string, readonly string[]>;
  /** Stage, unstage and discard writes of the worktree sent and not yet answered: the commit
   * button waits for them, so it never decides from lists about to change. */
  staging: Record<string, number>;
  set(repo: number, worktree: string, s: StagingUndoState): void;
  setCommitting(repo: number, worktree: string, on: boolean): void;
  setDiscarding(repo: number, worktree: string, on: boolean): void;
  /** Marks `paths` as being discarded (`on`), or answered. */
  setDiscardingPaths(repo: number, worktree: string, paths: readonly string[], on: boolean): void;
  /** `+1` when a staging write is sent, `-1` when it's answered (or failed). */
  addStaging(repo: number, worktree: string, delta: 1 | -1): void;
}

export const useStaging = create<StagingStore>((set) => ({
  states: {},
  committing: {},
  discarding: {},
  discardingPaths: {},
  staging: {},
  set: (repo, worktree, s) => set((st) => ({ states: { ...st.states, [stagingKey(repo, worktree)]: s } })),
  setCommitting: (repo, worktree, on) => set((st) => ({ committing: { ...st.committing, [stagingKey(repo, worktree)]: on } })),
  setDiscarding: (repo, worktree, on) => set((st) => ({ discarding: { ...st.discarding, [stagingKey(repo, worktree)]: on } })),
  setDiscardingPaths: (repo, worktree, paths, on) => set((st) => {
    const k = stagingKey(repo, worktree);
    const now = st.discardingPaths[k] ?? [];
    return { discardingPaths: { ...st.discardingPaths, [k]: on ? [...now, ...paths] : now.filter((p) => !paths.includes(p)) } };
  }),
  addStaging: (repo, worktree, delta) => set((st) => {
    const k = stagingKey(repo, worktree);
    return { staging: { ...st.staging, [k]: Math.max(0, (st.staging[k] ?? 0) + delta) } };
  }),
}));

export const useCommitting = (repo: number, worktree: string) => useStaging((s) => !!s.committing[stagingKey(repo, worktree)]);
/** A staging write of the worktree is in flight (`staging`). */
export const useStagingBusy = (repo: number, worktree: string) => useStaging((s) => (s.staging[stagingKey(repo, worktree)] ?? 0) > 0);
export const useDiscardingAll = (repo: number, worktree: string) => useStaging((s) => !!s.discarding[stagingKey(repo, worktree)]);

/** A Discard already running takes one of `paths`: a Discard all or unstaged of the worktree, or
 * a file, folder or hunk Discard of one of them. */
export function discardingAny(st: Pick<StagingStore, 'discarding' | 'discardingPaths'>, repo: number, worktree: string, paths: readonly string[]): boolean {
  const k = stagingKey(repo, worktree);
  if (st.discarding[k]) return true;
  const busy = st.discardingPaths[k];
  return !!busy?.length && paths.some((p) => busy.includes(p));
}
export const useDiscardingAny = (repo: number, worktree: string, paths: readonly string[]) => useStaging((s) => discardingAny(s, repo, worktree, paths));
