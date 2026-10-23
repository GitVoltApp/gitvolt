import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../api/client', () => ({ api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null), forgeAccounts: vi.fn(async () => []) }, errorMessage: String }));

await import('../pages');
const { DEFAULT_SETTINGS, EMPTY_PROFILE, useAppState } = await import('../../app/state');
const { allSettings, settingsPages } = await import('../registry');
const { useSettingsUi } = await import('../schema');
const { searchSettings } = await import('../search');
const { SearchResults } = await import('./SearchResults');

const ctx = { repoPath: null, repoName: null, tabId: null };
const results = (q: string) => render(<SearchResults query={q} results={searchSettings(q, allSettings(), settingsPages().filter((p) => p.scope !== 'repo'))} ctx={ctx} profileName="Work" />);

describe('SearchResults', () => {
  beforeEach(() => useAppState.setState({ settings: { ...DEFAULT_SETTINGS }, profile: { ...EMPTY_PROFILE, name: 'Work' } }));

  it('lists every match in place, grouped under where it lives, with the words highlighted', () => {
    const { container } = results('avatar');
    expect(screen.getByRole('heading', { name: '3 settings match "avatar"' })).toBeInTheDocument();
    expect(screen.getByText('Change them here, or open their page.')).toBeInTheDocument();
    const rows = [...container.querySelectorAll('[data-setting-id]')];
    expect(rows.map((r) => r.getAttribute('data-setting-id'))).toEqual(['gravatar', 'forgeAvatars', 'forgeAccounts']);
    for (const r of rows) expect(r.querySelector('mark'), r.getAttribute('data-setting-id')!).not.toBeNull();
    expect(screen.getByRole('group', { name: 'Graph › Avatars' })).toHaveTextContent('GitVolt › Graph › Avatars');
    expect(screen.getByRole('group', { name: 'Accounts › Accounts' })).toHaveTextContent('Profile · Work › Accounts › Accounts');
  });

  it('changes a setting right there', () => {
    results('avatar');
    fireEvent.click(screen.getByLabelText('From Gravatar'));
    expect(useAppState.getState().settings.gravatar).toBe(false);
  });

  it('"Open page" opens the page on its first match and clears the query', () => {
    act(() => { useSettingsUi.getState().show(); useSettingsUi.getState().setQuery('avatar'); });
    results('avatar');
    fireEvent.click(within(screen.getByRole('group', { name: 'Graph › Avatars' })).getByRole('button', { name: 'Open page' }));
    expect(useSettingsUi.getState()).toMatchObject({ page: 'graph', query: '', focus: 'gravatar' });
  });

  it('says when nothing matches', () => {
    results('zzzz');
    expect(screen.getByRole('heading', { name: 'No settings match "zzzz"' })).toBeInTheDocument();
  });
});
