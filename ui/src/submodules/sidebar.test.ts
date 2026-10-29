import { beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api/client';
import type { SidebarPayload } from '../api/gen/SidebarPayload';
import type { SubmoduleItem } from '../api/gen/SubmoduleItem';
import { getAction } from '../app/actions';
import * as runtime from '../app/runtime';
import { useRuntime } from '../app/runtime';
import { EMPTY_PROFILE, useAppState } from '../app/state';
import type { MenuEnv } from '../menu/menuEnv';
import type { RepoViewStore } from '../repo/store';
import { sidebarDoubleClick } from '../sidebar/itemActions';
import { buildPanels, sectionsOf } from '../sidebar/model';
import * as confirmDialog from '../ui/ConfirmDialog';
import { useToast } from '../ui/toastStore';
import { defaultSubmodulePath, submodulePathError } from './AddSubmoduleDialog';
import { addSubmodule, removeSubmodule } from './commands';
import { sidebarSubmoduleRows } from './feature';
import { submoduleCardState } from './labels';

const res = (outcome: unknown) => ({ outcome, journal: { undo: null, redo: null, undoBlocked: null, redoBlocked: null, banners: [] }, staging: { undo: null, redo: null, off: null }, wip: null });
const A = 'a'.repeat(40);
const B = 'b'.repeat(40);
const sub = (path: string, over: Partial<SubmoduleItem> = {}): SubmoduleItem => ({ path, name: path, url: `https://git.example.com/${path}.git`, recorded: A, head: A, branch: null, status: 'current', ...over });
const payload = (submodules: SidebarPayload['submodules']): SidebarPayload => ({ locals: [], remotes: [], worktrees: [], stashes: [], tags: [], submodules });
const write = { tabId: 't', repoId: 1, worktree: '/r' };
const env = (over: Partial<MenuEnv> = {}) => ({ write, inProgress: null, ...over }) as MenuEnv;
const labels = (rows: ReturnType<typeof sidebarSubmoduleRows>) => rows.map((r) => (r.kind === 'action' ? [r.label, r.disabledReason ?? null] : r.kind));

function repoTab(uses = true) {
  useAppState.setState({ loaded: true, profile: { ...EMPTY_PROFILE, id: 'default', tabs: [{ id: 't', kind: 'repo', path: '/r', alias: null }], activeTab: 't' } });
  useRuntime.getState().patch('t', { repo: { id: 1, path: '/r', name: 'r', worktree: '/r' }, worktree: '/r', info: { remotes: [], mainWorktree: null, commonDir: '/r/.git', submodules: uses ? { '/r': { unmapped: [] } } : {} } });
}

describe('the Submodules panel', () => {
  beforeEach(() => { vi.restoreAllMocks(); useToast.getState().dismiss(); });

  it('shows, last, the active worktree\'s submodules by path, and only when it has any', () => {
    const p = payload({ '/r': [sub('vendor/parser'), sub('vendor/theme', { head: null, status: 'uninitialized' })], '/r-linked': [sub('other')] });
    const panels = buildPanels(sectionsOf(p, null, null, '/r'), { filter: '', sort: {}, collapsed: new Set() });
    expect(panels.map((x) => x.section.label)).toEqual(['Local', 'Remote', 'Worktrees', 'Stashes', 'Tags', 'Submodules']);
    expect(panels[5].rows.map((r) => (r.type === 'item' ? r.label : r.name))).toEqual(['vendor/parser', 'vendor/theme']);
    expect(sectionsOf(p, null, null, '/r-linked').at(-1)?.items.map((i) => i.name)).toEqual(['other']);
    // A worktree without any (or none known yet): no panel at all.
    expect(sectionsOf(p, null, null, '/r-plain').map((s) => s.id)).not.toContain('submodules');
    expect(sectionsOf(p).map((s) => s.id)).not.toContain('submodules');
    // The filter matches a path.
    const filtered = buildPanels(sectionsOf(p, null, null, '/r'), { filter: 'theme', sort: {}, collapsed: new Set() });
    expect(filtered[5].matched).toBe(1);
  });

  it("a row's menu: Open in a tab, Update; one not initialised offers Initialise first, its Open greyed", () => {
    repoTab();
    expect(labels(sidebarSubmoduleRows({ what: 'submodule', submodule: sub('vendor/parser') }, env()))).toEqual([['Open in a tab', null], ['Update', null]]);
    expect(labels(sidebarSubmoduleRows({ what: 'submodule', submodule: sub('vendor/theme', { head: null, status: 'uninitialized' }) }, env()))).toEqual([
      ['Initialise', null], ['Open in a tab', 'Not initialised: Initialise checks it out'],
    ]);
    // Mid-merge, Update waits as the file list's row does; other targets get nothing.
    expect(labels(sidebarSubmoduleRows({ what: 'submodule', submodule: sub('vendor/parser') }, env({ inProgress: 'merge' })))[1]).toEqual(['Update', 'Finish or abort the merge first']);
    expect(sidebarSubmoduleRows({ what: 'ref', sha: A }, env())).toEqual([]);
  });

  it('a double-click opens a submodule as a tab, and initialises one not checked out', async () => {
    repoTab();
    const store = {} as RepoViewStore;
    vi.spyOn(api, 'peekRepo').mockResolvedValue({ remotes: [], branch: 'main' });
    const open = vi.spyOn(runtime, 'openPathInTab').mockResolvedValue();
    const update = vi.spyOn(api, 'submoduleUpdate').mockResolvedValue(res({ command: 'git submodule update --init --recursive -- vendor/theme' }) as never);
    const item = (m: SubmoduleItem) => ({ key: `submodule:${m.path}`, kind: 'submodule' as const, name: m.path, target: null, time: 0, submodule: m });
    expect(sidebarDoubleClick({ tabId: 't', store }, item(sub('vendor/parser', { head: B, status: 'newCommits' })))).toBe(true);
    await vi.waitFor(() => expect(open).toHaveBeenCalledWith('/r/vendor/parser'));
    sidebarDoubleClick({ tabId: 't', store }, item(sub('vendor/theme', { head: null, status: 'uninitialized' })));
    await vi.waitFor(() => expect(update).toHaveBeenCalledWith(1, '/r', 'vendor/theme', false));
    expect(open).toHaveBeenCalledTimes(1);
  });

  it('Remove arms in place, in red, then removes; a declined one sends nothing', async () => {
    repoTab();
    const ask = vi.spyOn(confirmDialog, 'confirmAction').mockResolvedValueOnce(false).mockResolvedValueOnce(true);
    const remove = vi.spyOn(api, 'submoduleRemove').mockResolvedValue(res({ command: 'git rm -- vendor/parser' }) as never);
    await removeSubmodule(write, 'vendor/parser');
    expect(remove).not.toHaveBeenCalled();
    await removeSubmodule(write, 'vendor/parser');
    expect(ask.mock.calls[0][0]).toMatchObject({ arm: 'Click again to remove vendor/parser', danger: true });
    expect(remove).toHaveBeenCalledWith(1, '/r', 'vendor/parser');
    await vi.waitFor(() => expect(useToast.getState().message).toBe('Removed vendor/parser: commit to record it'));
    expect(useToast.getState().detail).toBe('git rm -- vendor/parser');
  });

  it('Add submodule: the panel\'s +, and the palette in any repository tab, so the first one can be added', async () => {
    repoTab(false);
    expect(getAction('submodule.add')?.when?.()).toBe(true);
    expect(getAction('submodule.update')?.when?.()).toBe(false);
    const add = vi.spyOn(api, 'submoduleAdd').mockResolvedValue(res({ command: 'git submodule add -b main -- https://git.example.com/lib.git vendor/lib' }) as never);
    await addSubmodule(write, 'https://git.example.com/lib.git', 'vendor/lib', 'main');
    expect(add).toHaveBeenCalledWith(1, '/r', 'https://git.example.com/lib.git', 'vendor/lib', 'main');
    await vi.waitFor(() => expect(useToast.getState().message).toBe('Added the submodule vendor/lib: commit to record it'));
  });

  it("the dialog's path follows the URL as git's default does, and refuses one outside the repository", () => {
    expect(defaultSubmodulePath('https://git.example.com/group/library.git')).toBe('library');
    expect(defaultSubmodulePath('git@git.example.com:group/library.git/')).toBe('library');
    expect(defaultSubmodulePath('../theme-src')).toBe('theme-src');
    expect(submodulePathError('vendor/library')).toBeUndefined();
    expect(submodulePathError('')).toBe('Enter a path for the submodule');
    expect(submodulePathError('/abs/x')).toBe('A path inside the repository, not an absolute one');
    expect(submodulePathError('vendor/../../x')).toBe('A path inside the repository');
  });

  it("the hover card's last line says what the state means and what a double-click does", () => {
    expect(submoduleCardState('uninitialized')).toBe('Nothing checked out · double-click to initialise');
    expect(submoduleCardState('newCommits')).toMatch(/another commit than the one recorded/);
  });
});
