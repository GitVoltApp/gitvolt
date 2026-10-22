import { describe, expect, it } from 'vitest';
import type { GhostBranch } from '../api/gen/GhostBranch';
import type { RowPayload } from '../api/gen/RowPayload';
import { graphLayout, nodeRadius } from './draw';
import { laneX } from './geometry';
import { dottedAt, foldMarkAt, foldStub, ghostAt, ghostPaths, withAlpha } from './ghosts';

const m = { rowH: 28, laneW: 22, padX: 11 };
const row = (lane: number, kind: RowPayload['kind'], extra: Partial<RowPayload> = {}): RowPayload => ({
  id: 'x', kind, lane, color: lane, segments: [], summary: '', bodyFirstLine: '', authorName: '', authorEmail: '', authorTime: 0, committerTime: 0, parents: [], mrRefs: [], wip: null, ...extra,
});
const ghost = (color: number): GhostBranch => ({ branch: `spike/${color}`, color, tip: 'c'.repeat(40), summary: 'Gate graph prototype', authorName: 'Rafe L.', authorTime: 0 });

describe('ghostPaths', () => {
  it('ghost stubs stay inside their row and fade', () => {
    const top = 56;
    const r = row(1, 'commit', { ghosts: [ghost(2), ghost(3), ghost(4)] });
    const paths = ghostPaths(r, top, m);
    expect(paths).toHaveLength(3);
    const x = laneX(1, m), yc = top + m.rowH / 2;
    const ends: number[] = [];
    for (const p of paths) {
      // From the node centre, one quadratic curve up and right.
      expect(p[0]).toEqual({ op: 'M', x, y: yc });
      expect(p).toHaveLength(2);
      const q = p[1];
      if (q.op !== 'Q') throw new Error('a curve');
      // Inside the row: never above its top edge (the stroke's half width included), never
      // below the node; and right of the node, within about two lanes and a half.
      for (const [px, py] of [[q.cx, q.cy], [q.x, q.y]]) {
        expect(py).toBeGreaterThanOrEqual(top + 1);
        expect(py).toBeLessThanOrEqual(yc);
        expect(px).toBeGreaterThan(x);
        expect(px - x).toBeLessThanOrEqual(2.5 * m.laneW);
      }
      expect(q.y).toBe(top + 1);
      ends.push(q.x);
    }
    // Apart, so each reads as its own line.
    expect(new Set(ends).size).toBe(3);
    // No ghosts, no paths.
    expect(ghostPaths(row(0, 'commit'), 0, m)).toEqual([]);
  });

  it('fades through the theme colour at an alpha', () => {
    expect(withAlpha('#0669f7', 0.7)).toBe('rgba(6,105,247,0.7)');
    expect(withAlpha('#fff', 0)).toBe('rgba(255,255,255,0)');
    expect(withAlpha('#0669f7cc', 0.5)).toBe('rgba(6,105,247,0.5)');
    expect(withAlpha('rgb(1, 2, 3)', 0)).toBe('rgba(1,2,3,0)');
  });
});

describe('hit zones', () => {
  const rows = [
    row(0, 'commit', { ghosts: [ghost(1), ghost(2)] }),
    row(0, 'collapsed', { collapsed: { key: 'branch:feature/payments', branch: 'feature/payments', count: 58, authors: 3 } }),
    row(0, 'merge', { fold: { key: 'fold:m', branch: 'feature/trade-ui', count: 6 } }),
    row(0, 'commit'),
    row(0, 'merge'),
  ];
  const o = { rows, scrollTop: 0, width: 200, metrics: m };
  const x0 = laneX(0, m);
  const yc = (i: number) => i * m.rowH + m.rowH / 2;

  it('ghostAt finds the row under the stubs only', () => {
    // Up and right of the node, inside the stubs' box.
    expect(ghostAt(o, x0 + m.laneW, yc(0) - 4)).toBe(0);
    expect(ghostAt(o, x0 + 1.9 * m.laneW, 2)).toBe(0);
    // Not the node, not left of it, not below its centre, not past the farthest end.
    expect(ghostAt(o, x0, yc(0))).toBeNull();
    expect(ghostAt(o, x0 - 4, 4)).toBeNull();
    expect(ghostAt(o, x0 + m.laneW, yc(0) + 5)).toBeNull();
    expect(ghostAt(o, x0 + 2.2 * m.laneW, 4)).toBeNull();
    // A row without ghosts, and the scroll applied.
    expect(ghostAt(o, x0 + m.laneW, yc(3) - 4)).toBeNull();
    expect(ghostAt({ ...o, scrollTop: m.rowH }, x0 + m.laneW, yc(0) - 4 - m.rowH)).toBe(0);
    // Packed into the collapse zone: no ghosts there to hover.
    expect(ghostAt({ ...o, rows: [{ ...rows[0], lane: 9 }], width: 120, clipped: true }, graphLayout(120, m, true).packedX + 10, 4)).toBeNull();
  });

  it('dottedAt and foldMarkAt hit their zones and nothing else', () => {
    // The ⋮ row's dotted lane: its lane ± half a lane, the row's whole height.
    expect(dottedAt(o, x0, yc(1))).toBe(1);
    expect(dottedAt(o, x0 + m.laneW / 2 - 0.5, m.rowH + 1)).toBe(1);
    expect(dottedAt(o, x0 + m.laneW / 2 + 0.5, yc(1))).toBeNull();
    expect(dottedAt(o, x0, yc(0))).toBeNull();
    expect(dottedAt(o, x0, yc(3))).toBeNull();
    expect(dottedAt({ ...o, scrollX: 5 }, x0 - 5, yc(1))).toBe(1);
    // The folded merge: its node, and its stub (which reaches into the row below).
    const r = nodeRadius(m);
    expect(foldMarkAt(o, x0, yc(2))).toBe(2);
    expect(foldMarkAt(o, x0 + r - 0.5, yc(2))).toBe(2);
    const s = foldStub(x0, yc(2), m);
    expect(foldMarkAt(o, (s.x0 + s.x1) / 2, (s.y0 + s.y1) / 2)).toBe(2);
    expect(foldMarkAt(o, s.x1 - 0.5, s.y1 - 0.5)).toBe(2);
    expect(s.y1).toBeGreaterThan(3 * m.rowH);
    // Not the row below's own node, a plain merge, the ⋮ row or empty space.
    expect(foldMarkAt(o, x0, yc(3))).toBeNull();
    expect(foldMarkAt(o, x0, yc(4))).toBeNull();
    expect(foldMarkAt(o, x0, yc(1))).toBeNull();
    expect(foldMarkAt(o, x0 + 3 * m.laneW, yc(2))).toBeNull();
    expect(dottedAt(o, x0, yc(2))).toBeNull();
    expect(ghostAt(o, x0 + m.laneW, yc(2) - 4)).toBeNull();
  });
});
