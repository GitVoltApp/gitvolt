// Test-only: a write context over a read-engine test repository, as the write engine's tests make
// one (the read tests' `fakeCtx` already runs at the parity clock with its own DemoWrites), with the demo's default settings and
// its events collected.
import type { AppEvent } from '../../api/gen/AppEvent';
import type { DemoContext } from '../context';
import { fakeCtx, type TestRepo } from './read/testkit';
import { netOf } from './remote/network';
import { initialState } from './state';

export async function openForWrite(t: TestRepo): Promise<{ ctx: DemoContext & { events: AppEvent[] }; id: number; wt: string }> {
  const events: AppEvent[] = [];
  const base = fakeCtx(t.repo);
  const ctx = Object.assign(base, {
    state: initialState(base.repos.manifest, 1600),
    events,
    emit: (ev: AppEvent) => void events.push(ev),
  });
  netOf(ctx).fs = t.memfs;
  return { ctx, id: 1, wt: t.dir };
}
