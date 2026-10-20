import { afterEach, describe, expect, it, vi } from 'vitest';
import type { DiffSpec } from '../api/gen/DiffSpec';
import type { FileChange } from '../api/gen/FileChange';
import type { GraphPayload } from '../api/gen/GraphPayload';
import type { GraphWorktree } from '../api/gen/GraphWorktree';
import type { RowPayload } from '../api/gen/RowPayload';
import { setEditorRelease } from '../diff/editorRelease';
import { filesKey } from './services';
import { createRepoViewStore, targetFor } from './store';
import { recordingServices } from './testServices';

// A refresh where the selected WIP row is gone because a commit took its place (made, or amended,
// in the app, a terminal or by an agent): that commit is selected instead, with the WIP's open file.

const [A, B, C, N, X] = ['a', 'b', 'c', 'e', 'f'].map((c) => c.repeat(40));
const row = (id: string, parents: string[], kind: RowPayload['kind'] = 'commit', wip: RowPayload['wip'] = null): RowPayload => ({
  id, kind, lane: 0, color: 0, segments: [], summary: id.slice(0, 1), bodyFirstLine: '', authorName: 'Ada', authorEmail: 'ada@example.com', authorTime: 0, committerTime: 0, parents, mrRefs: [], wip,
});
const wip = (path: string, head: string) => row(`wip:${path}`, [head], 'wip', { worktreePath: path, worktreeName: path === '/r' ? null : 'w', modified: 1, added: 0, deleted: 0, renamed: 0, conflicted: 0 });
const tree = (path: string, head: string, branch = 'refs/heads/main'): GraphWorktree => ({ path, branch, head, isMain: path === '/r', locked: false, inProgress: null });
const graphOf = (rows: RowPayload[], head: string, worktrees: GraphWorktree[] = []): GraphPayload => ({
  rows, labels: [], maxLanes: 1, pinnedRefs: [], head: { branch: 'refs/heads/main', target: head, detached: false, unborn: false }, truncated: false, worktrees,
});
// HEAD is A (on B, on C), with changes.
const base = graphOf([wip('/r', A), row(A, [B]), row(B, [C]), row(C, [])], A);
const flush = () => new Promise((r) => setTimeout(r, 0));
const change = (path: string, status = 'M', oldPath: string | null = null): FileChange => ({ path, oldPath, status, additions: 1, deletions: 0, old: { kind: 'object', oid: B }, new: { kind: 'worktree', worktree: '/r' }, submodule: false });
const unstaged = (worktree = '/r'): DiffSpec => ({ kind: 'wip', worktree, staged: false });
const commitSpec = (id: string): DiffSpec => ({ kind: 'commit', id, parent: 0 });

afterEach(() => setEditorRelease(() => {}));

function setup(g: GraphPayload = base) {
  const rec = recordingServices();
  const s = createRepoViewStore(1, '/r', g, rec.services);
  const keep = vi.fn();
  setEditorRelease(() => {}, keep);
  const st = () => s.getState();
  /** Answers the commit's file list. */
  const list = async (id: string, files: FileChange[]) => {
    rec.resolve(`files ${filesKey(commitSpec(id))}`, { files, added: files.length, deleted: 0 });
    await flush();
  };
  return { ...rec, s, st, keep, list };
}

describe('a commit takes the selected WIP row\'s place', () => {
  it('a commit on HEAD is selected, and the open file moves to its diff of that file, in the same view and place', async () => {
    const { st, keep, list } = setup();
    st().selectRow(0);
    const open = { ...targetFor(change('src/a.ts'), unstaged()), view: 'file' as const };
    st().openFile(open);
    st().setGraph(graphOf([row(N, [A]), row(A, [B]), row(B, [C]), row(C, [])], N));
    expect(st().selection).toEqual({ kind: 'commit', index: 0, id: N });
    expect(st().sections[0].spec).toEqual(commitSpec(N));
    // Still open while the commit's files load.
    expect(st().diff).toBe(open);
    await list(N, [change('src/b.ts'), change('src/a.ts')]);
    expect(st().diff).toEqual({ ...targetFor(change('src/a.ts'), commitSpec(N)), view: 'file' });
    expect(keep).toHaveBeenCalledOnce();
  });

  it('an amend (the same parent as the old HEAD) is selected too', async () => {
    const { st, list } = setup();
    st().selectRow(0);
    st().openFile(targetFor(change('src/a.ts'), unstaged()));
    st().setGraph(graphOf([row(N, [B]), row(B, [C]), row(C, [])], N));
    expect(st().selection).toEqual({ kind: 'commit', index: 0, id: N });
    await list(N, [change('src/a.ts')]);
    expect(st().diff).toEqual(targetFor(change('src/a.ts'), commitSpec(N)));
  });

  it('a file the commit doesn\'t change closes', async () => {
    const { st, keep, list } = setup();
    st().selectRow(0);
    st().openFile(targetFor(change('src/a.ts'), unstaged()));
    st().setGraph(graphOf([row(N, [A]), row(A, [B]), row(B, [C]), row(C, [])], N));
    await list(N, [change('src/b.ts')]);
    expect(st().selection.kind).toBe('commit');
    expect(st().diff).toBeNull();
    expect(keep).not.toHaveBeenCalled();
  });

  it('a renamed file stays open, by either path', async () => {
    const { st, list } = setup();
    st().selectRow(0);
    // The WIP shows the rename's old path deleted; the commit records the rename.
    st().openFile(targetFor(change('old.ts', 'D'), unstaged()));
    st().setGraph(graphOf([row(N, [A]), row(A, [B]), row(B, [C]), row(C, [])], N));
    await list(N, [change('new.ts', 'R', 'old.ts')]);
    expect(st().diff?.path).toBe('new.ts');
    expect(st().diff?.key).toBe(`${filesKey(commitSpec(N))}|new.ts`);
  });

  it('unsaved edits ask first: cancelled, the WIP\'s file stays open', async () => {
    const { st, list } = setup();
    st().selectRow(0);
    const open = targetFor(change('src/a.ts'), unstaged());
    st().openFile(open);
    let go: (() => void) | null = null;
    st().setLeaveGuard((g) => { go = g; return true; });
    st().setGraph(graphOf([row(N, [A]), row(A, [B]), row(B, [C]), row(C, [])], N));
    await list(N, [change('src/a.ts')]);
    expect(st().selection.kind).toBe('commit');
    expect(st().diff).toBe(open);
    // Saved or discarded: it moves on.
    go!();
    expect(st().diff?.key).toBe(`${filesKey(commitSpec(N))}|src/a.ts`);
  });

  it('a linked worktree\'s WIP row: its new HEAD is selected', async () => {
    const g = graphOf([wip('/w', B), row(A, [B]), row(B, [C]), row(C, [])], A, [tree('/r', A), tree('/w', B, 'refs/heads/w')]);
    const { st, list } = setup(g);
    st().selectRow(0);
    st().openFile(targetFor(change('w.ts'), unstaged('/w')));
    st().setGraph(graphOf([row(A, [B]), row(N, [B]), row(B, [C]), row(C, [])], A, [tree('/r', A), tree('/w', N, 'refs/heads/w')]));
    expect(st().selection).toEqual({ kind: 'commit', index: 1, id: N });
    await list(N, [change('w.ts')]);
    expect(st().diff?.key).toBe(`${filesKey(commitSpec(N))}|w.ts`);
  });

  it('nothing open: the commit is selected, no file opens', () => {
    const { st, keep } = setup();
    st().selectRow(0);
    st().setGraph(graphOf([row(N, [A]), row(A, [B]), row(B, [C]), row(C, [])], N));
    expect(st().selection).toEqual({ kind: 'commit', index: 0, id: N });
    expect(st().diff).toBeNull();
    expect(keep).not.toHaveBeenCalled();
  });
});

describe('the selected WIP row otherwise', () => {
  it('a partial commit leaves changes: the WIP row stays selected, its file open', () => {
    const { st } = setup();
    st().selectRow(0);
    const open = targetFor(change('src/a.ts'), unstaged());
    st().openFile(open);
    st().setGraph(graphOf([wip('/r', N), row(N, [A]), row(A, [B]), row(B, [C]), row(C, [])], N));
    expect(st().selection).toEqual({ kind: 'wip', index: 0, worktree: '/r', name: null });
    expect(st().diff).toBe(open);
  });

  it.each([
    ['discarded (HEAD unchanged)', graphOf([row(A, [B]), row(B, [C]), row(C, [])], A, [tree('/r', A)])],
    ['a reset', graphOf([row(B, [C]), row(C, [])], B, [tree('/r', B)])],
    ['a checkout of another branch', graphOf([row(X, [A]), row(A, [B]), row(B, [C]), row(C, [])], X, [tree('/r', X, 'refs/heads/other')])],
  ])('%s: the selection clears, the file closes', (_, next) => {
    const { st } = setup({ ...base, worktrees: [tree('/r', A)] });
    st().selectRow(0);
    st().openFile(targetFor(change('src/a.ts'), unstaged()));
    st().setGraph(next);
    expect(st().selection).toEqual({ kind: 'none' });
    expect(st().diff).toBeNull();
  });
});
