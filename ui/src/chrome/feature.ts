import { AppWindow } from 'lucide-react';
import { registerActions, type Action } from '../app/actions';
import { useWindowChrome, windowOps } from './windowChrome';

/** Alt+Space: the window menu, as Windows opens it for a captioned window. The keys go to the page
 * (Chromium's window has no menu of its own), so GitVolt's title bar asks for it. Only on Windows
 * under that title bar: on Linux the window manager takes Alt+Space itself. */
export const WINDOW_MENU: Action = {
  id: 'window.menu', label: 'Window menu', group: 'View', icon: AppWindow,
  tooltip: 'Open the window menu: restore, move, size, minimize, maximize, close', shortcuts: ['Alt+Space'], menu: false,
  when: () => {
    const info = useWindowChrome.getState().info;
    return info?.platform === 'windows' && info.frame === 'custom';
  },
  run: async () => { await windowOps.windowManager('keyboard-menu'); },
};

const info = useWindowChrome.getState().info;
const off = registerActions(info?.platform === 'windows' && info.frame === 'custom' ? [WINDOW_MENU] : []);
import.meta.hot?.dispose(off);
