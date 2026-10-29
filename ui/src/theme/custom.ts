import type { CustomTheme } from '../api/gen/CustomTheme';
import { formatColour, isCssColour, parseColour } from '../ui/colour/colour';
import { COLOR_TOKENS, CORE_TOKENS, DEFAULT_THEME_ID, derive, DERIVED_TOKENS, isThemeId, THEMES, type ColorToken, type ThemeDef } from './themes';

/**
 * Custom themes (Settings › Appearance): a built-in base plus the colours the user set, resolved
 * into the same `ThemeDef` the built-ins are, so everything that reads a theme works unchanged.
 */

const CUSTOM_ID = /^custom-[0-9a-f]{8}$/;
export const isCustomThemeId = (id: string): boolean => CUSTOM_ID.test(id);

export function newCustomThemeId(): string {
  const b = crypto.getRandomValues(new Uint8Array(4));
  return `custom-${[...b].map((x) => x.toString(16).padStart(2, '0')).join('')}`;
}

/** `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(r, g, b)` or `rgba(r, g, b, a)` in range; nothing else
 * (Review Focus 1: only these may reach a custom property). */
export const isTokenColour = (v: unknown): v is string => isCssColour(v) && v === v.trim();

/** A valid colour as opaque `#rrggbb` (its alpha dropped): what `derive`'s maths and the first-paint
 * script read; `null` for anything else. */
export function opaqueHex(v: string): string | null {
  const c = isTokenColour(v) ? parseColour(v) : null;
  return c && formatColour({ ...c, a: 1 });
}

type Core = Parameters<typeof derive>[0];
const isLaneColour = (c: string) => /^#[0-9a-f]{6}$/i.test(c);

export function customThemeDef(ct: CustomTheme): ThemeDef {
  const baseId = isThemeId(ct.base) ? ct.base : DEFAULT_THEME_ID;
  const base = THEMES[baseId];
  const set: Partial<Record<ColorToken, string>> = Object.fromEntries(
    Object.entries(ct.colors).filter(([t, v]) => (COLOR_TOKENS as readonly string[]).includes(t) && isTokenColour(v)),
  );
  const core = Object.fromEntries(CORE_TOKENS.map((t) => [t, set[t] ?? base.colors[t]])) as Core;
  // derive's maths takes `#rrggbb`: a set colour in another form goes in as its opaque hex.
  const coreForDerive = Object.fromEntries(CORE_TOKENS.map((t) => [t, set[t] ? opaqueHex(set[t])! : base.colors[t]])) as Core;
  const baseDerived = derive(Object.fromEntries(CORE_TOKENS.map((t) => [t, base.colors[t]])) as Core, base.kind);
  const nowDerived = derive(coreForDerive, base.kind);
  // A derived token the base set by hand (≠ what derive gives it) keeps the base's value; the rest follow the edit.
  const derived = Object.fromEntries(DERIVED_TOKENS.map((t) => [t, base.colors[t] === baseDerived[t] ? nowDerived[t] : base.colors[t]]));
  return {
    id: ct.id, label: ct.name, kind: base.kind, editorTheme: base.editorTheme, base: baseId, laneText: base.laneText,
    colors: { ...core, ...derived, ...set } as Record<ColorToken, string>,
    graph: ct.graph.length === 10 && ct.graph.every(isLaneColour) ? ct.graph.map((c) => c.toLowerCase()) : base.graph,
  };
}

/** The theme `id` names: a built-in, else that custom theme, else (a custom id `customs` lacks) Default Dark. */
export function themeDef(id: string, customs: readonly CustomTheme[]): ThemeDef {
  if (isThemeId(id)) return THEMES[id];
  const ct = customs.find((c) => c.id === id);
  return ct ? customThemeDef(ct) : THEMES[DEFAULT_THEME_ID];
}

export type ColourGroup = 'Surfaces' | 'Text' | 'Diff' | 'Status';

/** The 12 core colours the editor shows first; the rest derive from them (or "All colours"). */
export const CORE_COLOURS: ReadonlyArray<{ group: ColourGroup; token: ColorToken; label: string; note?: string }> = [
  { group: 'Surfaces', token: 'app-bg0', label: 'Window background' },
  { group: 'Surfaces', token: 'panel-bg0', label: 'Panels', note: 'sidebar, details' },
  { group: 'Surfaces', token: 'panel-bg1', label: 'Menus and popovers' },
  { group: 'Text', token: 'text-normal', label: 'Text' },
  { group: 'Text', token: 'text-secondary', label: 'Secondary text' },
  { group: 'Text', token: 'blue', label: 'Accent', note: 'selection, links, focus' },
  { group: 'Diff', token: 'diff-inserted-line', label: 'Added lines' },
  { group: 'Diff', token: 'diff-removed-line', label: 'Removed lines' },
  { group: 'Status', token: 'green', label: 'Added, success' },
  { group: 'Status', token: 'red', label: 'Deleted, danger' },
  { group: 'Status', token: 'orange', label: 'Modified, warning' },
  { group: 'Status', token: 'yellow', label: 'Attention' },
];

/** A readable name per token, for "All colours". */
export const TOKEN_LABELS: Record<ColorToken, string> = {
  'app-bg0': 'Window background', 'panel-bg0': 'Panels', 'panel-bg1': 'Menus and popovers', 'toolbar-bg0': 'Status bar',
  'action-bar-bg': 'Action bar', 'tab-bar-bg': 'Tab bar', 'file-bar-bg': 'File bar',
  'text-selected': 'Brightest text', 'text-normal': 'Text', 'text-secondary': 'Secondary text', 'text-disabled': 'Disabled text',
  'text-on-accent': 'Text on accent',
  'stats-added': 'Added count', 'stats-deleted': 'Deleted count',
  red: 'Red', orange: 'Orange', yellow: 'Yellow', green: 'Green', blue: 'Accent', link: 'Links',
  'count-blue': 'Sidebar counts', 'checked-out-row': 'Checked-out row', 'checked-out': 'Checked-out mark', 'selection-bg': 'Text selection',
  'conflict-ours': 'Conflict: ours', 'conflict-theirs': 'Conflict: theirs', 'conflict-base': 'Conflict: base',
  'wip-input-bg': 'Commit message box', 'wip-input-border': 'Commit message box border', 'wip-input-placeholder': 'Commit message placeholder',
  'hover-row': 'Row under the pointer', 'selected-row': 'Selected row', 'section-border': 'Borders', 'focus-outline': 'Focus outline',
  'text-row-dimmed': 'Dimmed row', 'text-row-dimmed-branch': 'Dimmed branch', 'sidebar-icon': 'Sidebar icons', 'avatar-backdrop': 'Behind avatars',
  'stats-added-bright': 'Added total', 'stats-deleted-bright': 'Deleted total',
  'status-added': 'Added file', 'status-modified': 'Modified file', 'status-deleted': 'Deleted file', 'status-renamed': 'Renamed file', 'status-conflicted': 'Conflicted file',
  'scroll-thumb-bg': 'Scrollbar', 'scroll-thumb-hover-bg': 'Scrollbar under the pointer',
  'control-bg': 'Quiet buttons', 'control-hover-bg': 'Quiet buttons under the pointer', 'image-frame-border': 'Image frame',
  'shadow-modal': 'Dialog shadow', 'shadow-popup': 'Popup shadow', 'shadow-tooltip': 'Tooltip shadow', backdrop: 'Dialog backdrop',
  'node-fill': 'Commit dot fill', 'node-text': 'Commit dot text', 'collapse-strip': 'Collapsed commits shade',
  'conflict-ours-tick': 'Conflict tick: ours', 'conflict-theirs-tick': 'Conflict tick: theirs',
  'diff-inserted-text': 'Added text', 'diff-removed-text': 'Removed text', 'diff-inserted-line': 'Added lines', 'diff-removed-line': 'Removed lines',
  'diff-diagonal-fill': 'Past the end of a side',
  'md-diff-ins-text': 'Markdown: added words', 'md-diff-del-text': 'Markdown: removed words', 'md-diff-ins-block': 'Markdown: added blocks', 'md-diff-del-block': 'Markdown: removed blocks',
  'forge-gitlab': 'GitLab mark', 'forge-github': 'GitHub mark', 'forge-forgejo': 'Forgejo mark', purple: 'Merged',
  'switch-on': 'Switch on', 'switch-off': 'Switch off',
  'tg-ink': 'Tab group text', 'tg-blue': 'Tab group: blue', 'tg-purple': 'Tab group: purple', 'tg-cyan': 'Tab group: cyan', 'tg-orange': 'Tab group: orange',
  'tg-yellow': 'Tab group: yellow', 'tg-pink': 'Tab group: pink', 'tg-green': 'Tab group: green', 'tg-gray': 'Tab group: grey', 'tg-red': 'Tab group: red',
  'button-primary': 'Main button', 'button-warn': 'Warning button', 'button-warn-ink': 'Warning button text',
  'review-pending': 'Pending review', 'review-pending-bg': 'Pending review background',
};

export type TokenGroup = ColourGroup | 'Graph and canvas' | 'Forge and tabs' | 'Other';

/** Every token under a group, in the order "All colours" lists them. */
export const TOKEN_GROUPS: ReadonlyArray<{ group: TokenGroup; tokens: readonly ColorToken[] }> = [
  { group: 'Surfaces', tokens: ['app-bg0', 'panel-bg0', 'panel-bg1', 'toolbar-bg0', 'action-bar-bg', 'tab-bar-bg', 'file-bar-bg', 'hover-row', 'selected-row', 'section-border', 'control-bg', 'control-hover-bg', 'scroll-thumb-bg', 'scroll-thumb-hover-bg', 'shadow-modal', 'shadow-popup', 'shadow-tooltip', 'backdrop'] },
  { group: 'Text', tokens: ['text-selected', 'text-normal', 'text-secondary', 'text-disabled', 'text-on-accent', 'blue', 'link', 'focus-outline', 'selection-bg', 'sidebar-icon', 'count-blue', 'wip-input-bg', 'wip-input-border', 'wip-input-placeholder'] },
  { group: 'Diff', tokens: ['diff-inserted-text', 'diff-removed-text', 'diff-inserted-line', 'diff-removed-line', 'diff-diagonal-fill', 'md-diff-ins-text', 'md-diff-del-text', 'md-diff-ins-block', 'md-diff-del-block', 'conflict-ours', 'conflict-theirs', 'conflict-base', 'conflict-ours-tick', 'conflict-theirs-tick'] },
  { group: 'Status', tokens: ['green', 'red', 'orange', 'yellow', 'stats-added', 'stats-deleted', 'stats-added-bright', 'stats-deleted-bright', 'status-added', 'status-modified', 'status-deleted', 'status-renamed', 'status-conflicted'] },
  { group: 'Graph and canvas', tokens: ['node-fill', 'node-text', 'collapse-strip', 'avatar-backdrop', 'text-row-dimmed', 'text-row-dimmed-branch', 'checked-out-row', 'checked-out'] },
  { group: 'Forge and tabs', tokens: ['forge-gitlab', 'forge-github', 'forge-forgejo', 'purple', 'review-pending', 'review-pending-bg', 'tg-ink', 'tg-blue', 'tg-purple', 'tg-cyan', 'tg-orange', 'tg-yellow', 'tg-pink', 'tg-green', 'tg-gray', 'tg-red'] },
  { group: 'Other', tokens: ['image-frame-border', 'switch-on', 'switch-off', 'button-primary', 'button-warn', 'button-warn-ink'] },
];
