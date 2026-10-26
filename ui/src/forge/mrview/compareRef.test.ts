import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createStore } from 'zustand/vanilla';

const api = vi.hoisted(() => ({ dropCompareRefs: vi.fn(async () => null) }));
vi.mock('../../api/client', () => ({ api, errorMessage: String }));
type Sel = { kind: 'commit'; id: string } | { kind: 'compare'; from: string; to: string };
const graph = vi.hoisted(() => ({ store: null as unknown as import('zustand/vanilla').StoreApi<{ selection: Sel }> }));
vi.mock('../../app/tabStores', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../app/tabStores')>()),
  tabStore: () => graph.store,
}));

const { holdCompareRef, releaseCompareRef } = await import('./compareRef');
const { patchForge, useForge } = await import('../mrStore');

const BASE = 'b'.repeat(40);
const HEAD = 'h'.repeat(40);
const OTHER = 'o'.repeat(40);
const select = (selection: Sel) => graph.store.setState({ selection });

beforeEach(() => {
  releaseCompareRef('t');
  vi.clearAllMocks();
  graph.store = createStore(() => ({ selection: { kind: 'commit', id: BASE } as Sel }));
  useForge.setState({ byTab: {} });
});

describe("a Compare's fetched MR head", () => {
  it('stays while its compare shows, either way round, and goes when the tab leaves it', () => {
    holdCompareRef('t', 4, 12, HEAD);
    // The fetch, then the graph's refreshes: no compare shown yet, nothing ends it.
    select({ kind: 'commit', id: BASE });
    select({ kind: 'compare', from: BASE, to: HEAD });
    select({ kind: 'compare', from: HEAD, to: BASE });
    expect(api.dropCompareRefs).not.toHaveBeenCalled();
    select({ kind: 'commit', id: OTHER });
    expect(api.dropCompareRefs).toHaveBeenCalledExactlyOnceWith(4);
    select({ kind: 'compare', from: BASE, to: HEAD });
    expect(api.dropCompareRefs).toHaveBeenCalledOnce();
  });

  it('goes when another compare replaces it', () => {
    holdCompareRef('t', 4, 12, HEAD);
    select({ kind: 'compare', from: BASE, to: HEAD });
    select({ kind: 'compare', from: BASE, to: OTHER });
    expect(api.dropCompareRefs).toHaveBeenCalledExactlyOnceWith(4);
  });

  it("another MR's fetched head takes over without a drop: that fetch replaced the ref", () => {
    holdCompareRef('t', 4, 12, HEAD);
    select({ kind: 'compare', from: BASE, to: HEAD });
    holdCompareRef('t', 4, 13, OTHER);
    select({ kind: 'compare', from: BASE, to: OTHER });
    expect(api.dropCompareRefs).not.toHaveBeenCalled();
    select({ kind: 'commit', id: BASE });
    expect(api.dropCompareRefs).toHaveBeenCalledExactlyOnceWith(4);
  });

  it('goes when the MR view it was compared from closes', () => {
    patchForge('t', { openMr: 12 });
    holdCompareRef('t', 4, 12, HEAD);
    select({ kind: 'compare', from: BASE, to: HEAD });
    patchForge('t', { openMr: 12 });
    expect(api.dropCompareRefs).not.toHaveBeenCalled();
    patchForge('t', { openMr: null });
    expect(api.dropCompareRefs).toHaveBeenCalledExactlyOnceWith(4);
  });

  it("a Compare that fetched it and then failed drops it; one already shown isn't a failed one's", () => {
    holdCompareRef('t', 4, 12, HEAD);
    releaseCompareRef('t', OTHER);
    expect(api.dropCompareRefs).not.toHaveBeenCalled();
    select({ kind: 'compare', from: BASE, to: HEAD });
    releaseCompareRef('t', HEAD);
    expect(api.dropCompareRefs).not.toHaveBeenCalled();
    // The tab closes (`useRuntime.drop`).
    releaseCompareRef('t');
    expect(api.dropCompareRefs).toHaveBeenCalledExactlyOnceWith(4);
    select({ kind: 'commit', id: BASE });
    holdCompareRef('t', 4, 12, HEAD);
    releaseCompareRef('t', HEAD);
    expect(api.dropCompareRefs).toHaveBeenCalledTimes(2);
  });
});
