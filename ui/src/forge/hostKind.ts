import type { HostKind } from '../api/gen/HostKind';

/** The host a typed `input` names, as the core's `normalize_host` reads it: no scheme, path or
 * query, lowercase (`https://Git.Example.dev/` is `git.example.dev`). */
export function bareHost(input: string): string {
  const s = input.trim();
  const rest = s.includes('://') ? s.slice(s.indexOf('://') + 3) : s;
  return (rest.split(/[/?#]/)[0] ?? '').toLowerCase();
}

/** A host's forge type from its name: the Rust `host_kind` (core spec §14.4). The profile's
 * override still wins (`effectiveKind`). */
export function detectHostKind(host: string): HostKind {
  const h = host.toLowerCase();
  if (h === 'github.com' || h.endsWith('.github.com')) return 'github';
  if (h === 'gitlab.com' || h.split('.').some((label) => label.includes('gitlab'))) return 'gitlab';
  const name = h.split(':')[0];
  if (name === 'codeberg.org' || name.split('.').some((label) => label.includes('forgejo') || label.includes('gitea'))) return 'forgejo';
  return 'generic';
}

export const HOST_KIND_NAMES: Record<HostKind, string> = { gitlab: 'GitLab', github: 'GitHub', forgejo: 'Forgejo', generic: 'Generic' };
