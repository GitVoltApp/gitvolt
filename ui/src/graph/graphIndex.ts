import type { RefLabel } from '../api/gen/RefLabel';
import type { RowPayload } from '../api/gen/RowPayload';
import { branchMembership, labelsByRow, type BranchMembership } from './membership';

// The graph's per-payload indexes, computed once per payload and shared: GraphView renders
// from them, and the file menu (menu/menuEnv.ts) reads them on a right-click, so the menu
// never pays for them on its first opening. Keyed by the payload's
// own arrays, so a new payload gets new indexes and an unchanged array keeps its old ones.

const byRowCache = new WeakMap<readonly RefLabel[], Map<number, RefLabel[]>>();
/** A refreshed payload's labels → the labels it replaced, and both payloads' rows (`reuseUnchanged`). */
const predecessors = new WeakMap<readonly RefLabel[], { prev: readonly RefLabel[]; rows: readonly RowPayload[]; prevRows: readonly RowPayload[] }>();

/** `next` replaces `prev` (a refresh of the same graph, `rows` replacing `prevRows`):
 * `labelsByRowOf(next)` keeps the list of `prev`'s whose commit has the same labels, wherever its
 * row went (a commit on top moves every row down one), so those memoized rows needn't re-render.
 * Such a list's labels keep their old `row`: a list is found by its row, never by `label.row`. */
export function labelsFollow(next: readonly RefLabel[], prev: readonly RefLabel[], rows: readonly RowPayload[], prevRows: readonly RowPayload[]): void {
  if (next === prev) return;
  predecessors.set(next, { prev, rows, prevRows });
  // One generation back only: a chain of predecessors would keep every old payload's labels alive.
  predecessors.delete(prev);
}

/** The same label, on whichever row: all but `row` equal (a label is plain JSON). */
const sameLabel = (a: RefLabel, b: RefLabel) => a === b || JSON.stringify({ ...a, row: 0 }) === JSON.stringify({ ...b, row: 0 });

/** `labelsByRow(labels)`, once per labels array. */
export function labelsByRowOf(labels: RefLabel[]): Map<number, RefLabel[]> {
  let m = byRowCache.get(labels);
  if (!m) {
    m = labelsByRow(labels);
    const p = predecessors.get(labels);
    predecessors.delete(labels);
    const before = p && byRowCache.get(p.prev);
    if (p && before) {
      // Each labeled commit's old row, by id.
      const want = new Map<string, number>();
      for (const row of m.keys()) {
        const id = p.rows[row]?.id;
        if (id !== undefined) want.set(id, row);
      }
      const oldRow = new Map<number, number>();
      for (let i = 0; i < p.prevRows.length && oldRow.size < want.size; i++) {
        const row = want.get(p.prevRows[i].id);
        if (row !== undefined) oldRow.set(row, i);
      }
      for (const [row, list] of m) {
        const old = before.get(oldRow.get(row) ?? -1);
        if (old && old.length === list.length && old.every((l, i) => sameLabel(l, list[i]))) m.set(row, old);
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
