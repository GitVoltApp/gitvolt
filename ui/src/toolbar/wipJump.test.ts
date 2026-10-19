import { beforeEach, describe, expect, it } from 'vitest';
import type { DiffSpec } from '../api/gen/DiffSpec';
import type { FileChange } from '../api/gen/FileChange';
import type { FileListPayload } from '../api/gen/FileListPayload';
import type { GraphPayload } from '../api/gen/GraphPayload';
import type { RowPayload } from '../api/gen/RowPayload';
import { Loader } from '../data/loader';
import { Lru } from '../data/lru';
import { useFileListPrefs } from '../files/fileListPrefs';
import { createRepoViewStore, targetFor, type RepoViewStore } from '../repo/store';
import { fakeServices } from '../repo/testServices';
import { jumpToWip, onOpenWip, openWipCount } from './wipJump';

const commit = (id: string): RowPayload => ({ id, kind: 'commit', lane: 0, color: 0, segments: [], summary: id, bodyFirstLine: '', authorName: '', authorEmail: '', authorTime: 0, committerTime: 0, parents: [], mrRefs: [], wip: null });
const wipRow = (path: string, counts: Partial<NonNullable<RowPayload['wip']>> = {}): RowPayload => ({ ...commit(`wip:${path}`), kind: 'wip', wip: { worktreePath: path, worktreeName: null, modified: 0, added: 0, deleted: 0, renamed: 0, conflicted: 0, ...counts } });
const graphOf = (rows: RowPayload[], openWorktree?: string): GraphPayload => ({ rows, labels: [], maxLanes: 1, pinnedRefs: [], head: { branch: 'refs/heads/main', target: 'a', detached: false, unborn: false }, truncated: false, worktrees: [], ...(openWorktree ? { openWorktree } : {}) } as unknown as GraphPayload);
const change = (path: string, status = 'M', oldPath: string | null = null): FileChange => ({ path, oldPath, status, additions: 1, deletions: 0, old: { kind: 'absent' }, new: { kind: 'worktree', worktree: '/r' }, submodule: false });
const list = (...files: FileChange[]): FileListPayload => ({ files, added: 0, deleted: 0, version: 'v1' });

const UNSTAGED: DiffSpec = { kind: 'wip', worktree: '/r', staged: false };
const STAGED: DiffSpec = { kind: 'wip', worktree: '/r', staged: true };
const OLD: DiffSpec = { kind: 'commit', id: 'b', parent: 0 };
const flush = () => new Promise((r) => setTimeout(r, 0));

/** A WIP row for /r over commits a and b; the WIP's lists are `unstaged` and `staged`. */
function setup(unstaged: FileChange[], staged: FileChange[] = []): RepoViewStore {
  const lists = (spec: DiffSpec): FileListPayload => (spec.kind !== 'wip' ? list(change('old.txt')) : list(...(spec.staged ? staged : unstaged)));
  const services = fakeServices({ files: new Loader(async (k: string) => lists(JSON.parse(k) as DiffSpec), new Lru(16)) });
  return createRepoViewStore(1, '/r', graphOf([wipRow('/r', { modified: unstaged.length + staged.length }), commit('a'), commit('b')]), services);
}

/** Selects commit b and opens `file` from it. */
function openOld(store: RepoViewStore, file: FileChange, view: 'diff' | 'file' = 'diff') {
  store.getState().selectRow(2);
  store.getState().openFile({ ...targetFor(file, OLD), view });
}

beforeEach(() => useFileListPrefs.getState().set({ mode: 'path', sort: 'path', allFiles: false }));

describe('the WIP button (jump to the working directory changes)', () => {
  it("counts the active worktree's changed files, not another worktree's", () => {
    const s = (rows: RowPayload[], open?: string) => ({ graph: graphOf(rows, open), repoPath: '/r', selection: { kind: 'none' } as const });
    expect(openWipCount(s([commit('a')]))).toBe(0);
    expect(openWipCount(s([wipRow('/r', { modified: 1, added: 2, deleted: 1, renamed: 1, conflicted: 1 }), commit('a')]))).toBe(6);
    // The tab's active worktree is a linked one: its row, not the main worktree's.
    expect(openWipCount(s([wipRow('/r', { modified: 5 }), wipRow('/r-wt', { added: 1 }), commit('a')], '/r-wt'))).toBe(1);
    expect(openWipCount(s([wipRow('/r', { modified: 5 }), commit('a')], '/r-wt'))).toBe(0);
  });

  it('with nothing open, selects the WIP row and puts the keyboard in the graph', () => {
    const store = setup([change('a.txt')]);
    store.getState().selectRow(2);
    expect(onOpenWip(store.getState())).toBe(false);
    jumpToWip(store);
    expect(store.getState().selection).toMatchObject({ kind: 'wip', index: 0, worktree: '/r' });
    expect(onOpenWip(store.getState())).toBe(true);
    expect(store.getState().diff).toBeNull();
    expect(store.getState().focus).toBe('graph');
  });

  it("closes an open file the WIP doesn't change, so its lists show", async () => {
    const store = setup([change('a.txt')], [change('s.txt')]);
    openOld(store, change('old.txt'));
    jumpToWip(store);
    expect(store.getState().selection.kind).toBe('wip');
    await flush();
    expect(store.getState().diff).toBeNull();
    expect(store.getState().focus).toBe('graph');
  });

  it('keeps an open file the WIP changes, switched to its unstaged side when it is staged too, in the same view', async () => {
    const store = setup([change('a.txt'), change('old.txt')], [change('old.txt')]);
    openOld(store, change('old.txt'), 'file');
    jumpToWip(store);
    await flush();
    expect(store.getState().diff).toEqual({ ...targetFor(change('old.txt'), UNSTAGED), view: 'file' });
    expect(store.getState().focus).toBe('files');
  });

  it("matches a rename by either path, and opens the staged side when that's the only one", async () => {
    const store = setup([change('a.txt')], [change('renamed.txt', 'R', 'old.txt')]);
    openOld(store, change('old.txt'));
    jumpToWip(store);
    await flush();
    expect(store.getState().diff).toEqual(targetFor(change('renamed.txt', 'R', 'old.txt'), STAGED));
  });

  it('switches at once when the WIP lists are already held', async () => {
    const store = setup([change('old.txt')]);
    // The tab's watch is up: the open worktree's lists are loaded ahead and held (K44).
    store.getState().setWatched(true);
    await flush();
    openOld(store, change('old.txt'));
    jumpToWip(store);
    expect(store.getState().diff).toEqual(targetFor(change('old.txt'), UNSTAGED));
  });

  it('does nothing while the WIP row is selected, or when there are no changes', async () => {
    const store = setup([change('a.txt')]);
    store.getState().selectRow(0);
    await flush();
    const open = targetFor(change('a.txt'), UNSTAGED);
    store.getState().openFile(open);
    jumpToWip(store);
    expect(store.getState().diff).toBe(open);
    const clean = createRepoViewStore(1, '/r', graphOf([commit('a'), commit('b')]), fakeServices());
    clean.getState().selectRow(1);
    jumpToWip(clean);
    expect(clean.getState().selection).toMatchObject({ kind: 'commit', index: 1 });
  });
});
