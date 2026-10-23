import { afterEach, describe, expect, it, vi } from 'vitest';
import { countScopes } from './counts';
import type { FileScope } from './store';

const sc = (name: string, patterns: string[]): FileScope => ({ id: name, name, tint: 'green', patterns, startCollapsed: false });

afterEach(() => vi.useRealTimers());

describe('countScopes', () => {
  it("counts each scope's paths by first match", async () => {
    const scopes = [sc('Rs', ['*.rs']), sc('Src', ['src/**'])];
    expect(await countScopes(scopes, ['a.rs', 'src/b.rs', 'src/c.ts', 'd.md'])).toEqual([2, 1]);
  });
  it('yields between chunks', async () => {
    vi.useFakeTimers();
    const paths = Array.from({ length: 12_000 }, (_, i) => `f${i}.rs`);
    let done = false;
    const p = countScopes([sc('Rs', ['*.rs'])], paths).then((r) => { done = true; return r; });
    await Promise.resolve();
    expect(done).toBe(false);
    await vi.runAllTimersAsync();
    expect(await p).toEqual([12_000]);
    expect(vi.getTimerCount()).toBe(0);
  });
  it('stops when aborted', async () => {
    vi.useFakeTimers();
    const ctl = new AbortController();
    const paths = Array.from({ length: 12_000 }, (_, i) => `f${i}.rs`);
    const p = countScopes([sc('Rs', ['*.rs'])], paths, ctl.signal);
    const assertion = expect(p).rejects.toMatchObject({ name: 'AbortError' });
    ctl.abort();
    await vi.runAllTimersAsync();
    await assertion;
  });
});
