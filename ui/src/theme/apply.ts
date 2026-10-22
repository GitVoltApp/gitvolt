import type { CustomTheme } from '../api/gen/CustomTheme';
import { contrastRatio } from './contrast';
import { isCustomThemeId, opaqueHex } from './custom';
import { COLOR_TOKENS, isThemeId, type ThemeDef } from './themes';

/** What the canvases draw with: the theme's node and shade colours and its lanes; and the
 * initials colour for each lane (avatars). */
export interface ResolvedColors { nodeFill: string; avatarBackdrop: string; nodeText: string; collapseStrip: string; graph: string[]; laneText: string[] }

const HEX = /^#[0-9a-f]{6}$/i;
export const isHexColor = (v: unknown): v is string => typeof v === 'string' && HEX.test(v);

/** The theme's canvas colors. */
export function resolveColors(def: ThemeDef): ResolvedColors {
  const graph = [...def.graph];
  // White or the theme's dark ink, whichever reads better on the lane (a pastel lane takes dark).
  const ink = def.kind === 'dark' ? def.colors['app-bg0'] : def.colors['text-selected'];
  const laneText = graph.map((c) => def.laneText ?? (contrastRatio('#ffffff', c) >= contrastRatio(ink, c) ? '#ffffff' : ink));
  return { nodeFill: def.colors['node-fill'], avatarBackdrop: def.colors['avatar-backdrop'], nodeText: def.colors['node-text'], collapseStrip: def.colors['collapse-strip'], graph, laneText };
}

/** Writes the theme onto `root` as inline custom properties (which win over tokens.css). */
export function applyTheme(root: HTMLElement, def: ThemeDef, colors: ResolvedColors): void {
  for (const t of COLOR_TOKENS) root.style.setProperty(`--${t}`, def.colors[t]);
  colors.graph.forEach((c, i) => root.style.setProperty(`--graph-${i}`, c));
  root.style.colorScheme = def.kind;
  root.dataset.theme = def.id;
}

/**
 * The first-paint mirror (ruling R3): the saved theme, kept in localStorage so it applies before
 * the backend's settings arrive, the way zoom and density do (1C R4). `AppSettings.theme` stays the
 * source of truth; this only shadows it. It also carries the theme's `kind` and `app-bg0` (`bg`)
 * for index.html's inline script, which paints them before any CSS or module script runs (no dark
 * flash for a light theme, review I1); and a custom theme itself, so it resolves before the
 * settings load. `graph` is a built-in's lane overrides, as mirrors written before custom themes
 * carry them (read only: the settings' migration replaces them with a custom theme).
 */
export const THEME_STORAGE_KEY = 'gitvolt.theme.v1';
export interface ThemeMirror { id: string; custom?: CustomTheme; graph?: (string | null)[] }

const isCustomTheme = (v: unknown, id: string): v is CustomTheme => {
  const c = v as CustomTheme | null;
  return !!c && typeof c === 'object' && c.id === id && typeof c.name === 'string' && typeof c.base === 'string'
    && !!c.colors && typeof c.colors === 'object' && !Array.isArray(c.colors) && Array.isArray(c.graph);
};

export function readThemeMirror(): ThemeMirror | null {
  try {
    const v = JSON.parse(globalThis.localStorage.getItem(THEME_STORAGE_KEY) ?? 'null') as { id?: unknown; graph?: unknown; custom?: unknown } | null;
    if (!v || typeof v.id !== 'string') return null;
    if (isCustomThemeId(v.id)) return isCustomTheme(v.custom, v.id) ? { id: v.id, custom: v.custom } : null;
    if (!isThemeId(v.id)) return null;
    return { id: v.id, graph: Array.isArray(v.graph) ? v.graph.map((c) => (isHexColor(c) ? c : null)) : undefined };
  } catch {
    return null;
  }
}

/** Mirrors `def`, the saved theme; `custom` is its custom theme when it's one. */
export function writeThemeMirror(def: ThemeDef, custom?: CustomTheme): void {
  try {
    const bg = opaqueHex(def.colors['app-bg0']) ?? undefined;
    globalThis.localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify({ id: def.id, kind: def.kind, bg, ...(custom && { custom }) }));
  } catch {
    // Blocked or full storage: the next start paints Default Dark until the settings load.
  }
}
