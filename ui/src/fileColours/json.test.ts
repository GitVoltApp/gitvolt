import { describe, expect, it } from 'vitest';
import { appendScopes, parseScopesJson, scopesToJson } from './json';
import type { FileScope } from './store';

const sc = (name: string): FileScope => ({ id: `id-${name}`, name, tint: 'green', patterns: ['*.snap'], startCollapsed: true });

describe('file colours JSON', () => {
  it('scopesToJson writes the documented shape without ids', () => {
    const j = JSON.parse(scopesToJson([sc('Tests')]));
    expect(j).toEqual({ gitvolt: 'file-colours', version: 1, scopes: [{ name: 'Tests', tint: 'green', patterns: ['*.snap'], startCollapsed: true }] });
    expect(scopesToJson([sc('Tests')])).toContain('\n  "gitvolt"');
  });
  it('parseScopesJson round-trips', () => {
    expect(parseScopesJson(scopesToJson([sc('A'), sc('B')]))).toEqual([
      { name: 'A', tint: 'green', patterns: ['*.snap'], startCollapsed: true },
      { name: 'B', tint: 'green', patterns: ['*.snap'], startCollapsed: true },
    ]);
  });
  it('rejects other kinds, newer versions and non-arrays', () => {
    expect(parseScopesJson('nope')).toBeNull();
    expect(parseScopesJson('{"gitvolt":"theme","version":1,"scopes":[]}')).toBeNull();
    expect(parseScopesJson('{"gitvolt":"file-colours","version":2,"scopes":[]}')).toBeNull();
    expect(parseScopesJson('{"gitvolt":"file-colours","version":1,"scopes":{}}')).toBeNull();
    expect(parseScopesJson('{"gitvolt":"file-colours","version":1}')).toBeNull();
    const many = Array.from({ length: 201 }, (_, i) => ({ name: `n${i}`, patterns: ['a'] }));
    expect(parseScopesJson(JSON.stringify({ gitvolt: 'file-colours', version: 1, scopes: many }))).toBeNull();
    expect(parseScopesJson(`{"gitvolt":"file-colours","version":1,"scopes":[],"pad":"${'x'.repeat(1_000_001)}"}`)).toBeNull();
  });
  it('accepts texts between 64 KiB and 1 MB', () => {
    const big = JSON.stringify({ gitvolt: 'file-colours', version: 1, scopes: [{ name: 'A', patterns: ['a'] }], pad: 'x'.repeat(100_000) });
    expect(parseScopesJson(big)).toHaveLength(1);
  });
  it('drops nameless or empty scopes and maps an unknown tint to gray', () => {
    const text = JSON.stringify({
      gitvolt: 'file-colours', version: 1,
      scopes: [
        { name: '', patterns: ['a'] },
        { name: 'NoPat', patterns: ['  ', ''] },
        { name: ' Ok ', tint: 'chartreuse', patterns: [' a ', ''] },
        { name: 'Hex', tint: '#aabbcc', patterns: ['b'] },
      ],
    });
    expect(parseScopesJson(text)).toEqual([
      { name: 'Ok', tint: 'gray', patterns: ['a'], startCollapsed: false },
      { name: 'Hex', tint: '#aabbcc', patterns: ['b'], startCollapsed: false },
    ]);
  });
  it('caps patterns per scope at 200', () => {
    const patterns = Array.from({ length: 250 }, (_, i) => `p${i}`);
    const out = parseScopesJson(JSON.stringify({ gitvolt: 'file-colours', version: 1, scopes: [{ name: 'A', patterns }] }));
    expect(out?.[0].patterns).toHaveLength(200);
  });
  it('appendScopes renames clashes and gives fresh ids', () => {
    const existing = [sc('Tests')];
    const out = appendScopes(existing, [
      { name: 'Tests', tint: 'blue', patterns: ['x'], startCollapsed: false },
      { name: 'tests', tint: 'blue', patterns: ['y'], startCollapsed: false },
    ]);
    expect(out.map((s) => s.name)).toEqual(['Tests', 'Tests (2)', 'tests (3)']);
    expect(out[0]).toBe(existing[0]);
    expect(new Set(out.map((s) => s.id)).size).toBe(3);
  });
});
