import { KeyRound } from 'lucide-react';
import { AccountsSection } from '../../forge/AccountsSection';
import { registerSettings, registerSettingsPage } from '../registry';
import { useSettingsUi } from '../schema';

function ManageAccounts() {
  return <button type="button" id="input-forgeAccounts" onClick={() => useSettingsUi.getState().setPage('accounts')}>Manage accounts</button>;
}

const offs = [
  registerSettingsPage({ id: 'accounts', label: 'Accounts', scope: 'profile', order: 30, icon: KeyRound, summary: 'Forge accounts for merge requests, pull requests, forks and avatars.', Body: () => <AccountsSection /> }),
  registerSettings([
    {
      id: 'forgeAccounts', label: 'Forge accounts', page: 'accounts', group: 'Accounts', labelFor: 'group',
      keywords: 'gitlab github forgejo gitea codeberg token personal access token pat account login keyring merge request pull request forks',
      description: 'GitLab, GitHub and Forgejo accounts for merge requests, pull requests, forks and avatars.',
      Control: ManageAccounts,
    },
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
