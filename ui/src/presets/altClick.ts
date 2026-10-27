import type { BranchView } from '../api/gen/BranchView';
import type { ViewPayload } from '../api/gen/ViewPayload';
import { useRuntime } from '../app/runtime';
import { registerKeyHints } from '../shortcuts/hints';
import { hideBranch, showBranch } from './store';

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

/** Why Hide is off for `name` (the menus' disabled reason, alt+click's no-op), or null. A branch
 * checked out in another worktree hides by hand (with that worktree's WIP row), though no rule
 * hides it; the tab's own (`headBranch`) doesn't. */
export function cannotHide(view: ViewPayload | undefined | null, name: string, headBranch: string | null): string | null {
  if (view?.trunk.includes(name)) return 'The trunk is always shown';
  if (headBranch === name) return 'Checked out: always shown';
  return null;
}

/** Hides `name` when it's shown, collapsed or folded; shows it when hidden. The trunk and the
 * tab's checked-out branch stay as they are. */
export function toggleBranch(tabId: string, name: string): void {
  const g = useRuntime.getState().tabs[tabId]?.graph;
  if (branchViewOf(g?.view, name)?.state === 'hidden') showBranch(tabId, name);
  else if (!cannotHide(g?.view, name, g?.head.branch?.replace(/^refs\/heads\//, '') ?? null)) hideBranch(tabId, name);
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

// Shown in the Keyboard Shortcuts panel (Ctrl+/); metadata only.
registerKeyHints([
  { id: 'key.altclick.branch', section: 'Navigation', label: 'Hide or show a branch', keys: ['Alt+Click'], context: 'branch chip or sidebar branch', source: 'presets/altClick.ts' },
]);
