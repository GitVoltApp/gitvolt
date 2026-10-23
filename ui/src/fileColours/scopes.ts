import { uniqueName as uniqueShareName } from '../settings/share';
import { newScopeId, type FileScope } from './store';
import { TINT_PRESETS, type TintPreset } from './tint';

export function moveScope(scopes: readonly FileScope[], from: number, to: number): FileScope[] {
  const out = [...scopes];
  if (from < 0 || from >= out.length || to < 0 || to >= out.length) return out;
  out.splice(to, 0, ...out.splice(from, 1));
  return out;
}

/** Appends the pattern to the scope at `index`; a no-op if it is already there. */
export function addPattern(scopes: readonly FileScope[], index: number, pattern: string): FileScope[] {
  return scopes.map((s, i) => (i !== index || s.patterns.includes(pattern) ? s : { ...s, patterns: [...s.patterns, pattern] }));
}

/** `name`, else `"name (2)"`, `"name (3)"`… */
export function uniqueName(taken: readonly string[], name: string): string {
  return uniqueShareName(name, taken);
}

/** The first preset no scope uses, else gray. */
export function nextTint(scopes: readonly FileScope[]): TintPreset {
  const used = new Set(scopes.map((s) => s.tint));
  return (Object.keys(TINT_PRESETS) as TintPreset[]).find((p) => !used.has(p)) ?? 'gray';
}

export function newScope(scopes: readonly FileScope[], patterns: string[], name?: string): FileScope {
  return {
    id: newScopeId(),
    name: name ?? uniqueName(scopes.map((s) => s.name), 'New scope'),
    tint: nextTint(scopes),
    patterns,
    startCollapsed: false,
  };
}

/** The repository's folder name: `/h/orbital/.git` and `/srv/orbital.git` are both `orbital`. */
export function repoLabel(commonDir: string): string {
  const parts = commonDir.replace(/[\\/]+$/, '').split(/[\\/]/).filter(Boolean);
  let last = parts.pop() ?? '';
  if (last === '.git') last = parts.pop() ?? '';
  return last.replace(/\.git$/, '') || commonDir;
}
