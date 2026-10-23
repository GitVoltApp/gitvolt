import { X } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { readClipboardText } from '../../api/transport';
import { useModalKeys } from '../../app/modalKeys';
import { comboOf } from '../../app/shortcuts';
import { useAppState } from '../../app/state';
import { ClearableInput } from '../../ui/ClearableInput';
import { HoverTooltip } from '../../ui/HoverTooltip';
import { isTypingOrEditor } from '../../ui/keys';
import { displayChord, resolveChord } from '../../ui/platformKeys';
import { settingsPasteHandler } from '../paste';
import { allSettings, settingsPages, useSettingsRegistry } from '../registry';
import { useSettingsUi } from '../schema';
import { searchSettings } from '../search';
import { useSettingsCtx } from './ctx';
import { useSettingsFloat } from './floating';
import { SettingsNav } from './Nav';
import { SearchResults } from './SearchResults';
import { focusSettingControl } from './SettingRow';
import { SettingsPageView } from './SettingsPage';
import '../../ui/keycaps.css';
import '../settings.css';

/** Settings (Ctrl+,): a large overlay with the pages of the registry in a nav, a search over every
 * setting, and the shown page. A registered floating panel takes its place while `floating` names it. */
export function SettingsOverlay() {
  const open = useSettingsUi((s) => s.open);
  const floating = useSettingsUi((s) => s.floating);
  const Float = useSettingsFloat(floating);
  if (!open) return null;
  if (Float) return <Float />;
  return <SettingsDialog />;
}

const isPrintable = (e: KeyboardEvent) => e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey;

function SettingsDialog() {
  const close = useSettingsUi((s) => s.close);
  const stored = useSettingsUi((s) => s.page);
  const query = useSettingsUi((s) => s.query);
  const focus = useSettingsUi((s) => s.focus);
  const profileName = useAppState((s) => s.profile.name);
  const profileColor = useAppState((s) => s.profile.color);
  const ctx = useSettingsCtx();
  useSettingsRegistry();
  const pages = settingsPages().filter((p) => p.scope !== 'repo' || ctx.repoPath);
  // The stored page, or General when it isn't there (a Repository page with no repository).
  const page = pages.find((p) => p.id === stored) ?? pages.find((p) => p.id === 'general') ?? pages[0];
  const searching = query.trim() !== '';
  const results = useMemo(() => (searching ? searchSettings(query, allSettings(), pages) : []),
    // `pages` is rebuilt per render; its ids are what matter.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [searching, query, pages.map((p) => p.id).join()]);
  const counts = useMemo(() => (searching ? new Map(results.map((r) => [r.page.id, r.matches.length])) : null), [searching, results]);

  const searchRef = useRef<HTMLInputElement>(null);
  const navRef = useRef<HTMLElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const [navFocus, setNavFocus] = useState<string | null>(null);
  const roving = navFocus && pages.some((p) => p.id === navFocus) ? navFocus : page?.id ?? '';

  const focusSearch = () => {
    searchRef.current?.focus();
    searchRef.current?.select();
  };
  const navItems = () => [...(navRef.current?.querySelectorAll<HTMLButtonElement>('.settings-nav-item') ?? [])];

  // Every key is the overlay's while it's open (`useModalKeys`): the nav's arrows and the
  // search box's Esc are taken here, as React's own handlers never see them.
  const onKey = (e: KeyboardEvent): boolean => {
    const t = e.target instanceof HTMLElement ? e.target : null;
    const combo = comboOf(e);
    if (combo === resolveChord('Mod+F')) {
      focusSearch();
      return true;
    }
    if (combo === resolveChord('Mod+V') && !isTypingOrEditor(t)) {
      const paste = settingsPasteHandler();
      if (!paste) return false;
      void readClipboardText().then(paste, () => {});
      return true;
    }
    if (e.key === 'Escape' && useSettingsUi.getState().query !== '') {
      useSettingsUi.getState().setQuery('');
      focusSearch();
      return true;
    }
    // A switch toggles on Space here: its own key handler never sees the key (Enter clicks it natively).
    if (e.key === ' ' && t?.getAttribute('role') === 'switch') {
      if (!e.repeat) t.click();
      return true;
    }
    if (t && navRef.current?.contains(t) && t.classList.contains('settings-nav-item')) {
      const items = navItems();
      const i = items.indexOf(t as HTMLButtonElement);
      const to = e.key === 'ArrowDown' ? i + 1 : e.key === 'ArrowUp' ? i - 1 : e.key === 'Home' ? 0 : e.key === 'End' ? items.length - 1 : null;
      if (to !== null && !e.altKey && !e.ctrlKey && !e.metaKey) {
        items[Math.max(0, Math.min(items.length - 1, to))]?.focus();
        return true;
      }
      if (e.key === 'Enter' || e.key === ' ') {
        if (!e.repeat) t.click();
        return true;
      }
      if (isPrintable(e)) {
        useSettingsUi.getState().setQuery(useSettingsUi.getState().query + e.key);
        searchRef.current?.focus();
        return true;
      }
    }
    return false;
  };
  const rootRef = useModalKeys<HTMLDivElement>(true, close, 'Mod+,', onKey);

  useEffect(() => { bodyRef.current?.scrollTo?.(0, 0); }, [page?.id, searching]);

  // A deep link: its page is shown (`show`); focus the setting's control and flash its row.
  useEffect(() => {
    if (!focus) return;
    const el = bodyRef.current?.querySelector<HTMLElement>(`[data-setting-id="${CSS.escape(focus)}"]`);
    if (!el) return;
    focusSettingControl(el);
    el.scrollIntoView({ block: 'center' });
    el.classList.add('flash');
    const t = setTimeout(() => el.classList.remove('flash'), 1500);
    return () => clearTimeout(t);
  }, [focus, page?.id]);

  return (
    <div className="modal-backdrop" onPointerDown={close}>
      <div ref={rootRef} className="settings" role="dialog" aria-modal="true" aria-label="Settings" onPointerDown={(e) => e.stopPropagation()}>
        <div className="settings-side">
          <div className="settings-search">
            <ClearableInput
              ref={searchRef}
              autoFocus={!focus}
              type="search"
              aria-label="Search settings"
              placeholder="Search settings"
              spellCheck={false}
              value={query}
              onChange={(e) => useSettingsUi.getState().setQuery(e.target.value)}
              onClear={() => useSettingsUi.getState().setQuery('')}
            />
            {!query && <kbd className="sc-key settings-search-key" aria-hidden>{displayChord('Mod+F')}</kbd>}
          </div>
          <SettingsNav pages={pages} current={searching ? '' : page?.id ?? ''} roving={roving} counts={counts} ctx={ctx} profile={{ name: profileName, color: profileColor }} onFocusItem={setNavFocus} navRef={navRef} />
        </div>
        <div ref={bodyRef} className="settings-body">
          {searching
            ? <SearchResults query={query} results={results} ctx={ctx} profileName={profileName} />
            : page && <SettingsPageView key={page.id} page={page} ctx={ctx} />}
        </div>
        <HoverTooltip content="Close (Esc)">
          <button type="button" className="icon-button settings-close" aria-label="Close settings" onClick={close}><X size={14} /></button>
        </HoverTooltip>
      </div>
    </div>
  );
}
