import { act, fireEvent, render, screen } from '@testing-library/react';
import { KeyRound } from 'lucide-react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { SshOffer } from '../api/gen/SshOffer';
import { registerKeys } from '../ui/keyRouter';

const authAnswer = vi.fn(async (..._a: unknown[]) => null as unknown);
const authUseSsh = vi.fn(async (..._a: unknown[]) => null as unknown);
vi.mock('../api/client', () => ({ api: { authAnswer: (...a: unknown[]) => authAnswer(...a), authUseSsh: (...a: unknown[]) => authUseSsh(...a) }, onEvent: () => () => {} }));
const { AuthModal } = await import('./AuthModal');
const { useOps } = await import('../app/ops');
const { useMenu } = await import('../menu/menuStore');
const { RefPicker } = await import('../ui/RefPicker');

const ask = (prompt: number, text: string, secret: boolean, ssh: SshOffer | null = null) =>
  act(() => useOps.getState().apply({ type: 'authWaiting', prompt, op: 1, repo: null, text, secret, ssh }));
const ORIGIN: SshOffer = { remote: 'origin', url: 'git@gitlab.example.com:group/project.git' };

describe('AuthModal', () => {
  beforeEach(() => {
    authAnswer.mockClear();
    authAnswer.mockImplementation(async () => null);
    authUseSsh.mockClear();
    useOps.setState({ ops: {}, prompts: [], errors: [], unread: 0 });
  });

  it('asks with a password field for secrets and answers or cancels', () => {
    render(<AuthModal />);
    ask(5, "Password for 'https://ada@h': ", true);
    expect(screen.getByText("Password for 'https://ada@h':")).toBeInTheDocument();
    const input = screen.getByLabelText('Password');
    expect(input).toHaveAttribute('type', 'password');
    fireEvent.change(input, { target: { value: 's3cret' } });
    fireEvent.click(screen.getByRole('button', { name: 'Submit' }));
    expect(authAnswer).toHaveBeenLastCalledWith(5, 's3cret');
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(authAnswer).toHaveBeenLastCalledWith(5, null);
  });

  it('Ctrl+Enter in the field answers (its own registerKeys layer)', () => {
    render(<AuthModal />);
    ask(8, "Password for 'https://ada@h': ", true);
    const input = screen.getByLabelText('Password');
    fireEvent.change(input, { target: { value: 'pw' } });
    fireEvent.keyDown(input, { key: 'Enter', ctrlKey: true });
    expect(authAnswer).toHaveBeenLastCalledWith(8, 'pw');
  });

  it('asks with a plain text field otherwise, focused; the next prompt starts empty', () => {
    render(<AuthModal />);
    ask(6, "Username for 'https://h': ", false);
    const input = screen.getByLabelText('Answer');
    expect(input).toHaveAttribute('type', 'text');
    expect(input).toHaveFocus();
    fireEvent.change(input, { target: { value: 'ada' } });
    fireEvent.submit(input.closest('form')!);
    expect(authAnswer).toHaveBeenLastCalledWith(6, 'ada');
    act(() => useOps.getState().apply({ type: 'authResolved', prompt: 6 }));
    expect(screen.queryByRole('dialog')).toBeNull();
    ask(7, "Password for 'https://ada@h': ", true);
    expect(screen.getByLabelText('Password')).toHaveValue('');
  });

  it('owns the keyboard while open: the app layer behind it sees no key (ruling R6)', () => {
    const app = vi.fn(() => 'handled' as const);
    const off = registerKeys('app', app);
    render(<AuthModal />);
    ask(8, 'Passphrase: ', true);
    const input = screen.getByLabelText('Password');
    for (const key of ['Escape', 'w', 'F7', 'ArrowDown']) fireEvent.keyDown(input, { key, ctrlKey: key === 'w' });
    expect(app).not.toHaveBeenCalled();
    act(() => useOps.getState().apply({ type: 'authResolved', prompt: 8 }));
    fireEvent.keyDown(document.body, { key: 'Escape' });
    expect(app).toHaveBeenCalledTimes(1);
    off();
  });

  it('opening closes an open context menu and picker, so nothing else shares the menu layer\'s keys', () => {
    const onClose = vi.fn();
    render(
      <>
        <AuthModal />
        <RefPicker anchor={new DOMRect(0, 0, 10, 10)} items={[{ id: 'a', label: 'a' }]} placeholder="Find" onPick={() => {}} onClose={onClose} />
      </>,
    );
    act(() => useMenu.getState().show([{ kind: 'action', id: 'x', label: 'X', icon: KeyRound, tooltip: 't', run: () => {} }], 0, 0));
    ask(10, 'Username: ', false);
    expect(useMenu.getState().rows).toBeNull();
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('an HTTPS prompt tied to a remote offers its SSH URL; choosing it is all the modal sends', () => {
    render(<AuthModal />);
    ask(11, "Username for 'https://gitlab.example.com': ", false, ORIGIN);
    expect(screen.getByText('This remote uses HTTPS, so git needs a username and password.')).toBeInTheDocument();
    expect(screen.getByText('git@gitlab.example.com:group/project.git').tagName).toBe('CODE');
    expect(screen.getByText('Remembering credentials is up to your git credential helper.')).toBeInTheDocument();
    expect(screen.queryByText(/token/i)).toBeNull();
    const use = screen.getByRole('button', { name: 'Use SSH for origin instead' });
    expect(use).toHaveAccessibleDescription('git@gitlab.example.com:group/project.git');
    expect(screen.getByLabelText('Answer')).toHaveFocus();
    fireEvent.click(use);
    expect(authUseSsh).toHaveBeenCalledWith(11);
    expect(authAnswer).not.toHaveBeenCalled();
  });

  it('without a remote to tie it to, a prompt offers no SSH', () => {
    render(<AuthModal />);
    ask(12, "Enter passphrase for key '/home/ada/.ssh/id_ed25519': ", true);
    expect(screen.queryByText(/This remote uses HTTPS/)).toBeNull();
    expect(screen.queryByRole('button', { name: /Use SSH/ })).toBeNull();
  });

  it('Tab reaches the SSH button and stays inside the modal', () => {
    render(<AuthModal />);
    ask(13, "Password for 'https://ada@gitlab.example.com': ", true, ORIGIN);
    const order: (string | null)[] = [];
    for (let i = 0; i < 4; i++) {
      fireEvent.keyDown(document.activeElement!, { key: 'Tab' });
      order.push(document.activeElement?.textContent || document.activeElement?.getAttribute('aria-label') || null);
    }
    expect(order).toEqual(['Cancel', 'Submit', 'Use SSH for origin instead', 'Password']);
  });

  it('a prompt the backend no longer knows is dropped, so the modal never sticks', async () => {
    authAnswer.mockImplementation(async () => { throw { kind: 'NotFound', message: 'no such prompt' }; });
    render(<AuthModal />);
    ask(9, 'Username: ', false);
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Cancel' })); });
    expect(authAnswer).toHaveBeenLastCalledWith(9, null);
    expect(screen.queryByRole('dialog')).toBeNull();
  });
});
