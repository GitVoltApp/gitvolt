import { describe, expect, it } from 'vitest';
import { noteViewport, UNLOAD_AFTER_MS, unloadDue, type OlderState } from './olderWindow';

const base: OlderState = { baseRows: 100, lastInOlderAt: 0 };
const at = (now: number, o: Partial<{ firstRow: number; lastRow: number; selectedRow: number; revealRow: number | null }> = {}) => ({ firstRow: 0, lastRow: 20, selectedRow: 3, revealRow: null, now, ...o });

describe('unloadDue', () => {
  it('never while reading the older part', () => {
    const s = noteViewport(base, 90, 130, 1000);
    expect(s.lastInOlderAt).toBe(1000);
    expect(unloadDue(s, at(10 * UNLOAD_AFTER_MS, { firstRow: 90, lastRow: 130 }))).toBeNull();
    expect(unloadDue(s, at(10 * UNLOAD_AFTER_MS, { lastRow: 50 }))).toBeNull(); // still the second half of the base window
  });
  it('after three minutes back near the top', () => {
    const s = noteViewport(base, 90, 130, 1000);
    expect(unloadDue(s, at(1000 + 1))).toBe(UNLOAD_AFTER_MS - 1);
    expect(unloadDue(s, at(1000 + UNLOAD_AFTER_MS))).toBe(0);
    expect(unloadDue(s, at(1000 + UNLOAD_AFTER_MS + 5))).toBe(0);
    expect(noteViewport(s, 0, 20, 5000)).toBe(s); // the clock keeps its time while it's up top
  });
  it('not while the selection is in the older part', () => {
    expect(unloadDue(base, at(UNLOAD_AFTER_MS * 2, { selectedRow: 100 }))).toBeNull();
    expect(unloadDue(base, at(UNLOAD_AFTER_MS * 2, { selectedRow: 99 }))).toBe(0);
  });
  it('not while a reveal panel is open there', () => {
    expect(unloadDue(base, at(UNLOAD_AFTER_MS * 2, { revealRow: 120 }))).toBeNull();
    expect(unloadDue(base, at(UNLOAD_AFTER_MS * 2, { revealRow: 4 }))).toBe(0);
  });
});
