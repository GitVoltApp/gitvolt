import type { RepoProjects } from '../api/gen/RepoProjects';
import { hostName } from '../remotes/match';

/** The remotes with a project on `host` (`forgeRepoProjects`): the Create flyout's
 * From and To choices. The target project is `forgeOf(tabId)`, not kept again here. */
export function mappedRemotes(p: RepoProjects | null | undefined, host: string): string[] {
  return (p?.remotes ?? []).filter((r) => hostName(r.project?.host) === hostName(host)).map((r) => r.remote);
}
