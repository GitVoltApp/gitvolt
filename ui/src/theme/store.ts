import { create } from 'zustand';
import { applyTheme, resolveColors, type ResolvedColors } from './apply';
import { DEFAULT_THEME_ID, THEMES, type ColorToken, type ThemeDef } from './themes';

/** Unsaved colours shown over the theme while the user edits one (a picker being dragged). */
export type ThemePatch = { colors?: Partial<Record<ColorToken, string>>; graph?: readonly string[] };

interface ThemeState {
  id: string;
  /** The theme on screen: the applied one, with any preview over it. */
  def: ThemeDef;
  /** Bumped on every applied change. Canvas consumers depend on `colors`, which changes with it. */
  version: number;
  colors: ResolvedColors;
  set(def: ThemeDef): void;
  /** Shows `patch` over the applied theme without saving it; `null` goes back to the theme. */
  preview(patch: ThemePatch | null): void;
}

const withPatch = (def: ThemeDef, p: ThemePatch): ThemeDef => ({
  ...def,
  colors: { ...def.colors, ...p.colors } as ThemeDef['colors'],
  graph: p.graph ? [...p.graph] : def.graph,
});

/** The applied theme, without the preview. */
let applied: ThemeDef = THEMES[DEFAULT_THEME_ID];

/**
 * The applied theme: `set` writes it onto <html> (the DOM follows through the custom
 * properties) and publishes the resolved canvas colours, which the graph canvas, the ref chips
 * and the avatars read, so a switch repaints them without a reload. `bind.ts` drives it from the
 * settings; nothing else should call `set` but tests.
 */
export const useTheme = create<ThemeState>((set, get) => {
  const show = (def: ThemeDef) => {
    const colors = resolveColors(def);
    applyTheme(document.documentElement, def, colors);
    set({ id: def.id, def, colors, version: get().version + 1 });
  };
  return {
    id: DEFAULT_THEME_ID,
    def: applied,
    version: 0,
    colors: resolveColors(applied),
    set(def) {
      applied = def;
      show(def);
    },
    preview(patch) {
      show(patch ? withPatch(applied, patch) : applied);
    },
  };
});
