import { useLayoutEffect, useRef, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { ColourSwatch } from '../../ui/colour/ColourSwatch';
import { useKeys } from '../../ui/keyRouter';
import { useSortable } from '../../ui/sortable/useSortable';
import { moveLane, setLaneColour } from '../edit';
import { useTheme } from '../store';
import { useEditedTheme } from './actions';
import './editor.css';

/**
 * The edited theme's ten graph lane colours: drag one (or Alt+←/→ on it) to reorder, click (or
 * Enter) to change it in the picker, which previews on the graph as it moves. A change to a
 * built-in theme makes its custom copy (edit.ts).
 */
export function LaneColours({ compact = false }: { compact?: boolean }) {
  const { def } = useEditedTheme();
  const lanes = def.graph;
  const list = useRef<HTMLDivElement>(null);
  const sortable = useSortable({ count: lanes.length, axis: 'x', onMove: moveLane, label: (i) => `Lane ${i + 1}` });
  const focusAt = useRef<number | null>(null);

  // Alt+←/→ through the key router: in Settings the dialog's claim stops the key before the
  // item's own `onKeyDown`, so it's handed to it from here (the claim keeps it from running twice).
  useKeys('menu', (e) => {
    if (!e.altKey || e.ctrlKey || e.metaKey || (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight')) return;
    const item = e.target instanceof Element ? e.target.closest<HTMLElement>('[data-sortable-item]') : null;
    if (!item || item.parentElement !== list.current) return;
    const from = Number(item.dataset.sortableItem);
    const to = from + (e.key === 'ArrowRight' ? 1 : -1);
    if (to >= 0 && to < lanes.length) focusAt.current = to;
    sortable.itemProps(from).onKeyDown?.(e as unknown as ReactKeyboardEvent<HTMLElement>);
    e.preventDefault();
    return 'handled';
  });
  // The moved lane keeps the focus: its swatch, at its new place.
  useLayoutEffect(() => {
    const to = focusAt.current;
    if (to === null) return;
    focusAt.current = null;
    list.current?.querySelector<HTMLElement>(`:scope > [data-sortable-item="${to}"] .colour-swatch`)?.focus();
  });

  const preview = (graph: readonly string[] | null) => useTheme.getState().preview(graph && { graph });
  return (
    <div ref={list} className={compact ? 'theme-lanes compact' : 'theme-lanes'} role="list" aria-label="Graph lanes" {...sortable.containerProps}>
      {lanes.map((c, i) => (
        <div key={i} role="listitem" className="theme-lane" {...sortable.itemProps(i)}>
          {/* The swatch is the lane's handle too: a press that moves drags, one that doesn't clicks. */}
          <span className="theme-lane-swatch" {...sortable.handleProps(i)}>
            <ColourSwatch
              value={c}
              label={`Lane ${i + 1} colour`}
              size={compact ? 18 : 24}
              onPreview={(v) => preview(lanes.map((g, j) => (j === i ? v : g)))}
              onCommit={(v) => {
                preview(null);
                if (v !== c) setLaneColour(i, v);
              }}
              onCancel={() => preview(null)}
            />
          </span>
          {!compact && <span className="theme-lane-hex">{c}</span>}
        </div>
      ))}
      <span className="theme-sr" aria-live="polite">{sortable.liveText}</span>
    </div>
  );
}
