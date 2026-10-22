import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent, type RefObject } from 'react';
import { createPortal } from 'react-dom';
import { escOwners } from '../../app/modalKeys';
import { wheelAccumulator } from '../../image/zoom';
import { registerKeys } from '../keyRouter';
import { placeBelow } from '../HoverTooltip';
import { formatColour, fromOklch, oklchPixel, parseColour, toOklch, type Oklch, type Rgba } from './colour';
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
const sameRgb = (x: Rgba, y: Rgba) => x.r === y.r && x.g === y.g && x.b === y.b;
const clamp = (n: number, max: number) => Math.max(0, Math.min(max, n));
/** The chroma at the square's right edge and the Chroma slider's top (40 hundredths): sRGB's most
 * colourful blue is about 0.32. */
const CMAX = 0.4;
const C_STEPS = 40; // CMAX in hundredths (0.4 * 100 isn't quite 40)
/** The hue strip: every hue at one mid lightness and chroma, so the strip reads evenly. */
const HUE_STRIP = `linear-gradient(to right, ${Array.from({ length: 37 }, (_, i) => formatColour(fromOklch({ l: 72, c: 0.12, h: i * 10 }, 1))).join(', ')})`;

/** What a step changes: lightness, chroma, hue or opacity. */
type Channel = 'l' | 'c' | 'h' | 'a';
const ARROWS: Record<string, ['x' | 'y', 1 | -1]> = { ArrowRight: ['x', 1], ArrowLeft: ['x', -1], ArrowUp: ['y', 1], ArrowDown: ['y', -1] };
/** The channel a square or strip (its `data-step`) steps: the square's arrows move chroma across and
 * lightness up and down, and its wheel lightness. */
const channelOf = (step: string | undefined, axis: 'x' | 'y'): Channel => (step === 'square' ? (axis === 'x' ? 'c' : 'l') : (step as Channel));

/** The wheel over `ref` steps a value by 1 a notch (a trackpad's travel adds up to a notch), up for
 * more, where `over` says a slider is under the pointer (elsewhere the wheel scrolls as usual). A
 * native listener: React's wheel handler is passive, so it couldn't keep the page from scrolling. */
function useWheelSteps(ref: RefObject<HTMLElement | null>, over: (target: Element) => boolean, step: (dir: 1 | -1, target: Element) => void) {
  const live = useRef({ over, step });
  live.current = { over, step };
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const turn = wheelAccumulator();
    const wheel = (e: WheelEvent) => {
      if (!(e.target instanceof Element) || !live.current.over(e.target)) return;
      e.preventDefault();
      const dir = turn(e);
      if (dir !== 0) live.current.step(dir, e.target);
    };
    el.addEventListener('wheel', wheel, { passive: false });
    return () => el.removeEventListener('wheel', wheel);
  }, [ref]);
}

const ANYWHERE = () => true;

function NumberRange({ label, value, max, onChange }: { label: string; value: number; max: number; onChange(v: number): void }) {
  const set = (raw: string) => {
    const n = Number(raw);
    if (raw.trim() !== '' && Number.isFinite(n)) onChange(clamp(n, max));
  };
  const row = useRef<HTMLDivElement>(null);
  useWheelSteps(row, ANYWHERE, (dir) => onChange(clamp(Math.round(value) + dir, max)));
  return (
    <div ref={row} className="colour-range">
      <span className="colour-range-label">{label}</span>
      <input type="range" min={0} max={max} step={1} value={Math.round(value)} aria-label={`${label} slider`} onChange={(e) => set(e.target.value)} />
      <input type="number" className="colour-num" min={0} max={max} value={Math.round(value)} aria-label={label} onChange={(e) => set(e.target.value)} />
    </div>
  );
}

/** Where a pointer is in `e`'s target, 0-1 across and down; null while it has no size. */
function spot(e: ReactPointerEvent<HTMLElement>): { x: number; y: number } | null {
  const r = e.currentTarget.getBoundingClientRect();
  if (!r.width || !r.height) return null;
  return { x: clamp((e.clientX - r.left) / r.width, 1), y: clamp((e.clientY - r.top) / r.height, 1) };
}

/** Pointer handlers for a square or strip: a press picks the spot under it and a drag follows,
 * captured, so it keeps following outside. */
function dragProps(pick: (at: { x: number; y: number }) => void) {
  const at = (e: ReactPointerEvent<HTMLElement>) => { const s = spot(e); if (s) pick(s); };
  return {
    onPointerDown: (e: ReactPointerEvent<HTMLElement>) => {
      if (e.button !== 0) return;
      e.currentTarget.setPointerCapture?.(e.pointerId);
      e.currentTarget.focus();
      at(e);
    },
    onPointerMove: (e: ReactPointerEvent<HTMLElement>) => { if (e.currentTarget.hasPointerCapture?.(e.pointerId)) at(e); },
  };
}

/** Chroma (across, 0 to CMAX) by lightness (100 at the top) at one hue, drawn only when the hue
 * changes. What sRGB can't show stays clear, over the square's dim fill (colour.css), so its edge
 * is the gamut's. */
function ChromaSquare({ lch, colour, onPick }: { lch: Oklch; colour: string; onPick(l: number, c: number): void }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const { h } = lch;
  useLayoutEffect(() => {
    const el = canvas.current!;
    const dpr = window.devicePixelRatio || 1;
    const [w, ht] = [Math.round(el.clientWidth * dpr), Math.round(el.clientHeight * dpr)];
    const ctx = w && ht ? el.getContext?.('2d') : null;
    if (!ctx) return;
    [el.width, el.height] = [w, ht];
    const img = ctx.createImageData(w, ht);
    const d = img.data;
    for (let y = 0; y < ht; y++) {
      const l = 100 * (1 - (y + 0.5) / ht);
      for (let x = 0; x < w; x++) {
        const px = oklchPixel(l, (CMAX * (x + 0.5)) / w, h);
        if (!px) break; // more chroma at this lightness and hue is outside sRGB too
        const i = (y * w + x) * 4;
        [d[i], d[i + 1], d[i + 2], d[i + 3]] = [px[0], px[1], px[2], 255];
      }
    }
    ctx.putImageData(img, 0, 0);
  }, [h]);
  const [l, c] = [Math.round(lch.l), Math.round(lch.c * 100)];
  return (
    <div
      className="colour-square"
      role="slider"
      tabIndex={0}
      data-step="square"
      aria-label="Chroma and lightness"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={l}
      aria-valuetext={`Lightness ${l}, chroma ${c}`}
      {...dragProps(({ x, y }) => onPick(100 * (1 - y), CMAX * x))}
    >
      <canvas ref={canvas} />
      <span className="colour-thumb" style={{ left: `${Math.min(lch.c / CMAX, 1) * 100}%`, top: `${100 - lch.l}%`, background: colour }} />
    </div>
  );
}

function Strip({ label, step, value, max, background, onChange }: { label: string; step: Channel; value: number; max: number; background: string; onChange(v: number): void }) {
  return (
    <div
      className="colour-strip"
      role="slider"
      tabIndex={0}
      data-step={step}
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={max}
      aria-valuenow={Math.round(value)}
      style={{ '--colour-strip': background } as CSSProperties}
      {...dragProps(({ x }) => onChange(x * max))}
    >
      <span className="colour-thumb" style={{ left: `${(value / max) * 100}%` }} />
    </div>
  );
}

/**
 * GitVolt's colour picker: a portal popover with a Picker tab (an OKLCH chroma × lightness square,
 * hue, and opacity where `alpha`) and an OKLCH tab of sliders, a hex field, and Old / New swatches.
 * It edits in OKLCH, so lightness and chroma move without shifting the hue, and emits hex or
 * `rgba()`. Every change previews; Enter or a click outside commits; Esc cancels (the caller
 * restores the original).
 */
export function ColourPicker({ value, alpha = false, label, anchor, onPreview, onCommit, onCancel }: ColourPickerProps) {
  const original = useRef(parseColour(value) ?? FALLBACK);
  const [cur, setCur] = useState<Rgba>(original.current);
  const [tab, setTab] = useState<'picker' | 'oklch'>('picker');
  const [hex, setHex] = useState(() => formatColour(original.current));
  const [invalid, setInvalid] = useState(false);
  // The OKLCH last set here, chroma beyond the gamut included. Read back from RGB, OKLCH loses what
  // the bytes can't hold: a grey has no hue, black and white no chroma either, and a colour brought
  // into the gamut lost chroma. Kept while it still makes the current colour, so a lightness drag
  // to black and back comes back with the hue and chroma it left with.
  const [held, setHeld] = useState<Oklch | null>(null);
  const [pos, setPos] = useState<{ left: number; top: number; maxHeight?: number }>({ left: 0, top: 0 });
  const box = useRef<HTMLDivElement>(null);

  const out = (c: Rgba) => formatColour(alpha ? c : { ...c, a: 1 });
  const apply = (c: Rgba) => {
    const next = alpha ? c : { ...c, a: 1 };
    setCur(next);
    setHex(formatColour(next));
    setInvalid(false);
    onPreview(formatColour(next));
  };
  const lch = held && sameRgb(fromOklch(held, cur.a), cur) ? held : toOklch(cur);
  const setLch = (p: Partial<Oklch>) => {
    const n = { ...lch, ...p };
    setHeld(n);
    apply(fromOklch(n, cur.a));
  };
  /** One step (an arrow key, a wheel notch; Shift's arrows 10) from the shown, rounded value. */
  const nudge = (ch: Channel, by: number) => {
    if (ch === 'l') setLch({ l: clamp(Math.round(lch.l) + by, 100) });
    else if (ch === 'c') setLch({ c: clamp(Math.round(lch.c * 100) + by, C_STEPS) / 100 });
    else if (ch === 'h') setLch({ h: clamp(Math.round(lch.h) + by, 360) });
    else apply({ ...cur, a: clamp(Math.round(cur.a * 100) + by, 100) / 100 });
  };
  const live = useRef({ cur, onCommit, onCancel, nudge });
  live.current = { cur, onCommit, onCancel, nudge };

  useLayoutEffect(() => {
    const el = box.current!;
    const r = anchor.getBoundingClientRect();
    setPos(placeBelow({ left: r.left, top: r.top, bottom: r.bottom }, { width: el.offsetWidth, height: el.offsetHeight }));
  }, [anchor, tab]);

  useEffect(() => {
    box.current?.querySelector<HTMLElement>('[role="slider"], input')?.focus();
    return () => anchor.focus({ preventScroll: true });
  }, [anchor]);

  // Enter commits and Esc cancels, from anywhere in the popover, and the arrows step the focused
  // square or strip; the menu layer keeps a Settings modal underneath from taking them (it would
  // stop them before the element's own handler). The modal is offered the key in the same layer:
  // as an Esc owner, the picker keeps it from closing Settings too.
  useEffect(() => {
    const ownEsc = () => true;
    escOwners.add(ownEsc);
    const off = registerKeys('menu', (e) => {
      if (e.key === 'Enter') { e.preventDefault(); live.current.onCommit(formatColour(live.current.cur)); return 'handled'; }
      if (e.key === 'Escape') { e.preventDefault(); live.current.onCancel(); return 'handled'; }
      const slider = e.target instanceof HTMLElement ? e.target.closest<HTMLElement>('[data-step]') : null;
      const arrow = ARROWS[e.key];
      if (!slider || !arrow || !box.current?.contains(slider) || e.ctrlKey || e.altKey || e.metaKey) return;
      e.preventDefault();
      live.current.nudge(channelOf(slider.dataset.step, arrow[0]), arrow[1] * (e.shiftKey ? 10 : 1));
      return 'handled';
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

  // The wheel over the square steps lightness, over a strip its value. (The OKLCH tab's rows take
  // their own wheel: they have no `data-step`.)
  useWheelSteps(box, (t) => t.closest('[data-step]') !== null, (dir, t) => {
    live.current.nudge(channelOf(t.closest<HTMLElement>('[data-step]')!.dataset.step, 'y'), dir);
  });

  const typeHex = (text: string) => {
    setHex(text);
    const c = parseColour(text);
    if (!c) { setInvalid(true); return; }
    setInvalid(false);
    const next = alpha ? c : { ...c, a: 1 };
    setCur(next);
    onPreview(formatColour(next));
  };
  const opaque = formatColour({ ...cur, a: 1 });

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
        {(['picker', 'oklch'] as const).map((t) => (
          <button key={t} type="button" role="tab" aria-selected={tab === t} className="colour-tab" onClick={() => setTab(t)}>
            {t === 'picker' ? 'Picker' : 'OKLCH'}
          </button>
        ))}
      </div>
      {tab === 'picker' ? (
        <div className={alpha ? 'colour-body alpha' : 'colour-body'}>
          <ChromaSquare lch={lch} colour={opaque} onPick={(l, c) => setLch({ l, c })} />
          <Strip label="Hue" step="h" value={lch.h} max={360} background={HUE_STRIP} onChange={(h) => setLch({ h })} />
          {alpha && (
            <Strip
              label="Opacity"
              step="a"
              value={cur.a * 100}
              max={100}
              background={`linear-gradient(to right, ${formatColour({ ...cur, a: 0 })}, ${opaque})`}
              onChange={(p) => apply({ ...cur, a: Math.round(p) / 100 })}
            />
          )}
        </div>
      ) : (
        <div className={alpha ? 'colour-body alpha' : 'colour-body'}>
          <NumberRange label="Lightness" value={lch.l} max={100} onChange={(l) => setLch({ l })} />
          <NumberRange label="Chroma" value={lch.c * 100} max={C_STEPS} onChange={(c) => setLch({ c: c / 100 })} />
          <NumberRange label="Hue" value={lch.h} max={360} onChange={(h) => setLch({ h })} />
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
