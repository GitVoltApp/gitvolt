import { useEffect } from 'react';
import { api, errorMessage } from '../api/client';
import type { GbError } from '../api/gen/GbError';
import type { MrHead } from '../api/gen/MrHead';
import type { SshOffer } from '../api/gen/SshOffer';
import { sshSwitches, switchToSsh, toastAfterSwitch } from '../auth/sshSwitch';
import { writeCtx } from '../write/ctx';
import { useToast } from '../ui/toastStore';
import { showServerResult } from '../sync/serverOutput';
import { openActivityLog } from './activityLog';
import { useOps } from './ops';
import { platform } from './platform';
import { useRuntime } from './runtime';
import { clampFetchInterval } from '../settings/schema';
import { useAppState } from './state';
import { runOnce } from '../pending/store';
import { isIdle, useQueue } from '../queue/store';

export interface FetchDeps {
  now(): number;
  /** ms epoch of the tab's last fetch attempt; 0 = never. */
  lastFetchAt(): number;
  isMinimized(): Promise<boolean>;
  /** True: skipped because the repo's queue was busy (a user's op cancelled it, or was running). */
  fetch(): Promise<boolean | void>;
  /** Resolves once the tab has settled after opening (`useFetchScheduler`: its first graph, then
   * a few seconds): a tab's first fetch waits for it. Never resolves once `signal` aborts. */
  settled?(signal: AbortSignal): Promise<void>;
}

/** After a tab's first graph, how long its first background fetch waits: the open's own reads
 * (status, sidebar, avatars) run first, not beside a fetch's network round trips. */
export const FIRST_FETCH_SETTLE_MS = 3000;

export const FETCH_SKIPPED_AUTH = 'Fetch skipped: authentication required';

/**
 * The timing rules, independent of React so they're unit-testable with fake timers:
 * - started (the tab became active): one fetch at once if the last one is older than the interval;
 *   a tab that never fetched (just opened) waits until it has settled (`settled`) first;
 * - then one every interval;
 * - a tick while the window is minimized is skipped, and replayed once when the window gets the
 *   focus back. CEF has no minimize event, so each tick asks (`isMinimized`), and the focus event
 *   is what tells the window is back. Visible but unfocused, the ticks go on;
 * - an interval of 0 (or less) is off;
 * - a fetch skipped because the queue was busy (a user's op cancels a background fetch)
 *   is owed: it runs again once the queue is idle (`onIdle`), not an interval later.
 */
export class FetchScheduler {
  private timer: ReturnType<typeof setInterval> | null = null;
  private missed = false;
  private owed = false;
  private running = false;
  private settling: AbortController | null = null;
  private readonly intervalMs: number;
  private readonly deps: FetchDeps;

  constructor(intervalMs: number, deps: FetchDeps) {
    this.intervalMs = intervalMs;
    this.deps = deps;
  }

  start(): void {
    if (this.intervalMs <= 0 || this.timer) return;
    this.timer = setInterval(() => { void this.tick(); }, this.intervalMs);
    if (!this.due()) return;
    if (this.deps.lastFetchAt() > 0 || !this.deps.settled) {
      void this.tick();
      return;
    }
    const settling = new AbortController();
    this.settling = settling;
    // Still due once settled: an interval tick may have fetched meanwhile (a slow first graph).
    void this.deps.settled(settling.signal).then(() => { if (!settling.signal.aborted && this.due()) void this.tick(); });
  }

  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
    this.missed = false;
    this.settling?.abort();
    this.settling = null;
  }

  private due(): boolean {
    return this.deps.now() - this.deps.lastFetchAt() >= this.intervalMs;
  }

  async tick(): Promise<void> {
    // A fetch still in flight (a slow remote): this tick adds nothing.
    if (!this.timer || this.running) return;
    const minimized = await this.deps.isMinimized();
    // Stopped while asking (the tab was hidden or closed): it does no more work.
    if (!this.timer || this.running) return;
    if (minimized) {
      this.missed = true;
      return;
    }
    await this.run();
  }

  /** The window's focus changed: back from minimized, one fetch if a tick was skipped meanwhile. */
  onFocus(focused: boolean): void {
    if (focused && this.missed && this.timer && !this.running) void this.run();
  }

  /** The repo's queue went idle: the fetch a user's op cancelled (or skipped) runs now. */
  onIdle(): void {
    if (this.owed) void this.tick();
  }

  /** One fetch, which also makes up for any skipped tick. */
  private async run(): Promise<void> {
    this.running = true;
    this.missed = false;
    this.owed = false;
    try {
      this.owed = (await this.deps.fetch()) === true;
    } finally {
      this.running = false;
    }
  }
}

/** T.1: the remotes a failed fetch named, each with its short reason. A failure that names none
 * (git died before reaching a remote) marks the only remote when the repo has just one. */
function failedRemotes(e: GbError | null, repoId: number): Record<string, { reason: string; at: number }> {
  const at = Date.now();
  const out: Record<string, { reason: string; at: number }> = {};
  const detail = e?.detail;
  if (detail?.kind === 'fetchFailed') for (const r of detail.remotes) out[r.name] = { reason: r.reason, at };
  else {
    const tab = Object.values(useRuntime.getState().tabs).find((t) => t.repo?.id === repoId);
    const remotes = tab?.sidebar?.remotes ?? [];
    if (remotes.length === 1) out[remotes[0]!.name] = { reason: e?.kind === 'AuthFailed' ? 'authentication failed' : "couldn't reach the remote", at };
  }
  return out;
}

/** Repos (by id) whose last fetch failed: a background failure reaches the bell only when it
 * starts, not on every tick of an outage. */
const failing = new Set<number>();

/**
 * One fetch of the tab's repo; `remote`: only that one (a remote just added). `background`: the scheduler's, which never prompts
 *  and shows nothing while it runs or when it succeeds: it's only in the activity log
 * (`useOps().activity`). A fetch that would need credentials comes back skipped and shows as a
 * status-bar warning until a fetch succeeds. Errors: a background one goes to the bell's history
 * (once, when the failures start), a user's to a toast with git's message and an "Activity log"
 * link; a fetch that worked says nothing (7c303cc), a failure still toasts. A cancel is quiet. A user's fetch
 * that finds a background one running waits on that one: the Fetch button shows it as busy. "Use SSH for <remote> instead"
 * on its credential prompt switches the remote, then fetches again (`sshSwitch.ts`).
 */
export async function runFetch(tabId: string, background: boolean, remote?: string, mrHead?: MrHead): Promise<void> {
  await fetchOnce(tabId, background, remote, mrHead);
}

/** `runFetch`, resolving true when it was skipped because the queue was busy (the scheduler then
 * owes it, `onIdle`). */
async function fetchOnce(tabId: string, background: boolean, remote?: string, mrHead?: MrHead): Promise<boolean> {
  // A background fetch has its own key: it must not light the Fetch button.
  return !!await runOnce(tabId, 'fetch', `${background ? 'bg:' : ''}${remote ?? ''}${mrHead ? `:${mrHead.number}` : ''}`, () => fetchNow(tabId, background, remote, mrHead));
}

/** `mrHead` (with `remote`): only that MR/PR's head (the MR view's Compare). `switched`: the
 * remotes this fetch's prompts switched to SSH before this run. True when it was skipped because
 * the queue was busy. */
async function fetchNow(tabId: string, background: boolean, remote?: string, mrHead?: MrHead, switched: SshOffer[] = []): Promise<boolean> {
  const rt = useRuntime.getState().tabs[tabId];
  if (!rt?.repo) return false;
  const repo = rt.repo;
  const patch = (p: { lastFetchAt?: number; fetchSkipped?: string | null; remoteFetchErrors?: Record<string, { reason: string; at: number }> }) => {
    // The tab may have been closed (or re-pointed) while the fetch ran.
    if (useRuntime.getState().tabs[tabId]?.repo?.id === repo.id) useRuntime.getState().patch(tabId, p);
  };
  // A one-remote fetch speaks for that remote only: the others' errors, the skip and the
  // schedule's last fetch time are the full fetch's.
  const errors = () => useRuntime.getState().tabs[tabId]?.remoteFetchErrors ?? {};
  try {
    const out = await (remote === undefined ? api.fetch(repo.id, background) : mrHead ? api.fetch(repo.id, background, remote, mrHead) : api.fetch(repo.id, background, remote));
    if (out.status === 'done' || out.reason === 'authRequired') failing.delete(repo.id);
    if (out.status === 'done') {
      // A fetch that worked says nothing (except a user's with server output, which links it, and a server warning), user-initiated or not: the button's spinner and the
      // graph are enough, and a toast every time is noise.
      if (remote === undefined) patch({ lastFetchAt: Date.now(), fetchSkipped: null, remoteFetchErrors: {} });
      else patch({ remoteFetchErrors: Object.fromEntries(Object.entries(errors()).filter(([name]) => name !== remote)) });
      if (!background && out.server.lines > 0) showServerResult(`Fetched ${repo.name}`, `Fetched ${repo.name}; the server reported a problem`, out.server, out.op);
      else if (background && out.server.warning) showServerResult('', `Background fetch of ${repo.name}: the server reported a problem`, out.server, out.op);
    } else if (out.reason === 'authRequired') patch({ lastFetchAt: Date.now(), fetchSkipped: FETCH_SKIPPED_AUTH });
    else if (!background) {
      const running = Object.values(useOps.getState().ops).find((o) => o.kind === 'fetch' && o.repo === repo.id);
      if (running) useOps.getState().showOp(running.op);
      else useToast.getState().show('A fetch is already running');
    }
    return out.status === 'skipped' && out.reason === 'busy';
  } catch (e) {
    const ssh = sshSwitches(e as GbError | null);
    const ctx = writeCtx(tabId);
    if (ssh && ctx) {
      return (await switchToSsh(ctx, ssh)) ? fetchNow(tabId, background, remote, mrHead, [...switched, ...ssh]) : false;
    }
    if (remote === undefined) patch({ lastFetchAt: Date.now() });
    const kind = (e as GbError | null)?.kind;
    if (kind === 'Cancelled') {
      if (switched.length && ctx) toastAfterSwitch(ctx, switched, e as GbError, 'Fetch failed');
      return false;
    }
    if (remote === undefined) patch({ remoteFetchErrors: failedRemotes(e as GbError | null, repo.id) });
    else {
      const named = failedRemotes(e as GbError | null, repo.id)[remote];
      const entry = named ?? { reason: kind === 'AuthFailed' ? 'authentication failed' : "couldn't reach the remote", at: Date.now() };
      patch({ remoteFetchErrors: { ...errors(), [remote]: entry } });
    }
    // git's own first line for a rejected credential is the server's ("remote: authentication
    // required"); the kind says what happened.
    const text = kind === 'AuthFailed' ? `Authentication failed (${errorMessage(e)})` : errorMessage(e);
    const started = !failing.has(repo.id);
    failing.add(repo.id);
    // Long enough to read git's message, with the activity log (which keeps it) a click away.
    if (!background && switched.length && ctx) toastAfterSwitch(ctx, switched, e as GbError, 'Fetch failed');
    else if (!background) useToast.getState().show(`Fetch failed: ${text}`, { error: true, action: { label: 'Activity log', run: openActivityLog } });
    else if (started) useOps.getState().pushError(`Fetch failed (${repo.name}): ${text}`);
    return false;
  }
}

/** Resolves `FIRST_FETCH_SETTLE_MS` after tab `tabId` has its first graph (or failed to load
 * one); never once `signal` aborts. */
function tabSettled(tabId: string, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const loaded = () => (useRuntime.getState().tabs[tabId]?.status ?? 'loading') !== 'loading';
    const wait = () => {
      off();
      timer = setTimeout(resolve, FIRST_FETCH_SETTLE_MS);
    };
    const off = useRuntime.subscribe(() => { if (loaded()) wait(); });
    signal.addEventListener('abort', () => {
      off();
      clearTimeout(timer);
    }, { once: true });
    if (loaded()) wait();
  });
}

/**
 * The background fetch for one tab. RepoTab calls it, inside the tab's `<Activity>`: only the
 * shown tab's effect is live, so a hidden tab has no timer and does no work, and showing it again restarts the schedule (one fetch at once if it's due).
 */
export function useFetchScheduler(tabId: string, repoId: number | undefined): void {
  const intervalMs = clampFetchInterval(useAppState((s) => s.settings.fetchIntervalSecs)) * 1000;
  useEffect(() => {
    if (repoId === undefined || !(intervalMs > 0)) return;
    const s = new FetchScheduler(intervalMs, {
      now: Date.now,
      lastFetchAt: () => useRuntime.getState().tabs[tabId]?.lastFetchAt ?? 0,
      isMinimized: () => platform.isMinimized().catch(() => false),
      fetch: () => fetchOnce(tabId, true),
      settled: (signal) => tabSettled(tabId, signal),
    });
    s.start();
    const off = platform.onFocusChanged((f) => s.onFocus(f));
    // The queue going idle: a fetch a user's op cancelled runs again now (`onIdle`).
    const offQueue = useQueue.subscribe((st, prev) => {
      if (isIdle(st.byRepo[repoId]) && !isIdle(prev.byRepo[repoId])) s.onIdle();
    });
    return () => {
      s.stop();
      off();
      offQueue();
    };
  }, [tabId, repoId, intervalMs]);
}
