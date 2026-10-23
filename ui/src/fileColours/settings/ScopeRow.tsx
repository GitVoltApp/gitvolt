import { GripVertical } from 'lucide-react';
import type { HTMLAttributes } from 'react';
import { arm } from '../../ui/arm/store';
import type { FileScope } from '../store';
import { tintHex } from '../tint';
import { FieldInput } from './FieldInput';
import { PatternChips } from './PatternChips';
import { ScopeEditor } from './ScopeEditor';

export interface ScopeRowProps {
  scope: FileScope;
  open: boolean;
  /** `null` fields while counting: the space stays reserved. */
  counts: { changed: number | null; repo: number | null } | null;
  itemProps: HTMLAttributes<HTMLElement>;
  handleProps: HTMLAttributes<HTMLElement>;
  onToggle(): void;
  onChange(next: FileScope): void;
  onDelete(): void;
}

const countText = (c: ScopeRowProps['counts']) => (c && c.changed !== null && c.repo !== null ? `${c.changed} changed · ${c.repo} in repo` : '');

export function ScopeRow({ scope, open, counts, itemProps, handleProps, onToggle, onChange, onDelete }: ScopeRowProps) {
  const del = () => {
    void arm({
      arm: `Delete ${scope.name}`, tone: 'danger', title: `Delete ${scope.name}?`, body: 'Its files stop being coloured.', confirmLabel: `Delete ${scope.name}`,
    }).then((ok) => { if (ok) onDelete(); });
  };
  return (
    <div role="listitem" className="fc-row" data-open={open || undefined} {...itemProps}>
      <div className="fc-row-main" onClick={(e) => { if (!(e.target as Element).closest('button, input')) onToggle(); }}>
        <button type="button" className="fc-handle" aria-label={`Reorder ${scope.name}`} {...handleProps}><GripVertical size={14} aria-hidden /></button>
        <span className="fc-dot" style={{ background: tintHex(scope.tint) ?? undefined }} aria-hidden />
        {open
          ? <FieldInput className="fc-name-input" label="Scope name" value={scope.name} onCommit={(name) => { const n = name.trim(); if (n && n !== scope.name) onChange({ ...scope, name: n }); }} />
          : <button type="button" className="fc-name" aria-expanded={false} onClick={onToggle}>{scope.name}</button>}
        <PatternChips patterns={scope.patterns} editing={open} onChange={(patterns) => onChange({ ...scope, patterns })} />
        {scope.startCollapsed && <span className="fc-note">starts collapsed</span>}
        <span className="fc-counts">{countText(counts)}</span>
        <button type="button" className="fc-x" aria-label={`Delete ${scope.name}`} onClick={del}>✕</button>
      </div>
      {open && <ScopeEditor scope={scope} onChange={onChange} />}
    </div>
  );
}
