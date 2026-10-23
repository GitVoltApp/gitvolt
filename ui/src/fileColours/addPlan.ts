import { matchesPattern } from './glob';
import { matcherFor } from './match';
import type { FileScope } from './store';

export interface AddPlan { matching: number; shadowedBy: number | null; recolours: number }

/** Adding `pattern` to scope `target` (scopes.length: a new scope at the end) over `paths`:
 * `shadowedBy` is the highest scope above `target` that already takes some matching path (so
 * the add wouldn't change their colour); `recolours` counts the paths outside `pattern` whose
 * scope changes if `target`, with the pattern added, moves just above `shadowedBy`. */
export function planAdd(scopes: readonly FileScope[], target: number, pattern: string, paths: readonly string[]): AddPlan {
  const old = matcherFor(scopes);
  const matching = paths.filter((p) => matchesPattern(pattern, p));
  let shadowedBy: number | null = null;
  for (const p of matching) {
    const s = old.scopeOf(p);
    if (s >= 0 && s < target && (shadowedBy === null || s < shadowedBy)) shadowedBy = s;
  }
  if (shadowedBy === null) return { matching: matching.length, shadowedBy, recolours: 0 };

  // `ids[i]` is the original index of the scope now at position i (the target's moved up).
  const isNew = target >= scopes.length;
  const entries = isNew
    ? [...scopes, { id: '', name: '', tint: 'gray', patterns: [pattern], startCollapsed: false }]
    : scopes.map((s, i) => (i === target ? { ...s, patterns: [...s.patterns, pattern] } : s));
  const ids = entries.map((_, i) => i);
  const [moved] = entries.splice(target, 1);
  const [movedId] = ids.splice(target, 1);
  entries.splice(shadowedBy, 0, moved);
  ids.splice(shadowedBy, 0, movedId);
  const next = matcherFor(entries);
  const inPattern = new Set(matching);
  let recolours = 0;
  for (const p of paths) {
    if (inPattern.has(p)) continue;
    const after = next.scopeOf(p);
    if (old.scopeOf(p) !== (after < 0 ? -1 : ids[after])) recolours++;
  }
  return { matching: matching.length, shadowedBy, recolours };
}
