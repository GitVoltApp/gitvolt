import { useMemo, useSyncExternalStore } from 'react';

/** `relativeTime`'s units, largest first, as `[seconds]`; below the last one the text is "just now". */
const UNIT_SECS = [31_536_000, 2_592_000, 604_800, 86_400, 3600, 60];
const MIN_DELAY = 1;
const MAX_DELAY = 21_600;

/**
 * Unix seconds when `relativeTime(at, now)`'s text next changes: when its rounded count does, or
 * when its unit does, whichever is sooner. Clamped to 1 s .. 6 h after `now`, so a clock jump or a
 * commit dated in the future can't make the timer spin.
 */
export function nextChange(at: number, now: number): number {
  const age = Math.abs(now - at);
  const past = now >= at;
  const idx = UNIT_SECS.findIndex((secs) => age >= secs);
  let next = Infinity;
  if (idx < 0) {
    // "just now" until a minute after `at` (a future date within a minute reads the same).
    next = at + 60;
  } else {
    const secs = UNIT_SECS[idx];
    const count = Math.round(age / secs);
    if (past) {
      next = at + secs * (count + 0.5);
      if (idx > 0) next = Math.min(next, at + UNIT_SECS[idx - 1]);
    } else {
      // Moving toward `at`: the count drops, then the unit drops, as `at` nears.
      next = at - secs * (count - 0.5);
      next = Math.min(next, at - secs);
    }
  }
  return Math.min(Math.max(next, now + MIN_DELAY), now + MAX_DELAY);
}

interface Entry { at: number; now: number; next: number; listener: (() => void) | null }

const entries = new Set<Entry>();
let timer: ReturnType<typeof setTimeout> | undefined;

const nowSecs = () => Math.floor(Date.now() / 1000);

function schedule(): void {
  clearTimeout(timer);
  timer = undefined;
  if (entries.size === 0 || (typeof document !== 'undefined' && document.hidden)) return;
  const now = nowSecs();
  let soonest = Infinity;
  for (const e of entries) soonest = Math.min(soonest, e.next);
  // +1 s: `relativeTime` rounds a past half-way point toward zero, so its text changes just after it.
  const delay = Math.min(Math.max(soonest - now + 1, MIN_DELAY), MAX_DELAY);
  timer = setTimeout(() => tick(false), delay * 1000);
}

/** Moves the subscribers whose text can have changed (all of them when `all`) to the current time. */
function tick(all: boolean): void {
  const now = nowSecs();
  for (const e of [...entries]) {
    if (!all && now < e.next) continue;
    e.now = now;
    e.next = nextChange(e.at, now);
    e.listener?.();
  }
  schedule();
}

if (typeof document !== 'undefined') {
  document.addEventListener('visibilitychange', () => {
    if (document.hidden) {
      clearTimeout(timer);
      timer = undefined;
    } else tick(true);
  });
}

const NO_STORE = { subscribe: () => () => {}, getSnapshot: () => 0 };

/**
 * The `now` (unix seconds) to format the relative date `at` with. Re-renders the caller only when
 * its text can change; one shared timer serves every caller, and none runs while no relative date
 * shows (`enabled` false) or the page is hidden.
 */
export function useRelativeNow(at: number, enabled: boolean): number {
  const store = useMemo(() => {
    if (!enabled) return NO_STORE;
    const now = nowSecs();
    const entry: Entry = { at, now, next: nextChange(at, now), listener: null };
    return {
      subscribe: (listener: () => void) => {
        entry.listener = listener;
        entries.add(entry);
        schedule();
        return () => {
          entries.delete(entry);
          schedule();
        };
      },
      getSnapshot: () => entry.now,
    };
  }, [at, enabled]);
  return useSyncExternalStore(store.subscribe, store.getSnapshot);
}
