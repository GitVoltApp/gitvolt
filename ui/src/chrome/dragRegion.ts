import { windowOps, type WindowOps } from './windowChrome';

/** What a press inside a drag region leaves alone: the controls, and anything marked. */
const CONTROLS = [
  'button', 'a', 'input', 'select', 'textarea', 'summary', 'label',
  '[role="button"]', '[role="tab"]', '[role="menuitem"]', '[role="link"]', '[role="switch"]',
  '[contenteditable]:not([contenteditable="false"])', '[tabindex]:not([tabindex="-1"])', '[data-no-window-drag]',
].join(', ');

/** Whether a press on `target` is on `region`'s empty space (itself, or text and decoration in
 * it), not on a control inside it. */
export function isDragTarget(target: EventTarget | null, region: Element): boolean {
  if (!(target instanceof Element) || !region.contains(target)) return false;
  for (let el: Element | null = target; el && el !== region; el = el.parentElement) {
    if (el.matches(CONTROLS)) return false;
  }
  return true;
}

/** How far the pointer goes with the button down before the window moves: a click stays a click
 * (and the second press of a double click doesn't start a move). */
export const DRAG_THRESHOLD = 4;

interface Press { button: number; clientX: number; clientY: number }

/**
 * A left press on a drag region: once the pointer has moved [`DRAG_THRESHOLD`] px with the button
 * still down, the window manager takes over and moves the window (edge tiling and snapping
 * included). The listeners go as soon as that happens or the button is released.
 */
export function beginWindowDrag(press: Press, ops: WindowOps = windowOps, target: Pick<Document, 'addEventListener' | 'removeEventListener'> = document): void {
  if (press.button !== 0) return;
  const move = (e: MouseEvent) => {
    if ((e.buttons & 1) === 0) return stop();
    if (Math.hypot(e.clientX - press.clientX, e.clientY - press.clientY) < DRAG_THRESHOLD) return;
    stop();
    void ops.startDragging().catch((err: unknown) => console.warn('[gitvolt] window move refused', err));
  };
  const stop = () => {
    target.removeEventListener('mousemove', move, true);
    target.removeEventListener('mouseup', stop, true);
  };
  target.addEventListener('mousemove', move, true);
  target.addEventListener('mouseup', stop, true);
}
