// ui/src/stacks/feature.ts
import { offStackMenus } from './menus';

/** Stacks: the branch chip menu's Push stack and Rebase stack rows. */
const offs = [...offStackMenus];
import.meta.hot?.dispose(() => { for (const off of offs) off(); });
