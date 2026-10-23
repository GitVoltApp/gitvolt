import { Settings } from 'lucide-react';
import { DEFAULT_FILE_LIST_PREFS, useFileListPrefs } from '../../files/fileListPrefs';
import { LegacyBundleRow } from '../../updates/LegacyBundle';
import { appSetting, storeBinding } from '../bindings';
import { switchControl } from '../frame/controls';
import { PageGroups } from '../frame/SettingsPage';
import { registerSettings, registerSettingsPage, type SettingsCtx } from '../registry';

function GeneralBody({ ctx }: { ctx: SettingsCtx }) {
  return <PageGroups pageId="general" ctx={ctx} render={(group, rows) => (group === 'Updates' ? <>{rows}<LegacyBundleRow /></> : rows)} />;
}

const offs = [
  registerSettingsPage({ id: 'general', label: 'General', scope: 'app', order: 10, icon: Settings, summary: 'How GitVolt behaves day to day.', Body: GeneralBody }),
  registerSettings([
    {
      id: 'advanceAfterStage', label: 'After staging a file, show the next one', page: 'general', group: 'Staging',
      keywords: 'stage unstage discard next file advance move diff',
      description: 'Staging, unstaging or discarding the open file opens the next one in its list. Off keeps showing it.',
      Control: switchControl('advanceAfterStage'),
      binding: storeBinding(useFileListPrefs, () => useFileListPrefs.getState().advanceAfterStage, (v) => useFileListPrefs.getState().set({ advanceAfterStage: v }), DEFAULT_FILE_LIST_PREFS.advanceAfterStage),
    },
    {
      id: 'updateCheck', label: 'Check for updates automatically', page: 'general', group: 'Updates',
      keywords: 'update upgrade new version release github check daily',
      description: 'Asks GitHub for new releases at startup and once a day; nothing about you is sent. Off: only Help › Check for updates asks.',
      Control: switchControl('updateCheck'), binding: appSetting('updateCheck', true),
    },
    {
      id: 'updatePrereleases', label: 'Include pre-releases', page: 'general', group: 'Updates',
      keywords: 'update beta rc alpha pre-release preview',
      description: 'Also offers release candidates and betas.',
      Control: switchControl('updatePrereleases'), binding: appSetting('updatePrereleases', false),
    },
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
