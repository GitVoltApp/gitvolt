import { describe, expect, it } from 'vitest';
import { BAND_ALPHA, CONNECTOR_ALPHA, DOT_PATTERN, drawGraph, graphLayout, MARK_GAP, nodeAt, nodeRadius, PACKED_ALPHA, SELECTED_BAND_ALPHA, SHADE_ALPHA, SHADE_W, stashHalf, zoneWidth } from './draw';
import { laneX } from './geometry';
import type { RowPayload } from '../api/gen/RowPayload';
import { avatarLane } from '../avatars/color';
import { HALF_BOTTOM, HALF_TOP } from './segments';

function recorder() {
  const calls: string[] = [];
  // Every setLineDash pattern, as given (the calls list only says `object`).
  const dashes: number[][] = [];
  // Numeric args are kept exact (not rounded): the device-pixel snap produces values like
  // 11.5 or 17/1.5 that a naive Math.round would collapse into indistinguishable integers,
  // hiding the exact behavior the connector's crisp-pixel math depends on.
  // createLinearGradient hands back a gradient whose colour stops are recorded too.
  const gradient = { addColorStop: (...a: unknown[]) => { calls.push(`addColorStop(${a.join(',')})`); } };
  const ctx = new Proxy({} as Record<string, unknown>, {
    get: (_t, k: string) => (k in _t ? _t[k] : (...a: unknown[]) => { if (k === 'setLineDash') dashes.push(a[0] as number[]); calls.push(`${k}(${a.map((x) => (typeof x === 'number' ? x : typeof x)).join(',')})`); return k === 'createLinearGradient' ? gradient : undefined; }),
    set: (t, k: string, v) => { t[k] = v; calls.push(`${k}=${String(v)}`); return true; },
  });
  return { ctx: ctx as unknown as CanvasRenderingContext2D, calls, dashes };
}

const row = (lane: number, kind: RowPayload['kind'], segments: number[]): RowPayload => ({
  id: 'x', kind, lane, color: lane, segments, summary: '', bodyFirstLine: '', authorName: 'Ada Lovelace', authorEmail: '', authorTime: 0, committerTime: 0, parents: [], mrRefs: [], wip: null,
});

describe('drawGraph', () => {
  it('draws only visible rows, with lane colors and a node per row', () => {
    const { ctx, calls } = recorder();
    const fullLane1 = 1 | (1 << 10) | (2 << 20) | (1 << 22);
    const rows = [row(0, 'commit', [1 << 20]), row(1, 'merge', [fullLane1]), row(0, 'wip', [])];
    drawGraph(ctx, { rows, first: 1, last: 3, scrollTop: 22, width: 100, height: 44, metrics: { rowH: 22, laneW: 16, padX: 8 }, colors: ['#a', '#b'], nodeFill: '#000', labeledRows: new Set([1]), dpr: 1 });
    expect(calls.filter((c) => c.startsWith('arc(')).length).toBe(2);
    expect(calls).toContain('strokeStyle=#b');
    expect(calls.some((c) => c.startsWith('setLineDash('))).toBe(true);
  });

  it('a stash node is a dotted square with the stash (paper tray) icon inside, not the WIP ring', () => {
    const { ctx, calls } = recorder();
    const m = { rowH: 22, laneW: 16, padX: 8 };
    drawGraph(ctx, { rows: [row(0, 'stash', [])], first: 0, last: 1, scrollTop: 0, width: 100, height: 22, metrics: m, colors: ['#a'], nodeFill: '#000', labeledRows: new Set(), dpr: 1 });
    const s = stashHalf(m);
    const at = calls.indexOf(`rect(${16 - s},${11 - s},${2 * s},${2 * s})`);
    expect(at).toBeGreaterThan(-1);
    expect(calls.filter((c) => c.startsWith('arc('))).toEqual([]); // no ring
    // Dotted, with butt caps and a dash that divides the perimeter evenly.
    const dash = calls.slice(0, at).findLast((c) => c.startsWith('setLineDash('));
    expect(dash).toBe('setLineDash(object)');
    expect(calls.slice(0, at)).toContain('lineCap=butt');
    // The icon after the square, in the lane colour: the lid's rounded corners, the tray and the slot.
    const icon = calls.slice(at);
    expect(icon.filter((c) => c.startsWith('arcTo(')).length).toBe(6);
    expect(icon.filter((c) => c.startsWith('moveTo(')).length).toBe(3);
    expect(icon.filter((c) => c === 'stroke()').length).toBe(2);
  });

  it('draws a translucent band on every visible row, not only labeled ones', () => {
    const { ctx, calls } = recorder();
    // Three visible, unlabeled commit rows in different lanes.
    const rows = [row(0, 'commit', []), row(1, 'commit', []), row(2, 'commit', [])];
    drawGraph(ctx, { rows, first: 0, last: 3, scrollTop: 0, width: 100, height: 66, metrics: { rowH: 22, laneW: 16, padX: 8 }, colors: ['#a', '#b', '#c'], nodeFill: '#000', labeledRows: new Set(), dpr: 1 });
    // Two fillRect per visible row: the lane-color band and the bright lane-colored rail edge.
    // The graph fits (not clipped), so there's no overflow strip (F2).
    expect(calls.filter((c) => c.startsWith('fillRect(')).length).toBe(6);
    // No row is labeled, so no connector line should be drawn.
    expect(calls.filter((c) => c.startsWith('moveTo(0,')).length).toBe(0);
    // Bands come before any node arcs.
    expect(calls.findIndex((c) => c.startsWith('fillRect('))).toBeLessThan(calls.findIndex((c) => c.startsWith('arc(')));
  });

  it('while the lanes fit, nothing is packed, clipped, shaded or dimmed (F2, R11)', () => {
    const { ctx, calls } = recorder();
    const rows = [row(0, 'commit', [1 << 20]), row(4, 'commit', [])];
    drawGraph(ctx, { rows, first: 0, last: 2, scrollTop: 0, width: 100, height: 44, metrics: { rowH: 22, laneW: 16, padX: 8 }, colors: ['#a'], nodeFill: '#000', labeledRows: new Set(), dpr: 1, clipped: false });
    expect(calls.some((c) => c.startsWith('clip('))).toBe(false);
    expect(calls.some((c) => c.startsWith('createLinearGradient('))).toBe(false);
    expect(calls).not.toContain(`globalAlpha=${PACKED_ALPHA}`);
    // Lane 4 (x = 80) is drawn in its lane; its band runs to the edge.
    expect(calls.filter((c) => c.startsWith('arc(80,'))).toHaveLength(1);
    expect(calls).toContain('fillRect(80,24,20,18)');
  });

  it('draws a solid, full-alpha 2px rail in the lane color at the right edge, last (after packed nodes and the shade)', () => {
    const { ctx, calls } = recorder();
    const rows = [row(0, 'commit', []), row(5, 'commit', [])];
    drawGraph(ctx, { rows, first: 0, last: 2, scrollTop: 0, width: 100, height: 44, metrics: { rowH: 22, laneW: 16, padX: 8 }, colors: ['#a', '#b'], nodeFill: '#000', labeledRows: new Set(), dpr: 1, clipped: true });
    const lastNode = calls.findLastIndex((c) => c.startsWith('arc('));
    for (const [top, c] of [[0, '#a'], [22, '#b']] as const) {
      const rail = calls.indexOf(`fillRect(98,${top + 2},2,18)`);
      expect(rail).toBeGreaterThan(lastNode);
      // The fill state in effect for the rail: the lane color, at full alpha.
      const before = calls.slice(0, rail);
      expect(before.findLast((x) => x.startsWith('fillStyle='))).toBe(`fillStyle=${c}`);
      expect(before.findLast((x) => x.startsWith('globalAlpha='))).toBe('globalAlpha=1');
    }
  });

  it('paints initials and the collapse shade in the theme colours it is given', () => {
    const { ctx, calls } = recorder();
    const rows = [row(0, 'commit', []), row(5, 'commit', [])];
    drawGraph(ctx, { rows, first: 0, last: 2, scrollTop: 0, width: 100, height: 44, metrics: { rowH: 22, laneW: 16, padX: 8 }, colors: ['#a', '#b'], nodeFill: '#fff', nodeText: '#0b0d10', stripColor: 'rgba(0, 0, 0, 0.15)', labeledRows: new Set(), dpr: 1, clipped: true });
    const text = calls.findIndex((c) => c.startsWith('fillText('));
    expect(calls.slice(0, text).findLast((x) => x.startsWith('fillStyle='))).toBe('fillStyle=#0b0d10');
    expect(calls).toContain('addColorStop(1,rgba(0, 0, 0, 0.15))');
    // Without them: white initials and the black shade, as before themes.
    const plain = recorder();
    drawGraph(plain.ctx, { rows, first: 0, last: 2, scrollTop: 0, width: 100, height: 44, metrics: { rowH: 22, laneW: 16, padX: 8 }, colors: ['#a', '#b'], nodeFill: '#000', labeledRows: new Set(), dpr: 1, clipped: true });
    const t2 = plain.calls.findIndex((c) => c.startsWith('fillText('));
    expect(plain.calls.slice(0, t2).findLast((x) => x.startsWith('fillStyle='))).toBe('fillStyle=#fff');
    expect(plain.calls).toContain(`addColorStop(1,rgba(0,0,0,${SHADE_ALPHA}))`);
  });

  it('keeps the rail flush with the canvas edge and a whole number of device pixels wide at DPR 1.5', () => {
    const { ctx, calls } = recorder();
    // width 101 CSS px -> backing store round(151.5) = 152 device px; the rail is round(2 * 1.5) =
    // 3 device px, so it spans device px 149..152, i.e. CSS x 149/1.5, width 3/1.5 = 2.
    drawGraph(ctx, { rows: [row(0, 'commit', [])], first: 0, last: 1, scrollTop: 0, width: 101, height: 22, metrics: { rowH: 22, laneW: 16, padX: 8 }, colors: ['#a'], nodeFill: '#000', labeledRows: new Set(), dpr: 1.5 });
    expect(calls).toContain(`fillRect(${149 / 1.5},2,2,18)`);
  });

  it('insets the row band and the rail by the density\'s bandInset (H1: standard\'s bands are chip-high, 22 of 28 px)', () => {
    const { ctx, calls } = recorder();
    const rows = [row(0, 'commit', []), row(1, 'commit', [])];
    drawGraph(ctx, { rows, first: 0, last: 2, scrollTop: 0, width: 100, height: 56, metrics: { rowH: 28, laneW: 22, padX: 11, bandInset: 3 }, colors: ['#a', '#b'], nodeFill: '#000', labeledRows: new Set(), dpr: 1 });
    // Row 1: top 28, band from its node (laneX(1) = 11 + 22 + 11 = 44) to the edge.
    expect(calls).toContain('fillRect(44,31,56,22)');
    expect(calls).toContain('fillRect(98,31,2,22)');
  });

  it('the selected row\'s band is brighter (H14); the others keep BAND_ALPHA', () => {
    expect(SELECTED_BAND_ALPHA).toBeGreaterThan(2 * BAND_ALPHA);
    const { ctx, calls } = recorder();
    const rows = [row(0, 'commit', []), row(1, 'commit', []), row(0, 'commit', [])];
    drawGraph(ctx, { rows, first: 0, last: 3, scrollTop: 0, width: 100, height: 75, metrics: { rowH: 25, laneW: 16, padX: 8 }, colors: ['#a', '#b'], nodeFill: '#000', labeledRows: new Set(), dpr: 1, selected: 1 });
    const alphaFor = (rect: string) => calls.slice(0, calls.indexOf(rect)).findLast((c) => c.startsWith('globalAlpha='));
    expect(alphaFor('fillRect(16,2,84,21)')).toBe(`globalAlpha=${BAND_ALPHA}`);
    expect(alphaFor('fillRect(32,27,68,21)')).toBe(`globalAlpha=${SELECTED_BAND_ALPHA}`);
    expect(alphaFor('fillRect(16,52,84,21)')).toBe(`globalAlpha=${BAND_ALPHA}`);
  });

  it('every selected row gets the selected band (K15, K27)', () => {
    const { ctx, calls } = recorder();
    const rows = [row(0, 'commit', []), row(1, 'commit', []), row(0, 'commit', [])];
    drawGraph(ctx, { rows, first: 0, last: 3, scrollTop: 0, width: 100, height: 75, metrics: { rowH: 25, laneW: 16, padX: 8 }, colors: ['#a', '#b'], nodeFill: '#000', labeledRows: new Set(), dpr: 1, selected: 1, alsoSelected: new Set([0, 2]) });
    const alphaFor = (rect: string) => calls.slice(0, calls.indexOf(rect)).findLast((c) => c.startsWith('globalAlpha='));
    expect(alphaFor('fillRect(16,2,84,21)')).toBe(`globalAlpha=${SELECTED_BAND_ALPHA}`);
    expect(alphaFor('fillRect(32,27,68,21)')).toBe(`globalAlpha=${SELECTED_BAND_ALPHA}`);
    expect(alphaFor('fillRect(16,52,84,21)')).toBe(`globalAlpha=${SELECTED_BAND_ALPHA}`);
  });

  it('draws the label connector only on labeled rows, from x=0 to the node, snapped to a device pixel', () => {
    const { ctx, calls } = recorder();
    const rows = [row(0, 'commit', []), row(1, 'commit', [])];
    drawGraph(ctx, { rows, first: 0, last: 2, scrollTop: 0, width: 100, height: 44, metrics: { rowH: 22, laneW: 16, padX: 8 }, colors: ['#a', '#b'], nodeFill: '#000', labeledRows: new Set([1]), dpr: 1 });
    const connectors = calls.filter((c) => c.startsWith('moveTo(0,'));
    expect(connectors.length).toBe(1);
    // Row 1's center: top(22) + rowH/2(11) = 33 CSS px = 33 device px at dpr 1 (odd device
    // width), which snaps to the half-pixel boundary below it: floor(33)+0.5 = 33.5.
    expect(connectors[0]).toBe('moveTo(0,33.5)');
    expect(calls).toContain('lineTo(32,33.5)');
    expect(calls).toContain('lineCap=butt');
    // Reset before the segment-line pass, so the connector's cap never leaks onto graph lines.
    expect(calls).toContain('lineCap=round');
  });

  it('snaps the connector to a whole device pixel, at the correct stroke width, for DPR 1/1.5/2', () => {
    // Row 0's center is top(0) + rowH/2(11) = 11 CSS px; x = laneX(0, m) = 16.
    const base = { rows: [row(0, 'commit', [])], first: 0, last: 1, scrollTop: 0, width: 100, height: 22, metrics: { rowH: 22, laneW: 16, padX: 8 }, colors: ['#a'], nodeFill: '#000', labeledRows: new Set([0]) };

    // DPR 1: a 1-device-pixel-wide stroke (odd) centers on a half CSS pixel:
    // floor(11) + 0.5 = 11.5.
    const { ctx: ctx1, calls: calls1 } = recorder();
    drawGraph(ctx1, { ...base, dpr: 1 });
    expect(calls1).toContain('lineWidth=1');
    expect(calls1).toContain('moveTo(0,11.5)');
    expect(calls1).toContain('lineTo(16,11.5)');

    // DPR 2: still a 1-device-pixel-wide stroke (its CSS width, lineWidth, halves to stay
    // 1 device px), but 11 CSS px * 2 = 22 device px is even, so it centers on a whole pixel.
    const { ctx: ctx2, calls: calls2 } = recorder();
    drawGraph(ctx2, { ...base, dpr: 2 });
    expect(calls2).toContain('lineWidth=1');
    expect(calls2).toContain('moveTo(0,11)');
    expect(calls2).toContain('lineTo(16,11)');

    // DPR 1.5: rounds to a 2-device-pixel-wide stroke (even), so it also centers on a whole
    // device pixel: 11 * 1.5 = 16.5 device px -> round(16.5) = 17 -> 17/1.5 CSS px.
    const { ctx: ctx3, calls: calls3 } = recorder();
    drawGraph(ctx3, { ...base, dpr: 1.5 });
    expect(calls3).toContain(`lineWidth=${2 / 1.5}`);
    expect(calls3).toContain(`moveTo(0,${17 / 1.5})`);
    expect(calls3).toContain(`lineTo(16,${17 / 1.5})`);
  });

  it('strokes the connector in the lane colour at 25% alpha, then restores full alpha (F8)', () => {
    const { ctx, calls } = recorder();
    const rows = [row(0, 'commit', []), row(1, 'commit', [1 << 20])];
    drawGraph(ctx, { rows, first: 0, last: 2, scrollTop: 0, width: 100, height: 44, metrics: { rowH: 22, laneW: 16, padX: 8 }, colors: ['#a', '#b'], nodeFill: '#000', labeledRows: new Set([1]), dpr: 1 });
    expect(CONNECTOR_ALPHA).toBe(0.25);
    const stroke = calls.indexOf('stroke()', calls.indexOf('moveTo(0,33.5)'));
    const before = calls.slice(0, stroke);
    expect(before.findLast((c) => c.startsWith('globalAlpha='))).toBe('globalAlpha=0.25');
    expect(before.findLast((c) => c.startsWith('strokeStyle='))).toBe('strokeStyle=#b');
    // Everything after it (the next band, graph lines, nodes) is back at full alpha.
    const after = calls.slice(stroke);
    expect(after.find((c) => c.startsWith('globalAlpha='))).toBe('globalAlpha=1');
    const firstLine = calls.findIndex((c, i) => i > stroke && c === 'lineCap=round');
    expect(calls.slice(0, firstLine).findLast((c) => c.startsWith('globalAlpha='))).toBe('globalAlpha=1');
  });

  it("the checked-out branch's connector (headRow) is the graph line's width and the full lane colour (J21)", () => {
    const rows = [row(0, 'commit', []), row(1, 'commit', [1 << 20])];
    for (const dpr of [1, 1.5, 2]) {
      const { ctx, calls } = recorder();
      drawGraph(ctx, { rows, first: 0, last: 2, scrollTop: 0, width: 100, height: 44, metrics: { rowH: 22, laneW: 16, padX: 8 }, colors: ['#a', '#b'], nodeFill: '#000', labeledRows: new Set([0, 1]), dpr, headRow: 1 });
      const moves = calls.map((c, i) => [c, i] as const).filter(([c]) => c.startsWith('moveTo(0,'));
      expect(moves).toHaveLength(2);
      const style = (at: number, prop: string) => calls.slice(0, calls.indexOf('stroke()', at)).findLast((c) => c.startsWith(`${prop}=`));
      // Row 0: the quiet 1-device-pixel line at 25%.
      expect(style(moves[0][1], 'globalAlpha'), `dpr ${dpr}`).toBe('globalAlpha=0.25');
      expect(style(moves[0][1], 'lineWidth')).toBe(`lineWidth=${Math.max(1, Math.round(dpr)) / dpr}`);
      // HEAD's row: 2 CSS px (a whole number of device pixels, centred on a device-pixel line), at full alpha.
      expect(style(moves[1][1], 'globalAlpha'), `dpr ${dpr}`).toBe('globalAlpha=1');
      expect(style(moves[1][1], 'lineWidth')).toBe(`lineWidth=${Math.round(2 * dpr) / dpr}`);
      const y = Number(/moveTo\(0,(.*)\)/.exec(moves[1][0])![1]);
      expect((y * dpr) % 1).toBe(Math.round(2 * dpr) % 2 ? 0.5 : 0);
    }
  });

  it('places the connector against the device-snapped scroll, so it stays on the DOM half\'s device rows (K57)', () => {
    const rows = [row(0, 'commit', [])];
    const at = (scrollTop: number, dpr: number) => {
      const { ctx, calls } = recorder();
      drawGraph(ctx, { rows, first: 0, last: 1, scrollTop, width: 100, height: 22, metrics: { rowH: 22, laneW: 16, padX: 8 }, colors: ['#a'], nodeFill: '#000', labeledRows: new Set([0]), dpr });
      return calls.find((c) => c.startsWith('moveTo(0,'));
    };
    // Float noise in the scroll (0.3 at dpr 1 is no whole device px): the snapped scroll is 0,
    // the line's top edge device row 11 (centre 11 - 0.5, rounded), its centre a half pixel.
    expect(at(0.3, 1)).toBe('moveTo(0,11.5)');
    // At 125% Chromium scrolls by device px: 0.8 CSS px is one. Content: top edge round(11 *
    // 1.25 - 0.5) = 13 device px; less the one scrolled: 12, centre 12.5 device px.
    expect(at(0.8, 1.25)).toBe(`moveTo(0,${12.5 / 1.25})`);
  });
  it('draws a dashed lane as one run with whole-device-px dashes phased by its content y, and the WIP ring evenly dotted (K50)', () => {
    const { ctx, calls } = recorder();
    const dashedBottom = 0 | (0 << 10) | (1 << 20) | (1 << 26);
    const dashedFull = 0 | (0 << 10) | (2 << 20) | (1 << 26);
    const dashedTop = 0 | (0 << 10) | (0 << 20) | (1 << 26);
    const rows = [row(0, 'wip', [dashedBottom]), row(1, 'commit', [dashedFull]), row(1, 'commit', [dashedFull]), row(0, 'commit', [dashedTop])];
    const dpr = 1.25;
    drawGraph(ctx, { rows, first: 0, last: 4, scrollTop: 0.8, width: 100, height: 112, metrics: { rowH: 28, laneW: 16, padX: 8 }, colors: ['#a', '#b'], nodeFill: '#000', labeledRows: new Set(), dpr });
    // round(3 * 1.25) = 4 device px: 3.2 CSS px dash and gap.
    const dash = 4 / dpr;
    const dashCalls = calls.filter((c) => c === `setLineDash(object)`);
    expect(dashCalls.length).toBeGreaterThan(0);
    // One run: from the WIP node's centre (14 - 0.8) down to the HEAD's (3 * 28 + 14 - 0.8).
    const strokesAfterDash = calls.map((c, i) => [c, i] as const).filter(([c]) => c.startsWith('lineDashOffset=') && c !== 'lineDashOffset=0');
    expect(strokesAfterDash).toHaveLength(1);
    const i = strokesAfterDash[0][1];
    expect(calls.slice(i, i + 4)).toEqual([`lineDashOffset=${(14 % (2 * dash) + 2 * dash) % (2 * dash)}`, 'beginPath()', `moveTo(16,${14 - 0.8})`, `lineTo(16,${98 - 0.8})`]);
    expect(calls.slice(0, i).reverse().find((c) => c.startsWith('lineCap='))).toBe('lineCap=butt');
    // The ring: butt caps too, and a whole number of dash+gap pairs.
    const ringCap = calls.lastIndexOf('lineCap=butt');
    expect(ringCap).toBeGreaterThan(i);
  });

  it('a collapsed row draws no node and a dotted lane', () => {
    const { ctx, calls, dashes } = recorder();
    // tip → ⋮ → (a commit in lane 1 beside the dotted lane) → oldest. The layout marks the ⋮ row's
    // line down dotted; its line in from the tip comes solid and is dotted too.
    const bottom = 1 << 20, full = 2 << 20, dotted = 1 << 27;
    const rows = [row(0, 'commit', [bottom]), row(0, 'collapsed', [0, bottom | dotted]), row(1, 'commit', [full | dotted]), row(0, 'commit', [dotted])];
    const m = { rowH: 26, laneW: 16, padX: 8 };
    drawGraph(ctx, { rows, first: 0, last: 4, scrollTop: 0, width: 100, height: 104, metrics: m, colors: ['#a', '#b'], nodeFill: '#000', labeledRows: new Set(), dpr: 1 });
    // Nodes at rows 0, 2 and 3; none at the ⋮ row's centre (16, 39).
    expect(calls.filter((c) => c.startsWith('arc('))).toHaveLength(3);
    expect(calls.some((c) => c.startsWith('arc(16,39,'))).toBe(false);
    // One dotted run, round dots, from the ⋮ row's top edge to the oldest commit's centre, phased
    // by its content y (26 % 4).
    expect(DOT_PATTERN).toEqual([0, 4]);
    expect(dashes).toContainEqual(DOT_PATTERN);
    const at = calls.indexOf('moveTo(16,26)');
    expect(calls.slice(at - 2, at + 2)).toEqual(['lineDashOffset=2', 'beginPath()', 'moveTo(16,26)', 'lineTo(16,91)']);
    expect(calls.slice(0, at).findLast((c) => c.startsWith('lineCap='))).toBe('lineCap=round');
    // The tip's own line down to the ⋮ row stays solid.
    const solid = calls.indexOf('moveTo(16,13)');
    expect(solid).toBeGreaterThan(-1);
    expect(solid).toBeLessThan(at);
    expect(calls.slice(0, solid).filter((c) => c.startsWith('setLineDash('))).toHaveLength(0);
  });

  // Branch presets' marks: lines the layout placed, drawn with the ordinary segments (stubs.ts).
  const seg = (from: number, to: number, half: number, color: number, dotted = false) => from | (to << 10) | (half << 20) | (color << 22) | (Number(dotted) << 27);
  const mm = { rowH: 26, laneW: 16, padX: 8 };
  const marks = (rows: RowPayload[], foldOpen?: { row: number; lane: number }) => {
    const r = recorder();
    drawGraph(r.ctx, { rows, first: 0, last: rows.length, scrollTop: 0, width: 100, height: 26 * rows.length, metrics: mm, colors: ['#102030', '#0669f7', '#22aa44'], nodeFill: '#000', labeledRows: new Set(), dpr: 1, foldOpen });
    return r;
  };
  /** A folded merge (lane 0) with its tail into lane 1, and the commit under it, where it ends. */
  const folded = (): RowPayload[] => [
    { ...row(0, 'merge', [seg(0, 0, HALF_BOTTOM, 0), seg(0, 1, HALF_BOTTOM, 1)]), fold: { key: 'fold:x', branch: 'feature/trade-ui', count: 6, tailLane: 1 } },
    row(0, 'commit', [seg(0, 0, HALF_TOP, 0), seg(1, 1, HALF_TOP, 1, true), seg(0, 0, HALF_BOTTOM, 0)]),
  ];

  it('a folded merge is a merge node; its tail is the merge curve, then dots that stop short of the centre line', () => {
    const { calls, dashes } = marks(folded());
    const r = nodeRadius(mm);
    expect(calls).toContain(`arc(16,13,${26 * 0.18},0,${Math.PI * 2})`);
    expect(calls.filter((c) => c.startsWith('arc(16,13,'))).toHaveLength(1);
    expect(calls).not.toContain(`arc(16,13,${r * 0.4},0,${Math.PI * 2})`);
    // The curve: solid, in lane 1's colour, along the centre line and down out of the row.
    const x1 = laneX(1, mm);
    const down = calls.indexOf(`lineTo(${x1},26)`);
    expect(down).toBeGreaterThan(-1);
    expect(calls.slice(0, down).findLast((c) => c.startsWith('strokeStyle='))).toBe('strokeStyle=#0669f7');
    expect(calls.slice(0, down).filter((c) => c.startsWith('setLineDash('))).toHaveLength(0);
    // The end: the ⋮ row's dots from the row's top, MARK_GAP short of its centre (39).
    expect(MARK_GAP).toBe(4);
    const end = calls.indexOf(`moveTo(${x1},26)`);
    expect(calls.slice(end, end + 2)).toEqual([`moveTo(${x1},26)`, `lineTo(${x1},${39 - MARK_GAP})`]);
    expect(dashes).toContainEqual(DOT_PATTERN);
    expect(calls.slice(0, end).findLast((c) => c.startsWith('strokeStyle='))).toBe('strokeStyle=#0669f7');
    // Static: no gradient, nothing faded.
    expect(calls.filter((c) => c.startsWith('createLinearGradient('))).toEqual([]);
  });

  it('a hidden-branch stub starts in dots below the row above\'s centre line, then curves into its commit', () => {
    const { calls } = marks([
      row(0, 'commit', [seg(0, 0, HALF_BOTTOM, 0), seg(2, 2, HALF_BOTTOM, 2, true)]),
      { ...row(0, 'commit', [seg(0, 0, HALF_TOP, 0), seg(2, 0, HALF_TOP, 2)]), ghosts: [{ branch: 'spike/a', color: 2, tip: 'c'.repeat(40), summary: '', authorName: '', authorTime: 0 }], stubLane: 2 },
    ]);
    const x2 = laneX(2, mm);
    const start = calls.indexOf(`moveTo(${x2},${13 + MARK_GAP})`);
    expect(calls.slice(start, start + 2)).toEqual([`moveTo(${x2},${13 + MARK_GAP})`, `lineTo(${x2},26)`]);
    expect(calls.slice(0, start).findLast((c) => c.startsWith('setLineDash('))).toBe('setLineDash(object)');
    // The fork curve: solid, in the stub lane's colour, along row 1's centre line into the node.
    const curve = calls.indexOf(`moveTo(${x2},26)`);
    expect(curve).toBeGreaterThan(-1);
    expect(calls.slice(curve, curve + 4).at(-1)).toBe('lineTo(16,39)');
    expect(calls.slice(0, curve).findLast((c) => c.startsWith('strokeStyle='))).toBe('strokeStyle=#22aa44');
    expect(calls.filter((c) => c.startsWith('createLinearGradient('))).toEqual([]);
  });

  it('an open fold\'s panel continues its tail; a fold without one draws the curve into the panel\'s lane', () => {
    const x1 = laneX(1, mm);
    // Into the tail's own lane: no second curve over it.
    expect(marks(folded(), { row: 0, lane: 1 }).calls.filter((c) => c === `lineTo(${x1},26)`)).toHaveLength(1);
    // The window's last row: no tail, so the curve is drawn for the panel, in its lane's colour.
    const last: RowPayload[] = [{ ...row(0, 'merge', [seg(0, 0, HALF_BOTTOM, 0)]), fold: { key: 'fold:x', branch: 'feature/trade-ui', count: 6 } }];
    const { calls } = marks(last, { row: 0, lane: 2 });
    const x2 = laneX(2, mm);
    const end = calls.indexOf(`lineTo(${x2},26)`);
    expect(end).toBeGreaterThan(-1);
    expect(calls.slice(0, end)).toContain('moveTo(16,13)');
    expect(calls.slice(0, end).findLast((c) => c.startsWith('strokeStyle='))).toBe('strokeStyle=#22aa44');
    // Closed: nothing but its own line.
    expect(marks(last).calls.some((c) => c === `lineTo(${x2},26)`)).toBe(false);
  });

  it('draws a loaded avatar bitmap clipped to the node instead of initials', () => {
    const { ctx, calls } = recorder();
    const bitmap = {} as ImageBitmap;
    const rows = [{ ...row(0, 'commit', []), authorEmail: 'ada@example.com' }, { ...row(0, 'commit', []), authorEmail: 'nobody@example.com' }];
    const asked: string[] = [];
    drawGraph(ctx, { rows, first: 0, last: 2, scrollTop: 0, width: 100, height: 50, metrics: { rowH: 25, laneW: 16, padX: 8 }, colors: ['#a'], nodeFill: '#000', labeledRows: new Set(), dpr: 1, avatar: (e) => { asked.push(e); return e === 'ada@example.com' ? bitmap : null; } });
    expect(asked).toEqual(['ada@example.com', 'nobody@example.com']);
    const clip = calls.indexOf('clip()');
    expect(clip).toBeGreaterThan(-1);
    expect(calls.slice(clip).findIndex((c) => c.startsWith('drawImage('))).toBeGreaterThan(0);
    expect(calls.filter((c) => c.startsWith('drawImage(')).length).toBe(1);
    expect(calls).toContain('save()');
    expect(calls).toContain('restore()');
    // Downscaled smoothly (the decoded bitmap is larger than the node).
    const smooth = calls.indexOf('imageSmoothingQuality=high');
    expect(smooth).toBeGreaterThan(-1);
    expect(smooth).toBeLessThan(calls.findIndex((c) => c.startsWith('drawImage(')));
    // The second row has no avatar: it keeps its initials.
    expect(calls.filter((c) => c.startsWith('fillText(')).length).toBe(1);
  });

  it('puts a picture on the avatar backdrop, so a transparent one in dark ink stays visible', () => {
    const { ctx, calls } = recorder();
    const rows = [{ ...row(0, 'commit', []), authorEmail: 'ada@example.com' }];
    drawGraph(ctx, { rows, first: 0, last: 1, scrollTop: 0, width: 100, height: 25, metrics: { rowH: 25, laneW: 16, padX: 8 }, colors: ['#a'], nodeFill: '#000', avatarBackdrop: '#e6e7ea', labeledRows: new Set(), dpr: 1, avatar: () => ({}) as ImageBitmap });
    const image = calls.findIndex((c) => c.startsWith('drawImage('));
    expect(calls.slice(0, image).findLast((c) => c.startsWith('fillStyle='))).toBe('fillStyle=#e6e7ea');
  });

  it("draws a commit without a picture as the author's initials avatar: the <Avatar> colour and text", () => {
    const colors = ['#c0', '#c1', '#c2', '#c3', '#c4', '#c5'];
    const laneText = ['#t0', '#t1', '#t2', '#t3', '#t4', '#t5'];
    const people = [['Ada Lovelace', 'ada@example.com'], ['Grace Hopper', 'GRACE@example.com '], ['No Email', '']] as const;
    for (const [name, email] of people) {
      const { ctx, calls } = recorder();
      const rows = [{ ...row(0, 'commit', []), authorName: name, authorEmail: email }];
      drawGraph(ctx, { rows, first: 0, last: 1, scrollTop: 0, width: 100, height: 25, metrics: { rowH: 25, laneW: 16, padX: 8 }, colors, laneText, nodeFill: '#000', labeledRows: new Set(), dpr: 1 });
      const lane = avatarLane(name, email, colors.length);
      // The node's disc: filled in the person's colour, then ringed in the lane's (colour 0).
      const disc = calls.findLastIndex((c) => c.startsWith('arc('));
      expect(calls.slice(disc).find((c) => c.startsWith('fillStyle=')), name).toBe(`fillStyle=${colors[lane]}`);
      expect(calls.slice(disc).find((c) => c.startsWith('strokeStyle=')), name).toBe('strokeStyle=#c0');
      const text = calls.findIndex((c) => c.startsWith('fillText('));
      expect(calls.slice(0, text).findLast((c) => c.startsWith('fillStyle=')), name).toBe(`fillStyle=${laneText[lane]}`);
    }
  });

  it('keeps the node fill behind a picture and inside a WIP node', () => {
    for (const [kind, avatar] of [['commit', () => ({}) as ImageBitmap], ['wip', undefined]] as const) {
      const { ctx, calls } = recorder();
      drawGraph(ctx, { rows: [row(0, kind, [])], first: 0, last: 1, scrollTop: 0, width: 100, height: 25, metrics: { rowH: 25, laneW: 16, padX: 8 }, colors: ['#c0', '#c1'], laneText: ['#t0', '#t1'], nodeFill: '#nf', labeledRows: new Set(), dpr: 1, avatar });
      // The node's disc is the last fill() (the picture is drawn, the rail fillRect'ed).
      const disc = calls.lastIndexOf('fill()');
      expect(calls.slice(0, disc).findLast((c) => c.startsWith('fillStyle=')), kind).toBe('fillStyle=#nf');
      expect(calls.some((c) => c.startsWith('fillText(')), kind).toBe(false);
    }
  });
});

describe('drawGraph: the collapse zone, packed nodes and the minimum-width strip (F11, R11)', () => {
  const m = { rowH: 25, laneW: 16, padX: 8 };
  // r = 9; the zone = 18 + 2 * 3 + 2 (rail) = 26 px; at width 80 the lane area is 54 px, and the
  // packed column's centre sits mid-zone left of the rail: 54 + (26 - 2) / 2 = 66.
  const base = { first: 0, scrollTop: 0, metrics: m, colors: ['#a', '#b', '#c', '#d', '#e', '#f'], nodeFill: '#000', dpr: 1, clipped: true };
  const alphaAt = (calls: string[], i: number) => calls.slice(0, i).findLast((c) => c.startsWith('globalAlpha='));
  const TAU = Math.PI * 2;

  it('sizes the zone to fit a node plus a gap either side and the rail, per density', () => {
    expect(nodeRadius(m)).toBe(9);
    expect(zoneWidth(m)).toBe(26);
    expect(zoneWidth({ rowH: 28, laneW: 22, padX: 11 })).toBe(21 + 8);
    expect(graphLayout(80, m, false)).toEqual({ zone: 0, area: 80, packedX: 80, strip: false });
    expect(graphLayout(80, m, true)).toEqual({ zone: 26, area: 54, packedX: 66, strip: false });
    // Lane 0 needs 16 + 9 = 25 px: below that there are no lanes at all, only the strip.
    expect(graphLayout(26 + 25, m, true).strip).toBe(false);
    expect(graphLayout(26 + 24, m, true).strip).toBe(true);
    expect(graphLayout(26 + 24, m, false).strip).toBe(false);
  });

  it('packs every node whose lane runs past the lane area into ONE column in the zone, dimmed', () => {
    const { ctx, calls } = recorder();
    const rows = [row(0, 'commit', []), row(3, 'commit', []), row(5, 'merge', []), row(1, 'commit', [])];
    drawGraph(ctx, { ...base, rows, last: 4, width: 80, height: 100, labeledRows: new Set() });
    const arcs = calls.map((c, i) => [c, i] as const).filter(([c]) => c.startsWith('arc('));
    // Lanes 0 and 1 are inside the 54 px lane area (x 16, 32); lanes 3 (64) and 5 are packed.
    expect(arcs.map(([c]) => c.split(',')[0])).toEqual(['arc(16', 'arc(32', 'arc(66', 'arc(66']);
    for (const [c, i] of arcs) expect(alphaAt(calls, i), c).toBe(c.startsWith('arc(66') ? `globalAlpha=${PACKED_ALPHA}` : 'globalAlpha=1');
    // A packed merge is a node like the rest of the column (the commit's radius), not a dot.
    expect(arcs[3][0]).toBe(`arc(66,62.5,9,0,${TAU})`);
    // Packed rows' bands fill the zone only; a lane row's runs from its node to the edge.
    expect(calls).toContain('fillRect(54,27,26,21)');
    expect(calls).toContain('fillRect(32,77,48,21)');
  });

  it('clips the lines to the lane area: no line enters the zone or joins a packed node', () => {
    const { ctx, calls } = recorder();
    const toLane5 = 0 | (5 << 10) | (1 << 20);
    drawGraph(ctx, { ...base, rows: [row(0, 'commit', [toLane5]), row(5, 'commit', [])], last: 2, width: 80, height: 50, labeledRows: new Set() });
    const clip = calls.indexOf('clip()');
    expect(calls[clip - 1]).toBe('rect(0,0,54,50)');
    const restore = calls.indexOf('restore()', clip);
    // The merge line to lane 5 is drawn inside the clip; nothing is stroked from the area's edge.
    expect(calls.slice(clip, restore).some((c) => c.startsWith('quadraticCurveTo('))).toBe(true);
    expect(calls.some((c) => c.startsWith('moveTo(54,'))).toBe(false);
  });

  it("shades a gradient band at the zone's left edge, over the lanes, before the packed nodes", () => {
    const { ctx, calls } = recorder();
    drawGraph(ctx, { ...base, rows: [row(0, 'commit', []), row(5, 'commit', [])], last: 2, width: 80, height: 50, labeledRows: new Set() });
    const grad = calls.indexOf(`createLinearGradient(${54 - SHADE_W},0,54,0)`);
    expect(grad).toBeGreaterThan(-1);
    expect(calls.slice(grad, grad + 3)).toEqual([`createLinearGradient(${54 - SHADE_W},0,54,0)`, 'addColorStop(0,rgba(0,0,0,0))', `addColorStop(1,rgba(0,0,0,${SHADE_ALPHA}))`]);
    const shade = calls.indexOf(`fillRect(${54 - SHADE_W},0,${SHADE_W},50)`, grad);
    expect(shade).toBeGreaterThan(calls.indexOf(`arc(16,12.5,9,0,${TAU})`));
    expect(shade).toBeLessThan(calls.findIndex((c) => c.startsWith('arc(66,')));
    expect(alphaAt(calls, shade)).toBe('globalAlpha=1');
  });

  it("runs a packed row's label connector to its packed node", () => {
    const { ctx, calls } = recorder();
    drawGraph(ctx, { ...base, rows: [row(0, 'commit', []), row(5, 'commit', [])], last: 2, width: 80, height: 50, labeledRows: new Set([0, 1]) });
    expect(calls).toContain('lineTo(16,12.5)');
    expect(calls).toContain('lineTo(57,37.5)'); // 66 - 9
  });

  it('packs by the lane centre: a lane whose centre is inside keeps its nodes, even half under the shade', () => {
    const { ctx, calls } = recorder();
    // Lane 2's centre (48) is inside the 54 px lane area, its node reaching 57: drawn in its lane.
    drawGraph(ctx, { ...base, rows: [row(2, 'commit', [(2 << 10) | 2 | (2 << 20)])], last: 1, width: 80, height: 25, labeledRows: new Set() });
    expect(calls.filter((c) => c.startsWith('arc(')).map((c) => c.split(',')[0])).toEqual(['arc(48']);
    expect(calls).toContain('lineTo(48,25)');
  });

  it('shifts the lanes by scrollX and packs the ones scrolled out on either side', () => {
    const { ctx, calls } = recorder();
    const rows = [row(0, 'commit', [1 << 20]), row(3, 'commit', []), row(5, 'commit', [])];
    drawGraph(ctx, { ...base, rows, last: 3, width: 80, height: 75, scrollX: 32, labeledRows: new Set() });
    expect(calls).toContain('translate(-32,0)');
    // Lane 0: 16 - 32 < 0, packed. Lane 3: 64 - 32 = 32, shown. Lane 5: 96 - 32 = 64, past 54: packed.
    expect(calls.filter((c) => c.startsWith('arc(')).map((c) => c.split(',')[0])).toEqual(['arc(32', 'arc(66', 'arc(66']);
    expect(calls).toContain('fillRect(32,27,48,21)');
  });

  it('at the minimum width: one column of coloured nodes, dimmed like any packed node, and the rail; no bands, lines or shade (F11)', () => {
    const { ctx, calls } = recorder();
    const rows = [row(0, 'commit', [1 << 20]), row(2, 'merge', [(2 << 10) | 2 | (2 << 20)]), row(1, 'stash', [])];
    drawGraph(ctx, { ...base, rows, last: 3, width: 48, height: 75, labeledRows: new Set([0]) });
    // Centred left of the 2 px rail: (48 - 2) / 2 = 23. Merges are nodes too; a stash keeps its square.
    expect(calls.filter((c) => c.startsWith('arc(')).map((c) => c.split(',').slice(0, 3).join(','))).toEqual(['arc(23,12.5,9', 'arc(23,37.5,9']);
    const s = stashHalf({ rowH: 25, laneW: 16, padX: 8 });
    expect(calls).toContain(`rect(${23 - s},${62.5 - s},${2 * s},${2 * s})`);
    expect(calls).not.toContain(`globalAlpha=${BAND_ALPHA}`);
    // Every lane has run off: every node is packed, so dimmed (R11).
    for (const [i, c] of calls.entries()) if (c.startsWith('arc(') || c.startsWith('rect(')) expect(alphaAt(calls, i), c).toBe(`globalAlpha=${PACKED_ALPHA}`);
    expect(calls.some((c) => c.startsWith('createLinearGradient('))).toBe(false);
    // The only line is the label connector, to the node's edge (the continuity rule): every other
    // lineTo is the stash node's icon, after its square.
    const square = calls.indexOf(`rect(${23 - s},${62.5 - s},${2 * s},${2 * s})`);
    expect(calls.slice(0, square).filter((c) => c.startsWith('lineTo('))).toEqual(['lineTo(14,12.5)']);
    expect(calls).toContain('fillRect(46,2,2,21)'); // the rail edge
  });
});

describe('nodeAt (hit-testing the drawn commit nodes)', () => {
  const m = { rowH: 22, laneW: 16, padX: 8 };
  const base = { rows: [row(0, 'commit', []), row(1, 'commit', []), row(0, 'wip', []), row(0, 'stash', []), row(0, 'merge', [])], scrollTop: 0, width: 100, metrics: m };
  const r = nodeRadius(m);
  it('hits the centre and misses just outside the radius', () => {
    expect(nodeAt(base, laneX(0, m), 11)).toBe(0);
    expect(nodeAt(base, laneX(0, m) + r - 0.1, 11)).toBe(0);
    expect(nodeAt(base, laneX(0, m) + r + 0.1, 11)).toBeNull();
    expect(nodeAt(base, laneX(1, m), 22 + 11)).toBe(1);
    expect(nodeAt(base, laneX(1, m), 11)).toBeNull();
  });
  it('only commit nodes count: not wip, stash or a merge dot', () => {
    expect(nodeAt(base, laneX(0, m), 44 + 11)).toBeNull();
    expect(nodeAt(base, laneX(0, m), 66 + 11)).toBeNull();
    expect(nodeAt(base, laneX(0, m), 88 + 11)).toBeNull();
  });
  it('respects scrollTop and scrollX', () => {
    expect(nodeAt({ ...base, scrollTop: 22 }, laneX(1, m), 11)).toBe(1);
    expect(nodeAt({ ...base, scrollX: 10 }, laneX(0, m) - 10, 11)).toBe(0);
    expect(nodeAt({ ...base, scrollX: 10 }, laneX(0, m), 11)).toBeNull();
  });
  it('a packed or strip node is hit where it is drawn, and a scrolled-out lane at the packed column', () => {
    const { packedX } = graphLayout(100, m, true);
    expect(nodeAt({ ...base, clipped: true, scrollX: 20 }, packedX, 11)).toBe(0);
    expect(nodeAt({ ...base, clipped: true, scrollX: 20 }, laneX(0, m) - 20, 11)).toBeNull();
    const strip = { ...base, width: 12, clipped: true };
    expect(graphLayout(12, m, true).strip).toBe(true);
    expect(nodeAt(strip, 5, 11)).toBe(0);
    expect(nodeAt(strip, laneX(0, m), 11)).toBeNull();
  });
});
