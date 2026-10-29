import { Boxes, FolderOpen, FolderSync, Link2, Plus, SquarePlus, Trash2 } from 'lucide-react';
import type { SubmoduleItem } from '../api/gen/SubmoduleItem';
import { joinPath } from '../app/osPath';
import { activeTab, registerActions } from '../app/actions';
import type { FileTarget, MenuEnv, SidebarTarget } from '../menu/menuEnv';
import { registerMenu } from '../menu/registry';
import type { MenuRow } from '../menu/types';
import { COMMIT_QUEUED, stagingKey, useStaging } from '../stage/store';
import { useToast } from '../ui/toastStore';
import type { WriteCtx } from '../write/client';
import { writeCtx } from '../write/ctx';
import { registerSidebarDoubleClick, registerSidebarHeaderAction } from '../sidebar/itemActions';
import { offAddSubmoduleDialog, openAddSubmodule } from './AddSubmoduleDialog';
import { openSubmodule, pickSubmodule, removeSubmodule, syncSubmodules, updateSubmodules } from './commands';
import { supportOf, unmappedReason } from './support';

/**
 * Submodules, only in a worktree that uses them (`RepoInfoPayload.submodules`): palette actions,
 * a submodule row's menu in the file lists, and the sidebar's Submodules panel as GitKraken has
 * it: a row per submodule (double-click opens it, or initialises one not checked out yet), its
 * menu (Open, Update or Initialise, Open in file manager, Remove; the Copy rows are `menu/builders`'), and the panel's + to add
 * one. Add submodule is in the palette in any repository tab, so the first one can be added.
 */
const ctx = () => {
  const t = activeTab();
  return t?.kind === 'repo' ? writeCtx(t.id) : null;
};
const usable = () => {
  const c = ctx();
  return c !== null && supportOf(c.tabId, c.worktree) !== undefined;
};

/** Runs `f` in the active worktree, or says why git's submodule commands can't run there. */
const inWorktree = (f: (c: WriteCtx) => Promise<void>) => async () => {
  const c = ctx();
  const s = c && supportOf(c.tabId, c.worktree);
  if (!c || !s) return;
  const why = unmappedReason(s);
  if (why) useToast.getState().show(why, { tone: 'warning' });
  else await f(c);
};

const row = (r: Omit<Extract<MenuRow, { kind: 'action' }>, 'kind'>): MenuRow => ({ kind: 'action', ...r });

/** A gitlink's row: one side of it is a submodule commit. */
export const isSubmoduleRow = (t: FileTarget) => t.diff.old.kind === 'submodule' || t.diff.new.kind === 'submodule';

/** A submodule row's menu (any list): Open in a tab; in a working-tree list, Update this
 * submodule. Nothing for a removed one or any other row. */
export function submoduleRows(t: FileTarget, env: Pick<MenuEnv, 'write' | 'inProgress'>): MenuRow[] {
  if (!isSubmoduleRow(t) || t.deleted) return [];
  const open = row({ id: 'file.submodule.open', label: 'Open in a tab', icon: SquarePlus, tooltip: `Open the submodule ${t.path} as a repository tab`, run: () => void openSubmodule(t.root, t.path) });
  const worktree = t.wip?.worktree ?? (t.list === 'worktree' ? t.root : null);
  if (!env.write || worktree === null) return [open];
  const c = { ...env.write, worktree };
  const s = supportOf(c.tabId, worktree);
  // As its neighbours (Stage, Discard): not while a commit is queued, nor mid-merge or rebase (the
  // write refuses then).
  const disabledReason = (s && unmappedReason(s))
    || (useStaging.getState().committing[stagingKey(c.repoId, worktree)] ? COMMIT_QUEUED : undefined)
    || (env.inProgress ? `Finish or abort the ${env.inProgress} first` : undefined);
  return [open, row({ id: 'file.submodule.update', label: 'Update this submodule', icon: FolderSync, tooltip: `Check out ${t.path} at its recorded commit (git submodule update --init --recursive -- ${t.path})`, run: () => void updateSubmodules(c, t.path), disabledReason })];
}

/** Why a write from a submodule's menu can't run now: an unmapped gitlink, a queued commit or a
 * merge or rebase in progress (the write refuses then), as the file list's rows say. */
function blockedReason(env: Pick<MenuEnv, 'write' | 'inProgress'>): string | undefined {
  if (!env.write) return 'Not in a repository tab';
  const s = supportOf(env.write.tabId, env.write.worktree);
  return (s && unmappedReason(s))
    || (useStaging.getState().committing[stagingKey(env.write.repoId, env.write.worktree)] ? COMMIT_QUEUED : undefined)
    || (env.inProgress ? `Finish or abort the ${env.inProgress} first` : undefined);
}

/** A double-click on a Submodules row (and Enter): open it as a tab, or initialise one that has
 * nothing checked out yet, as GitKraken does. */
function activateSubmodule(tabId: string, m: SubmoduleItem): void {
  const c = writeCtx(tabId);
  if (!c) return;
  if (m.status !== 'uninitialized') {
    void openSubmodule(c.worktree, m.path);
    return;
  }
  const s = supportOf(c.tabId, c.worktree);
  const why = s && unmappedReason(s);
  if (why) useToast.getState().show(why, { tone: 'warning' });
  else void updateSubmodules(c, m.path);
}

/** A Submodules panel row's menu: Open in a tab, Update (Initialise when not checked out). */
export function sidebarSubmoduleRows(t: SidebarTarget, env: MenuEnv): MenuRow[] {
  if (t.what !== 'submodule' || !env.write) return [];
  const m = t.submodule;
  const c = env.write;
  const blocked = blockedReason(env);
  const uninit = m.status === 'uninitialized';
  const open = row({
    id: 'sidebar.submodule.open', label: 'Open in a tab', icon: SquarePlus, tooltip: `Open the submodule ${m.path} as a repository tab`,
    run: () => void openSubmodule(c.worktree, m.path), disabledReason: uninit ? 'Not initialised: Initialise checks it out' : undefined,
  });
  const update = uninit
    ? row({ id: 'sidebar.submodule.init', label: 'Initialise', icon: FolderSync, tooltip: `Clone ${m.path} and check out its recorded commit (git submodule update --init --recursive -- ${m.path})`, run: () => void updateSubmodules(c, m.path), disabledReason: blocked })
    : row({ id: 'sidebar.submodule.update', label: 'Update', icon: FolderSync, tooltip: `Check out ${m.path} at its recorded commit (git submodule update --init --recursive -- ${m.path})`, run: () => void updateSubmodules(c, m.path), disabledReason: blocked });
  return uninit ? [update, open] : [open, update];
}

const offs = [
  offAddSubmoduleDialog,
  registerSidebarHeaderAction('submodules', { id: 'submodule.add', icon: Plus, label: 'Add submodule', run: ({ tabId }) => openAddSubmodule(tabId) }),
  registerSidebarDoubleClick('submodule', ({ tabId }, item) => { if (item.kind === 'submodule') activateSubmodule(tabId, item.submodule); }),
  registerMenu<SidebarTarget, MenuEnv>({ id: 'sidebar.submodule', kind: 'sidebar', group: 'submodule', order: 0, when: (t) => t.what === 'submodule', rows: sidebarSubmoduleRows }),
  registerMenu<SidebarTarget, MenuEnv>({
    id: 'sidebar.submodule.reveal', kind: 'sidebar', group: 'open', order: 0,
    when: (t) => t.what === 'submodule' && t.submodule.status !== 'uninitialized',
    rows: (t, env) => (t.what === 'submodule' && env.write
      ? [row({ id: 'sidebar.submodule.reveal', label: 'Open in file manager', icon: FolderOpen, tooltip: "Show the submodule's directory in the file manager", run: () => env.act.openFolder(joinPath(env.write!.worktree, t.submodule.path)) })]
      : []),
  }),
  registerMenu<SidebarTarget, MenuEnv>({
    id: 'sidebar.submodule.remove', kind: 'sidebar', group: 'manage', order: 0,
    when: (t, env) => t.what === 'submodule' && !!env.write,
    rows: (t, env) => (t.what === 'submodule' && env.write
      ? [row({ id: 'sidebar.submodule.remove', label: 'Remove submodule…', icon: Trash2, tooltip: `Remove ${t.submodule.path}'s checkout and its .gitmodules entry, staged (git rm -- ${t.submodule.path})`, run: () => void removeSubmodule(env.write!, t.submodule.path), disabledReason: blockedReason(env) })]
      : []),
  }),
  registerActions([
    {
      id: 'submodule.update', label: 'Submodules: Update', group: 'Repository', section: 'Submodules', icon: FolderSync, shortcuts: ['Mod+Alt+B'],
      tooltip: 'Check out each submodule at its recorded commit, cloning any not initialised yet (git submodule update --init --recursive)',
      when: usable,
      run: inWorktree((c) => updateSubmodules(c)),
    },
    {
      id: 'submodule.sync', label: 'Submodules: Sync URLs', group: 'Repository', section: 'Submodules', icon: Link2,
      tooltip: "Copy the submodules' URLs from .gitmodules into the config (git submodule sync --recursive)",
      when: usable,
      run: inWorktree(syncSubmodules),
    },
    {
      id: 'submodule.open', label: 'Submodules: Open in a tab…', group: 'Repository', section: 'Submodules', icon: Boxes, shortcuts: ['Mod+Alt+O'],
      tooltip: 'Choose a submodule to open as a repository tab',
      when: usable,
      run: inWorktree(pickSubmodule),
    },
    {
      id: 'submodule.add', label: 'Submodules: Add…', group: 'Repository', section: 'Submodules', icon: Plus,
      tooltip: 'Clone a repository into this one as a submodule (git submodule add)',
      when: () => ctx() !== null,
      run: () => { const c = ctx(); if (c) openAddSubmodule(c.tabId); },
    },
  ]),
  registerMenu<FileTarget, MenuEnv>({ id: 'file.submodule', kind: 'file', group: 'submodule', order: 0, rows: submoduleRows }),
];
import.meta.hot?.dispose(() => { for (const off of offs) off(); });
