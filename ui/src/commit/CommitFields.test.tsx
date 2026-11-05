import { act, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { RepoContext } from '../app/repoContext';
import { EMPTY_REPO_SETTINGS, useAppState } from '../app/state';
import type { WipDraft } from './draft';
import { CommitFields } from './CommitFields';

function Harness({ onSubmit = () => {}, initial = { summary: 'Fix x', description: 'one\ntwo' }, path = '/r' }: { onSubmit?: () => void; initial?: WipDraft; path?: string }) {
  const [v, setV] = useState(initial);
  return (
    <RepoContext value={{ tabId: 't', repoId: 1, path, worktree: path, info: null }}>
      <CommitFields value={v} onChange={setV} onSubmit={onSubmit} />
    </RepoContext>
  );
}

const summary = () => screen.getByRole('textbox', { name: 'Commit summary' }) as HTMLInputElement;
const description = () => screen.getByRole('textbox', { name: 'Commit description' }) as HTMLTextAreaElement;

describe('the commit fields (spec #2 §8.1)', () => {
  it('spell-checks the message (the bundled en-US dictionary)', () => {
    render(<Harness />);
    expect([summary().getAttribute('spellcheck'), description().getAttribute('spellcheck')]).toEqual(['true', 'true']);
  });

  it('Enter or ↓ in the summary moves to the description, caret at its start', () => {
    render(<Harness />);
    summary().focus();
    fireEvent.keyDown(summary(), { key: 'Enter' });
    expect(document.activeElement).toBe(description());
    expect(description().selectionStart).toBe(0);
    summary().focus();
    fireEvent.keyDown(summary(), { key: 'ArrowDown' });
    expect(document.activeElement).toBe(description());
  });

  it('↑ on the description’s first line moves back to the summary, caret at its end; not from line 2', () => {
    render(<Harness />);
    description().focus();
    description().setSelectionRange(5, 5); // on line 2
    fireEvent.keyDown(description(), { key: 'ArrowUp' });
    expect(document.activeElement).toBe(description());
    description().setSelectionRange(1, 1);
    fireEvent.keyDown(description(), { key: 'ArrowUp' });
    expect(document.activeElement).toBe(summary());
    expect(summary().selectionStart).toBe('Fix x'.length);
  });

  it('Ctrl+Enter submits from either box; Esc blurs and keeps the text', () => {
    const onSubmit = vi.fn();
    render(<Harness onSubmit={onSubmit} />);
    fireEvent.keyDown(summary(), { key: 'Enter', ctrlKey: true });
    fireEvent.keyDown(description(), { key: 'Enter', ctrlKey: true });
    expect(onSubmit).toHaveBeenCalledTimes(2);
    description().focus();
    fireEvent.keyDown(description(), { key: 'Escape' });
    expect(document.activeElement).not.toBe(description());
    expect(description().value).toBe('one\ntwo');
  });

  it('the counter shows past 60 and warns past 72; nothing is truncated', () => {
    render(<Harness initial={{ summary: 'x'.repeat(61), description: '' }} />);
    expect(screen.getByTestId('commit-counter')).toHaveTextContent('61');
    fireEvent.change(summary(), { target: { value: 'x'.repeat(90) } });
    expect(screen.getByTestId('commit-counter')).toHaveClass('warn');
    expect(summary().value).toHaveLength(90);
  });
});

describe('gitmoji in the summary', () => {
  // The setting is the repository's: on for `/r` only.
  const gitmoji = (on: boolean) => act(() => useAppState.setState((s) => ({ profile: { ...s.profile, repos: { ...s.profile.repos, '/r': { ...EMPTY_REPO_SETTINGS, gitmoji: on || undefined } } } })));
  const type = (text: string) => fireEvent.change(summary(), { target: { value: text } });
  const options = () => screen.getAllByRole('option').map((o) => o.textContent);
  afterEach(() => gitmoji(false));

  it('off (the default), or on for another repository: a colon is just a colon', async () => {
    const { unmount } = render(<Harness initial={{ summary: '', description: '' }} />);
    summary().focus();
    type(':bug:');
    await act(async () => {});
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(summary().value).toBe(':bug:');
    unmount();
    gitmoji(true);
    render(<Harness initial={{ summary: '', description: '' }} path="/other" />);
    type(':');
    await act(async () => {});
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('a colon first opens the picker; typing narrows it to the codes starting with it; Enter inserts the emoji', async () => {
    gitmoji(true);
    render(<Harness initial={{ summary: '', description: '' }} />);
    summary().focus();
    type(':');
    expect(await screen.findByRole('listbox', { name: 'Gitmoji' })).toBeInTheDocument();
    expect(screen.getAllByRole('option').length).toBeGreaterThan(60);
    type(':bu');
    expect(options()).toEqual(expect.arrayContaining([expect.stringContaining(':bug:'), expect.stringContaining(':building_construction:')]));
    expect(options().every((o) => o?.includes(':bu'))).toBe(true);
    fireEvent.keyDown(summary(), { key: 'Enter' });
    expect(summary().value).toBe('🐛 ');
    expect(document.activeElement).toBe(summary());
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('after a space too, not inside a word; Esc closes the picker without leaving the summary', async () => {
    gitmoji(true);
    render(<Harness initial={{ summary: '', description: '' }} />);
    summary().focus();
    type('at 12:3');
    await act(async () => {});
    expect(screen.queryByRole('listbox')).toBeNull();
    type('fix :ar');
    expect(await screen.findByRole('listbox', { name: 'Gitmoji' })).toBeInTheDocument();
    fireEvent.keyDown(summary(), { key: 'Escape' });
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(document.activeElement).toBe(summary());
  });

  it('a whole :code: typed becomes its emoji', async () => {
    gitmoji(true);
    render(<Harness initial={{ summary: '', description: '' }} />);
    summary().focus();
    type('fix :');
    await screen.findByRole('listbox', { name: 'Gitmoji' });
    type('fix :bug:');
    expect(summary().value).toBe('fix 🐛');
    expect(summary().selectionStart).toBe('fix 🐛'.length);
  });

  it('in the description too: the picker opens on a later line, Enter inserts there, not a newline', async () => {
    gitmoji(true);
    render(<Harness initial={{ summary: 'Fix x', description: '' }} />);
    description().focus();
    fireEvent.change(description(), { target: { value: 'first line\n- :spa' } });
    expect(await screen.findByRole('listbox', { name: 'Gitmoji' })).toBeInTheDocument();
    expect(options().every((o) => o?.includes(':spa'))).toBe(true);
    fireEvent.keyDown(description(), { key: 'Enter' });
    expect(description().value).toMatch(/^first line\n- \S+ $/u);
    expect(description().value).not.toContain(':spa');
    expect(document.activeElement).toBe(description());
  });

  it('a caret at the window’s right edge: the picker shifts left to stay on screen', async () => {
    gitmoji(true);
    const rect = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      return (this.getAttribute('role') === 'listbox' ? new DOMRect(0, 0, 240, 200) : new DOMRect(window.innerWidth - 20, 100, 300, 24));
    });
    try {
      render(<Harness initial={{ summary: '', description: '' }} />);
      summary().focus();
      type(':');
      const list = await screen.findByRole('listbox', { name: 'Gitmoji' });
      expect(parseFloat(list.style.left) + 240).toBeLessThanOrEqual(window.innerWidth);
    } finally {
      rect.mockRestore();
    }
  });

  it('closes while the summary hasn’t focus, and is back with it', async () => {
    gitmoji(true);
    render(<Harness initial={{ summary: '', description: '' }} />);
    summary().focus();
    type(':');
    await screen.findByRole('listbox', { name: 'Gitmoji' });
    act(() => summary().blur());
    expect(screen.queryByRole('listbox')).toBeNull();
    act(() => summary().focus());
    expect(screen.getByRole('listbox', { name: 'Gitmoji' })).toBeInTheDocument();
  });

  it('follows the summary when the window resizes', async () => {
    gitmoji(true);
    let x = 100;
    const rect = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      return (this.getAttribute('role') === 'listbox' ? new DOMRect(0, 0, 240, 200) : new DOMRect(x, 100, 300, 24));
    });
    try {
      render(<Harness initial={{ summary: '', description: '' }} />);
      summary().focus();
      type(':');
      const list = await screen.findByRole('listbox', { name: 'Gitmoji' });
      expect(list.style.left).toBe('100px');
      x = 40;
      act(() => { window.dispatchEvent(new Event('resize')); });
      expect(list.style.left).toBe('40px');
    } finally {
      rect.mockRestore();
    }
  });

  it('in a short window the list gets shorter, to stay on screen', async () => {
    gitmoji(true);
    const height = window.innerHeight;
    Object.defineProperty(window, 'innerHeight', { value: 250, configurable: true });
    const rect = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      return (this.getAttribute('role') === 'listbox' ? new DOMRect(0, 0, 240, 200) : new DOMRect(100, 100, 300, 24));
    });
    try {
      render(<Harness initial={{ summary: '', description: '' }} />);
      summary().focus();
      type(':');
      const list = await screen.findByRole('listbox', { name: 'Gitmoji' });
      const max = parseFloat(list.style.maxHeight);
      expect(max).toBeLessThan(200);
      expect(parseFloat(list.style.top) + max).toBeLessThanOrEqual(250);
    } finally {
      rect.mockRestore();
      Object.defineProperty(window, 'innerHeight', { value: height, configurable: true });
    }
  });

  it('the pointer takes the choice only when it moves, not when the list scrolls under it', async () => {
    gitmoji(true);
    render(<Harness initial={{ summary: '', description: '' }} />);
    summary().focus();
    type(':');
    await screen.findByRole('listbox', { name: 'Gitmoji' });
    fireEvent.keyDown(summary(), { key: 'ArrowDown' });
    fireEvent.keyDown(summary(), { key: 'ArrowDown' });
    const selected = () => screen.getAllByRole('option').findIndex((o) => o.getAttribute('aria-selected') === 'true');
    expect(selected()).toBe(2);
    const [first, second] = screen.getAllByRole('option');
    fireEvent.mouseMove(first, { screenX: 5, screenY: 5 });
    fireEvent.mouseMove(first, { screenX: 5, screenY: 5 });
    expect(selected()).toBe(2);
    fireEvent.mouseMove(second, { screenX: 5, screenY: 30 });
    expect(selected()).toBe(1);
  });

  it('Enter while an input method composes is the composition’s, not the picker’s', async () => {
    gitmoji(true);
    render(<Harness initial={{ summary: '', description: '' }} />);
    summary().focus();
    type(':bu');
    await screen.findByRole('listbox', { name: 'Gitmoji' });
    fireEvent.keyDown(summary(), { key: 'Enter', isComposing: true });
    fireEvent.keyDown(summary(), { key: 'Enter', keyCode: 229 });
    expect(summary().value).toBe(':bu');
    expect(screen.getByRole('listbox', { name: 'Gitmoji' })).toBeInTheDocument();
  });

  it('a paste with a colon in it doesn’t open the picker; typing one after it does', async () => {
    gitmoji(true);
    render(<Harness initial={{ summary: '', description: '' }} />);
    summary().focus();
    fireEvent.input(summary(), { target: { value: 'fix :' }, inputType: 'insertFromPaste' });
    await act(async () => {});
    expect(screen.queryByRole('listbox')).toBeNull();
    type('fix : :');
    expect(await screen.findByRole('listbox', { name: 'Gitmoji' })).toBeInTheDocument();
  });

  it('inserts as typing does where the box can (so Cmd+Z takes it back): the picked emoji, a whole :code:', async () => {
    gitmoji(true);
    const typed: [string, number | null, number | null][] = [];
    Object.defineProperty(document, 'execCommand', {
      configurable: true,
      value: vi.fn((_: string, __: boolean, text: string) => { typed.push([text, summary().selectionStart, summary().selectionEnd]); return true; }),
    });
    try {
      render(<Harness initial={{ summary: '', description: '' }} />);
      summary().focus();
      type('x :bu');
      await screen.findByRole('listbox', { name: 'Gitmoji' });
      fireEvent.keyDown(summary(), { key: 'Enter' });
      expect(typed).toEqual([['🐛 ', 2, 5]]);
      expect(document.execCommand).toHaveBeenCalledWith('insertText', false, '🐛 ');
      type('fix :bug:');
      expect(summary().value).toBe('fix :bug:');
      await act(async () => {});
      expect(typed[1]).toEqual(['🐛', 4, 9]);
    } finally {
      delete (document as { execCommand?: unknown }).execCommand;
    }
  });
});

