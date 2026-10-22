import { useMemo, useState, type Ref } from 'react';
import { ClearableInput } from '../ui/ClearableInput';
import { chordKeycaps } from '../ui/platformKeys';
import { filterSections, shortcutSections } from './catalog';
import '../ui/keycaps.css';
import './shortcuts.css';

export const Keys = ({ chord }: { chord: string }) => (
  <span className="sc-chord">{chordKeycaps(chord).map((k, i) => <kbd key={i} className="sc-key">{k}</kbd>)}</span>
);

/** The app's shortcuts, from its real registrations (`catalog.ts`), with a filter box: the Keyboard
 * Shortcuts panel's body and Settings › Keyboard. */
export function ShortcutsList({ inputRef, placeholder = 'Filter shortcuts', autoFocus = false }: { inputRef?: Ref<HTMLInputElement>; placeholder?: string; autoFocus?: boolean }) {
  const [q, setQ] = useState('');
  const sections = useMemo(() => filterSections(shortcutSections(), q), [q]);
  return (
    <div className="sc-list">
      <ClearableInput ref={inputRef} autoFocus={autoFocus} type="search" className="sc-filter" placeholder={placeholder} aria-label="Filter shortcuts" value={q} onChange={(e) => setQ(e.target.value)} onClear={() => setQ('')} />
      <div className="sc-body">
        {sections.length === 0 && <div className="sc-empty">No shortcuts match.</div>}
        {sections.map((s) => (
          <section key={s.title} aria-label={s.title}>
            <h3>{s.title}</h3>
            <ul>
              {s.rows.map((r) => (
                <li key={r.id} className="sc-row">
                  <span className="sc-label">{r.label}{r.context && <span className="sc-ctx"> {r.context}</span>}</span>
                  <span className="sc-keys">{r.keys.map((k, i) => <span key={k} className="sc-alt">{i > 0 && <span className="sc-or">or</span>}<Keys chord={k} /></span>)}</span>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>
    </div>
  );
}
