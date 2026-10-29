import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../api/client', () => ({
  api: { gitIdentity: vi.fn(async () => ({ identity: null, signing: null })), saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null), forgeAccounts: vi.fn(async () => []), listOpeners: vi.fn(async () => []), openLogsFolder: vi.fn(async () => null) },
  errorMessage: String,
}));

// The feature registers the pages and the Settings action (Ctrl+,) the Keyboard page lists.
await import('../feature');
const { DEFAULT_SETTINGS, EMPTY_PROFILE, EMPTY_REPO_SETTINGS, useAppState } = await import('../../app/state');
const { useRuntime } = await import('../../app/runtime');
const { useDensity } = await import('../../theme/density');
const { allSettings, settingDef } = await import('../registry');
const { useSettingsUi } = await import('../schema');
const { SettingsOverlay } = await import('../frame/SettingsOverlay');
const { ConfirmDialog } = await import('../../ui/ConfirmDialog');
const { ContextMenu } = await import('../../menu/ContextMenu');
const { useMenu } = await import('../../menu/menuStore');
const { ArmLayer } = await import('../../ui/arm/ArmLayer');
const { disarm } = await import('../../ui/arm/store');
const { armClock, press } = await import('../../ui/arm/armTesting');

const PATH = '/r/orbital';
function show(focus?: string) {
  render(<><SettingsOverlay /><ConfirmDialog /><ContextMenu /><ArmLayer /></>);
  act(() => useSettingsUi.getState().show(focus));
  return screen.getByRole('dialog', { name: 'Settings' });
}
const groupTitles = (dialog: HTMLElement) => [...dialog.querySelectorAll('.settings-page .settings-group > h3')].map((h) => h.textContent);
const openPage = (name: string) => fireEvent.click(screen.getByRole('button', { name }));

beforeEach(() => {
  localStorage.clear();
  useDensity.getState().reload();
  useAppState.setState({
    settings: { ...DEFAULT_SETTINGS },
    profile: { ...EMPTY_PROFILE, name: 'Work', tabs: [{ id: 't1', kind: 'repo', path: PATH, alias: null }], activeTab: 't1' },
  });
  useRuntime.setState({
    tabs: { t1: { status: 'ready', error: null, repo: { id: 7, name: 'orbital', path: PATH } as never, graph: null, info: null, lastFetchAt: 0, fetchSkipped: null, limit: null, worktree: null,
      sidebar: { locals: [{ name: 'main', fullName: 'refs/heads/main' }, { name: 'feature/docking', fullName: 'refs/heads/feature/docking' }], remotes: [], tags: [], worktrees: [], stashes: [] } as never } },
  });
});
afterEach(() => act(() => { useSettingsUi.getState().close(); useSettingsUi.setState({ page: 'general', query: '' }); useMenu.getState().close(); }));

describe('settings pages', () => {
  it.each([
    ['advanceAfterStage', 'general'], ['updateCheck', 'general'], ['updatePrereleases', 'general'],
    ['theme', 'appearance'], ['graphColors', 'appearance'], ['themeColours', 'appearance'], ['density', 'appearance'], ['editorFontSize', 'appearance'],
    ['commitLimit', 'graph'], ['relativeDates', 'graph'], ['dateFormat', 'graph'], ['gravatar', 'graph'], ['forgeAvatars', 'graph'], ['collapseHoverOpen', 'graph'], ['collapsePlacement', 'graph'],
    ['stickyScroll', 'diffs'], ['fetchInterval', 'fetch'], ['prune', 'fetch'], ['pushFollowTags', 'fetch'],
    ['shortcuts', 'keyboard'], ['debugLogging', 'advanced'], ['openLogs', 'advanced'], ['resetAll', 'advanced'],
    ['extraGitconfig', 'git-identity'], ['editor', 'editor-folders'], ['reposFolder', 'editor-folders'],
    ['forgeAccounts', 'accounts'], ['hostOverrides', 'hosts'],
    ['repoEditor', 'repository'], ['repoPin', 'repository'], ['trunkPatterns', 'repository'], ['forgeTargetRemote', 'repository'], ['branchUpdate', 'repository'],
  ])('%s is registered once, on %s', (id, page) => {
    expect(allSettings().filter((d) => d.id === id)).toHaveLength(1);
    expect(settingDef(id)?.page).toBe(page);
  });

  it('every setting has a label, a group, keywords and a one-line description', () => {
    for (const d of allSettings()) expect(d.label && d.group && d.keywords && d.description, d.id).toBeTruthy();
  });

  it('General, Appearance, Graph, Fetch and push, and Advanced show their groups in order', () => {
    const dialog = show();
    expect(groupTitles(dialog)).toEqual(['Staging', 'Updates']);
    openPage('Appearance');
    expect(groupTitles(dialog)).toEqual(['Theme','Graph lanes · drag to reorder, click to change', 'Colours', 'Sizes']);
    expect(within(dialog).getByLabelText('Editor font size')).toHaveValue(13);
    openPage('Graph');
    expect(groupTitles(dialog)).toEqual(['History', 'Dates', 'Avatars', 'Hidden commits']);
    expect(allSettings().filter((d) => d.page === 'graph' && d.group === 'Dates').map((d) => d.id)).toEqual(['relativeDates', 'dateFormat']);
    openPage('Fetch and push');
    expect(groupTitles(dialog)).toEqual(['Fetch', 'Push']);
    openPage('Advanced');
    expect(groupTitles(dialog)).toEqual(['Logs', 'Reset']);
  });

  it('Reset all arms in place, then resets the GitVolt settings only', async () => {
    useAppState.getState().setSettings({ prune: false });
    useDensity.getState().setDensity('compact');
    useAppState.getState().updateProfile((p) => ({ ...p, reposFolder: '/srv/repos', repos: { [PATH]: { ...EMPTY_REPO_SETTINGS, editor: { kind: 'custom', template: 'zed {file}' } } } }));
    const clock = armClock();
    // happy-dom does no layout: a connected control reports one box, so it's "shown" and arms in place.
    const rects = vi.spyOn(HTMLElement.prototype, 'getClientRects').mockImplementation(function (this: HTMLElement) {
      return (this.isConnected ? [new DOMRect(10, 10, 24, 24)] : []) as unknown as DOMRectList;
    });
    try {
      show('resetAll');
      act(() => press(screen.getByRole('button', { name: 'Reset all settings…' })));
      const overlay = () => document.querySelector<HTMLElement>('[data-arm-overlay]');
      expect(overlay()).toHaveTextContent('Click again to reset every GitVolt setting');
      expect(useAppState.getState().settings.prune).toBe(false);
      clock.settle();
      await act(async () => { press(overlay()!); });
      await waitFor(() => expect(useAppState.getState().settings.prune).toBe(true));
      expect(useDensity.getState().density).toBe('standard');
      expect(useAppState.getState().profile.reposFolder).toBe('/srv/repos');
      expect(useAppState.getState().profile.repos[PATH].editor).toEqual({ kind: 'custom', template: 'zed {file}' });
    } finally {
      act(() => disarm());
      rects.mockRestore();
      clock.restore();
    }
  });

  it("Repository's pinned trunk writes the repository's pin, as the graph's pin button does", () => {
    show('repoPin');
    const pin = screen.getByRole('button', { name: 'Pinned trunk' });
    expect(pin).toHaveTextContent('Automatic');
    fireEvent.click(pin);
    expect(screen.getAllByRole('menuitem').map((m) => m.textContent)).toEqual(['Automatic', 'Off', 'main', 'feature/docking']);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Off' }));
    expect(useAppState.getState().profile.repos[PATH].pin).toEqual({ kind: 'off' });
    fireEvent.click(pin);
    fireEvent.click(screen.getByRole('menuitem', { name: 'feature/docking' }));
    expect(useAppState.getState().profile.repos[PATH].pin).toEqual({ kind: 'ref', name: 'refs/heads/feature/docking' });
  });

  it('Keyboard lists the shortcuts', () => {
    show('shortcuts');
    const row = screen.getAllByRole('listitem').find((li) => li.querySelector('.sc-label')?.textContent === 'Settings')!;
    expect(row).toBeTruthy();
    expect(within(row).getByText(',')).toBeInTheDocument();
  });

  it('Hosts sets a host to Forgejo, as to GitLab or GitHub', () => {
    const dialog = show();
    openPage('Hosts');
    fireEvent.click(within(dialog).getByRole('button', { name: 'Forge type for the added host' }));
    expect(useMenu.getState().rows?.map((r) => (r.kind === 'action' ? r.label : null))).toEqual(['GitLab', 'GitHub', 'Forgejo', 'Generic (no forge links)']);
    const forgejo = useMenu.getState().rows?.find((r) => r.kind === 'action' && r.label === 'Forgejo');
    act(() => { if (forgejo?.kind === 'action') forgejo.run(); useMenu.getState().close(); });
    fireEvent.change(within(dialog).getByLabelText('Add a host'), { target: { value: 'code.example.org' } });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Add' }));
    expect(useAppState.getState().profile.hostOverrides).toEqual({ 'code.example.org': 'forgejo' });
  });
});
