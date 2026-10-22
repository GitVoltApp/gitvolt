import { describe, expect, it } from 'vitest';
import { compilePattern } from './glob';
import { addPattern, moveScope, newScope, nextTint, repoLabel, uniqueName } from './scopes';
import { STARTER_SCOPES } from './starters';
import type { FileScope } from './store';

const sc = (name: string, tint = 'green', patterns: string[] = ['a']): FileScope => ({ id: name, name, tint, patterns, startCollapsed: false });

describe('scopes', () => {
  it("moveScope keeps the others' order", () => {
    const l = [sc('a'), sc('b'), sc('c'), sc('d')];
    expect(moveScope(l, 0, 2).map((s) => s.name)).toEqual(['b', 'c', 'a', 'd']);
    expect(moveScope(l, 3, 1).map((s) => s.name)).toEqual(['a', 'd', 'b', 'c']);
  });
  it('addPattern is a no-op when present', () => {
    const l = [sc('a')];
    expect(addPattern(l, 0, 'a')[0]).toBe(l[0]);
    expect(addPattern(l, 0, 'b')[0].patterns).toEqual(['a', 'b']);
  });
  it('uniqueName adds (2) then (3)', () => {
    expect(uniqueName(['Tests'], 'Tests')).toBe('Tests (2)');
    expect(uniqueName(['Tests', 'Tests (2)'], 'Tests')).toBe('Tests (3)');
    expect(uniqueName([], 'Tests')).toBe('Tests');
  });
  it('nextTint takes the first unused preset, then gray', () => {
    expect(nextTint([])).toBe('green');
    expect(nextTint([sc('a', 'green'), sc('b', 'blue')])).toBe('orange');
    const all = ['green', 'blue', 'orange', 'violet', 'rose', 'yellow', 'gray'].map((t) => sc(t, t));
    expect(nextTint(all)).toBe('gray');
  });
  it('newScope defaults its name and tint', () => {
    const n = newScope([sc('New scope')], ['x']);
    expect(n.name).toBe('New scope (2)');
    expect(n.tint).toBe('blue');
    expect(newScope([], ['x'], 'Mine').name).toBe('Mine');
  });
  it('repoLabel', () => {
    expect(repoLabel('/h/orbital/.git')).toBe('orbital');
    expect(repoLabel('/srv/orbital.git')).toBe('orbital');
    expect(repoLabel('/h/orbital/.git/')).toBe('orbital');
  });
  it('every starter pattern compiles', () => {
    for (const s of STARTER_SCOPES) for (const p of s.patterns) expect(compilePattern(p), p).not.toBeNull();
    expect(STARTER_SCOPES.map((s) => s.name)).toEqual(['Tests', 'Generated', 'Docs']);
  });
});
