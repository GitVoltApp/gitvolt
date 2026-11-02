import { api } from '../api/client';
import type { GitflowConfig } from '../api/gen/GitflowConfig';
import type { GitflowFinishOutcome } from '../api/gen/GitflowFinishOutcome';
import type { GitflowKind } from '../api/gen/GitflowKind';
import { useRuntime, worktreeOf } from '../app/runtime';
import { checkoutLocal } from '../branches/checkout';
import { createBranch } from '../branches/create';
import { startIntegrate } from '../integrate/integrate';
import { useToast } from '../ui/toastStore';
import { runWrite, type WriteCtx } from '../write/client';
import { baseOf, classify, prefixOf } from './model';

/** The tab's write context, when it has a repository open. */
export function ctxOf(tabId: string): WriteCtx | null {
  const rt = useRuntime.getState().tabs[tabId];
  const worktree = worktreeOf(rt);
  return rt?.repo && worktree ? { tabId, repoId: rt.repo.id, worktree } : null;
}

/** The tab's git flow settings, once it's set up. */
export const flowOf = (tabId: string): GitflowConfig | null => useRuntime.getState().tabs[tabId]?.sidebar?.gitflow ?? null;

const tipOf = (tabId: string, branch: string): string | null => useRuntime.getState().tabs[tabId]?.sidebar?.locals.find((b) => b.name === branch)?.target ?? null;

const headBranchOf = (tabId: string): string | null => useRuntime.getState().tabs[tabId]?.graph?.head.branch?.replace(/^refs\/heads\//, '') ?? null;

export async function initGitflow(ctx: WriteCtx, config: GitflowConfig, editing: boolean): Promise<void> {
  await runWrite(ctx, () => api.gitflowInit(ctx.repoId, ctx.worktree, config), {
    onSuccess: () => useToast.getState().show(editing ? 'Git flow settings saved' : 'Git flow is set up'),
  });
}

/** A new feature, release or hotfix: an ordinary create-and-checkout at its base branch's tip. */
export async function startGitflow(ctx: WriteCtx, config: GitflowConfig, kind: GitflowKind, name: string): Promise<void> {
  const base = baseOf(config, kind);
  const sha = tipOf(ctx.tabId, base);
  if (!sha) return void useToast.getState().show(`There's no branch ${base} to start from`, { error: true });
  await createBranch(ctx, { sha, ref: `refs/heads/${base}` }, `${prefixOf(config, kind)}${name}`, true);
}

const list = (names: string[]) => (names.length < 2 ? names.join('') : `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`);
const files = (n: number) => `${n} ${n === 1 ? 'file' : 'files'}`;

/** What a finish did, in one sentence. */
export function finishedText(branch: string, out: Extract<GitflowFinishOutcome, { status: 'done' }>): string {
  const done = [
    ...(out.merged.length ? [`merged into ${list(out.merged)}`] : []),
    ...(out.tag ? [`tagged ${out.tag}`] : []),
    ...(out.deleted ? ['deleted'] : []),
  ];
  // "merged into main and develop, and tagged v1.2.0": a comma where a step has its own "and".
  const said = done.length > 1 && done.some((d) => d.includes(' and ')) ? `${done.slice(0, -1).join(', ')}, and ${done[done.length - 1]}` : list(done);
  return done.length ? `Finished ${branch}: ${said}` : `${branch} is already finished`;
}

/** Finish: one write. A merge that would conflict is left for the user to make (the usual merge,
 * with its conflict banner); Finish again then does what's left. */
export async function finishGitflow(ctx: WriteCtx, branch: string, opts: { tag: string | null; tagMessage: string | null; keep: boolean }): Promise<void> {
  const config = flowOf(ctx.tabId);
  const kind = config && classify(config, branch)?.kind;
  if (!config || !kind) return;
  const names = [branch, ...(kind === 'feature' ? [config.develop] : [config.master, config.develop])];
  const refs: Record<string, string | null> = {};
  for (const n of names) { const tip = tipOf(ctx.tabId, n); if (tip) refs[`refs/heads/${n}`] = tip; }
  const out = await runWrite(ctx, () => api.gitflowFinish(ctx.repoId, ctx.worktree, { branch, tag: opts.tag, tagMessage: opts.tagMessage, keep: opts.keep, expect: { head: null, refs } }));
  if (!out) return;
  if (out.status === 'done') return void useToast.getState().show(finishedText(branch, out));
  const before = [...(out.merged.length ? [`merged into ${list(out.merged)}`] : []), ...(out.tag ? [`tagged ${out.tag}`] : [])];
  useToast.getState().show(`Merging ${branch} into ${out.into} will conflict in ${files(out.files.length)}`, {
    tone: 'warning',
    sticky: true,
    detail: `${before.length ? `Done so far: ${list(before)}. ` : ''}Make that merge and resolve the conflicts, then finish ${branch} again.`,
    action: { label: `Merge into ${out.into}`, run: () => { void mergeInto(ctx, out.into, branch); } },
  });
}

/** The merge a finish left: check `into` out, then the usual Merge (its preview, its conflicts). */
async function mergeInto(ctx: WriteCtx, into: string, branch: string): Promise<void> {
  if (headBranchOf(ctx.tabId) !== into) await checkoutLocal(ctx.tabId, into);
  if (headBranchOf(ctx.tabId) !== into) return;
  const now = ctxOf(ctx.tabId);
  if (now) await startIntegrate(now, 'merge', branch, into);
}
