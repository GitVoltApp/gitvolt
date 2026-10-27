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
const { useAppState, EMPTY_PROFILE, DEFAULT_SETTINGS } =await import('../app/state');
const { buildMenu } = await import('../menu/registry');
const { keyHints } = await import('../shortcuts/hints');
const { altClickBranch, altClickChip, altClickFolder, altClickTag, toggleTag, unitNameOf } = await import('./altClick');
const { usePresetSession } = await import('./store');
await import('./branchMenus');
const { GhostPopover } = await import('./GhostPopover');
const { useTheme } = await import('../theme/store');
import type { CommitTarget, MenuEnv, SidebarTarget, TagTarget } from '../menu/menuEnv';

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
  usePresetSession.setState({ expanded: {} });
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

  it('under All (no view) the pinned trunk is not hidden either, and the preset stays All', () => {
    setView(undefined);
    useRuntime.setState((s) => ({ tabs: { t: { ...s.tabs.t, graph: { ...s.tabs.t.graph!, pinnedRefs: ['refs/heads/dev', 'refs/remotes/origin/dev'] } } } }));
    altClickBranch('t', local('dev'), click());
    altClickBranch('t', remote('dev'), click());
    expect(useAppState.getState().profile.repos['/r']).toBeUndefined();
    expect(menu(local('dev')).map((r) => [r.label, r.disabledReason])).toEqual([['Hide branch', 'The trunk is always shown']]);
    // Only the unit itself: not a name it ends.
    expect(menu(local('feature/dev'))[0].disabledReason).toBeUndefined();
    altClickBranch('t', local('feature/a'), click());
    expect(picks().custom?.hidden).toEqual(['feature/a']);
  });

  it("a branch checked out in another worktree hides by hand, though the preset's rules keep it", () => {
    const v = view();
    setView({ ...v, branches: [...v.branches, { name: 'wt/dock', refs: ['refs/heads/wt/dock'], state: 'shown', by: { kind: 'checkedOut' } }] });
    expect(menu(local('wt/dock')).map((r) => [r.label, r.disabledReason])).toEqual([['Hide branch', undefined]]);
    altClickBranch('t', local('wt/dock'), click());
    expect(picks().custom?.hidden).toEqual(['wt/dock']);
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

  it('a branch expanded this session offers Collapse branch, which folds it again', () => {
    expect(menu(local('feature/a')).map((r) => r.label)).toEqual(['Hide branch']);
    usePresetSession.setState({ expanded: { t: ['branch:feature/a'], other: ['branch:feature/b'] } });
    expect(menu(local('feature/a')).map((r) => r.label)).toEqual(['Hide branch', 'Collapse branch']);
    // Another tab's expansion isn't this one's.
    expect(menu(local('feature/b')).map((r) => r.label)).toEqual(['Hide branch']);
    menu(local('feature/a'))[1].run();
    expect(usePresetSession.getState().expanded.t).toEqual([]);
    expect(api.graph).toHaveBeenCalled();
  });

  it("a merge whose fold is expanded offers Fold merged branch in its menu", () => {
    const merge = 'a'.repeat(40);
    const rows = () => buildMenu<CommitTarget, MenuEnv>('commit', { sha: merge, mrRefs: [], isWip: false, isStash: false, branch: null }, env)
      .filter((r): r is Extract<MenuRow, { kind: 'action' }> => r.kind === 'action' && r.id.startsWith('presets.'));
    expect(rows()).toEqual([]);
    usePresetSession.setState({ expanded: { t: [`fold:${merge}`] } });
    expect(rows().map((r) => r.label)).toEqual(['Fold merged branch']);
    rows()[0].run();
    expect(usePresetSession.getState().expanded.t).toEqual([]);
  });

  it('the hint is listed in the shortcuts panel', () => {
    expect(keyHints().find((h) => h.id === 'key.altclick.branch')).toMatchObject({ label: 'Hide or show a branch, tag or folder', context: 'branch or tag chip, sidebar branch, tag or folder' });
  });
});

describe('hiding a folder', () => {
  const folderMenu = (folder: string, hidden: boolean) => buildMenu<SidebarTarget, MenuEnv>('sidebar', { what: 'folder', folder, hidden }, env)
    .filter((r): r is Extract<MenuRow, { kind: 'action' }> => r.kind === 'action' && r.id.startsWith('presets.'));

  it('alt+click on a shown folder hides it; on a hidden one, shows it again', () => {
    altClickBranch('t', local('feature/a'), click());
    expect(picks().custom).toEqual({ shown: [], hidden: ['feature/a'] });
    const e = click();
    expect(altClickFolder('t', 'feature/', false, e)).toBe(true);
    expect(e.stopPropagation).toHaveBeenCalled();
    expect(picks().custom).toEqual({ shown: [], hidden: ['feature/'] });
    expect(altClickFolder('t', 'feature/', true, click())).toBe(true);
    expect(picks().custom).toEqual({ shown: [], hidden: [] });
    expect(altClickFolder('t', 'feature/', true, click(false))).toBe(false);
  });

  it('hiding drops the expansions under it', () => {
    // Already in Custom: switching presets would drop every expansion.
    altClickBranch('t', local('feature/b'), click());
    usePresetSession.setState({ expanded: { t: ['branch:feature/a', 'branch:main', 'fold:abc'] } });
    altClickFolder('t', 'feature/', false, click());
    expect(usePresetSession.getState().expanded.t).toEqual(['branch:main', 'fold:abc']);
  });

  it('its menu offers Hide folder or Show folder by state', () => {
    expect(folderMenu('feature/', false).map((r) => r.label)).toEqual(['Hide folder']);
    folderMenu('feature/', false)[0].run();
    expect(picks().custom?.hidden).toEqual(['feature/']);
    expect(folderMenu('feature/', true).map((r) => r.label)).toEqual(['Show folder']);
    folderMenu('feature/', true)[0].run();
    expect(picks().custom?.hidden).toEqual([]);
    // No row in a remote's menu.
    expect(buildMenu<SidebarTarget, MenuEnv>('sidebar', { what: 'remote', name: 'origin', url: null }, env).some((r) => r.kind === 'action' && r.id.startsWith('presets.'))).toBe(false);
  });
});

describe('hiding a tag or a tag folder', () => {
  const chip = (name: string, tag = true) => ({ name, tag, local: tag ? null : `refs/heads/${name}`, remotes: [] });
  const tagMenu = (name: string) => buildMenu<TagTarget, MenuEnv>('tag', { name, fullName: `refs/tags/${name}`, sha: 'a'.repeat(40) }, env)
    .filter((r): r is Extract<MenuRow, { kind: 'action' }> => r.kind === 'action' && r.id.startsWith('presets.'));
  const folderMenu = (folder: string, hidden: boolean) => buildMenu<SidebarTarget, MenuEnv>('sidebar', { what: 'folder', folder, hidden, tags: true }, env)
    .filter((r): r is Extract<MenuRow, { kind: 'action' }> => r.kind === 'action' && r.id.startsWith('presets.'));

  it("alt+click on a tag chip hides the tag in Custom, not a branch of the same name; on a hidden one, shows it", () => {
    const e = click();
    expect(altClickChip('t', chip('v1'), e)).toBe(true);
    expect(e.preventDefault).toHaveBeenCalled();
    expect(picks().custom).toEqual({ shown: [], hidden: [], hiddenTags: ['v1'] });
    altClickChip('t', chip('v1', false), click());
    expect(picks().custom).toEqual({ shown: [], hidden: ['v1'], hiddenTags: ['v1'] });
    expect(altClickChip('t', chip('v2'), click(false))).toBe(false);
    setView(view([], { hiddenTags: ['v1'] }));
    expect(altClickTag('t', 'v1', click())).toBe(true);
    expect(picks().custom).toEqual({ shown: [], hidden: ['v1'], shownTags: ['v1'] });
  });

  it("nothing keeps a tag: the trunk's or the checked-out commit's hides too (only its chip goes)", () => {
    toggleTag('t', 'release-main');
    expect(picks().custom?.hiddenTags).toEqual(['release-main']);
  });

  it('the menus offer Hide tag or Show tag by state', () => {
    expect(tagMenu('v1').map((r) => r.label)).toEqual(['Hide tag']);
    tagMenu('v1')[0].run();
    expect(picks().custom?.hiddenTags).toEqual(['v1']);
    setView(view([], { hiddenTags: ['v1'] }));
    expect(tagMenu('v1').map((r) => r.label)).toEqual(['Show tag']);
    tagMenu('v1')[0].run();
    expect(picks().custom?.shownTags).toEqual(['v1']);
  });

  it("a tag folder hides and shows by alt+click and its menu, and hiding drops its tags' expansions", () => {
    // Already in Custom: switching presets would drop every expansion.
    altClickBranch('t', local('feature/a'), click());
    usePresetSession.setState({ expanded: { t: ['tag:archive/old', 'tag:v1', 'branch:archive/x'] } });
    expect(altClickFolder('t', 'archive/', false, click(), true)).toBe(true);
    expect(picks().custom).toEqual({ shown: [], hidden: ['feature/a'], hiddenTags: ['archive/'] });
    expect(usePresetSession.getState().expanded.t).toEqual(['tag:v1', 'branch:archive/x']);
    expect(folderMenu('archive/', true).map((r) => [r.label, r.tooltip])).toEqual([['Show folder', 'Show the tags under archive/ in the graph again']]);
    folderMenu('archive/', true)[0].run();
    expect(picks().custom).toEqual({ shown: [], hidden: ['feature/a'] });
    expect(folderMenu('archive/', false).map((r) => [r.label, r.tooltip])).toEqual([['Hide folder', 'Hide every tag under archive/ from the graph, also those created later']]);
    folderMenu('archive/', false)[0].run();
    expect(picks().custom?.hiddenTags).toEqual(['archive/']);
  });
});

describe("the hidden-branch stub's popover", () => {
  // Every hidden branch of a row comes in its stub's lane colour (the core's).
  const row = { id: 'a'.repeat(40), lane: 0, stubLane: 2, ghosts: [
    { branch: 'spike/x', color: 2, tip: 't1', summary: 'Try a thing', authorName: 'Ada', authorTime: 1 },
    { branch: 'spike/y', color: 2, tip: 't2', summary: 'Try another', authorName: 'Bo', authorTime: 2 },
  ] } as unknown as RowPayload;
  const open = () => render(<GhostPopover tabId="t" row={row} at={new DOMRect(10, 10, 8, 8)} focus={false} onClose={vi.fn()} />);

  it("hovering a stub lists the hidden branches, their chips in the stub's colour", () => {
    open();
    expect(screen.getByRole('dialog', { name: '2 hidden branches start here' })).toBeTruthy();
    const chips = ['spike/x', 'spike/y'].map((b) => screen.getByText(b).style.getPropertyValue('--lane-color'));
    expect(chips[0]).toBe(useTheme.getState().colors.graph[2]);
    expect(chips[1]).toBe(chips[0]);
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

  it('lists a tag hidden by hand with a tag icon; Show and Show all show it as a tag', () => {
    const mixed = { ...row, ghosts: [
      row.ghosts![0],
      { branch: 'archive/old', tag: true, color: 2, tip: 't3', summary: 'Old spike', authorName: 'Cy', authorTime: 3 },
    ] } as RowPayload;
    const openMixed = () => render(<GhostPopover tabId="t" row={mixed} at={new DOMRect(10, 10, 8, 8)} focus={false} onClose={vi.fn()} />);
    const { unmount } = openMixed();
    expect(screen.getByRole('dialog', { name: '2 hidden branches and tags start here' })).toBeTruthy();
    expect(screen.getByText('archive/old').querySelector('[aria-label="tag"]')).toBeTruthy();
    expect(screen.getByText('spike/x').querySelector('[aria-label="tag"]')).toBeNull();
    expect(screen.getByText('Old spike')).toBeTruthy();
    expect(screen.getByText(/Cy/)).toBeTruthy();
    act(() => fireEvent.click(screen.getByRole('button', { name: 'Show tag archive/old' })));
    expect(picks().custom?.shownTags).toEqual(['archive/old']);
    expect(picks().custom?.shown).toEqual([]);
    unmount();
    openMixed();
    act(() => fireEvent.click(screen.getByRole('button', { name: 'Show all' })));
    expect(picks().custom?.shown).toEqual(['spike/x']);
    expect(picks().custom?.shownTags).toEqual(['archive/old']);
  });
});
