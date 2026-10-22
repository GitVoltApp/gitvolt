import { act, render } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { drawGraph } from '../../graph/draw';
import { METRICS } from '../../graph/metrics';
import { useTheme } from '../store';
import { THEMES } from '../themes';
import { LivePreview, PREVIEW_ROWS, PREVIEW_SELECTED } from './LivePreview';

vi.mock('../../graph/draw', async (importOriginal) => ({ ...(await importOriginal<typeof import('../../graph/draw')>()), drawGraph: vi.fn() }));
// A non-null context so GraphCanvas gets as far as calling drawGraph.
HTMLCanvasElement.prototype.getContext = (() => ({})) as never;

beforeEach(() => {
  act(() => useTheme.getState().set(THEMES['default-dark']));
  vi.mocked(drawGraph).mockClear();
});

describe('the live preview', () => {
  it("draws the graph's own rows and canvas, a file and diff lines, hidden from assistive tech and inert", () => {
    const { container } = render(<LivePreview />);
    const root = container.firstElementChild!;
    expect(root).toHaveAttribute('aria-hidden');
    const graph = root.querySelector('.theme-preview-graph')!;
    expect(graph).toHaveAttribute('inert');
    expect(graph).toHaveStyle({ height: `${PREVIEW_ROWS.length * METRICS.rowH}px` });
    const rows = graph.querySelectorAll('.graph-row');
    expect([...rows].map((r) => r.querySelector('.msg-summary')?.textContent)).toEqual(PREVIEW_ROWS.map((r) => r.summary));
    expect(rows[PREVIEW_SELECTED]).toHaveAttribute('aria-selected', 'true');
    // The chips: main (checked out) and the branch, then main's dimmed one on the selected row.
    expect([...graph.querySelectorAll('.ref-label .ref-name')].map((n) => n.textContent)).toEqual(['main', 'feature/trade-ui', 'main']);
    expect(graph.querySelector('.ref-label-head')).toHaveTextContent('main');
    expect(rows[PREVIEW_SELECTED].querySelector('.ref-label-dim')).not.toBeNull();
    expect(graph.querySelectorAll('canvas.graph-canvas')).toHaveLength(1);
    expect(root.querySelector('.theme-preview-file')).toHaveTextContent('economy.rs');
    expect(root.querySelector('.theme-preview-file .status-icon')).toHaveAttribute('data-status', 'modified');
    expect(root.querySelectorAll('.theme-preview-line')).toHaveLength(3);
    expect(root.innerHTML).toContain('var(--diff-removed-line)');
  });

  it("is drawn by the graph's draw code: its rows, the selected one and main's checked-out row", () => {
    render(<LivePreview />);
    const o = vi.mocked(drawGraph).mock.lastCall![1];
    expect(o.rows).toEqual(PREVIEW_ROWS);
    expect(o.selected).toBe(PREVIEW_SELECTED);
    expect(o.headRow).toBe(0);
    expect([...o.labeledRows]).toEqual([0, 1]);
    expect(o.first).toBe(0);
    expect(o.last).toBe(PREVIEW_ROWS.length);
    expect(o.colors).toEqual(THEMES['default-dark'].graph);
  });

  it('repaints when a colour is previewed, and when the preview ends', () => {
    render(<LivePreview />);
    const calls = vi.mocked(drawGraph).mock.calls.length;
    act(() => useTheme.getState().preview({ colors: { 'node-fill': '#123456' }, graph: ['#abcdef'] }));
    expect(vi.mocked(drawGraph).mock.calls.length).toBeGreaterThan(calls);
    expect(vi.mocked(drawGraph).mock.lastCall![1]).toMatchObject({ nodeFill: '#123456', colors: ['#abcdef'] });
    act(() => useTheme.getState().preview(null));
    expect(vi.mocked(drawGraph).mock.lastCall![1].colors).toEqual(THEMES['default-dark'].graph);
  });
});
