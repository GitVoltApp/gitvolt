// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import { onPointerLeftWindow } from './windowLeave';

describe('onPointerLeftWindow', () => {
  it('fires when the pointer goes off the document or the window blurs, not on a move between elements', () => {
    const cb = vi.fn();
    const off = onPointerLeftWindow(cb);
    const a = document.createElement('div');
    const b = document.createElement('div');
    document.body.append(a, b);
    a.dispatchEvent(new MouseEvent('mouseout', { bubbles: true, relatedTarget: b }));
    expect(cb).not.toHaveBeenCalled();
    a.dispatchEvent(new MouseEvent('mouseout', { bubbles: true, relatedTarget: null }));
    expect(cb).toHaveBeenCalledTimes(1);
    window.dispatchEvent(new Event('blur'));
    expect(cb).toHaveBeenCalledTimes(2);
    off();
    a.dispatchEvent(new MouseEvent('mouseout', { bubbles: true, relatedTarget: null }));
    window.dispatchEvent(new Event('blur'));
    expect(cb).toHaveBeenCalledTimes(2);
    document.body.replaceChildren();
  });
});
