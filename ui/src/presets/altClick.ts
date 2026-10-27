import type { BranchView } from '../api/gen/BranchView';
import type { ViewPayload } from '../api/gen/ViewPayload';
import { useRuntime } from '../app/runtime';
import { registerKeyHints } from '../shortcuts/hints';
import { hideBranch, hideFolder, hideTag, hideTagFolder, showBranch, showFolder, showTag, showTagFolder } from './store';

/** What a branch chip or sidebar row is, for hiding: its refs (the menus' `BranchRef`). */
export interface BranchLike { local: string | null; remotes: ReadonlyArray<{ fullName: string; remote: string }> }

/** The branch unit's name (Rulings 1): the short branch name, remote prefix stripped. A tag or a
 * detached HEAD has none. */
export function unitNameOf(b: BranchLike): string | null {
  if (b.local) return b.local.replace(/^refs\/heads\//, '');
  const r = b.remotes[0];
  return r ? r.fullName.slice(`refs/remotes/${r.remote}/`.length) : null;
}

export const branchViewOf = (view: ViewPayload | undefined | null, name: string): BranchView | undefined => view?.branches.find((b) => b.name === name);

/** Whether the graph's pinned refs (`GraphPayload.pinnedRefs`: the trunk, or the chosen pin, and
 * its pair) include unit `name`, local or on a remote. The core's trunk without a view. */
const pinned = (pinnedRefs: readonly string[], name: string): boolean =>
  pinnedRefs.some((r) => r === `refs/heads/${name}` || (r.startsWith('refs/remotes/') && r.slice('refs/remotes/'.length).replace(/^[^/]+\//, '') === name));

/** Why Hide is off for `name` (the menus' disabled reason, alt+click's no-op), or null. The trunk
 * is the view's; under All (no view) it's the graph's pinned refs. A branch checked out in another
 * worktree hides by hand (with that worktree's WIP row), though no rule hides it; the tab's own
 * (`headBranch`) doesn't. */
export function cannotHide(view: ViewPayload | undefined | null, name: string, headBranch: string | null, pinnedRefs: readonly string[] = []): string | null {
  if (view ? view.trunk.includes(name) : pinned(pinnedRefs, name)) return 'The trunk is always shown';
  if (headBranch === name) return 'Checked out: always shown';
  return null;
}

/** Hides `name` when it's shown, collapsed or folded; shows it when hidden. The trunk and the
 * tab's checked-out branch stay as they are. */
export function toggleBranch(tabId: string, name: string): void {
  const g = useRuntime.getState().tabs[tabId]?.graph;
  if (branchViewOf(g?.view, name)?.state === 'hidden') showBranch(tabId, name);
  else if (!cannotHide(g?.view, name, g?.head.branch?.replace(/^refs\/heads\//, '') ?? null, g?.pinnedRefs)) hideBranch(tabId, name);
}

/** Alt+click on a chip or sidebar branch row: true when it was one (the click is then spent). */
export function altClickBranch(tabId: string | null, b: BranchLike, e: { altKey: boolean; preventDefault(): void; stopPropagation(): void }): boolean {
  const name = e.altKey && tabId ? unitNameOf(b) : null;
  if (!name || !tabId) return false;
  e.preventDefault();
  e.stopPropagation();
  toggleBranch(tabId, name);
  return true;
}

/** Alt+click on a sidebar branch folder (`folder` is its unit-name prefix, `deployed-commit/`), or
 * a tag folder (`tags`): shows it when it looks hidden (`hidden`: its folder pick, or every branch
 * or tag under it, hides), hides it otherwise. True when it was ours (the click is then spent). */
export function altClickFolder(tabId: string | null, folder: string, hidden: boolean, e: { altKey: boolean; preventDefault(): void; stopPropagation(): void }, tags = false): boolean {
  if (!e.altKey || !tabId) return false;
  e.preventDefault();
  e.stopPropagation();
  toggleFolder(tabId, folder, hidden, tags);
  return true;
}

/** Shows `folder` when it looks hidden, hides it otherwise (Alt+click, the sidebar's eye, H);
 * `tags`: a Tags folder (`archive/`), whose tags it picks. */
export const toggleFolder = (tabId: string, folder: string, hidden: boolean, tags = false): void =>
  (tags ? (hidden ? showTagFolder : hideTagFolder) : hidden ? showFolder : hideFolder)(tabId, folder);

/** Whether the open preset hides tag `name` (by hand: no rule decides a tag). */
export const tagHidden = (view: ViewPayload | undefined | null, name: string): boolean => !!view?.hiddenTags?.includes(name);

/** Hides tag `name` when shown, shows it when hidden. Nothing keeps a tag: on the trunk or the
 * checked-out commit, only its chip goes. */
export function toggleTag(tabId: string, name: string): void {
  (tagHidden(useRuntime.getState().tabs[tabId]?.graph?.view, name) ? showTag : hideTag)(tabId, name);
}

/** Alt+click on a tag chip or sidebar tag row: true when it was one (the click is then spent). */
export function altClickTag(tabId: string | null, name: string, e: { altKey: boolean; preventDefault(): void; stopPropagation(): void }): boolean {
  if (!e.altKey || !tabId) return false;
  e.preventDefault();
  e.stopPropagation();
  toggleTag(tabId, name);
  return true;
}

/** Alt+click on a graph chip: its tag, or its branch. */
export const altClickChip = (tabId: string | null, l: BranchLike & { tag: boolean; name: string }, e: { altKey: boolean; preventDefault(): void; stopPropagation(): void }): boolean =>
  (l.tag ? altClickTag(tabId, l.name, e) : altClickBranch(tabId, l, e));

// Shown in the Keyboard Shortcuts panel (Ctrl+/); metadata only.
registerKeyHints([
  { id: 'key.altclick.branch', section: 'Navigation', label: 'Hide or show a branch, tag or folder', keys: ['Alt+Click'], context: 'branch or tag chip, sidebar branch, tag or folder', source: 'presets/altClick.ts' },
]);
