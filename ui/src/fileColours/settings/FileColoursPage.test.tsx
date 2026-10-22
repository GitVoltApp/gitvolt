// @vitest-environment happy-dom
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { createStore } from 'zustand/vanilla';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../api/client', () => ({ api: { saveProfile: vi.fn(async () => null), saveSettings: vi.fn(async () => null) }, errorMessage: String }));
vi.mock('../../ui/Select', () => ({
  Select: ({ value, options, onChange, ...r }: { value: string; options: [string, string][]; onChange(v: string): void; 'aria-label'?: string }) => (
    <select aria-label={r['aria-label']} value={value} onChange={(e) => onChange(e.target.value)}>{options.map(([v, l]) => <option key={v} value={v}>{l}</option>)}</select>
  ),
}));
vi.mock('../../ui/colour/ColourSwatch', () => ({
  ColourSwatch: ({ label, onCommit }: { label: string; onCommit(v: string): void }) => <button type="button" aria-label={label} onClick={() => onCommit('#112233')} />,
}));

const { EMPTY_PROFILE, useAppState } = await import('../../app/state');
const { useRuntime } = await import('../../app/runtime');
const { useTabViews } = await import('../../app/tabStores');
const { useSettingsUi } = await import('../../settings/schema');
const { settingsPasteHandler } = await import('../../settings/paste');
const { useAddPatternRequest, requestAddPattern } = await import('../addRequest');
const { ArmLayer } = await import('../../ui/arm/ArmLayer');
const { disarm } = await import('../../ui/arm/store');
const { armClock, press } = await import('../../ui/arm/armTesting');
const { FileColoursPage } = await import('./FileColoursPage');
await import('./page');

const DIR = '/r/orbital/.git';
const sc = (id: string, patterns: string[], extra: Partial<import('../store').FileScope> = {}) => ({ id, name: id, tint: 'green', patterns, startCollapsed: false, ...extra });
const ctx = { repoPath: '/r/orbital', repoName: 'orbital', tabId: 't1' };
const stored = () => useAppState.getState().profile.fileColours[DIR];
const setStored = (list: ReturnType<typeof sc>[]) => act(() => useAppState.setState((s) => ({ profile: { ...s.profile, fileColours: { [DIR]: list } as Record<string, import('../store').FileScope[]> } })));
const key = (target: Element, k: string, init: KeyboardEventInit = {}) => fireEvent.keyDown(target, { key: k, ...init });
const names = () => stored().map((s) => s.name);

beforeEach(() => {
  useAppState.setState({ profile: { ...EMPTY_PROFILE, activeTab: 't1', tabs: [{ id: 't1', kind: 'repo', path: '/r/orbital', alias: null }] } });
  useRuntime.setState({ tabs: { t1: { status: 'ready', info: { commonDir: DIR } as never, repo: { id: 7, name: 'orbital', path: '/r/orbital' } as never } as never } });
  const panel = { sections: [{ title: null, spec: {}, list: { status: 'ready', data: { files: [{ path: 'a/tests/x.rs' }, { path: 'notes.md' }] } } }] };
  const store = createStore(() => ({ panel, graph: { openWorktree: null }, repoPath: '/r/orbital' }));
  const services = { worktreeFiles: { get: async () => ['a/tests/x.rs', 'b/tests/y.rs', 'README.md', 'c.rs'] } };
  useTabViews.setState({ views: { t1: { repo: 7, services: services as never, store: store as never } } });
  useAddPatternRequest.setState({ req: null });
});
afterEach(() => { cleanup(); act(() => disarm()); });

const page = () => render(<><FileColoursPage ctx={ctx} /><ArmLayer /></>);

describe('FileColoursPage', () => {
  it('asks for a repository when none is open', () => {
    useRuntime.setState({ tabs: {} });
    page();
    expect(screen.getByText('Open a repository to set its file colours.')).toBeTruthy();
  });

  it("lists the repository's scopes with their patterns and counts", async () => {
    setStored([sc('Tests', ['**/tests/**', '[bad']), sc('Docs', ['*.md'], { startCollapsed: true })]);
    page();
    const rows = screen.getAllByRole('listitem');
    expect(rows).toHaveLength(2);
    expect(within(rows[0]).getByText('**/tests/**')).toBeTruthy();
    expect(within(rows[0]).getByText('[bad').closest('.fc-chip')?.getAttribute('aria-invalid')).toBe('true');
    expect(within(rows[1]).getByText('starts collapsed')).toBeTruthy();
    await waitFor(() => expect(within(rows[0]).getByText('1 changed · 2 in repo')).toBeTruthy());
    expect(within(rows[1]).getByText('1 changed · 1 in repo')).toBeTruthy();
  });

  it('an empty list offers the starter scopes', () => {
    page();
    fireEvent.click(screen.getByRole('button', { name: 'Generated' }));
    expect(names()).toEqual(['Generated']);
    expect(stored()[0].startCollapsed).toBe(true);
  });

  it('+ New scope adds and opens a scope', () => {
    setStored([sc('Tests', ['a'])]);
    page();
    fireEvent.click(screen.getByRole('button', { name: '+ New scope' }));
    expect(names()).toEqual(['Tests', 'New scope']);
    expect(screen.getByLabelText('Scope name')).toBeTruthy();
  });

  it('✕ arms, then deletes; Esc disarms', async () => {
    const clock = armClock();
    const rects = vi.spyOn(HTMLElement.prototype, 'getClientRects').mockImplementation(function (this: HTMLElement) {
      return (this.isConnected ? [new DOMRect(10, 10, 24, 24)] : []) as unknown as DOMRectList;
    });
    try {
      setStored([sc('Tests', ['a']), sc('Docs', ['b'])]);
      page();
      press(screen.getByRole('button', { name: 'Delete Tests' }));
      const overlay = () => document.querySelector<HTMLElement>('.arm-overlay');
      expect(overlay()).toHaveProperty('textContent', 'Delete Tests');
      expect(overlay()?.classList.contains('tone-danger')).toBe(true);
      fireEvent.keyDown(document.body, { key: 'Escape' });
      await waitFor(() => expect(overlay()).toBeNull());
      expect(names()).toEqual(['Tests', 'Docs']);
      press(screen.getByRole('button', { name: 'Delete Tests' }));
      clock.settle();
      press(overlay()!);
      await waitFor(() => expect(names()).toEqual(['Docs']));
    } finally { rects.mockRestore(); clock.restore(); }
  });

  it('a request from the file list opens the page with the strip', () => {
    setStored([sc('Tests', ['a'])]);
    page();
    expect(screen.queryByRole('group', { name: 'Add pattern' })).toBeNull();
    act(() => requestAddPattern({ commonDir: DIR, pattern: '*.png', via: 'Colour *.png files…' }));
    expect(useSettingsUi.getState().open).toBe(true);
    expect(useSettingsUi.getState().page).toBe('file-colours');
    expect((screen.getByLabelText('Pattern') as HTMLInputElement).value).toBe('*.png');
    fireEvent.click(screen.getByRole('button', { name: 'New scope' }));
    expect(stored().at(-1)?.patterns).toEqual(['*.png']);
  });

  describe('editing', () => {
    const open = (list = [sc('Tests', ['**/tests/**', '*.snap'])]) => {
      setStored(list);
      page();
      fireEvent.click(screen.getByRole('button', { name: list[0].name }));
    };

    it('opening a row makes its name and patterns editable; Enter saves a rename, Esc reverts', () => {
      open();
      const name = screen.getByLabelText('Scope name') as HTMLInputElement;
      fireEvent.change(name, { target: { value: 'Specs' } });
      key(name, 'Escape');
      expect(name.value).toBe('Tests');
      fireEvent.change(name, { target: { value: 'Specs' } });
      key(name, 'Enter');
      expect(names()).toEqual(['Specs']);
    });

    it('adding, changing and removing a pattern saves it', () => {
      open();
      fireEvent.click(screen.getByRole('button', { name: '+ pattern' }));
      let input = screen.getByLabelText('New pattern');
      fireEvent.change(input, { target: { value: 'docs/**' } });
      key(input, 'Enter');
      expect(stored()[0].patterns).toEqual(['**/tests/**', '*.snap', 'docs/**']);
      fireEvent.click(screen.getByRole('button', { name: '*.snap' }));
      input = screen.getByLabelText('Pattern *.snap');
      fireEvent.change(input, { target: { value: '*.golden' } });
      key(input, 'Enter');
      expect(stored()[0].patterns).toEqual(['**/tests/**', '*.golden', 'docs/**']);
      fireEvent.click(screen.getByRole('button', { name: 'Remove **/tests/**' }));
      expect(stored()[0].patterns).toEqual(['*.golden', 'docs/**']);
    });

    it('a preset swatch sets the tint, the + swatch keeps a picked hex, the switch sets start collapsed', () => {
      open();
      fireEvent.click(screen.getByRole('button', { name: 'violet' }));
      expect(stored()[0].tint).toBe('violet');
      fireEvent.click(screen.getByRole('button', { name: 'Custom colour' }));
      expect(stored()[0].tint).toBe('#112233');
      fireEvent.click(screen.getByRole('switch', { name: 'Folders start collapsed in tree view' }));
      expect(stored()[0].startCollapsed).toBe(true);
    });
  });

  describe('reordering and paste', () => {
    const three = () => { setStored([sc('A', ['a']), sc('B', ['b']), sc('C', ['c'])]); page(); };

    it('Alt+↓ moves the focused scope down', () => {
      three();
      const handle = screen.getByRole('button', { name: 'Reorder A' });
      handle.focus();
      key(handle, 'ArrowDown', { altKey: true });
      expect(names()).toEqual(['B', 'A', 'C']);
    });

    it('dragging a handle reorders', () => {
      three();
      const rect = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
        const i = Number(this.dataset.sortableItem ?? 0);
        return { top: i * 40, bottom: i * 40 + 40, left: 0, right: 100, width: 100, height: 40, x: 0, y: i * 40, toJSON() {} } as DOMRect;
      });
      try {
        const handle = screen.getByRole('button', { name: 'Reorder A' });
        fireEvent.pointerDown(handle, { button: 0, clientX: 5, clientY: 20 });
        fireEvent(window, new MouseEvent('pointermove', { clientX: 5, clientY: 140 }));
        fireEvent(window, new MouseEvent('pointerup', { clientX: 5, clientY: 140 }));
        expect(names()).toEqual(['B', 'C', 'A']);
      } finally { rect.mockRestore(); }
    });

    it('Ctrl+V pastes scopes', () => {
      three();
      const json = JSON.stringify({ gitvolt: 'file-colours', version: 1, scopes: [{ name: 'A', tint: 'blue', patterns: ['*.x'], startCollapsed: false }] });
      act(() => settingsPasteHandler()?.(json));
      expect(names()).toEqual(['A', 'B', 'C', 'A (2)']);
    });
  });
});
