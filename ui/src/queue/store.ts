import { create } from 'zustand';
import { api } from '../api/client';
import type { AppEvent } from '../api/gen/AppEvent';
import type { OpKind } from '../api/gen/OpKind';
import type { QueueStatePayload } from '../api/gen/QueueStatePayload';

export const IDLE: QueueStatePayload = { running: null, queued: [], stopped: null };

interface QueueStore {
  /** Each open repo id's queue (the backend announces it to every tab of the repository). */
  byRepo: Record<number, QueueStatePayload>;
  set(repo: number, s: QueueStatePayload): void;
}

export const useQueue = create<QueueStore>((set) => ({
  byRepo: {},
  set: (repo, s) => set((st) => ({ byRepo: { ...st.byRepo, [repo]: s } })),
}));

export function applyQueueEvent(ev: AppEvent): void {
  if (ev.type === 'queueChanged') useQueue.getState().set(ev.repo, { running: ev.running, queued: ev.queued, stopped: ev.stopped });
}

/** The state at the time a tab shows (events keep it current after). */
export async function loadQueue(repo: number): Promise<void> {
  const before = useQueue.getState().byRepo[repo];
  try {
    const s = await api.queueState(repo);
    // A queueChanged that arrived meanwhile is newer than this response.
    if (useQueue.getState().byRepo[repo] === before) useQueue.getState().set(repo, s);
  } catch (e) {
    console.warn('[gitvolt] queue state', e);
  }
}

export const isIdle = (q: QueueStatePayload | undefined) => !q || (!q.running && q.queued.length === 0 && !q.stopped);

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** `Running: push dev`; with others waiting it names them (`Push dev · Rebase and 2 more
 * waiting`); after a failure `Stopped: push dev failed · 2 not run`. */
export function chipText(q: QueueStatePayload): string {
  const n = q.queued.length;
  if (q.stopped) return `Stopped: ${q.stopped.label} failed · ${n} not run`;
  const waiting = n ? `${cap(q.queued[0].label)}${n > 1 ? ` and ${n - 1} more` : ''} waiting` : '';
  if (!q.running) return waiting;
  return n ? `${cap(q.running.label)} · ${waiting}` : `Running: ${q.running.label}`;
}

/** The chip's tooltip: with several waiting, every one in order. */
export function chipTitle(q: QueueStatePayload): string {
  const base = "The action queue: what runs now and what's next";
  return q.queued.length > 1 ? `${base}. Waiting: ${q.queued.map((i) => i.label).join(', ')}` : base;
}

/** An op of `kind` waits in `repo`'s queue: its button shows the queued badge. */
export const useQueuedKind = (repo: number, kind: OpKind) => useQueue((s) => (s.byRepo[repo]?.queued ?? []).some((i) => i.kind === kind));
