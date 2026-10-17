import { describe, expect, it } from 'vitest';
import { anchoredScrollTop, revealScrollTop } from './anchor';

const rows = (...ids: string[]) => ids.map((id) => ({ id }));

describe('anchoredScrollTop', () => {
  it('keeps the anchor row where it was when rows are inserted above it', () => {
    expect(anchoredScrollTop(rows('a', 'b', 'c'), rows('x', 'y', 'a', 'b', 'c'), 'b', 100, 25)).toBe(150);
  });
  it('uses the density row height it is given', () => {
    expect(anchoredScrollTop(rows('a', 'b'), rows('x', 'a', 'b'), 'b', 10, 28)).toBe(38);
  });
  it('at the very top, stays at the top: rows inserted above (a new row-0 WIP, K37) show', () => {
    expect(anchoredScrollTop(rows('a', 'b'), rows('x', 'a', 'b'), 'a', 0, 28)).toBe(0);
    expect(anchoredScrollTop(rows('a', 'b'), rows('x', 'y', 'a', 'b'), 'b', 0.5, 25)).toBe(0.5);
  });
  it('leaves the scroll alone without an anchor, or when the anchor disappeared', () => {
    expect(anchoredScrollTop(rows('a'), rows('b', 'a'), null, 40, 25)).toBe(40);
    expect(anchoredScrollTop(rows('a', 'b'), rows('c'), 'a', 40, 25)).toBe(40);
  });
  it('never goes negative', () => {
    expect(anchoredScrollTop(rows('x', 'y', 'a'), rows('a'), 'a', 10, 25)).toBe(0);
  });
});

describe('revealScrollTop', () => {
  // 30 rows of 25 px in a 100 px viewport: 4 rows show.
  it('does not move while the row is fully on screen', () => {
    expect(revealScrollTop(5, 100, 100, 25, 30)).toBe(100);
    expect(revealScrollTop(7, 100, 100, 25, 30)).toBe(100);
  });
  it('brings an off-screen row in with two rows of context above it', () => {
    expect(revealScrollTop(20, 0, 100, 25, 30)).toBe(450);
    expect(revealScrollTop(1, 500, 100, 25, 30)).toBe(0);
  });
  it('treats a partly visible row as off screen, and stays within the scrollable range', () => {
    expect(revealScrollTop(4, 10, 100, 25, 30)).toBe(50);
    expect(revealScrollTop(29, 0, 100, 25, 30)).toBe(650);
  });
  it('does nothing for a missing row', () => {
    expect(revealScrollTop(-1, 40, 100, 25, 30)).toBe(40);
  });
});
