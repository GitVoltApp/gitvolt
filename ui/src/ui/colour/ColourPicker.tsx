import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { RgbaColorPicker, RgbColorPicker } from 'react-colorful';
import { escOwners } from '../../app/modalKeys';
import { registerKeys } from '../keyRouter';
import { placeBelow } from '../HoverTooltip';
import { formatColour, fromHsl, parseColour, toHsl, type Rgba } from './colour';
import './colour.css';

export interface ColourPickerProps {
  value: string;
  alpha?: boolean;
  label: string;
  anchor: HTMLElement;
  onPreview(value: string): void;
  onCommit(value: string): void;
  onCancel(): void;
}

const FALLBACK: Rgba = { r: 0, g: 0, b: 0, a: 1 };

function NumberRange({ label, value, max, onChange }: { label: string; value: number; max: number; onChange(v: number): void }) {
  const set = (raw: string) => {
    const n = Number(raw);
    if (raw.trim() !== '' && Number.isFinite(n)) onChange(Math.max(0, Math.min(max, n)));
  };
  return (
    <div className="colour-range">
      <span className="colour-range-label">{label}</span>
      <input type="range" min={0} max={max} step={1} value={Math.round(value)} aria-label={`${label} slider`} onChange={(e) => set(e.target.value)} />
      <input type="number" className="colour-num" min={0} max={max} value={Math.round(value)} aria-label={label} onChange={(e) => set(e.target.value)} />
    </div>
  );
}

/**
 * GitVolt's colour picker: a portal popover with a Picker tab (saturation square, hue, opacity) and
 * an HSL tab, a hex field, and Old / New swatches. Every change previews; Enter or a click outside
 * commits; Esc cancels (the caller restores the original).
 */
export function ColourPicker({ value, alpha = false, label, anchor, onPreview, onCommit, onCancel }: ColourPickerProps) {
  const original = useRef(parseColour(value) ?? FALLBACK);
  const [cur, setCur] = useState<Rgba>(original.current);
  const [tab, setTab] = useState<'picker' | 'hsl'>('picker');
  const [hex, setHex] = useState(() => formatColour(original.current));
  const [invalid, setInvalid] = useState(false);
  const [pos, setPos] = useState<{ left: number; top: number; maxHeight?: number }>({ left: 0, top: 0 });
  const box = useRef<HTMLDivElement>(null);
  const live = useRef({ cur, onCommit, onCancel });
  live.current = { cur, onCommit, onCancel };

  const out = (c: Rgba) => formatColour(alpha ? c : { ...c, a: 1 });
  const apply = (c: Rgba) => {
    const next = alpha ? c : { ...c, a: 1 };
    setCur(next);
    setHex(formatColour(next));
    setInvalid(false);
    onPreview(formatColour(next));
  };
  const hsl = toHsl(cur);

  useLayoutEffect(() => {
    const el = box.current!;
    const r = anchor.getBoundingClientRect();
    setPos(placeBelow({ left: r.left, top: r.top, bottom: r.bottom }, { width: el.offsetWidth, height: el.offsetHeight }));
  }, [anchor, tab]);

  useEffect(() => {
    box.current?.querySelector<HTMLElement>('[role="slider"], input')?.focus();
    return () => anchor.focus({ preventScroll: true });
  }, [anchor]);

  // Enter commits and Esc cancels, from anywhere in the popover; the menu layer keeps a Settings
  // modal underneath from taking them. The modal is offered the key in the same layer: as an Esc
  // owner, the picker keeps it from closing Settings too.
  useEffect(() => {
    const ownEsc = () => true;
    escOwners.add(ownEsc);
    const off = registerKeys('menu', (e) => {
      if (e.key === 'Enter') { e.preventDefault(); live.current.onCommit(formatColour(live.current.cur)); return 'handled'; }
      if (e.key === 'Escape') { e.preventDefault(); live.current.onCancel(); return 'handled'; }
    });
    return () => { escOwners.delete(ownEsc); off(); };
  }, []);

  useEffect(() => {
    const down = (e: PointerEvent) => {
      if (e.target instanceof Node && box.current?.contains(e.target)) return;
      live.current.onCommit(formatColour(live.current.cur));
    };
    document.addEventListener('pointerdown', down, true);
    return () => document.removeEventListener('pointerdown', down, true);
  }, []);

  const typeHex = (text: string) => {
    setHex(text);
    const c = parseColour(text);
    if (!c) { setInvalid(true); return; }
    setInvalid(false);
    const next = alpha ? c : { ...c, a: 1 };
    setCur(next);
    onPreview(formatColour(next));
  };
  const setHsl = (p: Partial<{ h: number; s: number; l: number }>) => {
    const n = { ...hsl, ...p };
    apply(fromHsl(n.h, n.s, n.l, cur.a));
  };

  return createPortal(
    <div
      ref={box}
      className="colour-picker"
      role="dialog"
      aria-label={label}
      style={{ left: pos.left, top: pos.top, maxHeight: pos.maxHeight }}
    >
      <div className="colour-compare">
        <button type="button" className="colour-old" aria-label="Old" title="Restore the old colour" onClick={() => apply(original.current)}>
          <span className="colour-chip" style={{ background: out(original.current) }} />
          <span>Old</span>
        </button>
        <div className="colour-new">
          <span className="colour-chip" style={{ background: out(cur) }} />
          <span>New</span>
        </div>
      </div>
      <div className="colour-tabs" role="tablist">
        {(['picker', 'hsl'] as const).map((t) => (
          <button key={t} type="button" role="tab" aria-selected={tab === t} className="colour-tab" onClick={() => setTab(t)}>
            {t === 'picker' ? 'Picker' : 'HSL'}
          </button>
        ))}
      </div>
      {tab === 'picker' ? (
        alpha
          ? <RgbaColorPicker color={cur} onChange={apply} />
          : <RgbColorPicker color={{ r: cur.r, g: cur.g, b: cur.b }} onChange={(c) => apply({ ...c, a: 1 })} />
      ) : (
        <div className="colour-hsl">
          <NumberRange label="Hue" value={hsl.h} max={360} onChange={(h) => setHsl({ h })} />
          <NumberRange label="Saturation" value={hsl.s} max={100} onChange={(s) => setHsl({ s })} />
          <NumberRange label="Lightness" value={hsl.l} max={100} onChange={(l) => setHsl({ l })} />
          {alpha && <NumberRange label="Opacity" value={cur.a * 100} max={100} onChange={(p) => apply({ ...cur, a: p / 100 })} />}
        </div>
      )}
      <input
        className="colour-hex"
        type="text"
        spellCheck={false}
        aria-label="Hex"
        aria-invalid={invalid}
        value={hex}
        onChange={(e) => typeHex(e.target.value)}
      />
    </div>,
    document.body,
  );
}
