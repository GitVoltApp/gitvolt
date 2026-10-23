import { RefreshCw } from 'lucide-react';
import { HoverTooltip } from '../ui/HoverTooltip';
import './mr.css';
import { useTabForgeField } from './mrStore';
import { refreshListNow, refreshMrNow } from './poll';

/** A refresh button: asks the forge now, whatever the poll's timing and the client's kept answers
 * (`refreshMrNow`, `refreshListNow`). The icon turns, in place, while it runs (a transform alone,
 * and only then); another click meanwhile joins the refresh under way. `number`: that MR/PR;
 * none: the sidebar's list. `className` is the host's own button style. */
export function RefreshButton({ tabId, number, label, className }: { tabId: string; number?: number; label: string; className: string }) {
  const busy = useTabForgeField(tabId, 'refreshing').includes(number ?? 'list');
  return (
    <HoverTooltip content={label}>
      <button type="button" className={`${className}${busy ? ' is-refreshing' : ''}`} aria-label={label} aria-busy={busy} onClick={(e) => { e.stopPropagation(); void (number === undefined ? refreshListNow(tabId) : refreshMrNow(tabId, number)); }}>
        <RefreshCw size={number === undefined ? 12 : 14} aria-hidden />
      </button>
    </HoverTooltip>
  );
}
