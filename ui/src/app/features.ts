/**
 * Every shell feature, imported once for its registrations (actions, slots, menus): each feature
 * module registers what it contributes (`registerActions`, `registerAppSlot` /
 * `registerTabSlot`, `registerMenu`) at import time, so features never edit each other's files.
 * Add one line per feature module, in the order its slot entries should appear
 * when they share an `order`. Each module releases its registrations on a hot update
 * (`import.meta.hot?.dispose(off)`), or the re-run registration throws "already registered".
 */
import './coreActions';
import '../toolbar/feature';
import '../undo/feature';
import '../stage/feature';
import '../banner/feature';
import '../statusbar/feature';
import '../debug/feature';
import '../auth/feature';
import '../tabs/features';
import '../sidebar/actions';
import '../find/actions';
import '../open/feature';
import '../settings/feature';
import '../theme/feature';
import '../palette/feature';
import '../worktrees/feature';
import '../branches/feature';
import '../stash/feature';
import '../conflicts/menus';
import '../conflicts/feature';
import '../sync/pushFeature';
import '../integrate/feature';
import '../sync/pullFeature';
import '../branches/checkoutMenus';
import '../stacks/feature';
import '../tags/feature';
import '../gitflow/feature';
import '../history/feature';
import '../irebase/feature';
import '../pick/feature';
import '../files/createFileFeature';
import '../shortcuts/feature';
import '../ui/flyout/feature';
import '../remotes/feature';
import '../forge/mrview/feature';
// The sidebar MR/PR row's menu.
import '../forge/mrMenu';
import '../forge/entryPoints';
import '../forge/create/feature';
import '../forge/stack/createFeature';
import '../forge/stack/afterMergeFeature';
// Review comments: the top bar's review chip and Submit review….
import '../forge/review/feature';

import '../nav/feature';
import '../nav/mrPlaces';
import '../nav/repoPlaces';
// --- Shortcuts pass: the keyboard's actions (staging, commit, diff, focus, MR/PR) ---
import '../stage/keyActions';
import '../commit/keyActions';
import '../diff/keyActions';
import '../repo/focusActions';
import '../forge/mrview/keyActions';
// --- end Shortcuts pass ---
// Updates from GitHub Releases: Help → Check for updates and the update dialog.
import '../updates/feature';
// What's new: the bell's "Updated to GitVolt …" after an update, and Help → What's new.
import '../updates/whatsNewFeature';
// Paste anywhere to go to it: Ctrl+V outside a text field goes to what the clipboard names.
import '../goto/feature';
// Git LFS: LFS: Pull / Fetch, and a file row's Track / Stop tracking.
import '../lfs/feature';
// Submodules, for repositories that use them: the palette's Submodules: … and a submodule row's menu.
import '../submodules/feature';
// File colours: the file list's menu names a file's scope and offers to colour its type or folder.
import '../fileColours/feature';
// Branch presets: Next branch preset and Choose branch preset….
import '../presets/feature';
import '../presets/branchMenus';
// GitVolt's own title bar: Alt+Space opens the window menu (Windows).
import '../chrome/feature';
