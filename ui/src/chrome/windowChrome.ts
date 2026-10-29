import { create } from 'zustand';
import { inTauri } from '../api/transport';

/** The edge or corner a resize starts from (Tauri's `ResizeDirection`). */
export type ResizeDirection = 'North' | 'NorthEast' | 'East' | 'SouthEast' | 'South' | 'SouthWest' | 'West' | 'NorthWest';

/**
 * The window's frame and how its title bar behaves, as the app hands it over (gitvolt-app
 * chrome.rs): `custom`, the page draws the title bar; `system`, the window manager's. The
 * clicks are GNOME's `action-*-click-titlebar` values.
 */
export interface WindowChromeInfo {
  frame: 'custom' | 'system';
  platform: 'linux' | 'macos' | 'windows';
  layout: string;
  doubleClick: string;
  middleClick: string;
  rightClick: string;
}

declare global {
  interface Window {
    /** Set before the page's own scripts run, by the app's initialization script (or an e2e
     * test's); absent in a plain browser and the website demo, which have no window to draw. */
    __GITVOLT_WINDOW__?: WindowChromeInfo;
    /** e2e and dev: the window calls a title bar made where there's no window to make them on. */
    __gbWindowCalls?: string[];
  }
}

/** Must match gitvolt-app's `chrome::CHROME_EVENT`. */
export const CHROME_EVENT = 'gb:window-chrome';

interface ChromeState {
  info: WindowChromeInfo | null;
  /** The window is maximized: the maximize button restores, and there are no resize edges. */
  maximized: boolean;
}

export const useWindowChrome = create<ChromeState>(() => ({
  info: typeof window === 'undefined' ? null : window.__GITVOLT_WINDOW__ ?? null,
  maximized: false,
}));

/** The page draws the title bar (and ☰ moves up into it). */
export const useCustomTitleBar = () => useWindowChrome((s) => s.info?.frame === 'custom');

/** What the title bar asks of the window. */
export interface WindowOps {
  startDragging(): Promise<void>;
  startResizeDragging(direction: ResizeDirection): Promise<void>;
  toggleMaximize(): Promise<void>;
  minimize(): Promise<void>;
  close(): Promise<void>;
  /** The window manager's window menu, or the window lowered; whether it was asked. */
  windowManager(request: 'menu' | 'lower'): Promise<boolean>;
  isMaximized(): Promise<boolean>;
  /** Calls `cb` after every resize, and with a new [`WindowChromeInfo`] when the desktop's
   * settings change; returns the unsubscribe. */
  watch(onResized: () => void, onChrome: (info: WindowChromeInfo) => void): () => void;
}

const win = async () => (await import('@tauri-apps/api/window')).getCurrentWindow();

const tauriOps: WindowOps = {
  startDragging: async () => { await (await import('@tauri-apps/api/window')).getCurrentWindow().startDragging(); },
  startResizeDragging: async (direction) => { await (await import('@tauri-apps/api/window')).getCurrentWindow().startResizeDragging(direction); },
  toggleMaximize: async () => { await (await import('@tauri-apps/api/window')).getCurrentWindow().toggleMaximize(); },
  minimize: async () => { await (await import('@tauri-apps/api/window')).getCurrentWindow().minimize(); },
  close: async () => {
    // As Quit does: the settings written first, then the window's own close (which still asks
    // about unsaved edits, `diff/workingCopy.ts`).
    const { flushSaves } = await import('../app/state');
    await flushSaves();
    await (await import('@tauri-apps/api/window')).getCurrentWindow().close();
  },
  windowManager: async (request) => (await import('@tauri-apps/api/core')).invoke<boolean>('window_manager', { request }),
  isMaximized: async () => (await win()).isMaximized(),
  watch(onResized, onChrome) {
    const offs: (() => void)[] = [];
    let dead = false;
    const keep = (off: () => void) => { if (dead) off(); else offs.push(off); };
    void win().then((w) => w.onResized(() => onResized())).then(keep)
      .catch((e: unknown) => console.warn('[gitvolt] window resize events unavailable', e));
    void import('@tauri-apps/api/event').then(({ listen }) => listen<WindowChromeInfo>(CHROME_EVENT, (e) => onChrome(e.payload))).then(keep)
      .catch((e: unknown) => console.warn(`[gitvolt] couldn't listen for ${CHROME_EVENT}`, e));
    return () => { dead = true; offs.splice(0).forEach((off) => off()); };
  },
};

/** No window (e2e, dev in a browser): each call is recorded, and the window menu isn't there. */
const recordingOps: WindowOps = {
  startDragging: async () => { record('startDragging'); },
  startResizeDragging: async (direction) => { record(`startResizeDragging:${direction}`); },
  toggleMaximize: async () => {
    record('toggleMaximize');
    useWindowChrome.setState((s) => ({ maximized: !s.maximized }));
  },
  minimize: async () => { record('minimize'); },
  close: async () => { record('close'); },
  windowManager: async (request) => { record(`windowManager:${request}`); return false; },
  isMaximized: async () => useWindowChrome.getState().maximized,
  watch: () => () => {},
};

function record(call: string): void {
  (window.__gbWindowCalls ??= []).push(call);
}

export const windowOps: WindowOps = inTauri() ? tauriOps : recordingOps;

/** Follows the window's maximized state and the desktop's title bar settings while a custom
 * title bar is up; returns the unsubscribe. Nothing runs between events. */
export function followWindow(ops: WindowOps = windowOps): () => void {
  const sync = () => { void ops.isMaximized().then((maximized) => { if (maximized !== useWindowChrome.getState().maximized) useWindowChrome.setState({ maximized }); }).catch(() => {}); };
  sync();
  return ops.watch(sync, (info) => useWindowChrome.setState({ info }));
}

/** A title bar click's action, GNOME's `action-*-click-titlebar` names. Horizontal and vertical
 * maximizing count as maximizing; `toggle-shade` and `none` (and anything else) do nothing. */
export function runTitleBarAction(action: string, ops: WindowOps = windowOps): Promise<unknown> | null {
  switch (action) {
    case 'toggle-maximize':
    case 'toggle-maximize-horizontally':
    case 'toggle-maximize-vertically':
      return ops.toggleMaximize();
    case 'minimize':
      return ops.minimize();
    case 'lower':
      return ops.windowManager('lower');
    case 'menu':
      return ops.windowManager('menu');
    default:
      return null;
  }
}
