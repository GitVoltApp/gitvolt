import { beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api/client';
import { useAppState } from '../app/state';
import { useRuntime } from '../app/runtime';
import * as confirm from '../ui/ConfirmDialog';
import * as active from './active';
import { removeWorktree, removeWorktreeAndBranch } from './remove';

const ctx = { tabId: 't', repoId: 1, worktree: '/r' };
const res = (outcome: unknown) => ({ outcome, journal: { undo: null, redo: null, undoBlocked: null, redoBlocked: null, banners: [] }, staging: { undo: null, redo: null, off: null }, wip: null });

describe('Remove worktree (spec #2 §11.1)', () => {
  beforeEach(() => vi.restoreAllMocks());
  it('always confirms, and asks again before forcing', async () => {
    const ask = vi.spyOn(confirm, 'confirmAction').mockResolvedValue(true);
    const rm = vi.spyOn(api, 'worktreeRemove').mockResolvedValueOnce(res({ status: 'needsForce', reason: "It has changes that aren't committed" }) as never).mockResolvedValueOnce(res({ status: 'removed' }) as never);
    useRuntime.getState().patch('t', { repo: { id: 1, path: '/r', name: 'r', worktree: '/r' }, worktree: '/r', graph: { worktrees: [{ path: '/r', isMain: true }, { path: '/r-x', isMain: false }] } as never });
    await removeWorktree(ctx, '/r-x', 'x');
    expect(ask.mock.calls[0][0]).toMatchObject({ title: 'Remove worktree ../r-x?', body: 'Its directory is deleted; branch x stays.', confirmLabel: 'Remove', danger: true });
    expect(ask.mock.calls[1][0].body).toBe("../r-x has changes that aren't committed. They're lost: this can't be undone.");
    expect(rm).toHaveBeenLastCalledWith(1, '/r', '/r-x', true);
  });
  it('a tab whose active worktree it is moves to the main worktree first (Review Focus 5)', async () => {
    vi.spyOn(confirm, 'confirmAction').mockResolvedValue(true);
    vi.spyOn(api, 'worktreeRemove').mockResolvedValue(res({ status: 'removed' }) as never);
    const sw = vi.spyOn(active, 'setActiveWorktree').mockImplementation(() => {});
    useAppState.getState().setProfile({ ...useAppState.getState().profile, tabs: [{ id: 'u', kind: 'repo', path: '/r', alias: null, worktree: '/r-x' }] });
    useRuntime.getState().patch('u', { repo: { id: 1, path: '/r', name: 'r', worktree: '/r-x' }, worktree: '/r-x', graph: { worktrees: [{ path: '/r', isMain: true }, { path: '/r-x', isMain: false }] } as never });
    await removeWorktree(ctx, '/r-x', 'x');
    expect(sw).toHaveBeenCalledWith('u', '/r');
  });
  it('with a tab already on main, the removed worktree\'s own tab is closed, not left on a deleted folder', async () => {
    vi.restoreAllMocks();
    vi.spyOn(confirm, 'confirmAction').mockResolvedValue(true);
    vi.spyOn(api, 'worktreeRemove').mockResolvedValue(res({ status: 'removed' }) as never);
    useAppState.getState().setProfile({ ...useAppState.getState().profile, activeTab: 't', tabs: [{ id: 't', kind: 'repo', path: '/r', alias: null, worktree: '/r' }, { id: 'u', kind: 'repo', path: '/r', alias: null, worktree: '/r-x' }] });
    const graph = { worktrees: [{ path: '/r', isMain: true }, { path: '/r-x', isMain: false }] } as never;
    useRuntime.getState().patch('t', { repo: { id: 1, path: '/r', name: 'r', worktree: '/r' }, worktree: '/r', graph });
    useRuntime.getState().patch('u', { repo: { id: 1, path: '/r', name: 'r', worktree: '/r-x' }, worktree: '/r-x', graph });
    await removeWorktree(ctx, '/r-x', 'x');
    expect(useAppState.getState().profile.tabs.map((t) => t.id)).toEqual(['t']);
  });
  it('a failed remove puts the moved tab back', async () => {
    vi.restoreAllMocks();
    vi.spyOn(confirm, 'confirmAction').mockResolvedValue(true);
    vi.spyOn(api, 'worktreeRemove').mockRejectedValue(new Error('nope'));
    const sw = vi.spyOn(active, 'setActiveWorktree').mockImplementation(() => {});
    useAppState.getState().setProfile({ ...useAppState.getState().profile, tabs: [{ id: 'u', kind: 'repo', path: '/r', alias: null, worktree: '/r-x' }] });
    useRuntime.getState().patch('u', { repo: { id: 1, path: '/r', name: 'r', worktree: '/r-x' }, worktree: '/r-x', graph: { worktrees: [{ path: '/r', isMain: true }, { path: '/r-x', isMain: false }] } as never });
    await removeWorktree(ctx, '/r-x', 'x');
    expect(sw).toHaveBeenLastCalledWith('u', '/r-x');
  });

  describe('with its branch (#9)', () => {
    const on = (tabs: Array<[string, string]>) => {
      useAppState.getState().setProfile({ ...useAppState.getState().profile, activeTab: tabs[0][0], tabs: tabs.map(([id, wt]) => ({ id, kind: 'repo' as const, path: '/r', alias: null, worktree: wt })) });
      for (const [id, wt] of tabs) useRuntime.getState().patch(id, { repo: { id: 1, path: '/r', name: 'r', worktree: wt }, worktree: wt, graph: { worktrees: [{ path: '/r', isMain: true }, { path: '/r-x', isMain: false }] } as never });
    };
    const branch = { name: 'x', oid: 'x1' };
    beforeEach(() => { vi.restoreAllMocks(); on([['t', '/r']]); });

    it('one confirm names both; then the removal, then the local branch, from the main worktree', async () => {
      const ask = vi.spyOn(confirm, 'confirmAction').mockResolvedValue(true);
      const rm = vi.spyOn(api, 'worktreeRemove').mockResolvedValue(res({ status: 'removed' }) as never);
      const del = vi.spyOn(api, 'deleteBranch').mockResolvedValue(res({ status: 'deleted' }) as never);
      await removeWorktreeAndBranch({ ...ctx, worktree: '/r-x' }, '/r-x', branch);
      expect(ask).toHaveBeenCalledTimes(1);
      expect(ask.mock.calls[0][0]).toMatchObject({ title: 'Remove worktree ../r-x and delete x?', arm: 'Click again to remove ../r-x and delete x', confirmLabel: 'Remove and delete', danger: true });
      expect(rm).toHaveBeenCalledWith(1, '/r', '/r-x', false);
      expect(del).toHaveBeenCalledWith(1, '/r', { branch: 'x', local: true, remote: null, force: false, expect: { head: null, refs: { 'refs/heads/x': 'x1' } } });
      expect(rm.mock.invocationCallOrder[0]).toBeLessThan(del.mock.invocationCallOrder[0]);
    });
    it('a dirty worktree still asks before forcing, and an unmerged branch before deleting it anyway', async () => {
      const ask = vi.spyOn(confirm, 'confirmAction').mockResolvedValue(true);
      vi.spyOn(api, 'worktreeRemove').mockResolvedValueOnce(res({ status: 'needsForce', reason: 'dirty' }) as never).mockResolvedValueOnce(res({ status: 'removed' }) as never);
      const del = vi.spyOn(api, 'deleteBranch').mockResolvedValueOnce(res({ status: 'unmerged', branch: 'x', into: 'main', commits: 2 }) as never).mockResolvedValueOnce(res({ status: 'deleted' }) as never);
      await removeWorktreeAndBranch(ctx, '/r-x', branch);
      expect(ask.mock.calls.map((c) => c[0].title)).toEqual(['Remove worktree ../r-x and delete x?', 'Remove it anyway?', 'Delete x?']);
      expect(del.mock.calls.map((c) => c[2].force)).toEqual([false, true]);
    });
    it('a cancel at either removal question, or a failed removal, leaves the branch alone', async () => {
      const del = vi.spyOn(api, 'deleteBranch').mockResolvedValue(res({ status: 'deleted' }) as never);
      const rm = vi.spyOn(api, 'worktreeRemove').mockResolvedValue(res({ status: 'needsForce', reason: 'dirty' }) as never);
      vi.spyOn(confirm, 'confirmAction').mockResolvedValueOnce(false);
      await removeWorktreeAndBranch(ctx, '/r-x', branch);
      expect(rm).not.toHaveBeenCalled();
      vi.spyOn(confirm, 'confirmAction').mockResolvedValueOnce(true).mockResolvedValueOnce(false);
      await removeWorktreeAndBranch(ctx, '/r-x', branch);
      expect(rm).toHaveBeenCalledTimes(1);
      rm.mockRejectedValue(new Error('nope'));
      vi.spyOn(confirm, 'confirmAction').mockResolvedValue(true);
      await removeWorktreeAndBranch(ctx, '/r-x', branch);
      expect(del).not.toHaveBeenCalled();
    });
    it("removed from its own tab while another shows main, the branch is deleted in that other tab", async () => {
      on([['u', '/r-x'], ['t', '/r']]);
      vi.spyOn(confirm, 'confirmAction').mockResolvedValue(true);
      vi.spyOn(api, 'worktreeRemove').mockResolvedValue(res({ status: 'removed' }) as never);
      const del = vi.spyOn(api, 'deleteBranch').mockResolvedValue(res({ status: 'deleted' }) as never);
      const pending = vi.spyOn(await import('../pending/store'), 'withPending');
      await removeWorktreeAndBranch({ tabId: 'u', repoId: 1, worktree: '/r-x' }, '/r-x', branch);
      expect(useAppState.getState().profile.tabs.map((t) => t.id)).toEqual(['t']);
      expect(del).toHaveBeenCalledWith(1, '/r', expect.objectContaining({ branch: 'x' }));
      expect(pending.mock.calls.at(-1)?.[0]).toBe('t');
    });
  });
});
