import { offPickMenus } from './menus';

/** Cherry-pick and revert: the Commit group of the commit menu and of the
 * selection's menu. A stop shows in the commit panel (PickControl) and the merge tool. */
const offs = [...offPickMenus];
import.meta.hot?.dispose(() => { for (const off of offs) off(); });
