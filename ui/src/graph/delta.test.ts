import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { GraphDelta } from '../api/gen/GraphDelta';
import type { GraphPayload } from '../api/gen/GraphPayload';
import type { GraphReply } from '../api/gen/GraphReply';
import type { RowPayload } from '../api/gen/RowPayload';
import { applyDelta, graphOfReply } from './delta';

// (old, delta, new) triples from the core's own diff (gitvolt-graph's `delta::props`, which
// writes this file): the applier here must give exactly the core's new payload.
const golden = JSON.parse(readFileSync(new URL('./delta.golden.json', import.meta.url), 'utf8')) as { old: GraphPayload; delta: GraphReply; new: GraphPayload }[];

const row = (id: string, summary = id): RowPayload => ({ id, kind: 'commit', lane: 0, color: 0, segments: [], summary, bodyFirstLine: '', authorName: 'A', authorEmail: '', authorTime: 0, committerTime: 0, parents: [], mrRefs: [], wip: null });
const payload = (rows: RowPayload[]): GraphPayload => ({ rows, labels: [], maxLanes: 1, pinnedRefs: [], head: { branch: null, target: null, detached: false, unborn: false }, truncated: false, worktrees: [] });
const delta = (d: Partial<GraphDelta> & Pick<GraphDelta, 'rows'>): GraphDelta => ({ version: 2, since: 1, splices: [], patched: [], rest: payload([]), ...d });

describe('applyDelta', () => {
  it('replays the core\'s golden deltas', () => {
    expect(golden.length).toBeGreaterThan(5);
    for (const [i, t] of golden.entries()) {
      expect(t.delta.kind, `triple ${i}`).toBe('delta');
      const d = t.delta as GraphDelta & { kind: 'delta' };
      const old = structuredClone(t.old);
      const next = applyDelta(old, d);
      expect(next, `triple ${i}`).toEqual(t.new);
      // Every row the delta keeps is the old object.
      const olds = new Set(old.rows);
      const kept = next!.rows.filter((r) => olds.has(r)).length;
      const inserted = d.splices.reduce((n, s) => n + s.insert.length, 0);
      expect(kept + inserted + d.patched.length, `triple ${i}`).toBeGreaterThanOrEqual(next!.rows.length);
    }
  });

  it('keeps the old rows on top of which one was inserted, and the array itself when no row changed', () => {
    const old = payload([row('b'), row('c')]);
    const next = applyDelta(old, delta({ rows: 3, splices: [{ at: 0, remove: 0, insert: [row('a')] }] }))!;
    expect(next.rows.map((r) => r.id)).toEqual(['a', 'b', 'c']);
    expect(next.rows[1]).toBe(old.rows[0]);
    expect(next.rows[2]).toBe(old.rows[1]);
    expect(applyDelta(old, delta({ rows: 2 }))!.rows).toBe(old.rows);
  });

  it('refuses a delta made against another payload', () => {
    const old = payload([row('a')]);
    expect(applyDelta(old, delta({ rows: 2, splices: [{ at: 1, remove: 1, insert: [row('b'), row('c')] }] }))).toBeNull();
    expect(applyDelta(old, delta({ rows: 1, patched: [{ at: 0, row: row('z') }] }))).toBeNull();
    expect(applyDelta(old, delta({ rows: 3 }))).toBeNull();
  });
});

describe('graphOfReply', () => {
  const base = { version: 1, graph: payload([row('a')]) };

  it('takes a full reply whole, a delta against the payload held, and a plain payload as a full one with no version', () => {
    const full = payload([row('x')]);
    expect(graphOfReply({ kind: 'full', version: 3, graph: full }, base)).toEqual({ version: 3, graph: full });
    const d = graphOfReply({ kind: 'delta', ...delta({ rows: 2, since: 1, version: 2, splices: [{ at: 1, remove: 0, insert: [row('b')] }] }) }, base)!;
    expect(d.version).toBe(2);
    expect(d.graph.rows[0]).toBe(base.graph.rows[0]);
    expect(graphOfReply(full, base)).toEqual({ version: 0, graph: full });
  });

  it('gives null for a delta from another version than the one held', () => {
    expect(graphOfReply({ kind: 'delta', ...delta({ rows: 1, since: 7 }) }, base)).toBeNull();
    expect(graphOfReply({ kind: 'delta', ...delta({ rows: 1, since: 1 }) }, undefined)).toBeNull();
  });
});
