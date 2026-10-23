import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import type { GhostBranch } from '../api/gen/GhostBranch';
import type { RowPayload } from '../api/gen/RowPayload';
import { relativeTime } from '../format/relative';
import { registerKeyHints } from '../shortcuts/hints';
import { useTheme } from '../theme/store';
import { showBranch, showBranches, useCurrentPreset } from './store';
import './presets.css';

/** How long the pointer may be between the ghost lines and the popover before it closes. */
const LEAVE_GRACE_MS = 150;

interface Open { row: RowPayload; at: DOMRect; focus: boolean }

/**
 * The ghost lines' popover state for a graph: `hover` is fed the ghost zone under the pointer (or
 * null), `openFor` opens it from the keyboard (focus moves in). The popover closes shortly after
 * the pointer leaves both the zone and itself, on Esc, and on a click elsewhere.
 */
export function useGhostPopover(tabId: string | null): {
  hover(row: RowPayload | null, at?: DOMRect): void;
  openFor(row: RowPayload, at: DOMRect): void;
  element: ReactNode;
} {
  const [open, setOpen] = useState<Open | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const clear = () => { if (timer.current !== null) { clearTimeout(timer.current); timer.current = null; } };
  useEffect(() => clear, []);
  const close = useCallback(() => { clear(); setOpen(null); }, []);
  const hover = useCallback((row: RowPayload | null, at?: DOMRect) => {
    if (row && at) {
      clear();
      setOpen((o) => (o && o.row.id === row.id && !o.focus ? o : { row, at, focus: false }));
    } else if (!timer.current) {
      timer.current = setTimeout(() => { timer.current = null; setOpen((o) => (o?.focus ? o : null)); }, LEAVE_GRACE_MS);
    }
  }, []);
  const openFor = useCallback((row: RowPayload, at: DOMRect) => { clear(); setOpen({ row, at, focus: true }); }, []);
  const element = open && tabId ? (
    <GhostPopover tabId={tabId} row={open.row} at={open.at} focus={open.focus} onEnter={clear} onLeave={() => hover(null)} onClose={close} />
  ) : null;
  return { hover, openFor, element };
}

/** "N hidden branches start here": per branch its chip (in its lane colour), its tip's summary,
 * author and age, and Show; then which preset hides them and Show all. */
export function GhostPopover({ tabId, row, at, focus, onEnter, onLeave, onClose }: {
  tabId: string; row: RowPayload; at: DOMRect; focus: boolean; onEnter?: () => void; onLeave?: () => void; onClose: () => void;
}) {
  const ghosts = row.ghosts ?? [];
  const lanes = useTheme((s) => s.colors.graph);
  const { current } = useCurrentPreset(tabId);
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  // Beside the node, kept inside the window; measured before paint so it never jumps.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    setPos({
      left: Math.max(8, Math.min(at.right + 8, window.innerWidth - width - 8)),
      top: Math.max(8, Math.min(at.top, window.innerHeight - height - 8)),
    });
  }, [at, ghosts.length]);
  useEffect(() => {
    if (focus) ref.current?.querySelector<HTMLElement>('button')?.focus({ preventScroll: true });
  }, [focus]);
  useEffect(() => {
    const away = (e: PointerEvent) => { if (!ref.current?.contains(e.target as Node)) onClose(); };
    const esc = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('pointerdown', away, true);
    window.addEventListener('keydown', esc);
    return () => { window.removeEventListener('pointerdown', away, true); window.removeEventListener('keydown', esc); };
  }, [onClose]);
  useEffect(() => { if (ghosts.length === 0) onClose(); }, [ghosts.length, onClose]);
  if (ghosts.length === 0) return null;

  const show = (g: GhostBranch) => { showBranch(tabId, g.branch); onClose(); };
  const showAll = () => { showBranches(tabId, ghosts.map((g) => g.branch)); onClose(); };
  return createPortal(
    <div
      ref={ref}
      className="ghost-pop"
      role="dialog"
      aria-label={`${ghosts.length} hidden ${ghosts.length === 1 ? 'branch starts' : 'branches start'} here`}
      style={{ top: pos?.top ?? at.top, left: pos?.left ?? at.right + 8, visibility: pos ? 'visible' : 'hidden' }}
      onPointerEnter={onEnter}
      onPointerLeave={onLeave}
    >
      <div className="ghost-pop-title">{ghosts.length} hidden {ghosts.length === 1 ? 'branch starts' : 'branches start'} here</div>
      {ghosts.map((g) => (
        <div key={g.branch} className="ghost-pop-row">
          <span className="ghost-pop-chip" style={{ ['--lane-color' as string]: lanes[g.color % lanes.length] }} title={g.branch}>{g.branch}</span>
          <span className="ghost-pop-what">
            <span className="ghost-pop-summary" title={g.summary}>{g.summary}</span>
            <span className="ghost-pop-meta">{g.authorName} · {relativeTime(g.authorTime)}</span>
          </span>
          <button type="button" className="ghost-pop-btn" aria-label={`Show ${g.branch}`} onClick={() => show(g)}>Show</button>
        </div>
      ))}
      <div className="ghost-pop-foot">
        <span className="ghost-pop-by">Hidden by “{current.preset.name}”</span>
        <button type="button" className="ghost-pop-btn" onClick={showAll}>Show all</button>
      </div>
    </div>,
    document.body,
  );
}

// Shown in the Keyboard Shortcuts panel (Ctrl+/); metadata only.
registerKeyHints([
  { id: 'key.ghost.open', section: 'Navigation', label: 'List the hidden branches that start at a commit', keys: ['Enter'], context: 'on a commit with ghost lines', source: 'presets/GhostPopover.tsx' },
  { id: 'key.ghost.close', section: 'Navigation', label: 'Close the hidden branches list', keys: ['Esc'], context: 'when it is open', source: 'presets/GhostPopover.tsx' },
]);
