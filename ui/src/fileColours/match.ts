import { compilePattern, type CompiledPattern } from './glob';

export interface ScopeLike { patterns: readonly string[] }
export interface ScopeMatcher {
  scopeOf(path: string): number;          // first matching scope's index, -1 for none; cached per path
  folderScopeOf(folder: string): number;  // first scope covering the whole folder, -1; cached
  explain(path: string): { scope: number; pattern: string } | null; // winning scope and its first positive pattern that matched
}

interface CompiledScope {
  positives: CompiledPattern[];
  excludes: CompiledPattern[];
  posFile: RegExp | null;
  posFolder: RegExp | null;
  excFile: RegExp | null;
}

const alternation = (list: CompiledPattern[], pick: 'file' | 'folder'): RegExp | null =>
  list.length === 0 ? null : new RegExp(list.map((p) => `(?:${p[pick].source})`).join('|'));

function compileScope(scope: ScopeLike): CompiledScope {
  const all = scope.patterns.map(compilePattern).filter((p): p is CompiledPattern => p !== null);
  const positives = all.filter((p) => !p.exclude);
  const excludes = all.filter((p) => p.exclude);
  return {
    positives,
    excludes,
    posFile: alternation(positives, 'file'),
    posFolder: alternation(positives, 'folder'),
    excFile: alternation(excludes, 'file'),
  };
}

/** Could this exclude match a path below `folder`? */
function couldExclude(e: CompiledPattern, folder: string): boolean {
  if (!e.anchored) return true;
  const dir = `${folder}/`;
  return dir.startsWith(e.literalPrefix) || e.literalPrefix.startsWith(dir);
}

function build(scopes: readonly ScopeLike[]): ScopeMatcher {
  const compiled = scopes.map(compileScope);
  const files = new Map<string, number>();
  const folders = new Map<string, number>();
  const matches = (s: CompiledScope, path: string) =>
    s.posFile !== null && s.posFile.test(path) && !(s.excFile !== null && s.excFile.test(path));
  const scopeOf = (path: string) => {
    let hit = files.get(path);
    if (hit === undefined) {
      hit = compiled.findIndex((s) => matches(s, path));
      files.set(path, hit);
    }
    return hit;
  };
  return {
    scopeOf,
    folderScopeOf(folder) {
      let hit = folders.get(folder);
      if (hit === undefined) {
        hit = compiled.findIndex((s) =>
          s.posFolder !== null && s.posFolder.test(folder) && !s.excludes.some((e) => couldExclude(e, folder)));
        folders.set(folder, hit);
      }
      return hit;
    },
    explain(path) {
      const scope = scopeOf(path);
      if (scope < 0) return null;
      const pattern = compiled[scope].positives.find((p) => p.file.test(path));
      return pattern ? { scope, pattern: pattern.source } : null;
    },
  };
}

const cache = new WeakMap<readonly ScopeLike[], ScopeMatcher>();

/** One matcher per array identity: a new array (an edit) starts with empty caches. */
export function matcherFor(scopes: readonly ScopeLike[]): ScopeMatcher {
  let m = cache.get(scopes);
  if (!m) { m = build(scopes); cache.set(scopes, m); }
  return m;
}
