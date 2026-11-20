import { describe, expect, it } from 'vitest';
import type { GraphPayload } from '../api/gen/GraphPayload';
import type { RefLabel } from '../api/gen/RefLabel';
import type { RowPayload } from '../api/gen/RowPayload';
import { labelsByRowOf } from './graphIndex';
import { reuseUnchanged, sameJsonValue } from './reuse';

const row = (id: string, parents: string[], extra: Partial<RowPayload> = {}): RowPayload => ({ id, kind: 'commit', lane: 0, color: 0, segments: [1, 2], summary: id, bodyFirstLine: '', authorName: 'Mara Quill', authorEmail: 'mara@example.com', authorTime: 0, committerTime: 0, parents, mrRefs: [], wip: null, ...extra });
const label = (row: number, name: string, isHead = false): RefLabel => ({ row, name, local: `refs/heads/${name}`, remotes: [], tag: false, isHead, worktree: null, checkedOut: null });
const payload = (rows: RowPayload[], labels: RefLabel[]): GraphPayload => ({ rows, labels, maxLanes: 1, pinnedRefs: [], head: { branch: 'main', target: rows[0]?.id ?? null, detached: false, unborn: false }, truncated: false, worktrees: [] });
const clone = <T,>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

describe('sameJsonValue', () => {
  it('compares JSON values structurally', () => {
    expect(sameJsonValue({ a: [1, { b: null }] }, { a: [1, { b: null }] })).toBe(true);
    expect(sameJsonValue({ a: [1, 2] }, { a: [1, 2, 3] })).toBe(false);
    expect(sameJsonValue({ a: 1 }, { b: 1 })).toBe(false);
    expect(sameJsonValue({ a: 1 }, { a: 1, b: undefined })).toBe(false);
    expect(sameJsonValue([], {})).toBe(false);
    expect(sameJsonValue(null, {})).toBe(false);
  });
});

describe('reuseUnchanged: a refresh keeps the objects of what it left alone', () => {
  const prev = payload([row('b', ['a']), row('a', [])], [label(0, 'main', true), label(1, 'old')]);

  it('keeps the arrays themselves when nothing changed', () => {
    const next = reuseUnchanged(prev, clone(prev));
    expect(next.rows).toBe(prev.rows);
    expect(next.labels).toBe(prev.labels);
  });

  it('a commit on top: one new row, the others kept; moved labels are new', () => {
    const fresh = payload([row('c', ['b']), ...clone(prev.rows)], [label(0, 'main', true), label(2, 'old')]);
    const next = reuseUnchanged(prev, fresh);
    expect(next.rows[0]).toBe(fresh.rows[0]);
    expect(next.rows[1]).toBe(prev.rows[0]);
    expect(next.rows[2]).toBe(prev.rows[1]);
    expect(next.labels[0]).toBe(prev.labels[0]);
    expect(next.labels[1]).toBe(fresh.labels[1]);
  });

  it('a row whose content changed (its lanes, a WIP count) is the new one', () => {
    const fresh = clone(prev);
    fresh.rows[1] = { ...fresh.rows[1], segments: [1, 3] };
    const next = reuseUnchanged(prev, fresh);
    expect(next.rows[0]).toBe(prev.rows[0]);
    expect(next.rows[1]).toBe(fresh.rows[1]);
    expect(next.rows).not.toBe(prev.rows);
  });

  it('the same rows reordered make a new array of the old objects', () => {
    const fresh = payload([clone(prev.rows[1]), clone(prev.rows[0])], []);
    const next = reuseUnchanged(prev, fresh);
    expect(next.rows).not.toBe(prev.rows);
    expect(next.rows[0]).toBe(prev.rows[1]);
  });

  it('a row whose labels are the same objects keeps its label list (graphIndex)', () => {
    const before = labelsByRowOf(prev.labels);
    const fresh = clone(prev);
    fresh.labels = [label(0, 'main', true), label(1, 'old'), label(1, 'new')];
    const next = reuseUnchanged(prev, fresh);
    const after = labelsByRowOf(next.labels);
    expect(after.get(0)).toBe(before.get(0));
    expect(after.get(1)).not.toBe(before.get(1));
    expect(after.get(1)?.map((l) => l.name)).toEqual(['old', 'new']);
  });
});
