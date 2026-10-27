import { describe, expect, it, vi } from 'vitest';
import { useAppState } from '../app/state';
import { buildMenu } from '../menu/registry';
import type { CommitTarget, MenuEnv, SidebarTarget, WipTarget } from '../menu/menuEnv';
import type { MenuRow } from '../menu/types';

const setActiveWorktree = vi.hoisted(() => vi.fn());
vi.mock('./active', () => ({ setActiveWorktree, openWorktreeTab: vi.fn() }));
const remove = vi.hoisted(() => ({ removeWorktree: vi.fn(), removeWorktreeAndBranch: vi.fn() }));
vi.mock('./remove', () => remove);
await import('./menus');

const env = (activeWorktree: string) => ({ activeWorktree, mainWorktree: '/r', write: { tabId: 't', repoId: 1, worktree: activeWorktree }, worktreeShown: (p: string) => p.replace('/r-', '../r-') }) as unknown as MenuEnv;
const ids = (rows: ReturnType<typeof buildMenu>) => rows.flatMap((r) => (r.kind === 'action' ? [r.id] : []));

describe('worktree rows (spec #2 §14)', () => {
  it('a sidebar worktree row offers Switch to and Open in a new tab', () => {
    const t: SidebarTarget = { what: 'worktree', path: '/r-x', branch: 'x', head: 'b' };
    const rows = buildMenu('sidebar', t, env('/r'));
    expect(ids(rows)).toEqual(expect.arrayContaining(['sidebar.worktree.switch', 'sidebar.worktree.openTab']));
    const sw = rows.find((r) => r.kind === 'action' && r.id === 'sidebar.worktree.switch');
    if (sw?.kind === 'action') sw.run();
    expect(setActiveWorktree).toHaveBeenCalledWith('t', '/r-x');
    // The active one has nothing to switch to: no row.
    expect(ids(buildMenu('sidebar', t, env('/r-x')))).not.toContain('sidebar.worktree.switch');
  });
  it('a WIP row offers Switch to this worktree unless it is the active one', () => {
    const other: WipTarget = { worktree: '/r-x', name: 'r-x', active: false };
    expect(ids(buildMenu('wip', other, env('/r')))).toEqual(['wip.switch', 'wip.openTab', 'wip.remove']);
    // This tab's own worktree: nothing to switch to, and a new tab would only focus this one.
    expect(ids(buildMenu('wip', { ...other, active: true }, env('/r-x')))).toEqual(['wip.remove']);
  });

  it('Open in a new tab says Go to its tab when another tab already shows that worktree', () => {
    const other: WipTarget = { worktree: '/r-x', name: 'r-x', active: false };
    const label = () => { const r = buildMenu('wip', other, env('/r')).find((x) => x.kind === 'action' && x.id === 'wip.openTab'); return r?.kind === 'action' ? r.label : null; };
    useAppState.setState((s) => ({ profile: { ...s.profile, tabs: [{ id: 't', kind: 'repo', path: '/r', alias: null, worktree: null }] } }));
    expect(label()).toBe('Open in a new tab');
    useAppState.setState((s) => ({ profile: { ...s.profile, tabs: [...s.profile.tabs, { id: 'u', kind: 'repo', path: '/r', alias: null, worktree: '/r-x' }] } }));
    expect(label()).toBe('Go to its tab');
  });

  it('Remove is not offered on the main worktree, and greyed for a locked one', () => {
    const rows = (path: string, locked = false) => buildMenu('sidebar', { what: 'worktree', path, branch: 'x', head: 'b' } as SidebarTarget, { ...env('/r'), sidebar: { locals: [], worktrees: [{ path: '/r', isMain: true, locked: false }, { path, isMain: path === '/r', locked }] } } as unknown as MenuEnv);
    const remove = (r: ReturnType<typeof buildMenu>) => r.find((x) => x.kind === 'action' && x.id === 'sidebar.worktree.remove');
    const reason = (r: ReturnType<typeof buildMenu>) => { const x = remove(r); return x?.kind === 'action' ? x.disabledReason ?? null : 'missing'; };
    expect(reason(rows('/r'))).toBe('missing');
    expect(reason(rows('/r-x', true))).toBe('Locked: unlock it first');
    expect(reason(rows('/r-x'))).toBeNull();
  });

  describe('Remove worktree | Worktree | With branch | (#9)', () => {
    const worktrees = [{ path: '/r', branch: 'main', isMain: true, locked: false }, { path: '/r-x', branch: 'x', isMain: false, locked: false }, { path: '/r-d', branch: null, isMain: false, locked: false }, { path: '/r-l', branch: 'l', isMain: false, locked: true }];
    const locals = [{ name: 'main', fullName: 'refs/heads/main', target: 'm1', checkedOut: '/r' }, { name: 'x', fullName: 'refs/heads/x', target: 'x1', checkedOut: '/r-x' }, { name: 'l', fullName: 'refs/heads/l', target: 'l1', checkedOut: '/r-l' }, { name: 'y', fullName: 'refs/heads/y', target: 'y1', checkedOut: null }];
    const full = (active = '/r') => ({ ...env(active), labelsAt: () => [], sidebar: { locals, worktrees } }) as unknown as MenuEnv;
    const find = (rows: MenuRow[], id: string) => { const r = rows.find((x) => x.kind === 'action' && x.id === id); return r?.kind === 'action' ? r : undefined; };
    const wip = (worktree: string, active = '/r') => find(buildMenu('wip', { worktree, name: null, active: worktree === active }, full(active)), 'wip.remove');
    const side = (path: string, branch: string | null) => find(buildMenu('sidebar', { what: 'worktree', path, branch, head: 'b' } as SidebarTarget, full()), 'sidebar.worktree.remove');

    it('the WIP row of a linked worktree has it, the active one included; the main one never', () => {
      expect(wip('/r')).toBeUndefined();
      expect(wip('/r', '/r-x')).toBeUndefined();
      expect(wip('/r-x')?.label).toBe('Remove worktree');
      expect(wip('/r-x', '/r-x')?.label).toBe('Remove worktree');
    });
    it('Worktree (the default) removes it alone; With branch removes it and deletes its branch', () => {
      for (const r of [wip('/r-x')!, side('/r-x', 'x')!]) {
        expect(r.defaultVariant).toBe('worktree');
        expect(r.variants?.map((v) => [v.id, v.label])).toEqual([['worktree', 'Worktree'], ['branch', 'With branch']]);
        expect(r.variants![1].tooltip).toBe('Remove ../r-x and delete its branch x (you can undo deleting the branch)');
        remove.removeWorktree.mockClear();
        r.run();
        r.variants![0].run();
        expect(remove.removeWorktree.mock.calls).toEqual([[expect.anything(), '/r-x', 'x'], [expect.anything(), '/r-x', 'x']]);
        remove.removeWorktreeAndBranch.mockClear();
        r.variants![1].run();
        expect(remove.removeWorktreeAndBranch).toHaveBeenCalledWith({ tabId: 't', repoId: 1, worktree: '/r' }, '/r-x', { name: 'x', oid: 'x1' });
      }
    });
    it('a detached worktree has no branch variant; a locked one is greyed, variants and all', () => {
      expect(wip('/r-d')?.variants).toBeUndefined();
      expect(side('/r-d', null)?.variants).toBeUndefined();
      const locked = wip('/r-l')!;
      expect(locked.disabledReason).toBe('Locked: unlock it first');
      expect(locked.variants?.map((v) => v.disabledReason)).toEqual(['Locked: unlock it first', 'Locked: unlock it first']);
    });
    it("a branch label checked out in a linked worktree offers Delete with its worktree; elsewhere, none", () => {
      const label = (name: string, active = '/r') => {
        const t: CommitTarget = { sha: 's', mrRefs: [], isWip: false, isStash: false, branch: { name, local: `refs/heads/${name}`, remotes: [] } };
        return find(buildMenu('commit', t, full(active)), 'branch.removeWorktree');
      };
      const x = label('x')!;
      expect(x.label).toBe('Delete with its worktree');
      expect(x.disabledReason).toBeUndefined();
      remove.removeWorktreeAndBranch.mockClear();
      x.run();
      expect(remove.removeWorktreeAndBranch).toHaveBeenCalledWith({ tabId: 't', repoId: 1, worktree: '/r' }, '/r-x', { name: 'x', oid: 'x1' });
      // On the active linked worktree too; never for the main one's branch, nor one not checked out.
      expect(label('x', '/r-x')).toBeDefined();
      expect(label('main')).toBeUndefined();
      expect(label('y')).toBeUndefined();
      expect(label('l')?.disabledReason).toBe('../r-l is locked: unlock it first');
    });
  });
});
