import { RefreshCw } from 'lucide-react';
import { useAbout } from '../app/About';
import { registerActions } from '../app/actions';
import { registerAppSlot } from '../app/slots';
import { UpdateDialog } from './UpdateDialog';
import { checkAndShow } from './store';

/** Help → Check for updates (About shows the answer; an update it finds opens the update dialog
 *  over it), and the update dialog. */
const offs = [
  registerAppSlot('overlay', 'updates.dialog', UpdateDialog),
  registerActions([
    {
      id: 'help.checkUpdates', label: 'Check for updates', group: 'Help', icon: RefreshCw, tooltip: "Ask GitHub for a newer GitVolt release (GitVolt's own; nothing about your repositories)",
      run: () => {
        useAbout.getState().setOpen(true);
        void checkAndShow(() => useAbout.getState().setOpen(false));
      },
    },
  ]),
];
import.meta.hot?.dispose(() => { for (const off of offs) off(); });
