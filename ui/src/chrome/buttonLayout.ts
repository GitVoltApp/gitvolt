/** A window button the title bar draws, or a gap (GNOME's `spacer`). */
export type WindowButton = 'minimize' | 'maximize' | 'close' | 'spacer';

/** The buttons at each end of the title bar, in order. ☰ goes right after `start`'s. */
export interface ButtonSets {
  start: WindowButton[];
  end: WindowButton[];
}

const BUTTONS = new Set<WindowButton>(['minimize', 'maximize', 'close', 'spacer']);

/**
 * GNOME's `org.gnome.desktop.wm.preferences button-layout` (GTK's `gtk-decoration-layout`): the
 * buttons left of the `:`, then those right of it, each list comma-separated. `appmenu`, `menu`
 * and `icon` are the app's own menu and icon, which the title bar always draws as ☰ at its
 * start; unknown names and empty entries are skipped, and a button named twice is drawn once.
 * With no `:` every button is on the left, as GTK has it. A gap is kept only between buttons.
 */
export function parseButtonLayout(layout: string): ButtonSets {
  const colon = layout.indexOf(':');
  const sides = colon < 0 ? [layout, ''] : [layout.slice(0, colon), layout.slice(colon + 1)];
  const seen = new Set<WindowButton>();
  const side = (list: string): WindowButton[] => {
    const out: WindowButton[] = [];
    for (const raw of list.split(',')) {
      const name = raw.trim() as WindowButton;
      if (!BUTTONS.has(name)) continue;
      if (name !== 'spacer') {
        if (seen.has(name)) continue;
        seen.add(name);
      }
      out.push(name);
    }
    while (out[0] === 'spacer') out.shift();
    while (out.at(-1) === 'spacer') out.pop();
    return out.filter((b, i) => b !== 'spacer' || out[i - 1] !== 'spacer');
  };
  const [start, end] = sides.map(side);
  return { start, end };
}

/** What a platform draws: Linux follows the layout; Windows has its three buttons on the right
 * whatever the layout; macOS has its own traffic lights at the start (the title bar keeps their
 * slot), none drawn by the page. */
export function buttonSets(platform: string, layout: string): ButtonSets {
  if (platform === 'windows') return { start: [], end: ['minimize', 'maximize', 'close'] };
  if (platform === 'macos') return { start: [], end: [] };
  return parseButtonLayout(layout);
}
