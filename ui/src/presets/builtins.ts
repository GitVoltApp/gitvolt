import type { BranchPreset } from '../api/gen/BranchPreset';

export const BUILTIN_IDS = ['all', 'all-collapsed', 'mine'] as const;
export const ALL_ID = 'all';
/** The repository's one Custom preset, made by the first hide or show on a built-in. */
export const CUSTOM_ID = 'custom';

const builtin = (p: Pick<BranchPreset, 'id' | 'name' | 'color' | 'rules' | 'otherwise' | 'ghostLines'>): BranchPreset =>
  ({ scope: 'all', foldMerged: false, ghostPicks: false, inCycle: true, ...p });

/** Always there and read-only (Duplicate to change one). Their rules are the ones the graph
 * crate's `the_built_in_presets_decide_as_specified` decides with. */
export const BUILTIN_PRESETS: readonly BranchPreset[] = [
  builtin({ id: 'all', name: 'All', color: 'gray', rules: [], otherwise: 'show', ghostLines: false }),
  builtin({
    id: 'all-collapsed', name: 'All, collapsed', color: 'cyan',
    rules: [{ action: 'show', matcher: { kind: 'trunk' } }, { action: 'show', matcher: { kind: 'checkedOut' } }],
    otherwise: 'collapse', ghostLines: true,
  }),
  builtin({
    id: 'mine', name: 'Mine', color: 'green',
    rules: [
      { action: 'show', matcher: { kind: 'trunk' } },
      { action: 'show', matcher: { kind: 'checkedOut' } },
      { action: 'show', matcher: { kind: 'commitsBy', author: { kind: 'me' } } },
    ],
    otherwise: 'hide', ghostLines: true,
  }),
];
