import { ChevronsDownUp, Eye, EyeOff, FoldVertical } from 'lucide-react';
import { useRuntime } from '../app/runtime';
import type { CommitTarget, MenuEnv, SidebarTarget, TagTarget } from '../menu/menuEnv';
import { registerMenu } from '../menu/registry';
import { branchViewOf, cannotHide, tagHidden, toggleFolder, unitNameOf } from './altClick';
import { collapse, hideBranch, hideTag, isExpanded, showBranch, showTag } from './store';

/** Hide branch / Show branch in a branch label's menu: the graph chip's, and the sidebar branch
 * row's (the same `commit` menu with its `branch` set). Hidden branches stay listed in the
 * sidebar, which is where Show is usually found. Collapse branch, on a branch expanded this
 * session; Fold merged branch, in a merge's menu when its fold is expanded. Hide folder / Show
 * folder in a sidebar branch or tag folder's menu. Hide tag / Show tag in a tag's menu (its chip's
 * and its sidebar row's). */
export const offBranchPresetMenus = [
  registerMenu<CommitTarget, MenuEnv>({
    id: 'presets.hideBranch', kind: 'commit', group: 'view', order: 10,
    when: (t, env) => !!t.branch && !!env.write && unitNameOf(t.branch) !== null,
    rows: (t, env) => {
      const name = unitNameOf(t.branch!)!;
      const tabId = env.write!.tabId;
      const graph = useRuntime.getState().tabs[tabId]?.graph;
      const view = graph?.view;
      if (branchViewOf(view, name)?.state === 'hidden') {
        return [{ kind: 'action', id: 'presets.showBranch', label: 'Show branch', icon: Eye, tooltip: `Show ${name} in the graph again`, run: () => showBranch(tabId, name) }];
      }
      const why = cannotHide(view, name, env.headBranch, graph?.pinnedRefs);
      return [{ kind: 'action', id: 'presets.hideBranch', label: 'Hide branch', icon: EyeOff, tooltip: why ?? `Hide ${name}'s commits from the graph`, run: () => hideBranch(tabId, name), disabledReason: why ?? undefined }];
    },
  }),
  registerMenu<SidebarTarget, MenuEnv>({
    id: 'presets.hideFolder', kind: 'sidebar', group: 'view', order: 10,
    when: (t, env) => t.what === 'folder' && !!env.write,
    rows: (t, env) => {
      if (t.what !== 'folder') return [];
      const tabId = env.write!.tabId;
      const [what, all] = t.tags ? ['tag', 'tags'] : ['branch', 'branches'];
      return t.hidden
        ? [{ kind: 'action', id: 'presets.showFolder', label: 'Show folder', icon: Eye, tooltip: `Show the ${all} under ${t.folder} in the graph again`, run: () => toggleFolder(tabId, t.folder, true, t.tags) }]
        : [{ kind: 'action', id: 'presets.hideFolder', label: 'Hide folder', icon: EyeOff, tooltip: `Hide every ${what} under ${t.folder} from the graph, also those created later`, run: () => toggleFolder(tabId, t.folder, false, t.tags) }];
    },
  }),
  // A tag's chip and sidebar row: no rule decides a tag, so only a hand pick hides it.
  registerMenu<TagTarget, MenuEnv>({
    id: 'presets.hideTag', kind: 'tag', group: 'view', order: 10,
    when: (_t, env) => !!env.write,
    rows: (t, env) => {
      const tabId = env.write!.tabId;
      return tagHidden(useRuntime.getState().tabs[tabId]?.graph?.view, t.name)
        ? [{ kind: 'action', id: 'presets.showTag', label: 'Show tag', icon: Eye, tooltip: `Show ${t.name} in the graph again`, run: () => showTag(tabId, t.name) }]
        : [{ kind: 'action', id: 'presets.hideTag', label: 'Hide tag', icon: EyeOff, tooltip: `Hide ${t.name}'s label from the graph, and the commits only it keeps`, run: () => hideTag(tabId, t.name) }];
    },
  }),
  // Undoing "Expand in graph" (or going to a commit the preset hid): the preset decides again.
  registerMenu<CommitTarget, MenuEnv>({
    id: 'presets.collapseBranch', kind: 'commit', group: 'view', order: 11,
    when: (t, env) => !!t.branch && !!env.write && unitNameOf(t.branch) !== null && isExpanded(env.write.tabId, `branch:${unitNameOf(t.branch)}`),
    rows: (t, env) => {
      const name = unitNameOf(t.branch!)!;
      const tabId = env.write!.tabId;
      return [{ kind: 'action', id: 'presets.collapseBranch', label: 'Collapse branch', icon: ChevronsDownUp, tooltip: `Show ${name} as the branch preset has it again`, run: () => void collapse(tabId, `branch:${name}`) }];
    },
  }),
  registerMenu<CommitTarget, MenuEnv>({
    id: 'presets.foldMerge', kind: 'commit', group: 'view', order: 12,
    when: (t, env) => !t.isWip && !!env.write && isExpanded(env.write.tabId, `fold:${t.sha}`),
    rows: (t, env) => {
      const tabId = env.write!.tabId;
      return [{ kind: 'action', id: 'presets.foldMerge', label: 'Fold merged branch', icon: FoldVertical, tooltip: 'Fold the branch this commit merged back into it', run: () => void collapse(tabId, `fold:${t.sha}`) }];
    },
  }),
];
import.meta.hot?.dispose(() => { for (const off of offBranchPresetMenus) off(); });
