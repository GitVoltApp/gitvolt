import { GitFork } from 'lucide-react';
import { useRuntime } from '../../app/runtime';
import { useAppState } from '../../app/state';
import { registerSettings, registerSettingsPage } from '../../settings/registry';
import { useSettingsUi } from '../../settings/schema';
import { registerKeyHints } from '../../shortcuts/hints';
import { currentPreset, presetList } from '../model';
import { registerEditPresets } from '../store';
import { BranchPresetsPage, usePresetsPage } from './BranchPresetsPage';

/** "Edit presets…": Settings › Branch presets, on the shown tab's current preset. */
export function openPresetsPage(): void {
  const { settings, profile } = useAppState.getState();
  const path = profile.activeTab ? useRuntime.getState().tabs[profile.activeTab]?.repo?.path : undefined;
  const rs = path ? profile.repos[path] : undefined;
  usePresetsPage.setState({ selected: currentPreset(presetList(settings, rs), rs).preset.id, focusName: null });
  useSettingsUi.getState().setPage('branch-presets');
  useSettingsUi.getState().show();
}

const offs = [
  registerSettingsPage({
    id: 'branch-presets', label: 'Branch presets', scope: 'app', order: 40, icon: GitFork,
    summary: 'Views of the graph that hide, collapse or fold branches. The button beside the branch filter switches between them.',
    Body: BranchPresetsPage,
  }),
  registerSettings([{
    id: 'branchPresets', label: 'Branch presets', page: 'branch-presets', group: 'Presets',
    keywords: 'hide collapse fold branches preset view filter cycle mine custom rules picked ghost lines stub hidden fork',
    description: 'Named views of the graph: their rules, colours and which ones the button steps through.',
  }]),
  registerEditPresets(openPresetsPage),
  registerKeyHints([
    { id: 'key.bp.move', section: 'Settings', label: 'Move the focused preset or rule up or down', keys: ['Alt+↑', 'Alt+↓'], context: 'on the Branch presets page', source: 'presets/settings/sortableKeys.ts' },
    { id: 'key.bp.paste', section: 'Settings', label: 'Paste a branch preset from the clipboard', keys: ['Mod+V'], context: 'on the Branch presets page', source: 'presets/settings/BranchPresetsPage.tsx' },
  ]),
];

import.meta.hot?.dispose(() => offs.forEach((off) => off()));
