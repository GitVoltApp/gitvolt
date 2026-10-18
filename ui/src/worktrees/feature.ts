import { ArrowRightLeft, FolderPlus } from 'lucide-react';
import { activeTab, registerActions } from '../app/actions';
import { tabIdOf } from '../app/tabStores';
import type { RefLabel } from '../api/gen/RefLabel';
import type { RowPayload } from '../api/gen/RowPayload';
import { registerGraphDoubleClick } from '../graph/rowActions';
import { detachedWorktreeOf } from '../graph/worktreeLabel';
import type { RepoViewStore } from '../repo/store';
import { useRuntime } from '../app/runtime';
import { registerSidebarDoubleClick, registerSidebarHeaderAction } from '../sidebar/itemActions';
import { useToast } from '../ui/toastStore';
import { openRepoMenu } from '../toolbar/RepoButton';
import { activeWorktreeOf, setActiveWorktree } from './active';
import { offCreateWorktreeDialog, openCreateWorktree } from './CreateWorktreeDialog';
import { offWorktreeMenus } from './menus';

function switchToChip(store: RepoViewStore, _row: RowPayload, label: RefLabel): boolean {
  const tabId = tabIdOf(store);
  const path = detachedWorktreeOf(label);
  if (!tabId || !path || path === activeWorktreeOf(tabId)) return false;
  setActiveWorktree(tabId, path);
  return true;
}

/** The active worktree's entry points (spec #2 §11.2): double-click a sidebar worktree row or
 * another worktree's WIP row or detached-HEAD chip, their menus, the toolbar repo button's list, the palette. */
const offs = [
  ...offWorktreeMenus,
  offCreateWorktreeDialog,
  registerSidebarHeaderAction('worktrees', {
    id: 'worktree.create', icon: FolderPlus, label: 'Create worktree',
    run: ({ tabId }) => {
      const head = useRuntime.getState().tabs[tabId]?.graph?.head.target;
      if (head) openCreateWorktree({ tabId, at: head, branch: null });
      else useToast.getState().show('Nothing to create a worktree from: this repository has no commits yet');
    },
  }),
  registerSidebarDoubleClick('worktree', ({ tabId }, item) => { if (item.kind === 'worktree') setActiveWorktree(tabId, item.worktree.path); }),
  // A worktree's detached-HEAD chip: a double-click makes it the active one (a click selects the row).
  registerGraphDoubleClick({ label: switchToChip }),
  registerGraphDoubleClick({
    row: (store, row) => {
      const tabId = tabIdOf(store);
      const path = row.wip?.worktreePath;
      if (!tabId || !path || path === activeWorktreeOf(tabId)) return false;
      setActiveWorktree(tabId, path);
      return true;
    },
  }),
  registerActions([{
    id: 'worktree.switch', label: 'Switch worktree…', group: 'Repository', icon: ArrowRightLeft, tooltip: "Change the tab's active worktree",
    when: () => { const t = activeTab(); return t?.kind === 'repo' && !!t.path; },
    run: () => { const t = activeTab(); if (t) openRepoMenu(t.id); },
  }]),
];
import.meta.hot?.dispose(() => { for (const off of offs) off(); });
