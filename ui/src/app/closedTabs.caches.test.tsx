import { renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
  blame: vi.fn(async () => ({ lines: [], rows: [] })),
  releaseRepo: vi.fn(async () => null),
}));
vi.mock('../api/client', () => ({ api: new Proxy(api, { get: (t, k: string) => (t as Record<string, unknown>)[k] ?? (async () => null) }), errorMessage: String, onEvent: () => () => {} }));
const listTreeFiles = vi.hoisted(() => vi.fn(async () => ['a.txt', 'src/b.ts']));
vi.mock('./seams1b', async (orig) => ({ ...(await orig<object>()), listTreeFiles }));

const { useRuntime } = await import('./runtime');
const { fileEntries } = await import('../palette/sources');
const { peopleSource } = await import('../forge/pickerCache');
const { useBlame } = await import('../history/BlameGutter');
const { openFind, useFind } = await import('../find/findStore');
const { seedReveal, tabReveal } = await import('../graph/reveal/revealStore');

const open = (tabId: string, repo: number) => useRuntime.getState().patch(tabId, {
  status: 'ready',
  repo: { id: repo, path: `/r${repo}`, name: `r${repo}`, worktree: `/r${repo}` } as never,
  graph: { head: { target: 'h'.repeat(40) } } as never,
});

describe('closing tabs lets go of what other modules cached for them', () => {
  it("a tab's find and reveal state go with it; a repository's blame, file list and picker answers with its last tab", async () => {
    open('t1', 7);
    open('t2', 7); // the same repository
    open('t3', 8);
    const people = (repo: number) => peopleSource(repo, 'origin', async () => [{ name: 'ann' }]);
    await people(7).load('');
    await people(8).load('');
    await fileEntries('t1');
    const blame = renderHook(() => useBlame(7, '/r7', 'HEAD', 'a.txt'));
    await waitFor(() => expect(blame.result.current.status).toBe('ready'));
    blame.unmount();
    openFind('t1');
    seedReveal('t1', true, { key: 'fold:x' } as never);
    expect(useFind.getState().byTab.t1).toBeDefined();
    expect(tabReveal('t1').cache.size).toBe(1);

    useRuntime.getState().drop('t1');
    expect(useFind.getState().byTab.t1).toBeUndefined();
    expect(tabReveal('t1').cache.size).toBe(0);
    // t2 still shows repository 7: its caches stay.
    await fileEntries('t2');
    expect(listTreeFiles).toHaveBeenCalledOnce();
    expect(people(7).peek('')).toBeDefined();

    useRuntime.getState().drop('t2');
    expect(people(7).peek('')).toBeUndefined();
    expect(people(8).peek('')).toBeDefined(); // another repository's
    open('t4', 7);
    await fileEntries('t4');
    expect(listTreeFiles).toHaveBeenCalledTimes(2);
    const again = renderHook(() => useBlame(7, '/r7', 'HEAD', 'a.txt'));
    await waitFor(() => expect(again.result.current.status).toBe('ready'));
    expect(api.blame).toHaveBeenCalledTimes(2);
    again.unmount();
    for (const t of ['t3', 't4']) useRuntime.getState().drop(t);
  });

  it('a picker answer still on its way when its repository closes is not kept', async () => {
    open('t5', 9);
    let answer!: (v: Array<{ name: string }>) => void;
    const src = peopleSource(9, 'origin', () => new Promise<Array<{ name: string }>>((r) => { answer = r; }));
    const load = src.load('');
    useRuntime.getState().drop('t5');
    answer([{ name: 'ann' }]);
    expect(await load).toEqual([{ name: 'ann' }]);
    expect(src.peek('')).toBeUndefined();
  });
});
