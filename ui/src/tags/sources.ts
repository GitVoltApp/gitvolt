import type { TagRemoteLabel } from '../api/gen/TagRemoteLabel';
import type { TagRemoteState } from '../api/gen/TagRemoteState';

/**
 * A tag chip's source icons, from its remotes' known tags (`RefLabel.tagRemotes`, each fetch's
 * `git ls-remote --tags`): `null` while none is known (no icon at all). Otherwise `local` when no
 * known remote has the tag at this commit (the laptop), and the remotes that have a tag of its
 * name: at this commit, or (`state: 'other'`) at another one, drawn in orange.
 */
export function tagSources(remotes: readonly TagRemoteLabel[] | undefined): { local: boolean; remotes: TagRemoteLabel[] } | null {
  if (!remotes) return null;
  return { local: !remotes.some((r) => r.state === 'same'), remotes: remotes.filter((r) => r.state !== 'missing') };
}

/** The tag's state on `remote`: `null` when that remote's tags aren't known. */
export function tagOn(remotes: readonly TagRemoteLabel[] | undefined, remote: string): TagRemoteState | null {
  return remotes?.find((r) => r.remote === remote)?.state ?? null;
}
