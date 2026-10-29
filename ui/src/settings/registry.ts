import type { LucideIcon } from 'lucide-react';
import { useSyncExternalStore, type ComponentType } from 'react';
import type { SettingBinding } from './bindings';

/**
 * Settings' registry: the pages (in three scopes) and the settings on them. The overlay, its nav,
 * search, the modified dots, Reset all and the palette's `#` group are all built from it, so a
 * feature adds a page or a setting from its own module without editing the frame:
 *
 *   const off = [registerSettingsPage({ id: 'branch-presets', scope: 'app', order: 40, … }),
 *                registerSettings([{ id: 'collapsedHover', page: 'graph', group: 'Hidden commits', … }])];
 *   import.meta.hot?.dispose(() => off.forEach((f) => f()));
 *
 * A setting may name a page another module registers (or one not registered yet); it shows once
 * that page is there. Within a group, registration order is display order. Page ids in use:
 * general, appearance, theme, graph, branch-presets, diffs, fetch, keyboard, advanced (app);
 * git-identity, editor-folders, accounts, hosts (profile); repository, file-colours (repo).
 */
export type SettingsScope = 'app' | 'profile' | 'repo';
/** What a page or control is about: the shown tab's repository (repo pages), if any. */
export interface SettingsCtx { repoPath: string | null; repoName: string | null; tabId: string | null }
export interface SettingsPageDef {
  id: string; label: string; scope: SettingsScope; order: number; icon: LucideIcon;
  /** One line under the title; a function when it names the repository ("Settings for orbital only."). */
  summary: string | ((ctx: SettingsCtx) => string);
  /** Replaces the default body (the page's settings as rows, in titled groups, in registration order). */
  Body?: ComponentType<{ ctx: SettingsCtx }>;
}
export interface SettingDef {
  id: string; label: string; page: string; group: string; keywords: string;
  description?: string;
  /** The row's control (the page and search results render it). None: the row offers "Open page". */
  Control?: ComponentType<{ ctx: SettingsCtx }>;
  /** 'input' (default): the label is a <label for="input-<id>">; 'group': the control is labelled by `label-<id>`. */
  labelFor?: 'input' | 'group';
  /** Its value and default, for the modified dot, ↺ and Reset all. None: never marked (lists, accounts). */
  binding?: (ctx: SettingsCtx) => SettingBinding | null;
}

const SCOPE_RANK: Record<SettingsScope, number> = { app: 0, profile: 1, repo: 2 };
/** Nav order: GitVolt, then the profile, then the repository; `order` within each. */
export const byNavOrder = (a: SettingsPageDef, b: SettingsPageDef) => SCOPE_RANK[a.scope] - SCOPE_RANK[b.scope] || a.order - b.order;

const pages = new Map<string, SettingsPageDef>();
const settings = new Map<string, SettingDef>();
let sortedPages: readonly SettingsPageDef[] = [];
let settingList: readonly SettingDef[] = [];
let version = 0;
const listeners = new Set<() => void>();
function changed() {
  sortedPages = [...pages.values()].sort(byNavOrder);
  settingList = [...settings.values()];
  version++;
  for (const l of [...listeners]) l();
}

export function registerSettingsPage(page: SettingsPageDef): () => void {
  if (pages.has(page.id)) throw new Error(`settings page ${page.id} is already registered`);
  pages.set(page.id, page);
  changed();
  return () => {
    if (pages.get(page.id) === page) pages.delete(page.id);
    changed();
  };
}

export function registerSettings(defs: readonly SettingDef[]): () => void {
  const ids = new Set<string>();
  for (const d of defs) {
    if (settings.has(d.id) || ids.has(d.id)) throw new Error(`setting ${d.id} is already registered`);
    ids.add(d.id);
  }
  for (const d of defs) settings.set(d.id, d);
  changed();
  return () => {
    for (const d of defs) if (settings.get(d.id) === d) settings.delete(d.id);
    changed();
  };
}

export const settingsPages = (): readonly SettingsPageDef[] => sortedPages;
export const allSettings = (): readonly SettingDef[] => settingList;
export const settingDef = (id: string): SettingDef | undefined => settings.get(id);
export const settingsPage = (id: string): SettingsPageDef | undefined => pages.get(id);

function subscribe(cb: () => void): () => void {
  listeners.add(cb);
  return () => { listeners.delete(cb); };
}
const getVersion = () => version;
/** A version, bumped on (un)registration: views built from the registry re-render with it. */
export function useSettingsRegistry(): number {
  return useSyncExternalStore(subscribe, getVersion);
}

/** The nav group's and breadcrumbs' name for a scope: GitVolt, `Profile · Work`, `Repository · orbital`. */
export function scopeLabel(scope: SettingsScope, profileName: string, repoName: string | null): string {
  if (scope === 'app') return 'GitVolt';
  if (scope === 'profile') return `Profile · ${profileName}`;
  return `Repository · ${repoName ?? ''}`;
}

export const pageSummary = (page: SettingsPageDef, ctx: SettingsCtx): string => (typeof page.summary === 'function' ? page.summary(ctx) : page.summary);
