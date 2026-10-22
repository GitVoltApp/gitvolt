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
