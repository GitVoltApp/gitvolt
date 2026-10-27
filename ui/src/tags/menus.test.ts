import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api/client';
import type { CommitTarget, MenuEnv, SidebarTarget, TagTarget } from '../menu/menuEnv';
import { buildMenu } from '../menu/registry';
import type { MenuRow } from '../menu/types';
import type { TagRemoteLabel } from '../api/gen/TagRemoteLabel';
import type { TagRemoteState } from '../api/gen/TagRemoteState';
import './menus';

const ctx = { tabId: 't', repoId: 1, worktree: '/r' };
const A = 'a'.repeat(40);
const env = (remoteNames: string[]) => ({ write: ctx, remoteNames }) as unknown as MenuEnv;
type Action = Extract<MenuRow, { kind: 'action' }>;
const find = (rows: MenuRow[], id: string) => rows.find((r) => r.kind !== 'separator' && r.id === id);
const tag: TagTarget = { name: 'v1', fullName: 'refs/tags/v1', sha: A };
const ok = { outcome: { op: 7, remote: 'origin', tag: 'v1', upToDate: false, server: { lines: 0, warning: null } }, journal: { undo: null, redo: null, undoBlocked: null, redoBlocked: null, banners: [], paused: null }, staging: { undo: null, redo: null, off: null }, wip: null } as never;

afterEach(() => vi.restoreAllMocks());

describe('tag rows (spec #3 §3.9, §4.3)', () => {
  it('Create tag here: Lightweight | Annotated, on a commit (not a WIP row or a stash)', () => {
    const t: CommitTarget = { sha: A, mrRefs: [], isWip: false, isStash: false, branch: null };
    const row = find(buildMenu('commit', t, env([])), 'tag.createHere') as Action;
    expect(row.label).toBe('Create tag here');
    expect(row.variants?.map((v) => v.label)).toEqual(['Lightweight', 'Annotated']);
    expect(find(buildMenu('commit', { ...t, isStash: true }, env([])), 'tag.createHere')).toBeUndefined();
  });
  it('one remote: Push v1 to origin, and Delete | Local | Remote | Both |', () => {
    const rows = buildMenu('tag', tag, env(['origin']));
    expect((find(rows, 'tag.push') as Action).label).toBe('Push v1 to origin');
    expect((find(rows, 'tag.delete') as Action).variants?.map((v) => v.id)).toEqual(['local', 'remote', 'both']);
  });
  it('no remote: no Push, and Delete is local only', () => {
    const rows = buildMenu('tag', tag, env([]));
    expect(find(rows, 'tag.push')).toBeUndefined();
    expect((find(rows, 'tag.delete') as Action).variants?.map((v) => v.id)).toEqual(['local']);
  });
  it('several remotes: Push v1 to ▸ one row each; Delete Remote targets origin', () => {
    const rows = buildMenu('tag', tag, env(['fork', 'origin']));
    const push = find(rows, 'tag.push');
    expect(push?.kind).toBe('submenu');
    expect(push?.kind === 'submenu' && push.rows.map((r) => (r.kind === 'action' ? r.label : ''))).toEqual(['fork', 'origin']);
    expect((find(rows, 'tag.delete') as Action).variants?.find((v) => v.id === 'remote')?.tooltip).toBe("Delete v1 from origin (a push: can't be undone)");
  });
  describe("by the remotes' known tags", () => {
    const on = (remote: string, state: TagRemoteState, commit?: string): TagRemoteLabel => ({ remote, host: null, hostKind: 'generic', state, ...(commit ? { commit } : {}) });
    const known = (...remotes: TagRemoteLabel[]): TagTarget => ({ ...tag, remotes });
    const pushRows = (rows: MenuRow[]) => {
      const push = find(rows, 'tag.push');
      return push?.kind === 'submenu' ? push.rows.map((r) => (r.kind === 'action' ? r.label : '')) : push?.kind === 'action' ? [push.label] : [];
    };
    it('no Push to a remote that has the tag at this commit; none at all when every remote has it', () => {
      expect(pushRows(buildMenu('tag', known(on('origin', 'same'), on('fork', 'missing')), env(['fork', 'origin'])))).toEqual(['Push v1 to fork']);
      expect(find(buildMenu('tag', known(on('origin', 'same')), env(['origin'])), 'tag.push')).toBeUndefined();
      // A remote whose tags aren't known keeps its row.
      expect(pushRows(buildMenu('tag', known(on('origin', 'same')), env(['fork', 'origin'])))).toEqual(['Push v1 to fork']);
    });
    it('a remote with another v1 keeps its Push, and its tooltip says the push is refused', () => {
      const rows = buildMenu('tag', known(on('origin', 'other', 'b'.repeat(40))), env(['origin']));
      const push = find(rows, 'tag.push') as Action;
      expect(push.label).toBe('Push v1 to origin');
      expect(push.tooltip).toBe('origin has another v1 (at bbbbbb): it refuses the push');
    });
    it("Delete's Remote and Both target a remote known to have it, or not known; never one known to lack it", () => {
      const del = (t: TagTarget, names: string[]) => find(buildMenu('tag', t, env(names)), 'tag.delete') as Action;
      expect(del(known(on('origin', 'missing')), ['origin']).variants?.map((v) => v.id)).toEqual(['local']);
      expect(del(known(on('origin', 'missing'), on('fork', 'same')), ['fork', 'origin']).variants?.find((v) => v.id === 'remote')?.tooltip).toBe("Delete v1 from fork (a push: can't be undone)");
      expect(del(known(on('origin', 'other')), ['origin']).variants?.map((v) => v.id)).toEqual(['local', 'remote', 'both']);
      expect(del(known(on('fork', 'missing')), ['fork', 'origin']).variants?.find((v) => v.id === 'both')?.tooltip).toBe('Delete v1 from origin, then here');
    });
  });
  it("a remote's menu: Push all tags to origin", async () => {
    const push = vi.spyOn(api, 'pushTags').mockResolvedValue(ok);
    const target: SidebarTarget = { what: 'remote', name: 'origin', url: null };
    const row = find(buildMenu('sidebar', target, env(['origin'])), 'remote.pushTags') as Action;
    expect(row.label).toBe('Push all tags to origin');
    row.run();
    await vi.waitFor(() => expect(push).toHaveBeenCalledWith(1, '/r', 'origin', null));
  });
});
