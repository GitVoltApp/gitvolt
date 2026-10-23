// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { FileScope } from '../api/gen/FileScope';
import type { RepoInfoPayload } from '../api/gen/RepoInfoPayload';

vi.mock('../api/client', () => ({ api: { saveProfile: vi.fn(async () => null) }, errorMessage: (e: unknown) => String(e) }));

const { useAppState, EMPTY_PROFILE } = await import('../app/state');
const { useRuntime } = await import('../app/runtime');
const { NO_SCOPES, scopesOf, setScopes, commonDirOfTab } = await import('./store');

const scope = (id: string): FileScope => ({ id, name: id, tint: 'green', patterns: ['src/**'], startCollapsed: false });
const info = (commonDir: string) => ({ commonDir }) as unknown as RepoInfoPayload;
const profile = () => useAppState.getState().profile;

describe('file colours store', () => {
  beforeEach(() => {
    useAppState.setState({ profile: EMPTY_PROFILE });
    useRuntime.setState({ tabs: {} });
  });

  it('scopesOf returns NO_SCOPES for a null or unknown common dir', () => {
    expect(scopesOf(profile(), null)).toBe(NO_SCOPES);
    expect(scopesOf(profile(), '/nope/.git')).toBe(NO_SCOPES);
  });

  it("setScopes writes the repository's list, and an empty list removes its key", () => {
    setScopes('/r/.git', [scope('a')]);
    expect(scopesOf(profile(), '/r/.git')).toEqual([scope('a')]);
    setScopes('/r/.git', []);
    expect(Object.keys(profile().fileColours)).toEqual([]);
  });

  it('two tabs on different worktrees of one repository read the same scopes', () => {
    useRuntime.setState({ tabs: { t1: { info: info('/r/.git') }, t2: { info: info('/r/.git') } } as never });
    setScopes(commonDirOfTab('t1')!, [scope('a')]);
    expect(scopesOf(profile(), commonDirOfTab('t2'))).toEqual([scope('a')]);
  });

  it("a tab whose repo info hasn't loaded has no common dir and no scopes", () => {
    useRuntime.setState({ tabs: { t1: { info: null } } as never });
    expect(commonDirOfTab('t1')).toBeNull();
    expect(commonDirOfTab('missing')).toBeNull();
    expect(commonDirOfTab(null)).toBeNull();
    expect(scopesOf(profile(), commonDirOfTab('t1'))).toBe(NO_SCOPES);
  });

  it("scopesOf returns the same array until that repository's scopes change", () => {
    setScopes('/r/.git', [scope('a')]);
    const first = scopesOf(profile(), '/r/.git');
    setScopes('/s/.git', [scope('b')]);
    expect(scopesOf(profile(), '/r/.git')).toBe(first);
    setScopes('/r/.git', [scope('c')]);
    expect(scopesOf(profile(), '/r/.git')).not.toBe(first);
  });
});
