// JetBrains Mono, the editors' default font (diff/options.ts, DEFAULT_EDITOR_FONT_FAMILY), bundled
// from @fontsource/jetbrains-mono (OFL-1.1: its license is in the UI's notices, docs/licensing.md).
// Regular, bold, italic and bold italic, since editor themes use bold and italic; five subsets, each
// a face for its own characters (unicode-range), so Vietnamese uses JetBrains Mono's own marks. The
// files are the app's own assets (font-src is 'self'; vite.config.ts never inlines a font), and a
// face is only fetched once text needs it.
import RANGES from '@fontsource/jetbrains-mono/unicode.json' with { type: 'json' };
import cyrillic400 from '@fontsource/jetbrains-mono/files/jetbrains-mono-cyrillic-400-normal.woff2';
import cyrillic400i from '@fontsource/jetbrains-mono/files/jetbrains-mono-cyrillic-400-italic.woff2';
import cyrillic700 from '@fontsource/jetbrains-mono/files/jetbrains-mono-cyrillic-700-normal.woff2';
import cyrillic700i from '@fontsource/jetbrains-mono/files/jetbrains-mono-cyrillic-700-italic.woff2';
import greek400 from '@fontsource/jetbrains-mono/files/jetbrains-mono-greek-400-normal.woff2';
import greek400i from '@fontsource/jetbrains-mono/files/jetbrains-mono-greek-400-italic.woff2';
import greek700 from '@fontsource/jetbrains-mono/files/jetbrains-mono-greek-700-normal.woff2';
import greek700i from '@fontsource/jetbrains-mono/files/jetbrains-mono-greek-700-italic.woff2';
import latin400 from '@fontsource/jetbrains-mono/files/jetbrains-mono-latin-400-normal.woff2';
import latin400i from '@fontsource/jetbrains-mono/files/jetbrains-mono-latin-400-italic.woff2';
import latin700 from '@fontsource/jetbrains-mono/files/jetbrains-mono-latin-700-normal.woff2';
import latin700i from '@fontsource/jetbrains-mono/files/jetbrains-mono-latin-700-italic.woff2';
import latinExt400 from '@fontsource/jetbrains-mono/files/jetbrains-mono-latin-ext-400-normal.woff2';
import latinExt400i from '@fontsource/jetbrains-mono/files/jetbrains-mono-latin-ext-400-italic.woff2';
import latinExt700 from '@fontsource/jetbrains-mono/files/jetbrains-mono-latin-ext-700-normal.woff2';
import latinExt700i from '@fontsource/jetbrains-mono/files/jetbrains-mono-latin-ext-700-italic.woff2';
import vietnamese400 from '@fontsource/jetbrains-mono/files/jetbrains-mono-vietnamese-400-normal.woff2';
import vietnamese400i from '@fontsource/jetbrains-mono/files/jetbrains-mono-vietnamese-400-italic.woff2';
import vietnamese700 from '@fontsource/jetbrains-mono/files/jetbrains-mono-vietnamese-700-normal.woff2';
import vietnamese700i from '@fontsource/jetbrains-mono/files/jetbrains-mono-vietnamese-700-italic.woff2';

export const EDITOR_FONT_NAME = 'JetBrains Mono';

type Subset = 'latin' | 'latin-ext' | 'vietnamese' | 'cyrillic' | 'greek';
/** Each subset's files: regular, italic, bold, bold italic. */
const FILES: Record<Subset, readonly [string, string, string, string]> = {
  latin: [latin400, latin400i, latin700, latin700i],
  'latin-ext': [latinExt400, latinExt400i, latinExt700, latinExt700i],
  vietnamese: [vietnamese400, vietnamese400i, vietnamese700, vietnamese700i],
  cyrillic: [cyrillic400, cyrillic400i, cyrillic700, cyrillic700i],
  greek: [greek400, greek400i, greek700, greek700i],
};
const STYLES = [['400', 'normal'], ['400', 'italic'], ['700', 'normal'], ['700', 'italic']] as const;

/** The faces' descriptors and sources, as `@font-face` would declare them. */
export function editorFontFaces(): { url: string; weight: string; style: string; unicodeRange: string }[] {
  return (Object.keys(FILES) as Subset[]).flatMap((subset) => STYLES.map(([weight, style], i) => ({ url: FILES[subset][i], weight, style, unicodeRange: RANGES[subset] })));
}

/** Adds the faces to the document's fonts (unloaded: each loads when text first uses it). Call
 * once, before the first render. */
export function registerEditorFont(fonts: FontFaceSet | undefined = globalThis.document?.fonts): void {
  if (!fonts || typeof FontFace === 'undefined') return;
  for (const f of editorFontFaces()) {
    fonts.add(new FontFace(EDITOR_FONT_NAME, `url(${JSON.stringify(f.url)}) format('woff2')`, { weight: f.weight, style: f.style, unicodeRange: f.unicodeRange, display: 'swap' }));
  }
}
