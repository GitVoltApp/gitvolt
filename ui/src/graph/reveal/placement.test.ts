import { describe, expect, it } from 'vitest';
import { panelBox, REVEAL_MIN_ROWS, revealPlacement } from './placement';

const rowH = 28;
const at = (rowIndex: number, count: number, o: Partial<{ viewportTop: number; viewportH: number; fold: boolean }> = {}) => {
  const viewportTop = o.viewportTop ?? 0;
  return { rowTop: rowIndex * rowH, viewportTop, viewportBottom: viewportTop + (o.viewportH ?? 20 * rowH), rowH, count, fold: o.fold ?? false };
};

describe('revealPlacement', () => {
  it('opens above when there is room, else below', () => {
    // Room above for the title and 8 rows: above, its bottom edge on the row's top.
    expect(revealPlacement(at(9, 58))).toEqual({ below: false, top: 0, height: 9 * rowH });
    // One row short: below, under the row, as tall as the space left (title included).
    const b = revealPlacement(at(8, 58));
    expect(b.below).toBe(true);
    expect(b.top).toBe(9 * rowH);
    expect(b.height).toBe(11 * rowH);
    // A shorter run needs less room above: the title and its own rows.
    expect(revealPlacement(at(4, 3))).toEqual({ below: false, top: 0, height: 4 * rowH });
    expect(revealPlacement(at(3, 3)).below).toBe(true);
  });

  it('counts the room from the viewport, not the content', () => {
    // Row 30, scrolled so it sits 5 rows down: no room above.
    expect(revealPlacement(at(30, 58, { viewportTop: 25 * rowH })).below).toBe(true);
    // Lots of room above: the panel takes all of it, in whole rows.
    const p = revealPlacement(at(30, 58, { viewportTop: 10 * rowH }));
    expect(p).toEqual({ below: false, top: 10 * rowH, height: 20 * rowH });
  });

  it('a fold always opens below', () => {
    const p = revealPlacement(at(15, 6, { fold: true, viewportH: 30 * rowH }));
    expect(p.below).toBe(true);
    expect(p.top).toBe(16 * rowH);
    expect(p.height).toBe(7 * rowH);
  });

  it('keeps at least the title and one row when the viewport is tiny', () => {
    expect(panelBox(true, { rowTop: 0, viewportTop: 0, viewportBottom: rowH, rowH, rows: 40 })).toEqual({ top: rowH, height: 2 * rowH });
    expect(REVEAL_MIN_ROWS).toBe(8);
  });
});
