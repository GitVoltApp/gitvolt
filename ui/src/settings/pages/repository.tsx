import { FolderGit2 } from 'lucide-react';
import type { BranchUpdate } from '../../api/gen/BranchUpdate';
import type { EditorChoice } from '../../api/gen/EditorChoice';
import type { PinSetting } from '../../api/gen/PinSetting';
import { useRuntime } from '../../app/runtime';
import { flushSaves } from '../../app/state';
import { notifyForgeAccountsChanged } from '../../forge/accountsBus';
import { refreshPresetViews } from '../../presets/store';
import { Select } from '../../ui/Select';
import { EditorPicker } from '../EditorPicker';
import { repoField, useSettingValue } from '../bindings';
import { TextField } from '../frame/fields';
import { registerSettings, registerSettingsPage, settingDef, type SettingsCtx } from '../registry';

const short = (ref: string) => ref.replace(/^refs\/(remotes|heads)\//, '');
const pinKey = (pin: PinSetting | null | undefined) => (pin?.kind === 'ref' ? `ref:${pin.name}` : pin?.kind ?? 'auto');
const NONE: never[] = [];

function RepoEditorControl({ ctx }: { ctx: SettingsCtx }) {
  const [editor, set] = useSettingValue<EditorChoice | null>(settingDef('repoEditor')!, ctx);
  return <EditorPicker id="repoEditor" value={editor} inherit onChange={set} />;
}

/** The same `RepoSettings.pin` the graph header's pin button writes: Automatic, Off or a local branch. */
function PinControl({ ctx }: { ctx: SettingsCtx }) {
  const [pin, set] = useSettingValue<PinSetting | null>(settingDef('repoPin')!, ctx);
  const locals = useRuntime((s) => (ctx.tabId ? s.tabs[ctx.tabId]?.sidebar?.locals : undefined)) ?? NONE;
  const options: Array<[string, string]> = [['auto', 'Automatic'], ['off', 'Off'], ...locals.map((b): [string, string] => [`ref:${b.fullName}`, b.name])];
  if (pin?.kind === 'ref' && !options.some(([v]) => v === pinKey(pin))) options.push([pinKey(pin), short(pin.name)]);
  const choose = (v: string) => set(v === 'auto' ? { kind: 'auto' } : v === 'off' ? { kind: 'off' } : { kind: 'ref', name: v.slice(4) });
  return <Select<string> id="input-repoPin" aria-labelledby="label-repoPin" value={pinKey(pin)} onChange={choose} options={options} />;
}

function TargetRemoteControl({ ctx }: { ctx: SettingsCtx }) {
  const [remote, set] = useSettingValue<string | null>(settingDef('forgeTargetRemote')!, ctx);
  const remotes = useRuntime((s) => (ctx.tabId ? s.tabs[ctx.tabId]?.info?.remotes : undefined)) ?? NONE;
  const options: Array<[string, string]> = [['', 'Automatic'], ...remotes.map((r): [string, string] => [r.name, r.name])];
  if (remote && !options.some(([v]) => v === remote)) options.push([remote, remote]);
  return <Select<string> id="input-forgeTargetRemote" aria-labelledby="label-forgeTargetRemote" value={remote ?? ''} onChange={(v) => set(v || null)} options={options} />;
}

/** The trunk the graph detected: its pinned branches by name (`origin/main` and `main` are one). */
export function detectedTrunk(pinnedRefs: readonly string[]): string[] {
  return [...new Set(pinnedRefs.map((r) => r.replace(/^refs\/heads\//, '').replace(/^refs\/remotes\/[^/]+\//, '')))];
}
const samePatterns = (a: string[] | undefined, b: string[] | undefined) => (a ?? []).join(',') === (b ?? []).join(',');

/** Extra trunk branches for presets, as comma-separated globs; the detected trunk is named beside them. */
function TrunkPatternsControl({ ctx }: { ctx: SettingsCtx }) {
  const [patterns, set] = useSettingValue<string[] | undefined>(settingDef('trunkPatterns')!, ctx);
  const graph = useRuntime((s) => (ctx.tabId ? s.tabs[ctx.tabId]?.graph : undefined));
  // With the pin off nothing is pinned; a preset's payload still names the trunk it used.
  const trunk = graph ? (graph.pinnedRefs.length ? detectedTrunk(graph.pinnedRefs) : graph.view?.trunk ?? []) : [];
  const commit = (v: string) => {
    const next = v.split(',').map((x) => x.trim()).filter(Boolean);
    if (!samePatterns(next, patterns)) set(next.length ? next : undefined);
  };
  return (
    <span className="trunk-patterns">
      <TextField id="trunkPatterns" value={(patterns ?? []).join(', ')} placeholder="release/*, hotfix/*" onCommit={commit} />
      <span className="setting-note">{trunk.length ? `Detected: ${trunk.join(', ')}` : 'Detected: the pinned trunk, or the default branch with the pin off'}</span>
    </span>
  );
}

const UPDATE_CHOICES: Array<[BranchUpdate | '', string]> = [['', "The forge's default"], ['rebase', 'Rebase'], ['rebaseSkipCi', 'Rebase without pipeline'], ['merge', 'Merge']];
function BranchUpdateControl({ ctx }: { ctx: SettingsCtx }) {
  const [how, set] = useSettingValue<BranchUpdate | undefined>(settingDef('branchUpdate')!, ctx);
  return <Select<BranchUpdate | ''> id="input-branchUpdate" aria-labelledby="label-branchUpdate" value={how ?? ''} onChange={(v) => set(v || undefined)} options={UPDATE_CHOICES} />;
}

const offs = [
  registerSettingsPage({ id: 'repository', label: 'Repository', scope: 'repo', order: 10, icon: FolderGit2, summary: (ctx) => `Settings for ${ctx.repoName ?? 'this repository'} only.` }),
  registerSettings([
    {
      id: 'repoEditor', label: 'Editor for this repository', page: 'repository', group: 'Opening',
      keywords: 'override open in',
      description: "Overrides the profile's default editor here.",
      Control: RepoEditorControl, binding: repoField('editor', null),
    },
    {
      id: 'repoPin', label: 'Pinned trunk', page: 'repository', group: 'Graph', labelFor: 'group',
      keywords: 'pin trunk main branch left lane',
      description: "The branch kept at the graph's left. The pin button in the graph's header sets it too.",
      Control: PinControl,
      binding: repoField('pin', null, (_pin, ctx) => { if (ctx.tabId) void useRuntime.getState().refresh(ctx.tabId); }, (a, b) => pinKey(a) === pinKey(b)),
    },
    {
      id: 'trunkPatterns', label: 'Trunk branches', page: 'repository', group: 'Graph',
      keywords: 'trunk branch presets release patterns glob never hidden collapsed main',
      description: 'Branch presets never hide or collapse these, beside the detected trunk. Comma-separated globs, like release/*.',
      Control: TrunkPatternsControl,
      binding: repoField('trunkPatterns', undefined, (_v, ctx) => refreshPresetViews(ctx.repoPath ?? undefined), samePatterns),
    },
    {
      id: 'forgeTargetRemote', label: 'Merge requests target', page: 'repository', group: 'Merge requests', labelFor: 'group',
      keywords: 'merge request pull request target remote upstream fork project',
      description: 'The remote whose project merge requests and pull requests go to.',
      Control: TargetRemoteControl,
      binding: repoField('forgeTargetRemote', null, () => void flushSaves().then(notifyForgeAccountsChanged)),
    },
    {
      id: 'branchUpdate', label: "Update an MR's branch by", page: 'repository', group: 'Merge requests', labelFor: 'group',
      keywords: 'rebase merge update branch behind pipeline skip ci pull request',
      description: "What the merge panel's update button does. It remembers your last choice.",
      Control: BranchUpdateControl, binding: repoField('branchUpdate', undefined),
    },
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
