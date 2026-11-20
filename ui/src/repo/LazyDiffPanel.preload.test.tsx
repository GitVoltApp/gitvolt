import { render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { GraphPayload } from '../api/gen/GraphPayload';
import { createRepoViewStore, RepoViewContext, targetFor } from './store';
import { fakeServices } from './testServices';

const loadMonacoHost = vi.fn(async () => ({}));
vi.mock('../diff/monaco/load', () => ({ loadMonacoHost }));
vi.mock('../diff/DiffPanel', () => ({ DiffPanel: ({ target }: { target: { path: string } }) => <div data-testid="panel">{target.path}</div> }));
vi.mock('../conflicts/MergeTool', () => ({ MergeToolPanel: () => <div data-testid="merge" /> }));

const graph: GraphPayload = { rows: [], labels: [], maxLanes: 0, pinnedRefs: [], head: { branch: null, target: null, detached: false, unborn: true }, truncated: false, worktrees: [] };
const target = targetFor({ path: 'a.txt', oldPath: null, status: 'M', additions: 1, deletions: 0, old: { kind: 'absent' }, new: { kind: 'absent' }, submodule: false }, { kind: 'commit', id: 'c'.repeat(40), parent: 0 });

describe('preloadFirstDiff', () => {
  afterEach(() => vi.useRealTimers());

  it('loads Monaco and the panel once, in an idle moment; the panel then renders at once, with no fallback', async () => {
    vi.useFakeTimers();
    const { LazyDiffPanel, preloadFirstDiff } = await import('./LazyDiffPanel');
    // Never on its own in vitest: every FileList test would load the editor.
    preloadFirstDiff();
    await vi.runAllTimersAsync();
    expect(loadMonacoHost).not.toHaveBeenCalled();
    preloadFirstDiff(true);
    expect(loadMonacoHost).not.toHaveBeenCalled();
    await vi.runAllTimersAsync();
    expect(loadMonacoHost).toHaveBeenCalledTimes(1);
    preloadFirstDiff(true);
    await vi.runAllTimersAsync();
    expect(loadMonacoHost).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
    // No Suspense round (React holds a shown fallback 300 ms): the panel is there in the first render.
    render(<RepoViewContext value={createRepoViewStore(1, '/r', graph, fakeServices())}><LazyDiffPanel target={target} /></RepoViewContext>);
    expect(screen.getByTestId('panel')).toHaveTextContent('a.txt');
  });
});
