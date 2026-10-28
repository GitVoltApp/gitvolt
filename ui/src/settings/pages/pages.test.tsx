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
    ['theme', 'appearance'], ['graphColors', 'appearance'], ['themeColours', 'appearance'], ['density', 'appearance'],
    ['uiFontFamily', 'appearance'], ['readingFontFamily', 'appearance'], ['readingFontSize', 'appearance'], ['readingLineHeight', 'appearance'], ['readingLineWidth', 'appearance'],
    ['commitLimit', 'graph'], ['relativeDates', 'graph'], ['dateFormat', 'graph'], ['gravatar', 'graph'], ['forgeAvatars', 'graph'], ['collapseHoverOpen', 'graph'], ['collapsePlacement', 'graph'],
    ['editorFontSize', 'diffs'], ['editorFontFamily', 'diffs'], ['editorFontLigatures', 'diffs'], ['editorLineHeight', 'diffs'], ['editorTabSize', 'diffs'], ['editorWhitespace', 'diffs'], ['editorMinimap', 'diffs'],
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
    // Density and the UI and reading fonts first; the editor font is on Diffs and editors.
    expect(groupTitles(dialog)).toEqual(['Density', 'Fonts', 'Theme', 'Graph lanes · drag to reorder, click to change', 'Colours']);
    expect(within(dialog).queryByLabelText(/^font size/i)).toBeNull();
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

  describe('Appearance › Fonts', () => {
    const commit = (dialog: HTMLElement, label: string, value: string) => {
      const box = within(dialog).getByLabelText(label);
      fireEvent.change(box, { target: { value } });
      fireEvent.submit(box.closest('form')!);
    };

    it("the UI font, then the reading font, its size, line height and line width, at today's values", () => {
      const dialog = show('uiFontFamily');
      const rows = [...dialog.querySelectorAll('.settings-page .settings-group')].find((g) => g.querySelector('h3')?.textContent === 'Fonts')!;
      expect([...rows.querySelectorAll('.setting-row')].map((r) => r.getAttribute('data-setting-id')))
        .toEqual(['uiFontFamily', 'readingFontFamily', 'readingFontSize', 'readingLineHeight', 'readingLineWidth']);
      expect(within(dialog).getByLabelText('UI font')).toHaveValue('');
      expect(within(dialog).getByLabelText('Reading font')).toHaveValue('');
      expect(within(dialog).getByLabelText('Reading text size')).toHaveValue(13);
      expect(within(dialog).getByLabelText('Reading line height')).toHaveValue(1.55);
      expect(within(dialog).getByLabelText('Reading line width')).toHaveValue(null); // empty: none
      expect(within(dialog).getByText(/Over it, Ctrl\+= and Ctrl\+- change it, Ctrl\+0 resets it/)).toBeVisible();
    });

    it('each control writes its app setting, clamped; an empty line width is none again', () => {
      const dialog = show('readingFontSize');
      commit(dialog, 'Reading text size', '50');
      commit(dialog, 'Reading line height', '1.83');
      commit(dialog, 'Reading line width', '72');
      commit(dialog, 'Reading font', "  'Noto Serif', serif ");
      commit(dialog, 'UI font', 'Inter');
      expect(useAppState.getState().settings).toMatchObject({ readingFontSize: 32, readingLineHeight: 1.83, readingLineWidth: 72, readingFontFamily: "'Noto Serif', serif", uiFontFamily: 'Inter' });
      commit(dialog, 'Reading line height', '9');
      commit(dialog, 'Reading line width', '5');
      expect(useAppState.getState().settings).toMatchObject({ readingLineHeight: 2.4, readingLineWidth: 20 });
      commit(dialog, 'Reading line width', '');
      expect(useAppState.getState().settings.readingLineWidth).toBe(0);
    });

    it("the families' previews show Vietnamese and Latin in what's typed, before it applies; the reading one at its size and line height", () => {
      const dialog = show('readingFontFamily');
      const ui = within(dialog).getByTestId('uiFontFamily-preview');
      const reading = within(dialog).getByTestId('readingFontFamily-preview');
      expect(reading).toHaveTextContent('Đảo chiều tối thiểu — The quick brown fox');
      expect(ui.style.fontFamily).toContain('system-ui');
      expect(reading.style.fontFamily).toBe('var(--font-ui)');
      expect(reading.style.fontSize).toBe('13px');
      expect(reading.style.lineHeight).toBe('1.55');
      fireEvent.change(within(dialog).getByLabelText('Reading font'), { target: { value: 'Literata' } });
      expect(reading.style.fontFamily).toMatch(/^Literata, sans-serif/);
      fireEvent.change(within(dialog).getByLabelText('UI font'), { target: { value: 'Inter, serif' } });
      expect(ui.style.fontFamily).toMatch(/^Inter, serif/);
      expect(useAppState.getState().settings).toMatchObject({ readingFontFamily: '', uiFontFamily: '' });
      act(() => useAppState.getState().setSettings({ readingFontSize: 18, readingLineHeight: 2 }));
      expect(reading.style.fontSize).toBe('18px');
      expect(reading.style.lineHeight).toBe('2');
    });

    it('the zoom keys over rendered Markdown update the reading size field live, and leave the editor size alone', async () => {
      const { installFontZoom } = await import('../../diff/fontZoom');
      const { installZoom } = await import('../../ui/zoom');
      const offs = [installZoom(() => {}), installFontZoom()];
      try {
        const dialog = show('readingFontSize');
        const pane = document.createElement('div');
        pane.setAttribute('data-font-zoom', 'reading');
        pane.tabIndex = -1;
        document.body.append(pane);
        const zoomKey = (key: string) => act(() => { pane.dispatchEvent(new KeyboardEvent('keydown', { key, ctrlKey: true, bubbles: true, cancelable: true })); });
        zoomKey('=');
        zoomKey('=');
        expect(within(dialog).getByLabelText('Reading text size')).toHaveValue(15);
        expect(useAppState.getState().settings.editorFontSize).toBe(13);
        zoomKey('0');
        expect(within(dialog).getByLabelText('Reading text size')).toHaveValue(13);
        pane.remove();
      } finally {
        offs.forEach((off) => off());
      }
    });
  });

  describe('Diffs and editors', () => {
    it('one Editor group: the font, its size and ligatures, line height, tab width, whitespace, minimap and sticky scroll', () => {
      const dialog = show('editorFontSize');
      expect(groupTitles(dialog)).toEqual(['Editor']);
      expect([...dialog.querySelectorAll('.settings-page .setting-row')].map((r) => r.getAttribute('data-setting-id')))
        .toEqual(['editorFontSize', 'editorFontFamily', 'editorFontLigatures', 'editorLineHeight', 'editorTabSize', 'editorWhitespace', 'editorMinimap', 'stickyScroll']);
      expect(within(dialog).getByText(/Ctrl\+= and Ctrl\+- change it, Ctrl\+0 resets it/)).toBeVisible();
      expect(within(dialog).getByLabelText('Font size')).toHaveValue(13);
      expect(within(dialog).getByLabelText('Tab width')).toHaveValue(4);
      expect(within(dialog).getByLabelText('Line height')).toHaveValue(null); // empty: automatic
      expect(within(dialog).getByLabelText('Minimap')).toBeChecked();
      expect(within(dialog).getByLabelText('Font ligatures')).not.toBeChecked();
      expect(within(dialog).getByRole('button', { name: 'Show whitespace' })).toHaveTextContent('None');
    });

    it('each control writes its app setting, clamped', () => {
      const dialog = show('editorFontSize');
      const commit = (label: string, value: string) => {
        const box = within(dialog).getByLabelText(label);
        fireEvent.change(box, { target: { value } });
        fireEvent.submit(box.closest('form')!);
      };
      commit('Tab width', '40');
      commit('Line height', '3');
      commit('Font family', "  'Fira Code'  ");
      fireEvent.click(within(dialog).getByLabelText('Font ligatures'));
      fireEvent.click(within(dialog).getByLabelText('Minimap'));
      fireEvent.click(within(dialog).getByRole('button', { name: 'Show whitespace' }));
      fireEvent.click(screen.getByRole('menuitem', { name: 'At boundaries' }));
      expect(useAppState.getState().settings).toMatchObject({ editorTabSize: 16, editorLineHeight: 8, editorFontFamily: "'Fira Code'", editorFontLigatures: true, editorMinimap: false, editorWhitespace: 'boundary' });
      // An empty line height is automatic again.
      commit('Line height', '');
      expect(useAppState.getState().settings.editorLineHeight).toBe(0);
    });

    it("the font family's preview follows what's typed, before it applies, with monospace after a list that names no generic family", () => {
      const dialog = show('editorFontFamily');
      const preview = within(dialog).getByTestId('font-preview');
      expect(preview).toHaveTextContent('const tối = "thiểu"; // 0O1lI {}');
      expect(preview.style.fontFamily).toContain('JetBrains Mono');
      expect(preview.style.fontVariantLigatures).toBe('none');
      fireEvent.change(within(dialog).getByLabelText('Font family'), { target: { value: 'Iosevka' } });
      expect(preview.style.fontFamily).toBe('Iosevka, monospace');
      expect(useAppState.getState().settings.editorFontFamily).toBe('');
      act(() => useAppState.getState().setSettings({ editorFontLigatures: true }));
      expect(preview.style.fontVariantLigatures).toBe('normal');
    });

    it('the zoom keys over an editor update the font size field live', async () => {
      const { installFontZoom } = await import('../../diff/fontZoom');
      const { installZoom } = await import('../../ui/zoom');
      const offs = [installZoom(() => {}), installFontZoom()];
      try {
        const dialog = show('editorFontSize');
        const pane = document.createElement('div');
        pane.setAttribute('data-font-zoom', '');
        pane.tabIndex = -1;
        document.body.append(pane);
        act(() => { pane.dispatchEvent(new KeyboardEvent('keydown', { key: '=', ctrlKey: true, bubbles: true, cancelable: true })); });
        expect(useAppState.getState().settings.editorFontSize).toBe(14);
        expect(within(dialog).getByLabelText('Font size')).toHaveValue(14);
        act(() => { pane.dispatchEvent(new KeyboardEvent('keydown', { key: '0', ctrlKey: true, bubbles: true, cancelable: true })); });
        expect(within(dialog).getByLabelText('Font size')).toHaveValue(13);
        pane.remove();
      } finally {
        for (const off of offs) off();
      }
    });
  });

  it('Keyboard lists the shortcuts', () => {
    show('shortcuts');
    const row = screen.getAllByRole('listitem').find((li) => li.querySelector('.sc-label')?.textContent === 'Settings')!;
    expect(row).toBeTruthy();
    expect(within(row).getByText(',')).toBeInTheDocument();
  });
});
