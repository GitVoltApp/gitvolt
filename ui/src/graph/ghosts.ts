import type { RowPayload } from '../api/gen/RowPayload';
import { graphLayout, laneNodeX, LINE_W, nodeRadius, type NodeGeometry } from './draw';
import { laneX, type Metrics, type PathOp } from './geometry';

/**
 * Branch presets' marks on the canvas, beside draw.ts (which draws them): ghost lines where hidden
 * branches fork, the fold mark of a merge that holds a folded branch, and the hit zones of those
 * and of a ⋮ row's dotted lane. All static drawings: nothing here animates or redraws on its own.
 * Every x comes from `laneX`, as the lanes' own, so a reveal panel drawn with the same code lines
 * up with the graph.
 */

/** A ghost line's alpha where it leaves its node; it fades to 0 at its end. */
export const GHOST_ALPHA = 0.7;
/** The fold stub's alpha at the node; it fades to 0 at its end. */
export const STUB_ALPHA = 0.5;
/** The fold stub's length, in rows. */
export const STUB_LEN = 0.6;
/** The dot inside a folded merge's ring, as a share of a node's radius. */
export const FOLD_DOT = 0.4;

/** How far right the k-th ghost on a row ends, in lanes: about two, a little apart so a second
 * and third read as separate lines. */
const ghostReach = (k: number) => 1.5 + 0.5 * Math.min(k, 2);

/** The ghost stubs' paths for a row: one quadratic curve per ghost, from the node up and right, ending ~2 lanes
 * right of it and no higher than the row's top edge. */
export function ghostPaths(row: RowPayload, rowTop: number, m: Metrics): PathOp[][] {
  const ghosts = row.ghosts ?? [];
  if (ghosts.length === 0) return [];
  const x = laneX(row.lane, m);
  const yc = rowTop + m.rowH / 2;
  // The end sits half a line below the top edge, so the stroke stays inside the row.
  const top = rowTop + LINE_W / 2;
  return ghosts.map((_, k) => {
    const end = x + ghostReach(k) * m.laneW;
    // Leaves the node level, then turns up: the control point on the node's line, most of the
    // way across.
    return [{ op: 'M', x, y: yc }, { op: 'Q', cx: x + 0.7 * (end - x), cy: yc, x: end, y: top }];
  });
}

/** A folded merge's stub: from the ring's outer edge towards the lower right (the next lane, a row
 * down), STUB_LEN rows long. */
export function foldStub(x: number, y: number, m: Metrics): { x0: number; y0: number; x1: number; y1: number } {
  const len = Math.hypot(m.laneW, m.rowH);
  const [ux, uy] = [m.laneW / len, m.rowH / len];
  const from = nodeRadius(m) + LINE_W / 2;
  const to = from + STUB_LEN * m.rowH;
  return { x0: x + ux * from, y0: y + uy * from, x1: x + ux * to, y1: y + uy * to };
}

/** `color` (`#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb()` or `rgba()`, what themes allow) at `alpha`,
 * for a gradient's stops. Anything else is returned as is. */
export function withAlpha(color: string, alpha: number): string {
  const c = color.trim();
  const hex = c.match(/^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i);
  let rgb: number[] | null = null;
  if (hex) {
    const h = hex[1].length === 3 ? [...hex[1]].map((d) => d + d).join('') : hex[1].slice(0, 6);
    rgb = [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16));
  } else {
    const m = c.match(/^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*(?:,\s*[\d.]+\s*)?\)$/i);
    if (m) rgb = [Number(m[1]), Number(m[2]), Number(m[3])];
  }
  return rgb ? `rgba(${rgb.join(',')},${alpha})` : c;
}

type HitGeometry = NodeGeometry & { rows: RowPayload[]; scrollTop: number };

/** The row under content y `y` (canvas coordinates), with its index and top. */
function rowAt(o: HitGeometry, y: number): { row: RowPayload; i: number; top: number } | null {
  const i = Math.floor((y + o.scrollTop) / o.metrics.rowH);
  const row = o.rows[i];
  return row ? { row, i, top: i * o.metrics.rowH - o.scrollTop } : null;
}

/** The lane area's right edge while the lanes are clipped (marks are cut there), else the width. */
const areaOf = (o: NodeGeometry) => graphLayout(o.width, o.metrics, !!o.clipped || (o.scrollX ?? 0) > 0).area;

/** The row index whose ghost zone (the stubs' bounding box) holds the point, or null. */
export function ghostAt(o: HitGeometry, x: number, y: number): number | null {
  const at = rowAt(o, y);
  if (!at || !at.row.ghosts?.length) return null;
  const nx = laneNodeX(o, at.row);
  if (nx === null) return null;
  const m = o.metrics;
  // Not the node itself: that stays the node's (its tooltip, its click).
  if (Math.hypot(x - nx, y - (at.top + m.rowH / 2)) <= nodeRadius(m)) return null;
  const right = Math.min(nx + ghostReach(at.row.ghosts.length - 1) * m.laneW + LINE_W / 2, areaOf(o));
  return x >= nx && x <= right && y <= at.top + m.rowH / 2 + LINE_W / 2 ? at.i : null;
}

/** The row index of a ⋮ row whose dotted segment (its lane ± laneW/2) holds the point, or null. */
export function dottedAt(o: HitGeometry, x: number, y: number): number | null {
  const at = rowAt(o, y);
  if (!at || at.row.kind !== 'collapsed') return null;
  const nx = laneNodeX(o, at.row);
  return nx !== null && Math.abs(x - nx) <= o.metrics.laneW / 2 ? at.i : null;
}

/** The row index of a folded merge's node or stub under the point, or null. */
export function foldMarkAt(o: HitGeometry, x: number, y: number): number | null {
  // The stub reaches into the row below: the row above the point may own it.
  const at = rowAt(o, y);
  for (const cand of at ? [at, rowAt(o, y - o.metrics.rowH)] : []) {
    if (!cand?.row.fold) continue;
    const nx = laneNodeX(o, cand.row);
    if (nx === null) continue;
    const m = o.metrics;
    const ny = cand.top + m.rowH / 2;
    if (cand === at && Math.hypot(x - nx, y - ny) <= nodeRadius(m)) return cand.i;
    if (x <= areaOf(o) && nearSegment(x, y, foldStub(nx, ny, m), m.laneW / 4)) return cand.i;
  }
  return null;
}

/** The point is within `d` of the segment. */
function nearSegment(x: number, y: number, s: { x0: number; y0: number; x1: number; y1: number }, d: number): boolean {
  const [dx, dy] = [s.x1 - s.x0, s.y1 - s.y0];
  const t = Math.max(0, Math.min(1, ((x - s.x0) * dx + (y - s.y0) * dy) / (dx * dx + dy * dy)));
  return Math.hypot(x - (s.x0 + t * dx), y - (s.y0 + t * dy)) <= d;
}
