import { describe, expect, it } from 'vitest';
import type { GhostBranch } from '../api/gen/GhostBranch';
import type { RowPayload } from '../api/gen/RowPayload';
import { graphLayout, MARK_GAP, nodeRadius } from './draw';
import { laneX } from './geometry';
import { HALF_BOTTOM, HALF_FULL, HALF_TOP } from './segments';
import { dottedAt, foldMarkAt, foldText, hiddenText, isMarkEnd, stubAt } from './stubs';

const m = { rowH: 28, laneW: 22, padX: 11 };
const row = (lane: number, kind: RowPayload['kind'], extra: Partial<RowPayload> = {}): RowPayload => ({
  id: 'x', kind, lane, color: lane, segments: [], summary: '', bodyFirstLine: '', authorName: '', authorEmail: '', authorTime: 0, committerTime: 0, parents: [], mrRefs: [], wip: null, ...extra,
});
const ghost = (color: number): GhostBranch => ({ branch: `spike/${color}`, color, tip: 'c'.repeat(40), summary: 'Gate graph prototype', authorName: 'Rafe L.', authorTime: 0 });

describe('the marks', () => {
  it('a dotted half off its node is a tail end or a stub start; nothing else is', () => {
    const seg = (from: number, to: number, half: number, dotted = true) => ({ from, to, half, color: from, dashed: false, dotted });
    const r = row(0, 'commit');
    expect(isMarkEnd(seg(2, 2, HALF_TOP), r)).toBe(true);
    expect(isMarkEnd(seg(3, 3, HALF_BOTTOM), r)).toBe(true);
    // A ⋮ lane on its node, passing by, solid, or curving into the node.
    expect(isMarkEnd(seg(0, 0, HALF_BOTTOM), r)).toBe(false);
    expect(isMarkEnd(seg(2, 2, HALF_FULL), r)).toBe(false);
    expect(isMarkEnd(seg(2, 2, HALF_TOP, false), r)).toBe(false);
    expect(isMarkEnd(seg(2, 0, HALF_TOP), r)).toBe(false);
  });

  it('say how many commits folded and branches or tags hidden', () => {
    expect(foldText({ key: 'fold:m', branch: 'feature/trade-ui', count: 3 })).toEqual({ text: '⋮ 3 folded commits', label: '3 folded commits of feature/trade-ui' });
    expect(foldText({ key: 'fold:m', branch: 'fix/dock', count: 1 }).text).toBe('⋮ 1 folded commit');
    expect(hiddenText(Array.from({ length: 10 }, (_, i) => ghost(i)))).toEqual({ text: '⋮ 10 hidden', label: '10 hidden branches start here' });
    expect(hiddenText([ghost(2)]).label).toBe('1 hidden branch starts here');
    const tag = { ...ghost(2), branch: 'archive/old', tag: true };
    expect(hiddenText([tag]).label).toBe('1 hidden tag starts here');
    expect(hiddenText([tag, tag]).label).toBe('2 hidden tags start here');
    expect(hiddenText([ghost(2), tag])).toEqual({ text: '⋮ 2 hidden', label: '2 hidden branches and tags start here' });
  });
});

describe('hit zones', () => {
  const rows = [
    row(0, 'commit'),
    // Stub from lane 2 (its dotted start in row 0), into this commit.
    row(0, 'commit', { ghosts: [ghost(2), ghost(2)], stubLane: 2 }),
    row(0, 'collapsed', { collapsed: { key: 'branch:feature/payments', branch: 'feature/payments', count: 58, authors: 3 } }),
    // A fold with its tail in lane 1, ending in row 4.
    row(0, 'merge', { fold: { key: 'fold:m', branch: 'feature/trade-ui', count: 6, tailLane: 1 } }),
    row(0, 'commit'),
    row(0, 'merge'),
    // A fold on the last row: no tail.
    row(0, 'merge', { fold: { key: 'fold:n', branch: 'feature/docs', count: 2 } }),
  ];
  const o = { rows, scrollTop: 0, width: 200, metrics: m };
  const lx = (lane: number) => laneX(lane, m);
  const yc = (i: number) => i * m.rowH + m.rowH / 2;

  it('stubAt follows the stub down its lane from the row above, then along the centre line to the node', () => {
    // Its dotted start in the row above, the curve's vertical, its run to the node.
    expect(stubAt(o, lx(2), yc(0) + MARK_GAP + 2)).toBe(1);
    expect(stubAt(o, lx(2) + m.laneW / 2 - 0.5, m.rowH + 3)).toBe(1);
    expect(stubAt(o, lx(1), yc(1))).toBe(1);
    expect(stubAt(o, lx(1), yc(1) + m.rowH / 4)).toBe(1);
    // Not the node itself (its own tooltip and click), nor past the stub's lane, nor above the row
    // above's centre, nor the row above's node, nor below the curve.
    expect(stubAt(o, lx(0), yc(1))).toBeNull();
    expect(stubAt(o, lx(0) + nodeRadius(m) - 1, yc(1))).toBeNull();
    expect(stubAt(o, lx(3), yc(1))).toBeNull();
    expect(stubAt(o, lx(2), yc(0) - 2)).toBeNull();
    expect(stubAt(o, lx(0), yc(0))).toBeNull();
    expect(stubAt(o, lx(1), yc(1) + m.rowH / 4 + 1)).toBeNull();
    // The scroll applied, and a stub scrolled out of the lane area.
    expect(stubAt({ ...o, scrollTop: m.rowH }, lx(1), yc(1) - m.rowH)).toBe(1);
    expect(stubAt({ ...o, width: 90, clipped: true }, lx(2), yc(1) - 4)).toBeNull();
    // Packed into the collapse zone: nothing to hover.
    expect(stubAt({ ...o, rows: rows.map((r) => ({ ...r, lane: 9 })), width: 120, clipped: true }, graphLayout(120, m, true).packedX + 10, yc(1))).toBeNull();
  });

  it('foldMarkAt takes the node and the tail, into the row below; dottedAt the ⋮ lane', () => {
    const r = nodeRadius(m);
    expect(foldMarkAt(o, lx(0), yc(3))).toBe(3);
    expect(foldMarkAt(o, lx(0) + r - 0.5, yc(3))).toBe(3);
    // The tail's curve, its vertical in the merge's row, its dotted end in the row under it.
    expect(foldMarkAt(o, lx(1) - 4, yc(3) + 2)).toBe(3);
    expect(foldMarkAt(o, lx(1), yc(3) + m.rowH / 2 - 1)).toBe(3);
    expect(foldMarkAt(o, lx(1) + 3, 4 * m.rowH + 3)).toBe(3);
    // Not past its dotted end, nor the row below's own node, nor a plain merge, the ⋮ row or empty
    // space; the last row's fold is its node only.
    expect(foldMarkAt(o, lx(1), yc(4) - MARK_GAP + 1)).toBeNull();
    expect(foldMarkAt(o, lx(0), yc(4))).toBeNull();
    expect(foldMarkAt(o, lx(0), yc(5))).toBeNull();
    expect(foldMarkAt(o, lx(0), yc(2))).toBeNull();
    expect(foldMarkAt(o, lx(3), yc(3))).toBeNull();
    expect(foldMarkAt(o, lx(0), yc(6))).toBe(6);
    expect(foldMarkAt(o, lx(1), yc(6) + 4)).toBeNull();
    // The ⋮ row's dotted lane: its lane ± half a lane, the row's whole height.
    expect(dottedAt(o, lx(0), yc(2))).toBe(2);
    expect(dottedAt(o, lx(0) + m.laneW / 2 - 0.5, 2 * m.rowH + 1)).toBe(2);
    expect(dottedAt(o, lx(0) + m.laneW / 2 + 0.5, yc(2))).toBeNull();
    expect(dottedAt(o, lx(0), yc(3))).toBeNull();
    expect(dottedAt({ ...o, scrollX: 5 }, lx(0) - 5, yc(2))).toBe(2);
    // The marks don't claim each other's.
    expect(stubAt(o, lx(1) - 4, yc(3) + 2)).toBeNull();
    expect(foldMarkAt(o, lx(2), yc(0) + MARK_GAP + 2)).toBeNull();
  });
});
