import { useEffect, useMemo, useRef, useState } from 'react';
import { Select } from '../../ui/Select';
import { planAdd } from '../addPlan';
import { useAddPatternRequest } from '../addRequest';
import { addPattern, moveScope, newScope } from '../scopes';
import type { FileScope } from '../store';
import { tintHex } from '../tint';
import './fileColoursSettings.css';

export interface AddStripProps {
  commonDir: string;
  scopes: readonly FileScope[];
  repoPaths: readonly string[] | null;
  onChange(next: FileScope[]): void;
  onCreated?(id: string): void;
}

/** "Add [pattern] to [scope ▾]": what the file list's menu asked for, until it is added or cancelled. */
export function AddStrip(props: AddStripProps) {
  const req = useAddPatternRequest((s) => s.req);
  if (req?.commonDir !== props.commonDir) return null;
  return <Strip key={`${req.pattern}\n${req.via}`} {...props} pattern={req.pattern} via={req.via} />;
}

function Strip({ scopes, repoPaths, onChange, onCreated, pattern: initial, via }: AddStripProps & { pattern: string; via: string }) {
  const clear = useAddPatternRequest((s) => s.clear);
  const [pattern, setPattern] = useState(initial);
  const [target, setTarget] = useState(0);
  const primary = useRef<HTMLButtonElement>(null);
  useEffect(() => primary.current?.focus(), []);
  const t = Math.min(target, scopes.length - 1);
  const p = pattern.trim();
  const plan = useMemo(() => (p && scopes.length && repoPaths ? planAdd(scopes, t, p, repoPaths) : null), [scopes, t, p, repoPaths]);
  const higher = plan?.shadowedBy != null ? scopes[plan.shadowedBy] : null;
  const done = (next: FileScope[]) => { onChange(next); clear(); };
  const add = () => done(addPattern(scopes, t, p));
  const move = () => {
    if (plan?.shadowedBy == null) return;
    done(moveScope(addPattern(scopes, t, p), t, plan.shadowedBy));
  };
  const create = () => {
    const s = newScope(scopes, [p]);
    done([...scopes, s]);
    onCreated?.(s.id);
  };
  return (
    <div className="fc-strip" role="group" aria-label="Add pattern" onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); clear(); } }}>
      <div className="fc-strip-row">
        <span>Add</span>
        <input className="fc-strip-pattern" aria-label="Pattern" value={pattern} spellCheck={false} onChange={(e) => setPattern(e.target.value)} />
        {scopes.length > 0 && (
          <>
            <span>to</span>
            <Select
              aria-label="Scope"
              value={scopes[t].id}
              options={scopes.map((s) => [s.id, s.name] as const)}
              shown={<span className="fc-strip-scope"><i style={{ background: tintHex(scopes[t].tint) ?? undefined }} />{scopes[t].name}</span>}
              onChange={(id) => setTarget(Math.max(0, scopes.findIndex((s) => s.id === id)))}
            />
          </>
        )}
        <span className="fc-strip-spacer" />
        {scopes.length > 0 && <button ref={primary} type="button" className="fc-btn primary" disabled={!p} onClick={add}>Add</button>}
        <button ref={scopes.length ? undefined : primary} type="button" className="fc-btn" disabled={!p} onClick={create}>New scope</button>
        <button type="button" className="fc-btn" onClick={clear}>Cancel</button>
      </div>
      {higher && (
        <div className="fc-strip-warn" role="status">
          <span>{higher.name} is higher in the list and also matches these files, so they'd stay {higher.name}.</span>
          <button type="button" className="fc-btn" onClick={move}>
            Move {scopes[t].name} above {higher.name}{plan!.recolours > 0 ? ` (recolours ${plan!.recolours} other files)` : ''}
          </button>
        </div>
      )}
      <div className="fc-strip-via">From the file list: {via}</div>
    </div>
  );
}
