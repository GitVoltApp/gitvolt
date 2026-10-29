import { Maximize2, Minimize2, Minus, X } from 'lucide-react';
import { Fragment, useEffect, useSyncExternalStore, type MouseEvent as ReactMouseEvent } from 'react';
import { openContextMenu } from '../menu/menuStore';
import type { MenuRow } from '../menu/types';
import { HamburgerMenu } from '../tabs/HamburgerMenu';
import { buttonSets, type WindowButton } from './buttonLayout';
import { beginWindowDrag, isDragTarget } from './dragRegion';
import { ResizeEdges } from './ResizeEdges';
import { followWindow, runTitleBarAction, useWindowChrome, windowOps, type WindowOps } from './windowChrome';
import './chrome.css';

/** The page's title (the window's), followed as it changes; nothing runs in between. */
function subscribeTitle(changed: () => void): () => void {
  const observer = new MutationObserver(changed);
  observer.observe(document.head, { subtree: true, childList: true, characterData: true });
  return () => observer.disconnect();
}
const useDocumentTitle = () => useSyncExternalStore(subscribeTitle, () => document.title);

/** What a right click shows when the window manager has no window menu of its own to show. */
export function fallbackWindowMenu(maximized: boolean, ops: WindowOps = windowOps): MenuRow[] {
  return [
    { kind: 'action', id: 'window.minimize', label: 'Minimize', icon: Minus, tooltip: 'Minimize the window', run: () => void ops.minimize() },
    { kind: 'action', id: 'window.maximize', label: maximized ? 'Restore' : 'Maximize', icon: maximized ? Minimize2 : Maximize2, tooltip: maximized ? 'Restore the window to its size' : 'Maximize the window', run: () => void ops.toggleMaximize() },
    { kind: 'separator' },
    { kind: 'action', id: 'window.close', label: 'Close', icon: X, tooltip: 'Close the window', run: () => void ops.close() },
  ];
}

/** The title bar's own mouse handling: its empty space and title drag the window, and a double,
 * middle or right click does what the desktop says a title bar click does. */
export function titleBarMouse(ops: WindowOps = windowOps) {
  const own = (e: ReactMouseEvent<HTMLElement>) => isDragTarget(e.target, e.currentTarget);
  return {
    onMouseDown(e: ReactMouseEvent<HTMLElement>) {
      if (!own(e)) return;
      // No focus change or text selection, and no autoscroll on a middle press.
      e.preventDefault();
      beginWindowDrag(e, ops);
    },
    onDoubleClick(e: ReactMouseEvent<HTMLElement>) {
      if (!own(e)) return;
      void runTitleBarAction(useWindowChrome.getState().info?.doubleClick ?? 'toggle-maximize', ops);
    },
    onAuxClick(e: ReactMouseEvent<HTMLElement>) {
      if (e.button !== 1 || !own(e)) return;
      void runTitleBarAction(useWindowChrome.getState().info?.middleClick ?? 'none', ops);
    },
    onContextMenu(e: ReactMouseEvent<HTMLElement>) {
      if (!own(e)) return;
      e.preventDefault();
      e.stopPropagation();
      const action = useWindowChrome.getState().info?.rightClick ?? 'menu';
      if (action !== 'menu') {
        void runTitleBarAction(action, ops);
        return;
      }
      // The window manager's own menu; GitVolt's where it has none.
      const at = { clientX: e.clientX, clientY: e.clientY, timeStamp: e.timeStamp, preventDefault() {}, stopPropagation() {} };
      void ops.windowManager('menu').then((shown) => {
        if (!shown) openContextMenu(at, () => fallbackWindowMenu(useWindowChrome.getState().maximized, ops));
      }, () => openContextMenu(at, () => fallbackWindowMenu(useWindowChrome.getState().maximized, ops)));
    },
  };
}

const LABELS: Record<Exclude<WindowButton, 'spacer'>, (maximized: boolean) => string> = {
  minimize: () => 'Minimize',
  maximize: (maximized) => (maximized ? 'Restore' : 'Maximize'),
  close: () => 'Close',
};

function ButtonIcon({ button, maximized, platform }: { button: Exclude<WindowButton, 'spacer'>; maximized: boolean; platform: string }) {
  // Windows' glyphs fill the 10 px box with a 1 px line: its restore is two squares, the front
  // one at the bottom left.
  if (platform === 'windows') {
    return (
      <svg viewBox="0 0 10 10" aria-hidden>
        {button === 'minimize' && <path d="M0 5.5h10" />}
        {button === 'maximize' && !maximized && <rect x="0.5" y="0.5" width="9" height="9" />}
        {button === 'maximize' && maximized && <><rect x="0.5" y="2.5" width="7" height="7" /><path d="M2.5 2.5v-2h7v7h-2" /></>}
        {button === 'close' && <path d="M0 0l10 10M10 0L0 10" />}
      </svg>
    );
  }
  return (
    <svg viewBox="0 0 10 10" aria-hidden>
      {button === 'minimize' && <path d="M2 5h6" />}
      {button === 'maximize' && !maximized && <rect x="2" y="2" width="6" height="6" />}
      {button === 'maximize' && maximized && <><rect x="1.5" y="3.5" width="5" height="5" /><path d="M3.5 3.5V1.5h5v5h-2" /></>}
      {button === 'close' && <path d="M2 2l6 6M8 2l-6 6" />}
    </svg>
  );
}

function WindowButtons({ buttons, maximized, platform }: { buttons: WindowButton[]; maximized: boolean; platform: string }) {
  if (!buttons.length) return null;
  const run: Record<Exclude<WindowButton, 'spacer'>, () => Promise<unknown>> = {
    minimize: () => windowOps.minimize(),
    maximize: () => windowOps.toggleMaximize(),
    close: () => windowOps.close(),
  };
  return (
    <div className="title-bar-buttons">
      {buttons.map((b, i) => (b === 'spacer' ? <span key={`spacer-${i}`} className="title-bar-spacer" /> : (
        <button
          key={b}
          type="button"
          className={`wm-btn wm-${b}`}
          aria-label={LABELS[b](maximized)}
          // A window button doesn't take the focus from where it is, as the window manager's don't.
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => void run[b]().catch((err: unknown) => console.warn(`[gitvolt] window ${b} failed`, err))}
        >
          <ButtonIcon button={b} maximized={maximized} platform={platform} />
        </button>
      )))}
    </div>
  );
}

/**
 * GitVolt's own title bar (Linux, Windows and macOS, unless Settings › Appearance says to use the
 * system's): ☰ at its start, after any window buttons the desktop puts on the left; the window's
 * title in the middle; the window buttons at the end, as GNOME's `button-layout` has them
 * (Windows' three, flat, on the right). On macOS, Apple's traffic lights come first (the window
 * draws them over the bar's slot; full screen hides them, and the slot goes too), and nothing is
 * at the end. Its empty space and title move the window, and the window's edges resize it
 * (`ResizeEdges`; on Windows only the top, the system's border doing the rest; macOS keeps its
 * own frame). Nothing when the window has the system's title bar, or no window at all (a browser,
 * the website demo).
 */
export function TitleBar() {
  const info = useWindowChrome((s) => s.info);
  const maximized = useWindowChrome((s) => s.maximized);
  const fullscreen = useWindowChrome((s) => s.fullscreen);
  const custom = info?.frame === 'custom';
  const title = useDocumentTitle();
  useEffect(() => (custom ? followWindow() : undefined), [custom]);
  if (!custom) return null;
  const { start, end } = buttonSets(info.platform, info.layout);
  return (
    <Fragment>
      <div className={`title-bar platform-${info.platform}`} {...titleBarMouse()}>
        {/* macOS draws its own traffic lights over the window's corner: their slot. */}
        {info.platform === 'macos' && !fullscreen && <span className="title-bar-lights" />}
        <WindowButtons buttons={start} maximized={maximized} platform={info.platform} />
        <HamburgerMenu />
        <div className="title-bar-title"><span>{title}</span></div>
        <WindowButtons buttons={end} maximized={maximized} platform={info.platform} />
      </div>
      {info.platform !== 'macos' && !maximized && <ResizeEdges platform={info.platform} />}
    </Fragment>
  );
}
