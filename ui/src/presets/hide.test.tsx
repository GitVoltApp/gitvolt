import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { GraphPayload } from '../api/gen/GraphPayload';
import type { RowPayload } from '../api/gen/RowPayload';
import type { ViewPayload } from '../api/gen/ViewPayload';
import type { MenuRow } from '../menu/types';

const api = vi.hoisted(() => ({
  graph: vi.fn(async () => ({})),
  saveProfile: vi.fn(async () => null),
  saveSettings: vi.fn(async () => null),
}));
vi.mock('../api/client', () => ({ api, errorMessage: (e: unknown) => String(e) }));

const { useRuntime } = await import('../app/runtime');
const { useAppState, EMPTY_PROFILE, DEFAULT_SETTINGS } = await import('../app/state');
const { buildMenu } = await import('../menu/registry');
const { keyHints } = await import('../shortcuts/hints');
const { altClickBranch, unitNameOf } = await import('./altClick');
await import('./branchMenus');
const { GhostPopover } = await import('./GhostPopover');
import type { CommitTarget, MenuEnv } from '../menu/menuEnv';

const view = (hidden: string[] = [], over: Partial<ViewPayload> = {}): ViewPayload => ({
  branches: [
    { name: 'main', refs: ['refs/heads/main'], state: 'shown', by: { kind: 'trunk' } },
    { name: 'feature/a', refs: ['refs/heads/feature/a'], state: hidden.includes('feature/a') ? 'hidden' : 'shown', by: { kind: 'otherwise' } },
    { name: 'feature/b', refs: ['refs/remotes/origin/feature/b'], state: hidden.includes('feature/b') ? 'hidden' : 'shown', by: { kind: 'otherwise' } },
  ],
  trunk: ['main'],
  ...over,
});
const setView = (v: ViewPayload | undefined) => useRuntime.setState({ tabs: { t: { status: 'ready', error: null, repo: { id: 1, path: '/r', name: 'r', worktree: '/r' }, graph: { rows: [], head: { branch: 'refs/heads/main' }, ...(v && { view: v }) } as unknown as GraphPayload, info: null, sidebar: null, lastFetchAt: 0, fetchSkipped: null, limit: null, worktree: null } } });
const picks = () => useAppState.getState().profile.repos['/r']?.branchPicks ?? {};
const click = (altKey = true) => ({ altKey, preventDefault: vi.fn(), stopPropagation: vi.fn() });
const local = (n: string) => ({ local: `refs/heads/${n}`, remotes: [] });
const remote = (n: string) => ({ local: null, remotes: [{ fullName: `refs/remotes/origin/${n}`, remote: 'origin' }] });

const env = { write: { tabId: 't', repoId: 1, worktree: '/r' }, headBranch: 'main' } as unknown as MenuEnv;
const menu = (b: ReturnType<typeof local>) => buildMenu<CommitTarget, MenuEnv>('commit', { sha: 'a'.repeat(40), mrRefs: [], isWip: false, isStash: false, branch: { name: unitNameOf(b)!, ...b } }, env)
  .filter((r): r is Extract<MenuRow, { kind: 'action' }> => r.kind === 'action' && r.id.startsWith('presets.'));

beforeEach(() => {
  api.graph.mockClear();
  useAppState.setState({ settings: DEFAULT_SETTINGS, profile: { ...EMPTY_PROFILE, repos: {} } });
  setView(view());
});
afterEach(cleanup);

describe('hiding one branch', () => {
  it('alt+click on a chip hides its branch, both local and remote', () => {
    const e = click();
    expect(altClickBranch('t', local('feature/a'), e)).toBe(true);
    expect(e.preventDefault).toHaveBeenCalled();
    expect(picks().custom?.hidden).toEqual(['feature/a']);
    expect(altClickBranch('t', remote('feature/b'), click())).toBe(true);
    expect(picks().custom?.hidden).toEqual(['feature/a', 'feature/b']);
    // A plain click is not ours.
    expect(altClickBranch('t', local('feature/a'), click(false))).toBe(false);
  });

  it('alt+click on a hidden sidebar row shows it', () => {
    setView(view(['feature/a']));
    altClickBranch('t', local('feature/a'), click());
    expect(picks().custom?.shown).toEqual(['feature/a']);
    expect(picks().custom?.hidden ?? []).toEqual([]);
  });

  it('the trunk and the checked-out branch are not hidden', () => {
    altClickBranch('t', local('main'), click());
    expect(picks()).toEqual({});
  });

  it('the menus offer Hide or Show by state', () => {
    expect(menu(local('feature/a')).map((r) => [r.label, r.disabledReason])).toEqual([['Hide branch', undefined]]);
    setView(view(['feature/a']));
    expect(menu(local('feature/a')).map((r) => r.label)).toEqual(['Show branch']);
    menu(local('feature/a'))[0].run();
    expect(picks().custom?.shown).toEqual(['feature/a']);
    expect(menu(local('main'))[0].disabledReason).toBe('The trunk is always shown');
    setView(view([], { trunk: [] }));
    expect(menu(local('main'))[0].disabledReason).toBe('Checked out: always shown');
  });

  it('the hint is listed in the shortcuts panel', () => {
    expect(keyHints().find((h) => h.id === 'key.altclick.branch')).toMatchObject({ label: 'Hide or show a branch', context: 'branch chip or sidebar branch' });
  });
});

describe('the ghost popover', () => {
  const row = { id: 'a'.repeat(40), lane: 0, ghosts: [
    { branch: 'spike/x', color: 1, tip: 't1', summary: 'Try a thing', authorName: 'Ada', authorTime: 1 },
    { branch: 'spike/y', color: 2, tip: 't2', summary: 'Try another', authorName: 'Bo', authorTime: 2 },
  ] } as unknown as RowPayload;
  const open = () => render(<GhostPopover tabId="t" row={row} at={new DOMRect(10, 10, 8, 8)} focus={false} onClose={vi.fn()} />);

  it('hovering ghost lines lists the hidden branches', () => {
    open();
    expect(screen.getByRole('dialog', { name: '2 hidden branches start here' })).toBeTruthy();
    expect(screen.getByText('spike/x')).toBeTruthy();
    expect(screen.getByText('Try another')).toBeTruthy();
    expect(screen.getByText(/Ada/)).toBeTruthy();
    expect(screen.getByText('Hidden by “All”')).toBeTruthy();
  });

  it('Show adds one branch to the shown picks, Show all adds all', () => {
    const { unmount } = open();
    act(() => fireEvent.click(screen.getByRole('button', { name: 'Show spike/y' })));
    expect(picks().custom?.shown).toEqual(['spike/y']);
    unmount();
    open();
    act(() => fireEvent.click(screen.getByRole('button', { name: 'Show all' })));
    expect(picks().custom?.shown).toEqual(['spike/x', 'spike/y']);
  });
});
