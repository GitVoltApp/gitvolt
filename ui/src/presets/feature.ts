import { GitFork } from 'lucide-react';
import { activeTab, registerActions } from '../app/actions';
import { useRuntime } from '../app/runtime';
import { useAppState } from '../app/state';
import { leaveFileView, sidebarHidden } from '../repo/centerView';
import { openPresetMenu } from './PresetButton';
import { cyclePreset } from './store';

/** A repo tab whose repository is open. */
const openTab = () => {
  const t = activeTab();
  return t?.kind === 'repo' && useRuntime.getState().tabs[t.id]?.repo ? t : null;
};

const off = registerActions([
  {
    id: 'presets.next', label: 'Next branch preset', group: 'View', icon: GitFork, tooltip: 'Switch the graph to the next branch preset in the cycle', shortcuts: ['Mod+Alt+G'],
    when: () => !!openTab(),
    run: () => cyclePreset(openTab()!.id),
  },
  {
    id: 'presets.choose', label: 'Choose branch preset…', group: 'View', icon: GitFork, tooltip: 'List every branch preset, at the button beside the sidebar filter', shortcuts: ['Mod+Alt+Shift+G'],
    // The menu opens at the sidebar's button: not while the rebase editor has the sidebar's place.
    when: () => { const t = openTab(); return !!t && !sidebarHidden(t.id); },
    run: () => {
      const t = openTab()!;
      if (openPresetMenu(t.id)) return;
      // A narrow sidebar, or a file view over it: back to the full one, then open.
      leaveFileView(t.id);
      useAppState.getState().updateProfile((p) => ({ ...p, sidebarNarrow: false }));
      requestAnimationFrame(() => openPresetMenu(t.id));
    },
  },
]);
import.meta.hot?.dispose(off);
