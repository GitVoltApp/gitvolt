import { act, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { GraphPayload } from '../../api/gen/GraphPayload';
import type { RevealPayload } from '../../api/gen/RevealPayload';
import type { RowPayload } from '../../api/gen/RowPayload';
import { RepoContext } from '../../app/repoContext';
import { useAppState } from '../../app/state';
import { usePresetSession } from '../../presets/store';
import { DEFAULT_DENSITY, useDensity } from '../../theme/density';
import { laneX } from '../geometry';
import { GraphView } from '../GraphView';
import { REVEAL_HOVER_MS } from './placement';
import { cacheKey, closeReveal, openReveal, revealGraphChanged, seedReveal, tabReveal, useReveal } from './revealStore';

// The canvases record their props: the panel's must be drawn exactly as the graph's.
const rec = vi.hoisted(() => ({ canvases: [] as Record<string, unknown>[] }));
vi.mock('../GraphCanvas', () => ({
  GraphCanvas: (p: Record<string, unknown>) => {
    rec.canvases.push(p);
    return <canvas data-testid="graph-canvas" data-rows={(p.rows as unknown[]).length} style={{ left: p.left as number, width: p.width as number }} />;
  },
}));

Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, value: 1200 });
Object.defineProperty(HTMLElement.prototype, 'clientHeight', { configurable: true, value: 600 });

const TAB = 't1';
const KEY = 'branch:feature/payments';
const sha = (n: number) => n.toString(16).padStart(40, '0');
const row = (id: string, lane: number, o: Partial<RowPayload> = {}): RowPayload => ({
  id, kind: 'commit', lane, color: lane, segments: [], summary: `commit ${id.slice(-2)}`, bodyFirstLine: '', authorName: 'Ada Lovelace', authorEmail: 'ada@example.com', authorTime: 0, committerTime: 0, parents: [], mrRefs: [], wip: null, ...o,
});
const COLLAPSED_AT = 10;
// Ten trunk commits, the branch's tip, its ⋮ row, then the trunk and the branch's oldest.
const rows: RowPayload[] = [
  ...Array.from({ length: COLLAPSED_AT - 1 }, (_, i) => row(sha(i + 1), 0)),
  row(sha(0xf0), 1),
  { ...row('collapsed:feature/payments', 1, { kind: 'collapsed', summary: '', authorName: '', authorEmail: '' }), collapsed: { key: KEY, branch: 'feature/payments', count: 58, authors: 3 } },
  row(sha(0xf1), 1),
  ...Array.from({ length: 20 }, (_, i) => row(sha(0x100 + i), 0)),
];
const graph: GraphPayload = {
  rows, labels: [], maxLanes: 2, pinnedRefs: [], head: { branch: 'refs/heads/main', target: sha(1), detached: false, unborn: false }, truncated: false, worktrees: [],
  view: { branches: [], trunk: ['main'] },
};
const payload = (o: Partial<RevealPayload> = {}): RevealPayload => ({
  key: KEY, branch: 'feature/payments', count: 58, rows: Array.from({ length: 12 }, (_, i) => row(sha(0x200 + i), 1)), tags: [], entryLane: 1, maxLanes: 2, ...o,
});

const view = (props: Partial<Parameters<typeof GraphView>[0]> = {}) => (
  <RepoContext value={{ tabId: TAB, repoId: 1, path: '/repo', worktree: '/repo', info: null }}>
    <GraphView graph={graph} repoId="/repo" {...props} />
  </RepoContext>
);
const canvasProps = (n: number) => rec.canvases.filter((p) => (p.rows as unknown[]).length === n).at(-1)!;
const panelOf = (c: HTMLElement) => c.querySelector<HTMLElement>('.reveal-panel');

beforeEach(() => {
  useDensity.setState({ density: DEFAULT_DENSITY });
  useReveal.setState({ tabs: {} });
  rec.canvases.length = 0;
});
afterEach(() => vi.useRealTimers());

describe('the reveal panel', () => {
  it("the panel's canvas and columns line up with the graph's", () => {
    seedReveal(TAB, false, payload());
    const { container } = render(view());
    act(() => openReveal(TAB, KEY, COLLAPSED_AT, false, true));
    const panel = panelOf(container)!;
    expect(panel).not.toBeNull();
    // In the graph's scrolled content, at its left edge: no inset, border or padding (the frame is
    // a box-shadow, outside the box).
    expect(panel.parentElement).toBe(container.querySelector('.graph-scroll')!.firstElementChild);
    expect(panel.style.left).toBe('');
    const g = canvasProps(graph.rows.length), p = canvasProps(12);
    for (const k of ['width', 'left', 'metrics', 'scrollX', 'clipped'] as const) expect(p[k], k).toBe(g[k]);
    // The entry lane's x is the ⋮ row's lane's.
    const m = g.metrics as Parameters<typeof laneX>[1];
    expect(laneX(payload().entryLane, m)).toBe(laneX(rows[COLLAPSED_AT].lane, m));
    // Same row height and column widths, cell by cell.
    const graphCells = [...container.querySelectorAll<HTMLElement>('.graph-scroll > div > .graph-row')][0].querySelectorAll<HTMLElement>('[data-col]');
    const panelRow = panel.querySelector<HTMLElement>('.graph-row')!;
    expect(panelRow.style.height).toBe(`${m.rowH}px`);
    expect([...panelRow.querySelectorAll<HTMLElement>('[data-col]')].map((c) => c.style.width)).toEqual([...graphCells].map((c) => c.style.width));
    // Above: its bottom edge on the ⋮ row's top.
    expect(parseFloat(panel.style.top) + parseFloat(panel.style.height)).toBe(COLLAPSED_AT * m.rowH);
  });

  it('the title sits away from the row', () => {
    seedReveal(TAB, false, payload());
    seedReveal(TAB, true, payload({ tags: [{ row: 11, text: 'parent' }] }));
    const { container } = render(view());
    act(() => openReveal(TAB, KEY, COLLAPSED_AT, false, true));
    expect(panelOf(container)!.firstElementChild).toHaveClass('reveal-title');
    expect(panelOf(container)!.firstElementChild).toHaveTextContent('feature/payments· 58 hidden commitsExpand in graph');
    act(() => openReveal(TAB, KEY, COLLAPSED_AT, true, true));
    const panel = panelOf(container)!;
    expect(panel.lastElementChild).toHaveClass('reveal-title');
    expect(parseFloat(panel.style.top)).toBe((COLLAPSED_AT + 1) * 28);
    // Tags are small badges after the message.
    expect(panel.querySelector(`#reveal-row-${sha(0x20b)} .fold-count`)).toHaveTextContent('parent');
  });

  it('Expand in graph unfolds it in this tab and closes the panel', () => {
    seedReveal(TAB, false, payload());
    const { container } = render(view());
    act(() => openReveal(TAB, KEY, COLLAPSED_AT, false, true));
    fireEvent.click(container.querySelector('.reveal-expand')!);
    expect(usePresetSession.getState().expanded[TAB]).toEqual([KEY]);
    expect(tabReveal(TAB).open).toBeNull();
    expect(document.activeElement).toBe(container.querySelector('.graph-scroll'));
    usePresetSession.setState({ expanded: {} });
  });

  it('a new graph payload drops cached reveals and closes a panel whose key is gone', () => {
    seedReveal(TAB, false, payload());
    openReveal(TAB, KEY, COLLAPSED_AT, false, true);
    // The rows moved: the panel follows its row, and its cached rows go (they're laid out on the old lanes).
    const moved = [row(sha(0x999), 0), ...rows];
    revealGraphChanged(TAB, moved);
    expect(tabReveal(TAB).open).toEqual({ key: KEY, rowIndex: COLLAPSED_AT + 1, below: false, pinned: true });
    expect(tabReveal(TAB).cache.has(cacheKey(KEY, false))).toBe(false);
    revealGraphChanged(TAB, rows.filter((r) => r.kind !== 'collapsed'));
    expect(tabReveal(TAB).open).toBeNull();
  });

  it('hovering: the dotted lane opens at once, the rest of the row after the wait, leaving closes', () => {
    vi.useFakeTimers();
    seedReveal(TAB, false, payload());
    const { container } = render(view());
    const grid = container.querySelector<HTMLElement>('.graph-scroll')!;
    const m = canvasProps(graph.rows.length).metrics as Parameters<typeof laneX>[1];
    const labels = (canvasProps(graph.rows.length).left as number);
    const y = COLLAPSED_AT * m.rowH + m.rowH / 2;
    // The message cell, away from the dotted lane: after REVEAL_HOVER_MS.
    fireEvent.pointerMove(grid, { clientX: labels + 400, clientY: y });
    expect(tabReveal(TAB).open).toBeNull();
    act(() => { vi.advanceTimersByTime(REVEAL_HOVER_MS); });
    expect(tabReveal(TAB).open).toMatchObject({ key: KEY, rowIndex: COLLAPSED_AT, pinned: false });
    fireEvent.pointerMove(grid, { clientX: labels + 400, clientY: 2 * m.rowH + 4 });
    expect(tabReveal(TAB).open).toBeNull();
    // The dotted lane: at once.
    fireEvent.pointerMove(grid, { clientX: labels + laneX(1, m), clientY: y });
    expect(tabReveal(TAB).open).toMatchObject({ key: KEY, pinned: false });
    fireEvent.pointerLeave(grid);
    expect(tabReveal(TAB).open).toBeNull();
    // Off in the settings: only a click opens it, pinned, until a click elsewhere.
    act(() => useAppState.setState((s) => ({ settings: { ...s.settings, collapseHoverOpen: false } })));
    fireEvent.pointerMove(grid, { clientX: labels + laneX(1, m), clientY: y });
    act(() => { vi.advanceTimersByTime(REVEAL_HOVER_MS); });
    expect(tabReveal(TAB).open).toBeNull();
    fireEvent.pointerDown(grid, { button: 0, clientX: labels + 400, clientY: y });
    expect(tabReveal(TAB).open).toMatchObject({ key: KEY, pinned: true });
    fireEvent.pointerLeave(grid);
    expect(tabReveal(TAB).open).not.toBeNull();
    fireEvent.pointerDown(document.body, { button: 0 });
    expect(tabReveal(TAB).open).toBeNull();
    act(() => useAppState.setState((s) => ({ settings: { ...s.settings, collapseHoverOpen: true } })));
  });
});

describe('acting on hidden commits', () => {
  it('clicking a panel row selects it, and arrow keys move through the rows', () => {
    seedReveal(TAB, false, payload());
    const onSelectRevealed = vi.fn();
    const { container, rerender } = render(view({ onSelectRevealed }));
    act(() => openReveal(TAB, KEY, COLLAPSED_AT, false, true));
    const rowEl = container.querySelector<HTMLElement>(`#reveal-row-${sha(0x203)}`)!;
    fireEvent.mouseDown(rowEl, { button: 0 });
    expect(onSelectRevealed).toHaveBeenLastCalledWith(payload().rows[3]);
    rerender(view({ onSelectRevealed, revealedId: sha(0x203) }));
    expect(container.querySelector(`#reveal-row-${sha(0x203)}`)).toHaveAttribute('aria-selected', 'true');
    // The panel's canvas draws the selection band.
    expect(canvasProps(12).selected).toBe(3);
    const list = container.querySelector<HTMLElement>('.reveal-list')!;
    fireEvent.keyDown(list, { key: 'ArrowDown' });
    expect(onSelectRevealed).toHaveBeenLastCalledWith(payload().rows[4]);
    fireEvent.keyDown(list, { key: 'Home' });
    expect(onSelectRevealed).toHaveBeenLastCalledWith(payload().rows[0]);
  });

  it('the commit menu opens for a panel row, and pins the panel', () => {
    seedReveal(TAB, false, payload());
    const onContextMenu = vi.fn();
    const { container } = render(view({ onContextMenu }));
    act(() => openReveal(TAB, KEY, COLLAPSED_AT, false, false));
    fireEvent.contextMenu(container.querySelector<HTMLElement>(`#reveal-row-${sha(0x205)}`)!);
    expect(onContextMenu).toHaveBeenCalledTimes(1);
    expect((onContextMenu.mock.calls[0][1] as RowPayload).id).toBe(sha(0x205));
  });

  it('Enter on a ⋮ row opens the panel pinned; Escape returns focus to the row', () => {
    seedReveal(TAB, true, payload());
    seedReveal(TAB, false, payload());
    const { container } = render(view({ selected: COLLAPSED_AT }));
    const grid = container.querySelector<HTMLElement>('.graph-scroll')!;
    grid.focus();
    fireEvent.keyDown(grid, { key: 'Enter' });
    expect(tabReveal(TAB).open).toMatchObject({ key: KEY, rowIndex: COLLAPSED_AT, pinned: true });
    expect(document.activeElement).toBe(container.querySelector('.reveal-list'));
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    expect(tabReveal(TAB).open).toBeNull();
    expect(document.activeElement).toBe(grid);
    closeReveal(TAB);
  });
});
