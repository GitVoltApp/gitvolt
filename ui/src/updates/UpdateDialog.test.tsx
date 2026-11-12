import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { InstallKind } from '../api/gen/InstallKind';
import type { UpdateRelease } from '../api/gen/UpdateRelease';
import type { UpdateState } from '../api/gen/UpdateState';

const api = vi.hoisted(() => ({
  updateDownload: vi.fn(async (): Promise<unknown> => ({ state: 'idle' })),
  updateCancel: vi.fn(async (): Promise<unknown> => ({ state: 'idle' })),
  updateInstall: vi.fn(async (): Promise<unknown> => ({ outcome: 'installed' })),
  updateRestart: vi.fn(async () => null),
  openUrl: vi.fn(async () => null),
  saveSettings: vi.fn(async () => null),
}));
const copyText = vi.hoisted(() => vi.fn(async () => {}));
vi.mock('../api/client', () => ({ api, errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)), onEvent: () => () => {} }));
vi.mock('../api/transport', () => ({ copyText, inTauri: () => false }));
const choose = vi.hoisted(() => vi.fn());
vi.mock('../ui/ConfirmDialog', async (orig) => ({ ...(await orig<typeof import('../ui/ConfirmDialog')>()), chooseAction: choose }));

const { UpdateDialog } = await import('./UpdateDialog');
const { useUpdates, resetUpdatesForTest } = await import('./store');
const { useAppInfo } = await import('../app/appInfo');
const { useAppState, DEFAULT_SETTINGS } = await import('../app/state');
const { useWorkingCopy } = await import('../diff/workingCopy');

const release: UpdateRelease = { version: '0.3.0', name: 'GitVolt 0.3.0', notes: [{ version: '0.3.0', publishedAt: null, notes: '## Added\n\n- **Updates** from GitHub' }], url: 'https://github.example/GitVoltApp/gitvolt/releases/tag/v0.3.0', prerelease: false, publishedAt: null, asset: { name: 'GitVolt_0.3.0_amd64.deb', size: 98 * 1024 * 1024 } };
const downloading: UpdateState = { state: 'downloading', release, received: 0, total: 100 };

function open(kind: InstallKind, state: UpdateState) {
  useAppInfo.setState({ info: { appVersion: '0.2.0', build: null, gitVersion: '2.47.1', installKind: kind, home: null } });
  render(<UpdateDialog />);
  act(() => { useUpdates.getState().setState(state); useUpdates.getState().openDialog(); });
  return screen.getByRole('dialog', { name: 'Update to GitVolt 0.3.0' });
}
const button = (name: string | RegExp) => screen.getByRole('button', { name });
const set = (state: UpdateState) => act(() => useUpdates.getState().setState(state));
/** Lets the chain's awaits run. */
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });
const dirty = () => useWorkingCopy.setState({ copies: { t: { key: 'k', path: 'a.txt', worktree: '/r', repo: 1, base: 'h0', dirty: true, view: 'diff' } } });

describe('UpdateDialog', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetUpdatesForTest();
    useAppState.setState({ settings: { ...DEFAULT_SETTINGS } });
    useWorkingCopy.setState({ copies: {} });
    api.updateDownload.mockResolvedValue(downloading);
  });

  it('shows the rendered notes, the package and its size, and View on GitHub', async () => {
    const d = open('deb', { state: 'ready', release });
    expect(await screen.findByRole('heading', { name: 'Added' })).toBeInTheDocument();
    expect(d.querySelector('.update-notes strong')).toHaveTextContent('Updates');
    expect(d).toHaveTextContent('GitVolt_0.3.0_amd64.deb · 98.0 MB');
    expect(d).toHaveTextContent('You have GitVolt 0.2.0.');
    fireEvent.click(button('View on GitHub'));
    expect(api.openUrl).toHaveBeenCalledWith(release.url);
    fireEvent.keyDown(d, { key: 'Escape' });
    expect(useUpdates.getState().dialogOpen).toBe(false);
  });

  it('Install and restart downloads, installs and restarts with no further click; the settings are saved first', async () => {
    open('deb', { state: 'available', release });
    expect(screen.getByText("Downloads GitVolt 0.3.0, checks it against the release's SHA256SUMS, installs it with apt (your system asks for your password) and restarts.")).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Download' })).toBeNull();
    act(() => useAppState.getState().setSettings({ gravatar: false }));
    await act(async () => { fireEvent.click(button('Install and restart')); });
    expect(api.updateDownload).toHaveBeenCalledTimes(1);
    // The download's progress, and Cancel.
    set({ ...downloading, received: 25 });
    expect(screen.getByRole('progressbar', { name: 'Download' })).toHaveAttribute('aria-valuenow', '25');
    expect(screen.getByText('25%')).toBeInTheDocument();
    expect(api.updateInstall).not.toHaveBeenCalled();
    set({ state: 'ready', release });
    await settle();
    expect(api.updateInstall).toHaveBeenCalledTimes(1);
    expect(api.updateRestart).toHaveBeenCalledTimes(1);
    expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ gravatar: false }));
    expect(api.saveSettings.mock.invocationCallOrder[0]).toBeLessThan(api.updateInstall.mock.invocationCallOrder[0]);
    expect(choose).not.toHaveBeenCalled();
  });

  it('a second press starts nothing more; closing the dialog mid-download lets it carry on', async () => {
    open('deb', { state: 'available', release });
    const go = button('Install and restart');
    await act(async () => { fireEvent.click(go); fireEvent.click(go); });
    expect(api.updateDownload).toHaveBeenCalledTimes(1);
    expect(go).toBeDisabled();
    act(() => useUpdates.getState().closeDialog());
    // Nor does one from elsewhere meanwhile (the dialog reopened).
    await act(async () => { await useUpdates.getState().installAndRestart(); });
    expect(api.updateDownload).toHaveBeenCalledTimes(1);
    set({ state: 'ready', release });
    await settle();
    expect(api.updateInstall).toHaveBeenCalledTimes(1);
    expect(api.updateRestart).toHaveBeenCalledTimes(1);
  });

  it('a download already done: the same button installs and restarts', async () => {
    open('deb', { state: 'ready', release });
    expect(screen.getByText(/^Downloaded and checked against the release's SHA256SUMS: GitVolt installs it with apt \(your system asks for your password\)/)).toBeInTheDocument();
    await act(async () => { fireEvent.click(button('Install and restart')); });
    await settle();
    expect(api.updateDownload).not.toHaveBeenCalled();
    expect(api.updateInstall).toHaveBeenCalledTimes(1);
    expect(api.updateRestart).toHaveBeenCalledTimes(1);
  });

  it('unsaved edits ask before anything starts: Cancel starts nothing, Discard edits goes on', async () => {
    open('deb', { state: 'available', release });
    dirty();
    choose.mockResolvedValueOnce(null);
    await act(async () => { fireEvent.click(button('Install and restart')); });
    await settle();
    expect(choose).toHaveBeenCalledWith(expect.objectContaining({ title: 'Discard unsaved changes?' }));
    expect(api.updateDownload).not.toHaveBeenCalled();
    expect(button('Install and restart')).toBeEnabled();
    choose.mockResolvedValueOnce('discard');
    await act(async () => { fireEvent.click(button('Install and restart')); });
    expect(api.updateDownload).toHaveBeenCalledTimes(1);
    set({ state: 'ready', release });
    await settle();
    // Asked once, at the start.
    expect(choose).toHaveBeenCalledTimes(2);
    expect(api.updateRestart).toHaveBeenCalledTimes(1);
  });

  it('an edit made during the download asks again before the restart: kept, the update waits on Restart GitVolt', async () => {
    open('deb', { state: 'available', release });
    api.updateInstall.mockImplementationOnce(async () => { useUpdates.getState().setState({ state: 'installed', release }); return { outcome: 'installed' }; });
    await act(async () => { fireEvent.click(button('Install and restart')); });
    expect(choose).not.toHaveBeenCalled();
    dirty();
    choose.mockResolvedValueOnce(null);
    set({ state: 'ready', release });
    await settle();
    expect(api.updateInstall).toHaveBeenCalledTimes(1);
    expect(choose).toHaveBeenCalledTimes(1);
    expect(choose).toHaveBeenCalledWith(expect.objectContaining({ title: 'Discard unsaved changes?' }));
    expect(api.updateRestart).not.toHaveBeenCalled();
    expect(button('Restart GitVolt')).toBeEnabled();
    expect(screen.queryByRole('alert')).toBeNull();
    expect(useUpdates.getState().error).toBeNull();
  });

  it('edits the user already agreed to discard, still unsaved at the end, ask only once', async () => {
    open('deb', { state: 'available', release });
    dirty();
    choose.mockResolvedValueOnce('discard');
    await act(async () => { fireEvent.click(button('Install and restart')); });
    set({ state: 'ready', release });
    await settle();
    expect(choose).toHaveBeenCalledTimes(1);
    expect(api.updateRestart).toHaveBeenCalledTimes(1);
  });

  it('a failed or cancelled download installs nothing and never restarts; failed says why, and Try again', async () => {
    open('deb', { state: 'available', release });
    await act(async () => { fireEvent.click(button('Install and restart')); });
    set({ state: 'failed', message: "SHA256SUMS doesn't list GitVolt_0.3.0_amd64.deb, so GitVolt deleted the download and won't install it.", release });
    await settle();
    expect(screen.getByText(/SHA256SUMS doesn't list/)).toBeInTheDocument();
    expect(api.updateInstall).not.toHaveBeenCalled();
    expect(api.updateRestart).not.toHaveBeenCalled();
    await act(async () => { fireEvent.click(button('Try again')); });
    expect(api.updateDownload).toHaveBeenCalledTimes(2);
    api.updateCancel.mockResolvedValueOnce({ state: 'available', release });
    await act(async () => { fireEvent.click(button('Cancel')); });
    expect(api.updateCancel).toHaveBeenCalled();
    await settle();
    expect(api.updateInstall).not.toHaveBeenCalled();
    expect(button('Install and restart')).toBeEnabled();
    // The download couldn't start at all.
    api.updateDownload.mockRejectedValueOnce(new Error("This build has no package to download: get the update from its release page"));
    await act(async () => { fireEvent.click(button('Install and restart')); });
    await settle();
    expect(screen.getByRole('alert')).toHaveTextContent('This build has no package to download');
    expect(api.updateInstall).not.toHaveBeenCalled();
    expect(api.updateRestart).not.toHaveBeenCalled();
  });

  it("deb: an install that couldn't run shows the command with Copy and doesn't restart; nor does one that failed", async () => {
    open('deb', { state: 'ready', release });
    api.updateInstall.mockResolvedValueOnce({ outcome: 'manual', command: 'sudo apt install /c/GitVolt_0.3.0_amd64.deb', reason: 'GitVolt couldn\'t get permission to install it.', output: null });
    await act(async () => { fireEvent.click(button('Install and restart')); });
    await settle();
    expect(screen.getByRole('alert')).toHaveTextContent(/^GitVolt couldn't get permission to install it\.$/);
    expect(screen.queryByLabelText('Package manager output')).toBeNull();
    expect(screen.getByText('Run this in a terminal instead:')).toBeInTheDocument();
    fireEvent.click(button('Copy sudo apt install /c/GitVolt_0.3.0_amd64.deb'));
    expect(copyText).toHaveBeenCalledWith('sudo apt install /c/GitVolt_0.3.0_amd64.deb');
    expect(api.updateRestart).not.toHaveBeenCalled();
    // A failed apt: its output apart from the reason, then the command.
    api.updateInstall.mockResolvedValueOnce({ outcome: 'manual', command: 'sudo apt install /c/GitVolt_0.3.0_amd64.deb', reason: 'The install failed (exit 100).', output: 'E: Sub-process /usr/bin/dpkg returned an error code (1)' });
    await act(async () => { fireEvent.click(button('Install and restart')); });
    await settle();
    expect(screen.getByRole('alert')).toHaveTextContent(/^The install failed \(exit 100\)\.$/);
    expect(screen.getByLabelText('Package manager output')).toHaveTextContent('E: Sub-process /usr/bin/dpkg returned an error code (1)');
    // The core refused the install (the package changed since it was checked).
    api.updateInstall.mockRejectedValueOnce(new Error('The downloaded package changed since it was checked, so GitVolt deleted it. Download it again.'));
    await act(async () => { fireEvent.click(button('Install and restart')); });
    await settle();
    expect(screen.getByText(/The downloaded package changed/)).toBeInTheDocument();
    expect(api.updateRestart).not.toHaveBeenCalled();
  });

  it('installing: busy; installed (its restart failed): Restart GitVolt asks about unsaved edits, then restarts', async () => {
    open('deb', { state: 'installing', release });
    expect(button('Installing…')).toBeDisabled();
    api.updateRestart.mockRejectedValueOnce(new Error("Couldn't restart GitVolt: no such file"));
    set({ state: 'installed', release });
    await act(async () => { fireEvent.click(button('Restart GitVolt')); });
    await settle();
    expect(screen.getByRole('alert')).toHaveTextContent("Couldn't restart GitVolt");
    expect(api.updateInstall).not.toHaveBeenCalled();
    dirty();
    choose.mockResolvedValueOnce(null);
    await act(async () => { fireEvent.click(button('Restart GitVolt')); });
    await settle();
    expect(api.updateRestart).toHaveBeenCalledTimes(1);
    choose.mockResolvedValueOnce('discard');
    await act(async () => { fireEvent.click(button('Restart GitVolt')); });
    await settle();
    expect(api.updateRestart).toHaveBeenCalledTimes(2);
  });

  it('arch: asks how first, remembers the answer; pacman installs and restarts, an AUR helper shows its commands', async () => {
    open('arch', { state: 'available', release: { ...release, asset: { name: 'GitVolt-0.3.0-1-x86_64.pkg.tar.zst', size: 1000 } } });
    expect(screen.getByText(/How did you install GitVolt\?/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Install and restart' })).toBeNull();
    fireEvent.click(button('Install with pacman'));
    expect(useAppState.getState().settings.updateArchMethod).toBe('pacman');
    expect(button('Install with pacman')).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByText(/installs it with pacman \(your system asks for your password\) and restarts/)).toBeInTheDocument();
    expect(button('Install and restart')).toBeInTheDocument();
    fireEvent.click(button('I use an AUR helper'));
    expect(useAppState.getState().settings.updateArchMethod).toBe('aur');
    expect(screen.getByText(/If you installed GitVolt from the AUR/)).toBeInTheDocument();
    expect(screen.getByText('yay -S gitvolt-bin')).toBeInTheDocument();
    expect(screen.getByText('paru -S gitvolt-bin')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Install and restart|Download/ })).toBeNull();
  });

  it.each([
    ['nsis', 'Install and restart', /opens the installer and closes so it can be replaced; the installer's last page starts the new version/],
    ['msi', 'Install', /opens Windows Installer \(which may ask for permission\) and closes so it can be replaced; start GitVolt again/],
  ] as const)('%s: %s downloads and starts the installer; GitVolt quits by itself, no restart of its own', async (kind, name, note) => {
    open(kind, { state: 'available', release: { ...release, asset: { name: kind === 'msi' ? 'GitVolt_0.3.0_x64.msi' : 'GitVolt_0.3.0_x64-setup.exe', size: 1000 } } });
    expect(screen.getByText(note)).toBeInTheDocument();
    api.updateInstall.mockResolvedValueOnce({ outcome: 'quitting' });
    await act(async () => { fireEvent.click(button(name)); });
    set({ state: 'ready', release });
    await settle();
    expect(api.updateInstall).toHaveBeenCalledTimes(1);
    expect(api.updateRestart).not.toHaveBeenCalled();
  });

  it("dmg: copies the app over this one and restarts; one it can't replace says to open the image", async () => {
    open('dmg', { state: 'available', release: { ...release, asset: { name: 'GitVolt_0.3.0_aarch64.dmg', size: 1000 } } });
    expect(screen.getByText(/copies its GitVolt\.app over this one and restarts\.$/)).toBeInTheDocument();
    api.updateInstall.mockResolvedValueOnce({ outcome: 'manual', command: 'open /c/GitVolt_0.3.0_aarch64.dmg', reason: "GitVolt can't write to /Applications, so it can't replace itself there. Open the disk image and drag GitVolt to Applications, replacing the old one.", output: null });
    await act(async () => { fireEvent.click(button('Install and restart')); });
    set({ state: 'ready', release });
    await settle();
    expect(screen.getByText('open /c/GitVolt_0.3.0_aarch64.dmg')).toBeInTheDocument();
    expect(api.updateRestart).not.toHaveBeenCalled();
    await act(async () => { fireEvent.click(button('Install and restart')); });
    await settle();
    expect(api.updateRestart).toHaveBeenCalledTimes(1);
  });

  it('a build from source, or a release without this install\'s package, offers only the release page', () => {
    const d = open('none', { state: 'available', release: { ...release, asset: null } });
    expect(d).toHaveTextContent('built from source');
    expect(screen.queryByRole('button', { name: /Download|Install/ })).toBeNull();
    expect(d.querySelector('.update-file')).toBeNull();
    expect(button('View on GitHub')).toBeInTheDocument();
    cleanup();
    const n = open('deb', { state: 'available', release: { ...release, asset: null } });
    expect(n).toHaveTextContent('This release has no package for this install');
    expect(screen.queryByRole('button', { name: /Download|Install/ })).toBeNull();
  });

  it('one version has no divider; several have one each, newest first; an empty body says so', async () => {
    const one = open('deb', { state: 'available', release });
    await screen.findByRole('heading', { name: 'Added' });
    expect(one.querySelector('.update-version-label')).toBeNull();
    cleanup();
    // Running 1.5.0, offered 1.6.1: every version in between.
    const notes = [
      { version: '1.6.1', publishedAt: '2026-10-19T12:00:00Z', notes: '### Fixed\n\n- Newest fix' },
      { version: '1.6.0', publishedAt: '2026-10-12T12:00:00Z', notes: '' },
      { version: '1.5.1', publishedAt: null, notes: '### Added\n\n- Older thing' },
    ];
    const d = open('deb', { state: 'available', release: { ...release, notes } });
    await screen.findByRole('heading', { name: 'Fixed' });
    expect([...d.querySelectorAll('.update-version-label')].map((e) => e.textContent)).toEqual(['1.6.1 · Oct 19, 2026', '1.6.0 · Oct 12, 2026', '1.5.1']);
    expect(d.querySelectorAll('.update-version')[1]).toHaveTextContent('No release notes.');
    expect(screen.getByRole('heading', { name: 'Added' })).toBeInTheDocument();
    cleanup();
    const none = open('deb', { state: 'available', release: { ...release, notes: [notes[1]!, { ...notes[1]!, version: '1.5.1' }] } });
    await vi.waitFor(() => expect(none.querySelectorAll('.update-notes-empty')).toHaveLength(1));
    expect(none.querySelector('.update-version-label')).toBeNull();
  });

  it('a pre-release says so', () => {
    const d = open('deb', { state: 'available', release: { ...release, prerelease: true } });
    expect(d.querySelector('.update-badge')).toHaveTextContent('Pre-release');
  });
});
