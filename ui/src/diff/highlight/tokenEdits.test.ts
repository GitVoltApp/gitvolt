import { describe, expect, it } from 'vitest';
import { editTokens } from './tokenEdits';

// `let ab = 1`: `let ` (A), `ab` (B), ` = ` (C), `1` (D), as (end offset, metadata) pairs.
const A = 10;
const B = 20;
const C = 30;
const D = 40;
const line = Uint32Array.of(4, A, 6, B, 9, C, 10, D);
const run = (t: Uint32Array | undefined) => (t ? [...t] : t);

describe('editTokens', () => {
  it('typing continues the token it follows; the text after shifts', () => {
    expect(editTokens(line, line, 6, 6, ['c']).map(run)).toEqual([[4, A, 7, B, 10, C, 11, D]]);
    // At a token's start: the one before it goes on.
    expect(editTokens(line, line, 4, 4, ['x']).map(run)).toEqual([[5, A, 7, B, 10, C, 11, D]]);
  });

  it('a replacement takes the style of what it replaced; a deletion closes up', () => {
    expect(editTokens(line, line, 4, 6, ['value']).map(run)).toEqual([[4, A, 9, B, 12, C, 13, D]]);
    expect(editTokens(line, line, 4, 9, ['']).map(run)).toEqual([[4, A, 5, D]]);
  });

  it('a line break splits the line: the new line starts with the text after it', () => {
    expect(editTokens(line, line, 6, 6, ['', '  ']).map(run)).toEqual([[4, A, 6, B], [2, B, 5, C, 6, D]]);
  });

  it('joining lines keeps the first line\'s head and the last line\'s tail', () => {
    const next = Uint32Array.of(3, D, 5, A);
    expect(editTokens(line, next, 10, 3, ['']).map(run)).toEqual([[4, A, 6, B, 9, C, 10, D, 12, A]]);
  });

  it('an empty result keeps one token; unknown tokens stay unknown (plain)', () => {
    expect(editTokens(line, line, 0, 10, ['']).map(run)).toEqual([[0, A]]);
    expect(editTokens(undefined, line, 0, 0, ['a', 'b'])).toEqual([undefined, undefined]);
  });
});
