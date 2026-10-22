import { Server } from 'lucide-react';
import { useMemo, useState } from 'react';
import type { HostKind } from '../../api/gen/HostKind';
import { useRuntime } from '../../app/runtime';
import { useAppState } from '../../app/state';
import { Select } from '../../ui/Select';
import { registerSettings, registerSettingsPage, type SettingsCtx } from '../registry';

const HOST_KINDS: Array<[HostKind, string]> = [['gitlab', 'GitLab'], ['github', 'GitHub'], ['generic', 'Generic (no forge links)']];
const KIND_NAMES: Record<HostKind, string> = { gitlab: 'GitLab', github: 'GitHub', generic: 'Generic' };
const NONE: never[] = [];

/** The open repository's remote hosts and the hosts with an override, each with its forge type. */
function HostRows({ ctx }: { ctx: SettingsCtx }) {
  const remotes = useRuntime((s) => (ctx.tabId ? s.tabs[ctx.tabId]?.info?.remotes : undefined)) ?? NONE;
  const detected = useMemo<Array<[string, HostKind]>>(() => remotes.flatMap((r) => (r.host ? [[r.host, r.hostKind] as [string, HostKind]] : [])), [remotes]);
  const overrides = useAppState((s) => s.profile.hostOverrides);
  const updateProfile = useAppState((s) => s.updateProfile);
  const [added, setAdded] = useState('');
  const [addedKind, setAddedKind] = useState<HostKind>('gitlab');
  const hosts = useMemo(() => {
    const known = new Map(detected);
    for (const h of Object.keys(overrides)) if (!known.has(h)) known.set(h, 'generic');
    return [...known.entries()];
  }, [detected, overrides]);
  const set = (host: string, kind: HostKind | '') => updateProfile((p) => {
    const hostOverrides = { ...p.hostOverrides };
    if (kind) hostOverrides[host] = kind;
    else delete hostOverrides[host];
    return { ...p, hostOverrides };
  });
  const host = added.trim().toLowerCase().replace(/^[a-z]+:\/\//, '').replace(/\/.*$/, '');
  return (
    <div className="hosts-box">
      {hosts.length === 0 && <span className="dim">No remotes in the open repository. Add a host to set its type for every repository.</span>}
      {hosts.length > 0 && (
        <table className="hosts">
          <tbody>
            {hosts.map(([h, kind]) => (
              <tr key={h}>
                <td className="host-name">{h}</td>
                <td>
                  <Select<HostKind | ''> aria-label={`Forge type for ${h}`} value={overrides[h] ?? ''} onChange={(k) => set(h, k)} options={[['', `Detected (${KIND_NAMES[kind]})`], ...HOST_KINDS]} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <form className="host-add" onSubmit={(e) => { e.preventDefault(); if (host) { set(host, addedKind); setAdded(''); } }}>
        <input aria-label="Add a host" placeholder="git.example.com" spellCheck={false} value={added} onChange={(e) => setAdded(e.target.value)} />
        <Select<HostKind> aria-label="Forge type for the added host" value={addedKind} onChange={setAddedKind} options={HOST_KINDS} />
        <button type="submit" disabled={!host}>Add</button>
      </form>
    </div>
  );
}

const offs = [
  registerSettingsPage({ id: 'hosts', label: 'Hosts', scope: 'profile', order: 40, icon: Server, summary: 'Which forge a self-hosted host runs.' }),
  registerSettings([
    {
      id: 'hostOverrides', label: 'Forge type per host', page: 'hosts', group: 'Hosts', labelFor: 'group',
      keywords: 'gitlab github self-hosted icon links',
      description: 'For the remote icons and web links of a self-hosted GitLab or GitHub.',
      Control: HostRows,
    },
  ]),
];
import.meta.hot?.dispose(() => offs.forEach((off) => off()));
