import { api, errorMessage } from '../api/client';
import type { SubmoduleInfo } from '../api/gen/SubmoduleInfo';
import type { SubmoduleOutcome } from '../api/gen/SubmoduleOutcome';
import { activeTab } from '../app/actions';
import { joinPath } from '../app/osPath';
import { openPathInTab } from '../app/runtime';
import { usePalette } from '../palette/Palette';
import type { PaletteEntry } from '../palette/search';
import { currentOrigin } from '../ui/arm/origin';
import { confirmAction } from '../ui/ConfirmDialog';
import { useToast } from '../ui/toastStore';
import { runWrite, type WriteCtx } from '../write/client';
import { SUBMODULE_STATUS_TEXT } from './labels';

const toast = (msg: string, detail?: string) => useToast.getState().show(msg, detail ? { detail } : undefined);

/** Says what ran, as the terminal command. */
const ran = (done: string) => (o: SubmoduleOutcome) => toast(done, o.command);

/** `git submodule update --init --recursive`, of `path` only when given. A network command:
 * progress, Cancel and failures as for a fetch (the op and the error toast). */
export async function updateSubmodules(ctx: WriteCtx, path?: string): Promise<void> {
  // Commits on no branch in a submodule are asked about first (`submoduleOrphans`), armed in place.
  await runWrite(ctx, (_confirmed, asked) => api.submoduleUpdate(ctx.repoId, ctx.worktree, path, asked.discard), { onSuccess: ran(path ? `Updated ${path}` : 'Submodules updated') });
}

/** `git submodule sync --recursive`: `.gitmodules`' URLs into the config. */
export async function syncSubmodules(ctx: WriteCtx): Promise<void> {
  await runWrite(ctx, () => api.submoduleSync(ctx.repoId, ctx.worktree), { onSuccess: ran('Submodule URLs synced') });
}

/** `git submodule add`: clones `url` (a network command, as Update) and stages it. */
export async function addSubmodule(ctx: WriteCtx, url: string, path?: string, branch?: string): Promise<void> {
  await runWrite(ctx, () => api.submoduleAdd(ctx.repoId, ctx.worktree, url, path, branch), { onSuccess: ran(`Added the submodule ${path ?? url}: commit to record it`) });
}

/** Remove submodule (its sidebar menu): asked first, in place (danger); `git rm` takes out its
 * checkout, gitlink and `.gitmodules` entry and stages that. Its repository stays in
 * `.git/modules`; git refuses a checkout with changes of its own. */
export async function removeSubmodule(ctx: WriteCtx, path: string): Promise<void> {
  const origin = currentOrigin();
  const ok = await confirmAction({
    title: `Remove the submodule ${path}?`,
    body: `Its checkout, its entry in .gitmodules and the commit recorded here are removed and the removal is staged; commit to record it. Its repository stays in .git/modules. Not undoable from here: unstaging and Submodules: Update bring it back.`,
    caption: 'Removes the checkout and stages the removal (git rm)',
    arm: `Click again to remove ${path}`,
    confirmLabel: 'Remove submodule',
    danger: true,
  }, origin);
  if (!ok) return;
  await runWrite(ctx, () => api.submoduleRemove(ctx.repoId, ctx.worktree, path), { origin, onSuccess: ran(`Removed ${path}: commit to record it`) });
}

const notInitialised = (path: string) => toast(`${path} isn't initialised: Submodules: Update checks it out`);

/** Opens the submodule at `path` in `worktree` as a repository tab (the tab already showing it,
 * when there's one). One with nothing checked out says so instead. */
export async function openSubmodule(worktree: string, path: string): Promise<void> {
  const dir = joinPath(worktree, path);
  try {
    await api.peekRepo(dir);
  } catch {
    notInitialised(path);
    return;
  }
  await openPathInTab(dir);
}

export function submoduleEntries(worktree: string, list: SubmoduleInfo[]): PaletteEntry[] {
  return list.map((s) => ({
    id: `submodule:${s.path}`,
    group: 'choice',
    label: s.path,
    detail: SUBMODULE_STATUS_TEXT[s.status],
    run: () => { if (s.status === 'uninitialized') notInitialised(s.path); else void openSubmodule(worktree, s.path); },
  }));
}

/** The latest "Open in a tab…": a second one supersedes a list still loading. */
let picking = 0;

/** "Submodules: Open in a tab…": the palette lists the submodules (`git submodule status`). */
export async function pickSubmodule(ctx: WriteCtx): Promise<void> {
  // Not when the user moved on while the list loaded: another tab, the palette or a newer
  // "Open in a tab…" open.
  const mine = ++picking;
  const movedOn = () => mine !== picking || activeTab()?.id !== ctx.tabId || usePalette.getState().open;
  let list: SubmoduleInfo[];
  try {
    list = await api.submodules(ctx.repoId, ctx.worktree);
  } catch (e) {
    if (!movedOn()) useToast.getState().show(`Couldn't list the submodules: ${errorMessage(e)}`, { error: true });
    return;
  }
  if (movedOn()) return;
  if (list.length === 0) {
    toast('This worktree has no submodules');
    return;
  }
  usePalette.getState().choose({ title: 'Submodules', placeholder: 'Open a submodule in a tab', entries: submoduleEntries(ctx.worktree, list) });
}
