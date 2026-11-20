import { afterEach, describe, expect, it, vi } from 'vitest';
import { HIDDEN_HOLD_MS, releaseWhenHidden } from './hiddenHold';

describe('releaseWhenHidden', () => {
  afterEach(() => vi.useRealTimers());
  // Hidden by <Activity>, an element stays in the document; unmounted, it leaves it.
  const shown = { isConnected: true } as Element;

  it('releases a hidden element after the hold, once', () => {
    vi.useFakeTimers();
    const release = vi.fn();
    releaseWhenHidden(shown, release);
    vi.advanceTimersByTime(HIDDEN_HOLD_MS - 1);
    expect(release).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    vi.advanceTimersByTime(HIDDEN_HOLD_MS);
    expect(release).toHaveBeenCalledTimes(1);
  });

  it('shown again within the hold, nothing is released', () => {
    vi.useFakeTimers();
    const release = vi.fn();
    const cancel = releaseWhenHidden(shown, release);
    vi.advanceTimersByTime(HIDDEN_HOLD_MS / 2);
    cancel();
    vi.advanceTimersByTime(HIDDEN_HOLD_MS);
    expect(release).not.toHaveBeenCalled();
  });

  it('an unmounted element (out of the document) is released at once', () => {
    vi.useFakeTimers();
    const release = vi.fn();
    releaseWhenHidden({ isConnected: false } as Element, release);
    vi.advanceTimersByTime(0);
    expect(release).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(HIDDEN_HOLD_MS);
    expect(release).toHaveBeenCalledTimes(1);
  });
});
