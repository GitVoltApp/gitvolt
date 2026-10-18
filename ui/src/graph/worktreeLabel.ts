import type { RefLabel } from '../api/gen/RefLabel';

/**
 * The worktree a detached-HEAD chip stands for (labels.rs: no branch, remote or tag; its
 * `checkedOut` is the worktree's path), else null. Named by the worktree's folder; with the main
 * worktree alone it's the plain `HEAD` chip, which names no worktree and so isn't one.
 */
export const detachedWorktreeOf = (l: RefLabel): string | null => (l.local === null && !l.tag && l.remotes.length === 0 ? l.checkedOut : null);

/** After the open worktree's detached chip's name: it's HEAD, but no branch's. */
export const DETACHED = '(detached)';
