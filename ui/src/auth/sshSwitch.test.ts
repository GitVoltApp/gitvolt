// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { GbError } from '../api/gen/GbError';
import type { SshOffer } from '../api/gen/SshOffer';

const done = { outcome: null, journal: {}, staging: {}, wip: null };
const api = vi.hoisted(() => ({ switchRemoteToSsh: vi.fn(), switchRemoteBack: vi.fn(), journalState: vi.fn() }));
vi.mock('../api/client', () => ({ api, errorMessage: String, onEvent: () => () => {} }));
const toasted = vi.hoisted(() => ({ calls: [] as GbError[] }));
vi.mock('../debug/errorToast', () => ({ toastActionError: (e: GbError) => toasted.calls.push(e) }));

const { runWrite } = await import('../write/client');
const { sshSwitches, withSshRetry } = await import('./sshSwitch');
const { useToast } = await import('../ui/toastStore');

const ctx = { tabId: 't', repoId: 4, worktree: '/r' };
const ORIGIN: SshOffer = { remote: 'origin', url: 'git@gitlab.example.com:group/project.git' };
const useSsh = (remotes: SshOffer[]): GbError => ({ kind: 'Cancelled', message: 'Cancelled', commandId: null, stderr: null, detail: { kind: 'useSsh', remotes } });
const denied: GbError = { kind: 'AuthFailed', message: 'git@gitlab.example.com: Permission denied (publickey).', commandId: 7, stderr: null };
const pushed = { outcome: 'pushed', journal: {}, staging: {}, wip: null };

describe('Use SSH instead (spec §5.4)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    toasted.calls = [];
    api.switchRemoteToSsh.mockResolvedValue(done);
    api.switchRemoteBack.mockResolvedValue(done);
    useToast.getState().dismiss();
  });

  it('reads the switch only off a cancel that carries it', () => {
    expect(sshSwitches(useSsh([ORIGIN]))).toEqual([ORIGIN]);
    expect(sshSwitches({ kind: 'Cancelled', message: 'Cancelled', commandId: null, stderr: null })).toBeNull();
    expect(sshSwitches({ ...useSsh([ORIGIN]), kind: 'AuthFailed' })).toBeNull();
    expect(sshSwitches(useSsh([]))).toBeNull();
    expect(sshSwitches(null)).toBeNull();
  });

  it('switches the remote, then sends the same request again, saying nothing', async () => {
    const send = vi.fn().mockRejectedValueOnce(useSsh([ORIGIN])).mockResolvedValueOnce(pushed);
    expect(await runWrite(ctx, send)).toBe('pushed');
    expect(api.switchRemoteToSsh).toHaveBeenCalledWith(4, '/r', 'origin', 'git@gitlab.example.com:group/project.git');
    expect(send).toHaveBeenCalledTimes(2);
    expect(api.switchRemoteToSsh.mock.invocationCallOrder[0]).toBeLessThan(send.mock.invocationCallOrder[1]);
    expect(useToast.getState().message).toBeNull();
    expect(toasted.calls).toEqual([]);
  });

  it('a switch that fails sends nothing more', async () => {
    api.switchRemoteToSsh.mockRejectedValue({ kind: 'Stale', message: "origin doesn't use HTTPS any more", commandId: null, stderr: null });
    const send = vi.fn().mockRejectedValue(useSsh([ORIGIN]));
    expect(await runWrite(ctx, send)).toBeNull();
    expect(send).toHaveBeenCalledTimes(1);
    expect(toasted.calls.map((e) => e.kind)).toEqual(['Stale']);
  });

  it('a failure after the switch says so and offers Switch back to HTTPS', async () => {
    const send = vi.fn().mockRejectedValueOnce(useSsh([ORIGIN])).mockRejectedValueOnce(denied);
    expect(await runWrite(ctx, send)).toBeNull();
    const t = useToast.getState();
    expect(t.message).toBe('Authentication failed after switching origin to SSH: git@gitlab.example.com: Permission denied (publickey).');
    expect(t.actions.map((a) => a.label)).toEqual(['Switch back to HTTPS', 'Details']);
    expect(toasted.calls).toEqual([]);
    t.actions[0]!.run();
    await vi.waitFor(() => expect(useToast.getState().message).toBe('origin uses HTTPS again'));
    expect(api.switchRemoteBack).toHaveBeenCalledWith(4, '/r', 'origin');
  });

  it('a caller without its own handling sends once more after the switch', async () => {
    const send = vi.fn().mockRejectedValueOnce(useSsh([ORIGIN])).mockResolvedValueOnce('fetched');
    expect(await withSshRetry(ctx, send)).toBe('fetched');
    expect(api.switchRemoteToSsh).toHaveBeenCalledTimes(1);
    const fails = vi.fn().mockRejectedValue(denied);
    await expect(withSshRetry(ctx, fails)).rejects.toBe(denied);
    expect(fails).toHaveBeenCalledTimes(1);
  });

  it('a cancel after the switch (its SSH prompt dismissed) still offers the way back', async () => {
    const send = vi.fn().mockRejectedValueOnce(useSsh([ORIGIN])).mockRejectedValueOnce({ kind: 'Cancelled', message: 'Cancelled', commandId: null, stderr: null });
    expect(await runWrite(ctx, send)).toBeNull();
    expect(useToast.getState().message).toBe('Cancelled; origin now uses SSH');
    expect(useToast.getState().actions.map((a) => a.label)).toEqual(['Switch back to HTTPS']);
  });
});
