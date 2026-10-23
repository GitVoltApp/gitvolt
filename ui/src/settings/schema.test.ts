import { describe, expect, it } from 'vitest';
import { clampFetchInterval } from './schema';

describe('clampFetchInterval', () => {
  it('keeps 0 (or less, or NaN) as off and clamps the rest to a minute..a day', () => {
    expect(clampFetchInterval(0)).toBe(0);
    expect(clampFetchInterval(-5)).toBe(0);
    expect(clampFetchInterval(Number.NaN)).toBe(0);
    expect(clampFetchInterval(1)).toBe(60);
    expect(clampFetchInterval(300)).toBe(300);
    expect(clampFetchInterval(10 ** 9)).toBe(86_400);
  });
});
