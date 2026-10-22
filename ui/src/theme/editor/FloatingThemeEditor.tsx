import { GripVertical } from 'lucide-react';
import { useEffect, useId, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react';
import { flushSync } from 'react-dom';
import { useSettingsUi } from '../../settings/schema';
import { registerKeyHints } from '../../shortcuts/hints';
import { ClearableInput } from '../../ui/ClearableInput';
import { useKeys } from '../../ui/keyRouter';
import { useEditedTheme } from './actions';
import { LaneColours } from './LaneColours';
import { ThemeColourList } from './ThemeColourList';
import './editor.css';
import './floating.css';

interface Offset { x: number; y: number }
/** Where the panel was dragged to, from its place at the window's lower right: kept for the session. */
let saved: Offset = { x: 0, y: 0 };
const STEP = 16;
const ARROWS: Record<string, Offset> = { ArrowLeft: { x: -STEP, y: 0 }, ArrowRight: { x: STEP, y: 0 }, ArrowUp: { x: 0, y: -STEP }, ArrowDown: { x: 0, y: STEP } };

/** `next`, moved back inside the window. `at` is the offset the panel is drawn at now. */
function inWindow(el: HTMLElement, at: Offset, next: Offset): Offset {
  const r = el.getBoundingClientRect();
  const left = r.left - at.x;
  const top = r.top - at.y;
  return {
    x: Math.round(Math.max(-left, Math.min(window.innerWidth - r.width - left, next.x))),
    y: Math.round(Math.max(-top, Math.min(window.innerHeight - r.height - top, next.y))),
  };
}

/**
 * Settings › Appearance's "Edit on the app": the theme editor as a small panel over the live app,
 * which stays usable behind it (no backdrop, no dimming, no click-outside close, no claim on the
 * keys). Its title bar drags it (or Alt+arrows on it); Find a colour filters the colours. Done, or
 * Esc while focus is inside, brings Settings back on Appearance, focused on the button that opened it.
 */
export function FloatingThemeEditor() {
  const { def } = useEditedTheme();
  const [filter, setFilter] = useState('');
  const [at, setAt] = useState<Offset>(saved);
  const panel = useRef<HTMLDivElement>(null);
  const bar = useRef<HTMLDivElement>(null);
  const find = useRef<HTMLInputElement>(null);
  const opener = useRef<Element | null>(null);
  const drag = useRef<{ id: number; x: number; y: number; from: Offset } | null>(null);
  const titleId = useId();

  const moveTo = (next: Offset) => {
    if (!panel.current) return;
    saved = inWindow(panel.current, saved, next);
    setAt(saved);
  };

  // An effect, not autoFocus: Settings gives the focus back to where it was before it opened when
  // it unmounts, which runs first; that element is where focus goes when Settings closes later.
  useEffect(() => {
    opener.current = document.activeElement;
    find.current?.focus({ preventScroll: true });
  }, []);

  // Inside the window as it is now (it may have shrunk since the last time), and as it resizes.
  useEffect(() => {
    const fit = () => moveTo(saved);
    fit();
    window.addEventListener('resize', fit);
    return () => window.removeEventListener('resize', fit);
    // `moveTo` reads only refs and the module's `saved`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const done = () => {
    // Settings returns focus, when it closes, to what has it as it opens: the element from before.
    const back = opener.current;
    if (back instanceof HTMLElement && back !== document.body && back.isConnected) back.focus({ preventScroll: true });
    else if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
    flushSync(() => {
      useSettingsUi.getState().setPage('appearance');
      useSettingsUi.getState().setFloating(null);
    });
    document.querySelector<HTMLElement>('[data-float-opener="theme-editor"]')?.focus({ preventScroll: true });
  };

  // Through the key router, above the app's own keys (Esc closing the file, Alt+←/→ going back):
  // only for keys pressed inside the panel. The colour picker takes its Esc a layer higher.
  useKeys('overlay', (e) => {
    const t = e.target instanceof Element ? e.target : null;
    if (!t || !panel.current?.contains(t) || e.ctrlKey || e.metaKey) return;
    if (e.key === 'Escape' && !e.altKey && !e.shiftKey) {
      e.preventDefault();
      // As in Settings' search: Esc clears the find box first.
      if (t === find.current && filter !== '') setFilter('');
      else done();
      return 'handled';
    }
    const step = ARROWS[e.key];
    if (step && e.altKey && !e.shiftKey && t === bar.current) {
      e.preventDefault();
      moveTo({ x: at.x + step.x, y: at.y + step.y });
      return 'handled';
    }
  });

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || (e.target instanceof Element && e.target.closest('button'))) return;
    e.currentTarget.setPointerCapture?.(e.pointerId);
    drag.current = { id: e.pointerId, x: e.clientX, y: e.clientY, from: at };
  };
  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || d.id !== e.pointerId) return;
    moveTo({ x: d.from.x + e.clientX - d.x, y: d.from.y + e.clientY - d.y });
  };
  const onPointerEnd = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (drag.current?.id === e.pointerId) drag.current = null;
  };

  const q = filter.trim().toLowerCase();
  return (
    <div
      ref={panel}
      className="theme-float"
      role="dialog"
      aria-modal="false"
      aria-labelledby={titleId}
      style={{ transform: at.x || at.y ? `translate(${at.x}px, ${at.y}px)` : undefined }}
    >
      <div
        ref={bar}
        className="theme-float-bar"
        role="group"
        tabIndex={0}
        aria-label={`Move the ${def.label} editor`}
        aria-keyshortcuts="Alt+ArrowLeft Alt+ArrowRight Alt+ArrowUp Alt+ArrowDown"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerEnd}
        onPointerCancel={onPointerEnd}
      >
        <GripVertical size={14} className="theme-float-grip" aria-hidden />
        <h2 id={titleId} className="theme-float-title">{def.label}</h2>
        <button type="button" className="theme-float-done" onClick={done}>Done</button>
      </div>
      <div className="theme-float-find">
        <ClearableInput
          ref={find}
          type="search"
          aria-label="Find a colour"
          placeholder="Find a colour"
          spellCheck={false}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          onClear={() => setFilter('')}
        />
      </div>
      <div className="theme-float-body">
        <ThemeColourList compact filter={filter}>
          {(!q || 'graph lanes'.includes(q)) && (
            <div className="theme-colour-group" role="group" aria-labelledby={`${titleId}-lanes`}>
              <h4 id={`${titleId}-lanes`}>Graph lanes</h4>
              <LaneColours compact />
            </div>
          )}
        </ThemeColourList>
      </div>
    </div>
  );
}

// Shown in the Keyboard Shortcuts panel (Ctrl+/); metadata only.
registerKeyHints([
  { id: 'themeFloat.move', section: 'Theme editor', label: 'Move the floating theme editor', keys: ['Alt+Left', 'Alt+Right', 'Alt+Up', 'Alt+Down'], context: '(when its title bar has focus; or drag it)', source: 'theme/editor/FloatingThemeEditor.tsx' },
  { id: 'themeFloat.done', section: 'Theme editor', label: 'Back to Settings', keys: ['Esc'], context: '(when the floating theme editor has focus)', source: 'theme/editor/FloatingThemeEditor.tsx' },
]);
