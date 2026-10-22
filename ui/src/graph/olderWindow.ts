// When commits loaded past the base window ("Load older commits", Find's deepen) are let go
// again: pure, so the one timer that acts on it (runtime.ts) stays a few lines.

export const UNLOAD_AFTER_MS = 180_000;

/** `baseRows`: rows the base window had; `lastInOlderAt`: when the viewport last reached past them. */
export interface OlderState { baseRows: number; lastInOlderAt: number }

/** The viewport's rows reached past the base window: that is when the clock restarts. */
export function noteViewport(s: OlderState, _firstRow: number, lastRow: number, now: number): OlderState {
  return lastRow >= s.baseRows ? { ...s, lastInOlderAt: now } : s;
}

/**
 * When to check again (ms from now), or null: it can't become due without a scroll or a
 * selection. Unload now is `0`. Unloading needs the viewport in the first half of the base window,
 * and neither the selection nor an open reveal panel in the older part.
 */
export function unloadDue(s: OlderState, a: { firstRow: number; lastRow: number; selectedRow: number; revealRow: number | null; now: number }): number | null {
  if (a.lastRow >= s.baseRows / 2) return null;
  if (a.selectedRow >= s.baseRows) return null;
  if (a.revealRow !== null && a.revealRow >= s.baseRows) return null;
  return Math.max(0, s.lastInOlderAt + UNLOAD_AFTER_MS - a.now);
}
