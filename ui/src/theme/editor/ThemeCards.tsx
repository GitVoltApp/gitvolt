import { ClipboardPaste } from 'lucide-react';
import { useMemo, useRef, useState } from 'react';
import { readClipboardText } from '../../api/transport';
import { useAppState } from '../../app/state';
import { useKeys } from '../../ui/keyRouter';
import { displayChord } from '../../ui/platformKeys';
import { customThemeDef } from '../custom';
import { THEME_IDS, THEMES, type ThemeDef } from '../themes';
import { pasteTheme, useEditedTheme } from './actions';
import './editor.css';

const select = (id: string) => useAppState.getState().setSettings({ theme: id });

/** A card's picture of a theme, in that theme's own colours: background, a panel strip, an
 * accent bar, a text bar and four lanes. */
function CardPreview({ def }: { def: ThemeDef }) {
  const c = def.colors;
  return (
    <span className="theme-card-preview" aria-hidden style={{ background: c['app-bg0'] }}>
      <span className="theme-card-side" style={{ background: c['panel-bg0'] }} />
      <span className="theme-card-lines">
        <span className="theme-card-bar" style={{ background: c.blue, width: '70%' }} />
        <span className="theme-card-bar theme-card-text" style={{ background: c['text-normal'], width: '90%' }} />
        <span className="theme-card-lanes">{def.graph.slice(0, 4).map((g, i) => <i key={i} style={{ borderColor: g }} />)}</span>
      </span>
    </span>
  );
}

const NEXT: Record<string, 1 | -1> = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 };

/**
 * The themes as cards (built-ins, then custom themes, ✎ marked), a radio group: arrows move,
 * Space or Enter selects, a click selects. The last card pastes a theme from the clipboard.
 * Keys go through the key router: in Settings the dialog's claim stops them before React's.
 */
export function ThemeCards() {
  const { def: current, customs } = useEditedTheme();
  const cards = useMemo(() => [
    ...THEME_IDS.map((id) => ({ def: THEMES[id] as ThemeDef, custom: false })),
    ...customs.map((ct) => ({ def: customThemeDef(ct), custom: true })),
  ], [customs]);
  const [roving, setRoving] = useState<string | null>(null);
  const tabStop = roving && cards.some((c) => c.def.id === roving) ? roving : current.id;
  const group = useRef<HTMLDivElement>(null);

  useKeys('menu', (e) => {
    const card = e.target instanceof Element ? e.target.closest<HTMLElement>('[role="radio"]') : null;
    if (!card || !group.current?.contains(card) || e.altKey || e.ctrlKey || e.metaKey) return;
    const all = [...group.current.querySelectorAll<HTMLElement>('[role="radio"]')];
    const i = all.indexOf(card);
    const to = e.key in NEXT ? i + NEXT[e.key]! : e.key === 'Home' ? 0 : e.key === 'End' ? all.length - 1 : null;
    if (to !== null) {
      e.preventDefault();
      all[Math.max(0, Math.min(all.length - 1, to))]?.focus();
      return 'handled';
    }
    if (e.key === ' ' || e.key === 'Enter') {
      e.preventDefault();
      if (!e.repeat) select(card.dataset.themeId!);
      return 'handled';
    }
  });

  const paste = () => void readClipboardText().then(pasteTheme, () => pasteTheme(''));
  return (
    <div className="theme-cards">
      <div ref={group} role="radiogroup" aria-label="Theme" className="theme-cards-group">
        {cards.map(({ def, custom }, i) => {
          const checked = def.id === current.id;
          return (
            <div
              key={custom ? `${def.id}:${i}` : def.id}
              role="radio"
              aria-checked={checked}
              aria-label={def.label}
              id={checked ? 'input-theme' : undefined}
              tabIndex={def.id === tabStop ? 0 : -1}
              data-theme-id={def.id}
              className="theme-card"
              onClick={() => select(def.id)}
              onFocus={() => setRoving(def.id)}
            >
              <CardPreview def={def} />
              <span className="theme-card-name">
                <span className="theme-card-label">{def.label}</span>
                {custom && <span className="theme-card-mark" aria-hidden>✎</span>}
              </span>
            </div>
          );
        })}
      </div>
      <button type="button" className="theme-card theme-card-paste" onClick={paste}>
        <ClipboardPaste size={16} aria-hidden />
        <span>Paste a theme</span>
        <small>or {displayChord('Mod+V')} on this page</small>
      </button>
    </div>
  );
}
