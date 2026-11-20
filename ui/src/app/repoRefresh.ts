import type { AppEvent } from '../api/gen/AppEvent';
import type { OpKind } from '../api/gen/OpKind';

/** What a repo tab reloads for one of its repo's events: `refresh`'s options. */
export interface RefreshOpts { graphOnly?: boolean }

/** A running op, as `useOps` keeps it. */
export interface RunningOp { repo: number | null; kind: OpKind; step?: unknown }

/** Transfers: their ref moves come as a lone `refsUpdated`, never with a write's `repoChanged`. */
const TRANSFERS: ReadonlySet<OpKind> = new Set<OpKind>(['fetch', 'clone']);

/**
 * The refresh rules of one repo tab's event listener (`RepoTab`), one call per event. `null`:
 * nothing to reload. A write ends with `repoChanged` (`refs` among its kinds when a ref moved),
 * then `refsUpdated` for the same moves, then `opFinished`: one load covers all three.
 * - `repoChanged`: the graph; the sidebar and repo info too when refs, HEAD, stashes, config or
 *   submodules changed (a worktree/index change can't alter them, 1C review M5).
 * - `refsUpdated` right after a write's `repoChanged` with `refs`: that one's load already reads
 *   the moved refs. Anything else for the repo in between, and it reloads as usual.
 * - `refsUpdated` during a rebase's steps: the graph alone (2D T18); the sidebar and repo info
 *   are owed until a full load starts.
 * - `opFinished` of a rebase or pull: the full load, only if one is still owed.
 */
export function refreshGate(repoId: number): (ev: AppEvent, ops: readonly RunningOp[]) => RefreshOpts | null {
  let prev: AppEvent | null = null;
  let sideOwed = false;
  const full = (): RefreshOpts => { sideOwed = false; return {}; };
  const decide = (ev: AppEvent, last: AppEvent | null, ops: readonly RunningOp[]): RefreshOpts | null => {
    const mine = ops.filter((o) => o.repo === repoId);
    switch (ev.type) {
      case 'opFinished':
        return (ev.kind === 'rebase' || ev.kind === 'pull') && sideOwed ? full() : null;
      case 'refsUpdated':
        if (last?.type === 'repoChanged' && last.kinds.includes('refs') && mine.some((o) => !TRANSFERS.has(o.kind))) return null;
        if (mine.some((o) => o.step)) { sideOwed = true; return { graphOnly: true }; }
        return full();
      case 'repoChanged':
        return ev.kinds.some((k) => k === 'refs' || k === 'head' || k === 'stash' || k === 'config' || k === 'submodules') ? full() : { graphOnly: true };
      default:
        return null;
    }
  };
  return (ev, ops) => {
    if (!('repo' in ev) || ev.repo !== repoId) return null;
    const last = prev;
    prev = ev;
    return decide(ev, last, ops);
  };
}
