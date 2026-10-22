// @vitest-environment happy-dom
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BranchPreset } from '../../api/gen/BranchPreset';
import type { ViewSpec } from '../../api/gen/ViewSpec';

const api = vi.hoisted(() => ({
  saveProfile: vi.fn(async () => null),
  saveSettings: vi.fn(async () => null),
  graph: vi.fn(async () => ({ rows: [], labels: [], maxLanes: 1, pinnedRefs: [], head: { branch: null, target: null, detached: false, unborn: false }, truncated: false, worktrees: [] })),
  branchPresetPreview: vi.fn(async (_repo: number, _limit: number, _pin: unknown, view: ViewSpec) => ({
    rules: [...view.rules.map((_, i) => i + 2), 9], shown: 4, collapsed: 1, hidden: view.otherwise === 'hide' ? 20 : 0, folded: 3,
  })),
}));
vi.mock('../../api/client', () => ({ api, errorMessage: String }));
const clip = vi.hoisted(() => ({ text: '', copied: [] as string[] }));
vi.mock('../../api/transport', async (orig) => ({
  ...(await orig<typeof import('../../api/transport')>()),
  copyText: vi.fn(async (t: string) => { clip.copied.push(t); }),
  readClipboardText: vi.fn(async () => clip.text),
}));
vi.mock('../../ui/Select', () => ({
  Select: ({ value, options, onChange, ...r }: { value: string; options: [string, string][]; onChange(v: string): void; 'aria-label'?: string }) => (
    <select aria-label={r['aria-label']} value={value} onChange={(e) => onChange(e.target.value)}>{options.map(([v, l]) => <option key={v} value={v}>{l}</option>)}</select>
  ),
}));

const { DEFAULT_SETTINGS, EMPTY_PROFILE, EMPTY_REPO_SETTINGS, useAppState } = await import('../../app/state');
const { useRuntime } = await import('../../app/runtime');
const { useSettingsUi } = await import('../../settings/schema');
const { settingsPasteHandler } = await import('../../settings/paste');
const { settingDef } = await import('../../settings/registry');
const { useToast } = await import('../../ui/toastStore');
const { ArmLayer } = await import('../../ui/arm/ArmLayer');
const { disarm } = await import('../../ui/arm/store');
const { armClock, press } = await import('../../ui/arm/armTesting');
const { editPresetsAction, viewSpecFor } = await import('../store');
const { presetToJson } = await import('../json');
const { BranchPresetsPage, usePresetsPage } = await import('./BranchPresetsPage');
await import('./page');
await import('../../settings/pages/graph');
await import('../../settings/pages/repository');

const PATH = '/r/orbital';
const ctx = { repoPath: PATH, repoName: 'orbital', tabId: 't1' };
const preset = (id: string, over: Partial<BranchPreset> = {}): BranchPreset => ({
  id, name: id, color: 'blue', scope: 'all', rules: [{ action: 'hide', matcher: { kind: 'nameMatches', patterns: 'bot/*' } }], otherwise: 'show', foldMerged: false, ghostLines: true, inCycle: true, ...over,
});
const settings = () => useAppState.getState().settings;
const repo = () => useAppState.getState().profile.repos[PATH];
const setRepo = (over: Partial<typeof EMPTY_REPO_SETTINGS>) => act(() => useAppState.getState().updateRepo(PATH, (r) => ({ ...r, ...over })));
const page = () => render(<><BranchPresetsPage ctx={ctx} /><ArmLayer /></>);
const list = (name: string) => within(screen.getByRole('list', { name }));
const names = (name: string) => list(name).getAllByRole('listitem').map((li) => li.querySelector('.bp-name')?.textContent);
const select = (name: string) => fireEvent.click(screen.getByRole('button', { name, pressed: false }));
const commit = (input: HTMLElement, value: string) => { fireEvent.change(input, { target: { value } }); fireEvent.blur(input); };

beforeEach(() => {
  api.branchPresetPreview.mockClear();
  api.graph.mockClear();
  useAppState.setState({
    settings: { ...DEFAULT_SETTINGS, branchPresets: [preset('g1', { name: 'Quiet' }), preset('g2', { name: 'No bots', inCycle: false })] },
    profile: { ...EMPTY_PROFILE, activeTab: 't1', tabs: [{ id: 't1', kind: 'repo', path: PATH, alias: null }], repos: { [PATH]: { ...EMPTY_REPO_SETTINGS, branchPresets: [preset('r1', { name: 'Release work', scope: 'repo' })] } } },
  });
  useRuntime.setState({ tabs: { t1: { status: 'ready', error: null, repo: { id: 7, name: 'orbital', path: PATH } as never, graph: null, info: null, sidebar: null, lastFetchAt: 0, fetchSkipped: null, limit: null, worktree: null } } });
  usePresetsPage.setState({ selected: null, focusName: null });
  clip.text = '';
  clip.copied = [];
});
afterEach(() => { cleanup(); act(() => disarm()); });

describe('the branch preset list', () => {
  it('groups and order follow the settings', () => {
    setRepo({ customPreset: preset('custom', { name: 'Custom', scope: 'repo' }) });
    page();
    expect(names('Built in')).toEqual(['All', 'All, collapsed', 'Mine']);
    expect(names('All repositories')).toEqual(['Quiet', 'No bots']);
    expect(names('This repository')).toEqual(['Release work', 'Custom']);
    expect(screen.getByRole('button', { name: 'Custom' }).dataset.custom).toBe('true');
    expect(screen.getByText(/Drag to set the order\. The button's click steps through the ticked presets; the ▾ menu always lists them all\./)).toBeTruthy();
    // No repository open: no This repository.
    cleanup();
    render(<BranchPresetsPage ctx={{ repoPath: null, repoName: null, tabId: null }} />);
    expect(screen.queryByRole('list', { name: 'This repository' })).toBeNull();
  });

  it('dragging or Alt+arrows reorders within a group', () => {
    page();
    const handle = screen.getByRole('button', { name: 'Reorder Quiet' });
    handle.focus();
    fireEvent.keyDown(handle, { key: 'ArrowDown', altKey: true });
    expect(settings().branchPresets?.map((p) => p.id)).toEqual(['g2', 'g1']);
    fireEvent.keyDown(screen.getByRole('button', { name: 'Reorder Mine' }), { key: 'ArrowUp', altKey: true });
    expect(settings().builtinPresets.map((s) => s.id)).toEqual(['all', 'mine', 'all-collapsed']);
    const rect = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      const i = Number(this.dataset.sortableItem ?? 0);
      return { top: i * 26, bottom: i * 26 + 26, left: 0, right: 100, width: 100, height: 26, x: 0, y: i * 26, toJSON() {} } as DOMRect;
    });
    try {
      fireEvent.pointerDown(screen.getByRole('button', { name: 'Reorder All' }), { button: 0, clientX: 5, clientY: 10 });
      fireEvent(window, new MouseEvent('pointermove', { clientX: 5, clientY: 70 }));
      fireEvent(window, new MouseEvent('pointerup', { clientX: 5, clientY: 70 }));
      expect(settings().builtinPresets.map((s) => s.id)).toEqual(['mine', 'all-collapsed', 'all']);
    } finally {
      rect.mockRestore();
      // The click a drag's release makes is swallowed: let it go here.
      fireEvent.click(document.body);
    }
  });

  it('ticks change the cycle', () => {
    page();
    fireEvent.click(screen.getByRole('checkbox', { name: 'No bots in the cycle' }));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Mine in the cycle' }));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Release work in the cycle' }));
    expect(settings().branchPresets?.find((p) => p.id === 'g2')?.inCycle).toBe(true);
    expect(settings().builtinPresets.find((s) => s.id === 'mine')?.inCycle).toBe(false);
    expect(repo().branchPresets?.[0].inCycle).toBe(false);
  });

  it('new preset is added, selected and named', () => {
    page();
    fireEvent.click(screen.getByRole('button', { name: '+ New preset' }));
    const added = settings().branchPresets!.at(-1)!;
    expect(added).toMatchObject({ name: 'New preset', scope: 'all', otherwise: 'show', inCycle: true, rules: [] });
    expect(screen.getByRole('button', { name: 'New preset', pressed: true })).toBeTruthy();
    const name = screen.getByRole('textbox', { name: 'Name' });
    expect(document.activeElement).toBe(name);
    commit(name, 'Payments');
    expect(settings().branchPresets!.at(-1)!.name).toBe('Payments');
  });
});

describe('the branch preset editor', () => {
  it('opens on the repository\'s current preset; built-ins are read-only', () => {
    setRepo({ branchPreset: 'mine' });
    page();
    expect(screen.getByRole('button', { name: 'Mine', pressed: true })).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Delete preset' })).toBeNull();
    expect(screen.queryByRole('button', { name: '+ Add rule' })).toBeNull();
    expect(screen.getByText('has commits by')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Duplicate' }));
    expect(settings().branchPresets!.at(-1)).toMatchObject({ name: 'Mine (2)', scope: 'all' });
    expect(screen.getByRole('button', { name: 'Mine (2)', pressed: true })).toBeTruthy();
  });

  it('edits save to the right scope', () => {
    page();
    select('Release work');
    fireEvent.click(within(screen.getByRole('group', { name: 'Rule 1 action' })).getByRole('button', { name: 'Collapse' }));
    expect(repo().branchPresets?.[0].rules[0].action).toBe('collapse');
    fireEvent.click(screen.getByRole('button', { name: 'purple' }));
    expect(repo().branchPresets?.[0].color).toBe('purple');
    fireEvent.click(screen.getByRole('switch', { name: 'Fold branches merged into the trunk' }));
    expect(repo().branchPresets?.[0].foldMerged).toBe(true);
    fireEvent.change(screen.getByRole('combobox', { name: 'Rule 1 matches' }), { target: { value: 'olderThan' } });
    expect(repo().branchPresets?.[0].rules[0].matcher).toEqual({ kind: 'olderThan', days: 90 });
    commit(screen.getByRole('textbox', { name: 'Rule 1 days' }), '30');
    expect(repo().branchPresets?.[0].rules[0].matcher).toEqual({ kind: 'olderThan', days: 30 });
    fireEvent.click(screen.getByRole('button', { name: '+ Add rule' }));
    commit(screen.getByRole('textbox', { name: 'Rule 2 patterns' }), 'dependabot/*, renovate/*');
    expect(repo().branchPresets?.[0].rules[1]).toEqual({ action: 'hide', matcher: { kind: 'nameMatches', patterns: 'dependabot/*, renovate/*' } });
    // Available in: All repositories moves it to the app settings.
    fireEvent.click(screen.getByRole('button', { name: 'All repositories', pressed: false }));
    expect(repo().branchPresets).toEqual([]);
    expect(settings().branchPresets?.at(-1)).toMatchObject({ id: 'r1', scope: 'all', color: 'purple' });
    fireEvent.click(screen.getByRole('button', { name: 'orbital only' }));
    expect(repo().branchPresets?.map((p) => [p.id, p.scope])).toEqual([['r1', 'repo']]);
  });

  it('rule counts come from the preview of the edited preset', async () => {
    page();
    select('Quiet');
    await waitFor(() => expect(screen.getByText('2 branches')).toBeTruthy());
    expect(screen.getByText('9 branches')).toBeTruthy();
    expect(screen.getByText(/In orbital now:/).textContent).toBe('In orbital now: 4 shown · 1 collapsed · 0 hidden · 3 merged branches folded');
    fireEvent.click(within(screen.getByRole('group', { name: 'Everything else action' })).getByRole('button', { name: 'Hide' }));
    await waitFor(() => expect(screen.getByText(/In orbital now:/).textContent).toContain('20 hidden'));
    const [repoId, limit, , view] = api.branchPresetPreview.mock.calls.at(-1)!;
    expect([repoId, limit, view.otherwise, view.rules]).toEqual([7, 2000, 'hide', [{ action: 'hide', matcher: { kind: 'nameMatches', patterns: 'bot/*' } }]]);
  });

  it('picked-by-hand chips forget a branch with ✕', () => {
    setRepo({ branchPicks: { g1: { shown: ['feature/payments'], hidden: ['spike/warp-gates'] } } });
    page();
    select('Quiet');
    expect(screen.getByText('Always hidden:')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Forget spike/warp-gates' }));
    expect(repo().branchPicks?.g1).toEqual({ shown: ['feature/payments'], hidden: [] });
  });

  it('delete arms in place and deletes on the second click', async () => {
    const clock = armClock();
    const rects = vi.spyOn(HTMLElement.prototype, 'getClientRects').mockImplementation(function (this: HTMLElement) {
      return (this.isConnected ? [new DOMRect(10, 10, 24, 24)] : []) as unknown as DOMRectList;
    });
    try {
      page();
      select('No bots');
      act(() => press(screen.getByRole('button', { name: 'Delete preset' })));
      const overlay = () => document.querySelector<HTMLElement>('[data-arm-overlay]');
      expect(overlay()?.textContent).toBe('Delete No bots');
      expect(settings().branchPresets).toHaveLength(2);
      clock.settle();
      await act(async () => { press(overlay()!); });
      await waitFor(() => expect(settings().branchPresets?.map((p) => p.id)).toEqual(['g1']));
      expect(screen.getByRole('button', { name: 'Quiet', pressed: true })).toBeTruthy();
    } finally {
      rects.mockRestore();
      clock.restore();
    }
  });

  it('renaming Custom makes it a regular preset', () => {
    setRepo({ branchPreset: 'custom', customPreset: preset('custom', { name: 'Custom', scope: 'repo' }), branchPicks: { custom: { shown: [], hidden: ['spike/warp-gates'] } } });
    page();
    commit(screen.getByRole('textbox', { name: 'Name' }), 'Spikes hidden');
    expect(repo().customPreset).toBeUndefined();
    const [p] = repo().branchPresets!.slice(-1);
    expect(p).toMatchObject({ name: 'Spikes hidden', scope: 'repo' });
    expect(repo().branchPreset).toBe(p.id);
    expect(repo().branchPicks?.[p.id]).toEqual({ shown: [], hidden: ['spike/warp-gates'] });
    expect(screen.getByRole('button', { name: 'Spikes hidden', pressed: true })).toBeTruthy();
  });

  it('Ctrl+V outside a text field imports a preset; other clipboard text toasts', async () => {
    page();
    act(() => settingsPasteHandler()?.(presetToJson(preset('x', { name: 'Quiet', otherwise: 'collapse' }))));
    expect(settings().branchPresets?.map((p) => p.name)).toEqual(['Quiet', 'No bots', 'Quiet (2)']);
    expect(screen.getByRole('button', { name: 'Quiet (2)', pressed: true })).toBeTruthy();
    act(() => settingsPasteHandler()?.('{"gitvolt":"theme","version":1}'));
    expect(useToast.getState().message).toBe("That isn't a GitVolt branch preset");
    expect(settings().branchPresets).toHaveLength(3);
    // Copy as JSON, then the Paste preset button.
    fireEvent.click(screen.getByRole('button', { name: 'Copy as JSON' }));
    await waitFor(() => expect(clip.copied).toHaveLength(1));
    clip.text = clip.copied[0];
    fireEvent.click(screen.getByRole('button', { name: 'Paste preset' }));
    await waitFor(() => expect(settings().branchPresets?.map((p) => p.name)).toEqual(['Quiet', 'No bots', 'Quiet (2)', 'Quiet (3)']));
  });

  it('an edit to the current preset reloads the graph with it', () => {
    setRepo({ branchPreset: 'g1' });
    page();
    fireEvent.click(within(screen.getByRole('group', { name: 'Everything else action' })).getByRole('button', { name: 'Collapse' }));
    expect(api.graph).toHaveBeenCalledTimes(1);
    expect((api.graph.mock.calls[0] as unknown as [number, number, { view: ViewSpec }])[2].view.otherwise).toBe('collapse');
    // A colour isn't part of the view.
    fireEvent.click(screen.getByRole('button', { name: 'green' }));
    expect(api.graph).toHaveBeenCalledTimes(1);
  });
});

describe('Graph and Repository settings, and Edit presets…', () => {
  it('the two Graph settings change the panel\'s hover and the placement sent with the view', () => {
    setRepo({ branchPreset: 'mine' });
    act(() => settingDef('collapseHoverOpen')!.binding!(ctx)!.set(false));
    expect(settings().collapseHoverOpen).toBe(false);
    act(() => settingDef('collapsePlacement')!.binding!(ctx)!.set('pullTogether'));
    expect(viewSpecFor('t1')?.placement).toBe('pullTogether');
    expect(api.graph).toHaveBeenCalledTimes(1);
    expect(settingDef('collapsePlacement')).toMatchObject({ page: 'graph', group: 'Hidden commits', label: 'Collapsed branches' });
  });

  it('trunk patterns go into the view', () => {
    setRepo({ branchPreset: 'mine' });
    act(() => settingDef('trunkPatterns')!.binding!(ctx)!.set(['release/*']));
    expect(viewSpecFor('t1')?.trunkPatterns).toEqual(['release/*']);
    expect(api.graph).toHaveBeenCalledTimes(1);
    expect(settingDef('trunkPatterns')).toMatchObject({ page: 'repository', label: 'Trunk branches' });
  });

  it('Edit presets… opens the page on the current preset', async () => {
    setRepo({ branchPreset: 'r1' });
    usePresetsPage.setState({ selected: 'g2' });
    const { presetMenuRows } = await import('../PresetButton');
    const edit = presetMenuRows('t1', [], 'all').at(-1);
    expect(edit).toMatchObject({ kind: 'action', label: 'Edit presets…' });
    act(() => (edit as { run(): void }).run());
    expect(editPresetsAction()).not.toBeNull();
    expect(useSettingsUi.getState()).toMatchObject({ open: true, page: 'branch-presets' });
    page();
    expect(screen.getByRole('button', { name: 'Release work', pressed: true })).toBeTruthy();
    expect(screen.getByRole('textbox', { name: 'Name' })).toHaveProperty('value', 'Release work');
    act(() => useSettingsUi.getState().close());
  });
});
