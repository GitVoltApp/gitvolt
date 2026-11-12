import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
  appInfo: vi.fn(async () => ({ appVersion: '1.6.1', build: null, gitVersion: '2.47.1', installKind: 'none', home: null })),
  saveSettings: vi.fn(async () => null),
  openUrl: vi.fn(async () => null),
}));
vi.mock('../api/client', () => ({ api, errorMessage: (e: unknown) => String(e), onEvent: () => () => {} }));

const { checkLastRun, lastRunChange, resetWhatsNewForTest, useWhatsNew } = await import('./whatsNew');
const { WhatsNewDialog } = await import('./WhatsNewDialog');
const { WhatsNewNotes } = await import('./WhatsNew');
const { useAppInfo } = await import('../app/appInfo');
const { useAppState, DEFAULT_SETTINGS } = await import('../app/state');
const { runAction } = await import('../app/actions');
await import('./whatsNewFeature');

const startWith = async (lastRunVersion: string | null) => {
  useAppState.setState({ settings: { ...DEFAULT_SETTINGS, lastRunVersion } });
  await checkLastRun();
};

describe('lastRunChange', () => {
  it('only a newer version counts as an update', () => {
    expect(lastRunChange('1.5.0', '1.6.1')).toBe('updated');
    expect(lastRunChange('1.6.0-rc.1', '1.6.0')).toBe('updated');
    expect(lastRunChange(null, '1.6.1')).toBe('store');
    expect(lastRunChange('1.7.0', '1.6.1')).toBe('store');
    expect(lastRunChange('garbage', '1.6.1')).toBe('store');
    expect(lastRunChange('1.6.1', '1.6.1')).toBe('same');
    expect(lastRunChange('1.6.1+202611121200.abc1234', '1.6.1')).toBe('same');
  });
});

describe('checkLastRun at startup', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetWhatsNewForTest();
    useAppInfo.setState({ info: null });
  });

  it('an update: one unread notice from the last-run version, and the new version is stored', async () => {
    await startWith('1.5.0');
    expect(useWhatsNew.getState()).toMatchObject({ notice: { version: '1.6.1', since: '1.5.0' }, unread: true, updatedFrom: '1.5.0', open: null });
    expect(useAppState.getState().settings.lastRunVersion).toBe('1.6.1');
    // Once per session.
    useAppState.setState({ settings: { ...DEFAULT_SETTINGS, lastRunVersion: '1.0.0' } });
    await checkLastRun();
    expect(useWhatsNew.getState().notice).toEqual({ version: '1.6.1', since: '1.5.0' });
  });

  it('a first run stores the version without a notice', async () => {
    await startWith(null);
    expect(useWhatsNew.getState().notice).toBeNull();
    expect(useAppState.getState().settings.lastRunVersion).toBe('1.6.1');
  });

  it('a downgrade stores the version without a notice', async () => {
    await startWith('1.7.0');
    expect(useWhatsNew.getState().notice).toBeNull();
    expect(useAppState.getState().settings.lastRunVersion).toBe('1.6.1');
  });

  it('the same version changes nothing', async () => {
    const settings = { ...DEFAULT_SETTINGS, lastRunVersion: '1.6.1' };
    useAppState.setState({ settings });
    await checkLastRun();
    expect(useWhatsNew.getState().notice).toBeNull();
    expect(useAppState.getState().settings).toBe(settings);
  });
});

describe('the What\'s new panel', () => {
  afterEach(() => { cleanup(); act(() => useWhatsNew.getState().close()); });

  it('renders several versions newest first, each under its heading', async () => {
    render(<WhatsNewNotes entries={[
      { version: '1.6.1', date: '2026-11-12', body: '### Fixed\n\n- **Clone on Windows.** Works.' },
      { version: '1.6.0', date: '2026-11-06', body: '### Added\n\n- **Emoji completion.** Everywhere.' },
    ]} />);
    const sections = screen.getAllByRole('region');
    expect(sections.map((s) => s.getAttribute('aria-label'))).toEqual(['GitVolt 1.6.1', 'GitVolt 1.6.0']);
    expect(within(sections[0]).getByRole('heading', { level: 3 })).toHaveTextContent('1.6.1 · Nov 12, 2026');
    expect(await within(sections[0]).findByRole('heading', { level: 4, name: 'Fixed' })).toBeInTheDocument();
    expect(await within(sections[1]).findByRole('heading', { level: 4, name: 'Added' })).toBeInTheDocument();
    expect(sections[1].querySelector('strong')).toHaveTextContent('Emoji completion.');
  });

  it('says so when the version has no notes', () => {
    render(<WhatsNewNotes entries={[]} />);
    expect(screen.getByText('No notes for this version.')).toBeInTheDocument();
  });

  it('opens on the bundled changelog for a range, and Esc or Close closes it', async () => {
    useAppInfo.setState({ info: { appVersion: '1.6.1', build: null, gitVersion: '2.47.1', installKind: 'none', home: null } });
    render(<WhatsNewDialog />);
    expect(screen.queryByRole('dialog')).toBeNull();
    act(() => useWhatsNew.getState().show({ version: '1.6.1', since: '1.5.0' }));
    const d = screen.getByRole('dialog', { name: "What's new" });
    expect(d).toHaveTextContent("What's new in GitVolt 1.6.1");
    expect(d).toHaveTextContent('Every change since 1.5.0');
    const regions = await within(d).findAllByRole('region', { name: /^GitVolt / });
    expect(regions.map((s) => s.getAttribute('aria-label'))).toEqual(['GitVolt 1.6.1', 'GitVolt 1.6.0']);
    fireEvent.keyDown(d, { key: 'Escape' });
    expect(useWhatsNew.getState().open).toBeNull();
    act(() => useWhatsNew.getState().show({ version: '1.6.1', since: null }));
    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it("Help → What's new opens this version's notes, or this session's whole update", async () => {
    resetWhatsNewForTest();
    useAppInfo.setState({ info: { appVersion: '1.6.1', build: null, gitVersion: '2.47.1', installKind: 'none', home: null } });
    act(() => { expect(runAction('help.whatsNew')).toBe(true); });
    await vi.waitFor(() => expect(useWhatsNew.getState().open).toEqual({ version: '1.6.1', since: null }));
    act(() => useWhatsNew.setState({ updatedFrom: '1.5.0', open: null }));
    act(() => { runAction('help.whatsNew'); });
    await vi.waitFor(() => expect(useWhatsNew.getState().open).toEqual({ version: '1.6.1', since: '1.5.0' }));
  });
});
