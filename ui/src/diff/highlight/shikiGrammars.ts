// The engine's grammars: one Shiki highlighter with the Oniguruma WASM engine (inlined as base64 by
// shiki/wasm) and every editor theme, in the highlight worker (or, without one, on the main thread).
import { createHighlighterCore, type HighlighterCore, type ThemeRegistration } from 'shiki/core';
import { createOnigurumaEngine } from 'shiki/engine/oniguruma';
import { bundledLanguages } from '../shikiLanguages';
import { monacoColors } from './colors';
import type { Grammar, Grammars } from './engine';
import type { ThemeInfo } from './protocol';

export function shikiGrammars(): Grammars {
  let h: HighlighterCore | null = null;
  const loaded = new Set<string>();
  const remaps = new Map<string, Uint16Array | null>();
  const highlighter = () => {
    if (!h) throw new Error('highlighter not started');
    return h;
  };
  const theme = (name: string) => {
    const { colorMap } = highlighter().setTheme(name);
    let r = remaps.get(name);
    if (r === undefined) remaps.set(name, (r = monacoColors(colorMap).remap));
    return r;
  };
  return {
    async init(themes: ThemeRegistration[]): Promise<ThemeInfo[]> {
      h = await createHighlighterCore({ themes, langs: [], engine: createOnigurumaEngine(import('shiki/wasm')) });
      return h.getLoadedThemes().map((name) => {
        const { theme: resolved, colorMap } = h!.setTheme(name);
        return { name, theme: resolved, colors: monacoColors(colorMap).colors };
      });
    },
    has: (lang) => Object.hasOwn(bundledLanguages, lang),
    loaded: (lang) => loaded.has(lang),
    async load(lang) {
      await highlighter().loadLanguage(bundledLanguages[lang]);
      loaded.add(lang);
      return highlighter().getLoadedLanguages();
    },
    grammar: (lang) => highlighter().getLanguage(lang) as unknown as Grammar,
    theme,
    code(code, lang, name) {
      const r = highlighter().codeToTokens(code, { lang, theme: name });
      return { lines: r.tokens.map((line) => line.map((t) => ({ content: t.content, color: t.color, fontStyle: t.fontStyle }))), fg: r.fg, bg: r.bg };
    },
  };
}
