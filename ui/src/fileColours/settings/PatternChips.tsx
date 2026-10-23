import { useState } from 'react';
import { compilePattern } from '../glob';
import { FieldInput } from './FieldInput';

/** A scope's patterns as chips. Editable (`editing`): click a chip to change it, ✕ to remove, "+ pattern" to add. */
export function PatternChips({ patterns, editing, onChange }: { patterns: readonly string[]; editing: boolean; onChange(next: string[]): void }) {
  const [at, setAt] = useState<number | 'new' | null>(null);
  const stop = () => setAt(null);
  const commit = (i: number | 'new', raw: string) => {
    const v = raw.trim();
    stop();
    if (i === 'new') {
      if (v && !patterns.includes(v)) onChange([...patterns, v]);
    } else if (!v) onChange(patterns.filter((_, j) => j !== i));
    else if (v !== patterns[i] && !patterns.includes(v)) onChange(patterns.map((p, j) => (j === i ? v : p)));
  };
  return (
    <span className="fc-chips">
      {patterns.map((p, i) => (editing && at === i ? (
        <FieldInput key={`${i}:${p}`} className="fc-chip-input" label={`Pattern ${p}`} value={p} autoFocus onCommit={(v) => commit(i, v)} onCancel={stop} />
      ) : (
        <span key={`${i}:${p}`} className="fc-chip" aria-invalid={compilePattern(p) ? undefined : true} title={compilePattern(p) ? undefined : 'Not a valid pattern'}>
          {editing ? <button type="button" className="fc-chip-text" onClick={() => setAt(i)}>{p}</button> : <code>{p}</code>}
          {editing && <button type="button" className="fc-chip-x" aria-label={`Remove ${p}`} onClick={() => onChange(patterns.filter((_, j) => j !== i))}>✕</button>}
        </span>
      )))}
      {editing && (at === 'new'
        ? <FieldInput className="fc-chip-input" label="New pattern" value="" autoFocus onCommit={(v) => commit('new', v)} onCancel={stop} />
        : <button type="button" className="fc-chip-add" onClick={() => setAt('new')}>+ pattern</button>)}
    </span>
  );
}
