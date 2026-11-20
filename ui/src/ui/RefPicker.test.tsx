import { Clock } from 'lucide-react';
import { fireEvent, render, screen } from '@testing-library/react';
import { Profiler } from 'react';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it, vi } from 'vitest';
import { registerKeys } from './keyRouter';
import { RefPicker, type PickItem } from './RefPicker';

const ITEMS: PickItem[] = [
  { id: 'main', label: 'main', current: true }, { id: 'feature/login', label: 'feature/login' }, { id: 'hotfix', label: 'hotfix' },
];
const picker = (over: Partial<Parameters<typeof RefPicker>[0]> = {}) =>
  render(<RefPicker anchor={new DOMRect(0, 0, 100, 20)} placeholder="Find a branch" items={ITEMS} onPick={vi.fn()} onClose={vi.fn()} {...over} />);
const input = () => screen.getByPlaceholderText('Find a branch');
const selected = () => screen.getAllByRole('option').filter((o) => o.getAttribute('aria-selected') === 'true').map((o) => o.textContent);

describe('RefPicker toggle', () => {
  it('shows the toggle right of the search box; clicking it keeps the focus in the box', () => {
    const onToggle = vi.fn();
    picker({ toggle: { icon: Clock, label: 'Sorted A–Z: switch to most recent first', onToggle } });
    const btn = screen.getByRole('button', { name: 'Sorted A–Z: switch to most recent first' });
    expect(btn.closest('.picker-head')).toBe(input().closest('.picker-head'));
    fireEvent.pointerDown(btn);
    fireEvent.click(btn);
    expect(onToggle).toHaveBeenCalledOnce();
    expect(input()).toHaveFocus();
  });

  it('after a re-sort the cursor follows the current item, not the old index', () => {
    const { rerender } = picker();
    expect(selected()).toEqual(['main']);
    const resorted: PickItem[] = [ITEMS[2], ITEMS[1], ITEMS[0]];
    rerender(<RefPicker anchor={new DOMRect(0, 0, 100, 20)} placeholder="Find a branch" items={resorted} onPick={vi.fn()} onClose={vi.fn()} />);
    expect(selected()).toEqual(['main']);
  });
});

describe('RefPicker', () => {
  it('filters by substring and picks with the keyboard', () => {
    const onPick = vi.fn();
    picker({ onPick });
    fireEvent.change(input(), { target: { value: 'LOG' } });
    expect(screen.getAllByRole('option').map((o) => o.textContent)).toEqual(['feature/login']);
    fireEvent.keyDown(input(), { key: 'Enter' });
    expect(onPick).toHaveBeenCalledWith(expect.objectContaining({ id: 'feature/login' }));
  });

  it('starts on the current item; ↑/↓ move, clamped at both ends', () => {
    picker();
    expect(input()).toHaveFocus();
    expect(selected()).toEqual(['main']);
    fireEvent.keyDown(input(), { key: 'ArrowDown' });
    fireEvent.keyDown(input(), { key: 'ArrowDown' });
    fireEvent.keyDown(input(), { key: 'ArrowDown' });
    expect(selected()).toEqual(['hotfix']);
    fireEvent.keyDown(input(), { key: 'ArrowUp' });
    expect(selected()).toEqual(['feature/login']);
    expect(input()).toHaveAttribute('aria-activedescendant', screen.getAllByRole('option')[1].id);
  });

  it('starts on the current item wherever it is in the list', () => {
    picker({ items: [ITEMS[1], ITEMS[2], ITEMS[0]] });
    expect(selected()).toEqual(['main']);
  });

  it('says so when nothing matches, and Enter then picks nothing', () => {
    const onPick = vi.fn();
    picker({ onPick });
    fireEvent.change(input(), { target: { value: 'zzz' } });
    expect(screen.getByText('No matches')).toBeInTheDocument();
    fireEvent.keyDown(input(), { key: 'Enter' });
    expect(onPick).not.toHaveBeenCalled();
  });

  it('owns the keyboard in the key router\'s menu layer: Esc closes it, and never reaches the app', () => {
    const onClose = vi.fn();
    const app = vi.fn(() => 'handled' as const);
    const off = registerKeys('app', app);
    picker({ onClose });
    fireEvent.keyDown(input(), { key: 'w', ctrlKey: true });
    fireEvent.keyDown(input(), { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(app).not.toHaveBeenCalled();
    off();
  });

  it('a click picks; a press outside closes, but not one on the element that opened it', () => {
    const onPick = vi.fn();
    const onClose = vi.fn();
    const opener = document.createElement('button');
    document.body.append(opener);
    picker({ onPick, onClose, ignore: opener });
    fireEvent.pointerDown(opener);
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.pointerDown(document.body, { clientX: 500, clientY: 500 });
    expect(onClose).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByText('hotfix'));
    expect(onPick).toHaveBeenCalledWith(expect.objectContaining({ id: 'hotfix' }));
    opener.remove();
  });

  it('paints the current item active on the first commit, with no intermediate state', () => {
    const seen: (string | null)[][] = [];
    const scrolled: string[] = [];
    Element.prototype.scrollIntoView = function (this: Element) { scrolled.push(this.textContent ?? ''); };
    render(
      <Profiler id="p" onRender={() => seen.push(screen.getAllByRole('option').filter((o) => o.getAttribute('aria-selected') === 'true').map((o) => o.textContent))}>
        <RefPicker anchor={new DOMRect(0, 0, 100, 20)} placeholder="Find a branch" items={[ITEMS[1], ITEMS[2], ITEMS[0]]} onPick={vi.fn()} onClose={vi.fn()} />
      </Profiler>,
    );
    expect(seen).toEqual([['main']]);
    expect(scrolled).toEqual(['main']);
    delete (Element.prototype as { scrollIntoView?: unknown }).scrollIntoView;
  });

  it('filtering re-aims the active row in the same commit as the new list', () => {
    picker();
    fireEvent.change(input(), { target: { value: 'i' } });
    expect(selected()).toEqual(['main']);
    fireEvent.change(input(), { target: { value: 'ho' } });
    expect(selected()).toEqual(['hotfix']);
  });

  it('has no background transition on the rows', () => {
    const css = readFileSync(join(dirname(fileURLToPath(import.meta.url)), 'picker.css'), 'utf8');
    expect(css.match(/\.picker-item \{[^}]*\}/)?.[0]).not.toMatch(/transition/);
  });
});

describe('RefPicker placement', () => {
  it('opens under its anchor, kept inside the window when the anchor is low', () => {
    const { unmount } = picker({ anchor: new DOMRect(10, 100, 100, 20) });
    expect((document.querySelector('.picker') as HTMLElement).style.top).toBe('124px');
    unmount();
    picker({ anchor: new DOMRect(10, window.innerHeight + 200, 100, 20) });
    expect(parseFloat((document.querySelector('.picker') as HTMLElement).style.top)).toBeLessThanOrEqual(window.innerHeight - 4);
  });
});

describe('RefPicker: pressing its trigger again toggles it closed', () => {
  it('a press on the anchor closes it once, and the click that follows does not reach the trigger (no reopen)', () => {
    const onClose = vi.fn();
    const reopen = vi.fn();
    render(<button type="button" onClick={reopen}>trigger</button>);
    picker({ anchor: new DOMRect(10, 10, 100, 20), onClose });
    const trigger = screen.getByRole('button', { name: 'trigger' });
    fireEvent.pointerDown(trigger, { clientX: 20, clientY: 15 });
    // The real order: the popup's search box blurs towards the trigger, then the click.
    fireEvent.blur(input(), { relatedTarget: trigger });
    fireEvent.pointerUp(trigger, { clientX: 20, clientY: 15 });
    fireEvent.click(trigger, { clientX: 20, clientY: 15 });
    expect(onClose).toHaveBeenCalledOnce();
    expect(reopen).not.toHaveBeenCalled();
  });

  it('a press elsewhere closes it and the click is not swallowed', () => {
    const onClose = vi.fn();
    const other = vi.fn();
    render(<button type="button" onClick={other}>other</button>);
    picker({ anchor: new DOMRect(10, 10, 100, 20), onClose });
    const b = screen.getByRole('button', { name: 'other' });
    fireEvent.pointerDown(b, { clientX: 400, clientY: 400 });
    fireEvent.pointerUp(b, { clientX: 400, clientY: 400 });
    fireEvent.click(b, { clientX: 400, clientY: 400 });
    expect(onClose).toHaveBeenCalledOnce();
    expect(other).toHaveBeenCalledOnce();
  });
});

describe('RefPicker with many items', () => {
  const many = (n: number, current: number, extra: Partial<PickItem> = {}): PickItem[] =>
    Array.from({ length: n }, (_, i) => ({ id: `b${i}`, label: `branch-${String(i).padStart(4, '0')}`, current: i === current, ...extra }));
  const options = () => screen.getAllByRole('option');
  const active = () => input().getAttribute('aria-activedescendant');

  it('renders only the rows around the view, the current one among them on the first commit', () => {
    picker({ items: many(1250, 1000) });
    expect(options().length).toBeLessThan(80);
    expect(selected()).toEqual(['branch-1000']);
    expect(document.getElementById(active()!)?.textContent).toBe('branch-1000');
  });

  it('↑/↓ stop at both ends of the whole list, the active row always rendered', () => {
    const { unmount } = picker({ items: many(1250, 1249) });
    expect(selected()).toEqual(['branch-1249']);
    fireEvent.keyDown(input(), { key: 'ArrowDown' });
    expect(selected()).toEqual(['branch-1249']);
    fireEvent.keyDown(input(), { key: 'ArrowUp' });
    expect(selected()).toEqual(['branch-1248']);
    expect(document.getElementById(active()!)?.textContent).toBe('branch-1248');
    unmount();
    picker({ items: many(1250, 0) });
    fireEvent.keyDown(input(), { key: 'ArrowUp' });
    expect(selected()).toEqual(['branch-0000']);
    fireEvent.keyDown(input(), { key: 'ArrowDown' });
    expect(selected()).toEqual(['branch-0001']);
  });

  it('the filter searches every item, not only the rendered ones; Enter picks the match', () => {
    const onPick = vi.fn();
    picker({ items: many(1250, 0), onPick });
    fireEvent.change(input(), { target: { value: 'BRANCH-1234' } });
    expect(options().map((o) => o.textContent)).toEqual(['branch-1234']);
    fireEvent.keyDown(input(), { key: 'Enter' });
    expect(onPick).toHaveBeenCalledWith(expect.objectContaining({ id: 'b1234' }));
  });

  it('moving the active row re-renders the two rows it leaves and enters, not the list', () => {
    let renders = 0;
    const Counted = (() => { renders++; return null; }) as unknown as PickItem['icon'];
    picker({ items: many(1250, 0, { icon: Counted }) });
    renders = 0;
    fireEvent.pointerEnter(options()[3]);
    expect(selected()).toEqual(['branch-0003']);
    expect(renders).toBe(2);
    renders = 0;
    fireEvent.keyDown(input(), { key: 'ArrowDown' });
    expect(renders).toBe(2);
  });

  it('one tooltip for the list, each row\'s worked out only when it shows', () => {
    const tips = Array.from({ length: 1250 }, (_, i) => vi.fn(() => `Tip ${i}`));
    picker({ items: many(1250, 0).map((it, i) => ({ ...it, tooltip: tips[i] })) });
    expect(tips.some((t) => t.mock.calls.length)).toBe(false);
    fireEvent.mouseEnter(options()[2]);
    expect(screen.getByRole('tooltip').textContent).toBe('Tip 2');
    fireEvent.mouseLeave(options()[2]);
    fireEvent.mouseEnter(options()[5]);
    expect(screen.getAllByRole('tooltip').map((t) => t.textContent)).toEqual(['Tip 5']);
    expect(tips.filter((t) => t.mock.calls.length).length).toBe(2);
    // Filtering drops it: its row may be gone.
    fireEvent.change(input(), { target: { value: '12' } });
    expect(screen.queryByRole('tooltip')).toBeNull();
  });
});
