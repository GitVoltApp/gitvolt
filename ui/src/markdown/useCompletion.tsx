import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent, type ReactElement, type ReactNode, type RefObject } from 'react';
import { escOwners } from '../app/modalKeys';
import { placePopover, popoverRoom, viewport } from '../ui/arm/anchor';
import { insertToken, MAX_HITS, type Trigger } from './emojiComplete';
import './completion.css';

const MIRRORED = ['direction', 'boxSizing', 'width', 'height', 'overflowX', 'overflowY', 'borderTopWidth', 'borderRightWidth', 'borderBottomWidth', 'borderLeftWidth', 'borderStyle', 'paddingTop', 'paddingRight', 'paddingBottom', 'paddingLeft', 'fontStyle', 'fontVariant', 'fontWeight', 'fontStretch', 'fontSize', 'lineHeight', 'fontFamily', 'textAlign', 'textTransform', 'textIndent', 'letterSpacing', 'wordSpacing', 'tabSize', 'whiteSpace', 'wordWrap', 'overflowWrap'] as const;

/** A text box completion works in: a textarea, or a one-line input (the commit summary). */
export type TextBox = HTMLTextAreaElement | HTMLInputElement;

/** The viewport position (left, and the top and bottom of the line) of the character at `index`, found
 * by laying the text before it out in an off-screen copy of the text box. */
export function caretPoint(area: TextBox, index: number): { left: number; top: number; bottom: number } {
  const style = getComputedStyle(area);
  const mirror = document.createElement('div');
  for (const p of MIRRORED) mirror.style[p] = style[p];
  mirror.style.position = 'absolute';
  mirror.style.visibility = 'hidden';
  mirror.style.top = '0';
  mirror.style.left = '-9999px';
  mirror.style.whiteSpace = 'pre-wrap';
  mirror.style.overflow = 'hidden';
  mirror.textContent = area.value.slice(0, index);
  const mark = document.createElement('span');
  mark.textContent = '​';
  mirror.append(mark);
  document.body.append(mirror);
  const line = parseFloat(style.lineHeight) || parseFloat(style.fontSize) * 1.5 || 18;
  const x = mark.offsetLeft - area.scrollLeft;
  const y = mark.offsetTop - area.scrollTop;
  mirror.remove();
  const box = area.getBoundingClientRect();
  return { left: box.left + x, top: box.top + y, bottom: box.top + y + line };
}

/** Where the popup goes: under the trigger's colon, or under the caret when the colon has
 * scrolled out of a one-line box's view (a long summary). */
function anchorPoint(area: TextBox, start: number): { left: number; top: number; bottom: number } {
  const p = caretPoint(area, start);
  const box = area.getBoundingClientRect();
  return p.left >= box.left && p.left <= box.right ? p : caretPoint(area, area.selectionStart ?? start);
}

/** Replaces `from`..`to` of the box's text with `text` as typing would, so the box's own undo
 * (Cmd+Z) takes it back; false where the browser can't (the tests' DOM), and the caller then sets
 * the value itself. */
export function typeInto(box: TextBox, from: number, to: number, text: string): boolean {
  if (typeof document.execCommand !== 'function') return false;
  box.focus();
  box.setSelectionRange(from, to);
  // Deprecated, but still the only edit that goes through a text box's undo history.
  return document.execCommand('insertText', false, text);
}

/** One kind of completion: a trigger character and where its candidates come from. */
export interface Provider<T> {
  /** The popup's accessible name ("Emoji", "People"). */
  label: string;
  find(text: string, caret: number): Trigger | null;
  /** Candidates known now, best first. */
  local(query: string): T[];
  /** More candidates from a slow source (asked after a short debounce); optional. */
  remote?(query: string): Promise<T[]>;
  /** A class the popup takes besides its own (its width, its rows' look). */
  className?: string;
  /** How many candidates the popup lists (it scrolls); `MAX_HITS` by default. */
  max?: number;
  /** Called when the trigger first shows (to load the data). */
  prepare?(): void;
  key(item: T): string;
  /** The text that replaces the trigger and query (with its trailing space). */
  token(item: T): string;
  row(item: T): ReactNode;
}

export const REMOTE_DEBOUNCE_MS = 150;

/** The popup keeps at least this height (about four rows) in a window too short for more. */
const MIN_HEIGHT = 96;

export interface Completion {
  /** Handles a key for the popup; true when it was the popup's (the caller then does nothing). */
  keyDown(e: KeyboardEvent<HTMLElement>): boolean;
  /** Tell it the text or caret changed. */
  sync(): void;
  /** Props for the textarea (it keeps its textbox role, so it keeps its name in queries). */
  aria: { 'aria-autocomplete': 'list'; 'aria-expanded': boolean; 'aria-controls'?: string; 'aria-activedescendant'?: string };
  popup: ReactElement | null;
}

/** Autocomplete for a controlled text box from `providers` (the first whose trigger is at the
 * caret wins): the popup at the caret, ↑/↓, Enter/Tab to insert, Esc to close. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export function useCompletion(area: RefObject<TextBox | null>, value: string, onChange: (v: string) => void, providers: readonly (Provider<any> | null)[]): Completion {
  const id = useId();
  const [caret, setCaret] = useState(0);
  const [active, setActive] = useState(0);
  const [closedAt, setClosedAt] = useState<number | null>(null);
  // Closed while the box hasn't focus (a click elsewhere); back when it has it again.
  const [blurred, setBlurred] = useState(false);
  // A paste or a drop doesn't open it: only typing does. The text it left, until the caret is
  // known in it (the select event can come after the change).
  const pasted = useRef<string | null>(null);
  useEffect(() => {
    const mine = (e: Event) => e.target === area.current;
    const focusIn = (e: Event) => { if (mine(e)) setBlurred(false); };
    const focusOut = (e: Event) => { if (mine(e)) setBlurred(true); };
    const input = (e: Event) => { if (mine(e)) pasted.current = (e as InputEvent).inputType?.startsWith('insertFrom') ? area.current!.value : null; };
    document.addEventListener('focusin', focusIn);
    document.addEventListener('focusout', focusOut);
    document.addEventListener('input', input, true);
    return () => {
      document.removeEventListener('focusin', focusIn);
      document.removeEventListener('focusout', focusOut);
      document.removeEventListener('input', input, true);
    };
  }, [area]);
  const found = useMemo(() => {
    for (let i = 0; i < providers.length; i++) {
      const t = providers[i]?.find(value, caret) ?? null;
      if (t) return { i, t };
    }
    return null;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, caret, ...providers]);
  const provider = found ? providers[found.i]! : null;
  const trigger = found?.t ?? null;
  const query = trigger?.query;
  const which = found?.i;
  useEffect(() => { if (found) provider?.prepare?.(); }, [found !== null, provider]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { if (!trigger) setClosedAt(null); }, [trigger]);
  useEffect(() => setActive(0), [query, which]);
  const triggerAt = trigger?.start;
  useLayoutEffect(() => {
    if (pasted.current === null) return;
    if (pasted.current !== value) pasted.current = null;
    else if (triggerAt !== undefined) {
      pasted.current = null;
      setClosedAt(triggerAt);
    }
  }, [value, triggerAt]);

  const [more, setMore] = useState<{ tag: string; items: unknown[] }>({ tag: '', items: [] });
  const tag = `${which}:${query}`;
  const remote = provider?.remote;
  useEffect(() => {
    if (!remote || query === undefined) return;
    let live = true;
    const t = setTimeout(() => {
      remote(query).then((items) => { if (live) setMore({ tag, items }); }, () => undefined);
    }, REMOTE_DEBOUNCE_MS);
    return () => { live = false; clearTimeout(t); };
  }, [remote, query, tag]);

  const items = useMemo(() => {
    if (!provider || query === undefined) return [];
    const local = provider.local(query);
    const seen = new Set(local.map((x) => provider.key(x)));
    const extra = more.tag === tag ? more.items.filter((x) => !seen.has(provider.key(x))) : [];
    return [...local, ...extra].slice(0, provider.max ?? MAX_HITS);
  }, [provider, query, more, tag]);
  const open = trigger !== null && items.length > 0 && closedAt !== trigger.start && !blurred;
  const optionId = (i: number) => `${id}-opt-${i}`;
  const current = Math.min(active, items.length - 1);
  // ↑/↓ past the popup's height scroll the active row into view.
  useEffect(() => {
    if (open) document.getElementById(optionId(current))?.scrollIntoView?.({ block: 'nearest' });
  }, [open, current]); // eslint-disable-line react-hooks/exhaustive-deps

  const start = trigger?.start;
  // An open popup owns Esc inside a dialog (the MR view): it closes the popup, not the dialog.
  const openAt = open ? start : undefined;
  useEffect(() => {
    if (openAt === undefined) return;
    const own = () => { setClosedAt(openAt); return true; };
    escOwners.add(own);
    return () => { escOwners.delete(own); };
  }, [openAt]);

  const [point, setPoint] = useState<{ left: number; top: number; bottom: number } | null>(null);
  const pop = useRef<HTMLUListElement>(null);
  // Found again when the window or the box resizes, or a panel scrolls, while it's open.
  useEffect(() => {
    const a = area.current;
    if (!open || !a || start === undefined) { setPoint(null); return; }
    const locate = () => setPoint(anchorPoint(a, start));
    const scrolled = (e: Event) => { if (!(e.target instanceof Node && pop.current?.contains(e.target))) locate(); };
    locate();
    window.addEventListener('resize', locate);
    window.addEventListener('scroll', scrolled, true);
    const resized = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(locate);
    resized?.observe(a);
    return () => {
      window.removeEventListener('resize', locate);
      window.removeEventListener('scroll', scrolled, true);
      resized?.disconnect();
    };
  }, [open, start, value, area]);
  // Under the caret, but kept on screen: a caret near the window's right edge (the details
  // panel's summary) would leave the popup mostly off it, and a short window gets a shorter
  // list. On screen, not just in the window: a window can hang off the screen's bottom. Measured before paint, at the height the CSS gives it.
  const [place, setPlace] = useState<{ left: number; top: number; maxHeight?: number } | null>(null);
  useLayoutEffect(() => {
    const el = pop.current;
    if (!point || !el) { setPlace(null); return; }
    const capped = el.style.maxHeight;
    el.style.maxHeight = '';
    const r = el.getBoundingClientRect();
    el.style.maxHeight = capped;
    const vp = viewport();
    const h = Math.max(Math.min(r.height, popoverRoom(point, vp.h)), Math.min(r.height, MIN_HEIGHT));
    setPlace({ ...placePopover(point, { w: r.width, h }, vp), ...(h < r.height ? { maxHeight: h } : {}) });
  }, [point, items.length]);
  // The pointer picks a row only when it moves: the list scrolling under a still pointer (↑/↓)
  // must not take the keyboard's choice.
  const pointer = useRef<{ x: number; y: number } | null>(null);
  const hover = (e: MouseEvent, i: number) => {
    const p = pointer.current;
    pointer.current = { x: e.screenX, y: e.screenY };
    if (p && (p.x !== e.screenX || p.y !== e.screenY)) setActive(i);
  };

  const sync = () => {
    const a = area.current;
    if (a) setCaret(a.selectionStart ?? 0);
  };
  const insert = (item: unknown) => {
    const a = area.current;
    if (!a || !trigger || !provider) return;
    const at = a.selectionStart ?? value.length;
    const token = provider.token(item);
    if (typeInto(a, trigger.start, at, token)) {
      setCaret(trigger.start + token.length);
      return;
    }
    const r = insertToken(value, at, trigger, token);
    onChange(r.text);
    setCaret(r.caret);
    requestAnimationFrame(() => { a.focus(); a.setSelectionRange(r.caret, r.caret); });
  };
  const keyDown = (e: KeyboardEvent<HTMLElement>): boolean => {
    // Enter while an input method composes (accents, Japanese) is the composition's own.
    if (!open || e.ctrlKey || e.metaKey || e.altKey || e.nativeEvent.isComposing || e.keyCode === 229) return false;
    const stop = () => { e.preventDefault(); e.stopPropagation(); };
    switch (e.key) {
      case 'ArrowDown': stop(); setActive((i) => (i + 1) % items.length); return true;
      case 'ArrowUp': stop(); setActive((i) => (i - 1 + items.length) % items.length); return true;
      case 'Enter':
      case 'Tab': stop(); insert(items[current]); return true;
      case 'Escape': stop(); setClosedAt(trigger!.start); return true;
      default: return false;
    }
  };
  const popup = open && point && provider ? (
    <ul id={`${id}-list`} className={provider.className ? `md-emoji-pop ${provider.className}` : 'md-emoji-pop'} role="listbox" aria-label={provider.label} ref={pop} style={place ?? { left: point.left, top: point.bottom + 2 }} onMouseDown={(e) => e.preventDefault()}>
      {items.map((it, i) => (
        <li key={provider.key(it)} id={optionId(i)} role="option" aria-selected={i === current} className="md-emoji-opt" onMouseMove={(e) => hover(e, i)} onClick={() => insert(it)}>
          {provider.row(it)}
        </li>
      ))}
    </ul>
  ) : null;
  return {
    keyDown,
    sync,
    aria: { 'aria-autocomplete': 'list', 'aria-expanded': open, ...(open ? { 'aria-controls': `${id}-list`, 'aria-activedescendant': optionId(current) } : {}) },
    popup,
  };
}
