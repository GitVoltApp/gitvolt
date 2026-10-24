import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { FileChange } from '../api/gen/FileChange';

const stageAll = vi.hoisted(() => vi.fn(async () => true));
const stagePaths = vi.hoisted(() => vi.fn(async () => true));
const unstageAll = vi.hoisted(() => vi.fn(async () => true));
const discardUnstaged = vi.hoisted(() => vi.fn(async () => true));
const discardAll = vi.hoisted(() => vi.fn(async () => true));
vi.mock('./actions', async (orig) => ({ ...(await orig<typeof import('./actions')>()), stageAll, stagePaths, unstageAll, discardUnstaged, discardAll }));

import type { MenuRow } from '../menu/types';
import { discardAllRow, discardScopeOf, sectionRows } from './menuRows';
import { useStaging } from './store';

const ctx = { tabId: 't', repoId: 1, worktree: '/r' };
const f = (path: string, oldPath: string | null = null): FileChange => ({ path, oldPath, status: oldPath ? 'R' : 'M', additions: 1, deletions: 0, old: { kind: 'absent' }, new: { kind: 'worktree', worktree: '/r' }, submodule: false });
const all = { count: 3, confirmed: ['a.txt', 'b.txt', 'c.txt'], inProgress: false };
const labels = (rows: MenuRow[]) => rows.map((r) => (r.kind === 'action' ? r.label : r.kind));
const action = (rows: MenuRow[], id: string) => rows.find((r): r is Extract<MenuRow, { kind: 'action' }> => r.kind === 'action' && r.id === id)!;

beforeEach(() => {
  vi.clearAllMocks();
  useStaging.setState({ committing: {}, discarding: {} });
});

describe('a WIP section’s menu (its header buttons’ rows)', () => {
  it('Unstaged: Stage all, then Discard unstaged and Discard all; Staged: Unstage all, then Discard all', () => {
    expect(labels(sectionRows(ctx, 'unstaged', [f('a.txt')], { guardConflicts: false, all }))).toEqual(['Stage all changes', 'separator', 'Discard unstaged changes', 'Discard all changes…']);
    expect(labels(sectionRows(ctx, 'staged', [f('a.txt')], { guardConflicts: false, all }))).toEqual(['Unstage all changes', 'separator', 'Discard all changes…']);
    // An empty section: Discard all only; a submodule alone has no Discard unstaged.
    expect(labels(sectionRows(ctx, 'staged', [], { guardConflicts: false, all }))).toEqual(['Discard all changes…']);
    expect(labels(sectionRows(ctx, 'unstaged', [{ ...f('sub'), submodule: true }], { guardConflicts: false, all }))).toEqual(['Stage all changes', 'separator', 'Discard all changes…']);
  });

  it('runs the buttons’ writes; Stage all names the paths while files are conflicted', () => {
    const files = [f('a.txt'), f('new.txt', 'old.txt')];
    action(sectionRows(ctx, 'unstaged', files, { guardConflicts: false, all }), 'wip.stageAll').run();
    expect(stageAll).toHaveBeenCalledWith(ctx);
    action(sectionRows(ctx, 'unstaged', files, { guardConflicts: true, all }), 'wip.stageAll').run();
    expect(stagePaths).toHaveBeenCalledWith(ctx, ['a.txt', 'new.txt', 'old.txt']);
    action(sectionRows(ctx, 'unstaged', files, { guardConflicts: false, all }), 'wip.discardUnstaged').run();
    expect(discardUnstaged).toHaveBeenCalledWith(ctx);
    action(sectionRows(ctx, 'staged', files, { guardConflicts: false, all }), 'wip.unstageAll').run();
    expect(unstageAll).toHaveBeenCalledWith(ctx);
    action(sectionRows(ctx, 'staged', files, { guardConflicts: false, all }), 'wip.discardAll').run();
    expect(discardAll).toHaveBeenCalledWith(ctx, 3, ['a.txt', 'b.txt', 'c.txt']);
  });

  it('greys out as the buttons do: a queued commit, a discard running, mid-merge, nothing to discard', () => {
    const reason = (scope = all) => (discardAllRow(ctx, scope) as Extract<MenuRow, { kind: 'action' }>).disabledReason;
    expect(reason()).toBeUndefined();
    expect(reason({ ...all, inProgress: true })).toBe('Abort instead');
    expect(reason({ count: 0, confirmed: [], inProgress: false })).toBe('Nothing to discard');
    useStaging.getState().setDiscarding(1, '/r', true);
    expect(reason()).toBe('Discarding…');
    useStaging.getState().setCommitting(1, '/r', true);
    expect(reason()).toBe('Commit queued');
    expect(action(sectionRows(ctx, 'unstaged', [f('a.txt')], { guardConflicts: false, all }), 'wip.stageAll').disabledReason).toBe('Commit queued');
  });

  it('Discard all confirms each changed path once, a rename’s both', () => {
    expect(discardScopeOf([])).toBeNull();
    const scope = discardScopeOf([{ files: [f('a.txt'), f('new.txt', 'old.txt')], added: 0, deleted: 0 }, { files: [f('a.txt')], added: 0, deleted: 0 }]);
    expect(scope?.count).toBe(2);
    expect(scope?.confirmed?.sort()).toEqual(['a.txt', 'new.txt', 'old.txt']);
  });
});
