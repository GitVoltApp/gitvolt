import { Keyboard } from 'lucide-react';
import { ShortcutsList } from '../../shortcuts/ShortcutsList';
import { registerSettings, registerSettingsPage } from '../registry';

const offs = [
  registerSettingsPage({ id: 'keyboard', label: 'Keyboard', scope: 'app', order: 70, icon: Keyboard, summary: 'Every keyboard shortcut.', Body: () => <ShortcutsList /> }),
  registerSettings([
    { id: 'shortcuts', label: 'Keyboard shortcuts', page: 'keyboard', group: 'Shortcuts', keywords: 'keys hotkeys bindings chord', description: 'Every shortcut, and what it does.' },
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
