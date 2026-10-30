import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const FAMILIES = [
  { name: 'Atlas Sans', monospace: false },
  { name: 'JetBrains Mono', monospace: true }, // also installed: listed once, as the bundled one
  { name: 'Orbit Mono', monospace: true },
  { name: 'Zeta Serif', monospace: false },
];
const fontFamilies = vi.fn(async (_refresh?: boolean) => FAMILIES);
vi.mock('../../api/client', () => ({
  api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null), listOpeners: vi.fn(async () => []), fontFamilies: (refresh?: boolean) => fontFamilies(refresh) },
  errorMessage: String,
}));

await import('../feature');
const { DEFAULT_SETTINGS, EMPTY_PROFILE, useAppState } = await import('../../app/state');
const { useSettingsUi } = await import('../schema');
const { SettingsOverlay } = await import('./SettingsOverlay');
const { useFontFamilies } = await import('../../theme/fontList');
const { fontChoice, fontItems, placeList } = await import('./FontPicker');
const { EDITOR_FONT_ROLE } = await import('../pages/diffs');
const { mouseDown, mouseUp } = await import('../../menu/pressTesting');

async function show(focus: string) {
  render(<SettingsOverlay />);
  act(() => useSettingsUi.getState().show(focus));
  await waitFor(() => expect(useFontFamilies.getState().list).not.toBeNull());
  return screen.getByRole('dialog', { name: 'Settings' });
}
const button = (label = 'Font family') => screen.getByLabelText(label);
const search = () => screen.getByRole('combobox', { name: 'Search fonts' });
const options = () => screen.getAllByRole('option').map((o) => o.textContent);
const selected = () => screen.getAllByRole('option').find((o) => o.getAttribute('aria-selected') === 'true')?.textContent;
const key = (k: string, init: KeyboardEventInit = {}) => fireEvent.keyDown(document.activeElement ?? document.body, { key: k, ...init });
const preview = () => screen.getByTestId('editorFontFamily-preview');
const editorFont = () => useAppState.getState().settings.editorFontFamily;

beforeEach(() => {
  fontFamilies.mockClear();
  useFontFamilies.setState({ list: null, loading: false });
  useAppState.setState({ settings: { ...DEFAULT_SETTINGS }, profile: { ...EMPTY_PROFILE, name: 'Work' } });
});
afterEach(() => act(() => { useSettingsUi.getState().close(); useSettingsUi.setState({ page: 'general', query: '' }); }));

describe('the stored value', () => {
  it('is the default, one family (installed or not: case-insensitive, the list spelling), or a custom list', () => {
    expect(fontChoice('', FAMILIES)).toEqual({ kind: 'default' });
    expect(fontChoice('orbit mono', FAMILIES)).toEqual({ kind: 'family', name: 'Orbit Mono', installed: true });
    expect(fontChoice('"Orbit Mono"', FAMILIES)).toEqual({ kind: 'family', name: 'Orbit Mono', installed: true });
    expect(fontChoice('JetBrains Mono', [])).toEqual({ kind: 'family', name: 'JetBrains Mono', installed: true });
    expect(fontChoice('Ghost Mono', FAMILIES)).toEqual({ kind: 'family', name: 'Ghost Mono', installed: false });
    // Before the list loads, nothing is "not installed" yet.
    expect(fontChoice('Ghost Mono', null)).toEqual({ kind: 'family', name: 'Ghost Mono', installed: true });
    expect(fontChoice("'Fira Code', monospace", FAMILIES)).toEqual({ kind: 'custom' });
    expect(fontChoice('monospace', FAMILIES)).toEqual({ kind: 'custom' });
  });

  it("lists Default, the bundled font, a set font that isn't installed, the families, then Custom font list…", () => {
    const labels = (value: string, all: boolean, q = '') => fontItems(EDITOR_FONT_ROLE, value, FAMILIES, all, q).map((i) => `${i.label}${i.detail ? ` ${i.detail}` : ''}=${i.value}`);
    expect(labels('', false)).toEqual(['Default (JetBrains Mono)=', 'JetBrains Mono bundled=JetBrains Mono', 'Orbit Mono=Orbit Mono', "Custom font list…="]);
    expect(labels('', true)).toEqual(['Default (JetBrains Mono)=', 'JetBrains Mono bundled=JetBrains Mono', 'Atlas Sans=Atlas Sans', 'Orbit Mono=Orbit Mono', 'Zeta Serif=Zeta Serif', 'Custom font list…=']);
    // The set family stays listed when the monospace filter would hide it.
    expect(labels('Zeta Serif', false)).toContain('Zeta Serif=Zeta Serif');
    expect(labels('Ghost Mono', false)).toContain('Ghost Mono not installed=Ghost Mono');
    expect(labels('', true, 'SERIF')).toEqual(['Zeta Serif=Zeta Serif', 'Custom font list…=']);
  });
});

describe('the list placement', () => {
  const box = (left: number, top: number, right: number, bottom: number) => ({ left, top, right, bottom });
  const button = box(600, 100, 900, 128);
  it('opens under the button at its width, beside a preview in the left column', () => {
    expect(placeList(button, 60, 312, box(20, 140, 580, 400), box(0, 0, 1000, 900))).toEqual({ left: 600, top: 132, width: 300, view: 312, below: true });
  });

  it('opens above with no room below, unless that covers the preview; then it shortens below', () => {
    const low = box(600, 700, 900, 728);
    expect(placeList(low, 60, 312, box(20, 740, 580, 900), box(0, 0, 1000, 900))).toMatchObject({ top: 324, view: 312, below: false });
    // The preview spans the row above the button (a narrow dialog): the list stays under it, shorter.
    expect(placeList(low, 60, 312, box(20, 400, 900, 690), box(0, 0, 1000, 900))).toMatchObject({ top: 732, view: 100, below: true });
    // Never fewer than one row.
    expect(placeList(box(600, 850, 900, 878), 60, 312, box(20, 400, 900, 840), box(0, 0, 1000, 900)).view).toBe(26);
  });
});

describe('the font picker', () => {
  it('opens on the current choice, each family drawn in its own font with the fallbacks after it; the toggle lists all fonts', async () => {
    await show('editorFontFamily');
    expect(button()).toHaveTextContent('Default(JetBrains Mono)');
    expect(fontFamilies).toHaveBeenCalledTimes(1);
    fireEvent.click(button());
    expect(button()).toHaveAttribute('aria-expanded', 'true');
    expect(search()).toHaveFocus();
    expect(options()).toEqual(['Default(JetBrains Mono)', 'JetBrains Monobundled', 'Orbit Mono', 'Custom font list…']);
    expect(selected()).toBe('Default(JetBrains Mono)');
    const orbit = screen.getByRole('option', { name: 'Orbit Mono' });
    expect(orbit.querySelector<HTMLElement>('.font-picker-name')!.style.fontFamily).toBe('"Orbit Mono", "JetBrains Mono", "Noto Sans Mono", "Ubuntu Sans Mono", "Liberation Mono", monospace');
    fireEvent.click(screen.getByLabelText('Show all fonts'));
    expect(options()).toEqual(['Default(JetBrains Mono)', 'JetBrains Monobundled', 'Atlas Sans', 'Orbit Mono', 'Zeta Serif', 'Custom font list…']);
  });

  it('typing filters, ↑/↓ move, Enter picks and stores the family alone; the button shows it in its font', async () => {
    await show('editorFontFamily');
    fireEvent.click(button());
    fireEvent.change(search(), { target: { value: 'mono' } });
    expect(options()).toEqual(['Default(JetBrains Mono)', 'JetBrains Monobundled', 'Orbit Mono', 'Custom font list…']);
    fireEvent.change(search(), { target: { value: 'orb' } });
    expect(options()).toEqual(['Orbit Mono', 'Custom font list…']);
    expect(selected()).toBe('Orbit Mono');
    key('ArrowDown');
    expect(selected()).toBe('Custom font list…');
    key('ArrowUp');
    key('Enter');
    expect(editorFont()).toBe('Orbit Mono');
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(button()).toHaveFocus();
    expect(button()).toHaveTextContent('Orbit Mono');
    expect(button().querySelector<HTMLElement>('.font-picker-name')!.style.fontFamily).toMatch(/^"Orbit Mono", "JetBrains Mono"/);
    // The editor setting applies with the default's fallbacks after the family.
    expect(preview().style.fontFamily).toMatch(/^"Orbit Mono", "JetBrains Mono", "Noto Sans Mono"/);
  });

  it('↓ or a typed letter on the button opens it, the letter starting the search', async () => {
    await show('editorFontFamily');
    act(() => button().focus());
    key('o');
    expect(search()).toHaveValue('o');
    expect(options()).toContain('Orbit Mono');
    key('Escape');
    expect(screen.queryByRole('listbox')).toBeNull();
    key('ArrowDown');
    expect(screen.getByRole('listbox')).toBeInTheDocument();
  });

  it('the family under the pointer or the arrows previews; closing without picking restores the panel', async () => {
    await show('editorFontFamily');
    const before = preview().style.fontFamily;
    fireEvent.click(button());
    fireEvent.pointerEnter(screen.getByRole('option', { name: 'Orbit Mono' }));
    expect(preview().style.fontFamily).toMatch(/^"Orbit Mono"/);
    key('ArrowUp');
    expect(preview().style.fontFamily).toMatch(/^"JetBrains Mono"/);
    key('ArrowDown');
    expect(preview().style.fontFamily).toMatch(/^"Orbit Mono"/);
    // Esc closes the picker only, not Settings, and the panel shows the setting again.
    key('Escape');
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
    expect(preview().style.fontFamily).toBe(before);
    expect(editorFont()).toBe('');
    expect(button()).toHaveFocus();
  });

  it('a second click on the button closes it (the press closes it, its click must not reopen it)', async () => {
    await show('editorFontFamily');
    fireEvent.click(button());
    expect(screen.getByRole('listbox')).toBeInTheDocument();
    fireEvent.pointerDown(button());
    fireEvent.pointerUp(button());
    fireEvent.click(button());
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(button()).toHaveAttribute('aria-expanded', 'false');
    // A press anywhere else closes it too.
    fireEvent.click(button());
    fireEvent.pointerDown(screen.getByRole('dialog', { name: 'Settings' }));
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('press, drag onto a font (it previews on the way), release: it is picked; released on the button it stays open', async () => {
    await show('editorFontFamily');
    mouseDown(button());
    expect(screen.getByRole('listbox')).toBeInTheDocument();
    expect(search()).toHaveFocus();
    mouseUp(button(), 11, 10, button());
    expect(screen.getByRole('listbox')).toBeInTheDocument();
    const orbit = screen.getByRole('option', { name: 'Orbit Mono' });
    fireEvent.pointerEnter(orbit);
    expect(preview().style.fontFamily).toMatch(/^"Orbit Mono"/);
    // A new press on the button closes it (the toggle); the next opens it again.
    mouseDown(button());
    mouseUp(button(), 10, 10, button());
    expect(screen.queryByRole('listbox')).toBeNull();
    await new Promise((r) => setTimeout(r, 0));
    mouseDown(button());
    const row = screen.getByRole('option', { name: 'Orbit Mono' });
    fireEvent.pointerEnter(row);
    mouseUp(row, 60, 120, document.body);
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(editorFont()).toMatch(/Orbit Mono/);
  });

  it('Default stores nothing; a set font that is not installed shows so, and the fallbacks apply', async () => {
    useAppState.getState().setSettings({ editorFontFamily: 'Ghost Mono' });
    await show('editorFontFamily');
    expect(button()).toHaveTextContent('Ghost Mononot installed');
    fireEvent.click(button());
    expect(options()).toContain('Ghost Mononot installed');
    expect(selected()).toBe('Ghost Mononot installed');
    fireEvent.click(screen.getByRole('option', { name: /^Default/ }));
    expect(editorFont()).toBe('');
    expect(button()).toHaveTextContent('Default(JetBrains Mono)');
  });

  it('Custom font list… reveals a box for a CSS list, which shows as Custom', async () => {
    await show('editorFontFamily');
    expect(screen.queryByLabelText('Custom font list')).toBeNull();
    fireEvent.click(button());
    fireEvent.click(screen.getByRole('option', { name: 'Custom font list…' }));
    const box = screen.getByLabelText('Custom font list');
    expect(box).toHaveFocus();
    fireEvent.change(box, { target: { value: 'Iosevka, monospace' } });
    // What's typed previews before it applies.
    expect(preview().style.fontFamily).toBe('Iosevka, monospace');
    fireEvent.submit(box.closest('form')!);
    expect(editorFont()).toBe('Iosevka, monospace');
    expect(button()).toHaveTextContent('CustomIosevka, monospace');
    // Picking a family hides the box again; so does a reset to the default.
    fireEvent.click(button());
    fireEvent.click(screen.getByRole('option', { name: 'Orbit Mono' }));
    expect(editorFont()).toBe('Orbit Mono');
    expect(screen.queryByLabelText('Custom font list')).toBeNull();
  });

  it('the UI and reading pickers list every family, with the UI stack after one', async () => {
    const dialog = await show('uiFontFamily');
    fireEvent.click(button('UI font'));
    expect(options()).toEqual(['Default(system UI font)', 'JetBrains Monobundled', 'Atlas Sans', 'Orbit Mono', 'Zeta Serif', 'Custom font list…']);
    expect(screen.queryByLabelText('Show all fonts')).toBeNull();
    fireEvent.click(screen.getByRole('option', { name: 'Atlas Sans' }));
    expect(useAppState.getState().settings.uiFontFamily).toBe('Atlas Sans');
    expect(within(dialog).getByTestId('uiFontFamily-preview').style.fontFamily).toMatch(/^"Atlas Sans", system-ui, "Segoe UI"/);
    fireEvent.click(button('Reading font'));
    expect(options()[0]).toBe('Default(the UI font)');
    fireEvent.click(screen.getByRole('option', { name: 'Zeta Serif' }));
    expect(useAppState.getState().settings.readingFontFamily).toBe('Zeta Serif');
  });

  it('a long list renders only the rows in view, and still finds any family', async () => {
    const many = Array.from({ length: 400 }, (_, i) => ({ name: `Family ${String(i).padStart(3, '0')}`, monospace: true }));
    fontFamilies.mockResolvedValueOnce(many);
    await show('editorFontFamily');
    fireEvent.click(button());
    expect(screen.getAllByRole('option').length).toBeLessThan(40);
    fireEvent.change(search(), { target: { value: 'family 377' } });
    expect(options()).toEqual(['Family 377', 'Custom font list…']);
    fireEvent.change(search(), { target: { value: '' } });
    key('PageDown');
    key('PageDown');
    expect(selected()).toBe('Family 018');
  });

  it('each preview is a captioned picture: no focusable or interactive parts, none shaped like a control', async () => {
    const dialog = await show('uiFontFamily');
    for (const name of ['UI', 'reading']) {
      const box = within(dialog).getByRole('img', { name: `Preview of the ${name} font` });
      expect(box.querySelector('.font-preview-caption')).toHaveTextContent('Preview');
      expect(box.querySelectorAll('button, a, input, kbd, [tabindex], [role]')).toHaveLength(0);
      expect(box.querySelector('.font-preview')).toHaveAttribute('aria-hidden', 'true');
    }
    const ui = within(dialog).getByTestId('uiFontFamily-preview');
    expect(ui).not.toHaveTextContent(/menu|button|commit|Ctrl|⌘/i);
    act(() => useSettingsUi.getState().setPage('diffs'));
    const editor = within(screen.getByRole('dialog', { name: 'Settings' })).getByRole('img', { name: 'Preview of the editor font' });
    expect(editor.querySelectorAll('button, a, input, [tabindex], [role]')).toHaveLength(0);
    // Hovering a family previews it in the panel the list keeps clear of.
    fireEvent.click(button());
    fireEvent.pointerEnter(screen.getByRole('option', { name: 'Orbit Mono' }));
    expect(preview().style.fontFamily).toMatch(/^"Orbit Mono"/);
  });

  it('"List the fonts again" asks the core to read the fonts again', async () => {
    await show('editorFontFamily');
    fireEvent.click(button());
    fireEvent.click(screen.getByRole('button', { name: 'List the fonts again' }));
    await waitFor(() => expect(fontFamilies).toHaveBeenLastCalledWith(true));
  });
});
