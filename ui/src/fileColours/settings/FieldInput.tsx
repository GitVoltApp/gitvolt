import { useEffect, useRef, useState } from 'react';

const fields = new WeakMap<Element, { enter(): void; esc(): void }>();

/** Enter and Esc in a `FieldInput`. Settings' dialog takes every key before React's handlers see
 * it, so the page forwards them from the key router (`FileColoursPage`). */
export function fieldKey(e: KeyboardEvent): 'handled' | undefined {
  const f = e.target instanceof Element ? fields.get(e.target) : undefined;
  if (!f || e.isComposing) return;
  if (e.key === 'Enter') { f.enter(); return 'handled'; }
  if (e.key === 'Escape') { f.esc(); return 'handled'; }
}

export interface FieldInputProps {
  value: string;
  label: string;
  className?: string;
  placeholder?: string;
  autoFocus?: boolean;
  /** Enter or blur. */
  onCommit(value: string): void;
  /** Esc, after the draft went back to `value`. */
  onCancel?(): void;
}

/** A text field that commits on Enter or blur and reverts on Esc. */
export function FieldInput({ value, label, className, placeholder, autoFocus, onCommit, onCancel }: FieldInputProps) {
  const [draft, setDraft] = useState(value);
  const el = useRef<HTMLInputElement>(null);
  const live = useRef({ commit: () => {}, cancel: () => {} });
  live.current = { commit: () => onCommit(draft), cancel: () => { setDraft(value); onCancel?.(); } };
  useEffect(() => {
    const n = el.current;
    if (!n) return;
    fields.set(n, { enter: () => live.current.commit(), esc: () => live.current.cancel() });
    if (autoFocus) { n.focus(); n.select(); }
    return () => { fields.delete(n); };
  }, [autoFocus]);
  return (
    <input ref={el} className={className} aria-label={label} placeholder={placeholder} value={draft} spellCheck={false}
      onChange={(e) => setDraft(e.target.value)} onBlur={() => live.current.commit()} />
  );
}
