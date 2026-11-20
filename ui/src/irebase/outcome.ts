import type { IntegrateOutcome } from '../api/gen/IntegrateOutcome';
import { useToast } from '../ui/toastStore';

// --- what a rebase's Start, Continue or Abort tells afterwards ---

/**
 * Toasts what a rebase's outcome says: `done`, its success line, if any. A finished rebase with
 * a `warning` (something after it didn't happen) is a warning toast, the warning under `done`.
 * A stop with a `warning` (a hook refused the new message) is a warning
 * toast too. An Abort names the branch that kept the stop's commits: a warning toast, timed like
 * the others. (A kept stash has the core's own banner, `abortedWork`: no toast
 * for it. A conflict stop's own work is kept in that stash, so nothing is discarded.)
 */
export function toastRebaseOutcome(out: IntegrateOutcome | null | undefined, done?: string): void {
  if (!out) return;
  const show = useToast.getState().show;
  if (out.status === 'done' || out.status === 'upToDate') {
    if (out.warning) show(done ?? out.warning, { tone: 'warning', detail: done ? out.warning : undefined });
    else if (done) show(done);
    return;
  }
  if (out.status === 'stopped') {
    if (out.warning) show(out.warning, { tone: 'warning' });
    return;
  }
  if (out.branch) show(`Your commits from the stop are on ${out.branch}`, { tone: 'warning' });
}
