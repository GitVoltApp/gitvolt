import { useAppState } from '../app/state';
import { fullDate } from '../forge/mrview/RelTime';
import { HoverTooltip } from '../ui/HoverTooltip';
import { hideTooltip, showTooltip } from '../ui/tooltipStore';
import { formatDate } from './date';
import { relativeTime } from './relative';
import { useRelativeNow } from './relativeClock';

/**
 * A date in the graph, details, compare and find: the exact one in the date format, or ("Show
 * relative dates") "5 minutes ago", the other form on hover. `imperativeTip`: for the graph's
 * virtualized rows, which show it through the shared tooltip instead of mounting a tooltip each.
 */
export function DateText({ at, prefix, className, testId, imperativeTip = false }: { at: number; prefix?: string; className?: string; testId?: string; imperativeTip?: boolean }) {
  const relative = useAppState((s) => s.settings.relativeDates);
  const fmt = useAppState((s) => s.settings.dateFormat);
  const now = useRelativeNow(at, relative);
  const shown = relative ? relativeTime(at, now) : formatDate(at, fmt);
  const other = () => (relative ? fullDate(at, fmt) : relativeTime(at));
  const text = prefix ? `${prefix} ${shown}` : shown;
  if (imperativeTip) {
    return <span className={className} data-testid={testId} onMouseEnter={(e) => showTooltip(e.currentTarget, other())} onMouseLeave={hideTooltip}>{text}</span>;
  }
  return (
    <HoverTooltip content={other}>
      <span className={className} data-testid={testId}>{text}</span>
    </HoverTooltip>
  );
}
