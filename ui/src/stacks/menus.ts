// ui/src/stacks/menus.ts
import type { CommitTarget, MenuEnv } from '../menu/menuEnv';
import { registerMenu } from '../menu/registry';
import { registerSyncRows } from '../sync/menus';
import { pushStack, pushStackRow } from './push';
import { rebaseStack, rebaseStackRow } from './rebase';

/** The stack rows of a branch chip's menu. Rows come from the snapshot: the
 * graph's stacks (`env.stackOf`) and the sidebar. */
export const offStackMenus: Array<() => void> = [
  // Sync group: after Push (order 10).
  registerSyncRows(20, (t, env) => (env.write ? pushStackRow(t, env, (s) => { void pushStack(env.write!, s); }) : [])),
  // Integrate group, after the base rows and "Interactive rebase X onto Y" (order 1).
  registerMenu<CommitTarget, MenuEnv>({
    id: 'stack.rebase', kind: 'commit', group: 'integrate', order: 20,
    when: (t, env) => !!t.branch?.local && !t.isWip && !t.isStash && !!env.write,
    rows: (t, env) => rebaseStackRow(t, env, (s) => { void rebaseStack(env.write!, s); }),
  }),
];
