import { describe, expect, it, vi } from 'vitest';

// No Worker in Node: the client runs the same engine on this thread, over a fake Shiki.
const codeToTokens = vi.fn(() => ({ tokens: [[{ content: 'const', color: '#569cd6', fontStyle: 0 }, { content: ' a' }]], fg: '#d4d4d4', bg: '#1e1e1e' }));
const loadLanguage = vi.fn(async (l: { name?: string } | undefined) => { if (!l) throw new Error('no grammar'); });
const createHighlighterCore = vi.fn(async () => ({
  getLoadedThemes: () => ['dark-plus'],
  setTheme: () => ({ theme: { name: 'dark-plus', type: 'dark', colors: {}, settings: [] }, colorMap: ['', '#D4D4D4', '#1E1E1E', '#569cd6', '#569CD680'] }),
  loadLanguage,
  getLoadedLanguages: () => ['typescript'],
  codeToTokens,
}));
vi.mock('shiki/core', () => ({ createHighlighterCore }));
vi.mock('shiki/engine/oniguruma', () => ({ createOnigurumaEngine: () => ({}) }));
vi.mock('shiki/wasm', () => ({ default: {} }));

const { highlightClient } = await import('./client');
const { EDITOR_COLORS } = await import('../monaco/theme');

describe('the highlight client', () => {
  it('starts the highlighter with every theme carrying its editor colours, and hands back Monaco token colours', async () => {
    const c = await highlightClient();
    const [{ themes }] = createHighlighterCore.mock.calls[0] as unknown as [{ themes: Array<{ name: string; colors: Record<string, string> }> }];
    expect(themes[0].name).toBe('dark-plus');
    expect(themes[0].colors).toMatchObject(EDITOR_COLORS);
    expect(themes[0].colors['editor.background']).toBeDefined();
    // Monaco's numbering: six hex digits, a colour that differs only in alpha is the same one.
    expect(c.themes).toEqual([{ name: 'dark-plus', theme: expect.objectContaining({ type: 'dark' }), colors: ['#D4D4D4', '#1E1E1E', '#569CD6'] }]);
    expect(await highlightClient()).toBe(c);
  });

  it('tokenizes a Markdown code block in the worker; a cancelled one never asks', async () => {
    const c = await highlightClient();
    expect(await c.code('const a', 'typescript', 'dark-plus').result).toEqual({ lines: [[{ content: 'const', color: '#569cd6', fontStyle: 0 }, { content: ' a', color: undefined, fontStyle: undefined }]], fg: '#d4d4d4', bg: '#1e1e1e' });
    expect(codeToTokens).toHaveBeenLastCalledWith('const a', { lang: 'typescript', theme: 'dark-plus' });
    expect(await c.code('x', 'klingon', 'dark-plus').result).toBeNull();
    codeToTokens.mockClear();
    const gone = c.code('let b', 'typescript', 'dark-plus');
    gone.cancel();
    await new Promise((r) => setTimeout(r, 20));
    expect(codeToTokens).not.toHaveBeenCalled();
  });

  it('loads a grammar once per call, or says it failed', async () => {
    const c = await highlightClient();
    expect(await c.loadLanguage('typescript')).toEqual(['typescript']);
    loadLanguage.mockRejectedValueOnce(new Error('chunk failed'));
    expect(await c.loadLanguage('php')).toBeNull();
  });
});
