import { useVirtualizer } from '@tanstack/react-virtual';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode, type RefObject } from 'react';
import type { RevealPayload } from '../../api/gen/RevealPayload';
import type { RowPayload } from '../../api/gen/RowPayload';
import { useAppState } from '../../app/state';
import type { MenuEventLike } from '../../menu/menuStore';
import { expand } from '../../presets/store';
import { registerKeyHints } from '../../shortcuts/hints';
import { registerKeys } from '../../ui/keyRouter';
import { isEditableTarget } from '../../ui/keys';
import { lanesWidth, type ColumnWidths } from '../columns';
import type { Metrics } from '../geometry';
import { GraphCanvas, type GraphCanvasHandle } from '../GraphCanvas';
import { dottedAt, foldMarkAt } from '../ghosts';
import type { NodeGeometry } from '../draw';
import { CLOSED, hoverStep, type HoverEvent, type HoverState } from './hoverIntent';
import { panelBox, REVEAL_HOVER_MS, revealPlacement } from './placement';
import { cacheKey, closeReveal, fetchReveal, openReveal, revealGraphChanged, revealKeyOf, useReveal, type RevealOpen } from './revealStore';
import './reveal.css';

type HitGeometry = NodeGeometry & { rows: RowPayload[]; scrollTop: number };
interface Target { key: string; rowIndex: number; below: boolean }

/** A reveal panel row's DOM id: apart from the graph's (the panel may show a commit the graph
 * shows too, its "parent"). */
export const revealRowId = (id: string) => `reveal-row-${id}`;

/**
 * The reveal panel's opening and closing for one graph (branch presets): the pointer over a ⋮ row
 * or a folded merge's row (`move`, `leave`), a press on one (`down`), Enter on a selected one
 * (`keyDown`), a press elsewhere and Esc while one is open. One timeout while the pointer rests on
 * the row (REVEAL_HOVER_MS), and listeners only while a panel is open: nothing runs at rest.
 * Entering a row prefetches its panel's rows, on the side it would open on.
 */
export function useRevealPanel(tabId: string, rows: RowPayload[], scrollRef: RefObject<HTMLDivElement | null>, rowH: number) {
  const hoverOpens = useAppState((s) => s.settings.collapseHoverOpen);
  const opens = useRef(hoverOpens);
  opens.current = hoverOpens;
  const open = useReveal((s) => (tabId ? s.tabs[tabId]?.open ?? null : null));
  const state = useRef<HoverState>(CLOSED);
  const target = useRef<Target | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [focusRequest, setFocusRequest] = useState(0);
  const wantFocus = useRef(false);
  const rowsRef = useRef(rows);
  rowsRef.current = rows;

  const stopTimer = () => { if (timer.current !== null) { clearTimeout(timer.current); timer.current = null; } };
  const apply = useCallback(function apply(kind: HoverEvent['kind']) {
    const next = hoverStep(state.current, { kind }, opens.current);
    state.current = next;
    if (!next.waiting) stopTimer();
    else if (timer.current === null) timer.current = setTimeout(() => { timer.current = null; apply('timer'); }, REVEAL_HOVER_MS);
    const t = target.current;
    if (next.open && t) openReveal(tabId, t.key, t.rowIndex, t.below, next.pinned);
    else if (!next.open) {
      closeReveal(tabId);
      if (!next.waiting) target.current = null;
    }
  }, [tabId]);

  /** Makes row `i` the target, if it opens a panel. Not while a panel elsewhere is pinned, unless
   * `force` (a click or Enter on this one): that one closes first. */
  const aim = useCallback((i: number, force = false): boolean => {
    const row = rowsRef.current[i];
    const key = revealKeyOf(row);
    if (!key || !tabId) return false;
    if (target.current?.key === key) {
      target.current.rowIndex = i;
      return true;
    }
    if (state.current.pinned && !force) return false;
    if (target.current) {
      state.current = CLOSED;
      stopTimer();
      closeReveal(tabId);
    }
    const el = scrollRef.current;
    const top = el?.scrollTop ?? 0;
    const { below } = revealPlacement({ rowTop: i * rowH, viewportTop: top, viewportBottom: top + (el?.clientHeight ?? 0), rowH, count: row.collapsed?.count ?? row.fold?.count ?? 0, fold: !row.collapsed });
    target.current = { key, rowIndex: i, below };
    void fetchReveal(tabId, key, below);
    return true;
  }, [tabId, scrollRef, rowH]);

  /** The pointer moved over the grid: `x` in canvas coordinates, `y` from the scroller's top. */
  const move = useCallback((e: PointerEvent<HTMLElement>, geo: HitGeometry, x: number, y: number) => {
    if (!tabId) return;
    if (e.target instanceof Element && e.target.closest('.reveal-panel')) {
      if (target.current) apply('enterPanel');
      return;
    }
    const i = Math.floor((y + geo.scrollTop) / rowH);
    if (aim(i)) {
      const mark = rowsRef.current[i].collapsed ? dottedAt(geo, x, y) : foldMarkAt(geo, x, y);
      apply(mark === i ? 'enterDotted' : 'enterRow');
      return;
    }
    // A fold's stub reaches into the row below its merge.
    const f = foldMarkAt(geo, x, y);
    if (f !== null && aim(f)) {
      apply('enterDotted');
      return;
    }
    if (target.current) apply('leave');
  }, [tabId, rowH, aim, apply]);

  const leave = useCallback(() => { if (target.current) apply('leave'); }, [apply]);

  /** A press on the grid: on a ⋮ or folded merge row, opens its panel pinned. */
  const down = useCallback((e: PointerEvent<HTMLElement>) => {
    if (e.button !== 0 || !tabId || (e.target instanceof Element && e.target.closest('.reveal-panel'))) return;
    const el = e.currentTarget;
    const i = Math.floor((e.clientY - el.getBoundingClientRect().top + el.scrollTop) / rowH);
    if (aim(i, true)) apply('click');
  }, [tabId, rowH, aim, apply]);

  /** Enter on the grid with a ⋮ or folded merge row selected: its panel, pinned, focus inside. */
  const keyDown = useCallback((e: KeyboardEvent, selected: number): boolean => {
    if (e.key !== 'Enter' || e.target !== e.currentTarget || e.ctrlKey || e.altKey || e.metaKey || e.shiftKey) return false;
    if (!aim(selected, true)) return false;
    apply('click');
    wantFocus.current = true;
    setFocusRequest((n) => n + 1);
    return true;
  }, [aim, apply]);

  /** Whether the panel should take the keyboard now (once per Enter). */
  const takeFocus = useCallback(() => {
    const want = wantFocus.current;
    wantFocus.current = false;
    return want;
  }, []);

  /** A context menu from the panel: it stays while the pointer is on the menu. */
  const pin = useCallback(() => { if (state.current.open && !state.current.pinned) apply('click'); }, [apply]);

  // Closed or moved from outside (a new graph payload): follow it.
  useEffect(() => {
    if (!open && state.current.open) {
      state.current = CLOSED;
      target.current = null;
      stopTimer();
    } else if (open && target.current?.key === open.key) target.current.rowIndex = open.rowIndex;
  }, [open]);

  // A new graph payload drops the cached panels (their lanes are the old layout's).
  const seenRows = useRef<RowPayload[] | null>(null);
  useLayoutEffect(() => {
    if (!tabId || seenRows.current === rows) return;
    if (seenRows.current !== null) revealGraphChanged(tabId, rows);
    seenRows.current = rows;
  }, [tabId, rows]);

  useEffect(() => () => {
    stopTimer();
    if (tabId) closeReveal(tabId);
  }, [tabId]);

  // While a panel is open: a press anywhere but the panel and its row closes it, and so does Esc,
  // ahead of the app's own Esc (which would leave a compare).
  const isOpen = open !== null;
  const openRow = open?.rowIndex ?? -1;
  useEffect(() => {
    if (!isOpen) return;
    const away = (e: globalThis.PointerEvent) => {
      const t = e.target instanceof Element ? e.target : null;
      if (t?.closest('.reveal-panel')) return;
      const row = t?.closest('.graph-row');
      if (row && scrollRef.current?.contains(row) && row.getAttribute('aria-rowindex') === String(openRow + 1)) return;
      apply('clickOutside');
    };
    window.addEventListener('pointerdown', away, true);
    const off = registerKeys('overlay', (e) => {
      if (e.key !== 'Escape' || e.ctrlKey || e.altKey || e.metaKey || e.shiftKey) return;
      const el = scrollRef.current;
      if (!el || el.checkVisibility?.() === false) return;
      const t = e.target instanceof Element ? e.target : null;
      const inPanel = !!t?.closest('.reveal-panel');
      if (!inPanel && isEditableTarget(t)) return;
      apply('escape');
      // Back to the row: the grid has the keyboard, its selection on the row.
      if (inPanel || t === el) el.focus({ preventScroll: true });
      e.preventDefault();
      return 'handled';
    });
    return () => {
      window.removeEventListener('pointerdown', away, true);
      off();
    };
  }, [isOpen, openRow, apply, scrollRef]);

  return { open, focusRequest, takeFocus, move, leave, down, keyDown, pin };
}

/** The open panel's rows: fetched (or cached), and while a refetch for the same key runs (a new
 * graph payload), the ones shown so far. `null`: nothing to show. */
export function useRevealPayload(tabId: string, open: RevealOpen | null): RevealPayload | null {
  const cached = useReveal((s) => (open ? s.tabs[tabId]?.cache.get(cacheKey(open.key, open.below)) : undefined));
  const key = open?.key;
  const below = open?.below;
  useEffect(() => {
    if (key !== undefined && below !== undefined && cached === undefined) void fetchReveal(tabId, key, below);
  }, [tabId, key, below, cached]);
  const shown = useRef<RevealPayload | null>(null);
  if (!open) shown.current = null;
  else if (cached !== undefined) shown.current = cached;
  else if (shown.current?.key !== open.key) shown.current = null;
  return shown.current;
}

const NO_ROWS: Set<number> = new Set();

/** One panel row: GraphView's own row component, with the panel's selection and badges. */
export type RenderRevealRow = (row: RowPayload, index: number, start: number, selected: boolean, onSelect: (index: number) => void, badges: readonly string[] | undefined) => ReactNode;

export interface RevealPanelProps {
  tabId: string;
  open: RevealOpen;
  payload: RevealPayload;
  /** The graph's scroller: the viewport the panel is fitted to when it opens. */
  scrollRef: RefObject<HTMLDivElement | null>;
  viewportW: number;
  viewportH: number;
  metrics: Metrics;
  cols: ColumnWidths;
  /** The graph's lane clipping and lane scroll: the panel's lanes take the same x. */
  clipped: boolean;
  scrollX: number;
  avatar?: (email: string) => ImageBitmap | null;
  avatarVersion?: number;
  /** The selected commit when it's one of the panel's (a hidden commit), else null. */
  selectedId: string | null;
  onSelect(row: RowPayload): void;
  onMenu?(e: MenuEventLike, row: RowPayload): void;
  renderRow: RenderRevealRow;
  /** Bumped by Enter on the panel's row; `takeFocus` then says whether the panel takes the
   * keyboard (once per Enter, also when the panel only mounts afterwards). */
  focusRequest: number;
  takeFocus(): boolean;
}

/**
 * The commits a ⋮ row or a folded merge holds, over the graph, which never moves: above the row
 * (its bottom edge on the row's top) or below it, with the title row on the side away from it so
 * the lanes run straight into the graph. It sits in the graph's scrolled content at its left edge,
 * and draws with the graph's own row component, columns, metrics and canvas (GraphCanvas,
 * draw.ts), so its lanes and columns line up with the graph's; its frame and shadow are outside
 * its content box.
 */
export function RevealPanel({ tabId, open, payload, scrollRef, viewportW, viewportH, metrics, cols, clipped, scrollX, avatar, avatarVersion, selectedId, onSelect, onMenu, renderRow, focusRequest, takeFocus }: RevealPanelProps) {
  const rowH = metrics.rowH;
  const rows = payload.rows;
  const listRef = useRef<HTMLDivElement>(null);
  const canvas = useRef<GraphCanvasHandle>(null);
  // Fitted to the viewport as it was when the rows arrived: scrolling the graph afterwards moves
  // the panel with its row, never resizes it.
  const box = useMemo(() => {
    const top = scrollRef.current?.scrollTop ?? 0;
    return panelBox(open.below, { rowTop: open.rowIndex * rowH, viewportTop: top, viewportBottom: top + viewportH, rowH, rows: rows.length });
  }, [open.below, open.rowIndex, open.key, rowH, rows.length, viewportH, scrollRef]); // eslint-disable-line react-hooks/exhaustive-deps
  const listH = box.height - rowH;
  const sel = selectedId === null ? -1 : rows.findIndex((r) => r.id === selectedId);
  const badges = useMemo(() => {
    const m = new Map<number, string[]>();
    for (const t of payload.tags) m.set(t.row, [...(m.get(t.row) ?? []), t.text]);
    return m;
  }, [payload.tags]);
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => listRef.current, estimateSize: () => rowH, overscan: 10, initialRect: { width: 1200, height: listH } });
  const rowsRef = useRef(rows);
  rowsRef.current = rows;
  const select = useCallback((i: number) => { const r = rowsRef.current[i]; if (r) onSelect(r); }, [onSelect]);

  useEffect(() => {
    if (takeFocus()) listRef.current?.focus({ preventScroll: true });
  }, [focusRequest, takeFocus]);
  // The canvas's own effect ran before the list's ref was attached (children first), so it drew
  // nothing: draw now, as GraphView does for its own.
  useLayoutEffect(() => canvas.current?.sync(), []);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    const page = Math.max(1, Math.floor(listH / rowH) - 1);
    const moves: Record<string, number> = { ArrowDown: sel + 1, ArrowUp: sel - 1, PageDown: sel + page, PageUp: sel - page, Home: 0, End: rows.length - 1 };
    if (e.key in moves && !(e.shiftKey && (e.key === 'ArrowDown' || e.key === 'ArrowUp'))) {
      e.preventDefault();
      e.stopPropagation();
      const i = Math.max(0, Math.min(rows.length - 1, moves[e.key]));
      select(i);
      v.scrollToIndex(i, { align: 'auto' });
      canvas.current?.sync();
      return;
    }
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      e.preventDefault();
      e.stopPropagation();
      const el = sel >= 0 ? document.getElementById(revealRowId(rows[sel].id)) : null;
      if (!el || !onMenu) return;
      const r = el.getBoundingClientRect();
      onMenu({ preventDefault() {}, stopPropagation() {}, clientX: r.left, clientY: r.bottom, timeStamp: performance.now() }, rows[sel]);
      return;
    }
    // The grid's own keys (it's the panel's parent) stay out of the panel.
    if (e.key.startsWith('Arrow') || e.key === 'Enter' || e.shiftKey) e.stopPropagation();
  };

  const fold = payload.key.startsWith('fold:');
  const what = fold ? `${payload.count} ${payload.count === 1 ? 'commit' : 'commits'} merged` : `${payload.count} hidden ${payload.count === 1 ? 'commit' : 'commits'}`;
  const title = (
    <div className="reveal-title" style={{ height: rowH, width: viewportW }}>
      <span className="reveal-branch">{payload.branch}</span>
      <span className="reveal-what">· {what}</span>
      <button type="button" className="reveal-expand" onClick={() => { void expand(tabId, payload.key); closeReveal(tabId); scrollRef.current?.focus({ preventScroll: true }); }}>Expand in graph</button>
    </div>
  );
  // Lanes past the Graph column pack into its collapse zone, as the graph's do.
  const panelClipped = clipped || cols.graph < lanesWidth(payload.maxLanes, metrics);
  return (
    <div className="reveal-panel" data-below={open.below} style={{ top: box.top, height: box.height }}>
      {!open.below && title}
      <div ref={listRef} className="reveal-list" role="grid" aria-label={`${payload.branch}: ${what}`} aria-rowcount={rows.length} tabIndex={0} style={{ height: listH }} onKeyDown={onKeyDown} onScroll={() => canvas.current?.sync()}>
        <div style={{ height: v.getTotalSize(), position: 'relative' }}>
          {v.getVirtualItems().map((item) => renderRow(rows[item.index], item.index, item.start, item.index === sel, select, badges.get(item.index)))}
          <div className="graph-canvas-clip">
            <GraphCanvas ref={canvas} scroller={listRef} rows={rows} width={cols.graph} height={listH} left={cols.labels} metrics={metrics} labeledRows={NO_ROWS} avatar={avatar} avatarVersion={avatarVersion} clipped={panelClipped} scrollX={scrollX} selected={sel} />
          </div>
        </div>
      </div>
      {open.below && title}
    </div>
  );
}

// Shown in the Keyboard Shortcuts panel (Ctrl+/); metadata only.
registerKeyHints([
  { id: 'key.reveal.open', section: 'Graph', label: 'Show the commits a collapsed branch or folded merge holds', keys: ['Enter'], context: '(on a ⋮ row or a folded merge)', source: 'graph/reveal/RevealPanel.tsx' },
  { id: 'key.reveal.move', section: 'Graph', label: 'Select the previous / next hidden commit', keys: ['Up', 'Down'], context: '(in a reveal panel)', source: 'graph/reveal/RevealPanel.tsx' },
  { id: 'key.reveal.close', section: 'Graph', label: 'Close the reveal panel', keys: ['Esc'], context: '(while one is open)', source: 'graph/reveal/RevealPanel.tsx' },
]);
