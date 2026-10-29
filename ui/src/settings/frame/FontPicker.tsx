import { ChevronDown, RotateCw } from 'lucide-react';
import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import type { FontFamily } from '../../api/gen/FontFamily';
import { escOwners } from '../../app/modalKeys';
import { EDITOR_FONT_NAME } from '../../theme/editorFont';
import { loadFontFamilies, refreshFontFamilies, useFontFamilies } from '../../theme/fontList';
import { singleFamily, storedFamily, withFallbacks } from '../../theme/fontStack';
import { ClearableInput } from '../../ui/ClearableInput';
import { HoverTooltip, placeCard } from '../../ui/HoverTooltip';
import { registerKeys } from '../../ui/keyRouter';
import { swallowGestureClick } from '../../ui/swallowClick';
import '../../ui/picker.css';
import './fontPicker.css';

/** What a font setting holds, as the picker shows it: the default, one family (installed or not),
 * or a hand-written list. */
export type FontChoice = { kind: 'default' } | { kind: 'family'; name: string; installed: boolean } | { kind: 'custom' };

/** `name`'s spelling among the installed families (and the bundled font), or null. CSS matches
 * names regardless of case. */
export function installedName(name: string, list: readonly FontFamily[] | null): string | null {
  const key = name.toLowerCase();
  if (key === EDITOR_FONT_NAME.toLowerCase()) return EDITOR_FONT_NAME;
  return list?.find((f) => f.name.toLowerCase() === key)?.name ?? null;
}

/** `value` as the picker shows it. Before the list loads, a family counts as installed (no flash
 * of "not installed"). */
export function fontChoice(value: string, list: readonly FontFamily[] | null): FontChoice {
  if (!value.trim()) return { kind: 'default' };
  const one = singleFamily(value);
  if (one === null) return { kind: 'custom' };
  const known = installedName(one, list);
  return { kind: 'family', name: known ?? one, installed: known !== null || list === null };
}

/** One font setting's picker: which setting, what its default is, and what follows a family. */
export interface FontRole {
  /** The setting (its row's `input-<id>` is the picker's button). */
  id: string;
  /** What the default is, in grey after "Default". */
  defaultName: string;
  /** The default's CSS stack, which draws "Default". */
  defaultStack: string;
  /** What follows a family's name where it's drawn (the stack the setting applies after it). */
  fallback: string;
  /** Lists monospace families only, with a "Show all fonts" toggle (the editor's). */
  monospace?: boolean;
}

interface Item { key: string; value: string; label: string; detail?: string; font?: string; custom?: true }

const ROW = 26;
/** The list's height: rows past it scroll, and only the rows in view (and a few around) render. */
const LIST_MAX = 312;
const OVERSCAN = 6;
const CUSTOM_KEY = 'custom';

/** The list's entries for `query`: Default, the bundled font, a set font that isn't installed, the
 * installed families (monospace ones only unless `all`), then "Custom font list…". */
export function fontItems(role: FontRole, value: string, list: readonly FontFamily[] | null, all: boolean, query: string): Item[] {
  const choice = fontChoice(value, list);
  const items: Item[] = [{ key: 'default', value: '', label: 'Default', detail: `(${role.defaultName})`, font: role.defaultStack }];
  items.push({ key: `f:${EDITOR_FONT_NAME}`, value: EDITOR_FONT_NAME, label: EDITOR_FONT_NAME, detail: 'bundled', font: withFallbacks(EDITOR_FONT_NAME, role.fallback) });
  if (choice.kind === 'family' && !choice.installed) items.push({ key: `f:${choice.name}`, value: storedFamily(choice.name), label: choice.name, detail: 'not installed', font: role.fallback });
  const current = choice.kind === 'family' ? choice.name.toLowerCase() : null;
  for (const f of list ?? []) {
    if (f.name.toLowerCase() === EDITOR_FONT_NAME.toLowerCase()) continue;
    if (role.monospace && !all && !f.monospace && f.name.toLowerCase() !== current) continue;
    items.push({ key: `f:${f.name}`, value: storedFamily(f.name), label: f.name, font: withFallbacks(f.name, role.fallback) });
  }
  const needle = query.trim().toLowerCase();
  const shown = needle ? items.filter((i) => `${i.label} ${i.detail ?? ''}`.toLowerCase().includes(needle)) : items;
  return [...shown, { key: CUSTOM_KEY, value, label: 'Custom font list…', custom: true }];
}

/** Whether `item` is what `value` holds. */
const isCurrent = (item: Item, value: string, list: readonly FontFamily[] | null): boolean => {
  if (item.custom) return fontChoice(value, list).kind === 'custom';
  if (item.value === '') return value.trim() === '';
  const one = singleFamily(value);
  return one !== null && one.toLowerCase() === (singleFamily(item.value) ?? '').toLowerCase();
};

/**
 * A font setting's dropdown: the button shows the choice, its name drawn in that font; it opens a
 * search box over the families, each drawn in its own font (virtualised: a system can have
 * hundreds). Typing filters, ↑/↓ (Page Up/Down by ten) move, Enter picks, Esc or a press outside
 * closes, and so does a second click on the button. The family under the pointer or the arrows
 * previews (`onPreview`) until it closes; picking stores it (`onPick`). "Custom font list…" asks
 * for the hand-written list instead (`onCustom`).
 */
export function FontPicker({ role, value, onPick, onPreview, onCustom }: { role: FontRole; value: string; onPick(v: string): void; onPreview(v: string | null): void; onCustom(): void }) {
  const list = useFontFamilies((s) => s.list);
  const btn = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  // The list loads with the first picker on screen, not at startup.
  useEffect(() => { void loadFontFamilies(); }, []);
  const choice = fontChoice(value, list);

  // ↓/↑ on the focused button open it, and so does typing (the key starts the search). The key
  // router's menu layer, as Select does: the Settings dialog's claim would stop an onKeyDown.
  const [seed, setSeed] = useState('');
  const [focused, setFocused] = useState(false);
  useEffect(() => {
    if (!focused || open) return;
    return registerKeys('menu', (e) => {
      if (e.target !== btn.current || e.ctrlKey || e.altKey || e.metaKey) return;
      const typed = e.key.length === 1 && e.key !== ' ';
      if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp' && !typed) return;
      e.preventDefault();
      setSeed(typed ? e.key : '');
      setOpen(true);
      return 'handled';
    });
  }, [focused, open]);

  const close = (refocus: boolean) => {
    setOpen(false);
    setSeed('');
    onPreview(null);
    if (refocus) btn.current?.focus({ preventScroll: true });
  };
  const label = choice.kind === 'default'
    ? <><span className="font-picker-name" style={{ fontFamily: role.defaultStack }}>Default</span><span className="font-picker-detail">({role.defaultName})</span></>
    : choice.kind === 'custom'
      ? <><span className="font-picker-name">Custom</span><span className="font-picker-detail font-picker-list">{value}</span></>
      : <><span className="font-picker-name" style={{ fontFamily: choice.installed ? withFallbacks(choice.name, role.fallback) : role.fallback }}>{choice.name}</span>{!choice.installed && <span className="font-picker-detail">not installed</span>}</>;
  return (
    <>
      <button
        ref={btn}
        id={`input-${role.id}`}
        type="button"
        className="select font-picker-button"
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => (open ? close(true) : setOpen(true))}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
      >
        <span className="select-value">{label}</span>
        <ChevronDown size={14} aria-hidden className="select-caret" />
      </button>
      {open && btn.current && (
        <FontList
          role={role}
          value={value}
          list={list}
          seed={seed}
          anchor={btn.current}
          onPreview={onPreview}
          onPick={(item) => {
            close(!item.custom);
            if (item.custom) onCustom();
            else if (item.value !== value) onPick(item.value);
          }}
          onClose={close}
        />
      )}
    </>
  );
}

function FontList({ role, value, list, seed, anchor, onPreview, onPick, onClose }: {
  role: FontRole; value: string; list: readonly FontFamily[] | null; seed: string; anchor: HTMLElement;
  onPreview(v: string | null): void; onPick(item: Item): void; onClose(refocus: boolean): void;
}) {
  const uid = useId();
  const loading = useFontFamilies((s) => s.loading);
  const [query, setQuery] = useState(seed);
  const [all, setAll] = useState(false);
  const items = useMemo(() => fontItems(role, value, list, all, query), [role, value, list, all, query]);
  const [cursor, setCursor] = useState(() => Math.max(0, items.findIndex((i) => isCurrent(i, value, list))));
  const at = Math.min(cursor, items.length - 1);
  const box = useRef<HTMLDivElement>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const live = useRef({ items, at, onPick, onClose });
  live.current = { items, at, onPick, onClose };

  // The entry under the pointer or the arrows previews; "Custom font list…" previews what's set.
  const previewed = items[at];
  useEffect(() => { onPreview(previewed && !previewed.custom ? previewed.value : null); }, [previewed?.key]); // eslint-disable-line react-hooks/exhaustive-deps

  // Keeps the cursor's row in view: centred at the opening, at the nearest edge after.
  const placed = useRef(false);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const top = at * ROW;
    const view = Math.min(LIST_MAX, items.length * ROW);
    let next = el.scrollTop;
    if (!placed.current) next = top - (view - ROW) / 2;
    else if (top < el.scrollTop) next = top;
    else if (top + ROW > el.scrollTop + view) next = top + ROW - view;
    next = Math.max(0, Math.min(next, items.length * ROW - view));
    placed.current = true;
    if (next !== el.scrollTop) {
      el.scrollTop = next;
      setScrollTop(next);
    }
  }, [at, items.length]);

  // Placed under the button (above when there's no room), at least as wide.
  const [pos, setPos] = useState<{ left: number; top: number; width: number }>(() => ({ left: 0, top: 0, width: 0 }));
  useLayoutEffect(() => {
    const r = anchor.getBoundingClientRect();
    const width = Math.max(r.width, 320);
    const p = placeCard({ left: r.left, top: r.top, bottom: r.bottom }, { width, height: box.current?.offsetHeight ?? 0 }, 4);
    setPos({ left: p.left, top: p.top, width });
  }, [anchor, items.length]);

  // Esc closes it, not the Settings dialog under it (an Esc owner, as the colour picker is); the
  // arrows, Page Up/Down and Enter are taken in the menu layer, before the dialog's claim.
  useEffect(() => {
    const own = () => true;
    escOwners.add(own);
    const off = registerKeys('menu', (e) => {
      if (e.ctrlKey || e.altKey || e.metaKey || e.isComposing) return;
      const { items: its, at: i } = live.current;
      const move = { ArrowDown: 1, ArrowUp: -1, PageDown: 10, PageUp: -10 }[e.key];
      if (move !== undefined) setCursor(Math.max(0, Math.min(its.length - 1, i + move)));
      else if (e.key === 'Enter') { if (its[i]) live.current.onPick(its[i]!); }
      else if (e.key === 'Escape') live.current.onClose(true);
      else return;
      e.preventDefault();
      return 'handled';
    });
    return () => { escOwners.delete(own); off(); };
  }, []);

  // A press outside closes it; one on the button closes it too, and its click mustn't reopen it.
  useEffect(() => {
    const down = (e: PointerEvent) => {
      const t = e.target instanceof Node ? e.target : null;
      if (t && box.current?.contains(t)) return;
      if (t && anchor.contains(t)) swallowGestureClick((c) => c.target instanceof Node && anchor.contains(c.target));
      live.current.onClose(false);
    };
    document.addEventListener('pointerdown', down, true);
    return () => document.removeEventListener('pointerdown', down, true);
  }, [anchor]);

  const view = Math.min(LIST_MAX, items.length * ROW);
  const first = Math.max(0, Math.floor(scrollTop / ROW) - OVERSCAN);
  const last = Math.min(items.length - 1, Math.ceil((scrollTop + view) / ROW) + OVERSCAN);
  const rows: number[] = [];
  for (let i = first; i <= last; i++) rows.push(i);
  if (at < first || at > last) rows.push(at); // the active option is always in the DOM (aria-activedescendant)
  // Inside the Settings dialog, so its focus trap counts the search box and the toggle.
  const host = anchor.closest('[role="dialog"]') ?? document.body;
  return createPortal(
    <div
      ref={box}
      className="font-picker"
      style={{ left: pos.left, top: pos.top, width: pos.width }}
      onBlur={(e) => {
        const to = e.relatedTarget;
        if (to instanceof Node && (box.current?.contains(to) || anchor.contains(to))) return;
        if (to !== null) live.current.onClose(false); // Tab moved on; a click outside closes it itself
      }}
    >
      <div className="font-picker-head">
        <ClearableInput
          autoFocus
          className="picker-input"
          placeholder="Search fonts"
          aria-label="Search fonts"
          role="combobox"
          aria-expanded
          aria-controls={`${uid}-list`}
          aria-activedescendant={items[at] ? `${uid}-${at}` : undefined}
          spellCheck={false}
          autoComplete="off"
          value={query}
          onChange={(e) => { setQuery(e.target.value); setCursor(0); }}
          onClear={() => { setQuery(''); setCursor(0); }}
        />
        <HoverTooltip content="List the fonts again">
          <button
            type="button"
            className="icon-button"
            aria-label="List the fonts again"
            disabled={loading}
            onPointerDown={(e) => e.preventDefault()}
            onClick={() => { void refreshFontFamilies(); }}
          >
            <RotateCw size={13} aria-hidden />
          </button>
        </HoverTooltip>
      </div>
      {role.monospace && (
        <label className="font-picker-all">
          <input type="checkbox" checked={all} onChange={(e) => { setAll(e.target.checked); setCursor(0); }} />
          Show all fonts
        </label>
      )}
      <div ref={scroller} className="font-picker-list" style={{ height: view }} onMouseDown={(e) => e.preventDefault()} onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}>
        <div role="listbox" id={`${uid}-list`} aria-label="Fonts" style={{ height: items.length * ROW, position: 'relative' }}>
          {rows.map((i) => {
            const item = items[i]!;
            const style: CSSProperties = { top: i * ROW, height: ROW };
            return (
              <div
                key={item.key}
                id={`${uid}-${i}`}
                role="option"
                aria-selected={i === at}
                data-current={isCurrent(item, value, list) || undefined}
                className={item.custom ? 'font-picker-item font-picker-custom' : 'font-picker-item'}
                style={style}
                onPointerEnter={() => setCursor(i)}
                onClick={() => onPick(item)}
              >
                <span className="font-picker-name" style={item.font ? { fontFamily: item.font } : undefined}>{item.label}</span>
                {item.detail && <span className="font-picker-detail">{item.detail}</span>}
              </div>
            );
          })}
        </div>
      </div>
      {items.length === 1 && <div className="picker-empty">{list === null ? 'Listing the fonts…' : 'No matches'}</div>}
    </div>,
    host,
  );
}
