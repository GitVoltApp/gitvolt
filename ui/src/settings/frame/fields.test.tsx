import { act, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { HOLD_DELAY_MS, NumberField, REPEAT_MS, type NumberFieldProps } from './fields';

/** A field whose value follows its commits, as a setting's does. */
function Field(props: Omit<NumberFieldProps, 'value' | 'onCommit'> & { initial: number; onCommit?(n: number): void }) {
  const [value, setValue] = useState(props.initial);
  return <NumberField {...props} value={value} onCommit={(n) => { props.onCommit?.(n); setValue(n); }} />;
}
const box = () => screen.getByRole('spinbutton') as HTMLInputElement;
const plus = () => screen.getByRole('button', { name: 'Increase font size' });
const minus = () => screen.getByRole('button', { name: 'Decrease font size' });
const key = (k: string, shiftKey = false) => fireEvent.keyDown(box(), { key: k, shiftKey });
const wheel = (deltaY: number) => fireEvent.wheel(box(), { deltaY, deltaMode: 0 });
const press = (el: HTMLElement) => { fireEvent.pointerDown(el, { button: 0, pointerId: 1 }); };
const release = (el: HTMLElement) => { fireEvent.pointerUp(el, { button: 0, pointerId: 1 }); fireEvent.click(el, { detail: 1 }); };

afterEach(() => vi.useRealTimers());

describe('NumberField', () => {
  it('−/+ step at once, clamped, and are disabled at the limits; the browser\'s spin arrows are replaced', () => {
    const onCommit = vi.fn();
    render(<Field id="f" name="font size" initial={30} min={8} max={32} step={1} onCommit={onCommit} />);
    press(plus());
    release(plus());
    expect(box()).toHaveValue(31);
    press(plus());
    release(plus());
    expect(box()).toHaveValue(32);
    expect(plus()).toBeDisabled();
    expect(minus()).toBeEnabled();
    expect(onCommit.mock.calls).toEqual([[31], [32]]);
    // A keyboard click (Enter or Space on the button) steps too.
    fireEvent.click(minus(), { detail: 0 });
    expect(box()).toHaveValue(31);
  });

  it('a held button repeats after a delay, until it is let go or reaches the limit; no timer is left running', () => {
    vi.useFakeTimers();
    render(<Field id="f" name="font size" initial={10} min={8} max={20} step={1} />);
    press(plus());
    expect(box()).toHaveValue(11);
    act(() => { vi.advanceTimersByTime(HOLD_DELAY_MS - 1); });
    expect(box()).toHaveValue(11);
    act(() => { vi.advanceTimersByTime(1); });
    expect(box()).toHaveValue(12);
    act(() => { vi.advanceTimersByTime(REPEAT_MS * 3); });
    expect(box()).toHaveValue(15);
    release(plus());
    expect(vi.getTimerCount()).toBe(0);
    act(() => { vi.advanceTimersByTime(1000); });
    expect(box()).toHaveValue(15);
    // Held to the limit, it stops by itself.
    press(plus());
    act(() => { vi.advanceTimersByTime(HOLD_DELAY_MS + REPEAT_MS * 20); });
    expect(box()).toHaveValue(20);
    expect(vi.getTimerCount()).toBe(0);
    fireEvent.pointerCancel(plus(), { pointerId: 1 });
  });

  it('the wheel steps a focused field only, keeping the page from scrolling', () => {
    render(<Field id="f" name="font size" initial={13} min={8} max={32} step={1} />);
    expect(wheel(-100)).toBe(true); // not focused: the page scrolls, the value stays
    expect(box()).toHaveValue(13);
    act(() => box().focus());
    expect(wheel(-100)).toBe(false); // default prevented
    expect(box()).toHaveValue(14);
    wheel(100);
    wheel(100);
    expect(box()).toHaveValue(12);
  });

  it('↑/↓ step and apply at once, Shift by ten, clamped; typing still applies on Enter or blur', () => {
    const onCommit = vi.fn();
    render(<Field id="f" name="font size" initial={13} min={8} max={32} step={1} onCommit={onCommit} />);
    act(() => box().focus());
    key('ArrowUp');
    expect(box()).toHaveValue(14);
    key('ArrowUp', true);
    expect(box()).toHaveValue(24);
    key('ArrowUp', true);
    expect(box()).toHaveValue(32);
    key('ArrowDown', true);
    expect(box()).toHaveValue(22);
    expect(onCommit.mock.calls).toEqual([[14], [24], [32], [22]]);
    fireEvent.change(box(), { target: { value: '17' } });
    expect(onCommit).toHaveBeenCalledTimes(4);
    // A step goes from what's typed.
    key('ArrowDown');
    expect(box()).toHaveValue(16);
    fireEvent.change(box(), { target: { value: '99' } });
    fireEvent.submit(box().closest('form')!);
    expect(box()).toHaveValue(32);
    fireEvent.change(box(), { target: { value: '9' } });
    fireEvent.blur(box());
    expect(onCommit).toHaveBeenLastCalledWith(9);
  });

  it('keeps its decimals and step: the line height steps by 0.05', () => {
    render(<Field id="f" name="font size" initial={1.55} min={1.2} max={2.4} step={0.05} decimals={2} />);
    act(() => box().focus());
    key('ArrowUp');
    expect(box()).toHaveValue(1.6);
    key('ArrowUp', true);
    expect(box()).toHaveValue(2.1);
    key('ArrowUp', true);
    expect(box()).toHaveValue(2.4);
    key('ArrowDown');
    expect(box()).toHaveValue(2.35);
  });

  it('a blank field (automatic) steps up from its start value, and − at the minimum goes back to blank', () => {
    const onCommit = vi.fn();
    render(<Field id="f" name="font size" initial={0} min={8} max={64} step={1} blank={0} blankStart={18} placeholder="Auto" onCommit={onCommit} />);
    expect(box()).toHaveValue(null);
    expect(minus()).toBeDisabled();
    press(plus());
    release(plus());
    expect(box()).toHaveValue(18);
    expect(onCommit).toHaveBeenLastCalledWith(18);
    act(() => box().focus());
    fireEvent.change(box(), { target: { value: '8' } });
    key('ArrowDown');
    expect(box()).toHaveValue(null);
    expect(onCommit).toHaveBeenLastCalledWith(0);
    key('ArrowUp');
    expect(box()).toHaveValue(18);
  });

  it('a blank without a start steps up to the minimum; typing the blank value is blank', () => {
    render(<Field id="f" name="font size" initial={0} min={20} max={240} step={1} blank={0} />);
    press(plus());
    release(plus());
    expect(box()).toHaveValue(20);
    fireEvent.change(box(), { target: { value: '0' } });
    fireEvent.submit(box().closest('form')!);
    expect(box()).toHaveValue(null);
  });
});
