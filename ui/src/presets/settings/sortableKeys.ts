import type { KeyboardEvent as ReactKeyboardEvent, RefObject } from 'react';
import { useKeys } from '../../ui/keyRouter';
import type { useSortable } from '../../ui/sortable/useSortable';

/** Alt+↑/↓ on an item of `list`: Settings' dialog takes every key before React's handlers see it,
 * so the sortable's own key handler is called from the key router. */
export function useSortableKeys(list: RefObject<HTMLElement | null>, sortable: ReturnType<typeof useSortable>): void {
  useKeys('menu', (e) => {
    if (!e.altKey || e.ctrlKey || e.metaKey || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
    const item = e.target instanceof Element ? e.target.closest<HTMLElement>('[data-sortable-item]') : null;
    if (!item || item.parentElement !== list.current) return;
    sortable.itemProps(Number(item.dataset.sortableItem)).onKeyDown?.(e as unknown as ReactKeyboardEvent<HTMLElement>);
    e.preventDefault();
    return 'handled';
  });
}
