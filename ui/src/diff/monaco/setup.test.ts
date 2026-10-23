// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest';

const m = vi.hoisted(() => ({ opener: null as null | { open(uri: { scheme: string; toString(skip?: boolean): string }): boolean } }));
vi.mock('monaco-editor/editor/editor.api', () => ({ editor: { registerLinkOpener: (o: typeof m.opener) => { m.opener = o; return { dispose() {} }; } } }));
vi.mock('monaco-editor/features/register.all', () => ({}));
vi.mock('monaco-editor/editor/editor.worker.js?worker', () => ({ default: class {} }));
vi.mock('../../api/client', () => ({ api: { openUrl: vi.fn(async () => null) }, errorMessage: String }));

const { api } = await import('../../api/client');
await import('./setup');

const uri = (s: string) => ({ scheme: s.slice(0, s.indexOf(':')), toString: () => s });

describe("an editor's link", () => {
  beforeEach(() => vi.mocked(api.openUrl).mockClear());

  it('a web or mail link opens in the default browser, never a window of the app', () => {
    for (const url of ['https://example.com/a?b=1', 'http://example.org', 'mailto:ada@example.com']) {
      expect(m.opener!.open(uri(url))).toBe(true);
      expect(api.openUrl).toHaveBeenLastCalledWith(url);
    }
  });

  it('any other link is left to the editor', () => {
    expect(m.opener!.open(uri('file:///etc/passwd'))).toBe(false);
    expect(m.opener!.open(uri('command:editor.action.x'))).toBe(false);
    expect(api.openUrl).not.toHaveBeenCalled();
  });
});
