// Lean Monaco: the core editor API plus every editor and diff-editor contribution (find,
// folding, sticky scroll, the diff editor's own actions, …), but no Monarch languages and no
// TypeScript/CSS/HTML/JSON language services (spec §10.2); Shiki tokenizes (§10.3).
// monaco-editor 0.57's exports map is "./*" -> "./esm/vs/*.js", so deep paths drop "esm/vs/"
// (verified in plan 1B; the WebKit spike's `monaco-editor/esm/vs/...` paths don't resolve).
import * as monaco from 'monaco-editor/editor/editor.api';
import 'monaco-editor/features/register.all';
import EditorWorker from 'monaco-editor/editor/editor.worker.js?worker';
import { api, errorMessage } from '../../api/client';
import { useAppState } from '../../app/state';
import { useToast } from '../../ui/toastStore';
import { clampTabSize, EDITOR_FONT_SIZE, editorFontFamily, modelTabOptions } from '../options';

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

/** The font's four faces at once: Monaco measures bold and italic text too. */
const FACES = ['', 'bold ', 'italic ', 'italic bold '];

/**
 * Loads `family`'s faces, then has every editor measure its characters again. Monaco measures them
 * once per font, when the first editor using it lays out: measured while a face is still loading
 * (with the fallback's widths), the cursor and selections drift off the text. Installed fonts
 * resolve at once; a family the browser can't parse loads nothing.
 */
export async function loadEditorFont(family: string): Promise<void> {
  const fonts = globalThis.document?.fonts;
  if (!fonts) return;
  await Promise.all(FACES.map((face) => fonts.load(`${face}${EDITOR_FONT_SIZE}px ${family}`).catch(() => [])));
  monaco.editor.remeasureFonts();
}

let followingSettings = false;

/**
 * Every model's tab width follows the setting (display only: `modelTabOptions`), from its creation
 * on; a new font family is loaded and measured. The editors' other options each follow it where
 * they're made (host.ts, hexPanes.ts, conflicts/editors.ts). Called as an editor is created, as
 * `openLinksInBrowser` is.
 */
export function followEditorSettings(): void {
  if (followingSettings) return;
  followingSettings = true;
  let tabSize = useAppState.getState().settings.editorTabSize;
  monaco.editor.onDidCreateModel((m) => m.updateOptions(modelTabOptions(m.getOptions(), tabSize)));
  useAppState.subscribe(({ settings: s }, { settings: prev }) => {
    if (s.editorTabSize !== prev.editorTabSize) {
      tabSize = s.editorTabSize;
      for (const m of monaco.editor.getModels()) m.updateOptions({ tabSize: clampTabSize(tabSize) });
    }
    if (s.editorFontFamily !== prev.editorFontFamily) void loadEditorFont(editorFontFamily(s.editorFontFamily));
  });
}

export { monaco };
export type Monaco = typeof monaco;
