import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
  saveSettings: vi.fn(async () => null),
  saveProfile: vi.fn(async () => null),
  saveExport: vi.fn(async (_name: string, _contents: string): Promise<string | null> => '/home/ada/themes/nord-custom.gitvolt-theme.json'),
  forgeAccounts: vi.fn(async () => []),
  listOpeners: vi.fn(async () => []),
}));
vi.mock('../../api/client', () => ({ api, errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)) }));
const transport = vi.hoisted(() => ({ copyText: vi.fn(async (_t: string) => {}), inTauri: () => false, readClipboardText: vi.fn(async () => '') }));
vi.mock('../../api/transport', () => transport);

await import('../../settings/pages');
const { ThemeCards } = await import('./ThemeCards');
const { EditingStrip } = await import('./EditingStrip');
const { ThemeSelect } = await import('./ThemeSelect');
const { SettingsOverlay } = await import('../../settings/frame/SettingsOverlay');
const { registerSettingsFloat } = await import('../../settings/frame/floating');
const { useSettingsUi } = await import('../../settings/schema');
const { DEFAULT_SETTINGS, useAppState } = await import('../../app/state');
const { ContextMenu } = await import('../../menu/ContextMenu');
const { useMenu } = await import('../../menu/menuStore');
const { ConfirmDialog } = await import('../../ui/ConfirmDialog');
const { ArmLayer } = await import('../../ui/arm/ArmLayer');
const { disarm } = await import('../../ui/arm/store');
const { armClock, press } = await import('../../ui/arm/armTesting');
const { useToast } = await import('../../ui/toastStore');
const { themeFromJson } = await import('../themeJson');
const { THEME_IDS, THEMES } = await import('../themes');

const settings = () => useAppState.getState().settings;
const current = () => settings().customThemes.find((t) => t.id === settings().theme);
const mine = { id: 'custom-0a1b2c3d', name: 'Mine', base: 'light', colors: {}, graph: [] };
const nordCustom = { id: 'custom-1a2b3c4d', name: 'Nord (Custom)', base: 'nord', colors: { 'app-bg0': '#202020' }, graph: [] };
const use = (patch: Partial<typeof DEFAULT_SETTINGS>) => useAppState.setState({ settings: { ...DEFAULT_SETTINGS, customThemes: [], ...patch } });
const radio = (name: string) => screen.getByRole('radio', { name });

beforeEach(() => {
  vi.clearAllMocks();
  Element.prototype.scrollIntoView = vi.fn();
  use({ theme: 'default-dark' });
});
afterEach(() => act(() => { useMenu.getState().close(); useSettingsUi.getState().close(); useToast.getState().dismiss(); }));

describe('theme cards', () => {
  it('shows every built-in, then the custom themes (marked), with the current one checked', () => {
    use({ theme: mine.id, customThemes: [mine] });
    render(<ThemeCards />);
    const cards = screen.getAllByRole('radio');
    expect(cards.map((c) => c.getAttribute('aria-label'))).toEqual([...THEME_IDS.map((id) => THEMES[id].label), 'Mine']);
    expect(radio('Mine')).toHaveAttribute('aria-checked', 'true');
    expect(radio('Mine')).toHaveTextContent('✎');
    expect(radio('Nord')).not.toHaveTextContent('✎');
    expect(screen.getByRole('button', { name: /Paste a theme/ })).toBeInTheDocument();
  });

  it('arrows move the focus, Space or Enter selects, a click selects', () => {
    render(<ThemeCards />);
    const first = radio('Default Dark');
    expect(first).toHaveAttribute('tabindex', '0');
    act(() => first.focus());
    fireEvent.keyDown(first, { key: 'ArrowRight' });
    const second = screen.getAllByRole('radio')[1]!;
    expect(second).toHaveFocus();
    expect(settings().theme).toBe('default-dark');
    fireEvent.keyDown(second, { key: ' ' });
    expect(settings().theme).toBe(second.dataset.themeId);
    fireEvent.keyDown(second, { key: 'End' });
    const last = screen.getAllByRole('radio').at(-1)!;
    expect(last).toHaveFocus();
    fireEvent.keyDown(last, { key: 'Enter' });
    expect(settings().theme).toBe(last.dataset.themeId);
    fireEvent.click(radio('Nord'));
    expect(settings().theme).toBe('nord');
  });

  it('shows an unknown saved theme as Default Dark', () => {
    use({ theme: 'no-such-theme' });
    render(<ThemeCards />);
    expect(radio('Default Dark')).toHaveAttribute('aria-checked', 'true');
  });
});

describe('the editing strip', () => {
  it('for a built-in: the hint and Edit mode, no name or theme actions', () => {
    use({ theme: 'nord' });
    const off = registerSettingsFloat('theme-editor', () => null);
    render(<EditingStrip />);
    expect(screen.getByText('Change any colour below to make your own copy.')).toBeInTheDocument();
    expect(screen.queryByLabelText('Theme name')).toBeNull();
    expect(screen.getAllByRole('button').map((b) => b.textContent)).toEqual(['Edit mode']);
    fireEvent.click(screen.getByRole('button', { name: 'Edit mode' }));
    expect(useSettingsUi.getState().floating).toBe('theme-editor');
    act(() => { useSettingsUi.getState().setFloating(null); off(); });
  });

  it('for a custom theme: its name, which renames it on Enter or blur', () => {
    use({ theme: nordCustom.id, customThemes: [nordCustom, mine] });
    render(<EditingStrip />);
    const name = screen.getByLabelText('Theme name') as HTMLInputElement;
    expect(name.value).toBe('Nord (Custom)');
    expect(screen.getByRole('group', { name: 'Editing Nord (Custom)' })).toHaveTextContent('made from Nord; your other themes are untouched.');
    fireEvent.change(name, { target: { value: 'Night watch' } });
    fireEvent.submit(name.form!);
    expect(current()?.name).toBe('Night watch');
    fireEvent.change(name, { target: { value: 'mine' } });
    fireEvent.blur(name);
    expect(current()?.name).toBe('mine (2)');
    expect(name.value).toBe('mine (2)');
    fireEvent.change(name, { target: { value: '  ' } });
    fireEvent.submit(name.form!);
    expect(name.value).toBe('mine (2)');
  });

  it('Delete arms in place, and the second click deletes the theme and selects its base', async () => {
    use({ theme: nordCustom.id, customThemes: [nordCustom] });
    const clock = armClock();
    // happy-dom does no layout: a connected control reports one box, so it's "shown" and arms in place.
    const rects = vi.spyOn(HTMLElement.prototype, 'getClientRects').mockImplementation(function (this: HTMLElement) {
      return (this.isConnected ? [new DOMRect(10, 10, 24, 24)] : []) as unknown as DOMRectList;
    });
    try {
      render(<><div data-setting-id="theme"><ThemeCards /><EditingStrip /></div><ConfirmDialog /><ArmLayer /></>);
      act(() => press(screen.getByRole('button', { name: 'Delete' })));
      const overlay = () => document.querySelector<HTMLElement>('[data-arm-overlay]');
      expect(overlay()).toHaveTextContent('Click again to delete Nord (Custom)');
      expect(settings().customThemes).toHaveLength(1);
      clock.settle();
      await act(async () => { press(overlay()!); });
      await waitFor(() => expect(settings().customThemes).toEqual([]));
      expect(settings().theme).toBe('nord');
      expect(radio('Nord')).toHaveAttribute('aria-checked', 'true');
      expect(radio('Nord')).toHaveFocus();
    } finally {
      act(() => disarm());
      rects.mockRestore();
      clock.restore();
    }
  });
});

describe('sharing', () => {
  it('Copy as JSON writes JSON that reads back as the theme', async () => {
    use({ theme: nordCustom.id, customThemes: [nordCustom] });
    render(<EditingStrip />);
    fireEvent.click(screen.getByRole('button', { name: 'Copy as JSON' }));
    await waitFor(() => expect(transport.copyText).toHaveBeenCalledTimes(1));
    const back = themeFromJson(transport.copyText.mock.calls[0]![0], []);
    expect(back).toMatchObject({ name: 'Nord (Custom)', base: 'nord', colors: { 'app-bg0': '#202020' } });
    expect(useToast.getState().message).toBe('Copied Nord (Custom) as JSON');
  });

  it('Export file… saves "<name>.gitvolt-theme.json" where the user picks; a cancel says nothing', async () => {
    use({ theme: nordCustom.id, customThemes: [nordCustom] });
    render(<EditingStrip />);
    fireEvent.click(screen.getByRole('button', { name: 'Export file…' }));
    await waitFor(() => expect(useToast.getState().message).toBe('Saved nord-custom.gitvolt-theme.json'));
    expect(api.saveExport).toHaveBeenCalledWith('nord-custom.gitvolt-theme.json', expect.stringContaining('"gitvolt": "theme"'));
    expect(useToast.getState().detail).toBe('/home/ada/themes/nord-custom.gitvolt-theme.json');
    act(() => useToast.getState().dismiss());
    api.saveExport.mockResolvedValueOnce(null);
    fireEvent.click(screen.getByRole('button', { name: 'Export file…' }));
    await act(async () => {});
    expect(api.saveExport).toHaveBeenCalledTimes(2);
    expect(useToast.getState().message).toBeNull();
  });

  function showThemePage() {
    render(<><SettingsOverlay /><ConfirmDialog /><ContextMenu /></>);
    act(() => useSettingsUi.getState().show('theme'));
    return screen.getByRole('dialog', { name: 'Settings' });
  }
  const ctrlV = (el: Element) => fireEvent.keyDown(el, { key: 'v', code: 'KeyV', ctrlKey: true });

  it('Ctrl+V on the page adds a pasted theme under a free name, and selects it', async () => {
    use({ theme: nordCustom.id, customThemes: [nordCustom] });
    const dialog = showThemePage();
    expect(within(dialog).getByRole('radio', { name: 'Nord (Custom)' })).toHaveFocus();
    transport.readClipboardText.mockResolvedValueOnce(JSON.stringify({ gitvolt: 'theme', version: 1, name: 'Nord (Custom)', base: 'nord', colors: { 'app-bg0': '#101010' }, graph: [] }));
    ctrlV(document.activeElement!);
    await waitFor(() => expect(current()?.name).toBe('Nord (Custom) (2)'));
    expect(current()?.colors).toEqual({ 'app-bg0': '#101010' });
    expect(useToast.getState().message).toBe('Added Nord (Custom) (2)');
    expect(within(dialog).getByRole('radio', { name: 'Nord (Custom) (2)' })).toHaveAttribute('aria-checked', 'true');
  });

  it("Ctrl+V with something that isn't a theme says so and changes nothing", async () => {
    use({ theme: 'nord' });
    const dialog = showThemePage();
    const before = settings();
    transport.readClipboardText.mockResolvedValueOnce('hello');
    ctrlV(within(dialog).getByRole('radio', { name: 'Nord' }));
    await waitFor(() => expect(useToast.getState().message).toBe("The clipboard doesn't hold a GitVolt theme"));
    expect(settings()).toBe(before);
  });

  it('the Paste a theme card pastes from the clipboard', async () => {
    transport.readClipboardText.mockResolvedValueOnce(JSON.stringify({ gitvolt: 'theme', version: 1, name: 'Dusk', base: 'dracula', colors: {}, graph: [] }));
    render(<ThemeCards />);
    fireEvent.click(screen.getByRole('button', { name: /Paste a theme/ }));
    await waitFor(() => expect(current()?.name).toBe('Dusk'));
  });
});

describe('the theme dropdown (search results)', () => {
  it("lists every theme, built-ins then custom ones, in the app's own dropdown, and saves the chosen one", () => {
    use({ theme: 'default-dark', customThemes: [mine] });
    render(<><span id="label-theme">Theme</span><ThemeSelect /><ContextMenu /></>);
    const button = screen.getByRole('button', { name: 'Theme' });
    expect(button).toHaveTextContent('Default Dark');
    expect(document.querySelector('select')).toBeNull();
    fireEvent.click(button);
    expect(screen.getAllByRole('menuitem').map((r) => r.textContent)).toEqual([...THEME_IDS.map((id) => THEMES[id].label), 'Mine']);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Mine' }));
    expect(settings().theme).toBe(mine.id);
  });
});
