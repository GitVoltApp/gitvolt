import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../api/client', () => ({
  api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null), forgeAccounts: vi.fn(async () => []), listOpeners: vi.fn(async () => []) },
  errorMessage: String,
}));

await import('../../settings/pages');
const { asHex, ThemeColourList, typedColour } = await import('./ThemeColourList');
const { SettingsOverlay } = await import('../../settings/frame/SettingsOverlay');
const { useSettingsUi } = await import('../../settings/schema');
const { DEFAULT_SETTINGS, useAppState } = await import('../../app/state');
const { useTheme } = await import('../store');
const { COLOR_TOKENS, THEMES } = await import('../themes');

const settings = () => useAppState.getState().settings;
const current = () => settings().customThemes.find((t) => t.id === settings().theme);
const rootVar = (name: string) => document.documentElement.style.getPropertyValue(name);
const rows = (el: ParentNode = document) => [...el.querySelectorAll<HTMLElement>('.theme-colour')].map((r) => r.dataset.token);

beforeEach(() => {
  Element.prototype.scrollIntoView = vi.fn();
  useAppState.setState({ settings: { ...DEFAULT_SETTINGS, theme: 'nord', customThemes: [] } });
  act(() => useTheme.getState().set(THEMES.nord));
});
afterEach(() => act(() => useSettingsUi.getState().close()));

describe('the colours list', () => {
  it('shows the 12 core colours in four groups', () => {
    render(<ThemeColourList />);
    const groups = screen.getAllByRole('group');
    expect(groups.map((g) => g.querySelector('h4')?.textContent)).toEqual(['Surfaces', 'Text', 'Diff', 'Status']);
    expect(groups.map((g) => rows(g).length)).toEqual([3, 3, 2, 4]);
    expect(within(groups[0]!).getByText('sidebar, details')).toBeInTheDocument();
    expect((screen.getByLabelText('Window background') as HTMLInputElement).value).toBe(THEMES.nord.colors['app-bg0']);
  });

  it('a colour typed on a built-in makes and selects "<theme> (Custom)" with it; a bad one is marked, then snaps back', () => {
    render(<ThemeColourList />);
    const field = screen.getByLabelText('Window background') as HTMLInputElement;
    fireEvent.change(field, { target: { value: '#202020' } });
    expect(field).toHaveAttribute('aria-invalid', 'false');
    fireEvent.submit(field.form!);
    expect(current()?.name).toBe('Nord (Custom)');
    expect(current()?.colors).toEqual({ 'app-bg0': '#202020' });
    fireEvent.change(field, { target: { value: 'red; background: url(x)' } });
    expect(field).toHaveAttribute('aria-invalid', 'true');
    fireEvent.submit(field.form!);
    expect(current()?.colors).toEqual({ 'app-bg0': '#202020' });
    fireEvent.blur(field);
    expect(field.value).toBe('#202020');
    expect(settings().customThemes).toHaveLength(1);
  });

  it('takes hex without its #, and #rrggbbaa as rgba()', () => {
    expect(typedColour('ABCDEF')).toBe('#abcdef');
    expect(typedColour(' #f0a ')).toBe('#ff00aa');
    expect(typedColour('#a3be8c55')).toBe('rgba(163, 190, 140, 0.33)');
    expect(typedColour('var(--x)')).toBeNull();
    expect(typedColour('')).toBeNull();
    // The field shows a translucent colour as #rrggbbaa.
    expect(asHex('rgba(92, 184, 92, 0.12)')).toBe('#5cb85c1f');
    expect(asHex('#ABCDEF')).toBe('#abcdef');
  });

  it('the picker previews on the app without saving; Esc puts it back and leaves Settings open', () => {
    render(<SettingsOverlay />);
    act(() => useSettingsUi.getState().show('themeColours'));
    const before = settings();
    fireEvent.click(screen.getAllByRole('button', { name: 'Window background colour' })[0]!);
    const picker = screen.getByRole('dialog', { name: 'Window background colour' });
    fireEvent.change(within(picker).getByLabelText('Hex'), { target: { value: '#123456' } });
    expect(rootVar('--app-bg0')).toBe('#123456');
    expect(settings()).toBe(before);
    fireEvent.keyDown(within(picker).getByLabelText('Hex'), { key: 'Escape' });
    expect(screen.queryByRole('dialog', { name: 'Window background colour' })).toBeNull();
    expect(rootVar('--app-bg0')).toBe(THEMES.nord.colors['app-bg0']);
    expect(settings()).toBe(before);
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
  });

  it('the picker commits on Enter', () => {
    render(<ThemeColourList />);
    fireEvent.click(screen.getByRole('button', { name: 'Accent colour' }));
    const hex = within(screen.getByRole('dialog', { name: 'Accent colour' })).getByLabelText('Hex');
    fireEvent.change(hex, { target: { value: '#336699' } });
    fireEvent.keyDown(hex, { key: 'Enter' });
    expect(current()?.colors).toEqual({ blue: '#336699' });
  });

  it('the picker offers opacity only on a colour its built-in theme makes translucent', () => {
    const opacity = (row: string) => {
      fireEvent.click(screen.getByRole('button', { name: `${row} colour` }));
      const has = within(screen.getByRole('dialog', { name: `${row} colour` })).queryByRole('slider', { name: 'Opacity' }) !== null;
      fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
      return has;
    };
    expect(THEMES.nord.colors['diff-inserted-line']).toMatch(/^rgba/);
    const { unmount } = render(<ThemeColourList />);
    expect([opacity('Accent'), opacity('Added lines')]).toEqual([false, true]);
    unmount();
    // A custom theme follows its base, whatever it set: an opaque added-lines colour keeps its opacity.
    act(() => useAppState.setState({ settings: { ...DEFAULT_SETTINGS, theme: 'custom-00000001', customThemes: [{ id: 'custom-00000001', name: 'Mine', base: 'nord', colors: { 'diff-inserted-line': '#112233' }, graph: [] }] } }));
    render(<ThemeColourList />);
    expect([opacity('Accent'), opacity('Added lines')]).toEqual([false, true]);
  });

  it('All colours lists every token', () => {
    render(<ThemeColourList />);
    const toggle = screen.getByRole('button', { name: /All colours/ });
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(rows(document.querySelector('.theme-all')!).sort()).toEqual([...COLOR_TOKENS].sort());
  });

  it('a filter keeps the matching rows, and opens All colours when only rows there match', () => {
    const { rerender } = render(<ThemeColourList filter="diff" />);
    expect(rows()).toEqual(['diff-inserted-line', 'diff-removed-line']);
    expect(screen.getByRole('button', { name: /All colours/ })).toHaveAttribute('aria-expanded', 'false');
    rerender(<ThemeColourList filter="SCROLLBAR" />);
    expect(screen.getByRole('button', { name: /All colours/ })).toHaveAttribute('aria-expanded', 'true');
    expect(rows()).toEqual(['scroll-thumb-bg', 'scroll-thumb-hover-bg']);
    rerender(<ThemeColourList filter=".*(" />);
    expect(rows()).toEqual([]);
    expect(screen.getByText('No colour matches ".*("')).toBeInTheDocument();
  });
});
