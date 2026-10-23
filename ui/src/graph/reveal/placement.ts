// Where the reveal panel opens over the graph (branch presets): pure, in content coordinates
// (0 is the first row's top), so the panel can sit in the scrolled content beside the rows.

/** The rest of a ⋮ row (or a folded merge's row) opens its panel after this long; its dotted lane
 * (or the merge's mark) at once. */
export const REVEAL_HOVER_MS = 400;
/** Above the row only when the title and this many rows (fewer for a shorter run) fit there. */
export const REVEAL_MIN_ROWS = 8;

/** The panel on one side of the row at `rowTop`: `rows` rows and the title row, as many as fit
 * between the row and the viewport's edge on that side (at least one), in whole rows. */
export function panelBox(below: boolean, a: { rowTop: number; viewportTop: number; viewportBottom: number; rowH: number; rows: number }): { top: number; height: number } {
  const room = below ? a.viewportBottom - (a.rowTop + a.rowH) : a.rowTop - a.viewportTop;
  const fit = Math.floor(room / a.rowH) - 1;
  const n = Math.max(Math.min(a.rows, 1), Math.min(a.rows, fit));
  const height = (n + 1) * a.rowH;
  return { top: below ? a.rowTop + a.rowH : a.rowTop - height, height };
}

/** Above when the space above the row fits the title and min(count, REVEAL_MIN_ROWS) rows; the panel's height
 * is its rows (and title) clipped to the viewport side it opens on. */
export function revealPlacement(a: { rowTop: number; viewportTop: number; viewportBottom: number; rowH: number; count: number; fold: boolean }): { below: boolean; top: number; height: number } {
  // A fold's history is older than its merge: always below.
  const below = a.fold || a.rowTop - a.viewportTop < (1 + Math.min(a.count, REVEAL_MIN_ROWS)) * a.rowH;
  return { below, ...panelBox(below, { ...a, rows: a.count }) };
}
