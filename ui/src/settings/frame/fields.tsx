import { useEffect, useState } from 'react';
import { isWindowBlur, refocusWhenWindowReturns } from '../../ui/windowBlur';

// Enter applies through the form (implicit submission): the dialog's key claim (`useModalKeys`)
// stops the keydown before React's handlers, so an `onKeyDown` here would never see it.

/** A number box that applies on Enter or blur (not per keystroke, which would reload the graph
 * for every digit), and snaps back to the applied value when what's typed isn't valid. */
export function NumberField({ id, value, min, max, step, onCommit }: { id: string; value: number; min: number; max: number; step: number; onCommit(n: number): void }) {
  const [draft, setDraft] = useState(String(value));
  useEffect(() => setDraft(String(value)), [value]);
  const commit = () => {
    const n = Math.round(Number(draft));
    if (draft.trim() === '' || !Number.isFinite(n)) return setDraft(String(value));
    const clamped = Math.min(max, Math.max(min, n));
    setDraft(String(clamped));
    if (clamped !== value) onCommit(clamped);
  };
  return (
    <form className="inline-form" noValidate onSubmit={(e) => { e.preventDefault(); commit(); }}>
      <input id={`input-${id}`} type="number" min={min} max={max} step={step} value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={(e) => { if (isWindowBlur()) return refocusWhenWindowReturns(e.currentTarget); commit(); }} />
    </form>
  );
}

/** A text box that applies on Enter or blur. */
export function TextField({ id, value, placeholder, onCommit }: { id: string; value: string; placeholder?: string; onCommit(v: string): void }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const commit = () => { if (draft.trim() !== value) onCommit(draft.trim()); };
  return (
    <form className="inline-form" noValidate onSubmit={(e) => { e.preventDefault(); commit(); }}>
      <input id={`input-${id}`} spellCheck={false} value={draft} placeholder={placeholder} onChange={(e) => setDraft(e.target.value)} onBlur={(e) => { if (isWindowBlur()) return refocusWhenWindowReturns(e.currentTarget); commit(); }} />
    </form>
  );
}
