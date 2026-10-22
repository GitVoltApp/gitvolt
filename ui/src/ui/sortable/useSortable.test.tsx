import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useSortable } from './useSortable';

const names = ['a', 'b', 'c'];

function List({ onMove, axis = 'y' }: { onMove: (f: number, t: number) => void; axis?: 'x' | 'y' }) {
  const s = useSortable({ count: names.length, axis, onMove, label: (i) => names[i] });
  return (
    <div>
      <ul {...s.containerProps}>
        {names.map((n, i) => (
          <li key={n} tabIndex={0} data-testid={n} {...s.itemProps(i)}>{n}</li>
        ))}
      </ul>
      <div aria-live="polite" data-testid="live">{s.liveText}</div>
    </div>
  );
}

// 10 px per item along both axes (happy-dom has no layout).
const original = HTMLElement.prototype.getBoundingClientRect;
function mockLayout() {
  HTMLElement.prototype.getBoundingClientRect = function (this: HTMLElement) {
    const i = Number(this.getAttribute('data-sortable-item') ?? 0);
    return { top: i * 10, left: i * 10, height: 10, width: 10, bottom: i * 10 + 10, right: i * 10 + 10, x: 0, y: 0, toJSON() {} } as DOMRect;
  };
}
afterEach(() => { HTMLElement.prototype.getBoundingClientRect = original; });

describe('useSortable', () => {
  it('Alt+ArrowDown moves the item down, says so, and keeps focus on it', () => {
    const onMove = vi.fn();
    render(<List onMove={onMove} />);
    fireEvent.keyDown(screen.getByTestId('a'), { key: 'ArrowDown', altKey: true });
    expect(onMove).toHaveBeenCalledWith(0, 1);
    expect(screen.getByTestId('live')).toHaveTextContent('a moved to position 2 of 3');
    expect(document.activeElement).toBe(screen.getByTestId('b'));
  });

  it('Alt+ArrowUp on the first item and plain arrows do nothing', () => {
    const onMove = vi.fn();
    render(<List onMove={onMove} />);
    fireEvent.keyDown(screen.getByTestId('a'), { key: 'ArrowUp', altKey: true });
    fireEvent.keyDown(screen.getByTestId('a'), { key: 'ArrowDown' });
    expect(onMove).not.toHaveBeenCalled();
  });

  it('on the x axis Alt+ArrowRight moves, Alt+ArrowDown does not', () => {
    const onMove = vi.fn();
    render(<List onMove={onMove} axis="x" />);
    fireEvent.keyDown(screen.getByTestId('b'), { key: 'ArrowDown', altKey: true });
    expect(onMove).not.toHaveBeenCalled();
    fireEvent.keyDown(screen.getByTestId('b'), { key: 'ArrowRight', altKey: true });
    expect(onMove).toHaveBeenCalledWith(1, 2);
  });

  it('a pointer drag of 25 px on 10 px items moves item 0 to slot 2', () => {
    mockLayout();
    const onMove = vi.fn();
    render(<List onMove={onMove} />);
    fireEvent.pointerDown(screen.getByTestId('a'), { button: 0, clientX: 0, clientY: 0 });
    fireEvent.pointerMove(window, { clientX: 0, clientY: 25 });
    fireEvent.pointerUp(window, { clientX: 0, clientY: 25 });
    expect(onMove).toHaveBeenCalledWith(0, 2);
  });

  it('Escape mid-drag cancels it', () => {
    mockLayout();
    const onMove = vi.fn();
    render(<List onMove={onMove} />);
    fireEvent.pointerDown(screen.getByTestId('a'), { button: 0, clientX: 0, clientY: 0 });
    fireEvent.pointerMove(window, { clientX: 0, clientY: 25 });
    fireEvent.keyDown(window, { key: 'Escape' });
    fireEvent.pointerUp(window, { clientX: 0, clientY: 25 });
    expect(onMove).not.toHaveBeenCalled();
  });

  it('a press that travels less than the threshold is a click, not a drag', () => {
    mockLayout();
    const onMove = vi.fn();
    render(<List onMove={onMove} />);
    fireEvent.pointerDown(screen.getByTestId('a'), { button: 0, clientX: 0, clientY: 0 });
    fireEvent.pointerMove(window, { clientX: 0, clientY: 2 });
    fireEvent.pointerUp(window, { clientX: 0, clientY: 2 });
    expect(onMove).not.toHaveBeenCalled();
  });
});
