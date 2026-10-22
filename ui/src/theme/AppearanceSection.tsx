import { useEffect, useState } from 'react';
import { useAppState } from '../app/state';
import { Row } from '../settings/Row';
import { Select } from '../ui/Select';
import { isWindowBlur, refocusWhenWindowReturns } from '../ui/windowBlur';
import { isHexColor } from './apply';
import { themeDef } from './custom';
import { setLaneColour } from './edit';
import { THEME_IDS, THEMES } from './themes';
import './appearance.css';

const BUILT_IN_CHOICES = THEME_IDS.map((id) => [id, THEMES[id].label] as const);

/** A lane's colour box: applies on Enter or blur (`#` optional, any case, `#rgb` shorthand
 * expanded); anything else, or nothing, snaps back. */
function LaneField({ index, value, onCommit }: { index: number; value: string; onCommit(v: string): void }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const typed = draft.trim().toLowerCase().replace(/^#/, '');
  const hex = `#${/^[0-9a-f]{3}$/.test(typed) ? typed.replace(/./g, '$&$&') : typed}`;
  const commit = () => {
    if (!isHexColor(hex)) return setDraft(value);
    setDraft(hex);
    if (hex !== value) onCommit(hex);
  };
  return (
    <form className="inline-form" noValidate onSubmit={(e) => { e.preventDefault(); commit(); }}>
      <span className="lane-color">
        <span className="lane-swatch" aria-hidden style={{ background: isHexColor(hex) ? hex : value }} />
        <input
          id={`input-graphColors-${index}`}
          className="lane-hex"
          aria-label={`Lane ${index + 1} color`}
          spellCheck={false}
          maxLength={7}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={(e) => { if (isWindowBlur()) return refocusWhenWindowReturns(e.currentTarget); commit(); }}
        />
      </span>
    </form>
  );
}

/** Settings → Appearance: the theme (the app's own dropdown, never a native `<select>`, K92), the
 * built-ins then the custom themes, and the current theme's lane colours. A lane edit on a
 * built-in makes "<theme> (Custom)" (edit.ts). Both save to AppSettings; `bind.ts` applies them. */
/** The theme dropdown: the built-ins, then the custom themes. Also the theme's control in the
 * settings search results. */
export function ThemeSelect() {
  const theme = useAppState((s) => s.settings.theme);
  const customs = useAppState((s) => s.settings.customThemes);
  const setSettings = useAppState((s) => s.setSettings);
  const def = themeDef(theme, customs ?? []);
  const choices = [...BUILT_IN_CHOICES, ...(customs ?? []).map((t) => [t.id, t.name] as const)];
  return <Select<string> id="input-theme" aria-labelledby="label-theme" value={def.id} onChange={(v) => setSettings({ theme: v })} options={choices} />;
}

export function AppearanceSection() {
  const theme = useAppState((s) => s.settings.theme);
  const customs = useAppState((s) => s.settings.customThemes);
  const def = themeDef(theme, customs ?? []);

  return (
    <>
      <Row id="theme" group>
        <ThemeSelect />
      </Row>
      <Row id="graphColors" group>
        <div className="lane-colors" role="group" aria-labelledby="label-graphColors">
          {def.graph.map((c, i) => <LaneField key={i} index={i} value={c} onCommit={(v) => setLaneColour(i, v)} />)}
        </div>
      </Row>
    </>
  );
}
