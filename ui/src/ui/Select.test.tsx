import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ContextMenu } from '../menu/ContextMenu';
import { useMenu } from '../menu/menuStore';
import { mouseDown, mouseUp } from '../menu/pressTesting';
import { Select } from './Select';

const OPTIONS = [['a', 'Alpha'], ['b', 'Beta'], ['c', 'Gamma']] as const;

describe('Select: press-drag-release', () => {
  afterEach(() => act(() => useMenu.getState().close()));
  const setUp = () => {
    const onChange = vi.fn();
    render(<><Select aria-label="Letter" value="a" options={OPTIONS} onChange={onChange} disabledReason={(v) => (v === 'c' ? 'Not here' : undefined)} /><ContextMenu /></>);
    return { onChange, button: screen.getByRole('button', { name: 'Letter' }) };
  };

  it('opens on the press; the release on an option chooses it', () => {
    const { onChange, button } = setUp();
    mouseDown(button);
    expect(button).toHaveAttribute('aria-expanded', 'true');
    mouseUp(screen.getByText('Beta'), 40, 60, document.body);
    expect(onChange).toHaveBeenCalledExactlyOnceWith('b');
    expect(useMenu.getState().rows).toBeNull();
  });

  it('released on a disabled option, nothing is chosen and the list stays', () => {
    const { onChange, button } = setUp();
    mouseDown(button);
    mouseUp(screen.getByText('Gamma'), 40, 80, document.body);
    expect(onChange).not.toHaveBeenCalled();
    expect(useMenu.getState().rows).not.toBeNull();
  });

  it('a shaky click (under 4px) on the button chooses nothing; a second click closes it', () => {
    const { onChange, button } = setUp();
    mouseDown(button, 10, 10);
    mouseUp(button, 13, 11, button);
    expect(useMenu.getState().rows).not.toBeNull();
    mouseDown(button);
    mouseUp(button, 10, 10, button);
    expect(useMenu.getState().rows).toBeNull();
    expect(button).toHaveAttribute('aria-expanded', 'false');
    expect(onChange).not.toHaveBeenCalled();
  });

  it('the keyboard still opens it (Enter / Space click the button)', () => {
    const { button } = setUp();
    fireEvent.click(button, { detail: 0 });
    expect(useMenu.getState().rows).not.toBeNull();
  });
});
