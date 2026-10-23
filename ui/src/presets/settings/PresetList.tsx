import { GripVertical, Lock } from 'lucide-react';
import { useRef, type HTMLAttributes } from 'react';
import { HoverTooltip } from '../../ui/HoverTooltip';
import { useSortable } from '../../ui/sortable/useSortable';
import type { PresetEntry, PresetGroup } from '../model';
import { presetColour } from '../PresetButton';
import { CUSTOM_ID } from '../builtins';
import { useSortableKeys } from './sortableKeys';

export interface PresetListProps {
  entries: PresetEntry[];
  /** The open repository's name; null hides This repository. */
  repoName: string | null;
  selected: string;
  onSelect(id: string): void;
  onMove(group: PresetGroup, from: number, to: number): void;
  onCycle(id: string, inCycle: boolean): void;
  onNew(): void;
  onPaste(): void;
}

/** The left column: the presets in cycle order, by group, each with its grip, colour, name and tick. */
export function PresetList({ entries, repoName, selected, onSelect, onMove, onCycle, onNew, onPaste }: PresetListProps) {
  const of = (g: PresetGroup) => entries.filter((e) => e.group === g);
  const repo = of('repo');
  return (
    <div className="bp-side">
      <Group title="Built in" group="builtin" entries={of('builtin')} cycleHead {...{ selected, onSelect, onMove, onCycle }} />
      <Group title="All repositories" group="all" entries={of('all')} {...{ selected, onSelect, onMove, onCycle }} />
      {repoName !== null && (
        <Group title="This repository" detail={repoName} group="repo" entries={repo.filter((e) => e.preset.id !== CUSTOM_ID)} custom={repo.find((e) => e.preset.id === CUSTOM_ID)} {...{ selected, onSelect, onMove, onCycle }} />
      )}
      <div className="bp-side-actions">
        <button type="button" className="bp-link" onClick={onNew}>+ New preset</button>
        <button type="button" className="bp-link" onClick={onPaste}>Paste preset</button>
      </div>
      <p className="bp-side-hint">Drag to set the order. The button's click steps through the ticked presets; the ▾ menu always lists them all.</p>
    </div>
  );
}

interface GroupProps {
  title: string;
  detail?: string;
  group: PresetGroup;
  entries: PresetEntry[];
  /** This repository's Custom: last, and not dragged. */
  custom?: PresetEntry;
  cycleHead?: boolean;
  selected: string;
  onSelect(id: string): void;
  onMove(group: PresetGroup, from: number, to: number): void;
  onCycle(id: string, inCycle: boolean): void;
}

function Group({ title, detail, group, entries, custom, cycleHead, selected, onSelect, onMove, onCycle }: GroupProps) {
  const list = useRef<HTMLDivElement>(null);
  const sortable = useSortable({ count: entries.length, axis: 'y', onMove: (from, to) => onMove(group, from, to), label: (i) => entries[i]?.preset.name ?? '' });
  useSortableKeys(list, sortable);
  const row = (e: PresetEntry, i: number | null) => (
    <Row key={e.preset.id} entry={e} selected={e.preset.id === selected} onSelect={onSelect} onCycle={onCycle}
      itemProps={i === null ? undefined : sortable.itemProps(i)} handleProps={i === null ? undefined : sortable.handleProps(i)} />
  );
  return (
    <div className="bp-group">
      <h4 className="bp-group-head">
        <span>{title}{detail && <span className="bp-group-detail"> · {detail}</span>}</span>
        {cycleHead && <span className="bp-cycle-head">in the cycle</span>}
      </h4>
      <div ref={list} role="list" aria-label={title} className="bp-list" {...sortable.containerProps}>
        {entries.map((e, i) => row(e, i))}
        {custom && row(custom, null)}
        {entries.length === 0 && !custom && <p className="bp-none">None yet</p>}
      </div>
      <span className="bp-sr" aria-live="polite">{sortable.liveText}</span>
    </div>
  );
}

interface RowProps {
  entry: PresetEntry;
  selected: boolean;
  itemProps?: HTMLAttributes<HTMLElement>;
  handleProps?: HTMLAttributes<HTMLElement>;
  onSelect(id: string): void;
  onCycle(id: string, inCycle: boolean): void;
}

function Row({ entry, selected, itemProps, handleProps, onSelect, onCycle }: RowProps) {
  const { preset: p, inCycle, readOnly } = entry;
  const custom = p.id === CUSTOM_ID;
  return (
    <div role="listitem" className="bp-item" data-preset-id={p.id} data-selected={selected || undefined} {...itemProps}
      onClick={(e) => { if (!(e.target as Element).closest('button, input')) onSelect(p.id); }}>
      {handleProps
        ? <button type="button" className="bp-grip" aria-label={`Reorder ${p.name}`} {...handleProps}><GripVertical size={14} aria-hidden /></button>
        : <span className="bp-grip" aria-hidden />}
      <span className="bp-dot" style={{ background: presetColour(p.color) }} aria-hidden />
      <button type="button" className="bp-name" aria-pressed={selected} data-custom={custom || undefined} onClick={() => onSelect(p.id)}>{p.name}</button>
      {readOnly && (
        <HoverTooltip content="Built in: Duplicate it to change it">
          <span className="bp-lock" aria-label="Built in"><Lock size={11} aria-hidden /></span>
        </HoverTooltip>
      )}
      <input type="checkbox" className="bp-tick" aria-label={`${p.name} in the cycle`} checked={inCycle} onChange={(e) => onCycle(p.id, e.target.checked)} />
    </div>
  );
}
