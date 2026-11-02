import type { GitflowConfig } from '../api/gen/GitflowConfig';
import type { GitflowKind } from '../api/gen/GitflowKind';
import { branchNameError } from '../branches/branchName';
import { tagNameError } from '../tags/tagName';

/** Git flow's three kinds of branch, in the order the panel and the dialogs list them. */
export const KINDS: readonly GitflowKind[] = ['feature', 'release', 'hotfix'];
export const KIND_LABEL: Record<GitflowKind, string> = { feature: 'Feature', release: 'Release', hotfix: 'Hotfix' };

export const prefixOf = (c: GitflowConfig, kind: GitflowKind): string => c[kind];

/** A branch's kind and what follows its prefix (a release's version): the core's
 * `GitflowConfig::classify`, rule for rule. */
export function classify(c: GitflowConfig, name: string): { kind: GitflowKind; rest: string } | null {
  for (const kind of KINDS) {
    const prefix = prefixOf(c, kind);
    if (name.startsWith(prefix) && name.length > prefix.length) return { kind, rest: name.slice(prefix.length) };
  }
  return null;
}

/** Where a new branch of `kind` starts: a hotfix from production, the others from development. */
export const baseOf = (c: GitflowConfig, kind: GitflowKind): string => (kind === 'hotfix' ? c.master : c.develop);

/** The branches Finish merges into, in order. */
export const targetsOf = (c: GitflowConfig, kind: GitflowKind): string[] => (kind === 'feature' ? [c.develop] : [c.master, c.develop]);

/** The tag a release or hotfix gets on production; a feature gets none. */
export const tagOf = (c: GitflowConfig, kind: GitflowKind, rest: string): string | null => (kind === 'feature' ? null : `${c.versionTag}${rest}`);

/** A version number in a release's or hotfix's name: `3.0.0`, `v3.0.0`, `3.0`, `3.0.0-rc.1`. */
const VERSION = /^v?(\d+(?:\.\d+)*(?:[-+][0-9A-Za-z.-]+)?)$/;

/** The Finish dialog's tag: the version a release or hotfix is named after, after the version
 * tag prefix (`release/3.0.0` → `3.0.0`, `v3.0.0` with the prefix `v`); `''` when its name isn't
 * a version (`release/spring`): the user names it. A feature has no tag (`null`). */
export function suggestedTag(c: GitflowConfig, kind: GitflowKind, rest: string): string | null {
  if (kind === 'feature') return null;
  const m = VERSION.exec(rest);
  return m ? `${c.versionTag}${m[1]}` : '';
}

/** Init's suggestion: `git flow init`'s defaults, with the production branch the repository has. */
export function defaultConfig(locals: readonly string[]): GitflowConfig {
  const master = ['main', 'master'].find((b) => locals.includes(b)) ?? 'main';
  return { master, develop: 'develop', feature: 'feature/', release: 'release/', hotfix: 'hotfix/', versionTag: '' };
}

/** Why the settings can't be used: the core's `GitflowConfig::error`, word for word. */
export function configError(c: GitflowConfig): string | null {
  for (const [what, name] of [['production', c.master], ['development', c.develop]] as const) {
    const why = branchNameError(name);
    if (why) return `The ${what} branch: ${why}`;
  }
  if (c.master === c.develop) return 'The production and development branches must differ';
  const prefixes = KINDS.map((k) => [k, prefixOf(c, k)] as const);
  for (const [i, [what, prefix]] of prefixes.entries()) {
    if (!prefix) return `Enter a ${what} prefix`;
    const why = branchNameError(`${prefix}x`);
    if (why) return `The ${what} prefix: ${why}`;
    const other = prefixes.slice(0, i).find(([, p]) => p.startsWith(prefix) || prefix.startsWith(p));
    if (other) return `The ${what} and ${other[0]} prefixes overlap`;
    const branch = [c.master, c.develop].find((b) => b.startsWith(prefix));
    if (branch) return `The ${what} prefix would make ${branch} a ${what} branch`;
  }
  if (c.versionTag) {
    const why = tagNameError(`${c.versionTag}1.0`);
    if (why) return `The version tag prefix: ${why}`;
  }
  return null;
}

/** A new branch's name, before the prefix goes on: what the core would refuse, said of the name
 * the user typed (`feature/` is shown beside the box). */
export function startNameError(c: GitflowConfig, kind: GitflowKind, name: string, taken: (branch: string) => boolean): string | null {
  if (!name) return kind === 'feature' ? 'Enter a feature name' : 'Enter a version';
  const full = `${prefixOf(c, kind)}${name}`;
  const why = branchNameError(full);
  if (why) return why;
  if (taken(full)) return `A branch named ${full} already exists`;
  const tag = tagOf(c, kind, name);
  if (tag && tagNameError(tag)) return `${tag} isn't a valid tag name`;
  return null;
}
