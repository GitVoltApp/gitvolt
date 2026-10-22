import { UserRound } from 'lucide-react';
import { examplePath } from '../../app/osPath';
import { useAppState } from '../../app/state';
import { profileField } from '../bindings';
import { TextField } from '../frame/fields';
import { registerSettings, registerSettingsPage } from '../registry';

function ExtraGitconfigControl() {
  const path = useAppState((s) => s.profile.extraGitconfig);
  return <TextField id="extraGitconfig" value={path ?? ''} placeholder={examplePath('.gitconfig-work')} onCommit={(v) => useAppState.getState().updateProfile((p) => ({ ...p, extraGitconfig: v || null }))} />;
}

const offs = [
  registerSettingsPage({ id: 'git-identity', label: 'Git identity', scope: 'profile', order: 10, icon: UserRound, summary: "Who your commits say they're from in this profile." }),
  registerSettings([
    {
      id: 'extraGitconfig', label: 'Extra git config for this profile', page: 'git-identity', group: 'Config',
      keywords: 'include.path identity account signing',
      description: 'A gitconfig file added to every git command of this profile (as include.path): your identity, signing key.',
      Control: ExtraGitconfigControl, binding: profileField('extraGitconfig', null),
    },
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
