import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ColourSwatch } from './ColourSwatch';
import { parseColour, toOklch } from './colour';

function setup(value = '#4d88ff', alpha = false) {
  const fns = { onPreview: vi.fn(), onCommit: vi.fn(), onCancel: vi.fn() };
  render(<ColourSwatch value={value} alpha={alpha} label="Accent" {...fns} />);
  fireEvent.click(screen.getByRole('button', { name: 'Accent' }));
  return fns;
}
const type = (label: string, v: string) => fireEvent.change(screen.getByLabelText(label), { target: { value: v } });
const num = (label: string) => Number((screen.getByLabelText(label) as HTMLInputElement).value);
const oklchTab = () => fireEvent.click(screen.getByRole('tab', { name: 'OKLCH' }));
const square = () => screen.getByRole('slider', { name: 'Chroma and lightness' });
const last = (f: ReturnType<typeof setup>) => toOklch(parseColour(f.onPreview.mock.lastCall![0] as string)!);

describe('ColourPicker', () => {
  it('previews every change, commits on Enter', () => {
    const f = setup();
    type('Hex', '#ff0000');
    expect(f.onPreview).toHaveBeenLastCalledWith('#ff0000');
    fireEvent.keyDown(screen.getByLabelText('Hex'), { key: 'Enter' });
    expect(f.onCommit).toHaveBeenCalledWith('#ff0000');
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('Esc cancels and never commits', () => {
    const f = setup();
    type('Hex', '#ff0000');
    fireEvent.keyDown(screen.getByLabelText('Hex'), { key: 'Escape' });
    expect(f.onCancel).toHaveBeenCalledTimes(1);
    expect(f.onCommit).not.toHaveBeenCalled();
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('a click outside keeps the new colour', () => {
    const f = setup();
    type('Hex', '#ff0000');
    fireEvent.pointerDown(document.body);
    expect(f.onCommit).toHaveBeenCalledWith('#ff0000');
  });

  it('opens on the square, focused, and offers opacity only with alpha', () => {
    setup('#4d88ff', false);
    expect(document.activeElement).toBe(square());
    expect(screen.queryByRole('slider', { name: 'Opacity' })).toBeNull();
    oklchTab();
    expect(screen.queryByLabelText('Opacity')).toBeNull();
  });

  it('with alpha, the opacity strip and slider show and edit it, as rgba', () => {
    const f = setup('rgba(92, 184, 92, 0.12)', true);
    expect(screen.getByRole('slider', { name: 'Opacity' })).toHaveAttribute('aria-valuenow', '12');
    oklchTab();
    expect(screen.getByLabelText('Opacity')).toHaveValue(12);
    type('Opacity', '50');
    expect(f.onPreview).toHaveBeenLastCalledWith('rgba(92, 184, 92, 0.5)');
  });

  it('the OKLCH tab shows lightness, chroma in hundredths and hue', () => {
    setup('#ff0000');
    oklchTab();
    expect([num('Lightness'), num('Chroma'), num('Hue')]).toEqual([63, 26, 29]);
  });

  it('dragging lightness to black and back keeps the hue and chroma', () => {
    const f = setup('#3366cc');
    oklchTab();
    const [h, c] = [num('Hue'), num('Chroma')];
    for (const l of ['10', '1', '0', '1', '30', '50']) type('Lightness slider', l);
    expect([num('Hue'), num('Chroma')]).toEqual([h, c]);
    expect(last(f).h).toBeCloseTo(toOklch(parseColour('#3366cc')!).h, 0);
    // Through white, too.
    for (const l of ['90', '100', '70', '50']) type('Lightness slider', l);
    expect([num('Hue'), num('Chroma')]).toEqual([h, c]);
  });

  it('a lightness change never moves the hue, even where the gamut takes chroma', () => {
    const f = setup('#ff0000');
    oklchTab();
    for (const l of ['20', '40', '60', '80']) {
      type('Lightness', l);
      expect(last(f).h, `L ${l}`).toBeCloseTo(29.2, 0);
      expect(last(f).l, `L ${l}`).toBeCloseTo(Number(l), 0);
    }
  });

  it('chroma beyond the gamut is emitted at the gamut edge, and held: lowering lightness brings it back', () => {
    const f = setup('#808080');
    oklchTab();
    type('Hue', '264');
    type('Chroma', '40');
    expect(num('Chroma')).toBe(40);
    const edge = last(f).c;
    expect(edge).toBeLessThan(0.3);
    expect(last(f).h).toBeCloseTo(264, 0);
    type('Lightness', '45');
    expect(last(f).c).toBeGreaterThan(edge);
  });

  it('a grey keeps the hue and chroma it was given', () => {
    setup('#3366cc');
    oklchTab();
    type('Hue', '120');
    type('Chroma', '0');
    type('Chroma', '10');
    expect(num('Hue')).toBe(120);
  });

  it('the square: arrows move chroma across and lightness up and down, Shift by 10', () => {
    setup('#808080');
    const sq = square();
    const l0 = Number(sq.getAttribute('aria-valuenow'));
    fireEvent.keyDown(sq, { key: 'ArrowUp' });
    expect(sq).toHaveAttribute('aria-valuenow', String(l0 + 1));
    fireEvent.keyDown(sq, { key: 'ArrowDown', shiftKey: true });
    expect(sq).toHaveAttribute('aria-valuenow', String(l0 - 9));
    fireEvent.keyDown(sq, { key: 'ArrowRight', shiftKey: true });
    fireEvent.keyDown(sq, { key: 'ArrowRight' });
    expect(sq).toHaveAttribute('aria-valuetext', `Lightness ${l0 - 9}, chroma 11`);
    fireEvent.keyDown(sq, { key: 'ArrowLeft' });
    expect(sq).toHaveAttribute('aria-valuetext', `Lightness ${l0 - 9}, chroma 10`);
  });

  it('the hue and opacity strips step by the arrows', () => {
    const f = setup('rgba(255, 0, 0, 0.5)', true);
    const hue = screen.getByRole('slider', { name: 'Hue' });
    fireEvent.keyDown(hue, { key: 'ArrowRight' });
    expect(hue).toHaveAttribute('aria-valuenow', '30');
    fireEvent.keyDown(hue, { key: 'ArrowDown', shiftKey: true });
    expect(hue).toHaveAttribute('aria-valuenow', '20');
    fireEvent.keyDown(screen.getByRole('slider', { name: 'Opacity' }), { key: 'ArrowLeft' });
    expect(f.onPreview.mock.lastCall![0]).toMatch(/, 0\.49\)$/);
  });

  it('a press on the square picks the chroma and lightness under it', () => {
    const f = setup('#808080');
    const sq = square();
    sq.getBoundingClientRect = () => ({ left: 0, top: 0, width: 200, height: 100, right: 200, bottom: 100, x: 0, y: 0, toJSON: () => ({}) });
    fireEvent.pointerDown(sq, { clientX: 25, clientY: 40, button: 0 });
    expect(sq).toHaveAttribute('aria-valuetext', 'Lightness 60, chroma 5');
    expect(last(f).l).toBeCloseTo(60, 0);
    expect(f.onCommit).not.toHaveBeenCalled();
  });

  it('the wheel over an OKLCH row steps it by 1 a notch; a trackpad adds up to a notch', () => {
    setup('#3366cc');
    oklchTab();
    const start = num('Hue');
    fireEvent.wheel(screen.getByLabelText('Hue slider'), { deltaY: -100 });
    expect(num('Hue')).toBe(start + 1);
    fireEvent.wheel(screen.getByLabelText('Hue'), { deltaY: 100 });
    fireEvent.wheel(screen.getByLabelText('Hue'), { deltaY: 100 });
    expect(num('Hue')).toBe(start - 1);
    fireEvent.wheel(screen.getByLabelText('Hue slider'), { deltaY: -20 });
    expect(num('Hue')).toBe(start - 1);
    fireEvent.wheel(screen.getByLabelText('Hue slider'), { deltaY: -40 });
    expect(num('Hue')).toBe(start);
    // At the very end it stays put; the wheel never scrolls the page under it.
    type('Lightness', '100');
    const notch = new WheelEvent('wheel', { deltaY: -100, bubbles: true, cancelable: true });
    screen.getByLabelText('Lightness slider').dispatchEvent(notch);
    expect(screen.getByLabelText('Lightness')).toHaveValue(100);
    expect(notch.defaultPrevented).toBe(true);
  });

  it("the wheel over the Picker tab's strips steps them by 1, and over the square steps lightness", () => {
    const f = setup('rgba(255, 0, 0, 0.5)', true);
    const hue = screen.getByRole('slider', { name: 'Hue' });
    fireEvent.wheel(hue, { deltaY: -100 });
    expect(hue).toHaveAttribute('aria-valuenow', '30');
    fireEvent.wheel(screen.getByRole('slider', { name: 'Opacity' }), { deltaY: 100 });
    expect(f.onPreview.mock.lastCall![0]).toMatch(/, 0\.49\)$/);
    const l0 = Number(square().getAttribute('aria-valuenow'));
    const notch = new WheelEvent('wheel', { deltaY: -100, bubbles: true, cancelable: true });
    fireEvent(square(), notch);
    expect(notch.defaultPrevented).toBe(true);
    expect(square()).toHaveAttribute('aria-valuenow', String(l0 + 1));
    // Back on the Picker tab after the OKLCH one, the wheel still steps it.
    oklchTab();
    fireEvent.click(screen.getByRole('tab', { name: 'Picker' }));
    fireEvent.wheel(square(), { deltaY: -100 });
    expect(square()).toHaveAttribute('aria-valuenow', String(l0 + 2));
  });

  it('an invalid hex marks the field and changes nothing', () => {
    const f = setup();
    type('Hex', 'zz');
    expect(screen.getByLabelText('Hex')).toHaveAttribute('aria-invalid', 'true');
    expect(f.onPreview).not.toHaveBeenCalled();
  });

  it('Old restores the original colour', () => {
    const f = setup();
    type('Hex', '#ff0000');
    fireEvent.click(screen.getByRole('button', { name: 'Old' }));
    expect(f.onPreview).toHaveBeenLastCalledWith('#4d88ff');
  });
});
