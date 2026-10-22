import { GitBranch } from 'lucide-react';
import type { DateFormat } from '../../api/gen/DateFormat';
import { useRuntime } from '../../app/runtime';
import { flushSaves, useAppState } from '../../app/state';
import { resetAvatars } from '../../avatars/avatarStore';
import { appSetting, useSettingValue } from '../bindings';
import { selectControl, switchControl } from '../frame/controls';
import { NumberField } from '../frame/fields';
import { registerSettings, registerSettingsPage, settingDef, type SettingsCtx } from '../registry';

const COMMIT_LIMIT_MAX = 50_000;
const DATE_CHOICES: Array<[DateFormat, string]> = [['ymd12h', '2026-09-26 @ 3:14 PM'], ['ymd24h', '2026-09-26 15:14'], ['dmy24h', '26/09/2026 15:14'], ['mdy12h', '09/26/2026 3:14 PM']];

/** The shown tab reloads its graph with the new limit. */
function refreshActiveTab() {
  const tab = useAppState.getState().profile.activeTab;
  if (tab && useRuntime.getState().tabs[tab]?.repo) void useRuntime.getState().refresh(tab);
}
// The backend learns the setting with the save; what it answers changes after that.
const askAvatarsAgain = () => void flushSaves().then(() => resetAvatars());

function CommitLimitControl({ ctx }: { ctx: SettingsCtx }) {
  const [limit, set] = useSettingValue<number>(settingDef('commitLimit')!, ctx);
  return <NumberField id="commitLimit" value={limit} min={1} max={COMMIT_LIMIT_MAX} step={100} onCommit={set} />;
}

const offs = [
  registerSettingsPage({ id: 'graph', label: 'Graph', scope: 'app', order: 30, icon: GitBranch, summary: 'How the commit graph loads and draws. Branch visibility is in Branch presets.' }),
  registerSettings([
    {
      id: 'commitLimit', label: 'Commits loaded in the graph', page: 'graph', group: 'History',
      keywords: 'window history limit',
      description: 'Newest first. More is slower on a big history.',
      Control: CommitLimitControl, binding: appSetting('commitLimit', 2000, refreshActiveTab),
    },
    {
      id: 'relativeDates', label: 'Show relative dates', page: 'graph', group: 'Dates',
      keywords: 'relative ago yesterday humanize',
      description: '"5 minutes ago", "yesterday", "3 weeks ago". The exact format below shows on hover.',
      Control: switchControl('relativeDates'), binding: appSetting('relativeDates', false),
    },
    {
      id: 'dateFormat', label: 'Date format', page: 'graph', group: 'Dates', labelFor: 'group',
      keywords: 'time 24h 12h graph column',
      description: 'The exact date: in the graph, details, compare and find, or on hover under relative dates.',
      Control: selectControl('dateFormat', DATE_CHOICES), binding: appSetting('dateFormat', 'ymd12h'),
    },
    {
      id: 'gravatar', label: 'From Gravatar', page: 'graph', group: 'Avatars',
      keywords: 'avatar privacy images',
      description: "Looked up by a hash of the author's email, never the email itself. Off shows initials and makes no request.",
      Control: switchControl('gravatar'), binding: appSetting('gravatar', true, askAvatarsAgain),
    },
    {
      id: 'forgeAvatars', label: 'From your forge accounts', page: 'graph', group: 'Avatars',
      keywords: 'avatar gitlab github forge privacy images',
      description: "Asks the forge hosting the repository, when you have an account there, so colleagues without Gravatar get their avatar. Sends authors' emails (or names) to that forge.",
      Control: switchControl('forgeAvatars'), binding: appSetting('forgeAvatars', true, askAvatarsAgain),
    },
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
