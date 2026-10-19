import { describe, expect, it, vi } from 'vitest';

const worker = vi.hoisted(() => ({ code: vi.fn(), cancel: vi.fn() }));
vi.mock('../diff/highlight/client', () => ({
  highlightClient: async () => ({
    code: (code: string, lang: string, theme: string) => {
      worker.code(code, lang, theme);
      return { result: Promise.resolve({ lines: [[{ content: code }]] }), cancel: worker.cancel };
    },
  }),
}));
const { queueHighlight } = await import('./highlightQueue');

const settle = async () => { for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0)); };

describe('the highlight queue', () => {
  it("tokenizes a block in the highlight worker, in the editor theme; Shiki's language ids any case", async () => {
    expect(await queueHighlight('const a', 'TS').result).toEqual({ lines: [[{ content: 'const a' }]] });
    expect(worker.code).toHaveBeenCalledWith('const a', 'ts', expect.any(String));
  });

  it('a language Shiki lacks (or GitVolt dropped) is plain without asking; a cancelled block never asks', async () => {
    worker.code.mockClear();
    expect(await queueHighlight('x', 'klingon').result).toBeNull();
    expect(await queueHighlight('x', 'racket').result).toBeNull();
    const gone = queueHighlight('y', 'ts');
    gone.cancel();
    await settle();
    expect(worker.code).not.toHaveBeenCalled();
  });
});
