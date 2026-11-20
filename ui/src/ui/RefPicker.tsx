import { useVirtualizer } from '@tanstack/react-virtual';
import type { LucideIcon } from 'lucide-react';
import { memo, useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type MouseEvent } from 'react';
import { createPortal } from 'react-dom';
import { HoverTooltip, useHoverTooltip } from './HoverTooltip';
import { useKeys } from './keyRouter';
import { pointInRect, swallowGestureClick } from './swallowClick';
import { usePressDrag } from '../menu/pressDrag';
import './picker.css';
import { ClearableInput } from './ClearableInput';

/** `tooltip` may be a function: called when the row's tooltip shows, not for every row. */
export interface PickItem { id: string; label: string; detail?: string; icon?: LucideIcon; current?: boolean; tooltip?: string | (() => string) }

interface Props {
  /** The box it opens under (the button's `getBoundingClientRect()`). */
  anchor: DOMRect;
  items: PickItem[];
  placeholder: string;
  onPick(item: PickItem): void;
  onClose(): void;
  /** The element that opened it: a press on it doesn't count as "outside" (so its own click can
   * toggle the picker closed instead of closing and reopening it). */
  ignore?: Element | null;
  /** An optional toggle right of the search box (the branch picker's alphabetical/recent). */
  toggle?: { icon: LucideIcon; label: string; onToggle(): void };
}

const WIDTH = 300;

/** The open pickers' `onClose`s: something that takes the keyboard over (the auth modal) closes
 * them, so only one thing owns the key router's `menu` layer at a time. */
const openPickers = new Set<{ close(): void }>();
export function closePickers(): void {
  for (const p of [...openPickers]) p.close();
}

/**
 * A small searchable list anchored under a button: the toolbar's branch picker, the graph
 * header's pin picker (Task 16). Typing filters by case-insensitive substring; ↑/↓ move, Enter
 * picks, Esc closes; a press outside closes it. Opened by a press, a release on an item picks it
 * (`pressDrag.ts`).
 *
 * While open it owns the keyboard, as a menu does: it's in the key router's `menu` layer, so no key reaches the app behind it. Esc closes the picker and nothing else, never the open
 * file too, and Ctrl+W or F7 do nothing behind it. Every other key goes on to the search box.
 */
export function RefPicker({ anchor, items, placeholder, onPick, onClose, ignore, toggle }: Props) {
  const [q, setQ] = useState('');
  // The current item is active from the very first render (no effect after mount): the
  // cursor starts there, and a filter change re-aims it in the change handler itself.
  const [cursor, setCursor] = useState(() => Math.max(0, items.findIndex((i) => i.current)));
  const ref = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const uid = useId();
  // Opened by a press on its button: dragging onto an item and releasing picks it.
  usePressDrag(ref);
  const filter = useFilter(items);
  const shown = filter(q);
  // Only the rows in view are in the DOM (a picker may list thousands of branches). The first
  // render already holds the rows around the current item, centred as the layout effect below
  // then scrolls them.
  const v = useVirtualizer({
    count: shown.length,
    getScrollElement: () => listRef.current,
    estimateSize: () => ROW_H,
    overscan: 8,
    initialRect: { width: WIDTH, height: LIST_H },
    initialOffset: () => Math.max(0, cursor * ROW_H - (LIST_H - ROW_H) / 2),
  });
  // The hovered row's tooltip: one for the list, not one per row.
  const tip = useRowTooltip(shown);
  // Stable for the memoized rows, whatever the caller passes.
  const pickRef = useRef(onPick);
  pickRef.current = onPick;
  const pick = useCallback((item: PickItem) => pickRef.current(item), []);
  // A re-sort (the toggle) or new items: the cursor follows the current item again. Not on the
  // first render — the cursor's initial state already aims there.
  const lastItems = useRef(items);
  useLayoutEffect(() => {
    if (lastItems.current === items) return;
    lastItems.current = items;
    setCursor(Math.max(0, shown.findIndex((i) => i.current)));
  }, [items, shown]);
  const placed = useRef(false);
  // Before paint, so the first frame already has the active row in view: centred on the first
  // placement, the nearest edge after (arrow keys).
  // A row out of the rendered window (a re-sort or a filter re-aimed the cursor far away) isn't in
  // the DOM yet: the virtualizer scrolls to it.
  useLayoutEffect(() => {
    const row = document.getElementById(`${uid}-${cursor}`);
    if (row) row.scrollIntoView?.({ block: placed.current ? 'nearest' : 'center' });
    else if (cursor < v.options.count) v.scrollToIndex(cursor, { align: placed.current ? 'auto' : 'center' });
    placed.current = true;
  }, [uid, cursor, v]);
  useEffect(() => {
    const entry = { close: onClose };
    openPickers.add(entry);
    return () => { openPickers.delete(entry); };
  }, [onClose]);
  useLayoutEffect(() => {
    const onDown = (e: PointerEvent) => {
      const t = e.target as Node;
      if (ref.current?.contains(t) || ignore?.contains(t)) return;
      // A press on the trigger (the anchor's own box) closes it, and its click must not reopen it.
      if (pointInRect(e, anchor)) swallowGestureClick((c) => pointInRect(c, anchor));
      onClose();
    };
    window.addEventListener('pointerdown', onDown, true);
    return () => window.removeEventListener('pointerdown', onDown, true);
  }, [onClose, ignore, anchor]);

  useKeys('menu', (e) => {
    const plain = !e.ctrlKey && !e.altKey && !e.metaKey && !e.isComposing;
    if (plain && e.key === 'ArrowDown') setCursor((c) => Math.min(shown.length - 1, c + 1));
    else if (plain && e.key === 'ArrowUp') setCursor((c) => Math.max(0, c - 1));
    else if (plain && e.key === 'Enter') { if (shown[cursor]) onPick(shown[cursor]); }
    else if (e.key === 'Escape') onClose();
    else return 'native';
    e.preventDefault();
    return 'handled';
  });

  const left = Math.max(4, Math.min(anchor.left, window.innerWidth - WIDTH - 4));
  // Under the anchor, kept inside the window: an anchor low on the screen (Set upstream opens
  // under a tall branch menu) would otherwise push the list out of view. Measured before paint.
  const [top, setTop] = useState(anchor.bottom + 4);
  useLayoutEffect(() => {
    const h = ref.current?.offsetHeight ?? 0;
    setTop(Math.max(4, Math.min(anchor.bottom + 4, window.innerHeight - h - 4)));
  }, [anchor, shown.length]);
  return createPortal(
    <div ref={ref} className="picker" style={{ left, top }}>
      <div className="picker-head">
      <ClearableInput
        ref={input}
        autoFocus
        className="picker-input"
        placeholder={placeholder}
        aria-label={placeholder}
        role="combobox"
        aria-expanded
        aria-controls={`${uid}-list`}
        aria-activedescendant={shown[cursor] ? `${uid}-${cursor}` : undefined}
        spellCheck={false}
        autoComplete="off"
        value={q}
        onChange={(e) => {
          // The render that follows reuses this filtering (`useFilter`): once per key.
          const next = filter(e.target.value);
          setQ(e.target.value);
          setCursor(Math.max(0, next.findIndex((i) => i.current)));
        }}
        onClear={() => { setQ(''); setCursor(Math.max(0, items.findIndex((i) => i.current))); }}
      />
      {toggle && (
        <HoverTooltip content={toggle.label}>
          <button
            type="button"
            className="icon-button picker-toggle"
            aria-label={toggle.label}
            // The search box keeps the keyboard: the toggle never takes the focus.
            onPointerDown={(e) => e.preventDefault()}
            onClick={() => { toggle.onToggle(); input.current?.focus({ preventScroll: true }); }}
          >
            <toggle.icon size={14} aria-hidden />
          </button>
        </HoverTooltip>
      )}
      </div>
      <div ref={listRef} className="picker-list" role="listbox" id={`${uid}-list`} aria-label={placeholder}>
        {shown.length > 0 && (
          <div className="picker-rows" style={{ height: v.getTotalSize() }}>
            {v.getVirtualItems().map(({ index, start }) => (
              <PickRow key={shown[index].id} item={shown[index]} index={index} id={`${uid}-${index}`} selected={index === cursor} top={start} onEnter={setCursor} onPick={pick} tip={tip.handlers} />
            ))}
          </div>
        )}
        {shown.length === 0 && <div className="picker-empty">No matches</div>}
      </div>
      {tip.tooltip}
    </div>,
    document.body,
  );
}

const ROW_H = 24;
/** The list's height when full (the picker's 360 px less its search box): the virtualizer's
 * first guess, before it measures. */
const LIST_H = 316;

/** `items` narrowed to the labels holding `q` (case-insensitive), the last answer kept: the change
 * handler and the render that follows it ask the same question once. */
function useFilter(items: PickItem[]): (q: string) => PickItem[] {
  const lower = useMemo(() => items.map((i) => i.label.toLowerCase()), [items]);
  const last = useRef<{ lower: string[]; needle: string; out: PickItem[] } | null>(null);
  return (q: string) => {
    const needle = q.trim().toLowerCase();
    if (!needle) return items;
    const l = last.current;
    if (l?.lower === lower && l.needle === needle) return l.out;
    const out = items.filter((_, k) => lower[k].includes(needle));
    last.current = { lower, needle, out };
    return out;
  };
}

type RowTip = {
  enter(e: MouseEvent<HTMLElement>, item: PickItem): void;
  over(e: MouseEvent<HTMLElement>, item: PickItem): void;
  move(e: MouseEvent<HTMLElement>, item: PickItem): void;
  leave(e: MouseEvent<HTMLElement>): void;
};

/** One `HoverTooltip` for every row (the same delay, Esc and placement under the row), its
 * handlers stable so the memoized rows keep them. */
function useRowTooltip(shown: PickItem[]) {
  const hovered = useRef<PickItem | null>(null);
  const { triggerProps, tooltip, hide } = useHoverTooltip({
    content: () => {
      const t = hovered.current?.tooltip;
      return typeof t === 'function' ? t() : t;
    },
  });
  const props = useRef(triggerProps);
  props.current = triggerProps;
  const handlers = useMemo<RowTip>(() => {
    const enter = (e: MouseEvent<HTMLElement>, item: PickItem) => { hovered.current = item; props.current.onMouseEnter(e); };
    return {
      enter,
      // A row that came under the pointer without a mouseenter (the one under it was filtered
      // out): its first mouseover is its enter.
      over: (e, item) => (hovered.current === item ? props.current.onMouseOver(e) : enter(e, item)),
      move: (e, item) => { hovered.current = item; props.current.onMouseMove(e); },
      leave: (e) => { hovered.current = null; props.current.onMouseLeave(e); },
    };
  }, []);
  // A new list (a filter, a re-sort) drops the tooltip, as its row may be gone.
  useEffect(() => { hide(); }, [shown, hide]);
  return { handlers, tooltip };
}

/** A row, re-rendered only when its own props change: moving the cursor redraws two rows. */
const PickRow = memo(function PickRow({ item, index, id, selected, top, onEnter, onPick, tip }: {
  item: PickItem; index: number; id: string; selected: boolean; top: number; onEnter(i: number): void; onPick(item: PickItem): void; tip: RowTip;
}) {
  const Icon = item.icon;
  const has = !!item.tooltip;
  return (
    <div
      id={id}
      role="option"
      aria-selected={selected}
      data-current={item.current || undefined}
      className="picker-item"
      style={{ transform: `translateY(${top}px)` }}
      onPointerEnter={() => onEnter(index)}
      onClick={() => onPick(item)}
      onMouseEnter={has ? (e) => tip.enter(e, item) : undefined}
      onMouseOver={has ? (e) => tip.over(e, item) : undefined}
      onMouseMove={has ? (e) => tip.move(e, item) : undefined}
      onMouseLeave={has ? (e) => tip.leave(e) : undefined}
    >
      {Icon && <Icon size={13} aria-hidden />}
      <span className="picker-label">{item.label}</span>
      {item.detail && <span className="picker-detail">{item.detail}</span>}
    </div>
  );
});
