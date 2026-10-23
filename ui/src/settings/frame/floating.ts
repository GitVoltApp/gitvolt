import { useSyncExternalStore, type ComponentType } from 'react';

const floats = new Map<string, ComponentType>();
let version = 0;
const listeners = new Set<() => void>();
const changed = () => {
  version++;
  for (const l of [...listeners]) l();
};

/**
 * A floating panel Settings can turn into (the theme editor's "Edit mode"): while
 * `useSettingsUi.floating === id`, the overlay renders only `Component`, with no backdrop, no key
 * claim and no click-outside close, so the app behind stays usable. The panel handles its own keys
 * and calls `setFloating(null)` to bring Settings back.
 */
export function registerSettingsFloat(id: string, Component: ComponentType): () => void {
  if (floats.has(id)) throw new Error(`settings float ${id} is already registered`);
  floats.set(id, Component);
  changed();
  return () => {
    if (floats.get(id) === Component) floats.delete(id);
    changed();
  };
}

const subscribe = (cb: () => void) => {
  listeners.add(cb);
  return () => { listeners.delete(cb); };
};
const getVersion = () => version;
export function useSettingsFloat(id: string | null): ComponentType | null {
  useSyncExternalStore(subscribe, getVersion);
  return id ? floats.get(id) ?? null : null;
}
