import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { Check } from 'lucide-react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BranchPreset } from '../api/gen/BranchPreset';

const api = vi.hoisted(() => ({
  graph: vi.fn(async () => ({ rows: [], labels: [], maxLanes: 1, pinnedRefs: [], head: { branch: null, target: null, detached: false, unborn: false }, truncated: false, worktrees: [] })),
  saveProfile: vi.fn(async () => null),
  saveSettings: vi.fn(async () => null),
}));
vi.mock('../api/client', () => ({ api, errorMessage: (e: unknown) => String(e) }));

const { useRuntime } = await import('../app/runtime');
const { useAppState, EMPTY_PROFILE, DEFAULT_SETTINGS } = await import('../app/state');
const { useMenu } = await import('../menu/menuStore');
const { PresetButton, presetTooltip } = await import('./PresetButton');
const { nextInCycle, presetList } = await import('./model');
import type { MenuRow } from '../menu/types';

const release: BranchPreset = { id: 'g1', name: 'Release work', color: 'orange', scope: 'all', rules: [], otherwise: 'collapse', foldMerged: true, ghostLines: true, inCycle: true };
const current = () => useAppState.getState().profile.repos['/r']?.branchPreset ?? 'all';
const actions = () => (useMenu.getState().rows ?? []).filter((r): r is Extract<MenuRow, { kind: 'action' }> => r.kind === 'action');

describe('the preset button', () => {
  beforeEach(() => {
    useAppState.setState({
      settings: { ...DEFAULT_SETTINGS, builtinPresets: [{ id: 'all', inCycle: true }, { id: 'all-collapsed', inCycle: false }, { id: 'mine', inCycle: true }], branchPresets: [release] },
      profile: { ...EMPTY_PROFILE, repos: {} },
    });
    useRuntime.setState({ tabs: { t: { status: 'ready', error: null, repo: { id: 1, path: '/r', name: 'r', worktree: '/r' }, graph: null, info: null, sidebar: null, lastFetchAt: 0, fetchSkipped: null, limit: null, worktree: null } } });
  });
  afterEach(() => { cleanup(); act(() => useMenu.getState().close()); });

  it('the click moves to the next ticked preset and the tooltip names both', () => {
    render(<PresetButton tabId="t" />);
    const main = screen.getByRole('button', { name: 'Branches: All. Switch to Mine' });
    act(() => fireEvent.click(main));
    expect(current()).toBe('mine');
    act(() => fireEvent.click(screen.getByRole('button', { name: 'Branches: Mine. Switch to Release work' })));
    expect(current()).toBe('g1');
    expect(document.querySelector('.preset-dot')?.getAttribute('style')).toContain('var(--tg-orange)');
    const list = presetList(useAppState.getState().settings, useAppState.getState().profile.repos['/r']);
    expect(presetTooltip(list[3], nextInCycle(list, 'g1'))).toBe('Branches: Release work · click for All · ▾ all presets');
    expect(presetTooltip(list[3], null)).toBe('Branches: Release work · ▾ all presets');
  });

  it('looks off when nothing else is ticked, and ▾ still opens', () => {
    useAppState.setState({ settings: { ...DEFAULT_SETTINGS, builtinPresets: [{ id: 'all', inCycle: true }, { id: 'all-collapsed', inCycle: false }, { id: 'mine', inCycle: false }] } });
    render(<PresetButton tabId="t" />);
    const main = screen.getByRole('button', { name: 'Branches: All' });
    expect(main).toHaveAttribute('aria-disabled', 'true');
    act(() => fireEvent.click(main));
    expect(current()).toBe('all');
    act(() => fireEvent.click(screen.getByRole('button', { name: 'All branch presets' })));
    expect(actions().map((r) => r.label)).toEqual(['All', 'All, collapsed', 'Mine']);
  });

  it('the menu lists every preset with a tick on the current one', () => {
    useAppState.getState().updateRepo('/r', (r) => ({ ...r, branchPreset: 'mine', customPreset: { ...release, id: 'custom', name: 'Custom', scope: 'repo', color: 'gray' } }));
    render(<PresetButton tabId="t" />);
    act(() => fireEvent.click(screen.getByRole('button', { name: 'All branch presets' })));
    const rows = useMenu.getState().rows ?? [];
    expect(rows.filter((r) => r.kind === 'header').map((r) => (r as { label: string }).label)).toEqual(['Built in', 'All repositories', 'This repository']);
    expect(actions().map((r) => [r.label, r.icon === Check, r.swatch])).toEqual([
      ['All', false, 'var(--tg-gray)'], ['All, collapsed', false, 'var(--tg-cyan)'], ['Mine', true, 'var(--tg-green)'],
      ['Release work', false, 'var(--tg-orange)'], ['Custom', false, 'var(--tg-gray)'],
    ]);
    act(() => actions()[1].run());
    expect(current()).toBe('all-collapsed');
  });

  it('keyboard: Enter cycles, ArrowDown on the caret opens the menu', () => {
    render(<PresetButton tabId="t" />);
    const main = screen.getByRole('button', { name: 'Branches: All. Switch to Mine' });
    // A real <button>: Enter and Space click it.
    expect(main.tagName).toBe('BUTTON');
    main.focus();
    act(() => fireEvent.click(main));
    expect(current()).toBe('mine');
    const caret = screen.getByRole('button', { name: 'All branch presets' });
    act(() => caret.focus());
    act(() => { fireEvent.keyDown(caret, { key: 'ArrowDown' }); });
    expect(actions().map((r) => r.label)).toContain('Release work');
    expect(caret).toHaveAttribute('aria-expanded', 'true');
  });
});

describe('the preset actions', () => {
  it('the actions are registered with their shortcuts', async () => {
    await import('./feature');
    const { getAction } = await import('../app/actions');
    expect(getAction('presets.next')).toMatchObject({ label: 'Next branch preset', shortcuts: ['Ctrl+Alt+G'] });
    expect(getAction('presets.choose')).toMatchObject({ label: 'Choose branch preset…', shortcuts: ['Ctrl+Alt+Shift+G'] });
  });
});
