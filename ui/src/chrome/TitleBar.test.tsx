import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { WindowChromeInfo } from './windowChrome';

vi.mock('../api/client', () => ({
  api: { saveProfile: vi.fn(async () => null), saveSettings: vi.fn(async () => null), appInfo: vi.fn(async () => ({ appVersion: 'x', gitVersion: 'y' })) },
  errorMessage: (e: unknown) => String(e),
}));
vi.mock('../api/transport', () => ({ copyText: vi.fn(async () => {}), inTauri: () => false }));

const { TitleBar, fallbackWindowMenu } = await import('./TitleBar');
const { useWindowChrome } = await import('./windowChrome');
const { useMenu } = await import('../menu/menuStore');
const { useAppState, EMPTY_PROFILE } = await import('../app/state');
const { TabBar } = await import('../tabs/TabBar');

const linux = (layout: string, more: Partial<WindowChromeInfo> = {}): WindowChromeInfo => ({ frame: 'custom', platform: 'linux', layout, doubleClick: 'toggle-maximize', middleClick: 'lower', rightClick: 'menu', ...more });
const calls = () => window.__gbWindowCalls ?? [];
const bar = () => document.querySelector<HTMLElement>('.title-bar')!;
const title = () => document.querySelector<HTMLElement>('.title-bar-title span')!;
const show = (info: WindowChromeInfo | null) => { act(() => useWindowChrome.setState({ info, maximized: false })); return render(<TitleBar />); };

beforeEach(() => { document.title = 'GitVolt — orbital'; window.__gbWindowCalls = []; });
afterEach(() => { act(() => { useWindowChrome.setState({ info: null, maximized: false }); useMenu.getState().close(); }); });

describe('the title bar', () => {
  it("isn't there without a window to draw it for, or with the system's", () => {
    show(null);
    expect(bar()).toBeNull();
    show({ ...linux('appmenu:close'), frame: 'system' });
    expect(bar()).toBeNull();
    expect(document.querySelector('.window-edges')).toBeNull();
  });

  it("stock GNOME: ☰, the window's title, and only close", () => {
    show(linux('appmenu:close'));
    const buttons = within(bar()).getAllByRole('button');
    expect(buttons.map((b) => b.getAttribute('aria-label'))).toEqual(['Menu', 'Close']);
    expect(title()).toHaveTextContent('GitVolt — orbital');
  });

  it('follows the title as it changes', async () => {
    show(linux('appmenu:close'));
    act(() => { document.title = 'GitVolt — casino'; });
    await waitFor(() => expect(title()).toHaveTextContent('GitVolt — casino'));
  });

  it('a left-side layout puts the buttons first, with ☰ right of them', () => {
    show(linux('close,minimize,maximize:appmenu'));
    expect(within(bar()).getAllByRole('button').map((b) => b.getAttribute('aria-label'))).toEqual(['Close', 'Minimize', 'Maximize', 'Menu']);
  });

  it('the buttons ask the window, and maximize turns into restore', () => {
    show(linux(':minimize,maximize,close'));
    fireEvent.click(screen.getByRole('button', { name: 'Minimize' }));
    fireEvent.click(screen.getByRole('button', { name: 'Maximize' }));
    fireEvent.click(screen.getByRole('button', { name: 'Restore' }));
    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    return waitFor(() => expect(calls()).toEqual(['minimize', 'toggleMaximize', 'toggleMaximize', 'close']));
  });

  it("a window button doesn't take the focus", () => {
    show(linux('appmenu:close'));
    const down = new MouseEvent('mousedown', { bubbles: true, cancelable: true, button: 0 });
    screen.getByRole('button', { name: 'Close' }).dispatchEvent(down);
    expect(down.defaultPrevented).toBe(true);
  });

  it('dragging its empty space or its title moves the window; a button never does', () => {
    show(linux('appmenu:close'));
    fireEvent.mouseDown(title(), { button: 0, clientX: 300, clientY: 10 });
    fireEvent.mouseMove(document, { buttons: 1, clientX: 330, clientY: 12 });
    expect(calls()).toEqual(['startDragging']);
    fireEvent.mouseDown(screen.getByRole('button', { name: 'Close' }), { button: 0, clientX: 500, clientY: 10 });
    fireEvent.mouseMove(document, { buttons: 1, clientX: 530, clientY: 12 });
    fireEvent.mouseUp(document);
    fireEvent.mouseDown(screen.getByRole('button', { name: 'Menu' }), { button: 0, clientX: 10, clientY: 10 });
    fireEvent.mouseMove(document, { buttons: 1, clientX: 40, clientY: 12 });
    expect(calls()).toEqual(['startDragging']);
  });

  it("a double click does the desktop's double-click action, on empty space only", () => {
    show(linux('appmenu:close'));
    fireEvent.doubleClick(title());
    fireEvent.doubleClick(screen.getByRole('button', { name: 'Close' }));
    expect(calls()).toEqual(['toggleMaximize']);
    act(() => useWindowChrome.setState({ info: linux('appmenu:close', { doubleClick: 'minimize' }) }));
    fireEvent.doubleClick(bar());
    expect(calls()).toEqual(['toggleMaximize', 'minimize']);
  });

  it('a middle click lowers the window (or what the desktop says); none does nothing', () => {
    show(linux('appmenu:close'));
    fireEvent(title(), new MouseEvent('auxclick', { bubbles: true, button: 1 }));
    expect(calls()).toEqual(['windowManager:lower']);
    act(() => useWindowChrome.setState({ info: linux('appmenu:close', { middleClick: 'none' }) }));
    fireEvent(title(), new MouseEvent('auxclick', { bubbles: true, button: 1 }));
    expect(calls()).toEqual(['windowManager:lower']);
  });

  it("a right click asks for the window manager's menu, and shows GitVolt's when it has none", async () => {
    show(linux('appmenu:close'));
    fireEvent.contextMenu(title(), { clientX: 200, clientY: 12 });
    expect(calls()).toEqual(['windowManager:menu']);
    await waitFor(() => expect(useMenu.getState().rows?.map((r) => ('label' in r ? r.label : r.kind))).toEqual(['Minimize', 'Maximize', 'separator', 'Close']));
    expect(useMenu.getState()).toMatchObject({ x: 200, y: 12 });
  });

  it("a right click on a button is the button's, not the title bar's", () => {
    show(linux('appmenu:close'));
    fireEvent.contextMenu(screen.getByRole('button', { name: 'Close' }));
    expect(calls()).toEqual([]);
  });

  it('the fallback menu runs on the window and says Restore when maximized', () => {
    const ops = { minimize: vi.fn(async () => {}), toggleMaximize: vi.fn(async () => {}), close: vi.fn(async () => {}) };
    const rows = fallbackWindowMenu(true, ops as never);
    expect(rows.map((r) => ('label' in r ? r.label : r.kind))).toEqual(['Minimize', 'Restore', 'separator', 'Close']);
    for (const r of rows) if (r.kind === 'action') r.run();
    expect([ops.minimize, ops.toggleMaximize, ops.close].map((f) => f.mock.calls.length)).toEqual([1, 1, 1]);
  });
});

describe('the resize edges', () => {
  it('each edge and corner resizes from where it is; none while maximized', () => {
    show(linux('appmenu:close'));
    const edges = [...document.querySelectorAll<HTMLElement>('.window-edge')];
    expect(edges.map((e) => e.dataset.direction)).toEqual(['North', 'NorthEast', 'East', 'SouthEast', 'South', 'SouthWest', 'West', 'NorthWest']);
    fireEvent.mouseDown(document.querySelector('.window-edge-se')!, { button: 0 });
    fireEvent.mouseDown(document.querySelector('.window-edge-w')!, { button: 2 });
    expect(calls()).toEqual(['startResizeDragging:SouthEast']);
    act(() => useWindowChrome.setState({ maximized: true }));
    expect(document.querySelector('.window-edges')).toBeNull();
  });
});

describe('the tab bar under the title bar', () => {
  beforeEach(() => act(() => useAppState.setState({ loaded: true, profile: { ...EMPTY_PROFILE, id: 'default', tabs: [{ id: 'a', kind: 'repo', path: '/a', alias: null }], activeTab: 'a' } })));
  afterEach(() => act(() => useAppState.setState({ profile: EMPTY_PROFILE })));

  it('☰ is in the title bar, and the strip\'s empty space moves the window, its tabs never', () => {
    act(() => useWindowChrome.setState({ info: linux('appmenu:close') }));
    render(<TabBar />);
    expect(document.querySelector('.tab-bar')!.querySelector('[aria-label="Menu"]')).toBeNull();
    const strip = screen.getByRole('tablist');
    fireEvent.mouseDown(screen.getByRole('tab'), { button: 0, clientX: 40, clientY: 40 });
    fireEvent.mouseMove(document, { buttons: 1, clientX: 80, clientY: 40 });
    fireEvent.mouseUp(document);
    expect(calls()).toEqual([]);
    fireEvent.mouseDown(strip, { button: 0, clientX: 600, clientY: 40 });
    fireEvent.mouseMove(document, { buttons: 1, clientX: 640, clientY: 40 });
    expect(calls()).toEqual(['startDragging']);
  });

  it('with the system title bar, ☰ stays in the tab bar and the strip moves nothing', () => {
    render(<TabBar />);
    expect(document.querySelector('.tab-bar')!.querySelector('[aria-label="Menu"]')).not.toBeNull();
    fireEvent.mouseDown(screen.getByRole('tablist'), { button: 0, clientX: 600, clientY: 40 });
    fireEvent.mouseMove(document, { buttons: 1, clientX: 640, clientY: 40 });
    fireEvent.mouseUp(document);
    expect(calls()).toEqual([]);
  });
});
