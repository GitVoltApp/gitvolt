import { Copy, ExternalLink } from 'lucide-react';
import { lazy, Suspense, type ReactNode } from 'react';
import { api } from '../api/client';
import type { InstallKind } from '../api/gen/InstallKind';
import type { UpdateRelease } from '../api/gen/UpdateRelease';
import type { UpdateState } from '../api/gen/UpdateState';
import { copyText } from '../api/transport';
import { useAppInfo } from '../app/appInfo';
import { useModalKeys } from '../app/modalKeys';
import { useAppState } from '../app/state';
import { useToast } from '../ui/toastStore';
import { formatSize, percentOf, releaseOf, useUpdates } from './store';
import './updates.css';

const ReleaseNotes = lazy(() => import('./ReleaseNotes'));

/** What an AUR install runs instead (there's no `gitvolt-bin` on the AUR yet: "if you installed
 * from the AUR"). */
export const AUR_COMMANDS = ['yay -S gitvolt-bin', 'paru -S gitvolt-bin'] as const;

function copy(text: string) {
  void copyText(text).then(() => useToast.getState().show('Copied'), () => useToast.getState().show('Copy failed'));
}

/** A command to run in a terminal, with Copy. */
function Command({ text }: { text: string }) {
  return (
    <div className="update-command">
      <code>{text}</code>
      <button type="button" aria-label={`Copy ${text}`} onClick={() => copy(text)}><Copy size={12} aria-hidden /> Copy</button>
    </div>
  );
}

type Kind = Exclude<InstallKind, 'none'>;

/** What GitVolt does with the verified package, per install kind (the core's `install.rs`). */
const INSTALLS: Record<Kind, string> = {
  deb: 'installs it with apt (your system asks for your password) and restarts',
  arch: 'installs it with pacman (your system asks for your password) and restarts',
  rpm: 'installs it with dnf or zypper (your system asks for your password) and restarts',
  dmg: 'copies its GitVolt.app over this one and restarts',
  // The installer's last page starts GitVolt (its Start GitVolt box, ticked).
  nsis: "opens the installer and closes so it can be replaced; the installer's last page starts the new version",
  // Windows Installer doesn't start it again.
  msi: "opens Windows Installer (which may ask for permission) and closes so it can be replaced; start GitVolt again once it's done",
};

/** The one button's label: on Windows the installer, not GitVolt, finishes the job. */
const label = (kind: Kind) => (kind === 'msi' ? 'Install' : 'Install and restart');

/** The dialog's middle: what can be done now, for this install. */
function Action({ state, release, kind }: { state: UpdateState; release: UpdateRelease; kind: InstallKind }) {
  const { installAndRestart, cancel } = useUpdates.getState();
  const outcome = useUpdates((s) => s.outcome);
  const running = useUpdates((s) => s.running);
  const archMethod = useAppState((s) => s.settings.updateArchMethod);
  if (!release.asset || kind === 'none') {
    return <p className="update-message">{kind === 'none' ? 'This GitVolt was built from source: get the new version from its release page.' : `This release has no package for this install: get it from its release page.`}</p>;
  }
  const arch = kind === 'arch' ? (
    <div className="update-choices" role="group" aria-label="How do you install GitVolt?">
      <button type="button" aria-pressed={archMethod === 'pacman'} onClick={() => useAppState.getState().setSettings({ updateArchMethod: 'pacman' })}>Install with pacman</button>
      <button type="button" aria-pressed={archMethod === 'aur'} onClick={() => useAppState.getState().setSettings({ updateArchMethod: 'aur' })}>I use an AUR helper</button>
    </div>
  ) : null;
  if (kind === 'arch' && archMethod === 'aur') {
    return (
      <>
        {arch}
        <p className="update-message">If you installed GitVolt from the AUR, update it with your helper:</p>
        {AUR_COMMANDS.map((c) => <Command key={c} text={c} />)}
      </>
    );
  }
  // Arch asks how first: Install and restart then knows what to do at the end.
  const needsChoice = kind === 'arch' && !archMethod;
  const plan = (text: string) => <p className="update-message">{needsChoice ? 'How did you install GitVolt? GitVolt remembers your answer.' : text}</p>;
  const go = (text: string, busy: boolean) => !needsChoice && <div className="modal-actions"><button type="button" className="primary" disabled={busy} onClick={() => void installAndRestart()}>{text}</button></div>;
  const whole = `Downloads GitVolt ${release.version}, checks it against the release's SHA256SUMS, ${INSTALLS[kind]}.`;
  switch (state.state) {
    case 'available':
      return <>{arch}{plan(whole)}{go(label(kind), running)}</>;
    case 'downloading': {
      const pct = percentOf(state);
      return (
        <>
          {plan(whole)}
          <div className="update-progress">
            <span className="sb-update-bar" role="progressbar" aria-label="Download" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct}><span style={{ width: `${pct}%` }} /></span>
            <span>{pct}%</span>
            <button type="button" onClick={() => void cancel()}>Cancel</button>
          </div>
        </>
      );
    }
    case 'ready':
    case 'installing': {
      const busy = running || state.state === 'installing';
      return (
        <>
          {arch}
          {plan(`Downloaded and checked against the release's SHA256SUMS: GitVolt ${INSTALLS[kind]}.`)}
          {outcome?.outcome === 'manual' && (
            <>
              <p className="update-message error" role="alert">{outcome.reason}</p>
              {/* The package manager's own words, apart from GitVolt's: what to search for or report. */}
              {outcome.output && <pre className="update-output" aria-label="Package manager output">{outcome.output}</pre>}
              <p className="update-message">Run this in a terminal instead:</p>
              <Command text={outcome.command} />
            </>
          )}
          {go(busy ? 'Installing…' : label(kind), busy)}
        </>
      );
    }
    case 'installed':
      // A restart that didn't happen (it failed, or unsaved edits were kept).
      return (
        <>
          <p className="update-message">GitVolt {release.version} is installed. Restart GitVolt to use it.</p>
          <div className="modal-actions"><button type="button" className="primary" disabled={running} onClick={() => void installAndRestart()}>{running ? 'Restarting…' : 'Restart GitVolt'}</button></div>
        </>
      );
    case 'failed':
      return (
        <>
          {arch}
          <p className="update-message error">{state.message}</p>
          {go('Try again', running)}
        </>
      );
    default:
      return null;
  }
}

/**
 * The update dialog (from the status bar's pill, or a check from About): the notes of every
 * version since this one, rendered; the package and its size; and Install and restart, which
 * does what this install takes (the install kind the package baked in). A build from source, or
 * a release without this install's package, only links to the release page. View on GitHub opens
 * it in the browser.
 */
export function UpdateDialog() {
  const open = useUpdates((s) => s.dialogOpen);
  const state = useUpdates((s) => s.state);
  const error = useUpdates((s) => s.error);
  const info = useAppInfo((s) => s.info);
  const close = () => useUpdates.getState().closeDialog();
  const ref = useModalKeys<HTMLDivElement>(open, close);
  const release = releaseOf(state);
  if (!open || !release) return null;
  const kind: InstallKind = info?.installKind ?? 'none';
  let file: ReactNode = null;
  if (release.asset && kind !== 'none') file = <p className="update-file">{release.asset.name} · {formatSize(release.asset.size)}</p>;
  return (
    <div className="modal-backdrop" onPointerDown={close}>
      <div ref={ref} className="modal update-dialog" role="dialog" aria-label={`Update to GitVolt ${release.version}`} onPointerDown={(e) => e.stopPropagation()}>
        <h2>{release.name}{release.prerelease && <span className="update-badge">Pre-release</span>}</h2>
        <p className="update-sub">You have GitVolt {info?.appVersion ?? '…'}.</p>
        <div className="update-notes" tabIndex={0} aria-label="Release notes">
          <Suspense fallback={<p aria-busy="true">Loading…</p>}><ReleaseNotes notes={release.notes} /></Suspense>
        </div>
        {file}
        <Action state={state} release={release} kind={kind} />
        {error && <p className="update-message error" role="alert">{error}</p>}
        <div className="modal-actions">
          <button type="button" className="update-link" onClick={() => void api.openUrl(release.url)}><ExternalLink size={12} aria-hidden /> View on GitHub</button>
          <button type="button" autoFocus onClick={close}>Close</button>
        </div>
      </div>
    </div>
  );
}
