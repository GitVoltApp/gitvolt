import type { GraphDelta } from '../api/gen/GraphDelta';
import type { GraphPayload } from '../api/gen/GraphPayload';
import type { GraphReply } from '../api/gen/GraphReply';
import type { RowPayload } from '../api/gen/RowPayload';

// A graph read names the version it holds (`since`), and the core answers with what changed
// since (gitvolt-graph's `delta.rs`): a refresh after a commit sends one row and the small
// parts, not every row, and the rows it keeps stay the same objects here, so the memoized rows
// and the per-payload indexes skip them.

/** The payload a tab holds, and its version (0: none the core knows of). */
export interface VersionedGraph {
  version: number;
  graph: GraphPayload;
}

/** `prev` changed by `d` (as the core's `delta::apply`): the rows it keeps are `prev`'s own
 * objects, and `prev.rows` itself when no row changed. `null` when `d` doesn't fit `prev`. */
export function applyDelta(prev: GraphPayload, d: GraphDelta): GraphPayload | null {
  const old = prev.rows;
  if (d.splices.length === 0 && d.patched.length === 0) return d.rows === old.length ? { ...d.rest, rows: old } : null;
  const rows: RowPayload[] = [];
  let i = 0;
  for (const s of d.splices) {
    const end = s.at + s.remove;
    if (s.at < i || end > old.length) return null;
    for (let k = i; k < s.at; k++) rows.push(old[k]);
    for (const r of s.insert) rows.push(r);
    i = end;
  }
  for (let k = i; k < old.length; k++) rows.push(old[k]);
  if (rows.length !== d.rows) return null;
  for (const p of d.patched) {
    if (rows[p.at]?.id !== p.row.id) return null;
    rows[p.at] = p.row;
  }
  return { ...d.rest, rows };
}

/** The payload a `graph` reply stands for, given the one held: a full reply's, a delta applied
 * to `held`, or a plain payload (a backend that ignores `since`: the website demo) as a full one
 * with no version. `null`: a delta that doesn't fit what's held (read the whole graph again). */
export function graphOfReply(reply: GraphReply | GraphPayload, held: VersionedGraph | undefined): VersionedGraph | null {
  if (!('kind' in reply)) return { version: 0, graph: reply };
  if (reply.kind === 'full') return { version: reply.version, graph: reply.graph };
  if (!held || held.version !== reply.since) return null;
  const graph = applyDelta(held.graph, reply);
  return graph && { version: reply.version, graph };
}
