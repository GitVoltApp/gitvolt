import { describe, expect, it } from 'vitest';
import { EDITOR_FONT_NAME, editorFontFaces, registerEditorFont } from './editorFont';

describe('the bundled editor font', () => {
  it('JetBrains Mono: regular, italic, bold and bold italic, in five subsets, each for its own characters', () => {
    const faces = editorFontFaces();
    expect(EDITOR_FONT_NAME).toBe('JetBrains Mono');
    expect(faces).toHaveLength(20);
    expect(new Set(faces.map((f) => `${f.weight} ${f.style}`))).toEqual(new Set(['400 normal', '400 italic', '700 normal', '700 italic']));
    for (const f of faces) expect(f.url).toMatch(/jetbrains-mono-(latin|latin-ext|vietnamese|cyrillic|greek)-(400|700)-(normal|italic)\.woff2/);
    // Vietnamese uses the font's own precomposed letters and stacked marks (ố, ể).
    const vietnamese = faces.filter((f) => f.url.includes('-vietnamese-'));
    expect(vietnamese).toHaveLength(4);
    for (const f of vietnamese) expect(f.unicodeRange).toContain('U+1EA0-1EF9');
    expect(faces.find((f) => f.url.includes('-latin-400-normal'))?.unicodeRange).toContain('U+0000-00FF');
  });

  it('registers each face with the document, unloaded until text uses it', () => {
    const added: FontFace[] = [];
    const calls: unknown[][] = [];
    globalThis.FontFace = class { constructor(...a: unknown[]) { calls.push(a); } } as unknown as typeof FontFace;
    try {
      registerEditorFont({ add: (f: FontFace) => added.push(f) } as unknown as FontFaceSet);
    } finally {
      delete (globalThis as { FontFace?: unknown }).FontFace;
    }
    expect(added).toHaveLength(20);
    expect(calls[0]).toEqual(['JetBrains Mono', expect.stringMatching(/^url\(".+\.woff2"\) format\('woff2'\)$/), expect.objectContaining({ weight: '400', style: 'normal', display: 'swap', unicodeRange: expect.any(String) })]);
    // Without a FontFace (no DOM), nothing.
    expect(() => registerEditorFont(undefined)).not.toThrow();
  });
});
