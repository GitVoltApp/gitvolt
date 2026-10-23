import { describe, expect, it } from 'vitest';
import type { AppSettings } from '../api/gen/AppSettings';
import type { BranchPreset } from '../api/gen/BranchPreset';
import { importPreset, parsePresetJson, presetToJson } from './json';

const folded: BranchPreset = {
  id: 'p1', name: 'Mine, others folded', color: 'purple', scope: 'repo', inCycle: false, foldMerged: true, ghostLines: false, otherwise: 'collapse',
  rules: [
    { action: 'show', matcher: { kind: 'commitsBy', author: { kind: 'me' } } },
    { action: 'hide', matcher: { kind: 'nameMatches', patterns: 'dependabot/*' } },
    { action: 'collapse', matcher: { kind: 'olderThan', days: 30 } },
  ],
};

describe('preset JSON', () => {
  it('copy as JSON is self-describing and leaves picks out', () => {
    const json = JSON.parse(presetToJson(folded));
    expect(json).toEqual({ gitvolt: 'branch-preset', version: 1, name: 'Mine, others folded', color: 'purple', rules: folded.rules, otherwise: 'collapse', foldMerged: true, ghostLines: false });
    expect(Object.keys(json)).not.toContain('id');
    expect(parsePresetJson(presetToJson(folded))).toEqual({ name: folded.name, color: 'purple', rules: folded.rules, otherwise: 'collapse', foldMerged: true, ghostLines: false });
  });

  it('import rejects other JSON and renames clashes', () => {
    expect(parsePresetJson(JSON.stringify({ gitvolt: 'theme', version: 1, name: 'Dusk', base: 'dark' }))).toBeNull();
    expect(parsePresetJson('{}')).toBeNull();
    expect(parsePresetJson('not json {')).toBeNull();
    const body = JSON.parse(presetToJson(folded));
    expect(parsePresetJson(JSON.stringify({ ...body, color: '#ff0000' }))).toBeNull();
    expect(parsePresetJson(JSON.stringify({ ...body, rules: [{ action: 'explode', matcher: { kind: 'trunk' } }] }))).toBeNull();
    expect(parsePresetJson(JSON.stringify({ ...body, rules: [{ action: 'hide', matcher: { kind: 'olderThan', days: -1 } }] }))).toBeNull();

    const parsed = parsePresetJson(presetToJson(folded))!;
    let s = { branchPresets: [{ ...folded, scope: 'all' }] } as unknown as AppSettings;
    s = importPreset(s, parsed, 'n2');
    s = importPreset(s, parsed, 'n3');
    expect(s.branchPresets?.map((p) => [p.id, p.name, p.scope, p.inCycle])).toEqual([
      ['p1', 'Mine, others folded', 'all', false],
      ['n2', 'Mine, others folded (2)', 'all', true],
      ['n3', 'Mine, others folded (3)', 'all', true],
    ]);
    // A built-in's name is taken too.
    expect(importPreset({} as AppSettings, { ...parsed, name: 'Mine' }, 'n4').branchPresets?.[0].name).toBe('Mine (2)');
  });
});
