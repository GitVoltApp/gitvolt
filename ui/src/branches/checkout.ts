import { api } from '../api/client';
import type { CheckoutOutcome } from '../api/gen/CheckoutOutcome';
import type { CheckoutTarget } from '../api/gen/CheckoutTarget';
import type { Expect } from '../api/gen/Expect';
import type { GbError } from '../api/gen/GbError';
import type { RefLabel } from '../api/gen/RefLabel';
import type { RowPayload } from '../api/gen/RowPayload';
import { useRuntime, worktreeOf } from '../app/runtime';
import { tabIdOf } from '../app/tabStores';
import type { RepoViewStore } from '../repo/store';
import type { SidebarCtx } from '../sidebar/itemActions';
import type { SideItem } from '../sidebar/model';
import { currentOrigin, type Origin } from '../ui/arm/origin';
import { confirmAction } from '../ui/ConfirmDialog';
import { useToast } from '../ui/toastStore';
import { openWorktreeTab, setActiveWorktree } from '../worktrees/active';
import { predictedHeadOf, withHeadMove, type HeadMove } from '../pending/head';
import { endPending, startPending } from '../pending/store';
import { runWrite, type WriteCtx } from '../write/client';

/** "feature/x is checked out in ../shop-feature-x." [Switch to it] [Open in a new tab] (§9.3). */
function offerWorktree(ctx: WriteCtx, err: GbError): boolean {
  const d = err.detail;
  if (d?.kind !== 'checkedOutElsewhere') return false;
  const path = useRuntime.getState().tabs[ctx.tabId]?.graph?.worktrees.find((w) => w.branch === `refs/heads/${d.branch}`)?.path;
  if (!path) return false;
  useToast.getState().show(err.message, {
    error: true,
    actions: [
      { label: 'Switch to it', run: () => setActiveWorktree(ctx.tabId, path) },
      { label: 'Open in a new tab', run: () => { void openWorktreeTab(ctx.tabId, path); } },
    ],
  });
  return true;
}

/** How long a checkout's spinner waits for the refreshed graph before it goes anyway. */
const HANDOFF_MAX_MS = 3000;

const snapshot = (tabId: string) => { const t = useRuntime.getState().tabs[tabId]; return { graph: t?.graph ?? null, sidebar: t?.sidebar ?? null }; };

/** Resolves once both the tab's graph and sidebar are newer than `before` (the refresh a write
 * triggers), or after `maxMs`. */
function refreshedSince(tabId: string, before: ReturnType<typeof snapshot>, maxMs: number): Promise<void> {
  return new Promise((resolve) => {
    const done = () => { const now = snapshot(tabId); return now.graph !== before.graph && now.sidebar !== before.sidebar; };
    // No graph shown in this tab yet: nothing to hand over to.
    if (before.graph === null || done()) return resolve();
    const timer = setTimeout(finish, maxMs);
    const off = useRuntime.subscribe(() => { if (done()) finish(); });
    function finish() { clearTimeout(timer); off(); resolve(); }
  });
}

/** Where a checkout leaves HEAD (core `checkout.rs`'s cases): a remote branch with a local one
 * already lands on the local branch, fast-forwarded when it's only behind. */
function headMoveOf(tabId: string, target: CheckoutTarget, expect: Expect): HeadMove {
  if (target.kind === 'detached') return { branch: null, sha: target.oid };
  if (target.kind === 'branch') return { branch: target.name, sha: expect.refs[`refs/heads/${target.name}`] ?? undefined };
  const name = target.local ?? target.branch;
  const remoteRef = `refs/remotes/${target.remote}/${target.branch}`;
  const local = useRuntime.getState().tabs[tabId]?.sidebar?.locals.find((b) => b.name === name);
  const ff = !local || (local.upstream === remoteRef && local.ahead === 0 && local.behind > 0);
  return { branch: name, sha: ff ? expect.refs[remoteRef] ?? undefined : local.target };
}

/** One checkout (§9.3); the backend resolves the case when it runs. Diverged asks Reset/Cancel.
 * A repository in the way ("<path> is a repository in the way of the checkout: move it first") is
 * a plain refusal: the error toast shows it, with no Retry and nothing to force. Resolves to
 * whether HEAD landed: false when the write failed, was refused or the user said no. */
export async function checkout(ctx: WriteCtx, target: CheckoutTarget, expect: Expect, onDiverged?: 'reset', origin: Origin | null = currentOrigin()): Promise<boolean> {
  // `origin`: where it started; the question comes after the backend's answer (spec §ui confirms).
  // The ref the user acted on shows the spinner while the write runs (a remote one for a new local).
  const ref = target.kind === 'branch' ? `refs/heads/${target.name}` : target.kind === 'remote' ? `refs/remotes/${target.remote}/${target.branch}` : null;
  if (ref) startPending(ctx.tabId, ref, 'checkout');
  // The spinner hands straight over to the checkmark: on success it stays until the graph and
  // the sidebar that show the new HEAD have arrived, so there's no moment with neither.
  const before = snapshot(ctx.tabId);
  let out: CheckoutOutcome | undefined;
  try {
    // What's queued after it acts on the branch it checks out, until the graph shows it there.
    out = await withHeadMove(ctx, headMoveOf(ctx.tabId, target, expect), async () => {
      const o = (await runWrite(ctx, (ok) => api.checkout(ctx.repoId, ctx.worktree, target, expect, ok, onDiverged), { handle: (err) => offerWorktree(ctx, err), origin })) ?? undefined;
      if (o && o.status !== 'diverged') await refreshedSince(ctx.tabId, before, HANDOFF_MAX_MS);
      return o;
    });
  } finally {
    if (ref) endPending(ctx.tabId, ref, 'checkout');
  }
  if (out?.status !== 'diverged') return out?.status === 'done';
  const ok = await confirmAction({
    title: 'Branches have diverged',
    body: `${out.local} and ${out.remote} have diverged (${out.ahead} ahead, ${out.behind} behind).`,
    // The names are in the body: the button stays short, so its row never wraps.
    confirmLabel: 'Reset to remote',
    arm: `Click again to reset${out.ahead ? ` (drops ${out.ahead} local ${out.ahead === 1 ? 'commit' : 'commits'})` : ''}`,
    caption: `${out.local} and ${out.remote} have diverged (${out.ahead} ahead, ${out.behind} behind).`,
    danger: true,
  }, origin);
  // The Reset resend pins both refs to the oids the dialog showed; the core refuses without them.
  if (ok) {
    const refs = { [`refs/heads/${out.local}`]: out.localOid, [`refs/remotes/${out.remote}`]: out.remoteOid };
    return checkout(ctx, target, { head: expect.head, refs }, 'reset', origin);
  }
  return false;
}

const ctxOf = (tabId: string): WriteCtx | null => {
  const rt = useRuntime.getState().tabs[tabId];
  const worktree = worktreeOf(rt);
  return rt?.repo && worktree ? { tabId, repoId: rt.repo.id, worktree } : null;
};
/** HEAD's commit once the queued writes ran: what a checkout queued now expects. */
const headOf = (tabId: string) => predictedHeadOf(tabId).sha;
/** The checked-out branch once the queued writes ran: checking it out again does nothing. */
const isHead = (tabId: string, name: string) => predictedHeadOf(tabId).branch === name;
const shortLocal = (full: string) => full.replace(/^refs\/heads\//, '');

/** A branch chip's double-click (§9.3): its local branch, else its remote one. Tags aren't branches. */
export function checkoutLabel(store: RepoViewStore, row: RowPayload, label: RefLabel): boolean {
  const tabId = tabIdOf(store);
  const ctx = tabId ? ctxOf(tabId) : null;
  if (!ctx || label.tag || (!label.local && label.remotes.length === 0)) return false;
  // Checked out in another worktree: go there (the refusal toast's Switch to it, done directly).
  if (label.local && label.worktree) {
    setActiveWorktree(ctx.tabId, label.worktree);
    return true;
  }
  if (label.local && isHead(ctx.tabId, shortLocal(label.local))) return true; // already checked out: nothing to do
  const head = headOf(ctx.tabId);
  if (label.local) {
    void checkout(ctx, { kind: 'branch', name: shortLocal(label.local) }, { head, refs: { [label.local]: row.id } });
  } else {
    const r = label.remotes.find((x) => x.remote === 'origin') ?? label.remotes[0];
    const branch = r.fullName.slice(`refs/remotes/${r.remote}/`.length);
    void checkout(ctx, { kind: 'remote', remote: r.remote, branch }, { head, refs: { [r.fullName]: row.id } });
  }
  return true;
}

/** A sidebar branch row's double-click (§19 item 6). */
export function checkoutSideItem({ tabId }: SidebarCtx, item: SideItem): void {
  const ctx = ctxOf(tabId);
  if (!ctx || !item.target) return;
  if (item.kind === 'local' && item.branch.worktree) {
    setActiveWorktree(tabId, item.branch.worktree);
    return;
  }
  if (item.kind === 'local' && isHead(tabId, item.branch.name)) return;
  const head = headOf(tabId);
  if (item.kind === 'local') void checkout(ctx, { kind: 'branch', name: item.branch.name }, { head, refs: { [item.branch.fullName]: item.target } });
  if (item.kind === 'remote') void checkout(ctx, { kind: 'remote', remote: item.remote, branch: item.branch.name }, { head, refs: { [item.branch.fullName]: item.target } });
}

/** The toolbar branch picker and the palette: a local branch by name. */
export async function checkoutLocal(tabId: string, name: string): Promise<void> {
  const ctx = ctxOf(tabId);
  const b = useRuntime.getState().tabs[tabId]?.sidebar?.locals.find((l) => l.name === name);
  if (ctx && b) await checkout(ctx, { kind: 'branch', name }, { head: headOf(tabId), refs: { [b.fullName]: b.target } });
}

/** The palette's `@origin/x`: the remote branch. */
export async function checkoutRemote(tabId: string, remote: string, branch: string, target: string, local?: string): Promise<void> {
  const ctx = ctxOf(tabId);
  if (ctx) await checkout(ctx, { kind: 'remote', remote, branch, ...(local ? { local } : {}) }, { head: headOf(tabId), refs: { [`refs/remotes/${remote}/${branch}`]: target } });
}
