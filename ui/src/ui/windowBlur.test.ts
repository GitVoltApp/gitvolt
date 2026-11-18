// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { BLURRED_AFTER_MS, installWindowBlurredMark, isWindowBlur, refocusWhenWindowReturns } from './windowBlur';

describe('windowBlur (K41)', () => {
  afterEach(() => vi.restoreAllMocks());

  it('a blur while the document has no focus is the window losing it; with focus, a real move', () => {
    const has = vi.spyOn(document, 'hasFocus');
    has.mockReturnValue(false);
    expect(isWindowBlur()).toBe(true);
    has.mockReturnValue(true);
    expect(isWindowBlur()).toBe(false);
  });

  it('puts the caret back once the window returns, unless editing ended meanwhile', () => {
    const input = document.createElement('input');
    document.body.append(input);
    refocusWhenWindowReturns(input);
    window.dispatchEvent(new Event('focus'));
    expect(document.activeElement).toBe(input);

    input.blur();
    let editing = true;
    refocusWhenWindowReturns(input, () => editing);
    editing = false;
    window.dispatchEvent(new Event('focus'));
    expect(document.activeElement).not.toBe(input);
    input.remove();
  });
});

describe('installWindowBlurredMark', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it('marks the root once the window has been without focus a moment, not for the WM bounce of a click', () => {
    vi.useFakeTimers();
    const has = vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const root = document.documentElement;
    const uninstall = installWindowBlurredMark();
    expect(root.hasAttribute('data-window-blurred')).toBe(false);

    // A click's bounce: blurred, focused again a few ms later.
    has.mockReturnValue(false);
    window.dispatchEvent(new Event('blur'));
    vi.advanceTimersByTime(5);
    has.mockReturnValue(true);
    window.dispatchEvent(new Event('focus'));
    vi.advanceTimersByTime(BLURRED_AFTER_MS);
    expect(root.hasAttribute('data-window-blurred')).toBe(false);

    // Another window took the focus.
    has.mockReturnValue(false);
    window.dispatchEvent(new Event('blur'));
    vi.advanceTimersByTime(BLURRED_AFTER_MS);
    expect(root.hasAttribute('data-window-blurred')).toBe(true);
    has.mockReturnValue(true);
    window.dispatchEvent(new Event('focus'));
    expect(root.hasAttribute('data-window-blurred')).toBe(false);

    has.mockReturnValue(false);
    window.dispatchEvent(new Event('blur'));
    vi.advanceTimersByTime(BLURRED_AFTER_MS);
    uninstall();
    expect(root.hasAttribute('data-window-blurred')).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });
});
