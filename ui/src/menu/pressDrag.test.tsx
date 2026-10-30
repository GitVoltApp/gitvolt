import { act, fireEvent, render, screen } from '@testing-library/react';
import { Copy, Trash2 } from 'lucide-react';
import { useRef, useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ArmLayer } from '../ui/arm/ArmLayer';
import { armClock } from '../ui/arm/armTesting';
import { disarm, useArm } from '../ui/arm/store';
import { confirmAction } from '../ui/ConfirmDialog';
import { ContextMenu } from './ContextMenu';
import { openContextMenu, openMenuAt, useMenu } from './menuStore';
import { CONTEXT_GUARD_MS, pressOpener, usePressDrag } from './pressDrag';
import { mouseDown as down, mouseRelease, mouseUp as up } from './pressTesting';
import type { MenuRow } from './types';

const action = (id: string, run: () => void = () => {}, extra: Partial<Extract<MenuRow, { kind: 'action' }>> = {}): MenuRow => ({ kind: 'action', id, label: id.toUpperCase(), icon: Copy, tooltip: `tip ${id}`, run, ...extra });

function Opener({ rows }: { rows: () => MenuRow[] }) {
  return <button type="button" {...pressOpener((el) => openMenuAt(el, rows(), undefined, rows, 'Things'))}>open</button>;
}

const isOpen = () => useMenu.getState().rows !== null;
const row = (label: string) => screen.getByText(label).closest('[role="menuitem"]')!;

describe('press-drag-release on the context menu (an opener’s press)', () => {
  const pick = vi.fn();
  const off = vi.fn();
  const rows = () => [action('one', pick), { kind: 'separator' } as MenuRow, action('two', off, { disabledReason: 'Not now' }), { kind: 'submenu', id: 'more', label: 'More', icon: Copy, tooltip: 'more', rows: [action('deep', pick)] } as MenuRow];
  beforeEach(() => {
    pick.mockClear();
    off.mockClear();
    render(<><Opener rows={rows} /><ContextMenu /><p>outside</p></>);
  });
  afterEach(() => act(() => useMenu.getState().close()));
  const opener = () => screen.getByRole('button', { name: 'open' });

  it('opens on the press, not on its click', () => {
    down(opener());
    expect(isOpen()).toBe(true);
    expect(opener()).toHaveAttribute('aria-expanded', 'true');
  });

  it('released on the button (a click), the menu stays open and nothing runs; a second click closes it (the toggle)', () => {
    down(opener());
    up(opener(), 11, 10, opener());
    expect(isOpen()).toBe(true);
    expect(pick).not.toHaveBeenCalled();
    // The second click: its press closes the menu and its click doesn't reopen it.
    down(opener());
    up(opener(), 10, 10, opener());
    expect(isOpen()).toBe(false);
    expect(opener()).toHaveAttribute('aria-expanded', 'false');
  });

  it('dragged onto an enabled row and released, it picks that row and closes', () => {
    down(opener());
    fireEvent.pointerEnter(row('ONE'));
    expect(row('ONE')).toHaveAttribute('data-active', 'true');
    up(screen.getByText('ONE'), 40, 60, document.body);
    expect(pick).toHaveBeenCalledOnce();
    expect(isOpen()).toBe(false);
  });

  it('released on a disabled row, a separator, the padding or outside, nothing is picked and the menu stays', () => {
    const targets = [() => screen.getByText('TWO'), () => document.querySelector('.ctx-sep')!, () => screen.getByTestId('context-menu'), () => screen.getByText('outside')];
    for (const target of targets) {
      down(opener());
      up(target(), 40, 80, document.body);
      expect(isOpen()).toBe(true);
      act(() => useMenu.getState().close());
    }
    expect(pick).not.toHaveBeenCalled();
    expect(off).not.toHaveBeenCalled();
  });

  it('a release within 4px of the press is a click: it picks nothing, even over a row', () => {
    down(opener(), 40, 60);
    up(screen.getByText('ONE'), 42, 62, document.body);
    expect(pick).not.toHaveBeenCalled();
    expect(isOpen()).toBe(true);
  });

  it('released on a submenu row, it opens the submenu and picks nothing', () => {
    down(opener());
    up(screen.getByText('More'), 40, 90, document.body);
    expect(isOpen()).toBe(true);
    expect(screen.getByText('DEEP')).toBeInTheDocument();
    expect(pick).not.toHaveBeenCalled();
  });

  it('touch and pen open on their click, as before', () => {
    fireEvent.pointerDown(opener(), { pointerType: 'touch', pointerId: 2, button: 0 });
    expect(isOpen()).toBe(false);
    fireEvent.click(opener(), { detail: 1 });
    expect(isOpen()).toBe(true);
  });

  it('the keyboard opens it with the button’s click (Enter, Space)', () => {
    fireEvent.click(opener(), { detail: 0 });
    expect(isOpen()).toBe(true);
  });
});

describe('press-drag-release on a right press’s context menu', () => {
  let clock: ReturnType<typeof armClock>;
  const pick = vi.fn();
  beforeEach(() => {
    clock = armClock();
    pick.mockClear();
    render(<><div data-testid="target" onContextMenu={(e) => openContextMenu(e, () => [action('one', pick)])}>target</div><ContextMenu /></>);
  });
  afterEach(() => { act(() => useMenu.getState().close()); clock.restore(); });
  const rightPress = (x = 10, y = 10) => {
    const t = screen.getByTestId('target');
    down(t, x, y, 2);
    fireEvent.contextMenu(t, { clientX: x, clientY: y, button: 2, buttons: 2 });
  };

  it('a release under the pointer at once picks nothing: the menu opened under it', () => {
    rightPress();
    up(screen.getByText('ONE'), 12, 11, null, 2);
    expect(pick).not.toHaveBeenCalled();
    expect(isOpen()).toBe(true);
  });

  it('after the pointer moved 4px, or after the guard time, a release on a row picks it', () => {
    rightPress();
    up(screen.getByText('ONE'), 20, 30, null, 2);
    expect(pick).toHaveBeenCalledOnce();
    rightPress();
    clock.advance(CONTEXT_GUARD_MS);
    up(screen.getByText('ONE'), 11, 11, null, 2);
    expect(pick).toHaveBeenCalledTimes(2);
  });

  it('a right-click on Windows (its menu opens after the release) has no drag to take', () => {
    const t = screen.getByTestId('target');
    down(t, 10, 10, 2);
    mouseRelease(t, 10, 10, 2);
    fireEvent.contextMenu(t, { clientX: 10, clientY: 10, button: 2 });
    clock.advance(CONTEXT_GUARD_MS);
    mouseRelease(screen.getByText('ONE'), 30, 30, 2);
    expect(pick).not.toHaveBeenCalled();
  });
});

describe('press-drag-release on an arm-in-place row', () => {
  let clock: ReturnType<typeof armClock>;
  beforeEach(() => { clock = armClock(); });
  afterEach(() => { act(() => { disarm(); useMenu.getState().close(); }); clock.restore(); });

  it('released on it, the row arms; it runs only on a later click', async () => {
    const ran = vi.fn();
    const rows = () => [action('delete', () => { void confirmAction({ title: 'Delete?', body: 'b', confirmLabel: 'Delete', arm: 'Click again to delete', danger: true }).then((ok) => { if (ok) ran(); }); }, { icon: Trash2 })];
    render(<><Opener rows={rows} /><ContextMenu /><ArmLayer /></>);
    down(screen.getByRole('button', { name: 'open' }));
    up(screen.getByText('DELETE'), 40, 60, document.body);
    expect(useArm.getState().armed).not.toBeNull();
    expect(screen.getByRole('menuitem', { name: 'Click again to delete' })).toHaveAttribute('data-armed', 'danger');
    await Promise.resolve();
    expect(ran).not.toHaveBeenCalled();
    expect(isOpen()).toBe(true);
  });
});

/** A popup of its own (not the context menu) through the shared hook. */
function Popup({ onPick }: { onPick(v: string): void }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  usePressDrag(ref, open);
  return (
    <>
      <button type="button" {...pressOpener(() => setOpen((o) => !o))}>pop</button>
      {open && (
        <div ref={ref} role="listbox" data-testid="pop">
          <div role="option" aria-selected={false} onClick={() => { onPick('a'); setOpen(false); }}>A</div>
          <div role="option" aria-selected={false} aria-disabled="true" onClick={() => onPick('b')}>B</div>
        </div>
      )}
    </>
  );
}

describe('usePressDrag (a popup of its own)', () => {
  it('picks the option released on; a disabled one or the button picks nothing', () => {
    const onPick = vi.fn();
    render(<Popup onPick={onPick} />);
    const btn = screen.getByRole('button', { name: 'pop' });
    down(btn);
    up(screen.getByText('B'), 30, 50, document.body);
    expect(onPick).not.toHaveBeenCalled();
    expect(screen.getByTestId('pop')).toBeInTheDocument();
    // A press on the button again closes it (the toggle).
    down(btn);
    up(btn, 10, 10, btn);
    expect(screen.queryByTestId('pop')).toBeNull();
    down(btn);
    up(screen.getByText('A'), 30, 40, document.body);
    expect(onPick).toHaveBeenCalledExactlyOnceWith('a');
    expect(screen.queryByTestId('pop')).toBeNull();
  });
});
