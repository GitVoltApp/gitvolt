import { Settings } from 'lucide-react';
import { registerActions } from '../app/actions';
import { registerAppSlot } from '../app/slots';
import { SettingsOverlay } from './frame/SettingsOverlay';
import { useSettingsUi } from './schema';
import './pages';

/**
 * Settings' registrations (ruling R10: its own module, imported from `app/features.ts`): the
 * `file.settings` action (Ctrl+, and the hamburger's File menu), the overlay in the overlay slot,
 * and its pages (`./pages`). The palette's `#` group opens it on one setting with
 * `useSettingsUi.show(id)`.
 */
const offActions = registerActions([
  {
    id: 'file.settings', label: 'Settings', group: 'File', icon: Settings, tooltip: 'App, profile and repository settings', shortcuts: ['Mod+,'],
    run: () => useSettingsUi.getState().show(),
  },
]);
const offSlot = registerAppSlot('overlay', 'settings.dialog', SettingsOverlay);
// A dev-server hot update re-runs this module: release the old registrations first.
import.meta.hot?.dispose(() => {
  offActions();
  offSlot();
});
