import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
  appInfo: vi.fn(async () => ({ appVersion: '1.2.3', gitVersion: '2.45.0', build: null, installKind: 'deb' })),
  updateCheck: vi.fn(async (): Promise<unknown> => ({ state: 'upToDate' })),
}));
vi.mock('../api/client', () => ({ api, errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)), onEvent: () => () => {} }));

const { About, useAbout } = await import('./About');
const { useUpdates, resetUpdatesForTest } = await import('../updates/store');

describe('About: focus trap (fix round 1)', () => {
  afterEach(() => { useAbout.setState({ open: false }); });

  it('focuses Close on open, and returns focus to the opener on close (toggled, not remounted)', () => {
    render(
      <>
        <button type="button">Open About</button>
        <About />
      </>,
    );
    const opener = screen.getByRole('button', { name: 'Open About' });
    opener.focus();
    act(() => useAbout.getState().setOpen(true));

    const close = screen.getByRole('button', { name: 'Close' });
    expect(document.activeElement).toBe(close); // autoFocus, not the opener

    act(() => useAbout.getState().setOpen(false));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(document.activeElement).toBe(opener);
  });
});

describe('About: Check for updates', () => {
  afterEach(() => { useAbout.setState({ open: false }); resetUpdatesForTest(); });

  it('checks, and says up to date or why it failed; an update it finds opens the update dialog at once, About closing under it', async () => {
    render(<About />);
    act(() => useAbout.getState().setOpen(true));
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Check for updates' })); });
    expect(screen.getByRole('status')).toHaveTextContent('GitVolt is up to date.');
    api.updateCheck.mockRejectedValueOnce(new Error("Couldn't reach api.github.com"));
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Check for updates' })); });
    expect(screen.getByRole('status')).toHaveTextContent("Couldn't check for updates: Couldn't reach api.github.com");
    expect(useUpdates.getState().dialogOpen).toBe(false);
    const release = { version: '1.3.0', name: 'GitVolt 1.3.0', notes: [], url: 'u', prerelease: false, publishedAt: null, asset: null };
    api.updateCheck.mockResolvedValueOnce({ state: 'available', release });
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Check for updates' })); });
    expect(useUpdates.getState().dialogOpen).toBe(true);
    expect(useAbout.getState().open).toBe(false);
    // Opened again with the update known: its line, and no Show update (Check for updates opens it).
    act(() => { useUpdates.getState().closeDialog(); useAbout.getState().setOpen(true); });
    expect(screen.getByRole('status')).toHaveTextContent('GitVolt 1.3.0 is available.');
    expect(screen.queryByRole('button', { name: 'Show update' })).toBeNull();
    // One already downloaded: the check returns it as it is, and the dialog opens.
    api.updateCheck.mockResolvedValueOnce({ state: 'ready', release });
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Check for updates' })); });
    expect(useUpdates.getState().dialogOpen).toBe(true);
    expect(useAbout.getState().open).toBe(false);
  });

  it('Help › Check for updates opens About, and the update dialog over it when there is one', async () => {
    const { runAction } = await import('./actions');
    await import('../updates/feature');
    render(<About />);
    const release = { version: '1.3.0', name: 'GitVolt 1.3.0', notes: [], url: 'u', prerelease: false, publishedAt: null, asset: null };
    api.updateCheck.mockResolvedValueOnce({ state: 'upToDate' });
    await act(async () => { runAction('help.checkUpdates'); });
    expect(useAbout.getState().open).toBe(true);
    expect(useUpdates.getState().dialogOpen).toBe(false);
    api.updateCheck.mockResolvedValueOnce({ state: 'available', release });
    await act(async () => { runAction('help.checkUpdates'); });
    expect(useUpdates.getState().dialogOpen).toBe(true);
    expect(useAbout.getState().open).toBe(false);
  });
});
