import { Palette } from 'lucide-react';
import { registerSettingsPage } from '../../settings/registry';
import { useSettingsUi } from '../../settings/schema';
import { registerKeyHints } from '../../shortcuts/hints';
import { registerFileColoursOpener } from '../addRequest';
import { FileColoursPage } from './FileColoursPage';

const offs = [
  registerSettingsPage({
    id: 'file-colours', label: 'File colours', scope: 'repo', order: 20, icon: Palette,
    summary: (ctx) => (ctx.repoName ? `For ${ctx.repoName}. Top to bottom: a file takes the first scope that matches it.` : 'Top to bottom: a file takes the first scope that matches it.'),
    Body: FileColoursPage,
  }),
  // The file list's "Colour …" items exist while this can open.
  registerFileColoursOpener(() => {
    useSettingsUi.getState().setPage('file-colours');
    useSettingsUi.getState().show();
  }),
  registerKeyHints([
    { id: 'key.fc.move', section: 'Settings', label: 'Move the focused scope up or down', keys: ['Alt+↑', 'Alt+↓'], context: 'on the File colours page', source: 'fileColours/settings/FileColoursPage.tsx' },
    { id: 'key.fc.paste', section: 'Settings', label: 'Paste scopes from the clipboard', keys: ['Mod+V'], context: 'on the File colours page', source: 'fileColours/settings/FileColoursPage.tsx' },
  ]),
];

import.meta.hot?.dispose(() => offs.forEach((off) => off()));
