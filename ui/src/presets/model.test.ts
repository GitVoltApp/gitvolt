import { describe, expect, it } from 'vitest';
import type { AppSettings } from '../api/gen/AppSettings';
import type { BranchPreset } from '../api/gen/BranchPreset';
import type { Profile } from '../api/gen/Profile';
import type { RepoSettings } from '../api/gen/RepoSettings';
import { currentPreset, nextInCycle, pickBranch, presetList, viewSpecOf } from './model';

const preset = (id: string, over: Partial<BranchPreset> = {}): BranchPreset => ({
  id, name: id, color: 'blue', scope: 'all', rules: [{ action: 'hide', matcher: { kind: 'nameMatches', patterns: 'bot/*' } }],
  otherwise: 'show', foldMerged: false, ghostLines: true, inCycle: true, ...over,
});
const settings = (over: Partial<AppSettings> = {}): AppSettings => ({
  builtinPresets: [{ id: 'all', inCycle: true }, { id: 'all-collapsed', inCycle: true }, { id: 'mine', inCycle: true }],
  branchPresets: [], ...over,
} as AppSettings);
const repo = (over: Partial<RepoSettings> = {}): RepoSettings => ({ pin: null, columns: null, hiddenColumns: [], sidebarSort: {}, collapsed: [], editor: null, mrFilter: null, forgeTargetRemote: null, forkTrunkNoticeDismissed: false, ...over });
const profile = (r?: RepoSettings): Profile => ({ repos: r ? { '/r': r } : {} } as unknown as Profile);
const ids = (list: { preset: BranchPreset }[]) => list.map((e) => e.preset.id);

describe('the preset list', () => {
  it('lists built-ins, then all-repository presets, then this repository\'s, then Custom', () => {
    const s = settings({ builtinPresets: [{ id: 'mine', inCycle: false }, { id: 'all', inCycle: true }], branchPresets: [preset('g1')] });
    const list = presetList(s, repo({ branchPresets: [preset('r1', { scope: 'repo' })], customPreset: preset('custom', { scope: 'repo' }) }));
    // all-collapsed isn't in the settings (an older file): it still shows, ticked.
    expect(ids(list)).toEqual(['mine', 'all', 'all-collapsed', 'g1', 'r1', 'custom']);
    expect(list.map((e) => e.group)).toEqual(['builtin', 'builtin', 'builtin', 'all', 'repo', 'repo']);
    expect(list.map((e) => e.readOnly)).toEqual([true, true, true, false, false, false]);
    expect(list[0].inCycle).toBe(false);
  });

  it('a missing current preset falls back to All', () => {
    const list = presetList(settings(), repo({ branchPreset: 'deleted-global' }));
    expect(currentPreset(list, repo({ branchPreset: 'deleted-global' })).preset.id).toBe('all');
    expect(currentPreset(list, undefined).preset.id).toBe('all');
    expect(currentPreset(list, repo({ branchPreset: 'mine' })).preset.id).toBe('mine');
  });

  it('the click steps through ticked presets only, wrapping', () => {
    const list = presetList(settings({ builtinPresets: [{ id: 'all', inCycle: true }, { id: 'all-collapsed', inCycle: false }, { id: 'mine', inCycle: true }], branchPresets: [preset('g1'), preset('g2', { inCycle: false })] }), undefined);
    expect(nextInCycle(list, 'all')?.preset.id).toBe('mine');
    expect(nextInCycle(list, 'mine')?.preset.id).toBe('g1');
    expect(nextInCycle(list, 'g1')?.preset.id).toBe('all');
    // From an unticked one, on to the next ticked one after it.
    expect(nextInCycle(list, 'g2')?.preset.id).toBe('all');
  });

  it('nothing to step to when only one is ticked', () => {
    const list = presetList(settings({ builtinPresets: [{ id: 'all', inCycle: true }, { id: 'all-collapsed', inCycle: false }, { id: 'mine', inCycle: false }] }), undefined);
    expect(nextInCycle(list, 'all')).toBeNull();
    expect(nextInCycle(list, 'mine')?.preset.id).toBe('all');
  });

  it('All sends no view; others send their rules, this repository\'s picks and the trunk patterns', () => {
    const r = repo({ branchPicks: { g1: { shown: ['x'], hidden: ['y'] } }, trunkPatterns: ['release/*'] });
    const list = presetList(settings({ branchPresets: [preset('g1')] }), r);
    expect(viewSpecOf(list[0], r, ['branch:z'], 'keepDateOrder')).toBeNull();
    const view = viewSpecOf(list[3], r, ['branch:z'], 'pullTogether');
    expect(view).toEqual({
      rules: [{ action: 'hide', matcher: { kind: 'nameMatches', patterns: 'bot/*' } }], otherwise: 'show', picks: { shown: ['x'], hidden: ['y'] },
      foldMerged: false, ghostLines: true, trunkPatterns: ['release/*'], expanded: ['branch:z'], placement: 'pullTogether',
    });
    expect(view).not.toHaveProperty('me');
    expect(viewSpecOf(list[2], undefined, [], 'keepDateOrder')).toMatchObject({ otherwise: 'hide', picks: { shown: [], hidden: [] }, trunkPatterns: [] });
  });
});

describe('picking one branch', () => {
  it('hiding on a built-in creates Custom from it and switches', () => {
    const p = pickBranch(settings(), profile(repo({ branchPreset: 'mine', branchPicks: { custom: { shown: ['stale'], hidden: [] } } })), '/r', 'spike/x', false);
    const r = p.repos['/r'];
    expect(r.branchPreset).toBe('custom');
    expect(r.customPreset).toMatchObject({ id: 'custom', name: 'Custom', scope: 'repo', color: 'green', otherwise: 'hide', inCycle: true });
    expect(r.customPreset?.rules).toHaveLength(3);
    expect(r.branchPicks).toEqual({ custom: { shown: [], hidden: ['spike/x'] } });
    // A repository with no settings yet gets them.
    expect(pickBranch(settings(), profile(), '/r', 'x', false).repos['/r']).toMatchObject({ branchPreset: 'custom', customPreset: { otherwise: 'show' } });
  });

  it('hiding on a built-in reuses an existing Custom', () => {
    const custom = preset('custom', { scope: 'repo', name: 'Custom', otherwise: 'collapse' });
    const p = pickBranch(settings(), profile(repo({ branchPreset: 'all', customPreset: custom, branchPicks: { custom: { shown: [], hidden: ['a'] } } })), '/r', 'b', false);
    expect(p.repos['/r'].customPreset).toBe(custom);
    expect(p.repos['/r'].branchPreset).toBe('custom');
    expect(p.repos['/r'].branchPicks?.custom).toEqual({ shown: [], hidden: ['a', 'b'] });
  });

  it('hiding on a user preset adds to its picks', () => {
    const p = pickBranch(settings({ branchPresets: [preset('g1')] }), profile(repo({ branchPreset: 'g1' })), '/r', 'x', false);
    expect(p.repos['/r'].branchPreset).toBe('g1');
    expect(p.repos['/r'].customPreset).toBeUndefined();
    expect(p.repos['/r'].branchPicks).toEqual({ g1: { shown: [], hidden: ['x'] } });
  });

  it('showing a hidden branch moves it from hidden to shown', () => {
    const p = pickBranch(settings({ branchPresets: [preset('g1')] }), profile(repo({ branchPreset: 'g1', branchPicks: { g1: { shown: [], hidden: ['x', 'y'] } } })), '/r', 'x', true);
    expect(p.repos['/r'].branchPicks?.g1).toEqual({ shown: ['x'], hidden: ['y'] });
  });
});
