// The graph (laid out by the core's own code, as WebAssembly) and the sidebar.
import type { PinSetting } from '../../api/gen/PinSetting';
import type { ViewSpec } from '../../api/gen/ViewSpec';
import type { DemoContext } from '../context';
import { DEMO_IDENTITY } from '../engine/write/objects';
import type { GraphOptions } from '../engine/repo';
import type { HandlerTable } from '../transport';
import { buildSidebar } from '../engine/sidebar';
import { cachedForks, forgeOf, forgeRemotes } from '../forge/hub';
import { setFindWindow } from './find';

/** A graph read's layout, as api.rs's `build_options`: the forks' cached facts, and the view
 * with who "me" is (the demo's commit identity) and `now` (the demo's clock) filled in. */
async function graphOptions(ctx: DemoContext, repo: number, o: { limit: number | null; pin?: PinSetting; active?: string; view?: ViewSpec }): Promise<GraphOptions> {
  const demo = ctx.repos.get(repo);
  // The default trunk follows the root of the remotes' forks: the forge's cached facts only.
  const forks = cachedForks(await forgeOf(ctx, demo.fs), await forgeRemotes(ctx, repo));
  const view = o.view && { ...o.view, me: [DEMO_IDENTITY.email], now: Math.floor(ctx.clock.nowMs() / 1000) };
  return { limit: o.limit, pin: o.pin, active: o.active, forks, ...(view ? { view } : {}) };
}

export const graphHandlers = {
  // `rescan` needs nothing: the demo reads each worktree's status every time.
  graph: async ({ repo, limit, pin, active, view }, ctx) => {
    const g = await ctx.repos.get(repo).graph(await graphOptions(ctx, repo, { limit, pin, active, view }));
    // Find searches the commits this graph shows, as the core's FindSnapshot (plan 1b's find.ts).
    setFindWindow(ctx, repo, g.rows.filter((r) => r.kind !== 'wip').map((r) => r.id));
    return g;
  },
  // --- branch presets ---
  graphReveal: async ({ repo, limit, pin, active, view, key, below }, ctx) => ctx.repos.get(repo).reveal(await graphOptions(ctx, repo, { limit, pin, active, view }), key, below),
  branchPresetPreview: async ({ repo, limit, pin, view }, ctx) => ctx.repos.get(repo).preview(await graphOptions(ctx, repo, { limit, pin, view })),
  graphHiddenOwner: async ({ repo, limit, pin, active, view, sha }, ctx) => ctx.repos.get(repo).hiddenOwner(await graphOptions(ctx, repo, { limit, pin, active, view }), sha),
  // --- end branch presets ---
  sidebar: async ({ repo }, ctx) => buildSidebar(ctx.repos.get(repo)),
} satisfies Partial<HandlerTable>;
