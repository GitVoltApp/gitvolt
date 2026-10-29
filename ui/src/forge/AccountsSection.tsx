import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api, errorMessage } from '../api/client';
import type { ForgeAccountView } from '../api/gen/ForgeAccountView';
import type { ForgeKind } from '../api/gen/ForgeKind';
import type { HostKind } from '../api/gen/HostKind';
import { useRuntime } from '../app/runtime';
import { useAppState } from '../app/state';
import { resetAvatars } from '../avatars/avatarStore';
import { notifyForgeAccountsChanged } from './accountsBus';
import { escOwners } from '../app/modalKeys';
import { ForgeMark } from '../icons/brands';
import { HostCombobox, type HostSuggestion } from './HostCombobox';
import { confirmAction } from '../ui/ConfirmDialog';
import { bareHost, detectHostKind } from './hostKind';
import { forgeName } from './labels';
import { effectiveKind } from './urls';
import './accounts.css';

export const FILE_WARNING = 'Stored in a file, not the system keyring: install or start a Secret Service (GNOME Keyring, KWallet), then restart GitVolt to secure it.';
const KINDS: Array<[ForgeKind, string]> = [['gitlab', 'GitLab'], ['github', 'GitHub'], ['forgejo', 'Forgejo']];

/** What an account's last requests said, in plain words; `null` when all is well. */
export function statusText(v: ForgeAccountView): string | null {
  const s = v.status;
  switch (s.kind) {
    case 'ok': return null;
    case 'rateLimited': return `Rate limited until ${new Date(s.until * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`;
    case 'authFailed':
    case 'unreachable': return s.message;
    case 'tokenMissing': return 'Token missing: replace it to reconnect';
  }
}

/** The status chip: its tone and words. */
export function statusChip(v: ForgeAccountView): { tone: 'ok' | 'warn' | 'err'; label: string } {
  const s = v.status;
  switch (s.kind) {
    case 'ok': return { tone: 'ok', label: 'Connected' };
    case 'rateLimited': return { tone: 'warn', label: `Rate limited until ${new Date(s.until * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}` };
    case 'authFailed': return { tone: 'err', label: 'Token rejected' };
    case 'unreachable': return { tone: 'err', label: 'Unreachable' };
    case 'tokenMissing': return { tone: 'err', label: 'Token missing' };
  }
}

type Panel = { mode: 'add' } | { mode: 'replace'; host: string; kind: ForgeKind };
const TOKEN_HELP: Record<ForgeKind, string> = {
  gitlab: 'Opens your profile\'s token page with the api scope selected. Copy the token, then paste it here.',
  github: 'Opens GitHub\'s fine-grained token page with the permissions GitVolt needs selected. Some organizations cap these tokens\' lifetime (e.g. 90 days); use a classic token for those.',
  forgejo: 'Opens the new-token form in your Applications settings. Generate a token with the read:user, write:repository and write:issue scopes, then paste it here.',
};
/** Untyped, with no suggestion of its kind, the host a forge picked starts with (GitLab: none). */
const DEFAULT_HOSTS: Partial<Record<ForgeKind, string>> = { github: 'github.com', forgejo: 'codeberg.org' };
/** A host worth asking which forge it runs: a name with a dot, and maybe a port. */
const ASKABLE = /^[a-z0-9-]+(\.[a-z0-9-]+)+(:\d+)?$/;
const CLASSIC_HELP = 'Opens GitHub\'s classic token page with the repo and read:org scopes selected. Classic tokens don\'t expire unless you set a date.';
/** GitHub's answer when an organization caps fine-grained token lifetimes. */
const FINE_GRAINED_REFUSED = 'forbids access via a fine-grained';


/** Settings › Accounts (spec #4 §3.2): the active profile's forge accounts, one per host. */
export function AccountsSection() {
  const [accounts, setAccounts] = useState<ForgeAccountView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [panel, setPanel] = useState<Panel | null>(null);
  const addButton = useRef<HTMLButtonElement>(null);
  const refocusAdd = useRef(false);
  const reload = useCallback(async (keepError = false) => {
    try {
      setAccounts(await api.forgeAccounts());
      if (!keepError) setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);
  useEffect(() => { void reload(); }, [reload]);
  useEffect(() => {
    if (!panel && refocusAdd.current) { refocusAdd.current = false; addButton.current?.focus(); }
  }, [panel]);
  const close = useCallback(() => { refocusAdd.current = true; setPanel(null); }, []);
  // Avatars: what the backend answers depends on the accounts (forge avatars come first).
  const changed = async () => {
    await reload();
    resetAvatars();
    notifyForgeAccountsChanged();
  };
  const remove = async (host: string) => {
    const ok = await confirmAction({
      title: `Remove the ${host} account?`,
      body: `Its token is deleted from this computer. Revoke it on ${host} too if you no longer need it.`,
      confirmLabel: 'Remove account',
      arm: `Click again to remove the ${host} account`,
      danger: true,
    });
    if (!ok) return;
    try {
      await api.removeForgeAccount(host);
    } catch (e) {
      // The list is read again, but the error stays to be read.
      setError(errorMessage(e));
      await reload(true);
      return;
    }
    await changed();
  };
  const addButtonEl = (primary: boolean) => (
    <button ref={addButton} type="button" className={primary ? 'primary' : undefined} onClick={() => setPanel({ mode: 'add' })}>+ Add account</button>
  );
  return (
    <div className="accounts-box" id="setting-forgeAccounts" data-setting-id="forgeAccounts">
      <p className="dim accounts-help">
        Connect GitLab, GitHub or Forgejo to see merge requests, pull requests and forks. One account per host.
        {accounts?.length === 0 && ' Tokens are kept in the system keyring.'}
      </p>
      {error && <p role="alert" className="setting-error">{error}</p>}
      {accounts?.length === 0 && !panel && (
        <div className="accounts-empty">
          <p>No accounts yet.</p>
          {addButtonEl(true)}
        </div>
      )}
      {accounts && accounts.length > 0 && (
        <ul className="accounts" aria-label="Forge accounts">
          {accounts.map((v) => (
            <AccountRow key={v.account.host} v={v} onReplace={() => setPanel({ mode: 'replace', host: v.account.host, kind: v.account.kind })} onRemove={() => void remove(v.account.host)} />
          ))}
        </ul>
      )}
      {panel && <AddAccount key={panel.mode === 'replace' ? panel.host : 'add'} panel={panel} taken={accounts?.map((a) => a.account) ?? []} onAdded={async () => { await changed(); close(); }} onCancel={close} onReplace={(host, kind) => setPanel({ mode: 'replace', host, kind })} />}
      {!panel && accounts && accounts.length > 0 && addButtonEl(false)}
    </div>
  );
}

function AccountRow({ v, onReplace, onRemove }: { v: ForgeAccountView; onReplace(): void; onRemove(): void }) {
  const a = v.account;
  const note = statusText(v);
  const chip = statusChip(v);
  const file = a.storage === 'file';
  // Forgejo's version carries the Gitea it's compatible with (`11.0.1+gitea-1.22.0`).
  const version = a.version?.replace(/\+gitea-.*$/, '');
  return (
    <li className="account" aria-label={`${a.host} account`}>
      <span className={`forge-mark is-${a.kind}`}><ForgeMark kind={a.kind} size={18} /></span>
      <div className="account-main">
        <div className="account-line">
          <span className="account-name">{a.user.name}</span>{' '}
          <span className="dim">@{a.user.username}</span>
        </div>
        <div className="account-meta dim">
          <span>{a.host} · {forgeName(a.kind)}{version ? ` ${version}` : ''}</span>
          <span className={`chip is-${chip.tone}`}>● {chip.label}</span>
          <span className={`chip${file ? ' is-warn' : ''}`}>{file ? 'File — not secure' : 'System keyring'}</span>
        </div>
        {file && <p className="account-note is-warn">{FILE_WARNING}</p>}
        {note && chip.tone === 'err' && <p className="account-note is-err" role="status">{note}{v.status.kind === 'authFailed' ? ' Replace it to reconnect.' : ''}</p>}
      </div>
      <div className="account-actions">
        <button type="button" className={chip.tone === 'err' ? undefined : 'ghost'} aria-label={`Replace the token for ${a.host}`} onClick={onReplace}>Replace token</button>
        <button type="button" className="ghost danger" aria-label={`Remove the ${a.host} account`} onClick={onRemove}>Remove</button>
      </div>
    </li>
  );
}

/** Host (the open repo's forge hosts first), forge, Create token, the token; Cancel / Add account. */
function AddAccount({ panel, taken, onAdded, onCancel, onReplace }: { panel: Panel; taken: Array<{ host: string; kind: ForgeKind }>; onAdded(): Promise<void>; onCancel(): void; onReplace(host: string, kind: ForgeKind): void }) {
  const replacing = panel.mode === 'replace';
  const activeTab = useAppState((s) => s.profile.activeTab);
  const overrides = useAppState((s) => s.profile.hostOverrides);
  const tab = useRuntime((s) => (activeTab ? s.tabs[activeTab] : undefined));
  const remotes = tab?.info?.remotes;
  const repoName = tab?.repo?.name;
  const suggestions = useMemo<HostSuggestion[]>(() => {
    const seen = new Set<string>();
    const out: HostSuggestion[] = [];
    for (const r of remotes ?? []) {
      if (!r.host || seen.has(r.host) || taken.some((t) => t.host === r.host) || effectiveKind(r.host, r.hostKind, overrides) === 'generic') continue;
      seen.add(r.host);
      out.push({ host: r.host, source: repoName ? `${r.name} · ${repoName}` : r.name });
    }
    return out;
  }, [remotes, overrides, taken, repoName]);
  const [typed, setTyped] = useState<string | null>(replacing ? panel.host : null);
  const [pickedEarly, setPickedEarly] = useState<ForgeKind | null>(replacing ? panel.kind : null);
  // Untyped, the host follows the forge picked: its first suggestion, and otherwise github.com for
  // GitHub (GitHub Enterprise Server is out of scope) or codeberg.org for Forgejo.
  const kindOf = (h: string) => effectiveKind(h, detectHostKind(h), overrides);
  const host = typed
    ?? (pickedEarly === null
      ? suggestions[0]?.host ?? ''
      : suggestions.find((x) => kindOf(x.host) === pickedEarly)?.host ?? DEFAULT_HOSTS[pickedEarly] ?? '');
  // A pasted address (`https://git.example.dev/`) is detected by its host, as the core reads it.
  const bare = bareHost(host);
  const byName = bare ? effectiveKind(bare, detectHostKind(bare), overrides) : 'generic';
  const picked = pickedEarly;
  const setPicked = setPickedEarly;
  // One account per host: a host that has one already gets its token replaced instead.
  const takenBy = replacing || bare === '' ? undefined : taken.find((t) => t.host === bare);
  const takenHost = takenBy !== undefined;
  // A host its name doesn't place (`git.example.dev`) is asked which forge it runs, no token, once
  // it's settled: when the field is left, or when Create token or Add is clicked. Never while
  // typing, so a half-typed host (`git.ex`) is never asked. A forge picked, a replaced account's
  // or a taken host's is known already.
  const [asked, setAsked] = useState<{ host: string; kind: HostKind } | null>(null);
  const [askingHost, setAskingHost] = useState<string | null>(null);
  const inFlight = useRef<{ host: string; answer: Promise<HostKind> } | null>(null);
  const needsAsk = !replacing && picked === null && !takenHost && byName === 'generic' && ASKABLE.test(bare) && asked?.host !== bare;
  const ask = (): Promise<HostKind> | null => {
    if (!needsAsk) return null;
    if (inFlight.current?.host === bare) return inFlight.current.answer;
    const h = bare;
    const answer = api.detectForge(h).catch((): HostKind => 'generic');
    inFlight.current = { host: h, answer };
    setAskingHost(h);
    void answer.then((k) => {
      // Another host was asked since: this answer is stale.
      if (inFlight.current?.answer !== answer) return;
      inFlight.current = null;
      setAskingHost(null);
      setAsked({ host: h, kind: k });
    });
    return answer;
  };
  const asking = needsAsk && askingHost === bare;
  const detected = byName !== 'generic' ? byName : asked?.host === bare ? asked.kind : 'generic';
  const kind: ForgeKind = picked ?? (detected === 'generic' ? 'gitlab' : detected);
  // What an action that waited for the answer goes on with: the host and forge as they are then.
  const latest = useRef({ bare, picked });
  latest.current = { bare, picked };
  /** The forge Create token and Add use, once the host is asked if it must be; `null`: the host
   * changed while it was asked, so the click was for another host. */
  const settledKind = async (): Promise<ForgeKind | null> => {
    const answer = ask();
    if (!answer) return kind;
    const k = await answer;
    if (latest.current.bare !== bare) return null;
    return latest.current.picked ?? (k === 'generic' ? 'gitlab' : k);
  };
  const [opening, setOpening] = useState(false);
  const [classic, setClassic] = useState(false);
  const [token, setToken] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const formRef = useRef<HTMLFormElement>(null);
  const hostRef = useRef<HTMLInputElement>(null);
  const tokenRef = useRef<HTMLInputElement>(null);
  const listOpen = useRef(false);
  const closeList = useRef<(() => void) | null>(null);
  useEffect(() => { (replacing || host ? tokenRef : hostRef).current?.focus(); }, []); // eslint-disable-line react-hooks/exhaustive-deps
  // Esc: the host list first (its own handler), then the panel, before the Settings dialog sees it.
  useEffect(() => {
    const own = (e: KeyboardEvent) => {
      if (!(e.target instanceof Node) || !formRef.current?.contains(e.target)) return false;
      if (listOpen.current) { closeList.current?.(); return true; }
      setToken('');
      onCancel();
      return true;
    };
    escOwners.add(own);
    return () => { escOwners.delete(own); };
  }, [onCancel]);
  const openTokenPage = async () => {
    if (opening) return;
    setOpening(true);
    try {
      const k = await settledKind();
      if (k) await api.openUrl(await api.forgeTokenPage(host, k, k === 'github' ? classic : undefined));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setOpening(false);
    }
  };
  const submit = async () => {
    if (!host || !token.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      const k = await settledKind();
      if (!k) return;
      await api.addForgeAccount(host, k, token);
      setToken('');
      await onAdded();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  const title = replacing ? `Replace the token for ${panel.host}` : 'Add a forge account';
  return (
    <form ref={formRef} className="account-add" aria-label={title} onSubmit={(e) => { e.preventDefault(); void submit(); }}>
      <h4>{title}</h4>
      <div className="field">
        <label htmlFor="account-host">Host</label>
        {replacing
          ? <input id="account-host" aria-label="Host" readOnly value={host} />
          : <div><HostCombobox inputRef={hostRef} value={host} suggestions={suggestions} onChange={(h) => { setTyped(h); setPicked(null); }} onBlur={() => void ask()} onOpenChange={(o) => { listOpen.current = o; }} closeRef={closeList} /></div>}
        {takenBy && (
          <p className="field-help account-taken" role="status">
            {bare} already has an account.{' '}
            <button type="button" className="link-btn" onClick={() => onReplace(takenBy.host, takenBy.kind)}>Replace its token</button>
          </p>
        )}
      </div>
      <div className="field">
        <span className="field-label">Forge</span>
        <div className="field-row">
          <div className="seg" role="group" aria-label="Forge type">
            {KINDS.map(([k, name]) => <button key={k} type="button" aria-pressed={kind === k} className={kind === k ? 'is-on' : undefined} onClick={() => setPicked(k)}>{name}</button>)}
          </div>
          <span className="dim">{picked === null && host ? (asking ? `Asking ${bare} which forge it runs…` : 'Detected from the host') : ''}</span>
        </div>
      </div>
      {kind === 'github' && (
        <div className="field">
          <span className="field-label">Token type</span>
          <div className="field-row">
            <div className="seg" role="group" aria-label="Token type">
              {([[false, 'Fine-grained'], [true, 'Classic']] as const).map(([c, name]) => <button key={name} type="button" aria-pressed={classic === c} className={classic === c ? 'is-on' : undefined} onClick={() => setClassic(c)}>{name}</button>)}
            </div>
          </div>
        </div>
      )}
      <div className="field">
        <label htmlFor="account-token">Token</label>
        <div className="field-row">
          <input ref={tokenRef} id="account-token" aria-label="Token" type="password" autoComplete="off" spellCheck={false} placeholder="Paste the personal access token" value={token} onChange={(e) => setToken(e.target.value)} />
          <button type="button" disabled={!host || takenHost} aria-busy={opening && asking} title={!host ? 'Enter the host first' : undefined} onClick={() => void openTokenPage()}>Create token on {host || 'the host'} ↗</button>
        </div>
        <p className="dim field-help">{kind === 'github' && classic ? CLASSIC_HELP : TOKEN_HELP[kind]}</p>
      </div>
      <div className="account-foot">
        <p className="setting-error account-add-error" role={error ? 'alert' : undefined}>
          {error}
          {error && kind === 'github' && !classic && error.includes(FINE_GRAINED_REFUSED) && (
            <>{' '}<button type="button" className="link-btn" onClick={() => { setClassic(true); setError(null); }}>Use a classic token instead</button></>
          )}
        </p>
        <button type="button" className="ghost" onClick={() => { setToken(''); onCancel(); }}>Cancel</button>
        <button type="submit" className="primary account-submit" aria-busy={busy} disabled={!host || takenHost || !token.trim() || busy}>{busy ? (asking ? 'Asking…' : 'Checking…') : replacing ? 'Replace token' : 'Add account'}</button>
      </div>
    </form>
  );
}
