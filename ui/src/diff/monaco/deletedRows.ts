// The layout of a lazily drawn deleted-lines zone (deletedZone.ts), kept free of Monaco so it's
// unit-testable in plain Node: its old lines in chunks, the chunks a scroll position needs, and
// which old line a visual row is. A row is one visual line: an old line, or a part of one that
// wraps.

/** Deleted blocks of at least this many old lines are drawn lazily; smaller ones as Monaco does. */
export const LAZY_MIN_LINES = 400;
/** Old lines per chunk, the unit drawn and dropped. */
export const CHUNK_LINES = 100;

export class ZoneRows {
  /** Row `i`'s old line is the last line whose first row is at or before `i`. */
  private readonly firstRow: Int32Array;
  readonly rows: number;
  readonly viewLineCounts: readonly number[];
  readonly chunkLines: number;

  /** `viewLineCounts`: each old line's rows (1, or more when it wraps). */
  constructor(viewLineCounts: readonly number[], chunkLines = CHUNK_LINES) {
    this.viewLineCounts = viewLineCounts;
    this.chunkLines = chunkLines;
    this.firstRow = new Int32Array(viewLineCounts.length + 1);
    for (let i = 0; i < viewLineCounts.length; i++) this.firstRow[i + 1] = this.firstRow[i]! + viewLineCounts[i]!;
    this.rows = this.firstRow[viewLineCounts.length]!;
  }

  get lines(): number { return this.viewLineCounts.length; }
  get chunks(): number { return Math.ceil(this.lines / this.chunkLines); }

  /** The 0-based old line (within the block) at row `row`, clamped to the block. */
  lineOfRow(row: number): number {
    let lo = 0;
    let hi = this.lines - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this.firstRow[mid]! <= row) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }

  firstRowOf(line: number): number { return this.firstRow[line]!; }

  /** Chunk `c`'s old lines, `[start, end)`, and its first row. */
  chunk(c: number): { start: number; end: number; firstRow: number; rows: number } {
    const start = c * this.chunkLines;
    const end = Math.min(this.lines, start + this.chunkLines);
    return { start, end, firstRow: this.firstRow[start]!, rows: this.firstRow[end]! - this.firstRow[start]! };
  }

  /** The chunks with a row in `[fromRow, toRow)`, as `[first, last]`; null when none. */
  chunksIn(fromRow: number, toRow: number): [number, number] | null {
    const from = Math.max(0, Math.floor(fromRow));
    const to = Math.min(this.rows, Math.ceil(toRow));
    if (from >= to) return null;
    return [Math.floor(this.lineOfRow(from) / this.chunkLines), Math.floor(this.lineOfRow(to - 1) / this.chunkLines)];
  }
}

/**
 * The chunks to draw for a zone whose top is `top` px from the top of the editor's viewport
 * (Monaco's onDomNodeTop), `viewport` px tall: the rows on screen and half a viewport either side,
 * so a scroll shows lines already drawn. Null when none is near.
 */
export function chunksNear(rows: ZoneRows, top: number, viewport: number, lineHeight: number): [number, number] | null {
  return rows.chunksIn((-top - viewport / 2) / lineHeight, (-top + viewport * 1.5) / lineHeight);
}

/** The widest of `lines` in columns, a tab reaching the next multiple of `tabSize`: the zone's
 * minimum width, which Monaco measures as it draws every line. */
export function widestColumns(lines: Iterable<string>, tabSize: number): number {
  let widest = 0;
  for (const line of lines) {
    let col = 0;
    for (let i = 0; i < line.length; i++) col = line.charCodeAt(i) === 9 ? col + tabSize - (col % tabSize) : col + 1;
    widest = Math.max(widest, col);
  }
  return widest;
}

/** Each lazily drawn zone's rows, by its element (Monaco's `.line-delete` zone). */
const lazyZones = new WeakMap<Element, ZoneRows>();
export const registerLazyZone = (zone: Element, rows: ZoneRows): void => { lazyZones.set(zone, rows); };
/** A deleted-lines zone's rows when it's drawn lazily (only some of its `.view-line`s exist). */
export const lazyZoneRows = (zone: Element): ZoneRows | undefined => lazyZones.get(zone);

/** The row of `el` (inside a `.view-line`) in a lazily drawn zone: its chunk's first row plus its
 * place among the chunk's rows. -1 outside a chunk. */
export function rowOfElement(el: Element): number {
  const line = el.closest('.view-line');
  const chunk = line?.parentElement;
  const first = Number(chunk?.dataset.firstRow ?? NaN);
  if (!line || !chunk || Number.isNaN(first)) return -1;
  return first + [...chunk.children].indexOf(line);
}
