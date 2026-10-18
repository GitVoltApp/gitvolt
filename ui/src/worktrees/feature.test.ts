import { describe, expect, it, vi } from 'vitest';
import type { RefLabel } from '../api/gen/RefLabel';
import type { RowPayload } from '../api/gen/RowPayload';
import { graphLabelDoubleClick } from '../graph/rowActions';
import type { RepoViewStore } from '../repo/store';

const setActiveWorktree = vi.hoisted(() => vi.fn());
vi.mock('./active', () => ({ setActiveWorktree, activeWorktreeOf: () => '/r', openWorktreeTab: vi.fn(), mainWorktreeOf: () => '/r' }));
vi.mock('../app/tabStores', async (orig) => ({ ...(await orig<object>()), tabIdOf: () => 't' }));
await import('./feature');

const store = {} as RepoViewStore;
const row = { id: 'abc', kind: 'commit' } as RowPayload;
const chip = (path: string): RefLabel => ({ row: 0, name: path.slice(1), local: null, remotes: [], tag: false, isHead: false, worktree: path, checkedOut: path });

describe("a worktree's detached-HEAD chip (§11.2)", () => {
  it('a double-click makes its worktree the active one', () => {
    expect(graphLabelDoubleClick(store, row, chip('/r-e'))).toBe(true);
    expect(setActiveWorktree).toHaveBeenLastCalledWith('t', '/r-e');
  });
  it("leaves the active worktree's own chip and other labels alone", () => {
    setActiveWorktree.mockClear();
    expect(graphLabelDoubleClick(store, row, chip('/r'))).toBe(false);
    expect(graphLabelDoubleClick(store, row, { ...chip('/r-d'), local: 'refs/heads/x' })).toBe(false);
    expect(graphLabelDoubleClick(store, row, { ...chip('/r-d'), tag: true })).toBe(false);
    expect(setActiveWorktree).not.toHaveBeenCalled();
  });
});
