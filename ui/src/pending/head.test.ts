import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api/client';
import type { GraphPayload } from '../api/gen/GraphPayload';
import type { LocalBranch } from '../api/gen/LocalBranch';
import type { RefLabel } from '../api/gen/RefLabel';
import type { RowPayload } from '../api/gen/RowPayload';
import { useRuntime } from '../app/runtime';
import { useTabViews } from '../app/tabStores';
import { checkout, checkoutLabel, checkoutSideItem, checkoutLocal } from '../branches/checkout';
import { resetTo } from '../branches/reset';
import { fileMenuEnv } from '../menu/menuEnv';
import { createRepoViewStore } from '../repo/store';
import { fakeServices } from '../repo/testServices';
import { headBranchOf, pushBranch } from '../sync/push';
import { predictedHeadOf } from './head';

const [A, B, C] = ['a', 'b', 'c'].map((ch) => ch.repeat(40));
const row = (id: string): RowPayload => ({ id, kind: 'commit', lane: 0, color: 0, segments: [], summary: '', bodyFirstLine: '', authorName: '', authorEmail: '', authorTime: 0, committerTime: 0, parents: [], mrRefs: [], wip: null } as unknown as RowPayload);
const label = (r: number, name: string, isHead: boolean): RefLabel => ({ row: r, name, local: `refs/heads/${name}`, remotes: [], tag: false, isHead, worktree: null, checkedOut: isHead ? '/r' : null });
const local = (name: string, target: string, isHead: boolean): LocalBranch => ({ name, fullName: `refs/heads/${name}`, target, upstream: `refs/remotes/origin/${name}`, ahead: 1, behind: 0, gone: false, tipTime: 0, summary: '', author: '', isHead, worktree: null, checkedOut: isHead ? '/r' : null, pushTarget: `origin/${name}`, pushBehind: 0, rewritten: null });
const done = (outcome: unknown) => ({ outcome, journal: { undo: null, redo: null, undoBlocked: null, redoBlocked: null, banners: [] }, staging: { undo: null, redo: null, off: null }, wip: null });

/** Checkout of `a` done: HEAD is `a` (the graph and sidebar say so); `b` and `c` elsewhere. */
function setup() {
  const graph = {
    rows: [row(A), row(B), row(C)], labels: [label(0, 'a', true), label(1, 'b', false)], maxLanes: 1, pinnedRefs: [], truncated: false,
    head: { branch: 'refs/heads/a', target: A, detached: false, unborn: false },
    worktrees: [{ path: '/r', branch: 'refs/heads/a', head: A, isMain: true, locked: false, inProgress: null }],
  } as unknown as GraphPayload;
  const sidebar = { locals: [local('a', A, true), local('b', B, false)], remotes: [], tags: [], stashes: [], worktrees: [] };
  const store = createRepoViewStore(1, '/r', graph, fakeServices());
  useTabViews.setState({ views: { t: { repo: 1, services: fakeServices(), store } } });
  useRuntime.setState({ tabs: { t: { status: 'ready', error: null, repo: { id: 1, path: '/r', name: 'r' }, graph, info: null, sidebar, lastFetchAt: 0, fetchSkipped: null, limit: null, worktree: '/r' } as never } });
  return { store, graph, sidebar };
}

/** A write the backend holds (running, or queued behind one): resolved by `release`. */
function held() {
  const releases: ((v: unknown) => void)[] = [];
  const mock = () => new Promise((r) => releases.push(r)) as never;
  return { mock, release: (outcome: unknown) => { for (const r of releases.splice(0)) r(done(outcome)); } };
}

const ctx = { tabId: 't', repoId: 1, worktree: '/r' };
/** The refresh after the writes: a new graph and sidebar (the same here), so checkouts hand over. */
const refreshed = () => { const rt = useRuntime.getState().tabs.t; useRuntime.getState().patch('t', { graph: { ...rt.graph! }, sidebar: { ...rt.sidebar! } }); };

afterEach(() => {
  vi.restoreAllMocks();
  useTabViews.setState({ views: {} });
  useRuntime.setState({ tabs: {} });
});

describe('HEAD as the queued writes leave it (spec #2 §3.6)', () => {
  it('checkout a, push a (running), checkout b (queued): menus, double-click and the toolbar act on b', async () => {
    const { store, graph } = setup();
    const push = held();
    const co = held();
    vi.spyOn(api, 'push').mockImplementation(push.mock);
    const call = vi.spyOn(api, 'checkout').mockImplementation(co.mock);
    const pushing = pushBranch(ctx, local('a', A, true));
    const toB = checkout(ctx, { kind: 'branch', name: 'b' }, { head: A, refs: { 'refs/heads/b': B } });
    await vi.waitFor(() => expect(call).toHaveBeenCalledTimes(1));

    // The toolbar's Push and Pull target, the palette's interactive rebase…
    expect(headBranchOf('t')).toBe('b');
    expect(predictedHeadOf('t')).toEqual({ branch: 'b', sha: B });
    // …and the menus: HEAD is b, so b isn't offered for checkout and a is.
    const env = fileMenuEnv(store);
    expect(env.headBranch).toBe('b');
    expect(env.headSha).toBe(B);
    expect(env.labelsAt(A).map((l) => [l.name, l.isHead])).toEqual([['a', false]]);
    expect(env.labelsAt(B).map((l) => [l.name, l.isHead])).toEqual([['b', true]]);
    expect(env.sidebar?.locals.map((l) => [l.name, l.isHead])).toEqual([['a', false], ['b', true]]);
    // The graph itself still shows where HEAD is now.
    expect(useRuntime.getState().tabs.t.graph!.head.branch).toBe('refs/heads/a');

    // Double-clicking a's chip (HEAD now, not after the queue) queues a checkout of a, expecting b.
    expect(checkoutLabel(store, graph.rows[0], graph.labels[0])).toBe(true);
    await vi.waitFor(() => expect(call).toHaveBeenCalledTimes(2));
    expect(call).toHaveBeenLastCalledWith(1, '/r', { kind: 'branch', name: 'a' }, { head: B, refs: { 'refs/heads/a': A } }, false, undefined);
    expect(headBranchOf('t')).toBe('a');
    // …and b's sidebar row, b again.
    checkoutSideItem({ tabId: 't', store: null as never }, { kind: 'local', target: B, branch: local('b', B, false) } as never);
    await vi.waitFor(() => expect(call).toHaveBeenCalledTimes(3));
    expect(headBranchOf('t')).toBe('b');

    co.release({ status: 'done', branch: 'b' });
    push.release({ remote: 'origin', dst: 'a', branch: 'a', upToDate: true, forced: null, server: [], op: 1 });
    refreshed();
    await Promise.all([toB, pushing]);
    // Every queued write ended: back to what the graph says.
    expect(predictedHeadOf('t')).toEqual({ branch: 'a', sha: A });
  });

  it('a queued reset keeps the branch and moves the commit; a failed checkout drops out of the prediction', async () => {
    setup();
    const co = held();
    vi.spyOn(api, 'checkout').mockImplementationOnce(co.mock).mockRejectedValueOnce({ kind: 'RefMoved', message: 'b moved', commandId: null, stderr: null, detail: null });
    const reset = held();
    vi.spyOn(api, 'reset').mockImplementation(reset.mock);
    const toB = checkout(ctx, { kind: 'branch', name: 'b' }, { head: A, refs: { 'refs/heads/b': B } });
    const back = resetTo(ctx, C, 'hard', B);
    expect(predictedHeadOf('t')).toEqual({ branch: 'b', sha: C });
    co.release({ status: 'done', branch: 'b' });
    reset.release({});
    refreshed();
    await Promise.all([toB, back]);
    expect(predictedHeadOf('t')).toEqual({ branch: 'a', sha: A });

    await checkoutLocal('t', 'b');
    expect(predictedHeadOf('t')).toEqual({ branch: 'a', sha: A });
  });
});
