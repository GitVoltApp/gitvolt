import { describe, expect, it } from 'vitest';
import type { AppSettings } from '../../api/gen/AppSettings';
import type { BranchPreset } from '../../api/gen/BranchPreset';
import type { RepoSettings } from '../../api/gen/RepoSettings';
import { presetToJson } from '../json';
import { deletePreset, duplicatePreset, entriesOf, forgetPick, importInto, moveInGroup, newPreset, previewSpec, setInCycle, setScope, updatePreset, type PresetsDoc } from './edit';

const preset = (id: string, over: Partial<BranchPreset> = {}): BranchPreset => ({
  id, name: id, color: 'blue', scope: 'all', rules: [], otherwise: 'show', foldMerged: false, ghostLines: true, inCycle: true, ...over,
});
const doc = (s: Partial<AppSettings> = {}, r: Partial<RepoSettings> | null = {}): PresetsDoc => ({
  settings: { builtinPresets: [{ id: 'all', inCycle: true }, { id: 'all-collapsed', inCycle: true }, { id: 'mine', inCycle: true }], branchPresets: [], ...s } as AppSettings,
  repo: r && ({ pin: null, columns: null, hiddenColumns: [], sidebarSort: {}, collapsed: [], editor: null, mrFilter: null, forgeTargetRemote: null, forkTrunkNoticeDismissed: false, ...r }),
});
const listed = (d: PresetsDoc) => entriesOf(d).map((e) => `${e.group}:${e.preset.id}${e.inCycle ? '' : '-'}`);

describe('editing branch presets', () => {
  it('groups and order follow the settings', () => {
    const d = doc({ builtinPresets: [{ id: 'mine', inCycle: false }, { id: 'all', inCycle: true }], branchPresets: [preset('g1')] }, { branchPresets: [preset('r1', { scope: 'repo' })], customPreset: preset('custom', { scope: 'repo', name: 'Custom' }) });
    expect(listed(d)).toEqual(['builtin:mine-', 'builtin:all', 'builtin:all-collapsed', 'all:g1', 'repo:r1', 'repo:custom']);
  });

  it('reorders within a group; built-ins too, keeping every built-in', () => {
    const d = doc({ branchPresets: [preset('g1'), preset('g2'), preset('g3')] }, { branchPresets: [preset('r1', { scope: 'repo' }), preset('r2', { scope: 'repo' })] });
    expect(listed(moveInGroup(d, 'all', 0, 2))).toEqual(['builtin:all', 'builtin:all-collapsed', 'builtin:mine', 'all:g2', 'all:g3', 'all:g1', 'repo:r1', 'repo:r2']);
    expect(listed(moveInGroup(d, 'repo', 1, 0)).slice(-2)).toEqual(['repo:r2', 'repo:r1']);
    const b = moveInGroup(doc({ builtinPresets: [{ id: 'mine', inCycle: false }] }), 'builtin', 0, 2);
    expect(b.settings.builtinPresets).toEqual([{ id: 'all', inCycle: true }, { id: 'all-collapsed', inCycle: true }, { id: 'mine', inCycle: false }]);
  });

  it('ticks change the cycle wherever the preset lives', () => {
    let d = doc({ branchPresets: [preset('g1')] }, { branchPresets: [preset('r1', { scope: 'repo' })], customPreset: preset('custom', { scope: 'repo' }) });
    for (const id of ['all-collapsed', 'g1', 'r1', 'custom']) d = setInCycle(d, id, false);
    expect(listed(d)).toEqual(['builtin:all', 'builtin:all-collapsed-', 'builtin:mine', 'all:g1-', 'repo:r1-', 'repo:custom-']);
  });

  it('a new preset is for all repositories, shows everything and is in the cycle', () => {
    const d = newPreset(newPreset(doc(), 'n1'), 'n2');
    expect(d.settings.branchPresets?.map((p) => [p.id, p.name, p.scope, p.otherwise, p.rules.length, p.inCycle])).toEqual([
      ['n1', 'New preset', 'all', 'show', 0, true], ['n2', 'New preset (2)', 'all', 'show', 0, true],
    ]);
    expect(d.settings.branchPresets?.[0].color).not.toBe(d.settings.branchPresets?.[1].color);
  });

  it('edits save to the right scope, and Available in moves a preset between them', () => {
    let d = doc({ branchPresets: [preset('g1')] }, { branchPresets: [preset('r1', { scope: 'repo' })] });
    d = updatePreset(d, 'r1', (p) => ({ ...p, otherwise: 'hide' }), 'x').doc;
    expect(d.repo?.branchPresets?.[0].otherwise).toBe('hide');
    d = setScope(d, 'g1', 'repo', 'x').doc;
    expect(d.settings.branchPresets).toEqual([]);
    expect(d.repo?.branchPresets?.map((p) => [p.id, p.scope])).toEqual([['r1', 'repo'], ['g1', 'repo']]);
    d = setScope(d, 'r1', 'all', 'x').doc;
    expect(d.settings.branchPresets?.map((p) => [p.id, p.scope])).toEqual([['r1', 'all']]);
    expect(d.repo?.branchPresets?.map((p) => p.id)).toEqual(['g1']);
  });

  it('renaming Custom makes it a regular preset of this repository, with its picks', () => {
    const d0 = doc({}, { branchPreset: 'custom', customPreset: preset('custom', { scope: 'repo', name: 'Custom' }), branchPicks: { custom: { shown: [], hidden: ['spike/warp-gates'] } } });
    const { doc: d, id } = updatePreset(d0, 'custom', (p) => ({ ...p, name: 'Quiet' }), 'q1');
    expect(id).toBe('q1');
    expect(d.repo?.customPreset).toBeUndefined();
    expect(d.repo?.branchPresets?.map((p) => [p.id, p.name, p.scope])).toEqual([['q1', 'Quiet', 'repo']]);
    expect(d.repo?.branchPicks).toEqual({ q1: { shown: [], hidden: ['spike/warp-gates'] } });
    expect(d.repo?.branchPreset).toBe('q1');
    // Another edit keeps it Custom.
    expect(updatePreset(d0, 'custom', (p) => ({ ...p, otherwise: 'hide' }), 'q1').doc.repo?.customPreset?.otherwise).toBe('hide');
  });

  it('built-ins are read-only; duplicating one adds an editable copy for all repositories', () => {
    const d = doc();
    expect(updatePreset(d, 'mine', (p) => ({ ...p, name: 'x' }), 'x').doc).toBe(d);
    expect(deletePreset(d, 'mine')).toBe(d);
    const dup = duplicatePreset(d, 'mine', 'm2');
    expect(dup.settings.branchPresets?.map((p) => [p.id, p.name, p.scope, p.rules.length])).toEqual([['m2', 'Mine (2)', 'all', 3]]);
    const repoDup = duplicatePreset(doc({}, { branchPresets: [preset('r1', { scope: 'repo' })] }), 'r1', 'r2');
    expect(repoDup.repo?.branchPresets?.map((p) => [p.id, p.name])).toEqual([['r1', 'r1'], ['r2', 'r1 (2)']]);
  });

  it('delete removes the preset and its picks; a repository on it falls back to All', () => {
    const d = deletePreset(doc({ branchPresets: [preset('g1'), preset('g2')] }, { branchPreset: 'g1', branchPicks: { g1: { shown: ['a'], hidden: [] }, g2: { shown: [], hidden: ['b'] } } }), 'g1');
    expect(d.settings.branchPresets?.map((p) => p.id)).toEqual(['g2']);
    expect(d.repo?.branchPicks).toEqual({ g2: { shown: [], hidden: ['b'] } });
    expect(d.repo?.branchPreset).toBeUndefined();
    expect(deletePreset(doc({}, { customPreset: preset('custom') }), 'custom').repo?.customPreset).toBeUndefined();
  });

  it('✕ on a picked chip forgets it', () => {
    const d = forgetPick(doc({}, { branchPicks: { g1: { shown: ['a', 'b'], hidden: ['c'] } } }), 'g1', 'a', 'shown');
    expect(d.repo?.branchPicks?.g1).toEqual({ shown: ['b'], hidden: ['c'] });
  });

  it('import appends to all repositories under a free name; other text changes nothing', () => {
    const d = doc({ branchPresets: [preset('g1', { name: 'Quiet' })] });
    const got = importInto(d, presetToJson(preset('x', { name: 'Quiet', otherwise: 'collapse' })), 'n9');
    expect(got && [got.id, got.name]).toEqual(['n9', 'Quiet (2)']);
    expect(got?.doc.settings.branchPresets?.map((p) => [p.id, p.scope, p.otherwise])).toEqual([['g1', 'all', 'show'], ['n9', 'all', 'collapse']]);
    expect(importInto(d, '{"gitvolt":"theme","version":1}', 'n9')).toBeNull();
  });

  it('the live counts ask about the preset as edited, with its picks and the trunk patterns', () => {
    const p = preset('g1', { otherwise: 'hide' });
    expect(previewSpec(p, doc({}, { branchPicks: { g1: { shown: ['a'], hidden: [] } }, trunkPatterns: ['release/*'] }).repo, 'pullTogether')).toEqual({
      rules: [], otherwise: 'hide', picks: { shown: ['a'], hidden: [] }, foldMerged: false, ghostLines: true, trunkPatterns: ['release/*'], expanded: [], placement: 'pullTogether',
    });
  });
});
