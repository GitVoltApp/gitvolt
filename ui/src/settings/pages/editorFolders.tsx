import { FolderOpen } from 'lucide-react';
import { api } from '../../api/client';
import { examplePath } from '../../app/osPath';
import { useAppState } from '../../app/state';
import { EditorPicker } from '../EditorPicker';
import { profileField } from '../bindings';
import { TextField } from '../frame/fields';
import { registerSettings, registerSettingsPage } from '../registry';

function EditorControl() {
  const editor = useAppState((s) => s.profile.editor);
  return <EditorPicker id="editor" value={editor} inherit={false} onChange={(v) => useAppState.getState().updateProfile((p) => ({ ...p, editor: v }))} />;
}

function ReposFolderControl() {
  const folder = useAppState((s) => s.profile.reposFolder);
  const set = (reposFolder: string | null) => useAppState.getState().updateProfile((p) => ({ ...p, reposFolder }));
  return (
    <>
      <TextField id="reposFolder" value={folder ?? ''} placeholder={examplePath('repos')} onCommit={(v) => set(v || null)} />
      <button type="button" onClick={async () => { const d = await api.pickFolder(folder); if (d) set(d); }}>Choose…</button>
    </>
  );
}

const offs = [
  registerSettingsPage({ id: 'editor-folders', label: 'Editor and folders', scope: 'profile', order: 20, icon: FolderOpen, summary: 'Where repositories open from and which editor opens files.' }),
  registerSettings([
    {
      id: 'editor', label: 'Default editor', page: 'editor-folders', group: 'Editor',
      keywords: 'vscode phpstorm jetbrains zed sublime open in custom command template',
      description: 'What Open in uses. Custom runs your own command, with {file}, {line} and {repo} filled in.',
      Control: EditorControl, binding: profileField('editor', null),
    },
    {
      id: 'reposFolder', label: 'Default repos directory', page: 'editor-folders', group: 'Folders',
      keywords: 'clone destination scan your repos folder',
      description: 'Where Open Repository looks for your repositories, and where a clone goes by default.',
      Control: ReposFolderControl, binding: profileField('reposFolder', null),
    },
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
