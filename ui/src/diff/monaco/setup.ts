// Lean Monaco: the core editor API plus every editor and diff-editor contribution (find,
// folding, sticky scroll, the diff editor's own actions, …), but no Monarch languages and no
// TypeScript/CSS/HTML/JSON language services (spec §10.2); Shiki tokenizes (§10.3).
// monaco-editor 0.57's exports map is "./*" -> "./esm/vs/*.js", so deep paths drop "esm/vs/"
// (verified in plan 1B; the WebKit spike's `monaco-editor/esm/vs/...` paths don't resolve).
import * as monaco from 'monaco-editor/editor/editor.api';
import 'monaco-editor/features/register.all';
import EditorWorker from 'monaco-editor/editor/editor.worker.js?worker';
import { api, errorMessage } from '../../api/client';
import { useToast } from '../../ui/toastStore';

self.MonacoEnvironment = { getWorker: () => new EditorWorker() };

let linksToBrowser = false;

/** Ctrl+click on a link in an editor: Monaco's own opener calls `window.open`, which CEF turns into
 * a Chromium window of the app's. Web and mail links go to the default browser instead, as every
 * other link in the app does (the core checks the URL). Called as an editor is created, not on
 * import: registering starts Monaco's services, which nothing should do before an editor needs them. */
export function openLinksInBrowser(): void {
  if (linksToBrowser) return;
  linksToBrowser = true;
  monaco.editor.registerLinkOpener({
    open(uri) {
      if (!['http', 'https', 'mailto'].includes(uri.scheme)) return false;
      api.openUrl(uri.toString(true)).catch((e: unknown) => useToast.getState().show(errorMessage(e), { error: true }));
      return true;
    },
  });
}

export { monaco };
export type Monaco = typeof monaco;
