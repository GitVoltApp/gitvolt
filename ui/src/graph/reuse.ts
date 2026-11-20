import type { GraphPayload } from '../api/gen/GraphPayload';
import type { RefLabel } from '../api/gen/RefLabel';
import type { RowPayload } from '../api/gen/RowPayload';
import { labelsFollow } from './graphIndex';

// A refresh's payload is all-new objects, even where nothing changed (a commit moves HEAD: one
// new row on top, the rest as they were). Keeping the old objects for what's equal lets the
// memoized rows, and the per-payload indexes (graphIndex.ts), skip the work.

/** Structural equality of two JSON values (what a payload holds). */
export function sameJsonValue(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) return false;
  if (Array.isArray(a)) {
    if (!Array.isArray(b) || a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!sameJsonValue(a[i], b[i])) return false;
    return true;
  }
  if (Array.isArray(b)) return false;
  const ka = Object.keys(a);
  if (ka.length !== Object.keys(b).length) return false;
  const ra = a as Record<string, unknown>;
  const rb = b as Record<string, unknown>;
  for (const k of ka) if (!(k in rb) || !sameJsonValue(ra[k], rb[k])) return false;
  return true;
}

// Every field of a row: TypeScript fails here when one is added, so the comparison never misses it.
const ROW_FIELDS = Object.keys({
  id: 1, kind: 1, lane: 1, color: 1, segments: 1, summary: 1, bodyFirstLine: 1, authorName: 1, authorEmail: 1, authorTime: 1,
  committerTime: 1, parents: 1, mrRefs: 1, wip: 1, collapsed: 1, fold: 1, ghosts: 1, stubLane: 1,
} satisfies Record<keyof RowPayload, 1>) as (keyof RowPayload)[];

/** `sameJsonValue` for two rows, without listing their keys (a refresh compares thousands). */
export function sameRow(a: RowPayload, b: RowPayload): boolean {
  for (const k of ROW_FIELDS) {
    const x = a[k];
    const y = b[k];
    if (x !== y && !sameJsonValue(x, y)) return false;
  }
  return true;
}

/** `next`'s items, each one equal to `prev`'s with the same key replaced by that one; `prev`
 * itself when every item is kept, in the same order. */
function keep<T>(prev: T[], next: T[], key: (t: T) => string, same: (a: T, b: T) => boolean = sameJsonValue): T[] {
  const old = new Map<string, T>();
  for (const t of prev) old.set(key(t), t);
  let all = prev.length === next.length;
  const out = next.map((t, i) => {
    const o = old.get(key(t));
    if (o === undefined || !same(o, t)) {
      all = false;
      return t;
    }
    if (o !== prev[i]) all = false;
    return o;
  });
  return all ? prev : out;
}

const labelKey = (l: RefLabel) => `${l.row}\u0000${l.tag ? 't' : 'b'}\u0000${l.name}`;

/** `next` (a refresh of `prev`'s graph), with every row and label equal to one of `prev`'s
 * replaced by that object, and the arrays themselves kept when nothing in them changed. */
export function reuseUnchanged(prev: GraphPayload, next: GraphPayload): GraphPayload {
  const rows = keep<RowPayload>(prev.rows, next.rows, (r) => r.id, sameRow);
  const labels = keep<RefLabel>(prev.labels, next.labels, labelKey);
  labelsFollow(labels, prev.labels, rows, prev.rows);
  return rows === next.rows && labels === next.labels ? next : { ...next, rows, labels };
}
