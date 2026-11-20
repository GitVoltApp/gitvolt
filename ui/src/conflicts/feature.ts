import { registerTabSlot } from '../app/slots';
import { ConflictBanner } from './ConflictBanner';
// The merge tool's kept work joins the leave, tab-close and window-close guards from the
// start (a draft restored after a reload, its tool not opened yet).
import './mergeDrafts';

/** The `banner` slot's merge/rebase banner; after the notices (`banners`). */
const off = registerTabSlot('banner', 'conflict', ConflictBanner, 10);
import.meta.hot?.dispose(off);
