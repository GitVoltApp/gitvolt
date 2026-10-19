import { api } from '../api/client';
import type { GbError } from '../api/gen/GbError';
import type { SshOffer } from '../api/gen/SshOffer';
import { openDebug } from '../app/activityLog';
import { describeError } from '../errors/describe';
import { useToast } from '../ui/toastStore';
import { runWrite, type WriteCtx } from '../write/client';

/**
 * "Use SSH for <remote> instead" on an HTTPS credential prompt (spec §5.4). The backend cancels
 * the prompt and the op fails as Cancelled with `useSsh`; its caller (`runWrite`, the fetch)
 * switches each remote here, then sends the same request again, through the queue. If that run
 * fails too, the toast says the remote was switched and offers Switch back to HTTPS, which puts
 * back the URL the backend kept (the UI never sees it: it may carry a user name or a token).
 */

/** The remotes a cancelled op's prompt switched to SSH, or `null`: not that kind of cancel. */
export function sshSwitches(err: GbError | null | undefined): SshOffer[] | null {
  const d = err?.kind === 'Cancelled' ? err.detail : undefined;
  return d?.kind === 'useSsh' && d.remotes.length ? d.remotes : null;
}

const names = (remotes: SshOffer[]) => remotes.map((r) => r.remote).join(', ');

/** Points each remote at its SSH URL, one write each (a failure toasts as any write's). `true`
 * once every one is switched. */
export async function switchToSsh(ctx: WriteCtx, remotes: SshOffer[]): Promise<boolean> {
  for (const r of remotes) {
    let done = false;
    await runWrite(ctx, () => api.switchRemoteToSsh(ctx.repoId, ctx.worktree, r.remote, r.url), { origin: null, onSuccess: () => { done = true; } });
    if (!done) return false;
  }
  return true;
}

/** `send`, and once more after switching the remotes its prompt chose SSH for (a caller with
 * no failure handling of its own: a failure after that is the caller's to report). */
export async function withSshRetry<T>(ctx: WriteCtx, send: () => Promise<T>): Promise<T> {
  try {
    return await send();
  } catch (e) {
    const ssh = sshSwitches(e as GbError | null);
    if (!ssh || !(await switchToSsh(ctx, ssh))) throw e;
    return send();
  }
}

/** Switch back to HTTPS: each remote gets the URL the switch replaced. */
export async function switchBack(ctx: WriteCtx, remotes: SshOffer[]): Promise<void> {
  for (const r of remotes) {
    let done = false;
    await runWrite(ctx, () => api.switchRemoteBack(ctx.repoId, ctx.worktree, r.remote), { origin: null, onSuccess: () => { done = true; } });
    if (!done) return;
  }
  useToast.getState().show(`${names(remotes)} ${remotes.length === 1 ? 'uses' : 'use'} HTTPS again`);
}

/** The run after the switch failed (no key, an unknown host key, no access…) or was cancelled
 * (its SSH prompt dismissed): say the remote now uses SSH, with Switch back to HTTPS. `title`:
 * the caller's own ("Fetch failed"), else the error's. */
export function toastAfterSwitch(ctx: WriteCtx, remotes: SshOffer[], err: GbError, title?: string): void {
  const back = { label: 'Switch back to HTTPS', run: () => { void switchBack(ctx, remotes); } };
  const d = describeError(err);
  if (!d) {
    useToast.getState().show(`Cancelled; ${names(remotes)} now ${remotes.length === 1 ? 'uses' : 'use'} SSH`, { tone: 'warning', actions: [back] });
    return;
  }
  const details = { label: 'Details', run: () => openDebug(err.commandId === null ? 'actions' : 'commands', err.commandId) };
  useToast.getState().show(`${title ?? d.title} after switching ${names(remotes)} to SSH: ${d.message}`, { error: true, actions: [back, details] });
}
