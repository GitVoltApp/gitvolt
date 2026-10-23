// @vitest-environment happy-dom
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DiffTarget } from '../repo/store';
import type { MenuRow } from '../menu/types';

vi.mock('../api/client', () => ({ api: { saveProfile: vi.fn(async () => null) }, errorMessage: (e: unknown) => String(e) }));

const { buildMenu } = await import('../menu/registry');
const { useAppState, EMPTY_PROFILE } = await import('../app/state');
const { registerFileColoursOpener, useAddPatternRequest } = await import('./addRequest');
beforeAll(async () => { await import('../menu/builders'); await import('./feature'); });

import type { FileTarget, FolderTarget, MenuEnv } from '../menu/menuEnv';

const diff = { key: 'k', path: 'p', oldPath: null, status: 'M', old: { kind: 'object', oid: 'o'.repeat(40) }, new: { kind: 'object', oid: 'n'.repeat(40) }, view: 'diff' } as unknown as DiffTarget;
const fileT = (path: string): FileTarget => ({
  path, root: '/r', sha: null, upstream: null, diff, changed: true, deleted: false, list: 'commit', wip: null,
  openIn: { worktree: '/r', path, line: null, source: diff.new, fallback: null },
} as unknown as FileTarget);
const folderT = (path: string): FolderTarget => ({ path, root: '/r', openIn: { worktree: '/r', path, line: null, source: null, fallback: null }, wip: null } as unknown as FolderTarget);
const env = (over: Partial<MenuEnv> = {}): MenuEnv => ({
  forge: () => null, openers: { list: [], error: null, last: null }, act: {} as MenuEnv['act'], headBranch: 'main', headSha: null,
  inGraph: () => true, write: null, sidebar: null, labelsAt: () => [], activeWorktree: null, mainWorktree: null, inProgress: null,
  worktreeShown: (p) => p, commonDir: '/r/.git', ...over,
});
const labels = (rows: MenuRow[]) => rows.flatMap((r) => (r.kind === 'separator' ? [] : [r.label]));

describe('file colours menu rows', () => {
  beforeEach(() => {
    useAppState.setState({ profile: { ...EMPTY_PROFILE, fileColours: { '/r/.git': [{ id: 's1', name: 'Tests', tint: 'green', patterns: ['**/*.snap'], startCollapsed: false }] } } });
    useAddPatternRequest.getState().clear();
  });

  it("a file's menu names its scope and offers its type and its folder", () => {
    const open = vi.fn();
    const off = registerFileColoursOpener(open);
    const rows = buildMenu<FileTarget, MenuEnv>('file', fileT('crates/c/tests/snapshots/market.snap'), env());
    expect(rows.find((r) => r.kind === 'header')).toMatchObject({ label: 'Scope: Tests', detail: '**/*.snap' });
    expect(labels(rows)).toEqual(expect.arrayContaining(['Colour *.snap files…', 'Colour folder crates/c/tests/snapshots/…']));
    const item = rows.find((r) => r.kind === 'action' && r.label.startsWith('Colour folder'));
    if (item?.kind !== 'action') throw new Error('no folder item');
    item.run();
    expect(open).toHaveBeenCalledTimes(1);
    expect(useAddPatternRequest.getState().req?.pattern).toBe('crates/c/tests/snapshots/**');
    off();
  });

  it('without a File colours page, only the scope header shows', () => {
    const rows = buildMenu<FileTarget, MenuEnv>('file', fileT('a/market.snap'), env());
    expect(labels(rows).filter((l) => l.startsWith('Colour'))).toEqual([]);
    expect(rows.some((r) => r.kind === 'header')).toBe(true);
  });

  it('a file in no scope gets no header', () => {
    const rows = buildMenu<FileTarget, MenuEnv>('file', fileT('a/b.txt'), env());
    expect(rows.some((r) => r.kind === 'header')).toBe(false);
  });

  it('a folder row offers its folder', () => {
    const off = registerFileColoursOpener(() => {});
    const rows = buildMenu<FolderTarget, MenuEnv>('folder', folderT('src/a'), env());
    expect(labels(rows)).toContain('Colour folder src/a/…');
    expect(labels(rows)).not.toContain('Colour *.a files…');
    off();
  });

  it('a file at the root has no folder item', () => {
    const off = registerFileColoursOpener(() => {});
    const rows = buildMenu<FileTarget, MenuEnv>('file', fileT('Cargo.lock'), env());
    expect(labels(rows)).toContain('Colour *.lock files…');
    expect(labels(rows).some((l) => l.startsWith('Colour folder'))).toBe(false);
    off();
  });
});
