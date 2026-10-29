import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../api/client', () => ({
  api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null), forgeAccounts: vi.fn(async () => []), listOpeners: vi.fn(async () => []) },
  errorMessage: String,
}));

await import('../../settings/pages');
await import('../feature');
const { SettingsOverlay } = await import('../../settings/frame/SettingsOverlay');
const { useSettingsUi } = await import('../../settings/schema');
const { DEFAULT_SETTINGS, useAppState } = await import('../../app/state');
const { useTheme } = await import('../store');
const { THEMES } = await import('../themes');

const mine = { id: 'custom-0a1b2c3d', name: 'Nord (Custom)', base: 'nord', colors: {}, graph: [] };
const panel = () => screen.getByRole('dialog', { name: 'Nord (Custom)' });
const bar = () => screen.getByLabelText('Move the Nord (Custom) editor');
/** The panel's drag offset, from its `transform`. */
function offset(): { x: number; y: number } {
  const m = /translate\((-?\d+)px, (-?\d+)px\)/.exec(panel().style.transform);
  return m ? { x: Number(m[1]), y: Number(m[2]) } : { x: 0, y: 0 };
}

/** The app behind (a button), Settings on Theme, then Edit mode. happy-dom has no
 * layout: the panel's box is its CSS place (320 × 560 at the lower right of 1024 × 768), moved by its offset. */
function openFloating(behindClick = vi.fn()) {
  render(<><button type="button" onClick={behindClick}>Behind</button><SettingsOverlay /></>);
  screen.getByRole('button', { name: 'Behind' }).focus();
  act(() => useSettingsUi.getState().show('theme'));
  fireEvent.click(screen.getByRole('button', { name: 'Edit mode' }));
  vi.spyOn(panel(), 'getBoundingClientRect').mockImplementation(() => {
    const { x, y } = offset();
    return new DOMRect(1024 - 24 - 320 + x, 768 - 24 - 560 + y, 320, 560);
  });
  return behindClick;
}

beforeEach(() => {
  Element.prototype.scrollIntoView = vi.fn();
  window.innerWidth = 1024;
  window.innerHeight = 768;
  useAppState.setState({ settings: { ...DEFAULT_SETTINGS, theme: mine.id, customThemes: [mine] } });
  act(() => useTheme.getState().set(THEMES.nord));
});
afterEach(() => act(() => useSettingsUi.getState().close()));

describe('the floating theme editor', () => {
  it('takes the overlay\'s place with no backdrop, not modal, focused on Find a colour', () => {
    const { container } = render(<SettingsOverlay />);
    act(() => useSettingsUi.getState().show('theme'));
    fireEvent.click(screen.getByRole('button', { name: 'Edit mode' }));
    expect(screen.queryByRole('dialog', { name: 'Settings' })).toBeNull();
    expect(panel()).toHaveAttribute('aria-modal', 'false');
    expect(container.querySelector('.modal-backdrop')).toBeNull();
    expect(document.activeElement).toBe(screen.getByLabelText('Find a colour'));
    expect(screen.getByRole('group', { name: 'Graph lanes' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /^All colours/ })).toHaveAttribute('aria-expanded', 'false');
  });

  it('a click on the app behind reaches it and leaves the panel open', () => {
    const behind = openFloating();
    const target = screen.getByRole('button', { name: 'Behind' });
    fireEvent.pointerDown(target);
    fireEvent.click(target);
    expect(behind).toHaveBeenCalledOnce();
    expect(panel()).toBeInTheDocument();
    expect(useSettingsUi.getState().floating).toBe('theme-editor');
  });

  it('Find a colour filters the colours; "pan" shows Panels and hides the lanes', () => {
    openFloating();
    fireEvent.change(screen.getByLabelText('Find a colour'), { target: { value: 'pan' } });
    expect(screen.getByRole('textbox', { name: 'Panels' })).toBeInTheDocument();
    expect(screen.queryByRole('textbox', { name: 'Window background' })).toBeNull();
    expect(screen.queryByRole('group', { name: 'Graph lanes' })).toBeNull();
    fireEvent.change(screen.getByLabelText('Find a colour'), { target: { value: 'lanes' } });
    expect(screen.getByRole('group', { name: 'Graph lanes' })).toBeInTheDocument();
    expect(screen.queryByText(/No colour matches/)).toBeNull();
  });

  it('switched to a built-in while open (the palette), its next edit makes a new copy', () => {
    openFloating();
    act(() => useAppState.getState().setSettings({ theme: 'nord' }));
    expect(screen.getByRole('dialog', { name: 'Nord' })).toBeInTheDocument();
    const field = screen.getByRole('textbox', { name: 'Panels' }) as HTMLInputElement;
    fireEvent.change(field, { target: { value: '#123456' } });
    fireEvent.submit(field.form!);
    const { theme, customThemes } = useAppState.getState().settings;
    expect(customThemes.map((t) => t.name)).toEqual(['Nord (Custom)', 'Nord (Custom) (2)']);
    expect(customThemes.find((t) => t.id === theme)?.colors).toEqual({ 'panel-bg0': '#123456' });
    expect(screen.getByRole('dialog', { name: 'Nord (Custom) (2)' })).toBeInTheDocument();
  });

  it('opens from a built-in theme too; its first edit there makes "<theme> (Custom)"', () => {
    useAppState.setState({ settings: { ...DEFAULT_SETTINGS, theme: 'nord', customThemes: [] } });
    render(<SettingsOverlay />);
    act(() => useSettingsUi.getState().show('theme'));
    fireEvent.click(screen.getByRole('button', { name: 'Edit mode' }));
    const field = within(screen.getByRole('dialog', { name: 'Nord' })).getByRole('textbox', { name: 'Panels' }) as HTMLInputElement;
    fireEvent.change(field, { target: { value: '#123456' } });
    fireEvent.submit(field.form!);
    const { theme, customThemes } = useAppState.getState().settings;
    expect(customThemes.map((t) => t.name)).toEqual(['Nord (Custom)']);
    expect(customThemes.find((t) => t.id === theme)?.colors).toEqual({ 'panel-bg0': '#123456' });
    expect(screen.getByRole('dialog', { name: 'Nord (Custom)' })).toBeInTheDocument();
  });

  it('Esc inside clears the find box first, then returns to Settings on Theme, focused on Edit mode', () => {
    openFloating();
    const find = screen.getByLabelText('Find a colour');
    fireEvent.change(find, { target: { value: 'pan' } });
    fireEvent.keyDown(find, { key: 'Escape' });
    expect(find).toHaveValue('');
    expect(panel()).toBeInTheDocument();
    fireEvent.keyDown(find, { key: 'Escape' });
    expect(screen.queryByRole('dialog', { name: 'Nord (Custom)' })).toBeNull();
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
    expect(useSettingsUi.getState().page).toBe('theme');
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Edit mode' }));
    // Settings, closed, gives the focus back to the app behind, as it would have before.
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    expect(screen.queryByRole('dialog', { name: 'Settings' })).toBeNull();
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Behind' }));
  });

  it('Done returns to Settings; Esc with focus outside the panel leaves it be', () => {
    openFloating();
    const behind = screen.getByRole('button', { name: 'Behind' });
    behind.focus();
    fireEvent.keyDown(behind, { key: 'Escape' });
    expect(panel()).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Done' }));
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Edit mode' }));
  });

  it('Esc in the colour picker closes the picker, not the panel', () => {
    openFloating();
    fireEvent.click(screen.getByRole('button', { name: 'Panels colour' }));
    const picker = screen.getByRole('dialog', { name: 'Panels colour' });
    fireEvent.keyDown(picker, { key: 'Escape' });
    expect(screen.queryByRole('dialog', { name: 'Panels colour' })).toBeNull();
    expect(panel()).toBeInTheDocument();
  });

  it('Alt+arrows on the focused title bar move it 16 px', () => {
    openFloating();
    const start = offset();
    bar().focus();
    fireEvent.keyDown(bar(), { key: 'ArrowRight', altKey: true });
    expect(offset()).toEqual({ x: start.x + 16, y: start.y });
    fireEvent.keyDown(bar(), { key: 'ArrowLeft', altKey: true });
    fireEvent.keyDown(bar(), { key: 'ArrowLeft', altKey: true });
    fireEvent.keyDown(bar(), { key: 'ArrowDown', altKey: true });
    expect(offset()).toEqual({ x: start.x - 16, y: start.y + 16 });
  });

  it('drags by its title bar, within the window, and keeps its place for the session', () => {
    openFloating();
    const start = offset();
    fireEvent.pointerDown(bar(), { button: 0, pointerId: 1, clientX: 800, clientY: 300 });
    fireEvent.pointerMove(bar(), { pointerId: 1, clientX: 700, clientY: 250 });
    expect(offset()).toEqual({ x: start.x - 100, y: start.y - 50 });
    fireEvent.pointerMove(bar(), { pointerId: 1, clientX: -5000, clientY: -5000 });
    expect(offset()).toEqual({ x: -680, y: -184 });
    fireEvent.pointerUp(bar(), { pointerId: 1 });
    fireEvent.pointerMove(bar(), { pointerId: 1, clientX: 0, clientY: 0 });
    expect(offset()).toEqual({ x: -680, y: -184 });
    fireEvent.click(screen.getByRole('button', { name: 'Done' }));
    fireEvent.click(screen.getByRole('button', { name: 'Edit mode' }));
    expect(offset()).toEqual({ x: -680, y: -184 });
  });

  it('Ctrl+, (show) brings Settings back from it', () => {
    openFloating();
    act(() => useSettingsUi.getState().show());
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
  });
});
