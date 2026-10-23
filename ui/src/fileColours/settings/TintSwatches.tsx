import { ColourSwatch } from '../../ui/colour/ColourSwatch';
import { TINT_PRESETS, tintHex, type TintPreset } from '../tint';

const NAMES = Object.keys(TINT_PRESETS) as TintPreset[];

/** The seven preset tints and a "+" swatch that opens the colour picker (no opacity: strength comes from the theme). */
export function TintSwatches({ value, onChange }: { value: string; onChange(tint: string): void }) {
  const custom = !(value in TINT_PRESETS);
  const hex = tintHex(value) ?? '#8b949e';
  return (
    <div className="fc-tints" role="group" aria-label="Colour">
      {NAMES.map((n) => (
        <button key={n} type="button" className="fc-tint" aria-label={n} aria-pressed={value === n} style={{ background: TINT_PRESETS[n] }} onClick={() => onChange(n)} />
      ))}
      <span className="fc-tint-custom" data-selected={custom || undefined}>
        <ColourSwatch value={hex} label="Custom colour" size={20} onPreview={() => {}} onCommit={(v) => onChange(v.toLowerCase())} onCancel={() => {}} />
        {!custom && <span className="fc-tint-plus" aria-hidden>+</span>}
      </span>
    </div>
  );
}
