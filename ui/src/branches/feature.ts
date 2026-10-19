import { GitBranchPlus } from 'lucide-react';
import { activeRuntime, activeTab, registerActions } from '../app/actions';
import { worktreeOf } from '../app/runtime';
import { predictedHeadOf } from '../pending/head';
import { registerToolbarButton } from '../toolbar/registry';
import { createBranchAt } from './create';
import { offBranchMenus } from './menus';
import { offUpstreamPicker } from './upstream';

/** Branches (spec #2 §9): the toolbar Branch button, and the branch rows of the commit and
 * branch-label menus. Checkout and reset register from their own modules. */
const writeCtx = () => {
  const t = activeTab();
  const rt = activeRuntime();
  const worktree = worktreeOf(rt ?? undefined);
  return t?.kind === 'repo' && rt?.repo && worktree ? { tabId: t.id, repoId: rt.repo.id, worktree } : null;
};

const offs = [
  ...offBranchMenus,
  offUpstreamPicker,
  registerActions([{
    id: 'branch.create', label: 'Branch', group: 'Repository', icon: GitBranchPlus, tooltip: 'Create a branch at HEAD', shortcuts: ['Mod+Shift+N'],
    when: () => !!writeCtx() && !!activeRuntime()?.graph?.head.target,
    run: () => {
      const ctx = writeCtx();
      // HEAD once the queued writes ran: the branch starts where they leave it.
      const head = ctx ? predictedHeadOf(ctx.tabId) : null;
      if (ctx && head?.sha) void createBranchAt(ctx, { sha: head.sha, ref: head.branch && `refs/heads/${head.branch}` });
    },
  }]),
  registerToolbarButton({ action: 'branch.create', label: 'Branch', order: 30 }),
];
import.meta.hot?.dispose(() => { for (const off of offs) off(); });
