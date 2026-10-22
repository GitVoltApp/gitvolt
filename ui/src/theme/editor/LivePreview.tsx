import { useLayoutEffect, useRef, useState } from 'react';
import type { RefLabel } from '../../api/gen/RefLabel';
import type { RowPayload } from '../../api/gen/RowPayload';
import { StatusIcon } from '../../files/StatusIcon';
import { autoGraphWidth, type ColumnWidths } from '../../graph/columns';
import { GraphCanvas } from '../../graph/GraphCanvas';
import { GraphRow } from '../../graph/GraphView';
import type { BranchMembership } from '../../graph/membership';
import { useGraphMetrics } from '../../graph/metrics';
import { useDevicePixelRatio } from '../../graph/pixels';
import { HALF_BOTTOM, HALF_FULL, HALF_TOP } from '../../graph/segments';
import './editor.css';

/** A packed segment (`graph::Segment::pack`), in lane `color`. */
const seg = (from: number, to: number, half: number, color = from) => from | (to << 10) | (half << 20) | (color << 22);
const row = (n: number, kind: RowPayload['kind'], lane: number, segments: number[], summary: string, authorName: string): RowPayload => ({
  id: String(n).repeat(40), kind, lane, color: lane, segments, summary, bodyFirstLine: '', authorName, authorEmail: `${authorName.split(' ')[0].toLowerCase()}@example.com`,
  authorTime: 0, committerTime: 0, parents: [], mrRefs: [], wip: null,
});

/** The preview's history, as the graph builder lays it out: `feature/trade-ui` forks from main
 * (row 4), gets two commits beside one on main, and is merged back (row 0). */
export const PREVIEW_ROWS: readonly RowPayload[] = [
  row(1, 'merge', 0, [seg(0, 0, HALF_BOTTOM), seg(0, 1, HALF_BOTTOM, 1)], "Merge branch 'feature/trade-ui'", 'Mira Okonkwo'),
  row(2, 'commit', 1, [seg(0, 0, HALF_FULL), seg(1, 1, HALF_TOP), seg(1, 1, HALF_BOTTOM)], 'Trade screen lists the cargo hold', 'Tobin Vasquez'),
  row(3, 'commit', 0, [seg(0, 0, HALF_TOP), seg(0, 0, HALF_BOTTOM), seg(1, 1, HALF_FULL)], 'Fuel gauge reads the tank', 'Mira Okonkwo'),
  row(4, 'commit', 1, [seg(0, 0, HALF_FULL), seg(1, 1, HALF_TOP), seg(1, 1, HALF_BOTTOM)], 'Trade screen: buy and sell prices', 'Tobin Vasquez'),
  row(5, 'commit', 0, [seg(0, 0, HALF_TOP), seg(1, 0, HALF_TOP, 1), seg(0, 0, HALF_BOTTOM)], 'Cargo hold has a weight limit', 'Juno Halloran'),
];
const label = (r: number, name: string, isHead: boolean, remote: boolean): RefLabel => ({
  row: r, name, local: `refs/heads/${name}`, tag: false, isHead, worktree: null, checkedOut: null,
  remotes: remote ? [{ fullName: `refs/remotes/origin/${name}`, remote: 'origin', host: 'git.example.com', hostKind: 'generic' }] : [],
});
const LABELS: readonly RefLabel[][] = [[label(0, 'main', true, true)], [label(1, 'feature/trade-ui', false, false)], [], [], []];
/** The selected row: a commit on main below its tip, so it shows main's dimmed chip too (F7). */
export const PREVIEW_SELECTED = 2;
const MEMBERSHIP: BranchMembership = { name: 'main', color: 0, ref: 'refs/heads/main' };
const LABELED = new Set([0, 1]);
const LABELS_W = 120;
/** The width laid out before the preview is measured (tests: happy-dom has no layout). */
const FALLBACK_W = 338;
const noop = () => {};

/**
 * A few rows of the commit graph, drawn by the graph itself: its rows (GraphRow: chips, the
 * selected row, text colours) and its canvas (GraphCanvas: lines, merge curves, nodes, bands).
 * The canvas repaints on its own when the theme or a previewed colour changes (it reads the theme
 * store); nothing else runs. Inert: no hover, no clicks, no focus.
 */
function PreviewGraph() {
  const metrics = useGraphMetrics();
  const dpr = useDevicePixelRatio();
  const box = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(FALLBACK_W);
  // Measured before paint, then again only when the preview itself resizes.
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const measure = () => { if (el.clientWidth > 0) setWidth(el.clientWidth); };
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const graph = autoGraphWidth(2, metrics);
  const cols: ColumnWidths = { labels: LABELS_W, graph, message: Math.max(0, width - LABELS_W - graph), author: 0, date: 0, sha: 0, total: width };
  const rows = PREVIEW_ROWS as RowPayload[];
  return (
    <div ref={box} className="theme-preview-graph" style={{ height: rows.length * metrics.rowH }} inert>
      {rows.map((r, i) => (
        <GraphRow
          key={r.id}
          row={r}
          repoId=""
          index={i}
          start={i * metrics.rowH}
          rowH={metrics.rowH}
          dpr={dpr}
          selected={i === PREVIEW_SELECTED}
          cols={cols}
          labels={LABELS[i]}
          membership={i === PREVIEW_SELECTED ? MEMBERSHIP : null}
          onSelect={noop}
          onHover={noop}
          onCopySha={noop}
          dimmed={false}
          onBranchHover={noop}
          domId={`theme-preview-row-${i}`}
        />
      ))}
      <div className="graph-canvas-clip">
        <GraphCanvas rows={rows} width={graph} height={rows.length * metrics.rowH} left={LABELS_W} metrics={metrics} labeledRows={LABELED} selected={PREVIEW_SELECTED} headRow={0} />
      </div>
    </div>
  );
}

/**
 * A static slice of the app beside the colours: a few graph rows (the graph's own rows and
 * canvas), a changed file and three diff lines. They're drawn with the app's tokens and theme
 * store, so they show the applied theme (the edited one), previews included.
 */
export function LivePreview() {
  return (
    <div className="theme-preview" aria-hidden>
      <div className="theme-preview-title">Live preview</div>
      <PreviewGraph />
      <div className="theme-preview-file">
        <StatusIcon status="M" size={12} decorative />
        <span>economy.rs</span>
      </div>
      <div className="theme-preview-diff">
        <div className="theme-preview-line">{'  let base = cargo.value();'}</div>
        <div className="theme-preview-line" style={{ background: 'var(--diff-removed-line)' }}>{'- let fee = base * 0.9;'}</div>
        <div className="theme-preview-line" style={{ background: 'var(--diff-inserted-line)' }}>{'+ let fee = base * SELL_SHARE;'}</div>
      </div>
    </div>
  );
}
