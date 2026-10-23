import { describe, expect, it } from 'vitest';
import { growth } from '../test-perf';
import { matcherFor } from './match';

const m = (...scopes: string[][]) => matcherFor(scopes.map((patterns) => ({ patterns })));

describe('matcherFor', () => {
  it('a file takes the first scope that matches it', () => {
    const x = m(['**/tests/**'], ['*.snap']);
    expect(x.scopeOf('x/tests/a.snap')).toBe(0);
    expect(x.scopeOf('b/a.snap')).toBe(1);
    expect(x.scopeOf('c.rs')).toBe(-1);
  });

  it('an exclude removes a path from its own scope only', () => {
    const x = m(['**/tests/**', '!**/tests/fixtures/**'], ['**/fixtures/**']);
    expect(x.scopeOf('tests/fixtures/f.json')).toBe(1);
    expect(x.scopeOf('tests/a.rs')).toBe(0);
  });

  it("excludes don't depend on their position", () => {
    for (const patterns of [['!*.md', 'docs/**'], ['docs/**', '!*.md']]) {
      const x = m(patterns);
      expect(x.scopeOf('docs/a.md')).toBe(-1);
      expect(x.scopeOf('docs/a.txt')).toBe(0);
    }
  });

  it('a scope of only excludes matches nothing', () => {
    expect(m(['!*.md']).scopeOf('a.txt')).toBe(-1);
  });

  it('a folder takes a scope only when a pattern covers all of it', () => {
    expect(m(['**/tests/**']).folderScopeOf('crates/x/tests')).toBe(0);
    expect(m(['**/tests/**']).folderScopeOf('crates/x/tests/snap')).toBe(0);
    expect(m(['tests/**']).folderScopeOf('tests')).toBe(0);
    expect(m(['*.rs']).folderScopeOf('src')).toBe(-1);
    const d = m(['docs/**', '!docs/api/**']);
    expect(d.folderScopeOf('docs/guide')).toBe(0);
    expect(d.folderScopeOf('docs')).toBe(-1);
    expect(m(['docs/**', '!*.md']).folderScopeOf('docs')).toBe(-1);
  });

  it('explain names the winning scope and the pattern that matched', () => {
    const x = m(['**/tests/**'], ['a', '**/*.snap']);
    expect(x.explain('b/x.snap')).toEqual({ scope: 1, pattern: '**/*.snap' });
    expect(x.explain('c.rs')).toBeNull();
  });

  it('matcherFor returns the same matcher for the same array, and a new one for a new array', () => {
    const scopes = [{ patterns: ['*.rs'] }];
    expect(matcherFor(scopes)).toBe(matcherFor(scopes));
    expect(matcherFor([...scopes])).not.toBe(matcherFor(scopes));
  });

  it('matching grows linearly with the paths', () => {
    const paths = (n: number) => Array.from({ length: n }, (_, i) => `crates/c${i % 50}/src/mod${i % 97}/file${i}.${['rs', 'ts', 'snap', 'md'][i % 4]}`);
    const twentyScopesOfFive = () => Array.from({ length: 20 }, (_, s) => ({
      patterns: [`**/nomatch${s}/**`, `*.zz${s}`, `src${s}/*.rs`, `docs${s}/**`, `!**/skip${s}/**`],
    }));
    const g = growth((n) => {
      const ps = paths(n);
      const scopes = twentyScopesOfFive();
      return () => { const x = matcherFor([...scopes]); for (const p of ps) x.scopeOf(p); };
    }, 12_500, { rounds: 3 });
    expect(g.ratio, `12.5k: ${g.small.toFixed(0)} ms, 50k: ${g.large.toFixed(0)} ms`).toBeLessThan(8);
  }, 60_000);
});
