import { Eye, EyeOff } from 'lucide-react';
import { useRuntime } from '../app/runtime';
import type { CommitTarget, MenuEnv } from '../menu/menuEnv';
import { registerMenu } from '../menu/registry';
import { branchViewOf, cannotHide, unitNameOf } from './altClick';
import { hideBranch, showBranch } from './store';

/** Hide branch / Show branch in a branch label's menu: the graph chip's, and the sidebar branch
 * row's (the same `commit` menu with its `branch` set). Hidden branches stay listed in the
 * sidebar, which is where Show is usually found. */
export const offBranchPresetMenus = [
  registerMenu<CommitTarget, MenuEnv>({
    id: 'presets.hideBranch', kind: 'commit', group: 'view', order: 10,
    when: (t, env) => !!t.branch && !!env.write && unitNameOf(t.branch) !== null,
    rows: (t, env) => {
      const name = unitNameOf(t.branch!)!;
      const tabId = env.write!.tabId;
      const view = useRuntime.getState().tabs[tabId]?.graph?.view;
      if (branchViewOf(view, name)?.state === 'hidden') {
        return [{ kind: 'action', id: 'presets.showBranch', label: 'Show branch', icon: Eye, tooltip: `Show ${name} in the graph again`, run: () => showBranch(tabId, name) }];
      }
      const why = cannotHide(view, name, env.headBranch);
      return [{ kind: 'action', id: 'presets.hideBranch', label: 'Hide branch', icon: EyeOff, tooltip: why ?? `Hide ${name}'s commits from the graph`, run: () => hideBranch(tabId, name), disabledReason: why ?? undefined }];
    },
  }),
];
import.meta.hot?.dispose(() => { for (const off of offBranchPresetMenus) off(); });
