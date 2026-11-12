import { Sparkles } from 'lucide-react';
import { useAppInfo } from '../app/appInfo';
import { registerActions } from '../app/actions';
import { registerAppSlot } from '../app/slots';
import { WhatsNewDialog } from './WhatsNewDialog';
import { useWhatsNew } from './whatsNew';

/** Help → What's new (this version's notes, from the changelog bundled with the app; every version
 * since the last one when this session updated), and its panel. */
const offs = [
  registerAppSlot('overlay', 'updates.whatsNew', WhatsNewDialog),
  registerActions([
    {
      id: 'help.whatsNew', label: "What's new", group: 'Help', icon: Sparkles, tooltip: "This version's changes, from the notes that come with GitVolt",
      run: async () => {
        await useAppInfo.getState().load();
        const version = useAppInfo.getState().info?.appVersion;
        if (version) useWhatsNew.getState().show({ version, since: useWhatsNew.getState().updatedFrom });
      },
    },
  ]),
];
import.meta.hot?.dispose(() => { for (const off of offs) off(); });
