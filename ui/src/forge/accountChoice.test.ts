import { describe, expect, it } from 'vitest';
import type { ForgeAccountView } from '../api/gen/ForgeAccountView';
import type { RepoProjects } from '../api/gen/RepoProjects';
import { EMPTY_REPO_SETTINGS } from '../app/state';
import { byHost, hostDefault, removalEffects, repoAccountsOf } from './accountChoice';

const view = (id: string, host: string, username: string): ForgeAccountView => ({
  account: { id, host, kind: 'gitlab', user: { id: 1, username, name: username, avatarUrl: null, webUrl: '', email: null, emails: [] }, storage: 'keyring', version: null, versionCheckedAt: 0, addedAt: 0 },
  status: { kind: 'ok' },
});
const H = 'gitlab.example.com';
const ada = view(H, H, 'ada');
const octo = view('github.com', 'github.com', 'octocat');
const grace = view(`${H}#8`, H, 'grace');
const ported = view('gitlab.example.com:8443', 'gitlab.example.com:8443', 'linus');
const all = [ada, octo, grace, ported];

describe('forge account choice', () => {
  it("a host's default is its first account; rows group by host in the order hosts were added", () => {
    expect(hostDefault(all, H)).toBe(ada);
    expect(hostDefault(all, 'code.example.org')).toBeUndefined();
    expect(byHost(all)).toEqual([ada, grace, octo, ported]);
  });

  it("a repository's accounts are those on the hosts its remotes map to, port or not", () => {
    const projects = (accountIds: Array<string | null>): RepoProjects => ({
      remotes: accountIds.map((accountId, i) => ({ remote: `r${i}`, host: H, path: 'g/p', account: accountId ? 'gitlab' : null, accountId, project: null, error: null })),
      target: null, targetChosen: false,
    });
    expect(repoAccountsOf(all, projects([`${H}#8`, null]))).toEqual({ hosts: [H], accounts: [ada, grace] });
    // An SSH remote (no port) the core mapped to the ported host's account.
    expect(repoAccountsOf(all, projects(['gitlab.example.com:8443']))).toEqual({ hosts: ['gitlab.example.com:8443'], accounts: [ported] });
    expect(repoAccountsOf(all, projects([null]))).toEqual({ hosts: [], accounts: [] });
  });

  it("removing an account says when Automatic moves on and how many repositories chose it", () => {
    const repos = { '/a': { ...EMPTY_REPO_SETTINGS, forgeAccount: H }, '/b': { ...EMPTY_REPO_SETTINGS, forgeAccount: H }, '/c': { ...EMPTY_REPO_SETTINGS, forgeAccount: `${H}#8` } };
    expect(removalEffects(all, ada.account, repos)).toBe('Automatic then uses @grace; 2 repositories that chose it go back to Automatic.');
    expect(removalEffects(all, grace.account, repos)).toBe('The repository that chose it goes back to Automatic.');
    expect(removalEffects(all, grace.account, {})).toBe('');
    expect(removalEffects(all, octo.account, repos), 'the host has no other account: nothing else to say').toBe('');
  });
});
