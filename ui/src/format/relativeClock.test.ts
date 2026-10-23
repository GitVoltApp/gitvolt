// @vitest-environment happy-dom
import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextChange, useRelativeNow } from './relativeClock';

const NOW = 1_800_000_000;

describe('nextChange', () => {
  it('finds when the text next changes', () => {
    expect(nextChange(NOW - 30, NOW)).toBe(NOW - 30 + 60);
    expect(nextChange(NOW - 90, NOW)).toBe(NOW - 90 + 150);
    expect(nextChange(NOW - 10_800, NOW)).toBe(NOW - 10_800 + 12_600);
  });

  it('stays within 1 s to 6 h for any date, including the future', () => {
    for (const at of [NOW + 120, NOW + 86_400 * 400, NOW - 5, NOW - 86_400 * 800, NOW, NOW - 59, NOW + 59, NOW - 7200]) {
      const n = nextChange(at, NOW);
      expect(n).toBeGreaterThanOrEqual(NOW + 1);
      expect(n).toBeLessThanOrEqual(NOW + 21_600);
    }
    expect(nextChange(NOW + 120, NOW)).toBeGreaterThan(NOW);
  });
});

describe('useRelativeNow', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW * 1000);
    Object.defineProperty(document, 'hidden', { configurable: true, value: false });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('runs one timer for all subscribers and none when there are none', () => {
    const a = renderHook(() => useRelativeNow(NOW - 30, true));
    const b = renderHook(() => useRelativeNow(NOW - 600, true));
    expect(vi.getTimerCount()).toBe(1);
    a.unmount();
    expect(vi.getTimerCount()).toBe(1);
    b.unmount();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('subscribes nothing when disabled', () => {
    const { result } = renderHook(() => useRelativeNow(NOW - 30, false));
    expect(result.current).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('pauses while hidden and ticks at once when visible again', () => {
    let renders = 0;
    renderHook(() => { renders++; return useRelativeNow(NOW - 30, true); });
    expect(vi.getTimerCount()).toBe(1);
    Object.defineProperty(document, 'hidden', { configurable: true, value: true });
    document.dispatchEvent(new Event('visibilitychange'));
    expect(vi.getTimerCount()).toBe(0);
    vi.setSystemTime((NOW + 5) * 1000);
    const before = renders;
    Object.defineProperty(document, 'hidden', { configurable: true, value: false });
    act(() => { document.dispatchEvent(new Event('visibilitychange')); });
    expect(renders).toBeGreaterThan(before);
    expect(vi.getTimerCount()).toBe(1);
  });

  it('re-renders only the subscriber whose text changed', () => {
    let a = 0;
    let b = 0;
    renderHook(() => { a++; return useRelativeNow(NOW - 30, true); });
    renderHook(() => { b++; return useRelativeNow(NOW - 86_400, true); });
    const [a0, b0] = [a, b];
    act(() => { vi.advanceTimersByTime(31_000); });
    expect(a).toBeGreaterThan(a0);
    expect(b).toBe(b0);
  });

  it('survives the clock set back a day: one timer, at least 1 s', () => {
    renderHook(() => useRelativeNow(NOW - 30, true));
    vi.setSystemTime((NOW - 86_400) * 1000);
    act(() => { vi.advanceTimersByTime(31_000); });
    expect(vi.getTimerCount()).toBe(1);
    const spy = vi.spyOn(globalThis, 'setTimeout');
    act(() => { vi.advanceTimersToNextTimer(); });
    for (const c of spy.mock.calls) expect(c[1] as number).toBeGreaterThanOrEqual(1000);
    expect(vi.getTimerCount()).toBe(1);
  });
});
