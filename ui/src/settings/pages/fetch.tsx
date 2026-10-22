import { RefreshCw } from 'lucide-react';
import { appSetting } from '../bindings';
import { useAppState } from '../../app/state';
import { Select } from '../../ui/Select';
import { switchControl } from '../frame/controls';
import { registerSettings, registerSettingsPage } from '../registry';
import { clampFetchInterval } from '../schema';

const FETCH_CHOICES: Array<[number, string]> = [[0, 'Off'], [60, 'Every minute'], [300, 'Every 5 minutes'], [600, 'Every 10 minutes'], [1800, 'Every 30 minutes']];

/** A hand-edited interval shows as the scheduler runs it (clamped), with a choice of its own if it's none of these. */
function FetchIntervalControl() {
  const secs = clampFetchInterval(useAppState((s) => s.settings.fetchIntervalSecs));
  const options = FETCH_CHOICES.some(([v]) => v === secs) ? FETCH_CHOICES : [...FETCH_CHOICES, [secs, `Every ${Math.round(secs / 60)} min`] as [number, string]];
  return <Select<number> id="input-fetchInterval" aria-labelledby="label-fetchInterval" value={secs} onChange={(v) => useAppState.getState().setSettings({ fetchIntervalSecs: v })} options={options} />;
}

const offs = [
  registerSettingsPage({ id: 'fetch', label: 'Fetch and push', scope: 'app', order: 60, icon: RefreshCw, summary: 'Background fetch and what a push sends.' }),
  registerSettings([
    {
      id: 'fetchInterval', label: 'Background fetch interval', page: 'fetch', group: 'Fetch', labelFor: 'group',
      keywords: 'auto fetch timer minutes off',
      description: 'Fetches the shown repository on this schedule.',
      Control: FetchIntervalControl,
      // A hand-edited interval counts as what the scheduler runs (clamped).
      binding: appSetting('fetchIntervalSecs', 60, undefined, (a, b) => clampFetchInterval(a) === clampFetchInterval(b)),
    },
    {
      id: 'prune', label: 'Prune deleted remote branches on fetch', page: 'fetch', group: 'Fetch',
      keywords: 'prune --prune remote tracking',
      description: 'Fetch with --prune: remote-tracking branches the remote deleted are removed.',
      Control: switchControl('prune'), binding: appSetting('prune', true),
    },
    {
      id: 'pushFollowTags', label: 'Push tags with branches', page: 'fetch', group: 'Push',
      keywords: 'push follow-tags annotated tags release',
      description: "Push with --follow-tags: a branch's annotated tags the remote lacks go with it.",
      Control: switchControl('pushFollowTags'), binding: appSetting('pushFollowTags', true),
    },
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
