// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

const setTheme = vi.fn(async (_kind: string) => {});
vi.mock('../api/transport', () => ({ inTauri: () => true }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ setTheme }) }));

const { useTheme } = await import('./store');
const { THEMES } = await import('./themes');

describe("the window's appearance", () => {
  it("follows the applied theme's kind, once per change", async () => {
    useTheme.getState().set(THEMES.light);
    await vi.waitFor(() => expect(setTheme.mock.calls).toEqual([['light']]));
    useTheme.getState().set(THEMES.nord);
    useTheme.getState().preview({ colors: { 'app-bg0': '#101010' } });
    useTheme.getState().set(THEMES.monokai);
    await vi.waitFor(() => expect(setTheme.mock.calls).toEqual([['light'], ['dark']]));
  });

  it('tries again after a refusal', async () => {
    setTheme.mockClear();
    setTheme.mockRejectedValueOnce(new Error('no'));
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    useTheme.getState().set(THEMES.light);
    await vi.waitFor(() => expect(warn).toHaveBeenCalled());
    useTheme.getState().set(THEMES.light);
    await vi.waitFor(() => expect(setTheme.mock.calls).toEqual([['light'], ['light']]));
    warn.mockRestore();
  });
});
