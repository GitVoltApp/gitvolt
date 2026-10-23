/**
 * Calls `cb` when the pointer leaves the window, or the window loses focus. A pointer that leaves
 * fast, straight off an element at the window's edge, or into another app, can leave that
 * element's own leave event unsent, and a tooltip waiting for it would stay up. Returns the
 * unsubscribe.
 */
export function onPointerLeftWindow(cb: () => void): () => void {
  // `relatedTarget` null: the pointer went to no element of this document (off the window).
  const out = (e: MouseEvent) => {
    if (e.relatedTarget === null) cb();
  };
  const root = document.documentElement;
  document.addEventListener('mouseout', out);
  root.addEventListener('mouseleave', cb);
  window.addEventListener('blur', cb);
  return () => {
    document.removeEventListener('mouseout', out);
    root.removeEventListener('mouseleave', cb);
    window.removeEventListener('blur', cb);
  };
}
