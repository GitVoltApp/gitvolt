import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { RefLabel } from '../api/gen/RefLabel';
import type { RowPayload } from '../api/gen/RowPayload';
import type { DroppedLabel } from '../graph/branchDrag';
import type { MenuEnv } from '../menu/menuEnv';
import type { MenuRow } from '../menu/types';
import { checkout } from '../branches/checkout';
import { openRebaseEditor } from '../irebase/open';
import { useJournal } from '../undo/store';
import { branchDropRows } from './dropMenu';
import { startIntegrate } from './integrate';

vi.mock('./integrate', async (orig) => ({ ...(await orig<typeof import('./integrate')>()), startIntegrate: vi.fn(async () => {}) }));
vi.mock('../irebase/open', () => ({ openRebaseEditor: vi.fn(async () => true) }));
vi.mock('../branches/checkout', () => ({ checkout: vi.fn(async () => true) }));
vi.mock('../pending/head', () => ({ predictedHeadOf: () => ({ branch: 'feature/x', sha: 'x' }) }));

const ctx = { tabId: 't', repoId: 1, worktree: '/r' };
const row = (id: string) => ({ id, kind: 'commit', lane: 0, color: 0, segments: [], summary: id, bodyFirstLine: '', authorName: '', authorEmail: '', authorTime: 0, committerTime: 0, parents: [], mrRefs: [], wip: null }) as RowPayload;
const label = (name: string, r: number, extra: Partial<RefLabel> = {}): RefLabel => ({ row: r, name, local: `refs/heads/${name}`, remotes: [], tag: false, isHead: false, worktree: null, checkedOut: null, ...extra });
/** feature/x's tip is `x`, main's `m`. */
const X: DroppedLabel = { row: row('x'), label: label('feature/x', 1) };
const MAIN: DroppedLabel = { row: row('m'), label: label('main', 0, { isHead: true, checkedOut: '/r' }) };
const env = (over: Partial<MenuEnv> = {}) => ({ headBranch: 'main', headSha: 'm', write: ctx, inProgress: null, isAncestor: () => false, sidebar: null, activeWorktree: '/r', worktreeShown: (p: string) => p, ...over }) as MenuEnv;
const labels = (rows: MenuRow[]) => rows.map((r) => (r.kind === 'action' ? `${r.label}${r.disabledReason ? ` [${r.disabledReason}]` : ''}` : '---'));
const run = (rows: MenuRow[], name: string) => {
  const r = rows.find((x) => x.kind === 'action' && x.label === name);
  if (r?.kind !== 'action') throw new Error(`no row ${name}`);
  r.run();
};

beforeEach(() => vi.clearAllMocks());

describe('the branch drop menu', () => {
  it('B checked out, diverged or of unknown ancestry: merge, rebase, interactive rebase, in that order', () => {
    for (const isAncestor of [() => false, () => null]) {
      expect(labels(branchDropRows(X, MAIN, env({ isAncestor })))).toEqual(['Merge feature/x into main', 'Rebase main onto feature/x', 'Interactive rebase main onto feature/x']);
    }
  });

  it('B behind A: the fast-forward of B first, never a fast-forward of A', () => {
    expect(labels(branchDropRows(X, MAIN, env({ isAncestor: (a, b) => a === 'm' && b === 'x' })))).toEqual(['Fast-forward main to feature/x', 'Rebase main onto feature/x']);
  });

  it("A already in B: a greyed merge row says so, the interactive rebase stays unless A is on B's commit", () => {
    expect(labels(branchDropRows(X, MAIN, env({ isAncestor: (a, b) => a === 'x' && b === 'm' })))).toEqual(['Merge feature/x into main [main already contains feature/x]', 'Interactive rebase main onto feature/x']);
    const same: DroppedLabel = { row: row('m'), label: label('feature/x', 0) };
    expect(labels(branchDropRows(same, MAIN, env({ isAncestor: (a, b) => a === b })))).toEqual(['Merge feature/x into main [main already contains feature/x]']);
  });

  it('an operation in progress greys every row', () => {
    for (const isAncestor of [() => false, (a: string, b: string) => a === 'x' && b === 'm']) {
      const rows = branchDropRows(X, MAIN, env({ inProgress: 'rebase', isAncestor }));
      expect(rows.length).toBeGreaterThan(0);
      expect(rows.every((r) => r.kind === 'action' && r.disabledReason === 'Finish or abort the rebase first')).toBe(true);
    }
  });

  it('A on itself, or no write context: no rows', () => {
    expect(branchDropRows(MAIN, MAIN, env())).toEqual([]);
    expect(branchDropRows(X, MAIN, env({ write: null }))).toEqual([]);
  });

  it('B checked out: each row runs on B, with no checkout', () => {
    const rows = branchDropRows(X, MAIN, env());
    run(rows, 'Merge feature/x into main');
    expect(startIntegrate).toHaveBeenCalledWith(ctx, 'merge', 'feature/x', 'main', undefined);
    run(rows, 'Rebase main onto feature/x');
    expect(startIntegrate).toHaveBeenLastCalledWith(ctx, 'rebase', 'feature/x', 'main', undefined);
    run(rows, 'Interactive rebase main onto feature/x');
    expect(openRebaseEditor).toHaveBeenCalledWith('t', { branch: 'main', base: 'feature/x' });
    run(branchDropRows(X, MAIN, env({ isAncestor: (a, b) => a === 'm' && b === 'x' })), 'Fast-forward main to feature/x');
    expect(startIntegrate).toHaveBeenLastCalledWith(ctx, 'merge', 'feature/x', 'main', true);
    expect(checkout).not.toHaveBeenCalled();
  });

  describe('B not checked out here', () => {
    /** HEAD is feature/x; main isn't checked out anywhere. */
    const main: DroppedLabel = { row: row('m'), label: label('main', 0) };
    const onX = (over: Partial<MenuEnv> = {}) => env({ headBranch: 'feature/x', headSha: 'x', ...over });
    const tooltips = (rows: MenuRow[]) => rows.map((r) => (r.kind === 'action' ? r.tooltip : ''));

    it("B's rows, each saying it checks B out first", () => {
      const rows = branchDropRows(X, main, onX());
      expect(labels(rows)).toEqual(['Merge feature/x into main', 'Rebase main onto feature/x', 'Interactive rebase main onto feature/x']);
      expect(tooltips(rows).every((t) => t.endsWith(' Checks out main first.'))).toBe(true);
      expect(tooltips(rows)[0]).toBe('git merge feature/x on main. Checks out main first.');
    });

    it('a row checks B out, pinned to its tip, then runs the action on B', async () => {
      run(branchDropRows(X, main, onX()), 'Merge feature/x into main');
      await vi.waitFor(() => expect(startIntegrate).toHaveBeenCalledWith(ctx, 'merge', 'feature/x', 'main', undefined));
      expect(checkout).toHaveBeenCalledWith(ctx, { kind: 'branch', name: 'main' }, { head: 'x', refs: { 'refs/heads/main': 'm' } });
      expect(vi.mocked(checkout).mock.invocationCallOrder[0]).toBeLessThan(vi.mocked(startIntegrate).mock.invocationCallOrder[0]);
      run(branchDropRows(X, main, onX()), 'Interactive rebase main onto feature/x');
      await vi.waitFor(() => expect(openRebaseEditor).toHaveBeenCalledWith('t', { branch: 'main', base: 'feature/x' }));
    });

    it('a failed or cancelled checkout runs nothing more', async () => {
      vi.mocked(checkout).mockResolvedValue(false);
      const rows = branchDropRows(X, main, onX());
      for (const name of ['Merge feature/x into main', 'Rebase main onto feature/x', 'Interactive rebase main onto feature/x']) run(rows, name);
      await vi.waitFor(() => expect(checkout).toHaveBeenCalledTimes(3));
      await Promise.resolve();
      expect(startIntegrate).not.toHaveBeenCalled();
      expect(openRebaseEditor).not.toHaveBeenCalled();
      vi.mocked(checkout).mockResolvedValue(true);
    });

    it("a checkout whose restored changes conflict runs nothing more: you're left on B to resolve them", async () => {
      const banner = (entry: number, kind: 'autostashConflicts' | 'recovery') => ({ entry, kind, label: 'checkout main', stash: 's', stashMessage: 'autostash', target: null, snapshot: false, files: 1, canDrop: true, binary: false });
      const journal = (banners: ReturnType<typeof banner>[]) => useJournal.getState().set(1, '/r', { undo: null, redo: null, undoBlocked: null, redoBlocked: null, banners, paused: null });
      journal([banner(1, 'autostashConflicts')]);
      vi.mocked(checkout).mockImplementationOnce(async () => { journal([banner(1, 'autostashConflicts'), banner(2, 'autostashConflicts')]); return true; });
      run(branchDropRows(X, main, onX()), 'Merge feature/x into main');
      await vi.waitFor(() => expect(checkout).toHaveBeenCalledOnce());
      await Promise.resolve();
      expect(startIntegrate).not.toHaveBeenCalled();
      // An older conflict banner (or another kind) doesn't stop the next one.
      vi.mocked(checkout).mockImplementationOnce(async () => { journal([banner(1, 'autostashConflicts'), banner(2, 'autostashConflicts'), banner(3, 'recovery')]); return true; });
      run(branchDropRows(X, main, onX()), 'Merge feature/x into main');
      await vi.waitFor(() => expect(startIntegrate).toHaveBeenCalledOnce());
    });

    it('B behind A: Fast-forward main to feature/x checks out, then merges --ff-only', async () => {
      const rows = branchDropRows(X, main, onX({ isAncestor: (a, b) => a === 'm' && b === 'x' }));
      expect(labels(rows)).toEqual(['Fast-forward main to feature/x', 'Rebase main onto feature/x']);
      run(rows, 'Fast-forward main to feature/x');
      await vi.waitFor(() => expect(startIntegrate).toHaveBeenCalledWith(ctx, 'merge', 'feature/x', 'main', true));
      expect(checkout).toHaveBeenCalledOnce();
    });

    it('A already in B: the greyed merge row, then the interactive rebase that checks B out first', () => {
      const rows = branchDropRows(X, main, onX({ isAncestor: (a, b) => a === 'x' && b === 'm' }));
      expect(labels(rows)).toEqual(['Merge feature/x into main [main already contains feature/x]', 'Interactive rebase main onto feature/x']);
      expect(tooltips(rows)[1]).toMatch(/ Checks out main first\.$/);
    });

    it('B checked out in another worktree: every row greyed with where, "already contains" included', () => {
      const away: DroppedLabel = { row: row('m'), label: label('main', 0, { worktree: '/r-main', checkedOut: '/r-main' }) };
      const where = onX({ worktreeShown: (p) => `..${p}` });
      for (const isAncestor of [() => false, (a: string, b: string) => a === 'x' && b === 'm']) {
        const rows = branchDropRows(X, away, { ...where, isAncestor });
        expect(rows.length).toBeGreaterThan(0);
        expect(labels(rows).every((l) => l.endsWith(' [main is checked out in ../r-main]')), labels(rows).join()).toBe(true);
        run(rows, labels(rows)[0].replace(/ \[.*/, ''));
      }
      expect(checkout).not.toHaveBeenCalled();
    });

    it('on a detached HEAD, or with B in the active worktree as a queued checkout leaves it: the same rows', () => {
      const queued: DroppedLabel = { row: row('m'), label: label('main', 0, { checkedOut: '/r' }) };
      for (const [onto, over] of [[main, { headBranch: null, headSha: 'd' }], [queued, {}]] as const) {
        expect(labels(branchDropRows(X, onto, onX(over)))).toEqual(['Merge feature/x into main', 'Rebase main onto feature/x', 'Interactive rebase main onto feature/x']);
      }
    });

    it("an operation in progress greys B's rows with its reason, before the worktree's", () => {
      const away: DroppedLabel = { row: row('m'), label: label('main', 0, { worktree: '/r-main', checkedOut: '/r-main' }) };
      for (const onto of [main, away]) {
        const rows = branchDropRows(X, onto, onX({ inProgress: 'merge', headBranch: null }));
        expect(rows.length).toBeGreaterThan(0);
        expect(labels(rows).every((l) => l.endsWith(' [Finish or abort the merge first]')), labels(rows).join()).toBe(true);
      }
    });
  });
});
