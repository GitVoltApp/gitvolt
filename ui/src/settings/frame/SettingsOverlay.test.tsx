import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS, EMPTY_PROFILE } from '../../app/state';

const api = vi.hoisted(() => ({
  forgeAccounts: vi.fn(async () => []),
  saveSettings: vi.fn(async () => null),
  saveProfile: vi.fn(async () => null),
  pickFolder: vi.fn(async () => null),
  gitIdentity: vi.fn(async () => ({ identity: null, signing: null })),
  listOpeners: vi.fn(async () => [
    { id: 'vscode', name: 'Visual Studio Code', kind: 'editor' },
    { id: 'files', name: 'Files', kind: 'fileManager' },
  ]),
  validateEditorTemplate: vi.fn(async (t: string) => {
    if (t.startsWith('sh -c')) throw new Error("a shell or interpreter's code argument can't contain {file}, {line} or {repo}");
    return null;
  }),
}));
vi.mock('../../api/client', () => ({ api, errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)) }));
const transport = vi.hoisted(() => ({ copyText: vi.fn(async () => {}), inTauri: () => false, readClipboardText: vi.fn(async () => 'pasted text') }));
vi.mock('../../api/transport', () => transport);

await import('../pages');
const { SettingsOverlay } = await import('./SettingsOverlay');
const { registerSettingsFloat } = await import('./floating');
const { useSettingsPaste } = await import('../paste');
const { useSettingsUi } = await import('../schema');
const { allSettings, settingsPages } = await import('../registry');
const { useAppState } = await import('../../app/state');
const { useDensity } = await import('../../theme/density');
const { resetOpenersForTests } = await import('../../openIn/openers');
const { useRuntime } = await import('../../app/runtime');
const { ConfirmDialog } = await import('../../ui/ConfirmDialog');
const { avatars } = await import('../../avatars/avatarStore');
const { registerKeys } = await import('../../ui/keyRouter');
const { ContextMenu } = await import('../../menu/ContextMenu');
const { useMenu } = await import('../../menu/menuStore');

const tab = { id: 't1', kind: 'repo' as const, path: '/r/orbital', alias: null };
function show(focus?: string) {
  render(<><SettingsOverlay /><ConfirmDialog /><ContextMenu /></>);
  act(() => useSettingsUi.getState().show(focus));
  return screen.getByRole('dialog', { name: 'Settings' });
}
const key = (el: Element, k: string, init: KeyboardEventInit = {}) => fireEvent.keyDown(el, { key: k, ...init });
const navItem = (name: string) => screen.getByRole('button', { name });
const openPage = (name: string) => fireEvent.click(navItem(name));
/** Opens the dropdown button by its name and picks one of its rows. */
function pick(button: string, row: string) {
  fireEvent.click(screen.getByRole('button', { name: button }));
  fireEvent.click(screen.getByRole('menuitem', { name: row }));
}

beforeEach(() => {
  vi.clearAllMocks();
  Element.prototype.scrollIntoView = vi.fn();
  resetOpenersForTests();
  localStorage.clear();
  useDensity.getState().reload();
  useAppState.setState({ settings: DEFAULT_SETTINGS, profile: { ...EMPTY_PROFILE, name: 'Work', color: '#de9b43', tabs: [tab], activeTab: 't1' } });
  useRuntime.setState({
    tabs: { t1: { status: 'ready', error: null, repo: { id: 7, name: 'orbital', path: '/r/orbital' } as never, graph: null, sidebar: null, lastFetchAt: 0, fetchSkipped: null, limit: null, worktree: null,
      info: { remotes: [{ name: 'origin', url: 'u', host: 'code.example.com', path: 'acme/orbital', hostKind: 'generic' }] } as never } },
  });
});
afterEach(() => { act(() => { useSettingsUi.getState().close(); useSettingsUi.setState({ page: 'general', query: '' }); useMenu.getState().close(); }); });

describe('SettingsOverlay', () => {
  it('opens on General, with the nav in three groups named for the profile and the repository; nothing when closed', () => {
    const { container } = render(<SettingsOverlay />);
    expect(container.innerHTML).toBe('');
    act(() => useSettingsUi.getState().show());
    const nav = screen.getByRole('navigation', { name: 'Settings pages' });
    expect(within(nav).getByText('GitVolt')).toBeInTheDocument();
    expect(within(nav).getByText('Profile · Work')).toBeInTheDocument();
    expect(within(nav).getByText('Repository · orbital')).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'General' })).toBeInTheDocument();
    expect(navItem('General')).toHaveAttribute('aria-current', 'page');
    expect(screen.getByLabelText('Search settings')).toHaveFocus();
  });

  it('without a repository there is no Repository group, and a stored Repository page shows General', () => {
    useRuntime.setState({ tabs: {} });
    act(() => useSettingsUi.setState({ page: 'repository' }));
    show();
    expect(screen.queryByText(/^Repository · /)).toBeNull();
    expect(screen.queryByRole('button', { name: 'Repository' })).toBeNull();
    expect(screen.getByRole('region', { name: 'General' })).toBeInTheDocument();
  });

  it('every page lists the settings registered on it', () => {
    const dialog = show();
    for (const page of settingsPages()) {
      if (page.id === 'keyboard' || page.id === 'accounts') continue;
      openPage(page.label);
      for (const d of allSettings().filter((s) => s.page === page.id)) expect(dialog.querySelector(`[data-setting-id="${d.id}"]`), d.id).not.toBeNull();
    }
  });

  describe('keyboard', () => {
    it('↑/↓, Home and End move through the nav; Enter opens the focused page', () => {
      show();
      act(() => navItem('General').focus());
      key(navItem('General'), 'ArrowDown');
      expect(navItem('Appearance')).toHaveFocus();
      key(navItem('Appearance'), 'ArrowUp');
      expect(navItem('General')).toHaveFocus();
      key(navItem('General'), 'End');
      expect(navItem('Repository')).toHaveFocus();
      expect(navItem('Repository')).toHaveAttribute('tabindex', '0');
      expect(navItem('General')).toHaveAttribute('tabindex', '-1');
      key(navItem('Repository'), 'Home');
      key(navItem('General'), 'ArrowDown');
      key(navItem('Appearance'), 'Enter');
      expect(screen.getByRole('region', { name: 'Appearance' })).toBeInTheDocument();
    });

    it('a character typed in the nav goes to the search box', () => {
      show();
      act(() => navItem('Graph').focus());
      key(navItem('Graph'), 'a');
      const search = screen.getByLabelText('Search settings');
      expect(search).toHaveFocus();
      expect(search).toHaveValue('a');
      expect(useSettingsUi.getState().query).toBe('a');
    });

    it('Ctrl+F focuses the search box', () => {
      show('prune');
      expect(screen.getByLabelText('Prune deleted remote branches on fetch')).toHaveFocus();
      key(document.activeElement!, 'f', { code: 'KeyF', ctrlKey: true });
      expect(screen.getByLabelText('Search settings')).toHaveFocus();
    });

    it('Esc with a query clears it and stays open; Esc on an empty box closes Settings', () => {
      show();
      const search = screen.getByLabelText('Search settings');
      fireEvent.change(search, { target: { value: 'prune' } });
      expect(screen.getByRole('heading', { name: '1 setting matches "prune"' })).toBeInTheDocument();
      key(search, 'Escape');
      expect(search).toHaveValue('');
      expect(useSettingsUi.getState().open).toBe(true);
      expect(screen.getByRole('region', { name: 'General' })).toBeInTheDocument();
      key(search, 'Escape');
      expect(useSettingsUi.getState().open).toBe(false);
    });

    it('Space toggles a switch once (its own key handler never sees keys in Settings)', () => {
      show('prune');
      const sw = screen.getByLabelText('Prune deleted remote branches on fetch');
      key(sw, ' ');
      fireEvent.keyUp(sw, { key: ' ' });
      expect(useAppState.getState().settings.prune).toBe(false);
    });

    it('Escape closes it, and Ctrl+W typed in its fields never reaches the app', () => {
      const app = vi.fn();
      const off = registerKeys('app', app);
      show('commitLimit');
      const input = screen.getByLabelText('Commits loaded in the graph');
      key(input, 'w', { code: 'KeyW', ctrlKey: true });
      expect(app).not.toHaveBeenCalled();
      expect(useSettingsUi.getState().open).toBe(true);
      key(input, 'Escape');
      expect(useSettingsUi.getState().open).toBe(false);
      expect(app).not.toHaveBeenCalled();
      off();
    });
  });

  it('opened for one setting it shows its page, focuses the control and flashes the row', () => {
    const dialog = show('prune');
    expect(navItem('Fetch and push')).toHaveAttribute('aria-current', 'page');
    expect(dialog.querySelector('section[data-page="general"]')).toBeNull();
    expect(dialog.querySelector('#input-prune')).toHaveFocus();
    expect(dialog.querySelector('[data-setting-id="prune"]')).toHaveClass('flash');
  });

  it('Ctrl+V goes to the page\'s paste handler when the focus is on a button, not in a field', async () => {
    const handler = vi.fn();
    function Probe() { useSettingsPaste(handler); return null; }
    render(<><SettingsOverlay /><Probe /></>);
    act(() => useSettingsUi.getState().show());
    const search = screen.getByLabelText('Search settings');
    key(search, 'v', { code: 'KeyV', ctrlKey: true });
    await act(async () => {});
    expect(handler).not.toHaveBeenCalled();
    act(() => navItem('Graph').focus());
    key(navItem('Graph'), 'v', { code: 'KeyV', ctrlKey: true });
    await waitFor(() => expect(handler).toHaveBeenCalledWith('pasted text'));
  });

  it('a floating panel takes the overlay\'s place, with no backdrop', () => {
    const off = registerSettingsFloat('t-float', () => <div role="dialog" aria-label="Floating editor">floating</div>);
    const { container } = render(<SettingsOverlay />);
    act(() => { useSettingsUi.getState().show(); useSettingsUi.getState().setFloating('t-float'); });
    expect(screen.getByRole('dialog', { name: 'Floating editor' })).toBeInTheDocument();
    expect(screen.queryByRole('dialog', { name: 'Settings' })).toBeNull();
    expect(container.querySelector('.modal-backdrop')).toBeNull();
    act(() => useSettingsUi.getState().setFloating(null));
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
    off();
  });

  describe('settings', () => {
    it('density is a three-way radio over the density store, persisted', () => {
      const dialog = show('density');
      expect(screen.getByRole('radio', { name: 'Standard' })).toBeChecked();
      fireEvent.click(screen.getByRole('radio', { name: 'Compact' }));
      expect(useDensity.getState().density).toBe('compact');
      expect(localStorage.getItem('gitvolt.density.v1')).toBe('compact');
      expect(dialog.querySelectorAll('input[name="density"]')).toHaveLength(3);
    });

    it('sticky scroll toggles the editor setting, with its description shown', () => {
      show('stickyScroll');
      const sw = screen.getByLabelText('Sticky scroll');
      expect(sw).not.toBeChecked();
      fireEvent.click(sw);
      expect(useAppState.getState().settings.editorStickyScroll).toBe(true);
      expect(localStorage.getItem('gitvolt.editorSettings.v1')).toBeNull();
      expect(screen.getByText(/Pins the enclosing scope's first line/)).toBeVisible();
    });

    it('the date format, fetch interval, prune and push tags write the app settings', () => {
      show('dateFormat');
      pick('Date format', '2026-09-26 15:14');
      expect(useAppState.getState().settings.dateFormat).toBe('ymd24h');
      openPage('Fetch and push');
      fireEvent.click(screen.getByLabelText('Prune deleted remote branches on fetch'));
      expect(useAppState.getState().settings.prune).toBe(false);
      pick('Background fetch interval', 'Off');
      expect(useAppState.getState().settings.fetchIntervalSecs).toBe(0);
      const tags = screen.getByLabelText('Push tags with branches');
      expect(tags).toBeChecked();
      fireEvent.click(tags);
      expect(useAppState.getState().settings.pushFollowTags).toBe(false);
    });

    it('Updates: the automatic check is on and pre-releases off by default, and both write the app settings', () => {
      show('updateCheck');
      const auto = screen.getByLabelText('Check for updates automatically');
      const pre = screen.getByLabelText('Include pre-releases');
      expect(auto).toBeChecked();
      expect(pre).not.toBeChecked();
      fireEvent.click(auto);
      fireEvent.click(pre);
      expect(useAppState.getState().settings.updateCheck).toBe(false);
      expect(useAppState.getState().settings.updatePrereleases).toBe(true);
    });

    it('shows a hand-edited fetch interval clamped, and not as changed', () => {
      useAppState.setState({ settings: { ...DEFAULT_SETTINGS, fetchIntervalSecs: 1 } });
      const dialog = show('fetchInterval');
      expect(screen.getByLabelText('Background fetch interval')).toHaveTextContent('Every minute');
      expect(dialog.querySelector('[data-setting-id="fetchInterval"]')).not.toHaveAttribute('data-modified');
    });

    it('the commit limit applies on Enter, clamped, not per keystroke', () => {
      show('commitLimit');
      const input = screen.getByLabelText('Commits loaded in the graph');
      fireEvent.change(input, { target: { value: '3' } });
      expect(useAppState.getState().settings.commitLimit).toBe(2000);
      fireEvent.submit(input.closest('form')!);
      expect(useAppState.getState().settings.commitLimit).toBe(3);
      fireEvent.change(input, { target: { value: '999999' } });
      fireEvent.blur(input);
      expect(useAppState.getState().settings.commitLimit).toBe(50_000);
    });

    it("a window blur (the WM's focus bounce) doesn't commit a field mid-edit; a real blur does", () => {
      show('commitLimit');
      const input = screen.getByLabelText('Commits loaded in the graph');
      fireEvent.change(input, { target: { value: '77' } });
      const hasFocus = vi.spyOn(document, 'hasFocus').mockReturnValue(false);
      fireEvent.blur(input);
      expect(useAppState.getState().settings.commitLimit).toBe(2000);
      hasFocus.mockReturnValue(true);
      fireEvent.blur(input);
      expect(useAppState.getState().settings.commitLimit).toBe(77);
      hasFocus.mockRestore();
    });

    it('the Gravatar and forge avatar switches save, then make the avatars ask again', async () => {
      const reset = vi.spyOn(avatars, 'reset');
      show('gravatar');
      fireEvent.click(screen.getByLabelText('From Gravatar'));
      expect(useAppState.getState().settings.gravatar).toBe(false);
      await waitFor(() => expect(reset).toHaveBeenCalled());
      expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ gravatar: false }));
      reset.mockClear();
      fireEvent.click(screen.getByLabelText('From your forge accounts'));
      expect(useAppState.getState().settings.forgeAvatars).toBe(false);
      await waitFor(() => expect(reset).toHaveBeenCalled());
    });

    it('picks a detected editor, and a Custom command that the guard refuses shows its error inline and is not saved', async () => {
      show('editor');
      await waitFor(() => expect(api.listOpeners).toHaveBeenCalled());
      await act(async () => {});
      pick('Default editor', 'Visual Studio Code');
      expect(useAppState.getState().profile.editor).toEqual({ kind: 'opener', id: 'vscode' });

      pick('Default editor', 'Custom command…');
      const command = screen.getByLabelText('Custom editor command');
      fireEvent.change(command, { target: { value: 'sh -c "geany {file}"' } });
      const alert = await screen.findByRole('alert');
      expect(alert).toHaveTextContent("can't contain {file}, {line} or {repo}");
      expect(useAppState.getState().profile.editor).toEqual({ kind: 'opener', id: 'vscode' });

      fireEvent.change(command, { target: { value: 'subl {file}:{line}' } });
      await waitFor(() => expect(useAppState.getState().profile.editor).toEqual({ kind: 'custom', template: 'subl {file}:{line}' }));
      expect(screen.queryByRole('alert')).toBeNull();
      // Not clobbered by the echo of its own save.
      expect(command).toHaveValue('subl {file}:{line}');
    });

    it("the repository's editor override is its own, and 'Same as the profile' clears it", () => {
      show('repoEditor');
      expect(screen.getByText('Settings for orbital only.')).toBeInTheDocument();
      pick('Editor for this repository', 'Custom command…');
      pick('Editor for this repository', 'Same as the profile');
      expect(useAppState.getState().profile.repos['/r/orbital']?.editor ?? null).toBeNull();
    });

    it('host overrides: set one for a detected host, clear it, or add a host by hand', () => {
      show('hostOverrides');
      const select = screen.getByLabelText('Forge type for code.example.com');
      expect(select).toHaveTextContent('Detected (Generic)');
      pick('Forge type for code.example.com', 'GitLab');
      expect(useAppState.getState().profile.hostOverrides).toEqual({ 'code.example.com': 'gitlab' });
      pick('Forge type for code.example.com', 'Detected (Generic)');
      expect(useAppState.getState().profile.hostOverrides).toEqual({});

      fireEvent.change(screen.getByLabelText('Add a host'), { target: { value: 'https://Git.Corp.example/x' } });
      fireEvent.click(screen.getByRole('button', { name: 'Add' }));
      expect(useAppState.getState().profile.hostOverrides).toEqual({ 'git.corp.example': 'gitlab' });
    });

    it('the extra gitconfig path saves on Enter, and clears on an empty blur', () => {
      show('extraGitconfig');
      const gitconfig = screen.getByLabelText('Extra git config for this profile');
      fireEvent.change(gitconfig, { target: { value: ' /home/u/.gitconfig-work ' } });
      fireEvent.submit(gitconfig.closest('form')!);
      expect(useAppState.getState().profile.extraGitconfig).toBe('/home/u/.gitconfig-work');
      fireEvent.change(gitconfig, { target: { value: '' } });
      fireEvent.blur(gitconfig);
      expect(useAppState.getState().profile.extraGitconfig).toBeNull();
    });
  });

  describe('dropdown', () => {
    it('clicking the setting label does not open the dropdown (K103)', () => {
      const dialog = show('dateFormat');
      const btn = screen.getByRole('button', { name: 'Date format' });
      fireEvent.click(dialog.querySelector('#label-dateFormat')!);
      expect(btn).toHaveAttribute('aria-expanded', 'false');
      expect(screen.queryByRole('menu')).toBeNull();
    });

    it('opens on click with the current value checked, selects a row, and closes', () => {
      show('dateFormat');
      const btn = screen.getByRole('button', { name: 'Date format' });
      expect(btn).toHaveTextContent('2026-09-26 @ 3:14 PM');
      fireEvent.click(btn);
      expect(btn).toHaveAttribute('aria-expanded', 'true');
      expect(screen.getAllByRole('menuitem')).toHaveLength(4);
      fireEvent.click(screen.getByRole('menuitem', { name: '26/09/2026 15:14' }));
      expect(useAppState.getState().settings.dateFormat).toBe('dmy24h');
      expect(screen.queryByRole('menu')).toBeNull();
      expect(btn).toHaveTextContent('26/09/2026 15:14');
    });

    it('opens with ArrowDown, moves with the arrows, Enter selects; Esc closes only the menu and returns focus', () => {
      show('dateFormat');
      const btn = screen.getByRole('button', { name: 'Date format' });
      act(() => btn.focus());
      key(btn, 'ArrowDown');
      expect(screen.getByRole('menu')).toBeInTheDocument();
      key(screen.getByRole('menu'), 'Escape');
      expect(screen.queryByRole('menu')).toBeNull();
      expect(useSettingsUi.getState().open).toBe(true);
      expect(btn).toHaveFocus();
      key(btn, 'ArrowDown');
      key(screen.getByRole('menu'), 'ArrowDown');
      key(screen.getByRole('menu'), 'Enter');
      expect(useAppState.getState().settings.dateFormat).toBe('ymd24h');
      expect(btn).toHaveFocus();
    });
  });

  describe('accounts', () => {
    it('Accounts is a page of the profile, and deep links reach it', async () => {
      show('forgeAccounts');
      expect(screen.getByRole('region', { name: 'Accounts' })).toBeInTheDocument();
      expect(navItem('Accounts')).toHaveAttribute('aria-current', 'page');
      await waitFor(() => expect(api.forgeAccounts).toHaveBeenCalled());
    });

    it('Esc through the key router: host list, then the add panel, then Settings', async () => {
      useRuntime.setState({ tabs: { t1: { ...useRuntime.getState().tabs.t1, info: { remotes: [
        { name: 'origin', url: 'u', host: 'gitlab.example.com', path: 'g/p', hostKind: 'gitlab' },
        { name: 'upstream', url: 'u2', host: 'github.com', path: 'o/p', hostKind: 'github' },
      ] } as never } } as never });
      show('forgeAccounts');
      fireEvent.click(await screen.findByRole('button', { name: '+ Add account' }));
      const host = await screen.findByLabelText('Host');
      act(() => host.focus());
      await screen.findByRole('listbox');
      const esc = () => key(document.activeElement ?? document.body, 'Escape');
      esc();
      await waitFor(() => expect(screen.queryByRole('listbox')).toBeNull());
      expect(screen.getByRole('form', { name: 'Add a forge account' })).toBeTruthy();
      expect(screen.getByRole('dialog', { name: 'Settings' })).toBeTruthy();
      esc();
      await waitFor(() => expect(screen.queryByRole('form')).toBeNull());
      expect(screen.getByRole('dialog', { name: 'Settings' })).toBeTruthy();
      esc();
      await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Settings' })).toBeNull());
    });
  });
});
