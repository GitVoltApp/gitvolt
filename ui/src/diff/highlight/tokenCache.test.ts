import { describe, expect, it } from 'vitest';
import { TokenCache } from './tokenCache';

/** `n` lines of one 2-token line each: 48 + 16 bytes a line. */
const lines = (n: number) => Array.from({ length: n }, () => Uint32Array.of(1, 2, 3, 4));

describe('TokenCache', () => {
  it('hands back what it keeps, and the most recently used outlives the rest', () => {
    const c = new TokenCache(64 * 100, 3);
    const a = lines(10);
    c.set('a', a);
    c.set('b', lines(10));
    c.set('c', lines(10));
    expect(c.get('a')).toBe(a);
    c.set('d', lines(10));
    expect(c.get('b')).toBeUndefined();
    expect(c.get('a')).toBe(a);
    expect(c.size).toBe(3);
  });

  it('evicts by size, least recently used first, and never keeps a text over a quarter of it', () => {
    const c = new TokenCache(64 * 40, 100);
    c.set('a', lines(10));
    c.set('b', lines(10));
    c.set('c', lines(10));
    c.get('a');
    c.set('d', lines(10));
    c.set('e', lines(10));
    expect(['a', 'b', 'c', 'd', 'e'].map((k) => c.get(k) !== undefined)).toEqual([true, false, true, true, true]);
    c.set('huge', lines(11));
    expect(c.get('huge')).toBeUndefined();
    // Setting a key again replaces it.
    const fresh = lines(2);
    c.set('a', fresh);
    expect(c.get('a')).toBe(fresh);
  });
});
