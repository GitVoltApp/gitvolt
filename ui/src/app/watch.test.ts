import { beforeEach, describe, expect, it, vi } from 'vitest';

const calls: string[] = [];
const api = vi.hoisted(() => ({
  watchRepo: vi.fn(async (r: number) => { await new Promise((res) => setTimeout(res, 5)); calls.push(`watch ${r}`); }),
  unwatchRepo: vi.fn(async (r: number) => { await new Promise((res) => setTimeout(res, 5)); calls.push(`unwatch ${r}`); }),
  holdRepo: vi.fn(async (t: string, r: number) => { calls.push(`hold ${t} ${r}`); }),
  unwatchAll: vi.fn(async () => { calls.push('unwatchAll'); }),
  releaseAll: vi.fn(async () => { calls.push('releaseAll'); }),
  releaseRepo: vi.fn(async (t: string) => { calls.push(`release ${t}`); }),
}));
vi.mock('../api/client', () => ({ api, errorMessage: (e: unknown) => String(e) }));

const { setWatched, holdRepo, releaseRepo, forgetPreviousPage } = await import('./watch');

describe('setWatched', () => {
  beforeEach(() => { calls.length = 0; });

  it('setWatched coalesces to the final desired state', async () => {
    // StrictMode: mount, unmount, mount, all before the first call finishes.
    setWatched(1, true);
    setWatched(1, false);
    setWatched(1, true);
    await vi.waitFor(() => expect(calls.at(-1)).toBe('watch 1'));
    await new Promise((r) => setTimeout(r, 30));
    expect(calls).toEqual(['watch 1']);
  });

  it('serializes per repo and ends unwatched when that is the last request', async () => {
    setWatched(2, true);
    await vi.waitFor(() => expect(calls).toEqual(['watch 2']));
    setWatched(2, false);
    await vi.waitFor(() => expect(calls).toEqual(['watch 2', 'unwatch 2']));
  });

  it('resolves once the latest wish is applied', async () => {
    const first = setWatched(5, true);
    const second = setWatched(5, false);
    expect(second).toBe(first);
    await second;
    expect(calls).toEqual(['watch 5', 'unwatch 5']);
  });

  it("a tab's watch holds the repo first (the core watches only a held repo), once per tab and repo", async () => {
    await setWatched(6, true, 'tab');
    await setWatched(6, false);
    await setWatched(6, true, 'tab');
    expect(calls).toEqual(['hold tab 6', 'watch 6', 'unwatch 6', 'watch 6']);
  });

  it("an open's hold and its tab's watch send one holdRepo; a release, a new repo or a reload sends it again", async () => {
    const opening = holdRepo('o', 8);
    const watching = setWatched(8, true, 'o');
    await opening;
    await watching;
    expect(calls).toEqual(['hold o 8', 'watch 8']);
    calls.length = 0;
    await holdRepo('o', 9);
    releaseRepo('o');
    await holdRepo('o', 9);
    await forgetPreviousPage();
    await holdRepo('o', 9);
    expect(calls).toEqual(['hold o 9', 'release o', 'hold o 9', 'unwatchAll', 'releaseAll', 'hold o 9']);
  });

  it('a watch the caller just held goes out at once, ahead of a request sent right after', async () => {
    api.watchRepo.mockClear();
    const run = setWatched(7, true, 'tab', true);
    expect(api.watchRepo).toHaveBeenCalledWith(7);
    await run;
    await setWatched(7, false);
    await setWatched(7, true, 'tab');
    expect(calls).toEqual(['watch 7', 'unwatch 7', 'hold tab 7', 'watch 7']);
  });

  it('a failed hold is retried once, then logged', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    api.holdRepo.mockClear();
    api.holdRepo.mockRejectedValueOnce(new Error('busy'));
    await holdRepo('t', 1);
    expect(api.holdRepo).toHaveBeenCalledTimes(2);
    expect(warn).not.toHaveBeenCalled();
    api.holdRepo.mockClear();
    api.holdRepo.mockRejectedValueOnce(new Error('busy')).mockRejectedValueOnce(new Error('gone'));
    await holdRepo('t', 2);
    expect(api.holdRepo).toHaveBeenCalledTimes(2);
    expect(warn).toHaveBeenCalledWith('[gitvolt] holdRepo failed', 'Error: gone');
    // Lost: the next ask (the tab's watch) sends it again.
    await holdRepo('t', 2);
    expect(api.holdRepo).toHaveBeenCalledTimes(3);
    warn.mockRestore();
  });

  it("a reloaded page forgets the previous page's watches and holds", async () => {
    await forgetPreviousPage();
    expect(calls).toEqual(['unwatchAll', 'releaseAll']);
  });

  it('a tab switch (unwatch one repo, watch another) reaches both', async () => {
    setWatched(3, true);
    await vi.waitFor(() => expect(calls).toEqual(['watch 3']));
    setWatched(3, false);
    setWatched(4, true);
    await vi.waitFor(() => expect([...calls].sort()).toEqual(['unwatch 3', 'watch 3', 'watch 4']));
  });
});
