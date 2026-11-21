import { beforeEach, describe, expect, it, vi } from 'vitest';

const op = vi.hoisted(() => ({ inProgress: null as string | null, open: null as { staged: boolean } | null }));
vi.mock('./actions', async (orig) => ({ ...(await orig<typeof import('./actions')>()), stashPathsFor: vi.fn(), addToStashFor: vi.fn() }));
vi.mock('../stage/keyActions', () => ({
  openWipFile: () => (op.open ? { ctx: { tabId: 't', repoId: 1, worktree: '/r' }, staged: op.open.staged, file: { path: 'src/a.ts', oldPath: null } } : null),
}));
vi.mock('../app/tabStores', async (orig) => ({
  ...(await orig<typeof import('../app/tabStores')>()),
  tabStore: (id: string) => (id === 't' ? { getState: () => ({ graph: { worktrees: [{ path: '/r', inProgress: op.inProgress }] } }) } : undefined),
}));
import type { FileChange } from '../api/gen/FileChange';
import { getAction } from '../app/actions';
import { useRuntime } from '../app/runtime';
import type { CommitTarget, FileTarget, FolderTarget, MenuEnv, SidebarTarget, WipTarget } from '../menu/menuEnv';
import { buildMenu } from '../menu/registry';
import type { MenuRow } from '../menu/types';
import { COMMIT_QUEUED, stagingKey, useStaging } from '../stage/store';
import { addToStashFor, stashPathsFor } from './actions';
import { shortStashName } from './feature';

const env = { write: { tabId: 't', repoId: 1, worktree: '/r' } } as unknown as MenuEnv;
const ids = (rows: ReturnType<typeof buildMenu>) => rows.flatMap((r) => (r.kind === 'action' ? [r.id] : []));

describe('stash rows', () => {
  it('a sidebar stash row and a graph stash node get Apply, Pop, Delete', () => {
    const row: SidebarTarget = { what: 'stash', sha: 'oid', message: 'On main: x' };
    expect(ids(buildMenu('sidebar', row, env))).toEqual(expect.arrayContaining(['stash.apply', 'stash.pop', 'stash.drop']));
    const node: CommitTarget = { sha: 'oid', mrRefs: [], isWip: false, isStash: true, branch: null };
    expect(ids(buildMenu('commit', node, env)).filter((i) => i.startsWith('stash.'))).toEqual(['stash.apply', 'stash.pop', 'stash.drop']);
    expect(ids(buildMenu('commit', { ...node, isStash: false }, env)).filter((i) => i.startsWith('stash.'))).toEqual([]);
  });
  it('a WIP row offers Stash changes for its own worktree', () => {
    const t: WipTarget = { worktree: '/r-x', name: 'r-x', active: false };
    expect(ids(buildMenu('wip', t, env))).toContain('wip.stash');
  });

  describe("a WIP file's or folder's Stash", () => {
    type Action = Extract<MenuRow, { kind: 'action' }>;
    const find = (rows: MenuRow[], id: string) => rows.find((r): r is Action => r.kind === 'action' && r.id === id);
    const file = (wip: Partial<NonNullable<FileTarget['wip']>> | null, path = 'src/a.ts') =>
      ({ path, wip: wip && { worktree: '/r', staged: false, oldPath: null, status: 'M', ...wip } }) as unknown as FileTarget;
    const folder = (files: Array<Pick<FileChange, 'path' | 'status'>>, staged = false): FolderTarget =>
      ({ path: 'src', root: '/r', openIn: {} as FolderTarget['openIn'], wip: { worktree: '/r', conflicted: [], staged, files: files as FileChange[] } });
    beforeEach(() => {
      op.inProgress = null;
      useStaging.setState({ committing: {} });
      vi.mocked(stashPathsFor).mockClear();
    });

    it('an unstaged or staged file stashes its part in that list; not a conflicted one, a submodule or a commit’s file', () => {
      for (const [staged, label, part] of [[false, 'Stash unstaged changes', 'unstaged'], [true, 'Stash staged changes', 'staged']] as const) {
        const row = find(buildMenu('file', file({ staged }), env), 'file.stash');
        expect(row?.label).toBe(label);
        expect(row?.tooltip).toContain(`its ${staged ? 'unstaged' : 'staged'} changes and the other files stay`);
        expect(row?.disabledReason).toBeUndefined();
        row?.run();
        expect(stashPathsFor).toHaveBeenLastCalledWith({ tabId: 't', repoId: 1, worktree: '/r' }, ['src/a.ts'], part);
      }
      for (const t of [file({ status: 'U' }), file({ submodule: true }), file(null)]) expect(find(buildMenu('file', t, env), 'file.stash')).toBeUndefined();
    });

    it('a folder stashes its list’s part of everything under it; not while a file under it is conflicted', () => {
      for (const [staged, label, part] of [[false, 'Stash unstaged changes in src/', 'unstaged'], [true, 'Stash staged changes in src/', 'staged']] as const) {
        const row = find(buildMenu('folder', folder([{ path: 'src/a.ts', status: 'M' }, { path: 'src/b.ts', status: '?' }], staged), env), 'folder.stash');
        expect(row?.label).toBe(label);
        row?.run();
        expect(stashPathsFor).toHaveBeenLastCalledWith({ tabId: 't', repoId: 1, worktree: '/r' }, ['src'], part);
      }
      expect(find(buildMenu('folder', folder([{ path: 'src/a.ts', status: 'U' }]), env), 'folder.stash')).toBeUndefined();
      const mixed = find(buildMenu('folder', folder([{ path: 'src/a.ts', status: 'M' }, { path: 'src/b.ts', status: 'U' }]), env), 'folder.stash');
      expect(mixed?.disabledReason).toBe('Resolve the conflicted files in src/ first');
      const listed = folder([{ path: 'src/a.ts', status: 'M' }]);
      listed.wip!.conflicted = ['src/c.ts'];
      expect(find(buildMenu('folder', listed, env), 'folder.stash')?.disabledReason).toBe('Resolve the conflicted files in src/ first');
    });

    it('the shortcut stashes the open file’s part of the list it was opened from', () => {
      const action = getAction('stash.file')!;
      op.open = null;
      expect(action.when?.()).toBe(false);
      for (const [staged, part] of [[true, 'staged'], [false, 'unstaged']] as const) {
        op.open = { staged };
        expect(action.when?.()).toBe(true);
        action.run();
        expect(stashPathsFor).toHaveBeenLastCalledWith({ tabId: 't', repoId: 1, worktree: '/r' }, ['src/a.ts'], part);
      }
      op.open = null;
    });

    describe('Add to stash', () => {
      const stashes = (message: string | null) =>
        useRuntime.setState({ tabs: { t: { sidebar: message === null ? { stashes: [] } : { stashes: [{ index: 0, id: 's0', message, time: 0 }, { index: 1, id: 's1', message: 'On main: older', time: 0 }] } } } } as never);
      beforeEach(() => vi.mocked(addToStashFor).mockClear());

      it('offers the list’s part to the latest stash, by name, beside Stash; none without a stash', () => {
        stashes('On main: WIP on main: src/b.ts (staged)');
        for (const [staged, part] of [[false, 'unstaged'], [true, 'staged']] as const) {
          const rows = buildMenu('file', file({ staged }), env);
          expect(ids(rows).slice(ids(rows).indexOf('file.stash'), ids(rows).indexOf('file.stash') + 2)).toEqual(['file.stash', 'file.stashAdd']);
          const row = find(rows, 'file.stashAdd');
          expect(row?.label).toBe(`Add ${part} changes to "WIP on main: src/b.ts (staged)"`);
          expect(row?.tooltip).toBe(`Add this file’s ${part} changes to the latest stash, "WIP on main: src/b.ts (staged)", which keeps its name and its place (you can undo this)`);
          row?.run();
          expect(addToStashFor).toHaveBeenLastCalledWith({ tabId: 't', repoId: 1, worktree: '/r' }, ['src/a.ts'], part, 's0');
          const f = find(buildMenu('folder', folder([{ path: 'src/a.ts', status: 'M' }], staged), env), 'folder.stashAdd');
          expect(f?.label).toBe(`Add ${part} changes in src/ to "WIP on main: src/b.ts (staged)"`);
          f?.run();
          expect(addToStashFor).toHaveBeenLastCalledWith({ tabId: 't', repoId: 1, worktree: '/r' }, ['src'], part, 's0');
        }
        stashes(null);
        expect(find(buildMenu('file', file({}), env), 'file.stashAdd')).toBeUndefined();
        expect(find(buildMenu('folder', folder([{ path: 'src/a.ts', status: 'M' }]), env), 'folder.stashAdd')).toBeUndefined();
      });

      it('cuts a long name short', () => {
        expect(shortStashName('On main: Fix x')).toBe('Fix x');
        expect(shortStashName('On main: WIP on main: src/components/deeply/nested/file.ts (unstaged)')).toBe('WIP on main: src/components/dee…');
      });

      it('is off as Stash is: mid-merge, while a commit is queued, over a conflicted file', () => {
        stashes('On main: x');
        op.inProgress = 'rebase';
        expect(find(buildMenu('file', file({}), env), 'file.stashAdd')?.disabledReason).toBe('Finish or abort the rebase first');
        op.inProgress = null;
        useStaging.setState({ committing: { [stagingKey(1, '/r')]: true } });
        expect(find(buildMenu('file', file({ staged: true }), env), 'file.stashAdd')?.disabledReason).toBe(COMMIT_QUEUED);
        useStaging.setState({ committing: {} });
        const f = find(buildMenu('folder', folder([{ path: 'src/a.ts', status: 'M' }, { path: 'src/b.ts', status: 'U' }]), env), 'folder.stashAdd');
        expect(f?.disabledReason).toBe('Resolve the conflicted files in src/ first');
      });
    });

    it('is off mid-merge and while a commit is queued, as Stage and Discard are', () => {
      op.inProgress = 'merge';
      expect(find(buildMenu('file', file({}), env), 'file.stash')?.disabledReason).toBe('Finish or abort the merge first');
      op.inProgress = null;
      useStaging.setState({ committing: { [stagingKey(1, '/r')]: true } });
      expect(find(buildMenu('folder', folder([{ path: 'src/a.ts', status: 'M' }]), env), 'folder.stash')?.disabledReason).toBe(COMMIT_QUEUED);
    });
  });
});
