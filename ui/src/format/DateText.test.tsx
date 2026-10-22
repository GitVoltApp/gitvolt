// @vitest-environment happy-dom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useAppState } from '../app/state';
import { DateText } from './DateText';

const NOW = 1_800_000_000;
const setRelative = (relativeDates: boolean) => useAppState.setState((s) => ({ settings: { ...s.settings, relativeDates, dateFormat: 'ymd12h' } }));
const hover = () => { act(() => { fireEvent.mouseEnter(screen.getByTestId('d')); }); return screen.getByRole('tooltip').textContent ?? ''; };

describe('DateText', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW * 1000);
  });
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it('shows the exact date, and "ago" on hover, when relative is off', () => {
    setRelative(false);
    render(<DateText at={NOW - 300} testId="d" />);
    expect(screen.getByTestId('d').textContent).toMatch(/^\d{4}-\d{2}-\d{2}/);
    expect(hover()).toBe('5 minutes ago');
    expect(vi.getTimerCount()).toBe(0);
  });

  it('shows "5 minutes ago", the exact date on hover, and ticks without another timer', () => {
    setRelative(true);
    render(<DateText at={NOW - 300} testId="d" />);
    expect(screen.getByTestId('d').textContent).toBe('5 minutes ago');
    expect(hover()).toMatch(/^\d{4}-\d{2}-\d{2}/);
    const timers = vi.getTimerCount();
    act(() => { vi.advanceTimersByTime(60_000 * 2); });
    expect(screen.getByTestId('d').textContent).toBe('7 minutes ago');
    expect(vi.getTimerCount()).toBe(timers);
  });

  it('puts the prefix first', () => {
    setRelative(true);
    render(<DateText at={NOW - 300} prefix="authored" testId="d" />);
    expect(screen.getByTestId('d').textContent).toBe('authored 5 minutes ago');
  });
});
