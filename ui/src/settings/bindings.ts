import { useMemo, useSyncExternalStore } from 'react';
import type { AppSettings } from '../api/gen/AppSettings';
import type { Profile } from '../api/gen/Profile';
import type { RepoSettings } from '../api/gen/RepoSettings';
import { EMPTY_REPO_SETTINGS, useAppState } from '../app/state';
import { allSettings, settingsPage, type SettingDef, type SettingsCtx } from './registry';

/** A setting's value where it's stored (app settings, the profile, a localStorage store) and its
 * default: what the ↺ and Reset all read and write. */
export interface SettingBinding<T = unknown> { get(): T; set(v: T): void; defaultValue: T; subscribe(cb: () => void): () => void; equals?(a: T, b: T): boolean }

// An absent optional field (`branchUpdate`) is its null default.
const json = (v: unknown) => JSON.stringify(v === undefined ? null : v);
export function isModified(b: SettingBinding): boolean {
  return b.equals ? !b.equals(b.get(), b.defaultValue) : json(b.get()) !== json(b.defaultValue);
}

const noSubscribe = () => () => {};
const notModified = () => false;

export function useModified(def: SettingDef, ctx: SettingsCtx): boolean {
  const b = useMemo(() => def.binding?.(ctx) ?? null, [def, ctx]);
  return useSyncExternalStore(b ? b.subscribe : noSubscribe, b ? () => isModified(b) : notModified);
}

/** A setting's live value and its setter, through its binding (for controls built from it). */
export function useSettingValue<T>(def: SettingDef, ctx: SettingsCtx): [T, (v: T) => void] {
  const b = useMemo(() => def.binding?.(ctx) ?? null, [def, ctx]) as SettingBinding<T> | null;
  if (!b) throw new Error(`setting ${def.id} has no value here`);
  return [useSyncExternalStore(b.subscribe, b.get), b.set];
}

export function resetSetting(def: SettingDef, ctx: SettingsCtx): void {
  const b = def.binding?.(ctx);
  if (b && isModified(b)) b.set(b.defaultValue);
}

/** Reset all (Advanced): every setting on an app-scope page that has a binding; never profile/repo ones. */
export function resetAppSettings(ctx: SettingsCtx): void {
  for (const d of allSettings()) if (settingsPage(d.page)?.scope === 'app') resetSetting(d, ctx);
}

const subscribeAppState = (cb: () => void) => useAppState.subscribe(cb);

/** An `AppSettings` field. `after`: the side effect a change has (reload the graph, ask for avatars
 * again), run on set and on reset alike. */
export function appSetting<K extends keyof AppSettings>(key: K, defaultValue: AppSettings[K], after?: (v: AppSettings[K]) => void, equals?: (a: AppSettings[K], b: AppSettings[K]) => boolean): (ctx: SettingsCtx) => SettingBinding<AppSettings[K]> {
  return () => ({
    get: () => useAppState.getState().settings[key],
    set: (v) => {
      useAppState.getState().setSettings({ [key]: v } as Partial<AppSettings>);
      after?.(v);
    },
    defaultValue,
    subscribe: subscribeAppState,
    equals,
  });
}

/** A field of the active profile. */
export function profileField<K extends keyof Profile>(key: K, defaultValue: Profile[K]): (ctx: SettingsCtx) => SettingBinding<Profile[K]> {
  return () => ({
    get: () => useAppState.getState().profile[key],
    set: (v) => useAppState.getState().updateProfile((p) => ({ ...p, [key]: v })),
    defaultValue,
    subscribe: subscribeAppState,
  });
}

/** A field of the shown repository's settings (in the profile); null without a repository. */
export function repoField<K extends keyof RepoSettings>(key: K, defaultValue: RepoSettings[K], after?: (v: RepoSettings[K], ctx: SettingsCtx) => void, equals?: (a: RepoSettings[K], b: RepoSettings[K]) => boolean): (ctx: SettingsCtx) => SettingBinding<RepoSettings[K]> | null {
  return (ctx) => {
    const path = ctx.repoPath;
    if (!path) return null;
    return {
      get: () => (useAppState.getState().profile.repos[path] ?? EMPTY_REPO_SETTINGS)[key],
      set: (v) => {
        useAppState.getState().updateRepo(path, (r) => ({ ...r, [key]: v }));
        after?.(v, ctx);
      },
      defaultValue,
      subscribe: subscribeAppState,
      equals,
    };
  };
}

/** A value kept in a store of its own (density, file list prefs: localStorage). */
export function storeBinding<T>(store: { getState(): unknown; subscribe(cb: () => void): () => void }, get: () => T, set: (v: T) => void, defaultValue: T): () => SettingBinding<T> {
  return () => ({ get, set, defaultValue, subscribe: (cb) => store.subscribe(cb) });
}
