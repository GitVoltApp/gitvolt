import { create } from 'zustand';
import type { GraphPayload } from '../api/gen/GraphPayload';
import type { RefLabel } from '../api/gen/RefLabel';
import type { RowPayload } from '../api/gen/RowPayload';
import { DRAG_THRESHOLD, swallowNextClick, swallowReleaseClick } from '../irebase/chipDrag';
import { edgeScrollStep } from '../tabs/useTabDrag';
import { registerKeys } from '../ui/keyRouter';

/** A local branch chip being dragged: its ref, the other local branch under the pointer (`over`,
 * null over anything else), the pointer. Refs are full (`refs/heads/x`). */
export interface BranchDrag { from: string; over: string | null; x: number; y: number }

export const useBranchDrag = create<{ drag: BranchDrag | null }>(() => ({ drag: null }));

/** A chip's drag class for its local branch `ref`: the dragged one's, or the one under the
 * pointer's. One selector per chip, so a drag re-renders only the chips whose role changes. */
export const useBranchDragClass = (ref: string | null | undefined): string =>
  useBranchDrag((s) => (!ref || !s.drag ? '' : s.drag.from === ref ? ' is-drag-source' : s.drag.over === ref ? ' is-drop-target' : ''));

/** Ends the branch drag under way (its listeners, its Esc), dropping nothing. */
let cancelCurrent: (() => void) | null = null;
export const cancelBranchDrag = (): void => cancelCurrent?.();

/** The local branch an element is a chip of (`data-drag-branch`). A hover stack's rows are DOM
 * children of the first chip, so a tag's or a remote's row stops the lookup there. */
export const dragBranchOf = (el: Element | null): string | null =>
  el?.closest('[data-drag-branch], .ref-stack-row')?.getAttribute('data-drag-branch') ?? null;

/** A local branch's chip and the row it's on, in the graph's labels. */
export interface DroppedLabel { row: RowPayload; label: RefLabel }

export function labelOf(graph: GraphPayload, ref: string): DroppedLabel | null {
  const label = graph.labels.find((l) => l.local === ref);
  return label ? { row: graph.rows[label.row], label } : null;
}

type Press = { button: number; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean; altKey: boolean; clientX: number; clientY: number };

/**
 * A local branch chip dragged onto another. Pointer events, as the editor's chip drag
 * (`startChipDrag`): CEF never delivers native drag events inside the page. Unlike it, the press
 * is left alone, so it still selects the row, and a press with a modifier stays a selection
 * gesture (K27). Past DRAG_THRESHOLD it's a drag, and a release over another local branch chip
 * drops `from` on it. Esc, a cancel or the window losing focus (a release after switching apps
 * never comes here) end it with nothing dropped. Held near the scroller's top or bottom edge, it
 * scrolls the graph, so a branch far away can be reached.
 */
export function startBranchDrag(e: Press, from: string, scroller: HTMLElement, drop: (from: string, onto: string, x: number, y: number) => void): void {
  if (e.button !== 0 || e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return;
  cancelBranchDrag();
  const x0 = e.clientX;
  const y0 = e.clientY;
  let active = false;
  let ended = false;
  let x = x0;
  let y = y0;
  let frame = 0;
  const target = (x: number, y: number) => {
    const ref = dragBranchOf(document.elementFromPoint?.(x, y) ?? null);
    return ref === from ? null : ref;
  };
  const follow = (px: number, py: number) => {
    x = px;
    y = py;
    useBranchDrag.setState({ drag: { from, over: target(x, y), x, y } });
  };
  const step = () => {
    const r = scroller.getBoundingClientRect();
    return edgeScrollStep(y, r.top, r.bottom);
  };
  // Runs only while the pointer is in an edge zone and the graph still scrolls that way
  // (CONTRIBUTING's animation budget): the next pointer move starts it again.
  const scroll = () => {
    frame = 0;
    const dy = ended ? 0 : step();
    if (!dy) return;
    const before = scroller.scrollTop;
    scroller.scrollTop += dy;
    if (scroller.scrollTop === before) return;
    follow(x, y);
    frame = requestAnimationFrame(scroll);
  };
  const stop = () => {
    ended = true;
    cancelAnimationFrame(frame);
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    window.removeEventListener('pointercancel', stop);
    window.removeEventListener('blur', stop);
    unkey();
    if (cancelCurrent === stop) cancelCurrent = null;
    useBranchDrag.setState({ drag: null });
  };
  const move = (ev: PointerEvent) => {
    if (!active && Math.hypot(ev.clientX - x0, ev.clientY - y0) < DRAG_THRESHOLD) return;
    active = true;
    follow(ev.clientX, ev.clientY);
    if (!frame && step()) frame = requestAnimationFrame(scroll);
  };
  const up = (ev: PointerEvent) => {
    const over = active ? target(ev.clientX, ev.clientY) : null;
    stop();
    if (!active) return;
    swallowNextClick();
    if (over) drop(from, over, ev.clientX, ev.clientY);
  };
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
  window.addEventListener('blur', stop);
  cancelCurrent = stop;
}
