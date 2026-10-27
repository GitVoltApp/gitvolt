import type { AppSettings } from '../api/gen/AppSettings';
import type { Author } from '../api/gen/Author';
import type { BranchPreset } from '../api/gen/BranchPreset';
import type { Matcher } from '../api/gen/Matcher';
import type { PresetAction } from '../api/gen/PresetAction';
import type { Rule } from '../api/gen/Rule';
import { GROUP_COLORS } from '../app/tabGroups';
import { parseShareJson, toShareJson, uniqueName } from '../settings/share';
import { BUILTIN_PRESETS } from './builtins';

const VERSION = 1;
export type SharedPreset = Omit<BranchPreset, 'id' | 'scope' | 'inCycle'>;

/** Copy as JSON: what makes the preset, without where it lives or its picks (those are one repository's). */
export function presetToJson(p: BranchPreset): string {
  const { name, color, rules, otherwise, foldMerged, ghostLines, ghostPicks } = p;
  return toShareJson('branch-preset', VERSION, { name, color, rules, otherwise, foldMerged, ghostLines, ghostPicks });
}

const ACTIONS: readonly PresetAction[] = ['show', 'hide', 'collapse'];
const isAction = (v: unknown): v is PresetAction => ACTIONS.includes(v as PresetAction);
const isObj = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);

function author(v: unknown): Author | null {
  if (!isObj(v)) return null;
  if (v.kind === 'me') return { kind: 'me' };
  return v.kind === 'person' && typeof v.who === 'string' && v.who.trim() ? { kind: 'person', who: v.who } : null;
}

function matcher(v: unknown): Matcher | null {
  if (!isObj(v)) return null;
  switch (v.kind) {
    case 'trunk': case 'checkedOut': case 'remote': case 'local': case 'mergedIntoTrunk':
      return { kind: v.kind };
    case 'commitsBy': { const a = author(v.author); return a && { kind: 'commitsBy', author: a }; }
    case 'nameMatches': return typeof v.patterns === 'string' ? { kind: 'nameMatches', patterns: v.patterns } : null;
    case 'olderThan': return Number.isInteger(v.days) && (v.days as number) >= 0 ? { kind: 'olderThan', days: v.days as number } : null;
    default: return null;
  }
}

/** A pasted preset, checked field by field (it's untrusted text); null for anything else. */
export function parsePresetJson(text: string): SharedPreset | null {
  const env = parseShareJson(text, 'branch-preset', VERSION);
  if (!env.ok) return null;
  const { name, color, rules, otherwise, foldMerged = false, ghostLines = false, ghostPicks = false } = env.body;
  if (typeof name !== 'string' || !name.trim() || !GROUP_COLORS.includes(color as never)) return null;
  if (!Array.isArray(rules) || !isAction(otherwise) || typeof foldMerged !== 'boolean' || typeof ghostLines !== 'boolean' || typeof ghostPicks !== 'boolean') return null;
  const parsed: Rule[] = [];
  for (const r of rules) {
    const m = isObj(r) && isAction(r.action) ? matcher(r.matcher) : null;
    if (!m) return null;
    parsed.push({ action: (r as Rule).action, matcher: m });
  }
  return { name: name.trim(), color: color as string, rules: parsed, otherwise, foldMerged, ghostLines, ghostPicks };
}

/** Adds an imported preset at the end of the all-repositories list, in the cycle, renamed
 * " (2)", " (3)"… when its name is taken. */
export function importPreset(settings: AppSettings, parsed: SharedPreset, newId: string): AppSettings {
  const mine = settings.branchPresets ?? [];
  const name = uniqueName(parsed.name, [...BUILTIN_PRESETS, ...mine].map((p) => p.name));
  return { ...settings, branchPresets: [...mine, { ...parsed, name, id: newId, scope: 'all', inCycle: true }] };
}
