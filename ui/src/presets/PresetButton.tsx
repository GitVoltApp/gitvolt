import { Check, ChevronDown, GitFork, Settings2, type LucideIcon } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { GROUP_COLORS } from '../app/tabGroups';
import { openMenuAt, useMenu } from '../menu/menuStore';
import type { MenuRow } from '../menu/types';
import { HoverTooltip } from '../ui/HoverTooltip';
import { registerKeys } from '../ui/keyRouter';
import type { PresetEntry, PresetGroup } from './model';
import { cyclePreset, editPresetsAction, setPreset, useCurrentPreset } from './store';
import './presets.css';

/** No icon: the other rows line up with the current one's check (as ui/Select does). */
const Blank = (() => null) as unknown as LucideIcon;
const GROUPS: ReadonlyArray<readonly [PresetGroup, string]> = [['builtin', 'Built in'], ['all', 'All repositories'], ['repo', 'This repository']];

/** A preset's colour as CSS; a name outside the palette (a hand-edited file) shows gray. */
export const presetColour = (color: string): string => `var(--tg-${GROUP_COLORS.includes(color as never) ? color : 'gray'})`;

export function presetTooltip(current: PresetEntry, next: PresetEntry | null): string {
  return `Branches: ${current.preset.name} · ${next ? `click for ${next.preset.name} · ` : ''}▾ all presets`;
}

/** The ▾ menu: every preset by group, its colour and name, a check on the current one. */
export function presetMenuRows(tabId: string, list: PresetEntry[], currentId: string): MenuRow[] {
  const rows: MenuRow[] = [];
  for (const [group, label] of GROUPS) {
    const entries = list.filter((e) => e.group === group);
    if (!entries.length) continue;
    if (rows.length) rows.push({ kind: 'separator' });
    rows.push({ kind: 'header', id: `presets.group.${group}`, label });
    for (const { preset: p } of entries) {
      const current = p.id === currentId;
      rows.push({
        kind: 'action', id: `preset.${p.id}`, label: p.name, icon: current ? Check : Blank, swatch: presetColour(p.color),
        tooltip: current ? `The graph shows "${p.name}"` : `Show the graph as "${p.name}"`, run: () => setPreset(tabId, p.id),
      });
    }
  }
  const edit = editPresetsAction();
  if (edit) rows.push({ kind: 'separator' }, { kind: 'action', id: 'presets.edit', label: 'Edit presets…', icon: Settings2, tooltip: 'Open Settings › Branch presets', run: edit });
  return rows;
}

const openers = new Map<string, () => void>();
/** "Choose branch preset…": opens the tab's ▾ menu; false when its button isn't shown. */
export function openPresetMenu(tabId: string): boolean {
  const open = openers.get(tabId);
  open?.();
  return !!open;
}

/**
 * The branch preset button, right of the sidebar's filter: a click moves to the next preset in
 * the cycle, ▾ lists them all. Its icon carries the current preset's colour as a dot. Both parts
 * are real buttons (Enter / Space); ArrowDown on ▾ opens the menu too.
 */
export function PresetButton({ tabId }: { tabId: string }) {
  const { current, list, next } = useCurrentPreset(tabId);
  const caret = useRef<HTMLButtonElement>(null);
  const [focused, setFocused] = useState(false);
  const rows = () => presetMenuRows(tabId, list, current.preset.id);
  const show = () => { if (caret.current) openMenuAt(caret.current, rows(), `preset.${current.preset.id}`, rows, 'Branch presets'); };
  const showRef = useRef(show);
  showRef.current = show;

  useEffect(() => {
    const open = () => showRef.current();
    openers.set(tabId, open);
    return () => { if (openers.get(tabId) === open) openers.delete(tabId); };
  }, [tabId]);

  // The key router claims every key before React sees it, so ▾'s arrows are taken here.
  useEffect(() => {
    if (!focused) return;
    return registerKeys('menu', (e) => {
      if (e.target !== caret.current || useMenu.getState().rows) return;
      if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
      e.preventDefault();
      showRef.current();
      return 'handled';
    });
  }, [focused]);

  const name = current.preset.name;
  return (
    <HoverTooltip content={presetTooltip(current, next)}>
      <span className="preset-button" data-preset={current.preset.id}>
        <button
          type="button"
          className="preset-main"
          aria-label={next ? `Branches: ${name}. Switch to ${next.preset.name}` : `Branches: ${name}`}
          aria-disabled={next ? undefined : true}
          onClick={() => cyclePreset(tabId)}
        >
          <GitFork size={13} aria-hidden />
          <span className="preset-dot" style={{ background: presetColour(current.preset.color) }} aria-hidden />
        </button>
        <button
          ref={caret}
          type="button"
          className="preset-caret"
          aria-label="All branch presets"
          aria-haspopup="menu"
          onClick={show}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
        >
          <ChevronDown size={11} aria-hidden />
        </button>
      </span>
    </HoverTooltip>
  );
}
