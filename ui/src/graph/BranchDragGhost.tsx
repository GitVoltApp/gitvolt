import { createPortal } from 'react-dom';
import type { ReactNode } from 'react';
import type { GraphPayload } from '../api/gen/GraphPayload';
import { useTheme } from '../theme/store';
import { labelOf, useBranchDrag } from './branchDrag';

/** The branch being dragged, beside the pointer, in its chip's lane colour. It ignores the
 * pointer: the chip under it is the drop target. */
export function BranchDragGhost({ graph }: { graph: GraphPayload }): ReactNode {
  const drag = useBranchDrag((s) => s.drag);
  const lanes = useTheme((s) => s.colors.graph);
  if (!drag) return null;
  const from = labelOf(graph, drag.from);
  const color = from ? lanes[from.row.color % lanes.length] : undefined;
  return createPortal(
    <span className="ref-label branch-drag-ghost" style={{ ['--lane-color' as string]: color, left: drag.x + 10, top: drag.y + 6 }} aria-hidden="true">
      {from?.label.name ?? drag.from.replace(/^refs\/heads\//, '')}
    </span>,
    document.body,
  );
}
