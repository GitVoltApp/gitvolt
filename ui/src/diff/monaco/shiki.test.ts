import { describe, expect, it, vi } from 'vitest';
import type { Monaco } from './setup';

const highlightClient = vi.fn();
vi.mock('../highlight/client', () => ({ highlightClient }));
const useWorkerTokens = vi.fn();
vi.mock('../highlight/modelTokens', () => ({ useWorkerTokens, setTokenTheme: vi.fn() }));

const defineTheme = vi.fn();
const register = vi.fn();
const monaco = { editor: { setTheme: vi.fn(), defineTheme }, languages: { getLanguages: () => [{ id: 'plaintext' }], register } } as unknown as Monaco;
const client = {
  themes: [{ name: 'dark-plus', theme: { type: 'dark', colors: { 'editor.background': '#1e1e1e' }, settings: [{ scope: 'keyword', settings: { foreground: '#569cd6' } }] }, colors: ['#D4D4D4', '#1E1E1E', '#569CD6'] }],
  loadLanguage: vi.fn(async (lang: string) => (lang === 'broken' ? null : [lang, 'html'])),
};

describe('Shiki highlighter', () => {
  it('retries after the highlighter itself failed to start, instead of caching the failure', async () => {
    highlightClient.mockRejectedValueOnce(new Error('wasm failed')).mockResolvedValue(client);
    const { ensureLanguage } = await import('./shiki');
    await expect(ensureLanguage(monaco, 'php')).rejects.toThrow('wasm failed');
    await expect(ensureLanguage(monaco, 'php')).resolves.toBe('php');
    await expect(ensureLanguage(monaco, 'not-a-shiki-language')).resolves.toBe('plaintext');
    // Dropped from the bundle for their licenses, or for stating none (build/license-exceptions.json): plain text, no error.
    await expect(ensureLanguage(monaco, 'nginx')).resolves.toBe('plaintext');
    await expect(ensureLanguage(monaco, 'ada')).resolves.toBe('plaintext');
    await expect(ensureLanguage(monaco, 'dax')).resolves.toBe('plaintext');
  });

  it('defines every theme with its token colours in the worker\'s numbering, and highlights a language through the worker', async () => {
    vi.resetModules();
    highlightClient.mockReset().mockResolvedValue(client);
    defineTheme.mockClear();
    register.mockClear();
    const { ensureLanguage } = await import('./shiki');
    expect(await ensureLanguage(monaco, 'typescript')).toBe('typescript');
    expect(defineTheme).toHaveBeenCalledWith('dark-plus', expect.objectContaining({
      base: 'vs-dark',
      colors: { 'editor.background': '#1e1e1e' },
      rules: [expect.objectContaining({ token: 'keyword', foreground: '569cd6' })],
      encodedTokensColors: ['#D4D4D4', '#1E1E1E', '#569CD6'],
    }));
    // The language and the ones its grammar embeds become Monaco languages; it gets the worker's tokens.
    expect(register.mock.calls.map(([l]) => l.id)).toEqual(['typescript', 'html']);
    expect(useWorkerTokens).toHaveBeenCalledWith(monaco, client, 'typescript');
    // A grammar that fails to load is tried again next time.
    await expect(ensureLanguage(monaco, 'php')).resolves.toBe('php');
    client.loadLanguage.mockResolvedValueOnce(null);
    await expect(ensureLanguage(monaco, 'ruby')).rejects.toThrow('ruby');
    await expect(ensureLanguage(monaco, 'ruby')).resolves.toBe('ruby');
  });
});
