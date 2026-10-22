import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../api/client', () => ({ api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null) } }));

const { AppearanceSection } = await import('./AppearanceSection');
const { DEFAULT_SETTINGS, useAppState } = await import('../app/state');
const { ContextMenu } = await import('../menu/ContextMenu');
const { useMenu } = await import('../menu/menuStore');
const { THEME_IDS, THEMES } = await import('./themes');

const settings = () => useAppState.getState().settings;
const current = () => settings().customThemes.find((t) => t.id === settings().theme);
const lane = (n: number) => screen.getByLabelText(`Lane ${n} color`) as HTMLInputElement;
/** Types into a lane's box and applies it (Enter submits its form). */
function typeLane(n: number, value: string) {
  fireEvent.change(lane(n), { target: { value } });
  fireEvent.submit(lane(n).form!);
}
const mine = { id: 'custom-0a1b2c3d', name: 'Mine', base: 'light', colors: {}, graph: [] };

describe('AppearanceSection', () => {
  beforeEach(() => useAppState.setState({ settings: { ...DEFAULT_SETTINGS, theme: 'default-dark', customThemes: [] } }));
  afterEach(() => act(() => useMenu.getState().close()));

  it("lists every theme, built-ins then custom ones, in the app's own dropdown, and saves the chosen one", () => {
    useAppState.setState({ settings: { ...DEFAULT_SETTINGS, theme: 'default-dark', customThemes: [mine] } });
    render(<><AppearanceSection /><ContextMenu /></>);
    const button = screen.getByRole('button', { name: 'Theme' });
    expect(button).toHaveTextContent('Default Dark');
    expect(document.querySelector('select')).toBeNull();
    fireEvent.click(button);
    expect(screen.getAllByRole('menuitem').map((r) => r.textContent)).toEqual([...THEME_IDS.map((id) => THEMES[id].label), 'Mine']);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Monokai' }));
    expect(settings().theme).toBe('monokai');
    expect(screen.getByRole('button', { name: 'Theme' })).toHaveTextContent('Monokai');
    fireEvent.click(screen.getByRole('button', { name: 'Theme' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Mine' }));
    expect(settings().theme).toBe(mine.id);
    expect(lane(1).value).toBe(THEMES.light.graph[0]);
  });

  it('shows an unknown saved theme as Default Dark', () => {
    useAppState.setState({ settings: { ...DEFAULT_SETTINGS, theme: 'no-such-theme' } });
    render(<AppearanceSection />);
    expect(screen.getByRole('button', { name: 'Theme' })).toHaveTextContent('Default Dark');
  });

  it('a lane edit on a built-in creates and selects "<theme> (Custom)"; the next edits it', () => {
    useAppState.setState({ settings: { ...DEFAULT_SETTINGS, theme: 'nord', customThemes: [] } });
    render(<AppearanceSection />);
    expect(lane(3).value).toBe(THEMES.nord.graph[2]);
    typeLane(3, '#112233');
    expect(current()?.name).toBe('Nord (Custom)');
    expect(current()?.graph).toEqual(THEMES.nord.graph.map((c, i) => (i === 2 ? '#112233' : c)));
    expect(lane(3).value).toBe('#112233');
    typeLane(1, '#445566');
    expect(settings().customThemes).toHaveLength(1);
    expect(current()?.graph.slice(0, 3)).toEqual(['#445566', THEMES.nord.graph[1], '#112233']);
    expect(screen.queryByRole('button', { name: 'Reset lane colors' })).toBeNull();
  });

  it('takes a colour without its #, in any case, expands #rgb, and snaps an invalid or empty one back', () => {
    useAppState.setState({ settings: { ...DEFAULT_SETTINGS, theme: 'light', customThemes: [] } });
    render(<AppearanceSection />);
    typeLane(1, 'ABCDEF');
    expect(current()?.graph[0]).toBe('#abcdef');
    typeLane(3, '#F0a');
    expect(current()?.graph[2]).toBe('#ff00aa');
    expect(lane(3).value).toBe('#ff00aa');
    typeLane(2, 'blue-ish');
    expect(lane(2).value).toBe(THEMES.light.graph[1]);
    typeLane(2, '');
    expect(lane(2).value).toBe(THEMES.light.graph[1]);
    expect(current()?.graph[1]).toBe(THEMES.light.graph[1]);
  });
});
