import { useEffect, useLayoutEffect, useRef, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent, type RefObject } from 'react';
import { swallowGestureClick } from '../ui/swallowClick';

/**
 * Press-drag-release, as on a native menu: press an opener (or right-press for a context menu),
 * drag onto an item, release to pick it. Every menu and popup list opened by a press shares it:
 * the context menu (`ContextMenu`) and the popups that aren't it (`usePressDrag`) watch the press
 * that opened them; openers open on the press (`pressOpener`), a split button's main part on a
 * hold (`useLongPress`).
 *
 * The release of the press that opened the popup:
 * - on an enabled item (a row, an option, a button in the popup) clicks it, as a click there would:
 *   a submenu row opens its submenu, an arm-in-place row arms;
 * - anywhere else (the opener, a disabled row, a separator, the padding, outside) does nothing: the
 *   popup stays open, as after a click on the opener, for Esc or a press outside to close;
 * - within PRESS_SLOP_PX of the press point is a click on the opener, not a pick. A right press's
 *   menu opens under the pointer, so its release picks only once the pointer moved that far or
 *   CONTEXT_GUARD_MS passed since it opened.
 *
 * Mouse only: touch and pen open on their click and pick with a tap, as before.
 */

/** A release this near the press point is a click, not a drag onto an item. */
export const PRESS_SLOP_PX = 4;
/** A right press's menu: how long it must have been open before a release that didn't move picks. */
export const CONTEXT_GUARD_MS = 150;
/** How long a split button's main part is held before its ▾ menu opens. */
export const LONG_PRESS_MS = 400;

const ITEM = '[role="menuitem"], [role="menuitemradio"], [role="menuitemcheckbox"], [role="option"], [role="radio"], button';
const isOff = (el: Element) => el.matches('[aria-disabled="true"], :disabled');

interface Press { id: number; x: number; y: number; button: number }
/** The mouse press in progress, if any. */
let held: Press | null = null;
/** The opener whose press is opening its popup now: its mousedown mustn't move focus back to it. */
let opening: Element | null = null;
/** The click a release dispatches on its item: the swallows below let it through. */
let picking: Event | null = null;

if (typeof window !== 'undefined') {
  window.addEventListener('pointerdown', (e) => {
    held = e.pointerType === 'mouse' ? { id: e.pointerId, x: e.clientX, y: e.clientY, button: e.button } : null;
  }, true);
  const up = (e: PointerEvent) => {
    opening = null;
    if (held?.id === e.pointerId) held = null;
  };
  window.addEventListener('pointerup', up, true);
  window.addEventListener('pointercancel', up, true);
}

/** Eats the native click that ends the current press: the press already opened the popup (or a
 * hold did), so its click on the opener mustn't reopen or toggle it, nor act on whatever holds both
 * the opener and the release point. A pick's own click goes through. */
export function swallowPressClick(): void {
  swallowGestureClick((e) => e !== picking);
}

/**
 * Watches the mouse press that is opening a popup now (none held: a no-op) and picks on its
 * release, per the rules above. `root` is the popup's element; call it as the popup opens and the
 * returned end as it closes.
 */
export function beginPressDrag(root: () => Element | null): () => void {
  const p = held;
  if (!p) return () => {};
  const opened = performance.now();
  const up = (e: PointerEvent) => {
    if (e.pointerId !== p.id) return;
    end();
    const far = Math.hypot(e.clientX - p.x, e.clientY - p.y) >= PRESS_SLOP_PX;
    if (!far && !(p.button === 2 && performance.now() - opened >= CONTEXT_GUARD_MS)) return;
    // What's under the pointer, not the event's target: an editor that captured the press (Monaco)
    // would otherwise get the release.
    const under = document.elementFromPoint?.(e.clientX, e.clientY) ?? (e.target instanceof Element ? e.target : null);
    const box = root();
    const item = under?.closest(ITEM);
    if (!under || !box || !item || !box.contains(item) || isOff(item)) return;
    // The native click that follows (left button) lands on what holds the opener and the item.
    const eat = (c: MouseEvent) => { if (c !== picking) { c.stopPropagation(); c.preventDefault(); } };
    window.addEventListener('click', eat, true);
    setTimeout(() => window.removeEventListener('click', eat, true), 0);
    const click = new MouseEvent('click', { bubbles: true, cancelable: true, composed: true, view: window, detail: 1, clientX: e.clientX, clientY: e.clientY, screenX: e.screenX, screenY: e.screenY });
    picking = click;
    try {
      under.dispatchEvent(click);
    } finally {
      picking = null;
    }
  };
  const end = () => {
    window.removeEventListener('pointerup', up, true);
    window.removeEventListener('pointercancel', end, true);
  };
  window.addEventListener('pointerup', up, true);
  window.addEventListener('pointercancel', end, true);
  return end;
}

/** `beginPressDrag` for a popup component, from its mount (or while `active`) to its unmount. */
export function usePressDrag(ref: RefObject<Element | null>, active = true): void {
  useLayoutEffect(() => (active ? beginPressDrag(() => ref.current) : undefined), [ref, active]);
}

const opensOnPress = (e: { pointerType: string; button: number; ctrlKey: boolean }) => e.pointerType === 'mouse' && e.button === 0 && !e.ctrlKey;

/**
 * Handlers for a button that opens a popup list: a mouse press opens it at once (so a drag onto
 * an item and a release picks it), and the press's click is eaten; the keyboard's click (Enter,
 * Space), touch and pen open it on their click as before. `open` toggles as the button's click
 * did: a press on the button of an open popup closes it (the toggle rule).
 */
export function pressOpener<E extends HTMLElement>(open: (el: E) => void) {
  return {
    onPointerDown(e: ReactPointerEvent<E>) {
      if (!opensOnPress(e)) return;
      const el = e.currentTarget;
      if (isOff(el)) return;
      swallowPressClick();
      opening = el;
      // Focus goes to the button as a click's would, before the popup takes it (its focus is then
      // kept: `onMouseDown` stops the press from moving it back to the button).
      el.focus({ preventScroll: true });
      open(el);
    },
    onMouseDown(e: ReactMouseEvent<E>) {
      if (opening === e.currentTarget) e.preventDefault();
      opening = null;
    },
    onClick(e: ReactMouseEvent<E>) {
      open(e.currentTarget);
    },
  };
}

/**
 * A hold on a split button's main part opens its ▾ menu (`open`; null: it has none): after
 * LONG_PRESS_MS with the button still pressed and the pointer still on it. The menu then takes
 * the release (drag onto an item to pick it), and the press's click doesn't run the button. A
 * shorter press is a click, and leaving the button cancels the hold. One timer per press, cleared
 * on release, cancel or leave.
 */
export function useLongPress<E extends HTMLElement>(open: (() => void) | null) {
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const live = useRef(open);
  live.current = open;
  const cancel = () => {
    clearTimeout(timer.current);
    timer.current = undefined;
  };
  useEffect(() => cancel, []);
  return {
    onPointerDown(e: ReactPointerEvent<E>) {
      cancel();
      if (!live.current || e.button !== 0 || e.ctrlKey) return;
      timer.current = setTimeout(() => {
        timer.current = undefined;
        swallowPressClick();
        live.current?.();
      }, LONG_PRESS_MS);
    },
    onPointerUp: cancel,
    onPointerLeave: cancel,
    onPointerCancel: cancel,
  };
}
