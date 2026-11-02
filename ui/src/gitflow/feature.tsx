import { CircleCheckBig, Flag, Plus, Rocket, Settings2, Sparkles, Workflow, Wrench } from 'lucide-react';
import type { GitflowKind } from '../api/gen/GitflowKind';
import { activeTab, registerActions } from '../app/actions';
import { useRuntime } from '../app/runtime';
import { registerMenu } from '../menu/registry';
import type { CommitTarget, MenuEnv } from '../menu/menuEnv';
import { registerSettings } from '../settings/registry';
import { registerSidebarHeaderAction } from '../sidebar/itemActions';
import { flowOf } from './actions';
import { offGitflowDialog, openGitflowDialog } from './GitflowDialog';
import { classify, defaultConfig, KIND_LABEL } from './model';

/** Git flow (GitKraken's, after `git flow`): Init from the palette or the Repository settings, a
 * Git Flow sidebar panel once it's set up (its + starts a branch), and Finish on a feature,
 * release or hotfix branch's menu (its sidebar row, its graph label) or the palette. */
const repoTabId = (): string | null => {
  const t = activeTab();
  return t?.kind === 'repo' && useRuntime.getState().tabs[t.id]?.sidebar ? t.id : null;
};

/** The settings dialog: Init's suggestion before it's set up, the current settings after. */
export function openSettings(tabId: string): void {
  const flow = flowOf(tabId);
  const locals = useRuntime.getState().tabs[tabId]?.sidebar?.locals.map((b) => b.name) ?? [];
  openGitflowDialog({ kind: 'init', tabId, initial: flow ?? defaultConfig(locals), editing: !!flow });
}

/** The checked-out branch, when it's one of git flow's. */
const headFlowBranch = (tabId: string): string | null => {
  const flow = flowOf(tabId);
  const head = useRuntime.getState().tabs[tabId]?.graph?.head.branch?.replace(/^refs\/heads\//, '');
  return flow && head && classify(flow, head) ? head : null;
};

const START_ICON: Record<GitflowKind, typeof Sparkles> = { feature: Sparkles, release: Rocket, hotfix: Wrench };

function GitflowSettingControl({ ctx }: { ctx: { tabId: string | null } }) {
  const set = useRuntime((s) => (ctx.tabId ? !!s.tabs[ctx.tabId]?.sidebar?.gitflow : false));
  const tabId = ctx.tabId;
  if (!tabId) return null;
  return <button type="button" id="input-gitflow" onClick={() => openSettings(tabId)}>{set ? 'Edit settings…' : 'Set up git flow…'}</button>;
}

const offs = [
  offGitflowDialog,
  registerSidebarHeaderAction('gitflow', {
    id: 'gitflow.start', icon: Plus, label: 'Start a feature, release or hotfix',
    run: ({ tabId }) => openGitflowDialog({ kind: 'start', tabId, flow: 'feature' }),
  }),
  registerMenu<CommitTarget, MenuEnv>({
    id: 'gitflow.finish', kind: 'commit', group: 'integrate', order: 90,
    when: (t, env) => !!t.branch?.local && !!env.write && !!env.sidebar?.gitflow,
    rows: (t, env) => {
      const name = t.branch!.local!.replace(/^refs\/heads\//, '');
      const flow = classify(env.sidebar!.gitflow!, name);
      if (!flow) return [];
      const label = KIND_LABEL[flow.kind].toLowerCase();
      return [{
        kind: 'action', id: 'gitflow.finish', label: `Finish ${label} ${flow.rest}…`, icon: Flag,
        tooltip: flow.kind === 'feature' ? `Merge ${name} into ${env.sidebar!.gitflow!.develop} and delete it` : `Merge ${name} into ${env.sidebar!.gitflow!.master} and ${env.sidebar!.gitflow!.develop}, tag it, and delete it`,
        run: () => openGitflowDialog({ kind: 'finish', tabId: env.write!.tabId, branch: name }),
      }];
    },
  }),
  registerActions([
    {
      id: 'gitflow.init', label: 'Set up git flow…', group: 'Repository', icon: Workflow, tooltip: 'Choose git flow\'s branches and prefixes for this repository',
      when: () => { const id = repoTabId(); return !!id && !flowOf(id); },
      run: () => { const id = repoTabId(); if (id) openSettings(id); },
    },
    {
      id: 'gitflow.settings', label: 'Git flow settings…', group: 'Repository', icon: Settings2, tooltip: 'Change git flow\'s branches and prefixes for this repository',
      when: () => { const id = repoTabId(); return !!id && !!flowOf(id); },
      run: () => { const id = repoTabId(); if (id) openSettings(id); },
    },
    ...(['feature', 'release', 'hotfix'] as const).map((k) => ({
      id: `gitflow.start.${k}`, label: `Start ${KIND_LABEL[k].toLowerCase()}…`, group: 'Repository' as const, icon: START_ICON[k],
      tooltip: k === 'hotfix' ? `Start a git flow hotfix from the production branch` : `Start a git flow ${k} from the development branch`,
      when: () => { const id = repoTabId(); return !!id && !!flowOf(id); },
      run: () => { const id = repoTabId(); if (id) openGitflowDialog({ kind: 'start', tabId: id, flow: k }); },
    })),
    {
      id: 'gitflow.finish', label: 'Finish the checked-out git flow branch…', group: 'Repository', icon: CircleCheckBig, tooltip: 'Finish the feature, release or hotfix that\'s checked out',
      when: () => { const id = repoTabId(); return !!id && !!headFlowBranch(id); },
      run: () => { const id = repoTabId(); const branch = id && headFlowBranch(id); if (id && branch) openGitflowDialog({ kind: 'finish', tabId: id, branch }); },
    },
  ]),
  registerSettings([{
    id: 'gitflow', label: 'Git flow', page: 'repository', group: 'Branches',
    keywords: 'gitflow git-flow feature release hotfix develop production prefix version tag',
    description: 'Its production and development branches, and the prefixes of its feature, release and hotfix branches.',
    Control: GitflowSettingControl,
  }]),
];
import.meta.hot?.dispose(() => { for (const off of offs) off(); });
