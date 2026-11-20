import { render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';

const appInfo = vi.hoisted(() => vi.fn());
vi.mock('../api/client', () => ({ api: { appInfo } }));
import { GitTooOldScreen } from './GitTooOldScreen';
import { useGitCheck } from './gitCheck';
import { useAppInfo } from '../app/appInfo';

const tooOld = { kind: 'GitTooOld', message: 'git 2.25.1 is too old; GitVolt needs 2.40 or newer', commandId: 1, stderr: null } as const;

it('explains the minimum and offers Retry', () => {
  render(<GitTooOldScreen error={tooOld} />);
  expect(screen.getByRole('alertdialog').textContent).toContain('GitVolt needs git 2.40 or newer');
  expect(screen.getByRole('button', { name: 'Retry' })).toBeTruthy();
});

vi.spyOn(console, 'warn').mockImplementation(() => {});
it('check() blocks on GitTooOld and git-not-found, ignores other errors, and clears when git is fine', async () => {
  appInfo.mockRejectedValueOnce(tooOld);
  await useGitCheck.getState().check();
  expect(useGitCheck.getState().problem?.kind).toBe('GitTooOld');
  appInfo.mockResolvedValueOnce({ appVersion: 'x', gitVersion: '2.47.1' });
  await useGitCheck.getState().check();
  expect(useGitCheck.getState().problem).toBeNull();
  // Known fine: no second request. (Below, each check starts as the boot's, with nothing loaded.)
  await useGitCheck.getState().check();
  expect(appInfo).toHaveBeenCalledTimes(2);
  useAppInfo.setState({ info: null });
  appInfo.mockRejectedValueOnce({ kind: 'Io', message: 'failed to run git: No such file or directory (os error 2)', commandId: 2, stderr: null });
  await useGitCheck.getState().check();
  expect(useGitCheck.getState().problem?.kind).toBe('Io');
  useGitCheck.setState({ problem: null });
  appInfo.mockRejectedValueOnce({ kind: 'Io', message: 'failed to run git: Permission denied (os error 13)', commandId: 3, stderr: null });
  await useGitCheck.getState().check();
  expect(useGitCheck.getState().problem).toBeNull();
  useGitCheck.setState({ problem: null });
  appInfo.mockRejectedValueOnce(new Error('transport'));
  await useGitCheck.getState().check();
  expect(useGitCheck.getState().problem).toBeNull();
});

it('the git check and the components that load the app info at startup share one request', async () => {
  useAppInfo.setState({ info: null });
  appInfo.mockClear();
  let answer!: (v: unknown) => void;
  appInfo.mockReturnValueOnce(new Promise((r) => { answer = r; }));
  const all = Promise.all([useGitCheck.getState().check(), useAppInfo.getState().load(), useAppInfo.getState().load()]);
  answer({ appVersion: 'x', gitVersion: '2.47.1' });
  await all;
  expect(appInfo).toHaveBeenCalledTimes(1);
  expect(useAppInfo.getState().info?.gitVersion).toBe('2.47.1');
});
