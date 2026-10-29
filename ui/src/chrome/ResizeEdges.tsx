import { windowOps, type ResizeDirection } from './windowChrome';

/** The window's edges and corners, clockwise from the top: a press there has the window manager
 * resize the window from it. */
export const EDGES: ReadonlyArray<readonly [string, ResizeDirection]> = [
  ['n', 'North'], ['ne', 'NorthEast'], ['e', 'East'], ['se', 'SouthEast'],
  ['s', 'South'], ['sw', 'SouthWest'], ['w', 'West'], ['nw', 'NorthWest'],
];

/** The edges the page resizes from: all of them, but on Windows only the top ones, since the
 * window keeps the system's resize border at its sides and bottom (the runtime's
 * `platform/windows/chrome.rs`). */
export function pageEdges(platform: string): ReadonlyArray<readonly [string, ResizeDirection]> {
  return platform === 'windows' ? EDGES.filter(([side]) => side.startsWith('n')) : EDGES;
}

/** With no system frame, the window's own resize handles: a few px along each edge, a little
 * more at the corners, over the page, with the matching cursors. A maximized window has none. */
export function ResizeEdges({ platform }: { platform: string }) {
  return (
    <div className="window-edges" aria-hidden>
      {pageEdges(platform).map(([side, direction]) => (
        <div
          key={side}
          className={`window-edge window-edge-${side}`}
          data-direction={direction}
          onMouseDown={(e) => {
            if (e.button !== 0) return;
            e.preventDefault();
            void windowOps.startResizeDragging(direction).catch((err: unknown) => console.warn('[gitvolt] window resize refused', err));
          }}
        />
      ))}
    </div>
  );
}
