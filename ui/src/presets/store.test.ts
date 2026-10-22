// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ViewSpec } from '../api/gen/ViewSpec';

const api = vi.hoisted(() => ({
  graph: vi.fn(async () => ({ rows: [], labels: [], maxLanes: 1, pinnedRefs: [], head: { branch: null, target: null, detached: false, unborn: false }, truncated: false, worktrees: [] })),
  saveProfile: vi.fn(async () => null),
  saveSettings: vi.fn(async () => null),
}));
vi.mock('../api/client', () => ({ api, errorMessage: (e: unknown) => String(e) }));

const { useRuntime } = await import('../app/runtime');
const { useAppState, EMPTY_PROFILE, DEFAULT_SETTINGS } = await import('../app/state');
const { cyclePreset, expand, hideBranch, setPreset, usePresetSession, viewSpecFor } = await import('./store');

const tab = (id: number, path: string) => ({ status: 'ready' as const, error: null, repo: { id, path, name: 'r', worktree: path }, graph: null, info: null, sidebar: null, lastFetchAt: 0, fetchSkipped: null, limit: null, worktree: null });
const views = () => (api.graph.mock.calls as unknown as Array<[number, number, { view?: ViewSpec }]>).map(([repo, , extra]) => [repo, extra.view?.otherwise ?? null, extra.view?.expanded ?? null]);

describe('the preset store', () => {
  beforeEach(() => {
    api.graph.mockClear();
    useAppState.setState({ settings: DEFAULT_SETTINGS, profile: { ...EMPTY_PROFILE, repos: {} } });
    useRuntime.setState({ tabs: { a: tab(1, '/r'), b: tab(1, '/r'), c: tab(2, '/q') } });
    usePresetSession.setState({ expanded: {} });
  });

  it('switching presets saves the choice per repository and reloads the graph with its view', async () => {
    setPreset('a', 'mine');
    expect(useAppState.getState().profile.repos['/r'].branchPreset).toBe('mine');
    expect(useAppState.getState().profile.repos['/q']).toBeUndefined();
    // Both tabs on the repository reload; the other repository's doesn't.
    await vi.waitFor(() => expect(views()).toEqual([[1, 'hide', []], [1, 'hide', []]]));
    api.graph.mockClear();
    cyclePreset('a');
    expect(useAppState.getState().profile.repos['/r'].branchPreset).toBeUndefined(); // All again
    await vi.waitFor(() => expect(views()).toEqual([[1, null, null], [1, null, null]]));
  });

  it('expanded is per tab and cleared by a switch', async () => {
    setPreset('a', 'all-collapsed');
    expand('a', 'branch:feature/payments');
    expand('c', 'branch:other');
    expect(viewSpecFor('a')?.expanded).toEqual(['branch:feature/payments']);
    expect(viewSpecFor('b')?.expanded).toEqual([]);
    await vi.waitFor(() => expect(views()).toContainEqual([1, 'collapse', ['branch:feature/payments']]));
    setPreset('b', 'mine');
    expect(usePresetSession.getState().expanded).toEqual({ c: ['branch:other'] });
    expect(viewSpecFor('a')?.expanded).toEqual([]);
  });

  it('hiding a branch on a built-in switches to Custom, and unexpands it', () => {
    setPreset('a', 'all-collapsed');
    expand('a', 'branch:spike/x');
    hideBranch('a', 'spike/x');
    expect(useAppState.getState().profile.repos['/r'].branchPreset).toBe('custom');
    expect(viewSpecFor('a')).toMatchObject({ otherwise: 'collapse', picks: { hidden: ['spike/x'] }, expanded: [] });
  });
});
