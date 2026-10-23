import type { CollapsedRow } from '../api/gen/CollapsedRow';
import type { RowPayload } from '../api/gen/RowPayload';
import type { ColumnWidths } from './columns';

/** A ⋮ row's texts: its Message cell, its Author cell and its accessible name. */
export function collapsedTexts(c: CollapsedRow): { message: string; authors: string; label: string } {
  const authors = c.authors === 1 ? '1 author' : `${c.authors} authors`;
  return {
    message: `⋮ ${c.count} more commits on ${c.branch}`,
    authors,
    label: `${c.count} more commits on ${c.branch}, ${authors}. Press Enter to show them.`,
  };
}

/** What the layout gives a `collapsed` row without its `collapsed` field (never, by contract). */
const NONE: CollapsedRow = { key: '', branch: '', count: 0, authors: 0 };

/**
 * A collapsed branch's ⋮ row's cells after the Graph column: the commits it holds, dimmed (not
 * bold), and how many authors made them; Date and SHA are empty. It's no commit: no message
 * tooltip, no SHA to copy. `dim`: the row-dim classes, as a commit row's text cells take them.
 */
export function CollapsedCells({ row, cols, dim }: { row: RowPayload; cols: ColumnWidths; dim: string }) {
  const t = collapsedTexts(row.collapsed ?? NONE);
  return (
    <>
      <span role="gridcell" data-col="message" className={`col-msg${dim}`} style={{ width: cols.message }}>
        <span className="dim collapsed-msg">{t.message}</span>
      </span>
      {cols.author > 0 && <span role="gridcell" data-col="author" className={`col-author${dim}`} style={{ width: cols.author }}>{t.authors}</span>}
      {cols.date > 0 && <span role="gridcell" data-col="date" className={`col-date${dim}`} style={{ width: cols.date }} />}
      {cols.sha > 0 && <span role="gridcell" data-col="sha" className={`col-sha${dim}`} style={{ width: cols.sha }} />}
    </>
  );
}
