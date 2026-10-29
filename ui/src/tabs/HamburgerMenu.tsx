import { Menu } from 'lucide-react';
import { hamburgerRows } from '../app/actions';
import { openMenuAt } from '../menu/menuStore';
import { HoverTooltip } from '../ui/HoverTooltip';

/** Spec §6.1: the app menu (File / Edit / View / Repository / Help), at the start of GitVolt's
 * own title bar, or of the tab bar under the system's. */
export function HamburgerMenu() {
  return (
    <HoverTooltip content="Menu">
      <button
        type="button"
        className="tab-bar-btn"
        aria-label="Menu"
        onClick={(e) => openMenuAt(e.currentTarget, hamburgerRows(), undefined, hamburgerRows, 'Menu')}
      >
        <Menu size={16} aria-hidden />
      </button>
    </HoverTooltip>
  );
}
