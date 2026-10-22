import { FileDiff } from 'lucide-react';
import { DEFAULT_EDITOR_SETTINGS, useEditorSettings } from '../../diff/editorSettings';
import { storeBinding } from '../bindings';
import { switchControl } from '../frame/controls';
import { registerSettings, registerSettingsPage } from '../registry';

const offs = [
  registerSettingsPage({ id: 'diffs', label: 'Diffs and editors', scope: 'app', order: 50, icon: FileDiff, summary: 'The diff and file views.' }),
  registerSettings([
    {
      id: 'stickyScroll', label: 'Sticky scroll in the diff viewer', page: 'diffs', group: 'Editor',
      keywords: 'monaco pin scope header',
      description: "Pins the enclosing scope's first line at the top. With brace-on-next-line code (common in PHP) it may show only the brace.",
      Control: switchControl('stickyScroll'),
      binding: storeBinding(useEditorSettings, () => useEditorSettings.getState().settings.stickyScroll, (v) => useEditorSettings.getState().set({ stickyScroll: v }), DEFAULT_EDITOR_SETTINGS.stickyScroll),
    },
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
