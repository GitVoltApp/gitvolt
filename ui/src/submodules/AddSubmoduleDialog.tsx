import { useState } from 'react';
import { create } from 'zustand';
import { useModalKeys } from '../app/modalKeys';
import { registerAppSlot } from '../app/slots';
import { writeCtx } from '../write/ctx';
import { addSubmodule } from './commands';

const useDialog = create<{ tabId: string | null }>(() => ({ tabId: null }));
/** Opens Add submodule for a tab (the Submodules panel's +, the palette). */
export const openAddSubmodule = (tabId: string) => useDialog.setState({ tabId });
export const closeAddSubmodule = () => useDialog.setState({ tabId: null });

/** Where git puts a submodule of `url` when no path is given: the URL's last part, without `.git`. */
export function defaultSubmodulePath(url: string): string {
  const last = url.trim().replace(/[/\\]+$/, '').split(/[/\\:]/).pop() ?? '';
  return last.replace(/\.git$/i, '');
}

/** Why `path` can't be a submodule's path in the worktree, or undefined. */
export function submodulePathError(path: string): string | undefined {
  if (!path) return 'Enter a path for the submodule';
  if (path.startsWith('/') || /^[A-Za-z]:/.test(path)) return 'A path inside the repository, not an absolute one';
  if (path.split(/[/\\]/).some((p) => p === '..' || p === '.git')) return 'A path inside the repository';
  return undefined;
}

export function AddSubmoduleDialog() {
  const tabId = useDialog((s) => s.tabId);
  return tabId ? <Form tabId={tabId} key={tabId} /> : null;
}

function Form({ tabId }: { tabId: string }) {
  const [url, setUrl] = useState('');
  const [path, setPath] = useState<string | null>(null); // null: follows the URL
  const [branch, setBranch] = useState('');
  const ref = useModalKeys<HTMLDivElement>(true, closeAddSubmodule);
  const trimmed = url.trim();
  const effPath = (path ?? defaultSubmodulePath(trimmed)).trim();
  const urlError = trimmed ? undefined : 'Enter the URL of the repository to add';
  const pathError = submodulePathError(effPath);
  const branchError = branch.trim().startsWith('-') ? `"${branch.trim()}" isn't a branch name` : undefined;
  const error = urlError ?? pathError ?? branchError;

  const submit = () => {
    const ctx = writeCtx(tabId);
    if (error || !ctx) return;
    closeAddSubmodule();
    void addSubmodule(ctx, trimmed, effPath, branch.trim() || undefined);
  };

  return (
    <div className="modal-backdrop" onPointerDown={closeAddSubmodule}>
      <div ref={ref} className="modal add-submodule" role="dialog" aria-modal="true" aria-labelledby="add-submodule-title" onPointerDown={(e) => e.stopPropagation()}>
        <h2 id="add-submodule-title">Add submodule</h2>
        <form onSubmit={(e) => { e.preventDefault(); submit(); }}>
          <label className="modal-field">
            <span>URL</span>
            <input aria-label="Submodule URL" autoFocus spellCheck={false} placeholder="https://git.example.com/group/library.git" value={url} onChange={(e) => setUrl(e.target.value)} />
          </label>
          <label className="modal-field">
            <span>Path</span>
            <input aria-label="Submodule path" spellCheck={false} placeholder="vendor/library" value={path ?? defaultSubmodulePath(trimmed)} onChange={(e) => setPath(e.target.value)} />
          </label>
          <label className="modal-field">
            <span>Branch</span>
            <input aria-label="Submodule branch" spellCheck={false} placeholder="The remote's default branch" value={branch} onChange={(e) => setBranch(e.target.value)} />
          </label>
          {trimmed && error && <p role="alert" className="modal-error">{error}</p>}
          <div className="modal-actions">
            <button type="button" onClick={closeAddSubmodule}>Cancel</button>
            <button type="submit" className="primary" disabled={!!error}>Add submodule</button>
          </div>
        </form>
      </div>
    </div>
  );
}

export const offAddSubmoduleDialog = registerAppSlot('overlay', 'addSubmodule', AddSubmoduleDialog);
