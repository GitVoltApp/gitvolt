import { useLayoutEffect, useState, type RefObject } from 'react';
import { useZoom } from '../zoom';

const M = 8;
const GAP = 6;

/** A popover's place: under its anchor, left-aligned with it (above when there's no room below),
 * kept on screen; with no anchor, centred near the top. */
export function placePopover(anchor: { left: number; top: number; bottom: number } | null, size: { w: number; h: number }, vp: { w: number; h: number }): { left: number; top: number } {
  if (!anchor) return { left: Math.max(M, Math.round((vp.w - size.w) / 2)), top: 72 };
  const left = Math.max(M, Math.min(anchor.left, vp.w - size.w - M));
  let top = anchor.bottom + GAP;
  if (top + size.h > vp.h - M && anchor.top - GAP - size.h >= M) top = anchor.top - GAP - size.h;
  return { left, top: Math.max(M, Math.min(top, vp.h - size.h - M)) };
}

/** The most height a popover at `anchor` has on screen: the larger room, below it or above. */
export function popoverRoom(anchor: { top: number; bottom: number }, vpHeight: number): number {
  return Math.max(vpHeight - M - anchor.bottom - GAP, anchor.top - GAP - M);
}

/** What `visibleRoom` reads of the window and its screen. */
export interface WindowGeometry {
  innerWidth: number; innerHeight: number; outerWidth: number; outerHeight: number; screenX: number; screenY: number;
  screen: { availWidth: number; availHeight: number; availLeft?: number; availTop?: number };
}

/** The window's room on screen, in page pixels: the window less what hangs off the screen's
 * right or bottom edge (macOS lets a window be dragged there, and a popover placed in that part
 * is cut). `zoom` is the webview's (screen coordinates are device-independent pixels, the page's
 * are those over the zoom). The whole window when the numbers make no sense (tests). */
export function visibleRoom(g: WindowGeometry, zoom: number): { w: number; h: number } {
  const top = g.screenY + (g.outerHeight - g.innerHeight * zoom); // the title bar, if any, is above
  const left = g.screenX + (g.outerWidth - g.innerWidth * zoom) / 2;
  const h = ((g.screen.availTop ?? 0) + g.screen.availHeight - top) / zoom;
  const w = ((g.screen.availLeft ?? 0) + g.screen.availWidth - left) / zoom;
  const fit = (v: number, max: number) => (Number.isFinite(v) && v > 0 && g.outerHeight > 0 ? Math.min(v, max) : max);
  return { w: fit(w, g.innerWidth), h: fit(h, g.innerHeight) };
}

/** `visibleRoom` for this window: the room a popover is kept in. */
export function viewport(): { w: number; h: number } {
  const zoomed = '__TAURI_INTERNALS__' in window; // only the app's webview zooms (ui/zoom.ts)
  return visibleRoom(window, zoomed ? useZoom.getState().zoom / 100 : 1);
}

/** Places the popover `ref` at `anchor` from its measured size, before paint. */
export function usePopoverPlace(ref: RefObject<HTMLElement | null>, anchor: DOMRect | null): { left: number; top: number } | undefined {
  const [pos, setPos] = useState<{ left: number; top: number }>();
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos(placePopover(anchor, { w: r.width, h: r.height }, viewport()));
  }, [ref, anchor]);
  return pos;
}
