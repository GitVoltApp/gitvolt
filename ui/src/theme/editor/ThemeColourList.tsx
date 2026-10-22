import { ChevronRight } from 'lucide-react';
import { useEffect, useId, useState, type ReactNode } from 'react';
import { ColourSwatch } from '../../ui/colour/ColourSwatch';
import { formatColour, parseColour } from '../../ui/colour/colour';
import { isWindowBlur, refocusWhenWindowReturns } from '../../ui/windowBlur';
import { CORE_COLOURS, TOKEN_GROUPS, TOKEN_LABELS, type ColourGroup } from '../custom';
import { setThemeColour } from '../edit';
import { useTheme } from '../store';
import type { ColorToken } from '../themes';
import { useEditedTheme } from './actions';
import './editor.css';

const CORE_GROUPS: readonly ColourGroup[] = ['Surfaces', 'Text', 'Diff', 'Status'];

/** What's typed as a theme colour (`#` optional on hex), in the form themes keep; `null` if it isn't one. */
export function typedColour(text: string): string | null {
  const t = text.trim();
  const c = parseColour(/^[0-9a-f]{3,8}$/i.test(t) ? `#${t}` : t);
  return c && formatColour(c);
}

/** A colour as the field shows it: `#rrggbb`, or `#rrggbbaa` when translucent (shorter than rgba()). */
export function asHex(value: string): string {
  const c = parseColour(value);
  if (!c) return value;
  const hex = formatColour({ ...c, a: 1 });
  return c.a === 1 ? hex : hex + Math.round(c.a * 255).toString(16).padStart(2, '0');
}

const preview =(token: ColorToken, v: string | null) => useTheme.getState().preview(v === null ? null : { colors: { [token]: v } });

/** One colour: its swatch (the picker, previewing on the app as it moves), its name, and a text
 * field that applies on Enter or blur (a value that isn't a colour is marked, and snaps back on blur). */
function ColourRow({ token, label, note, value }: { token: ColorToken; label: string; note?: string; value: string }) {
  const shown = asHex(value);
  const [draft, setDraft] = useState(shown);
  useEffect(() => setDraft(shown), [shown]);
  const typed = typedColour(draft);
  const commit = (snap: boolean) => {
    if (!typed) {
      if (snap) setDraft(shown);
      return;
    }
    setDraft(asHex(typed));
    if (typed !== (typedColour(value) ?? value)) setThemeColour(token, typed);
  };
  return (
    <div className="theme-colour" data-token={token}>
      <ColourSwatch
        value={value}
        label={`${label} colour`}
        alpha
        onPreview={(v) => preview(token, v)}
        onCommit={(v) => {
          preview(token, null);
          if (v !== value) setThemeColour(token, v);
        }}
        onCancel={() => preview(token, null)}
      />
      <span className="theme-colour-label">{label}{note && <small> {note}</small>}</span>
      <form className="inline-form" noValidate onSubmit={(e) => { e.preventDefault(); commit(false); }}>
        <input
          className="theme-colour-value"
          aria-label={label}
          aria-invalid={!typed}
          spellCheck={false}
          maxLength={32}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={(e) => { if (isWindowBlur()) return refocusWhenWindowReturns(e.currentTarget); commit(true); }}
        />
      </form>
    </div>
  );
}

function Group({ title, children }: { title: string; children: ReactNode }) {
  const id = useId();
  return (
    <div className="theme-colour-group" role="group" aria-labelledby={id}>
      <h4 id={id}>{title}</h4>
      {children}
    </div>
  );
}

/**
 * The edited theme's colours: the core ones in four groups (two columns, one when `compact`),
 * then **All colours**, every token, behind a disclosure. A change to a built-in theme makes its
 * custom copy (edit.ts). `filter` keeps the rows whose name, note or token has it, and opens All
 * colours when only rows there match.
 */
export function ThemeColourList({ filter, compact = false }: { filter?: string; compact?: boolean }) {
  const { def } = useEditedTheme();
  const [open, setOpen] = useState<boolean | null>(null);
  const q = filter?.trim().toLowerCase() ?? '';
  const keep = (token: string, label: string, note?: string) => !q || [label, note ?? '', token].some((s) => s.toLowerCase().includes(q));
  const core = CORE_COLOURS.filter((c) => keep(c.token, c.label, c.note));
  const all = TOKEN_GROUPS.map(({ group, tokens }) => ({ group, tokens: tokens.filter((t) => keep(t, TOKEN_LABELS[t])) })).filter((g) => g.tokens.length);
  const expanded = open ?? (q !== '' && !core.length && all.length > 0);
  return (
    <div className={compact ? 'theme-colour-list compact' : 'theme-colour-list'}>
      {core.length > 0 && (
        <div className="theme-colour-groups">
          {CORE_GROUPS.map((g) => {
            const rows = core.filter((c) => c.group === g);
            return rows.length > 0 && (
              <Group key={g} title={g}>
                {rows.map((c) => <ColourRow key={c.token} token={c.token} label={c.label} note={c.note} value={def.colors[c.token]} />)}
              </Group>
            );
          })}
        </div>
      )}
      {all.length > 0 && (
        <button type="button" className="theme-all-toggle" aria-expanded={expanded} onClick={() => setOpen(!expanded)}>
          <ChevronRight size={14} aria-hidden />All colours<small>every token; the ones above set most of them</small>
        </button>
      )}
      {expanded && (
        <div className="theme-colour-groups theme-all">
          {all.map(({ group, tokens }) => (
            <Group key={group} title={group}>
              {tokens.map((t) => <ColourRow key={t} token={t} label={TOKEN_LABELS[t]} note={t} value={def.colors[t]} />)}
            </Group>
          ))}
        </div>
      )}
      {!core.length && !all.length && <p className="theme-colour-none">No colour matches "{filter?.trim()}"</p>}
    </div>
  );
}
