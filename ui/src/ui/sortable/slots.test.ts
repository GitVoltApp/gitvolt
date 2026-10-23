import { describe, expect, it } from 'vitest';
import { shiftsFor, slotFor } from './useSortable';

describe('sortable slots', () => {
  it('an item passes a neighbour once it has travelled past its middle', () => {
    expect(slotFor([10, 10, 10], 0, 16)).toBe(1);
    expect(slotFor([10, 10, 10], 2, -16)).toBe(1);
    expect(slotFor([10, 10, 10], 0, 4)).toBe(0);
    expect(slotFor([10, 10, 10], 0, 25)).toBe(2);
  });

  it('shifts show the order a release would make', () => {
    expect(shiftsFor([10, 10, 10], 0, 2)).toEqual([20, -10, -10]);
  });
});
