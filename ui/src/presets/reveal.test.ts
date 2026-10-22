// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { GraphPayload } from '../api/gen/GraphPayload';
import type { RowPayload } from '../api/gen/RowPayload';
import type { ViewSpec } from '../api/gen/ViewSpec';

const sha = (n: number) => n.toString(16).padStart(40, '0');
const row = (id: string): RowPayload => ({
  id, kind: 'commit', lane: 0, color: 0, segments: [], summary: id.slice(-2), bodyFirstLine: '', authorName: 'Ada', authorEmail: 'ada@example.com', authorTime: 0, committerTime: 0, parents: [], mrRefs: [], wip: null,
});
// Loaded but not rows under the preset: a collapsed run's middle commit, a folded commit, a hidden
// branch's commit, each with what hides it. Expanding that key makes it a row.
const HIDDEN: Record<string, string> = { [sha(0xa1)]: 'branch:feature/payments', [sha(0xb1)]: `fold:${sha(0xb0)}`, [sha(0xc1)]: 'branch:spike/warp-gates' };
const OUTSIDE = sha(0xdd);

const impl = {
  graph: vi.fn(async (_repo: number, _limit: number, extra: { view?: ViewSpec }): Promise<GraphPayload> => {
    const expanded = extra.view?.expanded ?? [];
    const rows = [row(sha(1)), row(sha(2)), ...Object.entries(HIDDEN).filter(([, key]) => expanded.includes(key)).map(([id]) => row(id))];
    return { rows, labels: [], maxLanes: 1, pinnedRefs: [], head: { branch: 'refs/heads/main', target: sha(1), detached: false, unborn: false }, truncated: false, worktrees: [], ...(extra.view ? { view: { branches: [], trunk: ['main'] } } : {}) };
  }),
  graphHiddenOwner: vi.fn(async (_repo: number, _limit: number, _extra: unknown, _view: ViewSpec, id: string) => HIDDEN[id] ?? null),
  locateCommit: vi.fn(async () => ({ found: false, limit: null })),
};
// Everything else the tab's view store may ask for (details, file lists) never answers.
const api = new Proxy(impl as Record<string, unknown>, { get: (t, k: string) => t[k] ?? (() => new Promise(() => {})) });
vi.mock('../api/client', () => ({ api, errorMessage: (e: unknown) => String(e) }));

const { useRuntime } = await import('../app/runtime');
const { useAppState, EMPTY_PROFILE, DEFAULT_SETTINGS } = await import('../app/state');
const { tabStore } = await import('../app/tabStores');
const { goToCommit } = await import('../app/graphNav');
const { loadCommit } = await import('../find/findStore');
const { setPreset, usePresetSession, viewSpecFor } = await import('./store');
const { revealCommit } = await import('./reveal');

const tab = (id: number, path: string) => ({ status: 'ready' as const, error: null, repo: { id, path, name: 'r', worktree: path }, graph: null, info: null, sidebar: null, lastFetchAt: 0, fetchSkipped: null, limit: null, worktree: null });
const selected = () => {
  const s = tabStore('a')?.getState().selection;
  return s?.kind === 'commit' ? s.id : null;
};

describe('going to a commit the preset hides', () => {
  beforeEach(async () => {
    impl.graph.mockClear();
    impl.graphHiddenOwner.mockClear();
    useAppState.setState({ settings: DEFAULT_SETTINGS, profile: { ...EMPTY_PROFILE, repos: { '/r': { branchPreset: 'all-collapsed' } as never } } });
    useRuntime.setState({ tabs: { a: tab(1, '/r') } });
    usePresetSession.setState({ expanded: {} });
    await useRuntime.getState().refresh('a', { graphOnly: true });
  });

  it('navigating to a hidden commit expands its owner and selects it', async () => {
    // A collapsed run's middle commit, through go to commit (a parent link, a sidebar row).
    const missing = vi.fn();
    goToCommit('a', sha(0xa1), {}, missing);
    await vi.waitFor(() => expect(selected()).toBe(sha(0xa1)));
    expect(usePresetSession.getState().expanded.a).toEqual(['branch:feature/payments']);
    // A folded commit, through Find's and go-to's loader.
    expect(await loadCommit('a', sha(0xb1))).toBe(true);
    expect(selected()).toBe(sha(0xb1));
    // A hidden branch's commit.
    expect(await revealCommit('a', sha(0xc1))).toBe(true);
    expect(selected()).toBe(sha(0xc1));
    expect(usePresetSession.getState().expanded.a).toEqual(['branch:feature/payments', `fold:${sha(0xb0)}`, 'branch:spike/warp-gates']);
    // Each asked with the tab's view, and the graph reloaded with the key expanded.
    expect(impl.graphHiddenOwner.mock.calls.map((c) => c[4])).toEqual([sha(0xa1), sha(0xb1), sha(0xc1)]);
    expect(impl.graph.mock.calls.at(-1)![2].view?.expanded).toContain('branch:spike/warp-gates');
    expect(missing).not.toHaveBeenCalled();
  });

  it('a commit outside the window falls through to the caller', async () => {
    const missing = vi.fn();
    goToCommit('a', OUTSIDE, {}, missing);
    await vi.waitFor(() => expect(missing).toHaveBeenCalledTimes(1));
    expect(usePresetSession.getState().expanded.a).toBeUndefined();
    // Under All nothing is hidden: no request at all.
    setPreset('a', 'all');
    impl.graphHiddenOwner.mockClear();
    expect(await revealCommit('a', sha(0xa1))).toBe(false);
    expect(impl.graphHiddenOwner).not.toHaveBeenCalled();
  });

  it('Expand in graph unfolds that branch until the preset changes', async () => {
    const { expand } = await import('./store');
    await expand('a', 'branch:feature/payments');
    expect(viewSpecFor('a')?.expanded).toEqual(['branch:feature/payments']);
    expect(tabStore('a')!.getState().indexById.has(sha(0xa1))).toBe(true);
    setPreset('a', 'mine');
    expect(viewSpecFor('a')?.expanded).toEqual([]);
  });
});
