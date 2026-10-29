import { windowOps, type ResizeDirection } from './windowChrome';

/** The window's edges and corners, clockwise from the top: a press there has the window manager
 * resize the window from it. */
export const EDGES: ReadonlyArray<readonly [string, ResizeDirection]> = [
  ['n', 'North'], ['ne', 'NorthEast'], ['e', 'East'], ['se', 'SouthEast'],
  ['s', 'South'], ['sw', 'SouthWest'], ['w', 'West'], ['nw', 'NorthWest'],
];

/** With no system frame, the window's own resize handles: a few px along each edge, a little
 * more at the corners, over the page, with the matching cursors. A maximized window has none. */
export function ResizeEdges() {
  return (
    <div className="window-edges" aria-hidden>
      {EDGES.map(([side, direction]) => (
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
