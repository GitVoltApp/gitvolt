/**
 * Scroll position that keeps `anchorId` at the same place on screen after the rows change
 * (a refresh after repo-changed/refs-updated, spec §4.4 "keeps the selection and scroll").
 * At the very top (`scrollTop` under 1 px) the view stays at the top instead, so a row
 * inserted above (a new row-0 WIP, "now", K37) shows; anchoring applies only once scrolled
 * down. `rowH` is the density's row height (`useGraphMetrics().rowH`).
 */
export function anchoredScrollTop(oldRows: readonly { id: string }[], newRows: readonly { id: string }[], anchorId: string | null, scrollTop: number, rowH: number): number {
  if (!anchorId || scrollTop < 1) return scrollTop;
  const before = oldRows.findIndex((r) => r.id === anchorId);
  const after = newRows.findIndex((r) => r.id === anchorId);
  if (before < 0 || after < 0) return scrollTop;
  return Math.max(0, scrollTop + (after - before) * rowH);
}

/**
 * Scroll position that shows row `index` after a write the user started moved HEAD: unchanged
 * while the row is fully on screen, else the row with `context` rows above it (a reveal's little
 * look-back, as for the tip after a rebase). Clamped to the scrollable range.
 */
export function revealScrollTop(index: number, scrollTop: number, viewportH: number, rowH: number, rowCount: number, context = 2): number {
  if (index < 0) return scrollTop;
  if (index * rowH >= scrollTop && (index + 1) * rowH <= scrollTop + viewportH) return scrollTop;
  const max = Math.max(0, rowCount * rowH - viewportH);
  return Math.min(max, Math.max(0, (index - context) * rowH));
}
