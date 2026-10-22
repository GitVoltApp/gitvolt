// @vitest-environment happy-dom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { FileScope } from '../store';

const clip = vi.hoisted(() => ({ text: '' }));
vi.mock('../../api/transport', () => ({
  copyText: vi.fn(async (t: string) => { clip.text = t; }),
  readClipboardText: vi.fn(async () => clip.text),
}));
vi.mock('../../api/client', () => ({ api: { saveProfile: vi.fn(async () => null) }, errorMessage: (e: unknown) => String(e) }));

const { useAppState, EMPTY_PROFILE } = await import('../../app/state');
const { useMenu } = await import('../../menu/menuStore');
const { useToast } = await import('../../ui/toastStore');
const { ShareBar } = await import('./ShareBar');

const sc = (id: string, patterns: string[]): FileScope => ({ id, name: id, tint: 'green', patterns, startCollapsed: false });
const flush = () => act(async () => { await Promise.resolve(); await Promise.resolve(); });
const toast = () => useToast.getState().message;

beforeEach(() => {
  clip.text = '';
  act(() => { useAppState.setState({ profile: EMPTY_PROFILE }); useToast.getState().dismiss(); useMenu.getState().close(); });
});
afterEach(cleanup);

describe('ShareBar', () => {
  it('Copy as JSON puts the scopes on the clipboard', async () => {
    render(<ShareBar commonDir="/r/.git" scopes={[sc('A', ['a'])]} onChange={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Copy as JSON' }));
    await flush();
    expect(JSON.parse(clip.text)).toMatchObject({ gitvolt: 'file-colours', version: 1, scopes: [{ name: 'A', patterns: ['a'] }] });
    expect(toast()).toBe('Copied 1 scope');
  });

  it('Paste scopes appends and renames clashes', async () => {
    clip.text = '{"gitvolt":"file-colours","version":1,"scopes":[{"name":"A","tint":"blue","patterns":["b"]}]}';
    const onChange = vi.fn();
    render(<ShareBar commonDir="/r/.git" scopes={[sc('A', ['a'])]} onChange={onChange} />);
    fireEvent.click(screen.getByRole('button', { name: 'Paste scopes' }));
    await flush();
    expect(onChange.mock.calls[0][0].map((s: FileScope) => s.name)).toEqual(['A', 'A (2)']);
  });

  it('pasting something else only shows a toast', async () => {
    clip.text = 'hello';
    const onChange = vi.fn();
    render(<ShareBar commonDir="/r/.git" scopes={[]} onChange={onChange} />);
    fireEvent.click(screen.getByRole('button', { name: 'Paste scopes' }));
    await flush();
    expect(onChange).not.toHaveBeenCalled();
    expect(toast()).toBe("That isn't GitVolt file colours");
  });

  it("Copy from another repository lists the others and appends the chosen one's", () => {
    const onChange = vi.fn();
    act(() => useAppState.setState({ profile: { ...EMPTY_PROFILE, fileColours: { '/r/.git': [sc('A', ['a'])], '/h/orbital/.git': [sc('Docs', ['docs/**']), sc('Gen', ['g'])] } } }));
    render(<ShareBar commonDir="/r/.git" scopes={[sc('A', ['a'])]} onChange={onChange} />);
    fireEvent.click(screen.getByRole('button', { name: 'Copy from another repository…' }));
    const rows = useMenu.getState().rows!;
    expect(rows.map((r) => (r.kind === 'action' ? r.label : ''))).toEqual(['orbital (2 scopes)']);
    act(() => { const r = rows[0]; if (r.kind === 'action') r.run(); });
    expect(onChange.mock.calls[0][0].map((s: FileScope) => s.name)).toEqual(['A', 'Docs', 'Gen']);
  });

  it('disables Copy from another repository when none has scopes', () => {
    render(<ShareBar commonDir="/r/.git" scopes={[]} onChange={vi.fn()} />);
    expect((screen.getByRole('button', { name: 'Copy from another repository…' }) as HTMLButtonElement).disabled).toBe(true);
  });
});
