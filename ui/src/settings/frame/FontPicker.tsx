import { ChevronDown, RotateCw } from 'lucide-react';
import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type RefObject } from 'react';
import { createPortal } from 'react-dom';
import type { FontFamily } from '../../api/gen/FontFamily';
import { escOwners } from '../../app/modalKeys';
import { EDITOR_FONT_NAME } from '../../theme/editorFont';
import { loadFontFamilies, refreshFontFamilies, useFontFamilies } from '../../theme/fontList';
import { singleFamily, storedFamily, withFallbacks } from '../../theme/fontStack';
import { ClearableInput } from '../../ui/ClearableInput';
import { HoverTooltip } from '../../ui/HoverTooltip';
import { registerKeys } from '../../ui/keyRouter';
import { swallowGestureClick } from '../../ui/swallowClick';
import { pressOpener, usePressDrag } from '../../menu/pressDrag';
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
  /** Which font, in "Preview of the … font". */
  noun: string;
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
/** The fewest rows a list squeezed for room shows before the page scrolls to make more. */
const MIN_ROWS = 5;
const EDGE = 8;
const GAP = 4;
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

interface Box { left: number; top: number; right: number; bottom: number }
/** Where the list goes: its box, and how tall its scrolled rows are (`view`). */
export interface ListPlace { left: number; top: number; width: number; view: number; below: boolean }

/**
 * The list's place by the button `anchor`: at its width, under it, else above it, wherever all of
 * it fits without covering `avoid` (the preview: what's under the pointer must show). With room for
 * neither, it takes the larger side that keeps clear and scrolls fewer rows. `chrome` is the list's
 * height besides its rows (the search box, the toggle), `rows` its rows' full height; it stays
 * within `bounds` (the dialog).
 */
export function placeList(anchor: Box, chrome: number, rows: number, avoid: Box | null, bounds: Box): ListPlace {
  const width = anchor.right - anchor.left;
  const left = Math.max(bounds.left + EDGE, Math.min(anchor.left, bounds.right - EDGE - width));
  const clear = (top: number, height: number) => !avoid
    || left >= avoid.right || left + width <= avoid.left || top >= avoid.bottom || top + height <= avoid.top;
  const height = chrome + rows;
  const roomBelow = bounds.bottom - EDGE - anchor.bottom - GAP;
  const roomAbove = anchor.top - GAP - EDGE - bounds.top;
  const below = (h: number): ListPlace => ({ left, top: anchor.bottom + GAP, width, view: h - chrome, below: true });
  const above = (h: number): ListPlace => ({ left, top: anchor.top - GAP - h, width, view: h - chrome, below: false });
  if (height <= roomBelow && clear(anchor.bottom + GAP, height)) return below(height);
  if (height <= roomAbove && clear(anchor.top - GAP - height, height)) return above(height);
  const okBelow = clear(anchor.bottom + GAP, roomBelow);
  const okAbove = clear(anchor.top - GAP - roomAbove, roomAbove);
  const fit = (room: number) => Math.max(chrome + ROW, Math.min(height, room));
  if (okAbove && (!okBelow || roomAbove > roomBelow)) return above(fit(roomAbove));
  return below(fit(roomBelow));
}

/** The nearest ancestor of `el` that scrolls vertically. */
function scrollParent(el: HTMLElement): HTMLElement | null {
  for (let p = el.parentElement; p; p = p.parentElement) {
    if (p.scrollHeight > p.clientHeight && /auto|scroll/.test(getComputedStyle(p).overflowY)) return p;
  }
  return null;
}

/**
 * A font setting's dropdown: the button shows the choice, its name drawn in that font; it opens a
 * search box over the families, each drawn in its own font (virtualised: a system can have
 * hundreds). Typing filters, ↑/↓ (Page Up/Down by ten) move, Enter picks, Esc or a press outside
 * closes, and so does a second click on the button. The family under the pointer or the arrows
 * previews (`onPreview`) until it closes; picking stores it (`onPick`), and the list never covers
 * `avoid` (the preview panel). "Custom font list…" asks for the hand-written list instead (`onCustom`).
 */
export function FontPicker({ role, value, avoid, onPick, onPreview, onCustom }: {
  role: FontRole; value: string; avoid?: RefObject<HTMLElement | null>; onPick(v: string): void; onPreview(v: string | null): void; onCustom(): void;
}) {
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
        {...pressOpener(() => (open ? close(true) : setOpen(true)))}
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
          avoid={avoid}
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

function FontList({ role, value, list, seed, anchor, avoid, onPreview, onPick, onClose }: {
  role: FontRole; value: string; list: readonly FontFamily[] | null; seed: string; anchor: HTMLElement; avoid?: RefObject<HTMLElement | null>;
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
  // Opened by a press on the button: dragging onto a font (previewing each on the way) and
  // releasing picks it.
  usePressDrag(box);
  const scroller = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const live = useRef({ items, at, onPick, onClose });
  live.current = { items, at, onPick, onClose };

  // The entry under the pointer or the arrows previews; "Custom font list…" previews what's set.
  const previewed = items[at];
  useEffect(() => { onPreview(previewed && !previewed.custom ? previewed.value : null); }, [previewed?.key]); // eslint-disable-line react-hooks/exhaustive-deps

  // Placed by the button, clear of the preview (before the first paint). Squeezed below the button for
  // room, it first scrolls the page up to make some, as far as the preview stays in view.
  const [place, setPlace] = useState<ListPlace | null>(null);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const chrome = el.offsetHeight - (scroller.current?.offsetHeight ?? 0);
    const rows = Math.min(LIST_MAX, items.length * ROW);
    const dlg = anchor.closest('[role="dialog"]')?.getBoundingClientRect();
    const host = dlg?.height ? dlg : null;
    const bounds = { left: Math.max(0, host?.left ?? 0), top: Math.max(0, host?.top ?? 0), right: Math.min(window.innerWidth, host?.right ?? Infinity), bottom: Math.min(window.innerHeight, host?.bottom ?? Infinity) };
    const where = () => placeList(anchor.getBoundingClientRect(), chrome, rows, avoid?.current?.getBoundingClientRect() ?? null, bounds);
    let p = where();
    const want = Math.min(rows, MIN_ROWS * ROW);
    const page = p.below && p.view < want ? scrollParent(anchor) : null;
    if (page) {
      const a = anchor.getBoundingClientRect();
      const lack = a.bottom + GAP + chrome + want + EDGE - bounds.bottom;
      const keep = Math.min(a.top, avoid?.current?.getBoundingClientRect().top ?? Infinity);
      page.scrollTop += Math.max(0, Math.min(lack, keep - page.getBoundingClientRect().top));
      p = where();
    }
    setPlace(p);
  }, [anchor, avoid, items.length]);
  const view = Math.min(place?.view ?? LIST_MAX, items.length * ROW);

  // Keeps the cursor's row in view: centred at the opening, at the nearest edge after.
  const placed = useRef(false);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el || !place) return;
    const top = at * ROW;
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
  }, [at, items.length, view, place]);

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
      style={place ? { left: place.left, top: place.top, width: place.width } : { left: 0, top: 0, width: anchor.offsetWidth }}
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
