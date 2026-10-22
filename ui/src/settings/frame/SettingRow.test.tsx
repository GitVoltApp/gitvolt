import { act, fireEvent, render, screen } from '@testing-library/react';
import { Settings } from 'lucide-react';
import { afterAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../api/client', () => ({ api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null) } }));

const { DEFAULT_SETTINGS, useAppState } = await import('../../app/state');
const { appSetting } = await import('../bindings');
const { registerSettings, registerSettingsPage } = await import('../registry');
const { useSettingsUi } = await import('../schema');
const { SettingRow } = await import('./SettingRow');
const { SettingsPageView } = await import('./SettingsPage');

const ctx = { repoPath: null, repoName: null, tabId: null };
const page = { id: 't-fetch', label: 'Fetch and push', scope: 'app' as const, order: 1, icon: Settings, summary: 'Background fetch.' };
const offs = [
  registerSettingsPage(page),
  registerSettings([
    {
      id: 't-prune', label: 'Prune deleted remote branches', page: 't-fetch', group: 'Fetch', keywords: '', description: 'Fetch with --prune.', binding: appSetting('prune', true),
      Control: () => <input id="input-t-prune" type="checkbox" checked={useAppState((s) => s.settings.prune)} onChange={(e) => useAppState.getState().setSettings({ prune: e.target.checked })} />,
    },
    { id: 't-list', label: 'Hosts', page: 't-hosts', group: 'Hosts', keywords: '' },
    { id: 't-tags', label: 'Push tags', page: 't-fetch', group: 'Push', keywords: '' },
  ]),
];
afterAll(() => offs.forEach((off) => off()));

describe('SettingRow', () => {
  beforeEach(() => useAppState.setState({ settings: { ...DEFAULT_SETTINGS } }));

  it('renders the label for its control and the description under it', () => {
    render(<SettingRow id="t-prune" ctx={ctx} />);
    expect(screen.getByLabelText('Prune deleted remote branches')).toHaveAttribute('type', 'checkbox');
    expect(screen.getByText('Fetch with --prune.')).toBeVisible();
  });

  it('an unmodified row keeps its ↺ in place, hidden; a modified one shows the ↺, which restores the default', () => {
    const { container } = render(<SettingRow id="t-prune" ctx={ctx} />);
    const reset = container.querySelector<HTMLButtonElement>('.setting-reset')!;
    expect(reset).toBeInTheDocument();
    expect(reset).not.toBeVisible();
    fireEvent.click(screen.getByLabelText('Prune deleted remote branches'));
    expect(useAppState.getState().settings.prune).toBe(false);
    expect(reset).toBeVisible();
    expect(screen.getByText('Changed')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Restore the default' }));
    expect(useAppState.getState().settings.prune).toBe(true);
    expect(screen.queryByText('Changed')).toBeNull();
    expect(reset).not.toBeVisible();
    expect(screen.getByLabelText('Prune deleted remote branches')).toHaveFocus();
  });

  it('without a control it offers "Open page", which opens its page and clears the query', () => {
    act(() => { useSettingsUi.getState().show(); useSettingsUi.getState().setQuery('hosts'); });
    render(<SettingRow id="t-list" ctx={ctx} />);
    fireEvent.click(screen.getByRole('button', { name: 'Open page' }));
    expect(useSettingsUi.getState()).toMatchObject({ page: 't-hosts', query: '' });
  });

  it('wraps the highlighted words in <mark>', () => {
    const { container } = render(<SettingRow id="t-prune" ctx={ctx} highlights={{ label: [{ start: 0, end: 5 }], description: [{ start: 11, end: 18 }] }} />);
    expect([...container.querySelectorAll('mark')].map((m) => m.textContent)).toEqual(['Prune', '--prune']);
  });

  it("a page shows its title, summary and groups in registration order", () => {
    render(<SettingsPageView page={page} ctx={ctx} />);
    expect(screen.getByRole('region', { name: 'Fetch and push' })).toBeInTheDocument();
    expect(screen.getByText('Background fetch.')).toBeInTheDocument();
    expect(screen.getAllByRole('group').map((g) => g.getAttribute('aria-labelledby') && document.getElementById(g.getAttribute('aria-labelledby')!)?.textContent)).toEqual(['Fetch', 'Push']);
  });
});
