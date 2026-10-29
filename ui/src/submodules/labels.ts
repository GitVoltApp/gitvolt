import type { SubmoduleState } from '../api/gen/SubmoduleState';
import type { SubmoduleStatus } from '../api/gen/SubmoduleStatus';

/** A submodule row's label: "submodule", and in a WIP list what's changed in `git status`'s words.
 * New commits are what the row stages, so they're all it names; edits inside the submodule only
 * when there's nothing else (they're committed in the submodule, not here). */
export function submoduleStateText(s: SubmoduleState | undefined): string {
  if (!s) return 'submodule';
  if (s.uninitialized) return 'submodule: not initialised';
  if (s.newCommits) return 'submodule: new commits';
  const inside = [s.modifiedContent && 'modified content', s.untrackedContent && 'untracked content'].filter(Boolean).join(', ');
  return inside ? `submodule: ${inside}` : 'submodule';
}

/** `git submodule status`'s state, for the picker. */
export const SUBMODULE_STATUS_TEXT: Record<SubmoduleStatus, string> = {
  current: 'checked out',
  newCommits: 'new commits',
  uninitialized: 'not initialised',
  conflicted: 'conflicted',
};

/** The sidebar card's last line: what the state means, and what a double-click does. */
export function submoduleCardState(s: SubmoduleStatus): string {
  switch (s) {
    case 'current': return 'At the recorded commit · double-click to open';
    case 'newCommits': return 'Checked out at another commit than the one recorded · double-click to open';
    case 'uninitialized': return 'Nothing checked out · double-click to initialise';
    case 'conflicted': return 'Conflicted in the index · double-click to open';
  }
}
