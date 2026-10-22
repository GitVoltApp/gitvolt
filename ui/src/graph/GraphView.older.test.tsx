import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { GraphPayload } from '../api/gen/GraphPayload';
import type { RowPayload } from '../api/gen/RowPayload';
import { RepoContext } from '../app/repoContext';
import { GraphView } from './GraphView';
import { METRICS } from './metrics';

vi.mock('../api/transport', () => ({ copyText: vi.fn(async () => {}), inTauri: () => false }));
const runtime = vi.hoisted(() => ({ loadOlder: vi.fn(async () => {}), olderViewport: vi.fn() }));
vi.mock('../app/runtime', async (orig) => ({ ...(await orig<typeof import('../app/runtime')>()), ...runtime }));
HTMLCanvasElement.prototype.getContext = (() => null) as never;
Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, value: 1200 });
Object.defineProperty(HTMLElement.prototype, 'clientHeight', { configurable: true, value: 600 });

const row = (i: number): RowPayload => ({ id: `${i}-`.padEnd(40, '0'), kind: 'commit', lane: 0, color: 0, segments: [], summary: `c${i}`, bodyFirstLine: '', authorName: 'A', authorEmail: '', authorTime: 0, committerTime: 0, parents: [], mrRefs: [], wip: null });
const graphOf = (n: number, truncated: boolean): GraphPayload => ({ rows: Array.from({ length: n }, (_, i) => row(i)), labels: [], maxLanes: 1, pinnedRefs: [], head: { branch: null, target: null, detached: false, unborn: false }, truncated, worktrees: [] });
const ui = (g: GraphPayload) => (
  <RepoContext.Provider value={{ tabId: 't', repoId: 1, path: '/r', worktree: '/r', info: null }}><GraphView graph={g} repoId="/r" /></RepoContext.Provider>
);

beforeEach(() => vi.clearAllMocks());

describe('Load older commits row', () => {
  it('a truncated graph ends with Load older commits', () => {
    render(ui(graphOf(5, true)));
    const btn = screen.getByRole('button', { name: /Load older commits/ });
    expect(btn.closest('.load-older-row')).toHaveStyle({ top: `${5 * METRICS.rowH}px` });
    expect(screen.getByText('5 commits loaded')).toBeInTheDocument();
    fireEvent.click(btn);
    expect(runtime.loadOlder).toHaveBeenCalledWith('t');
  });

  it('no row when the graph isn\'t truncated', () => {
    render(ui(graphOf(5, false)));
    expect(screen.queryByRole('button', { name: /Load older commits/ })).toBeNull();
  });

  it('arrow down from the last row reaches it, and arrow up leaves it', () => {
    render(ui(graphOf(3, true)));
    const grid = screen.getByRole('grid', { name: 'Commit graph' });
    fireEvent.keyDown(grid, { key: 'End' });
    fireEvent.keyDown(grid, { key: 'ArrowDown' });
    const btn = screen.getByRole('button', { name: /Load older commits/ });
    expect(btn).toHaveFocus();
    fireEvent.keyDown(btn, { key: 'ArrowUp' });
    expect(grid).toHaveFocus();
  });

  it('loading keeps the scroll position: the first visible row is unchanged', () => {
    const { rerender } = render(ui(graphOf(40, true)));
    const grid = screen.getByRole('grid', { name: 'Commit graph' });
    act(() => { grid.scrollTop = 10 * METRICS.rowH; fireEvent.scroll(grid); });
    const at = (i: number) => document.getElementById(`graph-row-${row(i).id}`);
    expect(at(10)).toHaveStyle({ top: `${10 * METRICS.rowH}px` });
    rerender(ui(graphOf(80, true)));
    expect(grid.scrollTop).toBe(10 * METRICS.rowH);
    expect(at(10)).toHaveStyle({ top: `${10 * METRICS.rowH}px` });
  });
});
