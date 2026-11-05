import { beforeEach, describe, expect, it, vi } from 'vitest';

const follow = vi.hoisted(() => vi.fn());
const confirm = vi.hoisted(() => vi.fn(async () => true));
const discard = vi.hoisted(() => vi.fn(async () => ({ outcome: null, wip: { worktree: '/r' } })));
const run = vi.hoisted(() => vi.fn());
vi.mock('./follow', () => ({ followOpenFile: follow, advanceFrom: () => null }));
vi.mock('../ui/ConfirmDialog', () => ({ confirmAction: confirm }));
vi.mock('../api/client', () => ({ api: { discard } }));
vi.mock('../write/client', () => ({ runWrite: run }));

import { discardAll, discardPaths, discardPatch, discardUnstaged, writeAndFollow } from './actions';
import { discardFiles } from './discardFiles';
import { stagingKey, useStaging } from './store';

const ctx = { tabId: 't', repoId: 1, worktree: '/r' };

beforeEach(() => {
  vi.clearAllMocks();
  useStaging.setState({ states: {}, committing: {}, discarding: {}, discardingPaths: {} });
  // runWrite as the real one: sends, answers the outcome, or null when the send throws.
  run.mockImplementation(async (_c: unknown, send: () => Promise<{ outcome: unknown }>) => {
    try { return (await send()).outcome; } catch { return null; }
  });
});

describe('writeAndFollow (spec #2 §7.1)', () => {
  it('follows after a write whose answer carries the fresh lists', async () => {
    expect(await writeAndFollow(ctx, async () => ({ wip: { worktree: '/r' } }) as never)).toBe(true);
    expect(follow).toHaveBeenCalledWith('t', '/r', null);
  });

  it('does not follow after a failed write', async () => {
    expect(await writeAndFollow(ctx, async () => { throw new Error('boom'); })).toBe(false);
    expect(follow).not.toHaveBeenCalled();
  });

  it('does not follow when the answer has no wip: the cached lists may be stale', async () => {
    expect(await writeAndFollow(ctx, async () => ({ wip: null }) as never)).toBe(true);
    expect(follow).not.toHaveBeenCalled();
  });
});

describe('discardAll', () => {
  it('confirms, then discards everything', async () => {
    expect(await discardAll(ctx)).toBe(true);
    expect(confirm).toHaveBeenCalledWith(expect.objectContaining({ title: 'Discard all changes?', danger: true }));
    expect(discard).toHaveBeenCalledWith(1, '/r', { kind: 'all' });
  });

  it('writes nothing when cancelled', async () => {
    confirm.mockResolvedValueOnce(false);
    expect(await discardAll(ctx)).toBe(false);
    expect(discard).not.toHaveBeenCalled();
  });

  // UX R1 C.2: a slow Discard all (a big repository) showed nothing while it ran, so the user
  // clicked again; each click sent another one. Now it sends the files confirmed, and one at a
  // time: another click meanwhile does nothing, and the button says it's running.
  it('sends the files confirmed, and one at a time', async () => {
    let finish!: () => void;
    discard.mockImplementationOnce(() => new Promise((res) => { finish = () => res({ outcome: null, wip: { worktree: '/r' } }); }));
    const first = discardAll(ctx, 2, ['a.txt', 'b.txt']);
    await vi.waitFor(() => expect(discard).toHaveBeenCalledTimes(1));
    expect(discard).toHaveBeenCalledWith(1, '/r', { kind: 'all', confirmed: ['a.txt', 'b.txt'] });
    expect(useStaging.getState().discarding[stagingKey(1, '/r')]).toBe(true);
    expect(await discardAll(ctx, 2, ['a.txt', 'b.txt'])).toBe(false);
    expect(confirm).toHaveBeenCalledTimes(1);
    finish();
    expect(await first).toBe(true);
    expect(discard).toHaveBeenCalledTimes(1);
    expect(useStaging.getState().discarding[stagingKey(1, '/r')]).toBe(false);
  });
});

// 1.5.0 report: a file's Discard clicked again before the first was answered (a big repository
// takes seconds) sent a second one, which found nothing left to discard and failed.
describe('a file Discard runs once at a time', () => {
  const held = () => {
    let finish!: () => void;
    discard.mockImplementationOnce(() => new Promise((res) => { finish = () => res({ outcome: null, wip: { worktree: '/r' } }); }));
    return () => finish();
  };
  const busy = () => useStaging.getState().discardingPaths[stagingKey(1, '/r')];

  it('another Discard of the same paths, from any entry point, is a no-op until it is answered', async () => {
    const finish = held();
    const first = discardPaths(ctx, ['a.txt', 'a.png']);
    await vi.waitFor(() => expect(discard).toHaveBeenCalledTimes(1));
    expect(busy()).toEqual(['a.txt', 'a.png']);
    expect(await discardPaths(ctx, ['a.txt'])).toBe(false);
    // The row's button and its menu arm nothing either: there is nothing to confirm.
    expect(await discardFiles(ctx, [{ path: 'a.png', oldPath: null } as never], 'a.png')).toBe(false);
    expect(confirm).not.toHaveBeenCalled();
    expect(await discardPatch(ctx, 'a.txt', { kind: 'hunks', hunks: [0] }, {} as never)).toBe(false);
    expect(discard).toHaveBeenCalledTimes(1);
    finish();
    expect(await first).toBe(true);
    expect(busy()).toEqual([]);
    // Answered, the file can be discarded again.
    expect(await discardPaths(ctx, ['a.txt'])).toBe(true);
    expect(discard).toHaveBeenCalledTimes(2);
  });

  it('other files go on meanwhile, and a failed Discard frees its paths', async () => {
    const finish = held();
    const first = discardPaths(ctx, ['a.txt']);
    await vi.waitFor(() => expect(discard).toHaveBeenCalledTimes(1));
    discard.mockRejectedValueOnce(new Error('boom'));
    expect(await discardPaths(ctx, ['b.txt'])).toBe(false);
    expect(discard).toHaveBeenCalledTimes(2);
    expect(busy()).toEqual(['a.txt']);
    finish();
    await first;
    expect(busy()).toEqual([]);
  });

  it('a Discard all or unstaged running takes every file; Discard unstaged runs once at a time', async () => {
    const finish = held();
    const first = discardUnstaged(ctx);
    await vi.waitFor(() => expect(discard).toHaveBeenCalledTimes(1));
    expect(useStaging.getState().discarding[stagingKey(1, '/r')]).toBe(true);
    expect(await discardUnstaged(ctx)).toBe(false);
    expect(await discardPaths(ctx, ['a.txt'])).toBe(false);
    expect(discard).toHaveBeenCalledTimes(1);
    finish();
    expect(await first).toBe(true);
    expect(useStaging.getState().discarding[stagingKey(1, '/r')]).toBe(false);
  });
});
