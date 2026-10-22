import { useRef, useState } from 'react';
import { ColourPicker } from './ColourPicker';
import { parseColour } from './colour';
import './colour.css';

export interface ColourSwatchProps {
  value: string;
  label: string;
  alpha?: boolean;
  onPreview(value: string): void;
  onCommit(value: string): void;
  onCancel(): void;
  size?: number;
}

/** A button showing a colour (over a checkerboard when translucent) that opens the picker. */
export function ColourSwatch({ value, label, alpha, onPreview, onCommit, onCancel, size = 22 }: ColourSwatchProps) {
  const btn = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  // A press on the swatch while the picker is open closes it (as an outside click); its click must not reopen it.
  const swallow = useRef(false);
  return (
    <>
      <button
        ref={btn}
        type="button"
        className="colour-swatch"
        aria-label={label}
        aria-haspopup="dialog"
        aria-expanded={open}
        style={{ width: size, height: size }}
        onPointerDownCapture={() => { swallow.current = open; }}
        onClick={() => { if (swallow.current) swallow.current = false; else setOpen(true); }}
      >
        <span className="colour-swatch-fill" style={{ background: parseColour(value) ? value : 'transparent' }} />
      </button>
      {open && btn.current && (
        <ColourPicker
          value={value}
          alpha={alpha}
          label={label}
          anchor={btn.current}
          onPreview={onPreview}
          onCommit={(v) => { setOpen(false); onCommit(v); }}
          onCancel={() => { setOpen(false); onCancel(); }}
        />
      )}
    </>
  );
}
