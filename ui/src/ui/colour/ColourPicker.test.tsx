import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ColourSwatch } from './ColourSwatch';

function setup(value = '#4d88ff', alpha = false) {
  const fns = { onPreview: vi.fn(), onCommit: vi.fn(), onCancel: vi.fn() };
  render(<ColourSwatch value={value} alpha={alpha} label="Accent" {...fns} />);
  fireEvent.click(screen.getByRole('button', { name: 'Accent' }));
  return fns;
}
const type = (label: string, v: string) => fireEvent.change(screen.getByLabelText(label), { target: { value: v } });

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

  it('shows the opacity slider only with alpha, and formats translucent values as rgba', () => {
    setup('#4d88ff', false);
    expect(screen.queryByLabelText('Alpha')).toBeNull();
    fireEvent.click(screen.getByRole('tab', { name: 'HSL' }));
    expect(screen.queryByLabelText('Opacity')).toBeNull();
  });

  it('formats translucent values as rgba through the HSL tab', () => {
    const f = setup('rgba(92, 184, 92, 0.12)', true);
    fireEvent.click(screen.getByRole('tab', { name: 'HSL' }));
    expect(screen.getByLabelText('Opacity')).toHaveValue(12);
    type('Opacity', '50');
    expect(f.onPreview).toHaveBeenLastCalledWith('rgba(92, 184, 92, 0.5)');
  });

  it('dragging lightness to black and back keeps the hue and saturation', () => {
    const f = setup('#3366cc');
    fireEvent.click(screen.getByRole('tab', { name: 'HSL' }));
    const [h, sat] = [screen.getByLabelText('Hue'), screen.getByLabelText('Saturation')].map((e) => (e as HTMLInputElement).value);
    for (const l of ['10', '1', '0', '1', '30', '50']) type('Lightness slider', l);
    expect(screen.getByLabelText('Hue')).toHaveValue(Number(h));
    expect(screen.getByLabelText('Saturation')).toHaveValue(Number(sat));
    expect(f.onPreview).toHaveBeenLastCalledWith('#3366cc');
  });

  it('the wheel over an HSL row steps it by 1 a notch; a trackpad adds up to a notch', () => {
    setup('#3366cc');
    fireEvent.click(screen.getByRole('tab', { name: 'HSL' }));
    const hue = () => Number((screen.getByLabelText('Hue') as HTMLInputElement).value);
    const start = hue();
    fireEvent.wheel(screen.getByLabelText('Hue slider'), { deltaY: -100 });
    expect(hue()).toBe(start + 1);
    fireEvent.wheel(screen.getByLabelText('Hue'), { deltaY: 100 });
    fireEvent.wheel(screen.getByLabelText('Hue'), { deltaY: 100 });
    expect(hue()).toBe(start - 1);
    fireEvent.wheel(screen.getByLabelText('Hue slider'), { deltaY: -20 });
    expect(hue()).toBe(start - 1);
    fireEvent.wheel(screen.getByLabelText('Hue slider'), { deltaY: -40 });
    expect(hue()).toBe(start);
    // At the very end it stays put; the wheel never scrolls the page under it.
    type('Lightness', '100');
    const notch = new WheelEvent('wheel', { deltaY: -100, bubbles: true, cancelable: true });
    screen.getByLabelText('Lightness slider').dispatchEvent(notch);
    expect(screen.getByLabelText('Lightness')).toHaveValue(100);
    expect(notch.defaultPrevented).toBe(true);
  });

  it("the wheel over the Picker tab's hue and opacity strips steps them by 1", () => {
    const f = setup('rgba(255, 0, 0, 0.5)', true);
    fireEvent.wheel(document.querySelector('.react-colorful__hue')!, { deltaY: -100 });
    expect(f.onPreview).toHaveBeenLastCalledWith('rgba(255, 4, 0, 0.5)');
    fireEvent.wheel(document.querySelector('.react-colorful__alpha')!, { deltaY: 100 });
    expect(f.onPreview).toHaveBeenLastCalledWith('rgba(255, 4, 0, 0.49)');
    const square = new WheelEvent('wheel', { deltaY: -100, bubbles: true, cancelable: true });
    document.querySelector('.react-colorful__saturation')!.dispatchEvent(square);
    expect(square.defaultPrevented).toBe(false);
  });

  it('the HSL tab edits through its number fields', () => {
    const f = setup('#ff0000');
    fireEvent.click(screen.getByRole('tab', { name: 'HSL' }));
    type('Lightness', '25');
    expect(f.onPreview).toHaveBeenLastCalledWith('#800000');
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
