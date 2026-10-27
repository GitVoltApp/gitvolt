import { ArrowRightLeft, FolderPlus, FolderX, SquarePlus } from 'lucide-react';
import { useAppState } from '../app/state';
import { tabWorktree } from '../app/tabs';
import { registerMenu, type MenuContribution } from '../menu/registry';
import type { CommitTarget, MenuEnv, SidebarTarget, WipTarget } from '../menu/menuEnv';
import type { MenuRow, Variant } from '../menu/types';
import { openWorktreeTab, setActiveWorktree } from './active';
import { openCreateWorktree } from './CreateWorktreeDialog';
import { removeWorktree, removeWorktreeAndBranch } from './remove';

const row = (r: Omit<Extract<MenuRow, { kind: 'action' }>, 'kind'>): MenuRow => ({ kind: 'action', ...r });

/** Switch to: not offered on the worktree that's already the active one. */
const switchRows = (id: string, label: string, path: string, env: MenuEnv): MenuRow[] => (path === env.activeWorktree ? [] : [row({
  id, label, icon: ArrowRightLeft, tooltip: `Make ${env.worktreeShown(path)} this tab's worktree (its WIP, commit box and Undo)`,
  run: () => env.write && setActiveWorktree(env.write.tabId, path),
})]);
/** Open in a new tab: none for this tab's own worktree (it would only focus this tab); "Go to its
 * tab" when another tab already shows it (that's what opening it does). */
const openTabRows = (id: string, path: string, env: MenuEnv): MenuRow[] => {
  if (path === env.activeWorktree) return [];
  const other = useAppState.getState().profile.tabs.find((t) => t.id !== env.write?.tabId && t.kind === 'repo' && tabWorktree(t) === path);
  return [row({
    id, label: other ? 'Go to its tab' : 'Open in a new tab', icon: SquarePlus,
    tooltip: other ? `Switch to the tab already showing ${env.worktreeShown(path)}` : `Open ${env.worktreeShown(path)} in its own tab (the same repository, already loaded)`,
    run: () => { if (env.write) void openWorktreeTab(env.write.tabId, path); },
  })];
};

/** The sidebar worktree row (spec #2 §11.2, §14): Switch to, Open in a new tab. Remove is below. */
const sidebarRows: MenuContribution<SidebarTarget, MenuEnv> = {
  id: 'sidebar.worktree', kind: 'sidebar', group: 'worktree', order: 0,
  when: (t) => t.what === 'worktree',
  rows: (t, env) => (t.what === 'worktree' ? [...switchRows('sidebar.worktree.switch', 'Switch to', t.path, env), ...openTabRows('sidebar.worktree.openTab', t.path, env)] : []),
};

/** A WIP row (§14): Switch to this worktree and Open in a new tab, neither on the active one's;
 * Remove is below. */
const wipRows: MenuContribution<WipTarget, MenuEnv> = {
  id: 'wip.worktree', kind: 'wip', group: 'worktree', order: 0,
  rows: (t, env) => [...(t.active ? [] : switchRows('wip.switch', 'Switch to this worktree', t.worktree, env)), ...openTabRows('wip.openTab', t.worktree, env)],
};

export const offWorktreeMenus: Array<() => void> = [registerMenu(sidebarRows), registerMenu(wipRows)];

// --- 2C T14 ---
/** Its local branch, with the tip a delete expects (from the sidebar): none when detached. */
const branchOf = (name: string | null, env: MenuEnv) => {
  const l = name ? env.sidebar?.locals.find((b) => b.name === name.replace(/^refs\/heads\//, '')) : undefined;
  return l ? { name: l.name, oid: l.target } : null;
};
const lockedWhy = 'Locked: unlock it first';

/** `Remove worktree | Worktree | With branch |` (#9): the branch variant also deletes its local
 * branch (none for a detached worktree). Never on the main worktree; greyed for a locked one,
 * with the reason (unlocking it makes it removable). */
function removeRows(id: string, path: string, branchName: string | null, env: MenuEnv): MenuRow[] {
  const w = env.sidebar?.worktrees.find((x) => x.path === path);
  if (!env.write || w?.isMain || path === env.mainWorktree) return [];
  const ctx = env.write;
  const shown = env.worktreeShown(path);
  const name = w?.branch ?? branchName;
  const branch = branchOf(name, env);
  const off = w?.locked ? { disabledReason: lockedWhy } : {};
  const tooltip = `Delete ${shown}'s directory (its branch stays)`;
  const alone = () => { void removeWorktree(ctx, path, name); };
  const variants: Variant[] | undefined = branch ? [
    { id: 'worktree', label: 'Worktree', tooltip, run: alone, ...off },
    { id: 'branch', label: 'With branch', tooltip: `Remove ${shown} and delete its branch ${branch.name} (you can undo deleting the branch)`, run: () => { void removeWorktreeAndBranch(ctx, path, branch); }, ...off },
  ] : undefined;
  return [row({ id, label: 'Remove worktree', icon: FolderX, tooltip, run: alone, variants, defaultVariant: branch ? 'worktree' : undefined, ...off })];
}

const sidebarRemoveRows: MenuContribution<SidebarTarget, MenuEnv> = {
  id: 'sidebar.worktree.remove', kind: 'sidebar', group: 'worktree', order: 10,
  when: (t) => t.what === 'worktree',
  rows: (t, env) => (t.what === 'worktree' ? removeRows('sidebar.worktree.remove', t.path, t.branch, env) : []),
};

/** The WIP row's Remove: any linked worktree's, the active one's included. */
const wipRemoveRows: MenuContribution<WipTarget, MenuEnv> = {
  id: 'wip.worktree.remove', kind: 'wip', group: 'worktree', order: 10,
  rows: (t, env) => removeRows('wip.remove', t.worktree, null, env),
};

/** A branch label whose local branch a linked worktree has checked out (#9): its Delete is greyed,
 * so this row, right after it, removes that worktree and deletes the branch. */
const branchRemoveRows: MenuContribution<CommitTarget, MenuEnv> = {
  id: 'branch.removeWorktree', kind: 'commit', group: 'manage', order: 1,
  when: (t, env) => !!t.branch?.local && !!env.write,
  rows: (t, env) => {
    const local = env.sidebar?.locals.find((b) => b.fullName === t.branch!.local);
    const w = local?.checkedOut ? env.sidebar?.worktrees.find((x) => x.path === local.checkedOut) : undefined;
    if (!local || !w || w.isMain || w.path === env.mainWorktree) return [];
    const ctx = env.write!;
    const shown = env.worktreeShown(w.path);
    return [row({
      id: 'branch.removeWorktree', label: 'Delete with its worktree', icon: FolderX, tooltip: `Remove ${shown}, then delete ${local.name} (you can undo deleting the branch)`,
      run: () => { void removeWorktreeAndBranch(ctx, w.path, { name: local.name, oid: local.target }); }, disabledReason: w.locked ? `${shown} is locked: unlock it first` : undefined,
    })];
  },
};

/** Create worktree from ▸ (core §7 Branch group): each branch at the commit that no worktree
 * has, a remote-only one (a new tracking branch), then a new branch here. */
const createRows: MenuContribution<CommitTarget, MenuEnv> = {
  id: 'branch.createWorktree', kind: 'commit', group: 'branch', order: 10,
  when: (t, env) => !t.isWip && !t.isStash && !!env.write,
  rows: (t, env) => {
    const tabId = env.write!.tabId;
    const sub: MenuRow[] = env.labelsAt(t.sha).flatMap((l): MenuRow[] => {
      if (l.tag) return [];
      if (l.local) {
        // A branch checked out in a worktree can't get another one: not listed.
        if (l.checkedOut) return [];
        const name = l.local.replace(/^refs\/heads\//, '');
        return [row({ id: `wt:${l.local}`, label: name, icon: FolderPlus, tooltip: `A worktree on ${name}`, run: () => openCreateWorktree({ tabId, at: t.sha, branch: { kind: 'existing', name } }) })];
      }
      const r = l.remotes[0];
      if (!r) return [];
      const branch = r.fullName.slice(`refs/remotes/${r.remote}/`.length);
      return [row({ id: `wt:${r.fullName}`, label: `${r.remote}/${branch}`, icon: FolderPlus, tooltip: `A worktree on a new ${branch} tracking ${r.remote}/${branch}`, run: () => openCreateWorktree({ tabId, at: t.sha, branch: { kind: 'remote', remote: r.remote, branch, name: branch } }) })];
    });
    const fresh = row({ id: 'wt:new', label: 'New branch here…', icon: FolderPlus, tooltip: `A worktree on a new branch at ${t.sha.slice(0, 7)}`, run: () => openCreateWorktree({ tabId, at: t.sha, branch: null }) });
    return [{ kind: 'submenu', id: 'branch.createWorktree', label: 'Create worktree from', icon: FolderPlus, tooltip: 'Check out a branch in a new directory', rows: [...sub, ...(sub.length ? [{ kind: 'separator' as const }] : []), fresh] }];
  },
};

offWorktreeMenus.push(registerMenu(sidebarRemoveRows), registerMenu(wipRemoveRows), registerMenu(branchRemoveRows), registerMenu(createRows));
// --- end 2C T14 ---
