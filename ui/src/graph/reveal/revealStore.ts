import { create } from 'zustand';
import { api } from '../../api/client';
import type { RevealPayload } from '../../api/gen/RevealPayload';
import type { RowPayload } from '../../api/gen/RowPayload';
import { onTabClosed } from '../../app/closedTabs';
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

// Fetches running, by tab and cache key: one gone from here (a new graph payload, the tab
// closed) is dropped when it answers.
const inflight = new Map<string, Promise<RevealPayload | null>>();
const dropInflight = (tabId: string) => { for (const k of [...inflight.keys()]) if (k.startsWith(`${tabId}\n`)) inflight.delete(k); };

// A closed tab's panel, its cached rows and its running fetches go with it.
onTabClosed((tabId) => {
  dropInflight(tabId);
  if (!(tabId in useReveal.getState().tabs)) return;
  useReveal.setState((s) => {
    const tabs = { ...s.tabs };
    delete tabs[tabId];
    return { tabs };
  });
});

/** A new graph payload: cached panels go, and an open panel follows its row, or closes when the
 * row is gone. */
export function revealGraphChanged(tabId: string, rows: readonly RowPayload[]): void {
  dropInflight(tabId);
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
  const run: Promise<RevealPayload | null> = api.graphReveal(req.repo, req.limit, req.extra, req.view, key, below)
    .catch(() => null)
    .then((payload) => {
      if (inflight.get(id) !== run) return payload;
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
