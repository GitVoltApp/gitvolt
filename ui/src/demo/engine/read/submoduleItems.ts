// The sidebar's Submodules section (core `submodules::sidebar_items`).
import type { SubmoduleItem } from '../../../api/gen/SubmoduleItem';
import { redact } from '../remotes';
import { readIndex } from './indexFile';
import type { ReadRepo } from './seam';
import { MODE_GITLINK } from './trees';
import { gitmodules, nestedBranch, nestedHead } from './worktree';

/** The sidebar's submodules of the worktree at `root` (core `submodules::sidebar_items`): each
 * gitlink of its index by path, with `.gitmodules`' name and URL and what's checked out there. */
export async function submoduleItems(repo: ReadRepo, root: string, gitdir: string): Promise<SubmoduleItem[]> {
  const links = new Map<string, { recorded: string; conflicted: boolean }>();
  for (const e of await readIndex(repo, gitdir)) {
    if (e.mode !== MODE_GITLINK) continue;
    const slot = links.get(e.path) ?? { recorded: e.oid, conflicted: false };
    if (e.stage !== 0) {
      slot.conflicted = true;
      if (e.stage === 2) slot.recorded = e.oid;
    }
    links.set(e.path, slot);
  }
  if (links.size === 0) return [];
  const modules = await gitmodules(repo, root);
  const out: SubmoduleItem[] = [];
  for (const [path, { recorded, conflicted }] of [...links].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) {
    const m = modules.find((x) => x.path === path);
    const head = await nestedHead(repo, root, path);
    const status = conflicted ? 'conflicted' : head === null ? 'uninitialized' : head === recorded ? 'current' : 'newCommits';
    out.push({ path, name: m?.name ?? path, url: m?.url == null ? null : redact(m.url), recorded, head, branch: await nestedBranch(repo, root, path), status });
  }
  return out;
}
