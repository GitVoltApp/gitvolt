import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../api/client', () => ({
  api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null), forgeAccounts: vi.fn(async () => []), listOpeners: vi.fn(async () => []) },
  errorMessage: String,
}));

await import('../../settings/pages');
const { LaneColours } = await import('./LaneColours');
const { SettingsOverlay } = await import('../../settings/frame/SettingsOverlay');
const { useSettingsUi } = await import('../../settings/schema');
const { DEFAULT_SETTINGS, useAppState } = await import('../../app/state');
const { useTheme } = await import('../store');
const { THEMES } = await import('../themes');

const settings = () => useAppState.getState().settings;
const current = () => settings().customThemes.find((t) => t.id === settings().theme);
const lane = (n: number) => screen.getByRole('button', { name: `Lane ${n} colour` });
const nord = THEMES.nord.graph;

beforeEach(() => {
  Element.prototype.scrollIntoView = vi.fn();
  useAppState.setState({ settings: { ...DEFAULT_SETTINGS, theme: 'nord', customThemes: [] } });
  act(() => useTheme.getState().set(THEMES.nord));
});
afterEach(() => act(() => useSettingsUi.getState().close()));

describe('graph lanes', () => {
  it('shows the ten lanes with their hex', () => {
    render(<LaneColours />);
    expect(screen.getAllByRole('listitem').map((l) => l.textContent)).toEqual([...nord]);
  });

  it('Alt+→ in Settings moves lane 1 after lane 2 on a custom copy, which is selected; the focus follows it', () => {
    render(<SettingsOverlay />);
    act(() => useSettingsUi.getState().show('graphColors'));
    expect(lane(1)).toHaveFocus();
    fireEvent.keyDown(lane(1), { key: 'ArrowRight', altKey: true });
    expect(current()?.name).toBe('Nord (Custom)');
    expect(current()?.graph).toEqual([nord[1], nord[0], ...nord.slice(2)]);
    expect(lane(2)).toHaveFocus();
    expect(screen.getByText('Lane 1 moved to position 2 of 10')).toBeInTheDocument();
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
  });

  it('picking a colour for lane 3 previews on the graph, then commits #rrggbb', () => {
    render(<LaneColours />);
    fireEvent.click(lane(3));
    const hex = within(screen.getByRole('dialog', { name: 'Lane 3 colour' })).getByLabelText('Hex');
    fireEvent.change(hex, { target: { value: '#123456' } });
    expect(document.documentElement.style.getPropertyValue('--graph-2')).toBe('#123456');
    expect(settings().customThemes).toEqual([]);
    fireEvent.keyDown(hex, { key: 'Enter' });
    expect(current()?.graph[2]).toBe('#123456');
    expect(current()?.graph.filter((c, i) => c !== nord[i])).toEqual(['#123456']);
  });
});
