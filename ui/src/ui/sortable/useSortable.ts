import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type HTMLAttributes, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from 'react';
import { DRAG_THRESHOLD, swallowReleaseClick, swallowNextClick } from '../../irebase/chipDrag';
import { groupTarget, slotShifts } from '../../irebase/rowDrag';
import { registerKeys } from '../keyRouter';
import './sortable.css';

export interface SortableOptions {
  count: number;
  axis: 'x' | 'y';
  onMove(from: number, to: number): void;
  label(index: number): string;
}

/** The slot item `from` lands in when dragged `delta` px along the axis. The delta is shrunk
 * toward zero by half the item's size, then the tab/row drag's rule applies (`groupTarget` with
 * a group of one): the item passes a neighbour once it has travelled past that neighbour's middle. */
export function slotFor(sizes: readonly number[], from: number, delta: number): number {
  const shrunk = Math.sign(delta) * Math.max(0, Math.abs(delta) - sizes[from] / 2);
  return groupTarget(sizes, [from], from, shrunk);
}

/** How far each item moves to show the order a release at slot `to` would make. */
export function shiftsFor(sizes: readonly number[], from: number, to: number): number[] {
  return slotShifts(sizes, [from], to);
}

interface Drag { from: number; to: number; d: number; sizes: number[] }

const OWN_PRESS = 'button, input, textarea, select, a, [contenteditable="true"]';

/** Item sizes along the axis, each as the distance to the next item's start (gaps included). */
function measure(parent: HTMLElement, axis: 'x' | 'y', count: number): number[] {
  const rects: DOMRect[] = [];
  for (let i = 0; i < count; i++) {
    const el = parent.querySelector<HTMLElement>(`:scope > [data-sortable-item="${i}"]`);
    if (!el) return [];
    rects.push(el.getBoundingClientRect());
  }
  const start = (r: DOMRect) => (axis === 'y' ? r.top : r.left);
  const size = (r: DOMRect) => (axis === 'y' ? r.height : r.width);
  return rects.map((r, i) => (i < count - 1 ? start(rects[i + 1]) - start(r) : size(r)));
}

/**
 * Reorder a list by dragging an item (or its handle) or with Alt+arrows. Items move with
 * `transform` only; listeners exist only while a drag runs, so an idle list costs nothing.
 * Spread `containerProps` on the list, `itemProps(i)` on each item (direct children of the list),
 * `handleProps(i)` on a handle (optional), and render `liveText` in an `aria-live="polite"` element.
 */
export function useSortable(opts: SortableOptions) {
  const { count, axis, onMove, label } = opts;
  const [drag, setDrag] = useState<Drag | null>(null);
  const [liveText, setLiveText] = useState('');
  const off = useRef<(() => void) | null>(null);
  const focusAt = useRef<number | null>(null);
  const ownerId = useRef(`s${Math.random().toString(36).slice(2, 8)}`).current;
  const latest = useRef({ count, axis, onMove, label });
  latest.current = { count, axis, onMove, label };
  useEffect(() => () => off.current?.(), []);

  // After a keyboard move the item has a new index (maybe a new DOM node): focus follows it.
  useLayoutEffect(() => {
    const to = focusAt.current;
    if (to === null) return;
    focusAt.current = null;
    const el = document.querySelector<HTMLElement>(`[data-sortable-owner="${ownerId}"] > [data-sortable-item="${to}"]`);
    el?.querySelector<HTMLElement>('[data-sortable-handle]')?.focus();
    if (el && el !== document.activeElement && !el.contains(document.activeElement)) el.focus();
  });

  const start = (e: ReactPointerEvent<HTMLElement>, index: number, fromHandle: boolean) => {
    if (e.button !== 0) return;
    if (!fromHandle && (e.target as HTMLElement).closest(OWN_PRESS)) return;
    e.stopPropagation();
    const item = (e.currentTarget as HTMLElement).closest<HTMLElement>('[data-sortable-item]');
    const parent = item?.parentElement;
    if (!parent) return;
    off.current?.();
    const ax = latest.current.axis;
    const x0 = e.clientX;
    const y0 = e.clientY;
    let active = false;
    let sizes: number[] = [];
    let cur: Drag | null = null;
    const stop = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      window.removeEventListener('pointercancel', stop);
      unkey();
      if (off.current === stop) off.current = null;
      setDrag(null);
    };
    const move = (ev: PointerEvent) => {
      if (!active) {
        if (Math.hypot(ev.clientX - x0, ev.clientY - y0) < DRAG_THRESHOLD) return;
        sizes = measure(parent, ax, latest.current.count);
        if (!sizes.length) return;
        active = true;
      }
      const d = ax === 'y' ? ev.clientY - y0 : ev.clientX - x0;
      cur = { from: index, to: slotFor(sizes, index, d), d, sizes };
      setDrag(cur);
    };
    const up = () => {
      const done = active ? cur : null;
      stop();
      if (!active) return;
      swallowNextClick();
      if (done && done.to !== done.from) latest.current.onMove(done.from, done.to);
    };
    // The menu layer, above everything else: Esc puts the item back and nothing else.
    const unkey = registerKeys('menu', (ev) => {
      if (ev.key !== 'Escape' || !active) return;
      ev.preventDefault();
      stop();
      swallowReleaseClick();
      return 'handled';
    });
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    window.addEventListener('pointercancel', stop);
    off.current = stop;
  };

  const onKeyDown = (e: ReactKeyboardEvent<HTMLElement>, index: number) => {
    if (!e.altKey || e.defaultPrevented) return;
    const { axis: ax, count: n, onMove: move, label: name } = latest.current;
    const back = ax === 'y' ? 'ArrowUp' : 'ArrowLeft';
    const fwd = ax === 'y' ? 'ArrowDown' : 'ArrowRight';
    if (e.key !== back && e.key !== fwd) return;
    e.preventDefault();
    const to = e.key === fwd ? index + 1 : index - 1;
    if (to < 0 || to >= n) return;
    const moved = name(index);
    focusAt.current = to;
    move(index, to);
    setLiveText(`${moved} moved to position ${to + 1} of ${n}`);
  };

  const shifts = drag ? shiftsFor(drag.sizes, drag.from, drag.to) : null;
  return {
    containerProps: { 'data-sortable-owner': ownerId } as HTMLAttributes<HTMLElement>,
    itemProps(index: number): HTMLAttributes<HTMLElement> & { style: CSSProperties } {
      const dragged = drag?.from === index;
      const shift = dragged ? drag.d : (shifts?.[index] ?? 0);
      return {
        'data-sortable-item': index,
        'data-sortable-state': drag ? (dragged ? 'dragging' : 'shifting') : undefined,
        onPointerDown: (e: ReactPointerEvent<HTMLElement>) => start(e, index, false),
        onKeyDown: (e: ReactKeyboardEvent<HTMLElement>) => onKeyDown(e, index),
        style: { transform: shift ? `translate${axis.toUpperCase()}(${shift}px)` : undefined },
      } as HTMLAttributes<HTMLElement> & { style: CSSProperties };
    },
    handleProps(index: number): HTMLAttributes<HTMLElement> {
      return {
        'data-sortable-handle': '',
        onPointerDown: (e: ReactPointerEvent<HTMLElement>) => start(e, index, true),
        onKeyDown: (e: ReactKeyboardEvent<HTMLElement>) => onKeyDown(e, index),
      } as HTMLAttributes<HTMLElement>;
    },
    dragging: drag ? drag.from : null,
    liveText,
  };
}
