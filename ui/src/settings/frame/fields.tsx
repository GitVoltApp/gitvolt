import { useEffect, useState } from 'react';
import { isWindowBlur, refocusWhenWindowReturns } from '../../ui/windowBlur';

// Enter applies through the form (implicit submission): the dialog's key claim (`useModalKeys`)
// stops the keydown before React's handlers, so an `onKeyDown` here would never see it.

/** A number box that applies on Enter or blur (not per keystroke, which would reload the graph
 * for every digit), and snaps back to the applied value when what's typed isn't valid. `blank`:
 * the value an empty box stands for (shown empty, with the placeholder), such as 0 for automatic. */
export function NumberField({ id, value, min, max, step, blank, placeholder, onCommit }: { id: string; value: number; min: number; max: number; step: number; blank?: number; placeholder?: string; onCommit(n: number): void }) {
  const shown = (v: number) => (v === blank ? '' : String(v));
  const [draft, setDraft] = useState(shown(value));
  useEffect(() => setDraft(shown(value)), [value]); // eslint-disable-line react-hooks/exhaustive-deps
  const commit = () => {
    const n = Math.round(Number(draft));
    const empty = draft.trim() === '';
    if (empty && blank !== undefined) {
      setDraft('');
      if (blank !== value) onCommit(blank);
      return;
    }
    if (empty || !Number.isFinite(n)) return setDraft(shown(value));
    const clamped = Math.min(max, Math.max(min, n));
    setDraft(shown(clamped));
    if (clamped !== value) onCommit(clamped);
  };
  return (
    <form className="inline-form" noValidate onSubmit={(e) => { e.preventDefault(); commit(); }}>
      <input id={`input-${id}`} type="number" min={min} max={max} step={step} value={draft} placeholder={placeholder} onChange={(e) => setDraft(e.target.value)} onBlur={(e) => { if (isWindowBlur()) return refocusWhenWindowReturns(e.currentTarget); commit(); }} />
    </form>
  );
}

/** A text box that applies on Enter or blur. `onDraft`: each keystroke's text (a live preview). */
export function TextField({ id, value, placeholder, onCommit, onDraft }: { id: string; value: string; placeholder?: string; onCommit(v: string): void; onDraft?(v: string): void }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const commit = () => { if (draft.trim() !== value) onCommit(draft.trim()); };
  return (
    <form className="inline-form" noValidate onSubmit={(e) => { e.preventDefault(); commit(); }}>
      <input id={`input-${id}`} spellCheck={false} value={draft} placeholder={placeholder} onChange={(e) => { setDraft(e.target.value); onDraft?.(e.target.value); }} onBlur={(e) => { if (isWindowBlur()) return refocusWhenWindowReturns(e.currentTarget); commit(); }} />
    </form>
  );
}
