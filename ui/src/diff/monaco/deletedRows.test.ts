import { describe, expect, it } from 'vitest';
import { chunksNear, widestColumns, ZoneRows } from './deletedRows';

describe("a lazily drawn deleted block's rows", () => {
  // Five old lines, the second wrapping onto three rows: rows 0 | 1 2 3 | 4 | 5 | 6.
  const rows = new ZoneRows([1, 3, 1, 1, 1], 2);

  it('counts every row, wrapped ones too, and maps each row to its old line and back', () => {
    expect(rows.rows).toBe(7);
    expect([0, 1, 2, 3, 4, 5, 6].map((r) => rows.lineOfRow(r))).toEqual([0, 1, 1, 1, 2, 3, 4]);
    expect([0, 1, 2, 3, 4].map((l) => rows.firstRowOf(l))).toEqual([0, 1, 4, 5, 6]);
    // Past either end: the first or last line.
    expect(rows.lineOfRow(-3)).toBe(0);
    expect(rows.lineOfRow(99)).toBe(4);
  });

  it('splits the lines into chunks, each with its first row and its rows', () => {
    expect(rows.chunks).toBe(3);
    expect(rows.chunk(0)).toEqual({ start: 0, end: 2, firstRow: 0, rows: 4 });
    expect(rows.chunk(1)).toEqual({ start: 2, end: 4, firstRow: 4, rows: 2 });
    expect(rows.chunk(2)).toEqual({ start: 4, end: 5, firstRow: 6, rows: 1 });
  });

  it('finds the chunks a range of rows touches, none outside the block', () => {
    expect(rows.chunksIn(0, 1)).toEqual([0, 0]);
    expect(rows.chunksIn(3, 5)).toEqual([0, 1]);
    expect(rows.chunksIn(-50, 50)).toEqual([0, 2]);
    expect(rows.chunksIn(7, 20)).toBeNull();
    expect(rows.chunksIn(-20, 0)).toBeNull();
  });

  it('draws the chunks on screen and half a screen either side; none when the zone is far away', () => {
    const big = new ZoneRows(Array.from({ length: 20_000 }, () => 1));
    // On screen from its top: rows 0-39 on a 760 px viewport, and down to row 59.
    expect(chunksNear(big, 0, 760, 19)).toEqual([0, 0]);
    // Scrolled 10,000 rows in: rows 9,980-10,059, chunks 99 and 100.
    expect(chunksNear(big, -10_000 * 19, 760, 19)).toEqual([99, 100]);
    // Below the viewport, and far above it.
    expect(chunksNear(big, 2000, 760, 19)).toBeNull();
    expect(chunksNear(big, -1e7, 760, 19)).toBeNull();
  });

  it("measures the widest line in columns, tabs to the next stop, as Monaco's minimum width", () => {
    expect(widestColumns(['ab', 'abcd', ''], 4)).toBe(4);
    expect(widestColumns(['\tx', 'ab\tx'], 4)).toBe(5);
    expect(widestColumns([], 4)).toBe(0);
  });
});
