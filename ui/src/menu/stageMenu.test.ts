import { describe, expect, it, vi } from 'vitest';

const stage = vi.hoisted(() => vi.fn(async () => true));
const discard = vi.hoisted(() => vi.fn(async () => true));
const unstage = vi.hoisted(() => vi.fn(async () => true));
const discardAll = vi.hoisted(() => vi.fn(async () => true));
vi.mock('../stage/actions', async (orig) => ({ ...(await orig<typeof import('../stage/actions')>()), stagePaths: stage, discardPaths: discard, unstageFiles: unstage, discardAll }));
const confirm = vi.hoisted(() => vi.fn(async (_r: { arm: string; danger?: boolean }) => true));
vi.mock('../ui/ConfirmDialog', async (orig) => ({ ...(await orig<typeof import('../ui/ConfirmDialog')>()), confirmAction: confirm }));
vi.mock('../app/actions', async (orig) => ({ ...(await orig<typeof import('../app/actions')>()), activeTab: () => ({ id: 't', kind: 'repo' }), activeStore: () => ({ getState: () => ({ repo: 1 }) }) }));

import './builders';
import { buildMenu } from './registry';
import type { FileChange } from '../api/gen/FileChange';
import type { FileTarget, FolderTarget, MenuEnv, WipTarget } from './menuEnv';
import type { MenuRow } from './types';

const target = (wip: FileTarget['wip']) => ({ path: 'a.txt', root: '/r', sha: null, upstream: null, diff: { key: 'k', path: 'a.txt', oldPath: null, status: 'M', old: { kind: 'absent' }, new: { kind: 'worktree', worktree: '/r' }, view: 'diff' }, changed: true, deleted: false, list: 'wip', openIn: { worktree: '/r', path: 'a.txt', line: null, source: null, fallback: null }, wip }) as unknown as FileTarget;
const env = { openers: { list: [], error: null, last: null }, forge: () => null, act: {} } as unknown as MenuEnv;
const labels = (t: FileTarget) => buildMenu('file', t, env).flatMap((r) => (r.kind === 'action' ? [r.label] : []));

describe('the WIP file menu (spec #2 §7.1)', () => {
  it('an unstaged file: Stage and Discard changes first; a staged one: Unstage; elsewhere: neither', () => {
    expect(labels(target({ worktree: '/r', staged: false, oldPath: null, status: 'M' })).slice(0, 2)).toEqual(['Stage', 'Discard changes…']);
    expect(labels(target({ worktree: '/r', staged: true, oldPath: null, status: 'M' }))[0]).toBe('Unstage');
    expect(labels(target(null))).not.toContain('Stage');
    expect(labels(target({ worktree: '/r', staged: false, oldPath: null, status: 'U' }))).not.toContain('Stage');
  });

  it('a submodule row offers Stage but no Discard', () => {
    const l = labels(target({ worktree: '/r', staged: false, oldPath: null, status: 'M', submodule: true }));
    expect(l).toContain('Stage');
    expect(l).not.toContain('Discard changes…');
  });

  it('an unstaged image format change stages and discards both halves; Discard arms in place first', async () => {
    const rows = buildMenu('file', target({ worktree: '/r', staged: false, oldPath: 'a.png', status: 'R' }), env);
    for (const r of rows.slice(0, 2)) if (r.kind === 'action') r.run();
    expect(stage).toHaveBeenLastCalledWith({ tabId: 't', repoId: 1, worktree: '/r' }, ['a.txt', 'a.png']);
    expect(confirm.mock.lastCall![0]).toMatchObject({ arm: 'Click again to discard a.txt', danger: true });
    await vi.waitFor(() => expect(discard).toHaveBeenLastCalledWith({ tabId: 't', repoId: 1, worktree: '/r' }, ['a.txt', 'a.png']));
  });

  it('runs on the row’s worktree', () => {
    const rows = buildMenu('file', target({ worktree: '/r', staged: false, oldPath: null, status: 'M' }), env);
    const first = rows[0];
    if (first.kind === 'action') first.run();
    expect(stage).toHaveBeenCalledWith({ tabId: 't', repoId: 1, worktree: '/r' }, ['a.txt']);
  });
});

const change = (path: string, extra: Partial<FileChange> = {}): FileChange => ({ path, oldPath: null, status: 'M', additions: 1, deletions: 0, old: { kind: 'absent' }, new: { kind: 'worktree', worktree: '/r' }, submodule: false, ...extra });
const folder = (wip: FolderTarget['wip']): FolderTarget => ({ path: 'src', root: '/r', openIn: { worktree: '/r', path: 'src/a.txt', line: null, source: null, fallback: null }, wip });
const folderRows = (t: FolderTarget) => buildMenu('folder', t, env).filter((r): r is Extract<MenuRow, { kind: 'action' }> => r.kind === 'action');
const ctx = { tabId: 't', repoId: 1, worktree: '/r' };

describe('a WIP folder’s menu: its hover buttons’ rows', () => {
  it('Unstaged: Stage and Discard first; Staged: Unstage; Conflicted or another list: neither', () => {
    const files = [change('src/a.txt'), change('src/new.txt', { status: '?' })];
    expect(folderRows(folder({ worktree: '/r', conflicted: [], staged: false, files })).slice(0, 2).map((r) => r.label)).toEqual(['Stage src/', 'Discard changes in src/…']);
    expect(folderRows(folder({ worktree: '/r', conflicted: [], staged: true, files }))[0].label).toBe('Unstage src/');
    const conflicted = [change('src/a.txt', { status: 'U' })];
    expect(folderRows(folder({ worktree: '/r', conflicted: ['src/a.txt'], staged: false, files: conflicted })).map((r) => r.id)).not.toContain('folder.stageFiles');
    expect(folderRows(folder(null)).map((r) => r.id)).not.toContain('folder.stageFiles');
    // Only submodules: Stage, no Discard (as the button).
    expect(folderRows(folder({ worktree: '/r', conflicted: [], staged: false, files: [change('src/sub', { submodule: true })] })).map((r) => r.id)).not.toContain('folder.discard');
  });

  it('runs the buttons’ writes: Stage and Unstage every file under it, Discard arms in place first', async () => {
    const files = [change('src/a.txt'), change('src/new.txt', { status: '?' }), change('src/sub', { submodule: true })];
    const rows = folderRows(folder({ worktree: '/r', conflicted: [], staged: false, files }));
    rows.find((r) => r.id === 'folder.stageFiles')!.run();
    expect(stage).toHaveBeenLastCalledWith(ctx, ['src/a.txt', 'src/new.txt', 'src/sub']);
    rows.find((r) => r.id === 'folder.discard')!.run();
    // The untracked file too; the submodule is discarded inside itself.
    expect(confirm.mock.lastCall![0]).toMatchObject({ arm: 'Click again to discard 2 files in src', danger: true });
    await vi.waitFor(() => expect(discard).toHaveBeenLastCalledWith(ctx, ['src/a.txt', 'src/new.txt']));
    confirm.mockResolvedValueOnce(false);
    discard.mockClear();
    rows.find((r) => r.id === 'folder.discard')!.run();
    await Promise.resolve();
    expect(discard).not.toHaveBeenCalled();
    folderRows(folder({ worktree: '/r', conflicted: [], staged: true, files }))[0].run();
    expect(unstage).toHaveBeenLastCalledWith(ctx, files);
  });
});

describe('the WIP row’s menu: Discard all changes', () => {
  const wip = (discard: WipTarget['discard']): WipTarget => ({ worktree: '/r', name: null, active: true, discard });
  const withWrite = { ...env, write: ctx } as unknown as MenuEnv;
  it('runs the WIP header’s Discard all on the row’s worktree, with what it confirms', () => {
    const rows = buildMenu('wip', wip({ count: 2, confirmed: ['a.txt', 'b.txt'], inProgress: false }), withWrite).filter((r): r is Extract<MenuRow, { kind: 'action' }> => r.kind === 'action');
    const row = rows.find((r) => r.id === 'wip.discardAll')!;
    expect(row.label).toBe('Discard all changes…');
    expect(row.tooltip).toMatch(/untracked/);
    row.run();
    expect(discardAll).toHaveBeenCalledWith(ctx, 2, ['a.txt', 'b.txt']);
    expect(buildMenu('wip', wip({ count: 2, inProgress: false }), env).map((r) => (r.kind === 'action' ? r.id : r.kind))).not.toContain('wip.discardAll');
  });
});
