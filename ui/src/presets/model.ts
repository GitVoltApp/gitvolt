import type { AppSettings } from '../api/gen/AppSettings';
import type { BranchPicks } from '../api/gen/BranchPicks';
import type { BranchPreset } from '../api/gen/BranchPreset';
import type { CollapsePlacement } from '../api/gen/CollapsePlacement';
import type { Profile } from '../api/gen/Profile';
import type { RepoSettings } from '../api/gen/RepoSettings';
import type { ViewSpec } from '../api/gen/ViewSpec';
import { ALL_ID, BUILTIN_PRESETS, CUSTOM_ID } from './builtins';

export type PresetGroup = 'builtin' | 'all' | 'repo';
export interface PresetEntry { preset: BranchPreset; group: PresetGroup; inCycle: boolean; readOnly: boolean }

/** Mirrors `EMPTY_REPO_SETTINGS` (app/state.ts), which this pure module doesn't import. */
const NO_REPO: RepoSettings = { pin: null, columns: null, hiddenColumns: [], sidebarSort: {}, collapsed: [], editor: null, mrFilter: null, forgeTargetRemote: null, forkTrunkNoticeDismissed: false };
const NO_PICKS: BranchPicks = { shown: [], hidden: [] };

/** Every preset, in list (and cycle) order: the built-ins in `builtinPresets` order, the
 * presets for all repositories, this repository's, then its Custom. */
export function presetList(settings: AppSettings, repo: RepoSettings | undefined): PresetEntry[] {
  const slots = settings.builtinPresets.filter((s) => BUILTIN_PRESETS.some((b) => b.id === s.id));
  // A built-in the settings don't list (an older file) still shows, ticked, after the listed ones.
  const missing = BUILTIN_PRESETS.filter((b) => !slots.some((s) => s.id === b.id)).map((b) => ({ id: b.id, inCycle: true }));
  const builtins = [...slots, ...missing].map((s): PresetEntry => ({ preset: BUILTIN_PRESETS.find((b) => b.id === s.id)!, group: 'builtin', inCycle: s.inCycle, readOnly: true }));
  const own = (preset: BranchPreset, group: PresetGroup): PresetEntry => ({ preset, group, inCycle: preset.inCycle, readOnly: false });
  return [
    ...builtins,
    ...(settings.branchPresets ?? []).map((p) => own(p, 'all')),
    ...(repo?.branchPresets ?? []).map((p) => own(p, 'repo')),
    ...(repo?.customPreset ? [own(repo.customPreset, 'repo')] : []),
  ];
}

/** The repository's current preset; All when it names none, or one that's gone (a deleted
 * global preset, a profile copied from elsewhere). */
export function currentPreset(list: PresetEntry[], repo: RepoSettings | undefined): PresetEntry {
  const id = repo?.branchPreset ?? ALL_ID;
  return list.find((e) => e.preset.id === id) ?? list.find((e) => e.preset.id === ALL_ID)!;
}

/** The ticked preset after `currentId` in list order, wrapping; null when no other is ticked. */
export function nextInCycle(list: PresetEntry[], currentId: string): PresetEntry | null {
  const at = list.findIndex((e) => e.preset.id === currentId);
  for (let i = 1; i <= list.length; i++) {
    const e = list[(at + i + list.length) % list.length];
    if (e.inCycle && e.preset.id !== currentId) return e;
  }
  return null;
}

/** What a graph request carries for `entry`: null for All, so its graph is today's. */
export function viewSpecOf(entry: PresetEntry, repo: RepoSettings | undefined, expanded: readonly string[], placement: CollapsePlacement): ViewSpec | null {
  const p = entry.preset;
  if (p.id === ALL_ID) return null;
  return {
    rules: p.rules, otherwise: p.otherwise, picks: repo?.branchPicks?.[p.id] ?? NO_PICKS, foldMerged: p.foldMerged, ghostLines: p.ghostLines, ghostPicks: p.ghostPicks,
    trunkPatterns: repo?.trunkPatterns ?? [], expanded: [...expanded], placement,
  };
}

const without = (list: string[], name: string) => list.filter((n) => n !== name);

/** Hide or show one branch; on a read-only preset, switches to Custom (made from it if none).
 * Hiding one that a folder pick hides (an exception shown inside it) only drops the exception. */
export function pickBranch(settings: AppSettings, profile: Profile, repoPath: string, name: string, show: boolean): Profile {
  return editPicks(settings, profile, repoPath, (old) => {
    if (show) return { shown: [...without(old.shown, name), name], hidden: without(old.hidden, name) };
    const rest = { shown: without(old.shown, name), hidden: without(old.hidden, name) };
    return folderPickHides(rest, name.slice(0, name.lastIndexOf('/') + 1)) ? rest : { ...rest, hidden: [...rest.hidden, name] };
  });
}

/** Whether the folder picks hide `folder` (a unit-name prefix with its slash, `deployed-commit/`):
 * as in the core, the nearest folder with a pick, from `folder` itself up, decides. */
export function folderPickHides(picks: BranchPicks | undefined, folder: string): boolean {
  if (!picks) return false;
  for (let i = folder.length - 1; i >= 0; i = folder.lastIndexOf('/', i - 1)) {
    const f = folder.slice(0, i + 1);
    if (picks.shown.includes(f)) return false;
    if (picks.hidden.includes(f)) return true;
  }
  return false;
}

/**
 * Hide or show every branch under `folder` (`deployed-commit/`), also those created later; on a
 * read-only preset, switches to Custom. Hiding replaces every pick under it, its own included,
 * with the folder in `hidden` (none when a folder above already hides it); a branch shown by hand
 * inside it afterwards is an exception the core lets win. Showing drops the hidden picks under it,
 * and adds the folder to `shown` only when that can't bring it back: a folder above still hides
 * it, or no pick hid it (the preset's rules do).
 */
export function pickFolder(settings: AppSettings, profile: Profile, repoPath: string, folder: string, show: boolean): Profile {
  const outside = (n: string) => !n.startsWith(folder);
  return editPicks(settings, profile, repoPath, (old) => {
    if (!show) {
      const rest = { shown: old.shown.filter(outside), hidden: old.hidden.filter(outside) };
      return folderPickHides(rest, folder) ? rest : { ...rest, hidden: [...rest.hidden, folder] };
    }
    const hidden = old.hidden.filter(outside);
    const pin = hidden.length === old.hidden.length || folderPickHides({ shown: old.shown, hidden }, folder);
    return pin ? { shown: [...old.shown.filter(outside), folder], hidden } : { shown: old.shown, hidden };
  });
}

/** Changes the current preset's picks; on a read-only preset, switches to Custom (made from it if none). */
function editPicks(settings: AppSettings, profile: Profile, repoPath: string, edit: (old: BranchPicks) => BranchPicks): Profile {
  let repo = profile.repos[repoPath] ?? NO_REPO;
  const cur = currentPreset(presetList(settings, repo), repo);
  let id = cur.preset.id;
  if (cur.readOnly) {
    id = CUSTOM_ID;
    if (!repo.customPreset) {
      // A fresh Custom starts with no picks: any left under its id belonged to one renamed away.
      const { [CUSTOM_ID]: _stale, ...picks } = repo.branchPicks ?? {};
      repo = { ...repo, customPreset: { ...cur.preset, id: CUSTOM_ID, name: 'Custom', scope: 'repo', inCycle: true }, branchPicks: picks };
    }
    repo = { ...repo, branchPreset: CUSTOM_ID };
  }
  repo = { ...repo, branchPicks: { ...repo.branchPicks, [id]: edit(repo.branchPicks?.[id] ?? NO_PICKS) } };
  return { ...profile, repos: { ...profile.repos, [repoPath]: repo } };
}
