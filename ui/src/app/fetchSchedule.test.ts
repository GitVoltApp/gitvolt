import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { FetchScheduler } from './fetchSchedule';

describe('FetchScheduler', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const make = (opts: { last?: number; minimized?: boolean } = {}) => {
    const deps = { now: () => Date.now(), lastFetchAt: () => opts.last ?? 0, isMinimized: vi.fn(async () => opts.minimized ?? false), fetch: vi.fn(async () => {}) };
    return { s: new FetchScheduler(60_000, deps), deps };
  };

  it('fetches at once when the last fetch is older than the interval, then every interval', async () => {
    const { s, deps } = make({ last: 0 });
    s.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(deps.fetch).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(deps.fetch).toHaveBeenCalledTimes(2);
    s.stop();
    await vi.advanceTimersByTimeAsync(120_000);
    expect(deps.fetch).toHaveBeenCalledTimes(2);
  });

  it("a tab's first fetch waits until it has settled; a later activation's doesn't", async () => {
    let settle: () => void = () => {};
    const state = { last: 0 };
    const deps = { now: () => Date.now(), lastFetchAt: () => state.last, isMinimized: vi.fn(async () => false), fetch: vi.fn(async () => { state.last = Date.now(); }), settled: vi.fn(() => new Promise<void>((r) => { settle = r; })) };
    const s = new FetchScheduler(60_000, deps);
    s.start();
    await vi.advanceTimersByTimeAsync(30_000);
    expect(deps.fetch).not.toHaveBeenCalled();
    settle();
    await vi.advanceTimersByTimeAsync(0);
    expect(deps.fetch).toHaveBeenCalledTimes(1);
    s.stop();
    // Shown again after the interval: at once, without waiting.
    await vi.advanceTimersByTimeAsync(60_000);
    s.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(deps.fetch).toHaveBeenCalledTimes(2);
    expect(deps.settled).toHaveBeenCalledTimes(1);
    s.stop();
  });

  it('settling after a stop, or after an interval tick already fetched, fetches nothing more', async () => {
    let settle: () => void = () => {};
    const state = { last: 0 };
    const deps = { now: () => Date.now(), lastFetchAt: () => state.last, isMinimized: vi.fn(async () => false), fetch: vi.fn(async () => { state.last = Date.now(); }), settled: vi.fn((signal: AbortSignal) => new Promise<void>((r) => { settle = r; signal.addEventListener('abort', () => r()); })) };
    const s = new FetchScheduler(60_000, deps);
    s.start();
    s.stop();
    await vi.advanceTimersByTimeAsync(0);
    expect(deps.fetch).not.toHaveBeenCalled();
    s.start();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(deps.fetch).toHaveBeenCalledTimes(1);
    settle();
    await vi.advanceTimersByTimeAsync(0);
    expect(deps.fetch).toHaveBeenCalledTimes(1);
    s.stop();
  });

  it('does not fetch on activation when a fetch happened recently', async () => {
    const { s, deps } = make({ last: Date.now() - 10_000 });
    s.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(deps.fetch).not.toHaveBeenCalled();
    s.stop();
  });

  it('skips ticks while minimized and fetches once when the window is focused again', async () => {
    const state = { minimized: true };
    const deps = { now: () => Date.now(), lastFetchAt: () => Date.now(), isMinimized: vi.fn(async () => state.minimized), fetch: vi.fn(async () => {}) };
    const s = new FetchScheduler(60_000, deps);
    s.start();
    await vi.advanceTimersByTimeAsync(180_000);
    expect(deps.isMinimized).toHaveBeenCalledTimes(3);
    expect(deps.fetch).not.toHaveBeenCalled();
    state.minimized = false;
    s.onFocus(true);
    s.onFocus(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(deps.fetch).toHaveBeenCalledTimes(1);
    s.stop();
  });

  it('focus with no missed tick, or losing focus, fetches nothing (unfocused but visible keeps the timer)', async () => {
    const { s, deps } = make({ last: Date.now() });
    s.start();
    s.onFocus(false);
    s.onFocus(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(deps.fetch).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(deps.fetch).toHaveBeenCalledTimes(1);
    s.stop();
  });

  it('a tick still asking whether the window is minimized when stopped fetches nothing', async () => {
    let answer: (v: boolean) => void = () => {};
    const deps = { now: () => Date.now(), lastFetchAt: () => 0, isMinimized: vi.fn(() => new Promise<boolean>((r) => { answer = r; })), fetch: vi.fn(async () => {}) };
    const s = new FetchScheduler(60_000, deps);
    s.start();
    s.stop();
    answer(false);
    await vi.advanceTimersByTimeAsync(0);
    expect(deps.fetch).not.toHaveBeenCalled();
    // And a missed tick from before the stop isn't replayed on a later focus.
    s.onFocus(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(deps.fetch).not.toHaveBeenCalled();
  });

  it('a tick while a fetch is still in flight starts no second one', async () => {
    let finish: () => void = () => {};
    const deps = { now: () => Date.now(), lastFetchAt: () => 0, isMinimized: vi.fn(async () => false), fetch: vi.fn(() => new Promise<void>((r) => { finish = r; })) };
    const s = new FetchScheduler(60_000, deps);
    s.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(deps.fetch).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(180_000);
    expect(deps.fetch).toHaveBeenCalledTimes(1);
    finish();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(deps.fetch).toHaveBeenCalledTimes(2);
    s.stop();
  });

  it('a normal tick that fetches clears a missed one: the next focus replays nothing', async () => {
    const state = { minimized: true };
    const deps = { now: () => Date.now(), lastFetchAt: () => Date.now(), isMinimized: vi.fn(async () => state.minimized), fetch: vi.fn(async () => {}) };
    const s = new FetchScheduler(60_000, deps);
    s.start();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(deps.fetch).not.toHaveBeenCalled();
    state.minimized = false;
    await vi.advanceTimersByTimeAsync(60_000);
    expect(deps.fetch).toHaveBeenCalledTimes(1);
    s.onFocus(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(deps.fetch).toHaveBeenCalledTimes(1);
    s.stop();
  });

  // A user's op cancels a background fetch (it comes back skipped: busy); it runs
  // again once the queue is idle, not an interval later. Owed only once: an idle queue with nothing
  // owed fetches nothing.
  it('a fetch skipped because the queue was busy runs again when the queue goes idle', async () => {
    const results = [true, false];
    const deps = { now: () => Date.now(), lastFetchAt: () => 0, isMinimized: vi.fn(async () => false), fetch: vi.fn(async () => results.shift() ?? false) };
    const s = new FetchScheduler(60_000, deps);
    s.onIdle();
    s.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(deps.fetch).toHaveBeenCalledTimes(1);
    s.onIdle();
    await vi.advanceTimersByTimeAsync(0);
    expect(deps.fetch).toHaveBeenCalledTimes(2);
    s.onIdle();
    await vi.advanceTimersByTimeAsync(0);
    expect(deps.fetch).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(deps.fetch).toHaveBeenCalledTimes(3);
    // Stopped (the tab hidden): an idle queue then fetches nothing.
    results.push(true);
    await vi.advanceTimersByTimeAsync(60_000);
    s.stop();
    s.onIdle();
    await vi.advanceTimersByTimeAsync(0);
    expect(deps.fetch).toHaveBeenCalledTimes(4);
  });

  it('an interval of 0 never fetches', async () => {
    const deps = { now: () => Date.now(), lastFetchAt: () => 0, isMinimized: vi.fn(async () => false), fetch: vi.fn(async () => {}) };
    const s = new FetchScheduler(0, deps);
    s.start();
    await vi.advanceTimersByTimeAsync(600_000);
    expect(deps.fetch).not.toHaveBeenCalled();
    expect(deps.isMinimized).not.toHaveBeenCalled();
  });
});
