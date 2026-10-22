import { textmateThemeToMonacoTheme } from '@shikijs/monaco';
import { bindTokenTheme, editorThemeName } from '../../theme/editorThemes';
import { useTheme } from '../../theme/store';
import type { ThemeDef } from '../../theme/themes';
import { editorColors } from './theme';
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

/** The built-in editor themes as defined, by name: a custom theme's starts from its base's. */
const builtIn = new Map<string, ThemeData>();
const customDefined = new Set<string>();

/** Defines (once per name) `def`'s Monaco theme and returns its name: a built-in's as is; a custom
 * theme's is its base's token colours under its own overlay (editorThemes.ts `editorThemeName`). */
export function defineEditorTheme(monaco: Monaco, def: ThemeDef): string {
  const name = editorThemeName(def);
  if (name === def.editorTheme || customDefined.has(name)) return name;
  const base = builtIn.get(def.editorTheme);
  // Not loaded yet: the base's own, until ensureTheme has run.
  if (!base) return def.editorTheme;
  monaco.editor.defineTheme(name, { ...base, colors: { ...base.colors, ...editorColors(def) } });
  customDefined.add(name);
  return name;
}

/** The app theme's Monaco theme, defined: what a new editor is created with. */
export const appEditorTheme = (monaco: Monaco): string => defineEditorTheme(monaco, useTheme.getState().def);

/** Defines the editor themes and applies the app's. It must resolve before the first editor is created:
 * Monaco falls back to its light `vs` theme for an unknown theme name, and a later defineTheme
 * doesn't re-apply it (so a plain-text-only session would stay light). */
export function ensureTheme(monaco: Monaco): Promise<void> {
  return (themed ??= highlightClient().then(
    (c) => {
      for (const t of c.themes) {
        const data: ThemeData = { ...(textmateThemeToMonacoTheme(t.theme) as unknown as ThemeData), encodedTokensColors: t.colors };
        builtIn.set(t.name, data);
        monaco.editor.defineTheme(t.name, data);
      }
      // Before the host's own binding (it calls Monaco's setTheme): open texts get their tokens in
      // the new theme's colours in the same task as Monaco switches.
      bindTokenTheme(setTokenTheme);
      monaco.editor.setTheme(appEditorTheme(monaco));
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
