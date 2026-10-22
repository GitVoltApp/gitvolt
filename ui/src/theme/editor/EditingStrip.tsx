import { useEffect, useState, type MouseEvent } from 'react';
import { flushSync } from 'react-dom';
import type { CustomTheme } from '../../api/gen/CustomTheme';
import { useAppState } from '../../app/state';
import { useSettingsFloat } from '../../settings/frame/floating';
import { useSettingsUi } from '../../settings/schema';
import { confirmAction } from '../../ui/ConfirmDialog';
import { isWindowBlur, refocusWhenWindowReturns } from '../../ui/windowBlur';
import { deleteTheme, renameTheme } from '../edit';
import { DEFAULT_THEME_ID, isThemeId, THEMES } from '../themes';
import { copyTheme, exportTheme, useEditedTheme } from './actions';

/** The custom theme's name: applies on Enter or blur; empty snaps back, a taken name gets " (2)". */
function NameField({ theme }: { theme: CustomTheme }) {
  const [draft, setDraft] = useState(theme.name);
  useEffect(() => setDraft(theme.name), [theme.name]);
  const commit = () => {
    renameTheme(theme.id, draft);
    setDraft(useAppState.getState().settings.customThemes?.find((t) => t.id === theme.id)?.name ?? theme.name);
  };
  return (
    <form className="inline-form" noValidate onSubmit={(e) => { e.preventDefault(); commit(); }}>
      <input
        className="theme-name"
        aria-label="Theme name"
        spellCheck={false}
        maxLength={80}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={(e) => { if (isWindowBlur()) return refocusWhenWindowReturns(e.currentTarget); commit(); }}
      />
    </form>
  );
}

/**
 * Under the cards: for a custom theme, its name and what to do with it (edit on the app, copy,
 * export, delete); for a built-in, a line saying an edit makes a copy. Both take the same height,
 * so the page doesn't move when one replaces the other.
 */
export function EditingStrip() {
  const { def, custom } = useEditedTheme();
  const floating = useSettingsFloat('theme-editor');
  if (!custom) return <div className="theme-strip theme-strip-hint">Change any colour below to make your own copy.</div>;

  const base = THEMES[isThemeId(custom.base) ? custom.base : DEFAULT_THEME_ID].label;
  const remove = async (e: MouseEvent<HTMLButtonElement>) => {
    const section = e.currentTarget.closest('[data-setting-id="theme"]');
    const ok = await confirmAction({
      title: `Delete ${custom.name}?`,
      body: `${base} is selected instead. A theme you copied or exported can be pasted back.`,
      confirmLabel: 'Delete',
      arm: `Click again to delete ${custom.name}`,
      danger: true,
    });
    if (!ok) return;
    flushSync(() => deleteTheme(custom.id));
    section?.querySelector<HTMLElement>('[role="radio"][aria-checked="true"]')?.focus();
  };
  return (
    <div className="theme-strip" role="group" aria-label={`Editing ${custom.name}`}>
      <span className="theme-strip-text">
        Editing <NameField theme={custom} /> made from <b>{base}</b>; your other themes are untouched.
      </span>
      <span className="theme-strip-actions">
        {floating && <button type="button" className="primary" onClick={() => useSettingsUi.getState().setFloating('theme-editor')}>Edit on the app</button>}
        <button type="button" onClick={() => void copyTheme(def, custom)}>Copy as JSON</button>
        <button type="button" onClick={() => void exportTheme(def, custom)}>Export file…</button>
        <button type="button" className="danger" onClick={(e) => void remove(e)}>Delete</button>
      </span>
    </div>
  );
}
