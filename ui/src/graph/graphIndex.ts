import type { RefLabel } from '../api/gen/RefLabel';
import type { RowPayload } from '../api/gen/RowPayload';
import { branchMembership, labelsByRow, type BranchMembership } from './membership';

// The graph's per-payload indexes, computed once per payload and shared: GraphView renders
// from them, and the file menu (menu/menuEnv.ts) reads them on a right-click, so the menu
// never pays for them on its first opening (spec §7 latency budget). Keyed by the payload's
// own arrays, so a new payload gets new indexes and an unchanged array keeps its old ones.

const byRowCache = new WeakMap<readonly RefLabel[], Map<number, RefLabel[]>>();
/** A refreshed payload's labels → the labels it replaced (`reuseUnchanged`). */
const predecessors = new WeakMap<readonly RefLabel[], readonly RefLabel[]>();

/** `next` replaces `prev` (a refresh of the same graph): `labelsByRowOf(next)` keeps the lists of
 * `prev`'s rows whose labels are the same objects, so those memoized rows needn't re-render. */
export function labelsFollow(next: readonly RefLabel[], prev: readonly RefLabel[]): void {
  if (next === prev) return;
  predecessors.set(next, prev);
  // One generation back only: a chain of predecessors would keep every old payload's labels alive.
  predecessors.delete(prev);
}

/** `labelsByRow(labels)`, once per labels array. */
export function labelsByRowOf(labels: RefLabel[]): Map<number, RefLabel[]> {
  let m = byRowCache.get(labels);
  if (!m) {
    m = labelsByRow(labels);
    const prev = predecessors.get(labels);
    predecessors.delete(labels);
    const before = prev && byRowCache.get(prev);
    if (before) {
      for (const [row, list] of m) {
        const old = before.get(row);
        if (old && old.length === list.length && old.every((l, i) => l === list[i])) m.set(row, old);
      }
    }
    byRowCache.set(labels, m);
  }
  return m;
}

const membershipCache = new WeakMap<readonly RowPayload[], { byRow: Map<number, RefLabel[]>; pinned: string; value: (BranchMembership | null)[] }>();

/** `branchMembership(rows, byRow, pinnedRefs)` (F7), once per rows array and inputs. */
export function membershipOf(rows: RowPayload[], byRow: Map<number, RefLabel[]>, pinnedRefs: readonly string[] = []): (BranchMembership | null)[] {
  const hit = membershipCache.get(rows);
  const pinned = pinnedRefs.join('\n');
  if (hit && hit.byRow === byRow && hit.pinned === pinned) return hit.value;
  const value = branchMembership(rows, byRow, pinnedRefs);
  membershipCache.set(rows, { byRow, pinned, value });
  return value;
}
