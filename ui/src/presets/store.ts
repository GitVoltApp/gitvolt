import { useMemo } from 'react';
import { create } from 'zustand';
import type { ViewSpec } from '../api/gen/ViewSpec';
import { tabsForRepo, useRuntime } from '../app/runtime';
import { useAppState } from '../app/state';
import { ALL_ID } from './builtins';
import { currentPreset, nextInCycle, pickBranch, presetList, viewSpecOf, type PresetEntry } from './model';

/** Session state, in memory only: what "Expand in graph" (or going to a hidden commit) unfolded,
 * per tab, until the tab switches presets. */
export const usePresetSession = create<{ expanded: Record<string, string[]> }>(() => ({ expanded: {} }));

const tabRepo = (tabId: string) => useRuntime.getState().tabs[tabId]?.repo ?? null;

function resolve(tabId: string) {
  const repo = tabRepo(tabId);
  if (!repo) return null;
  const { settings, profile } = useAppState.getState();
  const rs = profile.repos[repo.path];
  const list = presetList(settings, rs);
  return { repo, rs, list, current: currentPreset(list, rs) };
}

/** What the tab's next `graph` request carries: null under All. */
export function viewSpecFor(tabId: string): ViewSpec | null {
  const r = resolve(tabId);
  if (!r) return null;
  return viewSpecOf(r.current, r.rs, usePresetSession.getState().expanded[tabId] ?? [], useAppState.getState().settings.collapsePlacement);
}

/** The preset is the repository's, so every tab on it reloads; a tab whose preset changed loses
 * its expansions. `drop`: a key no tab may keep expanded (a branch just hidden by hand). */
function reload(repoId: number, switched: boolean, drop?: string): void {
  const tabs = tabsForRepo(repoId);
  usePresetSession.setState((s) => {
    const expanded = { ...s.expanded };
    for (const t of tabs) {
      if (switched) delete expanded[t];
      else if (drop && expanded[t]?.includes(drop)) expanded[t] = expanded[t].filter((k) => k !== drop);
    }
    return { expanded };
  });
  for (const t of tabs) void useRuntime.getState().refresh(t, { graphOnly: true });
}

/** Runs `edit` (Settings changing presets) and reloads the graph of every tab whose view it changed. */
export function editingViews(edit: () => void): void {
  const tabs = Object.keys(useRuntime.getState().tabs);
  const before = new Map(tabs.map((t) => [t, JSON.stringify(viewSpecFor(t))]));
  edit();
  for (const t of tabs) if (JSON.stringify(viewSpecFor(t)) !== before.get(t)) void useRuntime.getState().refresh(t, { graphOnly: true });
}

/** Reloads the graph of every tab under a preset (on `repoPath` only, if given): a setting its view carries changed. */
export function refreshPresetViews(repoPath?: string): void {
  for (const [t, rt] of Object.entries(useRuntime.getState().tabs)) {
    if (rt.repo && (!repoPath || rt.repo.path === repoPath) && viewSpecFor(t)) void useRuntime.getState().refresh(t, { graphOnly: true });
  }
}

/** Makes `id` the repository's current preset (saved per repository) and reloads its graph. */
export function setPreset(tabId: string, id: string): void {
  const r = resolve(tabId);
  if (!r || r.current.preset.id === id) return;
  useAppState.getState().updateRepo(r.repo.path, (rs) => ({ ...rs, branchPreset: id === ALL_ID ? undefined : id }));
  reload(r.repo.id, true);
}

/** The button's click: the next ticked preset. */
export function cyclePreset(tabId: string): void {
  const r = resolve(tabId);
  const next = r && nextInCycle(r.list, r.current.preset.id);
  if (next) setPreset(tabId, next.preset.id);
}

function pick(tabId: string, names: string[], show: boolean): void {
  const r = resolve(tabId);
  if (!r || names.length === 0) return;
  const app = useAppState.getState();
  app.updateProfile((p) => names.reduce((acc, name) => pickBranch(app.settings, acc, r.repo.path, name, show), p));
  reload(r.repo.id, resolve(tabId)?.current.preset.id !== r.current.preset.id, show ? undefined : `branch:${names[0]}`);
}

/** Hides one branch by hand; on a built-in preset, in the repository's Custom. */
export const hideBranch = (tabId: string, name: string): void => pick(tabId, [name], false);
export const showBranch = (tabId: string, name: string): void => pick(tabId, [name], true);
/** Shows several branches in one pick update (the ghost popover's "Show all"): one save, one reload. */
export const showBranches = (tabId: string, names: string[]): void => pick(tabId, names, true);

/** The `expanded` key that unfolds what `key` names: a collapsed run's own key
 * ("branch:<name>@<newest sha>", one per line of history) unfolds its whole branch. */
export const expandKeyOf = (key: string): string => (key.startsWith('branch:') ? key.replace(/@[0-9a-f]{40}$/, '') : key);

/** Whether `key` (any of a branch's run keys, or its own) is unfolded in this tab. */
export const isExpanded = (tabId: string, key: string): boolean => (usePresetSession.getState().expanded[tabId] ?? []).includes(expandKeyOf(key));

/** Unfolds `key` ("branch:<name>", a run's key, or "fold:<sha>") in this tab until it switches
 * presets or `collapse` folds it again. Settles once the tab's graph has reloaded with it. */
export function expand(tabId: string, key: string): Promise<void> {
  const k = expandKeyOf(key);
  const cur = usePresetSession.getState().expanded[tabId] ?? [];
  if (cur.includes(k)) return Promise.resolve();
  usePresetSession.setState((s) => ({ expanded: { ...s.expanded, [tabId]: [...cur, k] } }));
  return useRuntime.getState().refresh(tabId, { graphOnly: true });
}

/** Undoes `expand`: the preset decides `key` again. Settles once the tab's graph has reloaded. */
export function collapse(tabId: string, key: string): Promise<void> {
  const k = expandKeyOf(key);
  const cur = usePresetSession.getState().expanded[tabId] ?? [];
  if (!cur.includes(k)) return Promise.resolve();
  usePresetSession.setState((s) => ({ expanded: { ...s.expanded, [tabId]: cur.filter((x) => x !== k) } }));
  return useRuntime.getState().refresh(tabId, { graphOnly: true });
}

export function useCurrentPreset(tabId: string): { current: PresetEntry; list: PresetEntry[]; next: PresetEntry | null } {
  const path = useRuntime((s) => s.tabs[tabId]?.repo?.path);
  const settings = useAppState((s) => s.settings);
  const rs = useAppState((s) => (path ? s.profile.repos[path] : undefined));
  return useMemo(() => {
    const list = presetList(settings, rs);
    const current = currentPreset(list, rs);
    return { current, list, next: nextInCycle(list, current.preset.id) };
  }, [settings, rs]);
}

let editPresets: (() => void) | null = null;
/** The settings page registers what the ▾ menu's "Edit presets…" opens; until then there's no such row. */
export function registerEditPresets(open: () => void): () => void {
  editPresets = open;
  return () => { if (editPresets === open) editPresets = null; };
}
export const editPresetsAction = (): (() => void) | null => editPresets;
