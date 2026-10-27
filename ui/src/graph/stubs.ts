import type { FoldMark } from '../api/gen/FoldMark';
import type { GhostBranch } from '../api/gen/GhostBranch';
import type { RowPayload } from '../api/gen/RowPayload';
import { graphLayout, laneNodeX, MARK_GAP, nodeRadius, type NodeGeometry } from './draw';
import { laneX, type Metrics } from './geometry';
import { HALF_FULL, type Seg } from './segments';

/**
 * Branch presets' marks. The layout places them as lines of their own (layout.rs), so draw.ts
 * draws them with the ordinary segments: a folded merge's tail (the merge's curve into its lane,
 * then a dotted end in the row under it) and a hidden-branch stub (a dotted start in the row above
 * a commit, then the fork curve into it). Here: which segments are those dotted ends, the texts
 * after the rows' messages, and the hit zones of the marks and of a ⋮ row's dotted lane. All
 * static: nothing here animates or redraws on its own.
 */

/** A tail's dotted end or a stub's dotted start: a dotted half straight down a lane that isn't its
 * row's node's (the layout draws no other). */
export const isMarkEnd = (seg: Seg, row: RowPayload): boolean => seg.dotted && seg.from === seg.to && seg.from !== row.lane && seg.half !== HALF_FULL;

/** After a folded merge's summary, in the ⋮ row's dim style, and its accessible name. */
export function foldText(f: FoldMark): { text: string; label: string } {
  const commits = f.count === 1 ? '1 folded commit' : `${f.count} folded commits`;
  return { text: `⋮ ${commits}`, label: `${commits} of ${f.branch}` };
}

/** After the summary of a commit hidden branches or tags fork from. */
export function hiddenText(ghosts: GhostBranch[]): { text: string; label: string } {
  const count = ghosts.length;
  const tags = ghosts.filter((g) => g.tag).length;
  const what = tags === 0 ? (count === 1 ? 'branch' : 'branches') : tags === count ? (count === 1 ? 'tag' : 'tags') : 'branches and tags';
  return { text: `⋮ ${count} hidden`, label: `${count} hidden ${what} ${count === 1 ? 'starts' : 'start'} here` };
}

type HitGeometry = NodeGeometry & { rows: RowPayload[]; scrollTop: number };

/** The row under content y `y` (canvas coordinates), with its index and top. */
function rowAt(o: HitGeometry, y: number): { row: RowPayload; i: number; top: number } | null {
  const i = Math.floor((y + o.scrollTop) / o.metrics.rowH);
  const row = o.rows[i];
  return row ? { row, i, top: i * o.metrics.rowH - o.scrollTop } : null;
}

/** A lane's x as drawn (the lane scroll applied), or null where the lines are cut (past the
 * collapse zone's edge, or scrolled out). */
function drawnLaneX(o: NodeGeometry, lane: number): number | null {
  const x = laneX(lane, o.metrics) - (o.scrollX ?? 0);
  return x >= 0 && x <= graphLayout(o.width, o.metrics, !!o.clipped || (o.scrollX ?? 0) > 0).area ? x : null;
}

/** The point is on a mark's run, off its node: along the centre line `yc` from the node at `nx`
 * to the mark's lane at `lx`, or along that lane from the centre line to `yEnd`. */
function onRun(x: number, y: number, nx: number, yc: number, lx: number, yEnd: number, m: Metrics): boolean {
  if (Math.hypot(x - nx, y - yc) <= nodeRadius(m)) return false;
  const along = Math.abs(y - yc) <= m.rowH / 4 && x >= Math.min(nx, lx) && x <= Math.max(nx, lx);
  return along || (Math.abs(x - lx) <= m.laneW / 2 && y >= Math.min(yc, yEnd) && y <= Math.max(yc, yEnd));
}

/** The row index whose hidden-branch stub is under the point (its lane from the row above's
 * centre, then its curve along the row's centre line to the node), or null. */
export function stubAt(o: HitGeometry, x: number, y: number): number | null {
  // The stub starts in the row above its commit: the row below the point may own it.
  for (const at of [rowAt(o, y), rowAt(o, y + o.metrics.rowH)]) {
    if (!at || at.row.stubLane == null) continue;
    const nx = laneNodeX(o, at.row);
    const lx = drawnLaneX(o, at.row.stubLane);
    if (nx === null || lx === null) continue;
    const yc = at.top + o.metrics.rowH / 2;
    if (onRun(x, y, nx, yc, lx, yc - o.metrics.rowH, o.metrics)) return at.i;
  }
  return null;
}

/** The row index of a ⋮ row whose dotted segment (its lane ± laneW/2) holds the point, or null. */
export function dottedAt(o: HitGeometry, x: number, y: number): number | null {
  const at = rowAt(o, y);
  if (!at || at.row.kind !== 'collapsed') return null;
  const nx = laneNodeX(o, at.row);
  return nx !== null && Math.abs(x - nx) <= o.metrics.laneW / 2 ? at.i : null;
}

/** The row index of a folded merge whose node or tail (its curve along the centre line, then its
 * lane down to its dotted end in the row below) is under the point, or null. */
export function foldMarkAt(o: HitGeometry, x: number, y: number): number | null {
  const m = o.metrics;
  // The tail ends in the row under its merge: the row above the point may own it.
  const at = rowAt(o, y);
  for (const cand of at ? [at, rowAt(o, y - m.rowH)] : []) {
    if (!cand?.row.fold) continue;
    const nx = laneNodeX(o, cand.row);
    if (nx === null) continue;
    const yc = cand.top + m.rowH / 2;
    if (cand === at && Math.hypot(x - nx, y - yc) <= nodeRadius(m)) return cand.i;
    const lane = cand.row.fold.tailLane;
    const lx = lane == null ? null : drawnLaneX(o, lane);
    if (lx !== null && onRun(x, y, nx, yc, lx, yc + m.rowH - MARK_GAP, m)) return cand.i;
  }
  return null;
}
