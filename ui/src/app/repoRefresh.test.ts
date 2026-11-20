import { describe, expect, it } from 'vitest';
import type { AppEvent } from '../api/gen/AppEvent';
import type { ChangeKind } from '../api/gen/ChangeKind';
import type { OpKind } from '../api/gen/OpKind';
import { refreshGate, type RunningOp } from './repoRefresh';

const changed = (kinds: ChangeKind[], repo = 1): AppEvent => ({ type: 'repoChanged', repo, kinds, worktrees: ['/a'], versions: {} });
const refs = (repo = 1): AppEvent => ({ type: 'refsUpdated', repo });
const finished = (kind: OpKind, repo = 1): AppEvent => ({ type: 'opFinished', op: 7, kind, repo, outcome: 'ok', message: null, command: null });
const op = (kind: OpKind, step?: unknown): RunningOp => ({ repo: 1, kind, step });

describe('refreshGate', () => {
  it('a write\'s repoChanged, refsUpdated and opFinished make one full load', () => {
    const gate = refreshGate(1);
    const running = [op('checkout')];
    expect(gate(changed(['refs', 'head', 'worktree', 'index']), running)).toEqual({});
    expect(gate(refs(), running)).toBeNull();
    expect(gate(finished('checkout'), [])).toBeNull();
  });

  it('a rebase: its steps reload the graph alone; its end loads once, with the sidebar', () => {
    const gate = refreshGate(1);
    const stepping = [op('rebase', { n: 1, m: 2 })];
    expect(gate(refs(), stepping)).toEqual({ graphOnly: true });
    expect(gate(refs(), stepping)).toEqual({ graphOnly: true });
    expect(gate(changed(['refs', 'head', 'worktree', 'index']), stepping)).toEqual({});
    expect(gate(refs(), stepping)).toBeNull();
    expect(gate(finished('rebase'), [])).toBeNull();
  });

  it('a rebase whose end moved no ref still owes the sidebar its steps deferred', () => {
    const gate = refreshGate(1);
    const stepping = [op('rebase', { n: 1, m: 1 })];
    expect(gate(refs(), stepping)).toEqual({ graphOnly: true });
    expect(gate(changed(['worktree', 'index']), stepping)).toEqual({ graphOnly: true });
    expect(gate(finished('rebase'), [])).toEqual({});
    expect(gate(finished('pull'), [])).toBeNull();
  });

  it('a refsUpdated is the write\'s own only right after its repoChanged with refs, during a write', () => {
    const gate = refreshGate(1);
    // Anything else of the repo's in between: reloaded as usual.
    gate(changed(['refs']), [op('commit')]);
    gate({ type: 'journalChanged', repo: 1, worktree: '/a', state: {} as never }, [op('commit')]);
    expect(gate(refs(), [op('commit')])).toEqual({});
    // A repoChanged without refs (the watcher's worktree change), then a fetch's refsUpdated.
    gate(changed(['worktree']), [op('fetch')]);
    expect(gate(refs(), [op('fetch')])).toEqual({});
    // The watcher's repoChanged with refs, outside any write, then a fetch's refsUpdated.
    gate(changed(['refs']), [op('fetch')]);
    expect(gate(refs(), [op('fetch')])).toEqual({});
    // Another repo's events neither reload nor separate the pair.
    gate(changed(['refs']), [op('merge')]);
    expect(gate(refs(2), [op('merge')])).toBeNull();
    expect(gate(refs(), [op('merge')])).toBeNull();
  });

  it('a worktree or index change reloads the graph alone', () => {
    const gate = refreshGate(1);
    expect(gate(changed(['worktree', 'index']), [])).toEqual({ graphOnly: true });
    expect(gate(changed(['index', 'submodules']), [])).toEqual({});
    expect(gate(refs(), [])).toEqual({});
  });
});
