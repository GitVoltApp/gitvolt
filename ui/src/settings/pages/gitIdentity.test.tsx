import { act, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const gitIdentity = vi.fn();
vi.mock('../../api/client', () => ({
  api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null), gitIdentity: (...a: unknown[]) => gitIdentity(...a) },
  errorMessage: String,
}));

await import('./gitIdentity');
const { EMPTY_PROFILE, useAppState } = await import('../../app/state');
const { useRuntime } = await import('../../app/runtime');
const { settingDef } = await import('../registry');

function renderCard(ctx: { repoPath: string | null; repoName: string | null; tabId: string | null }) {
  const Control = settingDef('gitIdentity')!.Control!;
  render(<Control ctx={ctx} />);
}

beforeEach(() => {
  gitIdentity.mockReset();
  useAppState.setState({ profile: { ...EMPTY_PROFILE, name: 'Work' } });
  useRuntime.setState({ tabs: { t1: { status: 'ready', error: null, repo: { id: 7, name: 'orbital', path: '/r/orbital' } as never, graph: null, info: null, lastFetchAt: 0, fetchSkipped: null, limit: null, worktree: '/r/orbital', sidebar: null } as never } });
});

describe('Git identity card', () => {
  it('shows the name, the email with signing, and the repository sentence', async () => {
    gitIdentity.mockResolvedValue({ identity: { name: 'Robin Example', email: 'robin@example.com' }, signing: { format: 'ssh', key: '/keys/robin.pub' } });
    renderCard({ repoPath: '/r/orbital', repoName: 'orbital', tabId: 't1' });
    expect(await screen.findByText('Robin Example')).toBeInTheDocument();
    expect(screen.getByText('robin@example.com · signed (SSH key)')).toBeInTheDocument();
    expect(screen.getByText(/From orbital's git config, with this profile's extra config applied/)).toBeInTheDocument();
    expect(gitIdentity).toHaveBeenCalledWith(7, '/r/orbital');
  });

  it('warns when no identity is set', async () => {
    gitIdentity.mockResolvedValue({ identity: null, signing: null });
    renderCard({ repoPath: '/r/orbital', repoName: 'orbital', tabId: 't1' });
    expect(await screen.findByText(/No name and email set: commits will fail/)).toBeInTheDocument();
  });

  it('without a repository reads the global config, and again on window focus', async () => {
    gitIdentity.mockResolvedValue({ identity: { name: 'Robin Example', email: 'robin@example.com' }, signing: null });
    renderCard({ repoPath: null, repoName: null, tabId: null });
    expect(await screen.findByText(/From your global git config/)).toBeInTheDocument();
    expect(gitIdentity).toHaveBeenCalledWith(null, null);
    await act(async () => { window.dispatchEvent(new Event('focus')); });
    await waitFor(() => expect(gitIdentity).toHaveBeenCalledTimes(2));
  });
});
