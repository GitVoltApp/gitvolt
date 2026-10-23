import { describe, expect, it } from 'vitest';
import type { GraphPayload } from '../api/gen/GraphPayload';
import type { RowPayload } from '../api/gen/RowPayload';
import { createRepoViewStore, selectedIndex } from './store';
import { recordingServices as fakeServices } from './testServices';

const A = 'a'.repeat(40), B = 'b'.repeat(40), H = 'd'.repeat(40);
const row = (id: string): RowPayload => ({
  id, kind: 'commit', lane: 0, color: 0, segments: [], summary: id.slice(0, 1), bodyFirstLine: '', authorName: 'Ada', authorEmail: 'ada@example.com', authorTime: 0, committerTime: 0, parents: [], mrRefs: [], wip: null,
});
const all: GraphPayload = {
  rows: [row(A), row(B)], labels: [], maxLanes: 1, pinnedRefs: [], head: { branch: 'refs/heads/main', target: A, detached: false, unborn: false }, truncated: false, worktrees: [],
};
// Under a branch preset: some loaded commits aren't rows.
const viewed: GraphPayload = { ...all, view: { branches: [], trunk: ['main'] } };

describe('a commit a branch preset hides', () => {
  it('clicking a panel row shows it in the details panel', () => {
    const { services, calls } = fakeServices();
    const s = createRepoViewStore(1, '/r', viewed, services);
    s.getState().selectRevealed(H);
    expect(s.getState().selection).toEqual({ kind: 'commit', index: -1, id: H });
    expect(selectedIndex(s.getState())).toBe(-1);
    expect(calls).toContain(`details ${H}`);
    expect(calls).toContain(`files {"kind":"commit","id":"${H}","parent":0}`);
    // A commit that is a row is selected as one.
    s.getState().selectRevealed(B);
    expect(s.getState().selection).toEqual({ kind: 'commit', index: 1, id: B });
  });

  it('stays selected across graphs while a preset is on, and takes its row once it has one', () => {
    const { services } = fakeServices();
    const s = createRepoViewStore(1, '/r', viewed, services);
    s.getState().selectRevealed(H);
    s.getState().setGraph({ ...viewed, rows: [...viewed.rows] });
    expect(s.getState().selection).toEqual({ kind: 'commit', index: -1, id: H });
    // Expanded in the graph: its row.
    s.getState().setGraph({ ...viewed, rows: [...viewed.rows, row(H)] });
    expect(s.getState().selection).toEqual({ kind: 'commit', index: 2, id: H });
    // Under All every loaded commit is a row: not one, not loaded.
    s.getState().selectRevealed('e'.repeat(40));
    s.getState().setGraph({ ...all, rows: [...all.rows] });
    expect(s.getState().selection).toEqual({ kind: 'none' });
  });

  it('a Shift click from it selects the clicked row alone', () => {
    const { services } = fakeServices();
    const s = createRepoViewStore(1, '/r', viewed, services);
    s.getState().selectRevealed(H);
    s.getState().selectRow(1, { shift: true });
    expect(s.getState().selection).toEqual({ kind: 'commit', index: 1, id: B });
  });
});
