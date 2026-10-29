// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { followWindow, runTitleBarAction, useWindowChrome, windowOps, type WindowChromeInfo, type WindowOps } from './windowChrome';

const fakeOps = () => ({
  startDragging: vi.fn(async () => {}),
  startResizeDragging: vi.fn(async () => {}),
  toggleMaximize: vi.fn(async () => {}),
  minimize: vi.fn(async () => {}),
  close: vi.fn(async () => {}),
  windowManager: vi.fn(async () => true),
  isMaximized: vi.fn(async () => true),
  isFullscreen: vi.fn(async () => false),
  watch: vi.fn((_resized: () => void, _chrome: (i: WindowChromeInfo) => void) => () => {}),
}) satisfies WindowOps;

afterEach(() => { useWindowChrome.setState({ info: null, maximized: false, fullscreen: false }); window.__gbWindowCalls = undefined; });

describe("a title bar click's action (GNOME's action-*-click-titlebar)", () => {
  it.each([
    ['toggle-maximize', 'toggleMaximize', []],
    ['toggle-maximize-horizontally', 'toggleMaximize', []],
    ['toggle-maximize-vertically', 'toggleMaximize', []],
    ['minimize', 'minimize', []],
    ['lower', 'windowManager', ['lower']],
    ['menu', 'windowManager', ['menu']],
  ] as const)('%s', (action, method, args) => {
    const ops = fakeOps();
    void runTitleBarAction(action, ops);
    expect(ops[method]).toHaveBeenCalledWith(...args);
  });

  it('none, toggle-shade and anything unknown do nothing', () => {
    const ops = fakeOps();
    for (const action of ['none', 'toggle-shade', 'bogus', '']) expect(runTitleBarAction(action, ops)).toBeNull();
    for (const fn of Object.values(ops)) expect(fn).not.toHaveBeenCalled();
  });
});

describe('following the window', () => {
  it('reads the maximized state now and after each resize, and takes new desktop settings', async () => {
    const ops = fakeOps();
    const off = followWindow(ops);
    await vi.waitFor(() => expect(useWindowChrome.getState().maximized).toBe(true));
    const [resized, chrome] = ops.watch.mock.calls[0];
    ops.isMaximized.mockResolvedValue(false);
    resized();
    await vi.waitFor(() => expect(useWindowChrome.getState().maximized).toBe(false));
    const info: WindowChromeInfo = { frame: 'custom', platform: 'linux', layout: 'close:', doubleClick: 'minimize', middleClick: 'none', rightClick: 'menu' };
    chrome(info);
    expect(useWindowChrome.getState().info).toEqual(info);
    off();
  });

  it('follows full screen the same way: in after a resize, and out after the one that ends it', async () => {
    const ops = fakeOps();
    ops.isMaximized.mockResolvedValue(false);
    const off = followWindow(ops);
    const [resized] = ops.watch.mock.calls[0];
    ops.isFullscreen.mockResolvedValue(true);
    resized();
    await vi.waitFor(() => expect(useWindowChrome.getState().fullscreen).toBe(true));
    ops.isFullscreen.mockResolvedValue(false);
    resized();
    await vi.waitFor(() => expect(useWindowChrome.getState().fullscreen).toBe(false));
    expect(useWindowChrome.getState().maximized).toBe(false);
    off();
  });
});

describe('with no window (a browser, e2e)', () => {
  it('records each call, and has no window menu', async () => {
    await windowOps.startDragging();
    await windowOps.startResizeDragging('SouthEast');
    await windowOps.toggleMaximize();
    expect(useWindowChrome.getState().maximized).toBe(true);
    await windowOps.minimize();
    await windowOps.close();
    expect(await windowOps.windowManager('menu')).toBe(false);
    expect(window.__gbWindowCalls).toEqual(['startDragging', 'startResizeDragging:SouthEast', 'toggleMaximize', 'minimize', 'close', 'windowManager:menu']);
  });
});
