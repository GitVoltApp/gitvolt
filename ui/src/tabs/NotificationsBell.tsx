import { Bell, BellOff, History, Sparkles, TriangleAlert } from 'lucide-react';
import { useCustomTitleBar } from '../chrome/windowChrome';
import { useDragOrClick } from '../chrome/dragRegion';
import { copyText } from '../api/transport';
import { openActivityLog } from '../app/activityLog';
import { useOps } from '../app/ops';
import { openMenuAt } from '../menu/menuStore';
import type { MenuRow } from '../menu/types';
import { HoverTooltip } from '../ui/HoverTooltip';
import { useToast } from '../ui/toastStore';
import { useWhatsNew } from '../updates/whatsNew';

const BELL_ROWS = 20;
const BELL_LABEL = 80;

/** The bell's menu: "Updated to GitVolt …" after an update (it opens What's new), then the
 * background error history, newest first (spec §16.1). An error row copies its message; its
 * tooltip has the time and the whole message. */
function bellRows(): MenuRow[] {
  const { errors } = useOps.getState();
  const { notice } = useWhatsNew.getState();
  // K96: the activity log (every finished fetch and clone, with git's message) is one step away.
  const activity: MenuRow = { kind: 'action', id: 'bell.activity', label: 'Activity log…', icon: History, tooltip: 'Every finished fetch and clone, background ones included', run: openActivityLog };
  const updated: MenuRow[] = notice ? [{
    kind: 'action', id: 'bell.whatsNew', label: `Updated to GitVolt ${notice.version} — see what's new`, icon: Sparkles,
    tooltip: `The changes since ${notice.since}, from the notes that come with GitVolt`, run: () => useWhatsNew.getState().show(notice),
  }] : [];
  const clear: MenuRow = { kind: 'action', id: 'bell.clear', label: 'Clear notifications', icon: BellOff, tooltip: 'Forget these notifications', run: () => { useOps.getState().clearErrors(); useWhatsNew.getState().clearNotice(); } };
  if (errors.length === 0 && notice) return [...updated, { kind: 'separator' }, activity, clear];
  if (errors.length === 0) return [{ kind: 'action', id: 'bell.none', label: 'No notifications', icon: BellOff, tooltip: 'Background errors (a failed background fetch) appear here', run: () => {}, disabledReason: 'Nothing to show' }, { kind: 'separator' }, activity];
  const rows: MenuRow[] = errors.slice(0, BELL_ROWS).map((e, i) => ({
    kind: 'action', id: `bell.${i}`, label: e.message.length > BELL_LABEL ? `${e.message.slice(0, BELL_LABEL - 1)}…` : e.message, icon: TriangleAlert,
    tooltip: `${new Date(e.at).toLocaleString()}\n${e.message}\n\nClick to copy`,
    run: () => { void copyText(e.message).then(() => useToast.getState().show('Copied'), () => useToast.getState().show('Copy failed')); },
  }));
  return [...updated, ...rows, { kind: 'separator' }, activity, clear];
}

/** The notification bell, in the tab bar left of the settings gear: an unread badge, and the menu
 * of background errors (and an update's What's new) with the activity log. */
export function NotificationsBell() {
  const unread = useOps((s) => s.unread) + (useWhatsNew((s) => s.unread) ? 1 : 0);
  const drag = useDragOrClick(useCustomTitleBar());
  return (
    <HoverTooltip content="Notifications: background errors and updates">
      <button
        type="button"
        className="tab-bar-btn bell-btn"
        aria-label={`Notifications${unread ? ` (${unread} new)` : ''}`}
        aria-haspopup="menu"
        onMouseDown={drag.onMouseDown}
        onClick={(e) => {
          if (drag.guardClick(e)) return;
          if (openMenuAt(e.currentTarget, bellRows(), undefined, bellRows, 'Notifications')) { useOps.getState().markRead(); useWhatsNew.getState().markRead(); }
        }}
      >
        <Bell size={16} aria-hidden />{unread > 0 && <span className="bell-badge">{unread}</span>}
      </button>
    </HoverTooltip>
  );
}
