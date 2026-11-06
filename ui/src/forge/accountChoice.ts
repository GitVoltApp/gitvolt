import type { ForgeAccount } from '../api/gen/ForgeAccount';
import type { ForgeAccountView } from '../api/gen/ForgeAccountView';
import type { RepoProjects } from '../api/gen/RepoProjects';
import type { RepoSettings } from '../api/gen/RepoSettings';

/** A host's default account: its first (hub.rs `account_for`), what Automatic uses. */
export function hostDefault<T extends { account: ForgeAccount }>(accounts: readonly T[], host: string): T | undefined {
  return accounts.find((a) => a.account.host === host);
}

export const isHostDefault = (accounts: readonly ForgeAccountView[], a: ForgeAccount): boolean => hostDefault(accounts, a.host)?.account.id === a.id;

/** Settings › Accounts' order: by host (each host's accounts together, as added), hosts in the
 * order their first account was added. */
export function byHost(accounts: readonly ForgeAccountView[]): ForgeAccountView[] {
  const hosts = [...new Set(accounts.map((a) => a.account.host))];
  return hosts.flatMap((h) => accounts.filter((a) => a.account.host === h));
}

/** The accounts a repository can use: those on the hosts its remotes map to (each remote's
 * `accountId` names an account on the host the core matched, port or not). */
export interface RepoAccounts { hosts: string[]; accounts: ForgeAccountView[] }
export function repoAccountsOf(all: readonly ForgeAccountView[], projects: RepoProjects): RepoAccounts {
  const hosts: string[] = [];
  for (const r of projects.remotes) {
    const host = all.find((a) => a.account.id === r.accountId)?.account.host;
    if (host !== undefined && !hosts.includes(host)) hosts.push(host);
  }
  return { hosts, accounts: all.filter((a) => hosts.includes(a.account.host)) };
}

/** What removing `a` changes beyond itself, for its armed Remove: the host's default moving to
 * the next account (Automatic's repositories), and the repositories that chose it going back to
 * Automatic. Empty when nothing else changes. */
export function removalEffects(all: readonly ForgeAccountView[], a: ForgeAccount, repos: Readonly<Record<string, RepoSettings>>): string {
  const others = all.filter((v) => v.account.host === a.host && v.account.id !== a.id);
  if (others.length === 0) return '';
  const parts: string[] = [];
  if (isHostDefault(all, a)) parts.push(`Automatic then uses @${others[0].account.user.username}`);
  const chose = Object.values(repos).filter((r) => r.forgeAccount === a.id).length;
  if (chose > 0) parts.push(chose === 1 ? 'the repository that chose it goes back to Automatic' : `${chose} repositories that chose it go back to Automatic`);
  const text = parts.join('; ');
  return text && `${text[0].toUpperCase()}${text.slice(1)}.`;
}
