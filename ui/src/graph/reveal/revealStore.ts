import { create } from 'zustand';
import { api } from '../../api/client';
import type { RevealPayload } from '../../api/gen/RevealPayload';
import type { RowPayload } from '../../api/gen/RowPayload';
import { viewRequest } from '../../presets/reveal';

/** The reveal panel open in a tab: the ⋮ row's or folded merge's key ("branch:<name>@<newest
 * sha>", one per run, or "fold:<sha>"), its row, the side it opened on, and whether a click
 * pinned it. */
export interface RevealOpen { key: string; rowIndex: number; below: boolean; pinned: boolean }
/** Per tab. `cache`: fetched panels by `cacheKey`, `null` for a key the core no longer knows;
 * dropped with every new graph payload (the panel's rows are laid out on the graph's lanes). */
export interface TabReveal { open: RevealOpen | null; cache: Map<string, RevealPayload | null> }

export const useReveal = create<{ tabs: Record<string, TabReveal> }>(() => ({ tabs: {} }));

const NONE: TabReveal = { open: null, cache: new Map() };
export const tabReveal = (tabId: string): TabReveal => useReveal.getState().tabs[tabId] ?? NONE;
export const cacheKey = (key: string, below: boolean) => `${key}|${below ? 'below' : 'above'}`;

function patch(tabId: string, p: Partial<TabReveal>): void {
  useReveal.setState((s) => ({ tabs: { ...s.tabs, [tabId]: { ...(s.tabs[tabId] ?? NONE), ...p } } }));
}

export function openReveal(tabId: string, key: string, rowIndex: number, below: boolean, pinned: boolean): void {
  const cur = tabReveal(tabId).open;
  if (cur && cur.key === key && cur.rowIndex === rowIndex && cur.below === below && cur.pinned === pinned) return;
  patch(tabId, { open: { key, rowIndex, below, pinned } });
}

export function closeReveal(tabId: string): void {
  if (tabReveal(tabId).open) patch(tabId, { open: null });
}

/** The reveal key a graph row opens a panel for, if any: a ⋮ row's or a folded merge's. */
export const revealKeyOf = (row: RowPayload | undefined): string | null => row?.collapsed?.key ?? row?.fold?.key ?? null;

// Bumped by every new graph payload: a fetch started for an older one is dropped.
const generation = new Map<string, number>();
const inflight = new Map<string, Promise<RevealPayload | null>>();

/** A new graph payload: cached panels go, and an open panel follows its row, or closes when the
 * row is gone. */
export function revealGraphChanged(tabId: string, rows: readonly RowPayload[]): void {
  generation.set(tabId, (generation.get(tabId) ?? 0) + 1);
  for (const k of [...inflight.keys()]) if (k.startsWith(`${tabId}\n`)) inflight.delete(k);
  const t = useReveal.getState().tabs[tabId];
  if (!t) return;
  const open = t.open;
  const i = open ? (revealKeyOf(rows[open.rowIndex]) === open.key ? open.rowIndex : rows.findIndex((r) => revealKeyOf(r) === open.key)) : -1;
  patch(tabId, { cache: new Map(), open: open && i >= 0 ? { ...open, rowIndex: i } : null });
}

/** The panel's rows for `key` on side `below`, from the core (the same request shape as the
 * tab's `graph`), cached until the next graph payload. Null under All, or for a stale key. */
export function fetchReveal(tabId: string, key: string, below: boolean): Promise<RevealPayload | null> {
  const ck = cacheKey(key, below);
  const cached = tabReveal(tabId).cache.get(ck);
  if (cached !== undefined) return Promise.resolve(cached);
  const id = `${tabId}\n${ck}`;
  const running = inflight.get(id);
  if (running) return running;
  const req = viewRequest(tabId);
  if (!req) return Promise.resolve(null);
  const gen = generation.get(tabId) ?? 0;
  const run = api.graphReveal(req.repo, req.limit, req.extra, req.view, key, below)
    .catch(() => null)
    .then((payload) => {
      if ((generation.get(tabId) ?? 0) !== gen) return payload;
      inflight.delete(id);
      const cache = new Map(tabReveal(tabId).cache);
      cache.set(ck, payload);
      patch(tabId, { cache });
      return payload;
    });
  inflight.set(id, run);
  return run;
}

/** Tests: a panel's rows as if fetched. */
export function seedReveal(tabId: string, below: boolean, payload: RevealPayload): void {
  const cache = new Map(tabReveal(tabId).cache);
  cache.set(cacheKey(payload.key, below), payload);
  patch(tabId, { cache });
}
