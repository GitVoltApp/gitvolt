import { textmateThemeToMonacoTheme } from '@shikijs/monaco';
import { bindEditorTheme, currentEditorTheme } from '../../theme/editorThemes';
import { highlightClient } from '../highlight/client';
import { setTokenTheme, useWorkerTokens } from '../highlight/modelTokens';
import { bundledLanguages } from '../shikiLanguages';
import type { Monaco } from './setup';

export type { CodeTokens } from '../highlight/protocol';

// Shiki's grammars and the Oniguruma WASM run in the highlight worker (diff/highlight/), started
// with the first diff (spec §10.3). Every theme carries its GitVolt editor colours (theme.ts), and
// its token colours in the worker's numbering (`encodedTokensColors`), so the worker's tokens show
// in Monaco as they are.
let themed: Promise<void> | undefined;
type ThemeData = Parameters<Monaco['editor']['defineTheme']>[1];

/** Defines the editor themes and applies the app's. It must resolve before the first editor is created:
 * Monaco falls back to its light `vs` theme for an unknown theme name, and a later defineTheme
 * doesn't re-apply it (so a plain-text-only session would stay light). */
export function ensureTheme(monaco: Monaco): Promise<void> {
  return (themed ??= highlightClient().then(
    (c) => {
      for (const t of c.themes) monaco.editor.defineTheme(t.name, { ...(textmateThemeToMonacoTheme(t.theme) as unknown as ThemeData), encodedTokensColors: t.colors });
      // Before the host's own binding (it calls Monaco's setTheme): open texts get their tokens in
      // the new theme's colours in the same task as Monaco switches.
      bindEditorTheme(setTokenTheme);
      monaco.editor.setTheme(currentEditorTheme());
    },
    (e: unknown) => {
      themed = undefined;
      throw e;
    },
  ));
}

const languages = new Map<string, Promise<string>>();

async function load(monaco: Monaco, lang: string): Promise<string> {
  await ensureTheme(monaco);
  const c = await highlightClient();
  const loaded = await c.loadLanguage(lang);
  if (!loaded) throw new Error(`the ${lang} grammar failed to load`);
  const known = new Set(monaco.languages.getLanguages().map((l) => l.id));
  for (const id of [lang, ...loaded]) {
    if (known.has(id)) continue;
    monaco.languages.register({ id });
    known.add(id);
  }
  useWorkerTokens(monaco, c, lang);
  return lang;
}

/** Loads a language's grammar the first time a file of that language opens. Resolves to the
 * Monaco language id to use (`plaintext` for anything Shiki doesn't ship, or GitVolt dropped). */
export function ensureLanguage(monaco: Monaco, lang: string): Promise<string> {
  if (!Object.hasOwn(bundledLanguages, lang)) return Promise.resolve('plaintext');
  let p = languages.get(lang);
  if (!p) {
    p = load(monaco, lang);
    languages.set(lang, p);
    p.catch(() => languages.delete(lang));
  }
  return p;
}
