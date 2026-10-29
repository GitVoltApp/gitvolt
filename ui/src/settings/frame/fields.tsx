import { Minus, Plus } from 'lucide-react';
import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react';
import { wheelAccumulator } from '../../image/zoom';
import { registerKeys } from '../../ui/keyRouter';
import { isWindowBlur, refocusWhenWindowReturns } from '../../ui/windowBlur';
import { settingDef } from '../registry';

// Enter applies through the form (implicit submission): the dialog's key claim (`useModalKeys`)
// stops the keydown before React's handlers, so an `onKeyDown` here would never see it. The
// arrows are taken in the key router's menu layer instead, while the box has the focus.

/** A held −/+ repeats after this, then every `REPEAT_MS`. */
export const HOLD_DELAY_MS = 400;
export const REPEAT_MS = 60;

export interface NumberFieldProps {
  id: string; value: number; min: number; max: number; step: number;
  /** The value an empty box stands for (shown empty, with the placeholder), such as 0 for
   * automatic. Below `min`, it's the bottom of the range: − from `min` goes to it. */
  blank?: number;
  /** Where + from the empty box starts (default `min`): automatic's own value, say, so the first
   * step changes nothing on screen. */
  blankStart?: number;
  placeholder?: string;
  /** The places it keeps (whole numbers without it). */
  decimals?: number;
  /** The −/+ buttons' "Decrease …"/"Increase …" (default: the setting's label, lower-cased). */
  name?: string;
  onCommit(n: number): void;
}

/**
 * A number box with −/+ buttons. Typing applies on Enter or blur (not per keystroke, which would
 * reload the graph for every digit), and snaps back to the applied value when what's typed isn't
 * valid. A step applies at once, clamped: the buttons (held, they repeat), ↑/↓ (Shift: ×10) and
 * the wheel, only while the box has the focus (scrolling the page never changes a value).
 */
export function NumberField({ id, value, min, max, step, blank, blankStart, placeholder, decimals, name, onCommit }: NumberFieldProps) {
  const shown = (v: number) => (v === blank ? '' : String(v));
  const [draft, setDraft] = useState(shown(value));
  // The last step's value until the field renders with it: a held button's repeats (and steps
  // quicker than renders) go on from it.
  const lastStep = useRef<number | null>(null);
  useEffect(() => {
    lastStep.current = null;
    setDraft(shown(value));
  }, [value]); // eslint-disable-line react-hooks/exhaustive-deps
  const input = useRef<HTMLInputElement>(null);
  const round = (n: number) => (decimals ? Number(n.toFixed(decimals)) : Math.round(n));
  const clamp = (n: number) => Math.min(max, Math.max(min, n));
  const blankBelow = blank !== undefined && blank < min;
  const commitValue = (n: number) => {
    lastStep.current = n;
    setDraft(shown(n));
    if (n !== value) onCommit(n);
  };
  const commit = () => {
    const empty = draft.trim() === '';
    if (empty && blank !== undefined) return commitValue(blank);
    const n = round(Number(draft));
    if (empty || !Number.isFinite(n)) return setDraft(shown(value));
    commitValue(n === blank ? n : clamp(n));
  };

  /** The value a step starts from: what's typed when it's a number, else the applied value. */
  const current = (): number | null => {
    if (lastStep.current !== null) return lastStep.current === blank ? null : lastStep.current;
    const typed = draft.trim();
    if (typed === '') return blank !== undefined ? null : value;
    const n = Number(typed);
    return Number.isFinite(n) ? n : value === blank ? null : value;
  };
  const stepped = (dir: 1 | -1, times = 1): number => {
    const from = current();
    if (from === null) return dir > 0 ? clamp(round(blankStart ?? min)) : (blank as number);
    if (dir < 0 && blankBelow && from <= min) return blank as number;
    return clamp(round(from + dir * step * times));
  };
  const atTop = () => { const c = current(); return c !== null && c >= max; };
  const atBottom = () => { const c = current(); return c === null ? true : !blankBelow && c <= min; };
  const live = useRef({ stepped, commitValue, atTop, atBottom });
  live.current = { stepped, commitValue, atTop, atBottom };
  const stepBy = (dir: 1 | -1, times = 1) => live.current.commitValue(live.current.stepped(dir, times));

  // ↑/↓ step while the box has the focus (Shift: ten steps); the native step is prevented.
  const [focused, setFocused] = useState(false);
  useEffect(() => {
    if (!focused) return;
    return registerKeys('menu', (e) => {
      if (e.target !== input.current || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') || e.ctrlKey || e.altKey || e.metaKey) return;
      e.preventDefault();
      stepBy(e.key === 'ArrowUp' ? 1 : -1, e.shiftKey ? 10 : 1);
      return 'handled';
    });
  }, [focused]); // eslint-disable-line react-hooks/exhaustive-deps

  // The wheel steps only a focused box: a native listener, since React's is passive and couldn't
  // keep the page from scrolling. Ctrl+wheel stays the zoom's.
  useEffect(() => {
    const el = input.current;
    if (!el) return;
    const turn = wheelAccumulator();
    const wheel = (e: WheelEvent) => {
      if (document.activeElement !== el || e.ctrlKey || e.metaKey) return;
      e.preventDefault();
      const dir = turn(e);
      if (dir !== 0) stepBy(dir);
    };
    el.addEventListener('wheel', wheel, { passive: false });
    return () => el.removeEventListener('wheel', wheel);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // A held button: one step at the press, then repeats until the release (or the limit). Nothing
  // keeps running once it's let go or the field goes away.
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const stop = () => {
    if (timer.current !== null) clearTimeout(timer.current);
    timer.current = null;
  };
  useEffect(() => stop, []);
  const press = (dir: 1 | -1) => (e: ReactPointerEvent<HTMLButtonElement>) => {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture?.(e.pointerId);
    stop();
    stepBy(dir);
    const again = (delay: number) => {
      timer.current = setTimeout(() => {
        if (dir > 0 ? live.current.atTop() : live.current.atBottom()) return stop();
        stepBy(dir);
        again(REPEAT_MS);
      }, delay);
    };
    again(HOLD_DELAY_MS);
  };
  const label = name ?? lowerFirst(settingDef(id)?.label ?? 'value');
  const button = (dir: 1 | -1) => (
    <button
      type="button"
      tabIndex={-1}
      className="icon-button number-step"
      aria-label={`${dir > 0 ? 'Increase' : 'Decrease'} ${label}`}
      disabled={dir > 0 ? atTop() : atBottom()}
      // The box keeps the focus (and its typed draft) through a press.
      onMouseDown={(e) => e.preventDefault()}
      onPointerDown={press(dir)}
      onPointerUp={stop}
      onPointerCancel={stop}
      onLostPointerCapture={stop}
      // A keyboard click (Enter or Space on a focused button: detail 0); a pointer's stepped at the press.
      onClick={(e) => { if (e.detail === 0) stepBy(dir); }}
    >
      {dir > 0 ? <Plus size={12} aria-hidden /> : <Minus size={12} aria-hidden />}
    </button>
  );
  return (
    <form className="inline-form" noValidate onSubmit={(e) => { e.preventDefault(); commit(); }}>
      <span className="number-field">
        {button(-1)}
        <input
          ref={input}
          id={`input-${id}`}
          type="number"
          min={min}
          max={max}
          step={step}
          value={draft}
          placeholder={placeholder}
          onChange={(e) => {
            lastStep.current = null;
            setDraft(e.target.value);
          }}
          onFocus={() => setFocused(true)}
          onBlur={(e) => {
            setFocused(false);
            if (isWindowBlur()) return refocusWhenWindowReturns(e.currentTarget);
            commit();
          }}
        />
        {button(1)}
      </span>
    </form>
  );
}

const lowerFirst = (s: string) => s.charAt(0).toLowerCase() + s.slice(1);

/** A text box that applies on Enter or blur. `onDraft`: each keystroke's text (a live preview). */
export function TextField({ id, value, placeholder, label, onCommit, onDraft }: { id: string; value: string; placeholder?: string; label?: string; onCommit(v: string): void; onDraft?(v: string): void }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const commit = () => { if (draft.trim() !== value) onCommit(draft.trim()); };
  return (
    <form className="inline-form" noValidate onSubmit={(e) => { e.preventDefault(); commit(); }}>
      <input id={`input-${id}`} aria-label={label} spellCheck={false} value={draft} placeholder={placeholder} onChange={(e) => { setDraft(e.target.value); onDraft?.(e.target.value); }} onBlur={(e) => { if (isWindowBlur()) return refocusWhenWindowReturns(e.currentTarget); commit(); }} />
    </form>
  );
}
