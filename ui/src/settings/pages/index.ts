/**
 * Settings' own pages, each registering itself (and its settings) on import, in nav order within
 * its scope. Other features register their pages and settings from their own modules (see
 * `../registry.ts`); `app/features.ts` imports them after this one.
 */
import './general';
import './appearance';
import './theme';
import './graph';
import './diffs';
import './fetch';
import './keyboard';
import './advanced';
import './gitIdentity';
import './editorFolders';
import './accounts';
import './hosts';
import './repository';
