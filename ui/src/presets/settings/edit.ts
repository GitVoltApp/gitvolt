import type { AppSettings } from '../../api/gen/AppSettings';
import type { BranchPicks } from '../../api/gen/BranchPicks';
import type { BranchPreset } from '../../api/gen/BranchPreset';
import type { CollapsePlacement } from '../../api/gen/CollapsePlacement';
import type { PresetScope } from '../../api/gen/PresetScope';
import type { PresetSlot } from '../../api/gen/PresetSlot';
import type { RepoSettings } from '../../api/gen/RepoSettings';
import type { ViewSpec } from '../../api/gen/ViewSpec';
import { GROUP_COLORS } from '../../app/tabGroups';
import { uniqueName } from '../../settings/share';
import { BUILTIN_PRESETS, CUSTOM_ID } from '../builtins';
import { importPreset, parsePresetJson } from '../json';
import { presetList, type PresetGroup } from '../model';

/**
 * The settings page's edits, pure: each takes what the presets live in (the app settings and the
 * open repository's settings, null with none open) and returns both, changed.
 */
export interface PresetsDoc { settings: AppSettings; repo: RepoSettings | null }
/** Where a preset is stored: `custom` is the repository's one Custom. */
export type PresetHome = 'builtin' | 'all' | 'repo' | 'custom';

const move = <T>(list: readonly T[], from: number, to: number): T[] => {
  const out = [...list];
  const [x] = out.splice(from, 1);
  out.splice(to, 0, x);
  return out;
};

/** The built-ins' slots in list order, every built-in present (as `presetList` shows them). */
export function builtinSlots(settings: AppSettings): PresetSlot[] {
  const slots = settings.builtinPresets.filter((s) => BUILTIN_PRESETS.some((b) => b.id === s.id));
  return [...slots, ...BUILTIN_PRESETS.filter((b) => !slots.some((s) => s.id === b.id)).map((b) => ({ id: b.id, inCycle: true }))];
}

export function homeOf(doc: PresetsDoc, id: string): PresetHome | null {
  if (BUILTIN_PRESETS.some((b) => b.id === id)) return 'builtin';
  if (doc.settings.branchPresets?.some((p) => p.id === id)) return 'all';
  if (doc.repo?.branchPresets?.some((p) => p.id === id)) return 'repo';
  return doc.repo?.customPreset?.id === id ? 'custom' : null;
}

export const entriesOf = (doc: PresetsDoc) => presetList(doc.settings, doc.repo ?? undefined);
export const findPreset = (doc: PresetsDoc, id: string): BranchPreset | null => entriesOf(doc).find((e) => e.preset.id === id)?.preset ?? null;

/** Reorders within a group (drag or Alt+arrows); Custom stays last in This repository. */
export function moveInGroup(doc: PresetsDoc, group: PresetGroup, from: number, to: number): PresetsDoc {
  if (group === 'builtin') return { ...doc, settings: { ...doc.settings, builtinPresets: move(builtinSlots(doc.settings), from, to) } };
  if (group === 'all') return { ...doc, settings: { ...doc.settings, branchPresets: move(doc.settings.branchPresets ?? [], from, to) } };
  if (!doc.repo) return doc;
  return { ...doc, repo: { ...doc.repo, branchPresets: move(doc.repo.branchPresets ?? [], from, to) } };
}

const mapIn = (list: BranchPreset[] | undefined, id: string, fn: (p: BranchPreset) => BranchPreset) => list?.map((p) => (p.id === id ? fn(p) : p));

/** Changes a preset where it lives. Renaming Custom (or moving it elsewhere) makes it a regular
 * preset under `newId`, its picks and the repository's choice following it; the next hide makes a
 * fresh Custom. Returns the preset's id after the edit. */
export function updatePreset(doc: PresetsDoc, id: string, fn: (p: BranchPreset) => BranchPreset, newId: string): { doc: PresetsDoc; id: string } {
  const home = homeOf(doc, id);
  if (home === 'builtin' || home === null) return { doc, id };
  if (home === 'all') return { doc: { ...doc, settings: { ...doc.settings, branchPresets: mapIn(doc.settings.branchPresets, id, fn) } }, id };
  const repo = doc.repo!;
  if (home === 'repo') {
    const before = repo.branchPresets!.find((p) => p.id === id)!;
    const after = fn(before);
    if (after.scope === 'all') return { doc: toAll(doc, after), id };
    return { doc: { ...doc, repo: { ...repo, branchPresets: mapIn(repo.branchPresets, id, () => after) } }, id };
  }
  const custom = repo.customPreset!;
  const after = fn(custom);
  if (after.name === custom.name && after.scope === custom.scope) return { doc: { ...doc, repo: { ...repo, customPreset: after } }, id };
  // Custom becomes a regular preset of its own.
  const { [CUSTOM_ID]: picks, ...otherPicks } = repo.branchPicks ?? {};
  const regular = { ...after, id: newId };
  const freed: RepoSettings = {
    ...repo, customPreset: undefined,
    branchPicks: picks ? { ...otherPicks, [newId]: picks } : otherPicks,
    branchPreset: repo.branchPreset === CUSTOM_ID ? newId : repo.branchPreset,
  };
  if (regular.scope === 'all') return { doc: toAll({ ...doc, repo: freed }, regular), id: newId };
  return { doc: { ...doc, repo: { ...freed, branchPresets: [...(freed.branchPresets ?? []), { ...regular, scope: 'repo' }] } }, id: newId };
}

/** A repository's preset moved to all repositories: at the end of that list. */
function toAll(doc: PresetsDoc, p: BranchPreset): PresetsDoc {
  const repo = doc.repo && { ...doc.repo, branchPresets: doc.repo.branchPresets?.filter((x) => x.id !== p.id) };
  return { repo, settings: { ...doc.settings, branchPresets: [...(doc.settings.branchPresets ?? []), { ...p, scope: 'all' }] } };
}

/** "Available in": all repositories, or the open one only (at the end of the list it goes to). */
export function setScope(doc: PresetsDoc, id: string, scope: PresetScope, newId: string): { doc: PresetsDoc; id: string } {
  const home = homeOf(doc, id);
  if (home === 'all' && scope === 'repo' && doc.repo) {
    const p = doc.settings.branchPresets!.find((x) => x.id === id)!;
    return {
      doc: {
        settings: { ...doc.settings, branchPresets: doc.settings.branchPresets!.filter((x) => x.id !== id) },
        repo: { ...doc.repo, branchPresets: [...(doc.repo.branchPresets ?? []), { ...p, scope: 'repo' }] },
      },
      id,
    };
  }
  if (home === 'repo' || home === 'custom') return updatePreset(doc, id, (p) => ({ ...p, scope }), newId);
  return { doc, id };
}

export function setInCycle(doc: PresetsDoc, id: string, inCycle: boolean): PresetsDoc {
  const home = homeOf(doc, id);
  if (home === 'builtin') return { ...doc, settings: { ...doc.settings, builtinPresets: builtinSlots(doc.settings).map((s) => (s.id === id ? { ...s, inCycle } : s)) } };
  if (home === 'custom') return { ...doc, repo: { ...doc.repo!, customPreset: { ...doc.repo!.customPreset!, inCycle } } };
  return updatePreset(doc, id, (p) => ({ ...p, inCycle }), id).doc;
}

const names = (doc: PresetsDoc) => entriesOf(doc).map((e) => e.preset.name);

/** "+ New preset": for all repositories, in the cycle, showing everything until it gets rules. */
export function newPreset(doc: PresetsDoc, newId: string): PresetsDoc {
  const used = new Set((doc.settings.branchPresets ?? []).map((p) => p.color));
  const preset: BranchPreset = {
    id: newId, name: uniqueName('New preset', names(doc)), color: GROUP_COLORS.find((c) => c !== 'gray' && !used.has(c)) ?? 'blue', scope: 'all',
    rules: [], otherwise: 'show', foldMerged: false, ghostLines: true, inCycle: true,
  };
  return { ...doc, settings: { ...doc.settings, branchPresets: [...(doc.settings.branchPresets ?? []), preset] } };
}

/** A copy at the end of its group; a built-in's goes to all repositories. Picks aren't copied. */
export function duplicatePreset(doc: PresetsDoc, id: string, newId: string): PresetsDoc {
  const p = findPreset(doc, id);
  if (!p) return doc;
  const copy: BranchPreset = { ...p, id: newId, name: uniqueName(p.name, names(doc)), inCycle: true };
  const home = homeOf(doc, id);
  if ((home === 'repo' || home === 'custom') && doc.repo) return { ...doc, repo: { ...doc.repo, branchPresets: [...(doc.repo.branchPresets ?? []), { ...copy, scope: 'repo' }] } };
  return { ...doc, settings: { ...doc.settings, branchPresets: [...(doc.settings.branchPresets ?? []), { ...copy, scope: 'all' }] } };
}

/** Deletes a preset with the open repository's picks for it; a repository on it falls back to All. */
export function deletePreset(doc: PresetsDoc, id: string): PresetsDoc {
  const home = homeOf(doc, id);
  if (home === 'builtin' || home === null) return doc;
  const settings = home === 'all' ? { ...doc.settings, branchPresets: doc.settings.branchPresets!.filter((p) => p.id !== id) } : doc.settings;
  if (!doc.repo) return { ...doc, settings };
  const { [id]: _gone, ...picks } = doc.repo.branchPicks ?? {};
  const repo: RepoSettings = {
    ...doc.repo, branchPicks: picks,
    branchPresets: home === 'repo' ? doc.repo.branchPresets!.filter((p) => p.id !== id) : doc.repo.branchPresets,
    customPreset: home === 'custom' ? undefined : doc.repo.customPreset,
    branchPreset: doc.repo.branchPreset === id ? undefined : doc.repo.branchPreset,
  };
  return { settings, repo };
}

/** ✕ on a picked-by-hand chip: the branch goes back to the rules. */
export function forgetPick(doc: PresetsDoc, id: string, name: string, which: keyof BranchPicks): PresetsDoc {
  const old = doc.repo?.branchPicks?.[id];
  if (!doc.repo || !old) return doc;
  return { ...doc, repo: { ...doc.repo, branchPicks: { ...doc.repo.branchPicks, [id]: { ...old, [which]: old[which].filter((n) => n !== name) } } } };
}

/** A pasted preset, added at the end of all repositories' list; null (nothing changed) for other text. */
export function importInto(doc: PresetsDoc, text: string, newId: string): { doc: PresetsDoc; id: string; name: string } | null {
  const parsed = parsePresetJson(text);
  if (!parsed) return null;
  const settings = importPreset(doc.settings, { ...parsed }, newId);
  const added = settings.branchPresets!.find((p) => p.id === newId)!;
  return { doc: { ...doc, settings }, id: newId, name: added.name };
}

/** What the live counts ask about: the preset as edited, even All (whose graph request sends no view). */
export function previewSpec(p: BranchPreset, repo: RepoSettings | null, placement: CollapsePlacement): ViewSpec {
  return {
    rules: p.rules, otherwise: p.otherwise, picks: repo?.branchPicks?.[p.id] ?? { shown: [], hidden: [] }, foldMerged: p.foldMerged, ghostLines: p.ghostLines,
    trunkPatterns: repo?.trunkPatterns ?? [], expanded: [], placement,
  };
}
