import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api/client';
import type { GitflowConfig } from '../api/gen/GitflowConfig';
import { useRuntime } from '../app/runtime';
import { GitflowDialog, openGitflowDialog } from './GitflowDialog';

const flow: GitflowConfig = { master: 'main', develop: 'develop', feature: 'feature/', release: 'release/', hotfix: 'hotfix/', versionTag: 'v' };
const ok = { outcome: { status: 'done', merged: ['develop'], tag: null, deleted: true }, journal: { undo: null, redo: null, undoBlocked: null, redoBlocked: null, banners: [], paused: null }, staging: { undo: null, redo: null, off: null }, wip: null };
const tag = { name: '3.0.0', fullName: 'refs/tags/3.0.0', target: 'm'.repeat(40), time: 1, annotation: { message: 'Release 3.0.0', truncated: false, tagger: 'Ada', time: 1 } };

beforeEach(() => {
  useRuntime.setState({ tabs: {} });
  useRuntime.getState().patch('t', { repo: { id: 1, path: '/r' } as never, sidebar: { locals: [], remotes: [], worktrees: [], stashes: [], tags: [tag], submodules: {}, gitflow: flow } });
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('Finish dialog', () => {
  it('a release production has a tag for already (an earlier finish stopped before develop) keeps it', async () => {
    const asked = vi.spyOn(api, 'gitflowFinishedTag').mockResolvedValue('3.0.0');
    const finish = vi.spyOn(api, 'gitflowFinish').mockResolvedValue(ok as never);
    render(<GitflowDialog />);
    await act(async () => { openGitflowDialog({ kind: 'finish', tabId: 't', branch: 'release/spring' }); });
    expect(asked).toHaveBeenCalledWith(1, 'release/spring');
    const input = screen.getByRole('textbox', { name: 'Tag' });
    expect(input).toHaveValue('3.0.0');
    expect(input).toHaveAttribute('readonly');
    expect(screen.getByRole('textbox', { name: 'Tag message' })).toHaveValue('Release 3.0.0');
    expect(screen.getByText(/keeps main's tag 3\.0\.0/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Finish release' }));
    expect(finish).toHaveBeenCalledWith(1, '/r', expect.objectContaining({ branch: 'release/spring', tag: '3.0.0', tagMessage: null }));
  });

  it('a release not tagged yet gets the tag its version names', async () => {
    vi.spyOn(api, 'gitflowFinishedTag').mockResolvedValue(null);
    render(<GitflowDialog />);
    await act(async () => { openGitflowDialog({ kind: 'finish', tabId: 't', branch: 'release/1.2.0' }); });
    const input = screen.getByRole('textbox', { name: 'Tag' });
    expect(input).toHaveValue('v1.2.0');
    expect(input).not.toHaveAttribute('readonly');
    expect(screen.getByText(/tags main v1\.2\.0/)).toBeInTheDocument();
  });
});
